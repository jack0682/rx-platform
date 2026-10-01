use super::*;
use crate::resident_execution::Identity as SupervisorIdentity;
use rx_domain::{component::RegistrationState, resident_execution as execution};
use serde::Deserialize;
const SERVICE: &str = "rx.internal.resident-execution-service.v1";
const PEER: &str = "rx.resident-execution-peer.v1";
const CURRENT: &str = "rx.resident-execution-current.v1";
const ASSIGNMENT: &str = "rx.resident-execution-assignment.v1";
const VIEW: &str = "rx.resident-execution-view.v1";
const CLAIM: &str = "rx.resident-execution-claim.v1";
const CONTENT: &str = "rx.resident-execution-content.v1";
const OBSERVATION: &str = "rx.resident-execution-observation.v1";
#[derive(Serialize, Deserialize)]
struct Service {
    boot: Id,
    entries: BTreeMap<Name, execution::Enrollment>,
}
fn enrollment(
    tx: &mut dyn Transaction,
    meta: &Installation,
    principal: &Name,
) -> Result<execution::Enrollment> {
    let (_, service): (_, Service) = load(tx, "residentexecutionservice", "current", SERVICE)?;
    if service.boot != meta.runtime_boot {
        return reject(Reject::Unauthenticated);
    }
    service
        .entries
        .get(principal)
        .cloned()
        .ok_or(StoreError::Rejected(Reject::Forbidden))
}
fn supervisor(tx: &mut dyn Transaction, principal: &Name) -> Result<()> {
    let (_, p): (_, Principal) = load(tx, "principal", principal, PRINCIPAL)?;
    if !p.active || p.roles != BTreeSet::from([Role::Supervisor]) {
        return reject(Reject::Forbidden);
    }
    Ok(())
}
fn current(tx: &mut dyn Transaction, meta: &Installation, peer: &execution::Peer) -> Result<bool> {
    supervisor(tx, &peer.principal)?;
    let (_, pointer): (_, Id) = load(tx, "residentexecutionpeer", &peer.principal, CURRENT)?;
    let policy = enrollment(tx, meta, &peer.principal)?;
    Ok(pointer == peer.id
        && peer.installation == meta.id
        && peer.store_generation == meta.store_generation
        && peer.runtime_boot == meta.runtime_boot
        && peer.registry == policy.registry
        && peer.enrollment
            == policy
                .digest(&peer.principal)
                .map_err(StoreError::Invalid)?)
}
fn authenticated(
    tx: &mut dyn Transaction,
    meta: &Installation,
    identity: &SupervisorIdentity,
) -> Result<execution::Peer> {
    let (_, peer): (_, execution::Peer) =
        load(tx, "residentexecutionpeersession", &identity.session, PEER)?;
    if peer.id != identity.session
        || peer.principal != identity.principal
        || peer.authentication_binding != identity.authentication_binding
        || !current(tx, meta, &peer)?
    {
        return reject(Reject::Unauthenticated);
    }
    Ok(peer)
}
fn owned(principal: &Principal, assignment: &execution::Assignment) -> Result<()> {
    if principal.id != assignment.intent.owner && !principal.roles.contains(&Role::AccountAdmin) {
        return reject(Reject::Forbidden);
    }
    Ok(())
}
fn held(tx: &mut dyn Transaction, component: &Id) -> Result<Option<Id>> {
    tx.get(&key("residentexecutionclaim", component))?
        .map(|r| decode(&r, CLAIM))
        .transpose()
        .map(Option::flatten)
}
pub(super) fn require_unassigned(tx: &mut dyn Transaction, component: &Id) -> Result<()> {
    if held(tx, component)?.is_some() {
        return Err(StoreError::Invalid("resident execution still holds this component; stop and reconcile before changing its declaration".into()));
    }
    Ok(())
}
fn view(
    tx: &mut dyn Transaction,
    meta: &Installation,
    revision: Counter,
    assignment: execution::Assignment,
) -> Result<execution::View> {
    let peer_current = match assignment.content.as_ref() {
        None => false,
        Some(content) => match current(tx, meta, &content.preparation.peer) {
            Ok(value) => value,
            Err(StoreError::Rejected(_)) => false,
            Err(error) => return Err(error),
        },
    };
    let mut claims_held = false;
    for n in assignment.intent.nodes.values() {
        claims_held |= held(tx, &n.registration.id)?.as_ref() == Some(&assignment.intent.id);
    }
    Ok(execution::View {
        revision,
        assignment,
        peer_current,
        claims_held,
    })
}
fn record(
    tx: &mut dyn Transaction,
    id: &Id,
    expected: Option<Counter>,
    value: &execution::Assignment,
) -> Result<Counter> {
    let revision = save(tx, "residentexecution", id, expected, ASSIGNMENT, value)?;
    event(tx, "rx.event.resident-execution.v1", value)?;
    Ok(revision)
}
fn release(tx: &mut dyn Transaction, value: &execution::Assignment) -> Result<()> {
    for node in value.intent.nodes.values() {
        let k = key("residentexecutionclaim", &node.registration.id);
        if let Some(row) = tx.get(&k)? {
            let claim: Option<Id> = decode(&row, CLAIM)?;
            if claim.as_ref() == Some(&value.intent.id) {
                tx.put(&k, Some(row.revision), &doc(CLAIM, &Option::<Id>::None)?)?;
            }
        }
    }
    Ok(())
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn configure_resident_supervisors(
        &mut self,
        entries: BTreeMap<Name, execution::Enrollment>,
    ) -> Result<()> {
        for (name, policy) in &entries {
            policy.digest(name).map_err(StoreError::Invalid)?;
        }
        let meta = &self.installation;
        self.repository.transact(|tx| {
            lifecycle::require_serving(tx)?;
            if !entries.is_empty() {
                tx.require_resident_execution_reader()?;
            }
            let k = key("residentexecutionservice", "current");
            let old = tx.get(&k)?;
            let service = Service {
                boot: meta.runtime_boot.clone(),
                entries,
            };
            tx.put(&k, old.map(|r| r.revision), &doc(SERVICE, &service)?)?;
            event(tx, "rx.event.resident-supervisor-enrollment.v1", &service)?;
            Ok(())
        })
    }
    pub fn open_resident_supervisor(
        &mut self,
        principal: &Name,
        peer_boot: Id,
        authentication_binding: Digest,
        registry: Digest,
    ) -> Result<execution::Peer> {
        let meta = &self.installation;
        self.repository.transact(|tx| {
            lifecycle::require_serving(tx)?;
            supervisor(tx, principal)?;
            let policy = enrollment(tx, meta, principal)?;
            let fingerprint = policy.digest(principal).map_err(StoreError::Invalid)?;
            if policy.registry != registry {
                return Err(StoreError::Invalid(
                    "Supervisor registry is not enrolled".into(),
                ));
            }
            let k = key("residentexecutionpeer", principal);
            let previous = tx.get(&k)?;
            if let Some(row) = &previous {
                let old: Id = decode(row, CURRENT)?;
                let (_, peer): (_, execution::Peer) =
                    load(tx, "residentexecutionpeersession", &old, PEER)?;
                if peer.peer_boot == peer_boot
                    && peer.authentication_binding == authentication_binding
                    && current(tx, meta, &peer)?
                {
                    return Ok(peer);
                }
            }
            tx.require_resident_execution_reader()?;
            let peer = execution::Peer {
                registry,
                id: id(),
                principal: principal.clone(),
                peer_boot,
                installation: meta.id.clone(),
                store_generation: meta.store_generation.clone(),
                runtime_boot: meta.runtime_boot.clone(),
                authentication_binding,
                enrollment: fingerprint,
            };
            save(
                tx,
                "residentexecutionpeersession",
                &peer.id,
                None,
                PEER,
                &peer,
            )?;
            tx.put(&k, previous.map(|r| r.revision), &doc(CURRENT, &peer.id)?)?;
            event(tx, "rx.event.resident-supervisor-opened.v1", &peer)?;
            Ok(peer)
        })
    }
    pub fn propose_resident_execution(
        &mut self,
        identity: &Identity,
        key_: &str,
        input: execution::Propose,
    ) -> Result<execution::View> {
        input.validate().map_err(StoreError::Invalid)?;
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let principal = resident_component::author(tx, identity, meta, &now)?;
            let (scope, fingerprint) = request(
                meta,
                &principal,
                "ResidentExecution.Propose",
                key_,
                &(&principal.id, &input),
            )?;
            if let Some(old) = prior(tx, &scope, fingerprint, VIEW)? {
                return Ok(old);
            }
            lifecycle::require_serving(tx)?;
            supervisor(tx, &input.supervisor)?;
            let policy = enrollment(tx, meta, &input.supervisor)?;
            let mut nodes = BTreeMap::new();
            for (selection, value) in input.selections {
                let (revision, component) =
                    resident_component::owned(tx, &principal, meta, &value.component)?;
                check_revision(revision, value.expected_revision)?;
                if component.registration.state != RegistrationState::Accepted {
                    return reject(Reject::InvalidInput);
                }
                let catalog = &component.registration.declaration.catalog;
                if policy
                    .programs
                    .get(&catalog.program)
                    .is_none_or(|p| p.digest != catalog.digest)
                {
                    return Err(StoreError::Invalid(
                        "component catalog is not enrolled for this Supervisor".into(),
                    ));
                }
                let origin = component_intake::execution_origin(tx, &component.registration.id)?;
                if origin
                    .as_ref()
                    .is_some_and(|o| o.registry != policy.registry)
                {
                    return Err(StoreError::Invalid(
                        "imported component belongs to another source registry".into(),
                    ));
                }
                nodes.insert(
                    selection,
                    execution::Node {
                        origin,
                        registration: component.registration,
                        revision,
                        instance: id(),
                        selection: value,
                    },
                );
            }
            tx.require_resident_execution_reader()?;
            let intent = execution::Intent {
                id: id(),
                run: id(),
                owner: principal.id,
                supervisor: input.supervisor,
                environment: input.environment,
                profiles: input.profiles,
                nodes,
                created_at: now,
            };
            let assignment = execution::Assignment {
                intent,
                phase: execution::Phase::Proposed,
                content: None,
                grant: None,
                observed: None,
                stop_requested: false,
            };
            let revision = record(tx, &assignment.intent.id, None, &assignment)?;
            let result = view(tx, meta, revision, assignment)?;
            remember(tx, &scope, fingerprint, VIEW, &result)?;
            Ok(result)
        })
    }
    pub fn resident_execution(&mut self, identity: &Identity, id: &Id) -> Result<execution::View> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let principal = resident_component::author(tx, identity, meta, &clock.now())?;
            let (revision, value): (_, execution::Assignment) =
                load(tx, "residentexecution", id, ASSIGNMENT)?;
            owned(&principal, &value)?;
            view(tx, meta, revision, value)
        })
    }
    pub fn inspect_resident_execution(
        &mut self,
        identity: &SupervisorIdentity,
        id: &Id,
    ) -> Result<execution::View> {
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let peer = authenticated(tx, meta, identity)?;
            let (revision, value): (_, execution::Assignment) =
                load(tx, "residentexecution", id, ASSIGNMENT)?;
            if value.intent.supervisor != peer.principal {
                return reject(Reject::Forbidden);
            }
            view(tx, meta, revision, value)
        })
    }
    pub fn prepare_resident_execution(
        &mut self,
        identity: &SupervisorIdentity,
        key_: &str,
        input: execution::Preparation,
    ) -> Result<execution::ContentReceipt> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let peer = authenticated(tx, meta, identity)?;
            if input.peer != peer {
                return reject(Reject::Forbidden);
            }
            let principal = load::<Principal>(tx, "principal", &peer.principal, PRINCIPAL)?.1;
            let (scope, fingerprint) =
                request(meta, &principal, "ResidentExecution.Prepare", key_, &input)?;
            if let Some(old) = prior(tx, &scope, fingerprint, CONTENT)? {
                return Ok(old);
            }
            lifecycle::require_serving(tx)?;
            let (revision, mut value): (_, execution::Assignment) =
                load(tx, "residentexecution", &input.assignment, ASSIGNMENT)?;
            if value.intent.supervisor != peer.principal
                || value.phase != execution::Phase::Proposed
                || input.intent_digest != value.intent.digest().map_err(StoreError::Invalid)?
                || input.programs.keys().collect::<BTreeSet<_>>()
                    != value.intent.nodes.keys().collect()
            {
                return reject(Reject::InvalidInput);
            }
            let policy = enrollment(tx, meta, &peer.principal)?;
            if !policy.releases.contains(&input.release) {
                return Err(StoreError::Invalid(
                    "verified release is not enrolled".into(),
                ));
            }
            for (selection, verified) in &input.programs {
                let node = &value.intent.nodes[selection];
                if node.origin.as_ref().is_some_and(|origin| {
                    origin.registry != peer.registry
                        || input.legacy_source.as_ref() != Some(&origin.freeze)
                }) {
                    return Err(StoreError::Invalid(
                        "legacy registration source differs from preparation".into(),
                    ));
                }
                let catalog = &node.registration.declaration.catalog;
                if &verified.catalog != catalog
                    || policy
                        .programs
                        .get(&catalog.program)
                        .is_none_or(|p| p.digest != catalog.digest || p.effect != verified.effect)
                {
                    return Err(StoreError::Invalid(
                        "preparation catalog/effect differs".into(),
                    ));
                }
            }
            let receipt = execution::ContentReceipt {
                preparation: input,
                basis: execution::ContentBasis::EnrolledSupervisorVerifiedRelease,
                recorded_at: clock.now(),
            };
            value.content = Some(receipt.clone());
            value.phase = execution::Phase::Prepared;
            record(tx, &value.intent.id, Some(revision), &value)?;
            remember(tx, &scope, fingerprint, CONTENT, &receipt)?;
            Ok(receipt)
        })
    }
    pub fn approve_resident_execution(
        &mut self,
        identity: &Identity,
        key_: &str,
        input: execution::Approve,
    ) -> Result<execution::View> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let principal = resident_component::author(tx, identity, meta, &now)?;
            let (scope, fingerprint) = request(
                meta,
                &principal,
                "ResidentExecution.Approve",
                key_,
                &(&principal.id, &input),
            )?;
            let (revision, mut value): (_, execution::Assignment) =
                load(tx, "residentexecution", &input.assignment, ASSIGNMENT)?;
            owned(&principal, &value)?;
            if let Some(old) = prior(tx, &scope, fingerprint, VIEW)? {
                return Ok(old);
            }
            lifecycle::require_serving(tx)?;
            check_revision(revision, input.expected_revision)?;
            if value.phase != execution::Phase::Prepared
                || !(100..=30_000).contains(&input.start_window_ms.0)
            {
                return reject(Reject::InvalidInput);
            }
            let content = value
                .content
                .as_ref()
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
            if input.preparation_digest
                != content.preparation.digest().map_err(StoreError::Invalid)?
                || !current(tx, meta, &content.preparation.peer)?
            {
                return reject(Reject::StaleRevision);
            }
            if content
                .preparation
                .programs
                .values()
                .any(|p| p.effect == execution::Effect::RequiresPlatformAuthority)
            {
                return Err(StoreError::Invalid(
                    "physical lifecycle authority is not connected to this start path".into(),
                ));
            }
            for node in value.intent.nodes.values() {
                let (current_revision, component) =
                    resident_component::owned(tx, &principal, meta, &node.registration.id)?;
                if current_revision != node.revision || component.registration != node.registration
                {
                    return reject(Reject::StaleRevision);
                }
                require_unassigned(tx, &node.registration.id)?;
            }
            let until = now
                .ticks_ns
                .0
                .checked_add(input.start_window_ms.0 * 1_000_000)
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
            value.grant = Some(execution::Grant {
                id: id(),
                assignment: value.intent.id.clone(),
                intent_digest: value.intent.digest().map_err(StoreError::Invalid)?,
                preparation_digest: input.preparation_digest,
                peer: content.preparation.peer.clone(),
                issued_at: now.clone(),
                valid_until: TimePoint {
                    clock_id: now.clock_id,
                    ticks_ns: Counter(until),
                },
            });
            value.phase = execution::Phase::Granted;
            for node in value.intent.nodes.values() {
                let k = key("residentexecutionclaim", &node.registration.id);
                let old = tx.get(&k)?;
                tx.put(
                    &k,
                    old.map(|r| r.revision),
                    &doc(CLAIM, &Some(&value.intent.id))?,
                )?;
            }
            let revision = record(tx, &value.intent.id, Some(revision), &value)?;
            let result = view(tx, meta, revision, value)?;
            remember(tx, &scope, fingerprint, VIEW, &result)?;
            Ok(result)
        })
    }
    pub fn stop_resident_execution(
        &mut self,
        identity: &Identity,
        key_: &str,
        input: execution::Stop,
    ) -> Result<execution::View> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let principal = resident_component::author(tx, identity, meta, &clock.now())?;
            let (scope, fingerprint) = request(
                meta,
                &principal,
                "ResidentExecution.Stop",
                key_,
                &(&principal.id, &input),
            )?;
            let (revision, mut value): (_, execution::Assignment) =
                load(tx, "residentexecution", &input.assignment, ASSIGNMENT)?;
            owned(&principal, &value)?;
            if let Some(old) = prior(tx, &scope, fingerprint, VIEW)? {
                return Ok(old);
            }
            check_revision(revision, input.expected_revision)?;
            use execution::Phase as P;
            value.stop_requested = true;
            value.phase = match value.phase {
                P::Proposed | P::Prepared => P::Cancelled,
                P::Granted | P::Running => P::StopRequested,
                other => other,
            };
            let revision = record(tx, &value.intent.id, Some(revision), &value)?;
            let result = view(tx, meta, revision, value)?;
            remember(tx, &scope, fingerprint, VIEW, &result)?;
            Ok(result)
        })
    }
    pub fn observe_resident_execution(
        &mut self,
        identity: &SupervisorIdentity,
        key_: &str,
        input: execution::Observation,
    ) -> Result<execution::ObservationReceipt> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let peer = authenticated(tx, meta, identity)?;
            let principal = load::<Principal>(tx, "principal", &peer.principal, PRINCIPAL)?.1;
            let (scope, fingerprint) =
                request(meta, &principal, "ResidentExecution.Observe", key_, &input)?;
            let (revision, mut value): (_, execution::Assignment) =
                load(tx, "residentexecution", &input.assignment, ASSIGNMENT)?;
            let grant = value
                .grant
                .as_ref()
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
            if grant.peer != peer
                || grant.id != input.grant
                || value.intent.supervisor != peer.principal
            {
                return reject(Reject::Forbidden);
            }
            if let Some(old) = prior(tx, &scope, fingerprint, OBSERVATION)? {
                return Ok(old);
            }
            if input.sequence
                != value.observed.as_ref().map_or(Counter(1), |o| {
                    Counter(o.observation.sequence.0.saturating_add(1))
                })
                || input.nodes.keys().collect::<BTreeSet<_>>()
                    != value.intent.nodes.keys().collect()
            {
                return reject(Reject::InvalidInput);
            }
            use execution::ObservationState as O;
            for (selection, node) in &input.nodes {
                if node.instance != value.intent.nodes[selection].instance
                    || node.pid == Some(0)
                    || node.detail.is_empty()
                    || node.detail.len() > 1024
                    || (matches!(node.state, O::Running | O::Exited) && node.pid.is_none())
                    || (node.state == O::NotStarted
                        && (node.pid.is_some() || node.exit_code.is_some()))
                {
                    return reject(Reject::InvalidInput);
                }
                if let Some(previous) = value
                    .observed
                    .as_ref()
                    .map(|o| &o.observation.nodes[selection])
                    && ((matches!(previous.state, O::Exited | O::NotStarted)
                        && (node.state != previous.state
                            || node.pid != previous.pid
                            || node.exit_code != previous.exit_code))
                        || (previous.state == O::Unknown && node.state != O::Unknown)
                        || (previous.state == O::Running
                            && matches!(node.state, O::Assigned | O::NotStarted)))
                {
                    return reject(Reject::InvalidInput);
                }
            }
            let uncertain = input.reconciliation_required
                || input.nodes.values().any(|n| n.state == O::Unknown)
                || value.phase == execution::Phase::Unknown;
            let terminal = input
                .nodes
                .values()
                .all(|n| matches!(n.state, O::Exited | O::NotStarted));
            value.phase = if uncertain {
                execution::Phase::Unknown
            } else if terminal {
                if input.nodes.values().any(|n| n.state == O::Exited) {
                    execution::Phase::Exited
                } else {
                    execution::Phase::NotStarted
                }
            } else if value.stop_requested {
                execution::Phase::StopRequested
            } else if input.nodes.values().any(|n| n.state == O::Running) {
                execution::Phase::Running
            } else {
                execution::Phase::Granted
            };
            if terminal && !uncertain {
                release(tx, &value)?;
            }
            let receipt = execution::ObservationReceipt {
                observer: peer,
                observation: input,
                recorded_at: clock.now(),
            };
            value.observed = Some(receipt.clone());
            record(tx, &value.intent.id, Some(revision), &value)?;
            remember(tx, &scope, fingerprint, OBSERVATION, &receipt)?;
            Ok(receipt)
        })
    }
}
