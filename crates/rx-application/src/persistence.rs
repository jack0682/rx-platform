use rx_domain::{canonical, fault::Rejection, types::*};
use rx_ports::*;
use serde::{Serialize, de::DeserializeOwned};

pub fn reject<T>(reason: Rejection) -> Result<T> {
    Err(StoreError::Rejected(reason))
}
pub fn id() -> Id {
    Id::new(uuid::Uuid::now_v7().to_string()).expect("UUID generation")
}
pub fn name(value: impl Into<String>) -> Name {
    Name::new(value).expect("internal name")
}
pub fn key(kind: &str, value: impl Serialize) -> Name {
    name(format!(
        "{kind}/{}",
        canonical::digest("RX-ENTITY-KEY-v1", &value).expect("internal entity identity")
    ))
}
pub fn doc<T: Serialize>(schema: &str, value: &T) -> Result<Document> {
    Ok(Document {
        schema: name(schema),
        value: serde_json::to_value(value).map_err(|e| StoreError::Invalid(e.to_string()))?,
    })
}
pub fn decode<T: DeserializeOwned>(record: &Record, schema: &str) -> Result<T> {
    if record.document.schema.as_str() != schema {
        return Err(StoreError::Integrity("entity schema mismatch".into()));
    }
    canonical::decode_json(
        &canonical::bytes(&record.document.value)
            .map_err(|e| StoreError::Integrity(e.to_string()))?,
    )
    .map_err(|e| StoreError::Integrity(e.to_string()))
}
pub fn load<T: DeserializeOwned>(
    tx: &mut dyn Transaction,
    kind: &str,
    identity: impl Serialize,
    schema: &str,
) -> Result<(Counter, T)> {
    let record = tx
        .get(&key(kind, identity))?
        .ok_or(StoreError::Rejected(Rejection::NotFound))?;
    Ok((record.revision, decode(&record, schema)?))
}
pub fn save<T: Serialize>(
    tx: &mut dyn Transaction,
    kind: &str,
    identity: impl Serialize,
    revision: Option<Counter>,
    schema: &str,
    value: &T,
) -> Result<Counter> {
    Ok(tx
        .put(&key(kind, identity), revision, &doc(schema, value)?)?
        .revision)
}
pub fn event<T: Serialize>(tx: &mut dyn Transaction, kind: &str, value: &T) -> Result<()> {
    tx.append(&id(), &doc(kind, value)?)?;
    Ok(())
}
