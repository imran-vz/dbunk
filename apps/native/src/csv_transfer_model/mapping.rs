use super::*;

/// Indexed source identity is independent of duplicate or empty header text.
/// Target indices borrow identity from one immutable inspection; generated and
/// identity targets cannot be selected, including through restored UI callbacks.
pub struct Mapping {
    inspection: CsvInspectionId,
    targets: Vec<Option<usize>>,
    _lease: Lease,
}
impl Mapping {
    pub fn new(data: &CsvInspectionData, budget: Rc<Cell<usize>>) -> Result<Self, &'static str> {
        if data.checked_heap_bytes().is_none() || data.direction != CsvDirection::Import {
            return Err("Choose a bounded import inspection before mapping columns");
        }
        let lease = Lease::new(budget, SETUP_BYTES)?;
        let mut targets = vec![None; data.source_columns.len()];
        // Match baseline default name matching, but never reuse a target. The
        // user can always replace this suggestion with an explicit exact name.
        for (index, source) in data.source_columns.iter().enumerate() {
            let selected = data
                .target_columns
                .iter()
                .enumerate()
                .rev()
                .find(|(_, target)| eligible(target) && case_equal(&source.name, &target.name))
                .map(|(index, _)| index);
            if let Some(selected) = selected
                && !targets[..index].contains(&Some(selected))
            {
                targets[index] = Some(selected);
            }
        }
        Ok(Self {
            inspection: data.inspection_id,
            targets,
            _lease: lease,
        })
    }
    pub fn target_index(&self, source_index: usize) -> Option<usize> {
        self.targets.get(source_index).copied().flatten()
    }
    /// Move past generated, identity and already-used targets without trapping
    /// repeated Next/Previous actions on the first unavailable neighbour.
    pub fn adjacent_target(
        &self,
        data: &CsvInspectionData,
        source: usize,
        forward: bool,
    ) -> Result<Option<usize>, &'static str> {
        self.check_identity(data)?;
        if source >= self.targets.len() {
            return Err("CSV source column is unavailable");
        }
        let old = self.target_index(source);
        let mut choices = data
            .target_columns
            .iter()
            .enumerate()
            .filter(|(index, column)| {
                eligible(column)
                    && self
                        .targets
                        .iter()
                        .enumerate()
                        .all(|(other, target)| other == source || *target != Some(*index))
            })
            .map(|(index, _)| index);
        Ok(if forward {
            choices.find(|index| old.is_none_or(|old| *index > old))
        } else {
            choices.rfind(|index| old.is_none_or(|old| *index < old))
        })
    }
    pub fn set(
        &mut self,
        data: &CsvInspectionData,
        source_index: usize,
        target_index: Option<usize>,
    ) -> Result<(), &'static str> {
        self.check_identity(data)?;
        if source_index >= self.targets.len() {
            return Err("CSV source column is unavailable");
        }
        if let Some(index) = target_index {
            let target = data
                .target_columns
                .get(index)
                .ok_or("CSV target column is unavailable")?;
            if !eligible(target) {
                return Err("Generated and identity columns cannot be mapped");
            }
            if self
                .targets
                .iter()
                .enumerate()
                .any(|(source, target)| source != source_index && *target == Some(index))
            {
                return Err("A target column can be mapped only once");
            }
        }
        self.targets[source_index] = target_index;
        Ok(())
    }
    fn check_identity(&self, data: &CsvInspectionData) -> Result<(), &'static str> {
        if data.inspection_id != self.inspection
            || data.direction != CsvDirection::Import
            || data.source_columns.len() != self.targets.len()
        {
            return Err("Column mapping belongs to an older inspection");
        }
        Ok(())
    }
    pub fn first_missing_required<'a>(&self, data: &'a CsvInspectionData) -> Option<&'a str> {
        data.target_columns
            .iter()
            .enumerate()
            .find(|(index, column)| {
                eligible(column)
                    && !column.nullable
                    && !column.has_default
                    && !self.targets.contains(&Some(*index))
            })
            .map(|(_, column)| column.name.as_str())
    }
    pub fn validate(&self, data: &CsvInspectionData) -> Result<(), &'static str> {
        self.check_identity(data)?;
        if !self.targets.iter().any(Option::is_some) {
            return Err("Map at least one CSV source column");
        }
        for (index, target) in self.targets.iter().enumerate() {
            if let Some(target) = target {
                if data
                    .target_columns
                    .get(*target)
                    .is_none_or(|column| !eligible(column))
                {
                    return Err("CSV target column is missing, generated or identity");
                }
                if self.targets[..index].contains(&Some(*target)) {
                    return Err("A target column can be mapped only once");
                }
            }
        }
        if self.first_missing_required(data).is_some() {
            return Err("Map every required target column without a database default");
        }
        Ok(())
    }
    /// At most 1,600 exact target names, each at most 63 bytes. The setup lease
    /// accounts for indices; the caller's reviewed-handle lease/command permit
    /// must account for this newly owned request until it is consumed.
    pub fn to_backend(&self, data: &CsvInspectionData) -> Result<Vec<CsvMapping>, &'static str> {
        self.validate(data)?;
        if data.checked_heap_bytes().is_none() {
            return Err("CSV inspection exceeds its bounds");
        }
        Ok(self
            .targets
            .iter()
            .enumerate()
            .filter_map(|(source_index, target)| {
                target.map(|target| CsvMapping {
                    source_index,
                    target_column: data.target_columns[target].name.clone(),
                })
            })
            .collect())
    }
}
pub fn eligible(column: &CsvTargetColumn) -> bool {
    !column.generated && !column.identity
}

// No lowercase copies of potentially 64 KiB source headers are retained.
fn case_equal(left: &str, right: &str) -> bool {
    left.chars()
        .flat_map(char::to_lowercase)
        .eq(right.chars().flat_map(char::to_lowercase))
}
