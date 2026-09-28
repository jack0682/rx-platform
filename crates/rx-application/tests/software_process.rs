use rx_application::software_skill::{metrics::Window, process::*, *};
use rx_domain::{operation::Outcome, types::Id};
use rx_storage::SqliteRepository;
use serde_json::{Value, json};
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn skill(name: &str) -> Package {
    Package {
        name: name.into(),
        version: "1.0.0".into(),
        environment: "LOCAL_SIM".into(),
        inputs: [("value".into(), "integer".into())].into(),
        outputs: [("result".into(), "integer".into())].into(),
        timeout_ms: 1000,
        code: "def main(x): return {'result': x['value']+1}".into(),
    }
}
fn definition() -> Value {
    json!({
        "source":{"schema":"rx.process-source.v1","process":"two-skills","entry":"main","conditions":{},"flows":[{"id":"main","root":"sequence","nodes":[
            {"id":"sequence","body":{"kind":"SEQUENCE","children":["one","two"]}},
            {"id":"one","body":{"kind":"OPERATION","binding":"first"}},
            {"id":"two","body":{"kind":"OPERATION","binding":"second"}}
        ]}]},"version":"1.0.0","environment":"LOCAL_SIM","inputs":{"seed":"integer"},
        "bindings":{"first":{"skill":"increment","version":"1.0.0","inputs":{"value":{"kind":"INPUT","field":"seed"}}},
            "second":{"skill":"increment","version":"1.0.0","inputs":{"value":{"kind":"OUTPUT","step":"first","field":"result"}}}},
        "outputs":{"answer":{"kind":"OUTPUT","step":"second","field":"result"}}
    })
}
fn setup(path: &std::path::Path) -> Engine<SqliteRepository> {
    let mut e = Engine::open(SqliteRepository::open(path).unwrap(), 0).unwrap();
    e.register(skill("increment")).unwrap();
    e.register_process(serde_json::from_value(definition()).unwrap())
        .unwrap();
    e
}
fn start() -> StartProcess {
    StartProcess {
        request_id: id(),
        process: "two-skills".into(),
        version: "1.0.0".into(),
        input: json!({"seed":8}),
    }
}
fn done(e: &mut Engine<SqliteRepository>, run: &Run, output: i64, now: u64) {
    e.finish(
        Finish {
            run: run.request.request_id.clone(),
            worker: run.worker.clone().unwrap(),
            outcome: "SUCCEEDED".into(),
            output: Some(json!({"result":output})),
            error: None,
            duration_ms: 10,
        },
        now,
    )
    .unwrap();
}

#[test]
fn original_child_ids_and_data_survive_restart_between_steps() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let mut e = setup(&path);
    let request = start();
    let accepted = e.start_process(request.clone(), 100).unwrap();
    let child_ids = accepted
        .steps
        .iter()
        .map(|s| s.request_id.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        accepted.steps[0].execution.as_ref().unwrap().request.input,
        json!({"value":8})
    );
    assert!(accepted.steps[1].execution.is_none());
    // The exposed future child ID is reserved; a caller cannot inject its outcome/input.
    assert!(
        e.submit(
            Start {
                request_id: child_ids[1].clone(),
                skill: "increment".into(),
                version: "1.0.0".into(),
                input: json!({"value":999})
            },
            101
        )
        .is_err()
    );
    let (first, _) = e.claim(id(), 102).unwrap().unwrap();
    done(&mut e, &first, 9, 112);
    drop(e);
    let mut e = Engine::open_existing(SqliteRepository::open(&path).unwrap(), 120).unwrap();
    let read = e.start_process(request.clone(), 121).unwrap();
    assert_eq!(
        read.steps
            .iter()
            .map(|s| s.request_id.clone())
            .collect::<Vec<_>>(),
        child_ids
    );
    let (second, _) = e.claim(id(), 122).unwrap().unwrap();
    assert_eq!(second.request.request_id, child_ids[1]);
    assert_eq!(second.request.input, json!({"value":9}));
    done(&mut e, &second, 10, 132);
    let result = e.process_run(&request.request_id).unwrap();
    assert_eq!(result.status, Status::Succeeded);
    assert_eq!(result.output, Some(json!({"answer":10})));
    assert_eq!(e.runs().unwrap().len(), 2);
    assert!(e.claim(id(), 133).unwrap().is_none());
    let mut conflict = request;
    conflict.input = json!({"seed":30});
    assert!(e.start_process(conflict, 140).is_err());
}

#[test]
fn unknown_predecessor_never_admits_its_successor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let mut e = setup(&path);
    let request = start();
    e.start_process(request.clone(), 1).unwrap();
    let (first, _) = e.claim(id(), 2).unwrap().unwrap();
    drop(e);
    let mut e = Engine::open_existing(SqliteRepository::open(&path).unwrap(), 3).unwrap();
    let result = e.process_run(&request.request_id).unwrap();
    assert_eq!(result.status, Status::Unknown);
    assert!(result.steps[1].execution.is_none());
    assert!(e.claim(id(), 4).unwrap().is_none());
    assert_eq!(e.runs().unwrap().len(), 1);
    assert_eq!(
        e.get(&first.request.request_id)
            .unwrap()
            .operation
            .outcome(),
        Outcome::Unresolved
    );
    let retry = e.start_process(request, 5).unwrap();
    assert_eq!(retry.status, Status::Unknown);
    assert!(retry.steps[1].execution.is_none());
}

