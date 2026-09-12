use super::*;
pub async fn probe(
    channel: Channel,
    mut hello: base::PeerHello,
    definition: Vec<u8>,
    cell_hash: Vec<u8>,
) {
    hello.peer_id = "operator-api".into();
    hello.role = base::Role::OperatorApi as i32;
    hello.boot_id = id().to_string();
    let mut sessions = base::session_service_client::SessionServiceClient::new(channel.clone());
    let mut cells = cell::cell_service_client::CellServiceClient::new(channel.clone());
    let session = sessions.open(hello.clone()).await.unwrap().into_inner();
    let inspect = cell::CellCall {
        context: Some(call(&session)),
        cell_id: "cell/a".into(),
        expected_cell_revision: None,
    };
    assert_eq!(
        cells.inspect(inspect.clone()).await.unwrap_err().code(),
        tonic::Code::FailedPrecondition
    );
    let negotiate = cell::CellHello {
        base_session_id: session.session_id.clone(),
        peer_id: "operator-api".into(),
        base_manifest_hash: hello.supported_versions[0].schema_hash.clone(),
        cell_manifest_hash: cell_hash,
        cell_definition_digest: definition,
        shared_clock_id: hello.shared_clock_id.clone(),
    };
    cells.open(negotiate.clone()).await.unwrap();
    let first = cells.inspect(inspect.clone()).await.unwrap().into_inner();
    assert_eq!(first.cell_id, "cell/a");
    assert_eq!(first.mode, cell::OperatingMode::Setup as i32);
    assert_eq!(
        first.commissioning,
        cell::Commissioning::NotCommissioned as i32
    );
    assert!(first.blocks.is_empty());
    let mut invalid = inspect.clone();
    invalid.expected_cell_revision = Some(first.revision);
    assert_eq!(
        cells.inspect(invalid).await.unwrap_err().code(),
        tonic::Code::InvalidArgument
    );
    let mut other = inspect.clone();
    other.cell_id = "cell/b".into();
    assert_eq!(
        cells.inspect(other).await.unwrap_err().code(),
        tonic::Code::PermissionDenied
    );
    let error = cells
        .open_case(cell::OpenCaseRequest {
            call: Some(inspect.clone()),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::PermissionDenied);
    let mut workflow = base::workflow_service_client::WorkflowServiceClient::new(channel);
    assert_eq!(
        workflow
            .get_run(base::RunRequest {
                context: inspect.context.clone(),
                run_id: Some(id().to_string()),
                ..Default::default()
            })
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    let original = hello.clone();
    hello.boot_id = id().to_string();
    let second = sessions.open(hello.clone()).await.unwrap().into_inner();
    assert_ne!(second.session_id, session.session_id);
    assert_eq!(
        cells.inspect(inspect).await.unwrap_err().code(),
        tonic::Code::Unauthenticated
    );
    assert_eq!(
        sessions.open(original).await.unwrap_err().code(),
        tonic::Code::Unauthenticated
    );
    let next = cell::CellCall {
        context: Some(call(&second)),
        cell_id: "cell/a".into(),
        expected_cell_revision: None,
    };
    assert!(cells.inspect(next.clone()).await.is_err());
    let mut negotiate = negotiate;
    negotiate.base_session_id = second.session_id;
    cells.open(negotiate).await.unwrap();
    assert_eq!(cells.inspect(next).await.unwrap().into_inner(), first);
}
