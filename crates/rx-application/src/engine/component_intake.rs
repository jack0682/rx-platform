use super::*;
use crate::component_intake::{Context, Receipt};
use crate::{component_intake::*, resident_component as component};
use rx_domain::{
    component::{Registration, RegistrationState},
    component_transfer::*,
};
use rx_ports::{Record, StoredEvent};
use serde::Deserialize;
const SERVICE: &str = "rx.internal.component-intake-service.v1";
const STAGE: &str = "rx.component-intake-stage.v1";
const RECEIPT: &str = "rx.component-intake-receipt.v1";
const IMPORTED: &str = "rx.component-import-origin.v1";
fn member_key(transfer: &Id, component: &Id) -> Name {
    name(format!("componenttransfermember/{transfer}/{component}"))
}
fn archive_key(transfer: &Id, seq: Counter) -> Name {
    name(format!("componenttransferhistory/{transfer}/{:020}", seq.0))
}

#[derive(Serialize, Deserialize)]
struct Service {
    boot: Id,
    sources: Bindings,
}
#[derive(Clone, Serialize, Deserialize)]
struct Stage {
    input: Submit,
    key: Id,
    actor: Name,
    owner: Name,
    binding: SourceBinding,
    original: FreezeRecord,
    started_at: TimePoint,
    declarations: Counter,
    history_after: Counter,
    state: State,
}
#[derive(Serialize, Deserialize)]
struct Origin {
    transfer: Id,
    source_revision: Counter,
    versions: Counter,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    registration: Id,
    action: Name,
    entity: Record,
}
fn progress(id: &Id, s: &Stage) -> Progress {
    Progress {
        id: id.clone(),
        source: s.input.source.clone(),
        owner: s.owner.clone(),
        state: s.state.clone(),
        declarations: s.declarations,
        expected_declarations: s.original.declaration_count,
        history_after: s.history_after,
        history_head: s.original.history_head,
    }
}
fn source(tx: &mut dyn Transaction, meta: &Installation, name_: &Name) -> Result<SourceBinding> {
    let (_, service): (_, Service) = load(tx, "componentintake", "service", SERVICE)?;
    if service.boot != meta.runtime_boot {
        return reject(Reject::Unauthenticated);
    }
    service
        .sources
        .get(name_)
        .cloned()
        .ok_or(StoreError::Rejected(Reject::NotFound))
}
fn access(principal: &Principal, owner: &Name) -> Result<()> {
    if principal.id != *owner && !principal.roles.contains(&Role::AccountAdmin) {
        return reject(Reject::Forbidden);
    }
    Ok(())
}
fn ticket(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    t: &Ticket,
) -> Result<Principal> {
    let principal = resident_component::author(tx, &t.identity, meta, now)?;
    access(&principal, &t.binding.owner)?;
    if t.boot != meta.runtime_boot
        || t.installation != meta.id
        || source(tx, meta, &t.input.source)? != t.binding
        || t.binding.fingerprint(&t.input.source)? != t.input.expected_binding
    {
        return reject(Reject::StaleRevision);
    }
    let (_, owner): (_, Principal) = load(tx, "principal", &t.binding.owner, PRINCIPAL)?;
    if !owner.active
        || (!owner.roles.contains(&Role::Engineer) && !owner.roles.contains(&Role::AccountAdmin))
    {
        return reject(Reject::Forbidden);
    }
    lifecycle::require_serving(tx)?;
    Ok(principal)
}
fn staged(tx: &mut dyn Transaction, t: &Ticket) -> Result<(Counter, Stage)> {
    let (revision, stage): (_, Stage) = load(tx, "componenttransfer", &t.input.freeze, STAGE)?;
    if stage.input != t.input
        || stage.key != t.key
        || stage.actor != t.identity.principal
        || stage.binding != t.binding
        || stage.owner != t.binding.owner
        || stage.state != State::Receiving
    {
        return reject(Reject::StaleRevision);
    }
    Ok((revision, stage))
}
/// Every normal component read/write passes here; pending imported rows are not usable.
pub(super) fn require_accepted(tx: &mut dyn Transaction, component: &Id) -> Result<()> {
    if let Some(row) = tx.get(&key("componentimport", component))? {
        let origin: Origin = decode(&row, IMPORTED)?;
        let (_, stage): (_, Stage) = load(tx, "componenttransfer", &origin.transfer, STAGE)?;
        if stage.state != State::Accepted {
            return reject(Reject::InvalidInput);
        }
    }
    Ok(())
}

