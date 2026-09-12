//! Durable query plans use only Host reads and existing P evidence/handover transactions.
use super::*;
use rx_domain::operation::{Disposition, Integrity, Outcome, Phase};
async fn bounded<T>(
    future: impl std::future::Future<Output = Result<T, Status>>,
) -> Result<T, Error> {
    tokio::time::timeout(Duration::from_secs(2), future)
        .await
        .map_err(|_| Status::deadline_exceeded("Host query deadline"))?
        .map_err(Error::Rpc)
}
impl Dispatcher {
    pub(super) async fn query_tick(&mut self) -> Result<(), Error> {
        let Reply::ReconciliationRequests(plans) = self
            .call(Command::PendingReconciliations {
                identity: self.identity.clone(),
                after: self.plan_after.clone(),
                limit: 4,
            })
            .await?
        else {
            return Err(Error::Protocol("pending query plans"));
        };
        self.plan_after = plans.last().map(|p| p.operation.clone());
        for plan in plans {
            if self.delayed(&plan.id) {
                continue;
            }
            match self.query(&plan).await {
                Ok(true) => {
                    self.retries.remove(&plan.id);
                }
                Ok(false) => self.retry(&plan.id),
                Err(error) if permanent_issue(&error).is_some() => {
                    self.query_state(
                        &plan,
                        ReconciliationState::Attention,
                        permanent_issue(&error),
                    )
                    .await?;
                    self.retries.remove(&plan.id);
                    self.report_error(&error);
                }
                Err(error) => {
                    self.report_error(&error);
                    let _ = self
                        .query_state(
                            &plan,
                            ReconciliationState::Pending,
                            Some(ReconciliationIssue::SourceUnavailable),
                        )
                        .await;
                    self.retry(&plan.id);
                }
            }
        }
        Ok(())
    }
    async fn query_state(
        &self,
        plan: &ReconciliationRequest,
        state: ReconciliationState,
        issue: Option<ReconciliationIssue>,
    ) -> Result<(), Error> {
        self.call(Command::UpdateReconciliation {
            identity: self.identity.clone(),
            operation: plan.operation.clone(),
            request: plan.id.clone(),
            state,
            issue,
        })
        .await?;
        Ok(())
    }
    async fn query_plan(
        &self,
        plan: &ReconciliationRequest,
    ) -> Result<Box<ReconciliationPlan>, Error> {
        let Reply::ReconciliationPlan(value) = self
            .call(Command::PlanReconciliation {
                identity: self.identity.clone(),
                operation: plan.operation.clone(),
                request: plan.id.clone(),
            })
            .await?
        else {
            return Err(Error::Protocol("query plan reply"));
        };
        Ok(value)
    }
    async fn query(&self, request: &ReconciliationRequest) -> Result<bool, Error> {
        let mut plan = self.query_plan(request).await?;
        if plan.work.operation.disposition() == Disposition::Released {
            self.query_state(request, ReconciliationState::Complete, None)
                .await?;
            return Ok(true);
        }
        if plan.work.operation.outcome() == Outcome::None {
            let Some(message) = &plan.receipt_message else {
                self.query_state(
                    request,
                    ReconciliationState::Pending,
                    Some(ReconciliationIssue::WaitingDispatch),
                )
                .await?;
                return Ok(false);
            };
            let receipt = bounded(self.client.receipt(&request.operation)).await?;
            self.call(Command::RecordReceipt {
                identity: self.identity.clone(),
                message: message.clone(),
                receipt,
            })
            .await?;
            let batch = bounded(self.client.reconcile(&request.operation)).await?;
            if !batch.records.is_empty() {
                self.call(Command::IngestEvidence {
                    identity: self.identity.clone(),
                    batch,
                })
                .await?;
            }
            plan = self.query_plan(request).await?;
        }
        if plan.work.operation.phase() != Phase::Settled {
            self.query_state(
                request,
                ReconciliationState::Pending,
                Some(ReconciliationIssue::WaitingResult),
            )
            .await?;
            return Ok(false);
        }
        if plan.work.operation.integrity() != Integrity::Valid
            || plan.work.operation.outcome() == Outcome::Unresolved
        {
            self.query_state(
                request,
                ReconciliationState::Attention,
                Some(ReconciliationIssue::ContinuityUnproven),
            )
            .await?;
            return Ok(true);
        }
        let observations = bounded(self.client.handover(&request.operation)).await?;
        self.call(Command::RecordReconciliationObservations {
            identity: self.identity.clone(),
            operation: request.operation.clone(),
            request: request.id.clone(),
            observations: observations.clone(),
        })
        .await?;
        let key = Id::new(uuid::Uuid::new_v4().to_string())
            .map_err(|_| Error::Protocol("handover request key"))?;
        match self
            .call(Command::ReleaseResources {
                identity: self.identity.clone(),
                key,
                command: ReleaseResources {
                    operation: request.operation.clone(),
                    expected_operation: plan.work.operation.revision(),
                    expected_cell: plan.cell_revision,
                    observations,
                },
            })
            .await
        {
            Ok(Reply::Work(work)) if work.operation.disposition() == Disposition::Released => {
                self.query_state(request, ReconciliationState::Complete, None)
                    .await?;
                self.report
                    .send_modify(|r| r.reconciled = r.reconciled.saturating_add(1));
                Ok(true)
            }
            Ok(_) => Err(Error::Protocol("handover result")),
            Err(Error::Writer(WriterError::Rejected(StoreError::Rejected(
                rx_domain::fault::Rejection::ConditionUnknown
                | rx_domain::fault::Rejection::ConditionFailed
                | rx_domain::fault::Rejection::StaleRevision,
            )))) => {
                self.query_state(
                    request,
                    ReconciliationState::Pending,
                    Some(ReconciliationIssue::WaitingHandover),
                )
                .await?;
                Ok(false)
            }
            Err(Error::Writer(WriterError::Rejected(StoreError::Rejected(_)))) => {
                self.query_state(
                    request,
                    ReconciliationState::Attention,
                    Some(ReconciliationIssue::ContinuityUnproven),
                )
                .await?;
                Ok(true)
            }
            Err(error) => Err(error),
        }
    }
}

