//! Bounded investigation artifact verification. No device or writer mutation occurs here.
use rx_application::investigation as data;
use rx_domain::{canonical, types::*};
use rx_package::{PackagePath, SignatureEnvelope};
use std::{path::PathBuf, sync::Arc};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("investigation worker busy")]
    Busy,
    #[error("investigation worker unavailable")]
    Unavailable,
    #[error("investigation artifact or policy verification failed")]
    VerificationFailed,
}
pub struct Worker {
    root: PathBuf,
    policy_file: PathBuf,
    pin: Digest,
    policy: data::Policy,
    permit: Arc<tokio::sync::Semaphore>,
}
impl Worker {
    pub fn new(root: PathBuf, policy_file: PathBuf, pin: Digest) -> Result<Arc<Self>, String> {
        if !root.is_absolute() || !policy_file.is_absolute() {
            return Err("absolute investigation artifact and policy paths required".into());
        }
        let (policy, digest) = rx_package::policy::read_with_digest::<data::Policy>(&policy_file)
            .map_err(|e| e.to_string())?;
        if digest != pin {
            return Err("investigation policy file pin differs".into());
        }
        policy.digest()?;
        Ok(Arc::new(Self {
            root,
            policy_file,
            pin,
            policy,
            permit: Arc::new(tokio::sync::Semaphore::new(1)),
        }))
    }
    pub fn policy(&self) -> data::Policy {
        self.policy.clone()
    }
    pub fn policy_file_digest(&self) -> Digest {
        self.pin
    }
    fn current_policy(&self) -> Result<data::Policy, Error> {
        let (policy, pin) = rx_package::policy::read_with_digest::<data::Policy>(&self.policy_file)
            .map_err(|_| Error::VerificationFailed)?;
        if pin != self.pin
            || policy.digest().map_err(|_| Error::VerificationFailed)?
                != self
                    .policy
                    .digest()
                    .map_err(|_| Error::VerificationFailed)?
        {
            return Err(Error::VerificationFailed);
        }
        Ok(policy)
    }
    fn verify(
        &self,
        expected: &ArtifactRef,
        policy_digest: Digest,
        file_digest: Digest,
    ) -> Result<data::ValidatedProcedure, Error> {
        if file_digest != self.pin {
            return Err(Error::VerificationFailed);
        }
        let policy = self.current_policy()?;
        if policy.digest().map_err(|_| Error::VerificationFailed)? != policy_digest {
            return Err(Error::VerificationFailed);
        }
        let read = |suffix: &str, limit: u64| {
            let path = PackagePath::new(format!("{}{suffix}", expected.sha256))
                .map_err(|_| Error::VerificationFailed)?;
            rx_package::directory::read_relative_file(&self.root, &path, limit)
                .map_err(|_| Error::VerificationFailed)
        };
        let bytes = read(
            ".json",
            rx_process_contract::investigation::MAX_BYTES as u64,
        )?;
        let signature: SignatureEnvelope = canonical::decode_json(&read(".sig.json", 4096)?)
            .map_err(|_| Error::VerificationFailed)?;
        data::ValidatedProcedure::check(&policy, expected, &bytes, signature)
            .map_err(|_| Error::VerificationFailed)
    }
    pub async fn prepare_attestation(
        self: &Arc<Self>,
        ticket: data::Ticket,
    ) -> Result<data::Prepared, Error> {
        let permit = self
            .permit
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Busy)?;
        let worker = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let verified = worker.verify(
                ticket.procedure_reference(),
                ticket.policy_digest(),
                ticket.policy_file_digest(),
            )?;
            data::Prepared::new(ticket, verified).map_err(|_| Error::VerificationFailed)
        })
        .await
        .map_err(|_| Error::Unavailable)?
    }
    pub async fn prepare_disposition(
        self: &Arc<Self>,
        ticket: data::DispositionTicket,
    ) -> Result<data::PreparedDisposition, Error> {
        let permit = self
            .permit
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Busy)?;
        let worker = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let verified = worker.verify(
                ticket.procedure_reference(),
                ticket.policy_digest(),
                ticket.policy_file_digest(),
            )?;
            data::PreparedDisposition::new(ticket, verified).map_err(|_| Error::VerificationFailed)
        })
        .await
        .map_err(|_| Error::Unavailable)?
    }
}
