use super::*;
#[tonic::async_trait]
impl cell::cell_service_server::CellService for PlatformIngress {
    async fn open(
        &self,
        request: Request<cell::CellHello>,
    ) -> Result<Response<cell::CellSession>, Status> {
        let (fingerprint, principal) = self.certificate(&request)?;
        let hello = request.into_inner();
        if hello.peer_id != principal.as_str()
            || hello.base_manifest_hash != base_hash()
            || hello.cell_manifest_hash != cell_hash()
            || hello.shared_clock_id != self.configuration.installation.clock_id
        {
            return Err(Status::failed_precondition(
                "cell manifest/peer/clock mismatch",
            ));
        }
        let identity = Identity {
            principal: principal.clone(),
            session: id(&hello.base_session_id)?,
            terminal: None,
        };
        let Reply::ServicePeer(peer) = self
            .call(Command::InspectServicePeer(identity.clone()))
            .await?
        else {
            return Err(Status::internal("service peer reply"));
        };
        let definition = digest(&hello.cell_definition_digest)?;
        let command = match peer {
            rx_application::ServicePeer::Host(producer) => {
                if producer.authentication_binding != self.authentication_binding(fingerprint)? {
                    return Err(Status::unauthenticated("session/certificate mismatch"));
                }
                Command::NegotiateEvidenceCell {
                    identity,
                    definition,
                }
            }
            rx_application::ServicePeer::Executor(executor) => {
                if executor.authentication_binding
                    != self.executor_authentication_binding(fingerprint)?
                {
                    return Err(Status::unauthenticated("session/certificate mismatch"));
                }
                Command::NegotiateExecutorCell {
                    identity,
                    definition,
                }
            }
            rx_application::ServicePeer::OperatorApi(peer) => {
                if peer.authentication_binding
                    != self.operator_authentication_binding(fingerprint)?
                {
                    return Err(Status::unauthenticated("session/certificate mismatch"));
                }
                Command::NegotiateOperatorCell {
                    identity,
                    definition,
                }
            }
        };
        let Reply::CellName(cell) = self.call(command).await? else {
            return Err(Status::internal("cell negotiation reply"));
        };
        let hash = canonical::digest(
            "RX-CELL-NEGOTIATION-v1",
            &(&hello.base_session_id, cell, definition),
        )
        .map_err(|_| Status::internal("cell negotiation identity"))?;
        let mut bytes = [0; 16];
        bytes.copy_from_slice(&hash.as_bytes()[..16]);
        bytes[6] = (bytes[6] & 15) | 0x80;
        bytes[8] = (bytes[8] & 63) | 0x80;
        Ok(Response::new(cell::CellSession {
            session_id: uuid::Uuid::from_bytes(bytes).to_string(),
            base_session_id: hello.base_session_id,
            peer_id: principal.to_string(),
            manifest_hash: cell_hash(),
        }))
    }

    async fn inspect(
        &self,
        request: Request<cell::CellCall>,
    ) -> Result<Response<cell::CellContext>, Status> {
        let call = request.get_ref();
        if call.expected_cell_revision.is_some()
            || call
                .context
                .as_ref()
                .is_some_and(|c| c.expected_revision.is_some() || c.request_key.is_some())
        {
            return Err(Status::invalid_argument(
                "Inspect does not consume revision or request key",
            ));
        }
        let identity = self
            .validated_cell_peer_identity(&request, call.context.as_ref())
            .await?;
        let cell_id = rx_protocol_adapter::name(&call.cell_id)?;
        let Reply::Cell(revision, context) = self
            .call(Command::InspectPeerCell {
                identity,
                cell: cell_id,
            })
            .await?
        else {
            return Err(Status::internal("cell context reply"));
        };
        Ok(Response::new(rx_protocol_adapter::cell_context::view(
            revision, &context,
        )?))
    }

    async fn evaluate(
        &self,
        _: Request<cell::EvaluateRequest>,
    ) -> std::result::Result<Response<cell::EvaluationSet>, Status> {
        Err(Status::unimplemented(
            "This cell method is not enabled in this binding",
        ))
    }

