use crate::{DomainError, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Name(String);

impl Name {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let s = value.into();
        if s.is_empty()
            || s.len() > 128
            || !s.as_bytes()[0].is_ascii_alphanumeric()
            || !s
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._/-".contains(&c))
        {
            return Err(DomainError::InvalidInput("invalid Name".into()));
        }
        Ok(Self(s))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for Name {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self> {
        Self::new(s)
    }
}
impl From<Name> for String {
    fn from(s: Name) -> String {
        s.0
    }
}
impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Id(String);
impl Id {
    pub fn new(s: impl Into<String>) -> Result<Self> {
        let s = s.into();
        let parsed = uuid::Uuid::parse_str(&s)
            .map_err(|_| DomainError::InvalidInput("invalid UUID".into()))?;
        if parsed.hyphenated().to_string() != s {
            return Err(DomainError::InvalidInput(
                "UUID must be lowercase hyphenated".into(),
            ));
        }
        Ok(Self(s))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for Id {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self> {
        Self::new(s)
    }
}
impl From<Id> for String {
    fn from(s: Id) -> String {
        s.0
    }
}
impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Digest([u8; 32]);
impl Digest {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}
impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}
impl Serialize for Digest {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}
impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if s.len() != 64
            || !s
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(D::Error::custom(
                "Digest requires 64 lowercase hex characters",
            ));
        }
        let mut bytes = [0; 32];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(D::Error::custom)?;
        }
        Ok(Self(bytes))
    }
}

/// Counter/DurationMs is a decimal string on the JSON wire, including above 2^53.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Counter(pub u64);
impl Serialize for Counter {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.0.to_string())
    }
}
impl<'de> Deserialize<'de> for Counter {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if s.is_empty()
            || (s.len() > 1 && s.starts_with('0'))
            || !s.bytes().all(|c| c.is_ascii_digit())
        {
            return Err(D::Error::custom(
                "expected canonical unsigned decimal string",
            ));
        }
        s.parse::<u64>().map(Self).map_err(D::Error::custom)
    }
}
impl Counter {
    pub fn nonzero(self, field: &str) -> Result<Self> {
        if self.0 == 0 {
            return Err(DomainError::InvalidInput(format!(
                "{field} must be positive"
            )));
        }
        Ok(self)
    }
    pub fn increment(self) -> Result<Self> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or_else(|| DomainError::InvalidTransition("counter overflow".into()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Integer(pub i64);
impl Serialize for Integer {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.0.to_string())
    }
}
impl<'de> Deserialize<'de> for Integer {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let n = s.parse::<i64>().map_err(D::Error::custom)?;
        if n.to_string() != s {
            return Err(D::Error::custom("non-canonical signed decimal"));
        }
        Ok(Self(n))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Real(f64);
impl Real {
    pub fn new(value: f64) -> Result<Self> {
        if !value.is_finite() {
            return Err(DomainError::InvalidInput("non-finite real".into()));
        }
        Ok(Self(if value == 0.0 { 0.0 } else { value }))
    }
    pub fn get(self) -> f64 {
        self.0
    }
}
impl Serialize for Real {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_f64(self.0)
    }
}
impl<'de> Deserialize<'de> for Real {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Self::new(f64::deserialize(d)?).map_err(D::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimePoint {
    pub clock_id: String,
    pub ticks_ns: Counter,
}
impl TimePoint {
    pub fn age_ns(&self, older: &Self) -> Option<u64> {
        if self.clock_id.is_empty() || self.clock_id != older.clock_id {
            return None;
        }
        self.ticks_ns.0.checked_sub(older.ticks_ns.0)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRef {
    pub sha256: Digest,
    pub schema_id: Name,
    pub size_bytes: Counter,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RealVector {
    pub values: Vec<Real>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum TypedValue {
    Boolean(bool),
    Integer(Integer),
    Real(Real),
    Symbol(Name),
    Reals(RealVector),
}
