//! Bootstrap stages and typed reason codes only; never log arbitrary peer error text.
use super::Error;
use rx_domain::types::{Name, TimePoint};
use rx_ports::StoreError;
use rx_runtime::writer::WriterError;

#[derive(Debug)]
struct StageError {
    stage: &'static str,
    source: Error,
    read_timing: Option<serde_json::Value>,
}
impl std::fmt::Display for StageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Host connection stage {} failed", self.stage)
    }
}
impl std::error::Error for StageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}
pub(super) fn at(stage: &'static str, source: Error) -> Error {
    Box::new(StageError {
        stage,
        source,
        read_timing: None,
    })
}

pub(super) fn read_failure(
    source: Error,
    started: &TimePoint,
    finished: &TimePoint,
    read_started: &TimePoint,
    captured: &TimePoint,
    now: &TimePoint,
) -> Error {
    Box::new(StageError {
        stage: "PREPARE_LINK",
        source,
        read_timing: Some(serde_json::json!({
            "configuration_roundtrip_ns":finished.age_ns(started),
            "configuration_age_at_error_ns":now.age_ns(started),
            "source_capture_after_start_ns":captured.age_ns(read_started),
            "source_age_at_error_ns":now.age_ns(read_started),
            "configuration_to_source_ns":read_started.age_ns(finished)
        })),
    })
}

#[derive(Default)]
pub(super) struct Changes(Option<String>);
impl Changes {
    pub(super) fn next(&mut self, host: &Name, cell: &Name, error: &Error) -> Option<String> {
        let (stage, source) = error
            .downcast_ref::<StageError>()
            .map_or(("BOOTSTRAP", error.as_ref()), |e| {
                (e.stage, e.source.as_ref())
            });
        let code = if let Some(status) = source.downcast_ref::<tonic::Status>() {
            format!("RPC_{:?}", status.code())
        } else if let Some(writer) = source.downcast_ref::<WriterError<StoreError>>() {
            match writer {
                WriterError::Busy => "WRITER_BUSY".into(),
                WriterError::Unavailable => "WRITER_UNAVAILABLE".into(),
                WriterError::ThreadStart(_) => "WRITER_THREAD_START".into(),
                WriterError::Rejected(error) => match error {
                    StoreError::Rejected(reason) => reason.to_string(),
                    StoreError::Unavailable(_) => "STORE_UNAVAILABLE".into(),
                    StoreError::Ownership(error) => format!("STORE_OWNERSHIP_{:?}", error.kind),
                    StoreError::Integrity(_) => "STORE_INTEGRITY".into(),
                    StoreError::RevisionConflict(_) => "REVISION_CONFLICT".into(),
                    StoreError::KeyConflict => "KEY_CONFLICT".into(),
                    StoreError::OutboxConflict => "OUTBOX_CONFLICT".into(),
                    StoreError::Invalid(_) => "INVALID_DOCUMENT".into(),
                },
            }
        } else {
            "CONNECTION_BOUNDARY".into()
        };
        // Timing samples change on each attempt; deduplicate on the stage/reason only.
        let signature =
            serde_json::json!({"host":host,"cell":cell,"stage":stage,"code":code}).to_string();
        let line = serde_json::json!({"event":"rx.host-connection-error.v1", "host":host,
            "cell":cell, "stage":stage, "code":code, "read_timing":error.downcast_ref::<StageError>().and_then(|e| e.read_timing.as_ref())})
        .to_string();
        if self.0.as_ref() == Some(&signature) {
            return None;
        }
        self.0 = Some(signature);
        Some(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn varying_timing_does_not_repeat_the_same_failure() {
        let host = Name::new("host/a").unwrap();
        let cell = Name::new("cell/a").unwrap();
        let mut changes = Changes::default();
        for tick in [1, 2] {
            let source: Error = Box::new(WriterError::Rejected(StoreError::Rejected(
                rx_domain::fault::Rejection::ContinuityUnproven,
            )));
            let time = TimePoint {
                clock_id: "test".into(),
                ticks_ns: rx_domain::types::Counter(tick),
            };
            let error = read_failure(source, &time, &time, &time, &time, &time);
            assert_eq!(changes.next(&host, &cell, &error).is_some(), tick == 1);
        }
    }
    #[test]
    fn stage_and_typed_reason_survive_without_logging_remote_text() {
        let host = Name::new("host/a").unwrap();
        let cell = Name::new("cell/a").unwrap();
        let mut changes = Changes::default();
        let error = at(
            "READ_CONFIGURATION",
            Box::new(tonic::Status::data_loss("secret body")),
        );
        let line = changes.next(&host, &cell, &error).unwrap();
        assert!(line.contains("READ_CONFIGURATION"));
        assert!(line.contains("RPC_DataLoss"));
        assert!(!line.contains("secret"));
        assert!(changes.next(&host, &cell, &error).is_none());
        let error = at(
            "COMMIT_LINK",
            Box::new(WriterError::Rejected(StoreError::Rejected(
                rx_domain::fault::Rejection::ContinuityUnproven,
            ))),
        );
        let line = changes.next(&host, &cell, &error).unwrap();
        assert!(line.contains("COMMIT_LINK"));
        assert!(line.contains("ContinuityUnproven"));
    }
}
