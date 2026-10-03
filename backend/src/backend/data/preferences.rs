use super::*;

impl Backend {
    pub async fn load_table_preferences(
        &self,
        document: &DataDocument,
        schema: String,
        table: String,
    ) -> Result<Option<TableGridPrefs>, DataError> {
        self.data_call(document, move |state, document, _admission| async move {
            // SQLite checks byte length before returning JSON. Legacy/future
            // records remain untouched when the native host cannot retain them.
            let row: Option<(i64, Option<String>)> = sqlx::query_as(
                "SELECT length(CAST(prefs AS BLOB)), CASE WHEN length(CAST(prefs AS BLOB)) <= 65536 THEN prefs END FROM table_grid_prefs WHERE connection_id = ? AND schema = ? AND table_name = ?",
            ).bind(&document.0.connection).bind(schema).bind(table).fetch_optional(&state.pool).await.map_err(|_| DataError::Storage("Table preference read failed".into()))?;
            row.map(|(bytes, encoded)| {
                if bytes > 65536 { return Err(DataError::Storage("Table preferences exceed 64 KiB; stored settings preserved".into())); }
                let encoded = encoded.ok_or_else(|| DataError::Storage("Stored table preferences are invalid".into()))?;
                let value = serde_json::from_str(&encoded).map_err(|_| DataError::Storage("Stored table preferences are invalid; settings preserved".into()))?;
                let prefs = TableGridPrefs(value);
                validate_native_prefs(&prefs)?;
                Ok(prefs)
            }).transpose()
        })
        .await
    }

    pub async fn save_table_preferences(
        &self,
        document: &DataDocument,
        schema: String,
        table: String,
        prefs: TableGridPrefs,
    ) -> Result<(), DataError> {
        validate_native_prefs(&prefs)?;
        self.data_call(document, move |state, document, _admission| async move {
            browse_service::save_grid_prefs(
                &state,
                SaveTableGridPrefsPayload {
                    connection_id: document.0.connection.clone(),
                    schema,
                    table,
                    prefs,
                },
            )
            .await
            .map_err(DataError::Storage)
        })
        .await
    }

    pub async fn load_virtual_key(
        &self,
        document: &DataDocument,
        schema: String,
        table: String,
    ) -> Result<Option<VirtualKey>, DataError> {
        self.data_call(document, move |state, document, _admission| async move {
            let row: Option<(i64, Option<String>)> = sqlx::query_as(
                "SELECT length(CAST(virtual_key AS BLOB)), CASE WHEN length(CAST(virtual_key AS BLOB)) <= 16384 THEN virtual_key END FROM virtual_keys WHERE connection_id = ? AND schema = ? AND table_name = ?",
            ).bind(&document.0.connection).bind(schema).bind(table).fetch_optional(&state.pool).await.map_err(|_| DataError::Storage("Virtual key read failed".into()))?;
            row.map(|(bytes, encoded)| {
                if bytes > 16384 { return Err(DataError::Storage("Virtual key exceeds 16 KiB; stored key preserved".into())); }
                let encoded = encoded.ok_or_else(|| DataError::Storage("Stored virtual key is invalid".into()))?;
                let key: VirtualKey = serde_json::from_str(&encoded).map_err(|_| DataError::Storage("Stored virtual key is invalid; key preserved".into()))?;
                crate::storage::validate_virtual_key(&key).map_err(|_| DataError::Storage("Stored virtual key is invalid or unsupported; key preserved".into()))?;
                if key.columns.len() > 64 { return Err(DataError::Storage("Virtual key exceeds 64 columns; stored key preserved".into())); }
                Ok(key)
            }).transpose()
        })
        .await
    }

    pub async fn save_virtual_key(
        &self,
        document: &DataDocument,
        schema: String,
        table: String,
        columns: Vec<String>,
    ) -> Result<(), DataError> {
        self.data_call(document, move |state, document, _admission| async move {
            mutation_service::save_virtual_key(
                &state,
                SaveVirtualKeyPayload {
                    connection_id: document.0.connection.clone(),
                    schema,
                    table,
                    columns,
                },
            )
            .await
            .map_err(DataError::Mutation)
        })
        .await
    }

