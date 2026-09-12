use rx_application::{RunState, checkpoint_artifact::*};
use rx_protocol::base;
pub fn checkpoint_input(value: base::Checkpoint) -> Result<CheckpointView, tonic::Status> {
    let activations = value
        .activations
        .into_iter()
        .map(|a| {
            let slots = a
                .slots
                .into_iter()
                .map(|s| {
                    Ok(SlotSnapshot {
                        slot: crate::name(&s.slot)?,
                        operation: crate::id(&s.operation_id)?,
                        intent_digest: crate::digest(&s.intent_digest)?,
                    })
                })
                .collect::<Result<Vec<_>, tonic::Status>>()?;
            if a.visit == 0 {
                return Err(tonic::Status::invalid_argument("positive visit required"));
            }
            Ok(ActivationSnapshot {
                id: crate::id(&a.activation_id)?,
                node: crate::name(&a.node_id)?,
                visit: rx_domain::types::Counter(a.visit),
                slots,
            })
        })
        .collect::<Result<Vec<_>, tonic::Status>>()?;
    Ok(CheckpointView {
        run: crate::id(&value.run_id)?,
        revision: rx_domain::types::Counter(value.revision),
        executor_schema: crate::name(&value.executor_schema)?,
        payload: crate::artifact(
            value
                .payload
                .ok_or_else(|| tonic::Status::invalid_argument("checkpoint artifact required"))?,
        )?,
        activations,
    })
}
pub fn run_view(snapshot: &RunSnapshot) -> base::RunView {
    base::RunView {
        run_id: snapshot.run.id.to_string(),
        revision: snapshot.revision.0,
        recipe_digest: snapshot.run.recipe_digest.as_bytes().to_vec(),
        state: match snapshot.run.state {
            RunState::Prepared => base::RunState::Prepared,
            RunState::Executing => base::RunState::Executing,
            RunState::Paused => base::RunState::Paused,
            RunState::RecoveryRequired => base::RunState::RecoveryRequired,
            RunState::Completed => base::RunState::Completed,
            RunState::Abandoned => base::RunState::Abandoned,
        } as i32,
        checkpoint: Some(checkpoint(&snapshot.checkpoint)),
        executor_session_id: snapshot
            .run
            .executor_session
            .as_ref()
            .map(ToString::to_string),
    }
}
pub fn checkpoint(value: &CheckpointView) -> base::Checkpoint {
    base::Checkpoint {
        run_id: value.run.to_string(),
        revision: value.revision.0,
        executor_schema: value.executor_schema.to_string(),
        payload: Some(base::ArtifactRef {
            sha256: value.payload.sha256.as_bytes().to_vec(),
            schema_id: value.payload.schema_id.to_string(),
            size_bytes: value.payload.size_bytes.0,
        }),
        activations: value.activations.iter().map(activation_view).collect(),
    }
}

pub fn activation_view(value: &ActivationSnapshot) -> base::ActivationView {
    base::ActivationView {
        activation_id: value.id.to_string(),
        node_id: value.node.to_string(),
        visit: value.visit.0,
        slots: value
            .slots
            .iter()
            .map(|slot| base::SlotBinding {
                slot: slot.slot.to_string(),
                operation_id: slot.operation.to_string(),
                intent_digest: slot.intent_digest.as_bytes().to_vec(),
            })
            .collect(),
    }
}
pub fn part_view(value: &rx_application::PartSnapshot) -> rx_protocol::cell::PartAttempt {
    use rx_application::PartDisposition as P;
    use rx_protocol::cell::PartDisposition as W;
    rx_protocol::cell::PartAttempt {
        part_attempt_id: value.part.id.to_string(),
        run_id: value.part.run.to_string(),
        ordinal: value.part.ordinal.0,
        material_id: None,
        revision: value.revision.0,
        disposition: match value.part.disposition {
            P::InProgress => W::InProgress,
            P::ConfirmedCompleted => W::ConfirmedCompleted,
            P::Rejected => W::Rejected,
            P::Unresolved => W::Unresolved,
            P::NotProcessed => W::NotProcessed,
        } as i32,
    }
}

pub fn admission_receipt(value: &rx_application::AdmissionReceipt) -> base::Receipt {
    base::Receipt {
        operation_id: value.operation.to_string(),
        intent_digest: value.intent_digest.as_bytes().to_vec(),
        operation_revision: Some(value.operation_revision.0),
        stage: base::ReceiptStage::Admitted as i32,
        invocation_id: None,
        journal_id: value.journal.to_string(),
        journal_seq: value.sequence.0,
        host_state: None,
        cancel_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rx_application::Run;
    use rx_domain::types::*;
    fn id(value: u8) -> Id {
        Id::new(format!("00000000-0000-4000-8000-{value:012}")).unwrap()
    }
    fn name(value: &str) -> Name {
        Name::new(value).unwrap()
    }
    #[test]
    fn frozen_run_view_preserves_counters_slots_and_each_run_state() {
        for (state, expected) in [
            (RunState::Prepared, "PREPARED"),
            (RunState::Executing, "EXECUTING"),
            (RunState::Paused, "PAUSED"),
            (RunState::RecoveryRequired, "RECOVERY_REQUIRED"),
            (RunState::Completed, "COMPLETED"),
            (RunState::Abandoned, "ABANDONED"),
        ] {
            let digest = Digest::from_bytes([37; 32]);
            let snapshot = RunSnapshot {
                revision: Counter(u64::MAX),
                run: Run {
                    id: id(1),
                    cell: name("cell/a"),
                    recipe_digest: digest,
                    envelope_digest: digest,
                    purpose: None,
                    state,
                    budget: None,
                    executor_session: Some(id(2)),
                    mandate: None,
                    part_ids: vec![],
                    pending_attempt: None,
                },
                checkpoint: CheckpointView {
                    run: id(1),
                    revision: Counter(u64::MAX),
                    executor_schema: name(SCHEMA),
                    payload: ArtifactRef {
                        sha256: digest,
                        schema_id: name(SCHEMA),
                        size_bytes: Counter(99),
                    },
                    activations: vec![ActivationSnapshot {
                        id: id(3),
                        node: name("step/place"),
                        visit: Counter(9_007_199_254_740_993),
                        slots: vec![SlotSnapshot {
                            slot: name("main"),
                            operation: id(4),
                            intent_digest: digest,
                        }],
                    }],
                },
            };
            let view = run_view(&snapshot);
            let bytes = rx_protocol::json::to_vec(&view).unwrap();
            let restored: base::RunView = rx_protocol::json::from_slice(&bytes).unwrap();
            assert_eq!(restored, view);
            let json = rx_protocol::json::to_value(&view).unwrap();
            assert_eq!(json["state"].as_str(), Some(expected));
            assert_eq!(json["revision"].as_str(), Some("18446744073709551615"));
            assert_eq!(
                json["checkpoint"]["activations"][0]["visit"].as_str(),
                Some("9007199254740993")
            );
            assert_eq!(
                json["checkpoint"]["activations"][0]["slots"][0]["operation_id"].as_str(),
                Some(id(4).as_str())
            );
            assert_eq!(
                json["checkpoint"]["payload"]["sha256"].as_str(),
                Some(digest.to_string().as_str())
            );
        }
    }
}
