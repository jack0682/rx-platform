//! Raw validation happens before prost can merge singular fields or discard unknowns.
use crate::DESCRIPTORS;
use bytes::{Buf, BufMut};
use prost::{Message, Name};
use prost_reflect::{FieldDescriptor, Kind, MessageDescriptor};
use std::{collections::BTreeSet, marker::PhantomData};
use tonic::{
    Status,
    codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder},
};

pub fn decode<M: Message + Default + Name>(bytes: &[u8]) -> Result<M, Status> {
    let descriptor = DESCRIPTORS
        .get_message_by_name(&M::full_name())
        .ok_or_else(|| Status::internal("message descriptor absent"))?;
    validate(&descriptor, bytes)?;
    M::decode(bytes).map_err(|e| Status::invalid_argument(e.to_string()))
}

pub fn validate(descriptor: &MessageDescriptor, bytes: &[u8]) -> Result<(), Status> {
    if bytes.len() > rx_domain::canonical::MAX_MESSAGE_BYTES {
        return Err(Status::resource_exhausted("message exceeds 1 MiB"));
    }
    let mut field_budget = 65_536;
    scan(descriptor, bytes, 0, &mut field_budget)
}

fn invalid(message: impl Into<String>) -> Status {
    Status::invalid_argument(message.into())
}
fn varint(bytes: &[u8], at: &mut usize) -> Result<u64, Status> {
    let mut result = 0_u64;
    for shift in (0..70).step_by(7) {
        let b = *bytes.get(*at).ok_or_else(|| invalid("truncated varint"))?;
        *at += 1;
        if shift == 63 && b > 1 {
            return Err(invalid("varint overflow"));
        }
        result |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Ok(result);
        }
    }
    Err(invalid("varint overflow"))
}
fn slice<'a>(bytes: &'a [u8], at: &mut usize, len: usize) -> Result<&'a [u8], Status> {
    let end = at
        .checked_add(len)
        .ok_or_else(|| invalid("length overflow"))?;
    let result = bytes
        .get(*at..end)
        .ok_or_else(|| invalid("truncated field"))?;
    *at = end;
    Ok(result)
}
fn wire_type(kind: &Kind) -> u8 {
    match kind {
        Kind::Double | Kind::Fixed64 | Kind::Sfixed64 => 1,
        Kind::String | Kind::Bytes | Kind::Message(_) => 2,
        Kind::Float | Kind::Fixed32 | Kind::Sfixed32 => 5,
        _ => 0,
    }
}
fn scan(
    descriptor: &MessageDescriptor,
    bytes: &[u8],
    depth: usize,
    budget: &mut usize,
) -> Result<(), Status> {
    if depth >= 64 {
        return Err(invalid("message nesting exceeds 64"));
    }
    let mut at = 0;
    let mut singular = BTreeSet::new();
    let mut oneofs = BTreeSet::new();
    while at < bytes.len() {
        if *budget == 0 {
            return Err(Status::resource_exhausted("too many wire fields"));
        }
        *budget -= 1;
        let key = varint(bytes, &mut at)?;
        let tag = u32::try_from(key >> 3).map_err(|_| invalid("field number overflow"))?;
        let wire = (key & 7) as u8;
        if tag == 0 {
            return Err(invalid("field zero"));
        }
        let field = descriptor
            .get_field(tag)
            .ok_or_else(|| invalid(format!("unknown field {}.{tag}", descriptor.full_name())))?;
        if field.is_map() {
            // Neither frozen v1 schema defines maps. Fail closed if an unreviewed IDL adds one.
            return Err(invalid(
                "map schema requires reviewed key-uniqueness binding",
            ));
        }
        if !field.is_list() && !singular.insert(tag) {
            return Err(invalid(format!(
                "duplicate singular field {}",
                field.full_name()
            )));
        }
        if let Some(group) = field.containing_oneof()
            && !oneofs.insert(group.full_name().to_string())
        {
            return Err(invalid(format!(
                "multiple oneof values in {}",
                group.full_name()
            )));
        }
        let kind = field.kind();
        let expected = wire_type(&kind);
        if field.is_list() && expected != 2 && wire == 2 {
            let len =
                usize::try_from(varint(bytes, &mut at)?).map_err(|_| invalid("length overflow"))?;
            let packed = slice(bytes, &mut at, len)?;
            let mut packed_at = 0;
            while packed_at < packed.len() {
                if *budget == 0 {
                    return Err(Status::resource_exhausted("too many packed values"));
                }
                *budget -= 1;
                scalar(&field, &kind, packed, &mut packed_at)?;
            }
        } else {
            if wire != expected {
                return Err(invalid(format!(
                    "wire type mismatch for {}",
                    field.full_name()
                )));
            }
            if let Kind::Message(nested) = kind {
                let len = usize::try_from(varint(bytes, &mut at)?)
                    .map_err(|_| invalid("length overflow"))?;
                scan(&nested, slice(bytes, &mut at, len)?, depth + 1, budget)?;
            } else {
                scalar(&field, &kind, bytes, &mut at)?;
            }
        }
    }
    for group in descriptor.oneofs() {
        if !group.is_synthetic() && !oneofs.contains(group.full_name()) {
            return Err(invalid(format!("missing oneof {}", group.full_name())));
        }
    }
    Ok(())
}
fn scalar(
    field: &FieldDescriptor,
    kind: &Kind,
    bytes: &[u8],
    at: &mut usize,
) -> Result<(), Status> {
    match wire_type(kind) {
        0 => {
            let value = varint(bytes, at)?;
            if matches!(kind, Kind::Bool) && value > 1 {
                return Err(invalid("boolean is not 0 or 1"));
            }
            if matches!(kind, Kind::Uint32 | Kind::Sint32) && value > u64::from(u32::MAX) {
                return Err(invalid("uint32 overflow"));
            }
            if let Kind::Enum(en) = kind {
                let number = i32::try_from(value).map_err(|_| invalid("enum outside range"))?;
                if number == 0 || en.get_value(number).is_none() {
                    return Err(invalid(format!(
                        "unspecified or unknown enum {}",
                        en.full_name()
                    )));
                }
            }
        }
        1 => {
            let raw: [u8; 8] = slice(bytes, at, 8)?
                .try_into()
                .map_err(|_| invalid("double length"))?;
            if matches!(kind, Kind::Double) && !f64::from_le_bytes(raw).is_finite() {
                return Err(invalid("non-finite double"));
            }
        }
        5 => {
            let raw: [u8; 4] = slice(bytes, at, 4)?
                .try_into()
                .map_err(|_| invalid("float length"))?;
            if matches!(kind, Kind::Float) && !f32::from_le_bytes(raw).is_finite() {
                return Err(invalid("non-finite float"));
            }
        }
        2 => {
            let len =
                usize::try_from(varint(bytes, at)?).map_err(|_| invalid("length overflow"))?;
            let value = slice(bytes, at, len)?;
            if matches!(kind, Kind::String) {
                std::str::from_utf8(value)
                    .map_err(|_| invalid(format!("invalid UTF-8 {}", field.full_name())))?;
            }
        }
        _ => return Err(invalid("unsupported wire type")),
    }
    Ok(())
}

