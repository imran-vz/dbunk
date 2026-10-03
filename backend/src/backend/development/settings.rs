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
        });
    }
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
            credentials::read_all(
                &state.credentials,
                settings
                    .credential_storage_mode
                    .ok_or("Credential storage is not configured")?,
            )
            .await?;
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
    })
}
