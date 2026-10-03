use super::corpus::{self, CredentialFixture};
use super::*;
use crate::backend::{Backend, DevelopmentCredentialState, NativeProfileKind};
use std::sync::{Arc, Mutex};

const CHILD: &str = "DBUNK_LEGACY_IMPORT_TEST";

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    source: PathBuf,
}

/// Owned temporary root with a separate legacy config directory.
fn owned_fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    std::fs::create_dir(root.join("legacy-config")).unwrap();
    let source = root.join("legacy-config").join("dbunk.sqlite");
    Fixture {
        _temp: temp,
        root,
        source,
    }
}

async fn snapshot_of(fixture: &Fixture, name: &str, credentials: CredentialFixture) -> PathBuf {
    if !fixture.source.exists() {
        corpus::create(&fixture.source, credentials).await;
    }
    let snapshot = fixture.root.join(name);
    snapshot_legacy_profile(&fixture.source, &snapshot)
        .await
        .unwrap();
    snapshot
}

fn hashes(directory: &Path) -> BTreeMap<String, String> {
    std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            let name = entry.file_name().into_string().unwrap();
            let hash = if entry.file_type().unwrap().is_file() {
                snapshot::hash_file(&entry.path(), &name).unwrap().sha256
            } else {
                "directory".into()
            };
            (name, hash)
        })
        .collect()
}

/// Order-independent dump of every baseline table, excluding native-owned
/// settings (identity, journal) and migration timestamps.
async fn dump(database: &Path) -> BTreeMap<String, String> {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(database)
        .read_only(true);
    let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
    let mut dump = BTreeMap::new();
    for table in RECORD_TABLES
        .iter()
        .chain(&CREDENTIAL_TABLES)
        .chain(&["app_settings"])
    {
        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info(?) ORDER BY cid")
                .bind(table)
                .fetch_all(&mut connection)
                .await
                .unwrap();
        let filter = if *table == "app_settings" {
            " WHERE key NOT LIKE 'native.%'"
        } else {
            ""
        };
        let value: String = sqlx::query_scalar(&format!(
            "SELECT coalesce(json_group_array(json(row)), '[]') FROM
               (SELECT json_array({}) AS row FROM {table}{filter} ORDER BY row)",
            columns.join(", ")
        ))
        .fetch_one(&mut connection)
        .await
        .unwrap();
        dump.insert(table.to_string(), value);
    }
    connection.close().await.unwrap();
    dump
}

fn record_tables(dump: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    dump.iter()
        .filter(|(table, _)| RECORD_TABLES.contains(&table.as_str()))
        .map(|(table, value)| (table.clone(), value.clone()))
        .collect()
}

