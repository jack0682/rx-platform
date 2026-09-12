//! Exact stored cell metadata; no guessed mode, commissioning or block creation revision.
use rx_application::{BlockReason, Cell, Commissioning, OperatingMode};
use rx_domain::types::{ArtifactRef, Counter};
use rx_protocol::{base, cell};
use std::collections::BTreeSet;
use tonic::Status;
fn artifact(value: &ArtifactRef) -> base::ArtifactRef {
    base::ArtifactRef {
        sha256: value.sha256.as_bytes().to_vec(),
        schema_id: value.schema_id.to_string(),
        size_bytes: value.size_bytes.0,
    }
}
pub fn view(revision: Counter, value: &Cell) -> Result<cell::CellContext, Status> {
    let mode = value.mode.ok_or_else(|| {
        Status::failed_precondition("UPGRADE_REQUIRED: recorded operating context missing")
    })?;
    let commissioning = value.commissioning.ok_or_else(|| {
        Status::failed_precondition("UPGRADE_REQUIRED: recorded commissioning status missing")
    })?;
    if revision.0 == 0
        || value.epoch.0 == 0
        || value.scope_epochs.values().any(|e| e.0 == 0)
        || value.configuration.scopes.iter().collect::<BTreeSet<_>>()
            != value.scope_epochs.keys().collect()
        || value
            .blocks
            .iter()
            .map(|b| &b.id)
            .collect::<BTreeSet<_>>()
            .len()
            != value.blocks.len()
        || value.open_cases.iter().collect::<BTreeSet<_>>().len() != value.open_cases.len()
    {
        return Err(Status::data_loss("invalid stored cell context"));
    }
    if commissioning == Commissioning::Commissioned
        && value.qualification.as_ref().is_none_or(|q| {
            q.envelope_digest != value.configuration.envelope.sha256
                || q.environment != value.configuration.environment
        })
    {
        return Err(Status::data_loss("commissioning/qualification mismatch"));
    }
    let blocks = value
        .blocks
        .iter()
        .map(|b| {
            let created = b.created_revision.ok_or_else(|| {
                Status::failed_precondition("UPGRADE_REQUIRED: block creation revision missing")
            })?;
            if created.0 == 0
                || created > revision
                || b.scopes.is_empty()
                || b.scopes.iter().any(|s| !value.scope_epochs.contains_key(s))
            {
                return Err(Status::data_loss("invalid stored block metadata"));
            }
            Ok(cell::Block {
                block_id: b.id.to_string(),
                kind: if b.latched {
                    cell::BlockKind::Latched
                } else {
                    cell::BlockKind::Transient
                } as i32,
                reason: match b.reason {
                    BlockReason::CaseDiagnostic | BlockReason::CaseIntervention => {
                        cell::CellReason::BlockedByCase
                    }
                    BlockReason::RuntimeRestart | BlockReason::DeviceRestart => {
                        cell::CellReason::ContinuityUnproven
                    }
                    BlockReason::AuthorityRevoked => cell::CellReason::MandateRevoked,
                    BlockReason::ConditionLost | BlockReason::IntegrityConflict => {
                        cell::CellReason::ConditionUnknown
                    }
                    BlockReason::ConfigurationChange
                    | BlockReason::OutOfService
                    | BlockReason::ProcedureReported
                    | BlockReason::OperatorHold
                    | BlockReason::ExecutorPause => cell::CellReason::ExternalRestriction,
                } as i32,
                scope_ids: b.scopes.iter().map(ToString::to_string).collect(),
                condition_id: None,
                case_id: b.case_id.as_ref().map(ToString::to_string),
                created_revision: created.0,
            })
        })
        .collect::<Result<_, Status>>()?;
    let result = cell::CellContext {
        cell_id: value.configuration.id.to_string(),
        revision: revision.0,
        cell_epoch: value.epoch.0,
        scopes: value
            .scope_epochs
            .iter()
            .map(|(id, epoch)| cell::ScopeEpoch {
                scope_id: id.to_string(),
                epoch: epoch.0,
            })
            .collect(),
        definition: Some(artifact(&value.configuration.definition)),
        envelope: Some(artifact(&value.configuration.envelope)),
        qualification_id: value.qualification.as_ref().map(|q| q.id.to_string()),
        mode: match mode {
            OperatingMode::Setup => cell::OperatingMode::Setup,
            OperatingMode::Automatic => cell::OperatingMode::Automatic,
            OperatingMode::Recovery => cell::OperatingMode::Recovery,
            OperatingMode::Maintenance => cell::OperatingMode::Maintenance,
        } as i32,
        commissioning: match commissioning {
            Commissioning::NotCommissioned => cell::Commissioning::NotCommissioned,
            Commissioning::Commissioned => cell::Commissioning::Commissioned,
            Commissioning::RevalidationRequired => cell::Commissioning::RevalidationRequired,
        } as i32,
        blocks,
        open_case_ids: value.open_cases.iter().map(ToString::to_string).collect(),
    };
    rx_protocol::json::to_value(&result)?;
    Ok(result)
}
