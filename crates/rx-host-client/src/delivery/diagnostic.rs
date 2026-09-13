//! Small change-only diagnostics. No request, key, gRPC metadata or binary details are logged.
use super::{Error, StoreError, WriterError};
use rx_domain::types::Name;

const MAX_LINE_CHARS: usize = 512;

#[derive(Default)]
pub(super) struct Changes {
    previous: Option<String>,
}
impl Changes {
    pub(super) fn next(&mut self, host: &Name, error: &Error) -> Option<String> {
        let line = line(host, error);
        if self.previous.as_ref() == Some(&line) {
            return None;
        }
        self.previous = Some(line.clone());
        Some(line)
    }
}

fn fields(error: &Error) -> (&'static str, String, String) {
    match error {
        // Status Display also renders metadata/details. Only the message is diagnostic input.
        Error::Rpc(status) => (
            "HOST_RPC",
            format!("{:?}", status.code()),
            status.message().into(),
        ),
        Error::Protocol(message) => ("DELIVERY_BOUNDARY", "PROTOCOL".into(), (*message).into()),
        Error::Writer(WriterError::Busy) => (
            "STATE_WRITER",
            "BUSY".into(),
            "state writer queue is full".into(),
        ),
        Error::Writer(WriterError::Unavailable) => (
            "STATE_WRITER",
            "UNAVAILABLE".into(),
            "state writer unavailable".into(),
        ),
        Error::Writer(WriterError::ThreadStart(message)) => {
            ("STATE_WRITER", "THREAD_START".into(), message.clone())
        }
        Error::Writer(WriterError::Rejected(error)) => match error {
            StoreError::Rejected(reason) => (
                "APPLICATION_POLICY",
                reason.to_string(),
                "application policy rejected command".into(),
            ),
            StoreError::Unavailable(message) => {
                ("STATE_STORE", "UNAVAILABLE".into(), message.clone())
            }
            StoreError::Integrity(message) => ("STATE_STORE", "INTEGRITY".into(), message.clone()),
            // A RevisionConflict contains the storage key, not a safe explanatory message.
            StoreError::RevisionConflict(_) => (
                "STATE_STORE",
                "REVISION_CONFLICT".into(),
                "stored revision differs".into(),
            ),
            StoreError::KeyConflict => (
                "STATE_STORE",
                "KEY_CONFLICT".into(),
                "idempotency conflict".into(),
            ),
            StoreError::OutboxConflict => (
                "STATE_STORE",
                "OUTBOX_CONFLICT".into(),
                "outbox transition conflict".into(),
            ),
            StoreError::Invalid(message) => ("STATE_STORE", "INVALID".into(), message.clone()),
        },
    }
}

fn safe_message(message: &str) -> String {
    // Error strings sometimes append a serialized request. Its contents are never useful here.
    let boundary = message.find(['{', '[']);
    let prefix = boundary.map_or(message, |end| &message[..end]);
    let mut output = String::new();
    let mut token = String::new();
    let flush = |token: &mut String, output: &mut String| {
        if uuid::Uuid::parse_str(token).is_ok()
            || (token.len() >= 32 && token.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            output.push_str("<identifier omitted>");
        } else {
            output.push_str(token);
        }
        token.clear();
    };
    for character in prefix.chars() {
        if character.is_ascii_hexdigit() || character == '-' {
            token.push(character);
        } else {
            flush(&mut token, &mut output);
            output.push(if character.is_control() {
                ' '
            } else {
                character
            });
        }
    }
    flush(&mut token, &mut output);
    if boundary.is_some() {
        output.push_str("<structured payload omitted>");
    }
    output
}

fn line(host: &Name, error: &Error) -> String {
    let (category, code, message) = fields(error);
    let safe = safe_message(&message);
    let mut message: String = safe.chars().take(MAX_LINE_CHARS).collect();
    let mut truncated = safe.chars().count() > MAX_LINE_CHARS;
    loop {
        let line = serde_json::json!({
            "event":"rx.host-delivery-error.v1", "host":host,
            "category":category, "code":code, "message":message, "truncated":truncated,
        })
        .to_string();
        // The final newline written by report_error is included in the bound.
        if line.chars().count() < MAX_LINE_CHARS {
            return line;
        }
        let _ = message.pop();
        truncated = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::{Code, Status};

    #[test]
    fn suppresses_only_consecutive_identical_diagnostics_and_keeps_writer_reason() {
        let host = Name::new("host/sim").unwrap();
        let mut changes = Changes::default();
        let first = Error::Rpc(Status::failed_precondition("Host guard rejected"));
        let emitted = changes.next(&host, &first).unwrap();
        let value: serde_json::Value = serde_json::from_str(&emitted).unwrap();
        assert_eq!(value["host"], "host/sim");
        assert_eq!(value["category"], "HOST_RPC");
        assert_eq!(value["code"], "FailedPrecondition");
        assert!(changes.next(&host, &first).is_none());
        let other = Error::Writer(WriterError::Rejected(StoreError::Rejected(
            rx_domain::fault::Rejection::HostNotPrepared,
        )));
        let value: serde_json::Value =
            serde_json::from_str(&changes.next(&host, &other).unwrap()).unwrap();
        assert_eq!(value["category"], "APPLICATION_POLICY");
        assert_eq!(value["code"], "HostNotPrepared");
        assert!(changes.next(&host, &other).is_none());
        assert_eq!(changes.next(&host, &first).unwrap(), emitted);
    }

    #[test]
    fn bounds_the_complete_json_line_including_unicode_escaping_and_newline() {
        let host = Name::new(format!("host/{}", "x".repeat(123))).unwrap();
        let error = Error::Rpc(Status::internal(format!(
            "Boundary error {}",
            "\u{d55c}\u{ae00}\"\\\n".repeat(1000)
        )));
        let line = line(&host, &error);
        assert!(line.chars().count() < MAX_LINE_CHARS);
        assert!(!line.contains('\n'));
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["host"], host.as_str());
        assert_eq!(value["truncated"], true);
        assert!(
            value["message"]
                .as_str()
                .unwrap()
                .starts_with("Boundary error")
        );
    }

    #[test]
    fn excludes_rpc_metadata_details_structured_bodies_and_request_identifiers() {
        let key = "00000000-0000-4000-8000-000000000076";
        let secret = "do-not-log-this-request-body";
        let message = format!("guard rejected request {key}; payload={{\"private\":\"{secret}\"}}");
        let mut status = Status::with_details(
            Code::FailedPrecondition,
            message,
            secret.as_bytes().to_vec().into(),
        );
        status
            .metadata_mut()
            .insert("request-key", key.parse().unwrap());
        status
            .metadata_mut()
            .insert("private-body", secret.parse().unwrap());
        let line = line(&Name::new("host/sim").unwrap(), &Error::Rpc(status));
        assert!(!line.contains(key));
        assert!(!line.contains(secret));
        assert!(!line.contains("request-key"));
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert!(
            value["message"]
                .as_str()
                .unwrap()
                .contains("guard rejected")
        );
        assert!(
            value["message"]
                .as_str()
                .unwrap()
                .contains("<structured payload omitted>")
        );
        let line = super::line(
            &Name::new("host/sim").unwrap(),
            &Error::Writer(WriterError::Rejected(StoreError::RevisionConflict(
                format!("run/{key}"),
            ))),
        );
        assert!(!line.contains(key));
        assert!(!line.contains("run/"));
    }
}
