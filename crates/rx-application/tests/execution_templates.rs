//! Test-only signed declarations. No software review or native execution is asserted.
#[path = "support/execution_template_package.rs"]
mod package;
use package::*;
use rx_application::execution_templates::VerifiedTemplates;
use rx_domain::{canonical, intent::Body, types::*};
use rx_package::*;
fn check(f: &Fixture) -> std::result::Result<VerifiedTemplates, String> {
    VerifiedTemplates::check(&f.stored, &f.registration, &id(1), &n("cell/a"))
}

#[test]
fn signed_template_matches_exact_interface_and_only_the_registered_store_policy() {
    let f = fixture(|_| {}, false);
    let proof = check(&f).unwrap();
    assert_eq!(proof.manifest(), f.stored.object().manifest);
    assert_eq!(proof.signature(), f.stored.object().signature);
    assert!(
        proof
            .dependencies()
            .iter()
            .any(|r| r.sha256 == proof.manifest() && r.schema_id.as_str() == "rx.package.v2")
    );
    assert!(
        proof
            .dependencies()
            .iter()
            .any(|r| r.sha256 == proof.signature())
    );
    for file in &f.stored.package().manifest().files {
        assert!(
            proof
                .dependencies()
                .iter()
                .any(|r| r.sha256 == file.sha256 && r.size_bytes == file.size_bytes)
        );
    }
    let declared = &f.catalog.templates[&n("move")];
    proof
        .matches(&n("move"), &declared.action, &declared.contract)
        .unwrap();
    let mut registration = f.registration.clone();
    registration.store_owner = id(9);
    assert!(VerifiedTemplates::check(&f.stored, &registration, &id(1), &n("cell/a")).is_err());
    registration = f.registration.clone();
    registration.policy_fingerprint = Digest::from_bytes([9; 32]);
    assert!(VerifiedTemplates::check(&f.stored, &registration, &id(1), &n("cell/a")).is_err());
    let mut action = declared.action.clone();
    action.intent.execution_timeout_ms = Counter(999);
    assert!(
        proof
            .matches(&n("move"), &action, &declared.contract)
            .is_err()
    );
    let mut contract = declared.contract.clone();
    contract.parameters.get_mut(&n("force")).unwrap().unit = n("kg");
    assert!(
        proof
            .matches(&n("move"), &declared.action, &contract)
            .is_err()
    );
    assert!(
        proof
            .matches(&n("missing"), &declared.action, &declared.contract)
            .is_err()
    );
}
#[test]
fn signature_or_content_tampering_cannot_produce_the_stored_proof() {
    let f = fixture(|_| {}, false);
    let mut files = f.files.clone();
    files.get_mut(&path("program.json")).unwrap().push(b' ');
    assert!(verify_package(&f.manifest, &f.signature, files, &f.policy).is_err());
    let mut signature: SignatureEnvelope = canonical::decode_json(&f.signature).unwrap();
    signature.signature = "00".repeat(64);
    assert!(
        verify_package(
            &f.manifest,
            &canonical::bytes(&signature).unwrap(),
            f.files.clone(),
            &f.policy
        )
        .is_err()
    );
}
#[test]
fn valid_signature_does_not_excuse_foreign_scope_dangling_artifacts_or_legacy_semantics() {
    assert!(check(&fixture(|_| {}, true)).is_err());
    assert!(check(&fixture(|c| c.installation = id(9), false)).is_err());
    assert!(check(&fixture(|c| c.cell = n("cell/b"), false)).is_err());
    assert!(
        check(&fixture(
            |c| c.environment = rx_process_contract::device_catalog::Environment::Physical,
            false
        ))
        .is_err()
    );
    assert!(
        check(&fixture(
            |c| c.schema = n("rx.device-operation-catalog.v1"),
            false
        ))
        .is_err()
    );
    assert!(
        check(&fixture(
            |c| c.documents.get_mut(&n("family")).unwrap().path = "profile.json".into(),
            false
        ))
        .is_err()
    );
    assert!(
        check(&fixture(
            |c| {
                let Body::Program(p) =
                    &mut c.templates.get_mut(&n("move")).unwrap().action.intent.body
                else {
                    panic!()
                };
                p.program.sha256 = Digest::from_bytes([9; 32]);
            },
            false
        ))
        .is_err()
    );
    assert!(
        check(&fixture(
            |c| {
                let Body::Program(p) =
                    &mut c.templates.get_mut(&n("move")).unwrap().action.intent.body
                else {
                    panic!()
                };
                p.parameter_set.schema_id = n("rx.workflow-parameters.v1");
            },
            false
        ))
        .is_err()
    );
}
