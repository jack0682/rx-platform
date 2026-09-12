//! Three-valued bounded condition evaluation. No callbacks, scripts, network or wall clock.
use crate::{DomainError, Result, types::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    Pass,
    Fail,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum Condition {
    All {
        children: Vec<Condition>,
    },
    Any {
        children: Vec<Condition>,
    },
    Eq {
        fact: Name,
        schema: Name,
        unit: Name,
        expected: TypedValue,
    },
    Range {
        fact: Name,
        schema: Name,
        unit: Name,
        min: Real,
        max: Real,
    },
    SetContains {
        fact: Name,
        schema: Name,
        unit: Name,
        expected: Name,
    },
}
#[derive(Clone, Debug)]
pub enum FactValue {
    Scalar(TypedValue),
    Set(Vec<Name>),
}
#[derive(Clone, Debug)]
pub struct Fact {
    pub schema: Name,
    pub unit: Name,
    pub source_generation: Id,
    pub acquired_at: TimePoint,
    pub maximum_age_ns: Counter,
    pub acquisition_uncertainty_ns: Counter,
    pub quality_good: bool,
    pub origin_age_bounded: bool,
    pub disputed: bool,
    pub value: FactValue,
    pub evidence_id: Id,
}
pub struct Context<'a> {
    pub now: &'a TimePoint,
    pub facts: &'a BTreeMap<Name, Fact>,
    pub generations: &'a BTreeMap<Name, Id>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Evaluation {
    pub verdict: Verdict,
    pub evidence_ids: Vec<Id>,
    pub valid_until: Option<TimePoint>,
}

impl Condition {
    pub fn evaluate(&self, context: &Context<'_>) -> Result<Evaluation> {
        let mut remaining = 1024;
        self.eval(context, 0, &mut remaining)
    }
    fn eval(&self, ctx: &Context<'_>, depth: usize, remaining: &mut usize) -> Result<Evaluation> {
        if depth >= 32 || *remaining == 0 {
            return Err(DomainError::InvalidInput(
                "condition complexity limit".into(),
            ));
        }
        *remaining -= 1;
        match self {
            Self::All { children } | Self::Any { children } => {
                if children.is_empty() {
                    return Err(DomainError::InvalidInput("empty condition group".into()));
                }
                // Validate every branch: a PASS branch must not hide malformed input.
                let results: Vec<_> = children
                    .iter()
                    .map(|c| c.eval(ctx, depth + 1, remaining))
                    .collect::<Result<_>>()?;
                let all = matches!(self, Self::All { .. });
                let verdict = if results
                    .iter()
                    .any(|r| r.verdict == if all { Verdict::Fail } else { Verdict::Pass })
                {
                    if all { Verdict::Fail } else { Verdict::Pass }
                } else if results.iter().any(|r| r.verdict == Verdict::Unknown) {
                    Verdict::Unknown
                } else if all {
                    Verdict::Pass
                } else {
                    Verdict::Fail
                };
                let supporting: Vec<_> = if !all && verdict == Verdict::Pass {
                    results
                        .iter()
                        .filter(|r| r.verdict == Verdict::Pass)
                        .max_by_key(|r| r.valid_until.as_ref().map(|t| t.ticks_ns))
                        .into_iter()
                        .collect()
                } else {
                    results.iter().collect()
                };
                let valid_until = if verdict == Verdict::Pass
                    && supporting.iter().all(|r| r.valid_until.is_some())
                {
                    supporting
                        .iter()
                        .filter_map(|r| r.valid_until.as_ref())
                        .min_by_key(|t| t.ticks_ns)
                        .cloned()
                } else {
                    None
                };
                let mut evidence_ids: Vec<_> = supporting
                    .into_iter()
                    .flat_map(|r| r.evidence_ids.clone())
                    .collect();
                evidence_ids.sort();
                evidence_ids.dedup();
                Ok(Evaluation {
                    verdict,
                    evidence_ids,
                    valid_until,
                })
            }
            Self::Eq {
                fact,
                schema,
                unit,
                expected,
            } => leaf(ctx, fact, schema, unit, |value| match value {
                FactValue::Scalar(v)
                    if std::mem::discriminant(v) == std::mem::discriminant(expected) =>
                {
                    Some(v == expected)
                }
                _ => None,
            }),
            Self::Range {
                fact,
                schema,
                unit,
                min,
                max,
            } => {
                if min > max {
                    return Err(DomainError::InvalidInput("reversed range".into()));
                }
                leaf(ctx, fact, schema, unit, |value| match value {
                    FactValue::Scalar(TypedValue::Real(v)) => Some(v >= min && v <= max),
                    _ => None,
                })
            }
            Self::SetContains {
                fact,
                schema,
                unit,
                expected,
            } => leaf(ctx, fact, schema, unit, |value| match value {
                FactValue::Set(v) => Some(v.contains(expected)),
                _ => None,
            }),
        }
    }
}
fn leaf(
    ctx: &Context<'_>,
    id: &Name,
    schema: &Name,
    unit: &Name,
    compare: impl FnOnce(&FactValue) -> Option<bool>,
) -> Result<Evaluation> {
    let Some(fact) = ctx.facts.get(id) else {
        return Ok(Evaluation {
            verdict: Verdict::Unknown,
            evidence_ids: vec![],
            valid_until: None,
        });
    };
    let age = ctx
        .now
        .age_ns(&fact.acquired_at)
        .and_then(|age| age.checked_add(fact.acquisition_uncertainty_ns.0));
    let usable = fact.quality_good
        && fact.origin_age_bounded
        && !fact.disputed
        && &fact.schema == schema
        && &fact.unit == unit
        && ctx.generations.get(id) == Some(&fact.source_generation)
        && age.is_some_and(|age| age <= fact.maximum_age_ns.0);
    let verdict = if !usable {
        Verdict::Unknown
    } else {
        match compare(&fact.value) {
            Some(true) => Verdict::Pass,
            Some(false) => Verdict::Fail,
            None => Verdict::Unknown,
        }
    };
    Ok(Evaluation {
        verdict,
        evidence_ids: vec![fact.evidence_id.clone()],
        valid_until: if verdict == Verdict::Pass {
            fact.acquired_at
                .ticks_ns
                .0
                .checked_add(fact.maximum_age_ns.0)
                .and_then(|n| n.checked_sub(fact.acquisition_uncertainty_ns.0))
                .map(|ticks| TimePoint {
                    clock_id: ctx.now.clock_id.clone(),
                    ticks_ns: Counter(ticks),
                })
        } else {
            None
        },
    })
}
