use prost::Message;
use rx_domain::canonical;
use rx_protocol::{DESCRIPTORS, base, cell, json, strict};

const CV01: &str = r#"{"body":{"predicate":{"predicate_id":"fixture.closed","settle_ms":"0","target":{"boolean":true}}},"calibration_digests":[],"cancel_rule":"example/stop","completion_rule":"example/closed","execution_timeout_ms":"5000","kind":"ENSURE_STATE","prepare_validity_ms":"1000","profile_digest":"0000000000000000000000000000000000000000000000000000000000000000","resource_set":["controller/example"],"site_config_digest":"1111111111111111111111111111111111111111111111111111111111111111","target":"example/fixture"}"#;

#[test]
fn intent_crosses_json_protobuf_domain_without_changing_frozen_digest() {
    let intent: base::Intent = json::from_slice(CV01.as_bytes()).unwrap();
    let decoded: base::Intent = strict::decode(&intent.encode_to_vec()).unwrap();
    assert_eq!(json::to_vec(&decoded).unwrap(), CV01.as_bytes());
    assert_eq!(
        json::intent_to_domain(&decoded)
            .unwrap()
            .digest()
            .unwrap()
            .to_string(),
        "e94f6df366963718e61e68018e10b1fb0d453e36143119c1e1b59f07d53ddf92"
    );
}

#[test]
fn explicit_false_zero_and_optional_zero_survive_binary_roundtrip() {
    let false_value = strict::decode::<base::TypedValue>(&[8, 0]).unwrap();
    assert_eq!(json::to_vec(&false_value).unwrap(), br#"{"boolean":false}"#);
    let context = base::CallContext {
        expected_revision: Some(0),
        ..Default::default()
    };
    let decoded: base::CallContext = strict::decode(&context.encode_to_vec()).unwrap();
    assert_eq!(decoded.expected_revision, Some(0));
    let omitted: base::CallContext = strict::decode(&[]).unwrap();
    assert_eq!(omitted.expected_revision, None);
    // A missing context is not an authenticated request; application validation is separate.
}

#[test]
fn prost_last_wins_and_unknown_field_behavior_is_blocked_before_decode() {
    assert!(base::TypedValue::decode([8, 1, 8, 0].as_slice()).is_ok());
    assert!(strict::decode::<base::TypedValue>(&[8, 1, 8, 0]).is_err());
    assert!(base::TypedValue::decode([8, 1, 0xa0, 6, 1].as_slice()).is_ok());
    assert!(strict::decode::<base::TypedValue>(&[8, 1, 0xa0, 6, 1]).is_err());
    // Boolean and real are both valid arms, but cannot appear together.
    let mut two = vec![8, 1, 25];
    two.extend_from_slice(&0_f64.to_le_bytes());
    assert!(strict::decode::<base::TypedValue>(&two).is_err());
    assert!(strict::decode::<base::TypedValue>(&[]).is_err());
}

#[test]
fn unknown_enum_bad_wire_and_varint_overflow_are_rejected() {
    for wire in [
        vec![8, 0],
        vec![8, 127],
        vec![10, 0],
        vec![0],
        vec![8, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 2],
    ] {
        assert!(strict::decode::<base::Intent>(&wire).is_err(), "{wire:?}");
    }
    assert!(strict::decode::<base::TypedValue>(&[8, 2]).is_err());
    assert!(strict::decode::<base::TypedValue>(&[8, 0x80]).is_err());
}

#[test]
fn nonfinite_numbers_rejected_in_scalar_and_packed_vector() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut scalar = vec![25];
        scalar.extend_from_slice(&value.to_le_bytes());
        assert!(strict::decode::<base::TypedValue>(&scalar).is_err());
        let mut packed = vec![10, 8];
        packed.extend_from_slice(&value.to_le_bytes());
        assert!(strict::decode::<base::RealVector>(&packed).is_err());
    }
    let mut chunks = vec![10, 8];
    chunks.extend_from_slice(&1_f64.to_le_bytes());
    chunks.extend_from_slice(&[10, 8]);
    chunks.extend_from_slice(&2_f64.to_le_bytes());
    assert_eq!(
        strict::decode::<base::RealVector>(&chunks).unwrap().values,
        vec![1.0, 2.0]
    );
}

