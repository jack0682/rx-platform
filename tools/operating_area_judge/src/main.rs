//! Separate one-shot development judge. Never launched by the host provider.
#![forbid(unsafe_code)]
use rx_domain::{canonical, types::*};
use rx_package::{SignatureEnvelope, external_decision::*, operating_area as policy};
use rx_storage::mailbox::Mailbox;
use std::{
    path::{Path, PathBuf},
    process::Command,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn public_key(key: &Path) -> Result<[u8; 32]> {
    let meta = std::fs::symlink_metadata(key)?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err("judge/private-key-type".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err("judge/private-key-permissions".into());
        }
    }
    let output = Command::new("openssl")
        .args(["pkey", "-in"])
        .arg(key)
        .args(["-pubout", "-outform", "DER"])
        .output()?;
    if !output.status.success() || output.stdout.len() != 44 {
        return Err("judge/private-key-unavailable".into());
    }
    Ok(output.stdout[12..].try_into()?)
}
fn sign(key: &Path, key_id: &Name, bytes: &[u8]) -> Result<SignatureEnvelope> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("message");
    std::fs::write(&path, bytes)?;
    let output = Command::new("openssl")
        .args(["pkeyutl", "-sign", "-rawin", "-inkey"])
        .arg(key)
        .arg("-in")
        .arg(path)
        .output()?;
    if !output.status.success() || output.stdout.len() != 64 {
        return Err("judge/signature-failed".into());
    }
    Ok(SignatureEnvelope {
        key: key_id.clone(),
        signature: output.stdout.iter().map(|b| format!("{b:02x}")).collect(),
    })
}
fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 5 || !matches!(args[0].as_str(), "decide" | "revoke") {
        return Err("usage: rx-operating-area-judge decide|revoke MAILBOX CHALLENGE_ID PRIVATE_KEY TTL_MS_OR_REASON".into());
    }
    let id = Id::new(&args[2])?;
    let key = PathBuf::from(&args[3]);
    let public = public_key(&key)?;
    let area = policy::catalog()?
        .iter()
        .find(|a| a.public_key == public)
        .ok_or("judge/key-not-authored-development-key")?;
    let mailbox = Mailbox::open(&args[1])?;
    let guard = mailbox.lock()?;
    let request = guard
        .read(&format!("request-{id}.json"), 131_072)?
        .ok_or("judge/request-absent")?;
    let info: ChallengeInfo = canonical::decode_json(&request)?;
    if info.id != id {
        return Err("judge/request-id".into());
    }
    let key_id = Name::new(area.key_id)?;
    if info.key != key_id || info.issuer.as_str() != area.issuer {
        return Err("judge/issuer-scope".into());
    }
    if args[0] == "decide" {
        let decision = Id::new(uuid::Uuid::new_v4().to_string())?;
        match policy::judge(&info, decision, Counter(args[4].parse()?)) {
            Ok(claim) => {
                let signature = sign(&key, &key_id, &claim.signing_message(&key_id)?)?;
                let signed = SignedDecision { claim, signature };
                guard.publish_once(&format!("decision-{id}.json"), &canonical::bytes(&signed)?)?;
                println!(
                    "{}",
                    serde_json::json!({"result":"SIGNED_DEVELOPMENT_APPROVAL","rule":area.rule,"decision":signed.claim.decision,"physical_authority":"NONE"})
                );
            }
            Err(denial) => {
                guard.publish_once(&format!("denial-{id}.json"), &canonical::bytes(&denial)?)?;
                println!(
                    "{}",
                    serde_json::json!({"result":"POLICY_DENIED","denial":denial,"authority":"DEVELOPMENT_ONLY"})
                );
            }
        }
    } else {
        let bytes = guard
            .read(&format!("reference-{id}.json"), 131_072)?
            .ok_or("judge/verified-reference-absent")?;
        let reference: Reference = canonical::decode_json(&bytes)?;
        if reference.challenge != id
            || reference.key.as_str() != area.key_id
            || reference.issuer.as_str() != area.issuer
        {
            return Err("judge/revocation-reference-scope".into());
        }
        let claim = RevocationClaim {
            schema: Name::new("rx.external-decision-revocation.v1")?,
            target: reference,
            reason: Name::new(&args[4])?,
        };
        let signature = sign(&key, &key_id, &claim.signing_message(&key_id)?)?;
        guard.publish_once(
            &format!("revocation-{id}.json"),
            &canonical::bytes(&SignedRevocation { claim, signature })?,
        )?;
        println!(
            "{}",
            serde_json::json!({"result":"SIGNED_REVOCATION_PUBLISHED","challenge":id})
        );
    }
    Ok(())
}
