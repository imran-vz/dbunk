//! App-owned job observations survive comparison setup tabs. An unobserved
//! admission retains its exact request identity; polling never resubmits it.
use crate::{
    controller::{CompareCommand, CompareControls, CompareReceiver, CompareReply, Host},
    schema_compare_model::{JobCapture, Lease},
};
use dbunk_lib::backend::schema_comparisons::*;
use gpui::{Context, Task};
use std::{cell::Cell, rc::Rc, sync::Arc, time::Duration};

pub struct CompareStore {
    controls: Option<CompareControls>,
    replies: Option<CompareReceiver>,
    capture: Option<JobCapture>,
    pending: Option<u64>,
    pending_list: bool,
    pending_start: bool,
    serial: u64,
    admission: Option<SchemaComparisonStart>,
    current: bool,
    message: Option<String>,
    budget: Rc<Cell<usize>>,
    _lease: Option<Lease>,
    _observe: Task<()>,
}
impl CompareStore {
    pub fn new(host: Arc<Host>, budget: Rc<Cell<usize>>, cx: &mut Context<Self>) -> Self {
        let lease = Lease::new(budget.clone(), 128 * 1024);
        let (wake, awakened) = async_channel::bounded(1);
        let opened = match &lease {
            Ok(_) => host.open_comparisons(wake),
            Err(error) => Err(*error),
        };
        let (controls, replies, message) = match opened {
            Ok((controls, replies)) => (Some(controls), Some(replies), None),
            Err(error) => (None, None, Some(error.into())),
        };
        let observe = cx.spawn(async move |this, cx| {
            let mut poll = true;
            loop {
                let delay = this.update(cx, |this, cx| {
                    let changed = this.drain(cx);
                    if poll || changed {
                        this.refresh(cx);
                    }
                    if this.admission.is_some()
                        || this.capture.as_ref().is_some_and(JobCapture::has_active)
                    {
                        1
                    } else {
                        15
                    }
                });
                let Ok(delay) = delay else { break };
                let timer = cx.background_executor().timer(Duration::from_secs(delay));
                let wake = awakened.recv();
                futures_util::pin_mut!(timer, wake);
                poll = match futures_util::future::select(wake, timer).await {
                    futures_util::future::Either::Left((Ok(()), _)) => false,
                    futures_util::future::Either::Left((Err(_), _)) => break,
                    futures_util::future::Either::Right(_) => true,
                };
            }
        });
        Self {
            controls,
            replies,
            capture: None,
            pending: None,
            pending_list: false,
            pending_start: false,
            serial: 0,
            admission: None,
            current: false,
            message,
            budget,
            _lease: lease.ok(),
            _observe: observe,
        }
    }
    pub fn capture(&self) -> Option<&JobCapture> {
        self.capture.as_ref()
    }
    pub fn busy(&self) -> bool {
        self.controls.is_none() || (self.pending.is_some() && !self.pending_list)
    }
    pub fn uncertain(&self) -> bool {
        self.admission.is_some()
    }
    pub fn observation_current(&self) -> bool {
        self.current && self.controls.is_some()
    }
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }
    fn send(&mut self, make: impl FnOnce(u64) -> CompareCommand, cx: &mut Context<Self>) -> bool {
        let result = (|| {
            if self.pending.is_some() {
                return Err("A comparison observation or command is pending");
            }
            let id = self
                .serial
                .checked_add(1)
                .ok_or("Comparison request sequence exhausted")?;
            let command = make(id);
            let list = matches!(command, CompareCommand::List(_));
            let start = matches!(command, CompareCommand::Start(..));
            self.controls
                .as_ref()
                .ok_or("Comparison observer unavailable")?
                .send(command)?;
            self.serial = id;
            self.pending = Some(id);
            self.pending_list = list;
            self.pending_start = start;
            if !list {
                self.current = false;
                cx.notify();
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.message = Some(error.into());
            cx.notify();
            false
        } else {
            true
        }
    }
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.pending.is_none() && self.controls.is_some() {
            self.send(CompareCommand::List, cx);
        }
    }
    pub fn start(
        &mut self,
        source: Endpoint,
        target: Endpoint,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        if self.admission.is_some() {
            self.message = Some("Prior comparison admission is unobserved; refresh its exact request before starting another".into());
            cx.notify();
            return None;
        }
        let start = match SchemaComparisonStart::new(source, target) {
            Ok(start) => start,
            Err(error) => {
                self.message = Some(crate::schema_compare_model::failure_text(&error));
                cx.notify();
                return None;
            }
        };
        let id = start.request_id().to_owned();
        if !self.send(|request| CompareCommand::Start(request, start.clone()), cx) {
            return None;
        }
        self.admission = Some(start);
        self.message = None;
        cx.notify();
        Some(id)
    }
    pub fn cancel(&mut self, job: &str, cx: &mut Context<Self>) {
        if self.observation_current()
            && self
                .capture
                .as_ref()
                .and_then(|capture| capture.row(job))
                .is_some()
        {
            self.send(|id| CompareCommand::Cancel(id, job.to_owned()), cx);
        }
    }
    pub fn release(&mut self, job: &str, cx: &mut Context<Self>) {
        if self.observation_current()
            && self
                .capture
                .as_ref()
                .and_then(|capture| capture.row(job))
                .is_some()
        {
            self.send(|id| CompareCommand::Release(id, job.to_owned()), cx);
        }
    }
    fn drain(&mut self, cx: &mut Context<Self>) -> bool {
        let delivery = self
            .replies
            .as_ref()
            .and_then(|replies| replies.try_recv().ok());
        let Some(delivery) = delivery else {
            if self
                .replies
                .as_ref()
                .is_some_and(CompareReceiver::is_closed)
                && self.controls.take().is_some()
            {
                self.pending = None;
                self.current = false;
                self.message = Some(if self.admission.is_some() { "Comparison observer closed before admission was observed; request retained, never retried" } else { "Comparison observer closed; last observation retained" }.into());
                cx.notify();
            }
            return false;
        };
        if self.pending != Some(delivery.request) {
            return false;
        }
        self.pending = None;
        match delivery.result {
            Ok(CompareReply::List(list)) => {
                if let Some(start) = &self.admission
                    && list.jobs.iter().any(|job| {
                        job.request_id == start.request_id()
                            && &job.source == start.source()
                            && &job.target == start.target()
                    })
                {
                    self.admission = None;
                }
                let changed = self
                    .capture
                    .as_ref()
                    .is_none_or(|capture| !capture.matches(&list));
                if changed {
                    match JobCapture::new(list, self.budget.clone()) {
                        Ok(capture) => self.capture = Some(capture),
                        Err(error) => {
                            self.current = false;
                            self.message = Some(error.into());
                            cx.notify();
                            return false;
                        }
                    }
                }
                let notify = changed || !self.current;
                self.current = true;
                self.message = self.admission.as_ref().map(|_| "Comparison admission remains unobserved; absence is not proof it never started".into());
                if notify {
                    cx.notify();
                }
                false
            }
            Ok(CompareReply::Started(status)) => {
                if self.admission.as_ref().is_some_and(|start| {
                    status.request_id == start.request_id()
                        && &status.source == start.source()
                        && &status.target == start.target()
                }) {
                    self.admission = None;
                    self.message = None;
                } else {
                    self.message = Some(
                        "Comparison admission identity did not match; request retained".into(),
                    );
                }
                cx.notify();
                true
            }
            Ok(CompareReply::Cancelled | CompareReply::Released) => {
                self.message = None;
                cx.notify();
                true
            }
            Ok(CompareReply::Page(_)) => {
                self.current = false;
                self.message = Some("Comparison observer received an invalid reply".into());
                cx.notify();
                false
            }
            Err(error) => {
                // The facade's returned errors are pre-admission refusals. A
                // lost delivery instead leaves admission intact above.
                if self.pending_start {
                    self.admission = None;
                }
                self.current = false;
                self.message = Some(error.to_string());
                cx.notify();
                false
            }
        }
    }
}
