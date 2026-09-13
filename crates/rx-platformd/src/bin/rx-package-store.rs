//! Offline content intake tool. Does not connect to P authority or authorize native execution.
use rx_domain::types::{Digest, Name};
use rx_package::{
    PackagePath, policy,
    store::{ObjectId, Store},
};
use serde::Deserialize;
use std::path::PathBuf;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    investigation: None,
    schema: Name,
    store_root: PathBuf,
    import_root: PathBuf,
    policy_file: PathBuf,
    policy_digest: Digest,
}
fn main() {
    if let Err(error) = run() {
        eprintln!("rx-package-store: {error}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let importing = matches!(args.as_slice(), [verb,_,_] if verb=="import");
    let verifying = matches!(args.as_slice(), [verb,_,_,_] if verb=="verify");
    if !importing && !verifying {
        return Err("usage: rx-package-store import ABSOLUTE_CONFIG RELATIVE_PACKAGE | verify ABSOLUTE_CONFIG MANIFEST_SHA256 SIGNATURE_SHA256".into());
    }
    let config_path = PathBuf::from(&args[1]);
    if !config_path.is_absolute() {
        return Err("configuration path must be absolute".into());
    }
    let config: Config = policy::read(&config_path)?;
    if config.schema.as_str() != "rx.package-store-config.v1"
        || !config.store_root.is_absolute()
        || !config.import_root.is_absolute()
        || !config.policy_file.is_absolute()
    {
        return Err("invalid package-store configuration".into());
    }
    let (policy_doc, digest) = policy::read_with_digest::<policy::Policy>(&config.policy_file)?;
    if digest != config.policy_digest {
        return Err("verification policy differs from configured digest".into());
    }
    if policy_doc.assets.iter().any(|a| !a.path.is_absolute())
        || policy_doc
            .dependencies
            .iter()
            .any(|d| !d.path.is_absolute())
    {
        return Err("offline intake requires absolute dependency and asset paths".into());
    }
    let current_policy = policy_doc.load()?;
    let (object, package) = if importing {
        let relative = PackagePath::new(&args[2])?;
        let package = rx_package::directory::verify_relative(
            &config.import_root,
            &relative,
            &current_policy,
        )?;
        // Failed verification cannot create an object store or publish content.
        let mut store = Store::open(&config.store_root)?;
        let object = store.put(&package)?;
        let stored = store.verify(&object, &current_policy)?;
        (object, stored)
    } else {
        let object = ObjectId {
            manifest: serde_json::from_value(serde_json::Value::String(args[2].clone()))?,
            signature: serde_json::from_value(serde_json::Value::String(args[3].clone()))?,
        };
        let store = Store::open_existing(&config.store_root)?;
        let package = store.verify(&object, &current_policy)?;
        (object, package)
    };
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "schema":"rx.package-store-result.v1",
            "status":"CONTENT_VERIFIED_NOT_ADMITTED",
            "action":if importing {"IMPORT"}else{"VERIFY"},
            "object":object,
            "package":package.manifest().package,
            "version":package.manifest().version,
            "kind":package.manifest().entry.kind(),
            "policy_digest":digest,
            "target":current_policy.target,
            "contracts":current_policy.contracts,
            "content_semantics_verified":false,
            "activation_authorized":false
        }))?
    );
    Ok(())
}
