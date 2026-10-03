use super::*;
use std::collections::BTreeMap;

#[derive(Default)]
pub(crate) struct RecordingStore {
    pub(crate) state: Mutex<RecordingState>,
}

#[derive(Default)]
pub(crate) struct RecordingState {
    pub(crate) blobs: BTreeMap<(String, String), String>,
    pub(crate) calls: Vec<(&'static str, String, String)>,
    pub(crate) deny_read: bool,
    pub(crate) deny_write: bool,
}

fn denied() -> keyring::Error {
    keyring::Error::NoStorageAccess(Box::new(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "synthetic private OS detail",
    )))
}

impl BlobStore for RecordingStore {
    fn read(&self, identity: &Identity) -> Result<Option<String>, keyring::Error> {
        let mut state = self.state.lock().unwrap();
        state
            .calls
            .push(("read", identity.service.clone(), identity.account.clone()));
        if state.deny_read {
            return Err(denied());
        }
        Ok(state
            .blobs
            .get(&(identity.service.clone(), identity.account.clone()))
            .cloned())
    }

    fn write(&self, identity: &Identity, value: Option<&str>) -> Result<(), keyring::Error> {
        let mut state = self.state.lock().unwrap();
        state.calls.push((
            if value.is_some() { "write" } else { "delete" },
            identity.service.clone(),
            identity.account.clone(),
        ));
        if state.deny_write {
            return Err(denied());
        }
        let key = (identity.service.clone(), identity.account.clone());
        if let Some(value) = value {
            state.blobs.insert(key, value.into());
        } else {
            state.blobs.remove(&key);
        }
        Ok(())
    }
}

pub(crate) fn store(io: &Arc<RecordingStore>, namespace: &str, policy: ReadPolicy) -> Store {
    Store {
        identity: Identity {
            service: format!("dbunk-native-stage04-{namespace}"),
            account: format!("connection-credentials-{namespace}"),
        },
        policy,
        io: io.clone(),
        cache: Mutex::new(None),
    }
}
