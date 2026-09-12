use super::*;
use rx_application::{CellConfiguration, CompletionRule, Environment, FactSpec, StepBinding};
use rx_domain::{condition::Condition, intent::*};
use rx_package::{Architecture, OperatingSystem, PackageKind, Permission, Target};
use rx_process_contract::model::ActionBinding;
use serde_json::json;
use std::collections::BTreeSet;

pub fn export() -> Result<()> {
    let output = new_output()?;
    let installation = id();
    let architecture = match std::env::var("RX_CELL_DELIVERY_ARCH")
        .as_deref()
        .unwrap_or("arm64")
    {
        "arm64" => Architecture::Arm64,
        "amd64" => Architecture::Amd64,
        _ => return Err("arm64 or amd64 architecture required".into()),
    };
    let manifest_hash = |raw: &str| -> Result<Digest> {
        let value: serde_json::Value = canonical::decode_json(raw.as_bytes())?;
        Ok(rx_package::content_digest(&bytes(&value)?))
    };
    let contracts = ContractSet {
        base: manifest_hash(include_str!(
            "../../../../../spec/contracts/v1.0/protocol_manifest.json"
        ))?,
        cell: manifest_hash(include_str!(
            "../../../../../spec/cell_operations/v1.0/protocol_manifest.json"
        ))?,
        package_abi: name("rx.package-abi.v1"),
    };
    let signers = signing::fixtures();
    let public_signers: BTreeMap<_, _> = signers
        .keys
        .iter()
        .map(|k| (k.id.clone(), k.public_key))
        .collect();
    let mut pool = BTreeMap::new();
    let mut references = BTreeMap::new();
    let mut material = |schema: &str, body: serde_json::Value| -> Result<ArtifactRef> {
        let reference = add_artifact(&mut pool, schema, body)?;
        references.insert(reference.sha256, reference.clone());
        Ok(reference)
    };
    let release = material(
        "rx.delivery-test-release.v1",
        json!({"schema":"rx.delivery-test-release.v1","installation":installation,"environment":"SIMULATION","meaning":"integration release identity; actual image IDs recorded by external harness"}),
    )?;
    let definition = material(
        "rx.cell-definition.v1",
        json!({"schema":"rx.cell-definition.v1","cell":CELL,"environment":"SIMULATION","host":HOST,"executor":EXECUTOR,"scope":"zone/simulation","resource":"controller/simulation"}),
    )?;
    let envelope = material(
        "rx.operating-envelope.v1",
        json!({"schema":"rx.operating-envelope.v1","cell":CELL,"environment":"SIMULATION","purpose":"PRODUCTION","maximum_part_attempts":10,"native_backend":"FILE_SIMULATION","physical_motion_authorized":false,"dispatch_permit_validity_ns":"1000000000","read_snapshot_validity_ns":"100000000","timing_basis":"FILE_SIMULATION: Prepare and Authorize cross separate 100ms dispatcher ticks; native/condition/grant expiry checks remain mandatory"}),
    )?;
    let site = material(
        "rx.site-config.v1",
        json!({"schema":"rx.site-config.v1","installation":installation,"cell":CELL,"environment":"SIMULATION","native_endpoints":[],"source":"ready"}),
    )?;
    let profile = material(
        "rx.device-profile.v1",
        json!({"schema":"rx.device-profile.v1","environment":"SIMULATION","backend":"FILE_SIMULATION","ready_source":"ready","condition_id":"sim/ready","native_result":{"schema":"rx.sim.completed.v1","success_code":"0"},"physical_device":null,"dispatch_window_scope":"FILE_SIMULATION_ONLY","dispatch_permit_validity_ns":"1000000000"}),
    )?;
    let program = material(
        "rx.sim.program.v1",
        json!({"schema":"rx.sim.program.v1","effect":"FILE_SIMULATION_RECORD_ONLY","cell":CELL}),
    )?;
    let parameters = material(
        "rx.sim.parameters.v1",
        json!({"schema":"rx.sim.parameters.v1","operation":"cycle","physical_parameters":null}),
    )?;
    let initial_recipe = material(
        "rx.uncompiled-process-reference.v1",
        json!({"schema":"rx.uncompiled-process-reference.v1","process":"delivery/cycle","status":"AWAITING_SIGNED_S_COMPILATION"}),
    )?;
    let ready = Condition::Eq {
        fact: name("ready"),
        schema: name("boolean/v1"),
        unit: name("unitless"),
        expected: TypedValue::Boolean(true),
    };
    let intent = Intent {
        kind: Kind::FiniteAction,
        target: name("device/file-simulation"),
        profile_digest: profile.sha256,
        site_config_digest: site.sha256,
        calibration_digests: vec![],
        resource_set: vec![name("controller/simulation")],
        execution_timeout_ms: Counter(5000),
        prepare_validity_ms: Counter(1000),
        completion_rule: name("rx.sim.completed.v1"),
        cancel_rule: name("rx.sim.stop.v1"),
        body: Body::Program(ProgramGoal {
            program: program.clone(),
            parameter_set: parameters.clone(),
        }),
    };
    let configuration = CellConfiguration {
        process: None,
        id: name(CELL),
        environment: Environment::Simulation,
        definition,
        envelope,
        recipe: initial_recipe,
        site_config_digest: site.sha256,
        scopes: vec![name("zone/simulation")],
        hosts: vec![name(HOST)],
        executor: name(EXECUTOR),
        maximum_budget: Counter(10),
        // Read snapshots remain 100ms. This two-stage dispatch window is independently configured.
        permit_ttl_ns: Counter(1_000_000_000),
        start_timeout_ns: Counter(5_000_000_000),
        start_conditions: vec![ready.clone()],
        maintained_conditions: vec![ready.clone()],
        fact_specs: vec![FactSpec {
            id: name("ready"),
            host: name(HOST),
            schema: name("boolean/v1"),
            unit: name("unitless"),
            maximum_age_ns: Counter(2_000_000_000),
            maximum_uncertainty_ns: Counter(0),
        }],
        steps: vec![StepBinding {
            id: name("step/cycle"),
            host: name(HOST),
            predecessors: vec![],
            intent,
            conditions: vec![ready.clone()],
            condition_ids: vec![name("sim/ready")],
            condition_revision: Counter(1),
            handover_max_age_ns: Counter(500_000_000),
            completion: CompletionRule::Native {
                schema: name("rx.sim.completed.v1"),
                success: vec![Integer(0)],
                failure: vec![Integer(1)],
                postconditions: vec![ready],
            },
        }],
    };
    let source = json!({"schema":"rx.process-source.v1","process":"delivery/cycle","entry":"main","conditions":{},"flows":[{"id":"main","root":"cycle","nodes":[{"id":"cycle","body":{"kind":"OPERATION","binding":ALIAS}}]}]});
    let bindings = BTreeMap::from([(
        name(ALIAS),
        ActionBinding {
            host: name(HOST),
            intent: configuration.steps[0].intent.normalized()?,
        },
    )]);
    let input = CompileInput {
        device_sources: BTreeMap::new(),
        schema: name("rx.process-compile-input.v1"),
        draft: id(),
        cell: name(CELL),
        source_revision: Counter(1),
        binding_revision: Counter(1),
        source_document_digest: canonical::digest("RX-PROCESS-DRAFT-DOCUMENT-v1", &source)?,
        bindings_digest: rx_process_contract::compile_input::bindings_digest(
            &bindings,
            &BTreeMap::new(),
        )?,
        catalog_digest: canonical::digest(
            "RX-DRAFT-BINDING-CATALOG-v1",
            &(
                configuration.id.clone(),
                &configuration.definition,
                &configuration.envelope,
                configuration.site_config_digest,
                &configuration.steps,
            ),
        )?,
        source,
        bindings,
    };
    input.validate()?;
    let target = Target {
        os: OperatingSystem::Linux,
        architecture,
        ros_distribution: None,
    };
    let recipe = json!({"schema":"rx.process-package-recipe.v1","package":"delivery/cycle","version":"0.1.0","publisher":"delivery-test-only","contracts":contracts,"targets":[target],"dependencies":[],"assets":[program,parameters]});
    let policy = policy::Policy {
        additional_package_abis: vec![],
        schema: name("rx.package-verification-policy.v1"),
        contracts: contracts.clone(),
        target,
        keys: vec![policy::Key {
            id: name(PACKAGE_KEY),
            publisher: name("delivery-test-only"),
            verifying_key: public_signers[&name(PACKAGE_KEY)],
            kinds: BTreeSet::from([PackageKind::Process]),
            permissions: BTreeSet::from([
                Permission::ArtifactRead,
                Permission::OperationSubmit {
                    operation: name(ALIAS),
                },
            ]),
        }],
        assets: [program, parameters]
            .into_iter()
            .map(|reference| policy::Asset {
                path: PathBuf::from(format!("/config/assets/{}.bin", reference.sha256)),
                reference,
            })
            .collect(),
        dependencies: vec![],
    };
    let seed = Seed {
        schema: name("rx.delivery-fixture-seed.v1"),
        installation,
        release_digest: release.sha256,
        executor_journal: id(),
        host_unqualified_reference: id(),
        contracts,
        initial_cell_sha256: write_json(&output.join("initial-cell.json"), &configuration)?,
        compile_input_sha256: write_json(&output.join("compile-input.json"), &input)?,
        package_recipe_sha256: write_json(&output.join("package-recipe.json"), &recipe)?,
        package_policy_sha256: write_json(&output.join("package-policy.json"), &policy)?,
        artifacts: references,
        public_signers,
    };
    for (hash, data) in pool {
        write(&output.join("assets").join(format!("{hash}.bin")), &data)?;
    }
    write_json(&output.join("signing-fixtures.json"), &signers)?;
    write_json(&output.join("seed.json"), &seed)?;
    write_json(
        &output.join("scope.json"),
        &json!({"schema":"rx.delivery-fixture-scope.v1","environment":"SIMULATION","compiler_executed":false,"qualification_report_generated":false,"copy_to_runtime_forbidden":["signing-fixtures.json"],"mount_seed_public_root":"/config"}),
    )?;
    Ok(())
}
