//! Test-only signed declarations. No software review or native execution is asserted.
use ed25519_dalek::{Signer, SigningKey};
use rx_application::{
    execution_templates::{CATALOG_PATH, VerifiedTemplates},
    package_intake::Registration,
};
use rx_domain::{
    canonical,
    intent::{Body, Intent, Kind, ProgramGoal},
    types::*,
};
use rx_package::*;
use rx_process_contract::{ActionBinding, execution_v2::*};
use std::collections::BTreeMap;

fn n(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id(i: u8) -> Id {
    Id::new(format!("00000000-0000-4000-8000-{i:012}")).unwrap()
}
fn path(s: &str) -> PackagePath {
    PackagePath::new(s).unwrap()
}
fn artifact(schema: &str, bytes: &[u8]) -> ArtifactRef {
    ArtifactRef {
        schema_id: n(schema),
        sha256: content_digest(bytes),
        size_bytes: Counter(bytes.len() as u64),
    }
}
struct Fixture {
    _directory: tempfile::TempDir,
    stored: store::StoredPackage,
    registration: Registration,
    catalog: TemplateCatalog,
    manifest: Vec<u8>,
    signature: Vec<u8>,
    files: BTreeMap<PackagePath, Vec<u8>>,
    policy: VerificationPolicy,
}
fn fixture(change: impl FnOnce(&mut TemplateCatalog), omit_declaration: bool) -> Fixture {
    let mut files = BTreeMap::from([
        (path("program.json"), b"{\"test_program\":true}".to_vec()),
        (path("template.json"), b"{\"template_only\":true}".to_vec()),
    ]);
    let mut documents = BTreeMap::new();
    for role in ["family", "profile", "adapter"] {
        let file = format!("{role}.json");
        let bytes = format!("{{\"test_document\":\"{role}\"}}").into_bytes();
        documents.insert(
            n(role),
            TemplateDocument {
                path: file.clone(),
                artifact: artifact(&format!("test.{role}.v1"), &bytes),
            },
        );
        files.insert(path(&file), bytes);
    }
    let mut catalog = TemplateCatalog {
        schema: n(TEMPLATE_CATALOG_SCHEMA),
        installation: id(1),
        cell: n("cell/a"),
        environment: rx_process_contract::device_catalog::Environment::Simulation,
        documents,
        templates: BTreeMap::from([(
            n("move"),
            TemplateDeclaration {
                action: ActionBinding {
                    host: n("sim/host"),
                    intent: Intent {
                        kind: Kind::FiniteAction,
                        target: n("device"),
                        profile_digest: Digest::from_bytes([1; 32]),
                        site_config_digest: Digest::from_bytes([2; 32]),
                        calibration_digests: vec![],
                        resource_set: vec![n("resource")],
                        execution_timeout_ms: Counter(1000),
                        prepare_validity_ms: Counter(100),
                        completion_rule: n("done"),
                        cancel_rule: n("stop"),
                        body: Body::Program(ProgramGoal {
                            program: artifact("test.program.v1", &files[&path("program.json")]),
                            parameter_set: artifact(
                                PARAMETER_SCHEMA,
                                &files[&path("template.json")],
                            ),
                        }),
                    },
                },
                contract: NodeContract {
                    implementation: "simulation".into(),
                    version: "1".into(),
                    primitive: n("move"),
                    parameters: BTreeMap::from([(
                        n("force"),
                        ParameterContract {
                            unit: n("N"),
                            value_type: rx_domain::definition::ValueType::Number,
                            frame: None,
                        },
                    )]),
                },
            },
        )]),
    };
    change(&mut catalog);
    let catalog_bytes = canonical::bytes(&catalog).unwrap();
    let mut assets = catalog
        .documents
        .values()
        .map(|d| d.artifact.clone())
        .collect::<Vec<_>>();
    for declaration in catalog.templates.values() {
        let Body::Program(p) = &declaration.action.intent.body else {
            panic!()
        };
        assets.extend([p.program.clone(), p.parameter_set.clone()]);
    }
    assets.push(artifact(TEMPLATE_CATALOG_SCHEMA, &catalog_bytes));
    if !omit_declaration {
        files.insert(path(CATALOG_PATH), catalog_bytes);
    }
    let contracts = ContractSet {
        base: Digest::from_bytes([3; 32]),
        cell: Digest::from_bytes([4; 32]),
        package_abi: n("rx.package-abi.v2"),
    };
    let target = Target {
        os: OperatingSystem::Linux,
        architecture: Architecture::Arm64,
        ros_distribution: None,
    };
    let manifest = Manifest {
        schema: n("rx.package.v2"),
        package: n("test/templates"),
        version: "0.1.0".parse().unwrap(),
        publisher: n("test-only"),
        contracts: contracts.clone(),
        targets: vec![target.clone()],
        entry: EntryPoint::DeviceReference {
            family: path("family.json"),
            profiles: vec![path("profile.json")],
            adapter: path("adapter.json"),
        },
        permissions: vec![Permission::ArtifactRead],
        dependencies: vec![],
        assets: assets.clone(),
        files: files
            .iter()
            .map(|(path, bytes)| FileEntry {
                path: path.clone(),
                sha256: content_digest(bytes),
                size_bytes: Counter(bytes.len() as u64),
                executable: false,
            })
            .collect(),
    };
    let key = SigningKey::from_bytes(&[47; 32]);
    let key_id = n("test-key");
    let signature = SignatureEnvelope {
        key: key_id.clone(),
        signature: key
            .sign(&signing_message(&manifest, &key_id).unwrap())
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    };
    let policy = VerificationPolicy {
        additional_package_abis: Default::default(),
        publishers: BTreeMap::from([(
            key_id,
            TrustedPublisher {
                publisher: n("test-only"),
                verifying_key: key.verifying_key().to_bytes(),
                kinds: [PackageKind::Device].into(),
                permissions: [Permission::ArtifactRead].into(),
            },
        )]),
        contracts,
        target,
        dependencies: BTreeMap::new(),
        assets: assets.into_iter().map(|a| (a.sha256, a)).collect(),
        max_files: 32,
        max_content_bytes: 1024 * 1024,
    };
    let manifest = manifest_bytes(&manifest).unwrap();
    let signature = canonical::bytes(&signature).unwrap();
    let package = verify_package(&manifest, &signature, files.clone(), &policy).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut store = store::Store::open(&directory.path().join("store")).unwrap();
    let object = store.put(&package).unwrap();
    let stored = store.verify_owned(&object, &policy).unwrap();
    let registration = Registration {
        generation: id(2),
        store_owner: store.owner().clone(),
        policy_fingerprint: policy.fingerprint().unwrap(),
        policy_file_digest: Digest::from_bytes([5; 32]),
    };
    Fixture {
        _directory: directory,
        stored,
        registration,
        catalog,
        manifest,
        signature,
        files,
        policy,
    }
}
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
