//! View-owned reader applies only its latest navigation generation and reserves
//! native retained capacity before consuming an opaque backend response.
use crate::{
    controller::{CompareCommand, CompareControls, CompareReceiver, CompareReply, Host},
    schema_compare_model::{Dispatch, Intent, PageLease, ReaderState},
};
use gpui::Context;
use std::{cell::Cell, rc::Rc, sync::Arc};

pub struct CompareReader {
    state: Option<ReaderState>,
    controls: Option<CompareControls>,
    replies: Option<CompareReceiver>,
    budget: Rc<Cell<usize>>,
    message: Option<String>,
}
impl CompareReader {
    pub fn new(
        host: Arc<Host>,
        id: String,
        wake: async_channel::Sender<()>,
        budget: Rc<Cell<usize>>,
    ) -> Self {
        let state = ReaderState::new(budget.clone());
        let opened = match &state {
            Ok(_) => host.open_comparison_reader(id, wake),
            Err(error) => Err(*error),
        };
        let (controls, replies, message) = match opened {
            Ok((controls, replies)) => (Some(controls), Some(replies), None),
            Err(error) => (None, None, Some(error.into())),
        };
        Self {
            state: state.ok(),
            controls,
            replies,
            budget,
            message,
        }
    }
    pub fn state(&self) -> Option<&ReaderState> {
        self.state.as_ref()
    }
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }
    pub fn busy(&self) -> bool {
        self.controls.is_none() || self.state.as_ref().is_some_and(ReaderState::busy)
    }
    pub fn has_pending(&self) -> bool {
        self.replies.as_ref().is_some_and(|replies| {
            !replies.is_empty() || (replies.is_closed() && self.controls.is_some())
        })
    }
    pub fn enqueue(&mut self, intent: Intent, cx: &mut Context<Self>) {
        let next = self
            .state
            .as_mut()
            .ok_or("Comparison reader unavailable")
            .and_then(|state| state.enqueue(intent));
        self.advance(next, cx);
    }
    fn advance(&mut self, next: Result<Option<Dispatch>, &'static str>, cx: &mut Context<Self>) {
        match next {
            Ok(Some(dispatch)) => {
                let token = dispatch.token;
                let sent = self
                    .controls
                    .as_ref()
                    .ok_or("Comparison reader unavailable")
                    .and_then(|controls| controls.send(CompareCommand::Read(dispatch)));
                if let Err(error) = sent {
                    self.message = Some(error.into());
                    if let Some(state) = &mut self.state {
                        let _ = state.fail(token, false);
                    }
                } else {
                    self.message = None;
                }
            }
            Ok(None) => {}
            Err(error) => self.message = Some(error.into()),
        }
        cx.notify();
    }
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = &mut self.state {
            state.close();
        }
        self.message = None;
        cx.notify();
    }
    pub fn drain_one(&mut self, cx: &mut Context<Self>) -> bool {
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
                if let Some(state) = &mut self.state {
                    state.close();
                }
                self.message = Some("Comparison reader closed; reopen the Tool tab".into());
                cx.notify();
                return true;
            }
            return false;
        };
        let Some(token) = delivery.token else {
            return true;
        };
        let Some(state) = &mut self.state else {
            return true;
        };
        if !state.accepts(&token) {
            drop(delivery);
            let next = state.fail(token, false);
            self.advance(next, cx);
            return true;
        }
        let accepted = match delivery.result {
            Ok(CompareReply::Page(response)) => {
                match PageLease::new(response.data(), self.budget.clone()) {
                    Ok(lease) => match response.into_page() {
                        Ok(page) => state
                            .accept(token, page, lease)
                            .map_err(|error| (error.to_owned(), false)),
                        Err(error) => {
                            Err((crate::schema_compare_model::failure_text(&error), true))
                        }
                    },
                    Err(error) => Err((error.into(), false)),
                }
            }
            Err(error) => {
                let unavailable = matches!(
                    error,
                    crate::controller::CompareFailure::Backend(
                        dbunk_lib::backend::schema_comparisons::CompareError::Unavailable
                            | dbunk_lib::backend::schema_comparisons::CompareError::CaptureChanged
                    )
                );
                Err((error.to_string(), unavailable))
            }
            _ => Err(("Comparison reader received an invalid reply".into(), false)),
        };
        match accepted {
            Ok(next) => self.advance(Ok(next), cx),
            Err((error, unavailable)) => {
                let next = state.fail(token, unavailable);
                self.advance(next, cx);
                self.message = Some(error);
                cx.notify();
            }
        }
        true
    }
}
