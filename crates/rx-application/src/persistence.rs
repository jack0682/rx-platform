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
    // A Value already has unique object keys. Keep the canonical numeric conversion and
    // byte limit, but do not build a second untyped JSON tree before typed deserialization.
    let bytes = canonical::bytes(&record.document.value)
        .map_err(|e| StoreError::Integrity(e.to_string()))?;
    if bytes.len() > canonical::MAX_MESSAGE_BYTES {
        return Err(StoreError::Integrity("message exceeds 1 MiB".into()));
    }
    serde_json::from_slice(&bytes).map_err(|e| StoreError::Integrity(e.to_string()))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn record(value: serde_json::Value) -> Record {
        Record {
            key: name("test/value"),
            revision: Counter(1),
            document: Document {
                schema: name("test/v1"),
                value,
            },
        }
    }

    #[test]
    fn typed_record_decode_preserves_canonical_numbers_and_schema_errors() {
        #[derive(Debug, PartialEq, serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Value {
            count: u64,
            values: Vec<f64>,
        }
        for value in [
            serde_json::json!({"count": 1.0, "values": [-0.0, 1e-9, 1e20]}),
            serde_json::json!({"count": 1.5, "values": []}),
            serde_json::json!({"count": 1, "values": [], "extra": true}),
        ] {
            let row = record(value);
            let before =
                canonical::decode_json::<Value>(&canonical::bytes(&row.document.value).unwrap());
            let after = decode::<Value>(&row, "test/v1");
            assert_eq!(after.is_ok(), before.is_ok());
            if let (Ok(a), Ok(b)) = (after, before) {
                assert_eq!(a, b);
            }
        }
        assert!(decode::<Value>(&record(serde_json::json!({})), "other/v1").is_err());
    }

    #[test]
    fn typed_record_decode_keeps_the_message_byte_limit() {
        let row = record(serde_json::Value::String(
            "x".repeat(canonical::MAX_MESSAGE_BYTES),
        ));
        assert!(matches!(
            decode::<String>(&row, "test/v1"),
            Err(StoreError::Integrity(_))
        ));
    }
}
