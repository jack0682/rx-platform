use super::*;
use ed25519_dalek::{Signer, SigningKey};
use rx_application::{investigation as i, persistence as p};
use rx_domain::{
    canonical,
    operation::{Disposition, Integrity, Knowledge, Outcome, Phase},
};

struct Investigation {
    f: Fixture,
    actor: Identity,
    operation: Id,
    native: NativeEvidence,
    journal: Id,
    policy: i::Policy,
    procedure: i::Procedure,
    signature: rx_package::SignatureEnvelope,
}
fn fixture_investigation() -> Investigation {
    let mut f = fixture_configured((1, true, false, true, None, false, true), |mut c| {
        for step in &mut c.steps {
            if let CompletionRule::Native { postconditions, .. } = &mut step.completion {
                *postconditions = step.conditions.clone();
            }
        }
        c
    });
    let (work, mut native) = native_started(&mut f);
    native.native_details = Some(NativeEvidenceDetails {
        native_id: Some("original-native-result".into()),
        native_data: None,
    });
    f.app.hold(&f.operator, id().as_str(), &work.cell).unwrap();
    let journal = id();
    f.app
        .ingest_evidence(
            &f.hosts[0],
            EvidenceBatch {
                journal: journal.clone(),
                first: Counter(1),
                records: vec![native.clone()],
            },
        )
        .unwrap();
    let lead = add_identity(&mut f.app, &f.admin, "investigator", &[Role::RecoveryLead]);
    let actor = Identity {
        principal: lead.principal.clone(),
        session: f
            .app
            .authenticated_terminal_user_session(
                &lead.principal,
                id(),
                Counter(1_000_000_000_000),
                Digest::from_bytes([77; 32]),
            )
            .unwrap()
            .id,
        terminal: Some((name("panel/main"), Digest::from_bytes([77; 32]))),
    };
    let procedure=i::Procedure{schema:name("rx.investigation-procedure.v1"),id:name("investigate/unresolved"),revision:Counter(1),title:"Review original result uncertainty".into(),instructions:vec!["Review the retained original receipt and native result; record that the whole operation result remains unknown.".into()],cell:work.cell.clone(),definition:f.configuration.definition.sha256,environment:name("SIMULATION"),profiles:BTreeSet::from([work.intent.profile_digest]),action:i::Action::AbandonInvestigation};
    let key = SigningKey::from_bytes(&[42; 32]);
    let key_id = name("investigation-test");
    let signature = rx_package::SignatureEnvelope {
        key: key_id.clone(),
        signature: key
            .sign(&procedure.signing_message(&key_id).unwrap())
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    };
    let policy = i::Policy {
        schema: name(i::POLICY_SCHEMA),
        keys: vec![i::Key {
            id: key_id,
            public_key: Digest::from_bytes(key.verifying_key().to_bytes()),
            cells: BTreeSet::from([work.cell.clone()]),
            environments: BTreeSet::from([name("SIMULATION")]),
        }],
        procedures: vec![procedure.reference().unwrap()],
    };
    f.app
        .register_investigation_policy(
            policy.clone(),
            rx_package::content_digest(&canonical::bytes(&policy).unwrap()),
        )
        .unwrap();
    Investigation {
        f,
        actor,
        operation: work.operation.id().clone(),
        native,
        journal,
        policy,
        procedure,
        signature,
    }
}
fn repository(f: &Fixture) -> SqliteRepository {
    SqliteRepository::open(f._directory.path().join("platform.db")).unwrap()
}
impl Investigation {
    fn context(&mut self) -> i::Context {
        self.f
            .app
            .investigation_context(&self.actor, &self.operation)
            .unwrap()
    }
    fn submit(&mut self) -> i::AttestSubmit {
        let c = self.context();
        i::AttestSubmit{id:id(),operation:self.operation.clone(),expected_operation_revision:c.work.operation.revision(),context_digest:c.digest().unwrap(),procedure_digest:self.procedure.reference().unwrap().sha256,evidence_ids:c.evidence.keys().cloned().collect(),assertion:i::Assertion::ResultRemainsUnknown,note:"Original native success does not establish the lost postcondition continuity; result remains unknown.".into(),occurred_at:"2026-09-13T01:02:03Z".into()}
    }
    fn validated(&self) -> i::ValidatedProcedure {
        i::ValidatedProcedure::check(
            &self.policy,
            &self.procedure.reference().unwrap(),
            &canonical::bytes(&self.procedure).unwrap(),
            self.signature.clone(),
        )
        .unwrap()
    }
    fn prepared(&mut self, key: &Id, input: i::AttestSubmit) -> i::Prepared {
        let i::Preflight::Verify(t) = self
            .f
            .app
            .prepare_investigation_attestation(&self.actor, key, input)
            .unwrap()
        else {
            panic!("new attestation")
        };
        i::Prepared::new(*t, self.validated()).unwrap()
    }
    fn attest(&mut self) -> i::Attestation {
        let input = self.submit();
        let prepared = self.prepared(&id(), input);
        self.f
            .app
            .commit_investigation_attestation(prepared)
            .unwrap()
    }
    fn disposition(&self, a: &i::Attestation) -> i::RecordDisposition {
        let mut evidence = a.request.evidence_ids.clone();
        evidence.push(a.id.clone());
        i::RecordDisposition {
            operation: self.operation.clone(),
            expected_revision: a.context.work.operation.revision(),
            evidence_ids: evidence,
            procedure_digest: a.procedure_reference.sha256,
            disposition: i::Disposition::Quarantined,
            reason: i::Reason::UnknownOutcome,
        }
    }
    fn prepared_disposition(
        &mut self,
        key: &Id,
        input: i::RecordDisposition,
    ) -> i::PreparedDisposition {
        let i::DispositionPreflight::Verify(t) = self
            .f
            .app
            .prepare_investigation_disposition(&self.actor, key, input)
            .unwrap()
        else {
            panic!("new disposition")
        };
        i::PreparedDisposition::new(*t, self.validated()).unwrap()
    }
    fn invariants(&self) -> Vec<Record> {
        repository(&self.f)
            .snapshot()
            .unwrap()
            .1
            .into_iter()
            .filter(|r| {
                [
                    "cell/",
                    "resource/",
                    "permit/",
                    "mandate/",
                    "run/",
                    "part/",
                    "activation/",
                    "host/",
                    "fact/",
                    "evidence/",
                    "hostreceipt/",
                ]
                .iter()
                .any(|p| r.key.as_str().starts_with(p))
            })
            .collect()
    }
}

