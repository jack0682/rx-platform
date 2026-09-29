use super::*;
use rx_application::host_rejoin as rejoin;
impl Worker {
    async fn rebind_view(
        &self,
        identity: Identity,
        id: Id,
    ) -> Result<rejoin::RebindView, WorkerError> {
        match self
            .runtime
            .request(Command::GetHostRebind { identity, id })
            .await?
        {
            Reply::HostRebindView(v) => Ok(*v),
            _ => Err(WorkerError::InvalidRead),
        }
    }
    pub(super) async fn rebind_value(
        &self,
        identity: Identity,
        key: Id,
        input: rejoin::ApproveRebind,
    ) -> Result<rejoin::RebindView, WorkerError> {
        let view = match self
            .runtime
            .request(Command::ApproveHostRebind {
                identity: identity.clone(),
                key,
                input,
            })
            .await?
        {
            Reply::HostRebindView(v) => *v,
            _ => return Err(WorkerError::InvalidRead),
        };
        self.progress_rebind_value(identity, view.rebind.id).await
    }
    pub(super) async fn progress_rebind_value(
        &self,
        identity: Identity,
        id: Id,
    ) -> Result<rejoin::RebindView, WorkerError> {
        let mut view = self.rebind_view(identity.clone(), id.clone()).await?;
        if !view.current
            || matches!(
                view.rebind.phase,
                rejoin::RebindPhase::Bound | rejoin::RebindPhase::Attention
            )
        {
            return Ok(view);
        }
        let context = &view.proposal.context;
        let slot = self.slot(&context.host)?;
        let _held = slot.serial.try_lock().map_err(|_| WorkerError::Busy)?;
        let remote = transport::RebindingHost::new(self.connect_rejoin(&slot, context).await?);
        let read = self.read_rejoin(remote.reader(), context).await?;
        let ttls = slot
            .ttls
            .iter()
            .filter(|(c, _)| context.cells.contains_key(*c))
            .map(|(c, v)| (c.clone(), *v))
            .collect();
        view.rebind = match self
            .runtime
            .request(Command::PrepareHostRebind {
                id: id.clone(),
                read: Box::new(read),
                ttls,
            })
            .await?
        {
            Reply::HostRebind(v) => *v,
            _ => return Err(WorkerError::InvalidRead),
        };
        let cells = view.rebind.steps.keys().cloned().collect::<Vec<_>>();
        for cell in cells {
            if view.rebind.steps[&cell].phase == rejoin::GrantPhase::Acknowledged {
                continue;
            }
            let read = self.read_rejoin(remote.reader(), context).await?;
            let plan = match self
                .runtime
                .request(Command::PlanHostRebindGrant {
                    id: id.clone(),
                    cell: cell.clone(),
                    read: Box::new(read),
                })
                .await?
            {
                Reply::HostLink(v) => *v,
                _ => return Err(WorkerError::InvalidRead),
            };
            let (grant, host_boot) = remote.grant(&plan).await?;
            let commit = rx_application::host_link::Commit {
                plan: plan.id,
                fence_receipt: view.rebind.steps[&cell].fence.clone(),
                grant,
                grant_sent_at: plan.prepared_at,
            };
            view.rebind = match self
                .runtime
                .request(Command::RecordHostRebindGrant {
                    id: id.clone(),
                    cell,
                    commit: Box::new(commit),
                    host_boot,
                })
                .await?
            {
                Reply::HostRebind(v) => *v,
                _ => return Err(WorkerError::InvalidRead),
            };
            if view.rebind.phase == rejoin::RebindPhase::Attention {
                return self.rebind_view(identity, id).await;
            }
        }
        let read = self.read_rejoin(remote.reader(), context).await?;
        match self
            .runtime
            .request(Command::CommitHostRebind {
                id: id.clone(),
                read: Box::new(read),
            })
            .await?
        {
            Reply::HostRebind(_) => self.rebind_view(identity, id).await,
            _ => Err(WorkerError::InvalidRead),
        }
    }
}
