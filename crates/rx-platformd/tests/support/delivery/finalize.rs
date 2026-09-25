use super::*;
use rx_application::{
    CompletionRule, Environment, FactSpec, Principal, Role, Terminal, requalification as q,
};
use rx_domain::{condition::Condition, intent::*};
use rx_platformd::config::{
    Catalog, Config, Grpc, HostLink, Https, OperatorUi, PackageIntake, PinnedFile,
};
use rx_process_contract::{CompiledBody, ResolvedProcess};
use serde_json::json;
use std::collections::BTreeSet;

const NEGATIVE_CELL: &str = "cell/physical-unconfigured";
fn reference(schema: &str, data: &[u8]) -> ArtifactRef {
    ArtifactRef {
        sha256: rx_package::content_digest(data),
        schema_id: name(schema),
        size_bytes: Counter(data.len() as u64),
    }
}
fn config_pin(out: &Path, filename: &str, value: &impl Serialize) -> Result<PinnedFile> {
    Ok(PinnedFile {
        path: PathBuf::from(format!("/config/{filename}")),
        sha256: write_json(&out.join("config").join(filename), value)?,
    })
}
fn principal(id: &str, roles: &[Role], cells: &[&str]) -> Principal {
    Principal {
        id: name(id),
        client_namespace: name(&format!("delivery/{id}")),
        roles: roles.iter().copied().collect(),
        cells: cells.iter().map(|v| name(v)).collect(),
        active: true,
    }
}
fn target(
    initial: &CellConfiguration,
    input: &CompileInput,
    resolved: &ResolvedProcess,
    package_digest: Digest,
) -> Result<CellConfiguration> {
    rx_process_contract::validation::validate(resolved)?;
    let source = input.validate()?;
    if initial.environment != Environment::Simulation
        || initial.process.is_some()
        || initial.steps.len() != 1
        || resolved.package_digest != Some(package_digest)
        || resolved.process != source.process
        || resolved.source_digest != rx_package::content_digest(&bytes(&source)?)
        || bytes(&resolved.bindings)? != bytes(&input.bindings)?
        || !resolved.conditions.is_empty()
        || !matches!(&resolved.root.body,CompiledBody::Operation {binding} if binding.as_str()==ALIAS)
    {
        return Err("actual S resolved output is not the exact one-action seed/package".into());
    }
    let actual = resolved
        .bindings
        .get(&name(ALIAS))
        .ok_or("compiled binding absent")?;
    if actual.host != initial.steps[0].host
        || actual.intent.digest()? != initial.steps[0].intent.digest()?
    {
        return Err("compiled action differs from installed seed".into());
    }
    // Same transformation as process_change::Prepared for this deliberately one-operation input.
    // Public Change.after remains the final oracle; no application/qualification is performed here.
    let mut target = initial.clone();
    target.steps[0].id = resolved.root.id.clone();
    target.steps[0].predecessors.clear();
    let data = bytes(resolved)?;
    target.recipe = reference(resolved.schema.as_str(), &data);
    if target.recipe.sha256 != rx_process_contract::frontier::resolved_digest(resolved)? {
        return Err("resolved digest calculation differs".into());
    }
    target.process = Some(Box::new(resolved.clone()));
    Ok(target)
}
fn qualification_policy(
    seed: &Seed,
    target: &CellConfiguration,
    pool: &mut BTreeMap<Digest, Vec<u8>>,
    compiler: Digest,
    validator: Digest,
) -> Result<q::Policy> {
    let data = bytes(target)?;
    let configuration = reference("rx.cell-configuration.v1", &data);
    pool.insert(configuration.sha256, data);
    let mut refs = seed.artifacts.clone();
    refs.insert(target.recipe.sha256, target.recipe.clone());
    let dependencies = q::required_dependencies(target)
        .into_iter()
        .map(|hash| {
            refs.get(&hash)
                .cloned()
                .ok_or_else(|| format!("exact dependency {hash} absent"))
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let acceptance_plan = add_artifact(
        pool,
        "rx.delivery-acceptance-plan.v1",
        json!({
            "schema":"rx.delivery-acceptance-plan.v1","environment":"SIMULATION","cell":CELL,"compiler":compiler,
            "configuration":configuration,"scope":"fresh two product images before first qualification",
            "execution_evidence_required":true,"exporter_runs_tests":false,"physical_acceptance":false
        }),
    )?;
    let limitations = add_artifact(
        pool,
        "rx.delivery-limitations.v1",
        json!({
            "schema":"rx.delivery-limitations.v1","environment":"SIMULATION","native_backend":if target.steps[0].intent.target.as_str()=="device/file-simulation" {"FILE_SIMULATION"} else {"EXPLICIT_SIMULATION_ADAPTER"},
            "physical_cell":"NOT_COMMISSIONED","real_device_endpoints":[],"claim":"software commissioning evidence only; no physical safety or field acceptance"
        }),
    )?;
    let specifications = [
        (
            q::Area::Software,
            "software",
            "Actual signed-package S compile/review and P source verification; preserve command/output identities",
        ),
        (
            q::Area::Equipment,
            "equipment",
            "Actual selected SIMULATION Host identity, binding, durable journal and ready-source observation",
        ),
        (
            q::Area::CellIntegration,
            "cell-integration",
            "Actual mTLS P/Host registration, current Fence and process-context receipt matching the complete cell scope",
        ),
        (
            q::Area::Recovery,
            "recovery",
            "Inspect exact sealed recovery test evidence and current journal continuity; no claim from empty journals alone",
        ),
        (
            q::Area::Protection,
            "protection",
            "Actual unconfigured PHYSICAL negative-cell start denial and authorization denial; no physical endpoints",
        ),
        (
            q::Area::Operations,
            "operations",
            "Actual separated accounts and independent review; main Run count and native effects remain zero before qualification activation",
        ),
    ];
    let mut criteria = Vec::new();
    for (area, label, requirement) in specifications {
        let spec = add_artifact(
            pool,
            "rx.delivery-criterion.v1",
            json!({"schema":"rx.delivery-criterion.v1","area":area,"requirement":requirement,"evidence":"external harness must supply actual source-backed artifacts; exporter generates no verdict"}),
        )?;
        criteria.push(q::Criterion {
            id: name(&format!("delivery/{label}")),
            area,
            specification: spec,
            evidence_schema: name(&format!("rx.delivery.{label}-evidence.v1")),
        });
    }
    let policy = q::Policy {
        schema: name("rx.requalification-policy.v2"),
        profiles: vec![q::Profile {
            purposes: BTreeSet::from([name("PRODUCTION")]),
            cell: name(CELL),
            configuration,
            envelope: target.envelope.clone(),
            definition: target.definition.clone(),
            environment: Environment::Simulation,
            acceptance_plan,
            limitations,
            dependencies,
            criteria,
        }],
        keys: vec![q::Key {
            id: name(QUALIFICATION_KEY),
            public_key: seed.public_signers[&name(QUALIFICATION_KEY)],
            validators: BTreeSet::from([validator]),
            environments: BTreeSet::from([name("SIMULATION")]),
        }],
    };
    policy.digest()?;
    for profile in &policy.profiles {
        for r in profile.references() {
            let data = pool
                .get(&r.sha256)
                .ok_or("qualification dependency bytes absent")?;
            if data.len() as u64 != r.size_bytes.0 || rx_package::content_digest(data) != r.sha256 {
                return Err("qualification material reference differs".into());
            }
        }
    }
    Ok(policy)
}
fn negative(
    initial: &CellConfiguration,
    pool: &mut BTreeMap<Digest, Vec<u8>>,
) -> Result<CellConfiguration> {
    let mut cell = initial.clone();
    cell.id = name(NEGATIVE_CELL);
    cell.environment = Environment::Physical;
    cell.process = None;
    cell.definition = add_artifact(
        pool,
        "rx.cell-definition.v1",
        json!({"schema":"rx.cell-definition.v1","cell":NEGATIVE_CELL,"environment":"PHYSICAL","model":null,"endpoint":null,"commissioning":"NOT_COMMISSIONED"}),
    )?;
    cell.envelope = add_artifact(
        pool,
        "rx.operating-envelope.v1",
        json!({"schema":"rx.operating-envelope.v1","cell":NEGATIVE_CELL,"environment":"PHYSICAL","unconfigured":true,"physical_parameters":null}),
    )?;
    cell.recipe = add_artifact(
        pool,
        "rx.unconfigured-recipe.v1",
        json!({"schema":"rx.unconfigured-recipe.v1","cell":NEGATIVE_CELL,"process":null}),
    )?;
    cell.site_config_digest = add_artifact(
        pool,
        "rx.site-config.v1",
        json!({"schema":"rx.site-config.v1","cell":NEGATIVE_CELL,"model":null,"endpoint":null}),
    )?
    .sha256;
    let profile = add_artifact(
        pool,
        "rx.unconfigured-profile.v1",
        json!({"schema":"rx.unconfigured-profile.v1","cell":NEGATIVE_CELL,"model":null,"endpoint":null,"operational_support_claimed":false}),
    )?;
    cell.scopes = vec![name("zone/physical-unconfigured")];
    cell.hosts = vec![name("host/physical-unconfigured")];
    cell.executor = name("executor-physical-unconfigured");
    let condition = Condition::Eq {
        fact: name("physical/unknown"),
        schema: name("boolean/v1"),
        unit: name("unitless"),
        expected: TypedValue::Boolean(true),
    };
    cell.start_conditions = vec![condition.clone()];
    cell.maintained_conditions = vec![condition.clone()];
    cell.fact_specs = vec![FactSpec {
        id: name("physical/unknown"),
        host: cell.hosts[0].clone(),
        schema: name("boolean/v1"),
        unit: name("unitless"),
        maximum_age_ns: Counter(1_000_000_000),
        maximum_uncertainty_ns: Counter(0),
    }];
    let step = &mut cell.steps[0];
    step.id = name("step/physical-unconfigured");
    step.host = cell.hosts[0].clone();
    step.conditions = vec![condition];
    step.condition_ids = vec![name("physical/unknown")];
    step.completion = CompletionRule::Unobservable;
    step.intent = Intent {
        kind: Kind::EnsureState,
        target: name("device/physical-unconfigured"),
        profile_digest: profile.sha256,
        site_config_digest: cell.site_config_digest,
        calibration_digests: vec![],
        resource_set: vec![name("controller/physical-unconfigured")],
        execution_timeout_ms: Counter(1000),
        prepare_validity_ms: Counter(100),
        completion_rule: name("unobservable"),
        cancel_rule: name("unconfigured"),
        body: Body::Predicate(PredicateGoal {
            predicate_id: name("unconfigured"),
            target: TypedValue::Boolean(true),
            settle_ms: Counter(0),
        }),
    };
    Ok(cell)
}

pub fn export() -> Result<()> {
    let seed_root = env_path("RX_CELL_DELIVERY_SEED")?;
    let SeedMaterials {
        seed,
        cell: initial,
        input,
        policy: public_policy,
        mut pool,
    } = read_seed(&seed_root)?;
    let platform_peer = Name::new(seed.installation.as_str())?;
    let compiler = env_digest("RX_CELL_COMPILER_ID")?;
    let validator = env_digest("RX_CELL_QUALIFICATION_VALIDATOR_ID")?;
    let port: u16 = std::env::var("RX_CELL_DELIVERY_PORT")?.parse()?;
    if port == 0 {
        return Err("nonzero external HTTPS port required".into());
    }
    let mut local_policy = public_policy.clone();
    for asset in &mut local_policy.assets {
        asset.path = seed_root
            .join("assets")
            .join(format!("{}.bin", asset.reference.sha256));
    }
    let verification = local_policy.load()?;
    let package_root = env_path("RX_CELL_PACKAGE")?;
    let mut files = rx_package::directory::acquire_directory(&package_root, 32, 4 * 1024 * 1024)?;
    let manifest = files
        .remove(&PackagePath::new("manifest.json")?)
        .ok_or("sealed manifest absent")?;
    let signature = files
        .remove(&PackagePath::new("manifest.sig.json")?)
        .ok_or("sealed signature absent")?;
    let package = rx_package::verify_package(&manifest, &signature, files.clone(), &verification)?;
    let original = package
        .file(&PackagePath::new("authoring/compile-input.json")?)
        .ok_or("signed compile input absent")?;
    if original != bytes(&input)?.as_slice() {
        return Err("signed compile input is not phase A input".into());
    }
    let resolved_path = env_path("RX_CELL_RESOLVED")?;
    let resolved: ResolvedProcess = read(&resolved_path)?;
    let resolved_bytes = bytes(&resolved)?;
    let target = target(&initial, &input, &resolved, package.digest())?;
    pool.insert(target.recipe.sha256, resolved_bytes.clone());
    let q_policy = qualification_policy(&seed, &target, &mut pool, compiler, validator)?;
    let physical = negative(&initial, &mut pool)?;
    let output = new_output()?;
    let tls = tls::export(&output)?;
    let catalog = Catalog {
        schema: name("rx.platform-bootstrap-catalog.v1"),
        bootstrap: principal(
            "installer",
            &[Role::AccountAdmin, Role::Engineer, Role::Observer],
            &[CELL, NEGATIVE_CELL],
        ),
        principals: vec![
            principal(
                "engineer",
                &[Role::Engineer, Role::Observer],
                &[CELL, NEGATIVE_CELL],
            ),
            principal("verifier", &[Role::Verifier, Role::Observer], &[CELL]),
            principal("release", &[Role::ReleaseManager, Role::Observer], &[CELL]),
            principal(
                "operator",
                &[Role::Operator, Role::Observer],
                &[CELL, NEGATIVE_CELL],
            ),
            principal(HOST, &[Role::Host], &[CELL]),
            principal(EXECUTOR, &[Role::Executor], &[CELL]),
            principal(
                "host/physical-unconfigured",
                &[Role::Host],
                &[NEGATIVE_CELL],
            ),
            principal(
                "executor-physical-unconfigured",
                &[Role::Executor],
                &[NEGATIVE_CELL],
            ),
        ],
        terminals: vec![Terminal {
            id: name(TERMINAL),
            certificate_digest: tls.terminal_fingerprint,
            cells: [name(CELL), name(NEGATIVE_CELL)].into(),
            active: true,
        }],
        cells: vec![initial.clone(), physical.clone()],
    };
    let passwords: BTreeMap<_, _> = ["installer", "engineer", "verifier", "release", "operator"]
        .into_iter()
        .map(|who| (who.to_owned(), format!("delivery-test-only-{who}")))
        .collect();
    let credentials = rx_api::auth::Credentials {
        schema: "rx.local-credentials.v1".into(),
        accounts: passwords
            .iter()
            .map(|(who, password)| {
                Ok(rx_api::auth::LocalAccount {
                    principal: name(who),
                    password_hash: rx_api::auth::password_hash(password)?,
                })
            })
            .collect::<std::result::Result<Vec<_>, String>>()?,
    };
    let authority = rx_application::process_review::Authority {
        schema: name("rx.process-verification-authority.v1"),
        keys: vec![rx_application::process_review::VerifierKey {
            id: name(REVIEW_KEY),
            public_key: seed.public_signers[&name(REVIEW_KEY)],
            validators: [compiler].into(),
        }],
    };
    authority.digest()?;
    let policy_pin = config_pin(&output, "package-policy.json", &public_policy)?;
    if policy_pin.sha256 != seed.package_policy_sha256 {
        return Err("public P/S package policy bytes changed".into());
    }
    let mut config = Config {
        operator_ui: None,
        package_intake: Some(PackageIntake {
            import_root: PathBuf::from("/import"),
            policy: policy_pin,
            review_authority: Some(config_pin(&output, "review-authority.json", &authority)?),
            qualification_policy: Some(config_pin(
                &output,
                "qualification-policy.json",
                &q_policy,
            )?),
            device_review_authority: None,
        }),
        schema: name("rx.platform-startup.v1"),
        installation_id: seed.installation.clone(),
        release_digest: seed.release_digest,
        data_directory: PathBuf::from("/data/platform"),
        runtime_directory: PathBuf::from("/data/runtime"),
        catalog: config_pin(&output, "catalog.json", &catalog)?,
        credentials: config_pin(&output, "credentials.json", &credentials)?,
        https: Https {
            bind: "0.0.0.0:8443".parse()?,
            origin: format!("https://127.0.0.1:{port}"),
            tls: tls.p_server.files.clone(),
        },
        grpc: Grpc {
            bind: "0.0.0.0:7443".parse()?,
            tls: tls.p_server.files,
            allowed_certificates: [
                (tls.publisher.fingerprint, name(HOST)),
                (tls.executor.fingerprint, name(EXECUTOR)),
            ]
            .into(),
        },
        host_links: vec![HostLink {
            host: name(HOST),
            cell: name(CELL),
            uri: "https://s:7444".into(),
            server_name: "s".into(),
            server_fingerprint: tls.h_server.fingerprint,
            tls: tls.p_client.files.clone(),
            ttl_ms: Counter(10000),
        }],
    };
    if let Some(bundle) = std::env::var_os("RX_CELL_OPERATOR_BUNDLE") {
        let directory = fs::canonicalize(bundle)?;
        let manifest = directory.join(rx_api::operator_ui::MANIFEST_FILENAME);
        let hash = rx_package::content_digest(&read_bytes(&manifest)?);
        rx_api::operator_ui::OperatorBundle::load(&directory, &manifest, hash)?;
        config.operator_ui = Some(OperatorUi {
            directory: PathBuf::from("/operator"),
            manifest: PinnedFile {
                path: PathBuf::from("/operator/operator-bundle.json"),
                sha256: hash,
            },
        });
    }
    write_json(&output.join("config/startup.json"), &config)?;
    for asset in &public_policy.assets {
        write(
            &output
                .join("config/assets")
                .join(format!("{}.bin", asset.reference.sha256)),
            pool.get(&asset.reference.sha256)
                .ok_or("package asset absent")?,
        )?;
    }
    write(&output.join("import/package/manifest.json"), &manifest)?;
    write(&output.join("import/package/manifest.sig.json"), &signature)?;
    for (path, data) in files {
        write(&output.join("import/package").join(path.as_str()), &data)?;
    }
    write_json(&output.join("reference/initial-cell.json"), &initial)?;
    let target_sha = write_json(&output.join("reference/target-cell.json"), &target)?;
    write_json(
        &output.join("reference/physical-unconfigured.json"),
        &physical,
    )?;
    write(&output.join("reference/resolved.json"), &resolved_bytes)?;
    for (hash, data) in &pool {
        write(
            &output
                .join("qualification-materials/artifacts")
                .join(format!("{hash}.bin")),
            data,
        )?;
    }
    write_json(
        &output.join("qualification-materials/policy.json"),
        &q_policy,
    )?;
    let backend = if let Ok(path) = std::env::var("RX_CELL_ADAPTER_DESCRIPTOR") {
        let adapter: serde_json::Value = canonical::decode_json(&std::fs::read(path)?)?;
        json!({"kind":"VALIDATED_DRIVER","profile":adapter["profile"],"driver_digest":adapter["source_digest"],"endpoint":adapter["endpoint"]})
    } else {
        json!({"kind":"FILE_SIMULATION"})
    };
    write_json(
        &output.join("host-config/startup.template.json"),
        &json!({
            "schema":"rx.host-startup.v1","installation":seed.installation,"release_digest":seed.release_digest,"host":HOST,
            "bind":"0.0.0.0:7444","data_directory":"/data/host","runtime_directory":"/run/rx-host",
            "bindings":{"path":"/config/host/bindings.json","sha256":"PATCH_FROM_PARENT_COMPOSED_BINDINGS"},"backend":backend,
            "tls":tls.h_server.files,"allowed_platform_certificates":BTreeMap::from([(tls.p_client.fingerprint,platform_peer.clone())]),
            "publisher":{"uri":"https://p:7443","server_name":"p","tls":tls.publisher.files,"store_generation":"PATCH_AFTER_FIRST_PLATFORM_START"},"publication_drain_ms":"5000"
        }),
    )?;
    write_json(
        &output.join("host-config/binding-input.json"),
        &json!({"schema":"rx.delivery-host-binding-input.v1","platform":platform_peer,"host":HOST,"initial_cell":initial,"target_cell":target,"initial_unqualified_reference":seed.host_unqualified_reference,"qualification_authorized":false}),
    )?;
    write_json(
        &output.join("executor-config/cell.template.json"),
        &json!({
            "schema":"rx.executor-cell-service.v1","service_root":"/data/executor",
            "expected_service":{"journal":seed.executor_journal,"scope":{"installation":seed.installation,"store_generation":"PATCH_AFTER_FIRST_PLATFORM_START","principal":EXECUTOR,"release":seed.release_digest,"cell":CELL,"definition":target.definition.sha256}},
            "platform":{"uri":"https://p:7443","server_name":"p","ca":tls.executor.files.ca,"certificate":tls.executor.files.certificate,"key":tls.executor.files.key},
            "engine":{"path":"/opt/rx/bin/rx-bt-engine","sha256":"PATCH_FROM_ACTUAL_S_IMAGE_ENGINE"},
            "options":{"poll_ms":50,"communication_grace_ms":5000,"stop_timeout_ms":10000}
        }),
    )?;
    let mut p_files: Vec<String> = [
        "startup.json",
        "catalog.json",
        "credentials.json",
        "package-policy.json",
        "review-authority.json",
        "qualification-policy.json",
        "pki/ca.pem",
        "pki/p-server.pem",
        "pki/p-server.key",
        "pki/p-host-client.pem",
        "pki/p-host-client.key",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    p_files.extend(
        public_policy
            .assets
            .iter()
            .map(|a| format!("assets/{}.bin", a.reference.sha256)),
    );
    write_json(
        &output.join("browser-fixture.json"),
        &json!({
            "schema":"rx.cell-delivery-browser-fixture.v1","test_only":true,"origin":config.https.origin,"cell":CELL,"negative_cell":NEGATIVE_CELL,"terminal":TERMINAL,
            "credentials":passwords,"ca":"browser/pki/ca.pem","certificate":"browser/pki/terminal.pem","private_key":"browser/pki/terminal.key",
            "allowed_mounts":{"P":{"source":"config","target":"/config","files":p_files,"private_keys":["pki/p-server.key","pki/p-host-client.key"]},"H":{"source":"host-config","target":"/config/host","files":["startup.json","bindings.json","pki/ca.pem","pki/h-server.pem","pki/h-server.key","pki/h-publisher.pem","pki/h-publisher.key"],"private_keys":["pki/h-server.key","pki/h-publisher.key"]},"E":{"source":"executor-config","target":"/config/executor","files":["cell.json","pki/ca.pem","pki/executor-client.pem","pki/executor-client.key"],"private_keys":["pki/executor-client.key"]}},
            "never_mount":["browser","signing-fixtures.json"],"ca_private_key_exported":false
        }),
    )?;
    write_json(
        &output.join("delivery.json"),
        &json!({
            "schema":"rx.cell-delivery-fixture.v1","installation":seed.installation,"platform_peer":platform_peer,"release_digest":seed.release_digest,"cell":CELL,"negative_cell":NEGATIVE_CELL,
            "contracts":seed.contracts,"binding_selections":BTreeMap::from([(name(ALIAS),name("step/cycle"))]),"package":{"manifest":package.digest(),"signature":rx_package::content_digest(&signature)},
            "initial_configuration_digest":seed.initial_cell_sha256,"target_configuration_digest":target_sha,"resolved_digest":target.recipe.sha256,
            "compiler_validator":compiler,"qualification_validator":validator,"qualification_policy_digest":q_policy.digest()?,
            "qualification_report_generated":false,"compiler_execution_asserted_by_exporter":false,
            "required_patch_stage":{"after":"first actual rx-platformd run/overview","from":"installation.store_generation","targets":["host-config/startup.template.json:publisher.store_generation","executor-config/cell.template.json:expected_service.scope.store_generation"],"other":["Host bindings exact original Intent/condition and file pin","Executor engine SHA-256 from actual S image"]},
            "required_public_assertion":"process Change.after.sha256 == target_configuration_digest; mismatch requires a fresh disposable installation, never live policy replacement"
        }),
    )?;
    Ok(())
}
