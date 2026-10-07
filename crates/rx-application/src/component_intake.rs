//! Authenticated staged registration intake. Prepared source values are never wire inputs.
use crate::Identity;
use rx_domain::{canonical, component::Registration, component_transfer::*, types::*};
use rx_ports::{Record, SealedRepository, StoreError, StoredEvent};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceBinding {
    pub owner: Name,
    pub location: Digest,
}
impl SourceBinding {
    pub fn fingerprint(&self, source: &Name) -> rx_ports::Result<Digest> {
        canonical::digest("RX-REGISTRATION-SOURCE-BINDING-v1", &(source, self)).map_err(bad)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submit {
    pub source: Name,
    pub freeze: Id,
    pub expected_binding: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub source: Name,
    pub owner: Name,
    pub binding: Digest,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum State {
    Receiving,
    Accepted,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Progress {
    pub id: Id,
    pub source: Name,
    pub owner: Name,
    pub state: State,
    pub declarations: Counter,
    pub expected_declarations: Counter,
    pub history_after: Counter,
    pub history_head: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub id: Id,
    pub source: Name,
    pub owner: Name,
    pub input: Submit,
    pub source_binding: SourceBinding,
    pub original: FreezeRecord,
    pub imported_by: Name,
    pub imported_at: TimePoint,
    pub content_verification: crate::resident_component::Verification,
    pub execution_ownership: crate::resident_component::Ownership,
    pub work_use_permission: crate::resident_component::WorkUse,
}
#[derive(Clone)]
pub struct Ticket {
    pub(crate) identity: Identity,
    pub(crate) key: Id,
    pub(crate) input: Submit,
    pub(crate) boot: Id,
    pub(crate) installation: Id,
    pub(crate) binding: SourceBinding,
}
impl Ticket {
    pub fn source(&self) -> &Name {
        &self.input.source
    }
    pub fn binding(&self) -> &SourceBinding {
        &self.binding
    }
}
pub enum Preflight {
    Recorded(Box<Receipt>),
    Read(Box<Ticket>),
}
pub struct Begin {
    pub(crate) ticket: Ticket,
    pub(crate) freeze: FreezeRecord,
    pub(crate) selections: Option<Record>,
}
pub struct Declarations {
    pub(crate) ticket: Ticket,
    pub(crate) start: Counter,
    pub(crate) rows: Vec<Record>,
}
pub struct History {
    pub(crate) ticket: Ticket,
    pub(crate) after: Counter,
    pub(crate) events: Vec<StoredEvent>,
}
pub struct Finish {
    pub(crate) ticket: Ticket,
}

fn bad(e: impl std::fmt::Display) -> StoreError {
    StoreError::Integrity(e.to_string())
}
fn decode<T: serde::de::DeserializeOwned>(row: &Record, schema: &str) -> rx_ports::Result<T> {
    if row.document.schema.as_str() != schema {
        return Err(bad("source schema differs"));
    }
    canonical::decode_json(&canonical::bytes(&row.document.value).map_err(bad)?).map_err(bad)
}
/// Holds the actual configured source while its immutable cut is transferred.
/// The implementation must be a trusted persistence adapter, not deserialized proof data.
pub struct Reader<R> {
    source: R,
    ticket: Ticket,
    freeze: FreezeRecord,
    declarations: Vec<Record>,
    selections: Option<Record>,
}
impl<R: SealedRepository> Reader<R> {
    pub fn open(ticket: Ticket, mut source: R) -> rx_ports::Result<Self> {
        let expected = PREFIXES
            .into_iter()
            .map(|s| Name::new(s).expect("fixed prefix"))
            .collect::<BTreeSet<_>>();
        if source
            .sealed_namespaces()?
            .into_iter()
            .collect::<BTreeSet<_>>()
            != expected
        {
            return Err(bad("source namespace fences differ"));
        }
        let (freeze, declarations, selections) = source.transact(|tx| {
            let marker = tx
                .get(&Name::new(FREEZE).expect("key"))?
                .ok_or_else(|| bad("freeze marker absent"))?;
            let freeze: FreezeRecord = decode(&marker, FREEZE_SCHEMA)?;
            if freeze.request.id != ticket.input.freeze
                || freeze.request.target_installation != ticket.installation
                || freeze.history_head > tx.control_head()?
            {
                return Err(bad("freeze target/cut differs"));
            }
            let declarations = tx.scan(DECLARATIONS)?;
            let selections = tx.get(&Name::new(SELECTIONS).expect("key"))?;
            if declarations.is_empty()
                || declarations.len() as u64 != freeze.declaration_count.0
                || canonical::digest(
                    "RX-REGISTRATION-SOURCE-CUT-v1",
                    &(&declarations, &selections),
                )
                .map_err(bad)?
                    != freeze.declarations_digest
            {
                return Err(bad("frozen declarations differ"));
            }
            let mut ids = BTreeSet::new();
            for row in &declarations {
                let registration: Registration = decode(row, REGISTRATION_SCHEMA)?;
                row.revision.nonzero("source revision").map_err(bad)?;
                if row.key.as_str() != format!("{DECLARATIONS}{}", registration.id)
                    || !ids.insert(registration.id)
                {
                    return Err(bad("source declaration identity"));
                }
            }
            if let Some(row) = &selections {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Selection {
                    selection: Name,
                    registration: Id,
                }
                let entries: Vec<Selection> = decode(row, "rx.resident-selections.v1")?;
                let mut names = BTreeSet::new();
                let mut assigned = BTreeSet::new();
                for e in entries {
                    if !names.insert(e.selection)
                        || !assigned.insert(e.registration.clone())
                        || !ids.contains(&e.registration)
                    {
                        return Err(bad("source selection identity"));
                    }
                }
            }
            Ok((freeze, declarations, selections))
        })?;
        Ok(Self {
            source,
            ticket,
            freeze,
            declarations,
            selections,
        })
    }
    pub fn begin(&self) -> Begin {
        Begin {
            ticket: self.ticket.clone(),
            freeze: self.freeze.clone(),
            selections: self.selections.clone(),
        }
    }
    pub fn declarations(&self, start: Counter) -> rx_ports::Result<Declarations> {
        let start_usize = usize::try_from(start.0).map_err(bad)?;
        if start_usize > self.declarations.len() {
            return Err(bad("declaration cursor exceeds source"));
        }
        Ok(Declarations {
            ticket: self.ticket.clone(),
            start,
            rows: self
                .declarations
                .iter()
                .skip(start_usize)
                .take(8)
                .cloned()
                .collect(),
        })
    }
    pub fn history(&mut self, after: Counter) -> rx_ports::Result<History> {
        if after > self.freeze.history_head {
            return Err(bad("history cursor exceeds source"));
        }
        let events = if after == self.freeze.history_head {
            vec![]
        } else {
            self.source
                .control_events_after(after, 8)?
                .into_iter()
                .take_while(|e| e.seq <= self.freeze.history_head)
                .collect::<Vec<_>>()
        };
        if after < self.freeze.history_head && events.is_empty() {
            return Err(bad("source history ends before frozen cut"));
        }
        Ok(History {
            ticket: self.ticket.clone(),
            after,
            events,
        })
    }
    pub fn finish(&self) -> Finish {
        Finish {
            ticket: self.ticket.clone(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchivePage {
    pub id: Id,
    pub events: Vec<StoredEvent>,
    pub next_after: Option<Counter>,
}

pub type Bindings = BTreeMap<Name, SourceBinding>;
