//! Test-only signing material and a deliberately non-semantic process descriptor.
use ed25519_dalek::{Signer, SigningKey};
use rx_domain::{canonical, types::*};
use rx_package::*;
use std::{collections::BTreeMap, path::PathBuf};
pub struct Fixture {
    pub _dir: tempfile::TempDir,
    pub import_root: PathBuf,
    pub store: store::Store,
    pub policy: VerificationPolicy,
    pub object: store::ObjectId,
    pub policy_bytes: Vec<u8>,
}
pub fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let import_root = dir.path().join("incoming");
    let source = import_root.join("test-package");
    std::fs::create_dir_all(&source).unwrap();
    let name = |s: &str| Name::new(s).unwrap();
    let key = SigningKey::from_bytes(&[47; 32]);
    let data = b"{\"schema\":\"rx.process-source.v1\"}".to_vec();
    let path = PackagePath::new("process.json").unwrap();
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
    let manifest = Manifest {
        schema: name("rx.package.v1"),
        package: name("test/intake"),
        version: "0.1.0".parse().unwrap(),
        publisher: name("test-only"),
        contracts: contracts.clone(),
        targets: vec![target.clone()],
        entry: EntryPoint::Process {
            source: path.clone(),
        },
        permissions: vec![Permission::ArtifactRead],
        dependencies: vec![],
        assets: vec![],
        files: vec![FileEntry {
            path: path.clone(),
            sha256: content_digest(&data),
            size_bytes: Counter(data.len() as u64),
            executable: false,
        }],
    };
    let key_id = name("test-intake-key");
    let sig = SignatureEnvelope {
        key: key_id.clone(),
        signature: key
            .sign(&signing_message(&manifest, &key_id).unwrap())
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
            id: key_id,
            publisher: manifest.publisher.clone(),
            verifying_key: Digest::from_bytes(key.verifying_key().to_bytes()),
            kinds: [PackageKind::Process].into_iter().collect(),
            permissions: [Permission::ArtifactRead].into_iter().collect(),
        }],
        assets: vec![],
        dependencies: vec![],
    };
    let policy = policy_doc.load().unwrap();
    let policy_bytes = canonical::bytes(&policy_doc).unwrap();
    let m = manifest_bytes(&manifest).unwrap();
    let s = canonical::bytes(&sig).unwrap();
    std::fs::write(source.join("manifest.json"), &m).unwrap();
    std::fs::write(source.join("manifest.sig.json"), &s).unwrap();
    std::fs::write(source.join("process.json"), &data).unwrap();
    let package = verify_package(&m, &s, BTreeMap::from([(path, data)]), &policy).unwrap();
    let mut store = store::Store::open(&dir.path().join("store")).unwrap();
    let object = store.put(&package).unwrap();
    Fixture {
        _dir: dir,
        import_root,
        store,
        policy,
        object,
        policy_bytes,
    }
}
