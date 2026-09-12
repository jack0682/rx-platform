use ed25519_dalek::{Signer, SigningKey};
use rx_domain::types::*;
use rx_package::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn path(s: &str) -> PackagePath {
    PackagePath::new(s).unwrap()
}
fn contracts() -> ContractSet {
    ContractSet {
        base: Digest::from_bytes([1; 32]),
        cell: Digest::from_bytes([2; 32]),
        package_abi: name("rx.package-abi.v1"),
    }
}
fn target() -> Target {
    Target {
        os: OperatingSystem::Linux,
        architecture: Architecture::Amd64,
        ros_distribution: None,
    }
}
fn fixture() -> (
    Manifest,
    BTreeMap<PackagePath, Vec<u8>>,
    VerificationPolicy,
    SigningKey,
) {
    let key = SigningKey::from_bytes(&[42; 32]);
    let data = b"{\"schema\":\"rx.process-source.v1\"}".to_vec();
    let manifest = Manifest {
        schema: name("rx.package.v1"),
        package: name("example/tending"),
        version: "0.1.0".parse().unwrap(),
        publisher: name("example"),
        contracts: contracts(),
        targets: vec![target()],
        entry: EntryPoint::Process {
            source: path("process.json"),
        },
        permissions: vec![Permission::ArtifactRead],
        dependencies: vec![],
        assets: vec![],
        files: vec![FileEntry {
            path: path("process.json"),
            sha256: content_digest(&data),
            size_bytes: Counter(data.len() as u64),
            executable: false,
        }],
    };
    let trusted = TrustedPublisher {
        publisher: name("example"),
        verifying_key: key.verifying_key().to_bytes(),
        kinds: [PackageKind::Process, PackageKind::Ui, PackageKind::Device]
            .into_iter()
            .collect(),
        permissions: [
            Permission::ArtifactRead,
            Permission::NativeEndpoint { role: name("arm") },
            Permission::UiPanelRead {
                topic: name("overview"),
            },
        ]
        .into_iter()
        .collect(),
    };
    let policy = VerificationPolicy {
        additional_package_abis: Default::default(),
        publishers: BTreeMap::from([(name("test-key"), trusted)]),
        contracts: contracts(),
        target: target(),
        dependencies: BTreeMap::new(),
        assets: BTreeMap::new(),
        max_files: 128,
        max_content_bytes: 64 * 1024 * 1024,
    };
    (
        manifest,
        BTreeMap::from([(path("process.json"), data)]),
        policy,
        key,
    )
}
fn signed(manifest: &Manifest, key: &SigningKey) -> Vec<u8> {
    let key_id = name("test-key");
    let signature = key.sign(&signing_message(manifest, &key_id).unwrap());
    serde_json::to_vec(&SignatureEnvelope {
        key: key_id,
        signature: signature
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    })
    .unwrap()
}
fn check(
    manifest: &Manifest,
    files: BTreeMap<PackagePath, Vec<u8>>,
    policy: &VerificationPolicy,
    key: &SigningKey,
) -> Result<VerifiedPackage> {
    verify_package(
        &serde_json::to_vec(manifest).unwrap(),
        &signed(manifest, key),
        files,
        policy,
    )
}

