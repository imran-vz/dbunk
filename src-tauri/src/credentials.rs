//! App-wide credential storage.
//!
//! Per ADR-0007 dbunk supports three credential storage modes:
//!
//! - **Keychain** — OS keychain, one shared blob (see `keychain.rs`).
//! - **Encrypted SQLite** — `~/.config/dbunk/dbunk.sqlite::credentials`,
//!   passwords encrypted at rest with AES-256-GCM. The key derives from
//!   a user password via Argon2id and lives in process memory after
//!   `unlock`.
//! - **Plain SQLite** — same table, passwords stored as-is.
//!
//! ## Shape: [`CredentialBackend`] enum-with-methods
//!
//! Each mode is a variant with its own per-mode I/O struct
//! ([`KeychainBackend`], [`PlainSqliteBackend`],
//! [`EncryptedSqliteBackend`]). The variant's `impl` owns the per-mode
//! logic; the enum's methods are a 3-arm dispatch. Public functions
//! (`read_all`, `write_all`, `upsert`, …) construct a backend via
//! [`backend_for`] and delegate.
//!
//! ## Storage topology
//!
//! Three modes, **two** physical storage areas:
//!
//! - The OS keychain (independent).
//! - The SQLite `credentials` table (shared by Plain + Encrypted).
//!
//! [`clear_inactive_storage`] encodes this: when the active mode is
//! Keychain, we clear the SQLite credentials table; when the active
//! mode is either SQLite mode, we clear the keychain. We do NOT iterate
//! "every other mode and call `clear()`" because Plain and Encrypted
//! share rows — that would wipe what we just wrote.
//!
//! ## Process-local state
//!
//! - [`SESSION_KEY`] — the AES key for encrypted mode, set after a
//!   successful `unlock` or fresh `configure`. Cleared on `reset`,
//!   mode-switch away from Encrypted, and process exit.
//! - [`PASSWORD_CACHE`] — decoded credential map used by hot paths
//!   (`hydrate`, repeated `read_all`s).
//!
//! Both are `OnceLock<Mutex<…>>` globals. Tests that exercise the
//! Encrypted backend coordinate by setting/clearing `SESSION_KEY` in
//! setup.

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use rand::{rngs::OsRng, RngCore};
use sqlx::SqlitePool;

use crate::{keychain, storage, CredentialStorageMode, DatabaseEngine, StoredConnection};

const SETTING_ONBOARDING_COMPLETED: &str = "onboardingCompleted";
const SETTING_CREDENTIAL_STORAGE_MODE: &str = "credentialStorageMode";
const KDF_NAME: &str = "argon2id-v1";
const VERIFIER_TEXT: &[u8] = b"dbunk-credential-verifier-v1";

const ALL_MODES: [CredentialStorageMode; 3] = [
    CredentialStorageMode::Keychain,
    CredentialStorageMode::PlainSqlite,
    CredentialStorageMode::EncryptedSqlite,
];

static SESSION_KEY: OnceLock<Mutex<Option<[u8; 32]>>> = OnceLock::new();
static PASSWORD_CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn session_key() -> &'static Mutex<Option<[u8; 32]>> {
    SESSION_KEY.get_or_init(|| Mutex::new(None))
}

fn password_cache() -> &'static Mutex<HashMap<String, String>> {
    PASSWORD_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

// ---------------------------------------------------------------------------
// App-settings shims (onboarding flag + active mode)
// ---------------------------------------------------------------------------

pub async fn onboarding_completed(pool: &SqlitePool) -> Result<bool, String> {
    Ok(storage::get_setting(pool, SETTING_ONBOARDING_COMPLETED)
        .await?
        .as_deref()
        == Some("true"))
}

pub async fn credential_mode(pool: &SqlitePool) -> Result<Option<CredentialStorageMode>, String> {
    let Some(value) = storage::get_setting(pool, SETTING_CREDENTIAL_STORAGE_MODE).await? else {
        return Ok(None);
    };
    parse_mode(&value).map(Some)
}

pub async fn set_credential_mode(
    pool: &SqlitePool,
    mode: CredentialStorageMode,
) -> Result<(), String> {
    storage::set_setting(pool, SETTING_CREDENTIAL_STORAGE_MODE, mode.as_str()).await
}

pub async fn mark_onboarding_completed(pool: &SqlitePool) -> Result<(), String> {
    storage::set_setting(pool, SETTING_ONBOARDING_COMPLETED, "true").await
}

