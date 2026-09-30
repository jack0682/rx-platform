//! Scoped permission to publish attributed registry observations, not to operate components.
use rx_domain::{resident_reporting::Scope, types::*};
use serde::{Deserialize, Serialize};

/// Constructed only by the certificate-verifying ingress; never decoded from a request body.
#[derive(Clone, Debug)]
pub struct ReporterIdentity {
    pub principal: Name,
    pub session: Id,
    pub authentication_binding: Digest,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Issue {
    pub component: Id,
    pub expected_component_revision: Counter,
    pub reporter_session: Id,
    pub source_registration: Id,
    pub source_revision: Counter,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revoke {
    pub scope: Id,
    pub expected_revision: Counter,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeView {
    pub revision: Counter,
    pub scope: Scope,
}
