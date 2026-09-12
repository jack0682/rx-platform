use crate::{
    DomainError, Result,
    types::{Counter, Name},
};
use serde::Serialize;
use std::collections::BTreeMap;

/// Durable maxima are merged per scope; a received vector never replaces the map.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ScopeEpochs(BTreeMap<Name, Counter>);
impl ScopeEpochs {
    pub fn get(&self, scope: &Name) -> Option<Counter> {
        self.0.get(scope).copied()
    }
    pub fn install(&mut self, requested: &[(Name, Counter)]) -> Result<bool> {
        let mut next = self.0.clone();
        let mut seen = std::collections::BTreeSet::new();
        if requested.is_empty() {
            return Err(DomainError::InvalidInput("empty scope vector".into()));
        }
        for (scope, epoch) in requested {
            epoch.nonzero("scope epoch")?;
            if !seen.insert(scope) {
                return Err(DomainError::InvalidInput("duplicate scope".into()));
            }
            if self.0.get(scope).is_some_and(|old| old > epoch) {
                return Err(DomainError::Conflict("stale scope epoch".into()));
            }
            next.insert(scope.clone(), *epoch);
        }
        let changed = next != self.0;
        self.0 = next;
        Ok(changed)
    }
    pub fn matches_exact(&self, required: &[(Name, Counter)]) -> bool {
        !required.is_empty()
            && required
                .iter()
                .map(|(s, _)| s)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == required.len()
            && required
                .iter()
                .all(|(scope, epoch)| epoch.0 > 0 && self.0.get(scope) == Some(epoch))
    }
}
