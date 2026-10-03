//! Explicit v2 acceptance of input policies. V1 context facts confer no v2 rights.
use super::*;
use rx_domain::host_configuration as legacy;
pub const REQUEST_SCHEMA: &str = "rx.host-execution-configuration-request.v2";
pub const OBSERVATION_SCHEMA: &str = "rx.host-execution-configuration-observation.v2";
pub const RECEIPT_SCHEMA: &str = "rx.host-execution-configuration-receipt.v2";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub manifest: Digest,
    pub signature: Digest,
    pub catalog: ArtifactRef,
    pub template: Name,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellPolicy {
    pub publication: Reference,
    pub reference: ArtifactRef,
    pub policy: Policy,
    pub packages: BTreeMap<Name, Package>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema: Name,
    /// Reused scope/fence facts; this embedded record is not sent on the v1 service.
    pub context: legacy::Request,
    pub policies: BTreeMap<Name, CellPolicy>,
}
impl Request {
    pub fn validate(&self) -> Result<(), String> {
        self.context.validate()?;
        if self.schema.as_str() != REQUEST_SCHEMA
            || self.policies.is_empty()
            || canonical::bytes(self).map_err(|e| e.to_string())?.len() > 1_000_000
        {
            return Err("v2 Host request schema or size".into());
        }
        let required = self
            .context
            .cells
            .iter()
            .filter(|c| c.recipe.schema_id.as_str() == PLAN_SCHEMA)
            .map(|c| &c.cell)
            .collect::<BTreeSet<_>>();
        if self.policies.keys().collect::<BTreeSet<_>>() != required {
            return Err("v2 Host policy coverage differs".into());
        }
        for target in &self.context.cells {
            let Some(input) = self.policies.get(&target.cell) else {
                continue;
            };
            if target.environment.as_str() != "SIMULATION"
                || input.publication.revision != Counter(1)
                || input.publication.catalog != input.policy.workflow.catalog
            {
                return Err("v2 Host environment/publication".into());
            }
            nonzero(input.publication.digest)?;
            input.policy.validate()?;
            reference(&input.reference, POLICY_SCHEMA, MAX_POLICY_BYTES as u64)?;
            verify_artifact(
                &canonical::bytes(&input.policy).map_err(|e| e.to_string())?,
                &input.reference,
                MAX_POLICY_BYTES,
            )?;
            let local = input
                .policy
                .templates
                .iter()
                .filter(|(_, t)| t.host == self.context.host)
                .collect::<BTreeMap<_, _>>();
            if local.keys().copied().collect::<BTreeSet<_>>()
                != input.packages.keys().collect::<BTreeSet<_>>()
            {
                return Err("v2 Host template/package scope differs".into());
            }
            let intents = local
                .values()
                .map(|t| t.intent.digest().map_err(|e| e.to_string()))
                .collect::<Result<BTreeSet<_>, _>>()?;
            if intents != target.required_intents.iter().copied().collect() {
                return Err("v2 Host baseline template set differs".into());
            }
            for package in input.packages.values() {
                nonzero(package.manifest)?;
                nonzero(package.signature)?;
                reference(
                    &package.catalog,
                    TEMPLATE_CATALOG_SCHEMA,
                    MAX_POLICY_BYTES as u64,
                )?;
            }
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<Digest, String> {
        self.validate()?;
        // The inner v1 digest canonicalizes its scope sets while v2 binds the policies.
        canonical::digest(
            "RX-HOST-EXECUTION-CONFIGURATION-v2",
            &(self.context.digest()?, &self.policies),
        )
        .map_err(|e| e.to_string())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppliedPolicy {
    pub publication: Reference,
    pub policy: ArtifactRef,
    pub request: Id,
    pub receipt_sequence: Counter,
    pub configuration: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub schema: Name,
    pub request: Request,
    pub request_digest: Digest,
    pub context: legacy::Receipt,
    pub policies: BTreeMap<Name, AppliedPolicy>,
}
impl Receipt {
    pub fn digest(&self) -> Result<Digest, String> {
        self.validate()?;
        canonical::digest("RX-HOST-CONFIGURATION-RECEIPT-v2", self).map_err(|e| e.to_string())
    }
    pub fn validate(&self) -> Result<(), String> {
        self.context.validate()?;
        if self.schema.as_str() != RECEIPT_SCHEMA
            || self.request_digest != self.request.digest()?
            || self.context.request_digest != self.request.context.digest()?
        {
            return Err("v2 receipt request correlation differs".into());
        }
        if self.context.status == legacy::Status::NotApplied {
            if !self.policies.is_empty() {
                return Err("not-applied receipt cannot accept v2 policies".into());
            }
            return Ok(());
        }
        if self.policies.keys().ne(self.request.policies.keys()) {
            return Err("v2 acceptance policy coverage differs".into());
        }
        for (cell, source) in &self.request.policies {
            let accepted = &self.policies[cell];
            let target = self
                .request
                .context
                .cells
                .iter()
                .find(|c| &c.cell == cell)
                .ok_or("missing target")?;
            if accepted.publication != source.publication
                || accepted.policy != source.reference
                || accepted.request != self.request.context.id
                || accepted.receipt_sequence != self.context.sequence
                || accepted.configuration != target.after_configuration
            {
                return Err(
                    "v2 accepted policy differs from requested publication/configuration".into(),
                );
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub schema: Name,
    pub snapshot: legacy::Snapshot,
    pub policies: BTreeMap<Name, AppliedPolicy>,
    pub receipt: Option<Receipt>,
    pub context_matches_current_host: bool,
    pub activation_authorized: bool,
}
impl Observation {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema.as_str() != OBSERVATION_SCHEMA
            || self.activation_authorized
            || self.policies.len() > 64
        {
            return Err("v2 Host observation schema/scope".into());
        }
        if let Some(receipt) = &self.receipt {
            receipt.validate()?;
        }
        // Use the frozen v1 validator for unchanged facts without copying its currency rules.
        let mut facts = legacy::Observation {
            schema: Name::new("rx.host-process-configuration-observation.v1")
                .expect("static schema"),
            snapshot: self.snapshot.clone(),
            receipt: self.receipt.as_ref().map(|r| r.context.clone()),
            context_matches_current_host: false,
            activation_authorized: false,
        };
        let facts_current = if facts.validate().is_ok() {
            false
        } else {
            facts.context_matches_current_host = true;
            facts.validate()?;
            true
        };
        for (cell, accepted) in &self.policies {
            nonzero(accepted.publication.digest)?;
            if accepted.publication.revision != Counter(1) {
                return Err("v2 observed publication revision differs".into());
            }
            reference(&accepted.policy, POLICY_SCHEMA, MAX_POLICY_BYTES as u64)?;
            if !self.snapshot.cells.iter().any(|c| {
                &c.cell == cell
                    && c.applied.as_ref().is_some_and(|a| {
                        a.request == accepted.request
                            && a.receipt_sequence == accepted.receipt_sequence
                            && a.configuration == accepted.configuration
                    })
            }) {
                return Err("v2 policy observation lacks matching applied context".into());
            }
        }
        let current = facts_current
            && self.receipt.as_ref().is_some_and(|r| {
                r.policies
                    .iter()
                    .all(|(cell, accepted)| self.policies.get(cell) == Some(accepted))
            });
        if current != self.context_matches_current_host {
            return Err("v2 Host currency claim differs".into());
        }
        Ok(())
    }
}
