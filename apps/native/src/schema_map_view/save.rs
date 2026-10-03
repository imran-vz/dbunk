use super::*;
impl SchemaMapView {
    pub(super) fn save_image(&mut self, png: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(scene) = &self.scene else {
            return;
        };
        let mut prepared = match scene.svg(Viewport {
            camera: self.camera,
            ..self.viewport
        }) {
            Ok(svg) => svg,
            Err(error) => {
                self.report(error);
                return;
            }
        };
        let (plan, allowance) = if png {
            match crate::schema_map_png::PngPlan::new(&prepared).and_then(|plan| {
                let remaining = (128 * 1024 * 1024usize).saturating_sub(self.budget.get());
                let allowance =
                    remaining.min(plan.required_bytes().saturating_add(32 * 1024 * 1024));
                plan.with_working_limit(allowance)
                    .map(|plan| (Some(plan), allowance))
            }) {
                Ok(result) => result,
                Err(error) => {
                    self.report(error.to_string());
                    return;
                }
            }
        } else {
            (None, 256 * 1024)
        };
        let dimensions = plan.as_ref().map(|plan| plan.dimensions());
        let kind = if png { "PNG" } else { "SVG" };
        let file_lease = match Lease::new(self.budget.clone(), allowance) {
            Ok(lease) => lease,
            Err(error) => {
                self.report(error);
                return;
            }
        };
        let key = prepared.key;
        let token = files::Cancellation::default();
        self.file_cancel = Some(token.clone());
        self.file_busy = true;
        self.status = match dimensions {
            Some((width, height)) => format!(
                "Choose a new {width} × {height} PNG file for the captured viewport; existing files are never replaced"
            ),
            None => {
                "Choose a new SVG file for the captured viewport; existing files are never replaced"
                    .into()
            }
        };
        let picker = cx.prompt_for_new_path(
            std::path::Path::new("/tmp"),
            Some(if png {
                "schema-map.png"
            } else {
                "schema-map.svg"
            }),
        );
        let host = self.host.clone();
        // Keep PreparedSvg's lease in this detached UI task until the owned
        // blocking worker joins, even if its tab is closed during publication.
        cx.spawn_in(window, async move |this, cx| {
            let result = match picker.await {
                Ok(Ok(Some(path))) if !token.is_cancelled() => {
                    if path.as_os_str().len() > 4096 {
                        Err("File destination exceeds 4096 bytes".to_owned())
                    } else {
                        match prepared.take_bytes() {
                            Err(error) => Err(error.into()),
                            Ok(bytes) => {
                                let operation = move |cancel: &files::Cancellation| {
                                    let bytes = match plan {
                                        Some(plan) => {
                                            plan.encode(bytes, cancel).map_err(|e| e.to_string())?
                                        }
                                        None => bytes,
                                    };
                                    let source = files::Source {
                                        completeness: files::Completeness::Complete,
                                        scope: files::Scope::CompleteResult,
                                        row_count: 0,
                                    };
                                    let file = files::prepare_bytes(
                                        bytes,
                                        files::Compression::None,
                                        source,
                                        cancel,
                                    )
                                    .map_err(|e| e.to_string())?;
                                    files::publish_new(file, &path, cancel)
                                        .map(|result| result.bytes)
                                        .map_err(|e| e.to_string())
                                };
                                match host.files.start(&host.runtime, token.clone(), operation) {
                                    Ok(job) => job.finish().await.map(|n| {
                                        format!("Saved {n} {kind} bytes for the captured viewport")
                                    }),
                                    Err(error) => Err(error.into()),
                                }
                            }
                        }
                    }
                }
                Ok(Ok(_)) => Ok(format!("{kind} save cancelled before dispatch")),
                _ => Err(format!("{kind} file picker failed")),
            };
            drop(prepared);
            drop(file_lease);
            this.update_in(cx, |this, _, cx| {
                this.file_busy = false;
                this.file_cancel = None;
                let message =
                    result.unwrap_or_else(|error| format!("{kind} save refused: {error}"));
                if this.scene.as_ref().is_some_and(|scene| scene.key() == key) {
                    this.report(message);
                } else {
                    this.report(format!("Earlier captured viewport: {message}"));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