#[test]
fn device_reference_v2_has_explicit_version_and_no_executable_payload() {
    let (mut manifest, _, mut policy, key) = fixture();
    let files = ["family.json", "profile.json", "adapter.json"]
        .into_iter()
        .map(|p| (path(p), b"{}".to_vec()))
        .collect::<BTreeMap<_, _>>();
    manifest.files = files
        .iter()
        .map(|(p, bytes)| FileEntry {
            path: p.clone(),
            sha256: content_digest(bytes),
            size_bytes: Counter(bytes.len() as u64),
            executable: false,
        })
        .collect();
    manifest.schema = name("rx.package.v2");
    manifest.contracts.package_abi = name("rx.package-abi.v2");
    policy.contracts = manifest.contracts.clone();
    manifest.entry = EntryPoint::DeviceReference {
        family: path("family.json"),
        profiles: vec![path("profile.json")],
        adapter: path("adapter.json"),
    };
    let verified = check(&manifest, files.clone(), &policy, &key).unwrap();
    assert_eq!(verified.manifest().entry.kind(), PackageKind::Device);
    let mut bad = manifest.clone();
    bad.schema = name("rx.package.v1");
    assert!(check(&bad, files.clone(), &policy, &key).is_err());
    let mut bad = manifest.clone();
    bad.contracts.package_abi = name("rx.package-abi.v1");
    let mut p = policy.clone();
    p.contracts = bad.contracts.clone();
    assert!(check(&bad, files.clone(), &p, &key).is_err());
    let mut bad = manifest.clone();
    bad.files[0].executable = true;
    assert!(matches!(
        check(&bad, files.clone(), &policy, &key),
        Err(Error::Untrusted)
    ));
    let mut legacy = manifest.clone();
    legacy.schema = name("rx.package.v1");
    legacy.contracts.package_abi = name("rx.package-abi.v1");
    policy.contracts = legacy.contracts.clone();
    legacy.entry = EntryPoint::Device {
        family: path("family.json"),
        profiles: vec![path("profile.json")],
        adapter: path("adapter.json"),
    };
    assert!(check(&legacy, files.clone(), &policy, &key).is_err());
    legacy
        .files
        .iter_mut()
        .find(|f| f.path == path("adapter.json"))
        .unwrap()
        .executable = true;
    check(&legacy, files, &policy, &key).unwrap();
}
#[test]
fn signed_content_is_immutable_and_signature_is_bound_to_key_identity() {
    let (manifest, files, mut policy, key) = fixture();
    let original = files[&path("process.json")].clone();
    let verified = check(&manifest, files.clone(), &policy, &key).unwrap();
    assert_eq!(verified.file(&path("process.json")).unwrap(), original);
    let mut changed = files;
    changed.get_mut(&path("process.json")).unwrap().push(0);
    assert!(matches!(
        check(&manifest, changed, &policy, &key),
        Err(Error::Content(_))
    ));
    let mut envelope: SignatureEnvelope = serde_json::from_slice(&signed(&manifest, &key)).unwrap();
    policy
        .publishers
        .insert(name("alias"), policy.publishers[&name("test-key")].clone());
    envelope.key = name("alias");
    assert!(matches!(
        verify_package(
            &serde_json::to_vec(&manifest).unwrap(),
            &serde_json::to_vec(&envelope).unwrap(),
            BTreeMap::from([(path("process.json"), original)]),
            &policy
        ),
        Err(Error::Signature)
    ));
}
#[test]
fn closed_paths_and_inventory_reject_traversal_aliases_and_hidden_files() {
    for value in [
        "../secret",
        "/absolute",
        "a/../b",
        "a//b",
        "C:secret",
        "dir\\x",
        "CON.txt",
        "a/lpt1",
        "x/",
        "a.",
        "a b",
    ] {
        assert!(PackagePath::new(value).is_err(), "{value}");
    }
    let (mut manifest, mut files, policy, key) = fixture();
    files.insert(path("extra.txt"), vec![]);
    assert!(matches!(
        check(&manifest, files.clone(), &policy, &key),
        Err(Error::Content(_))
    ));
    files.remove(&path("extra.txt"));
    manifest.files.push(FileEntry {
        path: path("PROCESS.json"),
        sha256: manifest.files[0].sha256,
        size_bytes: manifest.files[0].size_bytes,
        executable: false,
    });
    assert!(matches!(
        check(&manifest, files, &policy, &key),
        Err(Error::Invalid(_))
    ));
}
#[test]
fn package_kind_target_contract_and_publisher_scope_are_checked() {
    let (mut manifest, files, mut policy, key) = fixture();
    policy.target.ros_distribution = Some(name("jazzy"));
    check(&manifest, files.clone(), &policy, &key).unwrap();
    manifest.targets[0].ros_distribution = Some(name("humble"));
    assert!(matches!(
        check(&manifest, files.clone(), &policy, &key),
        Err(Error::Incompatible)
    ));
    manifest.targets[0].ros_distribution = None;
    policy.target = target();
    manifest
        .permissions
        .push(Permission::NativeEndpoint { role: name("arm") });
    assert!(matches!(
        check(&manifest, files.clone(), &policy, &key),
        Err(Error::Untrusted)
    ));
    manifest.permissions.pop();
    policy.target.architecture = Architecture::Arm64;
    assert!(matches!(
        check(&manifest, files.clone(), &policy, &key),
        Err(Error::Incompatible)
    ));
    policy.target = target();
    policy.contracts.cell = Digest::from_bytes([7; 32]);
    assert!(matches!(
        check(&manifest, files.clone(), &policy, &key),
        Err(Error::Incompatible)
    ));
    policy.contracts = contracts();
    policy.publishers.get_mut(&name("test-key")).unwrap().kinds = BTreeSet::new();
    assert!(matches!(
        check(&manifest, files, &policy, &key),
        Err(Error::Untrusted)
    ));
}
#[test]
fn dependency_closure_is_pinned_and_rechecked_against_current_trust() {
    let (mut dependency, files, mut policy, key) = fixture();
    dependency.package = name("example/base");
    let verified = Arc::new(check(&dependency, files.clone(), &policy, &key).unwrap());
    let mut root = dependency.clone();
    root.package = name("example/root");
    root.dependencies.push(Dependency {
        package: dependency.package.clone(),
        version: dependency.version.clone(),
        manifest_digest: verified.digest(),
        kind: PackageKind::Process,
    });
    assert!(matches!(
        check(&root, files.clone(), &policy, &key),
        Err(Error::Dependency(_))
    ));
    policy
        .dependencies
        .insert(dependency.package.clone(), verified);
    check(&root, files.clone(), &policy, &key).unwrap();
    root.dependencies[0].manifest_digest = Digest::from_bytes([99; 32]);
    assert!(matches!(
        check(&root, files.clone(), &policy, &key),
        Err(Error::Dependency(_))
    ));
    root.dependencies[0].manifest_digest = policy.dependencies[&dependency.package].digest();
    // The direct dependency's signed file cannot turn into a different target after verification.
    policy.target = Target {
        os: OperatingSystem::Windows,
        ..target()
    };
    root.targets.push(policy.target.clone());
    assert!(matches!(
        check(&root, files, &policy, &key),
        Err(Error::Incompatible)
    ));
}
#[test]
fn canonical_set_order_is_stable_and_duplicate_json_fields_are_rejected() {
    let (mut manifest, files, policy, key) = fixture();
    manifest.targets.push(Target {
        architecture: Architecture::Arm64,
        ..target()
    });
    let before = manifest_bytes(&manifest).unwrap();
    manifest.targets.reverse();
    assert_eq!(manifest_bytes(&manifest).unwrap(), before);
    let body = serde_json::to_string(&manifest).unwrap();
    let duplicate = body.replacen('{', "{\"publisher\":\"attacker\",", 1);
    assert!(matches!(
        verify_package(
            duplicate.as_bytes(),
            &signed(&manifest, &key),
            files,
            &policy
        ),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn directory_verification_owns_bytes_and_rejects_links() {
    let (manifest, files, policy, key) = fixture();
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(
        temp.path().join("manifest.sig.json"),
        signed(&manifest, &key),
    )
    .unwrap();
    for (path, data) in &files {
        std::fs::write(temp.path().join(path.as_str()), data).unwrap();
    }
    let package = rx_package::directory::verify_directory(temp.path(), &policy).unwrap();
    std::fs::write(
        temp.path().join("process.json"),
        b"changed after verification",
    )
    .unwrap();
    assert_eq!(
        package.file(&path("process.json")).unwrap(),
        files[&path("process.json")]
    );
    assert!(rx_package::directory::verify_directory(temp.path(), &policy).is_err());
    #[cfg(unix)]
    {
        std::fs::remove_file(temp.path().join("process.json")).unwrap();
        std::os::unix::fs::symlink("/etc/hosts", temp.path().join("process.json")).unwrap();
        assert!(matches!(
            rx_package::directory::verify_directory(temp.path(), &policy),
            Err(Error::Content(_))
        ));
    }
}

#[test]
fn dependency_cannot_pull_an_older_copy_of_the_root_back_into_the_bundle() {
    let (mut old_root, files, mut policy, key) = fixture();
    old_root.package = name("example/a");
    let old = Arc::new(check(&old_root, files.clone(), &policy, &key).unwrap());
    policy
        .dependencies
        .insert(old_root.package.clone(), old.clone());
    let mut child = old_root.clone();
    child.package = name("example/b");
    child.dependencies = vec![Dependency {
        package: old_root.package.clone(),
        version: old_root.version.clone(),
        manifest_digest: old.digest(),
        kind: PackageKind::Process,
    }];
    let child_verified = Arc::new(check(&child, files.clone(), &policy, &key).unwrap());
    policy
        .dependencies
        .insert(child.package.clone(), child_verified.clone());
    let mut next = old_root;
    next.version = "0.2.0".parse().unwrap();
    next.dependencies = vec![Dependency {
        package: child.package,
        version: child.version,
        manifest_digest: child_verified.digest(),
        kind: PackageKind::Process,
    }];
    assert!(matches!(
        check(&next, files, &policy, &key),
        Err(Error::Dependency(_))
    ));
}

#[test]
fn a_dependency_verified_before_key_revocation_is_not_retrusted_implicitly() {
    let (mut dependency, files, mut policy, key) = fixture();
    dependency.package = name("example/dependency");
    let verified = Arc::new(check(&dependency, files.clone(), &policy, &key).unwrap());
    policy
        .dependencies
        .insert(dependency.package.clone(), verified.clone());
    let mut root = dependency.clone();
    root.package = name("example/root");
    root.dependencies = vec![Dependency {
        package: dependency.package,
        version: dependency.version,
        manifest_digest: verified.digest(),
        kind: PackageKind::Process,
    }];
    let root_key = SigningKey::from_bytes(&[43; 32]);
    let key_id = name("root-key");
    let trusted = TrustedPublisher {
        publisher: name("example"),
        verifying_key: root_key.verifying_key().to_bytes(),
        kinds: [PackageKind::Process].into_iter().collect(),
        permissions: [Permission::ArtifactRead].into_iter().collect(),
    };
    policy.publishers.insert(key_id.clone(), trusted);
    policy.publishers.remove(&name("test-key"));
    let signature = root_key.sign(&signing_message(&root, &key_id).unwrap());
    let envelope = SignatureEnvelope {
        key: key_id,
        signature: signature
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    };
    assert!(matches!(
        verify_package(
            &serde_json::to_vec(&root).unwrap(),
            &serde_json::to_vec(&envelope).unwrap(),
            files,
            &policy
        ),
        Err(Error::Untrusted)
    ));
}

fn write_package(
    root: &std::path::Path,
    manifest: &Manifest,
    files: &BTreeMap<PackagePath, Vec<u8>>,
    key: &SigningKey,
) {
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(
        root.join("manifest.json"),
        manifest_bytes(manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(root.join("manifest.sig.json"), signed(manifest, key)).unwrap();
    for (p, data) in files {
        let p = root.join(p.as_str());
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    }
}
fn object_path(root: &std::path::Path, id: &store::ObjectId) -> std::path::PathBuf {
    root.join(format!("{}-{}", id.manifest, id.signature))
}
#[test]
fn object_survives_source_change_restart_and_repeat_import_without_trust_persistence() {
    let (manifest, files, policy, key) = fixture();
    let temp = tempfile::tempdir().unwrap();
    let incoming = temp.path().join("input");
    let root = temp.path().join("store");
    write_package(&incoming, &manifest, &files, &key);
    let package = directory::verify_directory(&incoming, &policy).unwrap();
    let mut store = store::Store::open(&root).unwrap();
    let object = store.put(&package).unwrap();
    std::fs::remove_dir_all(&incoming).unwrap();
    assert_eq!(store.put(&package).unwrap(), object);
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 3);
    drop(store);
    let store = store::Store::open_existing(&root).unwrap();
    assert_eq!(
        store
            .verify(&object, &policy)
            .unwrap()
            .file(&path("process.json"))
            .unwrap(),
        files[&path("process.json")]
    );
    let mut revoked = policy.clone();
    revoked.publishers.clear();
    assert!(matches!(
        store.verify(&object, &revoked),
        Err(Error::Untrusted)
    ));
    let mut wrong_target = policy.clone();
    wrong_target.target.architecture = Architecture::Arm64;
    assert!(matches!(
        store.verify(&object, &wrong_target),
        Err(Error::Incompatible)
    ));
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 3);
}
#[test]
fn object_corruption_is_not_repaired_by_repeat_import() {
    let (manifest, files, policy, key) = fixture();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("store");
    let mut store = store::Store::open(&root).unwrap();
    let package = check(&manifest, files, &policy, &key).unwrap();
    let object = store.put(&package).unwrap();
    let file = object_path(&root, &object).join("process.json");
    std::fs::write(&file, b"changed").unwrap();
    assert!(store.verify(&object, &policy).is_err());
    assert!(store.put(&package).is_err());
    assert_eq!(std::fs::read(&file).unwrap(), b"changed");
}
#[test]
fn signing_key_alias_has_distinct_object_and_misnamed_valid_objects_are_rejected() {
    let (manifest, files, mut policy, key) = fixture();
    let package = check(&manifest, files.clone(), &policy, &key).unwrap();
    let alias = name("new-key");
    policy
        .publishers
        .insert(alias.clone(), policy.publishers[&name("test-key")].clone());
    let sig = SignatureEnvelope {
        key: alias.clone(),
        signature: key
            .sign(&signing_message(&manifest, &alias).unwrap())
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    };
    let next = verify_package(
        &manifest_bytes(&manifest).unwrap(),
        &rx_domain::canonical::bytes(&sig).unwrap(),
        files,
        &policy,
    )
    .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("store");
    let mut store = store::Store::open(&root).unwrap();
    let first = store.put(&package).unwrap();
    let second = store.put(&next).unwrap();
    assert_eq!(first.manifest, second.manifest);
    assert_ne!(first.signature, second.signature);
    std::fs::remove_dir_all(object_path(&root, &first)).unwrap();
    std::fs::rename(object_path(&root, &second), object_path(&root, &first)).unwrap();
    assert!(matches!(
        store.verify(&first, &policy),
        Err(Error::Content(_))
    ));
}
#[test]
fn store_ownership_marker_and_unpublished_objects_cannot_be_bypassed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("store");
    assert!(store::Store::open_existing(&root).is_err());
    assert!(!root.exists());
    let owner = store::Store::open(&root).unwrap();
    assert!(store::Store::open(&root).is_err());
    std::fs::create_dir(root.join(".incoming-interrupted")).unwrap();
    std::fs::write(
        root.join(".incoming-interrupted/process.json"),
        b"unfinished",
    )
    .unwrap();
    drop(owner);
    let mut store = store::Store::open_existing(&root).unwrap();
    let (manifest, files, policy, key) = fixture();
    let package = check(&manifest, files, &policy, &key).unwrap();
    let object = store.put(&package).unwrap();
    store.verify(&object, &policy).unwrap();
    assert_eq!(
        std::fs::read(root.join(".incoming-interrupted/process.json")).unwrap(),
        b"unfinished"
    );
    drop(store);
    std::fs::write(root.join("store.json"), b"{}").unwrap();
    assert!(store::Store::open(&root).is_err());
    std::fs::remove_file(root.join("store.json")).unwrap();
    assert!(store::Store::open(&root).is_err());
}
#[test]
fn deeply_nested_valid_package_roundtrips_and_excess_file_inventory_is_bounded() {
    let (mut manifest, _, policy, key) = fixture();
    let nested = path("a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/r/s/t/data.json");
    let data = b"{}".to_vec();
    manifest.entry = EntryPoint::Process {
        source: nested.clone(),
    };
    manifest.files = vec![FileEntry {
        path: nested.clone(),
        sha256: content_digest(&data),
        size_bytes: Counter(2),
        executable: false,
    }];
    let package = check(
        &manifest,
        BTreeMap::from([(nested.clone(), data)]),
        &policy,
        &key,
    )
    .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("store");
    let mut store = store::Store::open(&root).unwrap();
    let object = store.put(&package).unwrap();
    assert_eq!(store.put(&package).unwrap(), object);
    store.verify(&object, &policy).unwrap();
    let dir = object_path(&root, &object);
    std::fs::write(dir.join("extra.json"), b"{}").unwrap();
    assert!(directory::acquire_directory(&dir, 1, 100_000).is_err());
}
#[cfg(unix)]
#[test]
fn store_and_import_reject_symlinks_and_store_handles_survive_root_path_replacement() {
    use std::os::unix::fs::symlink;
    let (manifest, files, policy, key) = fixture();
    let package = check(&manifest, files.clone(), &policy, &key).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("store");
    let mut store = store::Store::open(&root).unwrap();
    let object = store.put(&package).unwrap();
    let moved = temp.path().join("moved");
    std::fs::rename(&root, &moved).unwrap();
    std::fs::create_dir(&root).unwrap();
    assert_eq!(store.put(&package).unwrap(), object);
    store.verify(&object, &policy).unwrap();
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    let object_dir = object_path(&moved, &object);
    std::fs::remove_file(object_dir.join("process.json")).unwrap();
    let external = temp.path().join("external");
    std::fs::write(&external, &files[&path("process.json")]).unwrap();
    symlink(&external, object_dir.join("process.json")).unwrap();
    assert!(store.verify(&object, &policy).is_err());
    assert!(store.put(&package).is_err());
    drop(store);
    let linked = temp.path().join("linked-store");
    symlink(&moved, &linked).unwrap();
    assert!(store::Store::open(&linked).is_err());
    std::fs::remove_file(moved.join("store.lock")).unwrap();
    symlink(&external, moved.join("store.lock")).unwrap();
    assert!(store::Store::open(&moved).is_err());
    let incoming = temp.path().join("incoming");
    std::fs::create_dir(&incoming).unwrap();
    let source = temp.path().join("source");
    write_package(&source, &manifest, &files, &key);
    symlink(&source, incoming.join("shortcut")).unwrap();
    assert!(directory::verify_relative(&incoming, &path("shortcut"), &policy).is_err());
    assert!(directory::verify_directory(&incoming.join("shortcut"), &policy).is_err());
}
#[test]
fn stored_package_dependencies_are_rechecked_after_revocation() {
    let (mut dependency, files, mut policy, key) = fixture();
    dependency.package = name("example/dep");
    let dep = Arc::new(check(&dependency, files.clone(), &policy, &key).unwrap());
    policy
        .dependencies
        .insert(dependency.package.clone(), dep.clone());
    let mut manifest = dependency.clone();
    manifest.package = name("example/root");
    manifest.dependencies = vec![Dependency {
        package: dependency.package,
        version: dependency.version,
        manifest_digest: dep.digest(),
        kind: PackageKind::Process,
    }];
    let package = check(&manifest, files, &policy, &key).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut store = store::Store::open(&temp.path().join("store")).unwrap();
    let object = store.put(&package).unwrap();
    store.verify(&object, &policy).unwrap();
    policy.dependencies.clear();
    assert!(matches!(
        store.verify(&object, &policy),
        Err(Error::Dependency(_))
    ));
}
#[test]
fn policy_reader_pins_exact_bytes_and_reacquires_dependencies() {
    let (manifest, files, _, key) = fixture();
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("dep");
    write_package(&source, &manifest, &files, &key);
    let asset_path = temp.path().join("asset");
    std::fs::write(&asset_path, b"asset").unwrap();
    let policy = policy::Policy {
        additional_package_abis: Default::default(),
        schema: name("rx.package-verification-policy.v1"),
        contracts: contracts(),
        target: target(),
        keys: vec![policy::Key {
            id: name("test-key"),
            publisher: name("example"),
            verifying_key: Digest::from_bytes(key.verifying_key().to_bytes()),
            kinds: BTreeSet::from([PackageKind::Process]),
            permissions: BTreeSet::from([Permission::ArtifactRead]),
        }],
        dependencies: vec![policy::DependencyInput {
            path: source.clone(),
            manifest_digest: content_digest(&manifest_bytes(&manifest).unwrap()),
        }],
        assets: vec![],
    };
    let config = temp.path().join("policy.json");
    let bytes = rx_domain::canonical::bytes(&policy).unwrap();
    std::fs::write(&config, &bytes).unwrap();
    let (read, digest) = policy::read_with_digest::<policy::Policy>(&config).unwrap();
    assert_eq!(digest, content_digest(&bytes));
    assert_eq!(read.load().unwrap().dependencies.len(), 1);
    std::fs::write(source.join("process.json"), b"changed").unwrap();
    assert!(read.load().is_err());
    let mut duplicate = read;
    duplicate.dependencies.clear();
    duplicate.keys.push(duplicate.keys[0].clone());
    assert!(duplicate.load().is_err());
    std::fs::write(&config, b"{\"keys\":[],\"keys\":[]}").unwrap();
    assert!(policy::read::<serde_json::Value>(&config).is_err());
    #[cfg(unix)]
    {
        std::fs::remove_file(&config).unwrap();
        std::os::unix::fs::symlink(&asset_path, &config).unwrap();
        assert!(policy::read::<serde_json::Value>(&config).is_err());
    }
}

#[test]
fn verification_policy_fingerprint_preserves_all_u64_limits() {
    let (_, _, mut policy, _) = fixture();
    policy.max_content_bytes = 9_007_199_254_740_992;
    let before = policy.fingerprint().unwrap();
    policy.max_content_bytes += 1;
    assert_ne!(before, policy.fingerprint().unwrap());
    let stable = policy.fingerprint().unwrap();
    assert_eq!(stable, policy.clone().fingerprint().unwrap());
}

#[test]
fn relative_review_artifact_reads_are_bounded_and_reject_link_components() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(root.join("report")).unwrap();
    std::fs::write(root.join("report/value.json"), b"12345").unwrap();
    let file = path("report/value.json");
    assert_eq!(
        directory::read_relative_file(&root, &file, 5).unwrap(),
        b"12345"
    );
    assert!(directory::read_relative_file(&root, &file, 4).is_err());
    assert!(directory::read_relative_file(&root, &file, 0).is_err());
    assert!(directory::read_relative_file(&root, &path("report"), 100).is_err());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.join("report"), root.join("alias")).unwrap();
        assert!(directory::read_relative_file(&root, &path("alias/value.json"), 100).is_err());
        std::os::unix::fs::symlink(root.join("report/value.json"), root.join("linked.json"))
            .unwrap();
        assert!(directory::read_relative_file(&root, &path("linked.json"), 100).is_err());
        let linked = temp.path().join("linked-root");
        std::os::unix::fs::symlink(&root, &linked).unwrap();
        assert!(directory::read_relative_file(&linked, &file, 100).is_err());
    }
}

