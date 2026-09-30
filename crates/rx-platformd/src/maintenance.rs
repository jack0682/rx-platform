//! Offline backup and restore of the platform store.
//!
//! Both commands run while no runtime owns the database: opening the store takes the writer
//! lock, so a running runtime makes them fail instead of racing it. Backup uses the SQLite
//! online-backup API into a new file and verifies the copy. Restore replaces the live database
//! with a verified copy of the backup and rotates the store generation, so every Host pinned
//! to the previous generation is refused until it is re-pinned and re-admitted.
use crate::{Result, config::Loaded};
use rx_application::{
    Clock, Installation,
    store_restore::{StoreRestore, restore_store},
};
use rx_domain::{canonical, types::*};
use rx_ports::Repository;
use rx_storage::SqliteRepository;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::{fs, path::Path};

#[derive(Serialize)]
pub struct BackupReport {
    pub schema: &'static str,
    pub installation_id: Id,
    pub destination: String,
    pub sha256: Digest,
    pub journal_head: Counter,
}

#[derive(Serialize)]
pub struct RestoreReport {
    pub schema: &'static str,
    pub installation_id: Id,
    pub source: String,
    pub restore: StoreRestore,
}

fn absolute_regular(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err("absolute path required".into());
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() {
        return Err("regular file required".into());
    }
    Ok(())
}

fn file_sha256(path: &Path) -> Result<Digest> {
    let bytes = fs::read(path)?;
    Ok(Digest::from_bytes(Sha256::digest(&bytes).into()))
}

fn database(loaded: &Loaded) -> Result<std::path::PathBuf> {
    let directory = &loaded.config.data_directory;
    if !fs::symlink_metadata(directory)?.is_dir()
        || fs::symlink_metadata(directory)?.file_type().is_symlink()
    {
        return Err("real data directory required".into());
    }
    let database = directory.join("platform.db");
    absolute_regular(&database).map_err(|_| "existing platform database required")?;
    Ok(database)
}

fn installation_of(repository: &mut SqliteRepository) -> Result<Installation> {
    repository
        .transact(|tx| {
            let record = tx
                .get(&rx_application::persistence::name("installation/current"))?
                .ok_or_else(|| rx_ports::StoreError::Integrity("installation absent".into()))?;
            rx_application::persistence::decode::<Installation>(
                &record,
                "rx.internal.installation.v1",
            )
        })
        .map_err(|e| format!("store installation record: {e}").into())
}

fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// A staged copy that is removed unless the restore renamed it into place.
struct Staged {
    path: std::path::PathBuf,
    armed: bool,
}
impl Staged {
    fn new(path: std::path::PathBuf) -> Result<Self> {
        if path.exists() {
            return Err("a staged restore already exists; inspect and remove it first".into());
        }
        Ok(Self { path, armed: true })
    }
    fn sidecar(&self, suffix: &str) -> std::path::PathBuf {
        self.path.with_file_name(format!(
            "{}{suffix}",
            self.path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
        ))
    }
    /// WAL/SHM of the verification open and the writer lock the store created next to it.
    fn remove_sidecars(&self) {
        for suffix in ["-wal", "-shm"] {
            let _ = fs::remove_file(self.sidecar(suffix));
        }
        let _ = fs::remove_file(self.path.with_extension("writer.lock"));
    }
    fn disarm(mut self) {
        self.armed = false;
    }
}
impl Drop for Staged {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
            self.remove_sidecars();
        }
    }
}

/// Copy the stopped runtime's store into a new file and verify the copy.
pub fn backup(config_path: &Path, destination: &Path) -> Result<BackupReport> {
    let loaded = Loaded::read(config_path)?;
    if !destination.is_absolute() {
        return Err("absolute backup destination required".into());
    }
    if destination.exists() {
        return Err("backup destination must be new".into());
    }
    let database = database(&loaded)?;
    let mut live = SqliteRepository::open(&database)
        .map_err(|e| format!("platform database must not be owned by a running runtime: {e}"))?;
    let installation = installation_of(&mut live)?;
    if installation.id != loaded.config.installation_id {
        return Err("store belongs to another installation".into());
    }
    live.backup(destination)?;
    drop(live);
    let mut copy = SqliteRepository::open(destination)?;
    copy.check_integrity()?;
    let journal_head = copy.journal_head()?;
    let copied = installation_of(&mut copy)?;
    if copied.id != installation.id || copied.store_generation != installation.store_generation {
        return Err("backup copy differs from the live store".into());
    }
    drop(copy);
    Ok(BackupReport {
        schema: "rx.platform-backup-report.v1",
        installation_id: installation.id,
        destination: destination.display().to_string(),
        sha256: file_sha256(destination)?,
        journal_head,
    })
}

/// Replace the stopped runtime's store with a verified copy of `source` and rotate the store
/// generation. The next `run` applies its restart invalidation to the restored cells.
pub fn restore(config_path: &Path, source: &Path) -> Result<RestoreReport> {
    let loaded = Loaded::read(config_path)?;
    absolute_regular(source).map_err(|_| "absolute regular backup file required")?;
    let database = database(&loaded)?;
    let backup_sha256 = file_sha256(source)?;
    // Verify the backup on a private copy inside the data directory before touching the live
    // database; the copy is what becomes the database, so the verified bytes are the used bytes.
    // The guard removes the copy on every early exit.
    let staged = Staged::new(
        loaded
            .config
            .data_directory
            .join(format!(".rx-restore-{}.db", std::process::id())),
    )?;
    fs::copy(source, &staged.path)?;
    fs::File::open(&staged.path)?.sync_all()?;
    let verified = {
        let mut copy = SqliteRepository::open(&staged.path)?;
        copy.check_integrity()?;
        let installation = installation_of(&mut copy)?;
        if installation.id != loaded.config.installation_id {
            return Err("backup belongs to another installation".into());
        }
        copy.close()?;
        installation
    };
    // Prove no runtime owns the live store, then replace it. The lock is released just before
    // the replacement; this is an operator action on a stopped installation, not a runtime path.
    SqliteRepository::open(&database)
        .map_err(|e| format!("platform database must not be owned by a running runtime: {e}"))?
        .close()?;
    for suffix in ["-wal", "-shm"] {
        remove_if_present(&database.with_file_name(format!("platform.db{suffix}")))?;
    }
    staged.remove_sidecars();
    fs::rename(&staged.path, &database)?;
    staged.disarm();
    fs::File::open(&loaded.config.data_directory)?.sync_all()?;
    let mut repository = SqliteRepository::open(&database)?;
    let restore = restore_store(
        &mut repository,
        &verified.id,
        backup_sha256,
        crate::InitClock.now(),
    )?;
    drop(repository);
    Ok(RestoreReport {
        schema: "rx.platform-restore-report.v1",
        installation_id: verified.id,
        source: source.display().to_string(),
        restore,
    })
}

pub fn report_json<T: Serialize>(report: &T) -> Result<String> {
    Ok(String::from_utf8(canonical::bytes(report)?)?)
}
