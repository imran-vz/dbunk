use super::*;

/// A review owns the exact analysis and plan shown by the service preview.
/// Discard it when the staged changes or selected document change.
pub struct MutationReview {
    document: DataDocument,
    analysis_id: u64,
    plan: MutationPlan,
    preview: PreviewResult,
}

impl MutationReview {
    pub fn belongs_to(&self, document: &DataDocument) -> bool {
        std::sync::Arc::ptr_eq(&self.document.0, &document.0)
    }
    pub fn preview(&self) -> &PreviewResult {
        &self.preview
    }

    /// Encoded retained intent for native delivery admission, without copying
    /// SQL or values into a temporary serialization buffer.
    pub fn retained_bytes(&self) -> usize {
        encoded_bytes(&(
            &self.plan,
            &self.preview,
            self.analysis_id,
            &self.document.0.window,
            &self.document.0.connection,
            &self.document.0.manager_tab,
        ))
    }
}

pub enum MutationSubmission {
    Applied(ApplyResult),
    NeedsConfirmation(Box<MutationConfirmation>),
}

/// Only a policy refusal can create this single-use acknowledgement. Neither
/// routing, plan nor bound values can be changed after the refusal.
pub struct MutationConfirmation {
    review: MutationReview,
    request_id: u64,
    statements: Vec<StatementClassSummary>,
}

impl MutationConfirmation {
    pub fn belongs_to(&self, document: &DataDocument) -> bool {
        self.review.belongs_to(document)
    }
    pub fn preview(&self) -> &PreviewResult {
        self.review.preview()
    }

    pub fn statements(&self) -> &[StatementClassSummary] {
        &self.statements
    }

    pub fn retained_bytes(&self) -> usize {
        self.review
            .retained_bytes()
            .saturating_add(encoded_bytes(&(self.request_id, &self.statements)))
    }
}

fn encoded_bytes(value: &impl serde::Serialize) -> usize {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .ok_or_else(|| std::io::Error::other("encoded size overflow"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    if serde_json::to_writer(&mut count, value).is_ok() {
        count.0
    } else {
        usize::MAX
    }
}

impl Backend {
    pub async fn review_mutations(
        &self,
        document: &DataDocument,
        analysis_id: u64,
        plan: MutationPlan,
    ) -> Result<MutationReview, DataError> {
        self.data_call(document, move |state, document, _admission| async move {
            let preview = mutation_service::preview(
                &state,
                PreviewResultMutationsPayload {
                    connection_id: document.0.connection.clone(),
                    tab_id: document.0.manager_tab.clone(),
                    analysis_id,
                    plan: plan.clone(),
                },
            )
            .await
            .map_err(DataError::Mutation)?;
            Ok(MutationReview {
                document,
                analysis_id,
                plan,
                preview,
            })
        })
        .await
    }

    pub async fn apply_review(
        &self,
        review: MutationReview,
        request_id: u64,
    ) -> Result<MutationSubmission, DataError> {
        self.apply_review_inner(review, request_id, false).await
    }

    pub async fn confirm_mutations(
        &self,
        confirmation: MutationConfirmation,
    ) -> Result<MutationSubmission, DataError> {
        self.apply_review_inner(confirmation.review, confirmation.request_id, true)
            .await
    }

    async fn apply_review_inner(
        &self,
        review: MutationReview,
        request_id: u64,
        confirmed: bool,
    ) -> Result<MutationSubmission, DataError> {
        let document = review.document.clone();
        self.data_call(&document, move |state, document, admission| async move {
            let payload = ApplyResultMutationsPayload {
                connection_id: document.0.connection.clone(),
                tab_id: document.0.manager_tab.clone(),
                request_id,
                analysis_id: review.analysis_id,
                plan: review.plan.clone(),
                confirmed,
            };
            let result = match mutation_service::start_apply(&state, payload).await {
                Ok(pending) => {
                    drop(admission);
                    pending.await
                }
                Err(error) => Err(error),
            };
            match result {
                Ok(applied) => Ok(MutationSubmission::Applied(applied)),
                Err(ResultMutationError::PolicyNeedsConfirmation { statements }) => Ok(
                    MutationSubmission::NeedsConfirmation(Box::new(MutationConfirmation {
                        review,
                        request_id,
                        statements,
                    })),
                ),
                Err(error) => Err(DataError::Mutation(error)),
            }
        })
        .await
    }
}