fn parse_mode(value: &str) -> Result<CredentialStorageMode, String> {
    match value {
        "keychain" => Ok(CredentialStorageMode::Keychain),
        "encrypted-sqlite" => Ok(CredentialStorageMode::EncryptedSqlite),
        "plain-sqlite" => Ok(CredentialStorageMode::PlainSqlite),
        _ => Err(format!("unknown credential storage mode '{value}'")),
    }
}

pub fn is_unlocked() -> bool {
    session_key()
        .lock()
        .expect("credential session key poisoned")
        .is_some()
}

// ---------------------------------------------------------------------------
// Credential backends
// ---------------------------------------------------------------------------

/// One of the three credential storage backends. Constructed via
/// [`backend_for`]. Variants own their per-mode state; the enum methods
/// are a 3-arm dispatch.
pub(crate) enum CredentialBackend {
    Keychain(KeychainBackend),
    PlainSqlite(PlainSqliteBackend),
    EncryptedSqlite(EncryptedSqliteBackend),
}

impl CredentialBackend {
    async fn read_all(&self) -> Result<HashMap<String, String>, String> {
        match self {
            Self::Keychain(b) => b.read_all().await,
            Self::PlainSqlite(b) => b.read_all().await,
            Self::EncryptedSqlite(b) => b.read_all().await,
        }
    }

    async fn write_all(&self, credentials: &HashMap<String, String>) -> Result<(), String> {
        match self {
            Self::Keychain(b) => b.write_all(credentials).await,
            Self::PlainSqlite(b) => b.write_all(credentials).await,
            Self::EncryptedSqlite(b) => b.write_all(credentials).await,
        }
    }
}

/// OS-keychain credential storage. Backend-wrapper over `keychain.rs`'s
/// single-blob shape (ADR-0005, retained as a backend by ADR-0007).
pub(crate) struct KeychainBackend;

impl KeychainBackend {
    async fn read_all(&self) -> Result<HashMap<String, String>, String> {
        Ok(keychain::get_all())
    }

    async fn write_all(&self, credentials: &HashMap<String, String>) -> Result<(), String> {
        keychain::replace_all(credentials)
    }
}

/// SQLite-backed plaintext credential storage. Same `credentials`
/// table as [`EncryptedSqliteBackend`]; difference is whether
/// `password_value` carries plaintext or AES-GCM ciphertext.
pub(crate) struct PlainSqliteBackend {
    pool: SqlitePool,
}

impl PlainSqliteBackend {
    async fn read_all(&self) -> Result<HashMap<String, String>, String> {
        let rows = storage::read_sqlite_credentials(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|(id, _, password)| (id, password))
            .collect())
    }

    async fn write_all(&self, credentials: &HashMap<String, String>) -> Result<(), String> {
        storage::clear_sqlite_credentials(&self.pool).await?;
        for (id, password) in credentials {
            storage::upsert_sqlite_credential(
                &self.pool,
                id,
                CredentialStorageMode::PlainSqlite,
                None,
                password,
            )
            .await?;
        }
        Ok(())
    }
}

/// SQLite-backed encrypted credential storage. AES-256-GCM with a
/// per-credential random nonce; the key is derived from a user
/// password via Argon2id and held in [`SESSION_KEY`]. The key is
/// resolved at construction time via [`backend_for`]; when locked,
/// `key` is `None` and read/write surface "Credential storage is
/// locked".
pub(crate) struct EncryptedSqliteBackend {
    pool: SqlitePool,
    key: Option<[u8; 32]>,
}

impl EncryptedSqliteBackend {
    fn key(&self) -> Result<&[u8; 32], String> {
        self.key
            .as_ref()
            .ok_or_else(|| "Credential storage is locked".to_string())
    }

    async fn read_all(&self) -> Result<HashMap<String, String>, String> {
        let key = self.key()?;
        let rows = storage::read_sqlite_credentials(&self.pool).await?;
        let mut map = HashMap::new();
        for (id, nonce, ciphertext) in rows {
            let nonce = nonce.ok_or_else(|| format!("Missing nonce for credential '{id}'"))?;
            map.insert(id, decrypt_text(key, &nonce, &ciphertext)?);
        }
        Ok(map)
    }

