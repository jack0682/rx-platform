//! Signed device declarations for review. They are not installed steps or device validation proofs.
use crate::native_outcome::NativeOutcomeTable;
use rx_domain::{canonical, intent::Intent, types::*};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Environment {
    Simulation,
    Physical,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub path: String,
    pub artifact: ArtifactRef,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub schema: Name,
    pub installation: Id,
    pub cell: Name,
    pub target: Name,
    pub environment: Environment,
    pub profile_digest: Digest,
    #[serde(deserialize_with = "unique_conditions")]
    pub condition_ids: BTreeSet<Name>,
    pub operations: BTreeMap<Name, Intent>,
    pub outcomes: Option<NativeOutcomeTable>,
    pub documents: BTreeMap<Name, Document>,
}
fn unique_conditions<'de, D: serde::Deserializer<'de>>(d: D) -> Result<BTreeSet<Name>, D::Error> {
    let names = Vec::<Name>::deserialize(d)?;
    let set = names.iter().cloned().collect::<BTreeSet<_>>();
    if names.len() != set.len() || names.len() > 32 {
        return Err(serde::de::Error::custom(
            "duplicate or oversized catalog conditions",
        ));
    }
    Ok(set)
}
impl Catalog {
    pub fn validate(&self) -> Result<(), String> {
        let err = || "device operation catalog shape or correlation differs".to_string();
        let roles = if self.outcomes.is_some() {
            vec!["family", "profile", "adapter", "operations", "outcomes"]
        } else {
            vec!["family", "profile", "adapter", "operations"]
        };
        if self.schema.as_str() != "rx.device-operation-catalog.v1"
            || self.condition_ids.is_empty()
            || self.condition_ids.len() > 32
            || self.operations.is_empty()
            || self.operations.len() > 64
            || self
                .documents
                .keys()
                .map(Name::as_str)
                .collect::<BTreeSet<_>>()
                != roles.into_iter().collect()
            || self
                .documents
                .values()
                .map(|d| &d.path)
                .collect::<BTreeSet<_>>()
                .len()
                != self.documents.len()
            || self.documents.values().any(|d| {
                d.path.is_empty()
                    || d.path.len() > 240
                    || d.artifact.size_bytes.0 == 0
                    || d.artifact.size_bytes.0 > 2 * 1024 * 1024
            })
        {
            return Err(err());
        }
        if let Some(t) = &self.outcomes {
            t.validate().map_err(|e| e.to_string())?;
            if t.profile_digest != self.profile_digest {
                return Err(err());
            }
        }
        for i in self.operations.values() {
            i.normalized().map_err(|e| e.to_string())?;
            if i.target != self.target
                || i.profile_digest != self.profile_digest
                || self
                    .outcomes
                    .as_ref()
                    .is_some_and(|t| t.completion_rule != i.completion_rule)
            {
                return Err(err());
            }
        }
        if canonical::bytes(self).map_err(|e| e.to_string())?.len() > 131_072 {
            return Err("device catalog exceeds 128 KiB".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_conditions_are_rejected_instead_of_normalized_away() {
        let value = serde_json::json!({"schema":"rx.device-operation-catalog.v1","installation":"00000000-0000-4000-8000-000000000001","cell":"cell/a","target":"robot","environment":"SIMULATION","profile_digest":"00".repeat(32),"condition_ids":["ready","ready"],"operations":{},"outcomes":null,"documents":{}});
        assert!(canonical::decode_json::<Catalog>(&canonical::bytes(&value).unwrap()).is_err());
    }
}
