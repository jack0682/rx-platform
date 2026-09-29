//! Off-writer recovery transport port. Every result still requires current writer authority.
//!
//! Implementations obtain transport pins and reads from release-owned configuration. Callers
//! cannot provide snapshots, endpoints, certificates, filesystem paths or RPC method names.
//! No method grants operation authority, renews a permit, Arms a cell or replays native work.
use crate::writer::WriterError;
use rx_application::{Identity, host_recovery as recovery};
use rx_domain::types::{Id, Name};
use rx_ports::StoreError;
use std::{future::Future, pin::Pin};

pub use recovery::QueryResult;

#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error("Host recovery writer rejected or lost the request")]
    Writer(#[from] WriterError<StoreError>),
    #[error("Host recovery transport unavailable")]
    Unavailable,
    #[error("Host recovery read could not be validated")]
    InvalidRead,
    #[error("Host recovery worker busy")]
    Busy,
}

pub type ServiceFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, WorkerError>> + Send + 'a>>;

/// Authenticated human requests only. Implementations must repeat current role, terminal,
/// installation and complete Host-cell scope checks through the authoritative writer.
/// An HTTP preflight is not a reusable authorization result.
pub trait Service: Send + Sync {
    fn rebind(
        &self,
        _identity: Identity,
        _key: Id,
        _input: rx_application::host_rejoin::ApproveRebind,
    ) -> ServiceFuture<'_, rx_application::host_rejoin::RebindView> {
        Box::pin(async { Err(WorkerError::Unavailable) })
    }
    fn progress_rebind(
        &self,
        _identity: Identity,
        _id: Id,
    ) -> ServiceFuture<'_, rx_application::host_rejoin::RebindView> {
        Box::pin(async { Err(WorkerError::Unavailable) })
    }

    fn settle_rejoin(
        &self,
        _identity: Identity,
        _key: Id,
        _command: rx_application::host_rejoin::ApproveSettlement,
    ) -> ServiceFuture<'_, rx_application::settlement::Authorization> {
        Box::pin(async { Err(WorkerError::Unavailable) })
    }

    fn query_rejoin(
        &self,
        _identity: Identity,
        _id: Id,
        _operation: Id,
    ) -> ServiceFuture<'_, rx_application::host_rejoin::QueryObservation> {
        Box::pin(async { Err(WorkerError::Unavailable) })
    }

    fn approve_rejoin(
        &self,
        _identity: Identity,
        _key: Id,
        _input: rx_application::host_rejoin::Approve,
    ) -> ServiceFuture<'_, rx_application::host_rejoin::BindingView> {
        Box::pin(async { Err(WorkerError::Unavailable) })
    }
    fn progress_rejoin(
        &self,
        _identity: Identity,
        _id: Id,
    ) -> ServiceFuture<'_, rx_application::host_rejoin::BindingView> {
        Box::pin(async { Err(WorkerError::Unavailable) })
    }

    fn propose_rejoin(
        &self,
        _identity: Identity,
        _key: Id,
        _input: rx_application::host_rejoin::Prepare,
    ) -> ServiceFuture<'_, rx_application::host_rejoin::ProposalView> {
        Box::pin(async { Err(WorkerError::Unavailable) })
    }
    fn context(
        &self,
        identity: Identity,
        host: Name,
        origin: Name,
    ) -> ServiceFuture<'_, recovery::Context>;

    /// Discover existing records for another authenticated browser/release manager without creating
    /// ownership or approval. The writer filters current access and bounds the page to 1..50.
    fn list(
        &self,
        identity: Identity,
        host: Name,
        after: Option<Id>,
        limit: usize,
    ) -> ServiceFuture<'_, recovery::RecoveryPage>;

    /// Recover the same original request key before creating any new proposal.
    fn propose(
        &self,
        identity: Identity,
        key: Id,
        input: recovery::Prepare,
    ) -> ServiceFuture<'_, recovery::View>;

    /// Only the exact existing proposal/CAS and original request key can be approved.
    /// Network loss may leave the returned binding FENCING, never operation-authorized.
    fn approve(
        &self,
        identity: Identity,
        key: Id,
        input: recovery::Approve,
    ) -> ServiceFuture<'_, recovery::View>;

    fn get(&self, identity: Identity, id: Id) -> ServiceFuture<'_, recovery::View>;

    /// Continue existing approved Fence tasks with their recorded request IDs. This does not
    /// create approval, replace a Fence key or make an unapproved proposal operational.
    fn progress(&self, identity: Identity, id: Id) -> ServiceFuture<'_, recovery::View>;

    /// Read/reconcile one existing operation. Neither a missing result nor a lost response is
    /// permission to resend its operation, mint a permit or select another Host RPC.
    fn query(&self, identity: Identity, id: Id, operation: Id) -> ServiceFuture<'_, QueryResult>;
}
