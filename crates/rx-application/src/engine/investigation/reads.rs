use super::*;
fn current_view(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    identity: &Identity,
    c: &inv::Context,
) -> Result<bool> {
    match current(tx, meta, now, identity, c) {
        Ok(()) => Ok(true),
        Err(StoreError::Rejected(_) | StoreError::Invalid(_)) => Ok(false),
        Err(e) => Err(e),
    }
}
fn attestation_view(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    identity: &Identity,
    a: inv::Attestation,
) -> Result<inv::AttestationView> {
    Ok(inv::AttestationView {
        current: current_view(tx, meta, now, identity, &a.context)?,
        attestation: a,
        operation_authorized: false,
        resource_release_authorized: false,
    })
}
fn receipt_view(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    identity: &Identity,
    r: inv::Receipt,
) -> Result<inv::ReceiptView> {
    let mut expected = r.attestation.context.clone();
    expected.work = r.after.clone();
    expected.work_revision = expected.work_revision.increment().map_err(domain_error)?;
    Ok(inv::ReceiptView {
        current: current_view(tx, meta, now, identity, &expected)?,
        receipt: r,
        operation_authorized: false,
        resource_release_authorized: false,
    })
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn investigation_attestation(
        &mut self,
        identity: &Identity,
        operation: &Id,
        id: &Id,
    ) -> Result<inv::AttestationView> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let (actor, _, _, _) = access(tx, meta, &now, identity, operation)?;
            let a = attestation(tx, id)?;
            if a.request.operation != *operation || a.context.installation != meta.id {
                return reject(Reject::Forbidden);
            }
            old_scope(&actor, &a.context)?;
            attestation_view(tx, meta, &now, identity, a)
        })
    }
    pub fn investigation_disposition(
        &mut self,
        identity: &Identity,
        operation: &Id,
        id: &Id,
    ) -> Result<inv::ReceiptView> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let (actor, _, _, _) = access(tx, meta, &now, identity, operation)?;
            let r = receipt(tx, id)?;
            if r.request.operation != *operation || r.attestation.context.installation != meta.id {
                return reject(Reject::Forbidden);
            }
            old_scope(&actor, &r.attestation.context)?;
            receipt_view(tx, meta, &now, identity, r)
        })
    }
    pub fn list_investigation_attestations(
        &mut self,
        identity: &Identity,
        operation: &Id,
        after: Option<&Id>,
        limit: usize,
    ) -> Result<inv::AttestationPage> {
        if limit == 0 || limit > 50 {
            return reject(Reject::InvalidInput);
        }
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let (actor, _, _, _) = access(tx, meta, &now, identity, operation)?;
            let rows = tx.scan(&format!("{ATTEST}/"))?;
            if rows.len() > inv::MAX_SCAN {
                return Err(StoreError::Invalid(
                    "INVESTIGATION_ATTESTATION_SCAN_LIMIT".into(),
                ));
            }
            let mut values = Vec::new();
            for row in rows {
                let a: inv::Attestation = decode(&row, inv::ATTESTATION_SCHEMA)?;
                if a.request.operation == *operation
                    && after.is_none_or(|id| a.id > *id)
                    && a.context.cells.keys().all(|c| actor.cells.contains(c))
                {
                    values.push(attestation(tx, &a.id)?);
                }
            }
            values.sort_by(|a, b| a.id.cmp(&b.id));
            let more = values.len() > limit;
            values.truncate(limit);
            let next = more.then(|| values.last().unwrap().id.clone());
            let mut items = Vec::new();
            for a in values {
                items.push(attestation_view(tx, meta, &now, identity, a)?);
            }
            Ok(inv::AttestationPage { items, next })
        })
    }
    pub fn list_investigation_dispositions(
        &mut self,
        identity: &Identity,
        operation: &Id,
        after: Option<&Id>,
        limit: usize,
    ) -> Result<inv::ReceiptPage> {
        if limit == 0 || limit > 50 {
            return reject(Reject::InvalidInput);
        }
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let (actor, _, _, _) = access(tx, meta, &now, identity, operation)?;
            let rows = tx.scan(&format!("{DISPOSITION}/"))?;
            if rows.len() > inv::MAX_SCAN {
                return Err(StoreError::Invalid(
                    "INVESTIGATION_DISPOSITION_SCAN_LIMIT".into(),
                ));
            }
            let mut values = Vec::new();
            for row in rows {
                let r: inv::Receipt = decode(&row, inv::DISPOSITION_SCHEMA)?;
                if r.request.operation == *operation
                    && after.is_none_or(|id| r.id > *id)
                    && r.attestation
                        .context
                        .cells
                        .keys()
                        .all(|c| actor.cells.contains(c))
                {
                    values.push(receipt(tx, &r.id)?);
                }
            }
            values.sort_by(|a, b| a.id.cmp(&b.id));
            let more = values.len() > limit;
            values.truncate(limit);
            let next = more.then(|| values.last().unwrap().id.clone());
            let mut items = Vec::new();
            for r in values {
                items.push(receipt_view(tx, meta, &now, identity, r)?);
            }
            Ok(inv::ReceiptPage { items, next })
        })
    }
}
