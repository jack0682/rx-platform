use super::*;
use rx_application::execution_inventory as inventory;
pub(super) fn exercise(
    f: &mut Fixture,
    target: &CellConfiguration,
    published: &publication::Publication,
    inputs: &v2::InputClosure,
    policy: &v2::Policy,
    bind_objects: bool,
) {
    let layouts = inputs.slot_resources(policy).unwrap();
    assert_eq!(
        layouts.len(),
        2,
        "distinct source/destination instances retain separate custody"
    );
    let layout = &layouts[0];
    let revision = f.app.inspect_cell(&f.admin, &target.id).unwrap().0;
    let create = inventory::CreateRun {
        cell: target.id.clone(),
        publication: published.reference.clone(),
        expected_cell: revision,
        count: Counter(1),
    };
    assert!(
        f.app
            .create_execution_run(&f.operator, &id(), create.clone())
            .is_err(),
        "geometry does not initialize inventory"
    );
    let initialize = inventory::Initialize {
        cell: target.id.clone(),
        resource: layout.resource.clone(),
        rule: layout.rule.clone(),
        expected_generation: None,
        reason: "Explicit initial SIMULATION stock declaration".into(),
    };
    let unbound = Identity {
        terminal: None,
        ..f.operator.clone()
    };
    assert!(
        f.app
            .initialize_execution_slots(&unbound, &id(), initialize.clone())
            .is_err()
    );
    let initial_key = id();
    let pool = f
        .app
        .initialize_execution_slots(&f.operator, &initial_key, initialize.clone())
        .unwrap();
    assert_eq!(pool.generation, Counter(1));
    assert!(pool.holds.is_empty());
    assert!(
        f.app
            .create_execution_run(&f.operator, &id(), create.clone())
            .is_err(),
        "every participating pool must be initialized"
    );
    assert!(
        f.app
            .execution_slot_pool(&f.operator, &layout.resource)
            .unwrap()
            .holds
            .is_empty()
    );
    let other_initialize = inventory::Initialize {
        resource: layouts[1].resource.clone(),
        rule: layouts[1].rule.clone(),
        ..initialize.clone()
    };
    f.app
        .initialize_execution_slots(&f.operator, &id(), other_initialize.clone())
        .unwrap();
    let key = id();
    f.failure.store(1, Ordering::SeqCst);
    assert!(
        f.app
            .create_execution_run(&f.operator, &key, create.clone())
            .is_err()
    );
    assert!(
        f.app
            .execution_slot_pool(&f.operator, &layout.resource)
            .unwrap()
            .holds
            .is_empty()
    );
    f.failure.store(2, Ordering::SeqCst);
    assert!(
        f.app
            .create_execution_run(&f.operator, &key, create.clone())
            .is_err()
    );
    let original = f
        .app
        .create_execution_run(&f.operator, &key, create.clone())
        .unwrap();
    assert_eq!(original.slots[0].ordinal, Counter(1));
    assert_eq!(original.slots[0].slot_ordinal, Counter(1));
    assert_eq!(original.slots[0].index, 0);
    let actual = f
        .app
        .execution_slot_pool(&f.operator, &layout.resource)
        .unwrap();
    assert_eq!(actual.holds.len(), 1);
    assert_eq!(actual.holds[&0].run, original.run);
    assert_eq!(
        f.app
            .execution_slot_pool(&f.operator, &layouts[1].resource)
            .unwrap()
            .holds[&0]
            .run,
        original.run
    );
    let actual_object = if bind_objects {
        let object = put(
            &mut f.app,
            &f.admin,
            save(
                &policy.workflow.catalog,
                Body::ObjectInstance {
                    base: policy.candidates[0].object_model.clone(),
                    values: BTreeMap::new(),
                },
            ),
        )
        .version
        .definition
        .reference;
        let command = inventory::BindObject {
            run: original.run.clone(),
            ordinal: Counter(1),
            object: object.clone(),
        };
        assert!(
            f.app
                .bind_execution_object(
                    &f.operator,
                    &id(),
                    inventory::BindObject {
                        object: policy.candidates[0].object_model.clone(),
                        ..command.clone()
                    }
                )
                .is_err()
        );
        let bind_key = id();
        f.failure.store(1, Ordering::SeqCst);
        assert!(
            f.app
                .bind_execution_object(&f.operator, &bind_key, command.clone())
                .is_err()
        );
        assert!(
            f.app
                .execution_object(&f.operator, &original.run, Counter(1))
                .is_err()
        );
        f.failure.store(2, Ordering::SeqCst);
        assert!(
            f.app
                .bind_execution_object(&f.operator, &bind_key, command.clone())
                .is_err()
        );
        let bound = f
            .app
            .bind_execution_object(&f.operator, &bind_key, command)
            .unwrap();
        assert_eq!(bound.object, object);
        assert_eq!(bound.run, original.run);
        assert_eq!(bound.slot, original.slots[0].index);
        Some(bound)
    } else {
        None
    };
    let (run_revision, run) = f.app.inspect_run(&f.operator, &original.run).unwrap();
    assert_eq!(run.state, RunState::Prepared);
    assert!(run.budget.is_none() && run.part_ids.is_empty());
    assert!(
        matches!(
            f.app.start_run(
                &f.operator,
                id().as_str(),
                StartRun {
                    run: run.id.clone(),
                    envelope_digest: target.envelope.sha256,
                    purpose: Purpose::Production,
                    budget_unit: rx_domain::budget::BudgetUnit::PartAttempt,
                    budget_limit: Counter(1),
                    expected_cell: revision,
                    expected_run: run_revision,
                }
            ),
            Err(StoreError::Rejected(Rejection::UnsupportedSchema))
        ),
        "legacy Start cannot activate v2 reservations"
    );
    let reset = inventory::Initialize {
        expected_generation: Some(Counter(1)),
        reason: "SIMULATION replenishment".into(),
        ..initialize.clone()
    };
    assert!(
        f.app
            .initialize_execution_slots(&f.operator, &id(), reset.clone())
            .is_err(),
        "Prepared Run owns its reservations"
    );
    f.app
        .abandon_run(
            &f.operator,
            id().as_str(),
            AbandonRun {
                run: run.id,
                expected_run: run_revision,
            },
        )
        .unwrap();
    let second = f
        .app
        .create_execution_run(&f.operator, &id(), create.clone())
        .unwrap();
    assert_eq!(second.slots[0].ordinal, Counter(1));
    assert_eq!(second.slots[0].slot_ordinal, Counter(2));
    assert_eq!(second.slots[0].index, 1);
    if let Some(first) = &actual_object {
        let old = f
            .app
            .definition(
                &f.admin,
                &first.object.catalog,
                &first.object.id,
                Some(first.object.revision),
            )
            .unwrap()
            .version
            .definition;
        let mut update = save(&first.object.catalog, old.body);
        update.id = first.object.id.clone();
        update.expected = Some(first.object.revision);
        update.label = "Same actual object with a revised label".into();
        let revised = put(&mut f.app, &f.admin, update)
            .version
            .definition
            .reference;
        assert!(
            f.app
                .bind_execution_object(
                    &f.operator,
                    &id(),
                    inventory::BindObject {
                        run: second.run.clone(),
                        ordinal: Counter(1),
                        object: revised
                    }
                )
                .is_err(),
            "revision cannot reset object custody"
        );
        assert_eq!(
            f.app
                .bind_execution_object(
                    &f.operator,
                    &first.request,
                    inventory::BindObject {
                        run: first.run.clone(),
                        ordinal: first.ordinal,
                        object: first.object.clone()
                    }
                )
                .unwrap()
                .object,
            first.object
        );
        assert!(
            f.app
                .bind_execution_object(
                    &f.operator,
                    &id(),
                    inventory::BindObject {
                        run: second.run.clone(),
                        ordinal: Counter(1),
                        object: first.object.clone()
                    }
                )
                .is_err(),
            "another Run cannot reuse the actual instance"
        );
        let other = put(
            &mut f.app,
            &f.admin,
            save(
                &policy.workflow.catalog,
                Body::ObjectInstance {
                    base: first.model.clone(),
                    values: BTreeMap::new(),
                },
            ),
        )
        .version
        .definition
        .reference;
        let bound = f
            .app
            .bind_execution_object(
                &f.operator,
                &id(),
                inventory::BindObject {
                    run: second.run.clone(),
                    ordinal: Counter(1),
                    object: other,
                },
            )
            .unwrap();
        assert_eq!(bound.values_digest, first.values_digest);
        assert_ne!(bound.object, first.object);
        assert_ne!(bound.run, first.run);
    }

    assert!(
        f.app
            .create_execution_run(&f.operator, &id(), create.clone())
            .is_err(),
        "abandon/new Run cannot reset stock; unapproved physical slots cannot be used"
    );
    let (r, second_run) = f.app.inspect_run(&f.operator, &second.run).unwrap();
    f.app
        .abandon_run(
            &f.operator,
            id().as_str(),
            AbandonRun {
                run: second_run.id,
                expected_run: r,
            },
        )
        .unwrap();
    let reset_key = id();
    let refreshed = f
        .app
        .initialize_execution_slots(&f.operator, &reset_key, reset.clone())
        .unwrap();
    assert_eq!(refreshed.generation, Counter(2));
    assert!(refreshed.holds.is_empty());
    assert!(
        f.app
            .initialize_execution_slots(&f.operator, &id(), reset)
            .is_err(),
        "stale generation cannot replenish twice"
    );
    assert_eq!(
        f.app
            .initialize_execution_slots(&f.operator, &initial_key, initialize)
            .unwrap()
            .generation,
        Counter(1),
        "original reply is history, not a new reset"
    );
    assert_eq!(
        f.app
            .execution_slot_pool(&f.operator, &layout.resource)
            .unwrap()
            .generation,
        Counter(2)
    );
    assert_eq!(
        f.app
            .create_execution_run(&f.operator, &key, create.clone())
            .unwrap()
            .run,
        original.run
    );
    assert!(
        f.app
            .execution_slot_pool(&f.operator, &layout.resource)
            .unwrap()
            .holds
            .is_empty(),
        "original creation replay cannot allocate again"
    );
    assert!(
        f.app
            .create_execution_run(&f.operator, &id(), create.clone())
            .is_err(),
        "resetting just one pool cannot release the other pool's slots"
    );
    f.app
        .initialize_execution_slots(
            &f.operator,
            &id(),
            inventory::Initialize {
                expected_generation: Some(Counter(1)),
                reason: "Explicit second SIMULATION pool refill".into(),
                ..other_initialize
            },
        )
        .unwrap();
    let next = f
        .app
        .create_execution_run(
            &f.operator,
            &id(),
            inventory::CreateRun {
                count: Counter(2),
                ..create
            },
        )
        .unwrap();
    assert_eq!(next.slots[0].index, 0);
    assert_eq!(next.pools[0].generation, Counter(2));
    if let Some(first) = actual_object {
        assert!(
            f.app
                .bind_execution_object(
                    &f.operator,
                    &id(),
                    inventory::BindObject {
                        run: next.run.clone(),
                        ordinal: Counter(1),
                        object: first.object.clone()
                    }
                )
                .is_err(),
            "pool refill does not reset object custody"
        );
        let fresh = put(
            &mut f.app,
            &f.admin,
            save(
                &policy.workflow.catalog,
                Body::ObjectInstance {
                    base: first.model,
                    values: BTreeMap::new(),
                },
            ),
        )
        .version
        .definition
        .reference;
        assert!(
            f.app
                .bind_execution_object(
                    &f.operator,
                    &id(),
                    inventory::BindObject {
                        run: next.run.clone(),
                        ordinal: Counter(2),
                        object: fresh
                    }
                )
                .is_err(),
            "cannot skip next Part ordinal"
        );
    }

    assert_eq!(
        f.app
            .execution_run(&f.operator, &original.run)
            .unwrap()
            .pools[0]
            .generation,
        Counter(1)
    );
    let installation = f.app.installation.id.clone();
    let bootstrap = fixture(1, false);
    let old = std::mem::replace(&mut f.app, bootstrap.app);
    drop(old.into_repository());
    let repository = FaultRepository {
        inner: SqliteRepository::open(f._directory.path().join("platform.db")).unwrap(),
        mode: f.failure.clone(),
    };
    f.app = Engine::open(
        repository,
        f.clock.clone(),
        SimulationAuthority,
        installation,
        principal("admin", &[Role::AccountAdmin]),
    )
    .unwrap();
    f.operator.session = f
        .app
        .authenticated_terminal_user_session(
            &f.operator.principal,
            id(),
            Counter(2_000_000_000_000),
            Digest::from_bytes([77; 32]),
        )
        .unwrap()
        .id;
    let persisted = f
        .app
        .execution_slot_pool(&f.operator, &layout.resource)
        .unwrap();
    assert_eq!(persisted.generation, Counter(2));
    assert_eq!(persisted.holds[&0].run, next.run);
    assert_eq!(
        f.app.execution_run(&f.operator, &original.run).unwrap().run,
        original.run
    );
}
