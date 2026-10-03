use super::definition_catalog::{access, author};
use super::*;
use crate::workflow_publication::*;
use rx_domain::definition::Reference;
use rx_process_contract::execution_v2 as v2;

const PREVIEW: &str = "rx.execution-preview.v2";
const PUBLICATION: &str = "rx.workflow-publication.v2";
const BLOBS: crate::artifact_storage::BlobStore =
    crate::artifact_storage::BlobStore::new("executionv2", v2::MAX_DEFINITION_BYTES);

fn load_preview(tx: &mut dyn Transaction, reference: &Reference) -> Result<Preview> {
    let (revision, preview): (_, Preview) = load(tx, "executionpreview", &reference.id, PREVIEW)?;
    if revision != Counter(1)
        || preview.reference.revision != Counter(1)
        || preview.reference != *reference
        || preview.workflow.catalog != reference.catalog
        || preview.policy.schema_id.as_str() != v2::POLICY_SCHEMA
        || preview.reference.digest != preview.digest().map_err(StoreError::Integrity)?
    {
        return Err(StoreError::Integrity(
            "execution preview reference differs".into(),
        ));
    }
    Ok(preview)
}
fn saved(tx: &mut dyn Transaction, reference: &Reference) -> Result<SavedPreview> {
    let preview = load_preview(tx, reference)?;
    let policy =
        v2::Policy::decode(&BLOBS.read(tx, &preview.policy)?).map_err(StoreError::Integrity)?;
    if preview.inputs != policy.definition_closure
        || preview.index != policy.report_index
        || preview.workflow != policy.workflow
    {
        return Err(StoreError::Integrity(
            "execution preview artifact links differ".into(),
        ));
    }
    let inputs = v2::InputClosure::decode(&BLOBS.read(tx, &preview.inputs)?, &policy)
        .map_err(StoreError::Integrity)?;
    let index = v2::ReportIndex::decode(&BLOBS.read(tx, &preview.index)?, &policy)
        .map_err(StoreError::Integrity)?;
    Ok(SavedPreview {
        preview,
        policy,
        inputs,
        index,
    })
}
fn current(tx: &mut dyn Transaction, inputs: &v2::InputClosure, slots: u16) -> Result<()> {
    let snapshot = workflow_model::current_snapshot(tx, &inputs.requests, slots)?;
    if canonical::bytes(&snapshot.input_closure()).map_err(domain_error)?
        != canonical::bytes(inputs).map_err(domain_error)?
    {
        return Err(StoreError::Invalid(
            "STALE_EXECUTION_REFERENCE input closure changed".into(),
        ));
    }
    Ok(())
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// Immutable candidate for existing change/qualification review; never installs or activates it.
    pub fn prepare_workflow_configuration(
        &mut self,
        identity: &Identity,
        configuration: CellConfiguration,
    ) -> Result<ArtifactRef> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            authorize(
                tx,
                identity,
                meta,
                &clock.now(),
                Some(&configuration.id),
                Role::Engineer,
                false,
            )?;
            lifecycle::require_serving(tx)?;
            if configuration.execution.is_none() {
                return reject(Reject::InvalidInput);
            }
            validate_configuration(&configuration)?;
            process_change::store_config(tx, &configuration)
        })
    }
    pub fn prepare_execution_preview(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: PreviewInput,
    ) -> Result<Preparation> {
        input.validate().map_err(StoreError::Invalid)?;
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let p = author(tx, identity, meta, &clock.now(), true)?;
            lifecycle::require_serving(tx)?;
            access(
                tx,
                identity,
                &p,
                &input.candidates[0].request.workflow.catalog,
                true,
            )?;
            let (scope, fingerprint) = request(
                meta,
                &p,
                "WorkflowExecution.Preview",
                key_.as_str(),
                &(&p.id, &input),
            )?;
            if let Some(prior) = prior(tx, &scope, fingerprint, PREVIEW)? {
                return Ok(Preparation::Recorded(Box::new(prior)));
            }
            if tx.get(&key("executionpreview", &input.id))?.is_some() {
                return reject(Reject::StaleRevision);
            }
            let requests = input
                .candidates
                .iter()
                .map(|c| c.request.clone())
                .collect::<Vec<_>>();
            let snapshot = workflow_model::current_snapshot(tx, &requests, input.slots)?;
            Ok(Preparation::Pending(Box::new(PreviewWork {
                input: input.clone(),
                snapshot,
            })))
        })
    }
    pub fn save_execution_preview(
        &mut self,
        identity: &Identity,
        key_: &Id,
        prepared: PreparedPreview,
    ) -> Result<Preview> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let p = author(tx, identity, meta, &now, true)?;
            lifecycle::require_serving(tx)?;
            let catalog = &prepared.inputs.workflow.catalog;
            access(tx, identity, &p, catalog, true)?;
            let (scope, fingerprint) = request(
                meta,
                &p,
                "WorkflowExecution.Preview",
                key_.as_str(),
                &(&p.id, &prepared.input),
            )?;
            if let Some(prior) = prior(tx, &scope, fingerprint, PREVIEW)? {
                return Ok(prior);
            }
            if tx
                .get(&key("executionpreview", &prepared.input.id))?
                .is_some()
            {
                return reject(Reject::StaleRevision);
            }
            current(tx, &prepared.inputs, prepared.input.slots)?;
            tx.require_workflow_execution_reader()?;
            let policy_bytes = canonical::bytes(&prepared.policy).map_err(domain_error)?;
            let inputs_bytes = canonical::bytes(&prepared.inputs).map_err(domain_error)?;
            let policy = artifact(v2::POLICY_SCHEMA, &policy_bytes);
            for (reference, bytes) in [
                (&policy, policy_bytes.as_slice()),
                (&prepared.policy.definition_closure, inputs_bytes.as_slice()),
                (&prepared.policy.report_index, prepared.index.as_slice()),
            ] {
                BLOBS.put(tx, reference.sha256, bytes)?;
            }
            let mut preview = Preview {
                reference: Reference {
                    catalog: catalog.clone(),
                    id: prepared.input.id.clone(),
                    revision: Counter(1),
                    digest: Digest::from_bytes([0; 32]),
                },
                workflow: prepared.inputs.workflow.clone(),
                policy,
                inputs: prepared.policy.definition_closure.clone(),
                index: prepared.policy.report_index.clone(),
                created_by: p.id,
                created_at: now,
            };
            preview.reference.digest = preview.digest().map_err(StoreError::Invalid)?;
            save(
                tx,
                "executionpreview",
                &preview.reference.id,
                None,
                PREVIEW,
                &preview,
            )?;
            remember(tx, &scope, fingerprint, PREVIEW, &preview)?;
            event(tx, "rx.workflow-execution.previewed.v2", &preview)?;
            Ok(preview)
        })
    }
    pub fn execution_preview(
        &mut self,
        identity: &Identity,
        reference: &Reference,
    ) -> Result<SavedPreview> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let p = author(tx, identity, meta, &clock.now(), false)?;
            access(tx, identity, &p, &reference.catalog, false)?;
            saved(tx, reference)
        })
    }
    pub fn prepare_workflow_publication(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: Publish,
    ) -> Result<PublishPreparation> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let p = author(tx, identity, meta, &now, true)?;
            lifecycle::require_serving(tx)?;
            access(tx, identity, &p, &input.preview.catalog, true)?;
            if !p.cells.contains(&input.cell) {
                return reject(Reject::Forbidden);
            }
            let (scope, fingerprint) = request(
                meta,
                &p,
                "WorkflowExecution.Publish",
                key_.as_str(),
                &(&p.id, &input),
            )?;
            if let Some(value) = prior(tx, &scope, fingerprint, PUBLICATION)? {
                return Ok(PublishPreparation::Recorded(Box::new(value)));
            }
            if tx.get(&key("workflowpublication", &input.id))?.is_some() {
                return reject(Reject::StaleRevision);
            }
            let saved = saved(tx, &input.preview)?;
            if input.bindings.keys().ne(saved.policy.templates.keys()) {
                return reject(Reject::InvalidInput);
            }
            current(tx, &saved.inputs, saved.policy.slot_order.len() as u16)?;
            let (_, cell): (_, Cell) = load(tx, "cell", &input.cell, CELL)?;
            if cell.configuration.environment != Environment::Simulation {
                return reject(Reject::InvalidInput);
            }
            let configuration_digest = package_intake::configuration_digest(&cell)?;
            let registration = package_intake::current(tx, meta)?.ok_or(
                StoreError::Unavailable("package intake not configured".into()),
            )?;
            let mut receipts = BTreeMap::new();
            for binding in input.bindings.values() {
                if receipts.contains_key(&binding.intake) {
                    continue;
                }
                let (revision, receipt): (_, crate::package_intake::Receipt) = load(
                    tx,
                    "packageintakereceipt",
                    &binding.intake,
                    "rx.package-intake-receipt.v1",
                )?;
                if revision != Counter(1)
                    || receipt.id != binding.intake
                    || receipt.cell != input.cell
                    || receipt.registration != registration
                    || receipt.configuration_digest != configuration_digest
                {
                    return reject(Reject::StaleRevision);
                }
                receipts.insert(binding.intake.clone(), receipt);
            }
            Ok(PublishPreparation::Verify(Box::new(PublishTicket {
                input: input.clone(),
                identity: identity.clone(),
                key: key_.clone(),
                preview: saved.preview,
                policy: saved.policy,
                registration,
                receipts,
                configuration_digest,
                installation: meta.id.clone(),
                boot: meta.runtime_boot.clone(),
                issued: now,
                definition_count: saved.inputs.definitions.len(),
            })))
        })
    }
    pub fn commit_workflow_publication(
        &mut self,
        prepared: PreparedPublication,
    ) -> Result<Publication> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let t = &prepared.ticket;
            let input = &t.input;
            let now = clock.now();
            let p = author(tx, &t.identity, meta, &now, true)?;
            lifecycle::require_serving(tx)?;
            access(tx, &t.identity, &p, &input.preview.catalog, true)?;
            if !p.cells.contains(&input.cell) {
                return reject(Reject::Forbidden);
            }
            let (scope, fingerprint) = request(
                meta,
                &p,
                "WorkflowExecution.Publish",
                t.key.as_str(),
                &(&p.id, input),
            )?;
            if let Some(value) = prior(tx, &scope, fingerprint, PUBLICATION)? {
                return Ok(value);
            }
            if tx.get(&key("workflowpublication", &input.id))?.is_some() {
                return reject(Reject::StaleRevision);
            }
            if t.boot != meta.runtime_boot
                || now.age_ns(&t.issued).is_none_or(|age| age >= TICKET_TTL_NS)
                || package_intake::current(tx, meta)?.as_ref() != Some(&t.registration)
            {
                return reject(Reject::StaleRevision);
            }
            let (_, cell): (_, Cell) = load(tx, "cell", &input.cell, CELL)?;
            if package_intake::configuration_digest(&cell)? != t.configuration_digest {
                return reject(Reject::StaleRevision);
            }
            let saved = saved(tx, &input.preview)?;
            if saved.preview.reference != t.preview.reference
                || saved.preview.policy != t.preview.policy
            {
                return reject(Reject::StaleRevision);
            }
            current(tx, &saved.inputs, saved.policy.slot_order.len() as u16)?;
            tx.require_workflow_execution_reader()?;
            let mut publication = Publication {
                reference: Reference {
                    catalog: input.preview.catalog.clone(),
                    id: input.id.clone(),
                    revision: Counter(1),
                    digest: Digest::from_bytes([0; 32]),
                },
                preview: input.preview.clone(),
                policy: saved.preview.policy,
                cell: input.cell.clone(),
                bindings: input.bindings.clone(),
                packages: prepared.packages,
                registration: t.registration.clone(),
                published_by: p.id,
                published_at: now,
            };
            publication.reference.digest = publication.digest().map_err(StoreError::Invalid)?;
            save(
                tx,
                "workflowpublication",
                &input.id,
                None,
                PUBLICATION,
                &publication,
            )?;
            remember(tx, &scope, fingerprint, PUBLICATION, &publication)?;
            event(tx, "rx.workflow-execution.published.v2", &publication)?;
            Ok(publication)
        })
    }
    pub fn workflow_publication(
        &mut self,
        identity: &Identity,
        reference: &Reference,
    ) -> Result<Publication> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let p = author(tx, identity, meta, &clock.now(), false)?;
            access(tx, identity, &p, &reference.catalog, false)?;
            let (revision, value): (_, Publication) =
                load(tx, "workflowpublication", &reference.id, PUBLICATION)?;
            if revision != Counter(1)
                || value.reference.revision != Counter(1)
                || value.reference != *reference
                || value.preview.catalog != reference.catalog
                || value.reference.digest != value.digest().map_err(StoreError::Integrity)?
            {
                return Err(StoreError::Integrity(
                    "workflow publication reference differs".into(),
                ));
            }
            let preview = load_preview(tx, &value.preview)?;
            if value.policy != preview.policy {
                return Err(StoreError::Integrity(
                    "publication policy differs from preview".into(),
                ));
            }
            Ok(value)
        })
    }
}
