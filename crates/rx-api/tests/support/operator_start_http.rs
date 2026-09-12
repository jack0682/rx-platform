use super::*;

#[tokio::test]
async fn operator_start_context_requires_scope_and_does_not_promote_a_session_without_terminal() {
    let f = fixture().await;
    let cookie = login(&f.app, "admin").await;
    let create = json!({"request_key":id(),"command":{"cell":"cell/a","recipe_digest":f.configuration.recipe.sha256,"site_config_digest":f.configuration.site_config_digest,"expected_cell":"1"}});
    let (status, run, _) = send(
        &f.app,
        request("POST", "/api/v1/runs", Some(&cookie), create.to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let path = format!(
        "/api/v1/run/start-context?cell=cell%2Fa&run={}&purpose=PRODUCTION&budget_limit=2",
        run["id"].as_str().unwrap()
    );
    let (_, before, _) = send(
        &f.app,
        request(
            "GET",
            "/api/v1/cell?id=cell%2Fa",
            Some(&cookie),
            String::new(),
        ),
    )
    .await;
    let (status, value, _) =
        send(&f.app, request("GET", &path, Some(&cookie), String::new())).await;
    assert_eq!(status, StatusCode::OK);
    let context: rx_application::operator_start::StartContext =
        serde_json::from_value(value).unwrap();
    assert!(!context.can_request);
    assert_eq!(
        context.blocking_reason,
        Some(rx_domain::fault::Rejection::Forbidden)
    );
    assert_eq!(context.request.budget_limit, Counter(2));
    assert_eq!(context.maximum_budget, f.configuration.maximum_budget);
    assert!(context.run.budget.is_none() && context.run.pending_attempt.is_none());
    let (_, after, _) = send(
        &f.app,
        request(
            "GET",
            "/api/v1/cell?id=cell%2Fa",
            Some(&cookie),
            String::new(),
        ),
    )
    .await;
    assert_eq!(before, after);
    assert_eq!(
        send(&f.app, request("GET", &path, None, String::new()))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let reader = login(&f.app, "reader").await;
    assert_eq!(
        send(&f.app, request("GET", &path, Some(&reader), String::new()))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let wrong = path.replace("cell%2Fa", "cell%2Fb");
    assert_eq!(
        send(&f.app, request("GET", &wrong, Some(&cookie), String::new()))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let malformed = path.replace("budget_limit=2", "budget_limit=-1");
    assert_eq!(
        send(
            &f.app,
            request("GET", &malformed, Some(&cookie), String::new())
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let other = fixture().await;
    assert_eq!(
        send(
            &other.app,
            request("GET", &path, Some(&cookie), String::new())
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    f.handle.close();
    f.handle.closed().await;
    other.handle.close();
    other.handle.closed().await;
}
