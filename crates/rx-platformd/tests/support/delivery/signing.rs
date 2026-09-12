use super::*;
use ed25519_dalek::{Signer, SigningKey};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SignerFixture {
    pub id: Name,
    pub private_seed_hex: Digest,
    pub public_key: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Signers {
    pub schema: Name,
    pub test_only: bool,
    pub keys: Vec<SignerFixture>,
}
pub(super) fn fixtures() -> Signers {
    Signers {
        schema: name("rx.delivery-test-signers.v1"),
        test_only: true,
        keys: [
            (PACKAGE_KEY, 47u8),
            (REVIEW_KEY, 53),
            (QUALIFICATION_KEY, 83),
        ]
        .into_iter()
        .map(|(id, byte)| {
            let seed = [byte; 32];
            let key = SigningKey::from_bytes(&seed);
            SignerFixture {
                id: name(id),
                private_seed_hex: Digest::from_bytes(seed),
                public_key: Digest::from_bytes(key.verifying_key().to_bytes()),
            }
        })
        .collect(),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    key: Name,
    message_hex: Option<String>,
    qualification_report: Option<PathBuf>,
}
pub fn sign() -> Result<()> {
    let seed_root = env_path("RX_CELL_DELIVERY_SEED")?;
    let seed = read_seed(&seed_root)?.seed;
    let signers: Signers = read(&seed_root.join("signing-fixtures.json"))?;
    if !signers.test_only || signers.schema != name("rx.delivery-test-signers.v1") {
        return Err("test signer metadata required".into());
    }
    let input: Input = read(&env_path("RX_CELL_SIGN_INPUT")?)?;
    if ![PACKAGE_KEY, REVIEW_KEY, QUALIFICATION_KEY].contains(&input.key.as_str()) {
        return Err("fixture signer not allowed".into());
    }
    let source = signers
        .keys
        .iter()
        .find(|k| k.id == input.key)
        .ok_or("fixture signer missing")?;
    let key = SigningKey::from_bytes(source.private_seed_hex.as_bytes());
    if Digest::from_bytes(key.verifying_key().to_bytes()) != source.public_key
        || seed.public_signers.get(&input.key) != Some(&source.public_key)
    {
        return Err("fixture signing key/public policy differs".into());
    }
    let (message, report_digest) = match (input.message_hex, input.qualification_report) {
        (Some(hex), None) => {
            if hex.is_empty()
                || hex.len() > 2_097_152
                || hex.len() % 2 != 0
                || !hex
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            {
                return Err("bounded lowercase message hex required".into());
            }
            let message = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            (message, None)
        }
        (None, Some(path)) if input.key.as_str() == QUALIFICATION_KEY && path.is_absolute() => {
            let report: rx_application::requalification::Report = read(&path)?;
            (report.signing_message(&input.key)?, Some(report.digest()?))
        }
        _ => {
            return Err(
                "exactly message_hex or qualification_report with qualification key required"
                    .into(),
            );
        }
    };
    let envelope = rx_package::SignatureEnvelope {
        key: input.key.clone(),
        signature: key
            .sign(&message)
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    };
    let output = env_path("RX_CELL_SIGN_OUTPUT")?;
    let signature_sha256 = write_json(&output, &envelope)?;
    write_json(
        &output.with_extension("metadata.json"),
        &serde_json::json!({
            "schema":"rx.delivery-test-signature-metadata.v1","test_only":true,"key":input.key,
            "public_key":source.public_key,"message_digest":rx_package::content_digest(&message),"report_digest":report_digest,"signature_sha256":signature_sha256
        }),
    )?;
    Ok(())
}
