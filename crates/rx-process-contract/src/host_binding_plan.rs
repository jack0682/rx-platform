//! Proposed Host binding inputs. This artifact is neither an install request nor an authority grant.
use rx_domain::{canonical, intent::Intent, types::*};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DevicePackage {
    pub manifest: Digest,
    pub signature: Digest,
    pub catalog: ArtifactRef,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostTarget {
    pub required_intents: Vec<Intent>,
    pub required_conditions: BTreeSet<Name>,
    pub device_packages: Vec<DevicePackage>,
    pub other_affected_cells: BTreeSet<Name>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub schema: Name,
    pub installation: Id,
    pub cell: Name,
    pub device_context_digest: Digest,
    pub process_review_digest: Digest,
    pub before_configuration: ArtifactRef,
    pub after_configuration: ArtifactRef,
    pub definition: ArtifactRef,
    pub envelope: ArtifactRef,
    pub environment: Name,
    pub scopes: BTreeSet<Name>,
    pub hosts: BTreeMap<Name, HostTarget>,
}
impl Plan {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema.as_str() != "rx.host-binding-plan.v1"
            || self.hosts.is_empty()
            || self.hosts.len() > 64
            || self.scopes.is_empty()
            || self.scopes.len() > 128
            || !matches!(self.environment.as_str(), "SIMULATION" | "PHYSICAL")
            || [
                &self.before_configuration,
                &self.after_configuration,
                &self.definition,
                &self.envelope,
            ]
            .iter()
            .any(|v| v.size_bytes.0 == 0)
            || canonical::bytes(self).map_err(|e| e.to_string())?.len() > 1_000_000
        {
            return Err("Host binding plan scope/size".into());
        }
        for host in self.hosts.values() {
            if host.required_intents.is_empty()
                || host.required_intents.len() > 4096
                || host.required_conditions.is_empty()
                || host.required_conditions.len() > 128
                || host.device_packages.len() > 16
                || host.other_affected_cells.len() > 63
                || host.other_affected_cells.contains(&self.cell)
            {
                return Err("Host target scope".into());
            }
            let digests = host
                .required_intents
                .iter()
                .map(|i| i.digest().map_err(|e| e.to_string()))
                .collect::<Result<Vec<_>, _>>()?;
            if digests.windows(2).any(|w| w[0] >= w[1])
                || host
                    .device_packages
                    .windows(2)
                    .any(|w| (w[0].manifest, w[0].signature) >= (w[1].manifest, w[1].signature))
            {
                return Err("Host targets must be unique and ordered".into());
            }
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<Digest, String> {
        self.validate()?;
        canonical::digest("RX-HOST-BINDING-PLAN-v1", self).map_err(|e| e.to_string())
    }
}
