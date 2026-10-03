//! Native cross-store lifecycle. The journal contains only an operation tag.
//! SQLite remains authoritative until its commit. A staged Keychain target is
//! rolled back after interruption; a committed SQLite target requires explicit
//! Keychain cleanup. No background recovery ever prompts the OS Keychain.
use super::*;

pub(super) const JOURNAL_KEY: &str = "native.credentials.transition.v1";
const ROLLBACK_KEYCHAIN: &str = "rollback-staged-keychain";
const CLEANUP_KEYCHAIN: &str = "cleanup-previous-keychain";
const RESET_KEYCHAIN: &str = "finish-keychain-reset";

pub(super) async fn ensure_settled(context: &Context) -> Result<(), String> {
    if context.lifecycle == LifecyclePolicy::Native && recovery_required(context).await? {
        return Err(
            "Credential recovery is required; retry recovery before using saved passwords".into(),
        );
    }
    Ok(())
}

pub(crate) async fn recovery_required(context: &Context) -> Result<bool, String> {
    #[cfg(feature = "isolated-profile")]
    if super::native_connections::recovery_required(context).await? {
        return Ok(true);
    }
    Ok(storage::get_setting(&context.pool, JOURNAL_KEY)
        .await?
        .is_some())
}

pub(super) async fn configure(
    context: &Context,
    mode: CredentialStorageMode,
    password: Option<&str>,
) -> Result<(), String> {
    let _guard = mutation_guard(context).await;
    ensure_settled(context).await?;
    ensure_onboarding_empty(context).await?;
    if mode == CredentialStorageMode::Keychain {
        ensure_keychain_empty(context)?;
        // A successful strict read proves accessibility without creating or
        // deleting anything. There are no secrets to transfer during onboarding.
    }
    sqlite::commit_configuration(context, Some(mode), password, &HashMap::new(), None).await
}

fn ensure_keychain_empty(context: &Context) -> Result<(), String> {
    if !context.keychain.get_all()?.is_empty() {
        return Err("Inactive Keychain contains credentials; entry preserved for recovery".into());
    }
    Ok(())
}

pub(super) async fn change_mode(
    context: &Context,
    from: CredentialStorageMode,
    to: CredentialStorageMode,
    password: Option<&str>,
) -> Result<(), String> {
    let _guard = mutation_guard(context).await;
    ensure_settled(context).await?;
    if credential_mode(&context.pool).await? != Some(from)
        || !onboarding_completed(&context.pool).await?
    {
        return Err("Credential storage mode changed; reload settings and retry".into());
    }
    let existing = backend_for(from, context).read_all().await?;
    if to == CredentialStorageMode::Keychain {
        if from == to {
            return Ok(());
        }
        ensure_keychain_empty(context)?;
        storage::set_setting(&context.pool, JOURNAL_KEY, ROLLBACK_KEYCHAIN).await?;
        context.keychain.replace_all(&existing)?;
        // Commit selects the new authoritative store and removes old rows,
        // verifier and recovery intent atomically. Failure keeps the old store.
        sqlite::commit_configuration(context, Some(to), None, &HashMap::new(), None).await?;
        *context
            .password_cache
            .lock()
            .expect("credential password cache poisoned") = existing;
        return Ok(());
    }
    let pending = (from == CredentialStorageMode::Keychain).then_some(CLEANUP_KEYCHAIN);
    sqlite::commit_configuration(context, Some(to), password, &existing, pending).await?;
    if pending.is_some() {
        finish_cleanup(context).await?;
    }
    Ok(())
}

pub(super) async fn reset(context: &Context) -> Result<(), String> {
    let _guard = mutation_guard(context).await;
    ensure_settled(context).await?;
    if credential_mode(&context.pool).await? == Some(CredentialStorageMode::Keychain) {
        // Persist explicit reset intent before removing any password. After a
        // crash recovery finishes that admitted reset instead of claiming Ready.
        storage::set_setting(&context.pool, JOURNAL_KEY, RESET_KEYCHAIN).await?;
        context.keychain.replace_all(&HashMap::new())?;
    }
    sqlite::commit_configuration(context, None, None, &HashMap::new(), None).await
}

async fn finish_cleanup(context: &Context) -> Result<(), String> {
    context.keychain.replace_all(&HashMap::new())?;
    sqlx::query("DELETE FROM app_settings WHERE key = ?")
        .bind(JOURNAL_KEY)
        .execute(&context.pool)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Invoked explicitly by the user behind the global lifecycle fence. Repeating
/// after denial or interruption is safe, including failure after OS deletion.
#[cfg(any(test, feature = "isolated-profile"))]
pub(crate) async fn recover(context: &Context) -> Result<(), String> {
    if context.lifecycle != LifecyclePolicy::Native {
        return Err("Recovery requires a native development profile".into());
    }
    #[cfg(feature = "isolated-profile")]
    super::native_connections::recover(context).await?;
    let _guard = mutation_guard(context).await;
    let pending = storage::get_setting(&context.pool, JOURNAL_KEY).await?;
    let mode = credential_mode(&context.pool).await?;
    match pending.as_deref() {
        None => Ok(()),
        Some(ROLLBACK_KEYCHAIN | CLEANUP_KEYCHAIN)
            if matches!(
                mode,
                Some(CredentialStorageMode::PlainSqlite | CredentialStorageMode::EncryptedSqlite)
            ) =>
        {
            finish_cleanup(context).await
        }
        Some(RESET_KEYCHAIN) if mode == Some(CredentialStorageMode::Keychain) => {
            context.keychain.replace_all(&HashMap::new())?;
            sqlite::commit_configuration(context, None, None, &HashMap::new(), None).await
        }
        Some(_) => {
            Err("Unknown or inconsistent credential recovery record; profile preserved".into())
        }
    }
}

#[cfg(test)]
mod tests;
