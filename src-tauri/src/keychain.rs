//! Connection-credential storage backed by the OS keychain.
//!
//! ## Shape (ADR-0005, retained as a backend by ADR-0007)
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
//! - [`get_all`] — read the complete credential map.
//! - [`replace_all`] — replace the complete credential map.
//! - [`clear_all`] — delete the keychain blob.

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

pub fn get_all() -> HashMap<String, String> {
    let mut guard = cache().lock().expect("password cache poisoned");
    guard.get_or_insert_with(read_blob).clone()
}

pub fn replace_all(next: &HashMap<String, String>) -> Result<(), String> {
    let mut guard = cache().lock().expect("password cache poisoned");
    write_blob(next)?;
    *guard = Some(next.clone());
    Ok(())
}

pub fn clear_all() -> Result<(), String> {
    replace_all(&HashMap::new())
}
