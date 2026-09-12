//! Volatile, writer-owned state; restarting the platform starts with no health claims.
use rx_application::{projection::Overview, service_health::*};
use rx_domain::{fault::Rejection, types::*};
use rx_ports::{Result, StoreError};
use std::collections::{BTreeMap, BTreeSet};
const AGE: u64 = 3_000_000_000;
#[derive(Default)]
pub(crate) struct Registry {
    boot: Option<Id>,
    entries: BTreeMap<Target, Entry>,
}
struct Entry {
    owner: Owner,
    sequence: Counter,
    sample: Option<(TimePoint, Report)>,
}
impl Registry {
    pub fn configured(&self) -> bool {
        self.boot.is_some()
    }
    pub fn initialize(&mut self, boot: Id, owners: Vec<Owner>) -> Result<()> {
        if self.configured() {
            return Err(StoreError::Rejected(Rejection::Busy));
        }
        if owners.len() > 64
            || owners
                .iter()
                .map(|o| &o.target)
                .collect::<BTreeSet<_>>()
                .len()
                != owners.len()
            || owners.iter().any(|o| o.runtime_boot != boot)
        {
            return Err(StoreError::Rejected(Rejection::InvalidInput));
        }
        self.entries = owners
            .into_iter()
            .map(|owner| {
                (
                    owner.target.clone(),
                    Entry {
                        owner,
                        sequence: Counter(0),
                        sample: None,
                    },
                )
            })
            .collect();
        self.boot = Some(boot);
        Ok(())
    }
    pub fn replace(&mut self, old: &Owner, new: Owner) -> Result<()> {
        let entry = self
            .entries
            .get_mut(&old.target)
            .ok_or(StoreError::Rejected(Rejection::NotFound))?;
        if &entry.owner != old
            || new.target != old.target
            || Some(&new.runtime_boot) != self.boot.as_ref()
            || new.id == old.id
        {
            return Err(StoreError::Rejected(Rejection::ContinuityUnproven));
        }
        *entry = Entry {
            owner: new,
            sequence: Counter(0),
            sample: None,
        };
        Ok(())
    }
    pub fn publish(&mut self, input: Publish, now: TimePoint) -> Result<()> {
        let entry = self
            .entries
            .get_mut(&input.owner.target)
            .ok_or(StoreError::Rejected(Rejection::NotFound))?;
        if input.owner != entry.owner || Some(&input.owner.runtime_boot) != self.boot.as_ref() {
            return Err(StoreError::Rejected(Rejection::ContinuityUnproven));
        }
        if input.sequence <= entry.sequence {
            return Err(StoreError::Rejected(Rejection::StaleRevision));
        }
        for time in [
            &input.report.last_observation_at,
            &input.report.last_delivery_pass_at,
        ]
        .into_iter()
        .flatten()
        {
            if now.age_ns(time).is_none() {
                return Err(StoreError::Rejected(Rejection::InvalidInput));
            }
        }
        entry.sequence = input.sequence;
        entry.sample = Some((now, input.report));
        Ok(())
    }
    pub fn decorate(&self, view: &mut Overview) {
        if self.boot.as_ref() != Some(&view.installation.runtime_boot) {
            return;
        }
        let now = &view.observed_at;
        for cell in &mut view.cells {
            for host in &mut cell.diagnostics.hosts {
                let mut s = Snapshot {
                    schema: "rx.host-service-health.v1",
                    availability: Availability::NotConfigured,
                    sampled_at: None,
                    valid_for_ns: Counter(0),
                    connection: None,
                    observation: None,
                    delivery: None,
                    last_observation_at: None,
                    last_delivery_pass_at: None,
                    observation_recent: false,
                    delivery_recent: false,
                    delivery_error_history: false,
                };
                if let Some(entry) = self.entries.get(&Target {
                    host: host.host.clone(),
                    cell: cell.cell.value.id.clone(),
                }) {
                    if entry.owner.definition != cell.cell.value.definition.sha256
                        || entry.owner.envelope != cell.cell.value.envelope.sha256
                    {
                        s.availability = Availability::ContextMismatch;
                    } else if let Some((received, report)) = &entry.sample {
                        s.sampled_at = Some(received.clone());
                        s.connection = Some(report.connection);
                        s.observation = Some(report.observation);
                        s.delivery = Some(report.delivery);
                        s.last_observation_at = report.last_observation_at.clone();
                        s.last_delivery_pass_at = report.last_delivery_pass_at.clone();
                        s.delivery_error_history = report.delivery_error_history;
                        if let Some(age) = now.age_ns(received).filter(|age| *age < AGE) {
                            s.availability = Availability::Fresh;
                            s.valid_for_ns = Counter(AGE - age);
                            for (time, recent) in [
                                (&s.last_observation_at, &mut s.observation_recent),
                                (&s.last_delivery_pass_at, &mut s.delivery_recent),
                            ] {
                                if let Some(age) = time
                                    .as_ref()
                                    .and_then(|t| now.age_ns(t))
                                    .filter(|age| *age < AGE)
                                {
                                    *recent = true;
                                    s.valid_for_ns = Counter(s.valid_for_ns.0.min(AGE - age));
                                }
                            }
                            cell.diagnostics.display_valid_for_ns = Counter(
                                cell.diagnostics
                                    .display_valid_for_ns
                                    .0
                                    .min(s.valid_for_ns.0),
                            );
                        } else {
                            s.availability = Availability::Stale;
                        }
                    } else {
                        s.availability = Availability::WaitingReport;
                    }
                }
                host.runtime = Some(s);
            }
        }
    }
}
