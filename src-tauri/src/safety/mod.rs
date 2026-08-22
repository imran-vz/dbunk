pub(crate) mod policy;

#[cfg(test)]
mod live;

pub(crate) const READ_ONLY_TAG: &str = "[policy:read-only]";
pub(crate) const CONFIRM_TAG: &str = "[policy:confirm]";
