use super::*;
use rx_process_contract::*;
use rx_protocol::executor_plan as wire;

pub fn configuration(configuration: CellConfiguration, kind: &str) -> CellConfiguration {
    let mut configuration = graph_configuration(configuration);
    let process = configuration.process.as_mut().unwrap();
    let make = |label: &str, body| {
        let source = SourceLocation {
            flow: name("main"),
            node: name(label),
            instantiation: vec!["flow:main".into()],
        };
        CompiledNode {
            id: name(&format!(
                "node/{}",
                canonical::digest("RX-PROCESS-NODE-v1", &(&process.process, &source)).unwrap()
            )),
            source,
            body,
        }
    };
    let leaf = process.root.clone();
    process
        .conditions
        .insert(name("ready"), configuration.start_conditions[0].clone());
    process.root = if kind == "BRANCH" {
        let other = make(
            "other",
            CompiledBody::Operation {
                binding: name("other"),
            },
        );
        let mut step = configuration.steps[0].clone();
        step.id = other.id.clone();
        process.bindings.insert(
            name("other"),
            ActionBinding {
                host: step.host.clone(),
                intent: step.intent.clone(),
            },
        );
        configuration.steps.push(step);
        make(
            "branch",
            CompiledBody::Branch {
                condition: name("ready"),
                when_true: Box::new(leaf),
                when_false: Box::new(other),
            },
        )
    } else {
        let wait = make(
            "wait",
            CompiledBody::Wait {
                condition: name("ready"),
                timeout_ns: Counter(1000),
            },
        );
        make(
            "sequence",
            CompiledBody::Sequence {
                children: vec![wait, leaf],
            },
        )
    };
    configuration.recipe.sha256 = frontier::resolved_digest(process).unwrap();
    configuration.recipe.size_bytes = Counter(canonical::bytes(process).unwrap().len() as u64);
    configuration
}

pub async fn exercise(
    channel: Channel,
    session: &base::Session,
    run: &Id,
    configuration: &CellConfiguration,
    kind: &str,
) {
    let node = validation::nodes(configuration.process.as_ref().unwrap())
        .into_iter()
        .find(|n| {
            matches!(
                n.body,
                CompiledBody::Branch { .. } | CompiledBody::Wait { .. }
            )
        })
        .unwrap()
        .id
        .clone();
    let mut plans =
        wire::executor_plan_service_client::ExecutorPlanServiceClient::new(channel.clone());
    let mut workflow = base::workflow_service_client::WorkflowServiceClient::new(channel.clone());
    let mut reads =
        rx_protocol::executor::executor_read_service_client::ExecutorReadServiceClient::new(
            channel,
        );
    let mut request = wire::PrepareCheckpoint {
        context: Some(call(session)),
        run_id: run.to_string(),
        node_id: node.to_string(),
        visit: 1,
        action: if kind == "BRANCH" {
            wire::CheckpointAction::ChooseBranch
        } else {
            wire::CheckpointAction::StartWait
        } as i32,
        binding_hash: manifest(include_str!(
            "../../../../spec/executor-plan/v1/binding.json"
        )),
    };
    let mut wrong = request.clone();
    wrong.binding_hash[0] ^= 1;
    assert_eq!(
        plans.prepare_checkpoint(wrong).await.unwrap_err().code(),
        tonic::Code::FailedPrecondition
    );
    let before = workflow
        .get_run(base::RunRequest {
            context: Some(call(session)),
            run_id: Some(run.to_string()),
            recipe_digest: vec![],
            site_config_digest: vec![],
        })
        .await
        .unwrap()
        .into_inner();
    let first = plans
        .prepare_checkpoint(request.clone())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(first.state, wire::PreparationState::Ready as i32);
    assert_eq!(first.expected_revision, before.revision);
    assert_eq!(
        first,
        plans
            .prepare_checkpoint(request.clone())
            .await
            .unwrap()
            .into_inner()
    );
    let unchanged = workflow
        .get_run(base::RunRequest {
            context: Some(call(session)),
            run_id: Some(run.to_string()),
            recipe_digest: vec![],
            site_config_digest: vec![],
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(unchanged, before);
    let mut commit = base::ChangeCheckpoint {
        context: Some(base::CallContext {
            request_key: Some(id().to_string()),
            ..call(session)
        }),
        run_id: run.to_string(),
        expected_revision: first.expected_revision,
        new_checkpoint: first.checkpoint.clone(),
    };
    let mut bad_context = commit.clone();
    bad_context.context.as_mut().unwrap().expected_revision = Some(first.expected_revision);
    assert_eq!(
        workflow
            .commit_checkpoint(bad_context)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::InvalidArgument
    );
    // The fixture drops the first successful commit response after its actual transaction.
    assert_eq!(
        workflow
            .commit_checkpoint(commit.clone())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unavailable
    );
    let accepted = workflow
        .commit_checkpoint(commit.clone())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(accepted.revision, before.revision + 1);
    assert_eq!(accepted.checkpoint, first.checkpoint);
    let mut changed = commit.clone();
    changed.new_checkpoint.as_mut().unwrap().activations.clear();
    changed.expected_revision += 1;
    assert_eq!(
        workflow
            .commit_checkpoint(changed)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::AlreadyExists
    );
    let applied = plans
        .prepare_checkpoint(request.clone())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(applied.state, wire::PreparationState::AlreadyApplied as i32);
    assert!(
        applied.checkpoint.is_none()
            && applied.prepared_at.is_none()
            && applied.valid_until.is_none()
    );
    if kind == "WAIT" {
        request.action = wire::CheckpointAction::CheckWait as i32;
        let ready = plans
            .prepare_checkpoint(request)
            .await
            .unwrap()
            .into_inner();
        assert_eq!(ready.state, wire::PreparationState::Ready as i32);
        commit.context.as_mut().unwrap().request_key = Some(id().to_string());
        commit.expected_revision = ready.expected_revision;
        commit.new_checkpoint = ready.checkpoint;
        let result = workflow
            .commit_checkpoint(commit)
            .await
            .unwrap()
            .into_inner();
        assert_eq!(result.revision, accepted.revision + 1);
    }
    let artifact = first.checkpoint.unwrap().payload.unwrap();
    let payload = reads
        .get_artifact(rx_protocol::executor::ArtifactRequest {
            context: Some(call(session)),
            run_id: run.to_string(),
            reference: Some(artifact),
            binding_hash: manifest(include_str!("../../../../spec/executor/v1/binding.json")),
        })
        .await
        .unwrap()
        .into_inner();
    let state: execution::ExecutorState = canonical::decode_json(&payload.payload).unwrap();
    assert_eq!(state.revision.0, accepted.revision);
    let cp = &state.process_checkpoints[0];
    if kind == "BRANCH" {
        assert!(cp.branches[&node].chosen);
    } else {
        assert_eq!(cp.wait_windows[&node].started_at.ticks_ns, Counter(1000));
        assert!(cp.waits.is_empty());
    }
}
