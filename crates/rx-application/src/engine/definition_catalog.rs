use super::*;
use crate::definition_catalog::{
    self as catalog, Access, Catalog, CatalogSave, Filter, Prepared, Version, View,
};
use rx_domain::definition::{self as definition, Definition, Reference};

const CATALOG: &str = "rx.internal.definition-catalog.v1";
const VERSION: &str = "rx.internal.definition-version.v1";
const VIEW: &str = "rx.definition-view.v1";
fn member_prefix(principal: &Name) -> Result<String> {
    Ok(format!(
        "defcatmember/{}/",
        canonical::digest("RX-DEFINITION-CATALOG-MEMBER-v1", principal).map_err(domain_error)?
    ))
}
pub(super) fn author(
    tx: &mut dyn Transaction,
    identity: &Identity,
    meta: &Installation,
    now: &TimePoint,
    write: bool,
) -> Result<Principal> {
    let p = authorize_identity(tx, identity, meta, now)?;
    if !p.roles.contains(&Role::Engineer) && (write || !p.roles.contains(&Role::Verifier)) {
        return reject(Reject::Forbidden);
    }
    Ok(p)
}
fn allowed(c: &Catalog, p: &Principal, identity: &Identity, write: bool) -> bool {
    (c.owner == p.id
        || c.members
            .get(&p.id)
            .is_some_and(|a| !write || *a == Access::Edit))
        && identity
            .terminal
            .as_ref()
            .is_none_or(|(id, _)| c.terminals.contains(id))
}
pub(super) fn access(
    tx: &mut dyn Transaction,
    identity: &Identity,
    p: &Principal,
    id: &Id,
    write: bool,
) -> Result<Catalog> {
    let (revision, c): (_, Catalog) = load(tx, "definitioncatalog", id, CATALOG)?;
    if c.id != *id || c.revision != revision {
        return Err(StoreError::Integrity(
            "catalog identity/revision differs".into(),
        ));
    }
    if !allowed(&c, p, identity, write) {
        return reject(Reject::Forbidden);
    }
    if write && c.archived {
        return reject(Reject::Busy);
    }
    Ok(c)
}
pub(super) fn version(
    tx: &mut dyn Transaction,
    catalog: &Id,
    id: &Id,
    revision: Option<Counter>,
) -> Result<Version> {
    let (row, current): (_, Version) = load(tx, "definition", id, VERSION)?;
    if current.definition.reference.catalog != *catalog {
        return reject(Reject::Forbidden);
    }
    if current.definition.reference.id != *id || current.definition.reference.revision != row {
        return Err(StoreError::Integrity(
            "definition identity/revision differs".into(),
        ));
    }
    let result = if let Some(rev) = revision {
        load::<Version>(tx, "definitionrevision", (id, rev), VERSION)?.1
    } else {
        current
    };
    if result.definition.reference.catalog != *catalog
        || result.definition.reference.id != *id
        || revision.is_some_and(|r| result.definition.reference.revision != r)
    {
        return Err(StoreError::Integrity("definition history differs".into()));
    }
    result.definition.verify().map_err(StoreError::Integrity)?;
    Ok(result)
}
fn dependencies(
    tx: &mut dyn Transaction,
    root: &Definition,
    new_references: bool,
) -> Result<BTreeMap<Reference, Definition>> {
    let mut result = BTreeMap::new();
    let mut pending = root
        .body
        .references()
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    while let Some(reference) = pending.pop() {
        if result.contains_key(&reference) {
            continue;
        }
        if reference.catalog != root.reference.catalog {
            return reject(Reject::Forbidden);
        }
        if result.len() >= 256 || reference.id == root.reference.id {
            return reject(Reject::InvalidInput);
        }
        if new_references && version(tx, &reference.catalog, &reference.id, None)?.archived {
            return reject(Reject::InvalidInput);
        }
        let dependency = version(
            tx,
            &reference.catalog,
            &reference.id,
            Some(reference.revision),
        )?;
        if dependency.definition.reference != reference {
            return reject(Reject::InvalidInput);
        }
        pending.extend(dependency.definition.body.references().into_iter().cloned());
        result.insert(reference, dependency.definition);
    }
    Ok(result)
}
fn view(tx: &mut dyn Transaction, v: Version) -> Result<View> {
    let deps = dependencies(tx, &v.definition, false)?;
    let effective = definition::resolve(&v.definition, &deps).map_err(StoreError::Integrity)?;
    Ok(View {
        version: v,
        effective,
    })
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn save_definition_catalog(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: CatalogSave,
    ) -> Result<Catalog> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let p = author(tx, identity, meta, &clock.now(), true)?;
            lifecycle::require_serving(tx)?;
            if input.title.trim().is_empty()
                || input.title.chars().count() > 120
                || input.members.len() > 128
                || input.terminals.len() > 128
            {
                return reject(Reject::InvalidInput);
            }
            let k = key("definitioncatalog", &input.id);
            let old = tx.get(&k)?;
            let previous = old
                .as_ref()
                .map(|r| decode::<Catalog>(r, CATALOG))
                .transpose()?;
            if let (Some(row), Some(previous)) = (&old, &previous)
                && (previous.id != input.id || previous.revision != row.revision)
            {
                return Err(StoreError::Integrity(
                    "catalog identity/revision differs".into(),
                ));
            }
            if let Some(previous) = &previous
                && (previous.owner != p.id || !allowed(previous, &p, identity, true))
            {
                return reject(Reject::Forbidden);
            }
            if previous.is_none()
                && identity
                    .terminal
                    .as_ref()
                    .is_some_and(|(t, _)| !input.terminals.contains(t))
            {
                return reject(Reject::Forbidden);
            }
            let (scope, fingerprint) = request(
                meta,
                &p,
                "DefinitionCatalog.Save",
                key_.as_str(),
                &(&p.id, &input),
            )?;
            if let Some(saved) = prior(tx, &scope, fingerprint, CATALOG)? {
                return Ok(saved);
            }
            for (member, permission) in &input.members {
                let (_, principal): (_, Principal) = load(tx, "principal", member, PRINCIPAL)?;
                if !principal.active
                    || !principal.roles.contains(&Role::Engineer)
                        && (*permission == Access::Edit
                            || !principal.roles.contains(&Role::Verifier))
                {
                    return reject(Reject::InvalidInput);
                }
            }
            for terminal in &input.terminals {
                let (_, record): (_, Terminal) = load(tx, "terminal", terminal, TERMINAL)?;
                if !record.active {
                    return reject(Reject::InvalidInput);
                }
            }
            let revision = match (old.as_ref(), input.expected) {
                (None, None) => Counter(1),
                (Some(r), Some(expected)) => {
                    check_revision(r.revision, expected)?;
                    r.revision.increment().map_err(domain_error)?
                }
                _ => return reject(Reject::StaleRevision),
            };
            let catalog = Catalog {
                id: input.id.clone(),
                revision,
                title: input.title,
                owner: p.id.clone(),
                members: input.members,
                terminals: input.terminals,
                archived: input.archived,
            };
            tx.put(&k, old.map(|r| r.revision), &doc(CATALOG, &catalog)?)?;
            let mut members = catalog.members.keys().cloned().collect::<BTreeSet<_>>();
            members.insert(catalog.owner.clone());
            if let Some(previous) = &previous {
                members.extend(previous.members.keys().cloned());
            }
            for member in members {
                let index = name(format!("{}{}", member_prefix(&member)?, catalog.id));
                let old_index = tx.get(&index)?;
                let value = (catalog.owner == member || catalog.members.contains_key(&member))
                    .then(|| catalog.id.clone());
                tx.put(
                    &index,
                    old_index.map(|r| r.revision),
                    &doc("rx.internal.catalog-membership.v1", &value)?,
                )?;
            }
            save(
                tx,
                "definitioncataloghistory",
                (&catalog.id, revision),
                None,
                CATALOG,
                &catalog,
            )?;
            event(tx, "rx.event.definition-catalog-saved.v1", &catalog)?;
            remember(tx, &scope, fingerprint, CATALOG, &catalog)?;
            Ok(catalog)
        })
    }
    pub fn definition_catalog(&mut self, identity: &Identity, id: &Id) -> Result<Catalog> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let p = author(tx, identity, meta, &clock.now(), false)?;
            access(tx, identity, &p, id, false)
        })
    }
    pub fn definition_catalogs(
        &mut self,
        identity: &Identity,
        after: Option<&Name>,
    ) -> Result<catalog::CatalogPage> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let p = author(tx, identity, meta, &clock.now(), false)?;
            let prefix = member_prefix(&p.id)?;
            if after.is_some_and(|a| !a.as_str().starts_with(&prefix)) {
                return reject(Reject::InvalidInput);
            }
            let records = tx.scan_page(&prefix, after, 50)?;
            let next = if records.len() == 50 {
                records.last().map(|r| r.key.clone())
            } else {
                None
            };
            let mut catalogs = vec![];
            for row in records {
                let member: Option<Id> = decode(&row, "rx.internal.catalog-membership.v1")?;
                if let Some(id) = member {
                    let (_, c): (_, Catalog) = load(tx, "definitioncatalog", &id, CATALOG)?;
                    if c.id != id {
                        return Err(StoreError::Integrity(
                            "catalog membership identity differs".into(),
                        ));
                    }
                    if allowed(&c, &p, identity, false) {
                        catalogs.push(c);
                    }
                }
            }
            Ok(catalog::CatalogPage { catalogs, next })
        })
    }
    pub fn save_definition(
        &mut self,
        identity: &Identity,
        key_: &Id,
        prepared: Prepared,
    ) -> Result<View> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let p = author(tx, identity, meta, &now, true)?;
            lifecycle::require_serving(tx)?;
            let input = &prepared.input;
            access(tx, identity, &p, &input.catalog, true)?;
            let (scope, fingerprint) =
                request(meta, &p, "Definition.Save", key_.as_str(), &(&p.id, input))?;
            if let Some(saved) = prior(tx, &scope, fingerprint, VIEW)? {
                return Ok(saved);
            }
            let k = key("definition", &input.id);
            let old = tx.get(&k)?;
            let previous = old
                .as_ref()
                .map(|r| decode::<Version>(r, VERSION))
                .transpose()?;
            if let (Some(row), Some(previous)) = (&old, &previous) {
                if previous.definition.reference.id != input.id
                    || previous.definition.reference.revision != row.revision
                {
                    return Err(StoreError::Integrity(
                        "definition identity/revision differs".into(),
                    ));
                }
                previous
                    .definition
                    .verify()
                    .map_err(StoreError::Integrity)?;
            }
            if let Some(old) = &previous
                && (old.definition.reference.catalog != input.catalog
                    || old.definition.body.kind() != input.body.kind())
            {
                return reject(Reject::Forbidden);
            }
            let revision = match (old.as_ref(), input.expected) {
                (None, None) => Counter(1),
                (Some(r), Some(expected)) => {
                    check_revision(r.revision, expected)?;
                    r.revision.increment().map_err(domain_error)?
                }
                _ => return reject(Reject::StaleRevision),
            };
            let definition = Definition::new(
                input.catalog.clone(),
                input.id.clone(),
                revision,
                input.label.clone(),
                input.body.clone(),
            )
            .map_err(StoreError::Invalid)?;
            let archiving_existing = input.archived
                && previous
                    .as_ref()
                    .is_some_and(|v| v.definition.body == input.body);
            let deps = dependencies(tx, &definition, !archiving_existing)?;
            let effective = definition::resolve(&definition, &deps).map_err(StoreError::Invalid)?;
            let value = Version {
                definition,
                archived: input.archived,
                created_by: previous.map_or_else(|| p.id.clone(), |v| v.created_by),
                updated_by: p.id,
                updated_at: now,
            };
            tx.put(&k, old.map(|r| r.revision), &doc(VERSION, &value)?)?;
            save(
                tx,
                "definitionrevision",
                (&input.id, revision),
                None,
                VERSION,
                &value,
            )?;
            let index = key(&format!("definitionindex/{}", input.catalog), &input.id);
            let prior_index = tx.get(&index)?;
            tx.put(
                &index,
                prior_index.map(|r| r.revision),
                &doc(
                    "rx.internal.definition-summary.v1",
                    &catalog::Summary::from(&value),
                )?,
            )?;
            event(tx, "rx.event.definition-saved.v1", &value)?;
            let result = View {
                version: value,
                effective,
            };
            remember(tx, &scope, fingerprint, VIEW, &result)?;
            Ok(result)
        })
    }
    pub fn definition_points(
        &mut self,
        identity: &Identity,
        query: &definition::pattern::Query,
    ) -> Result<definition::pattern::Page> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let p = author(tx, identity, meta, &clock.now(), false)?;
            if query.subject.catalog != query.rule.catalog {
                return reject(Reject::Forbidden);
            }
            access(tx, identity, &p, &query.subject.catalog, false)?;
            let subject = version(
                tx,
                &query.subject.catalog,
                &query.subject.id,
                Some(query.subject.revision),
            )?;
            let rule = version(
                tx,
                &query.rule.catalog,
                &query.rule.id,
                Some(query.rule.revision),
            )?;
            if subject.definition.reference != query.subject
                || rule.definition.reference != query.rule
            {
                return reject(Reject::InvalidInput);
            }
            let deps = dependencies(tx, &subject.definition, false)?;
            definition::pattern::generate(
                &subject.definition,
                &rule.definition,
                &deps,
                query.offset,
                query.limit,
            )
            .map_err(StoreError::Invalid)
        })
    }
    pub fn definition(
        &mut self,
        identity: &Identity,
        catalog: &Id,
        id: &Id,
        revision: Option<Counter>,
    ) -> Result<View> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let p = author(tx, identity, meta, &clock.now(), false)?;
            access(tx, identity, &p, catalog, false)?;
            let v = version(tx, catalog, id, revision)?;
            view(tx, v)
        })
    }
    pub fn definitions(
        &mut self,
        identity: &Identity,
        catalog: &Id,
        after: Option<&Name>,
        filter: &Filter,
    ) -> Result<catalog::Page> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let p = author(tx, identity, meta, &clock.now(), false)?;
            access(tx, identity, &p, catalog, false)?;
            if filter.query.chars().count() > 120 {
                return reject(Reject::InvalidInput);
            }
            let query = filter.query.trim().to_lowercase();
            let prefix = format!("definitionindex/{catalog}/");
            if after.is_some_and(|a| !a.as_str().starts_with(&prefix)) {
                return reject(Reject::InvalidInput);
            }
            let records = tx.scan_page(&prefix, after, 50)?;
            let next = if records.len() == 50 {
                records.last().map(|r| r.key.clone())
            } else {
                None
            };
            let mut definitions = vec![];
            for row in records {
                let summary: catalog::Summary = decode(&row, "rx.internal.definition-summary.v1")?;
                if summary.reference.catalog != *catalog {
                    return Err(StoreError::Integrity(
                        "definition index catalog differs".into(),
                    ));
                }
                if summary.label.to_lowercase().contains(&query)
                    && filter.kind.is_none_or(|k| k == summary.kind)
                    && filter.archived.is_none_or(|a| a == summary.archived)
                {
                    definitions.push(summary);
                }
            }
            Ok(catalog::Page {
                catalog: catalog.clone(),
                definitions,
                next,
            })
        })
    }
    pub fn definition_history(
        &mut self,
        identity: &Identity,
        catalog: &Id,
        id: &Id,
        before: Option<Counter>,
    ) -> Result<catalog::History> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let p = author(tx, identity, meta, &clock.now(), false)?;
            access(tx, identity, &p, catalog, false)?;
            let current = version(tx, catalog, id, None)?;
            if before == Some(Counter(0)) {
                return reject(Reject::InvalidInput);
            }
            let upper = before.map_or(current.definition.reference.revision.0, |b| {
                (b.0 - 1).min(current.definition.reference.revision.0)
            });
            let lower = upper.saturating_sub(49).max(1);
            let mut versions = vec![];
            for rev in (lower..=upper).rev() {
                versions.push(catalog::Summary::from(&version(
                    tx,
                    catalog,
                    id,
                    Some(Counter(rev)),
                )?));
            }
            Ok(catalog::History {
                catalog: catalog.clone(),
                id: id.clone(),
                versions,
                next: (lower > 1).then_some(Counter(lower)),
            })
        })
    }
}
