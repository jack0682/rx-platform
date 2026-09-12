use super::*;
use crate::intervention::*;
const CASE: &str = "rx.internal.intervention-case.v1";
const SNAPSHOT: &str = "rx.internal.case-snapshot.v1";
const ACK: &str = "rx.internal.case-acknowledgment.v1";
const DETAIL: &str = "rx.internal.case-detail.v1";
pub(super) fn reference(cell: &Cell) -> CaseRef {
    CaseRef {
        cell: cell.configuration.id.clone(),
        definition: cell.configuration.definition.sha256,
        cell_epoch: cell.epoch,
        scope_epochs: cell.scope_epochs.clone(),
    }
}
fn case_actor(
    tx: &mut dyn Transaction,
    identity: &Identity,
    meta: &Installation,
    now: &TimePoint,
    cell: &Name,
) -> Result<Principal> {
    let actor = authorize_read(tx, identity, meta, now, cell)?;
    if !actor.roles.iter().any(|r| {
        matches!(
            r,
            Role::Operator | Role::RecoveryLead | Role::Executor | Role::Host
        )
    }) {
        return reject(Reject::Forbidden);
    }
    Ok(actor)
}
fn unique<T: Ord>(items: &[T]) -> bool {
    items.iter().collect::<BTreeSet<_>>().len() == items.len()
}
pub(super) fn snapshot(tx: &mut dyn Transaction, case_id: &Id) -> Result<CaseSnapshot> {
    let (revision, case): (_, Case) = load(tx, "case", case_id, CASE)?;
    let (_, cell): (_, Cell) = load(tx, "cell", &case.cell, CELL)?;
    let mut acknowledgments = 0;
    let mut records = 0;
    for id in &case.record_ids {
        if let Some(row) = tx.get(&key("procedurerecord", id))? {
            let record: crate::procedure::StoredRecord =
                decode(&row, "rx.internal.procedure-record.v1")?;
            if record.record.action == crate::procedure::Action::Acknowledge {
                acknowledgments += 1;
            } else {
                records += 1;
            }
        } else {
            acknowledgments += 1;
        }
    }
    Ok(CaseSnapshot {
        acknowledgment_count: Counter(acknowledgments),
        procedure_record_count: Counter(records),
        revision,
        case,
        current: reference(&cell),
    })
}
pub(super) fn detail(tx: &mut dyn Transaction, case_id: &Id) -> Result<CaseDetail> {
    let snapshot = snapshot(tx, case_id)?;
    let mut acknowledgments = vec![];
    let mut procedure_records = vec![];
    for id in &snapshot.case.record_ids {
        if let Some(row) = tx.get(&key("procedurerecord", id))? {
            procedure_records.push(decode::<crate::procedure::StoredRecord>(
                &row,
                "rx.internal.procedure-record.v1",
            )?);
        } else {
            let (_, ack): (_, Acknowledgment) = load(tx, "caseack", id, ACK)?;
            acknowledgments.push(ack);
        }
    }
    let procedure_progress = tx
        .get(&key("procedureprogress", case_id))?
        .map(|r| decode::<crate::procedure::Progress>(&r, "rx.internal.procedure-progress.v1"))
        .transpose()?
        .unwrap_or_default();
    Ok(CaseDetail {
        procedure_progress,
        snapshot,
        acknowledgments,
        procedure_records,
    })
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn open_case(
        &mut self,
        identity: &Identity,
        key_: &str,
        command: OpenCase,
    ) -> Result<CaseSnapshot> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let actor = case_actor(tx, identity, meta, &now, &command.cell)?;
            let (scope, fingerprint) = request(meta, &actor, "Cell.OpenCase", key_, &command)?;
            if let Some(saved) = prior(tx, &scope, fingerprint, SNAPSHOT)? {
                return Ok(saved);
            }
            if command.expected_cell == Some(Counter(0))
                || command.procedure.size_bytes.0 == 0
                || command.scopes.len() > 128
                || command.operation_ids.len() > 128
                || command.material_ids.len() > 128
                || !unique(&command.scopes)
                || !unique(&command.operation_ids)
                || !unique(&command.material_ids)
            {
                return reject(Reject::InvalidInput);
            }
            let affected = affected_cells(tx, &command.cell)?;
            let (_, lead): (_, Principal) = load(tx, "principal", &command.lead, PRINCIPAL)?;
            if !lead.active
                || !crate::procedure::can_report(&lead.roles)
                || !lead.roles.contains(&Role::RecoveryLead)
                || !affected.iter().all(|c| lead.cells.contains(c))
            {
                return reject(Reject::Forbidden);
            }
            for operation in &command.operation_ids {
                let (_, work): (_, Work) = load(tx, "work", operation, WORK)?;
                if !affected.contains(&work.cell) {
                    return reject(Reject::InvalidInput);
                }
            }
            let (_, initial): (_, Cell) = load(tx, "cell", &command.cell, CELL)?;
            let scope_uncertain = !command.material_ids.is_empty()
                || command
                    .scopes
                    .iter()
                    .any(|s| !initial.configuration.scopes.contains(s));
            let case_id = id();
            let mut block_ids = vec![];
            let old_blocks = affected
                .iter()
                .map(|cell| {
                    let (_, cell): (_, Cell) = load(tx, "cell", cell, CELL)?;
                    Ok(cell
                        .blocks
                        .into_iter()
                        .map(|b| b.id)
                        .collect::<BTreeSet<_>>())
                })
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<BTreeSet<_>>();
            if command.kind != CaseType::DiagnosticOnly {
                invalidate_closure(tx, &command.cell, BlockReason::CaseIntervention)?;
            }
            for cell_id in &affected {
                let (revision, mut cell): (_, Cell) = load(tx, "cell", cell_id, CELL)?;
                if command.kind == CaseType::DiagnosticOnly {
                    cell.blocks.push(Block {
                        id: id(),
                        created_revision: Some(revision.increment().map_err(domain_error)?),
                        case_id: Some(case_id.clone()),
                        reason: BlockReason::CaseDiagnostic,
                        latched: false,
                        scopes: cell.configuration.scopes.clone(),
                    });
                }
                if command.kind != CaseType::DiagnosticOnly {
                    cell.mode = Some(if command.kind == CaseType::Maintenance {
                        OperatingMode::Maintenance
                    } else {
                        OperatingMode::Recovery
                    });
                }
                for block in &mut cell.blocks {
                    if !old_blocks.contains(&block.id) {
                        block.case_id = Some(case_id.clone());
                    }
                }
                block_ids.extend(
                    cell.blocks
                        .iter()
                        .filter(|b| !old_blocks.contains(&b.id))
                        .map(|b| b.id.clone()),
                );
                cell.open_cases.push(case_id.clone());
                save(tx, "cell", cell_id, Some(revision), CELL, &cell)?;
            }
            block_ids.sort();
            block_ids.dedup();
            let (_, current): (_, Cell) = load(tx, "cell", &command.cell, CELL)?;
            let mut related_runs = vec![];
            for row in tx.scan("run/")? {
                let run: Run = decode(&row, RUN)?;
                if affected.contains(&run.cell) {
                    related_runs.push(run.id);
                }
            }
            related_runs.sort();
            let case = Case {
                id: case_id.clone(),
                cell: command.cell,
                kind: command.kind,
                state: if command.kind == CaseType::DiagnosticOnly {
                    CaseState::Open
                } else {
                    CaseState::ContainmentPending
                },
                procedure: command.procedure,
                lead: command.lead,
                participants: vec![],
                operation_ids: command.operation_ids,
                material_ids: command.material_ids,
                record_ids: vec![],
                block_ids,
                requested_scopes: command.scopes,
                effective_cells: affected.into_iter().collect(),
                scope_uncertain,
                opened_by: actor.id,
                opened_at: now,
                reference: reference(&current),
                related_runs,
            };
            save(tx, "case", &case_id, None, CASE, &case)?;
            event(tx, "rx.event.case-opened.v1", &case)?;
            let result = snapshot(tx, &case_id)?;
            remember(tx, &scope, fingerprint, SNAPSHOT, &result)?;
            Ok(result)
        })
    }
    pub fn inspect_case(
        &mut self,
        identity: &Identity,
        cell: &Name,
        case: &Id,
    ) -> Result<CaseDetail> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            authorize_read(tx, identity, meta, &clock.now(), cell)?;
            let value = detail(tx, case)?;
            if !value.snapshot.case.effective_cells.contains(cell) {
                return reject(Reject::Forbidden);
            }
            Ok(value)
        })
    }
    pub fn cases(&mut self, identity: &Identity, cell: &Name) -> Result<CaseList> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            authorize_read(tx, identity, meta, &clock.now(), cell)?;
            let mut cases = vec![];
            for row in tx.scan("case/")? {
                let case: Case = decode(&row, CASE)?;
                if case.effective_cells.contains(cell) {
                    cases.push(snapshot(tx, &case.id)?);
                }
            }
            cases.sort_by(|a, b| b.case.id.cmp(&a.case.id));
            let truncated = cases.len() > 100;
            cases.truncate(100);
            Ok(CaseList {
                cell: cell.clone(),
                cases,
                truncated,
            })
        })
    }
    /// Notification acknowledgment only; never transitions procedure, access or restart state.
    pub fn acknowledge_case(
        &mut self,
        identity: &Identity,
        key_: &str,
        command: AcknowledgeCase,
    ) -> Result<CaseDetail> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx|{
            let now=clock.now();let actor=case_actor(tx,identity,meta,&now,&command.cell)?;
            let (scope,fingerprint)=request(meta,&actor,"Case.Acknowledge",key_,&command)?;
            if let Some(saved)=prior(tx,&scope,fingerprint,DETAIL)?{return Ok(saved);}
            if command.occurred_at.len()>128{return reject(Reject::InvalidInput);}
            let parsed=time::OffsetDateTime::parse(&command.occurred_at,&time::format_description::well_known::Rfc3339).map_err(|_|StoreError::Rejected(Reject::InvalidInput))?;
            if !parsed.offset().is_utc()||command.occurred_at.ends_with("-00:00"){return reject(Reject::InvalidInput);}
            let (revision,mut case):(_,Case)=load(tx,"case",&command.case,CASE)?;
            if !case.effective_cells.contains(&command.cell){return reject(Reject::Forbidden);}check_revision(revision,command.expected_case)?;
            let record=id();
            let assertions=serde_json::json!({"schema":"rx.procedure-assertions.v1","case_id":case.id,"case_revision":revision,"procedure_digest":case.procedure.sha256,"actor":actor.id,"action":"ACKNOWLEDGE","occurred_at":command.occurred_at,"scope_ids":case.reference.scope_epochs.keys().collect::<Vec<_>>(),"source":"authenticated-notification-ack","physical_claims":[]});
            use sha2::Digest as _;
            let bytes=canonical::bytes(&assertions).map_err(domain_error)?;
            let reference=ArtifactRef {sha256:Digest::from_bytes(sha2::Sha256::digest(&bytes).into()),schema_id:name("rx.procedure-assertions.v1"),size_bytes:Counter(bytes.len() as u64)};
            let ack=Acknowledgment {id:record.clone(),case:case.id.clone(),case_revision:revision,actor:actor.id,occurred_at:command.occurred_at,recorded_at:now,scopes:case.reference.scope_epochs.keys().cloned().collect(),assertions:reference.clone()};
            let artifact_key=key("procedureassertions",reference.sha256);let document=doc("rx.procedure-assertions.v1",&assertions)?;if let Some(old)=tx.get(&artifact_key)?{if old.document!=document{return Err(StoreError::Integrity("ack assertion artifact conflict".into()));}}else{tx.put(&artifact_key,None,&document)?;}
            save(tx,"caseack",&record,None,ACK,&ack)?;case.record_ids.push(record);save(tx,"case",&case.id,Some(revision),CASE,&case)?;
            event(tx,"rx.event.case-acknowledged.v1",&ack)?;let result=detail(tx,&case.id)?;remember(tx,&scope,fingerprint,DETAIL,&result)?;Ok(result)
        })
    }
}