#[test]
fn investigation_attestation_and_explicit_t5_preserve_original_native_success_and_all_holdings() {
    let mut f = fixture_investigation();
    let before = f.context();
    assert_eq!(before.work.operation.outcome(), Outcome::None);
    assert_eq!(before.work.operation.knowledge(), Knowledge::Unknown);
    assert!(before.evidence.contains_key(&f.native.id));
    let invariants = f.invariants();
    let outbox = repository(&f.f).pending_outbox(128).unwrap();
    f.f.clock.0.store(1_000_000_000, Ordering::SeqCst); // Historical evidence is not a current sample TTL.
    let a = f.attest();
    assert_eq!(
        f.f.app
            .inspect_work(&f.actor, &f.operation)
            .unwrap()
            .operation
            .outcome(),
        Outcome::None
    );
    let input = f.disposition(&a);
    let prepared = f.prepared_disposition(&id(), input);
    let r = f.f.app.commit_investigation_disposition(prepared).unwrap();
    assert_eq!(r.after.operation.phase(), Phase::Settled);
    assert_eq!(r.after.operation.outcome(), Outcome::Unresolved);
    assert_eq!(r.after.operation.knowledge(), Knowledge::Unknown);
    assert_eq!(r.after.operation.disposition(), Disposition::Quarantined);
    assert!(!r.operation_authorized && !r.resource_release_authorized && !r.resources_released);
    assert_eq!(
        r.attestation.context.evidence[&f.native.id].document.value,
        serde_json::to_value(&f.native).unwrap()
    );
    assert_eq!(f.invariants(), invariants);
    assert_eq!(repository(&f.f).pending_outbox(128).unwrap(), outbox);
    assert!(
        f.f.app
            .investigation_disposition(&f.actor, &f.operation, &r.id)
            .unwrap()
            .current
    );
}