pub struct StrictCodec<E, D>(PhantomData<(E, D)>);
impl<E, D> Default for StrictCodec<E, D> {
    fn default() -> Self {
        Self(PhantomData)
    }
}
pub struct StrictEncoder<E>(PhantomData<E>);
pub struct StrictDecoder<D>(PhantomData<D>);
impl<E, D> Codec for StrictCodec<E, D>
where
    E: Message + Name + Send + 'static,
    D: Message + Name + Default + Send + 'static,
{
    type Encode = E;
    type Decode = D;
    type Encoder = StrictEncoder<E>;
    type Decoder = StrictDecoder<D>;
    fn encoder(&mut self) -> Self::Encoder {
        StrictEncoder(PhantomData)
    }
    fn decoder(&mut self) -> Self::Decoder {
        StrictDecoder(PhantomData)
    }
}
impl<E: Message + Name> Encoder for StrictEncoder<E> {
    type Item = E;
    type Error = Status;
    fn encode(&mut self, item: E, dst: &mut EncodeBuf<'_>) -> Result<(), Status> {
        let bytes = item.encode_to_vec();
        let descriptor = DESCRIPTORS
            .get_message_by_name(&E::full_name())
            .ok_or_else(|| Status::internal("message descriptor absent"))?;
        validate(&descriptor, &bytes)?;
        dst.put_slice(&bytes);
        Ok(())
    }
}
impl<D: Message + Default + Name> Decoder for StrictDecoder<D> {
    type Item = D;
    type Error = Status;
    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<D>, Status> {
        let bytes = src.copy_to_bytes(src.remaining());
        decode(&bytes).map(Some)
    }
}
