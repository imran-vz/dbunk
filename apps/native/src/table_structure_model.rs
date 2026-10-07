//! Typed, read-only Structure presentation. Only the selected row is formatted;
//! definitions remain metadata text and never become execution authority.
use dbunk_lib::backend::table_structure::*;
use std::{
    cell::Cell,
    fmt::{self, Write},
    mem::size_of,
    rc::Rc,
};

const SHARED_BYTES: usize = 128 * 1024 * 1024;
const PRESENTATION_BYTES: usize = 64 * 1024;
// Read-only Editor + AccessibleEditor current/replacement overlap. The view
// disables soft wrapping, so visual wraps cannot add uncounted text runs.
// These are conservative accounting reservations, not measurements of peak RSS.
const DETAIL_BYTE_ALLOWANCE: usize = 64;
const LOGICAL_RUN_ALLOWANCE: usize = 4096;
pub const MAX_DETAIL_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_LABEL_CHARS: usize = 512;
pub const RELATION_ACL_SCOPE: &str = "Explicit relation ACL entries from pg_class.relacl only. Excludes column ACLs, inherited role membership, ownership, default privileges and effective authorization. An empty section does not mean no access.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Overview,
    Columns,
    PrimaryKey,
    ForeignKeys,
    ReferencedBy,
    Indexes,
    Constraints,
    Triggers,
    Policies,
    RelationGrants,
    Rules,
    Parents,
    Children,
}
impl Section {
    pub const ALL: [Self; 13] = [
        Self::Overview,
        Self::Columns,
        Self::PrimaryKey,
        Self::Indexes,
        Self::ForeignKeys,
        Self::ReferencedBy,
        Self::Constraints,
        Self::Triggers,
        Self::Policies,
        Self::RelationGrants,
        Self::Rules,
        Self::Parents,
        Self::Children,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Columns => "Columns",
            Self::PrimaryKey => "Primary key",
            Self::ForeignKeys => "Foreign keys",
            Self::ReferencedBy => "Referenced by",
            Self::Indexes => "Indexes",
            Self::Constraints => "Constraints",
            Self::Triggers => "Triggers",
            Self::Policies => "Policies",
            Self::RelationGrants => "Relation grants",
            Self::Rules => "Rules",
            Self::Parents => "Parents",
            Self::Children => "Children",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Navigation<'a> {
    pub schema: &'a str,
    pub table: &'a str,
    pub identity: TableIdentity,
}
pub struct Capture {
    data: TableStructureSnapshot,
    budget: Rc<Cell<usize>>,
    retained: usize,
    largest_detail: Counter,
}
impl Capture {
    /// The caller replaces its previous capture only after this succeeds. The
    /// lease covers bytes and logical text runs for current/replacement editor
    /// and accessibility text/node/geometry overlap. The view must disable soft
    /// wrapping and retain only one selected editor. Arbitrary retained detail
    /// strings need their own admission; this is not a process RSS guarantee.
    pub fn new(
        data: TableStructureSnapshot,
        budget: Rc<Cell<usize>>,
    ) -> Result<Self, &'static str> {
        let heap = data.checked_heap_bytes().ok_or(
            "Structure capture is invalid or exceeds its bounds; previous capture retained",
        )?;
        let mut largest = Counter::default();
        for section in Section::ALL {
            for index in 0..count(&data, section) {
                let mut counter = Counter::default();
                render(&data,section,index,&mut counter).map_err(|_|"Selected structure detail exceeds its formatting bound; previous capture retained")?;
                largest.bytes = largest.bytes.max(counter.bytes);
                largest.line_breaks = largest.line_breaks.max(counter.line_breaks);
            }
        }
        let retained = heap
            .checked_add(size_of::<Self>())
            .and_then(|n| n.checked_add(largest.editor_allowance()?))
            .and_then(|n| n.checked_add(PRESENTATION_BYTES))
            .ok_or("Structure retention size overflow")?;
        if retained > SHARED_BYTES.saturating_sub(budget.get()) {
            return Err(
                "Structure inspection exceeds available shared allowance; previous capture retained",
            );
        }
        budget.set(budget.get() + retained);
        Ok(Self {
            data,
            budget,
            retained,
            largest_detail: largest,
        })
    }
    /// Carry catalog identity and attnum directly; never parse formatted details.
    pub fn ddl_selection(
        &self,
        section: Section,
        index: usize,
    ) -> Option<crate::table_ddl_model::Selection> {
        if self.data.kind != StructureRelationKind::Table
            || self.data.is_partition
            || !self.data.parents.is_empty()
            || !self.data.partitions.is_empty()
        {
            return None;
        }
        let (column, attnum) = match section {
            Section::Overview if index == 0 => (None, None),
            Section::Columns => {
                let column = self.data.columns.get(index)?;
                (
                    Some(column.name.clone()),
                    Some(i16::try_from(column.number).ok()?),
                )
            }
            _ => return None,
        };
        crate::table_ddl_model::Selection::new(
            dbunk_lib::backend::table_ddl::TableDdlRequest {
                schema: self.data.schema.clone(),
                table: self.data.table.clone(),
                column,
                expected: Some(self.data.identity),
            },
            attnum,
        )
        .ok()
    }
    pub fn identity(&self) -> TableIdentity {
        self.data.identity
    }
    pub fn captured_at(&self) -> &str {
        &self.data.captured_at
    }
    #[cfg(test)]
    pub fn retained_bytes(&self) -> usize {
        self.retained
    }
    pub fn request(&self) -> TableStructureRequest {
        TableStructureRequest {
            schema: self.data.schema.clone(),
            table: self.data.table.clone(),
            expected: Some(self.data.identity),
        }
    }
    pub fn schema(&self) -> &str {
        &self.data.schema
    }
    pub fn table(&self) -> &str {
        &self.data.table
    }
    pub fn kind_label(&self) -> &'static str {
        kind(self.data.kind)
    }
    pub fn qualified_name(&self) -> String {
        format!(
            "{}.{}",
            Identifier(&self.data.schema),
            Identifier(&self.data.table)
        )
    }
    pub fn count(&self, section: Section) -> usize {
        count(&self.data, section)
    }
    /// Labels deliberately omit large definitions, types and privileges. Their
    /// complete original text is available in selected details.
    pub fn row_label(&self, section: Section, index: usize) -> Option<String> {
        if index >= self.count(section) {
            return None;
        }
        let d = &self.data;
        let label = match section {
            Section::Overview => self.qualified_name(),
            Section::Columns => format!(
                "{} · {}{}",
                d.columns[index].number,
                Identifier(&d.columns[index].name),
                if d.columns[index].nullable {
                    " · nullable"
                } else {
                    " · NOT NULL"
                }
            ),
            Section::PrimaryKey => format!(
                "{} · {} columns",
                Identifier(&d.primary_key.as_ref()?.name),
                d.primary_key.as_ref()?.columns.len()
            ),
            Section::ForeignKeys | Section::ReferencedBy => {
                let f = if section == Section::ForeignKeys {
                    &d.outbound[index]
                } else {
                    &d.inbound[index]
                };
                if section == Section::ForeignKeys {
                    format!(
                        "{} → {}.{}",
                        Identifier(&f.name),
                        Identifier(&f.target_schema),
                        Identifier(&f.target_table)
                    )
                } else {
                    format!(
                        "{} ← {}.{}",
                        Identifier(&f.name),
                        Identifier(&f.source_schema),
                        Identifier(&f.source_table)
                    )
                }
            }
            Section::Indexes => format!(
                "{}{}{}",
                Identifier(&d.indexes[index].name),
                if d.indexes[index].unique {
                    " · unique"
                } else {
                    ""
                },
                if !d.indexes[index].valid {
                    " · invalid"
                } else {
                    ""
                }
            ),
            Section::Constraints => Identifier(&d.constraints[index].name).to_string(),
            Section::Triggers => Identifier(&d.triggers[index].name).to_string(),
            Section::Policies => Identifier(&d.policies[index].name).to_string(),
            Section::RelationGrants => format!(
                "{} → {} · grant {}",
                Identifier(&d.privileges[index].grantor),
                Identifier(&d.privileges[index].grantee),
                index + 1
            ),
            Section::Rules => Identifier(&d.rules[index].name).to_string(),
            Section::Parents | Section::Children => {
                let r = if section == Section::Parents {
                    &d.parents[index]
                } else {
                    &d.partitions[index]
                };
                format!(
                    "{}.{}{}",
                    Identifier(&r.schema),
                    Identifier(&r.name),
                    if (section == Section::Parents && d.is_partition)
                        || (section == Section::Children && r.is_partition)
                    {
                        " · partition"
                    } else {
                        " · inheritance"
                    }
                )
            }
        };
        // All label fields are validated 63-byte identifiers. This guard also
        // makes a future DTO expansion fail closed instead of rendering a blob.
        (label.chars().count() <= MAX_LABEL_CHARS).then_some(label)
    }
    pub fn details(&self, section: Section, index: usize) -> Result<Option<String>, &'static str> {
        if index >= self.count(section) {
            return Ok(None);
        }
        let mut counter = Counter::default();
        render(&self.data, section, index, &mut counter)
            .map_err(|_| "Selected structure detail exceeds its formatting bound")?;
        if counter.bytes > self.largest_detail.bytes
            || counter.line_breaks > self.largest_detail.line_breaks
        {
            return Err("Selected structure detail exceeds its retained allowance");
        }
        let mut output = String::with_capacity(counter.bytes);
        render(&self.data, section, index, &mut output)
            .map_err(|_| "Structure formatting failed")?;
        Ok(Some(output))
    }
    /// Navigation is metadata-only and inherits the current connection. The UI
    /// must disable this action after its document/capture becomes stale.
    pub fn navigation(&self, section: Section, index: usize) -> Option<Navigation<'_>> {
        let (schema, table, relation_oid) = match section {
            Section::ForeignKeys => {
                let f = self.data.outbound.get(index)?;
                (&f.target_schema, &f.target_table, f.target_oid)
            }
            Section::ReferencedBy => {
                let f = self.data.inbound.get(index)?;
                (&f.source_schema, &f.source_table, f.source_oid)
            }
            Section::Parents => {
                let r = self.data.parents.get(index)?;
                (&r.schema, &r.name, r.oid)
            }
            Section::Children => {
                let r = self.data.partitions.get(index)?;
                (&r.schema, &r.name, r.oid)
            }
            _ => return None,
        };
        Some(Navigation {
            schema,
            table,
            identity: TableIdentity {
                database_oid: self.data.identity.database_oid,
                relation_oid,
            },
        })
    }
    pub fn empty_label(&self, section: Section) -> &'static str {
        match section {
            Section::RelationGrants => RELATION_ACL_SCOPE,
            Section::PrimaryKey => "No primary key in this capture",
            Section::Triggers => {
                "No user triggers in this capture; internal triggers are outside this section"
            }
            _ => "No entries in this captured section",
        }
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(self.retained));
    }
}
fn count(d: &TableStructureSnapshot, section: Section) -> usize {
    match section {
        Section::Overview => 1,
        Section::Columns => d.columns.len(),
        Section::PrimaryKey => usize::from(d.primary_key.is_some()),
        Section::ForeignKeys => d.outbound.len(),
        Section::ReferencedBy => d.inbound.len(),
        Section::Indexes => d.indexes.len(),
        Section::Constraints => d.constraints.len(),
        Section::Triggers => d.triggers.len(),
        Section::Policies => d.policies.len(),
        Section::RelationGrants => d.privileges.len(),
        Section::Rules => d.rules.len(),
        Section::Parents => d.parents.len(),
        Section::Children => d.partitions.len(),
    }
}
#[derive(Clone, Copy, Default)]
struct Counter {
    bytes: usize,
    line_breaks: usize,
}
impl Counter {
    fn editor_allowance(self) -> Option<usize> {
        self.bytes.checked_mul(DETAIL_BYTE_ALLOWANCE)?.checked_add(
            self.line_breaks
                .checked_add(1)?
                .checked_mul(LOGICAL_RUN_ALLOWANCE)?,
        )
    }
}
impl Write for Counter {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let bytes = self
            .bytes
            .checked_add(text.len())
            .filter(|n| *n <= MAX_DETAIL_BYTES)
            .ok_or(fmt::Error)?;
        // Count CR and LF independently: CRLF deliberately over-reserves, and
        // split formatter writes cannot lose a logical boundary.
        let line_breaks = self
            .line_breaks
            .checked_add(
                text.bytes()
                    .filter(|byte| matches!(*byte, b'\n' | b'\r'))
                    .count(),
            )
            .ok_or(fmt::Error)?;
        self.bytes = bytes;
        self.line_breaks = line_breaks;
        Ok(())
    }
}
struct Identifier<'a>(&'a str);
impl fmt::Display for Identifier<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_char('"')?;
        for c in self.0.chars() {
            if c == '"' {
                f.write_char('"')?;
            }
            f.write_char(c)?;
        }
        f.write_char('"')
    }
}
fn text(w: &mut impl Write, label: &str, value: &str) -> fmt::Result {
    if value.is_empty() {
        writeln!(w, "{label}: empty text (0 UTF-8 bytes)")
    } else {
        writeln!(w, "{label} (text, {} UTF-8 bytes):\n{value}", value.len())
    }
}
fn optional_text(w: &mut impl Write, label: &str, value: &Option<String>) -> fmt::Result {
    match value {
        Some(value) => text(w, label, value),
        None => writeln!(w, "{label}: NULL (absent)"),
    }
}
fn optional_number(
    w: &mut impl Write,
    label: &str,
    value: Option<impl fmt::Display>,
) -> fmt::Result {
    match value {
        Some(value) => writeln!(w, "{label}: {value}"),
        None => writeln!(w, "{label}: NULL (absent)"),
    }
}
fn render(
    d: &TableStructureSnapshot,
    section: Section,
    index: usize,
    w: &mut impl Write,
) -> fmt::Result {
    writeln!(
        w,
        "{} · {}.{}\nDatabase OID: {}; relation OID: {}\nCaptured: {}\n",
        section.label(),
        Identifier(&d.schema),
        Identifier(&d.table),
        d.identity.database_oid,
        d.identity.relation_oid,
        d.captured_at
    )?;
    match section {
        Section::Overview => {
            writeln!(
                w,
                "Kind: {}\nOwner: {}\nServer version number: {}\nRow security enabled: {}\nRow security forced: {}\nIs a partition: {}",
                kind(d.kind),
                Identifier(&d.owner),
                d.server_version,
                d.row_security.enabled,
                d.row_security.forced,
                d.is_partition
            )?;
            optional_text(w, "Relation comment", &d.comment)?;
            optional_text(w, "Partition key", &d.partition_key)?;
            optional_text(w, "Partition bound", &d.partition_bound)?;
            writeln!(
                w,
                "\n{RELATION_ACL_SCOPE}\nDefinitions are catalog renderings, not a canonical schema dump. Metadata may change after capture."
            )
        }
        Section::Columns => {
            let c = &d.columns[index];
            writeln!(
                w,
                "Column: {}\nAttribute number: {}\nNullable: {}\nIdentity: {}\nGenerated: {}",
                Identifier(&c.name),
                c.number,
                c.nullable,
                identity(c.identity),
                generated(c.generated)
            )?;
            text(w, "Data type", &c.data_type)?;
            optional_text(w, "Default / generated expression", &c.default_expression)?;
            optional_text(w, "Comment", &c.comment)?;
            optional_number(w, "Primary-key position", c.primary_key_position)?;
            optional_text(w, "Collation schema", &c.collation_schema)?;
            optional_text(w, "Collation name", &c.collation_name)
        }
        Section::PrimaryKey => {
            let p = d.primary_key.as_ref().ok_or(fmt::Error)?;
            writeln!(
                w,
                "Constraint: {}\nOID: {}\nDeferrable: {}\nInitially deferred: {}\nOrdered key columns:",
                Identifier(&p.name),
                p.oid,
                p.deferrable,
                p.initially_deferred
            )?;
            keys(w, &p.columns)
        }
        Section::ForeignKeys => foreign_key(w, &d.outbound[index]),
        Section::ReferencedBy => foreign_key(w, &d.inbound[index]),
        Section::Indexes => index_detail(w, &d.indexes[index]),
        Section::Constraints => {
            let c = &d.constraints[index];
            writeln!(
                w,
                "Constraint: {}\nOID: {}\nDeferrable: {}\nInitially deferred: {}\nValidated: {}",
                Identifier(&c.name),
                c.oid,
                c.deferrable,
                c.initially_deferred,
                c.validated
            )?;
            text(w, "Kind", &c.kind)?;
            text(w, "Definition", &c.definition)
        }
        Section::Triggers => {
            let t = &d.triggers[index];
            writeln!(
                w,
                "User trigger: {}\nOID: {}\nEnabled: {}\nTiming: {}\nLevel: {}\nFunction: {}.{}\nFunction OID: {}",
                Identifier(&t.name),
                t.oid,
                enabled(t.enabled),
                t.timing,
                t.level,
                Identifier(&t.function_schema),
                Identifier(&t.function_name),
                t.function_oid
            )?;
            optional_number(
                w,
                "Parent trigger OID (inherited/partition clone)",
                t.parent_trigger_oid,
            )?;
            writeln!(w, "Events, in captured order:")?;
            for (i, event) in t.events.iter().enumerate() {
                writeln!(w, "{}. {event}", i + 1)?;
            }
            writeln!(
                w,
                "UPDATE OF columns, in captured order ({}):",
                t.update_columns.len()
            )?;
            keys(w, &t.update_columns)?;
            text(w, "Definition", &t.definition)
        }
        Section::Policies => {
            let p = &d.policies[index];
            writeln!(
                w,
                "Policy: {}\nOID: {}\nPermissive: {}\nCommand: {}\nRelation row security enabled: {}\nRelation row security forced: {}\nRoles, in captured order:",
                Identifier(&p.name),
                p.oid,
                p.permissive,
                command(p.command),
                d.row_security.enabled,
                d.row_security.forced
            )?;
            for (i, role) in p.roles.iter().enumerate() {
                writeln!(w, "{}. {}", i + 1, Identifier(role))?;
            }
            optional_text(w, "USING expression", &p.using_expression)?;
            optional_text(w, "WITH CHECK expression", &p.with_check)
        }
        Section::RelationGrants => {
            let p = &d.privileges[index];
            writeln!(
                w,
                "{RELATION_ACL_SCOPE}\n\nGrantor: {}\nGrantee: {}\nGrant option: {}",
                Identifier(&p.grantor),
                Identifier(&p.grantee),
                p.grantable
            )?;
            text(w, "Privilege", &p.privilege)
        }
        Section::Rules => {
            let r = &d.rules[index];
            writeln!(
                w,
                "Rule: {}\nOID: {}\nEvent: {}\nINSTEAD: {}\nEnabled: {}",
                Identifier(&r.name),
                r.oid,
                r.event,
                r.instead,
                enabled(r.enabled)
            )?;
            text(w, "Definition", &r.definition)
        }
        Section::Parents | Section::Children => {
            let r = if section == Section::Parents {
                &d.parents[index]
            } else {
                &d.partitions[index]
            };
            writeln!(
                w,
                "Relation: {}.{}\nOID: {}\nInheritance sequence: {}\nIs a partition: {}",
                Identifier(&r.schema),
                Identifier(&r.name),
                r.oid,
                r.sequence,
                r.is_partition
            )?;
            optional_text(w, "Partition bound", &r.bound)
        }
    }
}
fn keys(w: &mut impl Write, columns: &[StructureKeyColumn]) -> fmt::Result {
    for (i, c) in columns.iter().enumerate() {
        writeln!(
            w,
            "{}. {} (attribute {})",
            i + 1,
            Identifier(&c.name),
            c.number
        )?;
    }
    Ok(())
}
fn foreign_key(w: &mut impl Write, f: &StructureForeignKey) -> fmt::Result {
    writeln!(
        w,
        "Constraint: {}\nOID: {}\nSource: {}.{}\nSource OID: {}\nTarget: {}.{}\nTarget OID: {}\nON UPDATE: {}\nON DELETE: {}\nMATCH: {}\nDeferrable: {}\nInitially deferred: {}\nValidated: {}\nOrdered column pairs:",
        Identifier(&f.name),
        f.oid,
        Identifier(&f.source_schema),
        Identifier(&f.source_table),
        f.source_oid,
        Identifier(&f.target_schema),
        Identifier(&f.target_table),
        f.target_oid,
        action(f.on_update),
        action(f.on_delete),
        f.match_type,
        f.deferrable,
        f.initially_deferred,
        f.validated
    )?;
    for (i, pair) in f.columns.iter().enumerate() {
        writeln!(
            w,
            "{}. {} (attribute {}) → {} (attribute {})",
            i + 1,
            Identifier(&pair.source),
            pair.source_number,
            Identifier(&pair.target),
            pair.target_number
        )?;
    }
    Ok(())
}
fn index_detail(w: &mut impl Write, i: &StructureIndex) -> fmt::Result {
    writeln!(
        w,
        "Index: {}\nOID: {}\nMethod: {}\nUnique: {}\nPrimary: {}\nValid: {}\nReady: {}",
        Identifier(&i.name),
        i.oid,
        Identifier(&i.method),
        i.unique,
        i.primary,
        i.valid,
        i.ready
    )?;
    optional_number(w, "Owning constraint OID", i.constraint_oid)?;
    writeln!(w, "Ordered index positions:")?;
    for key in &i.keys {
        writeln!(
            w,
            "\nPosition {}: {}",
            key.position,
            if key.included { "INCLUDE" } else { "key" }
        )?;
        optional_number(
            w,
            "Column attribute number (NULL means expression)",
            key.column_number,
        )?;
        optional_text(w, "Column name", &key.column_name)?;
        text(w, "Position definition", &key.definition)?;
    }
    optional_text(w, "Predicate", &i.predicate)?;
    text(w, "Full definition", &i.definition)
}
fn kind(value: StructureRelationKind) -> &'static str {
    match value {
        StructureRelationKind::Table => "Table",
        StructureRelationKind::PartitionedTable => "Partitioned table",
        StructureRelationKind::View => "View",
        StructureRelationKind::MaterializedView => "Materialized view",
        StructureRelationKind::ForeignTable => "Foreign table",
    }
}
fn identity(value: StructureIdentityKind) -> &'static str {
    match value {
        StructureIdentityKind::None => "None",
        StructureIdentityKind::Always => "GENERATED ALWAYS",
        StructureIdentityKind::ByDefault => "GENERATED BY DEFAULT",
    }
}
fn generated(value: StructureGeneratedKind) -> &'static str {
    match value {
        StructureGeneratedKind::None => "None",
        StructureGeneratedKind::Stored => "Stored",
        StructureGeneratedKind::Virtual => "Virtual",
    }
}
fn enabled(value: StructureTriggerEnabled) -> &'static str {
    match value {
        StructureTriggerEnabled::Origin => "Origin",
        StructureTriggerEnabled::Disabled => "Disabled",
        StructureTriggerEnabled::Replica => "Replica",
        StructureTriggerEnabled::Always => "Always",
    }
}
fn command(value: StructurePolicyCommand) -> &'static str {
    match value {
        StructurePolicyCommand::All => "ALL",
        StructurePolicyCommand::Select => "SELECT",
        StructurePolicyCommand::Insert => "INSERT",
        StructurePolicyCommand::Update => "UPDATE",
        StructurePolicyCommand::Delete => "DELETE",
    }
}
fn action(value: StructureReferentialAction) -> &'static str {
    match value {
        StructureReferentialAction::NoAction => "NO ACTION",
        StructureReferentialAction::Restrict => "RESTRICT",
        StructureReferentialAction::Cascade => "CASCADE",
        StructureReferentialAction::SetNull => "SET NULL",
        StructureReferentialAction::SetDefault => "SET DEFAULT",
    }
}
mod grid;
pub use grid::{ColumnSpec, Shown, Tone};
#[cfg(test)]
mod tests;
