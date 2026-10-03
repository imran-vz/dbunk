//! SQLite credential writes commit as one unit. Profile lifecycle changes also
//! include the verifier and settings, and publish in-memory state only afterward.

use super::*;
use sqlx::{Sqlite, Transaction};

pub(super) async fn replace(
    pool: &SqlitePool,
    credentials: &HashMap<String, String>,
    key: Option<&[u8; 32]>,
) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|error| error.to_string())?;
    replace_in(&mut tx, credentials, key).await?;
    tx.commit().await.map_err(|error| error.to_string())
}

pub(super) async fn replace_in(
    tx: &mut Transaction<'_, Sqlite>,
    credentials: &HashMap<String, String>,
    key: Option<&[u8; 32]>,
) -> Result<(), String> {
    sqlx::query("DELETE FROM credentials")
        .execute(&mut **tx)
        .await
        .map_err(|error| error.to_string())?;
    for (id, password) in credentials {
        let encrypted = key
            .map(|key| encrypt_text(key, password.as_bytes()))
            .transpose()?;
        let mode = if key.is_some() {
            CredentialStorageMode::EncryptedSqlite
        } else {
            CredentialStorageMode::PlainSqlite
        };
        sqlx::query(
            "INSERT INTO credentials (credential_id, storage_mode, nonce, password_value, updated_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(mode.as_str())
        .bind(encrypted.as_ref().map(|value| value.nonce.as_str()))
        .bind(encrypted.as_ref().map_or(password.as_str(), |value| value.ciphertext.as_str()))
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&mut **tx)
        .await
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Fixture profiles only support SQLite. General native Keychain transitions
/// need their own durable cross-store recovery protocol before they are exposed.
pub(super) fn require_sqlite(mode: CredentialStorageMode) -> Result<(), String> {
    if mode == CredentialStorageMode::Keychain {
        return Err("Keychain configuration is unavailable for this fixture profile".into());
    }
    Ok(())
}

pub(super) async fn configure(
    context: &Context,
    mode: CredentialStorageMode,
    password: Option<&str>,
) -> Result<(), String> {
    require_sqlite(mode)?;
    let _guard = mutation_guard(context).await;
    if onboarding_completed(&context.pool).await? {
        return Err("Credential storage is already configured; change its mode instead".into());
    }
    ensure_onboarding_empty(context).await?;
    commit_configuration(context, Some(mode), password, &HashMap::new(), None).await
}

pub(super) async fn change_mode(
    context: &Context,
    from: CredentialStorageMode,
    to: CredentialStorageMode,
    password: Option<&str>,
) -> Result<(), String> {
    require_sqlite(from)?;
    require_sqlite(to)?;
    let _guard = mutation_guard(context).await;
    if credential_mode(&context.pool).await? != Some(from)
        || !onboarding_completed(&context.pool).await?
    {
        return Err("Credential storage mode changed; reload settings and retry".into());
    }
    let existing = read_all(context, from).await?;
    commit_configuration(context, Some(to), password, &existing, None).await
}

pub(super) async fn reset(context: &Context) -> Result<(), String> {
    let _guard = mutation_guard(context).await;
    if let Some(mode) = credential_mode(&context.pool).await? {
        require_sqlite(mode)?;
    }
    commit_configuration(context, None, None, &HashMap::new(), None).await
}

pub(super) async fn commit_configuration(
    context: &Context,
    mode: Option<CredentialStorageMode>,
    password: Option<&str>,
    credentials: &HashMap<String, String>,
    pending: Option<&str>,
) -> Result<(), String> {
    // Derive/encrypt before opening the transaction. A missing password or KDF
    // failure cannot invalidate the old verifier or unlocked session key.
    let verifier = if mode == Some(CredentialStorageMode::EncryptedSqlite) {
        let password = password
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "Encryption password is required".to_string())?;
        let mut salt = [0u8; 16];
        OsRng.fill_bytes(&mut salt);
        let key = derive_key(password, &salt)?;
        Some((key, B64.encode(salt), encrypt_text(&key, VERIFIER_TEXT)?))
    } else {
        None
    };
    let key = verifier.as_ref().map(|(key, _, _)| key);
    let mut tx = context
        .pool
        .begin()
        .await
        .map_err(|error| error.to_string())?;
    replace_in(&mut tx, credentials, key).await?;
    sqlx::query("DELETE FROM credential_verifier")
        .execute(&mut *tx)
        .await
        .map_err(|error| error.to_string())?;
    if let Some((_, salt, encrypted)) = &verifier {
        sqlx::query("INSERT INTO credential_verifier (id, kdf, salt, nonce, ciphertext, updated_at) VALUES (1, ?, ?, ?, ?, ?)")
            .bind(KDF_NAME).bind(salt).bind(&encrypted.nonce).bind(&encrypted.ciphertext)
            .bind(chrono::Utc::now().to_rfc3339())
            .execute(&mut *tx).await.map_err(|error| error.to_string())?;
    }
    if let Some(mode) = mode {
        set_setting(&mut tx, SETTING_CREDENTIAL_STORAGE_MODE, mode.as_str()).await?;
    }
    set_setting(
        &mut tx,
        SETTING_ONBOARDING_COMPLETED,
        if mode.is_some() { "true" } else { "false" },
    )
    .await?;
    if context.lifecycle == LifecyclePolicy::Native {
        if let Some(pending) = pending {
            set_setting(&mut tx, super::native::JOURNAL_KEY, pending).await?;
        } else {
            sqlx::query("DELETE FROM app_settings WHERE key = ?")
                .bind(super::native::JOURNAL_KEY)
                .execute(&mut *tx)
                .await
                .map_err(|error| error.to_string())?;
        }
        if mode.is_none() {
            sqlx::query("DELETE FROM app_settings WHERE key = ?")
                .bind(SETTING_CREDENTIAL_STORAGE_MODE)
                .execute(&mut *tx)
                .await
                .map_err(|error| error.to_string())?;
        }
    }
    tx.commit().await.map_err(|error| error.to_string())?;
    *context
        .session_key
        .lock()
        .expect("credential session key poisoned") = key.copied();
    *context
        .password_cache
        .lock()
        .expect("credential password cache poisoned") = credentials.clone();
    Ok(())
}

pub(super) async fn set_setting(
    tx: &mut Transaction<'_, Sqlite>,
    name: &str,
    value: &str,
) -> Result<(), String> {
    sqlx::query("INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at")
        .bind(name).bind(value).bind(chrono::Utc::now().to_rfc3339())
        .execute(&mut **tx).await.map_err(|error| error.to_string())?;
    Ok(())
}
