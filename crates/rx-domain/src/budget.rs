use crate::{
    DomainError, Result,
    types::{Counter, Id},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BudgetUnit {
    PartAttempt,
    OperationCount,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Consumption {
    PartAttempt(Id),
    Operation(Id),
}

/// Owned by the run, never by a mandate, browser session or BT node visit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "BudgetSnapshot")]
pub struct RunBudget {
    unit: BudgetUnit,
    limit: Counter,
    revision: Counter,
    consumptions: BTreeSet<Consumption>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BudgetSnapshot {
    unit: BudgetUnit,
    limit: Counter,
    revision: Counter,
    consumptions: Vec<Consumption>,
}
impl TryFrom<BudgetSnapshot> for RunBudget {
    type Error = DomainError;
    fn try_from(s: BudgetSnapshot) -> Result<Self> {
        let mut budget = Self::new(s.unit, s.limit)?;
        for item in s.consumptions {
            if !budget.consume(item)? {
                return Err(DomainError::InvalidInput(
                    "duplicate stored budget consumption".into(),
                ));
            }
        }
        if s.revision != budget.revision {
            return Err(DomainError::InvalidInput(
                "budget revision does not match immutable consumption history".into(),
            ));
        }
        Ok(budget)
    }
}
impl RunBudget {
    pub fn new(unit: BudgetUnit, limit: Counter) -> Result<Self> {
        limit.nonzero("budget limit")?;
        Ok(Self {
            unit,
            limit,
            revision: Counter(1),
            consumptions: BTreeSet::new(),
        })
    }
    pub fn consumed(&self) -> Counter {
        Counter(self.consumptions.len() as u64)
    }
    pub fn unit(&self) -> BudgetUnit {
        self.unit
    }
    pub fn limit(&self) -> Counter {
        self.limit
    }
    pub fn remaining(&self) -> Counter {
        Counter(self.limit.0 - self.consumed().0)
    }
    pub fn revision(&self) -> Counter {
        self.revision
    }
    pub fn contains(&self, key: &Consumption) -> bool {
        self.consumptions.contains(key)
    }
    /// Returns false when the same logical attempt was already counted.
    pub fn consume(&mut self, key: Consumption) -> Result<bool> {
        if !matches!(
            (self.unit, &key),
            (BudgetUnit::PartAttempt, Consumption::PartAttempt(_))
                | (BudgetUnit::OperationCount, Consumption::Operation(_))
        ) {
            return Err(DomainError::InvalidInput(
                "budget unit and consumption kind differ".into(),
            ));
        }
        if self.consumptions.contains(&key) {
            return Ok(false);
        }
        if self.remaining().0 == 0 {
            return Err(DomainError::InvalidTransition("budget exhausted".into()));
        }
        let revision = self.revision.increment()?;
        self.consumptions.insert(key);
        self.revision = revision;
        Ok(true)
    }
}
