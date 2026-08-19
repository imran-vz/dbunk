pub(crate) mod builder;
mod executor;
pub(crate) mod interrupt;
mod manager;
pub(crate) mod postgres;
pub(crate) mod protocol;
mod service;

#[cfg(test)]
mod live;

pub(crate) use manager::TableBrowseManager;

use std::time::Duration;

pub(crate) const MAX_EXECUTORS: usize = 8;
pub(crate) const IDLE_TIMEOUT: Duration = Duration::from_secs(300);
pub(crate) const QUEUE_WAIT: Duration = Duration::from_secs(10);
pub(crate) const CLOSE_TIMEOUT: Duration = Duration::from_secs(3);
