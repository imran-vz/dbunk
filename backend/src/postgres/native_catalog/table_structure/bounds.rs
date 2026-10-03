use super::*;
use std::mem::size_of;
trait Heap {
    fn heap(&self) -> Option<usize>;
}
impl Heap for String {
    fn heap(&self) -> Option<usize> {
        (self.len() <= MAX_STRUCTURE_DEFINITION_BYTES && !self.contains('\0'))
            .then_some(self.capacity())
    }
}
impl<T: Heap> Heap for Option<T> {
    fn heap(&self) -> Option<usize> {
        self.as_ref().map_or(Some(0), Heap::heap)
    }
}
impl<T: Heap> Heap for Vec<T> {
    fn heap(&self) -> Option<usize> {
        if self.len() > MAX_STRUCTURE_COMPONENTS {
            return None;
        }
        self.iter()
            .try_fold(self.capacity().checked_mul(size_of::<T>())?, |n, v| {
                n.checked_add(v.heap()?)
            })
    }
}
macro_rules! heap {($t:ty:$($field:ident),+)=>{impl Heap for $t{fn heap(&self)->Option<usize>{let n=0usize;$(let n=n.checked_add(self.$field.heap()?)?;)+Some(n)}}};}
heap!(StructureColumn:name,data_type,default_expression,comment,collation_schema,collation_name);
heap!(StructureKeyColumn:name);
heap!(StructurePrimaryKey:name,columns);
heap!(StructureKeyPair:source,target);
heap!(StructureForeignKey:name,source_schema,source_table,target_schema,target_table,columns,match_type);
heap!(StructureIndexKey:column_name,definition);
heap!(StructureIndex:name,method,keys,predicate,definition);
heap!(StructureConstraint:name,kind,definition);
heap!(StructureTrigger:name,timing,events,update_columns,level,function_schema,function_name,definition);
heap!(StructurePolicy:name,roles,using_expression,with_check);
heap!(StructurePrivilege:grantor,grantee,privilege);
heap!(StructureRule:name,event,definition);
heap!(StructureRelative:schema,name,bound);
heap!(TableStructureSnapshot:schema,table,owner,comment,captured_at,columns,primary_key,outbound,inbound,indexes,constraints,triggers,policies,privileges,rules,partition_key,partition_bound,parents,partitions);
fn name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 63 && !s.contains('\0')
}
fn optional_name(s: &Option<String>) -> bool {
    s.as_deref().is_none_or(name)
}
fn metadata(s: &str) -> bool {
    s.len() <= MAX_STRUCTURE_METADATA_BYTES && !s.contains('\0')
}
fn key_columns(columns: &[StructureKeyColumn], max: usize) -> bool {
    columns.len() <= max
        && columns.iter().enumerate().all(|(i, c)| {
            c.number > 0
                && name(&c.name)
                && !columns[..i]
                    .iter()
                    .any(|p| p.number == c.number || p.name == c.name)
        })
}
fn unique<T>(rows: &[T], id: impl Fn(&T) -> u32) -> bool {
    rows.iter()
        .enumerate()
        .all(|(i, r)| id(r) > 0 && !rows[..i].iter().any(|p| id(p) == id(r)))
}
fn foreign_key(f: &StructureForeignKey) -> bool {
    f.oid > 0
        && f.source_oid > 0
        && f.target_oid > 0
        && [
            &f.name,
            &f.source_schema,
            &f.source_table,
            &f.target_schema,
            &f.target_table,
        ]
        .iter()
        .all(|s| name(s))
        && !f.columns.is_empty()
        && f.columns.len() <= 64
        && matches!(f.match_type.as_str(), "SIMPLE" | "FULL" | "PARTIAL")
        && f.columns.iter().enumerate().all(|(i, p)| {
            p.source_number > 0
                && p.target_number > 0
                && name(&p.source)
                && name(&p.target)
                && !f.columns[..i].iter().any(|old| {
                    old.source_number == p.source_number || old.target_number == p.target_number
                })
        })
}
impl TableStructureSnapshot {
    /// Checks exact metadata shape and owned capacities, including spare capacity.
    /// Definitions are retained text only, never executable authority.
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.server_version < MIN_STRUCTURE_SERVER_VERSION
            || self.identity.database_oid == 0
            || self.identity.relation_oid == 0
            || ![&self.schema, &self.table, &self.owner]
                .iter()
                .all(|s| name(s))
            || self.comment.as_deref().is_some_and(|s| !metadata(s))
            || self.captured_at.len() > 64
            || self.columns.len() > MAX_STRUCTURE_COLUMNS
            || self.is_partition != self.partition_bound.is_some()
        {
            return None;
        }
        if self.columns.iter().enumerate().any(|(i, c)| {
            c.number <= 0
                || !name(&c.name)
                || !metadata(&c.data_type)
                || c.comment.as_deref().is_some_and(|s| !metadata(s))
                || !optional_name(&c.collation_schema)
                || !optional_name(&c.collation_name)
                || c.collation_schema.is_some() != c.collation_name.is_some()
                || c.primary_key_position == Some(0)
                || (i > 0 && self.columns[i - 1].number >= c.number)
                || self.columns[..i].iter().any(|old| old.name == c.name)
        }) {
            return None;
        }
        let mut components = 1 + self.columns.len();
        if let Some(pk) = &self.primary_key {
            if pk.oid == 0
                || !name(&pk.name)
                || pk.columns.is_empty()
                || !key_columns(&pk.columns, 64)
            {
                return None;
            }
            components += 1 + pk.columns.len();
            for (i, key) in pk.columns.iter().enumerate() {
                if !self.columns.iter().any(|c| {
                    c.number == key.number
                        && c.name == key.name
                        && c.primary_key_position == Some((i + 1) as u16)
                }) {
                    return None;
                }
            }
        }
        for column in &self.columns {
            if let Some(position) = column.primary_key_position {
                if !self.primary_key.as_ref().is_some_and(|pk| {
                    pk.columns
                        .get(usize::from(position) - 1)
                        .is_some_and(|c| c.number == column.number && c.name == column.name)
                }) {
                    return None;
                }
            }
        }
        for (inbound, rows) in [(false, &self.outbound), (true, &self.inbound)] {
            if !unique(rows, |f| f.oid) {
                return None;
            }
            for f in rows {
                if !foreign_key(f)
                    || if inbound {
                        f.target_oid != self.identity.relation_oid
                            || f.target_schema != self.schema
                            || f.target_table != self.table
                    } else {
                        f.source_oid != self.identity.relation_oid
                            || f.source_schema != self.schema
                            || f.source_table != self.table
                    }
                {
                    return None;
                }
                components += 1 + f.columns.len();
                for p in &f.columns {
                    let (number, n) = if inbound {
                        (p.target_number, &p.target)
                    } else {
                        (p.source_number, &p.source)
                    };
                    if !self
                        .columns
                        .iter()
                        .any(|c| c.number == number && &c.name == n)
                    {
                        return None;
                    }
                }
            }
        }
        if !unique(&self.indexes, |i| i.oid)
            || !unique(&self.constraints, |c| c.oid)
            || !unique(&self.triggers, |t| t.oid)
            || !unique(&self.policies, |p| p.oid)
            || !unique(&self.rules, |r| r.oid)
            || !unique(&self.parents, |r| r.oid)
            || !unique(&self.partitions, |r| r.oid)
        {
            return None;
        }
        for index in &self.indexes {
            if !name(&index.name)
                || !name(&index.method)
                || index.keys.is_empty()
                || index.keys.len() > 32
            {
                return None;
            }
            let mut included = false;
            for (i, k) in index.keys.iter().enumerate() {
                if usize::from(k.position) != i + 1
                    || k.column_number.is_some() != k.column_name.is_some()
                    || !optional_name(&k.column_name)
                    || k.column_number.is_some_and(|n| {
                        !self
                            .columns
                            .iter()
                            .any(|c| c.number == n && Some(&c.name) == k.column_name.as_ref())
                    })
                    || k.included && k.column_number.is_none()
                    || included && !k.included
                {
                    return None;
                }
                included |= k.included;
            }
            components += 1 + index.keys.len();
        }
        for c in &self.constraints {
            if !name(&c.name) || !metadata(&c.kind) {
                return None;
            }
            components += 1;
        }
        for t in &self.triggers {
            if !name(&t.name)
                || !name(&t.function_schema)
                || !name(&t.function_name)
                || t.function_oid == 0
                || t.parent_trigger_oid == Some(0)
                || !matches!(t.timing.as_str(), "BEFORE" | "AFTER" | "INSTEAD OF")
                || !matches!(t.level.as_str(), "ROW" | "STATEMENT")
                || t.events.is_empty()
                || t.events.len() > 4
                || t.events.iter().enumerate().any(|(i, e)| {
                    !matches!(e.as_str(), "INSERT" | "UPDATE" | "DELETE" | "TRUNCATE")
                        || t.events[..i].contains(e)
                })
                || !key_columns(&t.update_columns, 1600)
            {
                return None;
            }
            if t.update_columns.iter().any(|k| {
                !self
                    .columns
                    .iter()
                    .any(|c| c.number == k.number && c.name == k.name)
            }) {
                return None;
            }
            components += 1 + t.events.len() + t.update_columns.len();
        }
        for p in &self.policies {
            if !name(&p.name) || p.roles.len() > 1024 || p.roles.iter().any(|r| !name(r)) {
                return None;
            }
            components += 1 + p.roles.len();
        }
        for p in &self.privileges {
            if !name(&p.grantor) || !name(&p.grantee) || !metadata(&p.privilege) {
                return None;
            }
            components += 1;
        }
        for r in &self.rules {
            if !name(&r.name)
                || !matches!(r.event.as_str(), "SELECT" | "INSERT" | "UPDATE" | "DELETE")
            {
                return None;
            }
            components += 1;
        }
        for r in self.parents.iter().chain(&self.partitions) {
            if !name(&r.schema)
                || !name(&r.name)
                || r.sequence <= 0
                || r.is_partition != r.bound.is_some()
            {
                return None;
            }
            components += 1;
        }
        if components > MAX_STRUCTURE_COMPONENTS {
            return None;
        }
        let heap = size_of::<Self>().checked_add(self.heap()?)?;
        if heap > MAX_STRUCTURE_BYTES {
            return None;
        }
        self.encoded_bytes()?;
        Some(heap)
    }
    pub fn encoded_bytes(&self) -> Option<usize> {
        struct Count(usize);
        impl std::io::Write for Count {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0 = self
                    .0
                    .checked_add(b.len())
                    .filter(|n| *n <= MAX_STRUCTURE_BYTES)
                    .ok_or_else(|| std::io::Error::other("structure limit"))?;
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut count = Count(0);
        serde_json::to_writer(&mut count, self).ok()?;
        Some(count.0)
    }
}