async fn setting(database: &Path, key: &str) -> Option<String> {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(database)
        .read_only(true);
    let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
    let value = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
        .bind(key)
        .fetch_optional(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
    value
}

fn leftovers(root: &Path, name: &str) -> Vec<String> {
    std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|entry| entry.starts_with(&format!(".{name}.dbunk-legacy-import")))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn corpus_round_trips_through_the_baseline_reader_and_schema_check() {
    let fixture = owned_fixture();
    corpus::create(&fixture.source, CredentialFixture::PlainSqlite).await;
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&fixture.source)
                .read_only(true),
        )
        .await
        .unwrap();
    let connections = crate::storage::read_baseline_connections(&pool)
        .await
        .unwrap();
    pool.close().await;
    let by_id: BTreeMap<_, _> = connections
        .iter()
        .map(|connection| (connection.id().to_string(), connection))
        .collect();
    assert_eq!(by_id.len(), 5);
    let crate::StoredConnection::PostgreSQL(primary) = by_id[corpus::PG_PRIMARY] else {
        panic!("primary is PostgreSQL");
    };
    assert_eq!(primary.organization.folder, "Production");
    assert!(primary.organization.is_favorite);
    assert_eq!(primary.organization.color, "red");
    assert!(primary.read_only);
    assert_eq!(primary.safe_mode, crate::SafeMode::Strict);
    assert_eq!(
        primary
            .tls_options
            .as_ref()
            .unwrap()
            .root_cert_path
            .as_deref(),
        Some("/synthetic/ca.pem")
    );
    let crate::StoredConnection::PostgreSQL(bastion) = by_id[corpus::PG_BASTION] else {
        panic!("bastion connection is PostgreSQL");
    };
    assert!(bastion.ssh_tunnel.enabled);
    assert_eq!(bastion.ssh_tunnel.jump_chain, [corpus::BASTION]);
    assert!(matches!(
        by_id[corpus::REDIS],
        crate::StoredConnection::Redis(redis) if redis.db_number == 3
    ));
    // Baseline order is name order; the corpus names encode it.
    assert_eq!(connections[0].id(), corpus::PG_PRIMARY);
    assert_eq!(connections[4].id(), corpus::REDIS);

    let legacy = inspect(&fixture.source).await.unwrap();
    assert_eq!(legacy.credential_mode.as_deref(), Some("plain-sqlite"));
    assert_eq!(legacy.unsupported_engines, [corpus::MYSQL, corpus::REDIS]);
    assert_eq!(legacy.columns, baseline_columns().await.unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_captures_uncheckpointed_wal_without_touching_the_source() {
    let fixture = owned_fixture();
    // A baseline host that has not quit: every committed page is still in WAL.
    let mut writer = corpus::open_writer(&fixture.source).await;
    corpus::build(&mut writer, CredentialFixture::PlainSqlite).await;
    let wal = fixture.root.join("legacy-config/dbunk.sqlite-wal");
    assert!(std::fs::metadata(&wal).unwrap().len() > 0);
    let main_only = fixture.root.join("main-only.sqlite");
    std::fs::copy(&fixture.source, &main_only).unwrap();
    let mut plain_copy = SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new().filename(&main_only),
    )
    .await
    .unwrap();
    let tables: i64 = sqlx::query_scalar("SELECT count(*) FROM sqlite_master")
        .fetch_one(&mut plain_copy)
        .await
        .unwrap();
    plain_copy.close().await.unwrap();
    assert_eq!(tables, 0, "a plain main-file copy loses the WAL content");

    let config = fixture.root.join("legacy-config");
    let before = hashes(&config);
    let snapshot = fixture.root.join("snapshot");
    let manifest = snapshot_legacy_profile(&fixture.source, &snapshot)
        .await
        .unwrap();
    assert_eq!(hashes(&config), before);
    let captured: Vec<_> = manifest
        .source_files
        .iter()
        .map(|file| (file.name.clone(), file.sha256.clone()))
        .collect();
    assert_eq!(
        captured,
        [
            ("dbunk.sqlite".to_string(), before["dbunk.sqlite"].clone()),
            (
                "dbunk.sqlite-wal".into(),
                before["dbunk.sqlite-wal"].clone()
            ),
        ]
    );
    assert_eq!(manifest.schema_version, Some(18));
    let verified = snapshot::verify(&snapshot).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&verified.database)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o400);
    }
    let snapshot_dump = dump(&verified.database).await;
    assert!(snapshot_dump["connections"].contains(corpus::REDIS));

    let destination = fixture.root.join("imported");
    import_legacy_profile(&snapshot, &destination)
        .await
        .unwrap();
    assert_eq!(hashes(&config), before, "import never touches the source");
    assert_eq!(snapshot::verify(&snapshot).unwrap().manifest, manifest);
    // The live writer still sees its own data and can continue.
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM connections")
        .fetch_one(&mut writer)
        .await
        .unwrap();
    assert_eq!(count, 5);
    writer.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_refuses_concurrent_writes_and_unsafe_paths() {
    let fixture = owned_fixture();
    corpus::create(&fixture.source, CredentialFixture::PlainSqlite).await;
    let snapshot = fixture.root.join("snapshot");
    let error = snapshot::snapshot_with(&fixture.source, &snapshot, || {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(&fixture.source)
            .unwrap()
            .write_all(b"concurrent host write")
            .unwrap();
    })
    .await
    .unwrap_err();
    assert!(error.contains("changed during capture"), "{error}");
    assert!(!snapshot.exists(), "a refused capture leaves no snapshot");

    let other = owned_fixture();
    corpus::create(&other.source, CredentialFixture::PlainSqlite).await;
    std::fs::create_dir(other.root.join("existing")).unwrap();
    std::os::unix::fs::symlink(&other.source, other.root.join("link.sqlite")).unwrap();
    for (source, target) in [
        (PathBuf::from("relative.sqlite"), other.root.join("a")),
        (
            other.source.clone(),
            other.root.join("legacy-config/inside"),
        ),
        (other.source.clone(), other.root.join("existing")),
        (other.root.join("link.sqlite"), other.root.join("b")),
        (other.root.join("missing.sqlite"), other.root.join("c")),
    ] {
        assert!(snapshot_legacy_profile(&source, &target).await.is_err());
    }
    assert_eq!(
        std::fs::read_dir(other.root.join("legacy-config"))
            .unwrap()
            .count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn import_preserves_ids_unknown_fields_and_inactive_records() {
    let fixture = owned_fixture();
    let snapshot = snapshot_of(&fixture, "snapshot", CredentialFixture::PlainSqlite).await;
    let source_hashes = hashes(&fixture.root.join("legacy-config"));
    let destination = fixture.root.join("native");
    let manifest = import_legacy_profile(&snapshot, &destination)
        .await
        .unwrap();
    let database = destination.join("dbunk.sqlite");

    assert_eq!(manifest.connection_ids.len(), 5);
    assert_eq!(
        manifest.unsupported_engine_connection_ids,
        [corpus::MYSQL, corpus::REDIS]
    );
    assert_eq!(
        manifest.unsupported_option_connection_ids,
        [corpus::PG_UNKNOWN_FIELD]
    );
    assert_eq!(manifest.credential_mode.as_deref(), Some("plain-sqlite"));
    assert_eq!(manifest.credential_rows_imported, 2);
    assert_eq!(manifest.credential_rows_not_imported, 1);
    assert!(!manifest.keychain_secrets_require_setup);
    assert_eq!(
        manifest.workspace,
        WorkspaceMapping {
            status: WorkspaceMappingStatus::Mapped,
            mapped_documents: 2,
            unmapped_tabs: 3,
        }
    );
    assert_eq!(manifest.tables["query_history"], 3);
    assert_eq!(manifest.tables["safety_overrides"], 2);
    assert_eq!(manifest.tables["ui_state"], 7);

    // Every baseline record, including React keys and unknown fields, is
    // byte-identical; native state is added only under its own key.
    let source = dump(&snapshot.join("legacy.sqlite")).await;
    let imported = dump(&database).await;
    let mut expected = record_tables(&source);
    let mut actual = record_tables(&imported);
    let native_ui: Vec<serde_json::Value> =
        serde_json::from_str::<Vec<serde_json::Value>>(&actual.remove("ui_state").unwrap())
            .unwrap();
    let source_ui: Vec<serde_json::Value> =
        serde_json::from_str(&expected.remove("ui_state").unwrap()).unwrap();
    // Native migration 19 appends `project`; baseline rows import ungrouped.
    let mut rows: Vec<Vec<serde_json::Value>> =
        serde_json::from_str(&actual["connections"]).unwrap();
    for row in &mut rows {
        assert_eq!(row.pop(), Some(serde_json::Value::from("")));
    }
    actual.insert("connections".into(), serde_json::to_string(&rows).unwrap());
    assert_eq!(actual, expected);
    let (native, react): (Vec<_>, Vec<_>) = native_ui
        .into_iter()
        .partition(|row| row[0] == "ui.v1.native.workspace");
    assert_eq!(react, source_ui);
    assert!(imported["connections"]
        .contains(&serde_json::to_string(corpus::UNKNOWN_DRIVER_OPTIONS).unwrap()));
    assert!(imported["ui_state"].contains(corpus::UNKNOWN_UI_KEY));
    assert_eq!(
        setting(&database, "futureSetting.v9").await.as_deref(),
        Some(r#"{"opaque":true}"#)
    );
    assert_eq!(setting(&database, "theme").await.as_deref(), Some("dark"));
    assert_eq!(
        setting(&database, "credentialStorageMode").await.as_deref(),
        Some("plain-sqlite")
    );

    let workspace: serde_json::Value =
        serde_json::from_str(native[0][1].as_str().unwrap()).unwrap();
    let documents = workspace["snapshot"]["documents"].as_array().unwrap();
    assert_eq!(documents[0]["id"], "tab-query-1");
    assert_eq!(documents[0]["sql"], corpus::QUERY_SQL);
    assert_eq!(documents[0]["pinned"], true);
    assert_eq!(
        documents[0]["selection"],
        serde_json::json!({"anchor": 7, "head": corpus::QUERY_SQL.find("as greeting").unwrap()})
    );
    assert_eq!(
        documents[1]["connectionId"],
        "c0ffee00-0000-4000-8000-0000000000ff"
    );
    assert_eq!(workspace["snapshot"]["activeDocumentId"], "tab-query-1");

    let credentials: Vec<serde_json::Value> =
        serde_json::from_str(&imported["credentials"]).unwrap();
    assert_eq!(credentials.len(), 2);
    assert!(credentials.iter().all(|row| row[1] == "plain-sqlite"));

    // The manifest is redacted: no secrets, SQL, hosts or local paths.
    let encoded = serde_json::to_string(&manifest).unwrap();
    for forbidden in [
        corpus::PRIMARY_SECRET,
        corpus::BASTION_SECRET,
        "select",
        "synthetic.invalid",
        fixture.root.to_str().unwrap(),
    ] {
        assert!(!encoded.contains(forbidden), "manifest leaked {forbidden}");
    }
    // Only files the ordinary opener admits; SQLite companions are transient.
    let mut names: Vec<_> = std::fs::read_dir(&destination)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| !matches!(name.as_str(), "dbunk.sqlite-wal" | "dbunk.sqlite-shm"))
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            ".dbunk-native-lock",
            ".dbunk-native-profile",
            "dbunk.sqlite"
        ]
    );
    assert!(leftovers(&fixture.root, "native").is_empty());

    // Re-running a completed import writes nothing and returns the same record.
    assert_eq!(
        import_legacy_profile(&snapshot, &destination)
            .await
            .unwrap(),
        manifest
    );
    assert_eq!(dump(&database).await, imported);
    assert_eq!(hashes(&fixture.root.join("legacy-config")), source_hashes);
}

