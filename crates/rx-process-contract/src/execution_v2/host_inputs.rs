//! Provider-independent membership against material verified and retained by the Host.
//! This value proves content membership, never caller identity, qualification or permission.
use super::*;

/// Immutable common envelope handed to a native provider only after Host admission.
/// Deserialization is not approval; the gate must validate against its own accepted domain.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundInput {
    pub binding: OperationBinding,
    pub parameters: Vec<u8>,
}

#[derive(Clone)]
pub struct DomainMaterial {
    pub policy: Vec<u8>,
    pub inputs: Vec<u8>,
    pub index: Vec<u8>,
}

pub struct VerifiedDomain {
    material: DomainMaterial,
    policy: Policy,
    policy_reference: ArtifactRef,
    inputs: InputClosure,
    index: ValidatedIndex,
}
impl VerifiedDomain {
    /// Import complete immutable material before qualification acknowledgement. The Host must
    /// separately bind this result to its own durable, current qualification acceptance record.
    pub fn verify(policy: &[u8], inputs: &[u8], index: &[u8]) -> Result<Self, String> {
        let material = DomainMaterial {
            policy: policy.to_vec(),
            inputs: inputs.to_vec(),
            index: index.to_vec(),
        };
        let reference = ArtifactRef {
            schema_id: Name::new(POLICY_SCHEMA).expect("static schema"),
            sha256: Digest::from_bytes(Sha256::digest(policy).into()),
            size_bytes: Counter(policy.len() as u64),
        };
        let policy = Policy::decode(policy)?;
        let inputs = InputClosure::decode(inputs, &policy)?;
        let index = ReportIndex::decode(index, &policy)?;
        for candidate in 0..policy.candidates.len() {
            for slot in &policy.slot_order {
                materialize(&policy, &inputs, candidate as u8, *slot)?.verify_index(
                    &policy,
                    &index,
                    candidate as u8,
                    *slot,
                )?;
            }
        }
        Ok(Self {
            material,
            policy,
            policy_reference: reference,
            inputs,
            index,
        })
    }
    pub fn material(&self) -> &DomainMaterial {
        &self.material
    }
    pub fn policy(&self) -> &Policy {
        &self.policy
    }
    pub fn reference(&self) -> &ArtifactRef {
        &self.policy_reference
    }

    /// Caller hashes are insufficient. Regenerate the selected member from this Host-owned
    /// domain and compare actual canonical bytes, then check its immutable operation links.
    /// The gate remains responsible for loading the qualification-linked domain and checking
    /// the authenticated P, Run/cell scope, epoch, permit, replay and existing operation record.
    pub fn verify_operation(
        &self,
        binding: &OperationBinding,
        host: &Name,
        intent: &Intent,
        parameters: &[u8],
    ) -> Result<(), String> {
        binding.validate()?;
        if binding.policy != self.policy_reference {
            return Err("operation policy differs from Host approval material".into());
        }
        let selection = &binding.selection;
        selection.verify_request(
            selection,
            &self.policy,
            &self.index,
            host,
            intent,
            parameters,
        )?;
        let generated = materialize(
            &self.policy,
            &self.inputs,
            selection.candidate,
            selection.slot,
        )?;
        generated.verify_index(
            &self.policy,
            &self.index,
            selection.candidate,
            selection.slot,
        )?;
        verify_artifact(generated.report(), &binding.report, MAX_REPORT_BYTES)?;
        let expected = generated
            .parameters()
            .get(&selection.node)
            .ok_or("unapproved local node")?;
        if expected.as_slice() != parameters {
            return Err("Prepare bytes differ from Host-owned approved member".into());
        }
        let action = generated
            .actions()
            .get(&selection.node)
            .ok_or("unapproved action")?;
        if &action.host != host
            || action.intent.digest().map_err(|e| e.to_string())?
                != intent.digest().map_err(|e| e.to_string())?
        {
            return Err("Prepare Intent differs from Host-owned approved action".into());
        }
        Ok(())
    }
}
