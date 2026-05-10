//! Connection-credential storage backed by the OS keychain.
//!
//! ## Shape (ADR-0005)
//!
//! All connection passwords live in **one** keychain entry — service `"dbunk"`,
//! account `"connection-credentials"` — whose value is a serialized JSON map
//! `{ connectionId: password }`. macOS prompts the user to unlock the keychain
//! per-entry, so consolidating into one entry collapses N prompts per session
//! down to 1. The decoded map is held in process memory behind a `OnceLock`,
//! so subsequent lookups don't hit the OS at all.
//!
//! ## Failure policy
//!
//! Read failures (corrupt blob, OS-level errors, no entry yet) are logged and
//! treated as an empty map. The caller's connection will then surface an
//! authentication error on next use, and the user can re-enter credentials.
//! Write failures bubble up so saves can't silently lose data.
//!
//! ## Public surface
//!
//! - [`get`] — look up a single password.
//! - [`upsert_many`] — apply a batch of inserts/clears, write the blob once.
//! - [`delete`] — drop a single id and write the blob.

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

const SERVICE: &str = "dbunk";
const ACCOUNT: &str = "connection-credentials";

static CACHE: OnceLock<Mutex<Option<HashMap<String, String>>>> = OnceLock::new();

fn cache() -> &'static Mutex<Option<HashMap<String, String>>> {
    CACHE.get_or_init(|| Mutex::new(None))
}

// ---------------------------------------------------------------------------
// OS keychain I/O
// ---------------------------------------------------------------------------

fn read_blob() -> HashMap<String, String> {
    let entry = match keyring::Entry::new(SERVICE, ACCOUNT) {
        Ok(entry) => entry,
        Err(error) => {
            eprintln!("Failed to open keychain entry: {error}");
            return HashMap::new();
        }
    };
    match entry.get_password() {
        Ok(blob) => serde_json::from_str(&blob).unwrap_or_else(|error| {
            eprintln!("Keychain blob is unreadable, ignoring: {error}");
            HashMap::new()
        }),
        Err(keyring::Error::NoEntry) => HashMap::new(),
        Err(error) => {
            eprintln!("Keychain read failed: {error}");
            HashMap::new()
        }
    }
}

fn write_blob(map: &HashMap<String, String>) -> Result<(), String> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT).map_err(|error| error.to_string())?;
    if map.is_empty() {
        return match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        };
    }
    let blob = serde_json::to_string(map).map_err(|error| error.to_string())?;
    entry.set_password(&blob).map_err(|error| error.to_string())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Look up the password for a connection.
///
/// The first call this session reads the keychain blob (one OS prompt on
/// macOS); subsequent calls hit the in-memory cache.
pub fn get(connection_id: &str) -> Option<String> {
    let mut guard = cache().lock().expect("password cache poisoned");
    let map = guard.get_or_insert_with(read_blob);
    map.get(connection_id).cloned()
}

/// One change to the credential store: either set a password or clear it.
pub enum CredentialUpdate<'a> {
    Set { id: &'a str, password: &'a str },
    Clear { id: &'a str },
}

/// Apply a batch of credential updates and write the blob *once* if anything
/// changed. Returns `Ok(true)` if the blob was rewritten.
pub fn upsert_many(updates: &[CredentialUpdate<'_>]) -> Result<bool, String> {
    let mut guard = cache().lock().expect("password cache poisoned");
    let map = guard.get_or_insert_with(read_blob);
    let mut changed = false;
    for update in updates {
        match update {
            CredentialUpdate::Set { id, password } => {
                if map.get(*id).map(String::as_str) != Some(*password) {
                    map.insert((*id).to_string(), (*password).to_string());
                    changed = true;
                }
            }
            CredentialUpdate::Clear { id } => {
                if map.remove(*id).is_some() {
                    changed = true;
                }
            }
        }
    }
    if changed {
        write_blob(map)?;
    }
    Ok(changed)
}

/// Drop a connection's password from the cache and the keychain blob.
pub fn delete(connection_id: &str) -> Result<(), String> {
    let mut guard = cache().lock().expect("password cache poisoned");
    let map = guard.get_or_insert_with(read_blob);
    if map.remove(connection_id).is_none() {
        return Ok(());
    }
    write_blob(map)
}
