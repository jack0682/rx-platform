//! Declarative investigation instructions. No physical procedure or authority is granted here.
use rx_domain::{canonical, types::*};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;

pub const SCHEMA: &str = "rx.investigation-procedure.v1";
pub const MAX_BYTES: usize = 65_536;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Action {
    AbandonInvestigation,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Procedure {
    pub schema: Name,
    pub id: Name,
    pub revision: Counter,
    pub title: String,
    pub instructions: Vec<String>,
    pub cell: Name,
    pub definition: Digest,
    pub environment: Name,
    pub profiles: BTreeSet<Digest>,
    pub action: Action,
}
impl Procedure {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema.as_str() != SCHEMA
            || self.revision.0 == 0
            || self.title.trim().is_empty()
            || self.title.len() > 256
            || self.instructions.is_empty()
            || self.instructions.len() > 32
            || self
                .instructions
                .iter()
                .any(|s| s.trim().is_empty() || s.len() > 2048)
            || self.profiles.is_empty()
            || self.profiles.len() > 128
            || !matches!(self.environment.as_str(), "SIMULATION" | "PHYSICAL")
            || canonical::bytes(self).map_err(|e| e.to_string())?.len() > MAX_BYTES
        {
            return Err("investigation procedure shape/scope differs".into());
        }
        Ok(())
    }
    pub fn reference(&self) -> Result<ArtifactRef, String> {
        self.validate()?;
        let bytes = canonical::bytes(self).map_err(|e| e.to_string())?;
        Ok(ArtifactRef {
            sha256: Digest::from_bytes(Sha256::digest(&bytes).into()),
            schema_id: Name::new(SCHEMA).map_err(|e| e.to_string())?,
            size_bytes: Counter(bytes.len() as u64),
        })
    }
    pub fn signing_message(&self, key: &Name) -> Result<Vec<u8>, String> {
        Ok(format!(
            "RX-INVESTIGATION-PROCEDURE-SIGNATURE-v1\n{key}\n{}",
            self.reference()?.sha256
        )
        .into_bytes())
    }
}
