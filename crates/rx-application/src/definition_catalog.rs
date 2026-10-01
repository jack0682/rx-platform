//! Authoring ACL and persistence commands; excluded from the shared execution SDK.
use rx_domain::{definition::*, types::*};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Access {
    Read,
    Edit,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogSave {
    pub id: Id,
    pub expected: Option<Counter>,
    pub title: String,
    pub members: BTreeMap<Name, Access>,
    pub terminals: BTreeSet<Name>,
    pub archived: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub id: Id,
    pub revision: Counter,
    pub title: String,
    pub owner: Name,
    pub members: BTreeMap<Name, Access>,
    pub terminals: BTreeSet<Name>,
    pub archived: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogPage {
    pub catalogs: Vec<Catalog>,
    pub next: Option<Name>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Save {
    pub catalog: Id,
    pub id: Id,
    pub expected: Option<Counter>,
    pub label: String,
    pub body: Body,
    pub archived: bool,
}
pub struct Prepared {
    pub(crate) input: Save,
}
impl Prepared {
    pub fn prepare(input: Save) -> Result<Self, String> {
        if input.label.trim().is_empty() || input.label.chars().count() > 120 {
            return Err("definition label".into());
        }
        input.body.validate_shape()?;
        Ok(Self { input })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Version {
    pub definition: Definition,
    pub archived: bool,
    pub created_by: Name,
    pub updated_by: Name,
    pub updated_at: TimePoint,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    pub version: Version,
    pub effective: Effective,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub reference: Reference,
    pub label: String,
    pub kind: Kind,
    pub archived: bool,
}
impl From<&Version> for Summary {
    fn from(v: &Version) -> Self {
        Self {
            reference: v.definition.reference.clone(),
            label: v.definition.label.clone(),
            kind: v.definition.body.kind(),
            archived: v.archived,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct Filter {
    pub query: String,
    pub kind: Option<Kind>,
    pub archived: Option<bool>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub catalog: Id,
    pub definitions: Vec<Summary>,
    pub next: Option<Name>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct History {
    pub catalog: Id,
    pub id: Id,
    pub versions: Vec<Summary>,
    pub next: Option<Counter>,
}
