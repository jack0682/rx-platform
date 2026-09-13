use super::*;
use crate::investigation as inv;
use rx_domain::operation::{Disposition, Knowledge, Phase};
mod context;
mod reads;
const POLICY: &str = "rx.internal.investigation-policy.v1";
const ATTEST: &str = "investigation-attestation";
const DISPOSITION: &str = "investigation-disposition";
const REF: &str = "rx.investigation-ref.v1";
#[derive(serde::Serialize, serde::Deserialize)]
struct Registration {
    generation: inv::PolicyGeneration,
    policy: inv::Policy,
}
fn policy(tx: &mut dyn Transaction, meta: &Installation) -> Result<Registration> {
    let row = tx
        .get(&name("investigation/policy"))?
        .ok_or(StoreError::Rejected(Reject::CapabilityMissing))?;
    let r: Registration = decode(&row, POLICY)?;
    if r.generation.runtime_boot != meta.runtime_boot
        || r.policy.digest().map_err(StoreError::Integrity)? != r.generation.digest
    {
        return reject(Reject::CapabilityMissing);
    }
    Ok(r)
}
fn eligible(work: &Work) -> bool {
    work.operation.integrity() == Integrity::Valid
        && work.operation.disposition() == Disposition::Quarantined
        && work.operation.knowledge() == Knowledge::Unknown
        && matches!(
            (work.operation.phase(), work.operation.outcome()),
            (Phase::Reconciling, Outcome::None) | (Phase::Settled, Outcome::Unresolved)
        )
}
fn access(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    identity: &Identity,
    operation: &Id,
) -> Result<(Principal, Counter, Work, crate::process_change::Impact)> {
    let actor = authorize(tx, identity, meta, now, None, Role::RecoveryLead, true)?;
    if !crate::procedure::can_report(&actor.roles) {
        return reject(Reject::Forbidden);
    }
    let (revision, work): (_, Work) = load(tx, "work", operation, WORK)?;
    if work.operation.id() != operation || !actor.cells.contains(&work.cell) {
        return reject(Reject::Forbidden);
    }
    let impact = process_change::prospective_impact(
        tx,
        &work.cell,
        &BTreeSet::from([work.host.clone()]),
        &work.intent.resource_set.iter().cloned().collect(),
    )?;
    if impact.cells.iter().any(|c| !actor.cells.contains(&c.id)) {
        return reject(Reject::Forbidden);
    }
    Ok((actor, revision, work, impact))
}
fn old_scope(actor: &Principal, c: &inv::Context) -> Result<()> {
    if c.cells.keys().any(|cell| !actor.cells.contains(cell)) {
        return reject(Reject::Forbidden);
    }
    Ok(())
}
fn same_author(actor: &Principal, stored: &inv::Actor) -> Result<()> {
    if actor.id != stored.principal {
        return reject(Reject::Forbidden);
    }
    Ok(())
}
fn attestation(tx: &mut dyn Transaction, id: &Id) -> Result<inv::Attestation> {
    let (revision, a): (_, inv::Attestation) = load(tx, ATTEST, id, inv::ATTESTATION_SCHEMA)?;
    if revision != Counter(1)
        || a.id != *id
        || a.request.id != *id
        || a.schema.as_str() != inv::ATTESTATION_SCHEMA
        || a.request.context_digest != a.context.digest().map_err(StoreError::Integrity)?
        || a.request.operation != *a.context.work.operation.id()
        || a.request.procedure_digest != a.procedure_reference.sha256
        || a.procedure.reference().map_err(StoreError::Integrity)? != a.procedure_reference
        || a.operation_authorized
        || a.resource_release_authorized
    {
        return Err(StoreError::Integrity(
            "investigation attestation differs".into(),
        ));
    }
    Ok(a)
}
fn receipt(tx: &mut dyn Transaction, id: &Id) -> Result<inv::Receipt> {
    let (revision, r): (_, inv::Receipt) = load(tx, DISPOSITION, id, inv::DISPOSITION_SCHEMA)?;
    if revision != Counter(1)
        || r.id != *id
        || r.schema.as_str() != inv::DISPOSITION_SCHEMA
        || r.before.operation.id() != &r.request.operation
        || r.after.operation.id() != &r.request.operation
        || r.after.operation.outcome() != Outcome::Unresolved
        || r.after.operation.disposition() != Disposition::Quarantined
        || r.attestation.reference().map_err(StoreError::Integrity)? != r.attestation_reference
        || r.operation_authorized
        || r.resource_release_authorized
        || r.resources_released
        || r.actor.principal != r.attestation.actor.principal
        || canonical::bytes(&r.before).map_err(domain_error)?
            != canonical::bytes(&r.attestation.context.work).map_err(domain_error)?
        || r.before.operation.outcome() != Outcome::None
    {
        return Err(StoreError::Integrity(
            "investigation disposition differs".into(),
        ));
    }
    let original = attestation(tx, &r.attestation.id)?;
    if original.reference().map_err(StoreError::Integrity)? != r.attestation_reference {
        return Err(StoreError::Integrity(
            "original investigation attestation differs".into(),
        ));
    }
    let mut expected = r.before.clone();
    let mut evidence = r.request.evidence_ids.clone();
    evidence.push(r.id.clone());
    expected
        .operation
        .conclude(rx_domain::operation::Conclusion {
            outcome: Outcome::Unresolved,
            evidence_ids: evidence,
        })
        .map_err(domain_error)?;
    if canonical::bytes(&expected).map_err(domain_error)?
        != canonical::bytes(&r.after).map_err(domain_error)?
    {
        return Err(StoreError::Integrity(
            "disposition changed fields outside its conclusion".into(),
        ));
    }
    Ok(r)
}
fn attest_fingerprint(input: &inv::AttestSubmit) -> impl Serialize + '_ {
    (
        &input.id,
        &input.operation,
        input.context_digest,
        input.procedure_digest,
        input.evidence_ids.iter().collect::<BTreeSet<_>>(),
        input.assertion,
        &input.note,
        &input.occurred_at,
    )
}
fn disposition_fingerprint(input: &inv::RecordDisposition) -> impl Serialize + '_ {
    (
        &input.operation,
        input.evidence_ids.iter().collect::<BTreeSet<_>>(),
        input.procedure_digest,
        input.disposition,
        input.reason,
    )
}
fn validate_submit(input: &inv::AttestSubmit, c: &inv::Context) -> Result<()> {
    let occurred = time::OffsetDateTime::parse(
        &input.occurred_at,
        &time::format_description::well_known::Rfc3339,
    )
    .map_err(|_| StoreError::Rejected(Reject::InvalidInput))?;
    if input.note.trim().is_empty()
        || input.note.len() > 4096
        || input.occurred_at.len() > 128
        || !occurred.offset().is_utc()
        || input.occurred_at.ends_with("-00:00")
        || input.operation != *c.work.operation.id()
        || input.expected_operation_revision != c.work.operation.revision()
        || input.context_digest != c.digest().map_err(StoreError::Integrity)?
        || input.evidence_ids.len() > inv::MAX_EVIDENCE
        || input.evidence_ids.iter().collect::<BTreeSet<_>>().len() != input.evidence_ids.len()
        || input.evidence_ids.iter().collect::<BTreeSet<_>>() != c.evidence.keys().collect()
        || !c
            .procedures
            .iter()
            .any(|p| p.sha256 == input.procedure_digest)
    {
        return reject(Reject::StaleRevision);
    }
    Ok(())
}
fn current(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    identity: &Identity,
    expected: &inv::Context,
) -> Result<()> {
    let (actor, revision, work, impact) =
        access(tx, meta, now, identity, expected.work.operation.id())?;
    old_scope(&actor, expected)?;
    let actual = context::build(tx, meta, revision, work, impact)?;
    if actual.digest().map_err(StoreError::Integrity)?
        != expected.digest().map_err(StoreError::Integrity)?
    {
        return reject(Reject::StaleRevision);
    }
    Ok(())
}
fn selected_attestation(
    tx: &mut dyn Transaction,
    actor: &Principal,
    input: &inv::RecordDisposition,
    c: &inv::Context,
) -> Result<inv::Attestation> {
    if input.evidence_ids.is_empty()
        || input.evidence_ids.len() > inv::MAX_EVIDENCE + 1
        || input.evidence_ids.iter().collect::<BTreeSet<_>>().len() != input.evidence_ids.len()
    {
        return reject(Reject::InvalidInput);
    }
    let mut selected = None;
    for id in &input.evidence_ids {
        if tx.get(&key(ATTEST, id))?.is_some() {
            if selected.is_some() {
                return reject(Reject::InvalidInput);
            }
            selected = Some(attestation(tx, id)?);
        }
    }
    let a = selected.ok_or(StoreError::Rejected(Reject::InvalidInput))?;
    let expected_ids: BTreeSet<_> = a
        .request
        .evidence_ids
        .iter()
        .chain(std::iter::once(&a.id))
        .collect();
    if a.actor.principal != actor.id
        || a.context.digest().map_err(StoreError::Integrity)?
            != c.digest().map_err(StoreError::Integrity)?
        || a.request.operation != input.operation
        || a.request.procedure_digest != input.procedure_digest
        || expected_ids != input.evidence_ids.iter().collect()
    {
        return reject(Reject::ContinuityUnproven);
    }
    Ok(a)
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// Internal pinned-policy registration. A policy is not accepted from an investigation body.
    pub fn register_investigation_policy(
        &mut self,
        policy_: inv::Policy,
        file_digest: Digest,
    ) -> Result<inv::PolicyGeneration> {
        let digest = policy_.digest().map_err(StoreError::Invalid)?;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let old = tx.get(&name("investigation/policy"))?;
            if let Some(row) = &old {
                let prior: Registration = decode(row, POLICY)?;
                if prior.generation.runtime_boot == meta.runtime_boot
                    && prior.generation.digest == digest
                    && prior.generation.file_digest == file_digest
                {
                    return Ok(prior.generation);
                }
            }
            let generation = inv::PolicyGeneration {
                id: id(),
                runtime_boot: meta.runtime_boot.clone(),
                digest,
                file_digest,
            };
            let r = Registration {
                generation: generation.clone(),
                policy: policy_,
            };
            save(
                tx,
                "investigation-policy-history",
                &generation.id,
                None,
                POLICY,
                &r,
            )?;
            tx.put(
                &name("investigation/policy"),
                old.map(|r| r.revision),
                &doc(POLICY, &r)?,
            )?;
            Ok(generation)
        })
    }
    pub fn investigation_context(
        &mut self,
        identity: &Identity,
        operation: &Id,
    ) -> Result<inv::Context> {
        let now = self.clock.now();
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let (_, revision, work, impact) = access(tx, meta, &now, identity, operation)?;
            context::build(tx, meta, revision, work, impact)
        })
    }
    pub fn prepare_investigation_attestation(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: inv::AttestSubmit,
    ) -> Result<inv::Preflight> {
        let now = self.clock.now();
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let (actor, revision, work, impact) =
                access(tx, meta, &now, identity, &input.operation)?;
            let (scope, fp) = request(
                meta,
                &actor,
                "Investigation.Attest",
                key_.as_str(),
                &attest_fingerprint(&input),
            )?;
            if let Some(id) = prior::<Id>(tx, &scope, fp, REF)? {
                let a = attestation(tx, &id)?;
                old_scope(&actor, &a.context)?;
                same_author(&actor, &a.actor)?;
                return Ok(inv::Preflight::Recorded(Box::new(a)));
            }
            if tx.get(&key(ATTEST, &input.id))?.is_some() {
                return Err(StoreError::KeyConflict);
            }
            let context = context::build(tx, meta, revision, work, impact)?;
            validate_submit(&input, &context)?;
            Ok(inv::Preflight::Verify(Box::new(inv::Ticket {
                identity: identity.clone(),
                key: key_.clone(),
                request: input,
                context,
            })))
        })
    }
    pub fn commit_investigation_attestation(
        &mut self,
        prepared: inv::Prepared,
    ) -> Result<inv::Attestation> {
        let now = self.clock.now();
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let inv::Prepared {
                ticket: t,
                procedure,
                reference,
                signature,
            } = prepared;
            let (actor, _, _, _) = access(tx, meta, &now, &t.identity, &t.request.operation)?;
            let (scope, fp) = request(
                meta,
                &actor,
                "Investigation.Attest",
                t.key.as_str(),
                &attest_fingerprint(&t.request),
            )?;
            if let Some(id) = prior::<Id>(tx, &scope, fp, REF)? {
                let a = attestation(tx, &id)?;
                old_scope(&actor, &a.context)?;
                same_author(&actor, &a.actor)?;
                return Ok(a);
            }
            current(tx, meta, &now, &t.identity, &t.context)?;
            validate_submit(&t.request, &t.context)?;
            if tx.get(&key(ATTEST, &t.request.id))?.is_some() {
                return Err(StoreError::KeyConflict);
            }
            let a = inv::Attestation {
                schema: name(inv::ATTESTATION_SCHEMA),
                id: t.request.id.clone(),
                request: t.request,
                actor: inv::Actor::from(&t.identity),
                recorded_at: now,
                context: t.context,
                procedure,
                procedure_reference: reference,
                signature,
                operation_authorized: false,
                resource_release_authorized: false,
            };
            save(tx, ATTEST, &a.id, None, inv::ATTESTATION_SCHEMA, &a)?;
            event(tx, "rx.event.investigation-attested.v1", &a)?;
            remember(tx, &scope, fp, REF, &a.id)?;
            Ok(a)
        })
    }
    pub fn prepare_investigation_disposition(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: inv::RecordDisposition,
    ) -> Result<inv::DispositionPreflight> {
        let now = self.clock.now();
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let (actor, revision, work, impact) =
                access(tx, meta, &now, identity, &input.operation)?;
            let (scope, fp) = request(
                meta,
                &actor,
                "Recovery.RecordDisposition",
                key_.as_str(),
                &disposition_fingerprint(&input),
            )?;
            if let Some(id) = prior::<Id>(tx, &scope, fp, REF)? {
                let r = receipt(tx, &id)?;
                old_scope(&actor, &r.attestation.context)?;
                same_author(&actor, &r.actor)?;
                return Ok(inv::DispositionPreflight::Recorded(Box::new(r)));
            }
            if tx
                .get(&key("investigation-disposition-slot", &input.operation))?
                .is_some()
            {
                return Err(StoreError::KeyConflict);
            }
            if work.operation.outcome() != Outcome::None
                || work.operation.revision() != input.expected_revision
            {
                return reject(Reject::StaleRevision);
            }
            let context = context::build(tx, meta, revision, work, impact)?;
            let a = selected_attestation(tx, &actor, &input, &context)?;
            Ok(inv::DispositionPreflight::Verify(Box::new(
                inv::DispositionTicket {
                    identity: identity.clone(),
                    key: key_.clone(),
                    request: input,
                    attestation: a,
                    context,
                },
            )))
        })
    }
    pub fn commit_investigation_disposition(
        &mut self,
        prepared: inv::PreparedDisposition,
    ) -> Result<inv::Receipt> {
        let now = self.clock.now();
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let t = prepared.ticket;
            let (actor, _, _, _) = access(tx, meta, &now, &t.identity, &t.request.operation)?;
            let (scope, fp) = request(
                meta,
                &actor,
                "Recovery.RecordDisposition",
                t.key.as_str(),
                &disposition_fingerprint(&t.request),
            )?;
            if let Some(id) = prior::<Id>(tx, &scope, fp, REF)? {
                let r = receipt(tx, &id)?;
                old_scope(&actor, &r.attestation.context)?;
                same_author(&actor, &r.actor)?;
                return Ok(r);
            }
            if tx
                .get(&key("investigation-disposition-slot", &t.request.operation))?
                .is_some()
            {
                return Err(StoreError::KeyConflict);
            }
            current(tx, meta, &now, &t.identity, &t.context)?;
            let a = selected_attestation(tx, &actor, &t.request, &t.context)?;
            if a.reference().map_err(StoreError::Integrity)?
                != t.attestation.reference().map_err(StoreError::Integrity)?
                || t.context.work.operation.outcome() != Outcome::None
            {
                return reject(Reject::StaleRevision);
            }
            let id = id();
            let before = t.context.work;
            let mut after = before.clone();
            let mut evidence = t.request.evidence_ids.clone();
            evidence.push(id.clone());
            after
                .operation
                .conclude(rx_domain::operation::Conclusion {
                    outcome: Outcome::Unresolved,
                    evidence_ids: evidence,
                })
                .map_err(domain_error)?;
            let r = inv::Receipt {
                schema: name(inv::DISPOSITION_SCHEMA),
                id: id.clone(),
                request: t.request,
                actor: inv::Actor::from(&t.identity),
                recorded_at: now,
                attestation_reference: a.reference().map_err(StoreError::Integrity)?,
                attestation: a,
                before,
                after: after.clone(),
                operation_authorized: false,
                resource_release_authorized: false,
                resources_released: false,
            };
            save(
                tx,
                "work",
                after.operation.id(),
                Some(t.context.work_revision),
                WORK,
                &after,
            )?;
            save(tx, DISPOSITION, &id, None, inv::DISPOSITION_SCHEMA, &r)?;
            save(
                tx,
                "investigation-disposition-slot",
                &r.request.operation,
                None,
                REF,
                &id,
            )?;
            event(tx, "rx.event.investigation-disposition.v1", &r)?;
            remember(tx, &scope, fp, REF, &id)?;
            Ok(r)
        })
    }
}
