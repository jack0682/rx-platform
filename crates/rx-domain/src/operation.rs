use crate::{DomainError, Result, types::*};
use serde::{Deserialize, Serialize};

macro_rules! states {
    ($name:ident { $($state:ident),+ }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "SCREAMING_SNAKE_CASE")]
        pub enum $name { $($state),+ }
    };
}
states!(Phase {
    Admitted,
    Active,
    Reconciling,
    Settled
});
states!(Knowledge {
    NotSent,
    MayHaveExecuted,
    Accepted,
    Running,
    Ended,
    Unknown
});
states!(Outcome {
    None,
    Succeeded,
    Failed,
    Canceled,
    NotExecuted,
    Unresolved
});
states!(Integrity { Valid, Disputed });
states!(Disposition {
    Held,
    Quarantined,
    Released
});

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "OperationSnapshot")]
pub struct Operation {
    operation_id: Id,
    revision: Counter,
    intent_digest: Digest,
    phase: Phase,
    execution_knowledge: Knowledge,
    outcome: Outcome,
    integrity: Integrity,
    disposition: Disposition,
    evidence_ids: Vec<Id>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationSnapshot {
    operation_id: Id,
    revision: Counter,
    intent_digest: Digest,
    phase: Phase,
    execution_knowledge: Knowledge,
    outcome: Outcome,
    integrity: Integrity,
    disposition: Disposition,
    evidence_ids: Vec<Id>,
}
impl TryFrom<OperationSnapshot> for Operation {
    type Error = DomainError;
    fn try_from(s: OperationSnapshot) -> Result<Self> {
        s.revision.nonzero("operation revision")?;
        if (s.phase == Phase::Settled) != (s.outcome != Outcome::None) {
            return Err(DomainError::InvalidInput(
                "phase/outcome contradiction in stored operation".into(),
            ));
        }
        if s.integrity == Integrity::Disputed && s.disposition != Disposition::Quarantined {
            return Err(DomainError::InvalidInput(
                "disputed operation is not quarantined".into(),
            ));
        }
        if s.disposition == Disposition::Released && s.phase != Phase::Settled {
            return Err(DomainError::InvalidInput(
                "unsettled operation released its resource".into(),
            ));
        }
        if s.phase == Phase::Settled {
            if s.evidence_ids.is_empty() {
                return Err(DomainError::InvalidInput(
                    "terminal operation has no evidence".into(),
                ));
            }
            let knowledge = match s.outcome {
                Outcome::Unresolved => Knowledge::Unknown,
                Outcome::NotExecuted => Knowledge::NotSent,
                _ => Knowledge::Ended,
            };
            if s.execution_knowledge != knowledge {
                return Err(DomainError::InvalidInput(
                    "terminal outcome/knowledge contradiction".into(),
                ));
            }
        }
        let unique: std::collections::BTreeSet<_> = s.evidence_ids.iter().collect();
        if unique.len() != s.evidence_ids.len() {
            return Err(DomainError::InvalidInput(
                "duplicate stored evidence reference".into(),
            ));
        }
        Ok(Self {
            operation_id: s.operation_id,
            revision: s.revision,
            intent_digest: s.intent_digest,
            phase: s.phase,
            execution_knowledge: s.execution_knowledge,
            outcome: s.outcome,
            integrity: s.integrity,
            disposition: s.disposition,
            evidence_ids: s.evidence_ids,
        })
    }
}

/// An application evaluator must establish these facts from a bound profile and immutable evidence.
/// This value is not an HTTP command and must never be deserialized from caller assertions.
#[derive(Clone, Debug)]
pub struct Conclusion {
    pub outcome: Outcome,
    pub evidence_ids: Vec<Id>,
}
#[derive(Clone, Copy, Debug)]
pub struct ReleaseConditions {
    pub no_residual_native: bool,
    pub control_handover_confirmed: bool,
    pub support_handover_confirmed: bool,
}
impl Operation {
    pub fn admitted(operation_id: Id, intent_digest: Digest) -> Self {
        Self {
            operation_id,
            intent_digest,
            revision: Counter(1),
            phase: Phase::Admitted,
            execution_knowledge: Knowledge::NotSent,
            outcome: Outcome::None,
            integrity: Integrity::Valid,
            disposition: Disposition::Held,
            evidence_ids: vec![],
        }
    }
    pub fn id(&self) -> &Id {
        &self.operation_id
    }
    pub fn intent_digest(&self) -> Digest {
        self.intent_digest
    }
    pub fn revision(&self) -> Counter {
        self.revision
    }
    pub fn phase(&self) -> Phase {
        self.phase
    }
    pub fn knowledge(&self) -> Knowledge {
        self.execution_knowledge
    }
    pub fn outcome(&self) -> Outcome {
        self.outcome
    }
    pub fn integrity(&self) -> Integrity {
        self.integrity
    }
    pub fn disposition(&self) -> Disposition {
        self.disposition
    }
    pub fn evidence_ids(&self) -> &[Id] {
        &self.evidence_ids
    }
    pub fn quarantine(&mut self) -> Result<()> {
        if self.disposition != Disposition::Quarantined {
            self.revision = self.revision.increment()?;
            self.disposition = Disposition::Quarantined;
        }
        Ok(())
    }
    pub fn dispute(&mut self) -> Result<()> {
        if self.integrity != Integrity::Disputed {
            self.revision = self.revision.increment()?;
            self.integrity = Integrity::Disputed;
        }
        self.disposition = Disposition::Quarantined;
        if self.phase != Phase::Settled {
            self.phase = Phase::Reconciling;
            self.execution_knowledge = Knowledge::Unknown;
        }
        Ok(())
    }

