use super::*;
#[allow(dead_code)]
mod fixture {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../rx-process-contract/tests/support/execution_v2_fixture.rs"
    ));
}
use fixture::{digest, id, n};
fn sample() -> (app::Work, app::Permit, Vec<u8>) {
    use rx_process_contract::execution_v2 as v;
    let policy = fixture::policy(1, 2);
    let (selection, intent, bytes) = fixture::selected(&policy);
    let mut publication = fixture::reference();
    publication.id = selection.publication.clone();
    let binding = v::OperationBinding {
        schema: n(v::OPERATION_SCHEMA),
        operation: id(40),
        mandate: id(41),
        publication,
        policy: fixture::artifact(v::POLICY_SCHEMA, &canonical::bytes(&policy).unwrap()),
        report: ArtifactRef {
            schema_id: n("rx.execution-report.v2"),
            sha256: selection.report_digest,
            size_bytes: Counter(10),
        },
        selection_digest: selection.digest().unwrap(),
        selection,
    };
    let work = app::Work {
        operation: rx_domain::operation::Operation::admitted(id(40), intent.digest().unwrap()),
        intent,
        cell: n("cell"),
        run: binding.selection.run.clone(),
        part: Some(binding.selection.part.clone()),
        activation: id(42),
        slot: n("main"),
        host: n("host"),
        permit: id(43),
        invocation: Some(id(44)),
        completion: app::CompletionRule::Unobservable,
        host_journal: id(45),
        handover_max_age_ns: Counter(100),
        execution: Some(Box::new(binding)),
    };
    let time = TimePoint {
        clock_id: id(46).to_string(),
        ticks_ns: Counter(1000),
    };
    let permit = app::Permit {
        id: id(43),
        operation: id(40),
        intent_digest: work.intent.digest().unwrap(),
        cell: work.cell.clone(),
        mandate: id(41),
        epoch: Counter(1),
        scopes: BTreeMap::new(),
        host: n("host"),
        host_boot: id(47),
        expires_at: time.clone(),
        state: app::PermitState::Issued,
        grant: app::Grant {
            id: id(48),
            fence: Counter(1),
            resources: work.intent.resource_set.clone(),
            owner: n("host"),
            valid_until: time.clone(),
            ttl_ms: Counter(100),
        },
        qualification: id(49),
        evidence_ids: vec![],
        qualification_revision: Counter(1),
        envelope_digest: digest(10),
        purpose: app::Purpose::Production,
        issued_at: time,
        condition_ids: vec![],
        condition_revision: Counter(1),
    };
    (work, permit, bytes)
}
#[derive(Clone)]
struct Server {
    corrupt: bool,
}
fn reply(corrupt: bool) -> wire::ExecutionReceipt {
    let (work, _, _) = sample();
    wire::ExecutionReceipt {
        operation_binding: if corrupt {
            vec![0; 32]
        } else {
            binding_digest(&work).unwrap()
        },
        receipt: Some(base::Receipt {
            operation_id: work.operation.id().to_string(),
            intent_digest: work.intent.digest().unwrap().as_bytes().to_vec(),
            invocation_id: Some(id(44).to_string()),
            journal_id: id(45).to_string(),
            journal_seq: 1,
            stage: base::ReceiptStage::HostPrepared as i32,
            host_state: Some(base::HostReceiptState::Prepared as i32),
            ..Default::default()
        }),
    }
}
#[tonic::async_trait]
impl wire::host_execution_service_server::HostExecutionService for Server {
    async fn prepare(
        &self,
        r: tonic::Request<wire::PrepareExecution>,
    ) -> Result<tonic::Response<wire::ExecutionReceipt>, Status> {
        let r = r.into_inner();
        let (work, _, bytes) = sample();
        assert_eq!(r.binding_hash, protocol());
        assert_eq!(r.parameters, bytes);
        assert_eq!(
            r.payload,
            canonical::bytes(work.execution.as_ref().unwrap()).unwrap()
        );
        assert_eq!(
            r.request.unwrap().base_request.unwrap().operation_id,
            work.operation.id().as_str()
        );
        Ok(tonic::Response::new(reply(self.corrupt)))
    }
    async fn authorize(
        &self,
        r: tonic::Request<wire::AuthorizeExecution>,
    ) -> Result<tonic::Response<wire::ExecutionReceipt>, Status> {
        let r = r.into_inner();
        assert_eq!(r.binding_hash, protocol());
        assert_eq!(r.operation_binding, binding_digest(&sample().0).unwrap());
        Ok(tonic::Response::new(reply(self.corrupt)))
    }
    async fn get_receipt(
        &self,
        r: tonic::Request<wire::ReadExecution>,
    ) -> Result<tonic::Response<wire::ExecutionReceipt>, Status> {
        assert_eq!(
            r.into_inner().operation_binding,
            binding_digest(&sample().0).unwrap()
        );
        Ok(tonic::Response::new(reply(self.corrupt)))
    }
    async fn reconcile(
        &self,
        r: tonic::Request<wire::ReadExecution>,
    ) -> Result<tonic::Response<wire::ExecutionEvidence>, Status> {
        assert_eq!(
            r.into_inner().operation_binding,
            binding_digest(&sample().0).unwrap()
        );
        Ok(tonic::Response::new(wire::ExecutionEvidence {
            operation_binding: if self.corrupt {
                vec![0; 32]
            } else {
                binding_digest(&sample().0).unwrap()
            },
            batch: Some(base::EvidenceBatch {
                producer_journal_id: id(50).to_string(),
                first_seq: 1,
                ..Default::default()
            }),
        }))
    }
}
#[tokio::test]
async fn v2_operation_transport_pins_binding_and_refuses_unsupported_or_corrupt_responses() {
    // Actual gRPC transport; test server is not a native Host or frozen-v1 binary.
    for (enabled, corrupt) in [(true, false), (true, true), (false, false)] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let paths = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = paths.clone();
        let server = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .layer(tower::layer::layer_fn(
                    move |service: tonic::service::Routes| {
                        let paths = observed.clone();
                        tower::service_fn(move |request: axum::http::Request<tonic::body::Body>| {
                            use tower::ServiceExt;
                            paths.lock().unwrap().push(request.uri().path().to_owned());
                            service.clone().oneshot(request)
                        })
                    },
                ))
                .add_optional_service(enabled.then(|| {
                    wire::host_execution_service_server::HostExecutionServiceServer::new(Server {
                        corrupt,
                    })
                }))
                .serve_with_shutdown(address, async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        let endpoint = Channel::from_shared(format!("http://{address}")).unwrap();
        let mut channel = None;
        for _ in 0..50 {
            match endpoint.clone().connect().await {
                Ok(c) => {
                    channel = Some(c);
                    break;
                }
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
            }
        }
        let client = HostClient {
            host_id: n("host"),
            session: base::Session {
                session_id: id(90).to_string(),
                ..Default::default()
            },
            channel: channel.unwrap(),
            transport_pin: None,
        };
        let (work, permit, parameters) = sample();
        for result in [
            client
                .prepare_execution(&id(60), &work, &permit, &parameters)
                .await,
            client.authorize(&id(61), &work, &permit).await,
            client.work_receipt(&work).await,
        ] {
            if enabled && !corrupt {
                assert_eq!(result.unwrap().operation, *work.operation.id());
            } else {
                assert_eq!(
                    result.unwrap_err().code(),
                    if corrupt {
                        tonic::Code::DataLoss
                    } else {
                        tonic::Code::Unimplemented
                    }
                );
            }
        }
        let evidence = client.work_evidence(&work).await;
        if enabled && !corrupt {
            assert!(evidence.unwrap().records.is_empty());
        } else {
            assert!(evidence.is_err());
        }
        assert_eq!(
            client
                .prepare(&id(62), &work, &permit)
                .await
                .unwrap_err()
                .code(),
            tonic::Code::FailedPrecondition
        );
        assert_eq!(
            client
                .prepare_execution(&id(62), &work, &permit, b"{}")
                .await
                .unwrap_err()
                .code(),
            tonic::Code::InvalidArgument
        );
        let observed = paths.lock().unwrap().clone();
        assert_eq!(observed.len(), 4);
        assert!(
            observed
                .iter()
                .all(|p| p.starts_with("/rx.host.execution.v2.HostExecutionService/")),
            "no fallback: {observed:?}"
        );
        let _ = stop.send(());
        server.await.unwrap();
    }
}

#[test]
fn operation_binding_rejects_mixed_versions_and_invalid_selection_bounds() {
    let (work, _, _) = sample();
    let original = work.execution.unwrap();
    for variant in 0..4 {
        let mut value = (*original).clone();
        match variant {
            0 => value.selection.schema = n("rx.execution-selection.v1"),
            1 => value.selection.slot_ordinal = Counter(0),
            2 => value.selection.authority_generation = Counter(0),
            _ => value.selection.parameter.schema_id = n("rx.parameters.v1"),
        }
        value.selection_digest = value.selection.digest().unwrap();
        assert!(value.validate().is_err());
    }
}
