use super::*;

async fn post(
    f: &Fixture,
    cookie: &str,
    path: &str,
    command: serde_json::Value,
) -> serde_json::Value {
    let (status, value, _) = send(
        &f.app,
        request(
            "POST",
            path,
            Some(cookie),
            json!({"request_key":id(),"command":command}).to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{path}: {value}");
    value
}
async fn definition(
    f: &Fixture,
    cookie: &str,
    catalog: &Id,
    body: serde_json::Value,
) -> serde_json::Value {
    post(f,cookie,"/api/v1/definitions",json!({"catalog":catalog,"id":id(),"expected":null,"label":"SIMULATION fixture","body":body,"archived":false})).await["version"]["definition"]["reference"].clone()
}
fn query(path: &str, reference: &serde_json::Value) -> String {
    format!(
        "{path}?catalog={}&id={}&revision={}&digest={}",
        reference["catalog"].as_str().unwrap(),
        reference["id"].as_str().unwrap(),
        reference["revision"].as_str().unwrap(),
        reference["digest"].as_str().unwrap()
    )
}
fn quantity(v: u64, unit: &str) -> serde_json::Value {
    json!({"unit":unit,"data":{"kind":"NUMBER","range":{"min":v,"max":v}}})
}

#[tokio::test]
async fn saved_execution_preview_publish_and_stale_diagnostics_use_real_http_writer() {
    let f = fixture().await;
    let cookie = login(&f.app, "admin").await;
    let catalog = id();
    post(&f,&cookie,"/api/v1/definition-catalogs",json!({"id":catalog,"expected":null,"title":"Execution inputs","members":{},"terminals":[],"archived":false})).await;
    let mut properties = std::collections::BTreeMap::new();
    for (key, kind, unit, category, over) in [
        ("target", "NUMBER", "mm", "EXECUTION", true),
        ("timeout", "NUMBER", "s", "EXECUTION", false),
        ("flag", "BOOLEAN", "unitless", "RESOURCE", false),
        ("text", "TEXT", "unitless", "RESOURCE", false),
    ] {
        let reference=definition(&f,&cookie,&catalog,json!({"kind":"PROPERTY","specification":{"value_type":kind,"category":category,"constraint_scope":null,"unit":unit,"minimum":null,"maximum":null,"choices":[],"vector_length":null,"overridable":over,"parameter_mapping":{}}})).await;
        properties.insert(key, reference);
    }
    let actor_type=definition(&f,&cookie,&catalog,json!({"kind":"RESOURCE_TYPE","parent":null,"fields":{
        "can":{"property":properties["flag"],"required":true},"implementation":{"property":properties["text"],"required":true},"version":{"property":properties["text"],"required":true}}})).await;
    let actor=definition(&f,&cookie,&catalog,json!({"kind":"RESOURCE_MODEL","resource_type":actor_type,"values":{"can":{"boolean":true},"implementation":{"text":"simulated"},"version":{"text":"1"}}})).await;
    let object_type = definition(
        &f,
        &cookie,
        &catalog,
        json!({"kind":"OBJECT_TYPE","parent":null,"fields":{}}),
    )
    .await;
    let object = definition(
        &f,
        &cookie,
        &catalog,
        json!({"kind":"OBJECT_MODEL","object_type":object_type,"values":{}}),
    )
    .await;
    let workflow_id = id();
    let mut model_input = json!({"catalog":catalog,"id":workflow_id,"expected":null,"label":"Saved input workflow","spec":{
        "schema":"rx.workflow-model.v1","contexts":{
            "actor":{"label":"Actor","kind":"RESOURCE","accepted_types":[actor_type],"required":true,"multiple":false},
            "object":{"label":"Object","kind":"OBJECT","accepted_types":[object_type],"required":true,"multiple":false}},
        "defaults":{"actor":[actor],"object":[object]},"property_sets":[],"rules":{},
        "constraints":{"limit":{"left":{"kind":"PROPERTY","name":"target"},"right":{"kind":"LITERAL","value":quantity(50,"mm")},"relation":"LE","property":"target","message":"maximum target"}},
        "tasks":{"act":{"label":"Act","contexts":["actor","object"],"properties":{
            "target":{"property":properties["target"],"sources":[{"kind":"OVERRIDE"},{"kind":"DEFAULT"}],"default":quantity(20,"mm")},
            "timeout":{"property":properties["timeout"],"sources":[{"kind":"DEFAULT"}],"default":quantity(10,"s")}},
            "constraints":["limit"],"capabilities":[{"name":"act","slot":"actor","field":"can"}],
            "skills":[{"capability":"act","slot":"actor","implementation_field":"implementation","version_field":"version","primitive":"set","parameters":{"value":"target"}}],
            "timeout_property":"timeout","done":{"observation":"done","property":properties["flag"],"equals":{"unit":"unitless","data":{"kind":"BOOLEAN","value":true}}},"on_failure":"STOP","on_unknown":"HOLD_AND_RECONCILE"}},
        "steps":[{"id":"node","task":"act"}]}});
    let model = post(&f, &cookie, "/api/v1/workflow-models", model_input.clone()).await;
    let template = json!({"host":"sim-host","intent":{"kind":"FINITE_ACTION","target":"device","profile_digest":"11".repeat(32),"site_config_digest":"22".repeat(32),"calibration_digests":[],"resource_set":["resource"],"execution_timeout_ms":"30000","prepare_validity_ms":"1000","completion_rule":"done","cancel_rule":"stop","body":{"program":{"program":{"sha256":"33".repeat(32),"schema_id":"test.program.v1","size_bytes":"1"},"parameter_set":{"sha256":"44".repeat(32),"schema_id":"rx.workflow-parameters.v2","size_bytes":"1"}}}}});
    let input = json!({"id":id(),"slots":2,"candidates":[{"key":"a","object_model":object,"request":{"workflow":model["reference"],"contexts":{},"property_sets":[],"overrides":{},"inputs":{},"slot_index":"0"}}],"templates":{"node":template},"node_contracts":{"node":{"implementation":"simulated","version":"1","primitive":"set","parameters":{"value":{"unit":"mm","value_type":"NUMBER","frame":null}}}}});
    let preview = post(
        &f,
        &cookie,
        "/api/v1/workflow-executions/preview",
        input.clone(),
    )
    .await;
    let report_url = query(
        "/api/v1/workflow-executions/preview-report",
        &preview["reference"],
    ) + "&candidate=0&slot=1";
    let (status, report, _) = send(
        &f.app,
        request("GET", &report_url, Some(&cookie), "".into()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{report}");
    assert_eq!(report["slot"], 1);
    assert_eq!(
        report["resolution"]["steps"][0]["properties"]["target"]["value"]["data"]["range"]["min"],
        20
    );
    let publication = post(
        &f,
        &cookie,
        "/api/v1/workflow-executions/publish",
        json!({"id":id(),"preview":preview["reference"]}),
    )
    .await;
    assert_eq!(publication["policy"], preview["policy"]);
    let (status, read, _) = send(
        &f.app,
        request(
            "GET",
            &query(
                "/api/v1/workflow-executions/publication",
                &publication["reference"],
            ),
            Some(&cookie),
            "".into(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read, publication);
    let (status, _, _) = send(&f.app, request("GET", &report_url, None, "".into())).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let reader = login(&f.app, "reader").await;
    let (status, _, _) = send(
        &f.app,
        request("GET", &report_url, Some(&reader), "".into()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let mut invalid = input;
    invalid["id"] = json!(id());
    invalid["candidates"][0]["request"]["overrides"] = json!({"node":{"target":quantity(60,"mm")}});
    let (status, error, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/workflow-executions/preview",
            Some(&cookie),
            json!({"request_key":id(),"command":invalid}).to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["code"], "INVALID_EXECUTION_INPUT");
    model_input["expected"] = json!("1");
    model_input["label"] = json!("Changed revision");
    post(&f, &cookie, "/api/v1/workflow-models", model_input).await;
    let (status, error, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/workflow-executions/publish",
            Some(&cookie),
            json!({"request_key":id(),"command":{"id":id(),"preview":preview["reference"]}})
                .to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["code"], "STALE_EXECUTION_REFERENCE");
    let message = error["message"].as_str().unwrap();
    assert!(
        message.contains("pinned 1") && message.contains("current 2"),
        "{message}"
    );
    let (_, reopened, _) = send(
        &f.app,
        request("GET", &report_url, Some(&cookie), "".into()),
    )
    .await;
    assert_eq!(reopened, report);
}
