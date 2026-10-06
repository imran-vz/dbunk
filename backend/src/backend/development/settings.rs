use super::*;
use crate::CredentialStorageMode;

/// Native DTOs deliberately exclude config paths, raw stores and secret-bearing
/// command payloads. Password arguments are consumed by services, never echoed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DevelopmentCredentialState {
    NeedsOnboarding,
    NeedsUnlock,
    NeedsRecovery,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DevelopmentStorageMode {
    Keychain,
    PlainSqlite,
    EncryptedSqlite,
}

impl DevelopmentStorageMode {
    /// Only Encrypted SQLite takes a credential password. A password sent
    /// with another mode would be silently ignored, which reads to the user as
    /// "any password is accepted", so it is refused instead.
    fn check_password(self, password: Option<&str>) -> Result<(), String> {
        match (self, password) {
            (Self::EncryptedSqlite, None | Some("")) => {
                Err("Encrypted SQLite needs a credential password".into())
            }
            (Self::EncryptedSqlite, Some(_)) | (_, None) => Ok(()),
            (_, Some(_)) => Err("Only Encrypted SQLite uses a credential password".into()),
        }
    }

    fn core(self) -> CredentialStorageMode {
        match self {
            Self::Keychain => CredentialStorageMode::Keychain,
            Self::PlainSqlite => CredentialStorageMode::PlainSqlite,
            Self::EncryptedSqlite => CredentialStorageMode::EncryptedSqlite,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevelopmentSettings {
    pub profile_id: String,
    pub mode: Option<DevelopmentStorageMode>,
    pub state: DevelopmentCredentialState,
    /// Set when the active OS Keychain could not be read (denied, locked,
    /// unavailable or unreadable). The workspace still loads and the entry is
    /// preserved; saved passwords stay unusable until access is granted and
    /// settings reload. The text is a fixed, secret-free message.
    pub keychain_unavailable: Option<String>,
}

impl Backend {
    pub(in crate::backend) fn development(&self) -> Result<Arc<Authority>, String> {
        self.0
            .development
            .clone()
            .ok_or_else(|| "Operation requires a stage04 development profile".into())
    }

    pub async fn development_settings(&self) -> Result<DevelopmentSettings, String> {
        let authority = self.development()?;
        self.development_call(move |state| async move { Ok(snapshot(&state, &authority).await) })
            .await
            .map_err(|_| "Native backend is closing".to_string())?
    }

    pub async fn configure_development_credentials(
        &self,
        mode: DevelopmentStorageMode,
        password: Option<String>,
    ) -> Result<DevelopmentSettings, String> {
        mode.check_password(password.as_deref())?;
        let authority = self.development()?;
        let inner = self.0.clone();
        self.development_call(move |state| async move {
            Ok(async {
                crate::backend::pg_tools::require_connection_settled(&inner, None)?;
                let _copy_retirement = crate::backend::table_copy::retire_connection(
                    &inner,
                    None,
                    tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                )
                .await?;
                let _seed_retirement = crate::backend::table_seed::retire_connection(
                    &inner,
                    None,
                    tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                )
                .await?;
                let _csv_retirement = crate::backend::csv_transfers::retire_connection(
                    &inner,
                    None,
                    tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                )
                .await?;
                crate::settings::configure_credential_storage_inner(
                    &state,
                    crate::ConfigureCredentialStoragePayload {
                        mode: mode.core(),
                        password,
                    },
                )
                .await?;
                snapshot(&state, &authority).await
            }
            .await)
        })
        .await
        .map_err(|_| "Native backend is closing".to_string())?
    }

    pub async fn unlock_development_credentials(
        &self,
        password: String,
    ) -> Result<DevelopmentSettings, String> {
        if password.is_empty() {
            return Err("Enter the credential password".into());
        }
        let authority = self.development()?;
        self.development_call(move |state| async move {
            Ok(async {
                credentials::unlock(&state.credentials, &password).await?;
                snapshot(&state, &authority).await
            }
            .await)
        })
        .await
        .map_err(|_| "Native backend is closing".to_string())?
    }

    pub async fn change_development_credentials(
        &self,
        mode: DevelopmentStorageMode,
        password: Option<String>,
        confirmed: bool,
    ) -> Result<DevelopmentSettings, String> {
        mode.check_password(password.as_deref())?;
        let authority = self.development()?;
        let inner = self.0.clone();
        self.development_call(move |state| async move {
            Ok(async {
                if !confirmed {
                    return Err("Credential storage change must be confirmed".into());
                }
                crate::backend::pg_tools::require_connection_settled(&inner, None)?;
                let _copy_retirement = crate::backend::table_copy::retire_connection(
                    &inner,
                    None,
                    tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                )
                .await?;
                let _seed_retirement = crate::backend::table_seed::retire_connection(
                    &inner,
                    None,
                    tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                )
                .await?;
                let _csv_retirement = crate::backend::csv_transfers::retire_connection(
                    &inner,
                    None,
                    tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                )
                .await?;
                crate::backend::data::retire_data(&inner, &state, None).await?;
                crate::settings::change_credential_storage_inner(
                    &state,
                    crate::ChangeCredentialStoragePayload {
                        mode: mode.core(),
                        password,
                        confirm: confirmed,
                    },
                )
                .await?;
                snapshot(&state, &authority).await
            }
            .await)
        })
        .await
        .map_err(|_| "Native backend is closing".to_string())?
    }

    pub async fn recover_development_credentials(&self) -> Result<DevelopmentSettings, String> {
        let authority = self.development()?;
        let inner = self.0.clone();
        self.development_call(move |state| async move {
            Ok(async {
                crate::backend::pg_tools::require_connection_settled(&inner, None)?;
                let _copy_retirement = crate::backend::table_copy::retire_connection(
                    &inner,
                    None,
                    tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                )
                .await?;
                let _seed_retirement = crate::backend::table_seed::retire_connection(
                    &inner,
                    None,
                    tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                )
                .await?;
                let _csv_retirement = crate::backend::csv_transfers::retire_connection(
                    &inner,
                    None,
                    tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                )
                .await?;
                crate::backend::data::retire_data(&inner, &state, None).await?;
                crate::socket_lifecycle::with_global_fence(
                    &state,
                    credentials::recover_native(&state.credentials),
                )
                .await?;
                snapshot(&state, &authority).await
            }
            .await)
        })
        .await
        .map_err(|_| "Native backend is closing".to_string())?
    }

    pub async fn reset_development_credentials(
        &self,
        confirmed_password_loss: bool,
    ) -> Result<DevelopmentSettings, String> {
        let authority = self.development()?;
        let inner = self.0.clone();
        if !confirmed_password_loss {
            return Err(
                "Reset must confirm loss of saved passwords; connections and drafts are kept"
                    .into(),
            );
        }
        self.development_call(move |state| async move {
            Ok(async {
                crate::backend::pg_tools::require_connection_settled(&inner, None)?;
                let _copy_retirement = crate::backend::table_copy::retire_connection(
                    &inner,
                    None,
                    tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                )
                .await?;
                let _seed_retirement = crate::backend::table_seed::retire_connection(
                    &inner,
                    None,
                    tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                )
                .await?;
                let _csv_retirement = crate::backend::csv_transfers::retire_connection(
                    &inner,
                    None,
                    tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                )
                .await?;
                crate::backend::data::retire_data(&inner, &state, None).await?;
                crate::settings::reset_credential_storage_inner(&state).await?;
                snapshot(&state, &authority).await
            }
            .await)
        })
        .await
        .map_err(|_| "Native backend is closing".to_string())?
    }
}

async fn snapshot(state: &AppState, authority: &Authority) -> Result<DevelopmentSettings, String> {
    let _credentials = credentials::mutation_guard(&state.credentials).await;
    let flag = storage::get_setting(&state.pool, "onboardingCompleted").await?;
    if !matches!(flag.as_deref(), None | Some("false" | "true")) {
        return Err("Development credential settings are unreadable; profile preserved".into());
    }
    let settings = crate::settings::load_app_settings(state).await?;
    if settings.onboarding_completed && settings.credential_storage_mode.is_none() {
        return Err("Development credential settings are inconsistent; profile preserved".into());
    }
    let mode = match settings.credential_storage_mode {
        None => None,
        Some(CredentialStorageMode::PlainSqlite) => Some(DevelopmentStorageMode::PlainSqlite),
        Some(CredentialStorageMode::EncryptedSqlite) => {
            Some(DevelopmentStorageMode::EncryptedSqlite)
        }
        Some(CredentialStorageMode::Keychain) => Some(DevelopmentStorageMode::Keychain),
    };
    if credentials::native_recovery_required(&state.credentials).await? {
        return Ok(DevelopmentSettings {
            profile_id: authority.profile_id.clone(),
            mode,
            state: DevelopmentCredentialState::NeedsRecovery,
            keychain_unavailable: None,
        });
    }
    let mut keychain_unavailable = None;
    if !settings.onboarding_completed {
        credentials::ensure_onboarding_empty(&state.credentials).await?;
    } else {
        let has_verifier = storage::read_verifier(&state.pool).await?.is_some();
        if has_verifier != (mode == Some(DevelopmentStorageMode::EncryptedSqlite)) {
            return Err(
                "Development credential verifier is inconsistent; profile preserved".into(),
            );
        }
        if matches!(settings.credential_state, crate::CredentialState::Ready) {
            let active = settings
                .credential_storage_mode
                .ok_or("Credential storage is not configured")?;
            let read = credentials::read_all(&state.credentials, active).await;
            keychain_unavailable = keychain_unavailable_from(active, read)?;
        }
    }
    let status = match settings.credential_state {
        crate::CredentialState::NeedsOnboarding => DevelopmentCredentialState::NeedsOnboarding,
        crate::CredentialState::NeedsUnlock => DevelopmentCredentialState::NeedsUnlock,
        crate::CredentialState::Ready => DevelopmentCredentialState::Ready,
    };
    Ok(DevelopmentSettings {
        profile_id: authority.profile_id.clone(),
        mode,
        state: status,
        keychain_unavailable,
    })
}

/// A Keychain read failure must not fail the whole workspace load: connections
/// and drafts stay usable, and the snapshot reports it so the UI can ask the
/// user to grant access and retry. SQLite read failures still fail the load
/// because they indicate local profile damage rather than an OS access policy.
fn keychain_unavailable_from<T>(
    mode: CredentialStorageMode,
    read: Result<T, String>,
) -> Result<Option<String>, String> {
    match read {
        Ok(_) => Ok(None),
        Err(error) if mode == CredentialStorageMode::Keychain => Ok(Some(error)),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod keychain_unavailable_tests {
    use super::*;

    #[test]
    fn only_keychain_read_failures_degrade_to_a_reported_state() {
        const DENIED: &str = "Credential Keychain access was denied or locked";
        let denied = || Err::<(), _>(DENIED.to_string());
        assert_eq!(
            keychain_unavailable_from(CredentialStorageMode::Keychain, denied()),
            Ok(Some(DENIED.to_string()))
        );
        assert_eq!(
            keychain_unavailable_from(CredentialStorageMode::Keychain, Ok(())),
            Ok(None)
        );
        for mode in [
            CredentialStorageMode::PlainSqlite,
            CredentialStorageMode::EncryptedSqlite,
        ] {
            assert!(keychain_unavailable_from(mode, denied()).is_err());
            assert_eq!(keychain_unavailable_from(mode, Ok(())), Ok(None));
        }
    }
}