// Unavailable reads may recover. Invalid identity, authority or proof requires attention.
fn permanent_issue(error: &Error) -> Option<ReconciliationIssue> {
    use rx_domain::fault::Rejection;
    match error {
        Error::Rpc(status) => match status.code() {
            Code::Unimplemented | Code::InvalidArgument => Some(ReconciliationIssue::Unsupported),
            Code::PermissionDenied | Code::Unauthenticated => {
                Some(ReconciliationIssue::PermissionChanged)
            }
            Code::DataLoss | Code::FailedPrecondition => {
                Some(ReconciliationIssue::ContinuityUnproven)
            }
            _ => None,
        },
        Error::Protocol(_)
        | Error::Writer(WriterError::Rejected(
            StoreError::Integrity(_) | StoreError::KeyConflict,
        )) => Some(ReconciliationIssue::ContinuityUnproven),
        Error::Writer(WriterError::Rejected(StoreError::Rejected(reason))) => match reason {
            Rejection::Forbidden | Rejection::Unauthenticated => {
                Some(ReconciliationIssue::PermissionChanged)
            }
            Rejection::ContinuityUnproven => Some(ReconciliationIssue::ContinuityUnproven),
            Rejection::CapabilityMissing | Rejection::InvalidInput => {
                Some(ReconciliationIssue::Unsupported)
            }
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_read_retries_but_broken_proof_and_revoked_authority_do_not() {
        assert!(permanent_issue(&Error::Rpc(Status::unavailable("offline"))).is_none());
        assert_eq!(
            permanent_issue(&Error::Rpc(Status::data_loss("journal changed"))),
            Some(ReconciliationIssue::ContinuityUnproven)
        );
        assert_eq!(
            permanent_issue(&Error::Rpc(Status::permission_denied("revoked"))),
            Some(ReconciliationIssue::PermissionChanged)
        );
        assert_eq!(
            permanent_issue(&Error::Writer(WriterError::Rejected(
                StoreError::Integrity("observation changed".into())
            ))),
            Some(ReconciliationIssue::ContinuityUnproven)
        );
    }
}
