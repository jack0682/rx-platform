use super::*;
use rx_application::host_rejoin as rejoin;
impl Worker {
    pub(super) async fn propose_rejoin_value(
        &self,
        identity: Identity,
        key: Id,
        input: rejoin::Prepare,
    ) -> Result<rejoin::ProposalView, WorkerError> {
        match self
            .runtime
            .request(Command::LookupHostRejoinProposal {
                identity: identity.clone(),
                key: key.clone(),
                input: input.clone(),
            })
            .await?
        {
            Reply::OptionalHostRejoinProposal(Some(v)) => return Ok(*v),
            Reply::OptionalHostRejoinProposal(None) => {}
            _ => return Err(WorkerError::InvalidRead),
        }
        let context = match self
            .runtime
            .request(Command::HostRejoinContext {
                identity: identity.clone(),
                host: input.host.clone(),
                origin: input.origin.clone(),
            })
            .await?
        {
            Reply::HostRejoinContext(c) => *c,
            _ => return Err(WorkerError::InvalidRead),
        };
        if !context.local_prerequisites_current {
            return Err(reject(Rejection::ContinuityUnproven));
        }
        if context.digest().map_err(|_| WorkerError::InvalidRead)? != input.expected_context
            || context.expected_cells() != input.expected_cells
        {
            return Err(reject(Rejection::StaleRevision));
        }
        let slot = self.slot(&context.host)?;
        let _held = slot.serial.try_lock().map_err(|_| WorkerError::Busy)?;
        let host_cells: BTreeSet<_> = context
            .cells
            .iter()
            .filter(|(_, v)| v.cell.configuration.hosts.contains(&context.host))
            .map(|(c, _)| c)
            .collect();
        if context.transport.as_ref() != Some(&slot.pin)
            || context.host != slot.configuration.host
            || host_cells.iter().any(|c| !slot.cells.contains(*c))
        {
            return Err(reject(Rejection::ContinuityUnproven));
        }
        let remote = self.connect_rejoin(&slot, &context).await?;
        let read = self.read_rejoin(&remote, &context).await?;
        match self
            .runtime
            .request(Command::ProposeHostRejoin {
                identity,
                key,
                input,
                read: Box::new(read),
            })
            .await?
        {
            Reply::HostRejoinProposal(v) => Ok(*v),
            _ => Err(WorkerError::InvalidRead),
        }
    }

