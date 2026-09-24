use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use rx_domain::fault::Rejection;
use rx_ports::StoreError;
use rx_runtime::writer::WriterError;
use serde::Serialize;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub outcome_unknown: bool,
}
impl ApiError {
    pub fn new(status: StatusCode, code: &'static str) -> Self {
        Self {
            status,
            code,
            outcome_unknown: false,
        }
    }
    pub fn unauthenticated() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "UNAUTHENTICATED")
    }
    pub fn forbidden() -> Self {
        Self::new(StatusCode::FORBIDDEN, "FORBIDDEN")
    }
    pub fn invalid() -> Self {
        Self::new(StatusCode::BAD_REQUEST, "INVALID_INPUT")
    }
    pub fn unavailable() -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "UNAVAILABLE",
            outcome_unknown: true,
        }
    }
    pub fn busy() -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, "BUSY")
    }
}
#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    outcome_unknown: bool,
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                code: self.code,
                outcome_unknown: self.outcome_unknown,
            }),
        )
            .into_response()
    }
}
impl From<WriterError<StoreError>> for ApiError {
    fn from(error: WriterError<StoreError>) -> Self {
        use StatusCode as S;
        match error {
            WriterError::Busy => Self::busy(),
            WriterError::Rejected(StoreError::Rejected(reason)) => {
                let (status, code) = match reason {
                    Rejection::Unauthenticated => (S::UNAUTHORIZED, "UNAUTHENTICATED"),
                    Rejection::Forbidden => (S::FORBIDDEN, "FORBIDDEN"),
                    Rejection::NotFound => (S::NOT_FOUND, "NOT_FOUND"),
                    Rejection::InvalidInput => (S::BAD_REQUEST, "INVALID_INPUT"),
                    Rejection::UnsupportedSchema => (S::BAD_REQUEST, "UNSUPPORTED_SCHEMA"),
                    Rejection::NotCommissioned => (S::CONFLICT, "NOT_COMMISSIONED"),
                    Rejection::QualificationRequired => (S::CONFLICT, "QUALIFICATION_REQUIRED"),
                    Rejection::ConditionFailed => (S::CONFLICT, "CONDITION_FAILED"),
                    Rejection::ConditionUnknown => (S::CONFLICT, "CONDITION_UNKNOWN"),
                    Rejection::StaleRevision => (S::CONFLICT, "STALE_REVISION"),
                    Rejection::Expired => (S::CONFLICT, "EXPIRED"),
                    Rejection::StaleEpoch => (S::CONFLICT, "STALE_EPOCH"),
                    Rejection::MandateRevoked => (S::CONFLICT, "MANDATE_REVOKED"),
                    Rejection::BudgetExhausted => (S::CONFLICT, "BUDGET_EXHAUSTED"),
                    Rejection::BlockedByCase => (S::CONFLICT, "BLOCKED_BY_CASE"),
                    Rejection::CapabilityMissing => (S::CONFLICT, "CAPABILITY_MISSING"),
                    Rejection::Busy => (S::CONFLICT, "BUSY"),
                    Rejection::HostNotPrepared => (S::CONFLICT, "HOST_NOT_PREPARED"),
                    Rejection::ContinuityUnproven => (S::CONFLICT, "CONTINUITY_UNPROVEN"),
                };
                Self::new(status, code)
            }
            WriterError::Rejected(StoreError::KeyConflict) => {
                Self::new(S::CONFLICT, "KEY_CONFLICT")
            }
            WriterError::Rejected(StoreError::RevisionConflict(_)) => {
                Self::new(S::CONFLICT, "STALE_REVISION")
            }
            WriterError::Rejected(StoreError::OutboxConflict) => {
                Self::new(S::CONFLICT, "DELIVERY_CONFLICT")
            }
            WriterError::Rejected(StoreError::Invalid(_)) => Self::invalid(),
            WriterError::Rejected(StoreError::Ownership(_)) => Self::unavailable(),
            // Never expose database paths, SQL or internals, or imply a failed reply undid a commit.
            _ => Self::unavailable(),
        }
    }
}
