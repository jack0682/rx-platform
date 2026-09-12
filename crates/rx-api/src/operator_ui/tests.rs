use super::*;
use serde_json::{Value, json};
use std::path::PathBuf;

struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    manifest: Value,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        fs::create_dir(root.join("assets")).unwrap();
        let mut files = vec![];
        for (path, bytes) in [
            ("assets/app.js", b"document.title = 'RX';".as_slice()),
            ("assets/style.css", b"body { color: black; }".as_slice()),
            ("index.html", b"<!doctype html><title>RX</title>".as_slice()),
        ] {
            fs::write(root.join(path), bytes).unwrap();
            files.push(
                json!({"path":path,"sha256":hash(bytes),"size_bytes":Counter(bytes.len() as u64)}),
            );
        }
        Self {
            _directory: directory,
            root,
            manifest: json!({"schema":SCHEMA,"api_schema":API_SCHEMA,"files":files}),
        }
    }
    fn publish(&self) -> Digest {
        let bytes = canonical::bytes(&self.manifest).unwrap();
        fs::write(self.root.join(MANIFEST_FILENAME), &bytes).unwrap();
        hash(&bytes)
    }
    fn load(&self) -> Result<OperatorBundle, String> {
        OperatorBundle::load(
            &self.root,
            &self.root.join(MANIFEST_FILENAME),
            self.publish(),
        )
    }
}

#[test]
fn owns_exact_verified_bytes_and_requires_a_pinned_manifest() {
    let f = Fixture::new();
    let bundle = f.load().unwrap();
    let original = bundle.assets["assets/app.js"].bytes.clone();
    fs::write(f.root.join("assets/app.js"), b"changed after validation").unwrap();
    fs::remove_file(f.root.join("index.html")).unwrap();
    assert_eq!(bundle.assets["assets/app.js"].bytes, original);
    assert_eq!(
        bundle.assets["index.html"].media_type,
        "text/html; charset=utf-8"
    );
    assert!(f.load().is_err());
    assert!(
        OperatorBundle::load(
            &f.root,
            &f.root.join(MANIFEST_FILENAME),
            Digest::from_bytes([0; 32])
        )
        .is_err()
    );
    assert!(OperatorBundle::load(&f.root, &f.root.join("other.json"), f.publish()).is_err());
}

#[test]
fn rejects_unlisted_missing_tampered_and_oversized_files() {
    let f = Fixture::new();
    fs::write(f.root.join("assets/extra.js"), b"extra").unwrap();
    assert!(f.load().is_err());
    fs::remove_file(f.root.join("assets/extra.js")).unwrap();
    fs::remove_file(f.root.join("assets/style.css")).unwrap();
    assert!(f.load().is_err());
    let f = Fixture::new();
    fs::write(f.root.join("assets/app.js"), vec![b'x'; 21]).unwrap();
    assert!(f.load().is_err());
    let f = Fixture::new();
    fs::OpenOptions::new()
        .write(true)
        .open(f.root.join("assets/app.js"))
        .unwrap()
        .set_len(MAX_FILE_BYTES + 1)
        .unwrap();
    assert!(f.load().is_err());
}

#[test]
fn strict_manifest_and_asset_namespace_reject_aliases_and_unsupported_inputs() {
    for (field, value) in [
        ("schema", json!("unknown")),
        ("api_schema", json!("future")),
        ("unknown", json!(true)),
    ] {
        let mut f = Fixture::new();
        f.manifest[field] = value;
        assert!(f.load().is_err(), "{field}");
    }
    for path in [
        "/assets/app.js",
        "assets/../app.js",
        "assets//app.js",
        "assets/%61pp.js",
        "assets\\app.js",
        "api/v1/session.json",
        "assets/source.map",
        "assets/page.html",
        "assets/{name}.js",
        "assets/con.js",
        "operator-bundle.json",
    ] {
        let mut f = Fixture::new();
        f.manifest["files"][0]["path"] = json!(path);
        assert!(f.load().is_err(), "{path}");
    }
    let mut f = Fixture::new();
    f.manifest["files"][0]["size_bytes"] = json!("0");
    assert!(
        f.load().is_err(),
        "empty assets are not produced or admitted"
    );
    let mut f = Fixture::new();
    f.manifest["files"][0]["size_bytes"] = json!(21);
    assert!(f.load().is_err(), "wire counters must remain strings");
    let mut f = Fixture::new();
    let duplicate = f.manifest["files"][0].clone();
    f.manifest["files"]
        .as_array_mut()
        .unwrap()
        .insert(0, duplicate);
    assert!(f.load().is_err());
    let mut f = Fixture::new();
    f.manifest["files"].as_array_mut().unwrap().reverse();
    assert!(f.load().is_err());
    let f = Fixture::new();
    let bytes = br#"{"schema":"rx.operator-ui-bundle.v1","schema":"rx.operator-ui-bundle.v1","api_schema":"rx.operator-api.v1","files":[]}"#;
    fs::write(f.root.join(MANIFEST_FILENAME), bytes).unwrap();
    assert!(OperatorBundle::load(&f.root, &f.root.join(MANIFEST_FILENAME), hash(bytes)).is_err());
}

