//! Versioned envelope: the contained v1 graph is a template, never a v1 executable recipe.
use super::*;
use crate::{CompiledBody, ResolvedProcess};
pub const BINDING_SCHEMA: &str = "rx.workflow-execution-binding.v2";
pub const PLAN_SCHEMA: &str = "rx.execution-plan.v2";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub schema: Name,
    pub publication: Reference,
    pub policy: ArtifactRef,
    /// Compiled operation ID -> published workflow node. Complete and one-to-one.
    pub nodes: BTreeMap<Name, Name>,
}
impl Binding {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema.as_str() != BINDING_SCHEMA
            || self.publication.revision != Counter(1)
            || self.nodes.is_empty()
            || self.nodes.len() > MAX_NODES
            || self.nodes.values().collect::<BTreeSet<_>>().len() != self.nodes.len()
        {
            return Err("execution binding schema, revision or node map differs".into());
        }
        nonzero(self.publication.digest)?;
        reference(&self.policy, POLICY_SCHEMA, MAX_POLICY_BYTES as u64)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub schema: Name,
    pub binding: Binding,
    pub process: ResolvedProcess,
}
impl Plan {
    pub fn validate(&self) -> Result<(), String> {
        self.binding.validate()?;
        crate::validation::validate(&self.process)?;
        let CompiledBody::Sequence { children } = &self.process.root.body else {
            return Err("v2 profile requires a finite sequence".into());
        };
        if self.schema.as_str() != PLAN_SCHEMA
            || children.len() != self.binding.nodes.len()
            || self.process.bindings.len() != children.len()
            || !self.process.conditions.is_empty()
        {
            return Err("v2 plan shape or count differs".into());
        }
        let mut used = BTreeSet::new();
        for child in children {
            let CompiledBody::Operation { binding } = &child.body else {
                return Err("v2 profile requires one operation per workflow node".into());
            };
            if !self.binding.nodes.contains_key(&child.id) || !used.insert(binding) {
                return Err("compiled node map is incomplete or reused".into());
            }
        }
        if canonical::bytes(self).map_err(|e| e.to_string())?.len() > canonical::MAX_MESSAGE_BYTES {
            return Err("v2 plan exceeds 1 MiB".into());
        }
        Ok(())
    }
    pub fn reference(&self) -> Result<ArtifactRef, String> {
        self.validate()?;
        let bytes = canonical::bytes(self).map_err(|e| e.to_string())?;
        Ok(ArtifactRef {
            schema_id: Name::new(PLAN_SCHEMA).expect("static schema"),
            sha256: Digest::from_bytes(Sha256::digest(&bytes).into()),
            size_bytes: Counter(bytes.len() as u64),
        })
    }
    pub fn verify_policy(&self, policy: &Policy, order: &[Name]) -> Result<(), String> {
        self.validate()?;
        policy.validate()?;
        verify_artifact(
            &canonical::bytes(policy).map_err(|e| e.to_string())?,
            &self.binding.policy,
            MAX_POLICY_BYTES,
        )?;
        let CompiledBody::Sequence { children } = &self.process.root.body else {
            unreachable!("validated sequence")
        };
        if children.len() != order.len() || children.len() != policy.templates.len() {
            return Err("published workflow node count differs".into());
        }
        for (child, node) in children.iter().zip(order) {
            let CompiledBody::Operation { binding } = &child.body else {
                unreachable!("validated operation")
            };
            if self.binding.nodes.get(&child.id) != Some(node)
                || canonical::bytes(&self.process.bindings[binding]).map_err(|e| e.to_string())?
                    != canonical::bytes(policy.templates.get(node).ok_or("unknown workflow node")?)
                        .map_err(|e| e.to_string())?
            {
                return Err("v2 graph order or template differs from publication".into());
            }
        }
        Ok(())
    }
}
