use super::*;
use rx_ports::{Document, Record, Repository, Transaction};

fn name(value: &str) -> Name {
    Name::new(value).expect("literal")
}
fn state_error(e: impl std::fmt::Display) -> Error {
    Error::State(e.to_string())
}
fn load<T: serde::de::DeserializeOwned>(record: &Record, schema: &str) -> Result<T> {
    if record.document.schema.as_str() != schema {
        return Err(state_error("schema"));
    }
    decode(&canonical::bytes(&record.document.value).map_err(state_error)?).map_err(state_error)
}
fn save(
    tx: &mut dyn Transaction,
    key: &Name,
    old: Option<Record>,
    schema: &str,
    value: &impl Serialize,
) -> Result<()> {
    let document = Document {
        schema: name(schema),
        value: serde_json::to_value(value).map_err(state_error)?,
    };
    let entity = tx.put(key, old.map(|r| r.revision), &document)?;
    tx.append_control(
        &Id::new(uuid::Uuid::new_v4().to_string()).expect("UUID"),
        &entity,
        &document,
    )?;
    Ok(())
}
fn revocation_state(tx: &mut dyn Transaction, current: &SignedRevocations) -> Result<()> {
    let key = name("release/revocations/solutions-development");
    let old = tx.get(&key)?;
    if let Some(record) = &old {
        let prior: SignedRevocations = load(record, "rx.release-revocation-state.v1")?;
        revocations(&prior).map_err(state_error)?;
        let a = &prior.revocations;
        let b = &current.revocations;
        if b.version < a.version
            || !a.revoked.is_subset(&b.revoked)
            || (b.version == a.version
                && canonical::bytes(a).map_err(state_error)?
                    != canonical::bytes(b).map_err(state_error)?)
        {
            return Err(Error::Rollback);
        }
        if a.version == b.version {
            return Ok(());
        }
    }
    save(tx, &key, old, "rx.release-revocation-state.v1", current)
}
fn transact<T>(
    repository: &mut impl Repository,
    operation: impl FnOnce(&mut dyn Transaction) -> Result<T>,
) -> Result<T> {
    // Every domain refusal is evaluated before writes. Storage failures after a
    // write must escape the callback so the repository rolls the transaction back.
    repository.transact(|tx| match operation(tx) {
        Err(Error::Store(error)) => Err(error),
        result => Ok(result),
    })?
}
/// Record an authenticated revocation checkpoint even when the candidate release
/// is now revoked or invalid. Restoring an older list cannot undo this record.
pub fn update_revocations(repository: &mut impl Repository, bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() {
        return Err(Error::Unsigned);
    }
    let current: SignedRevocations = decode(bytes)?;
    revocations(&current)?;
    transact(repository, |tx| revocation_state(tx, &current))
}
fn apply(tx: &mut dyn Transaction, current: &VerifiedRelease) -> Result<()> {
    let key = name("release/accepted/solutions-development");
    let old = tx.get(&key)?;
    if let Some(record) = &old {
        let prior: SignedRelease = load(record, "rx.release-state.v1")?;
        let digest = release(&prior).map_err(state_error)?;
        if current.version() < prior.manifest.version
            || (current.version() == prior.manifest.version && current.digest != digest)
        {
            return Err(Error::Rollback);
        }
    }
    let revocation_key = name("release/revocations/solutions-development");
    let record = tx
        .get(&revocation_key)?
        .ok_or_else(|| state_error("revocation checkpoint absent"))?;
    let prior: SignedRevocations = load(&record, "rx.release-revocation-state.v1")?;
    revocations(&prior).map_err(state_error)?;
    if prior.revocations.revoked.contains(&current.digest) {
        return Err(Error::Revoked);
    }
    if canonical::bytes(&prior.revocations).map_err(state_error)?
        != canonical::bytes(&current.revocations.revocations).map_err(state_error)?
    {
        return Err(Error::Rollback);
    }
    if let Some(record) = &old {
        let prior: SignedRelease = load(record, "rx.release-state.v1")?;
        if current.version() == prior.manifest.version {
            return Ok(());
        }
    }
    save(tx, &key, old, "rx.release-state.v1", &current.release)
}
/// Atomic high-water admission with intact retained local state. Full database
/// replacement/deletion is NOT_DETECTED. OS and state retention remain trusted.
/// Call update_revocations before verify/admit, even for a denied candidate.
/// This is an explicit checkpoint, not continuous permission or monitoring.
pub fn admit(repository: &mut impl Repository, current: &VerifiedRelease) -> Result<()> {
    transact(repository, |tx| apply(tx, current))
}
