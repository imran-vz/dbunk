//! Exact workspace-save barrier for app-owned seed attempts.
use super::*;
impl Workspace {
    pub(super) fn persist_seed(&mut self, request: u64, cx: &mut Context<Self>) {
        let refuse = if self.closing
            || self.cleanup_failed
            || self.busy
            || self.loading
            || self.dialog.is_some()
            || !self.restored
            || self.writer.is_none()
        {
            Some(
                "Workspace persistence is unavailable or busy; seed was not dispatched".to_string(),
            )
        } else {
            None
        };
        if let Some(error) = refuse {
            self.seeds
                .update(cx, |store, cx| store.saved(request, Err(error), cx));
            return;
        }
        self.changed(cx);
        if let SaveStatus::Failed(error) = &self.save_status {
            let error = error.to_string();
            self.seeds
                .update(cx, |store, cx| store.saved(request, Err(error), cx));
            return;
        }
        let writer = self.writer.as_ref().unwrap().clone();
        let revision = self.latest_revision;
        self.busy = true;
        let task = self
            .host
            .runtime
            .spawn(async move { writer.flush_revision(revision).await });
        self.action_task = Some(cx.spawn(async move |this, cx| {
            let result = task
                .await
                .map_err(|_| "Seed recovery save did not complete".to_string())
                .and_then(|result| result.map_err(|error| error.to_string()));
            this.update(cx, |this, cx| {
                this.busy = false;
                let result = if this.closing || this.cleanup_failed {
                    Err("Workspace is closing; seed was not dispatched".into())
                } else {
                    result
                };
                this.seeds
                    .update(cx, |store, cx| store.saved(request, result, cx));
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }
}