    async fn start_run(
        &self,
        _: Request<cell::StartRunRequest>,
    ) -> std::result::Result<Response<cell::StartAttempt>, Status> {
        Err(Status::unimplemented(
            "This cell method is not enabled in this binding",
        ))
    }

    async fn get_start_attempt(
        &self,
        _: Request<cell::GetStartAttemptRequest>,
    ) -> std::result::Result<Response<cell::StartAttempt>, Status> {
        Err(Status::unimplemented(
            "This cell method is not enabled in this binding",
        ))
    }

    async fn begin_part_attempt(
        &self,
        request: Request<cell::BeginPartAttemptRequest>,
    ) -> Result<Response<cell::PartAttempt>, Status> {
        let call = request
            .get_ref()
            .call
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("CellCall required"))?;
        let identity = self
            .validated_executor_identity(&request, call.context.as_ref())
            .await?;
        let value = request.into_inner();
        let call = value
            .call
            .ok_or_else(|| Status::invalid_argument("CellCall required"))?;
        let context = call
            .context
            .ok_or_else(|| Status::invalid_argument("context required"))?;
        if context.expected_revision.is_some()
            || call.expected_cell_revision == Some(0)
            || value.expected_budget_revision == 0
        {
            return Err(Status::invalid_argument("invalid revision placement/value"));
        }
        let key = id(context
            .request_key
            .as_deref()
            .ok_or_else(|| Status::invalid_argument("request key required"))?)?;
        let command = rx_application::BeginPartRequest {
            cell: rx_protocol_adapter::name(&call.cell_id)?,
            run: id(&value.run_id)?,
            mandate: id(&value.mandate_id)?,
            expected_budget: Counter(value.expected_budget_revision),
            expected_cell: call.expected_cell_revision.map(Counter),
        };
        let Reply::PartSnapshot(snapshot) = self
            .call(Command::ExecutorBeginPart {
                identity,
                key,
                command,
            })
            .await?
        else {
            return Err(Status::internal("part reply"));
        };
        let view = rx_protocol_adapter::workflow::part_view(&snapshot);
        rx_protocol::json::to_value(&view)?;
        Ok(Response::new(view))
    }

    async fn submit_operation(
        &self,
        request: Request<cell::SubmitOperationRequest>,
    ) -> Result<Response<base::Receipt>, Status> {
        let call = request
            .get_ref()
            .call
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("CellCall required"))?;
        let identity = self
            .validated_executor_identity(&request, call.context.as_ref())
            .await?;
        let value = request.into_inner();
        let call = value
            .call
            .ok_or_else(|| Status::invalid_argument("CellCall required"))?;
        let context = call
            .context
            .ok_or_else(|| Status::invalid_argument("context required"))?;
        let base_request = value
            .request
            .ok_or_else(|| Status::invalid_argument("SubmitOperation required"))?;
        let nested = base_request
            .context
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("nested context required"))?;
        if nested != &context || context.expected_revision.is_some() {
            return Err(Status::invalid_argument(
                "nested contexts must match; base revision must be absent",
            ));
        }
        let key = id(context
            .request_key
            .as_deref()
            .ok_or_else(|| Status::invalid_argument("request key required"))?)?;
        if value.expected_case_revision.is_some() {
            return Err(Status::failed_precondition(
                "recovery admission is not enabled",
            ));
        }
        let expected_cell = call
            .expected_cell_revision
            .filter(|v| *v > 0)
            .ok_or_else(|| Status::invalid_argument("cell revision required"))?;
        let expected_run = value
            .expected_run_revision
            .filter(|v| *v > 0)
            .ok_or_else(|| Status::invalid_argument("run revision required"))?;
        let Some(cell::permit_parent::Value::MandateId(mandate)) =
            value.parent.and_then(|p| p.value)
        else {
            return Err(Status::failed_precondition(
                "run mandate parent required; recovery binding is not enabled",
            ));
        };
        let intent = rx_protocol::json::intent_to_domain(
            &base_request
                .intent
                .ok_or_else(|| Status::invalid_argument("intent required"))?,
        )?;
        let command = rx_application::ExecutorSubmitRequest {
            cell: rx_protocol_adapter::name(&call.cell_id)?,
            mandate: id(&mandate)?,
            work: rx_application::SubmitWork {
                run: id(base_request
                    .run_id
                    .as_deref()
                    .ok_or_else(|| Status::invalid_argument("run_id required"))?)?,
                activation: id(base_request
                    .activation_id
                    .as_deref()
                    .ok_or_else(|| Status::invalid_argument("activation_id required"))?)?,
                part: value.part_attempt_id.as_deref().map(id).transpose()?,
                slot: rx_protocol_adapter::name(
                    base_request
                        .slot
                        .as_deref()
                        .ok_or_else(|| Status::invalid_argument("slot required"))?,
                )?,
                intent,
                expected_cell: Counter(expected_cell),
                expected_run: Counter(expected_run),
            },
        };
        let Reply::AdmissionReceipt(receipt) = self
            .call(Command::ExecutorSubmit {
                identity,
                key,
                command: Box::new(command),
            })
            .await?
        else {
            return Err(Status::internal("admission reply"));
        };
        let receipt = rx_protocol_adapter::workflow::admission_receipt(&receipt);
        rx_protocol::json::to_value(&receipt)?;
        Ok(Response::new(receipt))
    }

    async fn hold(
        &self,
        _: Request<cell::HoldRequest>,
    ) -> std::result::Result<Response<cell::CellContext>, Status> {
        Err(Status::unimplemented(
            "This cell method is not enabled in this binding",
        ))
    }

    async fn clear_transient_block(
        &self,
        _: Request<cell::ClearTransientBlockRequest>,
    ) -> std::result::Result<Response<cell::CellContext>, Status> {
        Err(Status::unimplemented(
            "This cell method is not enabled in this binding",
        ))
    }

    async fn open_case(
        &self,
        request: Request<cell::OpenCaseRequest>,
    ) -> Result<Response<cell::InterventionCase>, Status> {
        let call = request
            .get_ref()
            .call
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("cell call required"))?;
        let identity = self
            .validated_executor_identity(&request, call.context.as_ref())
            .await?;
        let value = request.into_inner();
        let call = value.call.unwrap();
        let context = call
            .context
            .ok_or_else(|| Status::invalid_argument("context required"))?;
        if context.expected_revision.is_some() {
            return Err(Status::invalid_argument("context revision not used"));
        }
        let key = id(context
            .request_key
            .as_deref()
            .ok_or_else(|| Status::invalid_argument("key required"))?)?;
        let command = rx_application::intervention::OpenCase {
            cell: rx_protocol_adapter::name(&call.cell_id)?,
            expected_cell: call.expected_cell_revision.map(Counter),
            kind: rx_protocol_adapter::intervention::case_type(value.r#type)?,
            scopes: value
                .scopes
                .iter()
                .map(|v| rx_protocol_adapter::name(v))
                .collect::<Result<_, _>>()?,
            procedure: rx_protocol_adapter::artifact(
                value
                    .procedure
                    .ok_or_else(|| Status::invalid_argument("procedure reference required"))?,
            )?,
            lead: rx_protocol_adapter::name(&value.lead)?,
            operation_ids: value
                .operation_ids
                .iter()
                .map(|v| id(v))
                .collect::<Result<_, _>>()?,
            material_ids: value
                .material_ids
                .iter()
                .map(|v| id(v))
                .collect::<Result<_, _>>()?,
        };
        let Reply::CaseSnapshot(snapshot) = self
            .call(Command::OpenCase {
                identity,
                key,
                command,
            })
            .await?
        else {
            return Err(Status::internal("case reply"));
        };
        let value = rx_protocol_adapter::intervention::case_view(&snapshot);
        rx_protocol::json::to_value(&value)?;
        Ok(Response::new(value))
    }

    async fn record_procedure(
        &self,
        request: Request<cell::RecordProcedureRequest>,
    ) -> Result<Response<cell::InterventionCase>, Status> {
        use rx_application::procedure::{Action, Record, Submission};
        let call = request
            .get_ref()
            .call
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("cell call required"))?;
        let identity = self
            .validated_executor_identity(&request, call.context.as_ref())
            .await?;
        let value = request.into_inner();
        let call = value.call.unwrap();
        let context = call
            .context
            .ok_or_else(|| Status::invalid_argument("context required"))?;
        if context.expected_revision.is_some() {
            return Err(Status::invalid_argument("context revision forbidden"));
        }
        let key = id(context
            .request_key
            .as_deref()
            .ok_or_else(|| Status::invalid_argument("request key required"))?)?;
        let record = value
            .record
            .ok_or_else(|| Status::invalid_argument("procedure record required"))?;
        let action = match cell::ProcedureAction::try_from(record.action) {
            Ok(cell::ProcedureAction::Acknowledge) => Action::Acknowledge,
            Ok(cell::ProcedureAction::EntryConditionsReported) => Action::EntryConditionsReported,
            Ok(cell::ProcedureAction::WorkStarted) => Action::WorkStarted,
            Ok(cell::ProcedureAction::WorkFinished) => Action::WorkFinished,
            Ok(cell::ProcedureAction::PersonnelAccounted) => Action::PersonnelAccounted,
            Ok(cell::ProcedureAction::IsolationStateReported) => Action::IsolationStateReported,
            Ok(cell::ProcedureAction::ResetObserved) => Action::ResetObserved,
            Ok(cell::ProcedureAction::HandoverAccepted) => Action::HandoverAccepted,
            Ok(cell::ProcedureAction::ConfigurationReported) => Action::ConfigurationReported,
            _ => return Err(Status::invalid_argument("procedure action required")),
        };
        let command =
            Submission {
                cell: rx_protocol_adapter::name(&call.cell_id)?,
                expected_cell: call.expected_cell_revision.map(Counter),
                expected_case: Counter(value.expected_case_revision),
                assertions: None,
                record: Record {
                    id: id(&record.record_id)?,
                    case: id(&record.case_id)?,
                    case_revision: Counter(record.case_revision),
                    action,
                    actor: rx_protocol_adapter::name(&record.actor)?,
                    scope_ids: record
                        .scope_ids
                        .iter()
                        .map(|s| rx_protocol_adapter::name(s))
                        .collect::<Result<_, _>>()?,
                    occurred_at: record.occurred_at,
                    evidence_ids: record
                        .evidence_ids
                        .iter()
                        .map(|s| id(s))
                        .collect::<Result<_, _>>()?,
                    assertions: rx_protocol_adapter::artifact(record.assertions.ok_or_else(
                        || Status::invalid_argument("assertion reference required"),
                    )?)?,
                },
            };
        let Reply::ProcedureReceipt(receipt) = self
            .call(Command::RecordProcedure {
                identity,
                key,
                command: Box::new(command),
            })
            .await?
        else {
            return Err(Status::internal("procedure receipt"));
        };
        if let Some(reason) = receipt.transition_error {
            use rx_domain::fault::Rejection;
            let mut status = match reason {
                Rejection::StaleRevision => Status::aborted("procedure transition CAS rejected"),
                Rejection::Forbidden => Status::permission_denied("procedure promotion forbidden"),
                _ => Status::failed_precondition("procedure transition needs evidence or policy"),
            };
            status
                .metadata_mut()
                .insert("rx-procedure-facts-recorded", "true".parse().unwrap());
            status.metadata_mut().insert(
                "rx-procedure-record-id",
                receipt
                    .record
                    .record
                    .id
                    .as_str()
                    .parse()
                    .map_err(|_| Status::internal("record ID metadata"))?,
            );
            status.metadata_mut().insert(
                "rx-procedure-case-id",
                receipt
                    .case
                    .case
                    .id
                    .as_str()
                    .parse()
                    .map_err(|_| Status::internal("case ID metadata"))?,
            );
            return Err(status);
        }
        let view = rx_protocol_adapter::intervention::case_view(&receipt.case);
        rx_protocol::json::to_value(&view)?;
        Ok(Response::new(view))
    }

    async fn set_recovery_plan(
        &self,
        _: Request<cell::SetRecoveryPlanRequest>,
    ) -> std::result::Result<Response<cell::InterventionCase>, Status> {
        Err(Status::unimplemented(
            "This cell method is not enabled in this binding",
        ))
    }

    async fn prepare_restart(
        &self,
        _: Request<cell::PrepareRestartRequest>,
    ) -> std::result::Result<Response<cell::RestartPreparation>, Status> {
        Err(Status::unimplemented(
            "This cell method is not enabled in this binding",
        ))
    }

    async fn get_restart_preparation(
        &self,
        _: Request<cell::GetRestartPreparationRequest>,
    ) -> std::result::Result<Response<cell::RestartPreparation>, Status> {
        Err(Status::unimplemented(
            "This cell method is not enabled in this binding",
        ))
    }

    async fn prepare_close(
        &self,
        _: Request<cell::PrepareCloseRequest>,
    ) -> std::result::Result<Response<cell::Clearance>, Status> {
        Err(Status::unimplemented(
            "This cell method is not enabled in this binding",
        ))
    }

    async fn restart_run(
        &self,
        _: Request<cell::RestartRunRequest>,
    ) -> std::result::Result<Response<cell::StartAttempt>, Status> {
        Err(Status::unimplemented(
            "This cell method is not enabled in this binding",
        ))
    }

    async fn close_without_restart(
        &self,
        _: Request<cell::CloseWithoutRestartRequest>,
    ) -> std::result::Result<Response<cell::CellContext>, Status> {
        Err(Status::unimplemented(
            "This cell method is not enabled in this binding",
        ))
    }

    async fn record_change(
        &self,
        _: Request<cell::RecordChangeRequest>,
    ) -> std::result::Result<Response<cell::ChangeRecord>, Status> {
        Err(Status::unimplemented(
            "This cell method is not enabled in this binding",
        ))
    }

    async fn register_qualification(
        &self,
        _: Request<cell::RegisterQualificationRequest>,
    ) -> std::result::Result<Response<cell::Qualification>, Status> {
        Err(Status::unimplemented(
            "This cell method is not enabled in this binding",
        ))
    }

    async fn get_case(
        &self,
        request: Request<cell::GetCaseRequest>,
    ) -> Result<Response<cell::InterventionCase>, Status> {
        let call = request
            .get_ref()
            .call
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("cell call required"))?;
        let identity = self
            .validated_executor_identity(&request, call.context.as_ref())
            .await?;
        let value = request.into_inner();
        let call = value.call.unwrap();
        if call.expected_cell_revision.is_some()
            || call
                .context
                .as_ref()
                .is_some_and(|c| c.expected_revision.is_some())
        {
            return Err(Status::invalid_argument("read revision forbidden"));
        }
        let Reply::CaseDetail(detail) = self
            .call(Command::InspectCase {
                identity,
                cell: rx_protocol_adapter::name(&call.cell_id)?,
                case: id(&value.case_id)?,
            })
            .await?
        else {
            return Err(Status::internal("case reply"));
        };
        let value = rx_protocol_adapter::intervention::case_view(&detail.snapshot);
        rx_protocol::json::to_value(&value)?;
        Ok(Response::new(value))
    }

    async fn get_material(
        &self,
        _: Request<cell::GetMaterialRequest>,
    ) -> std::result::Result<Response<cell::MaterialState>, Status> {
        Err(Status::unimplemented(
            "This cell method is not enabled in this binding",
        ))
    }
}
