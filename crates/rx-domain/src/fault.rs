use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Rejection {
    Forbidden,
    Unauthenticated,
    NotFound,
    NotCommissioned,
    QualificationRequired,
    ConditionFailed,
    ConditionUnknown,
    StaleRevision,
    Expired,
    StaleEpoch,
    MandateRevoked,
    BudgetExhausted,
    BlockedByCase,
    CapabilityMissing,
    Busy,
    InvalidInput,
    UnsupportedSchema,
    HostNotPrepared,
    ContinuityUnproven,
}
impl std::fmt::Display for Rejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