#[test]
fn nested_duplicate_singular_is_not_merged() {
    let body = base::Body {
        value: Some(base::body::Value::Predicate(base::PredicateGoal {
            predicate_id: "x".into(),
            target: Some(base::TypedValue {
                value: Some(base::typed_value::Value::Boolean(false)),
            }),
            settle_ms: 0,
        })),
    };
    let bytes = body.encode_to_vec();
    let mut doubled = bytes.clone();
    doubled.extend_from_slice(&bytes);
    assert!(strict::decode::<base::Body>(&doubled).is_err());
}

#[test]
fn json_is_rx_projection_not_protojson() {
    let counter: base::TimePoint =
        json::from_slice(br#"{"clock_id":"fixture/boottime","ticks_ns":"18446744073709551615"}"#)
            .unwrap();
    assert_eq!(counter.ticks_ns, u64::MAX);
    assert_eq!(
        json::to_vec(&counter).unwrap(),
        br#"{"clock_id":"fixture/boottime","ticks_ns":"18446744073709551615"}"#
    );
    for value in [
        br#"{"clockId":"x","ticks_ns":"1"}"#.as_slice(),
        br#"{"clock_id":"x","ticks_ns":1}"#,
        br#"{"clock_id":"x","ticks_ns":"01"}"#,
    ] {
        assert!(json::from_slice::<base::TimePoint>(value).is_err());
    }
    assert!(json::from_slice::<base::TypedValue>(br#"{"boolean":false,"real":0}"#).is_err());
    assert!(json::from_slice::<base::TypedValue>(br#"{"boolean":false,"boolean":true}"#).is_err());
    assert!(json::from_slice::<base::TypedValue>(br#"{"boolean":null}"#).is_err());
}

#[test]
fn fixed_digest_hex_and_oneof_contracts_are_checked() {
    let bad = CV01.replace(
        &"0".repeat(64),
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    );
    assert!(json::from_slice::<base::Intent>(bad.as_bytes()).is_err());
    let bad = CV01.replace(
        r#""resource_set":["controller/example"]"#,
        r#""resource_set":["controller/example","controller/example"]"#,
    );
    assert!(json::from_slice::<base::Intent>(bad.as_bytes()).is_err());
    assert!(json::intent_to_domain(&base::Intent::default()).is_err());
}

#[test]
fn core_enum_numbers_and_cell_union_tags_are_stable() {
    assert_eq!(base::Kind::EnsureState as i32, 2);
    assert_eq!(base::Outcome::Unresolved as i32, 6);
    assert_eq!(base::ReceiptStage::NotDispatched as i32, 6);
    assert_eq!(base::HostReceiptState::VoidedBeforeSend as i32, 6);
    assert_eq!(cell::CellReason::InvalidInput as i32, 24);
    assert_eq!(cell::CaseState::ReadyForRestart as i32, 5);
    let record = DESCRIPTORS
        .get_message_by_name("rx.cell.v1.CellJournalRecord")
        .unwrap();
    assert_eq!(record.get_field_by_name("cursor").unwrap().number(), 1);
    assert_eq!(record.get_field_by_name("base_event").unwrap().number(), 2);
    assert_eq!(record.get_field_by_name("cell_event").unwrap().number(), 3);
    assert!(
        record
            .get_field_by_name("cursor")
            .unwrap()
            .containing_oneof()
            .is_none()
    );
    assert!(
        record
            .get_field_by_name("base_event")
            .unwrap()
            .containing_oneof()
            .is_some()
    );
}

#[test]
fn numbered_fields_match_frozen_documents_independently_of_generator() {
    let field_pattern =
        regex::Regex::new(r"([A-Za-z_][A-Za-z_0-9]*):([A-Za-z_][A-Za-z_.0-9]*)(\[\]|\?)?#([0-9]+)")
            .unwrap();
    let mut checked = 0;
    for (package, document) in [
        (
            "rx.contract.v1",
            include_str!("../../../spec/contracts/v1.0/03_data_and_protocol.md"),
        ),
        (
            "rx.cell.v1",
            include_str!("../../../spec/cell_operations/v1.0/04_protocol_integration_ui.md"),
        ),
    ] {
        for line in document.lines().filter(|l| l.starts_with("| ")) {
            let Some(name) = line.split(" | ").next().map(|s| s.trim_start_matches("| ")) else {
                continue;
            };
            let Some(desc) = DESCRIPTORS.get_message_by_name(&format!("{package}.{name}")) else {
                continue;
            };
            let Some(code) = line.split('`').nth(1) else {
                continue;
            };
            for capture in field_pattern.captures_iter(code) {
                let field = desc
                    .get_field_by_name(&capture[1])
                    .unwrap_or_else(|| panic!("missing {name}.{}", &capture[1]));
                assert_eq!(
                    field.number(),
                    capture[4].parse::<u32>().unwrap(),
                    "{name}.{}",
                    &capture[1]
                );
                let modifier = capture.get(3).map(|v| v.as_str());
                assert_eq!(
                    field.is_list(),
                    modifier == Some("[]"),
                    "{name}.{}",
                    &capture[1]
                );
                assert_eq!(
                    field.field_descriptor_proto().proto3_optional(),
                    modifier == Some("?"),
                    "{name}.{}",
                    &capture[1]
                );
                let expected = capture[2].trim_start_matches("base.");
                let actual = match field.kind() {
                    prost_reflect::Kind::Message(m) => m.name().to_string(),
                    prost_reflect::Kind::Enum(e) => e.name().to_string(),
                    prost_reflect::Kind::String => "string".into(),
                    prost_reflect::Kind::Bytes => "bytes".into(),
                    prost_reflect::Kind::Uint64 => "uint64".into(),
                    prost_reflect::Kind::Uint32 => "uint32".into(),
                    prost_reflect::Kind::Sint64 => "sint64".into(),
                    prost_reflect::Kind::Double => "double".into(),
                    prost_reflect::Kind::Bool => "bool".into(),
                    other => panic!("unexpected kind {other:?}"),
                };
                let expected = match expected {
                    "Id" | "Name" | "UtcTime" => "string",
                    "Digest" => "bytes",
                    other => other,
                };
                assert_eq!(actual, expected, "{name}.{}", &capture[1]);
                checked += 1;
            }
        }
    }
    assert!(
        checked >= 350,
        "source coverage unexpectedly fell to {checked}"
    );
    eprintln!("independently matched {checked} numbered source fields");
}

#[test]
fn document_manifest_hashes_remain_the_negotiation_identity() {
    use sha2::{Digest, Sha256};
    let base: serde_json::Value = serde_json::from_str(include_str!(
        "../../../spec/contracts/v1.0/protocol_manifest.json"
    ))
    .unwrap();
    let cell: serde_json::Value = serde_json::from_str(include_str!(
        "../../../spec/cell_operations/v1.0/protocol_manifest.json"
    ))
    .unwrap();
    let base_hash = format!("{:x}", Sha256::digest(canonical::bytes(&base).unwrap()));
    assert_eq!(
        base_hash,
        "3499dff92023509d4daaded3bb9d89a822c948a9b2c025e954262c9bef9d5afe"
    );
    assert_eq!(
        cell["required_base_manifest_sha256"].as_str().unwrap(),
        base_hash
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(canonical::bytes(&cell).unwrap())),
        "0c167639b3d048f6717819f9cc9b92bba8c3a11a4c41c7068d3a04c44e7be0ea"
    );
}
