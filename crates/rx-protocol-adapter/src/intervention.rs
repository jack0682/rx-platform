use crate::*;
use rx_application::intervention::{CaseSnapshot, CaseState, CaseType};
use rx_protocol::cell;
pub fn case_type(value: i32) -> Result<CaseType, Status> {
    Ok(match cell::CaseType::try_from(value) {
        Ok(cell::CaseType::DiagnosticOnly) => CaseType::DiagnosticOnly,
        Ok(cell::CaseType::PlannedAccess) => CaseType::PlannedAccess,
        Ok(cell::CaseType::FaultRecovery) => CaseType::FaultRecovery,
        Ok(cell::CaseType::Maintenance) => CaseType::Maintenance,
        Ok(cell::CaseType::ChangeReview) => CaseType::ChangeReview,
        _ => return Err(Status::invalid_argument("case type required")),
    })
}
pub fn case_view(value: &CaseSnapshot) -> cell::InterventionCase {
    let case = &value.case;
    let reference = &value.current;
    cell::InterventionCase {
        case_id: case.id.to_string(),
        revision: value.revision.0,
        cell: Some(cell::CellRef {
            cell_id: reference.cell.to_string(),
            cell_epoch: reference.cell_epoch.0,
            scopes: reference
                .scope_epochs
                .iter()
                .map(|(scope, epoch)| cell::ScopeEpoch {
                    scope_id: scope.to_string(),
                    epoch: epoch.0,
                })
                .collect(),
        }),
        r#type: match case.kind {
            CaseType::DiagnosticOnly => cell::CaseType::DiagnosticOnly,
            CaseType::PlannedAccess => cell::CaseType::PlannedAccess,
            CaseType::FaultRecovery => cell::CaseType::FaultRecovery,
            CaseType::Maintenance => cell::CaseType::Maintenance,
            CaseType::ChangeReview => cell::CaseType::ChangeReview,
        } as i32,
        state: match case.state {
            CaseState::Open => cell::CaseState::Open,
            CaseState::ContainmentPending => cell::CaseState::ContainmentPending,
            CaseState::ProcedureActive => cell::CaseState::ProcedureActive,
            CaseState::Revalidating => cell::CaseState::Revalidating,
            CaseState::ReadyForRestart => cell::CaseState::ReadyForRestart,
            CaseState::Closed => cell::CaseState::Closed,
            CaseState::Escalated => cell::CaseState::Escalated,
        } as i32,
        procedure: Some(base::ArtifactRef {
            sha256: case.procedure.sha256.as_bytes().to_vec(),
            schema_id: case.procedure.schema_id.to_string(),
            size_bytes: case.procedure.size_bytes.0,
        }),
        lead: case.lead.to_string(),
        participants: case.participants.iter().map(ToString::to_string).collect(),
        operation_ids: case.operation_ids.iter().map(ToString::to_string).collect(),
        material_ids: case.material_ids.iter().map(ToString::to_string).collect(),
        record_ids: case.record_ids.iter().map(ToString::to_string).collect(),
        block_ids: case.block_ids.iter().map(ToString::to_string).collect(),
    }
}
