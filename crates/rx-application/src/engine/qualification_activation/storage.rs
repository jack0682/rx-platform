use super::*;
const V2: &str = "rx.qualification-host-task.v2";
const MAX: usize = 8 * 1024 * 1024;
const BLOBS: crate::artifact_storage::BlobStore =
    crate::artifact_storage::BlobStore::new("hostqualificationtask", MAX as u64);
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    id: Id,
    artifact: ArtifactRef,
}
pub(super) fn decode_task(tx: &mut dyn Transaction, row: &rx_ports::Record) -> Result<a::Task> {
    if row.document.schema.as_str() == TASK {
        let t: a::Task = decode(row, TASK)?;
        if !t.execution_policies.is_empty() {
            return Err(StoreError::Integrity("v2 task in legacy storage".into()));
        }
        return Ok(t);
    }
    let stored: Stored = decode(row, V2)?;
    let bytes = BLOBS.read(tx, &stored.artifact)?;
    let task: a::Task =
        serde_json::from_slice(&bytes).map_err(|e| StoreError::Integrity(e.to_string()))?;
    if task.id != stored.id
        || stored.artifact.schema_id.as_str() != V2
        || task.execution_policies.is_empty()
        || canonical::bytes(&task).map_err(domain_error)? != bytes
    {
        return Err(StoreError::Integrity(
            "v2 task storage identity/canonical bytes".into(),
        ));
    }
    Ok(task)
}
pub(super) fn persist(
    tx: &mut dyn Transaction,
    t: &a::Task,
    expected: Option<Counter>,
) -> Result<()> {
    if t.execution_policies.is_empty() {
        save(tx, "qualificationtask", &t.id, expected, TASK, t)?;
        return event(tx, "rx.event.qualification-host-task.v1", t);
    }
    tx.require_workflow_execution_reader()?;
    let bytes = canonical::bytes(t).map_err(domain_error)?;
    if bytes.len() > MAX {
        return reject(Reject::InvalidInput);
    }
    let artifact = ArtifactRef {
        schema_id: name(V2),
        sha256: rx_package::content_digest(&bytes),
        size_bytes: Counter(bytes.len() as u64),
    };
    BLOBS.put(tx, artifact.sha256, &bytes)?;
    let stored = Stored {
        id: t.id.clone(),
        artifact,
    };
    save(tx, "qualificationtask", &t.id, expected, V2, &stored)?;
    event(tx, "rx.event.qualification-host-task.v2", &stored)
}
