//! Drizzle-style table rows for each Structure section. Cells are bounded,
//! single-line display text; the selected-details editor stays the exact,
//! complete rendering of the same row.
use super::*;

/// Display cells keep at most this many characters before an ellipsis.
pub const CELL_CHARS: usize = 160;

/// One column of a section table. `fill` columns share the spare width and
/// never shrink below `width`; the others are exactly `width` pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColumnSpec {
    pub title: &'static str,
    pub width: f32,
    pub fill: bool,
    pub mono: bool,
}
const fn fixed(title: &'static str, width: f32) -> ColumnSpec {
    ColumnSpec {
        title,
        width,
        fill: false,
        mono: false,
    }
}
const fn fill(title: &'static str, width: f32) -> ColumnSpec {
    ColumnSpec {
        title,
        width,
        fill: true,
        mono: false,
    }
}
const fn mono(spec: ColumnSpec) -> ColumnSpec {
    ColumnSpec { mono: true, ..spec }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// Keys and identity: primary key, unique, foreign key.
    Key,
    Plain,
    /// States that need attention: invalid, disabled, not validated.
    Warn,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub text: String,
    pub tone: Tone,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shown {
    Text(String),
    /// Absent, default or otherwise low-signal values.
    Faint(String),
    Tags(Vec<Tag>),
}

impl Section {
    /// Column layout of this section's table.
    pub fn columns(self) -> &'static [ColumnSpec] {
        match self {
            Self::Overview => {
                const COLUMNS: &[ColumnSpec] = &[
                    fixed("Kind", 150.),
                    fixed("Owner", 140.),
                    fixed("Row-level security", 150.),
                    mono(fill("Partitioning", 160.)),
                    fill("Comment", 160.),
                ];
                COLUMNS
            }
            Self::Columns => {
                const COLUMNS: &[ColumnSpec] = &[
                    mono(fixed("#", 40.)),
                    fill("Name", 140.),
                    mono(fill("Type", 120.)),
                    mono(fill("Default", 120.)),
                    fixed("Nullable", 84.),
                    fixed("Keys", 150.),
                    fill("Comment", 100.),
                ];
                COLUMNS
            }
            Self::PrimaryKey => {
                const COLUMNS: &[ColumnSpec] = &[
                    fill("Name", 160.),
                    mono(fill("Columns", 160.)),
                    fixed("Deferrable", 110.),
                ];
                COLUMNS
            }
            Self::ForeignKeys => {
                const COLUMNS: &[ColumnSpec] = &[
                    fill("Name", 160.),
                    mono(fill("Columns", 120.)),
                    mono(fill("References", 180.)),
                    fixed("On update", 100.),
                    fixed("On delete", 100.),
                ];
                COLUMNS
            }
            Self::ReferencedBy => {
                const COLUMNS: &[ColumnSpec] = &[
                    fill("Name", 160.),
                    mono(fill("Referencing table", 180.)),
                    mono(fill("Columns", 120.)),
                    fixed("On update", 100.),
                    fixed("On delete", 100.),
                ];
                COLUMNS
            }
            Self::Indexes => {
                const COLUMNS: &[ColumnSpec] = &[
                    fill("Name", 160.),
                    mono(fill("Columns", 160.)),
                    fixed("Method", 80.),
                    fixed("Attributes", 180.),
                ];
                COLUMNS
            }
            Self::Constraints => {
                const COLUMNS: &[ColumnSpec] = &[
                    fill("Name", 160.),
                    fixed("Kind", 100.),
                    mono(fill("Definition", 220.)),
                    fixed("Attributes", 150.),
                ];
                COLUMNS
            }
            Self::Triggers => {
                const COLUMNS: &[ColumnSpec] = &[
                    fill("Name", 160.),
                    fill("Fires", 160.),
                    fixed("Level", 90.),
                    mono(fill("Function", 160.)),
                    fixed("State", 90.),
                ];
                COLUMNS
            }
            Self::Policies => {
                const COLUMNS: &[ColumnSpec] = &[
                    fill("Name", 160.),
                    fixed("Command", 90.),
                    fixed("Type", 100.),
                    fill("Roles", 120.),
                    mono(fill("Using", 160.)),
                ];
                COLUMNS
            }
            Self::RelationGrants => {
                const COLUMNS: &[ColumnSpec] = &[
                    fill("Grantee", 140.),
                    fill("Privilege", 140.),
                    fill("Grantor", 140.),
                    fixed("Grant option", 110.),
                ];
                COLUMNS
            }
            Self::Rules => {
                const COLUMNS: &[ColumnSpec] = &[
                    fill("Name", 160.),
                    fixed("Event", 90.),
                    fixed("Instead", 80.),
                    fixed("State", 90.),
                    mono(fill("Definition", 220.)),
                ];
                COLUMNS
            }
            Self::Parents | Self::Children => {
                const COLUMNS: &[ColumnSpec] = &[
                    mono(fill("Relation", 200.)),
                    fixed("Relationship", 120.),
                    mono(fill("Bound", 200.)),
                ];
                COLUMNS
            }
        }
    }
}

