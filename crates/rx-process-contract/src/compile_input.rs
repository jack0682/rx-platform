//! Matched authoring inputs for offline compilation. This is not a signed package or execution permit.
use crate::model::{ActionBinding, ProcessSource};
use rx_domain::{canonical, types::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileInput {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub device_sources: BTreeMap<Name, DeviceSource>,
    pub schema: Name,
    pub draft: Id,
    pub cell: Name,
    pub source_revision: Counter,
    pub binding_revision: Counter,
    pub source_document_digest: Digest,
    pub bindings_digest: Digest,
    pub catalog_digest: Digest,
    pub source: serde_json::Value,
    pub bindings: BTreeMap<Name, ActionBinding>,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingPlanRef {
    pub id: Id,
    pub revision: Counter,
    pub plan_digest: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceSource {
    pub plan: BindingPlanRef,
    pub binding: Name,
    pub step_digest: Digest,
    pub action_digest: Digest,
}
pub fn device_action_digest(action: &ActionBinding) -> Result<Digest, String> {
    canonical::digest("RX-COMPILER-DEVICE-ACTION-v1", action).map_err(|e| e.to_string())
}
pub fn bindings_digest(
    bindings: &BTreeMap<Name, ActionBinding>,
    sources: &BTreeMap<Name, DeviceSource>,
) -> Result<Digest, String> {
    if sources.is_empty() {
        canonical::digest("RX-DRAFT-COMPILE-BINDINGS-v1", bindings)
    } else {
        canonical::digest("RX-DRAFT-COMPILE-BINDINGS-v2", &(bindings, sources))
    }
    .map_err(|e| e.to_string())
}
impl CompileInput {
    pub fn validate(&self) -> Result<ProcessSource, String> {
        if !matches!(
            (self.schema.as_str(), self.device_sources.is_empty()),
            ("rx.process-compile-input.v1", true) | ("rx.process-compile-input.v2", false)
        ) || self.source_revision.0 == 0
            || self.binding_revision.0 == 0
            || self.device_sources.len() > 128
        {
            return Err("compile input schema/revision".into());
        }
        if canonical::digest("RX-PROCESS-DRAFT-DOCUMENT-v1", &self.source)
            .map_err(|e| e.to_string())?
            != self.source_document_digest
            || bindings_digest(&self.bindings, &self.device_sources)? != self.bindings_digest
        {
            return Err("compile input integrity differs".into());
        }
        let mut plans = std::collections::BTreeMap::new();
        for (alias, source) in &self.device_sources {
            if source.plan.revision.0 == 0
                || device_action_digest(
                    self.bindings
                        .get(alias)
                        .ok_or("device provenance alias missing")?,
                )? != source.action_digest
                || plans
                    .insert(&source.plan.id, &source.plan)
                    .is_some_and(|old| old != &source.plan)
            {
                return Err("device binding provenance differs".into());
            }
        }
        if plans.len() > 16 {
            return Err("too many device binding plans".into());
        }
        canonical::decode_json(&canonical::bytes(&self.source).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn altered_bundle_source_is_not_accepted_under_the_original_digest() {
        let n = |s: &str| Name::new(s).unwrap();
        let id = Id::new("00000000-0000-4000-8000-000000000001").unwrap();
        let source = serde_json::json!({"schema":"rx.process-source.v1","process":"demo","entry":"main","conditions":{},"flows":[]});
        let bindings = BTreeMap::new();
        let mut input = CompileInput {
            device_sources: BTreeMap::new(),
            schema: n("rx.process-compile-input.v1"),
            draft: id,
            cell: n("cell/a"),
            source_revision: Counter(1),
            binding_revision: Counter(1),
            source_document_digest: canonical::digest("RX-PROCESS-DRAFT-DOCUMENT-v1", &source)
                .unwrap(),
            bindings_digest: canonical::digest("RX-DRAFT-COMPILE-BINDINGS-v1", &bindings).unwrap(),
            catalog_digest: Digest::from_bytes([1; 32]),
            source,
            bindings,
        };
        assert!(input.validate().is_ok());
        input.source["process"] = serde_json::json!("changed");
        assert!(input.validate().is_err());
    }
}
