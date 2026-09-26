//! Expectations come only from the published rule-authored corpus. No golden output.
use prost::{Message, Name};
use prost_reflect::{DescriptorPool, DynamicMessage};
use rx_protocol::{DESCRIPTORS, base, executor, strict};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct Corpus {
    schema: String,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    name: String,
    rule: String,
    message: String,
    schema: String,
    input: Value,
    status: String,
    roundtrip_status: Option<String>,
}
fn hex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0);
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
        .collect()
}
fn varint(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    while value >= 128 {
        out.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    out.push(value as u8);
    out
}
fn delimited(tag: u64, body: Vec<u8>) -> Vec<u8> {
    let mut out = varint((tag << 3) | 2);
    out.extend(varint(body.len() as u64));
    out.extend(body);
    out
}
fn bytes(value: &Value) -> Vec<u8> {
    if let Some(v) = value.get("hex") {
        return hex(v.as_str().unwrap());
    }
    if let Some(v) = value.get("concat") {
        return v.as_array().unwrap().iter().flat_map(bytes).collect();
    }
    if let Some(v) = value.get("repeat") {
        return hex(v["hex"].as_str().unwrap()).repeat(v["count"].as_u64().unwrap() as usize);
    }
    if let Some(v) = value.get("delimited") {
        return delimited(v["tag"].as_u64().unwrap(), bytes(&v["body"]));
    }
    if let Some(v) = value.get("nest") {
        let mut out = bytes(&v["leaf"]);
        for _ in 0..v["levels"].as_u64().unwrap() {
            out = delimited(v["tag"].as_u64().unwrap(), out);
        }
        return out;
    }
    panic!("unknown fixed TCK recipe")
}
fn category<T>(result: &Result<T, tonic::Status>) -> &'static str {
    match result {
        Ok(_) => "OK",
        Err(e) if e.code() == tonic::Code::ResourceExhausted => "RESOURCE_EXHAUSTED",
        Err(e) if e.code() == tonic::Code::InvalidArgument => "INVALID_ARGUMENT",
        Err(e) => panic!("unexpected status {e}"),
    }
}
fn typed<M: Message + Default + Name>(data: &[u8]) -> Result<Vec<u8>, tonic::Status> {
    strict::decode::<M>(data).map(|value| value.encode_to_vec())
}
fn public_decode(name: &str, data: &[u8]) -> Result<Vec<u8>, tonic::Status> {
    match name {
        "rx.contract.v1.TimePoint" => typed::<base::TimePoint>(data),
        "rx.contract.v1.PeerHello" => typed::<base::PeerHello>(data),
        "rx.contract.v1.CallContext" => typed::<base::CallContext>(data),
        "rx.contract.v1.TypedValue" => typed::<base::TypedValue>(data),
        "rx.contract.v1.Version" => typed::<base::Version>(data),
        "rx.contract.v1.Body" => typed::<base::Body>(data),
        "rx.contract.v1.RealVector" => typed::<base::RealVector>(data),
        "rx.executor.v1.ReadPayload" => typed::<executor::ReadPayload>(data),
        _ => panic!("add actual public typed codec coverage for {name}"),
    }
}
#[test]
fn authored_profile_vectors_check_existing_rust_codec_and_synthetic_guards() {
    let corpus: Corpus =
        serde_json::from_str(include_str!("../../../proto/strict-wire-v1/vectors.json")).unwrap();
    assert_eq!(corpus.schema, "rx.strict-wire-conformance.v1");
    let guards = DescriptorPool::decode(
        include_bytes!(concat!(env!("OUT_DIR"), "/strict_probe_descriptor.bin")).as_slice(),
    )
    .unwrap();
    let mut mismatches = Vec::new();
    let (mut public, mut synthetic) = (0, 0);
    for case in &corpus.cases {
        let pool = if case.schema == "public" {
            public += 1;
            &*DESCRIPTORS
        } else {
            assert_eq!(case.schema, "guard");
            synthetic += 1;
            &guards
        };
        let descriptor = pool.get_message_by_name(&case.message).unwrap();
        let data = bytes(&case.input);
        let result = if case.schema == "public" {
            public_decode(&case.message, &data)
        } else {
            strict::validate(&descriptor, &data).and_then(|()| {
                DynamicMessage::decode(descriptor.clone(), data.as_slice())
                    .map(|value| value.encode_to_vec())
                    .map_err(|e| tonic::Status::invalid_argument(e.to_string()))
            })
        };
        let actual = category(&result);
        println!(
            "{} {} {} bytes={} expected={} actual={}",
            case.name,
            case.rule,
            case.schema,
            data.len(),
            case.status,
            actual
        );
        if actual != case.status {
            mismatches.push(format!(
                "{}: expected {}, got {actual}: {result:?}",
                case.name, case.status
            ));
        }
        if let Some(expected) = &case.roundtrip_status {
            assert!(result.is_ok());
            let encoded_result = strict::validate(&descriptor, result.as_ref().unwrap());
            println!(
                "{} re-encode expected={} actual={}",
                case.name,
                expected,
                category(&encoded_result)
            );
            if category(&encoded_result) != expected {
                mismatches.push(format!("{}: re-encoding category differs", case.name));
            }
        }
    }
    println!("TCK totals: public={public} synthetic={synthetic}");
    assert!(public > 60 && synthetic >= 10);
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}