    pub(super) async fn connect_rejoin(
        &self,
        slot: &Slot,
        context: &rejoin::Context,
    ) -> Result<RestrictedHost, WorkerError> {
        let installation = match self.runtime.request(Command::Installation).await? {
            Reply::Installation(v) => v,
            _ => return Err(WorkerError::InvalidRead),
        };
        if installation.id != context.installation
            || installation.store_generation != context.store_generation
            || installation.runtime_boot != context.runtime_boot
            || installation.clock_id != context.clock_id
        {
            return Err(reject(Rejection::StaleRevision));
        }
        RestrictedHost::connect_rejoin(
            slot,
            context,
            Hello {
                peer_id: Name::new(installation.id.as_str())
                    .map_err(|_| WorkerError::InvalidRead)?,
                boot_id: installation.runtime_boot,
                installation: installation.id,
                store_generation: installation.store_generation,
                release_digest: slot.configuration.release,
                clock_id: installation.clock_id,
            },
        )
        .await
    }
    pub(super) async fn read_rejoin(
        &self,
        remote: &RestrictedHost,
        context: &rejoin::Context,
    ) -> Result<data::VerifiedRead, WorkerError> {
        let started = self.now().await?;
        let configuration = remote.configuration().await?;
        let finished = self.now().await?;
        let mut cells = BTreeMap::new();
        for (cell, cut) in &context.cells {
            if !cut.cell.configuration.hosts.contains(&context.host) {
                continue;
            }
            let config = &context.cells[cell].cell.configuration;
            let sources = config
                .fact_specs
                .iter()
                .filter(|s| s.host == context.host)
                .map(|s| s.id.clone())
                .collect();
            let before = self.now().await?;
            let snapshot = remote.snapshot(cell, sources).await?;
            let after = self.now().await?;
            cells.insert(
                cell.clone(),
                data::SnapshotRead {
                    snapshot,
                    started: before,
                    finished: after,
                },
            );
        }
        data::VerifiedRead::new(
            remote.session()?,
            remote.pin().clone(),
            configuration,
            started,
            finished,
            cells,
        )
        .map_err(|_| WorkerError::InvalidRead)
    }
    async fn rejoin_view(
        &self,
        identity: Identity,
        id: Id,
    ) -> Result<rejoin::BindingView, WorkerError> {
        match self
            .runtime
            .request(Command::GetHostRejoinBinding { identity, id })
            .await?
        {
            Reply::HostRejoinBindingView(v) => Ok(*v),
            _ => Err(WorkerError::InvalidRead),
        }
    }
    pub(super) async fn approve_rejoin_value(
        &self,
        identity: Identity,
        key: Id,
        input: rejoin::Approve,
    ) -> Result<rejoin::BindingView, WorkerError> {
        let view = match self
            .runtime
            .request(Command::ApproveHostRejoin {
                identity: identity.clone(),
                key,
                input,
            })
            .await?
        {
            Reply::HostRejoinBindingView(v) => *v,
            _ => return Err(WorkerError::InvalidRead),
        };
        // Approval commits before I/O. An unavailable transport leaves a recoverable binding.
        self.progress_rejoin_value(identity, view.binding.id).await
    }
    pub(super) async fn progress_rejoin_value(
        &self,
        identity: Identity,
        id: Id,
    ) -> Result<rejoin::BindingView, WorkerError> {
        let mut view = self.rejoin_view(identity.clone(), id.clone()).await?;
        if !view.context_current
            || !matches!(
                view.binding.phase,
                data::Phase::Fencing | data::Phase::RecoveryOnly
            )
        {
            return Ok(view);
        }
        let context = &view.proposal.context;
        let slot = self.slot(&context.host)?;
        let _held = slot.serial.try_lock().map_err(|_| WorkerError::Busy)?;
        let remote = self.connect_rejoin(&slot, context).await?;
        let cells = view.binding.fences.keys().cloned().collect::<Vec<_>>();
        for cell in cells {
            if view.binding.fences[&cell].phase == data::FencePhase::Acknowledged {
                continue;
            }
            let read = self.read_rejoin(&remote, context).await?;
            let task = match self
                .runtime
                .request(Command::PlanHostRejoinFence {
                    id: id.clone(),
                    cell: cell.clone(),
                    read: Box::new(read),
                })
                .await?
            {
                Reply::HostRecoveryFence(v) => v,
                _ => return Err(WorkerError::InvalidRead),
            };
            let acknowledgment = remote.fence(&task).await?;
            view.binding = match self
                .runtime
                .request(Command::RecordHostRejoinFence {
                    id: id.clone(),
                    cell,
                    acknowledgment,
                })
                .await?
            {
                Reply::HostRejoinBinding(v) => *v,
                _ => return Err(WorkerError::InvalidRead),
            };
            if view.binding.phase == data::Phase::Attention {
                return self.rejoin_view(identity, id).await;
            }
        }
        let read = self.read_rejoin(&remote, context).await?;
        match self
            .runtime
            .request(Command::RefreshHostRejoin {
                id: id.clone(),
                read: Box::new(read),
            })
            .await?
        {
            Reply::HostRejoinBinding(_) => self.rejoin_view(identity, id).await,
            _ => Err(WorkerError::InvalidRead),
        }
    }
}

