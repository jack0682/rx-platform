use rx_package::release::{self, Error, VerifiedRelease};
use rx_ports::Repository;
use rx_storage::SqliteRepository;
use std::path::PathBuf;
fn fixture(label: &str, file: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/release")
            .join(label)
            .join(file),
    )
    .unwrap()
}
fn verified(label: &str) -> VerifiedRelease {
    release::verify(
        &fixture(label, "release.json"),
        &fixture(label, "revocations.json"),
        &fixture(label, "inventory.json"),
        |path, external| {
            assert_eq!(path, "tools/inert.txt");
            assert!(!external);
            Ok(fixture(label, "payload"))
        },
    )
    .unwrap()
}
fn changed(bytes: Vec<u8>, f: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let mut value = serde_json::from_slice(&bytes).unwrap();
    f(&mut value);
    serde_json::to_vec(&value).unwrap()
}
#[test]
fn compiled_root_distinguishes_unsigned_signature_key_and_content_refusals() {
    let inv = fixture("v1", "inventory.json");
    let rev = fixture("v1", "revocations.json");
    let check =
        |data: Vec<u8>| release::verify(&data, &rev, &inv, |_, _| Ok(fixture("v1", "payload")));
    assert!(matches!(check(vec![]), Err(Error::Unsigned)));
    assert!(matches!(
        check(changed(fixture("v1", "release.json"), |v| v["signature"] =
            serde_json::Value::Null)),
        Err(Error::Unsigned)
    ));
    assert!(matches!(
        check(changed(fixture("v1", "release.json"), |v| v["signature"]
            ["signature"] =
            "00".repeat(64).into())),
        Err(Error::Signature)
    ));
    assert!(matches!(
        check(changed(fixture("v1", "release.json"), |v| v["signature"]
            ["key"] =
            "attacker/key".into())),
        Err(Error::UnknownKey)
    ));
    assert!(matches!(
        check(changed(fixture("v1", "release.json"), |v| v["manifest"]
            ["version"] =
            "99".into())),
        Err(Error::Signature)
    ));
    assert!(matches!(
        release::verify(&fixture("v1", "release.json"), &rev, &inv, |_, _| Ok(
            b"substitution".to_vec()
        )),
        Err(Error::Content(_))
    ));
    assert!(matches!(
        release::verify(
            &fixture("v1", "release.json"),
            &rev,
            &fixture("v2", "inventory.json"),
            |_, _| panic!("must reject inventory before acquisition")
        ),
        Err(Error::Content(_))
    ));
    assert!(matches!(check(b"{bad".to_vec()), Err(Error::Malformed(_))));
    assert_eq!(verified("v1").version().0, 1);
}
#[test]
fn durable_floor_rejects_restart_downgrade_and_same_version_equivocation() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("release.db");
    let mut repo = SqliteRepository::open(&db).unwrap();
    release::update_revocations(&mut repo, &fixture("v1", "revocations.json")).unwrap();
    release::admit(&mut repo, &verified("v1")).unwrap();
    release::admit(&mut repo, &verified("v2")).unwrap();
    let head = repo.control_snapshot().unwrap();
    release::admit(&mut repo, &verified("v2")).unwrap();
    assert_eq!(head, repo.control_snapshot().unwrap());
    repo.close().unwrap();
    let mut repo = SqliteRepository::open(&db).unwrap();
    assert!(matches!(
        release::admit(&mut repo, &verified("v1")),
        Err(Error::Rollback)
    ));
    assert!(matches!(
        release::admit(&mut repo, &verified("equivocation")),
        Err(Error::Rollback)
    ));
    assert_eq!(head, repo.control_snapshot().unwrap());
}
#[test]
fn revocation_survives_denied_candidate_restart_and_replayed_or_shrunk_lists() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("release.db");
    let mut repo = SqliteRepository::open(&db).unwrap();
    release::update_revocations(&mut repo, &fixture("v2", "revocations.json")).unwrap();
    let old_proof = verified("v2");
    release::admit(&mut repo, &old_proof).unwrap();
    release::update_revocations(&mut repo, &fixture("revoked", "revocations.json")).unwrap();
    assert!(matches!(
        release::verify(
            &fixture("v2", "release.json"),
            &fixture("revoked", "revocations.json"),
            &fixture("v2", "inventory.json"),
            |_, _| panic!("revoked before content")
        ),
        Err(Error::Revoked)
    ));
    repo.close().unwrap();
    let mut repo = SqliteRepository::open(&db).unwrap();
    assert!(matches!(
        release::admit(&mut repo, &old_proof),
        Err(Error::Revoked)
    ));
    assert!(matches!(
        release::update_revocations(&mut repo, &fixture("v2", "revocations.json")),
        Err(Error::Rollback)
    ));
    assert!(matches!(
        release::update_revocations(&mut repo, &fixture("shrink", "revocations.json")),
        Err(Error::Rollback)
    ));
    let bad = changed(fixture("revoked", "revocations.json"), |v| {
        v["signature"] = serde_json::Value::Null
    });
    assert!(matches!(
        release::update_revocations(&mut repo, &bad),
        Err(Error::Unsigned)
    ));
}
