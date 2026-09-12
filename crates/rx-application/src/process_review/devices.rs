use super::*;
use crate::{device_binding, device_review};
use rx_process_contract::compile_input::{
    BindingPlanRef, CompileInput, DeviceSource, device_action_digest,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceDependency {
    pub plan: device_binding::Plan,
    pub job: device_review::Job,
    pub version: device_review::Version,
    pub decision: device_review::Decision,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceContext {
    pub catalog_digest: Digest,
    pub dependencies: Vec<DeviceDependency>,
}
impl DeviceContext {
    pub fn digest(&self) -> Result<Digest, String> {
        if self.dependencies.is_empty()
            || self.dependencies.len() > 16
            || self
                .dependencies
                .windows(2)
                .any(|p| p[0].plan.id >= p[1].plan.id)
            || canonical::bytes(self).map_err(|e| e.to_string())?.len() > 524_288
        {
            return Err("device review context scope/order/size".into());
        }
        canonical::digest("RX-PROCESS-DEVICE-REVIEW-CONTEXT-v1", self).map_err(|e| e.to_string())
    }
    pub fn required_cells(&self) -> BTreeSet<Name> {
        self.dependencies
            .iter()
            .flat_map(|d| d.plan.definition.impact.cells.iter().map(|c| c.id.clone()))
            .collect()
    }
}

pub(crate) fn configuration(
    job: &Job,
    input: Option<&CompileInput>,
) -> Result<CellConfiguration, String> {
    let Some(context) = &job.device_context else {
        if job.request.device_context_digest.is_some() {
            return Err("device review context missing".into());
        }
        return Ok(job.configuration.clone());
    };
    if job.request.device_context_digest != Some(context.digest()?) {
        return Err("device review context digest differs".into());
    }
    let before = &job.configuration;
    let base = canonical::digest(
        "RX-DRAFT-BINDING-CATALOG-v1",
        &(
            before.id.clone(),
            &before.definition,
            &before.envelope,
            before.site_config_digest,
            &before.steps,
        ),
    )
    .map_err(|e| e.to_string())?;
    let mut steps: BTreeMap<_, _> = before
        .steps
        .iter()
        .map(|s| (s.id.clone(), s.clone()))
        .collect();
    let mut origins = BTreeMap::new();
    let mut refs = vec![];
    for d in &context.dependencies {
        let p = &d.plan;
        if p.cell != job.request.cell
            || p.state != device_binding::State::ImpactReviewed
            || !p.definition.issues.is_empty()
            || p.plan_digest != p.digest()?
            || p.definition.base_configuration_digest != job.request.configuration_digest
        {
            return Err(
                "device plan is not a clean reviewed candidate for this configuration".into(),
            );
        }
        let reference = BindingPlanRef {
            id: p.id.clone(),
            revision: p.revision,
            plan_digest: p.plan_digest,
        };
        refs.push(reference.clone());
        for (id, candidate) in &p.definition.candidates {
            if origins.insert(id.clone(), reference.clone()).is_some() {
                return Err("device candidate collision".into());
            }
            steps.insert(id.clone(), candidate.step.clone());
        }
    }
    if steps.len() > 512
        || canonical::digest("RX-DRAFT-BINDING-CATALOG-DEVICE-v1", &(base, &refs, &steps))
            .map_err(|e| e.to_string())?
            != context.catalog_digest
    {
        return Err("device candidate catalog differs".into());
    }
    let mut expected = BTreeMap::new();
    let mut used = BTreeSet::new();
    for (alias, id) in &job.request.binding_selections {
        let step = steps.get(id).ok_or("selected candidate missing")?;
        if let Some(reference) = origins.get(id) {
            used.insert(reference.id.clone());
            expected.insert(
                alias.clone(),
                DeviceSource {
                    plan: reference.clone(),
                    binding: id.clone(),
                    step_digest: canonical::digest("RX-DRAFT-BINDING-STEP-v1", step)
                        .map_err(|e| e.to_string())?,
                    action_digest: device_action_digest(&ActionBinding {
                        host: step.host.clone(),
                        intent: step.intent.normalized().map_err(|e| e.to_string())?,
                    })?,
                },
            );
        }
    }
    if used.len() != refs.len() {
        return Err("unused device plan in review".into());
    }
    if let Some(input) = input
        && canonical::bytes(&expected).map_err(|e| e.to_string())?
            != canonical::bytes(&input.device_sources).map_err(|e| e.to_string())?
    {
        return Err("signed device provenance differs from reviewed candidates".into());
    }
    let mut cfg = before.clone();
    cfg.steps = steps.into_values().collect();
    Ok(cfg)
}

pub(super) fn verify_fresh(
    job: &Job,
    process: &StoredPackage,
    proofs: Vec<device_review::Validated>,
) -> Result<(), String> {
    configuration(job, None)?;
    let Some(context) = &job.device_context else {
        return if proofs.is_empty() {
            Ok(())
        } else {
            Err("unexpected device proof".into())
        };
    };
    if proofs.len() != context.dependencies.len() {
        return Err("fresh device proof coverage missing".into());
    }
    for (d, proof) in context.dependencies.iter().zip(proofs) {
        if d.plan.definition.builder_digest != device_binding::builder_digest()
            || proof.stored.owner() != process.owner()
            || proof.stored.policy_fingerprint() != process.policy_fingerprint()
            || proof.stored.object() != &d.plan.definition.object
            || proof.report.digest()? != d.version.report_digest
            || !proof.report.passed()
            || proof.report.request.digest()? != d.job.request.digest()?
            || d.version.review_digest != d.plan.definition.input.review.review_digest
            || d.version.revision != d.plan.definition.input.review.revision
            || d.decision.revision != d.plan.definition.input.review.decision_revision
            || d.decision.choice != device_review::Choice::Approve
        {
            return Err("fresh device proof differs from reviewed dependency".into());
        }
        let catalog = crate::device_catalog::extract(proof.stored.package())?
            .ok_or("verified device catalog absent")?;
        let (candidates, issues) = device_binding::build_candidates(
            &d.plan.definition.input,
            &job.configuration,
            &catalog,
        )?;
        if !issues.is_empty()
            || canonical::bytes(&candidates).map_err(|e| e.to_string())?
                != canonical::bytes(&d.plan.definition.candidates).map_err(|e| e.to_string())?
        {
            return Err("fresh device source no longer produces reviewed candidate".into());
        }
    }
    Ok(())
}
