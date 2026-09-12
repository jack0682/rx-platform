//! Test-only package/report signers. Production code never has these private keys.
use ed25519_dalek::{Signer, SigningKey};
use rx_application::{
    CellConfiguration,
    process_review::{Authority, Job, VerifierKey},
};
use rx_domain::{canonical, types::*};
use rx_package::*;
use rx_process_contract::{compile_input::CompileInput, model::*, package_review::Report};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
pub struct Fixture {
    pub _dir: tempfile::TempDir,
    pub package: PathBuf,
    pub policy_path: PathBuf,
    pub policy_bytes: Vec<u8>,
    pub policy: VerificationPolicy,
    pub store: store::Store,
    pub object: store::ObjectId,
    pub authority: Authority,
    pub resolved: ResolvedProcess,
    pub validator: Digest,
}
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn path(s: &str) -> PackagePath {
    PackagePath::new(s).unwrap()
}
fn bytes(v: &impl serde::Serialize) -> Vec<u8> {
    canonical::bytes(v).unwrap()
}
pub fn fixture(cfg: &CellConfiguration, validator: Digest) -> Fixture {
    fixture_mode(cfg, validator, false)
}
pub fn fixture_mode(cfg: &CellConfiguration, validator: Digest, device: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let package = dir.path().join("package");
    std::fs::create_dir(&package).unwrap();
    let source:ProcessSource=serde_json::from_value(serde_json::json!({"schema":"rx.process-source.v1","process":"test/review","entry":"main","conditions":{},"flows":[{"id":"main","root":"work","nodes":[{"id":"work","body":{"kind":"OPERATION","binding":"load"}}]}]})).unwrap();
    let bindings = BTreeMap::from([(
        name("load"),
        ActionBinding {
            host: cfg.steps[0].host.clone(),
            intent: cfg.steps[0].intent.normalized().unwrap(),
        },
    )]);
    let mut input = CompileInput {
        device_sources: BTreeMap::new(),
        schema: name("rx.process-compile-input.v1"),
        draft: Id::new("00000000-0000-4000-8000-000000000046").unwrap(),
        cell: cfg.id.clone(),
        source_revision: Counter(1),
        binding_revision: Counter(1),
        source_document_digest: canonical::digest("RX-PROCESS-DRAFT-DOCUMENT-v1", &source).unwrap(),
        bindings_digest: canonical::digest("RX-DRAFT-COMPILE-BINDINGS-v1", &bindings).unwrap(),
        catalog_digest: canonical::digest(
            "RX-DRAFT-BINDING-CATALOG-v1",
            &(
                cfg.id.clone(),
                &cfg.definition,
                &cfg.envelope,
                cfg.site_config_digest,
                &cfg.steps,
            ),
        )
        .unwrap(),
        source: serde_json::to_value(&source).unwrap(),
        bindings: bindings.clone(),
    };
    if device {
        use rx_process_contract::compile_input::{
            BindingPlanRef, DeviceSource, bindings_digest, device_action_digest,
        };
        input.schema = name("rx.process-compile-input.v2");
        input.device_sources.insert(
            name("load"),
            DeviceSource {
                plan: BindingPlanRef {
                    id: input.draft.clone(),
                    revision: Counter(2),
                    plan_digest: Digest::from_bytes([91; 32]),
                },
                binding: cfg.steps[0].id.clone(),
                step_digest: canonical::digest("RX-DRAFT-BINDING-STEP-v1", &cfg.steps[0]).unwrap(),
                action_digest: device_action_digest(&input.bindings[&name("load")]).unwrap(),
            },
        );
        input.bindings_digest = bindings_digest(&input.bindings, &input.device_sources).unwrap();
    }
    let contracts = ContractSet {
        base: Digest::from_bytes([1; 32]),
        cell: Digest::from_bytes([2; 32]),
        package_abi: name("rx.package-abi.v1"),
    };
    let target = Target {
        os: OperatingSystem::Linux,
        architecture: Architecture::Arm64,
        ros_distribution: None,
    };
    let recipe = serde_json::json!({"schema":"rx.process-package-recipe.v1","package":"test/review","version":"0.1.0","publisher":"test-only","contracts":contracts,"targets":[target],"dependencies":[],"assets":[]});
    let context = serde_json::json!({"schema":"rx.process-context-requirements.v1","catalog":input.catalog_digest,"profiles":[cfg.steps[0].intent.profile_digest],"site_configurations":[cfg.steps[0].intent.site_config_digest],"calibrations":cfg.steps[0].intent.calibration_digests,"tools":[],"transitions":[],"streams":[]});
    let files = BTreeMap::from([
        (path("authoring/compile-input.json"), bytes(&input)),
        (path("authoring/package-recipe.json"), bytes(&recipe)),
        (path("process/source.json"), bytes(&source)),
        (path("process/bindings.json"), bytes(&bindings)),
        (path("process/context-requirements.json"), bytes(&context)),
    ]);
    let manifest = Manifest {
        schema: name("rx.package.v1"),
        package: name("test/review"),
        version: "0.1.0".parse().unwrap(),
        publisher: name("test-only"),
        contracts: contracts.clone(),
        targets: vec![target.clone()],
        entry: EntryPoint::Process {
            source: path("process/source.json"),
        },
        permissions: vec![
            Permission::ArtifactRead,
            Permission::OperationSubmit {
                operation: name("load"),
            },
        ],
        dependencies: vec![],
        assets: vec![],
        files: files
            .iter()
            .map(|(p, b)| FileEntry {
                path: p.clone(),
                sha256: content_digest(b),
                size_bytes: Counter(b.len() as u64),
                executable: false,
            })
            .collect(),
    };
    let key = SigningKey::from_bytes(&[47; 32]);
    let signer = name("test-package-key");
    let signature = SignatureEnvelope {
        key: signer.clone(),
        signature: key
            .sign(&signing_message(&manifest, &signer).unwrap())
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    };
    let policy_doc = policy::Policy {
        additional_package_abis: Default::default(),
        schema: name("rx.package-verification-policy.v1"),
        contracts,
        target,
        keys: vec![policy::Key {
            id: signer,
            publisher: name("test-only"),
            verifying_key: Digest::from_bytes(key.verifying_key().to_bytes()),
            kinds: BTreeSet::from([PackageKind::Process]),
            permissions: manifest.permissions.iter().cloned().collect(),
        }],
        assets: vec![],
        dependencies: vec![],
    };
    let policy = policy_doc.load().unwrap();
    let policy_bytes = bytes(&policy_doc);
    let policy_path = dir.path().join("policy.json");
    std::fs::write(&policy_path, &policy_bytes).unwrap();
    for (p, b) in &files {
        let file = package.join(p.as_str());
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, b).unwrap();
    }
    std::fs::write(
        package.join("manifest.json"),
        manifest_bytes(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(package.join("manifest.sig.json"), bytes(&signature)).unwrap();
    let verified = verify_package(
        &manifest_bytes(&manifest).unwrap(),
        &bytes(&signature),
        files,
        &policy,
    )
    .unwrap();
    let mut store = store::Store::open(&dir.path().join("store")).unwrap();
    let object = store.put(&verified).unwrap();
    let location = SourceLocation {
        flow: name("main"),
        node: name("work"),
        instantiation: vec![],
    };
    let node = name(&format!(
        "node/{}",
        canonical::digest("RX-PROCESS-NODE-v1", &(&source.process, &location)).unwrap()
    ));
    let resolved = ResolvedProcess {
        schema: name("rx.resolved-process.v1"),
        package_digest: Some(verified.digest()),
        source_digest: content_digest(&bytes(&source)),
        process: source.process,
        root: CompiledNode {
            id: node,
            source: location,
            body: CompiledBody::Operation {
                binding: name("load"),
            },
        },
        bindings,
        conditions: BTreeMap::new(),
    };
    let authority = Authority {
        schema: name("rx.process-verification-authority.v1"),
        keys: vec![VerifierKey {
            id: name("test-verifier-key"),
            public_key: Digest::from_bytes(
                SigningKey::from_bytes(&[53; 32]).verifying_key().to_bytes(),
            ),
            validators: BTreeSet::from([validator]),
        }],
    };
    Fixture {
        _dir: dir,
        package,
        policy_path,
        policy_bytes,
        policy,
        store,
        object,
        authority,
        resolved,
        validator,
    }
}
impl Fixture {
    #[allow(
        dead_code,
        reason = "API integration obtains its report from the real S compiler"
    )]
    pub fn report(&self, job: &Job) -> (Report, SignatureEnvelope, Vec<u8>) {
        let data = bytes(&self.resolved);
        let report = Report {
            schema: name("rx.process-verification-report.v1"),
            request: job.request.clone(),
            validator_digest: self.validator,
            validator_policy_file_digest: content_digest(&self.policy_bytes),
            resolved: Some(ArtifactRef {
                sha256: content_digest(&data),
                schema_id: name("rx.resolved-process.v1"),
                size_bytes: Counter(data.len() as u64),
            }),
            issues: vec![],
        };
        let signature = sign(&report);
        (report, signature, data)
    }
}
pub fn sign(report: &Report) -> SignatureEnvelope {
    let id = name("test-verifier-key");
    SignatureEnvelope {
        key: id.clone(),
        signature: SigningKey::from_bytes(&[53; 32])
            .sign(&report.signing_message(&id).unwrap())
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    }
}