    async fn write_all(&self, credentials: &HashMap<String, String>) -> Result<(), String> {
        let key = self.key()?;
        storage::clear_sqlite_credentials(&self.pool).await?;
        for (id, password) in credentials {
            let encrypted = encrypt_text(key, password.as_bytes())?;
            storage::upsert_sqlite_credential(
                &self.pool,
                id,
                CredentialStorageMode::EncryptedSqlite,
                Some(&encrypted.nonce),
                &encrypted.ciphertext,
            )
            .await?;
        }
        Ok(())
    }
}

/// Resolve a backend for the given mode. Always infallible: the
/// EncryptedSqlite backend constructs with whatever session key
/// currently exists (or `None` if locked), and surfaces the "locked"
/// error from its read/write methods. This lets cross-backend
/// cleanup (`clear_inactive_storage`) run even when encrypted mode
/// can't be read — clearing the SQLite credentials table needs no
/// key.
fn backend_for(mode: CredentialStorageMode, pool: &SqlitePool) -> CredentialBackend {
    match mode {
        CredentialStorageMode::Keychain => CredentialBackend::Keychain(KeychainBackend),
        CredentialStorageMode::PlainSqlite => {
            CredentialBackend::PlainSqlite(PlainSqliteBackend { pool: pool.clone() })
        }
        CredentialStorageMode::EncryptedSqlite => {
            CredentialBackend::EncryptedSqlite(EncryptedSqliteBackend {
                pool: pool.clone(),
                key: session_key()
                    .lock()
                    .expect("credential session key poisoned")
                    .as_ref()
                    .copied(),
            })
        }
    }
}

/// Clear the storage areas the active mode doesn't own.
///
/// Three modes share **two** physical storage areas: the OS keychain
/// (independent) and the SQLite `credentials` table (shared by Plain
/// + Encrypted). Switching to Keychain means clearing the SQLite
/// table once. Switching to either SQLite mode means clearing the
/// keychain — and **not** also clearing SQLite, since `write_all`
/// already overwrites the table.
///
/// This is the single place encoding that topology. Iterating
/// `ALL_MODES.filter(!= active)` and calling `clear()` per-mode
/// would wipe what `write_all` just wrote.
async fn clear_inactive_storage(
    active: CredentialStorageMode,
    pool: &SqlitePool,
) -> Result<(), String> {
    match active {
        CredentialStorageMode::Keychain => storage::clear_sqlite_credentials(pool).await,
        CredentialStorageMode::PlainSqlite | CredentialStorageMode::EncryptedSqlite => {
            keychain::clear_all()
        }
    }
}

// ---------------------------------------------------------------------------
// Public surface — orchestration
// ---------------------------------------------------------------------------

pub async fn read_all(
    pool: &SqlitePool,
    mode: CredentialStorageMode,
) -> Result<HashMap<String, String>, String> {
    backend_for(mode, pool).read_all().await
}

pub async fn write_all(
    pool: &SqlitePool,
    mode: CredentialStorageMode,
    credentials: &HashMap<String, String>,
) -> Result<(), String> {
    backend_for(mode, pool).write_all(credentials).await?;
    clear_inactive_storage(mode, pool).await?;
    *password_cache()
        .lock()
        .expect("credential password cache poisoned") = credentials.clone();
    Ok(())
}

pub async fn upsert(
    pool: &SqlitePool,
    mode: CredentialStorageMode,
    connection: &StoredConnection,
) -> Result<(), String> {
    if matches!(connection.engine, DatabaseEngine::SQLite) || connection.password.is_empty() {
        return Ok(());
    }
    let mut all = read_all_cached(pool, mode).await?;
    all.insert(connection.id.clone(), connection.password.clone());
    write_all(pool, mode, &all).await
}

pub async fn delete(
    pool: &SqlitePool,
    mode: CredentialStorageMode,
    connection_id: &str,
) -> Result<(), String> {
    let mut all = read_all_cached(pool, mode).await?;
    if all.remove(connection_id).is_none() {
        return Ok(());
    }
    write_all(pool, mode, &all).await
}

pub async fn hydrate(
    pool: &SqlitePool,
    mode: CredentialStorageMode,
    connection: &mut StoredConnection,
) -> Result<(), String> {
    if matches!(connection.engine, DatabaseEngine::SQLite) {
        connection.password.clear();
        return Ok(());
    }
    let all = read_all_cached(pool, mode).await?;
    connection.password = all.get(&connection.id).cloned().unwrap_or_default();
    Ok(())
}

