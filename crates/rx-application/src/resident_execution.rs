//! The execution ingress constructs this identity only after certificate verification.
use rx_domain::types::*;
#[derive(Clone, Debug)]
pub struct Identity {
    pub principal: Name,
    pub session: Id,
    pub authentication_binding: Digest,
}
