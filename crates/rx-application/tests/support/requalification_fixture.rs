//! Test-only signatures and explicit simulation evidence. Not field qualification material.
use ed25519_dalek::{Signer, SigningKey};
use rx_application::{CellConfiguration, requalification as q};
use rx_domain::{canonical, types::*};
use std::collections::BTreeMap;
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
#[allow(dead_code, reason = "used by separate Host integration fixture")]
pub fn material(label: &str, schema: &str) -> ArtifactRef {
    add(&mut BTreeMap::new(), label, schema)
}
fn add(blobs: &mut BTreeMap<Digest, Vec<u8>>, label: &str, schema: &str) -> ArtifactRef {
    let b = canonical::bytes(
        &serde_json::json!({"schema":schema,"test_fixture":label,"scope":"SIMULATION_TEST_ONLY"}),
    )
    .unwrap();
    let r = ArtifactRef {
        sha256: rx_package::content_digest(&b),
        schema_id: name(schema),
        size_bytes: Counter(b.len() as u64),
    };
    blobs.insert(r.sha256, b);
    r
}
pub fn configure(mut c: CellConfiguration) -> (CellConfiguration, BTreeMap<Digest, Vec<u8>>) {
    let mut b = BTreeMap::new();
    c.definition = add(&mut b, "definition", "rx.cell-definition.v1");
    c.envelope = add(&mut b, "envelope", "rx.operating-envelope.v1");
    c.site_config_digest = add(&mut b, "site", "rx.site-config.v1").sha256;
    for s in &mut c.steps {
        if let rx_domain::intent::Body::Program(program) = &mut s.intent.body {
            program.program = add(&mut b, "program", "rx.sim.program.v1");
            program.parameter_set = add(&mut b, "parameters", "rx.sim.parameters.v1");
        }
        s.intent.site_config_digest = c.site_config_digest;
        s.intent.profile_digest = add(&mut b, "profile", "rx.device-profile.v1").sha256;
    }
    (c, b)
}
pub struct Fixture {
    pub policy: q::Policy,
    pub blobs: BTreeMap<Digest, Vec<u8>>,
}
pub fn policy(c: &CellConfiguration, mut b: BTreeMap<Digest, Vec<u8>>) -> Fixture {
    let raw = canonical::bytes(c).unwrap();
    let configuration = ArtifactRef {
        sha256: rx_package::content_digest(&raw),
        schema_id: name("rx.cell-configuration.v1"),
        size_bytes: Counter(raw.len() as u64),
    };
    b.insert(configuration.sha256, raw);
    let process = canonical::bytes(c.process.as_ref().unwrap()).unwrap();
    assert_eq!(rx_package::content_digest(&process), c.recipe.sha256);
    b.insert(c.recipe.sha256, process);
    let dependencies = q::required_dependencies(c)
        .into_iter()
        .map(|hash| {
            let bytes = &b[&hash];
            let value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
            ArtifactRef {
                sha256: hash,
                schema_id: name(value["schema"].as_str().unwrap()),
                size_bytes: Counter(bytes.len() as u64),
            }
        })
        .collect();
    let acceptance_plan = add(&mut b, "acceptance plan", "rx.test.acceptance-plan.v1");
    let limitations = add(&mut b, "simulation only", "rx.test.limitations.v1");
    let criteria = [
        q::Area::Software,
        q::Area::Equipment,
        q::Area::CellIntegration,
        q::Area::Recovery,
        q::Area::Protection,
        q::Area::Operations,
    ]
    .into_iter()
    .enumerate()
    .map(|(i, area)| q::Criterion {
        id: name(&format!("test/{i}")),
        area,
        specification: add(
            &mut b,
            &format!("specification/{i}"),
            "rx.test.criterion.v1",
        ),
        evidence_schema: name("rx.test.qualification-result.v1"),
    })
    .collect();
    let policy = q::Policy {
        schema: name("rx.requalification-policy.v2"),
        profiles: vec![q::Profile {
            purposes: [name("PRODUCTION"), name("SETUP")].into_iter().collect(),
            cell: c.id.clone(),
            configuration,
            envelope: c.envelope.clone(),
            definition: c.definition.clone(),
            environment: c.environment,
            acceptance_plan,
            limitations,
            dependencies,
            criteria,
        }],
        keys: vec![q::Key {
            id: name("test-qualification-signer"),
            public_key: Digest::from_bytes(
                SigningKey::from_bytes(&[83; 32]).verifying_key().to_bytes(),
            ),
            validators: [Digest::from_bytes([84; 32])].into_iter().collect(),
            environments: [name("SIMULATION")].into_iter().collect(),
        }],
    };
    policy.digest().unwrap();
    Fixture { policy, blobs: b }
}
pub fn sign(r: &q::Report) -> rx_package::SignatureEnvelope {
    let key = name("test-qualification-signer");
    rx_package::SignatureEnvelope {
        signature: SigningKey::from_bytes(&[83; 32])
            .sign(&r.signing_message(&key).unwrap())
            .to_bytes()
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect(),
        key,
    }
}
impl Fixture {
    pub fn report(
        &self,
        j: &q::Job,
    ) -> (
        q::Report,
        rx_package::SignatureEnvelope,
        BTreeMap<Digest, Vec<u8>>,
    ) {
        let mut blobs = self.blobs.clone();
        let checks = j
            .request
            .cells
            .iter()
            .flat_map(|c| c.profile.criteria.iter().map(move |r| (&c.profile.cell, r)))
            .map(|(cell, r)| q::Check {
                cell: cell.clone(),
                criterion: r.id.clone(),
                verdict: q::Verdict::Pass,
                evidence: vec![add(
                    &mut blobs,
                    &format!("result/{}", r.id),
                    r.evidence_schema.as_str(),
                )],
                note: "Test protocol fixture, no physical acceptance claim".into(),
            })
            .collect();
        let report = q::Report {
            schema: name("rx.requalification-report.v1"),
            request: j.request.clone(),
            validator: Digest::from_bytes([84; 32]),
            checks,
        };
        let refs: std::collections::BTreeSet<_> =
            report.references().iter().map(|r| r.sha256).collect();
        blobs.retain(|hash, _| refs.contains(hash));
        let sig = sign(&report);
        (report, sig, blobs)
    }
    #[allow(dead_code, reason = "shared with filesystem worker integration")]
    pub fn write(&self, root: &std::path::Path, j: &q::Job) -> q::Report {
        std::fs::create_dir_all(root.join("artifacts")).unwrap();
        let (r, s, b) = self.report(j);
        std::fs::write(
            root.join("qualification.json"),
            canonical::bytes(&r).unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("qualification.sig.json"),
            canonical::bytes(&s).unwrap(),
        )
        .unwrap();
        for (h, b) in b {
            std::fs::write(root.join("artifacts").join(format!("{h}.bin")), b).unwrap();
        }
        r
    }
}
