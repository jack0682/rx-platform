use super::*;

#[tonic::async_trait]
impl base::session_service_server::SessionService for PlatformIngress {
    async fn open(
        &self,
        request: Request<base::PeerHello>,
    ) -> Result<Response<base::Session>, Status> {
        let (fingerprint, principal) = self.certificate(&request)?;
        let hello = request.into_inner();
        let meta = &self.configuration.installation;
        if hello.peer_id != principal.as_str()
            || !matches!(
                base::Role::try_from(hello.role),
                Ok(base::Role::Host | base::Role::Executor | base::Role::OperatorApi)
            )
            || hello.installation_id != meta.id.as_str()
            || hello.store_generation != meta.store_generation.as_str()
            || hello.shared_clock_id != meta.clock_id
            || digest(&hello.release_digest)? != self.configuration.release_digest
            || !hello
                .supported_versions
                .iter()
                .any(|v| v.major == 1 && v.minor == 0 && v.schema_hash == base_hash())
        {
            return Err(Status::failed_precondition(
                "peer/release/clock/base contract mismatch",
            ));
        }
        let peer_boot = id(&hello.boot_id)?;
        let command = match base::Role::try_from(hello.role) {
            Ok(base::Role::Host) => {
                if hello.last_seq.is_none() {
                    return Err(Status::invalid_argument("evidence journal tail required"));
                }
                let journal = id(hello
                    .journal_id
                    .as_deref()
                    .ok_or_else(|| Status::invalid_argument("evidence journal required"))?)?;
                Command::OpenEvidenceProducer {
                    principal: principal.clone(),
                    peer_boot: peer_boot.clone(),
                    journal,
                    authentication_binding: self.authentication_binding(fingerprint)?,
                }
            }
            Ok(base::Role::Executor) => {
                if hello.journal_id.is_some() || hello.last_seq.is_some() {
                    return Err(Status::invalid_argument(
                        "executor journal binding is not enabled",
                    ));
                }
                Command::OpenExecutorPeer {
                    principal: principal.clone(),
                    peer_boot: peer_boot.clone(),
                    authentication_binding: self.executor_authentication_binding(fingerprint)?,
                }
            }
            Ok(base::Role::OperatorApi) => {
                if hello.journal_id.is_some() || hello.last_seq.is_some() {
                    return Err(Status::invalid_argument(
                        "operator API cannot publish an evidence journal",
                    ));
                }
                Command::OpenOperatorPeer {
                    principal: principal.clone(),
                    peer_boot: peer_boot.clone(),
                    authentication_binding: self.operator_authentication_binding(fingerprint)?,
                }
            }
            _ => return Err(Status::permission_denied("service role required")),
        };
        let Reply::Session(session) = self.call(command).await? else {
            return Err(Status::internal("service session reply"));
        };
        Ok(Response::new(base::Session {
            session_id: session.id.to_string(),
            peer_id: principal.to_string(),
            boot_id: peer_boot.to_string(),
            selected_version: Some(base::Version {
                major: 1,
                minor: 0,
                schema_hash: base_hash(),
            }),
            required_features: vec!["rx.cell.v1".into(), "strict-wire-v1".into()],
            limits: Some(base::Limits {
                max_message_bytes: 1_048_576,
                max_batch_records: 128,
                subscriber_buffer_bytes: 4_194_304,
                max_inflight: 32,
            }),
        }))
    }
}
