//! Review-only projection from a verified device package. Manufacturer semantic validation stays in S.
use rx_domain::{canonical, types::*};
use rx_package::{EntryPoint, PackagePath, VerifiedPackage};
use rx_process_contract::device_catalog::Catalog;
use serde::{Deserialize, Serialize};

pub fn extract(package: &VerifiedPackage) -> Result<Option<Catalog>, String> {
    let path = PackagePath::new("device-catalog.json")?;
    let Some(bytes) = package.file(&path) else {
        return Ok(None);
    };
    let EntryPoint::DeviceReference {
        family,
        profiles,
        adapter,
    } = &package.manifest().entry
    else {
        return Err("catalog requires DEVICE_REFERENCE package".into());
    };
    if bytes.len() > 131_072 {
        return Err("device catalog too large".into());
    }
    let c: Catalog = canonical::decode_json(bytes).map_err(|e| e.to_string())?;
    c.validate()?;
    if c.documents[&name("family")].path != family.as_str()
        || c.documents[&name("adapter")].path != adapter.as_str()
        || !profiles
            .iter()
            .any(|p| p.as_str() == c.documents[&name("profile")].path)
    {
        return Err("device catalog entry references differ".into());
    }
    for d in c.documents.values() {
        let p = PackagePath::new(&d.path)?;
        let data = package.file(&p).ok_or("catalog source absent")?;
        if rx_package::content_digest(data) != d.artifact.sha256
            || data.len() as u64 != d.artifact.size_bytes.0
            || package
                .manifest()
                .files
                .iter()
                .find(|f| f.path == p)
                .is_none_or(|f| f.executable)
        {
            return Err("catalog source bytes differ".into());
        }
    }
    let operations: std::collections::BTreeMap<Name, rx_domain::intent::Intent> =
        canonical::decode_json(
            package
                .file(&PackagePath::new(&c.documents[&name("operations")].path)?)
                .ok_or("operations absent")?,
        )
        .map_err(|e| e.to_string())?;
    if canonical::bytes(&operations).map_err(|e| e.to_string())?
        != canonical::bytes(&c.operations).map_err(|e| e.to_string())?
    {
        return Err("catalog operations differ".into());
    }
    if let Some(t) = &c.outcomes {
        let source: rx_process_contract::native_outcome::NativeOutcomeTable =
            canonical::decode_json(
                package
                    .file(&PackagePath::new(&c.documents[&name("outcomes")].path)?)
                    .ok_or("outcomes absent")?,
            )
            .map_err(|e| e.to_string())?;
        if canonical::bytes(&source).map_err(|e| e.to_string())?
            != canonical::bytes(t).map_err(|e| e.to_string())?
        {
            return Err("catalog outcomes differ".into());
        }
    }
    Ok(Some(c))
}
fn name(v: &str) -> Name {
    Name::new(v).expect("fixed role")
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Detail {
    pub cell: Name,
    pub intake: Id,
    pub object: rx_package::store::ObjectId,
    pub reference: Option<ArtifactRef>,
    pub catalog: Option<Catalog>,
    pub review_context_current: bool,
    pub content_reverification_required: bool,
    pub manufacturer_validation_required: bool,
    pub activation_authorized: bool,
}
