//! Password work is bounded and runs outside the application writer.
use crate::error::ApiError;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use rand_core::{OsRng, RngCore};
use rx_application::Identity;
use rx_domain::types::{Digest, Name};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

pub const SESSION_SECONDS: u64 = 3600;
const MAX_SESSIONS: usize = 1024;
pub const COOKIE: &str = "rx_session";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalAccount {
    pub principal: Name,
    pub password_hash: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    pub schema: String,
    pub accounts: Vec<LocalAccount>,
}

pub fn password_hash(password: &str) -> Result<String, String> {
    if password.len() < 12 || password.len() > 1024 {
        return Err("password must contain 12–1024 UTF-8 bytes".into());
    }
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| "password hashing failed".into())
}

#[derive(Clone)]
struct BrowserSession {
    identity: Identity,
    expires: Instant,
}
struct Mutable {
    sessions: BTreeMap<[u8; 32], BrowserSession>,
    attempts: VecDeque<Instant>,
}
pub struct Auth {
    credentials: Arc<BTreeMap<String, String>>,
    dummy: String,
    workers: Arc<Semaphore>,
    mutable: Mutex<Mutable>,
}
impl Auth {
    pub fn new(credentials: Credentials) -> Result<Self, String> {
        if credentials.schema != "rx.local-credentials.v1"
            || credentials.accounts.is_empty()
            || credentials.accounts.len() > 1024
        {
            return Err("invalid credential catalog".into());
        }
        let mut entries = BTreeMap::new();
        for entry in credentials.accounts {
            let parsed =
                PasswordHash::new(&entry.password_hash).map_err(|_| "invalid password hash")?;
            // Exactly the approved local profile. Prevent unbounded work from imported PHC settings.
            if parsed.algorithm.as_str() != "argon2id"
                || parsed.version != Some(19)
                || parsed.params.get_decimal("m") != Some(19_456)
                || parsed.params.get_decimal("t") != Some(2)
                || parsed.params.get_decimal("p") != Some(1)
                || parsed.params.iter().count() != 3
                || parsed.salt.is_none()
                || parsed.hash.as_ref().is_none_or(|h| h.len() != 32)
            {
                return Err("unsupported password hash profile".into());
            }
            if entries
                .insert(entry.principal.as_str().into(), entry.password_hash)
                .is_some()
            {
                return Err("duplicate credential principal".into());
            }
        }
        let dummy = password_hash(&token())?;
        Ok(Self {
            credentials: Arc::new(entries),
            dummy,
            workers: Arc::new(Semaphore::new(2)),
            mutable: Mutex::new(Mutable {
                sessions: BTreeMap::new(),
                attempts: VecDeque::new(),
            }),
        })
    }
    pub async fn verify(&self, principal: String, password: String) -> Result<Name, ApiError> {
        let password = Zeroizing::new(password);
        if principal.len() > 128 || password.len() > 1024 {
            return Err(ApiError::invalid());
        }
        {
            let now = Instant::now();
            let mut m = self.mutable.lock().map_err(|_| ApiError::unavailable())?;
            while m
                .attempts
                .front()
                .is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(60))
            {
                m.attempts.pop_front();
            }
            // Installation-wide admission bound, without persistent account lockout.
            if m.attempts.len() >= 30 {
                return Err(ApiError::busy());
            }
            m.attempts.push_back(now);
        }
        let permit = self
            .workers
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiError::busy())?;
        let known = self.credentials.get(&principal);
        let exists = known.is_some();
        let hash = known.unwrap_or(&self.dummy).clone();
        let accepted = tokio::task::spawn_blocking(move || {
            let _permit = permit; // Cancellation cannot release the bound while hashing is running.
            PasswordHash::new(&hash).ok().is_some_and(|parsed| {
                Argon2::default()
                    .verify_password(password.as_bytes(), &parsed)
                    .is_ok()
            }) && exists
        })
        .await
        .map_err(|_| ApiError::unavailable())?;
        if !accepted {
            return Err(ApiError::unauthenticated());
        }
        Name::new(principal).map_err(|_| ApiError::unauthenticated())
    }
    pub fn insert(&self, identity: Identity) -> Result<String, ApiError> {
        let mut m = self.mutable.lock().map_err(|_| ApiError::unavailable())?;
        let now = Instant::now();
        m.sessions.retain(|_, s| s.expires > now);
        if m.sessions.len() >= MAX_SESSIONS {
            return Err(ApiError::busy());
        }
        let token = token();
        let digest = Sha256::digest(token.as_bytes()).into();
        if m.sessions.contains_key(&digest) {
            return Err(ApiError::unavailable());
        }
        m.sessions.insert(
            digest,
            BrowserSession {
                identity,
                expires: now + Duration::from_secs(SESSION_SECONDS),
            },
        );
        Ok(token)
    }
    pub fn resolve(&self, token: &str) -> Result<Identity, ApiError> {
        let mut m = self.mutable.lock().map_err(|_| ApiError::unavailable())?;
        let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let s = m
            .sessions
            .get(&digest)
            .ok_or_else(ApiError::unauthenticated)?;
        if s.expires <= Instant::now() {
            m.sessions.remove(&digest);
            return Err(ApiError::unauthenticated());
        }
        Ok(s.identity.clone())
    }
    pub fn remove(&self, token: &str) -> Result<(), ApiError> {
        let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        self.mutable
            .lock()
            .map_err(|_| ApiError::unavailable())?
            .sessions
            .remove(&digest);
        Ok(())
    }
}
fn token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    Digest::from_bytes(bytes).to_string()
}
