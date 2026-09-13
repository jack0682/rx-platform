//! Platform process composition. Qualification and physical Host shutdown remain separate gates.
pub mod config;
use config::{Catalog, Loaded};
use rx_application::*;
use rx_domain::{canonical, types::*};
use rx_runtime::{
    application::{Application, Command, Handle, Reply},
    writer::{Status, Writer},
};
use rx_storage::SqliteRepository;
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::Path, sync::Arc};
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
/// The executable deliberately cannot mint qualification from a startup file or a PASS boolean.
/// A verified release/package authority must replace this port before physical qualification.
pub struct UnconnectedQualification;
impl QualificationAuthority for UnconnectedQualification {
    fn verify(&self, _: &CellConfiguration, _: &[ArtifactRef], _: &[Digest]) -> bool {
        false
    }
}
struct InitClock;
impl Clock for InitClock {
    fn now(&self) -> TimePoint {
        TimePoint {
            clock_id: "installation-init".into(),
            ticks_ns: Counter(0),
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Descriptor {
    schema: Name,
    installation_id: Id,
    catalog_digest: Digest,
    qualification_mode: String,
}
#[derive(Serialize)]
struct ProcessStatus<'a> {
    schema: &'static str,
    installation_id: &'a Id,
    runtime_boot: Option<&'a Id>,
    phase: &'a str,
    qualification_authority: &'static str,
    physical_shutdown_assessed: bool,
    stop: Option<&'a lifecycle::StopReport>,
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).expect("uuid")
}
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
pub fn initialize(config_path: &Path) -> Result<()> {
    let loaded = Loaded::read(config_path)?;
    let config = &loaded.config;
    // Validate credentials/TLS before creating an installation. They are not copied into its data store.
    rx_api::auth::Auth::new(loaded.credentials).map_err(|_| "invalid credential catalog")?;
    if config.data_directory.exists() {
        return Err("installation directory already exists".into());
    }
    let parent = config
        .data_directory
        .parent()
        .ok_or("data directory parent required")?;
    let temp = tempfile::Builder::new()
        .prefix(".rx-init-")
        .tempdir_in(parent)?;
    let mut engine = Engine::open(
        SqliteRepository::open(temp.path().join("platform.db"))?,
        InitClock,
        UnconnectedQualification,
        config.installation_id.clone(),
        loaded.catalog.bootstrap.clone(),
    )?;
    install_catalog(&mut engine, &loaded.catalog)?;
    drop(engine.into_repository());
    write_private(
        &temp.path().join("installation.json"),
        &canonical::bytes(&Descriptor {
            schema: Name::new("rx.platform-installation.v1")?,
            installation_id: config.installation_id.clone(),
            catalog_digest: config.catalog.sha256,
            qualification_mode: "UNCOMMISSIONED_DRAFT".into(),
        })?,
    )?;
    fs::File::open(temp.path())?.sync_all()?;
    fs::rename(temp.path(), &config.data_directory)?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}
fn install_catalog<R: rx_ports::Repository, C: Clock, A: QualificationAuthority>(
    engine: &mut Engine<R, C, A>,
    catalog: &Catalog,
) -> Result<()> {
    let session = engine.authenticated_session(
        &catalog.bootstrap.id,
        id(),
        TimePoint {
            clock_id: "installation-init".into(),
            ticks_ns: Counter(1_000_000),
        },
    )?;
    let identity = Identity {
        principal: catalog.bootstrap.id.clone(),
        session: session.id,
        terminal: None,
    };
    for p in &catalog.principals {
        engine.put_principal(&identity, p.clone(), None)?;
    }
    for t in &catalog.terminals {
        engine.put_terminal(&identity, t.clone(), None)?;
    }
    for c in &catalog.cells {
        engine.install_cell(&identity, c.clone())?;
    }
    engine.end_user_session(&identity)?;
    Ok(())
}

fn status(
    config: &config::Config,
    boot: Option<&Id>,
    phase: &str,
    stop: Option<&lifecycle::StopReport>,
) -> Result<()> {
    if !config.runtime_directory.is_dir()
        || fs::symlink_metadata(&config.runtime_directory)?
            .file_type()
            .is_symlink()
    {
        return Err("runtime directory must exist and be a real directory".into());
    }
    let target = config.runtime_directory.join("platform-status.json");
    if target.exists() {
        let meta = fs::symlink_metadata(&target)?;
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 1_048_576 {
            return Err("status path is not an owned status file".into());
        }
        let old: serde_json::Value = canonical::decode_json(&fs::read(&target)?)?;
        if old.get("schema").and_then(|v| v.as_str()) != Some("rx.platform-process-status.v1")
            || old.get("installation_id").and_then(|v| v.as_str())
                != Some(config.installation_id.as_str())
        {
            return Err("status file belongs to another owner".into());
        }
    }
    let bytes = canonical::bytes(&ProcessStatus {
        schema: "rx.platform-process-status.v1",
        installation_id: &config.installation_id,
        runtime_boot: boot,
        phase,
        qualification_authority: if config
            .package_intake
            .as_ref()
            .is_some_and(|p| p.qualification_policy.is_some())
        {
            "VERIFIED_REQUALIFICATION"
        } else {
            "NOT_CONNECTED"
        },
        physical_shutdown_assessed: false,
        stop,
    })?;
    let temporary = config.runtime_directory.join(format!(".status-{}", id()));
    write_private(&temporary, &bytes)?;
    fs::rename(&temporary, target)?;
    fs::File::open(&config.runtime_directory)?.sync_all()?;
    Ok(())
}

/// Compose the real API services and single writer. No Host/driver launch or run resume is hidden here.
pub async fn serve<C: Clock + 'static>(
    config_path: &Path,
    clock: C,
    shutdown: impl std::future::Future<Output = ()> + Send,
) -> Result<lifecycle::StopReport> {
    let loaded = Loaded::read(config_path)?;
    let config = loaded.config;
    let descriptor_path = config.data_directory.join("installation.json");
    let metadata = fs::symlink_metadata(&descriptor_path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 1_048_576 {
        return Err("invalid installation descriptor".into());
    }
    let descriptor: Descriptor = canonical::decode_json(&fs::read(descriptor_path)?)?;
    if descriptor.schema.as_str() != "rx.platform-installation.v1"
        || descriptor.installation_id != config.installation_id
        || descriptor.catalog_digest != config.catalog.sha256
        || descriptor.qualification_mode != "UNCOMMISSIONED_DRAFT"
    {
        return Err("installation/catalog identity mismatch".into());
    }
    let database = config.data_directory.join("platform.db");
    if !fs::symlink_metadata(&database)?.is_file() {
        return Err("existing platform database required".into());
    }
    if !fs::symlink_metadata(&config.data_directory)?.is_dir()
        || fs::symlink_metadata(&config.data_directory)?
            .file_type()
            .is_symlink()
    {
        return Err("real data directory required".into());
    }
    if !config.runtime_directory.is_dir()
        || fs::symlink_metadata(&config.runtime_directory)?
            .file_type()
            .is_symlink()
    {
        return Err("real runtime directory required".into());
    }
    let lock_path = config.runtime_directory.join("platform.lock");
    if let Ok(meta) = fs::symlink_metadata(&lock_path)
        && (!meta.is_file() || meta.file_type().is_symlink())
    {
        return Err("invalid runtime lock path".into());
    }
    let mut lock_options = fs::OpenOptions::new();
    lock_options
        .create(true)
        .truncate(false)
        .read(true)
        .write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        lock_options.mode(0o600);
    }
    let _runtime_ownership = lock_options.open(lock_path)?;
    _runtime_ownership
        .try_lock()
        .map_err(|_| "another process owns this runtime directory")?;
    status(&config, None, "VALIDATING", None)?;
    // Bind both sockets before opening authority; a port conflict never starts half a service set.
    let https_listener = tokio::net::TcpListener::bind(config.https.bind).await?;
    let grpc_listener = tokio::net::TcpListener::bind(config.grpc.bind).await?;
    let installation = config.installation_id.clone();
    let bootstrap = loaded.catalog.bootstrap;
    let writer = Writer::start(move || {
        Engine::open(
            SqliteRepository::open(database)?,
            clock,
            UnconnectedQualification,
            installation,
            bootstrap,
        )
        .map(Application::new)
    })
    .await?;
    let handle = Handle::new(writer);
    let Reply::Installation(installation) = handle.call(Command::Installation).await? else {
        return Err("installation reply mismatch".into());
    };
    let setup: Result<_> = async {
        let package_worker = if let Some(intake) = loaded.package_intake {
            let store_path = config.data_directory.join("packages");
            let worker = tokio::task::spawn_blocking(move || {
                let store =
                    rx_package::store::Store::open(&store_path).map_err(|e| e.to_string())?;
                let worker = rx_runtime::package_intake::Worker::new_pinned(
                    intake.import_root,
                    store,
                    intake.policy,
                    intake.file_digest,
                    intake.policy_file,
                )?;
                let worker = if let Some(source) = intake.review_authority {
                    rx_runtime::package_intake::Worker::with_review(
                        worker,
                        source.path,
                        source.sha256,
                    )
                } else {
                    Ok(worker)
                }?;
                let worker = if let Some(source) = intake.device_review_authority {
                    rx_runtime::package_intake::Worker::with_device_review(
                        worker,
                        source.path,
                        source.sha256,
                    )?
                } else {
                    worker
                };
                if let Some(source) = intake.qualification_policy {
                    rx_runtime::package_intake::Worker::with_requalification(
                        worker,
                        source.path,
                        source.sha256,
                    )
                } else {
                    Ok(worker)
                }
            })
            .await??;
            let Reply::PackageIntakeRegistration(Some(_)) = handle
                .call(Command::ConfigurePackageIntake(Some(worker.registration())))
                .await?
            else {
                return Err("package registration reply mismatch".into());
            };
            let Reply::ProcessReviewConfigured = handle
                .call(Command::ConfigureProcessReview(worker.review_digest()))
                .await?
            else {
                return Err("review authority configuration reply mismatch".into());
            };
            handle
                .call(Command::ConfigureRequalification(
                    worker.requalification().map(|w| w.policy()),
                ))
                .await?;
            handle
                .call(Command::ConfigureDeviceReview(
                    worker.device_review_digest(),
                ))
                .await?;
            Some(worker)
        } else {
            None
        };
        let investigation = if let Some(input) = config.investigation.clone() {
            let worker = tokio::task::spawn_blocking(move || {
                rx_runtime::investigation::Worker::new(
                    input.artifact_root,
                    input.policy.path,
                    input.policy.sha256,
                )
            })
            .await??;
            let Reply::InvestigationPolicy(_) = handle
                .call(Command::RegisterInvestigationPolicy {
                    policy: worker.policy(),
                    policy_file_digest: worker.policy_file_digest(),
                })
                .await?
            else {
                return Err("investigation policy registration reply differs".into());
            };
            Some(worker)
        } else {
            None
        };
        let recovery = Arc::new(rx_host_client::recovery::Worker::new(
            Arc::new(handle.clone()),
            loaded.host_links.clone(),
        )?);
        recovery.register().await?;
        let https = rx_api::terminal_https::TerminalHttps::new_with_investigation(
            Arc::new(handle.clone()),
            loaded.credentials,
            rx_api::terminal_https::HttpsPolicy::new(&config.https.origin)?,
            loaded.https_tls,
            package_worker,
            loaded.operator_ui,
            Some(recovery),
            investigation,
        )?;
        let grpc = rx_api::grpc::PlatformIngress::new(
            Arc::new(handle.clone()),
            rx_api::grpc::Configuration {
                installation: installation.clone(),
                release_digest: config.release_digest,
                allowed_certificates: config.grpc.allowed_certificates.clone(),
            },
        )
        .await?;
        let targets = loaded
            .host_links
            .iter()
            .map(|link| rx_application::service_health::Target {
                host: link.host.clone(),
                cell: link.cell.clone(),
            })
            .collect();
        let Reply::HostServiceOwners(owners) =
            handle.call(Command::ConfigureHostServices(targets)).await?
        else {
            return Err("service diagnostic owners reply".into());
        };
        Ok((https, grpc, loaded.grpc_tls, owners))
    }
    .await;
    let (https, grpc, grpc_tls, diagnostic_owners) = match setup {
        Ok(value) => value,
        Err(error) => {
            let _ = handle.call(Command::RequestRuntimeStop).await;
            let _ = handle.call(Command::CommitRuntimeProcessStop).await;
            handle.close();
            let _ = handle.closed().await;
            let _ = status(
                &config,
                Some(&installation.runtime_boot),
                "STARTUP_FAILED",
                None,
            );
            return Err(error);
        }
    };
    let host_links = loaded.host_links;
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let mut services = tokio::task::JoinSet::new();
    let mut diagnostics = tokio::task::JoinSet::new();
    let mut h_stop = stopped.clone();
    services.spawn(async move {
        (
            "https",
            https
                .serve(https_listener, async {
                    let _ = h_stop.changed().await;
                })
                .await
                .map_err(|e| e.to_string()),
        )
    });
    let mut g_stop = stopped.clone();
    services.spawn(async move {
        (
            "grpc",
            grpc.serve(grpc_listener, grpc_tls, async move {
                let _ = g_stop.changed().await;
            })
            .await
            .map_err(|e| e.to_string()),
        )
    });
    let monitor_runtime = Arc::new(handle.clone());
    let monitor_stopped = stopped.clone();
    services.spawn(async move {
        (
            "maintained-conditions",
            rx_host_client::observation::monitor_maintained(monitor_runtime, monitor_stopped)
                .await
                .map_err(|e| e.to_string()),
        )
    });
    for (configuration, owner) in host_links.into_iter().zip(diagnostic_owners) {
        let link = rx_host_client::connection::ConnectionService::new(
            Arc::new(handle.clone()),
            configuration,
        )?;
        let relay =
            rx_host_client::service_health::Relay::new(Arc::new(handle.clone()), owner, &link)?;
        diagnostics.spawn(relay.run(stopped.clone()));
        let receiver = stopped.clone();
        services.spawn(async move {
            (
                "host-link",
                link.run(receiver).await.map_err(|e| e.to_string()),
            )
        });
    }
    let status_failed = status(
        &config,
        Some(&installation.runtime_boot),
        "SOFTWARE_READY_UNCOMMISSIONED",
        None,
    )
    .is_err();
    tokio::pin!(shutdown);
    let mut health = tokio::time::interval(std::time::Duration::from_millis(100));
    let mut fault = status_failed;
    while !fault {
        tokio::select! {
            _=&mut shutdown=>break,
            result=services.join_next()=>{let _=result;fault=true;break;},
            _=health.tick()=>{if handle.status()!=Status::Running {fault=true;break;}}
        }
    }
    let barrier = handle.call(Command::RequestRuntimeStop).await;
    let _ = status(
        &config,
        Some(&installation.runtime_boot),
        "STOP_REQUESTED",
        match &barrier {
            Ok(Reply::RuntimeStop(r)) => Some(r),
            _ => None,
        },
    );
    let _ = stop.send(true);
    if tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while services.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        services.abort_all();
        while services.join_next().await.is_some() {}
        fault = true;
    }
    // Telemetry is not a critical authority service. It is bounded and drained separately.
    if tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while diagnostics.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        diagnostics.abort_all();
        while diagnostics.join_next().await.is_some() {}
    }
    let final_report = handle.call(Command::CommitRuntimeProcessStop).await;
    handle.close();
    let writer_state = handle.closed().await;
    if barrier.is_err() || writer_state != Status::Stopped {
        let _ = status(
            &config,
            Some(&installation.runtime_boot),
            "WRITER_FAULT",
            None,
        );
        return Err("runtime could not record a clean software stop".into());
    }
    let Reply::RuntimeStop(report) = final_report? else {
        return Err("stop report mismatch".into());
    };
    status(
        &config,
        Some(&installation.runtime_boot),
        if fault {
            "SERVICE_FAILED"
        } else {
            "PROCESS_STOPPED"
        },
        Some(&report),
    )?;
    if fault {
        return Err("one or more services failed; inspect the persisted stop report".into());
    }
    Ok(*report)
}

#[cfg(target_os = "linux")]
pub struct LinuxBoottime {
    id: String,
}
#[cfg(target_os = "linux")]
impl LinuxBoottime {
    pub fn new() -> Result<Self> {
        let boot = Id::new(fs::read_to_string("/proc/sys/kernel/random/boot_id")?.trim())?;
        Ok(Self {
            id: format!("linux-boottime/{boot}"),
        })
    }
}
#[cfg(target_os = "linux")]
impl Clock for LinuxBoottime {
    fn now(&self) -> TimePoint {
        let value = rustix::time::clock_gettime(rustix::time::ClockId::Boottime);
        let ticks = u64::try_from(value.tv_sec)
            .expect("nonnegative boottime")
            .checked_mul(1_000_000_000)
            .and_then(|v| v.checked_add(u64::try_from(value.tv_nsec).expect("nonnegative nanos")))
            .expect("boottime range");
        TimePoint {
            clock_id: self.id.clone(),
            ticks_ns: Counter(ticks),
        }
    }
}
