use rx_protocol::base;
use rx_protocol_adapter::{evidence_wire, native_evidence};

fn source() -> base::Evidence {
    base::Evidence {
        evidence_id: "8a993072-e9ec-40b2-83e4-3eb6ee97cdcc".into(),
        evidence_schema: "rx.native-result.v1".into(),
        profile_digest: vec![1; 32],
        body: Some(base::EvidenceBody {
            value: Some(base::evidence_body::Value::NativeResult(
                base::NativeResult {
                    correlation: Some(base::Correlation {
                        operation_id: Some("cbd8c5b5-714c-4f18-9916-156d46bf83b2".into()),
                        invocation_id: Some("2226e47a-865b-4b2d-92bd-90c61c00c303".into()),
                        native_id: Some("controller/result #77".into()),
                        device_session_id: "0e7a60de-0509-444b-bc48-b19b27da1438".into(),
                        profile_digest: vec![1; 32],
                        cancel_id: None,
                    }),
                    native_status_schema: "vendor/status.v1".into(),
                    native_status: -1,
                    native_data: Some(base::ArtifactRef {
                        sha256: vec![3; 32],
                        schema_id: "vendor/data.v1".into(),
                        size_bytes: 9007199254740993,
                    }),
                    captured_at: Some(base::TimePoint {
                        clock_id: "boot/test".into(),
                        ticks_ns: 9007199254740995,
                    }),
                },
            )),
        }),
    }
}
#[test]
fn native_result_round_trip_keeps_vendor_id_payload_and_full_width_values() {
    let wire = source();
    let domain = native_evidence(wire.clone()).unwrap();
    assert_eq!(
        domain.native_details.as_ref().unwrap().native_id.as_deref(),
        Some("controller/result #77")
    );
    assert_eq!(
        domain
            .native_details
            .as_ref()
            .unwrap()
            .native_data
            .as_ref()
            .unwrap()
            .size_bytes
            .0,
        9007199254740993
    );
    assert_eq!(evidence_wire(domain).unwrap(), wire);
}
#[test]
fn projection_only_evidence_cannot_be_claimed_as_a_complete_source() {
    let mut evidence = native_evidence(source()).unwrap();
    evidence.native_details = None;
    assert!(evidence_wire(evidence).is_err());
}
