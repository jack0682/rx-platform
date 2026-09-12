//! Data-only native result interpretation. Device schemas belong to solution profiles.
use rx_domain::{DomainError, Result, types::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NativeConclusion {
    Succeeded,
    Failed,
    Canceled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeOutcomeCase {
    pub status_schema: Name,
    pub statuses: Vec<Integer>,
    pub conclusion: NativeConclusion,
}

/// Exact schema/code pairs only. Absence of a match is never a terminal conclusion.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeOutcomeTable {
    pub schema: Name,
    pub profile_digest: Digest,
    pub completion_rule: Name,
    pub cases: Vec<NativeOutcomeCase>,
}
impl NativeOutcomeTable {
    pub fn validate(&self) -> Result<()> {
        if self.schema.as_str() != "rx.native-outcome-table.v1"
            || self.cases.is_empty()
            || self.cases.len() > 16
        {
            return Err(DomainError::InvalidInput(
                "native outcome table shape".into(),
            ));
        }
        let mut pairs = BTreeSet::new();
        for case in &self.cases {
            if case.statuses.is_empty() || case.statuses.len() > 64 {
                return Err(DomainError::InvalidInput("native outcome codes".into()));
            }
            for code in &case.statuses {
                if !pairs.insert((&case.status_schema, code.0)) || pairs.len() > 128 {
                    return Err(DomainError::InvalidInput(
                        "duplicate or oversized native outcome pairs".into(),
                    ));
                }
            }
        }
        Ok(())
    }
    pub fn resolve(&self, schema: &Name, status: Integer) -> Result<Option<NativeConclusion>> {
        self.validate()?;
        Ok(self.cases.iter().find_map(|case| {
            (&case.status_schema == schema && case.statuses.iter().any(|s| s.0 == status.0))
                .then_some(case.conclusion)
        }))
    }
}
