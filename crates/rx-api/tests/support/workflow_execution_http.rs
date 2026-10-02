#[path = "../../../rx-application/tests/support/execution_template_package.rs"]
mod signed_package;
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
    let mut f = fixture().await;
    let mut cookie = login(&f.app, "admin").await;
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
    let template = json!({"host":"sim-host","intent":{"kind":"FINITE_ACTION","target":"device","profile_digest":"11".repeat(32),"site_config_digest":"22".repeat(32),"calibration_digests":[],"resource_set":["resource"],"execution_timeout_ms":"30000","prepare_validity_ms":"1000","completion_rule":"done","cancel_rule":"stop","body":{"program":{"program":{"sha256":rx_package::content_digest(b"program"),"schema_id":"test.program.v1","size_bytes":"7"},"parameter_set":{"sha256":rx_package::content_digest(b"template"),"schema_id":"rx.workflow-parameters.v2","size_bytes":"8"}}}}});
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
    let Reply::Installation(installation) = f.handle.call(Command::Installation).await.unwrap()
    else {
        panic!("installation")
    };
    let package = signed_package::fixture(
        |catalog| {
            catalog.installation = installation.id.clone();
            catalog.cell = name("cell/a");
            catalog.templates = [(
                name("node"),
                rx_process_contract::execution_v2::TemplateDeclaration {
                    action: serde_json::from_value(input["templates"]["node"].clone()).unwrap(),
                    contract: serde_json::from_value(input["node_contracts"]["node"].clone())
                        .unwrap(),
                },
            )]
            .into();
        },
        false,
    );
    let object = package.stored.object().clone();
    let worker = rx_runtime::package_intake::Worker::new_pinned(
        package._directory.path().join("incoming"),
        package.store,
        package.policy,
        package.registration.policy_file_digest,
        package._directory.path().join("policy.json"),
    )
    .unwrap();
    f.handle
        .call(Command::ConfigurePackageIntake(Some(worker.registration())))
        .await
        .unwrap();
    f.app = rx_api::router_with_package_intake(
        Arc::new(f.handle.clone()),
        credentials(),
        LocalPolicy::new("http://127.0.0.1:8080").unwrap(),
        worker,
    )
    .unwrap();
    cookie = login(&f.app, "admin").await;
    let (_, context, _) = send(
        &f.app,
        request(
            "GET",
            "/api/v1/package-intake-context?cell=cell%2Fa",
            Some(&cookie),
            String::new(),
        ),
    )
    .await;
    let intake_id = id();
    post(&f,&cookie,"/api/v1/package-intakes",json!({"id":intake_id,"cell":"cell/a","title":"Signed workflow templates","relative_path":"templates","object":object,"configuration_digest":context["configuration_digest"],"policy_generation":context["registration"]["generation"]})).await;
    let mut publication_command = json!({"id":id(),"preview":preview["reference"],"cell":"cell/a","bindings":{"node":{"intake":intake_id,"template":"node"}}});
    let policy_path = package._directory.path().join("policy.json");
    let policy_bytes = std::fs::read(&policy_path).unwrap();
    let mut changed_policy = policy_bytes.clone();
    changed_policy.push(b'\n');
    std::fs::write(&policy_path, changed_policy).unwrap();
    let (status, error, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/workflow-executions/publish",
            Some(&cookie),
            json!({"request_key":id(),"command":publication_command}).to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["code"], "EXECUTION_TEMPLATE_REVERIFICATION_FAILED");
    std::fs::write(&policy_path, policy_bytes).unwrap();
    let mut unsigned = publication_command.clone();
    unsigned["bindings"]["node"]["template"] = json!("unsigned");
    let (status, error, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/workflow-executions/publish",
            Some(&cookie),
            json!({"request_key":id(),"command":unsigned}).to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["code"], "EXECUTION_TEMPLATE_REVERIFICATION_FAILED");
    let publication = post(
        &f,
        &cookie,
        "/api/v1/workflow-executions/publish",
        publication_command.clone(),
    )
    .await;
    assert_eq!(publication["policy"], preview["policy"]);
    assert_eq!(publication["cell"], "cell/a");
    assert_eq!(
        publication["packages"][intake_id.as_str()]["object"],
        serde_json::to_value(&object).unwrap()
    );
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
    // A v2 configuration references the published templates but is not installed by preparation.
    use rx_process_contract::{
        CompiledBody, CompiledNode, ResolvedProcess, SourceLocation, execution_v2 as v2,
    };
    let process_name = name("test/workflow");
    let make = |node: &str, body: CompiledBody| {
        let source = SourceLocation {
            flow: name("main"),
            node: name(node),
            instantiation: vec![],
        };
        let digest =
            rx_domain::canonical::digest("RX-PROCESS-NODE-v1", &(&process_name, &source)).unwrap();
        CompiledNode {
            id: name(&format!("node/{digest}")),
            source,
            body,
        }
    };
    let child = make(
        "node",
        CompiledBody::Operation {
            binding: name("node"),
        },
    );
    let node_id = child.id.clone();
    let process = ResolvedProcess {
        schema: name("rx.resolved-process.v1"),
        package_digest: None,
        source_digest: Digest::from_bytes([1; 32]),
        process: process_name.clone(),
        root: make(
            "root",
            CompiledBody::Sequence {
                children: vec![child],
            },
        ),
        bindings: serde_json::from_value(input["templates"].clone()).unwrap(),
        conditions: Default::default(),
    };
    let binding = v2::Binding {
        schema: name(v2::BINDING_SCHEMA),
        publication: serde_json::from_value(publication["reference"].clone()).unwrap(),
        policy: serde_json::from_value(publication["policy"].clone()).unwrap(),
        nodes: [(node_id.clone(), name("node"))].into(),
    };
    let plan = v2::Plan {
        schema: name(v2::PLAN_SCHEMA),
        binding: binding.clone(),
        process: process.clone(),
    };
    assert!(
        rx_domain::canonical::decode_json::<ResolvedProcess>(
            &rx_domain::canonical::bytes(&plan).unwrap()
        )
        .is_err(),
        "v1 DTO must refuse the v2 envelope (frozen binary test remains separate)"
    );
    let mut configuration = f.configuration.clone();
    assert!(
        serde_json::to_value(&configuration)
            .unwrap()
            .get("execution")
            .is_none()
    );
    assert_eq!(configuration.schema(), "rx.cell-configuration.v1");
    configuration.execution = Some(Box::new(binding));
    configuration.process = Some(Box::new(process));
    configuration.recipe = plan.reference().unwrap();
    let action = &plan.process.bindings[&name("node")];
    configuration.steps[0].id = node_id;
    configuration.steps[0].host = action.host.clone();
    configuration.steps[0].intent = action.intent.clone();
    configuration.steps[0].predecessors.clear();
    configuration.site_config_digest = action.intent.site_config_digest;
    configuration.hosts.push(action.host.clone());
    let (status, candidate, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/workflow-executions/configuration",
            Some(&cookie),
            serde_json::to_string(&configuration).unwrap(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{candidate}");
    assert_eq!(
        candidate["configuration"]["schema_id"],
        "rx.cell-configuration.v2"
    );
    assert_eq!(candidate["installed"], false);
    assert_eq!(candidate["qualified"], false);
    let (status, again, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/workflow-executions/configuration",
            Some(&cookie),
            serde_json::to_string(&configuration).unwrap(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(candidate, again);
    let Reply::Cell(_, live) = f
        .handle
        .call(Command::InspectCell {
            identity: f.admin.clone(),
            cell: f.configuration.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("cell")
    };
    assert!(live.configuration.execution.is_none());
    assert_eq!(live.configuration.recipe, f.configuration.recipe);
    let mut wrong = configuration.clone();
    wrong
        .execution
        .as_mut()
        .unwrap()
        .nodes
        .values_mut()
        .for_each(|v| *v = name("not-published"));
    let wrong_plan = v2::Plan {
        schema: name(v2::PLAN_SCHEMA),
        binding: *wrong.execution.clone().unwrap(),
        process: *wrong.process.clone().unwrap(),
    };
    wrong.recipe = wrong_plan.reference().unwrap();
    let (status, _, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/workflow-executions/configuration",
            Some(&cookie),
            serde_json::to_string(&wrong).unwrap(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
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
    publication_command["id"] = json!(id());
    let (status, error, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/workflow-executions/publish",
            Some(&cookie),
            json!({"request_key":id(),"command":publication_command}).to_string(),
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
