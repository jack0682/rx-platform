//! Signed v2 declarations, not software qualification, Host admission or an execution grant.
use rx_domain::{canonical, intent::Body, types::*};
use rx_package::{EntryPoint, PackagePath, VerifiedPackage, store::StoredPackage};
use rx_process_contract::{
    ActionBinding,
    execution_v2::{self as v2, NodeContract, TemplateCatalog},
};

pub const CATALOG_PATH: &str = "execution-template-catalog.json";
pub struct VerifiedTemplates {
    catalog: TemplateCatalog,
    manifest: Digest,
    signature: Digest,
    catalog_reference: ArtifactRef,
    dependencies: Vec<ArtifactRef>,
}
fn artifact(package: &VerifiedPackage, r: &ArtifactRef) -> Result<(), String> {
    if !package.manifest().assets.contains(r)
        || !package.files().any(|(path, bytes)| {
            bytes.len() as u64 == r.size_bytes.0
                && rx_package::content_digest(bytes) == r.sha256
                && package
                    .manifest()
                    .files
                    .iter()
                    .any(|f| &f.path == path && !f.executable)
        })
    {
        return Err(format!(
            "template artifact {} is not declared and present in the signed package",
            r.sha256
        ));
    }
    Ok(())
}
impl VerifiedTemplates {
    pub fn check(
        stored: &StoredPackage,
        registration: &crate::package_intake::Registration,
        installation: &Id,
        cell: &Name,
    ) -> Result<Self, String> {
        if stored.owner() != &registration.store_owner
            || stored.policy_fingerprint() != registration.policy_fingerprint
        {
            return Err("template package store owner or verification policy differs".into());
        }
        let package = stored.package();
        // VerifiedPackage can only be produced by the existing signature/content verifier.
        let manifest = package.manifest();
        let EntryPoint::DeviceReference {
            family,
            profiles,
            adapter,
        } = &manifest.entry
        else {
            return Err("v2 templates require DEVICE_REFERENCE".into());
        };
        if manifest.schema.as_str() != "rx.package.v2" || !manifest.dependencies.is_empty() {
            return Err("self-contained v2 device-reference package required".into());
        }
        let path = PackagePath::new(CATALOG_PATH)?;
        let bytes=package.file(&path).ok_or("explicit v2 template declaration is absent; v1 fixed inputs grant no substitution right")?;
        let catalog = TemplateCatalog::decode(bytes)?;
        if &catalog.installation != installation
            || &catalog.cell != cell
            || catalog.documents[&Name::new("family").expect("static role")].path != family.as_str()
            || catalog.documents[&Name::new("adapter").expect("static role")].path
                != adapter.as_str()
            || profiles.len() != 1
            || catalog.documents[&Name::new("profile").expect("static role")].path
                != profiles[0].as_str()
        {
            return Err("template package installation/cell/source binding differs".into());
        }
        let catalog_reference = ArtifactRef {
            sha256: rx_package::content_digest(bytes),
            schema_id: Name::new(v2::TEMPLATE_CATALOG_SCHEMA).expect("static schema"),
            size_bytes: Counter(bytes.len() as u64),
        };
        artifact(package, &catalog_reference)?;
        for document in catalog.documents.values() {
            artifact(package, &document.artifact)?;
            let path = PackagePath::new(&document.path)?;
            let actual = package
                .file(&path)
                .ok_or("template source document missing")?;
            v2::verify_artifact(actual, &document.artifact, 2 * 1024 * 1024)?;
        }
        for declaration in catalog.templates.values() {
            let Body::Program(goal) = &declaration.action.intent.body else {
                return Err("Program required".into());
            };
            artifact(package, &goal.program)?;
            artifact(package, &goal.parameter_set)?;
        }
        let mut dependencies = std::collections::BTreeMap::new();
        let mut insert = |r: ArtifactRef| -> Result<(), String> {
            if let Some(old) = dependencies.insert((r.sha256, r.schema_id.clone()), r.clone())
                && old != r
            {
                return Err("conflicting template dependency metadata".into());
            }
            Ok(())
        };
        for r in &manifest.assets {
            insert(r.clone())?;
        }
        // Include every signed file, even a file not used by the chosen template.
        for f in &manifest.files {
            insert(ArtifactRef {
                sha256: f.sha256,
                schema_id: Name::new("rx.package-file.v1").expect("static schema"),
                size_bytes: f.size_bytes,
            })?;
        }
        let manifest_bytes = rx_package::manifest_bytes(manifest).map_err(|e| e.to_string())?;
        insert(ArtifactRef {
            sha256: package.digest(),
            schema_id: manifest.schema.clone(),
            size_bytes: Counter(manifest_bytes.len() as u64),
        })?;
        let signature_bytes = canonical::bytes(package.signature()).map_err(|e| e.to_string())?;
        insert(ArtifactRef {
            sha256: rx_package::content_digest(&signature_bytes),
            schema_id: Name::new("rx.package-signature.v1").expect("static schema"),
            size_bytes: Counter(signature_bytes.len() as u64),
        })?;
        if dependencies.len() > v2::MAX_DEPENDENCIES {
            return Err("template qualification dependency bound exceeded".into());
        }
        Ok(Self {
            catalog,
            manifest: package.digest(),
            signature: rx_package::content_digest(
                &canonical::bytes(package.signature()).map_err(|e| e.to_string())?,
            ),
            catalog_reference,
            dependencies: dependencies.into_values().collect(),
        })
    }
    pub fn manifest(&self) -> Digest {
        self.manifest
    }
    pub fn signature(&self) -> Digest {
        self.signature
    }
    pub fn catalog_reference(&self) -> &ArtifactRef {
        &self.catalog_reference
    }
    pub fn catalog(&self) -> &TemplateCatalog {
        &self.catalog
    }
    /// Transitive signed inputs for qualification; this list is not qualification.
    pub fn dependencies(&self) -> &[ArtifactRef] {
        &self.dependencies
    }
    pub fn matches(
        &self,
        template: &Name,
        action: &ActionBinding,
        contract: &NodeContract,
    ) -> Result<(), String> {
        let expected = self
            .catalog
            .templates
            .get(template)
            .ok_or("named template absent from signed declaration")?;
        if canonical::bytes(&expected.action).map_err(|e| e.to_string())?
            != canonical::bytes(action).map_err(|e| e.to_string())?
            || canonical::bytes(&expected.contract).map_err(|e| e.to_string())?
                != canonical::bytes(contract).map_err(|e| e.to_string())?
        {
            return Err(
                "workflow template or parameter contract differs from signed declaration".into(),
            );
        }
        Ok(())
    }
}