impl Worker {
    async fn rejoin_query_plan(
        &self,
        id: &Id,
        operation: &Id,
    ) -> Result<data::QueryPlan, WorkerError> {
        match self
            .runtime
            .request(Command::HostRejoinQueryPlan {
                id: id.clone(),
                operation: operation.clone(),
            })
            .await?
        {
            Reply::HostRecoveryQuery(v) => Ok(v),
            _ => Err(WorkerError::InvalidRead),
        }
    }
    async fn refresh_rejoin_for_query(
        &self,
        remote: &RestrictedHost,
        view: &rejoin::BindingView,
    ) -> Result<(), WorkerError> {
        let read = self.read_rejoin(remote, &view.proposal.context).await?;
        match self
            .runtime
            .request(Command::RefreshHostRejoin {
                id: view.binding.id.clone(),
                read: Box::new(read),
            })
            .await?
        {
            Reply::HostRejoinBinding(v) if v.phase == data::Phase::RecoveryOnly => Ok(()),
            _ => Err(reject(Rejection::ContinuityUnproven)),
        }
    }
    pub(super) async fn query_rejoin_value(
        &self,
        identity: Identity,
        id: Id,
        operation: Id,
    ) -> Result<rejoin::QueryObservation, WorkerError> {
        let view = self.rejoin_view(identity, id.clone()).await?;
        if !view.context_current || view.binding.phase != data::Phase::RecoveryOnly {
            return Err(reject(Rejection::Forbidden));
        }
        let slot = self.slot(&view.proposal.context.host)?;
        let _held = slot.serial.try_lock().map_err(|_| WorkerError::Busy)?;
        let remote = self.connect_rejoin(&slot, &view.proposal.context).await?;
        self.refresh_rejoin_for_query(&remote, &view).await?;
        self.rejoin_query_plan(&id, &operation).await?;
        let receipt = match remote.receipt(&operation).await {
            Ok(v) => Some(v),
            Err(e) if e.code() == tonic::Code::NotFound => None,
            Err(e) => return Err(transport::map_rpc(e)),
        };
        // Revalidate before lookup, which may collect original native evidence but may not replay.
        self.refresh_rejoin_for_query(&remote, &view).await?;
        let plan = self.rejoin_query_plan(&id, &operation).await?;
        let (batch, lookup) = if plan.lookup_allowed {
            match remote.lookup(&operation).await {
                Ok(v) => (Some(v), data::QueryLookup::PrefixObserved),
                Err(e) if e.code() == tonic::Code::Unimplemented => {
                    (None, data::QueryLookup::Unsupported)
                }
                Err(e)
                    if matches!(
                        e.code(),
                        tonic::Code::NotFound
                            | tonic::Code::Unavailable
                            | tonic::Code::DeadlineExceeded
                            | tonic::Code::Cancelled
                            | tonic::Code::ResourceExhausted
                    ) =>
                {
                    (None, data::QueryLookup::Unavailable)
                }
                Err(e) => return Err(transport::map_rpc(e)),
            }
        } else {
            (None, data::QueryLookup::NotNeeded)
        };
        let query = rejoin::VerifiedQuery::new(id, operation, receipt, batch, lookup)
            .map_err(|_| WorkerError::InvalidRead)?;
        match self
            .runtime
            .request(Command::RecordHostRejoinQuery {
                query: Box::new(query),
            })
            .await?
        {
            Reply::HostRejoinQueryObservation(v) => Ok(*v),
            _ => Err(WorkerError::InvalidRead),
        }
    }
}

impl Worker {
    pub(super) async fn settle_rejoin_value(
        &self,
        identity: Identity,
        key: Id,
        command: rejoin::ApproveSettlement,
    ) -> Result<rx_application::settlement::Authorization, WorkerError> {
        let view = self
            .rejoin_view(identity.clone(), command.reference.binding.clone())
            .await?;
        let slot = self.slot(&view.proposal.context.host)?;
        let _held = slot.serial.try_lock().map_err(|_| WorkerError::Busy)?;
        let authorization = match self
            .runtime
            .request(Command::ApproveRejoinSettlement {
                identity,
                key,
                command,
            })
            .await?
        {
            Reply::Settlement(v) => *v,
            _ => return Err(WorkerError::InvalidRead),
        };
        // Original-key recovery of a completed settlement performs no new Host I/O.
        if authorization.applied_at.is_some() {
            return Ok(authorization);
        }
        let remote = self.connect_rejoin(&slot, &view.proposal.context).await?;
        self.refresh_rejoin_for_query(&remote, &view).await?;
        let observations = remote.handover(&authorization.operation).await?;
        // Final fresh cut cannot borrow currency from the earlier query or approval.
        let read = self.read_rejoin(&remote, &view.proposal.context).await?;
        let proof = rejoin::VerifiedHandover::new(read, observations)
            .map_err(|_| WorkerError::InvalidRead)?;
        match self
            .runtime
            .request(Command::ApplyRejoinSettlement {
                id: authorization.id,
                proof: Box::new(proof),
            })
            .await?
        {
            Reply::Settlement(v) => Ok(*v),
            _ => Err(WorkerError::InvalidRead),
        }
    }
}