#[test]
fn investigation_blank_publish_probe_does_not_change_context_but_new_original_evidence_does() {
    let mut f = fixture_investigation();
    let before = f.context().digest().unwrap();
    f.f.app
        .ingest_evidence(
            &f.f.hosts[0],
            EvidenceBatch {
                journal: f.journal.clone(),
                first: Counter(2),
                records: vec![],
            },
        )
        .unwrap();
    assert_eq!(f.context().digest().unwrap(), before);
    let mut next = f.native.clone();
    next.id = id();
    f.f.app
        .ingest_evidence(
            &f.f.hosts[0],
            EvidenceBatch {
                journal: f.journal.clone(),
                first: Counter(2),
                records: vec![next],
            },
        )
        .unwrap();
    assert_ne!(f.context().digest().unwrap(), before);
}

#[test]
fn investigation_rejects_wrong_signature_noncanonical_bytes_and_signer_scope() {
    let f = fixture_investigation();
    let reference = f.procedure.reference().unwrap();
    let bytes = canonical::bytes(&f.procedure).unwrap();
    let mut bad = f.signature.clone();
    bad.signature = "00".repeat(64);
    assert!(i::ValidatedProcedure::check(&f.policy, &reference, &bytes, bad).is_err());
    let mut noncanonical = bytes.clone();
    noncanonical.push(b'\n');
    assert!(
        i::ValidatedProcedure::check(&f.policy, &reference, &noncanonical, f.signature.clone())
            .is_err()
    );
    for axis in ["cell", "environment", "key", "reference"] {
        let mut p = f.policy.clone();
        match axis {
            "cell" => p.keys[0].cells = BTreeSet::from([name("other/cell")]),
            "environment" => p.keys[0].environments = BTreeSet::from([name("PHYSICAL")]),
            "key" => p.keys[0].public_key = Digest::from_bytes([7; 32]),
            _ => p.procedures[0].sha256 = Digest::from_bytes([9; 32]),
        };
        assert!(
            i::ValidatedProcedure::check(&p, &reference, &bytes, f.signature.clone()).is_err(),
            "{axis}"
        );
    }
}

#[test]
fn investigation_unknown_ids_assertion_and_human_clock_are_not_accepted_as_evidence() {
    for axis in ["evidence", "revision", "context", "procedure", "time"] {
        let mut f = fixture_investigation();
        let mut input = f.submit();
        match axis {
            "evidence" => input.evidence_ids.push(id()),
            "revision" => input.expected_operation_revision = Counter(1),
            "context" => input.context_digest = Digest::from_bytes([8; 32]),
            "procedure" => input.procedure_digest = Digest::from_bytes([9; 32]),
            _ => input.occurred_at = "2026-09-13T01:02:03-00:00".into(),
        };
        assert!(
            f.f.app
                .prepare_investigation_attestation(&f.actor, &id(), input)
                .is_err(),
            "{axis}"
        );
    }
    let mut f = fixture_investigation();
    let mut value = serde_json::to_value(f.submit()).unwrap();
    value["assertion"] = serde_json::json!("SUCCEEDED");
    assert!(serde_json::from_value::<i::AttestSubmit>(value).is_err());
}

#[test]
fn investigation_commit_rechecks_current_role_policy_and_original_evidence() {
    for axis in ["role", "policy", "evidence"] {
        let mut f = fixture_investigation();
        let input = f.submit();
        let prepared = f.prepared(&id(), input);
        match axis {
            "role" => {
                f.f.app.end_user_session(&f.actor).unwrap();
            }
            "policy" => {
                f.f.app
                    .register_investigation_policy(f.policy.clone(), Digest::from_bytes([99; 32]))
                    .unwrap();
            }
            _ => {
                let mut next = f.native.clone();
                next.id = id();
                f.f.app
                    .ingest_evidence(
                        &f.f.hosts[0],
                        EvidenceBatch {
                            journal: f.journal.clone(),
                            first: Counter(2),
                            records: vec![next],
                        },
                    )
                    .unwrap();
            }
        }
        assert!(
            f.f.app.commit_investigation_attestation(prepared).is_err(),
            "{axis}"
        );
        assert!(
            repository(&f.f)
                .snapshot()
                .unwrap()
                .1
                .iter()
                .all(|r| !r.key.as_str().starts_with("investigation-attestation/"))
        );
    }
}

