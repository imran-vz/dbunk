use super::*;
pub(super) struct Lease {
    budget: Rc<Cell<usize>>,
    bytes: usize,
}
impl Lease {
    pub(super) fn new(budget: Rc<Cell<usize>>, bytes: usize) -> Option<Self> {
        if bytes > (128 * MIB).saturating_sub(budget.get()) {
            return None;
        }
        budget.set(budget.get() + bytes);
        Some(Self { budget, bytes })
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(self.bytes));
    }
}
pub(super) struct Connections {
    pub(super) rows: Vec<(String, String)>,
    _lease: Lease,
}
impl Connections {
    /// Check borrowed metadata and reserve replacement overlap before cloning.
    pub(super) fn capture<'a>(
        rows: impl Iterator<Item = (&'a str, &'a str)> + Clone,
        budget: Rc<Cell<usize>>,
    ) -> Result<Self, &'static str> {
        let count = rows.clone().take(1025).count();
        if count > 1024
            || rows.clone().any(|(id, name)| {
                id.is_empty()
                    || id.len() > 128
                    || name.len() > 512
                    || id.contains('\0')
                    || name.contains('\0')
            })
        {
            return Err("Connection choices exceed their bounds; previous choices retained");
        }
        let lease = Lease::new(budget, MIB).ok_or(
            "Connection choices need 1 MiB of shared allowance; previous choices retained",
        )?;
        let mut captured = Vec::with_capacity(count);
        captured.extend(rows.map(|(id, name)| (id.to_owned(), name.to_owned())));
        if !valid_connections(&captured) {
            return Err("Connection choices are invalid; previous choices retained");
        }
        Ok(Self {
            rows: captured,
            _lease: lease,
        })
    }
}
pub(super) fn valid_name(value: &str) -> bool {
    !value.is_empty() && value.len() <= 63 && !value.contains('\0')
}
fn valid_connections(rows: &Vec<(String, String)>) -> bool {
    if rows.len() > 1024 || rows.capacity() > 1024 {
        return false;
    }
    let mut bytes = rows
        .capacity()
        .saturating_mul(std::mem::size_of::<(String, String)>());
    for (index, (id, name)) in rows.iter().enumerate() {
        if id.is_empty()
            || id.len() > 128
            || id.capacity() > 256
            || name.len() > 512
            || name.capacity() > 1024
            || id.contains('\0')
            || name.contains('\0')
            || rows[..index].iter().any(|(other, _)| other == id)
        {
            return false;
        }
        bytes = bytes
            .saturating_add(id.capacity())
            .saturating_add(name.capacity());
    }
    bytes <= MIB
}
