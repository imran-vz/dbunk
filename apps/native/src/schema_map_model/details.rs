use super::*;
#[cfg(test)]
struct Detail {
    bytes: usize,
    lines: usize,
    text: Option<String>,
}
#[cfg(test)]
impl Write for Detail {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.bytes = self
            .bytes
            .checked_add(text.len())
            .filter(|n| *n <= MAX_DETAIL_BYTES)
            .ok_or(fmt::Error)?;
        self.lines = self
            .lines
            .checked_add(text.bytes().filter(|b| matches!(*b, b'\r' | b'\n')).count())
            .filter(|n| *n < MAX_DETAIL_LINES)
            .ok_or(fmt::Error)?;
        if let Some(output) = &mut self.text {
            output.push_str(text);
        }
        Ok(())
    }
}
pub(super) fn identifier(w: &mut impl Write, value: &str) -> fmt::Result {
    w.write_char('"')?;
    for ch in value.chars() {
        if ch == '"' {
            w.write_char('"')?;
        }
        w.write_char(ch)?;
    }
    w.write_char('"')
}
fn qualified(w: &mut impl Write, t: &SchemaMapTable) -> fmt::Result {
    identifier(w, &t.schema)?;
    w.write_char('.')?;
    identifier(w, &t.name)
}
fn action(a: SchemaMapAction) -> &'static str {
    match a {
        SchemaMapAction::NoAction => "NO ACTION",
        SchemaMapAction::Restrict => "RESTRICT",
        SchemaMapAction::Cascade => "CASCADE",
        SchemaMapAction::SetNull => "SET NULL",
        SchemaMapAction::SetDefault => "SET DEFAULT",
    }
}
impl Scene {
    /// At most 32 KiB / 128 logical lines; no truncated identity or composite
    /// mapping is returned. Keep one selected read-only editor without wrapping.
    /// Larger node detail is explicitly refused; full typed metadata remains in
    /// snapshot() and the dedicated Structure inspector can show its sections.
    #[cfg(test)]
    pub fn details(&self, selection: Selection) -> Result<String, &'static str> {
        if !self.accepts(selection) {
            return Err("Map selection belongs to an older capture or layout");
        }
        let mut measured = Detail {
            bytes: 0,
            lines: 0,
            text: None,
        };
        self.write_details(selection, &mut measured)
            .map_err(|_| "Map details exceed 32 KiB or 128 lines; use the Structure inspector")?;
        let mut output = Detail {
            bytes: 0,
            lines: 0,
            text: Some(String::with_capacity(measured.bytes)),
        };
        self.write_details(selection, &mut output)
            .map_err(|_| "Map detail formatting failed")?;
        Ok(output.text.unwrap())
    }
    pub(super) fn write_details(&self, selection: Selection, w: &mut impl Write) -> fmt::Result {
        writeln!(
            w,
            "Database: {:?}\nDatabase OID: {}\nCaptured: {}",
            self.snapshot.database, self.snapshot.database_oid, self.snapshot.captured_at
        )?;
        match selection {
            Selection::Node { identity, .. } => {
                let t = self
                    .snapshot
                    .tables
                    .iter()
                    .find(|t| t.identity == identity)
                    .ok_or(fmt::Error)?;
                w.write_str("Table: ")?;
                qualified(w, t)?;
                writeln!(
                    w,
                    "\nRelation OID: {}\nSchema OID: {}\nKind: {:?}\nExternal target: {}\nJunction: {}\nColumns: {}\nUser triggers: {}",
                    identity.relation_oid,
                    t.schema_oid,
                    t.kind,
                    t.external,
                    t.junction,
                    t.columns.len(),
                    t.triggers.len()
                )?;
                for c in &t.columns {
                    write!(w, "{}: ", c.attnum)?;
                    identifier(w, &c.name)?;
                    write!(
                        w,
                        " {} | {}{}",
                        c.data_type,
                        if c.nullable { "NULL" } else { "NOT NULL" },
                        if c.primary_key { " | PRIMARY KEY" } else { "" }
                    )?;
                    if let Some(comment) = &c.comment {
                        write!(w, " | comment: {comment}")?;
                    }
                    w.write_char('\n')?;
                }
                for trigger in &t.triggers {
                    write!(w, "Trigger OID {} ", trigger.oid)?;
                    identifier(w, &trigger.name)?;
                    write!(
                        w,
                        " | {} {} {:?} | {:?} | function ",
                        trigger.timing, trigger.orientation, trigger.events, trigger.enabled
                    )?;
                    identifier(w, &trigger.function_schema)?;
                    w.write_char('.')?;
                    identifier(w, &trigger.function_name)?;
                    writeln!(
                        w,
                        " (OID {}) | UPDATE OF attnums {:?}",
                        trigger.function_oid, trigger.columns
                    )?;
                }
            }
            Selection::Edge { identity, .. } => {
                let fk = self
                    .snapshot
                    .foreign_keys
                    .iter()
                    .find(|f| {
                        f.constraint_oid == identity.constraint_oid
                            && f.database_oid == identity.database_oid
                    })
                    .ok_or(fmt::Error)?;
                let source = self
                    .snapshot
                    .tables
                    .iter()
                    .find(|t| t.identity == fk.source)
                    .ok_or(fmt::Error)?;
                let target = self
                    .snapshot
                    .tables
                    .iter()
                    .find(|t| t.identity == fk.target)
                    .ok_or(fmt::Error)?;
                w.write_str("Foreign key: ")?;
                identifier(w, &fk.name)?;
                writeln!(w, "\nConstraint OID: {}", fk.constraint_oid)?;
                w.write_str("Source: ")?;
                qualified(w, source)?;
                writeln!(w, " (OID {})", source.identity.relation_oid)?;
                w.write_str("Target: ")?;
                qualified(w, target)?;
                writeln!(w, " (OID {})", target.identity.relation_oid)?;
                for (i, pair) in fk.columns.iter().enumerate() {
                    let a = source
                        .columns
                        .iter()
                        .find(|c| c.attnum == pair.source)
                        .ok_or(fmt::Error)?;
                    let b = target
                        .columns
                        .iter()
                        .find(|c| c.attnum == pair.target)
                        .ok_or(fmt::Error)?;
                    write!(w, "Pair {}: ", i + 1)?;
                    identifier(w, &a.name)?;
                    write!(w, " (attnum {}) → ", a.attnum)?;
                    identifier(w, &b.name)?;
                    writeln!(w, " (attnum {})", b.attnum)?;
                }
                writeln!(
                    w,
                    "Cardinality: {:?}\nReason: {}\nReferencing columns unique: {}\nReferencing columns nullable: {}\nON UPDATE: {}\nON DELETE: {}\nMATCH: {}\nValidated: {}\nDeferrable: {}\nJunction participant: {}",
                    fk.cardinality,
                    fk.cardinality_reason,
                    fk.columns_unique,
                    fk.columns_nullable,
                    action(fk.on_update),
                    action(fk.on_delete),
                    fk.match_type,
                    fk.validated,
                    fk.deferrable,
                    fk.junction_participant
                )?;
                w.write_str("Cardinality describes catalog constraints, not measured row counts. Composite pairs retain declared order.\n")?;
            }
        }
        Ok(())
    }
}
