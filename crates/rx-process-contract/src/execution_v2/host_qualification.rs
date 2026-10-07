//! Host acceptance of the qualified v2 input domain; never an operation permit.
use super::host_configuration as configuration;
use super::*;
use rx_domain::host_qualification as legacy;

pub const REQUEST_SCHEMA: &str = "rx.host-execution-qualification-request.v2";
pub const RECEIPT_SCHEMA: &str = "rx.host-execution-qualification-receipt.v2";
pub const OBSERVATION_SCHEMA: &str = "rx.host-execution-qualification-observation.v2";
pub const MAX_BYTES: usize = 1_000_000;
fn bounded(value: &impl Serialize) -> Result<(), String> {
    if canonical::bytes(value).map_err(|e| e.to_string())?.len() > MAX_BYTES {
        return Err("v2 qualification payload exceeds bound".into());
    }
    Ok(())
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyBinding {
    pub publication: Reference,
    pub policy: ArtifactRef,
    pub configuration_request: Digest,
    pub configuration_receipt: Digest,
}
impl PolicyBinding {
    fn validate(&self) -> Result<(), String> {
        nonzero(self.publication.digest)?;
        nonzero(self.configuration_request)?;
        nonzero(self.configuration_receipt)?;
        if self.publication.revision != Counter(1) {
            return Err("qualification publication revision differs".into());
        }
        reference(&self.policy, POLICY_SCHEMA, MAX_POLICY_BYTES as u64)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema: Name,
    pub context: legacy::Request,
    pub policies: BTreeMap<Name, PolicyBinding>,
}
impl Request {
    pub fn validate(&self) -> Result<(), String> {
        bounded(self)?;
        self.context.validate()?;
        if self.schema.as_str() != REQUEST_SCHEMA
            || self.policies.is_empty()
            || self.policies.len() > 64
        {
            return Err("v2 qualification request schema/domain".into());
        }
        for (cell, binding) in &self.policies {
            binding.validate()?;
            let target = self
                .context
                .cells
                .iter()
                .find(|c| &c.cell == cell)
                .ok_or("v2 qualification cell absent")?;
            if target.environment.as_str() != "SIMULATION"
                || !target.dependencies.contains(&binding.policy.sha256)
            {
                return Err("v2 qualification environment/policy dependency".into());
            }
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<Digest, String> {
        self.validate()?;
        canonical::digest(
            "RX-HOST-QUALIFICATION-REQUEST-v2",
            &(self.context.digest()?, &self.policies),
        )
        .map_err(|e| e.to_string())
    }
    /// Required before acceptance on both P and Host, using the original durable receipt.
    pub fn matches_configuration(&self, configured: &configuration::Receipt) -> Result<(), String> {
        self.validate()?;
        configured.validate()?;
        let source = &configured.context;
        let targets: BTreeSet<_> = self.context.cells.iter().map(|c| &c.cell).collect();
        if source.status != rx_domain::host_configuration::Status::AppliedUnqualified
            || self.context.host != source.request.host
            || self.context.expected_host_boot != source.host_boot
            || self.context.delivery_journal != source.journal
            || self.context.binding_digest != source.request.binding_digest
            || self.context.change != source.request.change
            || targets != source.request.cells.iter().map(|c| &c.cell).collect()
            || self.policies.keys().ne(configured.request.policies.keys())
        {
            return Err("v2 qualification configuration identity/cohort differs".into());
        }
        let receipt_digest = configured.digest()?;
        for (cell, binding) in &self.policies {
            let policy = &configured.request.policies[cell];
            let target = self
                .context
                .cells
                .iter()
                .find(|c| &c.cell == cell)
                .ok_or("qualification target absent")?;
            let applied = source
                .request
                .cells
                .iter()
                .find(|c| &c.cell == cell)
                .ok_or("configuration target absent")?;
            if binding.publication != policy.publication
                || binding.policy != policy.reference
                || binding.configuration_request != configured.request_digest
                || binding.configuration_receipt != receipt_digest
                || target.context_request != source.request.id
                || target.context_sequence != source.sequence
                || target.configuration != applied.after_configuration
                || target.definition != applied.definition
                || target.envelope != applied.envelope
                || target.environment != applied.environment
                || target.allowed_intents.iter().collect::<BTreeSet<_>>()
                    != applied.required_intents.iter().collect()
            {
                return Err("v2 qualification differs from accepted configuration policy".into());
            }
            let mut dependencies = BTreeSet::from([
                policy.reference.sha256,
                policy.policy.report_index.sha256,
                policy.policy.definition_closure.sha256,
            ]);
            for package in policy.packages.values() {
                dependencies.extend([package.manifest, package.signature, package.catalog.sha256]);
            }
            for action in policy
                .policy
                .templates
                .values()
                .filter(|a| a.host == self.context.host)
            {
                let Body::Program(program) = &action.intent.body else {
                    return Err("non-program v2 template".into());
                };
                dependencies.extend([
                    program.program.sha256,
                    program.parameter_set.sha256,
                    action.intent.profile_digest,
                    action.intent.site_config_digest,
                ]);
                dependencies.extend(action.intent.calibration_digests.iter().copied());
            }
            if !dependencies.is_subset(&target.dependencies.iter().copied().collect()) {
                return Err(
                    "v2 qualification omits configured domain/template dependencies".into(),
                );
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedPolicy {
    pub binding: PolicyBinding,
    pub qualification: Id,
    pub qualification_revision: Counter,
    pub request: Id,
    pub acceptance_sequence: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub schema: Name,
    pub request: Request,
    pub request_digest: Digest,
    pub context: legacy::Receipt,
    pub policies: BTreeMap<Name, AcceptedPolicy>,
}
impl Receipt {
    pub fn validate(&self) -> Result<(), String> {
        bounded(self)?;
        self.context.validate()?;
        if self.schema.as_str() != RECEIPT_SCHEMA
            || self.request.digest()? != self.request_digest
            || self.context.request_digest != self.request.context.digest()?
        {
            return Err("v2 qualification receipt correlation differs".into());
        }
        if self.context.status == legacy::Status::NotAccepted {
            return if self.policies.is_empty() {
                Ok(())
            } else {
                Err("rejection cannot accept v2 policy".into())
            };
        }
        if self.policies.keys().ne(self.request.policies.keys()) {
            return Err("v2 qualification acceptance coverage differs".into());
        }
        for (cell, binding) in &self.request.policies {
            let accepted = &self.policies[cell];
            let target = self
                .request
                .context
                .cells
                .iter()
                .find(|c| &c.cell == cell)
                .ok_or("qualification target missing")?;
            if accepted.binding != *binding
                || accepted.qualification != target.qualification
                || accepted.qualification_revision != target.qualification_revision
                || accepted.request != self.request.context.id
                || accepted.acceptance_sequence != self.context.sequence
            {
                return Err("v2 qualification accepted policy differs".into());
            }
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<Digest, String> {
        self.validate()?;
        canonical::digest("RX-HOST-QUALIFICATION-RECEIPT-v2", self).map_err(|e| e.to_string())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfiguredPolicy {
    pub context: configuration::AppliedPolicy,
    pub request_digest: Digest,
    pub receipt_digest: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub schema: Name,
    pub snapshot: rx_domain::host_configuration::Snapshot,
    pub configured: BTreeMap<Name, ConfiguredPolicy>,
    pub accepted: Vec<legacy::AcceptedCell>,
    pub policies: BTreeMap<Name, AcceptedPolicy>,
    pub receipt: Option<Receipt>,
    pub receipt_matches_current_host: bool,
    pub activation_authorized: bool,
}
impl Observation {
    fn facts(&self) -> legacy::Observation {
        let mut facts = legacy::Observation {
            schema: Name::new("rx.host-qualification-observation.v1").expect("static schema"),
            snapshot: self.snapshot.clone(),
            receipt: self.receipt.as_ref().map(|r| r.context.clone()),
            accepted: self.accepted.clone(),
            receipt_matches_current_host: false,
            activation_authorized: false,
        };
        facts.receipt_matches_current_host = facts.current();
        facts
    }
    fn policy_current(&self, cell: &Name, accepted: &AcceptedPolicy) -> bool {
        self.configured.get(cell).is_some_and(|c| {
            c.context.publication == accepted.binding.publication
                && c.context.policy == accepted.binding.policy
                && c.request_digest == accepted.binding.configuration_request
                && c.receipt_digest == accepted.binding.configuration_receipt
                && self.accepted.iter().any(|a| {
                    &a.cell == cell
                        && a.request == accepted.request
                        && a.qualification == accepted.qualification
                        && a.qualification_revision == accepted.qualification_revision
                        && a.acceptance_sequence == accepted.acceptance_sequence
                        && a.context_request == c.context.request
                        && a.context_sequence == c.context.receipt_sequence
                        && a.configuration == c.context.configuration
                })
        })
    }
    pub fn current(&self) -> bool {
        self.facts().current()
            && self.receipt.as_ref().is_some_and(|r| {
                r.policies.iter().all(|(cell, p)| {
                    self.policies.get(cell) == Some(p) && self.policy_current(cell, p)
                })
            })
    }
    pub fn validate(&self) -> Result<(), String> {
        bounded(self)?;
        if self.schema.as_str() != OBSERVATION_SCHEMA
            || self.activation_authorized
            || self.configured.len() > 64
            || self.policies.len() > 64
        {
            return Err("v2 qualification observation shape".into());
        }
        self.facts().validate()?;
        if let Some(r) = &self.receipt {
            r.validate()?;
        }
        configuration::Observation {
            schema: Name::new(configuration::OBSERVATION_SCHEMA).expect("static schema"),
            snapshot: self.snapshot.clone(),
            policies: self
                .configured
                .iter()
                .map(|(c, p)| (c.clone(), p.context.clone()))
                .collect(),
            receipt: None,
            context_matches_current_host: false,
            activation_authorized: false,
        }
        .validate()?;
        for configured in self.configured.values() {
            nonzero(configured.request_digest)?;
            nonzero(configured.receipt_digest)?;
        }
        for (cell, accepted) in &self.policies {
            accepted.binding.validate()?;
            if accepted.qualification_revision.0 == 0
                || accepted.acceptance_sequence.0 == 0
                || !self.accepted.iter().any(|a| {
                    &a.cell == cell
                        && a.request == accepted.request
                        && a.qualification == accepted.qualification
                        && a.qualification_revision == accepted.qualification_revision
                        && a.acceptance_sequence == accepted.acceptance_sequence
                })
            {
                return Err("v2 qualification stamp lacks acceptance facts".into());
            }
        }
        if self.receipt_matches_current_host != self.current() {
            return Err("v2 qualification currency claim differs".into());
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<Digest, String> {
        self.validate()?;
        canonical::digest("RX-QUALIFICATION-OBSERVATION-v2", self).map_err(|e| e.to_string())
    }
}
