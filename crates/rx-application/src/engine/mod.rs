use crate::{model::*, persistence::*};
use rx_domain::{
    budget::{BudgetUnit, Consumption, RunBudget},
    canonical,
    condition::{Context, Fact, FactValue, Verdict},
    fault::Rejection as Reject,
    operation::{Integrity, Operation, Outcome},
    types::*,
};
use rx_ports::{Repository, RequestScope, Result, SavedRequest, StoreError, Transaction};
use serde::{Serialize, de::DeserializeOwned};
use std::collections::{BTreeMap, BTreeSet};

pub struct Engine<R, C, A> {
    pub(crate) repository: crate::control_journal::Journaled<R>,
    pub(crate) clock: C,
    pub(crate) authority: A,
    pub installation: Installation,
    configuration_reads: BTreeMap<Id, (TimePoint, Digest)>,
    qualification_reads: BTreeMap<Id, (TimePoint, Digest)>,
}
pub(super) struct ProcessingContext<'a> {
    pub identity: &'a Identity,
    pub meta: &'a Installation,
    pub now: &'a TimePoint,
}
const PRINCIPAL: &str = "rx.internal.principal.v1";
const SESSION: &str = "rx.internal.session.v1";
const TERMINAL: &str = "rx.internal.terminal.v1";
const CELL: &str = "rx.internal.cell.v1";
const RUN: &str = "rx.internal.run.v1";
const ATTEMPT: &str = "rx.internal.start-attempt.v1";
const MANDATE: &str = "rx.internal.mandate.v1";
const HOST: &str = "rx.internal.host-registration.v1";
const PART: &str = "rx.internal.part-attempt.v1";
const ACTIVATION: &str = "rx.internal.activation.v1";
const WORK: &str = "rx.internal.work.v1";
const PERMIT: &str = "rx.internal.permit.v1";
const RESOURCE: &str = "rx.internal.resource.v1";
const FACT: &str = "rx.internal.fact.v1";
const DELIVERY: &str = "rx.internal.delivery.v1";

mod access;
mod admission;
mod checkpoint_change;
mod closure;
mod configuration;
mod configuration_dispatch;
mod delivery;
mod device_binding;
mod device_review;
mod diagnostics;
mod dispatch;
mod draft_bindings;
mod evidence;
mod execution_read;
mod executor_peer;
mod executor_requests;
mod handover;
mod host_link;
mod identity;
mod intervention;
mod invalidation;
mod lifecycle;
mod observation;
mod operator_peer;
mod package_intake;
mod pause;
mod procedure;
mod process;
mod process_apply;
mod process_change;
mod process_draft;
mod process_review;
mod process_transition;
mod producer;
mod production;
mod qualification_activation;
mod queries;
mod reconciliation;
mod requalification;
mod requests;
mod run_configuration;
mod runtime_restrictions;
mod workflow;

use access::*;
use admission::*;
use invalidation::*;
use requests::*;

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// Local Runtime bootstrap only; this is not a public RPC.
    pub fn open(
        repository: R,
        clock: C,
        authority: A,
        installation_id: Id,
        bootstrap: Principal,
    ) -> Result<Self> {
        let mut repository = crate::control_journal::Journaled::new(repository);
        let now = clock.now();
        let runtime_boot = id();
        let installation = repository.transact(|tx| {
            let meta = tx.get(&name("installation/current"))?;
            let installation = if let Some(record) = meta {
                let mut old: Installation = decode(&record, "rx.internal.installation.v1")?;
                if old.id != installation_id {
                    return reject(Reject::InvalidInput);
                }
                let previous_installation = old.clone();
                old.runtime_boot = runtime_boot.clone();
                old.clock_id = now.clock_id.clone();
                tx.put(
                    &name("installation/current"),
                    Some(record.revision),
                    &doc("rx.internal.installation.v1", &old)?,
                )?;
                // Keep historical records, but never restore live execution authority on boot.
                for record in tx.scan("cell/")? {
                    let before: Cell = decode(&record, CELL)?;
                    let mut cell = before.clone();
                    let qualification_change = qualification_activation::cell_change(tx, &cell)?;
                    let before_blocks = cell.blocks.iter().map(|b| b.id.clone()).collect();
                    if let Some(q) = &cell.qualification
                        && (qualification_change.is_some()
                            || !authority.verify(&cell.configuration, &q.evidence, &q.dependencies))
                    {
                        save(
                            tx,
                            "qualificationhistory",
                            &q.id,
                            None,
                            "rx.internal.qualification-history.v1",
                            q,
                        )?;
                        cell.qualification = None;
                    }
                    invalidate_cell(tx, &mut cell, record.revision, BlockReason::RuntimeRestart)?;
                    crate::runtime_invalidation::record_restart(
                        tx,
                        &previous_installation,
                        &old,
                        record.revision,
                        &before,
                        &cell,
                    )?;
                    if let Some(change) = qualification_change {
                        qualification_activation::record_blocks(
                            tx,
                            &change,
                            &cell,
                            &before_blocks,
                        )?;
                    }
                }
                qualification_activation::suspend_changed_roots(tx, &old)?;
                old
            } else {
                if !bootstrap.roles.contains(&Role::AccountAdmin) || !bootstrap.active {
                    return reject(Reject::InvalidInput);
                }
                let value = Installation {
                    id: installation_id.clone(),
                    store_generation: id(),
                    runtime_boot: runtime_boot.clone(),
                    clock_id: now.clock_id.clone(),
                };
                tx.put(
                    &name("installation/current"),
                    None,
                    &doc("rx.internal.installation.v1", &value)?,
                )?;
                save(tx, "principal", &bootstrap.id, None, PRINCIPAL, &bootstrap)?;
                value
            };
            lifecycle::boot(tx, &installation)?;
            Ok(installation)
        })?;
        Ok(Self {
            repository,
            clock,
            authority,
            installation,
            configuration_reads: BTreeMap::new(),
            qualification_reads: BTreeMap::new(),
        })
    }
    pub fn into_repository(self) -> R {
        self.repository.into_inner()
    }
}

fn domain_error(e: rx_domain::DomainError) -> StoreError {
    StoreError::Invalid(e.to_string())
}
/// Stable identity of the existing operation's authorization delivery, not a new operation.
pub fn authorization_delivery_id(operation: &Id) -> Id {
    delivery::key_id(operation, "authorize")
}

fn check_revision(actual: Counter, expected: Counter) -> Result<()> {
    if actual != expected {
        return reject(Reject::StaleRevision);
    }
    Ok(())
}

fn missing_as(error: StoreError, fallback: Reject) -> StoreError {
    match error {
        StoreError::Rejected(Reject::NotFound) => StoreError::Rejected(fallback),
        other => other,
    }
}
