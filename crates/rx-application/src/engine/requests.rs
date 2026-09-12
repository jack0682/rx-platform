use super::*;

pub(super) fn request(
    meta: &Installation,
    p: &Principal,
    method: &str,
    key_: &str,
    payload: &impl Serialize,
) -> Result<(RequestScope, Digest)> {
    Id::new(key_).map_err(domain_error)?;
    Ok((
        RequestScope {
            installation_id: meta.id.clone(),
            client_namespace: p.client_namespace.clone(),
            method: name(method),
            key: key_.into(),
        },
        canonical::digest("RX-REQUEST-v1", &(method, payload)).map_err(domain_error)?,
    ))
}

pub(super) fn prior<T: DeserializeOwned>(
    tx: &mut dyn Transaction,
    scope: &RequestScope,
    fingerprint: Digest,
    schema: &str,
) -> Result<Option<T>> {
    tx.lookup(scope)?
        .map(|old| {
            if old.fingerprint != fingerprint {
                return Err(StoreError::KeyConflict);
            }
            if old.result.schema.as_str() != schema {
                return Err(StoreError::Integrity("cached reply schema".into()));
            }
            canonical::decode_json(&canonical::bytes(&old.result.value).map_err(domain_error)?)
                .map_err(domain_error)
        })
        .transpose()
}

pub(super) fn remember(
    tx: &mut dyn Transaction,
    scope: &RequestScope,
    fingerprint: Digest,
    schema: &str,
    value: &impl Serialize,
) -> Result<()> {
    tx.remember(
        scope,
        &SavedRequest {
            fingerprint,
            result: doc(schema, value)?,
        },
    )
}
