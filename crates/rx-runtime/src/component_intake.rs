//! Off-writer source I/O selected only by trusted startup configuration and authorized tickets.
use crate::{
    application::{ApplicationPort, CallFuture, Command, Reply},
    writer::{Status, WriterError},
};
use rx_application::component_intake::{self as intake, Reader, SourceBinding};
use rx_domain::{canonical, types::*};
use rx_ports::StoreError;
use rx_storage::SqliteRepository;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::Semaphore;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub path: PathBuf,
    pub owner: Name,
}
#[derive(Clone)]
struct Enrolled {
    path: PathBuf,
    binding: SourceBinding,
    identity: Arc<Mutex<Option<Digest>>>,
}
fn location(path: &std::path::Path) -> Result<Digest, StoreError> {
    let meta =
        std::fs::symlink_metadata(path).map_err(|e| StoreError::Unavailable(e.to_string()))?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() == 0 {
        return Err(StoreError::Invalid(
            "regular existing registry source required".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 {
            return Err(StoreError::Invalid(
                "aliased registry source refused".into(),
            ));
        }
        canonical::digest(
            "RX-REGISTRY-SOURCE-LOCATION-v1",
            &(path, meta.dev(), meta.ino()),
        )
        .map_err(|e| StoreError::Invalid(e.to_string()))
    }
    #[cfg(not(unix))]
    {
        Err(StoreError::Unavailable(
            "source identity supported on Unix only".into(),
        ))
    }
}
pub struct Service {
    inner: Arc<dyn ApplicationPort>,
    sources: BTreeMap<Name, Enrolled>,
    permit: Arc<Semaphore>,
}
impl Service {
    pub async fn configure(
        inner: Arc<dyn ApplicationPort>,
        sources: BTreeMap<Name, Source>,
    ) -> Result<Arc<Self>, String> {
        let mut enrolled = BTreeMap::new();
        for (name, source) in sources {
            if !source.path.is_absolute() {
                return Err("absolute configured source path required".into());
            }
            if source.path.components().any(|c| {
                matches!(
                    c,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )
            }) {
                return Err("source path must be normalized".into());
            }
            // Configured selectors are stable across boots; unavailable sources do not prevent P startup.
            let path = source.path;
            let binding = SourceBinding {
                owner: source.owner,
                location: canonical::digest("RX-REGISTRY-SOURCE-PATH-v1", &path)
                    .map_err(|e| e.to_string())?,
            };
            enrolled.insert(
                name,
                Enrolled {
                    path,
                    binding,
                    identity: Arc::new(Mutex::new(None)),
                },
            );
        }
        let bindings = enrolled
            .iter()
            .map(|(name, entry)| (name.clone(), entry.binding.clone()))
            .collect();
        match inner
            .request(Command::ConfigureComponentSources(bindings))
            .await
            .map_err(|e| format!("source registration failed: {e:?}"))?
        {
            Reply::ComponentSourcesConfigured => {}
            _ => return Err("source registration reply differs".into()),
        }
        Ok(Arc::new(Self {
            inner,
            sources: enrolled,
            permit: Arc::new(Semaphore::new(1)),
        }))
    }
    async fn import(
        &self,
        identity: rx_application::Identity,
        key: Id,
        input: intake::Submit,
    ) -> crate::application::CallResult {
        let reply = self
            .inner
            .request(Command::PrepareComponentIntake {
                identity,
                key,
                input,
            })
            .await?;
        let prepared = match reply {
            Reply::ComponentIntakePreflight(intake::Preflight::Recorded(r)) => {
                return Ok(Reply::ComponentIntakeReceipt(r));
            }
            Reply::ComponentIntakePreflight(intake::Preflight::Read(t)) => *t,
            _ => return Err(WriterError::Unavailable),
        };
        let _permit = self
            .permit
            .clone()
            .try_acquire_owned()
            .map_err(|_| WriterError::Busy)?;
        let source = self
            .sources
            .get(prepared.source())
            .cloned()
            .ok_or(WriterError::Unavailable)?;
        if &source.binding != prepared.binding() {
            return Err(WriterError::Unavailable);
        }
        let mut reader = tokio::task::spawn_blocking(move || {
            let original = std::fs::symlink_metadata(&source.path)
                .map_err(|e| StoreError::Unavailable(e.to_string()))?;
            if original.file_type().is_symlink() {
                return Err(StoreError::Invalid("source symlink refused".into()));
            }
            let path = std::fs::canonicalize(&source.path)
                .map_err(|e| StoreError::Unavailable(e.to_string()))?;
            let actual = location(&path)?;
            let mut identity = source
                .identity
                .lock()
                .map_err(|_| StoreError::Unavailable("source identity lock".into()))?;
            if identity.as_ref().is_some_and(|old| *old != actual) {
                return Err(StoreError::Integrity(
                    "registered source was replaced".into(),
                ));
            }
            let store = SqliteRepository::open_sealed_existing(&path)?;
            if location(&path)? != actual {
                return Err(StoreError::Integrity(
                    "registered source changed during open".into(),
                ));
            }
            let reader = Reader::open(prepared, store)?;
            *identity = Some(actual);
            Ok(reader)
        })
        .await
        .map_err(|_| WriterError::Unavailable)?
        .map_err(WriterError::Rejected)?;
        let Reply::ComponentIntakeProgress(mut progress) = self
            .inner
            .request(Command::BeginComponentIntake(Box::new(reader.begin())))
            .await?
        else {
            return Err(WriterError::Unavailable);
        };
        while progress.declarations < progress.expected_declarations {
            let chunk = reader
                .declarations(progress.declarations)
                .map_err(WriterError::Rejected)?;
            let Reply::ComponentIntakeProgress(next) = self
                .inner
                .request(Command::StageComponentDeclarations(Box::new(chunk)))
                .await?
            else {
                return Err(WriterError::Unavailable);
            };
            progress = next;
        }
        while progress.history_after < progress.history_head {
            let after = progress.history_after;
            let (returned, chunk) = tokio::task::spawn_blocking(move || {
                let chunk = reader.history(after);
                (reader, chunk)
            })
            .await
            .map_err(|_| WriterError::Unavailable)?;
            reader = returned;
            let Reply::ComponentIntakeProgress(next) = self
                .inner
                .request(Command::StageComponentHistory(Box::new(
                    chunk.map_err(WriterError::Rejected)?,
                )))
                .await?
            else {
                return Err(WriterError::Unavailable);
            };
            progress = next;
        }
        // Keep the source owner alive through authoritative target acceptance.
        let result = self
            .inner
            .request(Command::FinishComponentIntake(Box::new(reader.finish())))
            .await;
        drop(reader);
        result
    }
}
impl ApplicationPort for Service {
    fn request(&self, command: Command) -> CallFuture<'_> {
        Box::pin(async move {
            match command {
                Command::ImportComponents {
                    identity,
                    key,
                    input,
                } => self.import(identity, key, input).await,
                other => self.inner.request(other).await,
            }
        })
    }
    fn status(&self) -> Status {
        self.inner.status()
    }
}