#[test]
fn forward_references_missing_fields_and_type_changes_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = setup(&dir.path().join("store.db"));
    let mut d = definition();
    d["version"] = json!("2.0.0");
    d["bindings"]["first"]["inputs"]["value"] =
        json!({"kind":"OUTPUT","step":"second","field":"result"});
    assert!(
        e.register_process(serde_json::from_value(d).unwrap())
            .is_err()
    );
    let mut d = definition();
    d["version"] = json!("2.0.0");
    d["inputs"]["seed"] = json!("string");
    assert!(
        e.register_process(serde_json::from_value(d).unwrap())
            .is_err()
    );
    let mut d = definition();
    d["version"] = json!("2.0.0");
    d["bindings"]["second"]["inputs"]["value"]["field"] = json!("missing");
    assert!(
        e.register_process(serde_json::from_value(d).unwrap())
            .is_err()
    );
    let mut d = definition();
    d["environment"] = json!("PHYSICAL");
    assert!(
        e.register_process(serde_json::from_value(d).unwrap())
            .is_err()
    );
    let mut d = definition();
    d["bindings"]["second"]["inputs"]["value"] = json!({"kind":"LITERAL","value":20});
    assert!(
        e.register_process(serde_json::from_value(d).unwrap())
            .is_err()
    );
    assert_eq!(e.processes().unwrap().len(), 1);
    assert!(e.process_runs().unwrap().is_empty());
}

#[test]
fn oversized_composed_input_blocks_only_its_own_process() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::open(
        SqliteRepository::open(dir.path().join("store.db")).unwrap(),
        0,
    )
    .unwrap();
    let p = Package {
        name: "join".into(),
        version: "1.0.0".into(),
        environment: "LOCAL_SIM".into(),
        inputs: [
            ("left".into(), "string".into()),
            ("right".into(), "string".into()),
        ]
        .into(),
        outputs: [("result".into(), "integer".into())].into(),
        timeout_ms: 1000,
        code: "def main(x): return {'result':0}".into(),
    };
    e.register(p).unwrap();
    e.register(skill("increment")).unwrap();
    let d = json!({"source":{"schema":"rx.process-source.v1","process":"oversize","entry":"main","conditions":{},"flows":[{"id":"main","root":"one","nodes":[{"id":"one","body":{"kind":"OPERATION","binding":"one"}}]}]},"version":"1.0.0","environment":"LOCAL_SIM","inputs":{"text":"string"},"bindings":{"one":{"skill":"join","version":"1.0.0","inputs":{"left":{"kind":"INPUT","field":"text"},"right":{"kind":"INPUT","field":"text"}}}},"outputs":{}});
    e.register_process(serde_json::from_value(d).unwrap())
        .unwrap();
    let r = e
        .start_process(
            StartProcess {
                request_id: id(),
                process: "oversize".into(),
                version: "1.0.0".into(),
                input: json!({"text":"x".repeat(40_000)}),
            },
            10,
        )
        .unwrap();
    assert_eq!(r.status, Status::Blocked);
    assert!(r.issue.unwrap().contains("64 KiB"));
    assert!(r.steps[0].execution.is_none());
    let next = e
        .submit(
            Start {
                request_id: id(),
                skill: "increment".into(),
                version: "1.0.0".into(),
                input: json!({"value":2}),
            },
            11,
        )
        .unwrap();
    assert_eq!(
        e.claim(id(), 12).unwrap().unwrap().0.request.request_id,
        next.request.request_id
    );
}

#[test]
fn metrics_separate_unknown_versions_windows_and_clock_anomalies() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = setup(&dir.path().join("store.db"));
    for (at, outcome) in [(10, "SUCCEEDED"), (20, "FAILED"), (30, "UNKNOWN")] {
        let r = e
            .submit(
                Start {
                    request_id: id(),
                    skill: "increment".into(),
                    version: "1.0.0".into(),
                    input: json!({"value":1}),
                },
                at,
            )
            .unwrap();
        let (claimed, _) = e
            .claim(id(), if at == 20 { 19 } else { at + 5 })
            .unwrap()
            .unwrap();
        e.finish(
            Finish {
                run: r.request.request_id,
                worker: claimed.worker.unwrap(),
                outcome: outcome.into(),
                output: if outcome == "SUCCEEDED" {
                    Some(json!({"result":2}))
                } else {
                    None
                },
                error: if outcome == "SUCCEEDED" {
                    None
                } else {
                    Some("observed test condition".into())
                },
                duration_ms: 10,
            },
            at + 15,
        )
        .unwrap();
    }
    let mut v2 = skill("increment");
    v2.version = "2.0.0".into();
    e.register(v2).unwrap();
    e.submit(
        Start {
            request_id: id(),
            skill: "increment".into(),
            version: "2.0.0".into(),
            input: json!({"value":1}),
        },
        40,
    )
    .unwrap();
    let report = e.metrics(Window::default()).unwrap();
    assert_eq!(report.groups.len(), 2);
    let old = &report.groups[0];
    assert_eq!(old.counts.admitted, 3);
    assert_eq!(old.counts.unknown, 1);
    assert_eq!(old.decided_denominator, 2);
    assert_eq!(old.success_fraction_of_decided, Some(0.5));
    assert_eq!(old.execution_ms.samples, 2);
    assert_eq!(old.queue_wait_wall_ms.missing, 1);
    let new = &report.groups[1];
    assert_eq!(new.counts.queued, 1);
    assert_eq!(new.success_fraction_of_decided, None);
    let filtered = e
        .metrics(Window {
            since_ms: Some(20),
            until_ms: Some(30),
        })
        .unwrap();
    assert_eq!(filtered.groups.len(), 1);
    assert_eq!(filtered.groups[0].counts.failed, 1);
    assert_eq!(filtered.groups[0].counts.admitted, 1);
    assert!(
        e.metrics(Window {
            since_ms: Some(30),
            until_ms: Some(20)
        })
        .is_err()
    );
}
