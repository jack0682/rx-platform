use prost::Message;
use rx_protocol::{base, cell, json, strict};
use std::{fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let folder = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("fixture directory required")?,
    );
    fs::create_dir_all(&folder)?;
    let source = include_str!("../../../spec/contracts/v1.0/canonical_vectors.md");
    let cv01 = source
        .split("\x60\x60\x60json\n")
        .nth(1)
        .ok_or("CV01 missing")?
        .split("\n\x60\x60\x60")
        .next()
        .ok_or("CV01 end missing")?;
    if std::env::args().any(|s| s == "--verify-return") {
        let result = strict::decode::<base::Intent>(&fs::read(folder.join("cv01.return.pb"))?)?;
        if json::intent_to_domain(&result)?.digest()?.to_string()
            != "e94f6df366963718e61e68018e10b1fb0d453e36143119c1e1b59f07d53ddf92"
        {
            return Err("C++ roundtrip changed Intent identity".into());
        }
        let count =
            strict::decode::<base::TimePoint>(&fs::read(folder.join("counter.return.pb"))?)?;
        if count.ticks_ns != u64::MAX {
            return Err("C++ roundtrip lost uint64 precision".into());
        }
        let optional =
            strict::decode::<base::CallContext>(&fs::read(folder.join("optional.return.pb"))?)?;
        if optional.expected_revision != Some(0) {
            return Err("C++ lost optional zero presence".into());
        }
        let request = strict::decode::<cell::StartRunRequest>(&fs::read(
            folder.join("cell_start.return.pb"),
        )?)?;
        if request.budget_limit != 10
            || request.call.as_ref().map(|c| c.cell_id.as_str()) != Some("cell/simulation")
        {
            return Err("C++ cell request changed".into());
        }
        println!(
            "Rust -> C++ -> Rust: frozen Intent digest, uint64, optional presence and cell request preserved"
        );
        return Ok(());
    }
    let mut cases: Vec<(&str, &str, bool, Vec<u8>)> = vec![
        (
            "cv01",
            "rx.contract.v1.Intent",
            true,
            json::from_slice::<base::Intent>(cv01.as_bytes())?.encode_to_vec(),
        ),
        ("false", "rx.contract.v1.TypedValue", true, vec![8, 0]),
        (
            "duplicate",
            "rx.contract.v1.TypedValue",
            false,
            vec![8, 1, 8, 0],
        ),
        (
            "unknown",
            "rx.contract.v1.TypedValue",
            false,
            vec![8, 1, 0xa0, 6, 1],
        ),
        ("missing_oneof", "rx.contract.v1.TypedValue", false, vec![]),
        (
            "bad_boolean",
            "rx.contract.v1.TypedValue",
            false,
            vec![8, 2],
        ),
        ("unknown_enum", "rx.contract.v1.Intent", false, vec![8, 127]),
        (
            "unspecified_enum",
            "rx.contract.v1.Intent",
            false,
            vec![8, 0],
        ),
        ("wrong_wire", "rx.contract.v1.Intent", false, vec![10, 0]),
        (
            "truncated",
            "rx.contract.v1.TypedValue",
            false,
            vec![8, 0x80],
        ),
        (
            "overflow",
            "rx.contract.v1.TypedValue",
            false,
            vec![8, 255, 255, 255, 255, 255, 255, 255, 255, 255, 2],
        ),
    ];
    let mut nan = vec![25];
    nan.extend_from_slice(&f64::NAN.to_le_bytes());
    cases.push(("nan", "rx.contract.v1.TypedValue", false, nan));
    let mut two = vec![8, 1, 25];
    two.extend_from_slice(&0_f64.to_le_bytes());
    cases.push(("two_oneofs", "rx.contract.v1.TypedValue", false, two));
    let mut packed = vec![10, 8];
    packed.extend_from_slice(&f64::INFINITY.to_le_bytes());
    cases.push((
        "packed_infinity",
        "rx.contract.v1.RealVector",
        false,
        packed,
    ));
    cases.push((
        "counter",
        "rx.contract.v1.TimePoint",
        true,
        base::TimePoint {
            clock_id: "test/boottime".into(),
            ticks_ns: u64::MAX,
        }
        .encode_to_vec(),
    ));
    cases.push((
        "optional",
        "rx.contract.v1.CallContext",
        true,
        base::CallContext {
            expected_revision: Some(0),
            ..Default::default()
        }
        .encode_to_vec(),
    ));
    let start = cell::StartRunRequest {
        call: Some(cell::CellCall {
            cell_id: "cell/simulation".into(),
            expected_cell_revision: Some(8),
            context: Some(base::CallContext {
                session_id: "11111111-1111-4111-8111-111111111111".into(),
                call_id: "22222222-2222-4222-8222-222222222222".into(),
                request_key: Some("33333333-3333-4333-8333-333333333333".into()),
                expected_revision: None,
            }),
        }),
        run_id: "44444444-4444-4444-8444-444444444444".into(),
        envelope_digest: vec![5; 32],
        purpose: cell::Purpose::Production as i32,
        budget_unit: cell::BudgetUnit::PartAttempt as i32,
        budget_limit: 10,
        origin: cell::StartOrigin::Operator as i32,
    };
    cases.push((
        "cell_start",
        "rx.cell.v1.StartRunRequest",
        true,
        start.encode_to_vec(),
    ));
    let mut index = String::new();
    for (name, type_, valid, bytes) in &cases {
        fs::write(folder.join(format!("{name}.pb")), bytes)?;
        index.push_str(&format!(
            "{name}\t{type_}\t{}\n",
            if *valid { "valid" } else { "invalid" }
        ));
    }
    fs::write(folder.join("cases.tsv"), index)?;
    println!("Exported {} positive/negative wire fixtures", cases.len());
    Ok(())
}