#[test]
fn investigation_lost_reply_is_recovered_with_current_auth_after_relogin_and_different_key_cannot_replace_t5()
 {
    let mut f = fixture_investigation();
    let input = f.submit();
    let key = id();
    let prepared = f.prepared(&key, input.clone());
    f.f.failure.store(2, Ordering::SeqCst);
    assert!(f.f.app.commit_investigation_attestation(prepared).is_err());
    let old = f.actor.session.clone();
    f.f.app.end_user_session(&f.actor).unwrap();
    f.actor.session =
        f.f.app
            .authenticated_terminal_user_session(
                &f.actor.principal,
                id(),
                Counter(1_000_000_000_000),
                Digest::from_bytes([77; 32]),
            )
            .unwrap()
            .id;
    let i::Preflight::Recorded(a) =
        f.f.app
            .prepare_investigation_attestation(&f.actor, &key, input.clone())
            .unwrap()
    else {
        panic!("cached")
    };
    assert_eq!(a.actor.session, old);
    let d = f.disposition(&a);
    let dkey = id();
    let prepared = f.prepared_disposition(&dkey, d.clone());
    f.f.failure.store(2, Ordering::SeqCst);
    assert!(f.f.app.commit_investigation_disposition(prepared).is_err());
    let mut retry = d.clone();
    retry.expected_revision = Counter(999);
    retry.evidence_ids.reverse();
    let i::DispositionPreflight::Recorded(r) =
        f.f.app
            .prepare_investigation_disposition(&f.actor, &dkey, retry)
            .unwrap()
    else {
        panic!("cached disposition")
    };
    assert_eq!(r.after.operation.outcome(), Outcome::Unresolved);
    assert!(matches!(
        f.f.app
            .prepare_investigation_disposition(&f.actor, &id(), d),
        Err(StoreError::KeyConflict)
    ));
    let mut wrong = input;
    wrong.note.push('!');
    assert!(matches!(
        f.f.app
            .prepare_investigation_attestation(&f.actor, &key, wrong),
        Err(StoreError::KeyConflict)
    ));
    f.f.app.end_user_session(&f.actor).unwrap();
    assert!(
        f.f.app
            .investigation_disposition(&f.actor, &f.operation, &r.id)
            .is_err()
    );
}

#[test]
fn investigation_t5_rollback_preserves_attestation_work_resources_and_outbox() {
    let mut f = fixture_investigation();
    let a = f.attest();
    let input = f.disposition(&a);
    let key = id();
    let prepared = f.prepared_disposition(&key, input.clone());
    let before = repository(&f.f).snapshot().unwrap();
    let outbox = repository(&f.f).pending_outbox(128).unwrap();
    f.f.failure.store(1, Ordering::SeqCst);
    assert!(f.f.app.commit_investigation_disposition(prepared).is_err());
    assert_eq!(repository(&f.f).snapshot().unwrap(), before);
    assert_eq!(repository(&f.f).pending_outbox(128).unwrap(), outbox);
    let prepared = f.prepared_disposition(&key, input);
    f.f.app.commit_investigation_disposition(prepared).unwrap();
}

