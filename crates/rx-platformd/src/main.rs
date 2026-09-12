use std::path::Path;
#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [command, path] if command == "init" => {
            rx_platformd::initialize(Path::new(path)).map(|()| 0)
        }
        [command, path] if command == "run" => run(Path::new(path)).await,
        _ => Err("usage: rx-platformd init|run ABSOLUTE_CONFIG_JSON".into()),
    };
    match result {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("rx-platformd: {error}");
            std::process::exit(1);
        }
    }
}
#[cfg(target_os = "linux")]
async fn run(path: &Path) -> rx_platformd::Result<i32> {
    let clock = rx_platformd::LinuxBoottime::new()?;
    let report = rx_platformd::serve(path, clock, async {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut sigterm) => tokio::select! {_=sigterm.recv()=>{},_=tokio::signal::ctrl_c()=>{}},
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    })
    .await?;
    Ok(if report.attention_count.0 == 0 { 0 } else { 2 })
}
#[cfg(not(target_os = "linux"))]
async fn run(_: &Path) -> rx_platformd::Result<i32> {
    Err("platform runtime requires the Linux shared boottime clock; use the development API for local UI work".into())
}