pub(super) fn require_canonical_report_target(
    tx: &mut dyn Transaction,
    source: &Id,
    target: &Id,
) -> Result<()> {
    if source != target && tx.get(&key("componentimport", source))?.is_some() {
        return Err(StoreError::Invalid(
            "imported source requires its canonical component identity".into(),
        ));
    }
    Ok(())
}

pub(super) fn execution_origin(
    tx: &mut dyn Transaction,
    component: &Id,
) -> Result<Option<rx_domain::resident_execution::LegacyOrigin>> {
    let Some(row) = tx.get(&key("componentimport", component))? else {
        return Ok(None);
    };
    let origin: Origin = decode(&row, IMPORTED)?;
    let (_, stage): (_, Stage) = load(tx, "componenttransfer", &origin.transfer, STAGE)?;
    if stage.state != State::Accepted {
        return reject(Reject::InvalidInput);
    }
    Ok(Some(rx_domain::resident_execution::LegacyOrigin {
        registry: stage.binding.location,
        freeze: stage.original,
    }))
}

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// Trusted startup composition only. Paths never enter the authoritative store.
    pub fn configure_component_sources(&mut self, sources: Bindings) -> Result<()> {
        let meta = &self.installation;
        self.repository.transact(|tx| {
            lifecycle::require_serving(tx)?;
            let k = key("componentintake", "service");
            let old = tx.get(&k)?;
            let service = Service {
                boot: meta.runtime_boot.clone(),
                sources,
            };
            tx.put(&k, old.map(|r| r.revision), &doc(SERVICE, &service)?)?;
            event(tx, "rx.event.component-source-configuration.v1", &service)?;
            Ok(())
        })
    }
    pub fn component_intake_context(
        &mut self,
        identity: &Identity,
        name_: &Name,
    ) -> Result<Context> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let principal = resident_component::author(tx, identity, meta, &clock.now())?;
            let binding = source(tx, meta, name_)?;
            access(&principal, &binding.owner)?;
            Ok(Context {
                source: name_.clone(),
                owner: binding.owner.clone(),
                binding: binding.fingerprint(name_)?,
            })
        })
    }
    pub fn prepare_component_intake(
        &mut self,
        identity: &Identity,
        key_: Id,
        input: Submit,
    ) -> Result<Preflight> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let principal = resident_component::author(tx, identity, meta, &clock.now())?;
            let (scope, fingerprint) = request(
                meta,
                &principal,
                "Component.Import",
                key_.as_str(),
                &(&principal.id, &input),
            )?;
            if let Some(receipt) = prior::<Receipt>(tx, &scope, fingerprint, RECEIPT)? {
                access(&principal, &receipt.owner)?;
                return Ok(Preflight::Recorded(Box::new(receipt)));
            }
            let binding = source(tx, meta, &input.source)?;
            access(&principal, &binding.owner)?;
            if binding.fingerprint(&input.source)? != input.expected_binding {
                return reject(Reject::StaleRevision);
            }
            let prepared = Ticket {
                identity: identity.clone(),
                key: key_,
                input,
                boot: meta.runtime_boot.clone(),
                installation: meta.id.clone(),
                binding,
            };
            ticket(tx, meta, &clock.now(), &prepared)?;
            Ok(Preflight::Read(Box::new(prepared)))
        })
    }
    pub fn begin_component_intake(&mut self, input: Begin) -> Result<Progress> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            ticket(tx, meta, &clock.now(), &input.ticket)?;
            let id = &input.ticket.input.freeze;
            if tx.get(&key("componenttransfer", id))?.is_some() {
                let (_, s) = staged(tx, &input.ticket)?;
                if s.original != input.freeze {
                    return reject(Reject::StaleRevision);
                }
                return Ok(progress(id, &s));
            }
            tx.require_component_intake_reader()?;
            let s = Stage {
                input: input.ticket.input.clone(),
                key: input.ticket.key,
                actor: input.ticket.identity.principal,
                owner: input.ticket.binding.owner.clone(),
                binding: input.ticket.binding,
                original: input.freeze,
                started_at: clock.now(),
                declarations: Counter(0),
                history_after: Counter(0),
                state: State::Receiving,
            };
            save(tx, "componenttransfer", id, None, STAGE, &s)?;
            if let Some(index) = input.selections {
                tx.put(&key("componenttransferindex", id), None, &index.document)?;
            }
            event(
                tx,
                "rx.event.component-transfer-started.v1",
                &progress(id, &s),
            )?;
            Ok(progress(id, &s))
        })
    }
    pub fn stage_component_declarations(&mut self, input: Declarations) -> Result<Progress> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            ticket(tx, meta, &clock.now(), &input.ticket)?;
            let (revision, mut s) = staged(tx, &input.ticket)?;
            let id = &input.ticket.input.freeze;
            if s.declarations != input.start || input.rows.is_empty() || input.rows.len() > 8 {
                return reject(Reject::StaleRevision);
            }
            for row in input.rows {
                let registration: Registration = decode(&row, REGISTRATION_SCHEMA)?;
                if row.key.as_str() != format!("{DECLARATIONS}{}", registration.id) {
                    return Err(StoreError::Integrity("source registration key".into()));
                }
                if tx.get(&key("component", &registration.id))?.is_some()
                    || tx.get(&key("componentimport", &registration.id))?.is_some()
                {
                    return reject(Reject::StaleRevision);
                }
                for row in tx.scan("residentreportscope/")? {
                    let scope: rx_domain::resident_reporting::Scope =
                        decode(&row, "rx.resident-reporting-scope.v1")?;
                    if scope.active && scope.source_registration == registration.id
                        && scope.component != registration.id
                    {
                        return Err(StoreError::Invalid(
                            "revoke the prior diagnostic-component reporting scope before canonical import".into(),
                        ));
                    }
                }
                let record = component::Record {
                    installation: meta.id.clone(),
                    registration: registration.clone(),
                    owner: s.owner.clone(),
                    created_at: s.started_at.clone(),
                    changed_at: s.started_at.clone(),
                    changed_by: s.actor.clone(),
                };
                tx.insert_revision(
                    &key("component", &registration.id), row.revision,
                    &doc(resident_component::COMPONENT, &record)?,
                )?;
                save(tx, "componentimport", &registration.id, None, IMPORTED,
                    &Origin {transfer: id.clone(), source_revision: row.revision, versions: Counter(0)},
                )?;
                tx.put(&member_key(id, &registration.id), None,
                    &doc("rx.component-transfer-member.v1", &registration.id)?,
                )?;
                tx.put(&key("componenttransfersource", (id, &registration.id)), None, &row.document)?;
                s.declarations = s.declarations.increment().map_err(domain_error)?;
                if s.declarations > s.original.declaration_count {
                    return reject(Reject::InvalidInput);
                }
            }
            save(tx, "componenttransfer", id, Some(revision), STAGE, &s)?;
            Ok(progress(id, &s))
        })
    }

    pub fn stage_component_history(&mut self, input: History) -> Result<Progress> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            ticket(tx, meta, &clock.now(), &input.ticket)?;
            let (revision, mut s) = staged(tx, &input.ticket)?;
            let id = &input.ticket.input.freeze;
            if s.declarations != s.original.declaration_count
                || s.history_after != input.after
                || input.events.is_empty()
                || input.events.len() > 8
            {
                return reject(Reject::StaleRevision);
            }
            for original in input.events {
                if original.seq <= s.history_after || original.seq > s.original.history_head {
                    return reject(Reject::InvalidInput);
                }
                if original.document.schema.as_str() == HISTORY_SCHEMA {
                    let change: Change = canonical::decode_json(
                        &canonical::bytes(&original.document.value).map_err(domain_error)?,
                    )
                    .map_err(domain_error)?;
                    if change.entity.key.as_str().starts_with(DECLARATIONS) {
                        let registration: Registration =
                            decode(&change.entity, REGISTRATION_SCHEMA)?;
                        if change.registration != registration.id
                            || change.entity.key.as_str()
                                != format!("{DECLARATIONS}{}", registration.id)
                        {
                            return Err(StoreError::Integrity("source history identity".into()));
                        }
                        let (origin_revision, mut origin): (_, Origin) =
                            load(tx, "componentimport", &registration.id, IMPORTED)?;
                        if origin.transfer != *id
                            || change.entity.revision
                                != origin.versions.increment().map_err(domain_error)?
                            || change.entity.revision > origin.source_revision
                        {
                            return Err(StoreError::Integrity(
                                "source declaration history revision".into(),
                            ));
                        }
                        // State history must follow the legacy writer's irreversible retirement rule.
                        if origin.versions.0 == 0 {
                            if change.action.as_str() != "registered"
                                || registration.state != RegistrationState::Accepted
                            {
                                return Err(StoreError::Integrity(
                                    "source creation history".into(),
                                ));
                            }
                        } else {
                            let (_, old): (_, component::View) = load(
                                tx,
                                "componenthistory",
                                (&registration.id, origin.versions),
                                resident_component::SNAPSHOT,
                            )?;
                            if old.record.registration.state == RegistrationState::Retired {
                                return Err(StoreError::Integrity(
                                    "source rewrote a retired declaration".into(),
                                ));
                            }
                            match change.action.as_str() {
                                "declaration-changed"
                                    if registration.state == RegistrationState::Accepted => {}
                                "retired-new-assignments-prohibited"
                                    if registration.state == RegistrationState::Retired
                                        && registration.declaration
                                            == old.record.registration.declaration => {}
                                _ => {
                                    return Err(StoreError::Integrity(
                                        "source declaration transition".into(),
                                    ));
                                }
                            }
                        }
                        let record = component::Record {
                            installation: meta.id.clone(),
                            registration: registration.clone(),
                            owner: s.owner.clone(),
                            created_at: s.started_at.clone(),
                            changed_at: s.started_at.clone(),
                            changed_by: s.actor.clone(),
                        };
                        let view = component::View::new(change.entity.revision, record);
                        save(
                            tx,
                            "componenthistory",
                            (&registration.id, change.entity.revision),
                            None,
                            resident_component::SNAPSHOT,
                            &view,
                        )?;
                        origin.versions = change.entity.revision;
                        if origin.versions == origin.source_revision {
                            let (_, expected): (_, Registration) = load(
                                tx,
                                "componenttransfersource",
                                (id, &registration.id),
                                REGISTRATION_SCHEMA,
                            )?;
                            if expected != registration {
                                return Err(StoreError::Integrity(
                                    "source current declaration differs from history".into(),
                                ));
                            }
                        }
                        save(
                            tx,
                            "componentimport",
                            &registration.id,
                            Some(origin_revision),
                            IMPORTED,
                            &origin,
                        )?;
                    }
                }
                tx.insert_archive(&archive_key(id, original.seq), &original.document)?;
                save(
                    tx,
                    "componenttransferhistoryid",
                    (id, original.seq),
                    None,
                    "rx.component-transfer-history-id.v1",
                    &original.event_id,
                )?;
                s.history_after = original.seq;
            }
            save(tx, "componenttransfer", id, Some(revision), STAGE, &s)?;
            Ok(progress(id, &s))
        })
    }
    pub fn finish_component_intake(&mut self, input: Finish) -> Result<Receipt> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let principal = ticket(tx, meta, &clock.now(), &input.ticket)?;
            let (scope, fingerprint) = request(
                meta,
                &principal,
                "Component.Import",
                input.ticket.key.as_str(),
                &(&principal.id, &input.ticket.input),
            )?;
            if let Some(old) = prior(tx, &scope, fingerprint, RECEIPT)? {
                return Ok(old);
            }
            let (revision, mut s) = staged(tx, &input.ticket)?;
            let id = &input.ticket.input.freeze;
            if s.declarations != s.original.declaration_count
                || s.history_after != s.original.history_head
            {
                return reject(Reject::InvalidInput);
            }
            let rows = tx.scan(&format!("componenttransfermember/{id}/"))?;
            if rows.len() as u64 != s.declarations.0 {
                return Err(StoreError::Integrity("imported member count".into()));
            }
            for row in rows {
                let component: Id = decode(&row, "rx.component-transfer-member.v1")?;
                let (_, origin): (_, Origin) = load(tx, "componentimport", &component, IMPORTED)?;
                if origin.transfer != *id || origin.versions != origin.source_revision {
                    return Err(StoreError::Integrity(
                        "source declaration history incomplete".into(),
                    ));
                }
            }
            s.state = State::Accepted;
            save(tx, "componenttransfer", id, Some(revision), STAGE, &s)?;
            let receipt = Receipt {
                id: id.clone(),
                source: s.input.source.clone(),
                owner: s.owner,
                input: s.input,
                source_binding: s.binding,
                original: s.original,
                imported_by: principal.id,
                imported_at: clock.now(),
                content_verification: component::Verification::NotEstablished,
                execution_ownership: component::Ownership::NotEstablishedByRegistration,
                work_use_permission: component::WorkUse::NotEvaluated,
            };
            save(tx, "componenttransferreceipt", id, None, RECEIPT, &receipt)?;
            event(tx, "rx.event.component-transfer-accepted.v1", &receipt)?;
            remember(tx, &scope, fingerprint, RECEIPT, &receipt)?;
            Ok(receipt)
        })
    }
    /// Scoped metadata provenance, never a launch or content-verification grant.
    pub fn registration_target_acceptance(
        &mut self,
        identity: &crate::resident_reporting::ReporterIdentity,
        scope: &Id,
        freeze: &Id,
    ) -> Result<rx_domain::component_transfer::TargetAcceptance> {
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let (peer, scope) = resident_reporting::scoped(tx, meta, identity, scope)?;
            if scope.component != scope.source_registration {
                return reject(Reject::Forbidden);
            }
            let (_, origin): (_, Origin) = load(tx, "componentimport", &scope.component, IMPORTED)?;
            if origin.transfer != *freeze {
                return reject(Reject::Forbidden);
            }
            require_accepted(tx, &scope.component)?;
            let (_, receipt): (_, Receipt) = load(tx, "componenttransferreceipt", freeze, RECEIPT)?;
            Ok(rx_domain::component_transfer::TargetAcceptance {
                original: receipt.original.clone(),
                receipt_digest: canonical::digest("RX-REGISTRATION-ACCEPTANCE-v1", &receipt)
                    .map_err(domain_error)?,
                accepted_at: receipt.imported_at,
                component: scope.component,
                scope: scope.id,
                peer,
                process_ownership: rx_domain::component_transfer::ProcessOwnership::NotTransferred,
                content_verification:
                    rx_domain::component_transfer::ContentVerification::NotEstablished,
                work_use_permission: rx_domain::component_transfer::WorkUse::NotEvaluated,
            })
        })
    }

    pub fn component_intake_progress(&mut self, identity: &Identity, id: &Id) -> Result<Progress> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let principal = resident_component::author(tx, identity, meta, &clock.now())?;
            let (_, s): (_, Stage) = load(tx, "componenttransfer", id, STAGE)?;
            access(&principal, &s.owner)?;
            Ok(progress(id, &s))
        })
    }
    pub fn component_intake_receipt(&mut self, identity: &Identity, id: &Id) -> Result<Receipt> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let principal = resident_component::author(tx, identity, meta, &clock.now())?;
            let (_, r): (_, Receipt) = load(tx, "componenttransferreceipt", id, RECEIPT)?;
            access(&principal, &r.owner)?;
            Ok(r)
        })
    }
    pub fn component_intake_history(
        &mut self,
        identity: &Identity,
        id: &Id,
        after: Counter,
    ) -> Result<ArchivePage> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let principal = resident_component::author(tx, identity, meta, &clock.now())?;
            let (_, s): (_, Stage) = load(tx, "componenttransfer", id, STAGE)?;
            access(&principal, &s.owner)?;
            let mut events = Vec::new();
            let prefix = format!("componenttransferhistory/{id}/");
            let cursor = archive_key(id, after);
            let rows = tx.scan_page(&prefix, Some(&cursor), 8)?;
            for row in rows {
                let seq = Counter(
                    row.key
                        .as_str()
                        .rsplit('/')
                        .next()
                        .ok_or_else(|| StoreError::Integrity("history key".into()))?
                        .parse()
                        .map_err(|_| StoreError::Integrity("history cursor".into()))?,
                );
                if seq <= after {
                    continue;
                }
                let (_, event_id): (_, Id) = load(
                    tx,
                    "componenttransferhistoryid",
                    (id, seq),
                    "rx.component-transfer-history-id.v1",
                )?;
                events.push(StoredEvent {
                    seq,
                    event_id,
                    document: row.document,
                });
                if events.len() == 8 {
                    break;
                }
            }
            let next_after = events
                .last()
                .filter(|e| e.seq < s.history_after)
                .map(|e| e.seq);
            Ok(ArchivePage {
                id: id.clone(),
                events,
                next_after,
            })
        })
    }
}
