use super::*;

fn create(label: &str) -> Value {
    json!({"request_key":id(),"command":{"declaration":{
        "label":label,"catalog":{"program":"rx/status-http","digest":"49".repeat(32)}
    }}})
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
