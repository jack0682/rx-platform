//! RX JSON is not ProtoJSON: snake_case, hex Digest, decimal-string uint64,
//! unprefixed enum tokens, explicit empty repeated fields, and no null.
use crate::{DESCRIPTORS, strict};
use prost::{Message, Name};
use prost_reflect::{
    DynamicMessage, FieldDescriptor, Kind, MessageDescriptor, ReflectMessage, Value as PValue,
};
use rx_domain::{
    canonical,
    types::{Counter, Digest, Id, Integer, Name as DomainName, Real},
};
use serde_json::{Map, Value};
use std::{collections::BTreeMap, sync::LazyLock};
use tonic::Status;

static RULES: LazyLock<BTreeMap<String, String>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../proto/semantic_fields.json"))
        .expect("generated semantic rules must be valid")
});
fn invalid(message: impl Into<String>) -> Status {
    Status::invalid_argument(message.into())
}
fn descriptor<M: Name>() -> Result<MessageDescriptor, Status> {
    DESCRIPTORS
        .get_message_by_name(&M::full_name())
        .ok_or_else(|| Status::internal("descriptor absent"))
}
pub fn from_slice<M: Message + Default + Name>(bytes: &[u8]) -> Result<M, Status> {
    let value: Value = canonical::decode_json(bytes).map_err(|e| invalid(e.to_string()))?;
    let message = from_value(descriptor::<M>()?, &value, 0)?;
    strict::decode(&message.encode_to_vec())
}
pub fn to_value<M: Message + Name>(message: &M) -> Result<Value, Status> {
    let bytes = message.encode_to_vec();
    let desc = descriptor::<M>()?;
    strict::validate(&desc, &bytes)?;
    let dynamic =
        DynamicMessage::decode(desc, bytes.as_slice()).map_err(|e| invalid(e.to_string()))?;
    project(&dynamic)
}
pub fn to_vec<M: Message + Name>(message: &M) -> Result<Vec<u8>, Status> {
    canonical::bytes(&to_value(message)?).map_err(|e| invalid(e.to_string()))
}

