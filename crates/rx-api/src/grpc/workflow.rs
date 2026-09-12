use super::*;
#[tonic::async_trait]
impl base::workflow_service_server::WorkflowService for PlatformIngress {
    async fn get_run(
        &self,
        request: Request<base::RunRequest>,
    ) -> Result<Response<base::RunView>, Status> {
        let identity = self
            .validated_executor_identity(&request, request.get_ref().context.as_ref())
            .await?;
        if request
            .get_ref()
            .context
            .as_ref()
            .is_some_and(|c| c.expected_revision.is_some())
        {
            return Err(Status::invalid_argument(
                "GetRun does not accept entity revision",
            ));
        }
        let value = request.into_inner();
        if !value.recipe_digest.is_empty() || !value.site_config_digest.is_empty() {
            return Err(Status::invalid_argument(
                "GetRun does not accept creation digests",
            ));
        }
        let run = id(value
            .run_id
            .as_deref()
            .ok_or_else(|| Status::invalid_argument("run_id required"))?)?;
        let Reply::RunCheckpoint(snapshot) = self
            .call(Command::ExecutorRunCheckpoint { identity, run })
            .await?
        else {
            return Err(Status::internal("run checkpoint reply"));
        };
        let view = rx_protocol_adapter::workflow::run_view(&snapshot);
        // Validate the release's exact representation before returning it over the network.
        rx_protocol::json::to_value(&view)?;
        Ok(Response::new(view))
    }
    async fn create_run(
        &self,
        _: Request<base::RunRequest>,
    ) -> Result<Response<base::RunView>, Status> {
        Err(Status::unimplemented(
            "This workflow mutation is not enabled",
        ))
    }
    async fn start_run(
        &self,
        _: Request<base::RunRequest>,
    ) -> Result<Response<base::RunView>, Status> {
        Err(Status::unimplemented(
            "cell start requires the cell authorization contract",
        ))
    }
    async fn pause_run(
        &self,
        request: Request<base::RunRequest>,
    ) -> Result<Response<base::RunView>, Status> {
        let identity = self
            .validated_executor_identity(&request, request.get_ref().context.as_ref())
            .await?;
        let value = request.into_inner();
        if !value.recipe_digest.is_empty() || !value.site_config_digest.is_empty() {
            return Err(Status::invalid_argument(
                "PauseRun does not accept creation digests",
            ));
        }
        let context = value
            .context
            .ok_or_else(|| Status::invalid_argument("context required"))?;
        let key = id(context
            .request_key
            .as_deref()
            .ok_or_else(|| Status::invalid_argument("request key required"))?)?;
        let expected = context
            .expected_revision
            .filter(|r| *r > 0)
            .ok_or_else(|| Status::invalid_argument("run revision required"))?;
        let command = rx_application::PauseRunRequest {
            run: id(value
                .run_id
                .as_deref()
                .ok_or_else(|| Status::invalid_argument("run required"))?)?,
            expected_run: Counter(expected),
        };
        let Reply::RunCheckpoint(snapshot) = self
            .call(Command::PauseExecutorRun {
                identity,
                key,
                command,
            })
            .await?
        else {
            return Err(Status::internal("pause reply"));
        };
        let view = rx_protocol_adapter::workflow::run_view(&snapshot);
        rx_protocol::json::to_value(&view)?;
        Ok(Response::new(view))
    }
    async fn abandon_run(
        &self,
        _: Request<base::RunRequest>,
    ) -> Result<Response<base::RunView>, Status> {
        Err(Status::unimplemented(
            "This workflow mutation is not enabled",
        ))
    }
    async fn resolve_activation(
        &self,
        request: Request<base::ResolveActivation>,
    ) -> Result<Response<base::ActivationView>, Status> {
        let identity = self
            .validated_executor_identity(&request, request.get_ref().context.as_ref())
            .await?;
        let value = request.into_inner();
        let context = value
            .context
            .ok_or_else(|| Status::invalid_argument("context required"))?;
        let key = id(context
            .request_key
            .as_deref()
            .ok_or_else(|| Status::invalid_argument("request key required"))?)?;
        let expected = context
            .expected_revision
            .filter(|v| *v > 0)
            .ok_or_else(|| Status::invalid_argument("run revision required"))?;
        if value.visit == 0 {
            return Err(Status::invalid_argument("positive visit required"));
        }
        let command = rx_application::ResolveActivationRequest {
            run: id(&value.run_id)?,
            node: rx_protocol_adapter::name(&value.node_id)?,
            visit: Counter(value.visit),
            expected_run: Counter(expected),
        };
        let Reply::ActivationSnapshot(snapshot) = self
            .call(Command::ExecutorResolveActivation {
                identity,
                key,
                command,
            })
            .await?
        else {
            return Err(Status::internal("activation reply"));
        };
        let view = rx_protocol_adapter::workflow::activation_view(&snapshot);
        rx_protocol::json::to_value(&view)?;
        Ok(Response::new(view))
    }
    async fn commit_checkpoint(
        &self,
        request: Request<base::ChangeCheckpoint>,
    ) -> Result<Response<base::RunView>, Status> {
        let identity = self
            .validated_executor_identity(&request, request.get_ref().context.as_ref())
            .await?;
        let value = request.into_inner();
        let context = value
            .context
            .ok_or_else(|| Status::invalid_argument("context required"))?;
        if context.expected_revision.is_some() || value.expected_revision == 0 {
            return Err(Status::invalid_argument(
                "body revision required; context revision forbidden",
            ));
        }
        let key = id(context
            .request_key
            .as_deref()
            .ok_or_else(|| Status::invalid_argument("request key required"))?)?;
        let command = rx_application::CommitCheckpoint {
            run: id(&value.run_id)?,
            expected_revision: Counter(value.expected_revision),
            new_checkpoint: rx_protocol_adapter::workflow::checkpoint_input(
                value
                    .new_checkpoint
                    .ok_or_else(|| Status::invalid_argument("new checkpoint required"))?,
            )?,
        };
        let result = self
            .runtime
            .request(Command::CommitCheckpoint {
                identity,
                key,
                command: Box::new(command),
            })
            .await;
        let result = result.map_err(|error| {
            use rx_domain::fault::Rejection as R;
            use rx_protocol::checkpoint_rejection::{self, Rejection};
            match error {
                WriterError::Rejected(StoreError::Rejected(R::StaleRevision)) => {
                    checkpoint_rejection::status(Rejection::Revision)
                }
                WriterError::Rejected(StoreError::Rejected(R::Expired)) => {
                    checkpoint_rejection::status(Rejection::Expired)
                }
                other => super::failure(other),
            }
        })?;
        let Reply::RunCheckpoint(snapshot) = result else {
            return Err(Status::internal("checkpoint reply"));
        };
        let view = rx_protocol_adapter::workflow::run_view(&snapshot);
        rx_protocol::json::to_value(&view)?;
        Ok(Response::new(view))
    }
}