impl Capture {
    /// One display cell per `section.columns()` entry, or `None` past the end.
    pub fn cells(&self, section: Section, index: usize) -> Option<Vec<Shown>> {
        if index >= self.count(section) {
            return None;
        }
        let d = &self.data;
        Some(match section {
            Section::Overview => vec![
                text(if d.is_partition {
                    format!("{} · partition", kind(d.kind))
                } else {
                    kind(d.kind).to_owned()
                }),
                text(clip(&d.owner)),
                match (d.row_security.enabled, d.row_security.forced) {
                    (false, _) => faint("disabled"),
                    (true, forced) => Shown::Tags(
                        [
                            Some(tag("enabled", Tone::Key)),
                            forced.then(|| tag("forced", Tone::Key)),
                        ]
                        .into_iter()
                        .flatten()
                        .collect(),
                    ),
                },
                match (&d.partition_key, &d.partition_bound) {
                    (Some(key), _) => text(clip(&format!("BY {key}"))),
                    (None, Some(bound)) => text(clip(bound)),
                    (None, None) => faint("—"),
                },
                optional(&d.comment),
            ],
            Section::Columns => {
                let c = &d.columns[index];
                vec![
                    faint(&c.number.to_string()),
                    text(clip(&c.name)),
                    text(clip(&c.data_type)),
                    optional(&c.default_expression),
                    if c.nullable {
                        faint("NULL")
                    } else {
                        text("NOT NULL".into())
                    },
                    Shown::Tags(column_tags(d, c)),
                    optional(&c.comment),
                ]
            }
            Section::PrimaryKey => {
                let p = d.primary_key.as_ref()?;
                vec![
                    text(clip(&p.name)),
                    text(clip(&names(p.columns.iter().map(|c| c.name.as_str())))),
                    yes_no(p.deferrable),
                ]
            }
            Section::ForeignKeys => {
                let f = &d.outbound[index];
                vec![
                    text(clip(&f.name)),
                    text(clip(&names(f.columns.iter().map(|c| c.source.as_str())))),
                    text(clip(&format!(
                        "{}.{} ({})",
                        f.target_schema,
                        f.target_table,
                        names(f.columns.iter().map(|c| c.target.as_str()))
                    ))),
                    referential(f.on_update),
                    referential(f.on_delete),
                ]
            }
            Section::ReferencedBy => {
                let f = &d.inbound[index];
                vec![
                    text(clip(&f.name)),
                    text(clip(&format!(
                        "{}.{} ({})",
                        f.source_schema,
                        f.source_table,
                        names(f.columns.iter().map(|c| c.source.as_str()))
                    ))),
                    text(clip(&names(f.columns.iter().map(|c| c.target.as_str())))),
                    referential(f.on_update),
                    referential(f.on_delete),
                ]
            }
            Section::Indexes => {
                let i = &d.indexes[index];
                let (keys, included): (Vec<_>, Vec<_>) = i.keys.iter().partition(|k| !k.included);
                let mut columns = names(keys.iter().map(|k| k.definition.as_str()));
                if !included.is_empty() {
                    columns = format!(
                        "{columns} INCLUDE ({})",
                        names(included.iter().map(|k| k.definition.as_str()))
                    );
                }
                let tags = [
                    i.primary.then(|| tag("primary", Tone::Key)),
                    (i.unique && !i.primary).then(|| tag("unique", Tone::Key)),
                    i.predicate.is_some().then(|| tag("partial", Tone::Plain)),
                    (!i.valid).then(|| tag("invalid", Tone::Warn)),
                    (i.valid && !i.ready).then(|| tag("not ready", Tone::Warn)),
                ];
                vec![
                    text(clip(&i.name)),
                    text(clip(&columns)),
                    text(clip(&i.method)),
                    Shown::Tags(tags.into_iter().flatten().collect()),
                ]
            }
            Section::Constraints => {
                let c = &d.constraints[index];
                let tags = [
                    (!c.validated).then(|| tag("not validated", Tone::Warn)),
                    c.deferrable.then(|| {
                        tag(
                            if c.initially_deferred {
                                "deferred"
                            } else {
                                "deferrable"
                            },
                            Tone::Plain,
                        )
                    }),
                ];
                vec![
                    text(clip(&c.name)),
                    text(clip(&c.kind)),
                    text(clip(&c.definition)),
                    Shown::Tags(tags.into_iter().flatten().collect()),
                ]
            }
            Section::Triggers => {
                let t = &d.triggers[index];
                vec![
                    text(clip(&t.name)),
                    text(clip(&format!("{} {}", t.timing, t.events.join(" OR ")))),
                    text(clip(&t.level)),
                    text(clip(&format!(
                        "{}.{}()",
                        t.function_schema, t.function_name
                    ))),
                    rule_state(t.enabled),
                ]
            }
            Section::Policies => {
                let p = &d.policies[index];
                vec![
                    text(clip(&p.name)),
                    text(command(p.command).into()),
                    if p.permissive {
                        faint("permissive")
                    } else {
                        text("restrictive".into())
                    },
                    if p.roles.is_empty() {
                        faint("public")
                    } else {
                        text(clip(&names(p.roles.iter().map(String::as_str))))
                    },
                    optional(&p.using_expression),
                ]
            }
            Section::RelationGrants => {
                let p = &d.privileges[index];
                vec![
                    text(clip(&p.grantee)),
                    text(clip(&p.privilege)),
                    text(clip(&p.grantor)),
                    yes_no(p.grantable),
                ]
            }
            Section::Rules => {
                let r = &d.rules[index];
                vec![
                    text(clip(&r.name)),
                    text(clip(&r.event)),
                    yes_no(r.instead),
                    rule_state(r.enabled),
                    text(clip(&r.definition)),
                ]
            }
            Section::Parents | Section::Children => {
                let r = if section == Section::Parents {
                    &d.parents[index]
                } else {
                    &d.partitions[index]
                };
                let partition = (section == Section::Parents && d.is_partition)
                    || (section == Section::Children && r.is_partition);
                vec![
                    text(clip(&format!("{}.{}", r.schema, r.name))),
                    text(
                        if partition {
                            "partition"
                        } else {
                            "inheritance"
                        }
                        .into(),
                    ),
                    optional(&r.bound),
                ]
            }
        })
    }
}