/// Initial onboarding setup. Clears every storage area to a clean
/// slate, sets up mode-specific session state (verifier + key for
/// Encrypted; clear verifier + key for the others), and records the
/// chosen mode + onboarding-complete flag.
pub async fn configure(
    pool: &SqlitePool,
    mode: CredentialStorageMode,
    password: Option<&str>,
) -> Result<(), String> {
    setup_mode(pool, mode, password).await?;
    // Clean every backend — onboarding has no prior credentials to
    // preserve. Iterating all_modes is safe here because we haven't
    // written anything yet.
    for backend_mode in ALL_MODES {
        let _ = clear_storage_for(backend_mode, pool).await;
    }
    password_cache()
        .lock()
        .expect("credential password cache poisoned")
        .clear();
    set_credential_mode(pool, mode).await?;
    mark_onboarding_completed(pool).await
}

pub async fn unlock(pool: &SqlitePool, password: &str) -> Result<(), String> {
    let key = verify_password(pool, password).await?;
    *session_key()
        .lock()
        .expect("credential session key poisoned") = Some(key);
    password_cache()
        .lock()
        .expect("credential password cache poisoned")
        .clear();
    Ok(())
}

pub async fn reset(pool: &SqlitePool) -> Result<(), String> {
    storage::clear_sqlite_credentials(pool).await?;
    storage::clear_verifier(pool).await?;
    keychain::clear_all()?;
    storage::set_setting(pool, SETTING_ONBOARDING_COMPLETED, "false").await?;
    password_cache()
        .lock()
        .expect("credential password cache poisoned")
        .clear();
    *session_key()
        .lock()
        .expect("credential session key poisoned") = None;
    Ok(())
}

/// Migrate credentials from one mode to another, preserving every
/// entry. Reads the old mode in full, sets up the new mode's session
/// state, writes via the new backend, then clears whichever storage
/// areas the new mode doesn't own.
pub async fn change_mode(
    pool: &SqlitePool,
    from: CredentialStorageMode,
    to: CredentialStorageMode,
    password: Option<&str>,
) -> Result<(), String> {
    let existing = backend_for(from, pool).read_all().await?;
    setup_mode(pool, to, password).await?;
    backend_for(to, pool).write_all(&existing).await?;
    clear_inactive_storage(to, pool).await?;
    set_credential_mode(pool, to).await
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

async fn read_all_cached(
    pool: &SqlitePool,
    mode: CredentialStorageMode,
) -> Result<HashMap<String, String>, String> {
    {
        let cache = password_cache()
            .lock()
            .expect("credential password cache poisoned");
        if !cache.is_empty() {
            return Ok(cache.clone());
        }
    }
    let all = read_all(pool, mode).await?;
    *password_cache()
        .lock()
        .expect("credential password cache poisoned") = all.clone();
    Ok(all)
}

/// Per-mode session setup: derive verifier + cache key for
/// Encrypted; clear verifier + key for the others.
async fn setup_mode(
    pool: &SqlitePool,
    mode: CredentialStorageMode,
    password: Option<&str>,
) -> Result<(), String> {
    match mode {
        CredentialStorageMode::EncryptedSqlite => {
            let password = password
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "Encryption password is required".to_string())?;
            let key = create_verifier(pool, password).await?;
            *session_key()
                .lock()
                .expect("credential session key poisoned") = Some(key);
        }
        CredentialStorageMode::PlainSqlite | CredentialStorageMode::Keychain => {
            storage::clear_verifier(pool).await?;
            *session_key()
                .lock()
                .expect("credential session key poisoned") = None;
        }
    }
    Ok(())
}

/// Clear the storage owned by a specific mode. Used only from
/// `configure` (clean slate). Cross-backend cleanup elsewhere goes
/// through [`clear_inactive_storage`] which knows the shared-table
/// topology.
async fn clear_storage_for(
    mode: CredentialStorageMode,
    pool: &SqlitePool,
) -> Result<(), String> {
    match mode {
        CredentialStorageMode::Keychain => keychain::clear_all(),
        CredentialStorageMode::PlainSqlite | CredentialStorageMode::EncryptedSqlite => {
            storage::clear_sqlite_credentials(pool).await
        }
    }
}

// ---------------------------------------------------------------------------
// Crypto (Argon2id KDF + AES-256-GCM)
// ---------------------------------------------------------------------------

async fn create_verifier(pool: &SqlitePool, password: &str) -> Result<[u8; 32], String> {
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    let key = derive_key(password, &salt)?;
    let encrypted = encrypt_text(&key, VERIFIER_TEXT)?;
    storage::write_verifier(
        pool,
        KDF_NAME,
        &B64.encode(salt),
        &encrypted.nonce,
        &encrypted.ciphertext,
    )
    .await?;
    Ok(key)
}

