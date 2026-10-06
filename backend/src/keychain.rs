//! Connection-credential storage backed by the OS keychain.
//!
//! ## Shape (ADR-0005, retained as a backend by ADR-0007)
//!
//! All connection passwords live in **one** keychain entry — service `"dbunk"`,
//! account `"connection-credentials"` — whose value is a serialized JSON map
//! `{ connectionId: password }`. macOS prompts the user to unlock the keychain
//! per-entry, so consolidating into one entry collapses N prompts per session
//! down to 1. Each store owns its decoded cache. The Tauri store remains a
//! process-wide `OnceLock`; isolated stores must own a separate instance.
//!
//! ## Failure policy
//!
//! The legacy Tauri store logs read failures and treats them as an empty map,
//! preserving ADR-0005. Strict stores distinguish a missing entry from denial,
//! corruption and other OS errors, and never cache a failed read. Write/delete
//! failures leave the previous cache intact in both policies.
//!
//! ## Public surface
//!
//! Each credential context holds its store explicitly. No I/O method selects
//! a default identity or falls back to another store.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

const SERVICE: &str = "dbunk";
const ACCOUNT: &str = "connection-credentials";

type Credentials = HashMap<String, String>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadPolicy {
    LegacyEmptyOnError,
    Strict,
}

/// No secret or raw OS error text is carried into the strict caller's error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StoreError {
    AccessDenied,
    Unavailable,
    Corrupt,
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::AccessDenied => {
                "Credential Keychain access was denied or locked; unlock it and retry"
            }
            Self::Unavailable => "Credential Keychain is unavailable; retry the operation",
            Self::Corrupt => "Credential Keychain data is unreadable; the entry has been preserved",
        })
    }
}

struct Identity {
    service: String,
    account: String,
}

/// Injectable only inside this module. Every operation receives the same owned
/// identity, including deletion; implementations cannot select a fallback entry.
trait BlobStore: Send + Sync {
    fn read(&self, identity: &Identity) -> Result<Option<String>, keyring::Error>;
    fn write(&self, identity: &Identity, value: Option<&str>) -> Result<(), keyring::Error>;
}

struct OsStore;

