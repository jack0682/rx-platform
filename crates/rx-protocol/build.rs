use std::{env, path::PathBuf};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).join("../../proto");
    let workspace = root.parent().ok_or("workspace root")?;
    for relative_binding in [
        "spec/executor/v1/binding.json",
        "spec/executor-plan/v1/binding.json",
        "spec/production/v1/binding.json",
        "spec/assignment/v1/binding.json",
        "spec/host-read/v1/binding.json",
        "spec/host-configuration/v1/binding.json",
        "spec/host-qualification/v1/binding.json",
    ] {
        let binding = workspace.join(relative_binding);
        let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(&binding)?)?;
        for (relative, expected) in manifest["source_sha256"]
            .as_object()
            .ok_or("binding inventory")?
        {
            use sha2::Digest as _;
            let source = workspace.join(relative);
            let actual = format!("{:x}", sha2::Sha256::digest(std::fs::read(&source)?));
            if Some(actual.as_str()) != expected.as_str() {
                return Err(format!("executor read binding source mismatch: {relative}").into());
            }
            println!("cargo:rerun-if-changed={}", source.display());
        }
        println!("cargo:rerun-if-changed={}", binding.display());
    }
    let output = PathBuf::from(env::var("OUT_DIR")?);
    let mut config = tonic_prost_build::Config::new();
    config.protoc_executable(protoc_bin_vendored::protoc_bin_path()?);
    config.enable_type_names();
    config.boxed(".rx.cell.v1.SnapshotEntity.value.base_entity");
    config.boxed(".rx.cell.v1.SnapshotEntity.value.cell_entity");
    tonic_prost_build::configure()
        .codec_path("crate::strict::StrictCodec")
        .file_descriptor_set_path(output.join("rx_descriptor.bin"))
        .compile_with_config(
            config,
            &[
                root.join("rx/contract/v1/contract.proto"),
                root.join("rx/cell/v1/cell.proto"),
                root.join("rx/executor/v1/executor.proto"),
                root.join("rx/executor/plan/v1/plan.proto"),
                root.join("rx/executor/production/v1/production.proto"),
                root.join("rx/executor/assignment/v1/assignment.proto"),
                root.join("rx/host/read/v1/read.proto"),
                root.join("rx/host/configuration/v1/configuration.proto"),
                root.join("rx/host/qualification/v1/qualification.proto"),
            ],
            std::slice::from_ref(&root),
        )?;
    // Separate TCK-only descriptors: no generated public types or RPC services.
    let mut probes = tonic_prost_build::Config::new();
    probes.protoc_executable(protoc_bin_vendored::protoc_bin_path()?);
    probes.file_descriptor_set_path(output.join("strict_probe_descriptor.bin"));
    probes.compile_protos(
        &[root.join("strict-wire-v1/probe.proto")],
        std::slice::from_ref(&root),
    )?;
    println!("cargo:rerun-if-changed={}", root.display());
    Ok(())
}
