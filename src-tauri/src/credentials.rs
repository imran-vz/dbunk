//! App-wide credential storage backends.

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

static SESSION_KEY: OnceLock<Mutex<Option<[u8; 32]>>> = OnceLock::new();
static PASSWORD_CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn session_key() -> &'static Mutex<Option<[u8; 32]>> {
    SESSION_KEY.get_or_init(|| Mutex::new(None))
}

fn password_cache() -> &'static Mutex<HashMap<String, String>> {
    PASSWORD_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

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

pub async fn configure(
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
            storage::clear_sqlite_credentials(pool).await?;
            keychain::clear_all()?;
        }
        CredentialStorageMode::PlainSqlite => {
            storage::clear_verifier(pool).await?;
            storage::clear_sqlite_credentials(pool).await?;
            keychain::clear_all()?;
            *session_key()
                .lock()
                .expect("credential session key poisoned") = None;
        }
        CredentialStorageMode::Keychain => {
            storage::clear_verifier(pool).await?;
            storage::clear_sqlite_credentials(pool).await?;
            keychain::clear_all()?;
            *session_key()
                .lock()
                .expect("credential session key poisoned") = None;
        }
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

pub async fn read_all(
    pool: &SqlitePool,
    mode: CredentialStorageMode,
) -> Result<HashMap<String, String>, String> {
    match mode {
        CredentialStorageMode::Keychain => Ok(keychain::get_all()),
        CredentialStorageMode::PlainSqlite => {
            let rows = storage::read_sqlite_credentials(pool).await?;
            Ok(rows
                .into_iter()
                .map(|(id, _, password)| (id, password))
                .collect())
        }
        CredentialStorageMode::EncryptedSqlite => {
            let key = current_key()?;
            let rows = storage::read_sqlite_credentials(pool).await?;
            let mut map = HashMap::new();
            for (id, nonce, ciphertext) in rows {
                let nonce = nonce.ok_or_else(|| format!("Missing nonce for credential '{id}'"))?;
                map.insert(id, decrypt_text(&key, &nonce, &ciphertext)?);
            }
            Ok(map)
        }
    }
}

pub async fn write_all(
    pool: &SqlitePool,
    mode: CredentialStorageMode,
    credentials: &HashMap<String, String>,
) -> Result<(), String> {
    match mode {
        CredentialStorageMode::Keychain => {
            keychain::replace_all(credentials)?;
            storage::clear_sqlite_credentials(pool).await?;
        }
        CredentialStorageMode::PlainSqlite => {
            storage::clear_sqlite_credentials(pool).await?;
            for (id, password) in credentials {
                storage::upsert_sqlite_credential(pool, id, mode, None, password).await?;
            }
            keychain::clear_all()?;
        }
        CredentialStorageMode::EncryptedSqlite => {
            let key = current_key()?;
            storage::clear_sqlite_credentials(pool).await?;
            for (id, password) in credentials {
                let encrypted = encrypt_text(&key, password.as_bytes())?;
                storage::upsert_sqlite_credential(
                    pool,
                    id,
                    mode,
                    Some(&encrypted.nonce),
                    &encrypted.ciphertext,
                )
                .await?;
            }
            keychain::clear_all()?;
        }
    }
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

pub async fn change_mode(
    pool: &SqlitePool,
    from: CredentialStorageMode,
    to: CredentialStorageMode,
    password: Option<&str>,
) -> Result<(), String> {
    let existing = read_all(pool, from).await?;
    match to {
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
            if to != CredentialStorageMode::EncryptedSqlite {
                storage::clear_verifier(pool).await?;
                *session_key()
                    .lock()
                    .expect("credential session key poisoned") = None;
            }
        }
    }
    write_all(pool, to, &existing).await?;
    set_credential_mode(pool, to).await
}

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

fn current_key() -> Result<[u8; 32], String> {
    session_key()
        .lock()
        .expect("credential session key poisoned")
        .as_ref()
        .copied()
        .ok_or_else(|| "Credential storage is locked".to_string())
}

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
