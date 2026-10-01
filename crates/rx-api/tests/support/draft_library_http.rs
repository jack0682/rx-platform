use super::*;

#[tokio::test]
async fn library_filter_archive_and_history_use_the_real_writer() {
    let f = fixture().await;
    let cookie = login(&f.app, "admin").await;
    let draft = id();
    let mut command = json!({"id":draft,"cell":"cell/a","expected":null,"title":"Laser material supply",
        "document":{"schema":"rx.process-source.v1","process":"process/test","entry":"main","conditions":{},"flows":[]},
        "library":{"site":"site/0f","service":"service/laser","archived":false}});
    let (status, first, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/process-drafts",
            Some(&cookie),
            json!({"request_key":id(),"command":command}).to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let query = "/api/v1/process-drafts?cell=cell%2Fa&q=laser&site=site%2F0f&service=service%2Flaser&archived=false";
    let (status, page, _) = send(&f.app, request("GET", query, Some(&cookie), "".into())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["drafts"].as_array().unwrap().len(), 1);
    assert_eq!(page["drafts"][0]["library"]["archived"], false);
    command["expected"] = json!("1");
    command["library"]["archived"] = json!(true);
    let (status, _, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/process-drafts",
            Some(&cookie),
            json!({"request_key":id(),"command":command}).to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, page, _) = send(&f.app, request("GET", query, Some(&cookie), "".into())).await;
    assert!(page["drafts"].as_array().unwrap().is_empty());
    let mut omitted_state = command.clone();
    omitted_state["expected"] = json!("2");
    omitted_state["library"]
        .as_object_mut()
        .unwrap()
        .remove("archived");
    let (status, _, _) = send(
        &f.app,
        request(
            "POST",
            "/api/v1/process-drafts",
            Some(&cookie),
            json!({"request_key":id(),"command":omitted_state}).to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let path = format!("/api/v1/process-draft-history?cell=cell%2Fa&id={draft}");
    let (status, history, _) = send(&f.app, request("GET", &path, Some(&cookie), "".into())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(history["versions"].as_array().unwrap().len(), 2);
    assert_eq!(history["versions"][0]["revision"], "2");
    assert_eq!(history["versions"][0]["library"]["archived"], true);
    assert_eq!(history["versions"][1]["library"]["archived"], false);
    assert_eq!(
        history["versions"][0]["document_digest"],
        first["version"]["document_digest"]
    );
    let (status, _, _) = send(&f.app, request("GET", &path, None, "".into())).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = send(
        &f.app,
        request(
            "GET",
            &path.replace("cell%2Fa", "cell%2Fb"),
            Some(&cookie),
            "".into(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = send(
        &f.app,
        request("GET", &(path + "&before=0"), Some(&cookie), "".into()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