#[test]
fn investigation_attestation_from_another_principal_or_stale_original_cut_cannot_authorize_t5() {
    for axis in ["principal", "policy", "late-evidence"] {
        let mut f = fixture_investigation();
        let a = f.attest();
        let input = f.disposition(&a);
        match axis {
            "principal" => {
                let lead = add_identity(
                    &mut f.f.app,
                    &f.f.admin,
                    "other-investigator",
                    &[Role::RecoveryLead],
                );
                f.actor = Identity {
                    principal: lead.principal.clone(),
                    session: f
                        .f
                        .app
                        .authenticated_terminal_user_session(
                            &lead.principal,
                            id(),
                            Counter(999_000),
                            Digest::from_bytes([77; 32]),
                        )
                        .unwrap()
                        .id,
                    terminal: Some((name("panel/main"), Digest::from_bytes([77; 32]))),
                };
            }
            "policy" => {
                f.f.app
                    .register_investigation_policy(f.policy.clone(), Digest::from_bytes([98; 32]))
                    .unwrap();
            }
            _ => {
                let mut e = f.native.clone();
                e.id = id();
                f.f.app
                    .ingest_evidence(
                        &f.f.hosts[0],
                        EvidenceBatch {
                            journal: f.journal.clone(),
                            first: Counter(2),
                            records: vec![e],
                        },
                    )
                    .unwrap();
            }
        }
        assert!(
            f.f.app
                .prepare_investigation_disposition(&f.actor, &id(), input)
                .is_err(),
            "{axis}"
        );
        assert_eq!(
            f.f.app
                .inspect_work(&f.actor, &f.operation)
                .unwrap()
                .operation
                .outcome(),
            Outcome::None
        );
    }
}

#[test]
fn investigation_late_native_failure_after_t5_disputes_without_rewriting_the_original_conclusion() {
    let mut f = fixture_investigation();
    let a = f.attest();
    let d = f.disposition(&a);
    let prepared = f.prepared_disposition(&id(), d);
    let r = f.f.app.commit_investigation_disposition(prepared).unwrap();
    let mut next = f.native.clone();
    next.id = id();
    next.status = Integer(1);
    f.f.app
        .ingest_evidence(
            &f.f.hosts[0],
            EvidenceBatch {
                journal: f.journal.clone(),
                first: Counter(2),
                records: vec![next],
            },
        )
        .unwrap();
    let live = f.f.app.inspect_work(&f.actor, &f.operation).unwrap();
    assert_eq!(live.operation.outcome(), Outcome::Unresolved);
    assert_eq!(live.operation.integrity(), Integrity::Disputed);
    assert_eq!(live.operation.disposition(), Disposition::Quarantined);
    let historical =
        f.f.app
            .investigation_disposition(&f.actor, &f.operation, &r.id)
            .unwrap();
    assert!(!historical.current);
    assert_eq!(
        historical.receipt.after.operation.integrity(),
        Integrity::Valid
    );
}

#[test]
fn investigation_t5_commit_rechecks_authority_policy_and_late_evidence_after_off_writer_verification()
 {
    for axis in ["role", "policy", "evidence", "cell"] {
        let mut f = fixture_investigation();
        let a = f.attest();
        let input = f.disposition(&a);
        let prepared = f.prepared_disposition(&id(), input);
        match axis {
            "role" => {
                f.f.app.end_user_session(&f.actor).unwrap();
            }
            "policy" => {
                f.f.app
                    .register_investigation_policy(f.policy.clone(), Digest::from_bytes([97; 32]))
                    .unwrap();
            }
            "cell" => {
                f.f.app
                    .hold(&f.f.operator, id().as_str(), &name("cell/a"))
                    .unwrap();
            }
            _ => {
                let mut e = f.native.clone();
                e.id = id();
                f.f.app
                    .ingest_evidence(
                        &f.f.hosts[0],
                        EvidenceBatch {
                            journal: f.journal.clone(),
                            first: Counter(2),
                            records: vec![e],
                        },
                    )
                    .unwrap();
            }
        }
        assert!(
            f.f.app.commit_investigation_disposition(prepared).is_err(),
            "{axis}"
        );
        assert_eq!(
            f.f.app
                .inspect_work(&f.f.admin, &f.operation)
                .unwrap()
                .operation
                .outcome(),
            Outcome::None
        );
        assert!(
            repository(&f.f)
                .snapshot()
                .unwrap()
                .1
                .iter()
                .all(|r| !r.key.as_str().starts_with("investigation-disposition/"))
        );
    }
}

