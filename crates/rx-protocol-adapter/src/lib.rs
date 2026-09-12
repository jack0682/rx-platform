//! Complete boundary conversions shared by incoming Publish and outgoing Reconcile.
//! The core/application do not depend on generated wire types.
pub mod intervention;
pub mod operation;
pub mod workflow;
use rx_application as app;
use rx_domain::types::*;
use rx_protocol::base;
use tonic::Status;

pub fn id(value: &str) -> Result<Id, Status> {
    Id::new(value).map_err(|_| Status::invalid_argument("canonical UUID required"))
}
pub fn name(value: &str) -> Result<Name, Status> {
    Name::new(value).map_err(|_| Status::invalid_argument("Name required"))
}
pub fn digest(value: &[u8]) -> Result<Digest, Status> {
    Ok(Digest::from_bytes(value.try_into().map_err(|_| {
        Status::invalid_argument("SHA-256 digest required")
    })?))
}
pub fn artifact(value: base::ArtifactRef) -> Result<ArtifactRef, Status> {
    Ok(ArtifactRef {
        sha256: digest(&value.sha256)?,
        schema_id: name(&value.schema_id)?,
        size_bytes: Counter(value.size_bytes),
    })
}
pub fn evidence_batch(value: base::EvidenceBatch) -> Result<app::EvidenceBatch, Status> {
    if value.first_seq == 0 || value.records.len() > 128 {
        return Err(Status::invalid_argument("invalid evidence batch range"));
    }
    Ok(app::EvidenceBatch {
        journal: id(&value.producer_journal_id)?,
        first: Counter(value.first_seq),
        records: value
            .records
            .into_iter()
            .map(native_evidence)
            .collect::<Result<_, _>>()?,
    })
}
pub fn native_evidence(record: base::Evidence) -> Result<app::NativeEvidence, Status> {
    if record.evidence_schema != "rx.native-result.v1" {
        return Err(Status::failed_precondition(
            "unsupported native evidence schema",
        ));
    }
    let profile = digest(&record.profile_digest)?;
    let Some(base::evidence_body::Value::NativeResult(result)) =
        record.body.and_then(|body| body.value)
    else {
        return Err(Status::failed_precondition("unsupported evidence body"));
    };
    let correlation = result
        .correlation
        .ok_or_else(|| Status::invalid_argument("correlation required"))?;
    if correlation.profile_digest != profile.as_bytes() || correlation.cancel_id.is_some() {
        return Err(Status::failed_precondition(
            "unexpected native evidence correlation",
        ));
    }
    let captured = result
        .captured_at
        .ok_or_else(|| Status::invalid_argument("capture time required"))?;
    if captured.clock_id.is_empty() {
        return Err(Status::invalid_argument("capture clock required"));
    }
    Ok(app::NativeEvidence {
        id: id(&record.evidence_id)?,
        operation: id(correlation
            .operation_id
            .as_deref()
            .ok_or_else(|| Status::invalid_argument("operation required"))?)?,
        invocation: id(correlation
            .invocation_id
            .as_deref()
            .ok_or_else(|| Status::invalid_argument("invocation required"))?)?,
        profile_digest: profile,
        device_session: id(&correlation.device_session_id)?,
        status_schema: name(&result.native_status_schema)?,
        status: Integer(result.native_status),
        captured_at: TimePoint {
            clock_id: captured.clock_id,
            ticks_ns: Counter(captured.ticks_ns),
        },
        native_details: Some(app::NativeEvidenceDetails {
            native_id: correlation.native_id,
            native_data: result.native_data.map(artifact).transpose()?,
        }),
    })
}
pub fn evidence_wire(record: app::NativeEvidence) -> Result<base::Evidence, Status> {
    let details = record.native_details.ok_or_else(|| {
        Status::failed_precondition("legacy evidence is not a complete source record")
    })?;
    Ok(base::Evidence {
        evidence_id: record.id.to_string(),
        evidence_schema: "rx.native-result.v1".into(),
        profile_digest: record.profile_digest.as_bytes().to_vec(),
        body: Some(base::EvidenceBody {
            value: Some(base::evidence_body::Value::NativeResult(
                base::NativeResult {
                    correlation: Some(base::Correlation {
                        operation_id: Some(record.operation.to_string()),
                        invocation_id: Some(record.invocation.to_string()),
                        native_id: details.native_id,
                        device_session_id: record.device_session.to_string(),
                        profile_digest: record.profile_digest.as_bytes().to_vec(),
                        cancel_id: None,
                    }),
                    native_status_schema: record.status_schema.to_string(),
                    native_status: record.status.0,
                    native_data: details.native_data.map(|a| base::ArtifactRef {
                        sha256: a.sha256.as_bytes().to_vec(),
                        schema_id: a.schema_id.to_string(),
                        size_bytes: a.size_bytes.0,
                    }),
                    captured_at: Some(base::TimePoint {
                        clock_id: record.captured_at.clock_id,
                        ticks_ns: record.captured_at.ticks_ns.0,
                    }),
                },
            )),
        }),
    })
}
pub fn durable_ack(commit: app::EvidenceCommit) -> base::DurableAck {
    base::DurableAck {
        producer_journal_id: commit.producer.journal.to_string(),
        through_seq: commit.producer.through.0,
        platform_cursor: Some(base::Cursor {
            installation_id: commit.installation.to_string(),
            store_generation: commit.store_generation.to_string(),
            seq: commit.platform_sequence.0,
            view_id: rx_application::control_journal::VIEW_ID.into(),
        }),
    }
}

pub mod cell_context;
