use super::definition_catalog::{access, author, version as definition_version};
use super::*;
use crate::workflow_model::{
    Page, Preparation, PreparedResolution, PreparedSave, Receipt, Snapshot, Summary, Version,
};
use rx_domain::{
    definition::{Definition, Reference},
    workflow as workflow_data,
};

const MODEL: &str = "rx.internal.workflow-model.v1";
const RECEIPT: &str = "rx.internal.workflow-resolution.v1";
fn version(
    tx: &mut dyn Transaction,
    catalog: &Id,
    id: &Id,
    revision: Option<Counter>,
) -> Result<Version> {
    let (row, current): (_, Version) = load(tx, "workflowmodel", id, MODEL)?;
    if current.reference.catalog != *catalog {
        return reject(Reject::Forbidden);
    }
    if current.reference.id != *id || current.reference.revision != row {
        return Err(StoreError::Integrity(
            "workflow current reference differs".into(),
        ));
    }
    let value = if let Some(rev) = revision {
        load::<Version>(tx, "workflowmodelrevision", (id, rev), MODEL)?.1
    } else {
        current
    };
    if value.reference.id != *id
        || value.reference.catalog != *catalog
        || revision.is_some_and(|r| r != value.reference.revision)
    {
        return Err(StoreError::Integrity(
            "workflow historical reference differs".into(),
        ));
    }
    value.verify().map_err(StoreError::Integrity)?;
    Ok(value)
}
fn definitions(
    tx: &mut dyn Transaction,
    catalog: &Id,
    refs: Vec<Reference>,
) -> Result<BTreeMap<Reference, Definition>> {
    let mut result = BTreeMap::new();
    let mut pending = refs;
    while let Some(r) = pending.pop() {
        if result.contains_key(&r) {
            continue;
        }
        if r.catalog != *catalog {
            return reject(Reject::Forbidden);
        }
        if result.len() >= 512 {
            return reject(Reject::InvalidInput);
        }
        if definition_version(tx, catalog, &r.id, None)?.archived {
            return reject(Reject::InvalidInput);
        }
        let value = definition_version(tx, catalog, &r.id, Some(r.revision))?.definition;
        if value.reference != r {
            return reject(Reject::InvalidInput);
        }
        pending.extend(value.body.references().into_iter().cloned());
        result.insert(r, value);
    }
    Ok(result)
}
fn snapshot(tx: &mut dyn Transaction, request: workflow_data::Request) -> Result<Snapshot> {
    let r = &request.workflow;
    let model = version(tx, &r.catalog, &r.id, Some(r.revision))?;
    if model.reference != *r {
        return reject(Reject::InvalidInput);
    }
    let mut refs = model.spec.references();
    refs.extend(request.contexts.values().flatten().cloned());
    refs.extend(request.property_sets.iter().cloned());
    let definitions = definitions(tx, &r.catalog, refs)?;
    Ok(Snapshot {
        model,
        request,
        definitions,
    })
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn save_workflow_model(
        &mut self,
        identity: &Identity,
        key_: &Id,
        prepared: PreparedSave,
    ) -> Result<Version> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let p = author(tx, identity, meta, &now, true)?;
            lifecycle::require_serving(tx)?;
            let input = &prepared.input;
            access(tx, identity, &p, &input.catalog, true)?;
            let (scope, fingerprint) = request(
                meta,
                &p,
                "WorkflowModel.Save",
                key_.as_str(),
                &(&p.id, input),
            )?;
            if let Some(saved) = prior(tx, &scope, fingerprint, MODEL)? {
                return Ok(saved);
            }
            let k = key("workflowmodel", &input.id);
            let old = tx.get(&k)?;
            let previous = old
                .as_ref()
                .map(|r| decode::<Version>(r, MODEL))
                .transpose()?;
            if let Some(previous) = &previous {
                if previous.reference.catalog != input.catalog {
                    return reject(Reject::Forbidden);
                }
                previous.verify().map_err(StoreError::Integrity)?;
                if previous.reference.id != input.id
                    || old
                        .as_ref()
                        .is_none_or(|r| r.revision != previous.reference.revision)
                {
                    return Err(StoreError::Integrity(
                        "workflow identity/revision differs".into(),
                    ));
                }
            }
            let revision = match (&old, input.expected) {
                (None, None) => Counter(1),
                (Some(row), Some(expected)) => {
                    check_revision(row.revision, expected)?;
                    row.revision.increment().map_err(domain_error)?
                }
                _ => return reject(Reject::StaleRevision),
            };
            definitions(tx, &input.catalog, input.spec.references())?;
            let digest = Version::digest(
                &input.catalog,
                &input.id,
                revision,
                &input.label,
                &input.spec,
            )
            .map_err(StoreError::Invalid)?;
            let value = Version {
                reference: Reference {
                    catalog: input.catalog.clone(),
                    id: input.id.clone(),
                    revision,
                    digest,
                },
                label: input.label.clone(),
                spec: input.spec.clone(),
                created_by: previous.map_or_else(|| p.id.clone(), |v| v.created_by),
                updated_by: p.id,
                updated_at: now,
            };
            tx.put(&k, old.map(|r| r.revision), &doc(MODEL, &value)?)?;
            save(
                tx,
                "workflowmodelrevision",
                (&input.id, revision),
                None,
                MODEL,
                &value,
            )?;
            let index = key(&format!("workflowmodelindex/{}", input.catalog), &input.id);
            let index_old = tx.get(&index)?;
            tx.put(
                &index,
                index_old.map(|r| r.revision),
                &doc(
                    "rx.internal.workflow-model-summary.v1",
                    &Summary {
                        reference: value.reference.clone(),
                        label: value.label.clone(),
                    },
                )?,
            )?;
            event(tx, "rx.event.workflow-model-saved.v1", &value.reference)?;
            remember(tx, &scope, fingerprint, MODEL, &value)?;
            Ok(value)
        })
    }
    pub fn workflow_model(
        &mut self,
        identity: &Identity,
        catalog: &Id,
        id: &Id,
        revision: Option<Counter>,
    ) -> Result<Version> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let p = author(tx, identity, meta, &clock.now(), false)?;
            access(tx, identity, &p, catalog, false)?;
            version(tx, catalog, id, revision)
        })
    }
    pub fn workflow_models(
        &mut self,
        identity: &Identity,
        catalog: &Id,
        after: Option<&Name>,
    ) -> Result<Page> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let p = author(tx, identity, meta, &clock.now(), false)?;
            access(tx, identity, &p, catalog, false)?;
            let prefix = format!("workflowmodelindex/{catalog}/");
            if after.is_some_and(|a| !a.as_str().starts_with(&prefix)) {
                return reject(Reject::InvalidInput);
            }
            let rows = tx.scan_page(&prefix, after, 50)?;
            let next = (rows.len() == 50).then(|| rows.last().expect("nonempty page").key.clone());
            let workflows = rows
                .iter()
                .map(|r| {
                    let s: Summary = decode(r, "rx.internal.workflow-model-summary.v1")?;
                    if s.reference.catalog != *catalog {
                        return Err(StoreError::Integrity("workflow index scope differs".into()));
                    }
                    Ok(s)
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(Page {
                catalog: catalog.clone(),
                workflows,
                next,
            })
        })
    }
    pub fn prepare_workflow_resolution(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: workflow_data::Request,
    ) -> Result<Preparation> {
        input.validate_shape().map_err(StoreError::Invalid)?;
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let p = author(tx, identity, meta, &clock.now(), true)?;
            lifecycle::require_serving(tx)?;
            access(tx, identity, &p, &input.workflow.catalog, true)?;
            let (scope, fingerprint) = request(
                meta,
                &p,
                "Workflow.Resolve",
                key_.as_str(),
                &(&p.id, &input),
            )?;
            if let Some(saved) = prior(tx, &scope, fingerprint, RECEIPT)? {
                return Ok(Preparation::Recorded(Box::new(saved)));
            }
            Ok(Preparation::Pending(Box::new(snapshot(tx, input)?)))
        })
    }
    pub fn save_workflow_resolution(
        &mut self,
        identity: &Identity,
        key_: &Id,
        prepared: PreparedResolution,
    ) -> Result<Receipt> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let p = author(tx, identity, meta, &now, true)?;
            lifecycle::require_serving(tx)?;
            let input = &prepared.report.request;
            access(tx, identity, &p, &input.workflow.catalog, true)?;
            let (scope, fingerprint) =
                request(meta, &p, "Workflow.Resolve", key_.as_str(), &(&p.id, input))?;
            if let Some(saved) = prior(tx, &scope, fingerprint, RECEIPT)? {
                return Ok(saved);
            }
            // Current access/archive checks are repeated after the CPU worker. Pinned records are immutable.
            snapshot(tx, input.clone())?;
            let id = id();
            let digest = canonical::digest("RX-WORKFLOW-RESOLUTION-v1", &prepared.report)
                .map_err(domain_error)?;
            let value = Receipt {
                reference: Reference {
                    catalog: input.workflow.catalog.clone(),
                    id: id.clone(),
                    revision: Counter(1),
                    digest,
                },
                report: prepared.report,
                created_by: p.id,
                created_at: now,
            };
            save(tx, "workflowresolution", &id, None, RECEIPT, &value)?;
            let index = name(format!(
                "workflowresolutionindex/{}/{}",
                value.reference.catalog, value.reference.id
            ));
            tx.put(
                &index,
                None,
                &doc(
                    "rx.internal.workflow-resolution-summary.v1",
                    &crate::workflow_model::ReportSummary {
                        reference: value.reference.clone(),
                        workflow: value.report.request.workflow.clone(),
                        slot_index: value.report.request.slot_index,
                        status: value.report.status.clone(),
                    },
                )?,
            )?;
            event(tx, "rx.event.workflow-resolved.v1", &value.reference)?;
            remember(tx, &scope, fingerprint, RECEIPT, &value)?;
            Ok(value)
        })
    }
    pub fn workflow_resolutions(
        &mut self,
        identity: &Identity,
        catalog: &Id,
        after: Option<&Name>,
    ) -> Result<crate::workflow_model::Reports> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let p = author(tx, identity, meta, &clock.now(), false)?;
            access(tx, identity, &p, catalog, false)?;
            let prefix = format!("workflowresolutionindex/{catalog}/");
            if after.is_some_and(|v| !v.as_str().starts_with(&prefix)) {
                return reject(Reject::InvalidInput);
            }
            let rows = tx.scan_page(&prefix, after, 50)?;
            let next = (rows.len() == 50).then(|| rows.last().expect("nonempty page").key.clone());
            let reports = rows
                .iter()
                .map(|r| {
                    let value: crate::workflow_model::ReportSummary =
                        decode(r, "rx.internal.workflow-resolution-summary.v1")?;
                    if value.reference.catalog != *catalog {
                        return Err(StoreError::Integrity(
                            "resolution index scope differs".into(),
                        ));
                    }
                    Ok(value)
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(crate::workflow_model::Reports {
                catalog: catalog.clone(),
                reports,
                next,
            })
        })
    }
    pub fn workflow_resolution(
        &mut self,
        identity: &Identity,
        catalog: &Id,
        id: &Id,
    ) -> Result<Receipt> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let p = author(tx, identity, meta, &clock.now(), false)?;
            access(tx, identity, &p, catalog, false)?;
            let (row, value): (_, Receipt) = load(tx, "workflowresolution", id, RECEIPT)?;
            if value.reference.catalog != *catalog {
                return reject(Reject::Forbidden);
            }
            if value.reference.id != *id
                || value.reference.revision != row
                || row != Counter(1)
                || canonical::digest("RX-WORKFLOW-RESOLUTION-v1", &value.report)
                    .map_err(domain_error)?
                    != value.reference.digest
            {
                return Err(StoreError::Integrity("resolution receipt differs".into()));
            }
            Ok(value)
        })
    }
}
