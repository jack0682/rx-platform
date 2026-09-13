use super::*;

impl Worker {
    pub(super) async fn progress_locked(
        &self,
        identity: Identity,
        slot: &Slot,
        mut binding: data::Binding,
    ) -> Result<data::View, WorkerError> {
        if !matches!(
            binding.phase,
            data::Phase::Fencing | data::Phase::RecoveryOnly
        ) {
            return self.view(identity, binding.id).await;
        }
        let remote = self.connect(slot, &binding.context).await?;
        if binding.phase == data::Phase::RecoveryOnly {
            let read = self.read(&remote, &binding.context).await?;
            match self
                .runtime
                .request(Command::RefreshHostRecovery {
                    id: binding.id.clone(),
                    read: Box::new(read),
                })
                .await?
            {
                Reply::HostRecoveryBinding(_) => return self.view(identity, binding.id).await,
                _ => return Err(WorkerError::InvalidRead),
            }
        }
        let cells: Vec<_> = binding.fences.keys().cloned().collect();
        for cell in cells {
            if binding.fences[&cell].phase == data::FencePhase::Acknowledged {
                continue;
            }
            let read = self.read(&remote, &binding.context).await?;
            let task = match self
                .runtime
                .request(Command::PlanHostRecoveryFence {
                    id: binding.id.clone(),
                    cell: cell.clone(),
                    read: Box::new(read),
                })
                .await?
            {
                Reply::HostRecoveryFence(value) => value,
                _ => return Err(WorkerError::InvalidRead),
            };
            // The writer has already durably marked this exact request/body before I/O.
            let acknowledgment = remote.fence(&task).await?;
            binding = match self
                .runtime
                .request(Command::RecordHostRecoveryFence {
                    id: binding.id.clone(),
                    cell,
                    acknowledgment,
                })
                .await?
            {
                Reply::HostRecoveryBinding(value) => *value,
                _ => return Err(WorkerError::InvalidRead),
            };
            if binding.phase == data::Phase::Attention {
                return self.view(identity, binding.id).await;
            }
        }
        let read = self.read(&remote, &binding.context).await?;
        match self
            .runtime
            .request(Command::CommitHostRecovery {
                id: binding.id.clone(),
                expected_revision: binding.revision,
                read: Box::new(read),
            })
            .await?
        {
            Reply::HostRecoveryBinding(_) => self.view(identity, binding.id).await,
            _ => Err(WorkerError::InvalidRead),
        }
    }
}
