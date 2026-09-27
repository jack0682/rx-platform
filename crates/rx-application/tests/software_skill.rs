use rx_application::software_skill::*;
use rx_domain::{
    operation::{Knowledge, Outcome},
    types::Id,
};
use rx_storage::SqliteRepository;
use serde_json::json;
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn package() -> Package {
    Package {
        name: "add".into(),
        version: "1.0.0".into(),
        environment: "LOCAL_SIM".into(),
        inputs: [("value".into(), "integer".into())].into(),
        outputs: [("result".into(), "integer".into())].into(),
        timeout_ms: 1000,
        code: "def main(x): return {'result': x['value'] + 1}".into(),
    }
}
fn start() -> Start {
    Start {
        request_id: id(),
        skill: "add".into(),
        version: "1.0.0".into(),
        input: json!({"value":1}),
    }
}
#[test]
fn lost_reply_reuses_original_operation_and_rejects_changed_input() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::open(SqliteRepository::open(dir.path().join("s.db")).unwrap(), 1).unwrap();
    e.register(package()).unwrap();
    let r = start();
    let a = e.submit(r.clone(), 2).unwrap();
    let b = e.submit(r.clone(), 3).unwrap();
    assert_eq!(a.operation, b.operation);
    let mut different = r;
    different.input = json!({"value":2});
    assert!(e.submit(different, 4).is_err());
    let w = id();
    let (claimed, _) = e.claim(w.clone(), 5).unwrap().unwrap();
    assert_eq!(claimed.operation.id(), a.operation.id());
    assert!(e.claim(id(), 6).unwrap().is_none());
    let f = Finish {
        run: a.request.request_id,
        worker: w,
        outcome: "SUCCEEDED".into(),
        output: Some(json!({"result":2})),
        error: None,
        duration_ms: 3,
    };
    let done = e.finish(f.clone(), 8).unwrap();
    assert_eq!(done.operation.outcome(), Outcome::Succeeded);
    assert_eq!(e.finish(f.clone(), 9).unwrap().operation, done.operation);
    let mut f = f;
    f.output = Some(json!({"result":99}));
    assert!(e.finish(f, 10).is_err());
    assert_eq!(e.runs().unwrap().len(), 1);
}
#[test]
fn server_crash_quarantines_started_work_without_replaying_queued_work() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let active = start();
    let queued = start();
    {
        let mut e = Engine::open(SqliteRepository::open(&path).unwrap(), 1).unwrap();
        e.register(package()).unwrap();
        e.submit(active.clone(), 2).unwrap();
        e.claim(id(), 3).unwrap();
        e.submit(queued.clone(), 4).unwrap();
    }
    let mut e = Engine::open(SqliteRepository::open(&path).unwrap(), 8).unwrap();
    let old = e.get(&active.request_id).unwrap();
    assert_eq!(old.operation.outcome(), Outcome::Unresolved);
    assert_eq!(old.operation.knowledge(), Knowledge::Unknown);
    assert_eq!(e.submit(active, 9).unwrap().operation, old.operation);
    assert_eq!(e.claim(id(), 10).unwrap().unwrap().0.request, queued);
}
#[test]
fn versions_schema_physical_scope_and_worker_ownership_are_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::open(SqliteRepository::open(dir.path().join("s.db")).unwrap(), 1).unwrap();
    let mut p = package();
    p.environment = "PHYSICAL".into();
    assert!(e.register(p).is_err());
    e.register(package()).unwrap();
    let mut p = package();
    p.code.push('\n');
    assert!(e.register(p).is_err());
    let mut r = start();
    r.input = json!({"value":"one"});
    assert!(e.submit(r, 2).is_err());
    let r = start();
    e.submit(r.clone(), 3).unwrap();
    let worker = id();
    e.claim(worker.clone(), 4).unwrap();
    let mut f = Finish {
        run: r.request_id.clone(),
        worker: id(),
        outcome: "SUCCEEDED".into(),
        output: Some(json!({"result":2})),
        error: None,
        duration_ms: 1,
    };
    assert!(e.finish(f.clone(), 5).is_err());
    f.worker = worker.clone();
    f.output = Some(json!({"wrong":2}));
    assert!(e.finish(f, 5).is_err());
    assert_eq!(e.abandon_worker(worker, 6).unwrap(), 1);
    assert_eq!(
        e.get(&r.request_id).unwrap().operation.outcome(),
        Outcome::Unresolved
    );
}
