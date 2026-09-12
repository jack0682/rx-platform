use super::*;
use crate::{draft_bindings::*, process_draft::Version as DraftVersion};
use rx_process_contract::model::ActionBinding;
const DRAFT: &str = "rx.internal.process-draft-version.v1";
const BINDINGS: &str = "rx.internal.draft-bindings.v1";
fn catalog(cell: &CellConfiguration) -> Result<Catalog> {
    let digest = canonical::digest(
        "RX-DRAFT-BINDING-CATALOG-v1",
        &(
            cell.id.clone(),
            &cell.definition,
            &cell.envelope,
            cell.site_config_digest,
            &cell.steps,
        ),
    )
    .map_err(domain_error)?;
    let candidates = cell
        .steps
        .iter()
        .map(|step| {
            Ok(Candidate {
                device_plan: None,
                step: step.id.clone(),
                host: step.host.clone(),
                target: step.intent.target.clone(),
                kind: step.intent.kind,
                resources: step.intent.resource_set.clone(),
                intent_digest: step.intent.digest().map_err(domain_error)?,
                step_digest: canonical::digest("RX-DRAFT-BINDING-STEP-v1", step)
                    .map_err(domain_error)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Catalog {
        device_plans: vec![],
        required_cells: vec![],
        cell: cell.id.clone(),
        definition: cell.definition.sha256,
        envelope: cell.envelope.sha256,
        catalog_digest: digest,
        candidates,
    })
}
struct SelectedCatalog {
    view: Catalog,
    steps: BTreeMap<Name, StepBinding>,
    sources: BTreeMap<Name, BindingPlanRef>,
}
fn scope_access(actor: &Principal, cells: &[Name]) -> Result<()> {
    if cells.iter().any(|c| !actor.cells.contains(c)) {
        return reject(Reject::Forbidden);
    }
    Ok(())
}
fn selected_catalog(
    tx: &mut dyn Transaction,
    meta: &Installation,
    cell: &CellConfiguration,
    actor: &Principal,
    plans: &[BindingPlanRef],
) -> Result<SelectedCatalog> {
    let mut view = catalog(cell)?;
    let mut steps = cell
        .steps
        .iter()
        .map(|s| (s.id.clone(), s.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut sources = BTreeMap::new();
    let mut required = BTreeSet::new();
    if plans.len() > 16 || plans.iter().map(|p| &p.id).collect::<BTreeSet<_>>().len() != plans.len()
    {
        return reject(Reject::InvalidInput);
    }
    let mut sorted = plans.to_vec();
    sorted.sort();
    for reference in &sorted {
        let plan = device_binding::read(tx, &reference.id, &cell.id)?;
        process_change::access(actor, &plan.definition.impact)?;
        if plan.revision != reference.revision
            || plan.plan_digest != reference.plan_digest
            || plan.state != crate::device_binding::State::ImpactReviewed
            || !plan.definition.issues.is_empty()
            || !device_binding::current(tx, &plan)?
        {
            return reject(Reject::StaleRevision);
        }
        device_binding::approved(tx, meta, &cell.id, &plan.definition.input.review)?;
        required.extend(plan.definition.impact.cells.iter().map(|c| c.id.clone()));
        for (id, candidate) in plan.definition.candidates {
            if sources.insert(id.clone(), reference.clone()).is_some() {
                return reject(Reject::InvalidInput);
            }
            steps.insert(id, candidate.step);
        }
    }
    if steps.len() > 512 {
        return reject(Reject::InvalidInput);
    }
    if !plans.is_empty() {
        view.catalog_digest = canonical::digest(
            "RX-DRAFT-BINDING-CATALOG-DEVICE-v1",
            &(view.catalog_digest, &sorted, &steps),
        )
        .map_err(domain_error)?;
        view.device_plans = sorted;
        view.required_cells = required.into_iter().collect();
        view.candidates = steps
            .values()
            .map(|s| {
                Ok(Candidate {
                    device_plan: sources.get(&s.id).cloned(),
                    step: s.id.clone(),
                    host: s.host.clone(),
                    target: s.intent.target.clone(),
                    kind: s.intent.kind,
                    resources: s.intent.resource_set.clone(),
                    intent_digest: s.intent.digest().map_err(domain_error)?,
                    step_digest: canonical::digest("RX-DRAFT-BINDING-STEP-v1", s)
                        .map_err(domain_error)?,
                })
            })
            .collect::<Result<_>>()?;
    }
    Ok(SelectedCatalog {
        view,
        steps,
        sources,
    })
}
fn draft(tx: &mut dyn Transaction, cell: &Name, id: &Id) -> Result<DraftVersion> {
    let (_, d): (_, DraftVersion) = load(tx, "processdraft", id, DRAFT)?;
    if d.cell != *cell {
        return reject(Reject::Forbidden);
    }
    Ok(d)
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn draft_device_binding_catalog(
        &mut self,
        identity: &Identity,
        cell: &Name,
        plans: Vec<BindingPlanRef>,
    ) -> Result<Catalog> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            process_draft::read_access(tx, identity, meta, &clock.now(), cell)?;
            let actor = authorize_identity(tx, identity, meta, &clock.now())?;
            let (_, c): (_, Cell) = load(tx, "cell", cell, CELL)?;
            Ok(selected_catalog(tx, meta, &c.configuration, &actor, &plans)?.view)
        })
    }
    pub fn draft_binding_catalog(&mut self, identity: &Identity, cell: &Name) -> Result<Catalog> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            super::process_draft::read_access(tx, identity, meta, &clock.now(), cell)?;
            let (_, c): (_, Cell) = load(tx, "cell", cell, CELL)?;
            catalog(&c.configuration)
        })
    }
    pub fn save_draft_bindings(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: Save,
    ) -> Result<Version> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let p = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&input.cell),
                Role::Engineer,
                false,
            )?;
            let d = draft(tx, &input.cell, &input.draft)?;
            let (scope, fingerprint) = request(
                meta,
                &p,
                "ProcessDraft.Bindings.Save",
                key_.as_str(),
                &input,
            )?;
            if let Some(v) = prior::<Version>(tx, &scope, fingerprint, BINDINGS)? {
                scope_access(&p, &v.required_cells)?;
                return Ok(v);
            }
            if d.revision != input.source_revision {
                return reject(Reject::StaleRevision);
            }
            if !d.validation.structurally_valid {
                return reject(Reject::InvalidInput);
            }
            let (_, c): (_, Cell) = load(tx, "cell", &input.cell, CELL)?;
            let current = selected_catalog(tx, meta, &c.configuration, &p, &input.device_plans)?;
            if current.view.catalog_digest != input.catalog_digest {
                return reject(Reject::StaleRevision);
            }
            if input.selections.len() > 128
                || input
                    .selections
                    .keys()
                    .any(|b| !d.validation.required_bindings.contains(b))
            {
                return reject(Reject::InvalidInput);
            }
            let record_key = key("draftbindings", &input.draft);
            let old = tx.get(&record_key)?;
            let revision = match (&old, input.expected) {
                (None, None) => Counter(1),
                (Some(r), Some(expected)) => {
                    check_revision(r.revision, expected)?;
                    r.revision.increment().map_err(domain_error)?
                }
                _ => return reject(Reject::StaleRevision),
            };
            let mut resolved = BTreeMap::new();
            let mut origins = BTreeMap::new();
            let mut device_sources = BTreeMap::new();
            let mut used = BTreeSet::new();
            for (binding, step) in &input.selections {
                let selected = current
                    .steps
                    .get(step)
                    .ok_or(StoreError::Rejected(Reject::CapabilityMissing))?;
                resolved.insert(
                    binding.clone(),
                    ActionBinding {
                        host: selected.host.clone(),
                        intent: selected.intent.normalized().map_err(domain_error)?,
                    },
                );
                origins.insert(
                    binding.clone(),
                    canonical::digest("RX-DRAFT-BINDING-STEP-v1", selected)
                        .map_err(domain_error)?,
                );
                if let Some(plan) = current.sources.get(step) {
                    used.insert(plan.id.clone());
                    device_sources.insert(
                        binding.clone(),
                        DeviceSource {
                            plan: plan.clone(),
                            binding: step.clone(),
                            step_digest: origins[binding],
                            action_digest:
                                rx_process_contract::compile_input::device_action_digest(
                                    &resolved[binding],
                                )
                                .map_err(StoreError::Invalid)?,
                        },
                    );
                }
            }
            if used
                != current
                    .view
                    .device_plans
                    .iter()
                    .map(|p| p.id.clone())
                    .collect()
            {
                return reject(Reject::InvalidInput);
            }
            let missing = d
                .validation
                .required_bindings
                .iter()
                .filter(|b| !resolved.contains_key(*b))
                .cloned()
                .collect::<Vec<_>>();
            let value = Version {
                device_plans: current.view.device_plans,
                required_cells: current.view.required_cells,
                device_sources,
                draft: input.draft.clone(),
                cell: input.cell,
                revision,
                source_revision: d.revision,
                source_digest: d.document_digest,
                catalog_digest: input.catalog_digest,
                selections: input.selections,
                origins,
                resolved,
                complete: missing.is_empty(),
                missing,
                updated_by: p.id,
                updated_at: now,
            };
            if canonical::bytes(&value).map_err(domain_error)?.len() > 262144 {
                return reject(Reject::InvalidInput);
            }
            tx.put(
                &record_key,
                old.map(|r| r.revision),
                &doc(BINDINGS, &value)?,
            )?;
            tx.put(
                &key("draftbindingsrevision", (&value.draft, revision)),
                None,
                &doc(BINDINGS, &value)?,
            )?;
            event(tx, "rx.event.draft-bindings-saved.v1", &value)?;
            remember(tx, &scope, fingerprint, BINDINGS, &value)?;
            Ok(value)
        })
    }
    pub fn draft_bindings(
        &mut self,
        identity: &Identity,
        cell: &Name,
        id: &Id,
        revision: Option<Counter>,
    ) -> Result<View> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            super::process_draft::read_access(tx, identity, meta, &clock.now(), cell)?;
            let d = draft(tx, cell, id)?;
            let (_, c): (_, Cell) = load(tx, "cell", cell, CELL)?;
            let mut current = catalog(&c.configuration)?;
            let binding = if let Some(revision) = revision {
                Some(load::<Version>(tx, "draftbindingsrevision", (id, revision), BINDINGS)?.1)
            } else {
                tx.get(&key("draftbindings", id))?
                    .map(|r| decode::<Version>(&r, BINDINGS))
                    .transpose()?
            };
            let mut stale = vec![];
            if let Some(b) = &binding {
                let actor = authorize_identity(tx, identity, meta, &clock.now())?;
                scope_access(&actor, &b.required_cells)?;
                if !b.device_plans.is_empty() {
                    match selected_catalog(tx, meta, &c.configuration, &actor, &b.device_plans) {
                        Ok(selected) => current = selected.view,
                        Err(StoreError::Rejected(
                            Reject::StaleRevision
                            | Reject::QualificationRequired
                            | Reject::NotFound,
                        )) => stale.push(StaleReason::DevicePlanChanged),
                        Err(e) => return Err(e),
                    }
                }
                if b.cell != *cell || b.draft != *id {
                    return Err(StoreError::Integrity(
                        "draft binding identity differs".into(),
                    ));
                }
                if b.source_digest != d.document_digest {
                    stale.push(StaleReason::SourceChanged);
                }
                if b.catalog_digest != current.catalog_digest {
                    stale.push(StaleReason::CatalogChanged);
                }
            }
            Ok(View {
                cell: cell.clone(),
                draft: id.clone(),
                current_source_revision: d.revision,
                current_catalog_digest: current.catalog_digest,
                binding,
                stale,
            })
        })
    }
    pub fn draft_compile_input(
        &mut self,
        identity: &Identity,
        cell: &Name,
        id: &Id,
        source_revision: Counter,
        binding_revision: Counter,
    ) -> Result<rx_process_contract::compile_input::CompileInput> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            super::process_draft::read_access(tx, identity, meta, &clock.now(), cell)?;
            let d = draft(tx, cell, id)?;
            check_revision(d.revision, source_revision)?;
            let (_, b): (_, Version) = load(tx, "draftbindings", id, BINDINGS)?;
            check_revision(b.revision, binding_revision)?;
            let (_, c): (_, Cell) = load(tx, "cell", cell, CELL)?;
            let actor = authorize_identity(tx, identity, meta, &clock.now())?;
            scope_access(&actor, &b.required_cells)?;
            let current = selected_catalog(tx, meta, &c.configuration, &actor, &b.device_plans)?;
            if b.cell != *cell
                || b.draft != *id
                || b.source_digest != d.document_digest
                || b.catalog_digest != current.view.catalog_digest
            {
                return reject(Reject::StaleRevision);
            }
            if !b.complete || !d.validation.structurally_valid {
                return reject(Reject::InvalidInput);
            }
            if b.required_cells != current.view.required_cells
                || b.device_plans != current.view.device_plans
                || b.selections.len() != b.resolved.len()
                || b.selections.len() != b.origins.len()
            {
                return Err(StoreError::Integrity(
                    "binding snapshot shape differs".into(),
                ));
            }
            let mut provenance = BTreeMap::new();
            for (alias, step) in &b.selections {
                let selected = current
                    .steps
                    .get(step)
                    .ok_or(StoreError::Rejected(Reject::StaleRevision))?;
                let action = ActionBinding {
                    host: selected.host.clone(),
                    intent: selected.intent.normalized().map_err(domain_error)?,
                };
                let origin = canonical::digest("RX-DRAFT-BINDING-STEP-v1", selected)
                    .map_err(domain_error)?;
                let stored = b.resolved.get(alias).ok_or(StoreError::Integrity(
                    "binding snapshot alias missing".into(),
                ))?;
                if b.origins.get(alias) != Some(&origin)
                    || canonical::bytes(stored).map_err(domain_error)?
                        != canonical::bytes(&action).map_err(domain_error)?
                {
                    return Err(StoreError::Integrity(
                        "binding snapshot source differs".into(),
                    ));
                }
                if let Some(plan) = current.sources.get(step) {
                    provenance.insert(
                        alias.clone(),
                        DeviceSource {
                            plan: plan.clone(),
                            binding: step.clone(),
                            step_digest: origin,
                            action_digest:
                                rx_process_contract::compile_input::device_action_digest(&action)
                                    .map_err(StoreError::Invalid)?,
                        },
                    );
                }
            }
            if canonical::bytes(&provenance).map_err(domain_error)?
                != canonical::bytes(&b.device_sources).map_err(domain_error)?
            {
                return Err(StoreError::Integrity("device provenance changed".into()));
            }
            let (_, source): (_, serde_json::Value) = load(
                tx,
                "processdraftdocument",
                (cell, d.document_digest),
                "rx.internal.process-draft-document.v1",
            )?;
            let input = rx_process_contract::compile_input::CompileInput {
                schema: name(if b.device_sources.is_empty() {
                    "rx.process-compile-input.v1"
                } else {
                    "rx.process-compile-input.v2"
                }),
                draft: id.clone(),
                cell: cell.clone(),
                source_revision: d.revision,
                binding_revision: b.revision,
                source_document_digest: d.document_digest,
                bindings_digest: rx_process_contract::compile_input::bindings_digest(
                    &b.resolved,
                    &b.device_sources,
                )
                .map_err(StoreError::Invalid)?,
                catalog_digest: current.view.catalog_digest,
                source,
                bindings: b.resolved,
                device_sources: b.device_sources,
            };
            input
                .validate()
                .map_err(|e| StoreError::Integrity(e.to_string()))?;
            Ok(input)
        })
    }
}