    pub async fn clear_virtual_key(
        &self,
        document: &DataDocument,
        schema: String,
        table: String,
    ) -> Result<(), DataError> {
        self.data_call(document, move |state, document, _admission| async move {
            mutation_service::clear_virtual_key(
                &state,
                ClearVirtualKeyPayload {
                    connection_id: document.0.connection.clone(),
                    schema,
                    table,
                },
            )
            .await
            .map_err(DataError::Mutation)
        })
        .await
    }
}

fn validate_native_prefs(prefs: &TableGridPrefs) -> Result<(), DataError> {
    if prefs.0.get("version").and_then(serde_json::Value::as_u64) != Some(1) {
        return Err(DataError::Storage(
            "Unsupported table preferences; stored settings preserved".into(),
        ));
    }
    struct Bound(usize);
    impl std::io::Write for Bound {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len());
            if self.0 > 65536 {
                return Err(std::io::Error::other("preference budget"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Bound(0), prefs).map_err(|_| {
        DataError::Storage("Table preferences exceed 64 KiB; settings were not saved".into())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn native_preference_reads_refuse_large_future_and_corrupt_records_without_rewriting() {
        let directory = crate::backend::profile::directory();
        let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
            .await
            .unwrap();
        let connection = backend.fixture().id;
        let document = backend
            .open_data_document("preferences-test", "tab", &connection)
            .await
            .unwrap();
        for encoded in [
            format!("{{\"version\":1,\"payload\":\"{}\"}}", "x".repeat(65536)),
            "{\"version\":2,\"future\":true}".into(),
            "{broken".into(),
        ] {
            sqlx::query("INSERT OR REPLACE INTO table_grid_prefs (connection_id,schema,table_name,prefs,updated_at) VALUES (?,'public','rows',?,'test')").bind(&connection).bind(&encoded).execute(&backend.0.state.pool).await.unwrap();
            assert!(matches!(
                backend
                    .load_table_preferences(&document, "public".into(), "rows".into())
                    .await,
                Err(DataError::Storage(_))
            ));
            let retained: String = sqlx::query_scalar("SELECT prefs FROM table_grid_prefs WHERE connection_id = ? AND schema = 'public' AND table_name = 'rows'").bind(&connection).fetch_one(&backend.0.state.pool).await.unwrap();
            assert_eq!(retained, encoded);
        }
        assert!(backend
            .save_table_preferences(
                &document,
                "public".into(),
                "rows".into(),
                TableGridPrefs(serde_json::json!({"version":1,"escaped":"\0".repeat(12000)}))
            )
            .await
            .is_err());
        assert!(backend
            .load_virtual_key(&document, "public".into(), "rows".into())
            .await
            .unwrap()
            .is_none());
        for encoded in [
            format!("{{\"version\":1,\"columns\":[\"{}\"]}}", "x".repeat(16384)),
            "{\"version\":2,\"columns\":[\"id\"]}".into(),
            "{broken".into(),
            "{\"version\":1,\"columns\":[\"id\",\"id\"]}".into(),
        ] {
            sqlx::query("INSERT OR REPLACE INTO virtual_keys (connection_id,schema,table_name,virtual_key,updated_at) VALUES (?,'public','rows',?,'test')").bind(&connection).bind(&encoded).execute(&backend.0.state.pool).await.unwrap();
            assert!(backend
                .load_virtual_key(&document, "public".into(), "rows".into())
                .await
                .is_err());
            let retained: String = sqlx::query_scalar("SELECT virtual_key FROM virtual_keys WHERE connection_id = ? AND schema = 'public' AND table_name = 'rows'").bind(&connection).fetch_one(&backend.0.state.pool).await.unwrap();
            assert_eq!(retained, encoded);
        }
        let columns = vec!["actual.name".to_owned(), "a,b".to_owned()];
        backend
            .save_virtual_key(&document, "public".into(), "rows".into(), columns.clone())
            .await
            .unwrap();
        assert_eq!(
            backend
                .load_virtual_key(&document, "public".into(), "rows".into())
                .await
                .unwrap()
                .unwrap()
                .columns,
            columns
        );
        backend
            .clear_virtual_key(&document, "public".into(), "rows".into())
            .await
            .unwrap();
        assert!(backend
            .load_virtual_key(&document, "public".into(), "rows".into())
            .await
            .unwrap()
            .is_none());
        backend.close_data_document(&document).await.unwrap();
        backend.shutdown().await.unwrap();
    }
}
