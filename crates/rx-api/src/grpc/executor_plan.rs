use super::*;
use rx_application::{CheckpointAction, CheckpointPreparation, CheckpointTarget};
use rx_protocol::executor_plan as wire;
#[tonic::async_trait]
impl wire::executor_plan_service_server::ExecutorPlanService for PlatformIngress {
    async fn prepare_checkpoint(
        &self,
        request: Request<wire::PrepareCheckpoint>,
    ) -> Result<Response<wire::CheckpointPreparation>, Status> {
        let identity = self
            .validated_executor_identity(&request, request.get_ref().context.as_ref())
            .await?;
        let value = request.into_inner();
        if value.binding_hash
            != manifest_hash(include_str!(
                "../../../../spec/executor-plan/v1/binding.json"
            ))
        {
            return Err(Status::failed_precondition(
                "executor plan binding mismatch",
            ));
        }
        let context = value
            .context
            .ok_or_else(|| Status::invalid_argument("context required"))?;
        if context.request_key.is_some() || context.expected_revision.is_some() || value.visit == 0
        {
            return Err(Status::invalid_argument(
                "positive visit and preparation context required",
            ));
        }
        let action = match wire::CheckpointAction::try_from(value.action) {
            Ok(wire::CheckpointAction::ChooseBranch) => CheckpointAction::ChooseBranch,
            Ok(wire::CheckpointAction::StartWait) => CheckpointAction::StartWait,
            Ok(wire::CheckpointAction::CheckWait) => CheckpointAction::CheckWait,
            _ => return Err(Status::invalid_argument("checkpoint action required")),
        };
        let target = CheckpointTarget {
            run: id(&value.run_id)?,
            node: rx_protocol_adapter::name(&value.node_id)?,
            visit: Counter(value.visit),
            action,
        };
        let Reply::CheckpointPreparation(result) = self
            .call(Command::PrepareCheckpoint { identity, target })
            .await?
        else {
            return Err(Status::internal("preparation reply"));
        };
        let mut reply = wire::CheckpointPreparation {
            state: 0,
            expected_revision: 0,
            checkpoint: None,
            prepared_at: None,
            valid_until: None,
        };
        match *result {
            CheckpointPreparation::Waiting { run_revision } => {
                reply.state = wire::PreparationState::Waiting as i32;
                reply.expected_revision = run_revision.0;
            }
            CheckpointPreparation::AlreadyApplied { run_revision } => {
                reply.state = wire::PreparationState::AlreadyApplied as i32;
                reply.expected_revision = run_revision.0;
            }
            CheckpointPreparation::Ready { proposal } => {
                reply.state = wire::PreparationState::Ready as i32;
                reply.expected_revision = proposal.expected_revision.0;
                reply.checkpoint = Some(rx_protocol_adapter::workflow::checkpoint(
                    &proposal.checkpoint,
                ));
                reply.prepared_at = Some(base::TimePoint {
                    clock_id: proposal.prepared_at.clock_id,
                    ticks_ns: proposal.prepared_at.ticks_ns.0,
                });
                reply.valid_until = Some(base::TimePoint {
                    clock_id: proposal.valid_until.clock_id,
                    ticks_ns: proposal.valid_until.ticks_ns.0,
                });
            }
        }
        rx_protocol::json::to_value(&reply)?;
        Ok(Response::new(reply))
    }
}