#[test]
fn bounds_manifest_before_loading_asset_bytes() {
    let f = Fixture::new();
    let mut manifest: Manifest =
        canonical::decode_json(&canonical::bytes(&f.manifest).unwrap()).unwrap();
    manifest.files[0].size_bytes = Counter(MAX_FILE_BYTES + 1);
    assert!(validate_manifest(&manifest).is_err());
    manifest.files = (0..8)
        .map(|i| FileEntry {
            path: format!("assets/{i}.js"),
            sha256: Digest::from_bytes([0; 32]),
            size_bytes: Counter(MAX_FILE_BYTES),
        })
        .chain(std::iter::once(FileEntry {
            path: "index.html".into(),
            sha256: Digest::from_bytes([0; 32]),
            size_bytes: Counter(1),
        }))
        .collect();
    assert!(validate_manifest(&manifest).is_err(), "64 MiB plus index");
    manifest.files = (0..MAX_FILES)
        .map(|i| FileEntry {
            path: format!("assets/{i:04}.js"),
            sha256: Digest::from_bytes([0; 32]),
            size_bytes: Counter(1),
        })
        .chain(std::iter::once(FileEntry {
            path: "index.html".into(),
            sha256: Digest::from_bytes([0; 32]),
            size_bytes: Counter(1),
        }))
        .collect();
    assert!(
        validate_manifest(&manifest).is_err(),
        "1024 assets plus index"
    );
    manifest.files.pop();
    assert!(validate_manifest(&manifest).is_err(), "index required");
}

#[cfg(unix)]
#[test]
fn rejects_symlink_roots_ancestors_assets_and_manifest() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let pin = f.publish();
    let alias_directory = tempfile::tempdir().unwrap();
    let aliases = fs::canonicalize(alias_directory.path()).unwrap();
    let root_alias = aliases.join("root");
    symlink(&f.root, &root_alias).unwrap();
    assert!(OperatorBundle::load(&root_alias, &root_alias.join(MANIFEST_FILENAME), pin).is_err());
    let child = root_alias.join("assets");
    assert!(real_directory(&child).is_err());
    fs::remove_file(f.root.join("assets/app.js")).unwrap();
    symlink(f.root.join("index.html"), f.root.join("assets/app.js")).unwrap();
    assert!(f.load().is_err());
    let f = Fixture::new();
    let pin = f.publish();
    fs::remove_file(f.root.join(MANIFEST_FILENAME)).unwrap();
    symlink(f.root.join("index.html"), f.root.join(MANIFEST_FILENAME)).unwrap();
    assert!(OperatorBundle::load(&f.root, &f.root.join(MANIFEST_FILENAME), pin).is_err());
    let f = Fixture::new();
    let pin = f.publish();
    fs::rename(f.root.join("assets"), aliases.join("actual-assets")).unwrap();
    symlink(aliases.join("actual-assets"), f.root.join("assets")).unwrap();
    assert!(OperatorBundle::load(&f.root, &f.root.join(MANIFEST_FILENAME), pin).is_err());
}

#[cfg(unix)]
#[test]
fn acquisition_rejects_replaced_fifos_without_waiting_for_a_writer() {
    use std::{sync::mpsc, time::Duration};
    for relative in ["assets/app.js", MANIFEST_FILENAME] {
        let f = Fixture::new();
        f.publish();
        // Discovery is only a snapshot. A replacement before actual acquisition must not block.
        inventory(&f.root).unwrap();
        let file = f.root.join(relative);
        fs::remove_file(&file).unwrap();
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&file)
                .status()
                .unwrap()
                .success()
        );
        let root = f.root.clone();
        let (send, receive) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = send.send(read_file(&root, relative, MAX_FILE_BYTES));
        });
        let result = receive
            .recv_timeout(Duration::from_secs(3))
            .expect("FIFO acquisition must not wait for a writer");
        assert!(result.is_err());
        worker.join().unwrap();
    }
}

#[cfg(unix)]
#[test]
fn acquisition_rejects_component_links_replaced_after_inventory() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    f.publish();
    inventory(&f.root).unwrap();
    let other = Fixture::new();
    fs::remove_file(f.root.join("assets/app.js")).unwrap();
    symlink(
        other.root.join("assets/app.js"),
        f.root.join("assets/app.js"),
    )
    .unwrap();
    assert!(read_file(&f.root, "assets/app.js", MAX_FILE_BYTES).is_err());
    fs::remove_dir_all(f.root.join("assets")).unwrap();
    symlink(other.root.join("assets"), f.root.join("assets")).unwrap();
    assert!(read_file(&f.root, "assets/app.js", MAX_FILE_BYTES).is_err());
}
