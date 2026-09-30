use super::*;

fn create(label: &str) -> Value {
    json!({"request_key":id(),"command":{"declaration":{
        "label":label,"catalog":{"program":"rx/status-http","digest":"49".repeat(32)}
    }}})
}

#[tokio::test]
async fn reporting_http_owner_controls_scope_and_reads_attributed_history() {
    use rx_domain::{component::Binding, resident_reporting::*};
    let f = fixture().await;
    let admin = login(&f.app, "admin").await;
    let reader = login(&f.app, "reader").await;
    let (status, component, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/components",
            Some(&admin),
            create("report-test").to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let auth = Digest::from_bytes([71; 32]);
    let Reply::ResidentReporter(peer) = f
        .handle
        .call(Command::OpenResidentReporter {
            principal: name("reader"),
            peer_boot: id(),
            authentication_binding: auth,
        })
        .await
        .unwrap()
    else {
        panic!("reporter");
    };
    let source = id();
    let issue = json!({"request_key":id(),"command":{
        "component":component["record"]["registration"]["id"],
        "expected_component_revision":component["revision"],
        "reporter_session":peer.id,"source_registration":source,"source_revision":"1"
    }});
    for (cookie, expected) in [
        (None, StatusCode::UNAUTHORIZED),
        (Some(reader.as_str()), StatusCode::FORBIDDEN),
    ] {
        let (status, _, _) = send(
            &f.app,
            request(
                "POST",
                "/api/v1/components/reporting",
                cookie,
                issue.to_string(),
            ),
        )
        .await;
        assert_eq!(status, expected);
    }
    let (status, issued, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/components/reporting",
            Some(&admin),
            issue.to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{issued}");
    let scope: Scope = serde_json::from_value(issued["scope"].clone()).unwrap();
    let (_, replay, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/components/reporting",
            Some(&admin),
            issue.to_string(),
        ),
    )
    .await;
    assert_eq!(replay, issued);
    let scope_path = format!("/api/v1/component/reporting-scope?id={}", scope.id);
    let (status, read, _) = send(
        &f.app,
        request("GET", &scope_path, Some(&admin), String::new()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read, issued);
    let instance = id();
    f.handle
        .call(Command::PublishResidentReport {
            identity: rx_application::resident_reporting::ReporterIdentity {
                principal: peer.principal.clone(),
                session: peer.id.clone(),
                authentication_binding: auth,
            },
            key: id(),
            report: Report {
                scope: scope.id.clone(),
                source: Binding {
                    registration: source,
                    registration_revision: Counter(1),
                    catalog: scope.catalog,
                    run: id(),
                    selection: name("report-test"),
                    instance: instance.clone(),
                },
                sequence: Counter(1),
                state: ExecutionState::Unknown,
                pid: None,
                exit_code: None,
                detail: "no live process claim".into(),
            },
        })
        .await
        .unwrap();
    let report_path = format!(
        "/api/v1/component/report?component={}&instance={instance}",
        scope.component
    );
    for path in [&scope_path, &report_path] {
        let (status, _, _) = send(&f.app, request("GET", path, Some(&reader), String::new())).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    let (status, report, _) = send(
        &f.app,
        request("GET", &report_path, Some(&admin), String::new()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        report["receipt"]["execution_ownership"],
        "NOT_ESTABLISHED_BY_REPORT"
    );
    assert_eq!(report["reporter_session_current"], true);
    let revoke = json!({"request_key":id(),"command":{"scope":scope.id,"expected_revision":issued["revision"]}});
    let (status, _, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/components/reporting/revoke",
            Some(&reader),
            revoke.to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, revoked, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/components/reporting/revoke",
            Some(&admin),
            revoke.to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{revoked}");
    assert_eq!(revoked["scope"]["active"], false);
    let (status, history, _) = send(
        &f.app,
        request("GET", &report_path, Some(&admin), String::new()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(history["reporter_session_current"], false);
    assert_eq!(history["receipt"], report["receipt"]);
}

#[tokio::test]
async fn component_http_writes_use_the_authoritative_writer_and_preserve_history() {
    let f = fixture().await;
    let cookie = login(&f.app, "admin").await;
    let (_, before, _) = send(
        &f.app,
        request("GET", "/api/v1/overview", Some(&cookie), String::new()),
    )
    .await;
    let payload = create("camera");
    let (status, first, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/components",
            Some(&cookie),
            payload.to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["record"]["owner"], "admin");
    assert_eq!(first["content_verification"], "NOT_ESTABLISHED");
    assert_eq!(
        first["execution_ownership"],
        "NOT_ESTABLISHED_BY_REGISTRATION"
    );
    assert_eq!(first["work_use_permission"], "NOT_EVALUATED");
    // Treat the first response as lost and repeat exactly the original request.
    let (status, recovered, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/components",
            Some(&cookie),
            payload.to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(recovered, first);
    let id = first["record"]["registration"]["id"].as_str().unwrap();
    let mut update = create("camera-v2");
    update["command"]["id"] = json!(id);
    update["command"]["expected_revision"] = first["revision"].clone();
    let (status, changed, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/components/update",
            Some(&cookie),
            update.to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{changed}");
    let path = format!(
        "/api/v1/component?id={id}&revision={}",
        first["revision"].as_str().unwrap()
    );
    let (status, historical, _) =
        send(&f.app, request("GET", &path, Some(&cookie), String::new())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(historical, first);
    let retire = json!({"request_key":super::id(),"command":{"id":id,"expected_revision":changed["revision"]}});
    let (status, retired, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/components/retire",
            Some(&cookie),
            retire.to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{retired}");
    assert_eq!(retired["record"]["registration"]["state"], "RETIRED");
    let (_, after, _) = send(
        &f.app,
        request("GET", "/api/v1/overview", Some(&cookie), String::new()),
    )
    .await;
    assert_eq!(before["cells"], after["cells"]);
}

#[tokio::test]
async fn component_http_rejects_forged_authority_unauthenticated_and_wrong_role_requests() {
    let f = fixture().await;
    let payload = create("camera");
    let (status, _, _) = send(
        &f.app,
        request("POST", "/api/v1/components", None, payload.to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let reader = login(&f.app, "reader").await;
    let (status, _, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/components",
            Some(&reader),
            payload.to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let admin = login(&f.app, "admin").await;
    for extra in [
        "id",
        "installation",
        "owner",
        "state",
        "execution_ownership",
        "work_use_permission",
    ] {
        let mut forged = payload.clone();
        forged["command"][extra] = json!(true);
        let (status, _, _) = send(
            &f.app,
            request(
                "POST",
                "/api/v1/components",
                Some(&admin),
                forged.to_string(),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{extra}");
    }
    let (status, created, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/components",
            Some(&admin),
            payload.to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = created["record"]["registration"]["id"].as_str().unwrap();
    let (status, _, _) = send(
        &f.app,
        request(
            "GET",
            &format!("/api/v1/component?id={id}"),
            Some(&reader),
            String::new(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
