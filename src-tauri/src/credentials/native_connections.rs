//! Native-only connection writes. SQLite commits metadata and credentials
//! together. Keychain writes use a scoped rollback entry and a secret-free
//! SQLite phase marker so interrupted saves cannot silently split the stores.
use super::*;

const JOURNAL: &str = "native.connections.change.v1";
const SAVE_FAILED: &str = "Connection change could not be saved; reload credentials and retry";
const RECOVERY: &str = "An interrupted connection change requires credential recovery";

// One local command, never queued or retained in a collection. Boxing its
// metadata would add an allocation without reducing any bounded buffer.
#[allow(clippy::large_enum_variant)]
pub(crate) enum Change {
    Save {
        connection: StoredConnection,
        password: String,
        copy_from: Option<String>,
    },
    Delete {
        id: String,
    },
}

pub(crate) async fn recovery_required(context: &Context) -> Result<bool, String> {
    Ok(storage::get_setting(&context.pool, JOURNAL)
        .await?
        .is_some())
}

pub(crate) async fn ensure_settled(context: &Context) -> Result<(), String> {
    if recovery_required(context).await? {
        return Err(RECOVERY.into());
    }
    Ok(())
}

/// The caller holds canonical connection fences. This lock serializes changes
/// with credential mode/reset and metadata preparation, including duplicate IDs.
pub(crate) async fn mutate(
    context: &Context,
    prepare: impl FnOnce(&[StoredConnection]) -> Result<Change, String>,
) -> Result<Option<StoredConnection>, String> {
    if context.lifecycle != LifecyclePolicy::Native {
        return Err("Connection operation requires a native development profile".into());
    }
    let _guard = mutation_guard(context).await;
    native::ensure_settled(context).await?;
    ensure_settled(context).await?;
    if !onboarding_completed(&context.pool).await? {
        return Err("Configure credential storage before saving connections".into());
    }
    let mode = credential_mode(&context.pool)
        .await?
        .ok_or("Credential storage is not configured")?;
    let rows = storage::read_native_connections(&context.pool)
        .await
        .map_err(|_| SAVE_FAILED)?;
    let invalid = rows
        .iter()
        .filter(|(_, valid)| !valid)
        .map(|(connection, _)| connection.id().to_owned())
        .collect::<Vec<_>>();
    let all = rows
        .into_iter()
        .map(|(connection, _)| connection)
        .collect::<Vec<_>>();
    let change = prepare(&all)?;
    let (target, source) = match &change {
        Change::Save {
            connection,
            copy_from,
            ..
        } => (connection.id(), copy_from.as_deref()),
        Change::Delete { id } => (id.as_str(), None),
    };
    if invalid
        .iter()
        .any(|id| id == target || Some(id.as_str()) == source)
    {
        return Err(
            "Stored connection options are unsupported or unreadable; metadata preserved".into(),
        );
    }
    let mut secrets = read_all(context, mode).await?;
    let old_secrets = secrets.clone();
    match &change {
        Change::Save {
            connection,
            password,
            copy_from,
        } => {
            if let Some(source) = copy_from {
                if let Some(secret) = secrets.get(source).cloned() {
                    secrets.insert(connection.id().into(), secret);
                }
            } else if !password.is_empty() {
                secrets.insert(connection.id().into(), password.clone());
            }
        }
        Change::Delete { id } => {
            secrets.remove(id);
        }
    }
    let key = if mode == CredentialStorageMode::EncryptedSqlite {
        Some(
            context
                .session_key
                .lock()
                .expect("credential session key poisoned")
                .as_ref()
                .copied()
                .ok_or("Credential storage is locked")?,
        )
    } else {
        None
    };
    if mode == CredentialStorageMode::Keychain {
        storage::set_setting(&context.pool, JOURNAL, "preparing")
            .await
            .map_err(|_| SAVE_FAILED)?;
        context.keychain.write_connection_backup(&old_secrets)?;
        storage::set_setting(&context.pool, JOURNAL, "prepared")
            .await
            .map_err(|_| SAVE_FAILED)?;
        // Leave the marker even on denial: the OS outcome may be uncertain.
        context.keychain.replace_all(&secrets)?;
    }
    let write = async {
        let mut tx = context.pool.begin().await.map_err(|_| SAVE_FAILED)?;
        match &change {
            Change::Save { connection, .. } => {
                storage::upsert_connection_with(&mut *tx, connection)
                    .await
                    .map_err(|_| SAVE_FAILED)?
            }
            Change::Delete { id } => {
                let result = sqlx::query("DELETE FROM connections WHERE id = ?")
                    .bind(id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|_| SAVE_FAILED)?;
                if result.rows_affected() != 1 {
                    return Err("Connection no longer exists; reload and retry".to_string());
                }
            }
        }
        if mode == CredentialStorageMode::Keychain {
            sqlite::set_setting(&mut tx, JOURNAL, "committed")
                .await
                .map_err(|_| SAVE_FAILED)?;
        } else {
            sqlite::replace_in(&mut tx, &secrets, key.as_ref())
                .await
                .map_err(|_| SAVE_FAILED)?;
        }
        tx.commit().await.map_err(|_| SAVE_FAILED.to_string())
    }
    .await;
    if let Err(error) = write {
        if mode == CredentialStorageMode::Keychain {
            // Rollback failure remains explicit and durable for next launch.
            recover_locked(context).await?;
        }
        return Err(error);
    }
    *context
        .password_cache
        .lock()
        .expect("credential password cache poisoned") = secrets;
    if mode == CredentialStorageMode::Keychain {
        finish_journal(context)
            .await
            .map_err(|_| "Connection saved; credential cleanup requires recovery".to_string())?;
    }
    Ok(match change {
        Change::Save { mut connection, .. } => {
            connection.set_password(String::new());
            Some(connection)
        }
        Change::Delete { .. } => None,
    })
}

