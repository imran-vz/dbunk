//! Opt-in macOS Keychain acceptance against one newly created stage04 profile.
//! Never run in CI. Secrets are generated in memory and never printed or passed
//! on argv. The real OS adapter refuses every identity outside this exact profile.
use dbunk_lib::backend::{
    Backend, DevelopmentCredentialState, DevelopmentFixtures, DevelopmentPostgresConnection,
    DevelopmentStorageMode as Mode, WorkspaceDocument, WorkspaceSelection, WorkspaceSnapshot,
};
use keyring::credential::{CredentialApi, CredentialBuilderApi};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Deserialize)]
struct Marker {
    profile_id: String,
    credential_namespace: String,
}

#[derive(Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Ownership {
    probe: String,
    profile_id: String,
    credential_namespace: String,
}

#[derive(Serialize)]
struct Call {
    operation: &'static str,
    service: String,
    account: String,
}

struct GuardedBuilder {
    inner: Box<keyring::CredentialBuilder>,
    service: String,
    primary: String,
    backup: String,
    calls: Arc<Mutex<Vec<Call>>>,
}

impl CredentialBuilderApi for GuardedBuilder {
    fn build(
        &self,
        target: Option<&str>,
        service: &str,
        account: &str,
    ) -> keyring::Result<Box<keyring::Credential>> {
        if target.is_some()
            || service != self.service
            || (account != self.primary && account != self.backup)
        {
            self.calls.lock().unwrap().push(Call {
                operation: "REFUSED-UNSCOPED",
                service: service.into(),
                account: account.into(),
            });
            return Err(keyring::Error::NoStorageAccess(Box::new(
                std::io::Error::other("Probe refused an unowned credential identity"),
            )));
        }
        Ok(Box::new(GuardedCredential {
            inner: self.inner.build(target, service, account)?,
            service: service.into(),
            account: account.into(),
            calls: self.calls.clone(),
        }))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

struct GuardedCredential {
    inner: Box<keyring::Credential>,
    service: String,
    account: String,
    calls: Arc<Mutex<Vec<Call>>>,
}

impl GuardedCredential {
    fn record(&self, operation: &'static str) {
        self.calls.lock().unwrap().push(Call {
            operation,
            service: self.service.clone(),
            account: self.account.clone(),
        });
    }
}

impl CredentialApi for GuardedCredential {
    fn set_secret(&self, secret: &[u8]) -> keyring::Result<()> {
        self.record("write");
        self.inner.set_secret(secret)
    }

    fn get_secret(&self) -> keyring::Result<Vec<u8>> {
        self.record("read");
        self.inner.get_secret()
    }

    fn delete_credential(&self) -> keyring::Result<()> {
        self.record("delete");
        self.inner.delete_credential()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path, maximum: u64) -> Result<T, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| "Probe input unavailable")?
        .take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Probe input unreadable")?;
    if bytes.len() as u64 > maximum {
        return Err("Probe input exceeds its size limit".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "Probe input is invalid".into())
}

fn require(condition: bool, message: &str) -> Result<(), String> {
    condition.then_some(()).ok_or_else(|| message.into())
}

fn credentials(service: &str, account: &str) -> Result<Option<HashMap<String, String>>, String> {
    let entry =
        keyring::Entry::new(service, account).map_err(|_| "Owned Keychain entry unavailable")?;
    match entry.get_password() {
        Ok(value) => serde_json::from_str(&value)
            .map(Some)
            .map_err(|_| "Owned Keychain entry corrupt; preserved".into()),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err("Owned Keychain read failed; unlock it and retry".into()),
    }
}

fn form() -> DevelopmentPostgresConnection {
    DevelopmentPostgresConnection {
        name: "Owned Keychain acceptance".into(),
        host: "127.0.0.1".into(),
        port: 15432,
        database: "dbunk_demo".into(),
        user: "dbunk".into(),
        environment: Default::default(),
        safe_mode: Default::default(),
        read_only: true,
        tls: Default::default(),
        driver_options: Default::default(),
    }
}

async fn seed(backend: &Backend, service: &str, primary: &str, backup: &str) -> Result<(), String> {
    require(
        credentials(service, primary)?.is_none(),
        "Primary entry already exists; refused",
    )?;
    require(
        credentials(service, backup)?.is_none(),
        "Rollback entry already exists; refused",
    )?;
    require(
        backend.development_connections().await?.is_empty(),
        "Probe profile already has connections",
    )?;
    require(
        backend.development_settings().await?.state == DevelopmentCredentialState::NeedsOnboarding,
        "Probe profile is not fresh",
    )?;
    backend
        .configure_development_credentials(Mode::Keychain, None)
        .await?;
    let secret = uuid::Uuid::new_v4().to_string();
    let first = backend
        .save_development_connection(None, form(), secret.clone())
        .await?;
    let second = backend
        .duplicate_development_connection(first.id.clone())
        .await?;
    let mut edited = form();
    edited.name = "Blank-password edit preserved".into();
    backend
        .save_development_connection(Some(first.id.clone()), edited, String::new())
        .await?;
    let stored = credentials(service, primary)?.ok_or("Primary entry missing")?;
    require(
        stored.len() == 2
            && stored.get(&first.id) == Some(&secret)
            && stored.get(&second.id) == Some(&secret),
        "Create/duplicate/blank edit did not preserve passwords",
    )?;
    require(
        credentials(service, backup)?.is_none(),
        "Rollback entry not cleaned after commit",
    )?;
    let document_id = uuid::Uuid::new_v4().to_string();
    backend
        .save_development_workspace(
            None,
            WorkspaceSnapshot {
                documents: vec![WorkspaceDocument {
                    query_changes: None,
                    schema_changes: None,
                    table_ddl: None,
                    admin_control: None,
                    maintenance: None,
                    tool: None,
                    saved_query_id: None,
                    table: None,
                    id: document_id.clone(),
                    name: "Acceptance draft".into(),
                    connection_id: Some(first.id),
                    sql: "SELECT 'draft survives credential reset';".into(),
                    pinned: false,
                    selection: WorkspaceSelection::default(),
                }],
                active_document_id: Some(document_id),
                ..WorkspaceSnapshot::default()
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

async fn reopen(
    mut backend: Backend,
    path: &Path,
    fixtures: &DevelopmentFixtures,
    service: &str,
    primary: &str,
    backup: &str,
) -> Result<(), String> {
    let original = credentials(service, primary)?.ok_or("Seeded primary entry missing")?;
    require(
        original.len() == 2,
        "Seed phase did not leave two passwords",
    )?;
    require(
        backend.development_settings().await?.state == DevelopmentCredentialState::Ready,
        "Reopened Keychain profile is not Ready",
    )?;
    let draft = backend
        .load_development_workspace()
        .await
        .map_err(|error| error.to_string())?
        .snapshot
        .ok_or("Seed draft missing")?;
    require(
        draft.documents.len() == 1 && backend.development_connections().await?.len() == 2,
        "Seeded profile data missing",
    )?;
    backend
        .change_development_credentials(Mode::PlainSqlite, None, true)
        .await?;
    require(
        credentials(service, primary)?.is_none(),
        "Previous Keychain entry was not cleaned",
    )?;
    let unlock = uuid::Uuid::new_v4().to_string();
    backend
        .change_development_credentials(Mode::EncryptedSqlite, Some(unlock.clone()), true)
        .await?;
    backend.shutdown().await?;
    drop(backend);
    backend = Backend::open_development(path, fixtures).await?;
    require(
        backend.development_settings().await?.state == DevelopmentCredentialState::NeedsUnlock,
        "Reopened encrypted profile did not lock",
    )?;
    require(
        backend
            .unlock_development_credentials("deliberately-wrong-probe-password".into())
            .await
            .is_err(),
        "Wrong password was accepted",
    )?;
    backend.unlock_development_credentials(unlock).await?;
    backend
        .change_development_credentials(Mode::Keychain, None, true)
        .await?;
    require(
        credentials(service, primary)?.as_ref() == Some(&original),
        "Credential mode round trip changed secrets",
    )?;
    let connections = backend.development_connections().await?;
    backend
        .delete_development_connection(connections[1].id.clone())
        .await?;
    require(
        credentials(service, primary)?.is_some_and(|map| {
            map.len() == 1 && map.get(&connections[0].id) == original.get(&connections[0].id)
        }),
        "Deleting one connection altered its peer password",
    )?;
    require(
        backend.reset_development_credentials(false).await.is_err(),
        "Unconfirmed reset was accepted",
    )?;
    backend.reset_development_credentials(true).await?;
    require(
        backend.development_connections().await?.len() == 1,
        "Credential reset deleted metadata",
    )?;
    require(
        backend
            .load_development_workspace()
            .await
            .map_err(|error| error.to_string())?
            .snapshot
            == Some(draft),
        "Credential reset changed SQL drafts",
    )?;
    require(
        credentials(service, primary)?.is_none() && credentials(service, backup)?.is_none(),
        "Owned Keychain cleanup incomplete",
    )?;
    backend.shutdown().await?;
    drop(backend);
    let backend = Backend::open_development(path, fixtures).await?;
    let settings = backend.development_settings().await?;
    require(
        settings.state == DevelopmentCredentialState::NeedsOnboarding && settings.mode.is_none(),
        "Reset did not persist across reopen",
    )?;
    backend.shutdown().await
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Native Keychain acceptance failed: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    require(
        cfg!(target_os = "macos"),
        "Real Keychain probe requires macOS",
    )?;
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let [operation, path, manifest] = args.as_slice() else {
        return Err("Usage: native_keychain_probe <prepare|seed|reopen|cleanup> <new-disposable-profile> <verified-fixture-manifest>".into());
    };
    let fixtures = DevelopmentFixtures::from_json(
        &serde_json::to_string(&read_json::<serde_json::Value>(Path::new(manifest), 4096)?)
            .map_err(|_| "Invalid fixture manifest")?,
    )?;
    let path = PathBuf::from(path);
    let operation = operation.to_str().ok_or("Invalid operation")?;
    require(
        matches!(operation, "prepare" | "seed" | "reopen" | "cleanup"),
        "Unknown operation",
    )?;
    let backend = if operation == "prepare" {
        Backend::create_development(&path, fixtures.clone()).await?
    } else {
        Backend::open_development(&path, &fixtures).await?
    };
    let marker: Marker = read_json(&path.join(".dbunk-native-stage04"), 8192)?;
    let ownership = Ownership {
        probe: "native-keychain-v1".into(),
        profile_id: marker.profile_id.clone(),
        credential_namespace: marker.credential_namespace.clone(),
    };
    let service = format!("dbunk-native-stage04-{}", marker.credential_namespace);
    let primary = format!("connection-credentials-{}", marker.credential_namespace);
    let backup = format!("{primary}-connection-rollback-v1");
    if operation == "prepare" {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options
            .open(path.join("launch.json"))
            .map_err(|_| "Probe ownership marker could not be created")?;
        file.write_all(&serde_json::to_vec(&ownership).map_err(|_| "Probe ownership invalid")?)
            .and_then(|_| file.sync_all())
            .map_err(|_| "Probe ownership could not be committed")?;
        backend.shutdown().await?;
        println!(
            "{}",
            serde_json::json!({"phase":"prepared-no-keychain-access", "profile":path,"profile_id":marker.profile_id,"service":service,"accounts":[primary,backup]})
        );
        return Ok(());
    }
    require(
        read_json::<Ownership>(&path.join("launch.json"), 4096)? == ownership,
        "Not an owned Keychain acceptance profile",
    )?;
    let calls = Arc::new(Mutex::new(Vec::new()));
    keyring::set_default_credential_builder(Box::new(GuardedBuilder {
        inner: keyring::default::default_credential_builder(),
        service: service.clone(),
        primary: primary.clone(),
        backup: backup.clone(),
        calls: calls.clone(),
    }));
    let result = match operation {
        "seed" => {
            let result = seed(&backend, &service, &primary, &backup).await;
            let shutdown = backend.shutdown().await;
            result.and(shutdown)
        }
        "reopen" => reopen(backend, &path, &fixtures, &service, &primary, &backup).await,
        "cleanup" => {
            // Explicit failure cleanup touches only the two recorded disposable
            // identities. Preserve the profile and its journals for inspection.
            for account in [&primary, &backup] {
                let entry = keyring::Entry::new(&service, account)
                    .map_err(|_| "Owned entry unavailable")?;
                match entry.delete_credential() {
                    Ok(()) | Err(keyring::Error::NoEntry) => {}
                    Err(_) => return Err("Owned entry cleanup failed; retry after unlock".into()),
                }
            }
            backend.shutdown().await?;
            require(
                credentials(&service, &primary)?.is_none()
                    && credentials(&service, &backup)?.is_none(),
                "Owned cleanup incomplete",
            )
        }
        _ => unreachable!(),
    };
    let calls = calls.lock().unwrap();
    println!(
        "{}",
        serde_json::json!({"phase":operation,"passed":result.is_ok(),"profile_id":marker.profile_id,"calls":*calls,"network":"none; metadata only","secret_values":"omitted"})
    );
    require(
        !calls
            .iter()
            .any(|call| call.operation == "REFUSED-UNSCOPED"),
        "An unscoped Keychain operation was blocked",
    )?;
    result
}
