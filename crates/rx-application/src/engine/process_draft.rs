use super::*;
use crate::process_draft::*;
const META: &str = "rx.internal.process-draft-version.v1";
const CONTENT: &str = "rx.internal.process-draft-document.v1";
const DETAIL: &str = "rx.process-draft-detail.v1";
pub(super) fn read_access(
    tx: &mut dyn Transaction,
    identity: &Identity,
    meta: &Installation,
    now: &TimePoint,
    cell: &Name,
) -> Result<()> {
    let p = authorize_identity(tx, identity, meta, now)?;
    if !p.cells.contains(cell)
        || !(p.roles.contains(&Role::Engineer) || p.roles.contains(&Role::Verifier))
    {
        return reject(Reject::Forbidden);
    }
    Ok(())
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn save_process_draft(
        &mut self,
        identity: &Identity,
        request_key: &Id,
        prepared: PreparedSave,
    ) -> Result<Detail> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let command = &prepared.input;
            let principal = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&command.cell),
                Role::Engineer,
                false,
            )?;
            let _: (_, Cell) = load(tx, "cell", &command.cell, CELL)?;
            let (scope, fingerprint) = request(
                meta,
                &principal,
                "ProcessDraft.Save",
                request_key.as_str(),
                command,
            )?;
            if let Some(saved) = prior(tx, &scope, fingerprint, DETAIL)? {
                return Ok(saved);
            }
            let k = key("processdraft", &command.id);
            let old = tx.get(&k)?;
            let previous = old
                .as_ref()
                .map(|r| decode::<Version>(r, META))
                .transpose()?;
            let revision = match (&old, &previous, command.expected) {
                (None, None, None) => Counter(1),
                (Some(row), Some(previous), Some(expected)) => {
                    if previous.cell != command.cell {
                        return reject(Reject::Forbidden);
                    }
                    check_revision(row.revision, expected)?;
                    row.revision.increment().map_err(domain_error)?
                }
                _ => return reject(Reject::StaleRevision),
            };
            let library = command
                .library
                .clone()
                .or_else(|| previous.as_ref().and_then(|p| p.library.clone()));
            if let Some(previous) = previous
                .as_ref()
                .filter(|p| p.library.as_ref().is_some_and(|l| l.archived))
            {
                let restoring = command.library.as_ref().is_some_and(|l| !l.archived);
                if !restoring {
                    return reject(Reject::Busy);
                }
                // Restore is explicit and cannot smuggle edits into an archived version.
                let before = previous.library.as_ref().expect("archived library");
                let after = library.as_ref().expect("explicit restore library");
                if prepared.digest != previous.document_digest
                    || command.title != previous.title
                    || command
                        .presentation
                        .as_ref()
                        .or(previous.presentation.as_ref())
                        != previous.presentation.as_ref()
                    || before.site != after.site
                    || before.service != after.service
                {
                    return reject(Reject::InvalidInput);
                }
            }
            let document_key = key("processdraftdocument", (&command.cell, prepared.digest));
            let document = doc(CONTENT, &command.document)?;
            if let Some(old) = tx.get(&document_key)? {
                if old.document != document {
                    return Err(StoreError::Integrity(
                        "draft content digest collision".into(),
                    ));
                }
            } else {
                tx.put(&document_key, None, &document)?;
            }
            let version = Version {
                library,
                id: command.id.clone(),
                cell: command.cell.clone(),
                revision,
                title: command.title.clone(),
                document_digest: prepared.digest,
                validation: prepared.report,
                presentation: command
                    .presentation
                    .clone()
                    .or_else(|| previous.as_ref().and_then(|p| p.presentation.clone())),
                created_by: previous.map_or_else(|| principal.id.clone(), |p| p.created_by),
                updated_by: principal.id,
                updated_at: now,
            };
            let saved = tx.put(&k, old.map(|r| r.revision), &doc(META, &version)?)?;
            if saved.revision != revision {
                return Err(StoreError::Integrity("draft revision differs".into()));
            }
            tx.put(
                &key("processdraftrevision", (&version.id, revision)),
                None,
                &doc(META, &version)?,
            )?;
            let index_key = key("processdraftindex", &version.id);
            let prior_index = tx.get(&index_key)?;
            if prior_index.as_ref().map(|r| r.revision) != command.expected {
                return Err(StoreError::Integrity("draft index revision differs".into()));
            }
            tx.put(
                &index_key,
                prior_index.map(|r| r.revision),
                &doc(
                    "rx.internal.process-draft-summary.v1",
                    &Summary::from(&version),
                )?,
            )?;
            event(tx, "rx.event.process-draft-saved.v1", &version)?;
            let detail = Detail {
                version,
                document: command.document.clone(),
            };
            remember(tx, &scope, fingerprint, DETAIL, &detail)?;
            Ok(detail)
        })
    }
    pub fn process_draft(
        &mut self,
        identity: &Identity,
        cell: &Name,
        id: &Id,
        revision: Option<Counter>,
    ) -> Result<Detail> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            read_access(tx, identity, meta, &clock.now(), cell)?;
            let (_, current): (_, Version) = load(tx, "processdraft", id, META)?;
            if current.cell != *cell {
                return reject(Reject::Forbidden);
            }
            let version = if let Some(revision) = revision {
                load::<Version>(tx, "processdraftrevision", (id, revision), META)?.1
            } else {
                current
            };
            let (_, document): (_, serde_json::Value) = load(
                tx,
                "processdraftdocument",
                (cell, version.document_digest),
                CONTENT,
            )?;
            if canonical::digest("RX-PROCESS-DRAFT-DOCUMENT-v1", &document).map_err(domain_error)?
                != version.document_digest
            {
                return Err(StoreError::Integrity(
                    "draft document integrity differs".into(),
                ));
            }
            Ok(Detail { version, document })
        })
    }
    pub fn process_drafts(
        &mut self,
        identity: &Identity,
        cell: &Name,
        after: Option<&Id>,
        filter: &Filter,
    ) -> Result<Page> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            read_access(tx, identity, meta, &clock.now(), cell)?;
            if filter.query.chars().count() > 120 {
                return reject(Reject::InvalidInput);
            }
            Library {
                site: filter.site.clone(),
                service: filter.service.clone(),
                archived: false,
            }
            .validate()
            .map_err(|_| StoreError::Rejected(Reject::InvalidInput))?;
            let query = filter.query.trim().to_lowercase();
            let mut values = tx
                .scan("processdraftindex/")?
                .iter()
                .map(|r| decode::<Summary>(r, "rx.internal.process-draft-summary.v1"))
                .collect::<Result<Vec<_>>>()?;
            values.retain(|v| {
                let library = v.library.clone().unwrap_or_default();
                v.cell == *cell
                    && after.is_none_or(|id| &v.id > id)
                    && v.title.to_lowercase().contains(&query)
                    && filter
                        .site
                        .as_ref()
                        .is_none_or(|site| library.site.as_ref() == Some(site))
                    && filter
                        .service
                        .as_ref()
                        .is_none_or(|service| library.service.as_ref() == Some(service))
                    && filter
                        .archived
                        .is_none_or(|archived| library.archived == archived)
            });
            values.sort_by(|a, b| a.id.cmp(&b.id));
            let more = values.len() > 50;
            values.truncate(50);
            let next = if more {
                values.last().map(|v| v.id.clone())
            } else {
                None
            };
            Ok(Page {
                cell: cell.clone(),
                drafts: values,
                next,
            })
        })
    }
    pub fn process_draft_history(
        &mut self,
        identity: &Identity,
        cell: &Name,
        id: &Id,
        before: Option<Counter>,
    ) -> Result<History> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            read_access(tx, identity, meta, &clock.now(), cell)?;
            let (_, current): (_, Version) = load(tx, "processdraft", id, META)?;
            if current.cell != *cell {
                return reject(Reject::Forbidden);
            }
            if before == Some(Counter(0)) {
                return reject(Reject::InvalidInput);
            }
            let upper = before.map_or(current.revision.0, |b| (b.0 - 1).min(current.revision.0));
            let lower = upper.saturating_sub(49).max(1);
            let mut versions = Vec::new();
            for revision in (lower..=upper).rev() {
                let (_, version): (_, Version) =
                    load(tx, "processdraftrevision", (id, Counter(revision)), META)?;
                if version.id != *id
                    || version.cell != *cell
                    || version.revision != Counter(revision)
                {
                    return Err(StoreError::Integrity(
                        "draft history identity differs".into(),
                    ));
                }
                versions.push(Summary::from(&version));
            }
            Ok(History {
                cell: cell.clone(),
                draft: id.clone(),
                versions,
                next: (lower > 1).then_some(Counter(lower)),
            })
        })
    }
}
