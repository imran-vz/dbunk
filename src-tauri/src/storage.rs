//! On-disk JSON persistence for connections, query history, and saved queries.
//!
//! This module owns the rule "where does dbunk write its files" and the
//! per-entity read/write helpers. Constructing a [`Paths`] from a
//! `tempfile::tempdir()` is a complete fake — the persistence layer can be
//! exercised in pure Rust without a Tauri runtime.
//!
//! Entity types (`StoredConnection`, `QueryHistoryEntry`, `SavedQuery`) live
//! in `lib.rs` because they're also command-payload shapes.
//!
//! ## Corruption tolerance
//!
//! The three entities deliberately have different rules. `connections.json`
//! is load-bearing — silently returning an empty list would hide the user's
//! saved databases and the next save would overwrite the corrupted file.
//! `query_history.json` and `saved_queries.json` are recoverable — losing
//! either is annoying but not destructive, so we log the parse failure and
//! return an empty list so the app still boots.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{de::DeserializeOwned, Serialize};
use tauri::{path::BaseDirectory, AppHandle, Manager};

use crate::{QueryHistoryEntry, SavedQuery, StoredConnection};

const CONNECTIONS_FILE: &str = "connections.json";
const QUERY_HISTORY_FILE: &str = "query_history.json";
const SAVED_QUERIES_FILE: &str = "saved_queries.json";

/// Resolved location of dbunk's config directory.
///
/// Construct once via [`Paths::from_app`] at app start and pass a reference
/// (or hold via `tauri::State`) wherever persistence is needed. Tests can
/// build one with [`Paths::from_dir`] over a tempdir.
pub struct Paths {
    config_dir: PathBuf,
}

impl Paths {
    pub fn from_app(app: &AppHandle) -> Result<Self, String> {
        let config_dir = resolve_config_dir(app)?;
        Ok(Self { config_dir })
    }

    /// Test/seam constructor — accepts any directory as the config root.
    #[allow(dead_code)]
    pub fn from_dir(config_dir: PathBuf) -> Self {
        Self { config_dir }
    }

    pub fn connections_file(&self) -> PathBuf {
        self.config_dir.join(CONNECTIONS_FILE)
    }

    pub fn query_history_file(&self) -> PathBuf {
        self.config_dir.join(QUERY_HISTORY_FILE)
    }

    pub fn saved_queries_file(&self) -> PathBuf {
        self.config_dir.join(SAVED_QUERIES_FILE)
    }

    fn ensure_dir(&self) -> Result<(), String> {
        fs::create_dir_all(&self.config_dir).map_err(|error| error.to_string())
    }
}

fn resolve_config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        app.path()
            .resolve("dbunk", BaseDirectory::AppData)
            .map_err(|error| error.to_string())
    }
    #[cfg(not(target_os = "windows"))]
    {
        app.path()
            .resolve(".config/dbunk", BaseDirectory::Home)
            .map_err(|error| error.to_string())
    }
}

/// Read a JSON file into `T`. Returns `T::default()` for missing or empty
/// files. **Refuses corrupt JSON** — load-bearing files use this so a bad
/// blob fails loud rather than silently zeroing the user's state.
fn read_json_strict<T: DeserializeOwned + Default>(path: &Path) -> Result<T, String> {
    if !path.exists() {
        return Ok(T::default());
    }
    let data = fs::read_to_string(path).map_err(|error| error.to_string())?;
    if data.trim().is_empty() {
        return Ok(T::default());
    }
    serde_json::from_str(&data).map_err(|error| error.to_string())
}

/// Read a JSON file into `T`. Returns `T::default()` for missing, empty, or
/// **corrupt** files (logs the parse error). For recoverable state where
/// "start fresh" is friendlier than "refuse to boot".
fn read_json_lossy<T: DeserializeOwned + Default>(path: &Path, label: &str) -> T {
    if !path.exists() {
        return T::default();
    }
    let data = match fs::read_to_string(path) {
        Ok(data) => data,
        Err(error) => {
            eprintln!("{label} unreadable: {error}");
            return T::default();
        }
    };
    if data.trim().is_empty() {
        return T::default();
    }
    match serde_json::from_str(&data) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{label} is unreadable, ignoring: {error}");
            T::default()
        }
    }
}

fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), String> {
    let data = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    fs::write(path, data).map_err(|error| error.to_string())
}

// ---------------------------------------------------------------------------
// Connections
// ---------------------------------------------------------------------------

pub fn read_connections(paths: &Paths) -> Result<Vec<StoredConnection>, String> {
    read_json_strict(&paths.connections_file())
}

pub fn write_connections(
    paths: &Paths,
    connections: &[StoredConnection],
) -> Result<(), String> {
    paths.ensure_dir()?;
    write_json(&paths.connections_file(), connections)
}

// ---------------------------------------------------------------------------
// Query history
// ---------------------------------------------------------------------------

pub fn read_query_history(paths: &Paths) -> Vec<QueryHistoryEntry> {
    read_json_lossy(&paths.query_history_file(), "query_history.json")
}

pub fn write_query_history(
    paths: &Paths,
    entries: &[QueryHistoryEntry],
) -> Result<(), String> {
    paths.ensure_dir()?;
    write_json(&paths.query_history_file(), entries)
}

// ---------------------------------------------------------------------------
// Saved queries
// ---------------------------------------------------------------------------

pub fn read_saved_queries(paths: &Paths) -> Vec<SavedQuery> {
    read_json_lossy(&paths.saved_queries_file(), "saved_queries.json")
}

pub fn write_saved_queries(paths: &Paths, queries: &[SavedQuery]) -> Result<(), String> {
    paths.ensure_dir()?;
    write_json(&paths.saved_queries_file(), queries)
}

#[cfg(test)]
mod tests {
    //! These tests prove the testability claim of the storage module: by
    //! constructing `Paths::from_dir(tempdir)` we get full coverage of the
    //! persistence layer without needing a Tauri runtime, an `AppHandle`, or
    //! a real filesystem outside the test sandbox.
    use super::*;
    use crate::DatabaseEngine;

    fn fixture_paths() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths::from_dir(dir.path().to_path_buf());
        (dir, paths)
    }

    fn sample_connection(id: &str) -> StoredConnection {
        StoredConnection {
            id: id.to_string(),
            name: format!("conn {id}"),
            database: "core".into(),
            engine: DatabaseEngine::PostgreSQL,
            host: "localhost".into(),
            port: 5432,
            user: "u".into(),
            password: String::new(),
            role: "read/write".into(),
            last_activity_at: None,
        }
    }

    #[test]
    fn read_connections_returns_empty_when_file_missing() {
        let (_dir, paths) = fixture_paths();
        let connections = read_connections(&paths).expect("read");
        assert!(connections.is_empty());
    }

    #[test]
    fn write_then_read_round_trips_connections() {
        let (_dir, paths) = fixture_paths();
        let original = vec![sample_connection("a"), sample_connection("b")];
        write_connections(&paths, &original).expect("write");

        let loaded = read_connections(&paths).expect("read");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, "a");
        assert_eq!(loaded[1].name, "conn b");
    }

    #[test]
    fn read_connections_refuses_corrupt_json() {
        // Load-bearing file: a parse failure must surface, not silently
        // wipe the user's saved connections.
        let (_dir, paths) = fixture_paths();
        std::fs::create_dir_all(paths.connections_file().parent().unwrap()).unwrap();
        std::fs::write(paths.connections_file(), "{ not json").unwrap();

        let result = read_connections(&paths);
        assert!(result.is_err(), "expected strict parse failure");
    }

    #[test]
    fn read_query_history_tolerates_corrupt_json() {
        // Recoverable file: corruption logs and returns empty so the app
        // still boots.
        let (_dir, paths) = fixture_paths();
        std::fs::create_dir_all(paths.query_history_file().parent().unwrap()).unwrap();
        std::fs::write(paths.query_history_file(), "{ not json").unwrap();

        let entries = read_query_history(&paths);
        assert!(entries.is_empty());
    }

    #[test]
    fn read_saved_queries_tolerates_corrupt_json() {
        let (_dir, paths) = fixture_paths();
        std::fs::create_dir_all(paths.saved_queries_file().parent().unwrap()).unwrap();
        std::fs::write(paths.saved_queries_file(), "{ not json").unwrap();

        let entries = read_saved_queries(&paths);
        assert!(entries.is_empty());
    }
}