/// PK position, FK membership, single-column uniqueness and identity.
fn column_tags(d: &TableStructureSnapshot, c: &StructureColumn) -> Vec<Tag> {
    let mut tags = Vec::new();
    if let Some(position) = c.primary_key_position {
        let composite = d
            .primary_key
            .as_ref()
            .is_some_and(|key| key.columns.len() > 1);
        tags.push(tag(
            &if composite {
                format!("PK {position}")
            } else {
                "PK".into()
            },
            Tone::Key,
        ));
    }
    if d.outbound
        .iter()
        .any(|f| f.columns.iter().any(|pair| pair.source_number == c.number))
    {
        tags.push(tag("FK", Tone::Key));
    }
    let unique = d.indexes.iter().any(|i| {
        i.unique
            && !i.primary
            && i.predicate.is_none()
            && matches!(i.keys.as_slice(), [key] if key.column_number == Some(c.number))
    });
    if unique {
        tags.push(tag("unique", Tone::Key));
    }
    match c.identity {
        StructureIdentityKind::None => {}
        StructureIdentityKind::Always => tags.push(tag("identity", Tone::Plain)),
        StructureIdentityKind::ByDefault => tags.push(tag("identity by default", Tone::Plain)),
    }
    if c.generated != StructureGeneratedKind::None {
        tags.push(tag("generated", Tone::Plain));
    }
    tags
}

