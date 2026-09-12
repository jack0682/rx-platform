pub mod finalize;
pub mod seed;
pub mod signing;
mod tls;

use rx_application::CellConfiguration;
use rx_domain::{canonical, types::*};
use rx_package::{ContractSet, PackagePath, policy};
use rx_process_contract::compile_input::CompileInput;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
const PACKAGE_KEY: &str = "delivery-package-signer";
const REVIEW_KEY: &str = "delivery-review-signer";
const QUALIFICATION_KEY: &str = "delivery-qualification-signer";
const CELL: &str = "cell/a";
const HOST: &str = "host/sim";
const EXECUTOR: &str = "executor";
const TERMINAL: &str = "panel/sim";
const ALIAS: &str = "cycle";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Seed {
    schema: Name,
    installation: Id,
    release_digest: Digest,
    executor_journal: Id,
    host_unqualified_reference: Id,
    contracts: ContractSet,
    initial_cell_sha256: Digest,
    compile_input_sha256: Digest,
    package_recipe_sha256: Digest,
    package_policy_sha256: Digest,
    artifacts: BTreeMap<Digest, ArtifactRef>,
    public_signers: BTreeMap<Name, Digest>,
}
fn name(value: &str) -> Name {
    Name::new(value).expect("fixture literal")
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).expect("UUID")
}
fn bytes(value: &impl Serialize) -> Result<Vec<u8>> {
    Ok(canonical::bytes(value)?)
}
fn read<T: DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(canonical::decode_json(&read_bytes(path)?)?)
}
fn read_bytes(path: &Path) -> Result<Vec<u8>> {
    let parent = path.parent().ok_or("input parent required")?;
    let leaf = path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or("input filename required")?;
    Ok(rx_package::directory::read_relative_file(
        parent,
        &PackagePath::new(leaf)?,
        1_048_576,
    )?)
}
fn env_path(key: &str) -> Result<PathBuf> {
    let value = PathBuf::from(std::env::var(key).map_err(|_| format!("{key} required"))?);
    if !value.is_absolute() {
        return Err(format!("{key} must be absolute").into());
    }
    Ok(value)
}
fn env_digest(key: &str) -> Result<Digest> {
    let value = std::env::var(key).map_err(|_| format!("{key} required"))?;
    Ok(serde_json::from_value(serde_json::Value::String(value))?)
}
fn new_output() -> Result<PathBuf> {
    let path = env_path("RX_CELL_DELIVERY_OUTPUT")?;
    fs::create_dir(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(path)
}
fn write(path: &Path, data: &[u8]) -> Result<()> {
    fs::create_dir_all(path.parent().ok_or("output parent required")?)?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(data)?;
    file.sync_all()?;
    Ok(())
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<Digest> {
    let data = bytes(value)?;
    let hash = rx_package::content_digest(&data);
    write(path, &data)?;
    Ok(hash)
}
fn add_artifact(
    pool: &mut BTreeMap<Digest, Vec<u8>>,
    schema: &str,
    value: serde_json::Value,
) -> Result<ArtifactRef> {
    let data = bytes(&value)?;
    if value.get("schema").and_then(|v| v.as_str()) != Some(schema) {
        return Err("artifact schema mismatch".into());
    }
    let reference = ArtifactRef {
        sha256: rx_package::content_digest(&data),
        schema_id: name(schema),
        size_bytes: Counter(data.len() as u64),
    };
    pool.insert(reference.sha256, data);
    Ok(reference)
}
struct SeedMaterials {
    seed: Seed,
    cell: CellConfiguration,
    input: CompileInput,
    policy: policy::Policy,
    pool: BTreeMap<Digest, Vec<u8>>,
}

fn read_seed(path: &Path) -> Result<SeedMaterials> {
    let seed: Seed = read(&path.join("seed.json"))?;
    if seed.schema != name("rx.delivery-fixture-seed.v1") {
        return Err("seed schema differs".into());
    }
    for (file, expected) in [
        ("initial-cell.json", seed.initial_cell_sha256),
        ("compile-input.json", seed.compile_input_sha256),
        ("package-recipe.json", seed.package_recipe_sha256),
        ("package-policy.json", seed.package_policy_sha256),
    ] {
        if rx_package::content_digest(&read_bytes(&path.join(file))?) != expected {
            return Err(format!("seed {file} changed").into());
        }
    }
    let cell: CellConfiguration = read(&path.join("initial-cell.json"))?;
    let input: CompileInput = read(&path.join("compile-input.json"))?;
    input.validate()?;
    let policy: policy::Policy = read(&path.join("package-policy.json"))?;
    let mut pool = BTreeMap::new();
    for (hash, reference) in &seed.artifacts {
        let data = read_bytes(&path.join("assets").join(format!("{hash}.bin")))?;
        if *hash != reference.sha256
            || rx_package::content_digest(&data) != *hash
            || data.len() as u64 != reference.size_bytes.0
        {
            return Err("seed artifact changed".into());
        }
        pool.insert(*hash, data);
    }
    Ok(SeedMaterials {
        seed,
        cell,
        input,
        policy,
        pool,
    })
}
