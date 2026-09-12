use rx_domain::types::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PackagePath(String);
impl PackagePath {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.is_empty() || value.len() > 240 || !value.is_ascii() || value.contains(['\\', ':'])
        {
            return Err("invalid package path".into());
        }
        for component in value.split('/') {
            if component.is_empty()
                || component == "."
                || component == ".."
                || component.len() > 100
                || component.ends_with('.')
                || !component
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            {
                return Err("invalid package path component".into());
            }
            let base = component
                .split('.')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            if ["con", "prn", "aux", "nul"].contains(&base.as_str())
                || (base.len() == 4
                    && (base.starts_with("com") || base.starts_with("lpt"))
                    && matches!(base.as_bytes()[3], b'1'..=b'9'))
            {
                return Err("reserved platform filename".into());
            }
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for PackagePath {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        Self::new(s)
    }
}
impl From<PackagePath> for String {
    fn from(value: PackagePath) -> String {
        value.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PackageKind {
    Device,
    Process,
    Ui,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OperatingSystem {
    Linux,
    Windows,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Architecture {
    Amd64,
    Arm64,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub os: OperatingSystem,
    pub architecture: Architecture,
    pub ros_distribution: Option<Name>,
}
impl Target {
    /// None declares no ROS runtime requirement; it is not a prohibition on ROS in the image.
    pub fn supports(&self, environment: &Self) -> bool {
        self.os == environment.os
            && self.architecture == environment.architecture
            && (self.ros_distribution.is_none()
                || self.ros_distribution == environment.ros_distribution)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum Permission {
    ArtifactRead,
    ObservationRead { schema: Name },
    OperationSubmit { operation: Name },
    NativeEndpoint { role: Name },
    UiPanelRead { topic: Name },
}
impl Permission {
    pub fn allowed_for(&self, kind: PackageKind) -> bool {
        match self {
            Self::ArtifactRead => true,
            Self::ObservationRead { .. } => {
                matches!(kind, PackageKind::Device | PackageKind::Process)
            }
            Self::OperationSubmit { .. } => kind == PackageKind::Process,
            Self::NativeEndpoint { .. } => kind == PackageKind::Device,
            Self::UiPanelRead { .. } => kind == PackageKind::Ui,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileEntry {
    pub path: PackagePath,
    pub sha256: Digest,
    pub size_bytes: Counter,
    pub executable: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependency {
    pub package: Name,
    pub version: semver::Version,
    pub manifest_digest: Digest,
    pub kind: PackageKind,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractSet {
    pub base: Digest,
    pub cell: Digest,
    pub package_abi: Name,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum EntryPoint {
    Device {
        family: PackagePath,
        profiles: Vec<PackagePath>,
        adapter: PackagePath,
    },
    /// v2 package only: immutable reference to a release-owned implementation, never executable content.
    DeviceReference {
        family: PackagePath,
        profiles: Vec<PackagePath>,
        adapter: PackagePath,
    },
    Process {
        source: PackagePath,
    },
    Ui {
        panel: PackagePath,
    },
}
impl EntryPoint {
    pub fn kind(&self) -> PackageKind {
        match self {
            Self::Device { .. } | Self::DeviceReference { .. } => PackageKind::Device,
            Self::Process { .. } => PackageKind::Process,
            Self::Ui { .. } => PackageKind::Ui,
        }
    }
    pub fn paths(&self) -> Vec<&PackagePath> {
        match self {
            Self::Device {
                family,
                profiles,
                adapter,
            }
            | Self::DeviceReference {
                family,
                profiles,
                adapter,
            } => std::iter::once(family)
                .chain(profiles)
                .chain(std::iter::once(adapter))
                .collect(),
            Self::Process { source } => vec![source],
            Self::Ui { panel } => vec![panel],
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: Name,
    pub package: Name,
    pub version: semver::Version,
    pub publisher: Name,
    pub contracts: ContractSet,
    pub targets: Vec<Target>,
    pub entry: EntryPoint,
    pub permissions: Vec<Permission>,
    pub dependencies: Vec<Dependency>,
    pub assets: Vec<ArtifactRef>,
    pub files: Vec<FileEntry>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignatureEnvelope {
    pub key: Name,
    pub signature: String,
}
#[derive(Clone)]
pub struct TrustedPublisher {
    pub publisher: Name,
    pub verifying_key: [u8; 32],
    pub kinds: BTreeSet<PackageKind>,
    pub permissions: BTreeSet<Permission>,
}
