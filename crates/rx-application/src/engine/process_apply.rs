use super::*;
use crate::{configuration_dispatch::Phase, process_change::*};
use rx_domain::host_configuration as wire;
const CHANGE: &str = "rx.process-change.v1";
const SELECTION: &str = "rx.active-configuration-selection.v1";
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    change: Id,
    cell: Name,
    configuration: ArtifactRef,
    epoch: Counter,
}
fn proofs(
    tx: &mut dyn Transaction,
    meta: &Installation,
    c: &Change,
    now: &TimePoint,
    reads: &BTreeMap<Id, (TimePoint, Digest)>,
) -> Result<Vec<HostProof>> {
    process_change::current(tx, meta, c)?;
    configuration_dispatch::barrier(tx, c)?;
    let prep = c
        .preparation
        .as_ref()
        .ok_or(StoreError::Rejected(Reject::HostNotPrepared))?;
    let hosts: BTreeSet<_> = c
        .impact
        .cells
        .iter()
        .flat_map(|c| c.hosts.iter().cloned())
        .collect();
    if hosts.is_empty() {
        return reject(Reject::HostNotPrepared);
    }
    let mut proofs = Vec::new();
    for host in hosts {
        let row = tx
            .get(&configuration_dispatch::key_for(&c.id, prep.attempt, &host))?
            .ok_or(StoreError::Rejected(Reject::HostNotPrepared))?;
        let id: Id = decode(&row, "rx.internal.host-config-task-ref.v1")?;
        let (_, t) = configuration_dispatch::read(tx, &id)?;
        configuration_dispatch::current_change(tx, meta, &t)?;
        if t.phase != Phase::SendEntered
            || t.integrity_disputed
            || t.issue.is_some()
            || !configuration_dispatch::generation_matches(tx, &t)?
        {
            return reject(Reject::ContinuityUnproven);
        }
        let receipt = t
            .receipt
            .as_ref()
            .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
        let observation = t
            .observation
            .as_ref()
            .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
        observation.validate().map_err(StoreError::Integrity)?;
        if receipt.status != wire::Status::AppliedUnqualified
            || !observation.context_matches_current_host
            || observation.snapshot.cells.len() != t.cells.len()
        {
            return reject(Reject::HostNotPrepared);
        }
        for target in &t.cells {
            if !observation.snapshot.cells.iter().any(|v| {
                v.cell == target.cell
                    && v.definition == target.definition
                    && v.envelope == target.envelope
                    && v.environment == target.environment
                    && target.change_blocks.iter().all(|b| v.blocked.contains(b))
            }) {
                return reject(Reject::ContinuityUnproven);
            }
        }
        let digest = canonical::digest("RX-HOST-CONFIGURATION-OBSERVATION-v1", observation)
            .map_err(domain_error)?;
        let (at, known) = reads
            .get(&id)
            .ok_or(StoreError::Rejected(Reject::Expired))?;
        if *known != digest || now.age_ns(at).is_none_or(|age| age > 3_000_000_000) {
            return reject(Reject::Expired);
        }
        proofs.push(HostProof {
            host,
            task: id,
            request_digest: receipt.request_digest,
            receipt_digest: canonical::digest("RX-HOST-CONFIGURATION-RECEIPT-v1", receipt)
                .map_err(domain_error)?,
            read_started: at.clone(),
            observation_digest: digest,
        });
    }
    Ok(proofs)
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn prepare_process_change_apply(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: Transition,
    ) -> Result<Preflight> {
        let meta = &self.installation;
        let clock = &self.clock;
        let reads = &self.configuration_reads;
        self.repository.transact(|tx| {
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let principal = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&input.cell),
                Role::ReleaseManager,
                true,
            )?;
            let c = process_change::change(tx, &input.change, &input.cell)?;
            process_change::access(&principal, &c.impact)?;
            let (scope, fp) = request(
                meta,
                &principal,
                "ProcessChange.Apply",
                key_.as_str(),
                &input,
            )?;
            if let Some(c) = prior(tx, &scope, fp, CHANGE)? {
                return Ok(Preflight::Recorded(Box::new(c)));
            }
            if c.state != State::Staged
                || c.revision != input.expected
                || c.plan_digest != input.plan_digest
            {
                return reject(Reject::StaleRevision);
            }
            proofs(tx, meta, &c, &now, reads)?;
            let (job, version, decision, resolved) =
                process_change::review(tx, meta, &input.cell, &c.review)?;
            let registration = package_intake::current(tx, meta)?.ok_or(
                StoreError::Unavailable("package verifier unavailable".into()),
            )?;
            Ok(Preflight::Verify(Box::new(Ticket {
                mode: c.mode,
                action: Action::Apply(input),
                identity: identity.clone(),
                key: key_.clone(),
                job,
                version,
                decision,
                registration,
                impact: c.impact,
                resolved,
                issued: now,
                boot: meta.runtime_boot.clone(),
            })))
        })
    }
    pub fn commit_process_change_apply(&mut self, p: Prepared) -> Result<Change> {
        let meta = &self.installation;
        let clock = &self.clock;
        let reads = &self.configuration_reads;
        self.repository.transact(|tx| {
            let t = &p.ticket;
            let Action::Apply(input) = &t.action else {
                return reject(Reject::InvalidInput);
            };
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let principal = authorize(
                tx,
                &t.identity,
                meta,
                &now,
                Some(&input.cell),
                Role::ReleaseManager,
                true,
            )?;
            let mut c = process_change::change(tx, &input.change, &input.cell)?;
            process_change::access(&principal, &c.impact)?;
            let (scope, fp) = request(
                meta,
                &principal,
                "ProcessChange.Apply",
                t.key.as_str(),
                input,
            )?;
            if let Some(c) = prior(tx, &scope, fp, CHANGE)? {
                return Ok(c);
            }
            if c.state != State::Staged
                || c.revision != input.expected
                || c.plan_digest != input.plan_digest
            {
                return reject(Reject::StaleRevision);
            }
            process_change::check_prepared(tx, meta, &now, &p)?;
            if process_change::config_ref(&p.target)? != c.after || p.origins != c.step_origins {
                return Err(StoreError::Integrity(
                    "Applied target differs from reviewed plan".into(),
                ));
            }
            let host_proofs = proofs(tx, meta, &c, &now, reads)?;
            // Freeze legacy Run references before publishing any changed selection.
            let affected: BTreeSet<_> = c.impact.cells.iter().map(|c| c.id.clone()).collect();
            for row in tx.scan("run/")? {
                let run: Run = decode(&row, RUN)?;
                if affected.contains(&run.cell) {
                    let (_, cell): (_, Cell) = load(tx, "cell", &run.cell, CELL)?;
                    run_configuration::bind(tx, &run, &cell.configuration)?;
                }
            }
            let mut cells = Vec::new();
            let mut fences = Vec::new();
            for affected in &c.impact.cells {
                let (revision, mut cell): (_, Cell) = load(tx, "cell", &affected.id, CELL)?;
                let before = process_change::store_config(tx, &cell.configuration)?;
                let previous_qualification = cell.qualification.take();
                if let Some(q) = &previous_qualification {
                    let k = key("qualificationhistory", &q.id);
                    let d = doc("rx.internal.qualification-history.v1", q)?;
                    if let Some(old) = tx.get(&k)? {
                        if old.document != d {
                            return Err(StoreError::Integrity(
                                "Qualification history differs".into(),
                            ));
                        }
                    } else {
                        tx.put(&k, None, &d)?;
                    }
                }
                if affected.id == c.cell {
                    cell.configuration = p.target.clone();
                }
                let after = process_change::store_config(tx, &cell.configuration)?;
                cell.mode = Some(OperatingMode::Setup);
                if cell.commissioning != Some(Commissioning::NotCommissioned) {
                    cell.commissioning = Some(Commissioning::RevalidationRequired);
                }
                let old_blocks = cell.blocks.iter().map(|b| b.id.clone()).collect();
                for (host, message) in invalidate_for_change(tx, &mut cell, revision)? {
                    fences.push(FenceTarget {
                        cell: affected.id.clone(),
                        host,
                        message,
                        epoch: cell.epoch,
                        scopes: cell.scope_epochs.clone(),
                    });
                }
                qualification_activation::record_blocks(tx, &c.id, &cell, &old_blocks)?;
                let k = key("configurationselection", &affected.id);
                let old = tx.get(&k)?;
                let selection = Selection {
                    change: c.id.clone(),
                    cell: affected.id.clone(),
                    configuration: after.clone(),
                    epoch: cell.epoch,
                };
                tx.put(&k, old.map(|r| r.revision), &doc(SELECTION, &selection)?)?;
                save(
                    tx,
                    "configurationselectionhistory",
                    (&c.id, &affected.id),
                    None,
                    SELECTION,
                    &selection,
                )?;
                cells.push(AppliedCell {
                    cell: affected.id.clone(),
                    before,
                    after,
                    previous_qualification,
                    epoch: cell.epoch,
                    scopes: cell.scope_epochs,
                });
            }
            let application = ApplicationRecord {
                preparation: c.preparation.as_ref().unwrap().attempt,
                runtime_boot: meta.runtime_boot.clone(),
                actor: principal.id,
                terminal: t.identity.terminal.as_ref().unwrap().0.clone(),
                applied_at: now,
                cells,
                host_proofs,
                fences,
            };
            let expected = c.revision;
            c.revision = c.revision.increment().map_err(domain_error)?;
            c.state = State::AppliedUnqualified;
            c.application = Some(application);
            process_change::record(tx, &c, Some(expected))?;
            remember(tx, &scope, fp, CHANGE, &c)?;
            Ok(c)
        })
    }
}
pub(super) fn detail(
    tx: &mut dyn Transaction,
    meta: &Installation,
    c: Change,
    before: CellConfiguration,
    after: CellConfiguration,
) -> Result<Detail> {
    let a = c.application.as_ref().ok_or(StoreError::Integrity(
        "Applied change record missing".into(),
    ))?;
    let mut blockers = if c.state == State::QualifiedActive {
        vec![]
    } else {
        vec![Blocker::RequalificationRequired]
    };
    let mut current = true;
    for applied in &a.cells {
        let (_, cell): (_, Cell) = load(tx, "cell", &applied.cell, CELL)?;
        current &= process_change::config_ref(&cell.configuration)? == applied.after;
    }
    if !current {
        blockers.push(Blocker::ContextChanged);
    }
    let mut summary = configuration_dispatch::summary(tx, meta, &c)?;
    summary.platform_configuration_applied = true;
    // Old preparation proofs remain historical. New epoch needs a fresh fence/qualification path.
    summary.revalidation_required_before_platform_apply = false;
    Ok(Detail {
        host_configuration: summary,
        change: c,
        before,
        after,
        blocker_count: Counter(blockers.len() as u64),
        blockers,
        blockers_truncated: false,
        applied: true,
        activation_authorized: false,
    })
}