fn enum_prefix(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if i > 0 && c.is_ascii_uppercase() {
            out.push('_');
        }
        out.push(c.to_ascii_uppercase());
    }
    out.push('_');
    out
}
fn from_value(
    desc: MessageDescriptor,
    value: &Value,
    depth: usize,
) -> Result<DynamicMessage, Status> {
    if depth >= 64 {
        return Err(invalid("message nesting exceeds 64"));
    }
    let object = value
        .as_object()
        .ok_or_else(|| invalid("message must be an object"))?;
    let mut message = DynamicMessage::new(desc.clone());
    let mut groups = std::collections::BTreeSet::new();
    for (name, value) in object {
        let field = desc
            .get_field_by_name(name)
            .ok_or_else(|| invalid(format!("unknown field {}.{name}", desc.full_name())))?;
        if value.is_null() {
            return Err(invalid(format!("null field {}", field.full_name())));
        }
        if let Some(group) = field.containing_oneof()
            && !groups.insert(group.full_name().to_string())
        {
            return Err(invalid("multiple oneof values"));
        }
        let converted = if field.is_map() {
            return Err(invalid("unreviewed map schema"));
        } else if field.is_list() {
            let values = value
                .as_array()
                .ok_or_else(|| invalid("repeated field must be array"))?;
            validate_set(&field, values)?;
            PValue::List(
                values
                    .iter()
                    .map(|v| parse_scalar(&field, v, depth))
                    .collect::<Result<_, _>>()?,
            )
        } else {
            parse_scalar(&field, value, depth)?
        };
        message
            .try_set_field(&field, converted)
            .map_err(|e| invalid(e.to_string()))?;
    }
    for group in desc.oneofs() {
        if !group.is_synthetic() && !groups.contains(group.full_name()) {
            return Err(invalid(format!("missing oneof {}", group.full_name())));
        }
    }
    Ok(message)
}
fn parse_scalar(field: &FieldDescriptor, value: &Value, depth: usize) -> Result<PValue, Status> {
    let bad = || invalid(format!("invalid value for {}", field.full_name()));
    match field.kind() {
        Kind::Message(desc) => Ok(PValue::Message(from_value(desc, value, depth + 1)?)),
        Kind::Bool => value.as_bool().map(PValue::Bool).ok_or_else(bad),
        Kind::Uint64 | Kind::Fixed64 => {
            let count: Counter = serde_json::from_value(value.clone()).map_err(|_| bad())?;
            Ok(PValue::U64(count.0))
        }
        Kind::Sint64 | Kind::Int64 | Kind::Sfixed64 => {
            let integer: Integer = serde_json::from_value(value.clone()).map_err(|_| bad())?;
            Ok(PValue::I64(integer.0))
        }
        Kind::Uint32 | Kind::Fixed32 => value
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .map(PValue::U32)
            .ok_or_else(bad),
        Kind::Int32 | Kind::Sint32 | Kind::Sfixed32 => value
            .as_i64()
            .and_then(|v| i32::try_from(v).ok())
            .map(PValue::I32)
            .ok_or_else(bad),
        Kind::Double => {
            let real = Real::new(value.as_f64().ok_or_else(bad)?).map_err(|_| bad())?;
            Ok(PValue::F64(real.get()))
        }
        Kind::Float => {
            let f = value.as_f64().ok_or_else(bad)? as f32;
            if !f.is_finite() {
                return Err(bad());
            }
            Ok(PValue::F32(if f == 0.0 { 0.0 } else { f }))
        }
        Kind::Enum(en) => {
            let token = value.as_str().ok_or_else(bad)?;
            let entry = en
                .get_value_by_name(&(enum_prefix(en.name()) + token))
                .ok_or_else(bad)?;
            if entry.number() == 0 {
                return Err(bad());
            }
            Ok(PValue::EnumNumber(entry.number()))
        }
        Kind::Bytes => {
            let digest: Digest = serde_json::from_value(value.clone()).map_err(|_| bad())?;
            Ok(PValue::Bytes(digest.as_bytes().to_vec().into()))
        }
        Kind::String => {
            let value = value.as_str().ok_or_else(bad)?;
            match RULES.get(field.full_name()).map(String::as_str) {
                Some("Name") => {
                    DomainName::new(value).map_err(|_| bad())?;
                }
                Some("Id") => {
                    Id::new(value).map_err(|_| bad())?;
                }
                _ => {}
            }
            if field.name() == "detail" && value.len() > 4096 {
                return Err(bad());
            }
            Ok(PValue::String(value.to_string()))
        }
    }
}
fn project(message: &DynamicMessage) -> Result<Value, Status> {
    let mut object = Map::new();
    for field in message.descriptor().fields() {
        if field.supports_presence() && !message.has_field(&field) {
            continue;
        }
        let value = message.get_field(&field);
        let projected = if field.is_list() {
            let PValue::List(values) = value.as_ref() else {
                return Err(invalid("expected repeated field"));
            };
            let mut values = values
                .iter()
                .map(|v| scalar_json(&field, v))
                .collect::<Result<Vec<_>, _>>()?;
            validate_set(&field, &values)?;
            if is_set(&field) {
                values.sort_by_cached_key(|v| v.to_string());
            }
            Value::Array(values)
        } else {
            scalar_json(&field, value.as_ref())?
        };
        object.insert(field.name().to_string(), projected);
    }
    Ok(Value::Object(object))
}
fn scalar_json(field: &FieldDescriptor, value: &PValue) -> Result<Value, Status> {
    match value {
        PValue::Message(m) => project(m),
        PValue::Bool(v) => Ok((*v).into()),
        PValue::U64(v) => Ok(v.to_string().into()),
        PValue::I64(v) => Ok(v.to_string().into()),
        PValue::U32(v) => Ok((*v).into()),
        PValue::I32(v) => Ok((*v).into()),
        PValue::F64(v) => {
            let real = Real::new(*v).map_err(|e| invalid(e.to_string()))?;
            serde_json::Number::from_f64(real.get())
                .map(Value::Number)
                .ok_or_else(|| invalid("non-finite real"))
        }
        PValue::F32(v) => serde_json::Number::from_f64(f64::from(*v))
            .map(Value::Number)
            .ok_or_else(|| invalid("non-finite float")),
        PValue::String(v) => Ok(v.clone().into()),
        PValue::Bytes(v) => {
            if v.len() != 32 {
                return Err(invalid(format!("invalid Digest {}", field.full_name())));
            }
            Ok(Digest::from_bytes(
                v.as_ref()
                    .try_into()
                    .map_err(|_| invalid("Digest length"))?,
            )
            .to_string()
            .into())
        }
        PValue::EnumNumber(n) => {
            let Kind::Enum(en) = field.kind() else {
                return Err(invalid("enum type mismatch"));
            };
            let value = en
                .get_value(*n)
                .filter(|v| v.number() != 0)
                .ok_or_else(|| invalid("unknown or unspecified enum"))?;
            let prefix = enum_prefix(en.name());
            let token = value
                .name()
                .strip_prefix(&prefix)
                .ok_or_else(|| Status::internal("enum binding prefix"))?;
            Ok(token.into())
        }
        PValue::List(_) | PValue::Map(_) => Err(invalid("unexpected collection")),
    }
}
fn is_set(field: &FieldDescriptor) -> bool {
    matches!(
        field.name(),
        "resource_set" | "calibration_digests" | "evidence_ids" | "scope"
    )
}
fn validate_set(field: &FieldDescriptor, values: &[Value]) -> Result<(), Status> {
    if is_set(field) {
        let mut seen = std::collections::BTreeSet::new();
        for value in values {
            if !seen.insert(value.to_string()) {
                return Err(invalid(format!(
                    "duplicate set member {}",
                    field.full_name()
                )));
            }
        }
    }
    Ok(())
}

pub fn intent_to_domain(intent: &crate::base::Intent) -> Result<rx_domain::intent::Intent, Status> {
    let domain: rx_domain::intent::Intent =
        canonical::decode_json(&to_vec(intent)?).map_err(|e| invalid(e.to_string()))?;
    domain.normalized().map_err(|e| invalid(e.to_string()))
}