async fn verify_password(pool: &SqlitePool, password: &str) -> Result<[u8; 32], String> {
    let Some((kdf, salt, nonce, ciphertext)) = storage::read_verifier(pool).await? else {
        return Err("No encrypted credential verifier is configured".to_string());
    };
    if kdf != KDF_NAME {
        return Err(format!("Unsupported credential KDF '{kdf}'"));
    }
    let salt = B64.decode(salt).map_err(|error| error.to_string())?;
    let key = derive_key(password, &salt)?;
    let verifier = decrypt_bytes(&key, &nonce, &ciphertext)?;
    if verifier != VERIFIER_TEXT {
        return Err("Incorrect credential password".to_string());
    }
    Ok(key)
}

fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; 32], String> {
    let params = Params::new(19_456, 2, 1, Some(32)).map_err(|error| error.to_string())?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|error| error.to_string())?;
    Ok(key)
}

struct EncryptedText {
    nonce: String,
    ciphertext: String,
}

fn encrypt_text(key: &[u8; 32], plaintext: &[u8]) -> Result<EncryptedText, String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|error| error.to_string())?;
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext)
        .map_err(|error| error.to_string())?;
    Ok(EncryptedText {
        nonce: B64.encode(nonce_bytes),
        ciphertext: B64.encode(ciphertext),
    })
}

fn decrypt_text(key: &[u8; 32], nonce: &str, ciphertext: &str) -> Result<String, String> {
    let bytes = decrypt_bytes(key, nonce, ciphertext)?;
    String::from_utf8(bytes).map_err(|error| error.to_string())
}

