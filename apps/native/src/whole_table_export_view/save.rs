use super::*;
impl WholeTableExportView {
    pub(super) fn save(
        &mut self,
        options: ExportOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let work = match Lease::new(self.budget.clone(), model::FILE_WORK_BYTES) {
            Ok(lease) => lease,
            Err(error) => {
                self.status = error.into();
                return;
            }
        };
        let Some(capture) = self.capture.clone() else {
            return;
        };
        let name = format!(
            "table.{}{}",
            model::extension(options.format),
            if options.compression == ExportCompression::Gzip {
                ".gz"
            } else {
                ""
            }
        );
        let picker = cx.prompt_for_new_path(std::path::Path::new("/tmp"), Some(&name));
        let token = files::Cancellation::default();
        self.cancellation = Some(token.clone());
        self.saving = true;
        self.null
            .update(cx, |field, cx| field.set_readonly(true, cx));
        let host = self.host.clone();
        cx.spawn_in(window,async move|this,cx|{
            let result=match picker.await{
                Ok(Ok(Some(path))) if !token.is_cancelled()=>{
                    let source=capture.source.clone();
                    let operation=move|cancel:&files::Cancellation|{
                        let prepared=model::prepare(source.data(),&options,cancel)?;
                        files::publish_new(prepared,&path,cancel).map_err(|e|e.to_string())
                    };
                    match host.files.start(&host.runtime,token.clone(),operation){Ok(job)=>job.finish().await.map(|published|format!("Saved {} bytes, {} complete captured rows. Capture interval is unchanged; this does not reread the table.",published.bytes,published.source.row_count)),Err(error)=>Err(error.to_owned())}
                }
                Ok(Ok(_))=>Ok("Export cancelled".into()),_=>Err("Export picker failed".into()),
            };
            // Detached waiter keeps both payload and working reservations until
            // the host-owned blocking file worker has really joined.
            drop(capture);drop(work);
            this.update_in(cx,|this,_,cx|{this.saving=false;this.cancellation=None;this.null.update(cx,|field,cx|field.set_readonly(!this.editable||this.reading,cx));this.status=result.unwrap_or_else(|error|format!("Export refused: {error}"));cx.notify();}).ok();
        }).detach();
    }
}
