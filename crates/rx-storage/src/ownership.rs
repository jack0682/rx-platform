//! A non-cloneable cooperating-process lock, not a security boundary against raw syscalls.
use rx_ports::{OwnershipError, OwnershipFailure};
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

/// Rust cannot prohibit fork from copying this object. The creator-process check
/// is the enforcement, not the type. It is local, ephemeral identity, never a
/// persisted PID capability or authority to adopt a process after restart.
/// ```compile_fail
/// use rx_storage::ExclusiveFileLock;
/// fn clone_owner(owner: ExclusiveFileLock) { let _ = owner.clone(); }
/// ```
/// ```compile_fail
/// use rx_storage::ExclusiveFileLock;
/// fn extract_file(owner: ExclusiveFileLock) { let _ = owner.file; }
/// ```
pub struct ExclusiveFileLock {
    file: Option<File>,
    creator: u32,
}
fn error(kind: OwnershipFailure, detail: impl ToString) -> OwnershipError {
    OwnershipError {
        kind,
        detail: detail.to_string(),
    }
}
impl ExclusiveFileLock {
    pub fn acquire(path: impl AsRef<Path>) -> Result<Self, OwnershipError> {
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(path)
            .map_err(|e| error(OwnershipFailure::Acquire, e))?;
        file.try_lock().map_err(|e| match e {
            std::fs::TryLockError::WouldBlock => error(OwnershipFailure::Contended,
                "exclusive owner not established; a live owner or inherited description may retain the lock"),
            std::fs::TryLockError::Error(e) => error(OwnershipFailure::Acquire,e),
        })?;
        // A failed contender never constructs an unlocking guard.
        Ok(Self {
            file: Some(file),
            creator: std::process::id(),
        })
    }
    pub(crate) fn creator(&self) -> u32 {
        self.creator
    }
    pub fn check_process(&self) -> Result<(), OwnershipError> {
        check_creator(self.creator)
    }
    pub fn close(mut self) -> Result<(), OwnershipError> {
        self.release()
    }
    fn release(&mut self) -> Result<(), OwnershipError> {
        self.check_process()?;
        let Some(file) = self.file.take() else {
            return Ok(());
        };
        if let Err(e) = file.unlock() {
            // No automatic retry. Keep the descriptor until process exit;
            // an unsuccessful release is never reported as confirmed.
            std::mem::forget(file);
            return Err(error(OwnershipFailure::Release, e));
        }
        Ok(())
    }
    pub(crate) fn retain_until_process_exit(mut self) {
        if let Some(file) = self.file.take() {
            std::mem::forget(file);
        }
    }
}
impl Drop for ExclusiveFileLock {
    fn drop(&mut self) {
        if self.creator == std::process::id() {
            let _ = self.release();
        }
        // In a fork child, close only this copy. Never LOCK_UN the parent's OFD.
    }
}

pub(crate) fn check_creator(creator: u32) -> Result<(), OwnershipError> {
    if creator != std::process::id() {
        Err(error(
            OwnershipFailure::ForeignProcess,
            "inherited ownership cannot be used or released by this process",
        ))
    } else {
        Ok(())
    }
}
