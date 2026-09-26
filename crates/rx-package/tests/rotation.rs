use rx_package::release::{self, Error};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/release")
}
fn read(path: impl AsRef<Path>) -> Vec<u8> {
    std::fs::read(path).unwrap()
}
fn fixture(group: &str, name: &str) -> Vec<u8> {
    read(root().join(group).join(name))
}
fn check(release_bytes: &[u8]) -> Result<release::VerifiedRelease, Error> {
    release::verify(
        release_bytes,
        &fixture("v1", "revocations.json"),
        &fixture("v1", "inventory.json"),
        |path, external| {
            assert_eq!(path, "tools/inert.txt");
            assert!(!external);
            Ok(fixture("v1", "payload"))
        },
    )
}

#[test]
fn rotated_root_refuses_old_foreign_wrong_id_and_domain_confusion() {
    assert!(matches!(
        check(&fixture("legacy-root", "release.json")),
        Err(Error::UnknownKey)
    ));
    assert!(matches!(
        check(&fixture("rotation", "foreign-key-normal-id.json")),
        Err(Error::Signature)
    ));
    assert!(matches!(
        check(&fixture("rotation", "normal-key-wrong-id.json")),
        Err(Error::UnknownKey)
    ));
    assert!(matches!(
        check(&fixture("rotation", "release-under-revocation-domain.json")),
        Err(Error::Signature)
    ));
    assert!(check(&fixture("v1", "release.json")).is_ok());
}

#[test]
fn signature_corruption_is_rejected_but_json_whitespace_is_not_malleation() {
    let original = fixture("v1", "release.json");
    let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
    let signature = value["signature"]["signature"].as_str().unwrap().to_owned();
    let mut changed = signature.as_bytes().to_vec();
    changed[0] = if changed[0] == b'0' { b'1' } else { b'0' };
    value["signature"]["signature"] = String::from_utf8(changed).unwrap().into();
    assert!(matches!(
        check(&serde_json::to_vec(&value).unwrap()),
        Err(Error::Signature)
    ));
    value["signature"]["signature"] = signature.to_uppercase().into();
    assert!(matches!(
        check(&serde_json::to_vec(&value).unwrap()),
        Err(Error::Signature)
    ));
    value["signature"]["signature"] = "00".into();
    assert!(matches!(
        check(&serde_json::to_vec(&value).unwrap()),
        Err(Error::Malformed(_)) | Err(Error::Signature)
    ));
    let parsed: serde_json::Value = serde_json::from_slice(&original).unwrap();
    assert!(check(&serde_json::to_vec_pretty(&parsed).unwrap()).is_ok());
}

#[test]
fn zero_is_malformed_and_maximum_is_valid_without_advancing_a_floor() {
    assert!(matches!(
        check(&fixture("rotation", "version-zero.json")),
        Err(Error::Malformed(_))
    ));
    let maximum = check(&fixture("rotation", "version-max.json")).unwrap();
    assert_eq!(maximum.version().0, u64::MAX);
}

#[test]
fn old_root_revocations_are_unknown_and_rotation_has_no_dual_window() {
    let directory = tempfile::tempdir().unwrap();
    let mut repository =
        rx_storage::SqliteRepository::open(directory.path().join("state.db")).unwrap();
    assert!(matches!(
        release::update_revocations(&mut repository, &fixture("legacy-root", "revocations.json")),
        Err(Error::UnknownKey)
    ));
    assert!(!std::hint::black_box(release::root::DUAL_ROOT_WINDOW));
    assert_eq!(release::root::PREVIOUS_ROOT_STATUS, "RETIRED_NOT_ACCEPTED");
    assert_eq!(
        release::root::DEVELOPMENT_SIGNING_CUSTODY,
        "ESTABLISHED_BY_TWO_COPY_RECOVERY_REHEARSAL"
    );
    assert_eq!(release::root::PRODUCT_SIGNING_CUSTODY, "NOT_ESTABLISHED");
    assert_eq!(
        release::root::OFFLINE_REVOCATION_FRESHNESS,
        "NOT_ESTABLISHED"
    );
}
