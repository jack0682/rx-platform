//! Test-only signed declarations. No software review or native execution is asserted.
use ed25519_dalek::{Signer, SigningKey};
use rx_application::{execution_templates::CATALOG_PATH, package_intake::Registration};
use rx_domain::{
    canonical,
    intent::{Body, Intent, Kind, ProgramGoal},
    types::*,
};
use rx_package::*;
use rx_process_contract::{ActionBinding, execution_v2::*};
use std::collections::BTreeMap;

pub(super) fn n(s: &str) -> Name {
    Name::new(s).unwrap()
}
pub(super) fn id(i: u8) -> Id {
    Id::new(format!("00000000-0000-4000-8000-{i:012}")).unwrap()
}
pub(super) fn path(s: &str) -> PackagePath {
    PackagePath::new(s).unwrap()
}
pub(super) fn artifact(schema: &str, bytes: &[u8]) -> ArtifactRef {
    ArtifactRef {
        schema_id: n(schema),
        sha256: content_digest(bytes),
        size_bytes: Counter(bytes.len() as u64),
    }
}
#[allow(dead_code)] // Shared signing fixture; each suite consumes different evidence fields.
pub(super) struct Fixture {
    pub(super) _directory: tempfile::TempDir,
    pub(super) store: store::Store,
    pub(super) stored: store::StoredPackage,
    pub(super) registration: Registration,
    pub(super) catalog: TemplateCatalog,
    pub(super) manifest: Vec<u8>,
    pub(super) signature: Vec<u8>,
    pub(super) files: BTreeMap<PackagePath, Vec<u8>>,
    pub(super) policy: VerificationPolicy,
}
pub(super) fn fixture(
    change: impl FnOnce(&mut TemplateCatalog),
    omit_declaration: bool,
) -> Fixture {
    let mut files = BTreeMap::from([
        (path("program.json"), b"program".to_vec()),
        (path("template.json"), b"template".to_vec()),
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
    assets.sort_by_key(|r| (r.sha256, r.schema_id.clone()));
    assets.dedup();
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
    let directory = tempfile::tempdir().unwrap();
    let incoming = directory.path().join("incoming/templates");
    std::fs::create_dir_all(&incoming).unwrap();
    for (path, bytes) in &files {
        std::fs::write(incoming.join(path.as_str()), bytes).unwrap();
    }
    let policy_doc = policy::Policy {
        additional_package_abis: Default::default(),
        schema: n("rx.package-verification-policy.v1"),
        contracts: contracts.clone(),
        target: target.clone(),
        keys: vec![policy::Key {
            id: key_id.clone(),
            publisher: n("test-only"),
            verifying_key: Digest::from_bytes(key.verifying_key().to_bytes()),
            kinds: [PackageKind::Device].into(),
            permissions: [Permission::ArtifactRead].into(),
        }],
        assets: assets
            .iter()
            .map(|r| policy::Asset {
                reference: r.clone(),
                path: incoming.join(
                    files
                        .iter()
                        .find(|(_, bytes)| content_digest(bytes) == r.sha256)
                        .map(|(p, _)| p.as_str())
                        .unwrap_or("missing-external-asset"),
                ),
            })
            .collect(),
        dependencies: vec![],
    };
    // Some negative cases deliberately pin an external asset not present in this package.
    // Only positive fixtures are used by the file-pinned API worker.
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
        assets: assets.into_iter().map(|r| (r.sha256, r)).collect(),
        max_files: 32,
        max_content_bytes: 4 * 1024 * 1024,
    };
    let policy_bytes = canonical::bytes(&policy_doc).unwrap();
    let manifest = manifest_bytes(&manifest).unwrap();
    let signature = canonical::bytes(&signature).unwrap();
    let package = verify_package(&manifest, &signature, files.clone(), &policy).unwrap();
    let mut store = store::Store::open(&directory.path().join("store")).unwrap();
    let object = store.put(&package).unwrap();
    let stored = store.verify_owned(&object, &policy).unwrap();
    let registration = Registration {
        generation: id(2),
        store_owner: store.owner().clone(),
        policy_fingerprint: policy.fingerprint().unwrap(),
        policy_file_digest: content_digest(&policy_bytes),
    };
    let incoming = directory.path().join("incoming/templates");
    std::fs::create_dir_all(&incoming).unwrap();
    for (path, bytes) in &files {
        std::fs::write(incoming.join(path.as_str()), bytes).unwrap();
    }
    std::fs::write(incoming.join("manifest.json"), &manifest).unwrap();
    std::fs::write(incoming.join("manifest.sig.json"), &signature).unwrap();
    std::fs::write(directory.path().join("policy.json"), policy_bytes).unwrap();
    Fixture {
        store,
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
