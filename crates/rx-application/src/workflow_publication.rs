//! Saved v2 preview and publication data; neither confers qualification or a permit.
use crate::workflow_model::ExecutionSnapshot;
use rx_domain::{canonical, definition::Reference, types::*, workflow};
use rx_process_contract::{ActionBinding, execution_v2 as v2};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateInput {
    pub key: Name,
    pub object_model: Reference,
    pub request: workflow::Request,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewInput {
    pub id: Id,
    pub candidates: Vec<CandidateInput>,
    pub slots: u16,
    pub templates: BTreeMap<Name, ActionBinding>,
    pub node_contracts: BTreeMap<Name, v2::NodeContract>,
}
impl PreviewInput {
    pub fn validate(&self) -> Result<(), String> {
        if self.candidates.is_empty()
            || self.candidates.len() > v2::MAX_VARIANTS
            || self.slots == 0
            || usize::from(self.slots) > v2::MAX_SLOTS
            || canonical::bytes(self).map_err(|e| e.to_string())?.len() > v2::MAX_POLICY_BYTES
        {
            return Err("execution preview input bounds".into());
        }
        Ok(())
    }
}
pub struct PreviewWork {
    pub(crate) input: PreviewInput,
    pub(crate) snapshot: ExecutionSnapshot,
}
pub enum Preparation {
    Recorded(Box<Preview>),
    Pending(Box<PreviewWork>),
}
pub struct PreparedPreview {
    pub(crate) input: PreviewInput,
    pub(crate) policy: v2::Policy,
    pub(crate) inputs: v2::InputClosure,
    pub(crate) index: Vec<u8>,
}
pub(crate) fn artifact(schema: &str, bytes: &[u8]) -> ArtifactRef {
    ArtifactRef {
        schema_id: Name::new(schema).expect("static schema"),
        sha256: rx_package::content_digest(bytes),
        size_bytes: Counter(bytes.len() as u64),
    }
}
impl PreparedPreview {
    /// Stream the whole bounded domain. A failed candidate returns no prepared result.
    pub fn prepare(work: PreviewWork) -> Result<Self, String> {
        let input = work.input;
        input.validate()?;
        let inputs = work.snapshot.input_closure();
        let resolver_digest = work.snapshot.resolve(0, 0)?.resolver_digest;
        let mut policy = v2::Policy {
            schema: Name::new(v2::POLICY_SCHEMA).expect("static schema"),
            workflow: inputs.workflow.clone(),
            definition_closure: inputs.artifact()?,
            resolver_digest,
            compiler_digest: v2::compiler_digest(),
            candidates: input
                .candidates
                .iter()
                .map(|c| {
                    Ok(v2::Candidate {
                        key: c.key.clone(),
                        object_model: c.object_model.clone(),
                        context_digest: v2::context_digest(&c.request)?,
                    })
                })
                .collect::<Result<_, String>>()?,
            slot_order: (0..input.slots).collect(),
            templates: input.templates.clone(),
            node_contracts: input.node_contracts.clone(),
            // Never persisted: report content excludes its containing index.
            report_index: artifact(v2::INDEX_SCHEMA, b"uncommitted"),
        };
        let mut index = v2::ReportIndex {
            schema: Name::new(v2::INDEX_SCHEMA).expect("static schema"),
            entries: Vec::with_capacity(input.candidates.len() * usize::from(input.slots)),
        };
        for candidate in 0..input.candidates.len() {
            for slot in 0..input.slots {
                let generated = v2::materialize(&policy, &inputs, candidate as u8, slot)?;
                index
                    .entries
                    .push((candidate as u8, slot, generated.report_digest()));
            }
        }
        let index = canonical::bytes(&index).map_err(|e| e.to_string())?;
        policy.report_index = artifact(v2::INDEX_SCHEMA, &index);
        v2::ReportIndex::decode(&index, &policy)?;
        Ok(Self {
            input,
            policy,
            inputs,
            index,
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preview {
    pub reference: Reference,
    pub workflow: Reference,
    pub policy: ArtifactRef,
    pub inputs: ArtifactRef,
    pub index: ArtifactRef,
    pub created_by: Name,
    pub created_at: TimePoint,
}
impl Preview {
    pub fn digest(&self) -> Result<Digest, String> {
        canonical::digest(
            "RX-EXECUTION-PREVIEW-v2",
            &(
                &self.reference.catalog,
                &self.reference.id,
                self.reference.revision,
                &self.workflow,
                &self.policy,
                &self.inputs,
                &self.index,
                &self.created_by,
                &self.created_at,
            ),
        )
        .map_err(|e| e.to_string())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Publish {
    pub id: Id,
    pub preview: Reference,
    pub cell: Name,
    pub bindings: BTreeMap<Name, TemplateBinding>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateBinding {
    pub intake: Id,
    pub template: Name,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageEvidence {
    pub object: rx_package::store::ObjectId,
    pub catalog: ArtifactRef,
    pub dependencies: Vec<ArtifactRef>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Publication {
    pub reference: Reference,
    pub preview: Reference,
    pub policy: ArtifactRef,
    pub cell: Name,
    pub bindings: BTreeMap<Name, TemplateBinding>,
    pub packages: BTreeMap<Id, PackageEvidence>,
    pub registration: crate::package_intake::Registration,
    pub published_by: Name,
    pub published_at: TimePoint,
}
impl Publication {
    pub fn digest(&self) -> Result<Digest, String> {
        canonical::digest(
            "RX-WORKFLOW-PUBLICATION-v2",
            &(
                &self.reference.catalog,
                &self.reference.id,
                self.reference.revision,
                &self.preview,
                &self.policy,
                &self.cell,
                &self.bindings,
                &self.packages,
                &self.registration,
                &self.published_by,
                &self.published_at,
            ),
        )
        .map_err(|e| e.to_string())
    }
}
pub enum PublishPreparation {
    Recorded(Box<Publication>),
    Verify(Box<PublishTicket>),
}
pub struct PublishTicket {
    pub(crate) input: Publish,
    pub(crate) identity: crate::Identity,
    pub(crate) key: Id,
    pub(crate) preview: Preview,
    pub(crate) policy: v2::Policy,
    pub(crate) registration: crate::package_intake::Registration,
    pub(crate) receipts: BTreeMap<Id, crate::package_intake::Receipt>,
    pub(crate) configuration_digest: Digest,
    pub(crate) installation: Id,
    pub(crate) boot: Id,
    pub(crate) issued: TimePoint,
    pub(crate) definition_count: usize,
}
impl PublishTicket {
    pub fn registration(&self) -> &crate::package_intake::Registration {
        &self.registration
    }
    pub fn objects(&self) -> BTreeMap<Id, rx_package::store::ObjectId> {
        self.receipts
            .iter()
            .map(|(id, r)| (id.clone(), r.object.clone()))
            .collect()
    }
}
pub struct PreparedPublication {
    pub(crate) ticket: PublishTicket,
    pub(crate) packages: BTreeMap<Id, PackageEvidence>,
}
impl PreparedPublication {
    pub fn verify(
        ticket: PublishTicket,
        packages: BTreeMap<Id, rx_package::store::StoredPackage>,
    ) -> Result<Self, String> {
        if packages.keys().ne(ticket.receipts.keys()) {
            return Err("publication package coverage differs".into());
        }
        let mut verified = BTreeMap::new();
        let mut evidence = BTreeMap::new();
        let mut dependencies = std::collections::BTreeSet::new();
        for (id, stored) in packages {
            let receipt = &ticket.receipts[&id];
            if stored.object() != &receipt.object
                || rx_package::manifest_bytes(stored.package().manifest())
                    .map_err(|e| e.to_string())?
                    != rx_package::manifest_bytes(&receipt.manifest).map_err(|e| e.to_string())?
            {
                return Err("reverified publication package differs from intake".into());
            }
            let proof = crate::execution_templates::VerifiedTemplates::check(
                &stored,
                &ticket.registration,
                &ticket.installation,
                &ticket.input.cell,
            )?;
            for r in proof.dependencies() {
                dependencies.insert((r.sha256, r.schema_id.clone()));
            }
            evidence.insert(
                id.clone(),
                PackageEvidence {
                    object: receipt.object.clone(),
                    catalog: proof.catalog_reference().clone(),
                    dependencies: proof.dependencies().to_vec(),
                },
            );
            verified.insert(id, proof);
        }
        if dependencies.len() + ticket.definition_count + 3 > v2::MAX_DEPENDENCIES {
            return Err("combined publication dependency bound exceeded".into());
        }
        for (node, binding) in &ticket.input.bindings {
            verified
                .get(&binding.intake)
                .ok_or("missing template package")?
                .matches(
                    &binding.template,
                    &ticket.policy.templates[node],
                    &ticket.policy.node_contracts[node],
                )?;
        }
        Ok(Self {
            ticket,
            packages: evidence,
        })
    }
}
/// Loaded saved bytes; CPU recomputation stays outside the writer transaction.
pub struct SavedPreview {
    pub(crate) preview: Preview,
    pub(crate) policy: v2::Policy,
    pub(crate) inputs: v2::InputClosure,
    pub(crate) index: v2::ValidatedIndex,
}
impl SavedPreview {
    pub fn preview(&self) -> &Preview {
        &self.preview
    }
    pub fn policy(&self) -> &v2::Policy {
        &self.policy
    }
    pub fn report(&self, candidate: u8, slot: u16) -> Result<v2::Materialized, String> {
        let generated = v2::materialize(&self.policy, &self.inputs, candidate, slot)?;
        generated.verify_index(&self.policy, &self.index, candidate, slot)?;
        Ok(generated)
    }
}
