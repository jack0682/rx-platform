//! Explicit signed package declaration for v2 Program input substitution.
//! A legacy fixed-operation catalog never implies this declaration.
use super::*;
pub const TEMPLATE_CATALOG_SCHEMA: &str = "rx.execution-template-catalog.v2";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateDocument {
    pub path: String,
    pub artifact: ArtifactRef,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateDeclaration {
    pub action: ActionBinding,
    pub contract: NodeContract,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateCatalog {
    pub schema: Name,
    pub installation: Id,
    pub cell: Name,
    pub environment: crate::device_catalog::Environment,
    pub templates: BTreeMap<Name, TemplateDeclaration>,
    pub documents: BTreeMap<Name, TemplateDocument>,
}
impl TemplateCatalog {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema.as_str() != TEMPLATE_CATALOG_SCHEMA
            || self.environment != crate::device_catalog::Environment::Simulation
            || self.templates.is_empty()
            || self.templates.len() > MAX_NODES
            || self
                .documents
                .keys()
                .map(Name::as_str)
                .collect::<BTreeSet<_>>()
                != BTreeSet::from(["family", "profile", "adapter"])
            || self
                .documents
                .values()
                .map(|d| &d.path)
                .collect::<BTreeSet<_>>()
                .len()
                != 3
            || canonical::bytes(self).map_err(|e| e.to_string())?.len() > MAX_POLICY_BYTES
        {
            return Err("execution template catalog schema, scope or bounds differ".into());
        }
        for document in self.documents.values() {
            nonzero(document.artifact.sha256)?;
            if document.path.is_empty()
                || document.path.len() > 240
                || document.artifact.size_bytes.0 == 0
                || document.artifact.size_bytes.0 > 2 * 1024 * 1024
            {
                return Err("template source document bound".into());
            }
        }
        for declaration in self.templates.values() {
            declaration.contract.validate()?;
            let intent = &declaration.action.intent;
            nonzero(intent.profile_digest)?;
            nonzero(intent.site_config_digest)?;
            for digest in &intent.calibration_digests {
                nonzero(*digest)?;
            }
            if intent.kind != Kind::FiniteAction
                || canonical::bytes(intent).map_err(|e| e.to_string())?
                    != canonical::bytes(&intent.normalized().map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())?
            {
                return Err("normalized finite Program template required".into());
            }
            let Body::Program(goal) = &intent.body else {
                return Err("Program template required".into());
            };
            nonzero(goal.program.sha256)?;
            if goal.program.size_bytes.0 == 0 {
                return Err("empty program artifact".into());
            }
            reference(&goal.parameter_set, PARAMETER_SCHEMA, MAX_PARAMETER_BYTES)?;
        }
        Ok(())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let value: Self = decode_canonical(bytes, MAX_POLICY_BYTES)?;
        value.validate()?;
        Ok(value)
    }
}
