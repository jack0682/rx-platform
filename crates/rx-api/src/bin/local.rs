//! Local development service only. No Host transport, commissioning authority or device launcher.
use rx_api::{
    LocalPolicy,
    auth::{Credentials, LocalAccount, password_hash},
    router,
};
use rx_application::*;
use rx_domain::{canonical, types::*};
use rx_runtime::{
    application::{Application, Command, Handle},
    writer::Writer,
};
use rx_storage::SqliteRepository;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use zeroize::Zeroizing;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalInstallation {
    schema: String,
    installation_id: Id,
    bootstrap: Principal,
}
struct LocalClock {
    origin: Instant,
    id: String,
}
impl Clock for LocalClock {
    fn now(&self) -> TimePoint {
        TimePoint {
            clock_id: self.id.clone(),
            ticks_ns: Counter(
                u64::try_from(self.origin.elapsed().as_nanos()).expect("local clock range"),
            ),
        }
    }
}
struct NoQualification;
impl QualificationAuthority for NoQualification {
    fn verify(&self, _: &CellConfiguration, _: &[ArtifactRef], _: &[Digest]) -> bool {
        false
    }
}
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("rx-platform-local: {error}");
        std::process::exit(1);
    }
}
async fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("init") if args.len() >= 2 => initialize(Path::new(&args[1]),&args[2..]),
        Some("serve") if (2..=4).contains(&args.len()) => {
            let bind:SocketAddr = args.get(2).map(String::as_str).unwrap_or("127.0.0.1:8080").parse()?;
            if !bind.ip().is_loopback() {return Err("development listener must bind a loopback address".into());}
            let origin = args.get(3).cloned().unwrap_or_else(||format!("http://{bind}"));
            serve(PathBuf::from(&args[1]),bind,LocalPolicy::new(&origin)?).await
        }
        _ => Err("usage: rx-platform-local init NEW_DIRECTORY [CELL_ID ...] < password-file; or serve DIRECTORY [LOOPBACK:PORT [PUBLIC_ORIGIN]]. Default account: admin; default allowed cell: cell/demo. Development only.".into()),
    }
}
fn initialize(directory: &Path, cells: &[String]) -> Result<()> {
    let mut input = Zeroizing::new(String::new());
    io::stdin().take(1026).read_to_string(&mut input)?;
    if input.ends_with('\n') {
        input.pop();
        if input.ends_with('\r') {
            input.pop();
        }
    }
    let hash = password_hash(&input)?;
    let allowed = if cells.is_empty() {
        vec!["cell/demo".to_owned()]
    } else {
        cells.to_vec()
    };
    let allowed = allowed
        .into_iter()
        .map(Name::new)
        .collect::<std::result::Result<_, _>>()?;
    let config = LocalInstallation {
        schema: "rx.development-installation.v1".into(),
        installation_id: Id::new(uuid::Uuid::new_v4().to_string())?,
        bootstrap: Principal {
            id: Name::new("admin")?,
            client_namespace: Name::new("local/admin")?,
            roles: [
                Role::AccountAdmin,
                Role::Engineer,
                Role::Operator,
                Role::RecoveryLead,
                Role::Observer,
            ]
            .into_iter()
            .collect(),
            cells: allowed,
            active: true,
        },
    };
    let credentials = Credentials {
        schema: "rx.local-credentials.v1".into(),
        accounts: vec![LocalAccount {
            principal: config.bootstrap.id.clone(),
            password_hash: hash,
        }],
    };
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(directory)?; // Refuse to overwrite an installation or follow an existing symlink.
    write_new(&directory.join("installation.json"), &config)?;
    write_new(&directory.join("credentials.json"), &credentials)?;
    File::open(directory)?.sync_all()?;
    println!(
        "Local development installation initialized. Account: admin. No cells qualified; no hardware enabled."
    );
    Ok(())
}
fn write_new(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    Ok(())
}
fn private_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.len() > 1024 * 1024 {
        return Err("configuration must be a bounded regular file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err("configuration files must not be group/world accessible".into());
        }
    }
    Ok(canonical::decode_json(&fs::read(path)?)?)
}
async fn serve(directory: PathBuf, bind: SocketAddr, policy: LocalPolicy) -> Result<()> {
    let config: LocalInstallation = private_file(&directory.join("installation.json"))?;
    if config.schema != "rx.development-installation.v1" {
        return Err("not a local development installation".into());
    }
    let credentials: Credentials = private_file(&directory.join("credentials.json"))?;
    let database = directory.join("platform.db");
    let clock = LocalClock {
        origin: Instant::now(),
        id: format!("local-process/{}", uuid::Uuid::new_v4()),
    };
    let writer = Writer::start(move || {
        let store = SqliteRepository::open(database)?;
        Engine::open(
            store,
            clock,
            NoQualification,
            config.installation_id,
            config.bootstrap,
        )
        .map(Application::new)
    })
    .await?;
    let handle = Handle::new(writer);
    let service_path = directory.join("package-service.json");
    let app = if service_path.exists() {
        let c: PackageService = private_file(&service_path)?;
        if c.schema != "rx.development-package-service.v1"
            || !c.import_root.is_absolute()
            || c.import_root.starts_with(&directory)
        {
            return Err("invalid development package service".into());
        }
        let (document, digest) =
            rx_package::policy::read_with_digest::<rx_package::policy::Policy>(&c.policy.path)?;
        if digest != c.policy.sha256 {
            return Err("development package policy pin differs".into());
        }
        let store = rx_package::store::Store::open(&directory.join("packages"))?;
        let worker = rx_runtime::package_intake::Worker::new_pinned(
            c.import_root,
            store,
            document.load()?,
            digest,
            c.policy.path,
        )?;
        let worker = if let Some(review) = c.review_authority {
            rx_runtime::package_intake::Worker::with_review(worker, review.path, review.sha256)?
        } else {
            worker
        };
        handle
            .call(Command::ConfigurePackageIntake(Some(worker.registration())))
            .await?;
        let worker = if let Some(source) = c.device_review_authority {
            rx_runtime::package_intake::Worker::with_device_review(
                worker,
                source.path,
                source.sha256,
            )?
        } else {
            worker
        };
        handle
            .call(Command::ConfigureDeviceReview(
                worker.device_review_digest(),
            ))
            .await?;
        handle
            .call(Command::ConfigureProcessReview(worker.review_digest()))
            .await?;
        rx_api::router_with_package_intake(Arc::new(handle.clone()), credentials, policy, worker)?
    } else {
        router(Arc::new(handle.clone()), credentials, policy)?
    };
    let listener = tokio::net::TcpListener::bind(bind).await?;
    println!(
        "RX development API listening on {}. Device control disabled; terminal authentication unavailable.",
        listener.local_addr()?
    );
    let result = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await;
    handle.close();
    let status = handle.closed().await;
    result?;
    if status != rx_runtime::writer::Status::Stopped {
        return Err("state writer faulted".into());
    }
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {_ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {}}
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PinnedInput {
    path: PathBuf,
    sha256: Digest,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PackageService {
    device_review_authority: Option<PinnedInput>,
    schema: String,
    import_root: PathBuf,
    policy: PinnedInput,
    review_authority: Option<PinnedInput>,
}