impl BlobStore for OsStore {
    fn read(&self, identity: &Identity) -> Result<Option<String>, keyring::Error> {
        let entry = keyring::Entry::new(&identity.service, &identity.account)?;
        match entry.get_password() {
            Ok(blob) => Ok(Some(blob)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn write(&self, identity: &Identity, value: Option<&str>) -> Result<(), keyring::Error> {
        let entry = keyring::Entry::new(&identity.service, &identity.account)?;
        if let Some(blob) = value {
            entry.set_password(blob)
        } else {
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(error) => Err(error),
            }
        }
    }
}

pub(crate) struct Store {
    identity: Identity,
    policy: ReadPolicy,
    io: Arc<dyn BlobStore>,
    cache: Mutex<Option<Credentials>>,
}

pub(crate) fn legacy() -> Arc<Store> {
    static STORE: OnceLock<Arc<Store>> = OnceLock::new();
    STORE
        .get_or_init(|| {
            Arc::new(Store {
                identity: Identity {
                    service: SERVICE.into(),
                    account: ACCOUNT.into(),
                },
                policy: ReadPolicy::LegacyEmptyOnError,
                io: Arc::new(OsStore),
                cache: Mutex::new(None),
            })
        })
        .clone()
}

// ---------------------------------------------------------------------------
// OS keychain I/O
// ---------------------------------------------------------------------------

impl Store {
    /// A separate rollback entry exists only in the validated native namespace.
    /// It shares the injected I/O adapter, never the primary entry's cache.
    #[cfg(feature = "isolated-profile")]
    fn connection_backup_identity(&self) -> Result<Identity, String> {
        if self.policy != ReadPolicy::Strict
            || !self.identity.service.starts_with("dbunk-native-stage04-")
            || !self.identity.account.starts_with("connection-credentials-")
        {
            return Err("Connection rollback requires an isolated credential identity".into());
        }
        Ok(Identity {
            service: self.identity.service.clone(),
            account: format!("{}-connection-rollback-v1", self.identity.account),
        })
    }

    #[cfg(feature = "isolated-profile")]
    pub(crate) fn write_connection_backup(&self, previous: &Credentials) -> Result<(), String> {
        let identity = self.connection_backup_identity()?;
        // Persist even an empty map: missing backup must never mean empty store.
        let blob = serde_json::to_string(previous).map_err(|_| StoreError::Corrupt.to_string())?;
        self.io
            .write(&identity, Some(&blob))
            .map_err(|_| StoreError::Unavailable.to_string())
    }

    #[cfg(feature = "isolated-profile")]
    pub(crate) fn read_connection_backup(&self) -> Result<Credentials, String> {
        let identity = self.connection_backup_identity()?;
        let blob = self
            .io
            .read(&identity)
            .map_err(|_| StoreError::Unavailable.to_string())?
            .ok_or_else(|| {
                "Connection credential rollback entry is missing; profile preserved".to_string()
            })?;
        serde_json::from_str(&blob).map_err(|_| StoreError::Corrupt.to_string())
    }

    #[cfg(feature = "isolated-profile")]
    pub(crate) fn clear_connection_backup(&self) -> Result<(), String> {
        let identity = self.connection_backup_identity()?;
        self.io
            .write(&identity, None)
            .map_err(|_| StoreError::Unavailable.to_string())
    }

    fn read_blob(&self) -> Result<Credentials, String> {
        let result = match self.io.read(&self.identity) {
            Ok(Some(blob)) => serde_json::from_str(&blob).map_err(|_| StoreError::Corrupt),
            Ok(None) => Ok(Credentials::new()),
            Err(keyring::Error::NoStorageAccess(_)) => Err(StoreError::AccessDenied),
            Err(_) => Err(StoreError::Unavailable),
        };
        match result {
            Ok(map) => Ok(map),
            Err(error) if self.policy == ReadPolicy::Strict => Err(error.to_string()),
            Err(error) => {
                eprintln!("Keychain read failed, treating as empty: {error}");
                Ok(Credentials::new())
            }
        }
    }

    pub(crate) fn get_all(&self) -> Result<Credentials, String> {
        let mut cache = self.cache.lock().expect("password cache poisoned");
        if let Some(map) = cache.as_ref() {
            return Ok(map.clone());
        }
        let map = self.read_blob()?;
        *cache = Some(map.clone());
        Ok(map)
    }

    pub(crate) fn replace_all(&self, next: &Credentials) -> Result<(), String> {
        let mut cache = self.cache.lock().expect("password cache poisoned");
        let blob = if next.is_empty() {
            None
        } else {
            Some(serde_json::to_string(next).map_err(|_| StoreError::Corrupt.to_string())?)
        };
        self.io
            .write(&self.identity, blob.as_deref())
            .map_err(|error| {
                if self.policy == ReadPolicy::LegacyEmptyOnError {
                    error.to_string()
                } else if matches!(error, keyring::Error::NoStorageAccess(_)) {
                    StoreError::AccessDenied.to_string()
                } else {
                    StoreError::Unavailable.to_string()
                }
            })?;
        *cache = Some(next.clone());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Async adapters
// ---------------------------------------------------------------------------

const INTERRUPTED: &str = "Credential Keychain operation did not complete; retry the operation";

/// Keychain I/O can block on an OS authorization prompt or on IPC with the
/// security daemon. Async callers run it on Tokio's blocking pool so runtime
/// workers (and anything waiting behind a held lifecycle gate) stay responsive.
/// The synchronous methods remain for blocking contexts and tests.
async fn off_thread<T: Send + 'static>(
    store: &Arc<Store>,
    operation: impl FnOnce(&Store) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let store = Arc::clone(store);
    tokio::task::spawn_blocking(move || operation(&store))
        .await
        .map_err(|_| INTERRUPTED.to_string())?
}

pub(crate) async fn load(store: &Arc<Store>) -> Result<Credentials, String> {
    off_thread(store, |store| store.get_all()).await
}

pub(crate) async fn replace(store: &Arc<Store>, next: Credentials) -> Result<(), String> {
    off_thread(store, move |store| store.replace_all(&next)).await
}

#[cfg(feature = "isolated-profile")]
pub(crate) async fn write_backup(store: &Arc<Store>, previous: Credentials) -> Result<(), String> {
    off_thread(store, move |store| store.write_connection_backup(&previous)).await
}

#[cfg(feature = "isolated-profile")]
pub(crate) async fn read_backup(store: &Arc<Store>) -> Result<Credentials, String> {
    off_thread(store, |store| store.read_connection_backup()).await
}

#[cfg(feature = "isolated-profile")]
pub(crate) async fn clear_backup(store: &Arc<Store>) -> Result<(), String> {
    off_thread(store, |store| store.clear_connection_backup()).await
}

/// Stage 03 has no Keychain capability, even if a later caller accidentally
/// requests one. It must never construct an OS entry.
#[cfg(any(test, feature = "isolated-profile"))]
pub(crate) fn disabled() -> Arc<Store> {
    struct Disabled;
    impl BlobStore for Disabled {
        fn read(&self, _: &Identity) -> Result<Option<String>, keyring::Error> {
            Err(keyring::Error::NoStorageAccess(Box::new(
                std::io::Error::other("Keychain disabled"),
            )))
        }
        fn write(&self, _: &Identity, _: Option<&str>) -> Result<(), keyring::Error> {
            Err(keyring::Error::NoStorageAccess(Box::new(
                std::io::Error::other("Keychain disabled"),
            )))
        }
    }
    Arc::new(Store {
        identity: Identity {
            service: "disabled".into(),
            account: "disabled".into(),
        },
        policy: ReadPolicy::Strict,
        io: Arc::new(Disabled),
        cache: Mutex::new(None),
    })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) mod testing;

/// UUID-isolated native namespace derived only from a validated native marker.
/// The stage04 prefix is retained for both explicit workspace profile kinds;
/// it never selects the legacy production service.
/// Construction is lazy: opening a SQLite profile performs no OS operation.
#[cfg(feature = "isolated-profile")]
pub(crate) fn isolated(namespace: uuid::Uuid) -> Arc<Store> {
    Arc::new(Store {
        identity: Identity {
            service: format!("dbunk-native-stage04-{namespace}"),
            account: format!("connection-credentials-{namespace}"),
        },
        policy: ReadPolicy::Strict,
        io: Arc::new(OsStore),
        cache: Mutex::new(None),
    })
}
