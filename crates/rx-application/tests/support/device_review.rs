use super::*;
use ed25519_dalek::{Signer, SigningKey};
use rx_application::device_review as dr;
use rx_process_contract::device_review::{Check, ResultKind, Scope};
fn key() -> SigningKey {
    SigningKey::from_bytes(&[65; 32])
}
pub(super) fn setup_for(
    adjust: impl FnOnce(&mut CellConfiguration),
) -> (
    Fixture,
    intake_support::Fixture,
    dr::Authority,
    dr::Job,
    Identity,
) {
    let mut f = fixture_complete(1, true, false, true);
    let before = f.configuration.clone();
    adjust(&mut f.configuration);
    let p = device_catalog_tests::fixture_package(&f, |_| {});
    f.configuration = before;
    let input = intake_input(&mut f, &p);
    let prepared = intake_prepared(&mut f, &p, &id(), input.clone());
    f.app.commit_package_intake(prepared).unwrap();
    let authority = dr::Authority {
        schema: name("rx.device-verification-authority.v1"),
        keys: vec![dr::VerifierKey {
            id: name("test/verifier"),
            public_key: Digest::from_bytes(key().verifying_key().to_bytes()),
            validators: [Digest::from_bytes([65; 32])].into(),
        }],
    };
    f.app
        .configure_device_review(Some(authority.digest().unwrap()))
        .unwrap();
    let job = f
        .app
        .create_device_review(
            &f.admin,
            &id(),
            dr::Create {
                id: id(),
                intake: input.id,
                cell: input.cell,
                configuration_digest: input.configuration_digest,
                policy_generation: input.policy_generation,
            },
        )
        .unwrap();
    let admin = f.admin.clone();
    let reviewer = add_identity(&mut f.app, &admin, "device-reviewer", &[Role::Verifier]);
    (f, p, authority, job, reviewer)
}
pub(super) fn setup() -> (
    Fixture,
    intake_support::Fixture,
    dr::Authority,
    dr::Job,
    Identity,
) {
    setup_for(|_| {})
}
pub(super) fn report(job: &dr::Job, passed: bool) -> dr::Report {
    dr::Report {
        schema: name("rx.device-verification-report.v1"),
        request: job.request.clone(),
        validator_digest: Digest::from_bytes([65; 32]),
        validator_policy_file_digest: Digest::from_bytes([66; 32]),
        scope: Scope::DevicePackageSoftware,
        checks: [
            (Check::ContentSignature, ResultKind::Passed),
            (Check::CatalogRequestBinding, ResultKind::Passed),
            (
                Check::DeviceSourceConsistency,
                if passed {
                    ResultKind::Passed
                } else {
                    ResultKind::Failed
                },
            ),
        ]
        .into(),
        issues: if passed {
            vec![]
        } else {
            vec![rx_process_contract::package_review::Issue {
                code: name("SOURCE_FAILED"),
                location: "device".into(),
                detail: "test negative".into(),
            }]
        },
    }
}
fn signature(r: &dr::Report) -> rx_package::SignatureEnvelope {
    let id = name("test/verifier");
    rx_package::SignatureEnvelope {
        key: id.clone(),
        signature: key()
            .sign(&r.signing_message(&id).unwrap())
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    }
}
pub(super) fn verified(
    job: &dr::Job,
    p: &intake_support::Fixture,
    a: &dr::Authority,
    r: dr::Report,
) -> dr::Validated {
    let sig = signature(&r);
    dr::Validated::check(
        job,
        p.store.verify_owned(&p.object, &p.policy).unwrap(),
        a,
        r,
        sig,
    )
    .unwrap()
}
pub(super) fn submit(
    f: &mut Fixture,
    p: &intake_support::Fixture,
    a: &dr::Authority,
    j: &dr::Job,
    expected: Option<Counter>,
    passed: bool,
) -> dr::Version {
    let r = report(j, passed);
    let input = dr::Submit {
        review: j.request.id.clone(),
        cell: j.request.cell.clone(),
        expected,
        directory: rx_package::PackagePath::new("report").unwrap(),
        report_digest: r.digest().unwrap(),
    };
    let dr::Preflight::Verify(t) = f.app.prepare_device_report(&f.admin, &id(), input).unwrap()
    else {
        panic!()
    };
    f.app
        .commit_device_report(dr::Prepared::new(*t, verified(j, p, a, r)).unwrap())
        .unwrap()
}
pub(super) fn decision(v: &dr::Version, expected: Option<Counter>) -> dr::Decide {
    dr::Decide {
        review: v.review.clone(),
        cell: v.cell.clone(),
        report_revision: v.revision,
        review_digest: v.review_digest,
        expected,
        choice: dr::Choice::Approve,
        note: "source and scope reviewed".into(),
    }
}
#[test]
fn independent_device_approval_requires_exact_latest_report_and_leaves_cell_unchanged() {
    let (mut f, p, a, j, reviewer) = setup();
    let before = f.app.inspect_cell(&f.admin, &j.request.cell).unwrap();
    let v = submit(&mut f, &p, &a, &j, None, true);
    let input = decision(&v, None);
    assert!(
        f.app
            .prepare_device_decision(&f.admin, &id(), input.clone())
            .is_err()
    );
    let dr::DecisionPreflight::Verify(t) = f
        .app
        .prepare_device_decision(&reviewer, &id(), input.clone())
        .unwrap()
    else {
        panic!()
    };
    let d = f
        .app
        .commit_device_decision(
            dr::PreparedDecision::approve(*t, verified(&j, &p, &a, v.report.clone())).unwrap(),
        )
        .unwrap();
    assert_eq!(d.scope.as_str(), "DEVICE_PACKAGE_SOFTWARE");
    let detail = f
        .app
        .device_review(&reviewer, &j.request.cell, &j.request.id, None)
        .unwrap();
    assert!(detail.approval_matches_current_review);
    assert!(!detail.activation_authorized);
    assert_eq!(
        rx_domain::canonical::bytes(&before).unwrap(),
        rx_domain::canonical::bytes(&f.app.inspect_cell(&f.admin, &j.request.cell).unwrap())
            .unwrap()
    );
    let next = submit(&mut f, &p, &a, &j, Some(v.revision), true);
    assert!(
        !f.app
            .device_review(&reviewer, &j.request.cell, &j.request.id, None)
            .unwrap()
            .approval_matches_current_review
    );
    assert!(
        f.app
            .prepare_device_decision(
                &reviewer,
                &id(),
                dr::Decide {
                    expected: Some(d.revision),
                    ..input
                }
            )
            .is_err()
    );
    assert!(
        !f.app
            .device_review(&reviewer, &j.request.cell, &j.request.id, Some(v.revision))
            .unwrap()
            .is_latest
    );
    assert_eq!(next.revision, Counter(2));
}
#[test]
fn signature_validator_request_and_catalog_mismatch_cannot_be_recorded_as_verified() {
    let (f, p, a, j, _) = setup();
    for variant in 0..5 {
        let mut r = report(&j, true);
        let mut authority = a.clone();
        match variant {
            0 => r.request.id = id(),
            1 => r.request.catalog.sha256 = Digest::from_bytes([88; 32]),
            2 => r.validator_digest = Digest::from_bytes([99; 32]),
            3 => authority.keys.clear(),
            _ => {
                r.checks.remove(&Check::DeviceSourceConsistency);
            }
        }
        let sig = if variant == 4 {
            signature(&report(&j, true))
        } else {
            signature(&r)
        };
        assert!(
            dr::Validated::check(
                &j,
                p.store.verify_owned(&p.object, &p.policy).unwrap(),
                &authority,
                r,
                sig
            )
            .is_err()
        );
    }
    assert!(f.app.installation.id == j.request.installation);
}
#[test]
fn failed_checks_are_recorded_but_not_approved_and_rejection_survives_policy_revocation() {
    let (mut f, p, a, j, reviewer) = setup();
    let v = submit(&mut f, &p, &a, &j, None, false);
    assert!(!v.ready_for_software_approval);
    let input = decision(&v, None);
    assert!(
        f.app
            .prepare_device_decision(&reviewer, &id(), input.clone())
            .is_err()
    );
    f.app.configure_device_review(None).unwrap();
    let dr::DecisionPreflight::Verify(t) = f
        .app
        .prepare_device_decision(
            &reviewer,
            &id(),
            dr::Decide {
                choice: dr::Choice::Reject,
                ..input
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let d = f
        .app
        .commit_device_decision(dr::PreparedDecision::reject(*t).unwrap())
        .unwrap();
    assert_eq!(d.choice, dr::Choice::Reject);
}
#[test]
fn report_and_decision_commit_faults_recover_one_original_version_and_decision() {
    for fault in [1, 2] {
        let (mut f, p, a, j, reviewer) = setup();
        let r = report(&j, true);
        let key = id();
        let input = dr::Submit {
            review: j.request.id.clone(),
            cell: j.request.cell.clone(),
            expected: None,
            directory: rx_package::PackagePath::new("report").unwrap(),
            report_digest: r.digest().unwrap(),
        };
        let dr::Preflight::Verify(t) = f
            .app
            .prepare_device_report(&f.admin, &key, input.clone())
            .unwrap()
        else {
            panic!()
        };
        let prepared = dr::Prepared::new(*t, verified(&j, &p, &a, r.clone())).unwrap();
        f.failure.store(fault, Ordering::SeqCst);
        assert!(f.app.commit_device_report(prepared).is_err());
        let v = match f.app.prepare_device_report(&f.admin, &key, input).unwrap() {
            dr::Preflight::Recorded(v) => *v,
            dr::Preflight::Verify(t) => f
                .app
                .commit_device_report(dr::Prepared::new(*t, verified(&j, &p, &a, r)).unwrap())
                .unwrap(),
        };
        assert_eq!(v.revision, Counter(1));
        let input = decision(&v, None);
        let key = id();
        let dr::DecisionPreflight::Verify(t) = f
            .app
            .prepare_device_decision(&reviewer, &key, input.clone())
            .unwrap()
        else {
            panic!()
        };
        let prepared =
            dr::PreparedDecision::approve(*t, verified(&j, &p, &a, v.report.clone())).unwrap();
        f.failure.store(fault, Ordering::SeqCst);
        assert!(f.app.commit_device_decision(prepared).is_err());
        let d = match f
            .app
            .prepare_device_decision(&reviewer, &key, input.clone())
            .unwrap()
        {
            dr::DecisionPreflight::Recorded(d) => *d,
            dr::DecisionPreflight::Verify(t) => f
                .app
                .commit_device_decision(
                    dr::PreparedDecision::approve(*t, verified(&j, &p, &a, v.report.clone()))
                        .unwrap(),
                )
                .unwrap(),
        };
        assert_eq!(d.revision, Counter(1));
        f.app.configure_device_review(None).unwrap();
        assert!(matches!(
            f.app
                .prepare_device_decision(&reviewer, &key, input)
                .unwrap(),
            dr::DecisionPreflight::Recorded(_)
        ));
        assert!(
            !f.app
                .device_review(&reviewer, &j.request.cell, &j.request.id, None)
                .unwrap()
                .approval_matches_current_review
        );
    }
}
#[test]
fn fresh_proof_loses_applicability_when_authority_or_report_changes_before_commit() {
    for changed in [false, true] {
        let (mut f, p, a, j, reviewer) = setup();
        let v = submit(&mut f, &p, &a, &j, None, true);
        let input = decision(&v, None);
        let dr::DecisionPreflight::Verify(t) = f
            .app
            .prepare_device_decision(&reviewer, &id(), input)
            .unwrap()
        else {
            panic!()
        };
        let prepared = dr::PreparedDecision::approve(*t, verified(&j, &p, &a, v.report)).unwrap();
        if changed {
            submit(&mut f, &p, &a, &j, Some(Counter(1)), true);
        } else {
            f.app.configure_device_review(None).unwrap();
        }
        assert!(f.app.commit_device_decision(prepared).is_err());
    }
}

#[test]
fn list_returns_bounded_summaries_instead_of_copying_large_reports() {
    let (mut f, p, a, j, reviewer) = setup();
    let mut r = report(&j, false);
    r.issues = vec![
        rx_process_contract::package_review::Issue {
            code: name("SOURCE_FAILED"),
            location: "device".into(),
            detail: "x".repeat(2000)
        };
        32
    ];
    let input = dr::Submit {
        review: j.request.id.clone(),
        cell: j.request.cell.clone(),
        expected: None,
        directory: rx_package::PackagePath::new("report").unwrap(),
        report_digest: r.digest().unwrap(),
    };
    let dr::Preflight::Verify(t) = f.app.prepare_device_report(&f.admin, &id(), input).unwrap()
    else {
        panic!()
    };
    f.app
        .commit_device_report(dr::Prepared::new(*t, verified(&j, &p, &a, r)).unwrap())
        .unwrap();
    let page = f
        .app
        .device_reviews(&reviewer, &j.request.cell, &j.request.intake, None)
        .unwrap();
    assert_eq!(page.reviews.len(), 1);
    assert_eq!(page.reviews[0].report_revision, Some(Counter(1)));
    assert!(!page.reviews[0].ready_for_software_approval);
    let value = serde_json::to_value(&page).unwrap();
    assert!(value["reviews"][0].get("report").is_none());
    assert!(rx_domain::canonical::bytes(&page).unwrap().len() < 4096);
    let detail = f
        .app
        .device_review(&reviewer, &j.request.cell, &j.request.id, None)
        .unwrap();
    assert_eq!(detail.version.unwrap().report.issues.len(), 32);
}
