use super::*;

impl Worker {
    async fn query_plan(&self, id: &Id, operation: &Id) -> Result<data::QueryPlan, WorkerError> {
        match self
            .runtime
            .request(Command::HostRecoveryQuery {
                id: id.clone(),
                operation: operation.clone(),
            })
            .await?
        {
            Reply::HostRecoveryQuery(value) => Ok(value),
            _ => Err(WorkerError::InvalidRead),
        }
    }
    async fn refresh_for_query(
        &self,
        remote: &RestrictedHost,
        binding: &data::Binding,
    ) -> Result<(), WorkerError> {
        let read = self.read(remote, &binding.context).await?;
        match self
            .runtime
            .request(Command::RefreshHostRecovery {
                id: binding.id.clone(),
                read: Box::new(read),
            })
            .await?
        {
            Reply::HostRecoveryBinding(value) if value.phase == data::Phase::RecoveryOnly => Ok(()),
            Reply::HostRecoveryBinding(_) => Err(reject(Rejection::ContinuityUnproven)),
            _ => Err(WorkerError::InvalidRead),
        }
    }
    pub(super) async fn query_operation(
        &self,
        identity: Identity,
        id: Id,
        operation: Id,
    ) -> Result<data::QueryResult, WorkerError> {
        let view = self.view(identity, id.clone()).await?;
        if view.binding.phase != data::Phase::RecoveryOnly {
            return Err(reject(Rejection::Forbidden));
        }
        let slot = self.slot(&view.binding.context.host)?;
        let _held = slot.serial.try_lock().map_err(|_| WorkerError::Busy)?;
        let remote = self.connect(&slot, &view.binding.context).await?;
        self.refresh_for_query(&remote, &view.binding).await?;
        let plan = self.query_plan(&id, &operation).await?;
        let receipt = match remote.receipt(&operation).await {
            Ok(receipt) => {
                if receipt.operation != operation
                    || receipt.digest != plan.operation.intent_digest
                    || receipt.journal != plan.operation.host_journal
                    || receipt.invocation.as_ref().is_some_and(|received| {
                        plan.operation
                            .invocation
                            .as_ref()
                            .is_some_and(|known| known != received)
                    })
                    || (receipt.invocation.is_none()
                        && receipt.state != rx_application::ReceiptState::VoidedBeforeSend)
                {
                    return Err(WorkerError::InvalidRead);
                }
                let message = if receipt.state == rx_application::ReceiptState::Prepared {
                    plan.operation.messages.iter().find(|id| *id == &operation)
                } else {
                    plan.operation
                        .messages
                        .iter()
                        .find(|id| *id != &operation)
                        .or_else(|| plan.operation.messages.first())
                }
                .ok_or(WorkerError::InvalidRead)?;
                match self
                    .runtime
                    .request(Command::RecordHostRecoveryReceipt {
                        id: id.clone(),
                        message: message.clone(),
                        receipt: receipt.clone(),
                    })
                    .await?
                {
                    Reply::Work(_) => {}
                    _ => return Err(WorkerError::InvalidRead),
                }
                Some(receipt)
            }
            Err(error) if error.code() == tonic::Code::NotFound => None,
            Err(error) => return Err(transport::map_rpc(error)),
        };
        // A receipt may reveal the original invocation. Revalidate current authority/read
        // before the next RPC instead of borrowing permission from the earlier query.
        self.refresh_for_query(&remote, &view.binding).await?;
        let plan = self.query_plan(&id, &operation).await?;
        let mut result = data::QueryResult {
            binding: id,
            operation: operation.clone(),
            receipt,
            lookup: data::QueryLookup::NotNeeded,
            evidence: vec![],
            evidence_complete: false,
            publication_required: false,
            operation_authorized: false,
        };
        if !plan.lookup_allowed {
            return Ok(result);
        }
        match remote.lookup(&operation).await {
            Ok(batch) => {
                if batch.journal != view.binding.context.producer.journal
                    || batch.first.0 == 0
                    || batch.records.len() > 128
                {
                    return Err(WorkerError::InvalidRead);
                }
                let invocation = result
                    .receipt
                    .as_ref()
                    .and_then(|r| r.invocation.as_ref())
                    .or(plan.operation.invocation.as_ref())
                    .or_else(|| {
                        plan.original_receipt
                            .as_ref()
                            .and_then(|r| r.invocation.as_ref())
                    })
                    .ok_or(WorkerError::InvalidRead)?;
                let now = self.now().await?;
                for evidence in batch
                    .records
                    .into_iter()
                    .filter(|e| e.operation == operation)
                {
                    if &evidence.invocation != invocation
                        || evidence.profile_digest != plan.operation.profile_digest
                        || now.age_ns(&evidence.captured_at).is_none()
                    {
                        return Err(WorkerError::InvalidRead);
                    }
                    result.evidence.push(evidence);
                }
                // This filtered diagnostic view is NOT ingested as a new EvidenceBatch.
                // The original Host publisher conveys the full ordered source journal.
                result.lookup = data::QueryLookup::PrefixObserved;
                result.publication_required = true;
            }
            Err(error) if error.code() == tonic::Code::Unimplemented => {
                result.lookup = data::QueryLookup::Unsupported
            }
            Err(error)
                if matches!(
                    error.code(),
                    tonic::Code::NotFound
                        | tonic::Code::Unavailable
                        | tonic::Code::DeadlineExceeded
                        | tonic::Code::Cancelled
                        | tonic::Code::ResourceExhausted
                ) =>
            {
                result.lookup = data::QueryLookup::Unavailable;
                // A lost lookup reply can still have appended evidence on H.
                result.publication_required = true;
            }
            Err(error) => return Err(transport::map_rpc(error)),
        }
        Ok(result)
    }
}
