use super::*;
#[tonic::async_trait]
impl base::operation_service_server::OperationService for PlatformIngress {
    async fn get(
        &self,
        request: Request<base::OperationRef>,
    ) -> Result<Response<base::OperationView>, Status> {
        let identity = self
            .validated_executor_identity(&request, request.get_ref().context.as_ref())
            .await?;
        let value = request.into_inner();
        if value
            .context
            .as_ref()
            .is_some_and(|c| c.expected_revision.is_some())
        {
            return Err(Status::invalid_argument("Get does not accept revision"));
        }
        let operation = id(&value.operation_id)?;
        let Reply::Work(work) = self
            .call(Command::ExecutorWork {
                identity,
                operation,
            })
            .await?
        else {
            return Err(Status::internal("work reply"));
        };
        let view = rx_protocol_adapter::operation::view(&work)?;
        rx_protocol::json::to_value(&view)?;
        Ok(Response::new(view))
    }
    async fn submit(
        &self,
        _: Request<base::SubmitOperation>,
    ) -> Result<Response<base::Receipt>, Status> {
        Err(Status::unimplemented(
            "use the mandatory cell admission contract",
        ))
    }
    async fn lookup(
        &self,
        _: Request<base::CallContext>,
    ) -> Result<Response<base::Receipt>, Status> {
        Err(Status::unimplemented(
            "Cell.SubmitOperation replies are recovered by retrying the same cell request",
        ))
    }
    async fn request_cancel(
        &self,
        _: Request<base::CancelRequest>,
    ) -> Result<Response<base::OperationView>, Status> {
        Err(Status::unimplemented("cancel binding is not enabled"))
    }
    async fn reconcile(
        &self,
        request: Request<base::OperationRef>,
    ) -> Result<Response<base::OperationView>, Status> {
        let identity = self
            .validated_executor_identity(&request, request.get_ref().context.as_ref())
            .await?;
        let value = request.into_inner();
        if value
            .context
            .as_ref()
            .is_some_and(|c| c.expected_revision.is_some())
        {
            return Err(Status::invalid_argument(
                "Reconcile does not accept entity revision",
            ));
        }
        let Reply::Work(work) = self
            .call(Command::RequestReconciliation {
                identity,
                operation: id(&value.operation_id)?,
            })
            .await?
        else {
            return Err(Status::internal("reconciliation reply"));
        };
        let view = rx_protocol_adapter::operation::view(&work)?;
        rx_protocol::json::to_value(&view)?;
        Ok(Response::new(view))
    }
}
