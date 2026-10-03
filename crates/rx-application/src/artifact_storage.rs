//! Immutable, bounded content blobs over the existing atomic document store.
//! The namespace/limit are trusted application constants, never request fields.
use crate::persistence::{doc, key, load};
use base64::Engine as _;
use rx_domain::types::*;
use rx_ports::{Result, StoreError, Transaction};

const CHUNK: usize = 256 * 1024;
#[derive(serde::Serialize, serde::Deserialize)]
struct Blob {
    size: Counter,
    chunks: Vec<Digest>,
}
pub(crate) struct BlobStore {
    namespace: &'static str,
    max_bytes: u64,
}
impl BlobStore {
    pub const fn new(namespace: &'static str, max_bytes: u64) -> Self {
        Self {
            namespace,
            max_bytes,
        }
    }
    pub fn put(&self, tx: &mut dyn Transaction, hash: Digest, bytes: &[u8]) -> Result<()> {
        if bytes.is_empty()
            || bytes.len() as u64 > self.max_bytes
            || rx_package::content_digest(bytes) != hash
        {
            return Err(StoreError::Invalid(
                "artifact blob size or digest differs".into(),
            ));
        }
        let mut chunks = Vec::new();
        for (index, part) in bytes.chunks(CHUNK).enumerate() {
            chunks.push(rx_package::content_digest(part));
            let k = key(
                &format!("{}chunk", self.namespace),
                (hash, Counter(index as u64)),
            );
            let d = doc(
                &format!("rx.internal.{}-chunk.v1", self.namespace),
                &base64::engine::general_purpose::STANDARD.encode(part),
            )?;
            if let Some(old) = tx.get(&k)? {
                if old.document != d || old.revision != Counter(1) {
                    return Err(StoreError::Integrity("artifact chunk collision".into()));
                }
            } else {
                tx.put(&k, None, &d)?;
            }
        }
        let k = key(&format!("{}blob", self.namespace), hash);
        let d = doc(
            &format!("rx.internal.{}-blob.v1", self.namespace),
            &Blob {
                size: Counter(bytes.len() as u64),
                chunks,
            },
        )?;
        if let Some(old) = tx.get(&k)? {
            if old.document != d || old.revision != Counter(1) {
                return Err(StoreError::Integrity("artifact blob collision".into()));
            }
        } else {
            tx.put(&k, None, &d)?;
        }
        Ok(())
    }
    pub fn read(&self, tx: &mut dyn Transaction, r: &ArtifactRef) -> Result<Vec<u8>> {
        let (revision, blob): (_, Blob) = load(
            tx,
            &format!("{}blob", self.namespace),
            r.sha256,
            &format!("rx.internal.{}-blob.v1", self.namespace),
        )?;
        if revision != Counter(1)
            || blob.size != r.size_bytes
            || blob.size.0 == 0
            || blob.size.0 > self.max_bytes
            || blob.chunks.len() != blob.size.0.div_ceil(CHUNK as u64) as usize
        {
            return Err(StoreError::Integrity("artifact blob shape differs".into()));
        }
        let mut bytes = Vec::with_capacity(blob.size.0 as usize);
        for (index, hash) in blob.chunks.iter().enumerate() {
            let (revision, s): (_, String) = load(
                tx,
                &format!("{}chunk", self.namespace),
                (r.sha256, Counter(index as u64)),
                &format!("rx.internal.{}-chunk.v1", self.namespace),
            )?;
            let part = base64::engine::general_purpose::STANDARD
                .decode(s)
                .map_err(|_| StoreError::Integrity("artifact encoding".into()))?;
            if revision != Counter(1)
                || part.len() != (blob.size.0 as usize - bytes.len()).min(CHUNK)
                || rx_package::content_digest(&part) != *hash
            {
                return Err(StoreError::Integrity("artifact chunk differs".into()));
            }
            bytes.extend(part);
        }
        if bytes.len() as u64 != r.size_bytes.0 || rx_package::content_digest(&bytes) != r.sha256 {
            return Err(StoreError::Integrity("artifact bytes differ".into()));
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rx_domain::canonical;
    use rx_ports::Repository;
    use rx_storage::SqliteRepository;
    #[test]
    fn large_artifact_roundtrip_is_immutable_and_each_document_keeps_legacy_limit() {
        let directory = tempfile::tempdir().unwrap();
        let mut repository = SqliteRepository::open(directory.path().join("blobs.db")).unwrap();
        let store = BlobStore::new("executionv2", 16 * 1024 * 1024);
        let bytes = (0..1_450_373).map(|i| (i % 251) as u8).collect::<Vec<_>>();
        let reference = ArtifactRef {
            schema_id: Name::new("test.bytes.v1").unwrap(),
            sha256: rx_package::content_digest(&bytes),
            size_bytes: Counter(bytes.len() as u64),
        };
        repository
            .transact(|tx| {
                store.put(tx, reference.sha256, &bytes)?;
                store.put(tx, reference.sha256, &bytes)?;
                assert_eq!(store.read(tx, &reference)?, bytes);
                Ok(())
            })
            .unwrap();
        let (_, records) = repository.snapshot().unwrap();
        assert_eq!(records.len(), bytes.len().div_ceil(CHUNK) + 1);
        assert!(records.iter().all(|r| r.revision == Counter(1)
            && canonical::bytes(&r.document).unwrap().len() < canonical::MAX_MESSAGE_BYTES));
        let mut forged = reference.clone();
        forged.size_bytes.0 -= 1;
        assert!(repository.transact(|tx| store.read(tx, &forged)).is_err());
        assert!(
            repository
                .transact(|tx| store.put(tx, reference.sha256, b"changed"))
                .is_err()
        );
        repository
            .transact(|tx| {
                let k = key("executionv2chunk", (reference.sha256, Counter(0)));
                tx.put(
                    &k,
                    Some(Counter(1)),
                    &doc("rx.internal.executionv2-chunk.v1", &"tampered")?,
                )?;
                Ok(())
            })
            .unwrap();
        assert!(
            repository
                .transact(|tx| store.read(tx, &reference))
                .is_err()
        );
    }
}
