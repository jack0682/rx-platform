use super::*;
use rx_domain::canonical;

#[test]
fn runtime_skill_catalog_is_read_only_and_is_not_start_authority() {
    let mut f = fixture_with_process(1, false, false, true, Some(TestProcess::Branch));
    let before = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap();
    let catalog = f.app.runtime_skill_catalog(&f.operator).unwrap();
    assert_eq!(catalog.bindings.len(), 1);
    assert_eq!(
        catalog.bindings[0].binding.name,
        f.configuration.process.as_ref().unwrap().process
    );
    assert_eq!(
        catalog.bindings[0].binding.input_mode,
        "BOUND_CONFIGURATION"
    );
    assert_eq!(
        catalog.bindings[0].commissioning,
        Some(Commissioning::NotCommissioned)
    );
    let after = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap();
    assert_eq!(
        canonical::bytes(&before).unwrap(),
        canonical::bytes(&after).unwrap()
    );
    let run = f
        .app
        .create_run(&f.operator, id().as_str(), create_command(&f, after.0))
        .unwrap();
    assert!(
        f.app
            .start_run(
                &f.operator,
                id().as_str(),
                start_command(&f, &run, after.0, 1)
            )
            .is_err()
    );
    let result = f.app.runtime_skill_result(&f.operator, &run.id).unwrap();
    assert_eq!(result.result_owner, "PLATFORM");
    assert_eq!(result.run.value.state, RunState::Prepared);
    assert!(result.parts.is_empty() && result.work.is_empty());
    assert!(result.current_binding_matches);
    assert_eq!(
        result.binding.binding_digest,
        catalog.bindings[0].binding.binding_digest
    );
}

#[test]
fn runtime_skill_reads_follow_current_principal_scope_and_preserve_run_identity() {
    let mut f = fixture_with_process(1, true, false, true, Some(TestProcess::Branch));
    let catalog = f.app.runtime_skill_catalog(&f.operator).unwrap();
    let revision = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .0;
    let run = f
        .app
        .create_run(&f.operator, id().as_str(), create_command(&f, revision))
        .unwrap();
    let view = f.app.runtime_skill_result(&f.operator, &run.id).unwrap();
    assert_eq!(view.run.value.id, run.id);
    assert_eq!(view.binding.recipe.sha256, run.recipe_digest);
    assert_eq!(
        view.binding.binding_digest,
        catalog.bindings[0].binding.binding_digest
    );
    let mut changed = principal("operator", &[Role::Operator]);
    changed.cells.clear();
    f.app
        .put_principal(&f.admin, changed, Some(Counter(1)))
        .unwrap();
    assert!(
        f.app
            .runtime_skill_catalog(&f.operator)
            .unwrap()
            .bindings
            .is_empty()
    );
    assert!(f.app.runtime_skill_result(&f.operator, &run.id).is_err());
}
