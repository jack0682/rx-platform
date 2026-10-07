//! Independent deterministic checks added to, never replacing, signed qualification criteria.
use crate::{CellConfiguration, requalification as q, workflow_publication::Publication};
use rx_domain::{canonical, definition::Reference, types::*};
use rx_process_contract::execution_v2 as v2;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verification {
    pub schema: Name,
    pub configuration: ArtifactRef,
    pub publication: Reference,
    pub policy: ArtifactRef,
    pub index: ArtifactRef,
    pub compiler_digest: Digest,
    pub candidates: Counter,
    pub slots: Counter,
    pub reports_checked: Counter,
    pub dependencies: Counter,
}
fn bytes<'a>(blobs: &'a BTreeMap<Digest, Vec<u8>>, r: &ArtifactRef) -> Result<&'a [u8], String> {
    let bytes = blobs
        .get(&r.sha256)
        .ok_or("derived qualification artifact missing")?;
    v2::verify_artifact(bytes, r, v2::MAX_DEFINITION_BYTES as usize)?;
    Ok(bytes)
}
fn declared(profile: &q::Profile, r: &ArtifactRef) -> Result<(), String> {
    if !profile
        .references()
        .into_iter()
        .any(|reference| reference == r)
    {
        return Err(format!(
            "derived qualification dependency {} ({}) is not declared",
            r.sha256, r.schema_id
        ));
    }
    Ok(())
}
pub(crate) fn verify(
    policy: &q::Policy,
    report: &q::Report,
    blobs: &BTreeMap<Digest, Vec<u8>>,
) -> Result<Vec<Verification>, String> {
    let mut results = Vec::new();
    for target in &report.request.cells {
        let profile = &target.profile;
        if profile.configuration.schema_id.as_str() != "rx.cell-configuration.v2" {
            continue;
        }
        if policy.schema.as_str() != q::DERIVED_POLICY {
            return Err("v2 execution requires explicit derived qualification policy v3".into());
        }
        let configuration: CellConfiguration =
            canonical::decode_json(bytes(blobs, &profile.configuration)?)
                .map_err(|e| e.to_string())?;
        crate::engine::validate_configuration(&configuration).map_err(|e| e.to_string())?;
        if configuration.definition != profile.definition
            || configuration.envelope != profile.envelope
            || configuration.environment != profile.environment
        {
            return Err("qualification configuration/profile differs".into());
        }
        if !q::required_dependencies(&configuration)
            .is_subset(&profile.references().into_iter().map(|r| r.sha256).collect())
        {
            return Err("configuration dependency missing from qualification profile".into());
        }
        if configuration.schema() != profile.configuration.schema_id.as_str()
            || configuration.id != profile.cell
            || configuration.environment != crate::Environment::Simulation
        {
            return Err("derived configuration schema/cell/environment differs".into());
        }
        let binding = configuration
            .execution
            .as_ref()
            .ok_or("missing execution v2 binding")?;
        declared(profile, &binding.policy)?;
        let execution_policy = v2::Policy::decode(bytes(blobs, &binding.policy)?)?;
        declared(profile, &execution_policy.definition_closure)?;
        declared(profile, &execution_policy.report_index)?;
        let inputs = v2::InputClosure::decode(
            bytes(blobs, &execution_policy.definition_closure)?,
            &execution_policy,
        )?;
        let index = v2::ReportIndex::decode(
            bytes(blobs, &execution_policy.report_index)?,
            &execution_policy,
        )?;
        let plan = v2::Plan {
            schema: Name::new(v2::PLAN_SCHEMA).expect("static schema"),
            binding: (**binding).clone(),
            process: *configuration
                .process
                .clone()
                .ok_or("missing graph template")?,
        };
        if plan.reference()? != configuration.recipe
            || bytes(blobs, &configuration.recipe)?
                != canonical::bytes(&plan).map_err(|e| e.to_string())?
        {
            return Err("qualification recipe differs from v2 plan".into());
        }
        plan.verify_policy(
            &execution_policy,
            &inputs
                .spec
                .steps
                .iter()
                .map(|s| s.id.clone())
                .collect::<Vec<_>>(),
        )?;
        let mut publication = None;
        for reference in profile
            .references()
            .into_iter()
            .filter(|r| r.schema_id.as_str() == "rx.workflow-publication.v2")
        {
            let candidate: Publication =
                canonical::decode_json(bytes(blobs, reference)?).map_err(|e| e.to_string())?;
            if candidate.reference == binding.publication
                && publication.replace(candidate).is_some()
            {
                return Err("duplicate publication evidence".into());
            }
        }
        let publication = publication.ok_or("qualification publication evidence missing")?;
        if publication.reference.digest != publication.digest()?
            || publication.reference.revision != Counter(1)
            || publication.cell != configuration.id
            || publication.policy != binding.policy
            || publication
                .bindings
                .keys()
                .ne(execution_policy.templates.keys())
        {
            return Err("qualification publication identity or template coverage differs".into());
        }
        let mut dependencies = BTreeSet::new();
        for reference in profile.references() {
            dependencies.insert((reference.sha256, reference.schema_id.clone()));
        }
        // Definition bytes are carried by the verified closure, not omitted as opaque inputs.
        for definition in &inputs.definitions {
            dependencies.insert((
                rx_package::content_digest(
                    &canonical::bytes(definition).map_err(|e| e.to_string())?,
                ),
                Name::new("rx.definition.v1").expect("static schema"),
            ));
        }
        for package in publication.packages.values() {
            declared(profile, &package.catalog)?;
            if !package
                .dependencies
                .iter()
                .any(|r| r.sha256 == package.object.manifest)
                || !package
                    .dependencies
                    .iter()
                    .any(|r| r.sha256 == package.object.signature)
            {
                return Err("package signature/manifest dependency missing".into());
            }
            for reference in &package.dependencies {
                declared(profile, reference)?;
                bytes(blobs, reference)?;
            }
        }
        for (node, source) in &publication.bindings {
            let package = publication
                .packages
                .get(&source.intake)
                .ok_or("publication package missing")?;
            let catalog = v2::TemplateCatalog::decode(bytes(blobs, &package.catalog)?)?;
            let template = catalog
                .templates
                .get(&source.template)
                .ok_or("signed template missing")?;
            if catalog.cell != configuration.id
                || canonical::bytes(&template.action).map_err(|e| e.to_string())?
                    != canonical::bytes(&execution_policy.templates[node])
                        .map_err(|e| e.to_string())?
                || canonical::bytes(&template.contract).map_err(|e| e.to_string())?
                    != canonical::bytes(&execution_policy.node_contracts[node])
                        .map_err(|e| e.to_string())?
            {
                return Err("qualification signed template differs".into());
            }
        }
        if dependencies.len() > v2::MAX_DEPENDENCIES {
            return Err("derived dependency closure exceeds 1024 identities".into());
        }
        let mut count = 0;
        for candidate in 0..execution_policy.candidates.len() {
            for slot in 0..execution_policy.slot_order.len() {
                let computed =
                    v2::materialize(&execution_policy, &inputs, candidate as u8, slot as u16)?;
                computed.verify_index(&execution_policy, &index, candidate as u8, slot as u16)?;
                count += 1;
            }
        }
        results.push(Verification {
            schema: Name::new("rx.execution-domain-verification.v2").expect("static schema"),
            configuration: profile.configuration.clone(),
            publication: publication.reference,
            policy: binding.policy.clone(),
            index: execution_policy.report_index.clone(),
            compiler_digest: execution_policy.compiler_digest,
            candidates: Counter(execution_policy.candidates.len() as u64),
            slots: Counter(execution_policy.slot_order.len() as u64),
            reports_checked: Counter(count),
            dependencies: Counter(dependencies.len() as u64),
        });
    }
    Ok(results)
}
