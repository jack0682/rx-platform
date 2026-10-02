//! Check v2 publication links before persisting configuration; this grants no authority.
use super::*;
use rx_process_contract::execution_v2 as v2;
const BLOBS: crate::artifact_storage::BlobStore =
    crate::artifact_storage::BlobStore::new("executionv2", v2::MAX_DEFINITION_BYTES);
pub(super) fn plan(c: &CellConfiguration) -> Result<Option<v2::Plan>> {
    c.execution
        .as_ref()
        .map(|binding| {
            if c.environment != Environment::Simulation {
                return reject(Reject::InvalidInput);
            }
            let process = c
                .process
                .as_ref()
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
            let plan = v2::Plan {
                schema: name(v2::PLAN_SCHEMA),
                binding: (**binding).clone(),
                process: (**process).clone(),
            };
            plan.validate().map_err(StoreError::Invalid)?;
            Ok(plan)
        })
        .transpose()
}
pub(super) fn verify(tx: &mut dyn Transaction, c: &CellConfiguration) -> Result<()> {
    let Some(plan) = plan(c)? else {
        return Ok(());
    };
    if plan.reference().map_err(StoreError::Invalid)? != c.recipe {
        return reject(Reject::InvalidInput);
    }
    let (revision, p): (_, crate::workflow_publication::Publication) = load(
        tx,
        "workflowpublication",
        &plan.binding.publication.id,
        "rx.workflow-publication.v2",
    )?;
    if revision != Counter(1)
        || p.reference != plan.binding.publication
        || p.cell != c.id
        || p.policy != plan.binding.policy
        || p.reference.digest != p.digest().map_err(StoreError::Integrity)?
    {
        return reject(Reject::InvalidInput);
    }
    let (revision, preview): (_, crate::workflow_publication::Preview) = load(
        tx,
        "executionpreview",
        &p.preview.id,
        "rx.execution-preview.v2",
    )?;
    if revision != Counter(1)
        || preview.reference != p.preview
        || preview.reference.digest != preview.digest().map_err(StoreError::Integrity)?
        || preview.policy != p.policy
    {
        return Err(StoreError::Integrity("publication preview differs".into()));
    }
    let policy = v2::Policy::decode(&BLOBS.read(tx, &p.policy)?).map_err(StoreError::Integrity)?;
    let inputs = v2::InputClosure::decode(&BLOBS.read(tx, &policy.definition_closure)?, &policy)
        .map_err(StoreError::Integrity)?;
    v2::ReportIndex::decode(&BLOBS.read(tx, &policy.report_index)?, &policy)
        .map_err(StoreError::Integrity)?;
    if preview.inputs != policy.definition_closure
        || preview.index != policy.report_index
        || preview.workflow != policy.workflow
    {
        return Err(StoreError::Integrity(
            "execution policy provenance differs".into(),
        ));
    }
    let order = inputs
        .spec
        .steps
        .iter()
        .map(|s| s.id.clone())
        .collect::<Vec<_>>();
    plan.verify_policy(&policy, &order)
        .map_err(StoreError::Invalid)?;
    let current =
        workflow_model::current_snapshot(tx, &inputs.requests, policy.slot_order.len() as u16)?;
    if canonical::bytes(&current.input_closure()).map_err(domain_error)?
        != canonical::bytes(&inputs).map_err(domain_error)?
    {
        return reject(Reject::StaleRevision);
    }
    tx.require_workflow_execution_reader()
}
pub(super) fn host_policy(
    tx: &mut dyn Transaction,
    c: &CellConfiguration,
    host: &Name,
) -> Result<Option<v2::host_configuration::CellPolicy>> {
    let Some(binding) = &c.execution else {
        return Ok(None);
    };
    verify(tx, c)?;
    let (_, p): (_, crate::workflow_publication::Publication) = load(
        tx,
        "workflowpublication",
        &binding.publication.id,
        "rx.workflow-publication.v2",
    )?;
    let policy = v2::Policy::decode(&BLOBS.read(tx, &p.policy)?).map_err(StoreError::Integrity)?;
    let mut packages = BTreeMap::new();
    for (node, _) in policy.templates.iter().filter(|(_, a)| &a.host == host) {
        let source = p
            .bindings
            .get(node)
            .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
        let package = p
            .packages
            .get(&source.intake)
            .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
        packages.insert(
            node.clone(),
            v2::host_configuration::Package {
                manifest: package.object.manifest,
                signature: package.object.signature,
                catalog: package.catalog.clone(),
                template: source.template.clone(),
            },
        );
    }
    Ok(Some(v2::host_configuration::CellPolicy {
        publication: p.reference,
        reference: p.policy,
        policy,
        packages,
    }))
}
