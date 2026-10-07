use super::*;
use crate::resident_reporting::{Continue, Issue, ReporterIdentity, Revoke, ScopeView};
use rx_domain::{component::RegistrationState, resident_reporting::*};

const PEER: &str = "rx.internal.resident-reporter.v1";
const CURRENT: &str = "rx.internal.resident-reporter-current.v1";
const SCOPE: &str = "rx.resident-reporting-scope.v1";
const RECEIPT: &str = "rx.resident-report-receipt.v1";
const SCOPE_REPLY: &str = "rx.resident-reporting-scope-view.v1";

fn observer(tx: &mut dyn Transaction, principal: &Name) -> Result<Principal> {
    let (_, principal): (_, Principal) = load(tx, "principal", principal, PRINCIPAL)?;
    if !principal.active || principal.roles != BTreeSet::from([Role::Observer]) {
        return reject(Reject::Forbidden);
    }
    Ok(principal)
}

fn current_peer(tx: &mut dyn Transaction, meta: &Installation, peer: &Peer) -> Result<bool> {
    let (_, current): (_, Id) = load(tx, "residentpeer", &peer.principal, CURRENT)?;
    Ok(current == peer.id
        && peer.installation == meta.id
        && peer.store_generation == meta.store_generation
        && peer.runtime_boot == meta.runtime_boot)
}

fn authenticated(
    tx: &mut dyn Transaction,
    meta: &Installation,
    identity: &ReporterIdentity,
) -> Result<Peer> {
    observer(tx, &identity.principal)?;
    let (_, peer): (_, Peer) = load(tx, "residentpeersession", &identity.session, PEER)?;
    if peer.id != identity.session
        || peer.principal != identity.principal
        || peer.authentication_binding != identity.authentication_binding
        || !current_peer(tx, meta, &peer)?
    {
        return reject(Reject::Unauthenticated);
    }
    Ok(peer)
}

pub(super) fn scoped(
    tx: &mut dyn Transaction,
    meta: &Installation,
    identity: &ReporterIdentity,
    id: &Id,
) -> Result<(Peer, Scope)> {
    let peer = authenticated(tx, meta, identity)?;
    let (_, scope): (_, Scope) = load(tx, "residentreportscope", id, SCOPE)?;
    if scope.id != *id || !scope.active || scope.reporter_session != peer.id {
        return reject(Reject::Forbidden);
    }
    Ok((peer, scope))
}