/// One line, at most `CELL_CHARS` characters. Line breaks and tabs collapse
/// to single spaces; other control characters show as U+FFFD.
pub fn clip(value: &str) -> String {
    let mut out = String::with_capacity(value.len().min(CELL_CHARS * 4));
    let mut chars = 0;
    for c in value.chars() {
        if chars == CELL_CHARS {
            out.push('…');
            break;
        }
        if matches!(c, '\n' | '\r' | '\t') {
            if !out.ends_with(' ') {
                out.push(' ');
                chars += 1;
            }
            continue;
        }
        out.push(if c.is_control() { '\u{FFFD}' } else { c });
        chars += 1;
    }
    out
}
fn names<'a>(items: impl Iterator<Item = &'a str>) -> String {
    let mut out = String::new();
    for item in items {
        if !out.is_empty() {
            out.push_str(", ");
        }
        out.push_str(item);
        // Stop early; `clip` bounds the final cell.
        if out.len() > CELL_CHARS * 4 {
            break;
        }
    }
    out
}
fn text(value: String) -> Shown {
    Shown::Text(value)
}
fn faint(value: &str) -> Shown {
    Shown::Faint(value.to_owned())
}
fn optional(value: &Option<String>) -> Shown {
    match value {
        Some(value) if value.is_empty() => faint("''"),
        Some(value) => text(clip(value)),
        None => faint("—"),
    }
}
fn yes_no(value: bool) -> Shown {
    if value {
        text("yes".into())
    } else {
        faint("no")
    }
}
fn tag(text: &str, tone: Tone) -> Tag {
    Tag {
        text: text.to_owned(),
        tone,
    }
}
fn referential(value: StructureReferentialAction) -> Shown {
    let label = action(value).to_owned();
    if value == StructureReferentialAction::NoAction {
        Shown::Faint(label)
    } else {
        Shown::Text(label)
    }
}
fn rule_state(value: StructureTriggerEnabled) -> Shown {
    match value {
        StructureTriggerEnabled::Origin => faint("enabled"),
        StructureTriggerEnabled::Disabled => Shown::Tags(vec![tag("disabled", Tone::Warn)]),
        StructureTriggerEnabled::Replica => text("replica".into()),
        StructureTriggerEnabled::Always => text("always".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{capture, snapshot};
    use super::*;

    #[test]
    fn every_row_has_one_cell_per_column_in_each_section() {
        let mut data = snapshot();
        data.columns[0].nullable = false;
        data.primary_key = Some(StructurePrimaryKey {
            oid: 1,
            name: "pk".into(),
            columns: vec![StructureKeyColumn {
                number: 1,
                name: "a".into(),
            }],
            deferrable: false,
            initially_deferred: false,
        });
        data.columns[0].primary_key_position = Some(1);
        let capture = capture(data);
        for section in Section::ALL {
            for index in 0..capture.count(section) {
                let cells = capture.cells(section, index).unwrap();
                assert_eq!(cells.len(), section.columns().len(), "{section:?}");
            }
            assert_eq!(capture.cells(section, capture.count(section)), None);
        }
    }

    #[test]
    fn column_keys_mark_primary_foreign_and_single_column_unique() {
        let mut data = snapshot();
        data.columns[0].primary_key_position = Some(2);
        data.columns[1].primary_key_position = Some(1);
        data.primary_key = Some(StructurePrimaryKey {
            oid: 1,
            name: "pk".into(),
            columns: vec![
                StructureKeyColumn {
                    number: 3,
                    name: "b".into(),
                },
                StructureKeyColumn {
                    number: 1,
                    name: "a".into(),
                },
            ],
            deferrable: false,
            initially_deferred: false,
        });
        data.outbound.push(StructureForeignKey {
            oid: 2,
            name: "fk".into(),
            source_oid: 22,
            source_schema: "Quoted.Schema".into(),
            source_table: "t\"able".into(),
            target_oid: 33,
            target_schema: "s".into(),
            target_table: "u".into(),
            columns: vec![StructureKeyPair {
                source_number: 3,
                source: "b".into(),
                target_number: 1,
                target: "id".into(),
            }],
            on_update: StructureReferentialAction::NoAction,
            on_delete: StructureReferentialAction::Cascade,
            match_type: "SIMPLE".into(),
            deferrable: false,
            initially_deferred: false,
            validated: true,
        });
        let unique = |oid, number, name: &str, predicate: Option<&str>| StructureIndex {
            oid,
            name: "u".into(),
            method: "btree".into(),
            unique: true,
            primary: false,
            valid: true,
            ready: true,
            keys: vec![StructureIndexKey {
                position: 1,
                column_number: Some(number),
                column_name: Some(name.into()),
                definition: "x".into(),
                included: false,
            }],
            predicate: predicate.map(Into::into),
            definition: "CREATE UNIQUE INDEX".into(),
            constraint_oid: None,
        };
        // A partial unique index does not make the column unique.
        data.indexes
            .push(unique(3, 1, "a", Some("deleted_at IS NULL")));
        data.indexes.push(unique(4, 3, "b", None));
        let capture = capture(data);
        let keys = |index| match &capture.cells(Section::Columns, index).unwrap()[5] {
            Shown::Tags(tags) => tags.iter().map(|t| t.text.clone()).collect::<Vec<_>>(),
            other => panic!("{other:?}"),
        };
        assert_eq!(keys(0), ["PK 2"]);
        assert_eq!(keys(1), ["PK 1", "FK", "unique"]);
        let fk = capture.cells(Section::ForeignKeys, 0).unwrap();
        assert_eq!(fk[2], Shown::Text("s.u (id)".into()));
        assert_eq!(fk[3], Shown::Faint("NO ACTION".into()));
        assert_eq!(fk[4], Shown::Text("CASCADE".into()));
    }

    #[test]
    fn cells_stay_on_one_bounded_line() {
        assert_eq!(clip("a\r\n\tb"), "a b");
        assert_eq!(clip("x\u{7}y"), "x\u{FFFD}y");
        let long = clip(&"é".repeat(CELL_CHARS * 3));
        assert_eq!(long.chars().count(), CELL_CHARS + 1);
        assert!(long.ends_with('…'));
        assert_eq!(clip(&"z".repeat(CELL_CHARS)), "z".repeat(CELL_CHARS));
    }
}
