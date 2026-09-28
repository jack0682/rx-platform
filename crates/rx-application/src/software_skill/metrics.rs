//! Server-derived, version-scoped metrics. Unknown is not counted as failure.
use super::*;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub since_ms: Option<u64>,
    pub until_ms: Option<u64>,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct Counts {
    pub admitted: u64,
    pub queued: u64,
    pub active: u64,
    pub succeeded: u64,
    pub failed: u64,
    pub canceled: u64,
    pub not_executed: u64,
    pub unknown: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct Distribution {
    pub samples: usize,
    pub missing: usize,
    pub mean_ms: Option<f64>,
    pub p50_ms: Option<u64>,
    pub p95_ms: Option<u64>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Group {
    pub skill: String,
    pub version: String,
    pub package_digest: Digest,
    pub counts: Counts,
    pub decided_denominator: u64,
    pub success_fraction_of_decided: Option<f64>,
    pub execution_ms: Distribution,
    pub queue_wait_wall_ms: Distribution,
}
#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub ledger_sequence: Counter,
    pub window: Window,
    pub groups: Vec<Group>,
}
fn distribution(mut values: Vec<u64>, missing: usize) -> Distribution {
    values.sort_unstable();
    let n = values.len();
    Distribution {
        samples: n,
        missing,
        mean_ms: if n == 0 {
            None
        } else {
            Some(values.iter().map(|v| *v as f64).sum::<f64>() / n as f64)
        },
        p50_ms: if n == 0 {
            None
        } else {
            Some(values[(n * 50).div_ceil(100) - 1])
        },
        p95_ms: if n == 0 {
            None
        } else {
            Some(values[(n * 95).div_ceil(100) - 1])
        },
    }
}
impl<R: Repository> Engine<R> {
    pub fn metrics(&mut self, window: Window) -> Result<Report> {
        if window
            .since_ms
            .zip(window.until_ms)
            .is_some_and(|(a, b)| a >= b)
        {
            return Err(invalid("metrics window must have since < until"));
        }
        let (sequence, rows) = self.repository.snapshot()?;
        let mut groups: BTreeMap<(String, String, Digest), Vec<Run>> = BTreeMap::new();
        for row in rows
            .into_iter()
            .filter(|r| r.key.as_str().starts_with("local-sim/run/"))
        {
            let run: Run = decode(&row, RUN)?;
            if window.since_ms.is_some_and(|s| run.submitted_ms < s)
                || window.until_ms.is_some_and(|u| run.submitted_ms >= u)
            {
                continue;
            }
            groups
                .entry((
                    run.request.skill.clone(),
                    run.request.version.clone(),
                    run.package_digest,
                ))
                .or_default()
                .push(run);
        }
        let mut result = vec![];
        for ((skill, version, package_digest), runs) in groups {
            let mut counts = Counts::default();
            let mut execution = vec![];
            let mut wait = vec![];
            let mut missing_execution = 0;
            let mut missing_wait = 0;
            for run in runs {
                counts.admitted += 1;
                if run.operation.integrity() != rx_domain::operation::Integrity::Valid {
                    counts.unknown += 1;
                    continue;
                }
                match run.operation.outcome() {
                    Outcome::Succeeded => counts.succeeded += 1,
                    Outcome::Failed => counts.failed += 1,
                    Outcome::Canceled => counts.canceled += 1,
                    Outcome::NotExecuted => counts.not_executed += 1,
                    Outcome::Unresolved => counts.unknown += 1,
                    Outcome::None => {
                        if run.started_ms.is_some() {
                            counts.active += 1;
                        } else {
                            counts.queued += 1;
                        }
                    }
                }
                if matches!(
                    run.operation.outcome(),
                    Outcome::Succeeded | Outcome::Failed
                ) {
                    if let Some(duration) = run.duration_ms {
                        execution.push(duration);
                    } else {
                        missing_execution += 1;
                    }
                }
                if let Some(start) = run.started_ms {
                    if let Some(duration) = start.checked_sub(run.submitted_ms) {
                        wait.push(duration);
                    } else {
                        missing_wait += 1;
                    }
                }
            }
            let denominator = counts.succeeded + counts.failed;
            let fraction = if denominator == 0 {
                None
            } else {
                Some(counts.succeeded as f64 / denominator as f64)
            };
            result.push(Group {
                skill,
                version,
                package_digest,
                counts,
                decided_denominator: denominator,
                success_fraction_of_decided: fraction,
                execution_ms: distribution(execution, missing_execution),
                queue_wait_wall_ms: distribution(wait, missing_wait),
            });
        }
        Ok(Report {
            ledger_sequence: sequence,
            window,
            groups: result,
        })
    }
}
