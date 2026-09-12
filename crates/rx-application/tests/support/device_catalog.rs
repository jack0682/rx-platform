use super::*;
use ed25519_dalek::{Signer, SigningKey};
use rx_domain::canonical;
use rx_package::{store::Store, *};
use rx_process_contract::{
    device_catalog::{Catalog, Document, Environment as CatalogEnvironment},
    native_outcome::{NativeConclusion, NativeOutcomeCase, NativeOutcomeTable},
};
pub(super) fn fixture_package(
    f: &Fixture,
    mutate: impl FnOnce(&mut Catalog),
) -> intake_support::Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("incoming");
    let package = root.join("test-package");
    std::fs::create_dir_all(&package).unwrap();
    let intent = f.configuration.steps[0].intent.clone();
    let operations: BTreeMap<_, _> = [(name("load"), intent.clone())].into();
    let outcomes = NativeOutcomeTable {
        schema: name("rx.native-outcome-table.v1"),
        profile_digest: intent.profile_digest,
        completion_rule: intent.completion_rule.clone(),
        cases: vec![NativeOutcomeCase {
            status_schema: name("test/done"),
            statuses: vec![Integer(0)],
            conclusion: NativeConclusion::Succeeded,
        }],
    };
    let mut files = BTreeMap::new();
    let mut documents = BTreeMap::new();
    for (role, schema, value) in [
        (
            "family",
            "test/family",
            serde_json::json!({"schema":"test/family"}),
        ),
        (
            "profile",
            "test/profile",
            serde_json::json!({"schema":"test/profile"}),
        ),
        (
            "adapter",
            "test/adapter",
            serde_json::json!({"schema":"test/adapter"}),
        ),
        (
            "operations",
            "rx.device-operation-map.v1",
            serde_json::to_value(&operations).unwrap(),
        ),
        (
            "outcomes",
            "rx.native-outcome-table.v1",
            serde_json::to_value(&outcomes).unwrap(),
        ),
    ] {
        let path = PackagePath::new(format!("{role}.json")).unwrap();
        let bytes = canonical::bytes(&value).unwrap();
        documents.insert(
            name(role),
            Document {
                path: path.as_str().into(),
                artifact: ArtifactRef {
                    sha256: content_digest(&bytes),
                    schema_id: name(schema),
                    size_bytes: Counter(bytes.len() as u64),
                },
            },
        );
        files.insert(path, bytes);
    }
    let mut catalog = Catalog {
        schema: name("rx.device-operation-catalog.v1"),
        installation: f.app.installation.id.clone(),
        cell: f.configuration.id.clone(),
        target: intent.target.clone(),
        environment: CatalogEnvironment::Simulation,
        profile_digest: intent.profile_digest,
        condition_ids: [name("ready")].into(),
        operations,
        outcomes: Some(outcomes),
        documents,
    };
    mutate(&mut catalog);
    files.insert(
        PackagePath::new("device-catalog.json").unwrap(),
        canonical::bytes(&catalog).unwrap(),
    );
    let signing = SigningKey::from_bytes(&[58; 32]);
    let kid = name("test/device-key");
    let contracts = ContractSet {
        base: Digest::from_bytes([1; 32]),
        cell: Digest::from_bytes([2; 32]),
        package_abi: name("rx.package-abi.v2"),
    };
    let target = Target {
        os: OperatingSystem::Linux,
        architecture: Architecture::Arm64,
        ros_distribution: None,
    };
    let manifest = Manifest {
        schema: name("rx.package.v2"),
        package: name("test/device"),
        version: "1.0.0".parse().unwrap(),
        publisher: name("test/publisher"),
        contracts: contracts.clone(),
        targets: vec![target.clone()],
        entry: EntryPoint::DeviceReference {
            family: PackagePath::new("family.json").unwrap(),
            profiles: vec![PackagePath::new("profile.json").unwrap()],
            adapter: PackagePath::new("adapter.json").unwrap(),
        },
        permissions: vec![Permission::ArtifactRead],
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
    let signature = SignatureEnvelope {
        key: kid.clone(),
        signature: signing
            .sign(&signing_message(&manifest, &kid).unwrap())
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
            id: kid,
            publisher: manifest.publisher.clone(),
            verifying_key: Digest::from_bytes(signing.verifying_key().to_bytes()),
            kinds: [PackageKind::Device].into(),
            permissions: [Permission::ArtifactRead].into(),
        }],
        assets: vec![],
        dependencies: vec![],
    };
    let policy = policy_doc.load().unwrap();
    let policy_bytes = canonical::bytes(&policy_doc).unwrap();
    let m = manifest_bytes(&manifest).unwrap();
    let sig = canonical::bytes(&signature).unwrap();
    for (p, b) in &files {
        std::fs::write(package.join(p.as_str()), b).unwrap();
    }
    std::fs::write(package.join("manifest.json"), &m).unwrap();
    std::fs::write(package.join("manifest.sig.json"), &sig).unwrap();
    let verified = verify_package(&m, &sig, files, &policy).unwrap();
    let mut store = Store::open(&dir.path().join("store")).unwrap();
    let object = store.put(&verified).unwrap();
    intake_support::Fixture {
        _dir: dir,
        import_root: root,
        store,
        policy,
        object,
        policy_bytes,
    }
}
#[test]
fn device_catalog_intake_is_atomic_preserves_source_and_never_changes_active_configuration() {
    for fault in [1, 2] {
        let mut f = fixture_complete(1, true, false, true);
        let p = fixture_package(&f, |_| {});
        let input = intake_input(&mut f, &p);
        let request = id();
        let before = f.app.inspect_cell(&f.admin, &input.cell).unwrap();
        let prepared = intake_prepared(&mut f, &p, &request, input.clone());
        f.failure.store(fault, Ordering::SeqCst);
        assert!(f.app.commit_package_intake(prepared).is_err());
        let receipt = match f
            .app
            .prepare_package_intake(&f.admin, &request, input.clone())
            .unwrap()
        {
            package_intake::Preflight::Recorded(r) => *r,
            package_intake::Preflight::Verify(t) => f
                .app
                .commit_package_intake(
                    package_intake::Prepared::new(
                        *t,
                        p.store.verify_owned(&p.object, &p.policy).unwrap(),
                    )
                    .unwrap(),
                )
                .unwrap(),
        };
        assert!(receipt.device_catalog.is_some());
        let detail = f
            .app
            .package_device_catalog(&f.admin, &input.cell, &input.id)
            .unwrap();
        assert_eq!(detail.object, p.object);
        assert_eq!(
            detail.reference.as_ref().unwrap().sha256,
            content_digest(&canonical::bytes(detail.catalog.as_ref().unwrap()).unwrap())
        );
        assert!(
            detail.review_context_current
                && detail.content_reverification_required
                && detail.manufacturer_validation_required
        );
        assert!(!detail.activation_authorized);
        assert_eq!(
            canonical::bytes(&before).unwrap(),
            canonical::bytes(&f.app.inspect_cell(&f.admin, &input.cell).unwrap()).unwrap()
        );
        assert!(
            f.app
                .package_device_catalog(&f.admin, &name("cell/b"), &input.id)
                .is_err()
        );
        assert!(
            f.app
                .package_device_catalog(&f.operator, &input.cell, &input.id)
                .is_err()
        );
        f.app.configure_package_intake(None).unwrap();
        let historical = f
            .app
            .package_device_catalog(&f.admin, &input.cell, &input.id)
            .unwrap();
        assert!(!historical.review_context_current);
        assert!(historical.catalog.is_some());
    }
}
#[test]
fn signed_catalog_mismatches_cannot_enter_the_writer_as_valid_declarations() {
    for variant in 0..6 {
        let mut f = fixture_complete(1, true, false, true);
        let p = fixture_package(&f, |c| match variant {
            0 => c.cell = name("cell/b"),
            1 => {
                c.documents
                    .get_mut(&name("profile"))
                    .unwrap()
                    .artifact
                    .sha256 = Digest::from_bytes([99; 32])
            }
            2 => c.documents.get_mut(&name("profile")).unwrap().path = "../profile.json".into(),
            3 => {
                c.operations
                    .get_mut(&name("load"))
                    .unwrap()
                    .execution_timeout_ms = Counter(10000)
            }
            4 => c.outcomes.as_mut().unwrap().cases[0].conclusion = NativeConclusion::Failed,
            _ => c.documents.get_mut(&name("adapter")).unwrap().path = "profile.json".into(),
        });
        let input = intake_input(&mut f, &p);
        let package_intake::Preflight::Verify(t) = f
            .app
            .prepare_package_intake(&f.admin, &id(), input)
            .unwrap()
        else {
            panic!()
        };
        assert!(
            package_intake::Prepared::new(*t, p.store.verify_owned(&p.object, &p.policy).unwrap())
                .is_err(),
            "{variant}"
        );
    }
}
#[test]
fn installation_environment_and_expired_tickets_do_not_commit_a_catalog() {
    for variant in 0..3 {
        let mut f = fixture_complete(1, true, false, true);
        if variant == 2 {
            f.admin.session = f
                .app
                .authenticated_session(&name("admin"), id(), expiry(100_000_000_000))
                .unwrap()
                .id;
        }
        let p = fixture_package(&f, |c| match variant {
            0 => c.installation = id(),
            1 => c.environment = CatalogEnvironment::Physical,
            _ => {}
        });
        let input = intake_input(&mut f, &p);
        let prepared = intake_prepared(&mut f, &p, &id(), input.clone());
        if variant == 2 {
            f.clock.0.store(40_000_000_000, Ordering::SeqCst);
        }
        assert!(f.app.commit_package_intake(prepared).is_err());
    }
}
#[test]
fn old_packages_without_catalog_remain_readable_without_claiming_device_validation() {
    let mut f = fixture(1, true);
    let p = intake_support::fixture();
    let input = intake_input(&mut f, &p);
    let prepared = intake_prepared(&mut f, &p, &id(), input.clone());
    f.app.commit_package_intake(prepared).unwrap();
    let detail = f
        .app
        .package_device_catalog(&f.admin, &input.cell, &input.id)
        .unwrap();
    assert!(detail.catalog.is_none() && detail.reference.is_none());
    assert!(!detail.activation_authorized);
}
