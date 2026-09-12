use rx_application::Work;
use rx_domain::{intent::Kind, operation as d};
use rx_protocol::base as w;
use tonic::Status;
pub fn view(work: &Work) -> Result<w::OperationView, Status> {
    if work.intent.kind == Kind::ControlSession {
        return Err(Status::failed_precondition(
            "continuous control view is not enabled",
        ));
    }
    let operation = &work.operation;
    if work
        .intent
        .digest()
        .map_err(|_| Status::data_loss("stored intent is invalid"))?
        != operation.intent_digest()
    {
        return Err(Status::data_loss("work/operation intent mismatch"));
    }
    let reason = if operation.integrity() == d::Integrity::Disputed {
        w::ReasonCode::IntegrityConflict
    } else if operation.knowledge() == d::Knowledge::Unknown
        || operation.outcome() == d::Outcome::Unresolved
    {
        w::ReasonCode::UnknownOutcome
    } else {
        w::ReasonCode::Ok
    };
    let mut evidence_ids = operation
        .evidence_ids()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    evidence_ids.sort();
    Ok(w::OperationView {
        operation_id: operation.id().to_string(),
        revision: operation.revision().0,
        intent_digest: operation.intent_digest().as_bytes().to_vec(),
        phase: match operation.phase() {
            d::Phase::Admitted => w::Phase::Admitted,
            d::Phase::Active => w::Phase::Active,
            d::Phase::Reconciling => w::Phase::Reconciling,
            d::Phase::Settled => w::Phase::Settled,
        } as i32,
        execution_knowledge: match operation.knowledge() {
            d::Knowledge::NotSent => w::Knowledge::NotSent,
            d::Knowledge::MayHaveExecuted => w::Knowledge::MayHaveExecuted,
            d::Knowledge::Accepted => w::Knowledge::Accepted,
            d::Knowledge::Running => w::Knowledge::Running,
            d::Knowledge::Ended => w::Knowledge::Ended,
            d::Knowledge::Unknown => w::Knowledge::Unknown,
        } as i32,
        outcome: match operation.outcome() {
            d::Outcome::None => w::Outcome::None,
            d::Outcome::Succeeded => w::Outcome::Succeeded,
            d::Outcome::Failed => w::Outcome::Failed,
            d::Outcome::Canceled => w::Outcome::Canceled,
            d::Outcome::NotExecuted => w::Outcome::NotExecuted,
            d::Outcome::Unresolved => w::Outcome::Unresolved,
        } as i32,
        integrity: match operation.integrity() {
            d::Integrity::Valid => w::Integrity::Valid,
            d::Integrity::Disputed => w::Integrity::Disputed,
        } as i32,
        disposition: match operation.disposition() {
            d::Disposition::Held => w::Disposition::Held,
            d::Disposition::Quarantined => w::Disposition::Quarantined,
            d::Disposition::Released => w::Disposition::Released,
        } as i32,
        evidence_ids,
        reason: Some(w::Reason {
            code: reason as i32,
            detail: None,
            related_ids: vec![],
        }),
        control_state: None,
        cancel_ids: vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rx_application::CompletionRule;
    use rx_domain::{intent::*, types::*};
    fn name(s: &str) -> Name {
        Name::new(s).unwrap()
    }
    fn id(n: u8) -> Id {
        Id::new(format!("00000000-0000-4000-8000-{n:012}")).unwrap()
    }
    #[test]
    fn unknown_and_late_contradiction_preserve_distinct_result_integrity_and_release_axes() {
        let intent = Intent {
            kind: Kind::EnsureState,
            target: name("sim/fixture"),
            profile_digest: Digest::from_bytes([1; 32]),
            site_config_digest: Digest::from_bytes([2; 32]),
            calibration_digests: vec![],
            resource_set: vec![name("resource/fixture")],
            execution_timeout_ms: Counter(100),
            prepare_validity_ms: Counter(10),
            completion_rule: name("sim/closed"),
            cancel_rule: name("sim/stop"),
            body: Body::Predicate(PredicateGoal {
                predicate_id: name("closed"),
                target: TypedValue::Boolean(true),
                settle_ms: Counter(0),
            }),
        };
        let mut work = Work {
            operation: d::Operation::admitted(id(1), intent.digest().unwrap()),
            intent,
            cell: name("cell/a"),
            run: id(2),
            part: Some(id(3)),
            activation: id(4),
            slot: name("main"),
            host: name("host/sim"),
            permit: id(5),
            invocation: None,
            completion: CompletionRule::Unobservable,
            host_journal: id(6),
            handover_max_age_ns: Counter(100),
        };
        work.operation.sent().unwrap();
        work.operation.lose_continuity().unwrap();
        let unknown = view(&work).unwrap();
        assert_eq!(unknown.execution_knowledge, w::Knowledge::Unknown as i32);
        assert_eq!(unknown.outcome, w::Outcome::None as i32);
        assert_eq!(unknown.disposition, w::Disposition::Quarantined as i32);
        assert_eq!(
            unknown.reason.unwrap().code,
            w::ReasonCode::UnknownOutcome as i32
        );
        work.operation
            .conclude(d::Conclusion {
                outcome: d::Outcome::Succeeded,
                evidence_ids: vec![id(7)],
            })
            .unwrap();
        let succeeded = view(&work).unwrap();
        assert_eq!(succeeded.outcome, w::Outcome::Succeeded as i32);
        assert_eq!(succeeded.disposition, w::Disposition::Quarantined as i32);
        work.operation
            .release(
                d::ReleaseConditions {
                    no_residual_native: true,
                    control_handover_confirmed: true,
                    support_handover_confirmed: true,
                },
                vec![id(8)],
            )
            .unwrap();
        assert_eq!(
            view(&work).unwrap().disposition,
            w::Disposition::Released as i32
        );
        work.operation
            .conclude(d::Conclusion {
                outcome: d::Outcome::Failed,
                evidence_ids: vec![id(9)],
            })
            .unwrap();
        let disputed = view(&work).unwrap();
        assert_eq!(disputed.outcome, w::Outcome::Succeeded as i32);
        assert_eq!(disputed.integrity, w::Integrity::Disputed as i32);
        assert_eq!(disputed.disposition, w::Disposition::Quarantined as i32);
        assert_eq!(
            disputed.reason.as_ref().unwrap().code,
            w::ReasonCode::IntegrityConflict as i32
        );
        let bytes = rx_protocol::json::to_vec(&disputed).unwrap();
        assert_eq!(
            rx_protocol::json::from_slice::<w::OperationView>(&bytes).unwrap(),
            disputed
        );
        work.intent.target = name("substituted/target");
        assert_eq!(view(&work).unwrap_err().code(), tonic::Code::DataLoss);
    }
}
