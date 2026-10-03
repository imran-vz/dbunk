use super::*;
impl DdlExportView {
    pub(super) fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(capture) = &self.capture else {
            return;
        };
        let file_lease = match capture.file_lease() {
            Ok(lease) => lease,
            Err(error) => {
                self.message = Some(error.into());
                return;
            }
        };
        let artifact = capture.artifact.clone();
        let capture_lease = capture.lease.clone();
        let token = files::Cancellation::default();
        self.cancellation = Some(token.clone());
        self.file_busy = true;
        self.message=Some("Choose a new SQL file. The complete captured artifact will be saved; existing files are never overwritten.".into());
        let picker = cx.prompt_for_new_path(std::path::Path::new("/tmp"), Some("schema.sql"));
        let host = self.host.clone();
        let revision = self.revision;
        // The detached UI task keeps both leases even when this child is gone.
        // Host FileRuntime independently owns and joins the blocking worker.
        cx.spawn_in(window,async move|this,cx|{
            let result=match picker.await {
                Ok(Ok(Some(path))) if !token.is_cancelled()=>{
                    if path.as_os_str().len()>4096 {
                        Err("File destination exceeds 4096 bytes".to_owned())
                    } else {
                        let operation=move|cancel:&files::Cancellation|{
                            let prepared=prepare_sql(&artifact.sql,cancel)?;
                            // Return only the count: FileRuntime's settled entry
                            // need not retain the destination path or SQL bytes.
                            files::publish_new(prepared,&path,cancel).map(|published|published.bytes).map_err(|e|e.to_string())
                        };
                        match host.files.start(&host.runtime,token.clone(),operation){
                            Ok(job)=>job.finish().await.map(|bytes|format!("Saved all {} captured SQL bytes to the selected new file. Reconstruction omissions still apply.",*bytes)),
                            Err(error)=>Err(error.into()),
                        }
                    }
                },
                Ok(Ok(_))=>Ok("SQL file save cancelled before dispatch".into()),
                _=>Err("SQL file picker failed".into()),
            };
            drop(file_lease);drop(capture_lease);
            this.update_in(cx,|this,_,cx|{
                this.file_busy=false;this.cancellation=None;
                let message=result.unwrap_or_else(|error|format!("SQL file save refused: {}",bounded_status(&error)));
                this.message=Some(if this.revision==revision{message}else{format!("Earlier capture: {message}")});
                cx.notify();
            }).ok();
        }).detach();
    }
}

pub(super) fn prepare_sql(
    sql: &str,
    cancel: &files::Cancellation,
) -> Result<files::PreparedFile, String> {
    if cancel.is_cancelled() {
        return Err("SQL file save cancelled before preparation".into());
    }
    if sql.len() > dbunk_lib::backend::ddl_export::MAX_DDL_EXPORT_SQL_BYTES {
        return Err("SQL artifact exceeds 4 MiB".into());
    }
    let source = files::Source {
        completeness: files::Completeness::Complete,
        scope: files::Scope::CompleteResult,
        row_count: 0,
    };
    files::prepare_bytes(
        sql.as_bytes().to_vec(),
        files::Compression::None,
        source,
        cancel,
    )
    .map_err(|error| error.to_string())
}