    pub fn sent(&mut self) -> Result<()> {
        if self.phase != Phase::Admitted || self.execution_knowledge != Knowledge::NotSent {
            return Err(DomainError::InvalidTransition(
                "native dispatch already entered or unresolved".into(),
            ));
        }
        self.revision = self.revision.increment()?;
        self.phase = Phase::Active;
        self.execution_knowledge = Knowledge::MayHaveExecuted;
        Ok(())
    }
    pub fn lose_continuity(&mut self) -> Result<()> {
        if self.phase == Phase::Settled {
            return Ok(());
        }
        self.revision = self.revision.increment()?;
        self.phase = Phase::Reconciling;
        self.execution_knowledge = Knowledge::Unknown;
        self.disposition = Disposition::Quarantined;
        Ok(())
    }
    pub fn observed_running(&mut self, evidence: Id) -> Result<()> {
        if self.phase == Phase::Settled {
            return Err(DomainError::InvalidTransition(
                "current motion does not rewrite historical outcome".into(),
            ));
        }
        self.revision = self.revision.increment()?;
        self.execution_knowledge = Knowledge::Running;
        // A running observation does not prove continuity or release quarantined resources.
        if self.phase != Phase::Reconciling {
            self.phase = Phase::Active;
        }
        self.add_evidence(evidence);
        Ok(())
    }
    pub fn conclude(&mut self, conclusion: Conclusion) -> Result<()> {
        if conclusion.outcome == Outcome::None || conclusion.evidence_ids.is_empty() {
            return Err(DomainError::InvalidInput(
                "terminal result requires immutable evidence".into(),
            ));
        }
        if self.outcome == conclusion.outcome
            && conclusion
                .evidence_ids
                .iter()
                .all(|e| self.evidence_ids.contains(e))
        {
            return Ok(());
        }
        self.revision = self.revision.increment()?;
        for evidence in conclusion.evidence_ids {
            self.add_evidence(evidence);
        }
        if self.phase == Phase::Settled && self.outcome != conclusion.outcome {
            self.integrity = Integrity::Disputed;
            self.disposition = Disposition::Quarantined;
            return Ok(());
        }
        self.phase = Phase::Settled;
        self.outcome = conclusion.outcome;
        self.execution_knowledge = match conclusion.outcome {
            Outcome::Unresolved => Knowledge::Unknown,
            Outcome::NotExecuted => Knowledge::NotSent,
            _ => Knowledge::Ended,
        };
        if conclusion.outcome == Outcome::Unresolved {
            self.disposition = Disposition::Quarantined;
        }
        Ok(())
    }
    pub fn release(&mut self, conditions: ReleaseConditions, evidence: Vec<Id>) -> Result<()> {
        if self.phase != Phase::Settled
            || self.integrity == Integrity::Disputed
            || !conditions.no_residual_native
            || !conditions.control_handover_confirmed
            || !conditions.support_handover_confirmed
            || evidence.is_empty()
        {
            return Err(DomainError::InvalidTransition(
                "resource handover is unproven".into(),
            ));
        }
        self.revision = self.revision.increment()?;
        self.disposition = Disposition::Released;
        for id in evidence {
            self.add_evidence(id);
        }
        Ok(())
    }
    fn add_evidence(&mut self, evidence: Id) {
        if !self.evidence_ids.contains(&evidence) {
            self.evidence_ids.push(evidence);
            self.evidence_ids.sort();
        }
    }
}
