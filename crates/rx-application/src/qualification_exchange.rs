//! Versioned durable exchange values. V1 serialization stays flat and byte-identical.
use rx_domain::{canonical, host_qualification as v1, types::*};
use rx_process_contract::execution_v2::host_qualification as v2;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Request {
    V2(Box<v2::Request>),
    V1(Box<v1::Request>),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Receipt {
    V2(Box<v2::Receipt>),
    V1(Box<v1::Receipt>),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Observation {
    V2(Box<v2::Observation>),
    V1(Box<v1::Observation>),
}
impl From<v1::Request> for Request {
    fn from(v: v1::Request) -> Self {
        Self::V1(Box::new(v))
    }
}
impl From<v2::Request> for Request {
    fn from(v: v2::Request) -> Self {
        Self::V2(Box::new(v))
    }
}
impl From<v1::Observation> for Observation {
    fn from(v: v1::Observation) -> Self {
        Self::V1(Box::new(v))
    }
}
impl From<v2::Observation> for Observation {
    fn from(v: v2::Observation) -> Self {
        Self::V2(Box::new(v))
    }
}
impl Request {
    pub fn context(&self) -> &v1::Request {
        match self {
            Self::V1(v) => v,
            Self::V2(v) => &v.context,
        }
    }
    pub fn is_v2(&self) -> bool {
        matches!(self, Self::V2(_))
    }
    pub fn digest(&self) -> Result<Digest, String> {
        match self {
            Self::V1(v) => v.digest(),
            Self::V2(v) => v.digest(),
        }
    }
}
impl Receipt {
    pub fn context(&self) -> &v1::Receipt {
        match self {
            Self::V1(v) => v,
            Self::V2(v) => &v.context,
        }
    }
    pub fn is_v2(&self) -> bool {
        matches!(self, Self::V2(_))
    }
    pub fn request_digest(&self) -> Digest {
        match self {
            Self::V1(v) => v.request_digest,
            Self::V2(v) => v.request_digest,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::V1(v) => v.validate(),
            Self::V2(v) => v.validate(),
        }
    }
    pub fn digest(&self) -> Result<Digest, String> {
        canonical::digest(
            if self.is_v2() {
                "RX-HOST-QUALIFICATION-RECEIPT-v2"
            } else {
                "RX-HOST-QUALIFICATION-RECEIPT-v1"
            },
            self,
        )
        .map_err(|e| e.to_string())
    }
}
impl Observation {
    pub fn accepted(&self) -> &[v1::AcceptedCell] {
        match self {
            Self::V1(v) => &v.accepted,
            Self::V2(v) => &v.accepted,
        }
    }
    pub fn snapshot(&self) -> &rx_domain::host_configuration::Snapshot {
        match self {
            Self::V1(v) => &v.snapshot,
            Self::V2(v) => &v.snapshot,
        }
    }
    pub fn is_v2(&self) -> bool {
        matches!(self, Self::V2(_))
    }
    pub fn receipt(&self) -> Option<Receipt> {
        match self {
            Self::V1(v) => v.receipt.clone().map(|r| Receipt::V1(Box::new(r))),
            Self::V2(v) => v.receipt.clone().map(|r| Receipt::V2(Box::new(r))),
        }
    }
    pub fn receipt_matches_current_host(&self) -> bool {
        match self {
            Self::V1(v) => v.receipt_matches_current_host,
            Self::V2(v) => v.receipt_matches_current_host,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::V1(v) => v.validate(),
            Self::V2(v) => v.validate(),
        }
    }
    pub fn digest(&self) -> Result<Digest, String> {
        canonical::digest(
            if self.is_v2() {
                "RX-QUALIFICATION-OBSERVATION-v2"
            } else {
                "RX-QUALIFICATION-OBSERVATION-v1"
            },
            self,
        )
        .map_err(|e| e.to_string())
    }
}