fn decrypt_bytes(key: &[u8; 32], nonce: &str, ciphertext: &str) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|error| error.to_string())?;
    let nonce = B64.decode(nonce).map_err(|error| error.to_string())?;
    let ciphertext = B64.decode(ciphertext).map_err(|error| error.to_string())?;
    cipher
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(|_| "Incorrect credential password".to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    //! Backend tests run against a tempdir SQLite pool. Keychain
    //! integration is deliberately skipped — it hits the OS and
    //! flakes in sandboxed CI. The trait-shape orchestration (cross-
    //! backend cleanup, mode-switch credential survival) is covered
    //! via the two SQLite-backed variants, which exercises the same
    //! enum dispatch + topology helper.
    //!
    //! Tests are `#[serial]` because they share the process-global
    //! `SESSION_KEY` and `PASSWORD_CACHE`. Without serialization a
    //! test that sets the key races with one that clears it. The
    //! globals are a deliberate design choice (see module-level
    //! docs) — the serial gate is the cost.
    use super::*;
    use crate::storage::{open_pool, Paths};
    use tempfile::TempDir;

    async fn fixture() -> (TempDir, SqlitePool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths::from_dir(dir.path().to_path_buf());
        let pool = open_pool(&paths).await.expect("open_pool");
        (dir, pool)
    }

    /// Seed `connections` rows for the credential IDs we'll write —
    /// the `credentials` table has a FK to `connections(id)` (ADR-0007:
    /// credential row deletes when its connection deletes), so we have
    /// to populate the parent first.
    async fn seed_connections(pool: &SqlitePool, ids: &[&str]) {
        for id in ids {
            let connection = StoredConnection {
                id: (*id).to_string(),
                name: format!("test {id}"),
                database: "test_db".into(),
                engine: DatabaseEngine::PostgreSQL,
                host: "localhost".into(),
                port: 5432,
                user: "u".into(),
                password: String::new(),
                role: "read/write".into(),
                last_activity_at: None,
                use_https: false,
                url_path: String::new(),
            };
            storage::upsert_connection(pool, &connection)
                .await
                .expect("seed connection");
        }
    }

    fn creds(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries
            .iter()
            .map(|(id, pw)| ((*id).to_string(), (*pw).to_string()))
            .collect()
    }

    fn set_session_key(key: [u8; 32]) {
        *session_key()
            .lock()
            .expect("credential session key poisoned") = Some(key);
    }

    fn clear_session_key() {
        *session_key()
            .lock()
            .expect("credential session key poisoned") = None;
        password_cache()
            .lock()
            .expect("credential password cache poisoned")
            .clear();
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn plain_sqlite_round_trip() {
        let (_dir, pool) = fixture().await;
        clear_session_key();
        seed_connections(&pool, &["conn-1", "conn-2"]).await;
        let backend = backend_for(CredentialStorageMode::PlainSqlite, &pool);
        let original = creds(&[("conn-1", "secret-one"), ("conn-2", "secret-two")]);
        backend.write_all(&original).await.expect("write_all");
        let roundtrip = backend.read_all().await.expect("read_all");
        assert_eq!(roundtrip, original);
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn encrypted_sqlite_round_trip() {
        // Set up a fresh verifier so `read_all`'s decrypt path
        // exercises the same key that wrote the ciphertext.
        let (_dir, pool) = fixture().await;
        seed_connections(&pool, &["conn-1", "conn-2"]).await;
        let key = create_verifier(&pool, "correct horse battery staple")
            .await
            .expect("verifier");
        set_session_key(key);
        let backend = backend_for(CredentialStorageMode::EncryptedSqlite, &pool);
        let original = creds(&[("conn-1", "secret-one"), ("conn-2", "secret-two")]);
        backend.write_all(&original).await.expect("write_all");
        let roundtrip = backend.read_all().await.expect("read_all");
        assert_eq!(roundtrip, original);
        clear_session_key();
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn encrypted_read_fails_when_locked() {
        let (_dir, pool) = fixture().await;
        clear_session_key();
        let backend = backend_for(CredentialStorageMode::EncryptedSqlite, &pool);
        let error = backend
            .read_all()
            .await
            .expect_err("expected locked error");
        assert!(error.contains("locked"));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn write_all_to_plain_preserves_rows_after_topology_cleanup() {
        // Regression: with three backends naively iterating ALL_MODES
        // and calling clear(), Encrypted.clear() would wipe the row
        // PlainSqliteBackend just wrote. `clear_inactive_storage` is
        // the topology-aware helper that prevents this — confirm it
        // does.
        let (_dir, pool) = fixture().await;
        clear_session_key();
        seed_connections(&pool, &["conn-1", "conn-2"]).await;
        let entries = creds(&[("conn-1", "alpha"), ("conn-2", "beta")]);
        write_all(&pool, CredentialStorageMode::PlainSqlite, &entries)
            .await
            .expect("write_all");
        let roundtrip = read_all(&pool, CredentialStorageMode::PlainSqlite)
            .await
            .expect("read_all");
        assert_eq!(roundtrip, entries);
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn change_mode_plain_to_encrypted_preserves_credentials() {
        // The headline mode-switch invariant: every credential
        // survives the move. Run a full Plain → Encrypted migration
        // and assert the new backend reports the same map.
        let (_dir, pool) = fixture().await;
        clear_session_key();
        seed_connections(&pool, &["a", "b", "c"]).await;
        let entries = creds(&[("a", "alpha"), ("b", "beta"), ("c", "gamma")]);
        write_all(&pool, CredentialStorageMode::PlainSqlite, &entries)
            .await
            .expect("seed plain");
        change_mode(
            &pool,
            CredentialStorageMode::PlainSqlite,
            CredentialStorageMode::EncryptedSqlite,
            Some("a different passphrase"),
        )
        .await
        .expect("change_mode");
        let migrated = read_all(&pool, CredentialStorageMode::EncryptedSqlite)
            .await
            .expect("read encrypted");
        assert_eq!(migrated, entries);
        clear_session_key();
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn change_mode_encrypted_to_plain_preserves_credentials() {
        // The reverse direction. Migrating away from encrypted also
        // clears the session key (setup_mode for non-encrypted modes
        // unsets it), so we verify final state via a fresh PlainSqlite
        // backend without a session key.
        let (_dir, pool) = fixture().await;
        clear_session_key();
        seed_connections(&pool, &["a", "b"]).await;
        let key = create_verifier(&pool, "passphrase")
            .await
            .expect("verifier");
        set_session_key(key);
        let entries = creds(&[("a", "alpha"), ("b", "beta")]);
        write_all(&pool, CredentialStorageMode::EncryptedSqlite, &entries)
            .await
            .expect("seed encrypted");
        change_mode(
            &pool,
            CredentialStorageMode::EncryptedSqlite,
            CredentialStorageMode::PlainSqlite,
            None,
        )
        .await
        .expect("change_mode");
        assert!(!is_unlocked(), "session key cleared after move to plain");
        let migrated = read_all(&pool, CredentialStorageMode::PlainSqlite)
            .await
            .expect("read plain");
        assert_eq!(migrated, entries);
    }
}
