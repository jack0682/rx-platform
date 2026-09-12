//! Optional executor-plan binding: only confirmed atomic checkpoint rejections.
use crate::base::{ErrorDetail, Reason, ReasonCode, RetryAction};
use prost::Message;
use tonic::{Code, Status, metadata::MetadataValue};
const HEADER: &str = "rx-checkpoint-error-bin";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    Revision,
    Expired,
}
pub fn status(reason: Rejection) -> Status {
    let (code, reason_code, retry) = match reason {
        Rejection::Revision => (
            Code::Aborted,
            ReasonCode::RevisionConflict,
            RetryAction::Refresh,
        ),
        Rejection::Expired => (
            Code::FailedPrecondition,
            ReasonCode::Expired,
            RetryAction::Reconcile,
        ),
    };
    let detail = ErrorDetail {
        reason: Some(Reason {
            code: reason_code as i32,
            detail: None,
            related_ids: vec![],
        }),
        retry_action: retry as i32,
        operation_id: None,
        expected_revision: None,
        earliest_cursor: None,
        next_expected_seq: None,
    };
    let mut status = Status::new(code, "checkpoint proposal rejected before commit");
    status
        .metadata_mut()
        .insert_bin(HEADER, MetadataValue::from_bytes(&detail.encode_to_vec()));
    status
}
pub fn confirmed(status: &Status) -> Option<Rejection> {
    let values = status.metadata().get_all_bin(HEADER);
    let mut values = values.iter();
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    let bytes = value.to_bytes().ok()?;
    if bytes.len() > 1024 {
        return None;
    }
    let detail: ErrorDetail = crate::strict::decode(&bytes).ok()?;
    crate::json::to_value(&detail).ok()?;
    let reason = detail.reason?;
    if reason.detail.is_some()
        || !reason.related_ids.is_empty()
        || detail.operation_id.is_some()
        || detail.expected_revision.is_some()
        || detail.earliest_cursor.is_some()
        || detail.next_expected_seq.is_some()
    {
        return None;
    }
    match (
        status.code(),
        ReasonCode::try_from(reason.code).ok()?,
        RetryAction::try_from(detail.retry_action).ok()?,
    ) {
        (Code::Aborted, ReasonCode::RevisionConflict, RetryAction::Refresh) => {
            Some(Rejection::Revision)
        }
        (Code::FailedPrecondition, ReasonCode::Expired, RetryAction::Reconcile) => {
            Some(Rejection::Expired)
        }
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_transport_errors_and_malformed_or_duplicated_details_do_not_prove_rejection() {
        for rejection in [Rejection::Revision, Rejection::Expired] {
            let mut value = status(rejection);
            assert_eq!(confirmed(&value), Some(rejection));
            let metadata = value.metadata().get_bin(HEADER).unwrap().clone();
            value.metadata_mut().append_bin(HEADER, metadata);
            assert_eq!(confirmed(&value), None);
        }
        for value in [
            Status::aborted("REVISION_CONFLICT"),
            Status::failed_precondition("EXPIRED"),
            Status::unavailable("EXPIRED"),
        ] {
            assert_eq!(confirmed(&value), None);
        }
        let mut value = Status::failed_precondition("untrusted text");
        value
            .metadata_mut()
            .insert_bin(HEADER, MetadataValue::from_bytes(&[255, 0]));
        assert_eq!(confirmed(&value), None);
        let mut value = Status::unavailable("wrong code");
        value.metadata_mut().insert_bin(
            HEADER,
            status(Rejection::Expired)
                .metadata()
                .get_bin(HEADER)
                .unwrap()
                .clone(),
        );
        assert_eq!(confirmed(&value), None);
    }
}