fn same_lineage(tx: &mut dyn Transaction, scope: &Scope, receipt: &Receipt) -> Result<()> {
    let (_, origin): (_, Scope) = load(tx, "residentreportscope", &receipt.report.scope, SCOPE)?;
    if origin.root_scope() != scope.root_scope()
        || receipt.component != scope.component
        || receipt.component_revision != scope.component_revision
        || receipt.report.source.registration != scope.source_registration
        || receipt.report.source.registration_revision != scope.source_revision
        || receipt.report.source.catalog != scope.catalog
    {
        return reject(Reject::Forbidden);
    }
    Ok(())
}

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// Trusted mTLS composition only. No ordinary user session or Host registration is created.
    pub fn open_resident_reporter(
        &mut self,
        principal: &Name,
        peer_boot: Id,
        authentication_binding: Digest,
    ) -> Result<Peer> {
        let meta = &self.installation;
        self.repository.transact(|tx| {
            observer(tx, principal)?;
            lifecycle::require_serving(tx)?;
            let pointer = key("residentpeer", principal);
            let old = tx.get(&pointer)?;
            if let Some(old) = &old {
                let previous: Id = decode(old, CURRENT)?;
                let (_, peer): (_, Peer) = load(tx, "residentpeersession", &previous, PEER)?;
                if peer.id != previous || peer.principal != *principal {
                    return Err(StoreError::Integrity("reporter identity differs".into()));
                }
                if peer.peer_boot == peer_boot
                    && peer.authentication_binding == authentication_binding
                    && current_peer(tx, meta, &peer)?
                {
                    return Ok(peer);
                }
            }
            let peer = Peer {
                id: id(),
                principal: principal.clone(),
                peer_boot,
                installation: meta.id.clone(),
                store_generation: meta.store_generation.clone(),
                runtime_boot: meta.runtime_boot.clone(),
                authentication_binding,
            };
            save(tx, "residentpeersession", &peer.id, None, PEER, &peer)?;
            tx.put(&pointer, old.map(|v| v.revision), &doc(CURRENT, &peer.id)?)?;
            event(tx, "rx.event.resident-reporter-opened.v1", &peer)?;
            Ok(peer)
        })
    }

    pub fn issue_resident_reporting(
        &mut self,
        identity: &Identity,
        request_key: &str,
        input: Issue,
    ) -> Result<ScopeView> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let principal = resident_component::author(tx, identity, meta, &now)?;
            let (revision, component) =
                resident_component::owned(tx, &principal, meta, &input.component)?;
            component_intake::require_canonical_report_target(
                tx,
                &input.source_registration,
                &input.component,
            )?;
            let (request, fingerprint) = request(
                meta,
                &principal,
                "Component.IssueReporting",
                request_key,
                &(&principal.id, &input),
            )?;
            if let Some(previous) = prior(tx, &request, fingerprint, SCOPE_REPLY)? {
                return Ok(previous);
            }
            lifecycle::require_serving(tx)?;
            check_revision(revision, input.expected_component_revision)?;
            input
                .source_revision
                .nonzero("source revision")
                .map_err(domain_error)?;
            if component.registration.state != RegistrationState::Accepted {
                return reject(Reject::InvalidInput);
            }
            let (_, peer): (_, Peer) =
                load(tx, "residentpeersession", &input.reporter_session, PEER)?;
            observer(tx, &peer.principal)?;
            if !current_peer(tx, meta, &peer)? {
                return reject(Reject::Unauthenticated);
            }
            let scope = Scope {
                id: id(),
                component: input.component,
                component_revision: revision,
                source_registration: input.source_registration,
                source_revision: input.source_revision,
                catalog: component.registration.declaration.catalog,
                reporter_session: peer.id,
                issued_by: principal.id,
                issued_at: now,
                active: true,
                continuation: None,
            };
            let revision = save(tx, "residentreportscope", &scope.id, None, SCOPE, &scope)?;
            let view = ScopeView { revision, scope };
            event(tx, "rx.event.resident-reporting-issued.v1", &view)?;
            remember(tx, &request, fingerprint, SCOPE_REPLY, &view)?;
            Ok(view)
        })
    }

    pub fn revoke_resident_reporting(
        &mut self,
        identity: &Identity,
        request_key: &str,
        input: Revoke,
    ) -> Result<ScopeView> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let principal = resident_component::author(tx, identity, meta, &clock.now())?;
            let (revision, mut scope): (_, Scope) =
                load(tx, "residentreportscope", &input.scope, SCOPE)?;
            resident_component::owned(tx, &principal, meta, &scope.component)?;
            let (request, fingerprint) = request(
                meta,
                &principal,
                "Component.RevokeReporting",
                request_key,
                &(&principal.id, &input),
            )?;
            if let Some(previous) = prior(tx, &request, fingerprint, SCOPE_REPLY)? {
                return Ok(previous);
            }
            check_revision(revision, input.expected_revision)?;
            scope.active = false;
            let revision = save(
                tx,
                "residentreportscope",
                &scope.id,
                Some(revision),
                SCOPE,
                &scope,
            )?;
            let view = ScopeView { revision, scope };
            event(tx, "rx.event.resident-reporting-revoked.v1", &view)?;
            remember(tx, &request, fingerprint, SCOPE_REPLY, &view)?;
            Ok(view)
        })
    }

    /// Reauthorize metadata continuity only; old receipts retain their original peer/scope.
    pub fn continue_resident_reporting(
        &mut self,
        identity: &Identity,
        request_key: &str,
        input: Continue,
    ) -> Result<ScopeView> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let principal = resident_component::author(tx, identity, meta, &now)?;
            let (revision, mut previous): (_, Scope) =
                load(tx, "residentreportscope", &input.scope, SCOPE)?;
            resident_component::owned(tx, &principal, meta, &previous.component)?;
            component_intake::require_canonical_report_target(
                tx,
                &previous.source_registration,
                &previous.component,
            )?;
            let (request, fingerprint) = request(
                meta,
                &principal,
                "Component.ContinueReporting",
                request_key,
                &(&principal.id, &input),
            )?;
            if let Some(old) = prior(tx, &request, fingerprint, SCOPE_REPLY)? {
                return Ok(old);
            }
            lifecycle::require_serving(tx)?;
            check_revision(revision, input.expected_revision)?;
            // A revoked or already continued scope cannot branch into a second successor.
            if !previous.active {
                return reject(Reject::Forbidden);
            }
            let (_, peer): (_, Peer) =
                load(tx, "residentpeersession", &input.reporter_session, PEER)?;
            observer(tx, &peer.principal)?;
            if !current_peer(tx, meta, &peer)? {
                return reject(Reject::Unauthenticated);
            }
            let mut scope = previous.clone();
            scope.id = id();
            scope.reporter_session = peer.id;
            scope.issued_by = principal.id;
            scope.issued_at = now;
            scope.continuation = Some(Continuation {
                previous_scope: previous.id.clone(),
                root_scope: previous.root_scope().clone(),
            });
            previous.active = false;
            save(
                tx,
                "residentreportscope",
                &previous.id,
                Some(revision),
                SCOPE,
                &previous,
            )?;
            let revision = save(tx, "residentreportscope", &scope.id, None, SCOPE, &scope)?;
            let view = ScopeView { revision, scope };
            event(tx, "rx.event.resident-reporting-continued.v1", &view)?;
            remember(tx, &request, fingerprint, SCOPE_REPLY, &view)?;
            Ok(view)
        })
    }

    pub fn resident_report_head(
        &mut self,
        identity: &ReporterIdentity,
        scope_id: &Id,
        instance: &Id,
    ) -> Result<Head> {
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let (_, scope) = scoped(tx, meta, identity, scope_id)?;
            let row = tx.get(&key("residentreport", (&scope.component, instance)))?;
            let receipt = row
                .map(|row| decode::<Receipt>(&row, RECEIPT))
                .transpose()?;
            if let Some(receipt) = &receipt {
                same_lineage(tx, &scope, receipt)?;
            }
            Ok(Head {
                scope: scope.id,
                instance: instance.clone(),
                receipt,
            })
        })
    }

    pub fn resident_reporting_scope(
        &mut self,
        identity: &ReporterIdentity,
        scope: &Id,
    ) -> Result<Scope> {
        let meta = &self.installation;
        self.repository
            .transact(|tx| scoped(tx, meta, identity, scope).map(|(_, scope)| scope))
    }

    pub fn inspect_resident_reporting(
        &mut self,
        identity: &Identity,
        scope: &Id,
    ) -> Result<ScopeView> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let principal = resident_component::author(tx, identity, meta, &clock.now())?;
            let (revision, scope): (_, Scope) = load(tx, "residentreportscope", scope, SCOPE)?;
            resident_component::owned(tx, &principal, meta, &scope.component)?;
            Ok(ScopeView { revision, scope })
        })
    }

    pub fn publish_resident_report(
        &mut self,
        identity: &ReporterIdentity,
        request_key: &str,
        report: Report,
    ) -> Result<Receipt> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let (peer, scope) = scoped(tx, meta, identity, &report.scope)?;
            let principal = observer(tx, &peer.principal)?;
            let (request, fingerprint) = request(
                meta,
                &principal,
                "Component.PublishReport",
                request_key,
                &(&peer.id, &report),
            )?;
            if let Some(previous) = prior(tx, &request, fingerprint, RECEIPT)? {
                return Ok(previous);
            }
            report
                .sequence
                .nonzero("report sequence")
                .map_err(domain_error)?;
            if report.source.registration != scope.source_registration
                || report.source.registration_revision != scope.source_revision
                || report.source.catalog != scope.catalog
                || report.pid == Some(0)
                || report.detail.len() > 2048
            {
                return reject(Reject::InvalidInput);
            }
            let latest = key(
                "residentreport",
                (&scope.component, &report.source.instance),
            );
            let old = tx.get(&latest)?;
            if let Some(old) = &old {
                let previous: Receipt = decode(old, RECEIPT)?;
                same_lineage(tx, &scope, &previous)?;
                if previous.report.source != report.source {
                    return reject(Reject::Forbidden);
                }
                if previous.report.sequence.increment().map_err(domain_error)? != report.sequence {
                    return reject(Reject::StaleRevision);
                }
                if matches!(
                    previous.report.state,
                    ExecutionState::Exited | ExecutionState::NotStarted
                ) && (report.state != previous.report.state
                    || report.exit_code != previous.report.exit_code)
                {
                    return reject(Reject::InvalidInput);
                }
            } else {
                if report.sequence != Counter(1) {
                    return reject(Reject::InvalidInput);
                }
            }
            let receipt = Receipt {
                id: id(),
                component: scope.component,
                component_revision: scope.component_revision,
                reporter: peer,
                report,
                accepted_at: clock.now(),
                basis: Basis::ReportedRegistrySnapshot,
                execution_ownership: Ownership::NotEstablishedByReport,
                work_use_permission: WorkUse::NotEvaluated,
            };
            tx.put(&latest, old.map(|r| r.revision), &doc(RECEIPT, &receipt)?)?;
            save(
                tx,
                "residentreportreceipt",
                &receipt.id,
                None,
                RECEIPT,
                &receipt,
            )?;
            event(tx, "rx.event.resident-report-received.v1", &receipt)?;
            remember(tx, &request, fingerprint, RECEIPT, &receipt)?;
            Ok(receipt)
        })
    }

    pub fn resident_report(
        &mut self,
        identity: &Identity,
        component: &Id,
        instance: &Id,
    ) -> Result<View> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let principal = resident_component::author(tx, identity, meta, &clock.now())?;
            let (revision, registration) =
                resident_component::owned(tx, &principal, meta, component)?;
            let (_, receipt): (_, Receipt) =
                load(tx, "residentreport", (component, instance), RECEIPT)?;
            let (_, scope): (_, Scope) =
                load(tx, "residentreportscope", &receipt.report.scope, SCOPE)?;
            let observer_active = match observer(tx, &receipt.reporter.principal) {
                Ok(_) => true,
                Err(StoreError::Rejected(Reject::Forbidden | Reject::NotFound)) => false,
                Err(error) => return Err(error),
            };
            let reporter_session_current =
                scope.active && observer_active && current_peer(tx, meta, &receipt.reporter)?;
            Ok(View {
                registration_revision_current: revision == receipt.component_revision
                    && registration.registration.state == RegistrationState::Accepted,
                reporter_session_current,
                receipt,
            })
        })
    }
}