pub(crate) async fn recover(context: &Context) -> Result<(), String> {
    if context.lifecycle != LifecyclePolicy::Native {
        return Err("Recovery requires a native development profile".into());
    }
    let _guard = mutation_guard(context).await;
    recover_locked(context).await
}

async fn recover_locked(context: &Context) -> Result<(), String> {
    let phase = storage::get_setting(&context.pool, JOURNAL).await?;
    let Some(phase) = phase else {
        return Ok(());
    };
    if credential_mode(&context.pool).await? != Some(CredentialStorageMode::Keychain) {
        return Err(
            "Connection recovery journal conflicts with credential mode; profile preserved".into(),
        );
    }
    match phase.as_str() {
        "prepared" => {
            let previous = context.keychain.read_connection_backup()?;
            context.keychain.replace_all(&previous)?;
            *context
                .password_cache
                .lock()
                .expect("credential password cache poisoned") = previous;
            storage::set_setting(&context.pool, JOURNAL, "rolled-back").await?;
        }
        "preparing" | "committed" | "rolled-back" => {
            let current = context.keychain.get_all()?;
            *context
                .password_cache
                .lock()
                .expect("credential password cache poisoned") = current;
        }
        _ => return Err("Connection recovery journal is unreadable; profile preserved".into()),
    }
    finish_journal(context).await
}