fn normalized(mut manifest: LegacyImportManifest) -> LegacyImportManifest {
    manifest.import_id.clear();
    manifest.profile_id.clear();
    manifest
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interruption_at_every_stage_resumes_to_the_reference_result() {
    let fixture = owned_fixture();
    let snapshot = snapshot_of(&fixture, "snapshot", CredentialFixture::PlainSqlite).await;
    let snapshot_hashes = hashes(&snapshot);
    let source_hashes = hashes(&fixture.root.join("legacy-config"));
    let reference_path = fixture.root.join("reference");
    let reference = import_legacy_profile(&snapshot, &reference_path)
        .await
        .unwrap();
    let reference_dump = dump(&reference_path.join("dbunk.sqlite")).await;

    for (index, failpoint) in ALL_FAILPOINTS.into_iter().enumerate() {
        let name = format!("interrupted-{index}");
        let destination = fixture.root.join(&name);
        let error = import_with(&snapshot, &destination, Some(failpoint))
            .await
            .unwrap_err();
        assert!(error.contains("Injected"), "{failpoint:?}: {error}");
        // Ready is never observable before the verified journal is published.
        assert_eq!(
            destination.exists(),
            failpoint == Failpoint::AfterPublish,
            "{failpoint:?}"
        );
        let resumed = import_legacy_profile(&snapshot, &destination)
            .await
            .unwrap_or_else(|error| panic!("{failpoint:?}: {error}"));
        assert_eq!(normalized(resumed.clone()), normalized(reference.clone()));
        assert_eq!(
            dump(&destination.join("dbunk.sqlite")).await,
            reference_dump,
            "{failpoint:?}"
        );
        assert_eq!(
            import_legacy_profile(&snapshot, &destination)
                .await
                .unwrap(),
            resumed
        );
        assert!(leftovers(&fixture.root, &name).is_empty(), "{failpoint:?}");
    }
    assert_eq!(hashes(&snapshot), snapshot_hashes);
    assert_eq!(hashes(&fixture.root.join("legacy-config")), source_hashes);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refuses_future_versions_other_identities_and_other_snapshots() {
    let future = owned_fixture();
    corpus::create(&future.source, CredentialFixture::PlainSqlite).await;
    corpus::add_future_version(&future.source).await;
    let snapshot = future.root.join("snapshot");
    let manifest = snapshot_legacy_profile(&future.source, &snapshot)
        .await
        .unwrap();
    assert_eq!(manifest.schema_version, Some(19));
    let destination = future.root.join("native");
    let error = import_legacy_profile(&snapshot, &destination)
        .await
        .unwrap_err();
    assert!(
        error.contains("newer than the supported baseline"),
        "{error}"
    );
    assert!(!destination.exists());
    assert!(leftovers(&future.root, "native").is_empty());

    let fixture = owned_fixture();
    let first = snapshot_of(&fixture, "first", CredentialFixture::PlainSqlite).await;
    let second = snapshot_of(&fixture, "second", CredentialFixture::PlainSqlite).await;

    // An existing empty directory or unmarked legacy profile is never adopted.
    let empty = fixture.root.join("empty");
    std::fs::create_dir(&empty).unwrap();
    let unmarked = fixture.root.join("unmarked");
    std::fs::create_dir(&unmarked).unwrap();
    std::fs::copy(&fixture.source, unmarked.join("dbunk.sqlite")).unwrap();
    for path in [&empty, &unmarked] {
        let before = hashes(path);
        let error = import_legacy_profile(&first, path).await.unwrap_err();
        assert!(error.contains("another identity"), "{error}");
        assert_eq!(hashes(path), before);
    }

    // A completed import of one snapshot refuses a different snapshot.
    let destination = fixture.root.join("native");
    import_legacy_profile(&first, &destination).await.unwrap();
    let before = dump(&destination.join("dbunk.sqlite")).await;
    let error = import_legacy_profile(&second, &destination)
        .await
        .unwrap_err();
    assert!(error.contains("another identity"), "{error}");
    assert_eq!(dump(&destination.join("dbunk.sqlite")).await, before);

    // Unfinished staging for one snapshot is not resumed with another.
    let pending = fixture.root.join("pending");
    import_with(&first, &pending, Some(Failpoint::AfterRecordsCommit))
        .await
        .unwrap_err();
    let error = import_legacy_profile(&second, &pending).await.unwrap_err();
    assert!(error.contains("different snapshot"), "{error}");
    import_legacy_profile(&first, &pending).await.unwrap();

    // A modified snapshot is refused before any destination exists.
    let tampered = fixture.root.join("tampered");
    let database = second.join("legacy.sqlite");
    std::fs::set_permissions(
        &database,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .unwrap();
    {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(&database)
            .unwrap()
            .write_all(b"x")
            .unwrap();
    }
    std::fs::set_permissions(
        &database,
        std::os::unix::fs::PermissionsExt::from_mode(0o400),
    )
    .unwrap();
    assert!(import_legacy_profile(&second, &tampered).await.is_err());
    assert!(!tampered.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refuses_a_native_profile_presented_as_legacy() {
    let fixture = owned_fixture();
    let snapshot = snapshot_of(&fixture, "snapshot", CredentialFixture::PlainSqlite).await;
    let native = fixture.root.join("native");
    import_legacy_profile(&snapshot, &native).await.unwrap();
    let recaptured = fixture.root.join("recaptured");
    snapshot_legacy_profile(&native.join("dbunk.sqlite"), &recaptured)
        .await
        .unwrap();
    let error = import_legacy_profile(&recaptured, &fixture.root.join("again"))
        .await
        .unwrap_err();
    // A current native profile is newer than the baseline schema; either
    // refusal leaves nothing imported.
    assert!(
        error.contains("native-owned") || error.contains("newer than the supported baseline"),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn credential_metadata_fixtures_import_without_secret_stores() {
    for (credentials, mode) in [
        (
            CredentialFixture::EncryptedMetadata,
            Some("encrypted-sqlite"),
        ),
        (CredentialFixture::KeychainMetadata, Some("keychain")),
        (CredentialFixture::NotOnboarded, None),
    ] {
        let fixture = owned_fixture();
        let snapshot = snapshot_of(&fixture, "snapshot", credentials).await;
        let destination = fixture.root.join("native");
        let manifest = import_legacy_profile(&snapshot, &destination)
            .await
            .unwrap();
        let database = destination.join("dbunk.sqlite");
        let source = dump(&snapshot.join("legacy.sqlite")).await;
        let imported = dump(&database).await;
        assert_eq!(manifest.credential_mode.as_deref(), mode);
        match credentials {
            CredentialFixture::EncryptedMetadata => {
                // Opaque verifier and ciphertext are carried byte-for-byte.
                assert_eq!(imported["credentials"], source["credentials"]);
                assert_eq!(
                    imported["credential_verifier"],
                    source["credential_verifier"]
                );
                assert_eq!(manifest.credential_rows_imported, 2);
                assert_eq!(
                    setting(&database, "credentialStorageMode").await.as_deref(),
                    Some("encrypted-sqlite")
                );
            }
            _ => {
                assert_eq!(imported["credentials"], "[]");
                assert_eq!(imported["credential_verifier"], "[]");
                assert_eq!(setting(&database, "credentialStorageMode").await, None);
                assert_eq!(setting(&database, "onboardingCompleted").await, None);
                assert_eq!(
                    manifest.keychain_secrets_require_setup,
                    credentials == CredentialFixture::KeychainMetadata
                );
            }
        }
        assert_eq!(record_tables(&imported).len(), RECORD_TABLES.len());
    }
}

fn record_keychain() -> Arc<Mutex<Vec<(String, String)>>> {
    #[derive(Debug)]
    struct Recording(Arc<Mutex<Vec<(String, String)>>>);
    impl keyring::credential::CredentialBuilderApi for Recording {
        fn build(
            &self,
            _: Option<&str>,
            service: &str,
            account: &str,
        ) -> keyring::Result<Box<keyring::credential::Credential>> {
            self.0
                .lock()
                .unwrap()
                .push((service.into(), account.into()));
            Err(keyring::Error::NoStorageAccess(Box::new(
                std::io::Error::other("injected denial"),
            )))
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn persistence(&self) -> keyring::credential::CredentialPersistence {
            keyring::credential::CredentialPersistence::EntryOnly
        }
    }
    let calls = Arc::new(Mutex::new(Vec::new()));
    keyring::set_default_credential_builder(Box::new(Recording(calls.clone())));
    calls
}

/// The published destination is an ordinary general native profile. Runs in a
/// child process because a process may select only one native profile.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_profile_opens_through_the_ordinary_native_opener() {
    const CASE: &str = "imported_profile_opens_through_the_ordinary_native_opener";
    if let Some(path) = std::env::var_os(CHILD) {
        let calls = record_keychain();
        let backend = Backend::open_native_profile(Path::new(&path))
            .await
            .unwrap();
        assert_eq!(
            backend.native_profile_kind(),
            Some(NativeProfileKind::GeneralPostgres)
        );
        assert_eq!(
            backend.development_settings().await.unwrap().state,
            DevelopmentCredentialState::Ready
        );
        let connections = backend.development_connections().await.unwrap();
        assert_eq!(connections.len(), 5);
        for connection in &connections {
            let inactive = [
                corpus::MYSQL,
                corpus::REDIS,
                corpus::PG_BASTION,
                corpus::PG_UNKNOWN_FIELD,
            ]
            .contains(&connection.id.as_str());
            assert_eq!(
                connection.unsupported_reason.is_some(),
                inactive,
                "{}",
                connection.id
            );
        }
        let workspace = backend
            .load_development_workspace()
            .await
            .unwrap()
            .snapshot
            .unwrap();
        assert_eq!(workspace.documents.len(), 2);
        assert_eq!(workspace.documents[0].sql, corpus::QUERY_SQL);
        backend.shutdown().await.unwrap();
        assert!(calls.lock().unwrap().is_empty(), "no Keychain access");
        return;
    }
    let fixture = owned_fixture();
    let snapshot = snapshot_of(&fixture, "snapshot", CredentialFixture::PlainSqlite).await;
    let destination = fixture.root.join("native");
    import_legacy_profile(&snapshot, &destination)
        .await
        .unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("backend::legacy_import::tests::{CASE}"),
            "--nocapture",
        ])
        .env(CHILD, &destination)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}
