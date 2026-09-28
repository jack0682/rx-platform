//! Container-local development API for authored Python simulation skills.
//! Contains no Python execution, device transport, or physical commissioning path.
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::Html,
    routing::{get, post},
};
use rx_application::software_skill::{Engine, Finish, Package, Start};
use rx_domain::types::Id;
use rx_runtime::{
    software_skill::{Application, Command},
    writer::{Writer, WriterError},
};
use rx_storage::SqliteRepository;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

type W = Writer<Application<SqliteRepository>>;
#[derive(Clone)]
struct App {
    writer: W,
    client: [u8; 32],
    worker: [u8; 32],
}
type Error = (StatusCode, Json<Value>);
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_millis()
        .try_into()
        .expect("clock range")
}
fn error(code: StatusCode, detail: impl ToString) -> Error {
    (code, Json(json!({"error":detail.to_string()})))
}
fn auth(headers: &HeaderMap, expected: &[u8; 32]) -> Result<(), Error> {
    let value = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or("");
    let digest: [u8; 32] = Sha256::digest(value.as_bytes()).into();
    if digest
        .iter()
        .zip(expected)
        .fold(0u8, |a, (x, y)| a | (x ^ y))
        != 0
    {
        return Err(error(StatusCode::UNAUTHORIZED, "authentication required"));
    }
    Ok(())
}
async fn request(app: &App, c: Command) -> Result<Json<Value>, Error> {
    let result = app
        .writer
        .enqueue(c)
        .map_err(writer_error)?
        .wait()
        .await
        .map_err(writer_error)?;
    Ok(Json(result))
}
fn writer_error(e: WriterError<rx_ports::StoreError>) -> Error {
    match e {
        WriterError::Rejected(rx_ports::StoreError::KeyConflict) => error(
            StatusCode::CONFLICT,
            "immutable version or request key conflict",
        ),
        WriterError::Rejected(rx_ports::StoreError::Invalid(s)) => {
            error(StatusCode::BAD_REQUEST, s)
        }
        _ => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "state service unavailable; query the original request ID",
        ),
    }
}
async fn health() -> Json<Value> {
    Json(json!({"profile":"LOCAL_SIM","version":"0.3.0-rc.1","physical_control":false}))
}
async fn skills(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, Error> {
    auth(&h, &a.client)?;
    request(&a, Command::Skills).await
}
async fn register(
    State(a): State<App>,
    h: HeaderMap,
    Json(p): Json<Package>,
) -> Result<Json<Value>, Error> {
    auth(&h, &a.client)?;
    request(&a, Command::Register(p)).await
}
async fn runs(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, Error> {
    auth(&h, &a.client)?;
    request(&a, Command::Runs).await
}
async fn submit(
    State(a): State<App>,
    h: HeaderMap,
    Json(r): Json<Start>,
) -> Result<Json<Value>, Error> {
    auth(&h, &a.client)?;
    request(&a, Command::Submit(r, now())).await
}
async fn run(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, Error> {
    auth(&h, &a.client)?;
    request(
        &a,
        Command::Get(Id::new(id).map_err(|_| error(StatusCode::BAD_REQUEST, "invalid run ID"))?),
    )
    .await
}
async fn claim(
    State(a): State<App>,
    h: HeaderMap,
    Json(id): Json<Id>,
) -> Result<Json<Value>, Error> {
    auth(&h, &a.worker)?;
    request(&a, Command::Claim(id, now())).await
}
async fn finish(
    State(a): State<App>,
    h: HeaderMap,
    Json(f): Json<Finish>,
) -> Result<Json<Value>, Error> {
    auth(&h, &a.worker)?;
    request(&a, Command::Finish(f, now())).await
}
async fn abandon(
    State(a): State<App>,
    h: HeaderMap,
    Json(id): Json<Id>,
) -> Result<Json<Value>, Error> {
    auth(&h, &a.worker)?;
    request(&a, Command::Abandon(id, now())).await
}
async fn index() -> Html<&'static str> {
    Html(include_str!("../software-skills.html"))
}
fn token(path: PathBuf) -> Result<[u8; 32], Box<dyn std::error::Error>> {
    let raw = std::fs::read_to_string(path)?;
    let raw = raw.trim();
    if raw.len() != 64 || !raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("64-character random token required".into());
    }
    Ok(Sha256::digest(raw.as_bytes()).into())
}
fn prepare_store(data: &std::path::Path) -> Result<bool, Box<dyn std::error::Error>> {
    use std::io::Write;
    let marker = data.join("installation.profile");
    let database = data.join("skills.db");
    if marker.exists() {
        if !marker.symlink_metadata()?.is_file()
            || std::fs::read(&marker)? != b"rx.local-sim.installation.v1\n"
        {
            return Err("invalid local simulation installation descriptor".into());
        }
        if !database
            .symlink_metadata()
            .is_ok_and(|m| m.is_file() && m.len() >= 100)
        {
            return Err(
                "existing installation database missing; automatic recreation refused".into(),
            );
        }
        return Ok(false);
    } else {
        if data.read_dir()?.next().is_some() {
            return Err("nonempty data directory has no installation descriptor".into());
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(marker)?;
        file.write_all(b"rx.local-sim.installation.v1\n")?;
        file.sync_all()?;
        std::fs::File::open(data)?.sync_all()?;
    }
    Ok(true)
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err(
            "usage: rx-skill-server DATA_DIRECTORY CLIENT_TOKEN_FILE WORKER_TOKEN_FILE".into(),
        );
    }
    let data = PathBuf::from(&args[0]);
    std::fs::create_dir_all(&data)?;
    let client = token(PathBuf::from(&args[1]))?;
    let worker = token(PathBuf::from(&args[2]))?;
    if client == worker {
        return Err("separate client and worker identities required".into());
    }
    let new_installation = prepare_store(&data)?;
    let writer = W::start(move || {
        let repository = SqliteRepository::open(data.join("skills.db"))?;
        Ok(Application(if new_installation {
            Engine::open(repository, now())?
        } else {
            Engine::open_existing(repository, now())?
        }))
    })
    .await
    .map_err(|e| format!("writer start: {e}"))?;
    let app = App {
        writer: writer.clone(),
        client,
        worker,
    };
    let router = Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .route("/v1/skills", get(skills).post(register))
        .route("/v1/runs", get(runs).post(submit))
        .route("/v1/runs/{id}", get(run))
        .route("/internal/claim", post(claim))
        .route("/internal/finish", post(finish))
        .route("/internal/abandon", post(abandon))
        .layer(DefaultBodyLimit::max(524_288))
        .with_state(app);
    // The installer publishes this container port on host loopback only.
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            #[cfg(unix)]
            {
                let mut term =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                        .expect("SIGTERM");
                tokio::select! {_ = tokio::signal::ctrl_c()=>{},_ = term.recv()=>{}}
            }
            #[cfg(not(unix))]
            {
                let _ = tokio::signal::ctrl_c().await;
            }
        })
        .await?;
    writer.close();
    writer.closed().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lost_database_and_descriptor_are_not_fresh_installations() {
        let dir = tempfile::tempdir().unwrap();
        prepare_store(dir.path()).unwrap();
        assert!(prepare_store(dir.path()).is_err());
        let path = dir.path().join("skills.db");
        let repository = SqliteRepository::open(&path).unwrap();
        repository.close().unwrap();
        prepare_store(dir.path()).unwrap();
        std::fs::rename(&path, dir.path().join("preserved.db")).unwrap();
        assert!(prepare_store(dir.path()).is_err());
        assert!(!path.exists());
        let unrelated = tempfile::tempdir().unwrap();
        std::fs::write(unrelated.path().join("keep"), "existing content").unwrap();
        assert!(prepare_store(unrelated.path()).is_err());
    }
}
