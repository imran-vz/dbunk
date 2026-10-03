//! A reference uses every mapped source value from one exact retained page.
//! No SQL interpolation, staged values, or guessed partial composite keys.
use crate::results::encoded_size;
use dbunk_lib::backend::{data::*, objects::ForeignKey};
use serde::Serialize;
use std::{cell::Cell, rc::Rc};

const WORKSPACE_BYTES: usize = 128 * 1024 * 1024;
const RETAINED_BYTES: usize = 8 * 1024 * 1024;
const CHOICES_BYTES: usize = 2 * 1024 * 1024;
const FILTER_BYTES: usize = 60 * 1024;
#[derive(Clone)]
pub struct Selection {
    page: Rc<BrowseTableResult>,
    pub connection: String,
    pub row: usize,
    column: usize,
}
impl Selection {
    pub fn new(
        page: Rc<BrowseTableResult>,
        connection: String,
        row: usize,
        column: usize,
    ) -> Result<Self, &'static str> {
        if connection.is_empty() || connection.len() > 256 || connection.contains('\0') {
            return Err("The table connection is unavailable");
        }
        if page.omitted_rows > 0 || page.truncated_cells > 0 {
            return Err("Foreign-key navigation requires a complete, untruncated loaded page");
        }
        if column >= page.columns.len()
            || page
                .rows
                .get(row)
                .is_none_or(|values| values.len() != page.columns.len())
        {
            return Err("Select a loaded source cell");
        }
        Ok(Self {
            page,
            connection,
            row,
            column,
        })
    }
    pub fn matches(
        &self,
        page: &Rc<BrowseTableResult>,
        connection: Option<&str>,
        cell: Option<(usize, usize)>,
    ) -> bool {
        Rc::ptr_eq(&self.page, page)
            && connection == Some(self.connection.as_str())
            && cell == Some((self.row, self.column))
    }
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Target {
    pub schema: String,
    pub table: String,
    pub filters: Vec<BrowseFilter>,
}
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub enum Refusal {
    InvalidMapping,
    MissingSource,
    NoReferencedRow,
    ValuesTooLarge,
}
impl Refusal {
    pub fn message(self) -> &'static str {
        match self {
            Self::InvalidMapping => "The foreign-key mapping is incomplete or ambiguous",
            Self::MissingSource => {
                "A source column is missing or appears more than once in the loaded page"
            }
            Self::NoReferencedRow => {
                "A foreign-key component is NULL; there is no referenced row to open"
            }
            Self::ValuesTooLarge => "Reference filters exceed 60 KiB; no navigation was created",
        }
    }
}
#[derive(Serialize)]
pub struct Choice {
    pub key: ForeignKey,
    pub target: Result<Target, Refusal>,
}
pub struct Navigation {
    pub selection: Selection,
    pub choices: Vec<Choice>,
    pub selected: usize,
    budget: Rc<Cell<usize>>,
}
impl Navigation {
    pub fn new(
        selection: Selection,
        keys: Vec<ForeignKey>,
        budget: Rc<Cell<usize>>,
    ) -> Result<Self, &'static str> {
        if keys.len() > 256 || encoded_size(&keys) > 1024 * 1024 {
            return Err("Foreign-key metadata exceeds 256 constraints or 1 MiB");
        }
        if budget.get() > WORKSPACE_BYTES - RETAINED_BYTES {
            return Err(
                "Foreign-key review needs 8 MiB of workspace allowance; clear another result",
            );
        }
        budget.set(budget.get() + RETAINED_BYTES);
        let mut review = Self {
            selection,
            choices: vec![],
            selected: 0,
            budget,
        };
        let source = &review.selection.page.columns[review.selection.column].name;
        let mut bytes = 0usize;
        for key in keys.into_iter().filter(|key| key.columns.contains(source)) {
            let target = target(&review.selection.page, review.selection.row, &key);
            let choice = Choice { key, target };
            bytes = bytes.saturating_add(encoded_size(&choice));
            if bytes > CHOICES_BYTES {
                return Err("Foreign-key choices exceed the 2 MiB review limit");
            }
            review.choices.push(choice);
        }
        if review.choices.is_empty() {
            return Err("No foreign key uses the selected source column");
        }
        Ok(review)
    }
    pub fn choice(&self) -> &Choice {
        &self.choices[self.selected]
    }
    pub fn next(&mut self) {
        self.selected = (self.selected + 1) % self.choices.len();
    }
}
impl Drop for Navigation {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(RETAINED_BYTES));
    }
}
fn target(page: &BrowseTableResult, row: usize, key: &ForeignKey) -> Result<Target, Refusal> {
    let valid_name = |name: &str| !name.is_empty() && name.len() <= 256 && !name.contains('\0');
    if !valid_name(&key.name)
        || !valid_name(&key.referenced_schema)
        || !valid_name(&key.referenced_table)
        || key.columns.is_empty()
        || key.columns.len() > 64
        || key.columns.len() != key.referenced_columns.len()
        || key
            .columns
            .iter()
            .chain(&key.referenced_columns)
            .any(|name| !valid_name(name))
        || key
            .columns
            .iter()
            .enumerate()
            .any(|(i, name)| key.columns[..i].contains(name))
        || key
            .referenced_columns
            .iter()
            .enumerate()
            .any(|(i, name)| key.referenced_columns[..i].contains(name))
    {
        return Err(Refusal::InvalidMapping);
    }
    let row = page.rows.get(row).ok_or(Refusal::MissingSource)?;
    let values = key
        .columns
        .iter()
        .map(|name| {
            let mut matches = page
                .columns
                .iter()
                .enumerate()
                .filter(|(_, column)| &column.name == name);
            let index = matches.next().ok_or(Refusal::MissingSource)?.0;
            if matches.next().is_some() {
                return Err(Refusal::MissingSource);
            }
            row.get(index)
                .ok_or(Refusal::MissingSource)
                .map(Option::as_deref)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if values.iter().any(Option::is_none) {
        return Err(Refusal::NoReferencedRow);
    }
    if encoded_size(&values) > FILTER_BYTES {
        return Err(Refusal::ValuesTooLarge);
    }
    let filters = key
        .referenced_columns
        .iter()
        .zip(values)
        .map(|(column, value)| BrowseFilter::Comparison {
            column: column.clone(),
            operator: ComparisonOperator::Eq,
            value: value.unwrap().to_owned(),
        })
        .collect::<Vec<_>>();
    if encoded_size(&filters) > FILTER_BYTES {
        return Err(Refusal::ValuesTooLarge);
    }
    Ok(Target {
        schema: key.referenced_schema.clone(),
        table: key.referenced_table.clone(),
        filters,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn page() -> Rc<BrowseTableResult> {
        Rc::new(BrowseTableResult {
            request_id: 1,
            columns: ["second", "first"]
                .into_iter()
                .map(|name| BrowseColumn {
                    name: name.into(),
                    cast_type: "text".into(),
                    nullable: true,
                })
                .collect(),
            rows: vec![vec![Some("O'Reilly\\雪".into()), Some("".into())]],
            identity: BrowseIdentity {
                kind: BrowseIdentityKind::None,
                columns: vec![],
            },
            row_identity: None,
            page_info: BrowsePageInfo {
                mode: BrowsePageMode::Offset,
                page: Some(1),
                has_more: false,
                next_cursor: None,
            },
            count: BrowseCount {
                kind: BrowseCountKind::Unknown,
                value: None,
            },
            inspection: BrowseInspection {
                sql: "SELECT ...".into(),
                params: vec![],
            },
            omitted_rows: 0,
            truncated_cells: 0,
            runtime_ms: 0,
        })
    }
    fn key() -> ForeignKey {
        ForeignKey {
            name: "key".into(),
            columns: vec!["first".into(), "second".into()],
            referenced_schema: "odd\"schema".into(),
            referenced_table: "target.table".into(),
            referenced_columns: vec!["target first".into(), "target\"second".into()],
            on_update: None,
            on_delete: None,
        }
    }
    #[test]
    fn composite_mapping_preserves_order_empty_text_and_bound_escaping() {
        let target = target(&page(), 0, &key()).unwrap();
        assert_eq!(target.schema, "odd\"schema");
        assert_eq!(target.table, "target.table");
        assert_eq!(
            target.filters,
            vec![
                BrowseFilter::Comparison {
                    column: "target first".into(),
                    operator: ComparisonOperator::Eq,
                    value: "".into()
                },
                BrowseFilter::Comparison {
                    column: "target\"second".into(),
                    operator: ComparisonOperator::Eq,
                    value: "O'Reilly\\雪".into()
                },
            ]
        );
    }
    #[test]
    fn null_missing_duplicate_partial_and_large_values_refuse() {
        let mut page = page();
        Rc::make_mut(&mut page).rows[0][1] = None;
        assert_eq!(target(&page, 0, &key()), Err(Refusal::NoReferencedRow));
        Rc::make_mut(&mut page).columns[1].name = "missing".into();
        assert_eq!(target(&page, 0, &key()), Err(Refusal::MissingSource));
        Rc::make_mut(&mut page).columns[1].name = "second".into();
        assert_eq!(target(&page, 0, &key()), Err(Refusal::MissingSource));
        let mut page = self::page();
        Rc::make_mut(&mut page).truncated_cells = 1;
        assert!(Selection::new(page, "connection".into(), 0, 0).is_err());
        let mut page = self::page();
        Rc::make_mut(&mut page).omitted_rows = 1;
        assert!(Selection::new(page, "connection".into(), 0, 0).is_err());
        let mut page = self::page();
        Rc::make_mut(&mut page).rows[0][0] = Some("\n".repeat(FILTER_BYTES));
        assert_eq!(target(&page, 0, &key()), Err(Refusal::ValuesTooLarge));
        let mut invalid = key();
        invalid.referenced_columns.pop();
        assert_eq!(
            target(&self::page(), 0, &invalid),
            Err(Refusal::InvalidMapping)
        );
    }
    #[test]
    fn request_identity_choices_and_budget_release_are_exact() {
        let page = page();
        let selection = Selection::new(page.clone(), "connection".into(), 0, 0).unwrap();
        assert!(selection.matches(&page, Some("connection"), Some((0, 0))));
        assert!(!selection.matches(&Rc::new((*page).clone()), Some("connection"), Some((0, 0))));
        assert!(!selection.matches(&page, Some("other"), Some((0, 0))));
        assert!(!selection.matches(&page, Some("connection"), Some((0, 1))));
        let budget = Rc::new(Cell::new(17));
        let mut second = key();
        second.name = "second choice".into();
        let mut review =
            Navigation::new(selection.clone(), vec![key(), second], budget.clone()).unwrap();
        assert_eq!(review.choices.len(), 2);
        review.next();
        assert_eq!(review.choice().key.name, "second choice");
        assert_eq!(budget.get(), 17 + RETAINED_BYTES);
        drop(review);
        assert_eq!(budget.get(), 17);
        assert!(Navigation::new(selection.clone(), vec![], budget.clone()).is_err());
        assert_eq!(budget.get(), 17);
        budget.set(WORKSPACE_BYTES);
        assert!(Navigation::new(selection, vec![key()], budget.clone()).is_err());
        assert_eq!(budget.get(), WORKSPACE_BYTES);
    }
}
