use rx_storage::mailbox::Mailbox;
#[test]
fn publication_is_immutable_and_live_cooperating_writer_excluded() {
    let dir = tempfile::tempdir().unwrap();
    let mailbox = Mailbox::open(dir.path()).unwrap();
    let guard = mailbox.lock().unwrap();
    assert!(mailbox.lock().is_err());
    guard.publish_once("request-1.json", b"first").unwrap();
    assert_eq!(
        guard.read("request-1.json", 100).unwrap().unwrap(),
        b"first"
    );
    guard.publish_once("request-1.json", b"first").unwrap();
    assert!(guard.publish_once("request-1.json", b"different").is_err());
    assert!(guard.read("../escape", 100).is_err());
    drop(guard);
    let next = mailbox.lock().unwrap();
    assert!(mailbox.lock().is_err());
    assert_eq!(next.read("request-1.json", 100).unwrap().unwrap(), b"first");
}
#[test]
fn partial_publication_and_oversized_input_are_refused_not_erased() {
    let dir = tempfile::tempdir().unwrap();
    let mailbox = Mailbox::open(dir.path()).unwrap();
    let guard = mailbox.lock().unwrap();
    std::fs::write(dir.path().join(".pending-request-2.json"), b"partial").unwrap();
    assert!(guard.publish_once("request-2.json", b"complete").is_err());
    assert!(guard.read("request-2.json", 100).unwrap().is_none());
    assert_eq!(
        std::fs::read(dir.path().join(".pending-request-2.json")).unwrap(),
        b"partial"
    );
    assert!(guard.publish_once("large.json", &vec![0; 131073]).is_err());
    assert!(guard.read("request-1.json", 131073).is_err());
}
#[cfg(unix)]
#[test]
fn symlink_and_nonregular_payloads_are_refused_without_following() {
    let dir = tempfile::tempdir().unwrap();
    let mailbox = Mailbox::open(dir.path()).unwrap();
    let guard = mailbox.lock().unwrap();
    std::fs::write(dir.path().join("outside"), b"outside").unwrap();
    std::os::unix::fs::symlink(dir.path().join("outside"), dir.path().join("reply.json")).unwrap();
    assert!(guard.read("reply.json", 100).is_err());
    std::fs::create_dir(dir.path().join("directory.json")).unwrap();
    assert!(guard.read("directory.json", 100).is_err());
}