async fn finish_journal(context: &Context) -> Result<(), String> {
    // Only committed/rolled-back phases reach here. Cleanup failure retains
    // its phase so a retry never needs a backup that was already removed.
    context.keychain.clear_connection_backup()?;
    sqlx::query("DELETE FROM app_settings WHERE key = ?")
        .bind(JOURNAL)
        .execute(&context.pool)
        .await
        .map_err(|_| RECOVERY)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keychain::testing::{self, RecordingStore};

    async fn context() -> (tempfile::TempDir, Arc<Context>, Arc<RecordingStore>) {
        let directory = tempfile::tempdir().unwrap();
        let paths = storage::Paths::from_dir(directory.path().to_owned());
        let pool = storage::open_pool(&paths).await.unwrap();
        let io = Arc::new(RecordingStore::default());
        let store = Arc::new(testing::store(
            &io,
            "connection-tests",
            keychain::ReadPolicy::Strict,
        ));
        let context = Context::new(pool, store, LifecyclePolicy::Native);
        set_credential_mode(&context.pool, CredentialStorageMode::Keychain)
            .await
            .unwrap();
        mark_onboarding_completed(&context.pool).await.unwrap();
        (directory, context, io)
    }

    fn save(name: &str, password: &str) -> Change {
        let mut connection =
            crate::app::test_postgres_connection("owned", crate::SafeMode::Protected, false);
        let StoredConnection::PostgreSQL(pg) = &mut connection else {
            unreachable!()
        };
        pg.name = name.into();
        pg.password.clear();
        Change::Save {
            connection,
            password: password.into(),
            copy_from: None,
        }
    }

    #[tokio::test]
    async fn keychain_sql_failure_restores_both_stores_and_cleans_owned_backup() {
        let (_directory, context, io) = context().await;
        mutate(&context, |_| Ok(save("before", "old-secret")))
            .await
            .unwrap();
        sqlx::query("CREATE TRIGGER reject_native_metadata BEFORE UPDATE ON connections BEGIN SELECT RAISE(ABORT, 'injected metadata failure'); END").execute(&context.pool).await.unwrap();
        assert!(mutate(&context, |_| Ok(save("after", "new-secret")))
            .await
            .is_err());
        assert_eq!(
            storage::read_connection_by_id(&context.pool, "owned")
                .await
                .unwrap()
                .unwrap()
                .name(),
            "before"
        );
        assert_eq!(context.keychain.get_all().unwrap()["owned"], "old-secret");
        assert!(!recovery_required(&context).await.unwrap());
        let state = io.state.lock().unwrap();
        assert_eq!(state.blobs.len(), 1);
        assert!(state.calls.iter().all(|(_, service, account)| service
            == "dbunk-native-stage04-connection-tests"
            && account.starts_with("connection-credentials-connection-tests")));
    }

    #[tokio::test]
    async fn committed_save_reports_cleanup_failure_and_empty_rollback_is_not_missing() {
        let (_directory, context, io) = context().await;
        sqlx::query("CREATE TRIGGER reject_native_metadata BEFORE INSERT ON connections BEGIN SELECT RAISE(ABORT, 'injected metadata failure'); END").execute(&context.pool).await.unwrap();
        assert!(mutate(&context, |_| Ok(save("never saved", "new-secret")))
            .await
            .is_err());
        assert!(storage::read_connections(&context.pool)
            .await
            .unwrap()
            .is_empty());
        assert!(context.keychain.get_all().unwrap().is_empty());
        assert!(io.state.lock().unwrap().blobs.is_empty());
        assert!(!recovery_required(&context).await.unwrap());
        sqlx::query("DROP TRIGGER reject_native_metadata")
            .execute(&context.pool)
            .await
            .unwrap();
        sqlx::query("CREATE TRIGGER reject_native_cleanup BEFORE DELETE ON app_settings WHEN OLD.key = 'native.connections.change.v1' BEGIN SELECT RAISE(ABORT, 'injected cleanup failure'); END").execute(&context.pool).await.unwrap();
        let error = mutate(&context, |_| Ok(save("saved", "new-secret")))
            .await
            .unwrap_err();
        assert!(error.contains("Connection saved"));
        assert!(!error.contains("new-secret"));
        assert_eq!(
            storage::read_connection_by_id(&context.pool, "owned")
                .await
                .unwrap()
                .unwrap()
                .name(),
            "saved"
        );
        assert_eq!(context.keychain.get_all().unwrap()["owned"], "new-secret");
        assert_eq!(
            storage::get_setting(&context.pool, JOURNAL)
                .await
                .unwrap()
                .as_deref(),
            Some("committed")
        );
        assert_eq!(
            io.state.lock().unwrap().blobs.len(),
            1,
            "backup cleaned before marker"
        );
        sqlx::query("DROP TRIGGER reject_native_cleanup")
            .execute(&context.pool)
            .await
            .unwrap();
        recover(&context).await.unwrap();
        assert!(!recovery_required(&context).await.unwrap());
        assert_eq!(context.keychain.get_all().unwrap()["owned"], "new-secret");
    }

    #[tokio::test]
    async fn every_interrupted_keychain_phase_is_retryable_and_missing_backup_fails_closed() {
        for phase in ["preparing", "prepared", "committed", "rolled-back"] {
            let (_directory, context, io) = context().await;
            let old = HashMap::from([("owned".into(), "old".into())]);
            let new = HashMap::from([("owned".into(), "new".into())]);
            context.keychain.replace_all(&new).unwrap();
            context.keychain.write_connection_backup(&old).unwrap();
            storage::set_setting(&context.pool, JOURNAL, phase)
                .await
                .unwrap();
            assert!(mutate(&context, |_| Ok(save("blocked", "other")))
                .await
                .is_err());
            io.state.lock().unwrap().deny_write = true;
            assert!(recover(&context).await.is_err());
            assert!(recovery_required(&context).await.unwrap());
            io.state.lock().unwrap().deny_write = false;
            recover(&context).await.unwrap();
            assert_eq!(
                context.keychain.get_all().unwrap()["owned"],
                if phase == "prepared" { "old" } else { "new" }
            );
            assert!(!recovery_required(&context).await.unwrap());
            assert_eq!(io.state.lock().unwrap().blobs.len(), 1);
        }
        let (_directory, context, _io) = context().await;
        storage::set_setting(&context.pool, JOURNAL, "prepared")
            .await
            .unwrap();
        assert!(recover(&context).await.unwrap_err().contains("missing"));
        assert!(recovery_required(&context).await.unwrap());
        storage::set_setting(&context.pool, JOURNAL, "unknown")
            .await
            .unwrap();
        assert!(recover(&context).await.unwrap_err().contains("unreadable"));
    }
}
