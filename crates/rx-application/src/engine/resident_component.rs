use super::*;
use crate::resident_component::{self as component, Record, View};
use rx_domain::component::{Declaration, Registration, RegistrationState};

pub(super) const COMPONENT: &str = "rx.internal.resident-component.v1";
pub(super) const SNAPSHOT: &str = "rx.resident-component-view.v1";

pub(super) fn author(
    tx: &mut dyn Transaction,
    identity: &Identity,
    meta: &Installation,
    now: &TimePoint,
) -> Result<Principal> {
    let principal = authorize_identity(tx, identity, meta, now)?;
    if !principal.roles.contains(&Role::Engineer) && !principal.roles.contains(&Role::AccountAdmin)
    {
        return reject(Reject::Forbidden);
    }
    Ok(principal)
}

pub(super) fn owned(
    tx: &mut dyn Transaction,
    principal: &Principal,
    meta: &Installation,
    id: &Id,
) -> Result<(Counter, Record)> {
    let (revision, record): (_, Record) = load(tx, "component", id, COMPONENT)?;
    if record.installation != meta.id || record.registration.id != *id {
        return Err(StoreError::Integrity("component identity differs".into()));
    }
    if record.owner != principal.id && !principal.roles.contains(&Role::AccountAdmin) {
        return reject(Reject::Forbidden);
    }
    super::component_intake::require_accepted(tx, id)?;
    Ok((revision, record))
}

fn persist(tx: &mut dyn Transaction, expected: Option<Counter>, record: Record) -> Result<View> {
    let revision = save(
        tx,
        "component",
        &record.registration.id,
        expected,
        COMPONENT,
        &record,
    )?;
    let view = View::new(revision, record);
    // Keep every accepted declaration cut independently of later edits or retirement.
    save(
        tx,
        "componenthistory",
        (&view.record.registration.id, revision),
        None,
        SNAPSHOT,
        &view,
    )?;
    event(tx, "rx.event.resident-component-recorded.v1", &view)?;
    Ok(view)
}

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn create_component(
        &mut self,
        identity: &Identity,
        request_key: &str,
        input: component::Create,
    ) -> Result<View> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let principal = author(tx, identity, meta, &now)?;
            let (scope, fingerprint) = request(
                meta,
                &principal,
                "Component.Create",
                request_key,
                &(&principal.id, &input),
            )?;
            if let Some(view) = prior(tx, &scope, fingerprint, SNAPSHOT)? {
                return Ok(view);
            }
            lifecycle::require_serving(tx)?;
            let view = persist(
                tx,
                None,
                Record {
                    installation: meta.id.clone(),
                    registration: Registration {
                        id: id(),
                        declaration: input.declaration,
                        state: RegistrationState::Accepted,
                    },
                    owner: principal.id.clone(),
                    created_at: now.clone(),
                    changed_at: now,
                    changed_by: principal.id.clone(),
                },
            )?;
            remember(tx, &scope, fingerprint, SNAPSHOT, &view)?;
            Ok(view)
        })
    }

    pub fn update_component(
        &mut self,
        identity: &Identity,
        request_key: &str,
        input: component::Update,
    ) -> Result<View> {
        self.modify_component(
            identity,
            request_key,
            &input.id,
            input.expected_revision,
            Some(input.declaration),
        )
    }

    pub fn retire_component(
        &mut self,
        identity: &Identity,
        request_key: &str,
        input: component::Retire,
    ) -> Result<View> {
        self.modify_component(
            identity,
            request_key,
            &input.id,
            input.expected_revision,
            None,
        )
    }

    fn modify_component(
        &mut self,
        identity: &Identity,
        request_key: &str,
        id: &Id,
        expected: Counter,
        declaration: Option<Declaration>,
    ) -> Result<View> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let principal = author(tx, identity, meta, &now)?;
            let (revision, mut record) = owned(tx, &principal, meta, id)?;
            let method = if declaration.is_some() {
                "Component.Update"
            } else {
                "Component.Retire"
            };
            let (scope, fingerprint) = request(
                meta,
                &principal,
                method,
                request_key,
                &(&principal.id, id, expected, &declaration),
            )?;
            if let Some(view) = prior(tx, &scope, fingerprint, SNAPSHOT)? {
                return Ok(view);
            }
            lifecycle::require_serving(tx)?;
            super::resident_execution::require_unassigned(tx, id)?;
            check_revision(revision, expected)?;
            if record.registration.state == RegistrationState::Retired {
                return reject(Reject::InvalidInput);
            }
            if let Some(declaration) = declaration {
                record.registration.declaration = declaration;
            } else {
                record.registration.state = RegistrationState::Retired;
            }
            record.changed_at = now;
            record.changed_by = principal.id;
            let view = persist(tx, Some(revision), record)?;
            remember(tx, &scope, fingerprint, SNAPSHOT, &view)?;
            Ok(view)
        })
    }

    pub fn component(
        &mut self,
        identity: &Identity,
        id: &Id,
        revision: Option<Counter>,
    ) -> Result<View> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let principal = author(tx, identity, meta, &clock.now())?;
            let (current, record) = owned(tx, &principal, meta, id)?;
            if let Some(revision) = revision {
                revision.nonzero("revision").map_err(domain_error)?;
                let (_, view): (_, View) = load(tx, "componenthistory", (id, revision), SNAPSHOT)?;
                if view.revision != revision
                    || view.record.registration.id != *id
                    || view.record.installation != meta.id
                    || view.record.owner != record.owner
                {
                    return Err(StoreError::Integrity("component history differs".into()));
                }
                Ok(view)
            } else {
                Ok(View::new(current, record))
            }
        })
    }
}