#[test]
fn explicit_multi_abi_policy_preserves_manifest_kind_target_and_trust_checks() {
    let (process, process_files, mut policy, key) = fixture();
    let mut device = process.clone();
    device.schema = name("rx.package.v2");
    device.package = name("example/device");
    device.contracts.package_abi = name("rx.package-abi.v2");
    device.entry = EntryPoint::DeviceReference {
        family: path("family.json"),
        profiles: vec![path("profile.json")],
        adapter: path("adapter.json"),
    };
    let files = ["family.json", "profile.json", "adapter.json"]
        .into_iter()
        .map(|p| (path(p), b"{}".to_vec()))
        .collect::<BTreeMap<_, _>>();
    device.files = files
        .iter()
        .map(|(p, b)| FileEntry {
            path: p.clone(),
            sha256: content_digest(b),
            size_bytes: Counter(b.len() as u64),
            executable: false,
        })
        .collect();
    assert!(matches!(
        check(&device, files.clone(), &policy, &key),
        Err(Error::Incompatible)
    ));
    let legacy = policy.fingerprint().unwrap();
    policy
        .additional_package_abis
        .insert(name("rx.package-abi.v2"));
    assert_ne!(legacy, policy.fingerprint().unwrap());
    let p = check(&process, process_files.clone(), &policy, &key).unwrap();
    let d = check(&device, files.clone(), &policy, &key).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let location = dir.path().join("store");
    let mut store = store::Store::open(&location).unwrap();
    let pi = store.put(&p).unwrap();
    let di = store.put(&d).unwrap();
    drop(store);
    let store = store::Store::open_existing(&location).unwrap();
    store.verify_owned(&pi, &policy).unwrap();
    store.verify_owned(&di, &policy).unwrap();
    let mut wrong = device.clone();
    wrong.schema = name("rx.package.v1");
    assert!(check(&wrong, files.clone(), &policy, &key).is_err());
    let mut wrong = device.clone();
    wrong.contracts.base = Digest::from_bytes([99; 32]);
    assert!(matches!(
        check(&wrong, files.clone(), &policy, &key),
        Err(Error::Incompatible)
    ));
    let mut restricted = policy.clone();
    restricted
        .publishers
        .get_mut(&name("test-key"))
        .unwrap()
        .kinds = [PackageKind::Device].into();
    assert!(matches!(
        check(&process, process_files, &restricted, &key),
        Err(Error::Untrusted)
    ));
    policy.additional_package_abis.clear();
    assert_eq!(legacy, policy.fingerprint().unwrap());
    store.verify_owned(&pi, &policy).unwrap();
    assert!(store.verify_owned(&di, &policy).is_err());
}
#[test]
fn policy_document_requires_explicit_v2_schema_for_additional_abis() {
    let p = policy::Policy {
        additional_package_abis: vec![],
        schema: name("rx.package-verification-policy.v1"),
        contracts: contracts(),
        target: target(),
        keys: vec![],
        assets: vec![],
        dependencies: vec![],
    };
    assert!(p.load().unwrap().additional_package_abis.is_empty());
    assert!(
        serde_json::to_value(&p)
            .unwrap()
            .get("additional_package_abis")
            .is_none()
    );
    let mut extra = p.clone();
    extra.additional_package_abis = vec![name("rx.package-abi.v2")];
    assert!(extra.load().is_err());
    extra.schema = name("rx.package-verification-policy.v2");
    assert!(
        extra
            .load()
            .unwrap()
            .additional_package_abis
            .contains(&name("rx.package-abi.v2"))
    );
    let mut duplicate = extra.clone();
    duplicate
        .additional_package_abis
        .push(name("rx.package-abi.v2"));
    assert!(duplicate.load().is_err());
    extra.additional_package_abis = vec![extra.contracts.package_abi.clone()];
    assert!(extra.load().is_err());
    extra.additional_package_abis.clear();
    assert!(extra.load().is_err());
}
