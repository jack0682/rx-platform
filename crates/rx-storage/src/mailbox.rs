//! Bounded offline exchange under a cooperating G1 writer lock. No authority,
//! key loading or interpretation of payload bytes belongs in this transport.
use crate::ExclusiveFileLock;
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Mailbox {
    root: PathBuf,
}
pub struct MailboxGuard {
    root: PathBuf,
    ownership: ExclusiveFileLock,
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
impl Mailbox {
    /// The operator explicitly supplies an existing real directory, not a key.
    pub fn open(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = root.as_ref();
        let meta = fs::symlink_metadata(root)?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(invalid("mailbox/root-not-real-directory"));
        }
        Ok(Self {
            root: fs::canonicalize(root)?,
        })
    }
    pub fn lock(&self) -> io::Result<MailboxGuard> {
        let path = self.root.join("exchange.lock");
        match fs::symlink_metadata(&path) {
            Ok(m) if !m.is_file() || m.file_type().is_symlink() => {
                return Err(invalid("mailbox/lock-file-type"));
            }
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        let ownership = ExclusiveFileLock::acquire(&path).map_err(|e| {
            if e.kind == rx_ports::OwnershipFailure::Contended {
                io::Error::new(io::ErrorKind::WouldBlock, e)
            } else {
                io::Error::other(e)
            }
        })?;
        let count = fs::read_dir(&self.root)?
            .take(2049)
            .collect::<io::Result<Vec<_>>>()?
            .len();
        if count > 2048 {
            return Err(invalid("mailbox/capacity"));
        }
        Ok(MailboxGuard {
            root: self.root.clone(),
            ownership,
        })
    }
}
impl MailboxGuard {
    fn path(&self, name: &str) -> io::Result<PathBuf> {
        self.ownership.check_process().map_err(io::Error::other)?;
        if name.is_empty()
            || name.len() > 100
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            || name.starts_with('.')
        {
            return Err(invalid("mailbox/flat-file-name-required"));
        }
        Ok(self.root.join(name))
    }
    pub fn read(&self, name: &str, limit: usize) -> io::Result<Option<Vec<u8>>> {
        let path = self.path(name)?;
        if limit > 131_072 {
            return Err(invalid("mailbox/read-limit"));
        }
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(
                (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32,
            );
        }
        let file = match options.open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let meta = file.metadata()?;
        if !meta.is_file() || meta.len() > limit as u64 {
            return Err(invalid("mailbox/file-type-or-size"));
        }
        let mut data = Vec::new();
        file.take(limit as u64 + 1).read_to_end(&mut data)?;
        if data.len() > limit {
            return Err(invalid("mailbox/file-too-large"));
        }
        Ok(Some(data))
    }
    /// Complete, synced immutable publication. A stale partial file is refused,
    /// never silently cleared. Repeated identical publication is idempotent.
    pub fn publish_once(&self, name: &str, data: &[u8]) -> io::Result<()> {
        let destination = self.path(name)?;
        if data.len() > 131_072 {
            return Err(invalid("mailbox/publication-too-large"));
        }
        if let Some(old) = self.read(name, 131_072)? {
            return if old == data {
                Ok(())
            } else {
                Err(invalid("mailbox/immutable-publication-conflict"))
            };
        }
        let temporary = self.root.join(format!(".pending-{name}"));
        let mut options = fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        // hard_link never overwrites a publication, including a racing outsider.
        fs::hard_link(&temporary, &destination)?;
        fs::remove_file(&temporary)?;
        fs::File::open(&self.root)?.sync_all()?;
        Ok(())
    }
}
