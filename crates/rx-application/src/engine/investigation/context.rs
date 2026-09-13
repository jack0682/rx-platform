use super::*;
fn bounded(tx: &mut dyn Transaction, prefix: &str) -> Result<Vec<rx_ports::Record>> {
    let records = tx.scan(prefix)?;
    if records.len() > inv::MAX_SCAN {
        return Err(StoreError::Invalid(
            "INVESTIGATION_SCAN_LIMIT: complete original evidence coverage required".into(),
        ));
    }
    Ok(records)
}
fn insert(
    evidence: &mut BTreeMap<Id, inv::Evidence>,
    id: Id,
    kind: inv::EvidenceKind,
    row: &rx_ports::Record,
) -> Result<()> {
    let reference = inv::reference(row.document.schema.as_str(), &row.document.value)
        .map_err(StoreError::Integrity)?;
    let next = inv::Evidence {
        id: id.clone(),
        kind,
        reference,
        document: row.document.clone(),
    };
    if let Some(old) = evidence.insert(id, next.clone())
        && old.document != next.document
    {
        return Err(StoreError::Integrity(
            "investigation evidence ID conflict".into(),
        ));
    }
    if evidence.len() > inv::MAX_EVIDENCE {
        return Err(StoreError::Invalid("INVESTIGATION_EVIDENCE_LIMIT".into()));
    }
    Ok(())
}
pub(super) fn build(
    tx: &mut dyn Transaction,
    meta: &Installation,
    revision: Counter,
    work: Work,
    impact: crate::process_change::Impact,
) -> Result<inv::Context> {
    if !eligible(&work) {
        return reject(Reject::ContinuityUnproven);
    }
    let registration = policy(tx, meta)?;
    let (_, permit): (_, Permit) = load(tx, "permit", &work.permit, PERMIT)?;
    if permit.id != work.permit
        || permit.operation != *work.operation.id()
        || permit.host != work.host
        || permit.cell != work.cell
        || permit.intent_digest != work.intent.digest().map_err(domain_error)?
    {
        return Err(StoreError::Integrity(
            "original investigation permit differs".into(),
        ));
    }
    let mut cells = BTreeMap::new();
    for target in impact.cells {
        let (revision, cell): (_, Cell) = load(tx, "cell", &target.id, CELL)?;
        cells.insert(
            target.id,
            inv::CellCut {
                revision,
                epoch: cell.epoch,
                scopes: cell.scope_epochs,
                configuration_digest: crate::runtime_invalidation::configuration_digest(
                    &cell.configuration,
                )
                .map_err(StoreError::Integrity)?,
                definition: cell.configuration.definition.sha256,
                environment: name(match cell.configuration.environment {
                    Environment::Simulation => "SIMULATION",
                    Environment::Physical => "PHYSICAL",
                }),
                blocks: cell.blocks,
            },
        );
    }
    let mut resources = BTreeMap::new();
    for id in &work.intent.resource_set {
        let (_, r): (_, Resource) = load(tx, "resource", id, RESOURCE)?;
        if r.id != *id {
            return Err(StoreError::Integrity(
                "investigation resource identity differs".into(),
            ));
        }
        resources.insert(id.clone(), r);
    }
    let mut evidence = BTreeMap::new();
    for row in bounded(tx, "evidence/")? {
        let e: NativeEvidence = decode(&row, "rx.internal.native-evidence.v1")?;
        if e.operation != *work.operation.id() {
            continue;
        }
        let (_, owner): (_, Name) =
            load(tx, "evidenceowner", &e.id, "rx.internal.evidence-owner.v1")?;
        if row.key != key("evidence", &e.id)
            || owner != work.host
            || e.profile_digest != work.intent.profile_digest
            || work.invocation.as_ref() != Some(&e.invocation)
        {
            return Err(StoreError::Integrity(
                "original native evidence owner/correlation differs".into(),
            ));
        }
        insert(&mut evidence, e.id, inv::EvidenceKind::Native, &row)?;
    }
    for row in bounded(tx, "hostreceipt/")? {
        let receipt: HostReceipt = decode(&row, "rx.internal.host-receipt.v1")?;
        if receipt.operation != *work.operation.id() {
            continue;
        }
        if row.key
            != key(
                "hostreceipt",
                (&work.host, &receipt.journal, receipt.sequence),
            )
            || receipt.journal != work.host_journal
            || receipt.digest != work.intent.digest().map_err(domain_error)?
            || receipt.sequence.0 == 0
            || (receipt.invocation.is_none() && receipt.state != ReceiptState::VoidedBeforeSend)
            || receipt
                .invocation
                .as_ref()
                .is_some_and(|i| work.invocation.as_ref() != Some(i))
        {
            return Err(StoreError::Integrity(
                "original receipt owner/correlation differs".into(),
            ));
        }
        let id = super::super::delivery::key_id(&receipt.journal, &receipt.sequence.0.to_string());
        let original = tx
            .get(&key("receiptevidence", &id))?
            .ok_or_else(|| StoreError::Integrity("receipt evidence missing".into()))?;
        if original.document != row.document {
            return Err(StoreError::Integrity(
                "receipt evidence original differs".into(),
            ));
        }
        insert(&mut evidence, id, inv::EvidenceKind::Receipt, &row)?;
    }
    Ok(inv::Context {
        installation: meta.id.clone(),
        store_generation: meta.store_generation.clone(),
        runtime_boot: meta.runtime_boot.clone(),
        work_revision: revision,
        work,
        permit,
        resources,
        cells,
        evidence,
        policy: registration.generation,
        procedures: registration.policy.procedures,
        operation_authorized: false,
        resource_release_authorized: false,
    })
}
