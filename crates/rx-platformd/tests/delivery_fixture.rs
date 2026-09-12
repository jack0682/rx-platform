//! Offline inputs for an actual two-image SIMULATION delivery test. No Runtime is started here.
#[path = "support/delivery/mod.rs"]
mod delivery;

#[test]
#[ignore = "explicit phase77 seed export; requires a fresh output directory"]
fn export_delivery_seed() -> delivery::Result<()> {
    delivery::seed::export()
}

#[test]
#[ignore = "explicit phase77 final export; requires actual S product compiler/package outputs"]
fn export_delivery_final() -> delivery::Result<()> {
    delivery::finalize::export()
}

#[test]
#[ignore = "explicit external test-only signer; never a product signing service"]
fn sign_delivery() -> delivery::Result<()> {
    delivery::signing::sign()
}
