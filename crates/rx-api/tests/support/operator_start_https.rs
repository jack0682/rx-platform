use super::*;

#[tokio::test]
async fn terminal_https_start_context_and_attempt_reads_preserve_real_start_and_identity_checks() {
    let f = fixture(true).await;
    let (cookie, _) = login(&f, &f.a).await;
    let create = json!({"request_key":id(),"command":{"cell":"cell/a","recipe_digest":f.configuration.recipe.sha256,"site_config_digest":f.configuration.site_config_digest,"expected_cell":"2"}});
    // Read the current revision instead of assuming what qualification changed during setup.
    let current: Value =
        f.a.get(format!("{}/api/v1/cell?id=cell%2Fa", f.origin))
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    let mut create = create;
    create["command"]["expected_cell"] = current["revision"].clone();
    let response = post(&f, &f.a, "/api/v1/runs", Some(&cookie), create).await;
    assert_eq!(response.status(), StatusCode::OK);
    let run: Value = response.json().await.unwrap();
    let context_path = format!(
        "{}/api/v1/run/start-context?cell=cell%2Fa&run={}&purpose=PRODUCTION&budget_limit=2",
        f.origin,
        run["id"].as_str().unwrap()
    );
    let response =
        f.a.get(&context_path)
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let context: rx_application::operator_start::StartContext = response.json().await.unwrap();
    assert!(context.can_request && context.blocking_reason.is_none());
    assert!(context.run.budget.is_none() && context.run.pending_attempt.is_none());
    let mut stale = context.request.clone();
    stale.expected_run = Counter(0);
    assert_eq!(
        post(
            &f,
            &f.a,
            "/api/v1/runs/start",
            Some(&cookie),
            json!({"request_key":id(),"command":stale})
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    let start = json!({"request_key":id(),"command":context.request});
    let response = post(&f, &f.a, "/api/v1/runs/start", Some(&cookie), start.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let attempt: StartAttempt = response.json().await.unwrap();
    let path = format!(
        "{}/api/v1/run/start-attempt?cell=cell%2Fa&run={}&id={}",
        f.origin, context.run.id, attempt.id
    );
    let response =
        f.a.get(&path)
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let view: rx_application::operator_start::AttemptContext = response.json().await.unwrap();
    assert_eq!(view.attempt.id, attempt.id);
    assert_eq!(view.attempt.status, StartStatus::Arming);
    assert_eq!(view.run.state, RunState::Prepared);
    assert_eq!(view.run.budget.as_ref().unwrap().limit(), Counter(2));
    let repeated: StartAttempt = post(&f, &f.a, "/api/v1/runs/start", Some(&cookie), start)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(repeated.id, attempt.id);
    let blocked: rx_application::operator_start::StartContext =
        f.a.get(&context_path)
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    assert_eq!(
        blocked.blocking_reason,
        Some(rx_domain::fault::Rejection::Busy)
    );
    assert_eq!(
        f.b.get(&path)
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.a.get(path.replace("cell%2Fa", "cell%2Fb"))
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let mut terminal = f.terminal.clone();
    terminal.active = false;
    f.runtime
        .call(Command::PutTerminal {
            identity: f.root.clone(),
            terminal,
            expected: Some(Counter(1)),
        })
        .await
        .unwrap();
    assert_eq!(
        f.a.get(&context_path)
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.a.get(&path)
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    finish(f).await;
}