#[test]
fn investigation_signed_allowed_artifact_must_still_match_the_original_operation_profile() {
    let mut f = fixture_investigation();
    f.procedure.profiles = BTreeSet::from([Digest::from_bytes([99; 32])]);
    f.policy.procedures = vec![f.procedure.reference().unwrap()];
    let key = SigningKey::from_bytes(&[42; 32]);
    f.signature.signature = key
        .sign(&f.procedure.signing_message(&f.signature.key).unwrap())
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    f.f.app
        .register_investigation_policy(
            f.policy.clone(),
            rx_package::content_digest(&canonical::bytes(&f.policy).unwrap()),
        )
        .unwrap();
    let input = f.submit();
    let i::Preflight::Verify(t) =
        f.f.app
            .prepare_investigation_attestation(&f.actor, &id(), input)
            .unwrap()
    else {
        panic!("ticket")
    };
    assert!(i::Prepared::new(*t, f.validated()).is_err());
}

#[test]
fn investigation_full_shared_scope_and_human_role_are_required_before_cache_lookup() {
    let mut f = fixture_investigation();
    let a = f.attest();
    let mut second = f.f.configuration.clone();
    second.id = name("cell/b");
    f.f.app.install_cell(&f.f.admin, second).unwrap();
    let terminal = Terminal {
        id: name("panel/main"),
        certificate_digest: Digest::from_bytes([77; 32]),
        cells: BTreeSet::from([name("cell/a")]),
        active: true,
    };
    f.f.app
        .put_terminal(&f.f.admin, terminal, Some(Counter(1)))
        .unwrap();
    f.actor.session =
        f.f.app
            .authenticated_terminal_user_session(
                &f.actor.principal,
                id(),
                Counter(999_000),
                Digest::from_bytes([77; 32]),
            )
            .unwrap()
            .id;
    assert!(matches!(
        f.f.app
            .investigation_attestation(&f.actor, &f.operation, &a.id),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
    assert!(
        f.f.app
            .investigation_context(&f.f.hosts[0], &f.operation)
            .is_err()
    );
}

#[test]
fn investigation_actual_owner_and_receipt_original_are_verified_and_pages_are_bounded() {
    let mut f = fixture_investigation();
    let first = f.attest();
    let second = f.attest();
    let page =
        f.f.app
            .list_investigation_attestations(&f.actor, &f.operation, None, 1)
            .unwrap();
    assert_eq!(page.items.len(), 1);
    assert!(page.next.is_some());
    let next =
        f.f.app
            .list_investigation_attestations(&f.actor, &f.operation, page.next.as_ref(), 1)
            .unwrap();
    assert_eq!(
        BTreeSet::from([
            page.items[0].attestation.id.clone(),
            next.items[0].attestation.id.clone()
        ]),
        BTreeSet::from([first.id, second.id])
    );
    assert!(
        f.f.app
            .list_investigation_attestations(&f.actor, &f.operation, None, 51)
            .is_err()
    );
    repository(&f.f)
        .transact(|tx| {
            let (rev, _): (_, Name) = p::load(
                tx,
                "evidenceowner",
                &f.native.id,
                "rx.internal.evidence-owner.v1",
            )?;
            p::save(
                tx,
                "evidenceowner",
                &f.native.id,
                Some(rev),
                "rx.internal.evidence-owner.v1",
                &name("other/host"),
            )?;
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        f.f.app.investigation_context(&f.actor, &f.operation),
        Err(StoreError::Integrity(_))
    ));
}

#[test]
fn investigation_service_role_cannot_recover_cached_human_attestation_even_with_recovery_lead_role()
{
    let mut f = fixture_investigation();
    let input = f.submit();
    let key = id();
    let prepared = f.prepared(&key, input.clone());
    f.f.app.commit_investigation_attestation(prepared).unwrap();
    f.f.app
        .put_principal(
            &f.f.admin,
            principal("investigator", &[Role::RecoveryLead, Role::Host]),
            Some(Counter(1)),
        )
        .unwrap();
    assert!(matches!(
        f.f.app
            .prepare_investigation_attestation(&f.actor, &key, input),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
}
