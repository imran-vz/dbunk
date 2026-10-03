use super::*;

fn values(values: &Vec<MutationValue>) -> usize {
    values.capacity() * std::mem::size_of::<MutationValue>()
        + values
            .iter()
            .map(|value| value.column.capacity() + value.value.as_ref().map_or(0, String::capacity))
            .sum::<usize>()
}
fn table(table: &MutationTable) -> usize {
    table.schema.capacity() + table.table.capacity()
}
fn operation(operation: &MutationOp) -> usize {
    match operation {
        MutationOp::Insert {
            table: target,
            values: entries,
        } => table(target) + values(entries),
        MutationOp::Update {
            table: target,
            identity,
            guards,
            set,
        } => table(target) + values(identity) + values(guards) + values(set),
        MutationOp::Delete {
            table: target,
            identity,
            guards,
        } => table(target) + values(identity) + values(guards),
    }
}
impl MutationDraft {
    /// Heap retention for intent only; analysis has its own host reservation.
    pub fn retained_bytes(&self) -> usize {
        self.changes.capacity() * std::mem::size_of::<Change>()
            + self
                .changes
                .iter()
                .map(|change| {
                    operation(&change.operation)
                        + change.row.as_ref().map_or(0, |row| {
                            table(&row.table) + values(&row.identity) + values(&row.originals)
                        })
                })
                .sum::<usize>()
    }
    pub fn recovery_bytes(draft: &WorkspaceMutationDraft) -> usize {
        draft.changes.capacity() * std::mem::size_of::<WorkspaceStagedChange>()
            + draft
                .changes
                .iter()
                .map(|change| {
                    change.id.capacity() + values(&change.originals) + operation(&change.operation)
                })
                .sum::<usize>()
    }
}
