//! Loopback-only SIMULATION fixture; reuse the real signed publication/qualification path.
#![allow(dead_code, unused_imports)]
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../rx-application/tests/transactions.rs"
));
#[path = "support/execution_v2_network.rs"]
mod network;
#[tokio::main]
async fn main() {
    network::run().await;
}
