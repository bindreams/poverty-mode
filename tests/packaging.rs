//! Validates packaging metadata + license files required for `cargo install`.

use std::path::Path;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn manifest() -> toml::Value {
    let text = std::fs::read_to_string(root().join("Cargo.toml")).unwrap();
    // `toml` 1.x parses a full document with `from_str`; `str::parse`/`FromStr for Value`
    // parses a single bare value and rejects a document ("unexpected content").
    toml::from_str::<toml::Value>(&text).expect("Cargo.toml must parse")
}

#[test]
fn package_has_install_metadata() {
    let m = manifest();
    let pkg = m.get("package").expect("[package]");
    assert_eq!(pkg.get("name").and_then(|v| v.as_str()), Some("poverty-mode"));
    assert!(
        pkg.get("description")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty()),
        "description required for cargo install / crates.io"
    );
    assert_eq!(
        pkg.get("license").and_then(|v| v.as_str()),
        Some("MIT OR Apache-2.0"),
        "dual MIT/Apache license expected"
    );
    assert!(
        pkg.get("repository")
            .and_then(|v| v.as_str())
            .is_some_and(|s| s.starts_with("http")),
        "repository URL required"
    );
    assert_eq!(
        pkg.get("edition").and_then(|v| v.as_str()),
        Some("2021"),
        "contract pins edition 2021"
    );
}

#[test]
fn binary_target_is_named_poverty_mode() {
    let m = manifest();
    let named = m
        .get("bin")
        .and_then(|b| b.as_array())
        .map(|arr| {
            arr.iter()
                .any(|e| e.get("name").and_then(|v| v.as_str()) == Some("poverty-mode"))
        })
        .unwrap_or(false);
    let default_bin = m.get("package").and_then(|p| p.get("name")).and_then(|v| v.as_str()) == Some("poverty-mode")
        && root().join("src/main.rs").exists();
    assert!(
        named || default_bin,
        "must produce a `poverty-mode` binary for cargo install"
    );
}

#[test]
fn both_license_files_exist_in_repo() {
    // The manifest declares "MIT OR Apache-2.0"; the dual-license convention and the
    // release packaging step both require these files to be present.
    assert!(
        root().join("LICENSE-MIT").exists(),
        "LICENSE-MIT must exist (declared license is MIT OR Apache-2.0)"
    );
    assert!(
        root().join("LICENSE-APACHE").exists(),
        "LICENSE-APACHE must exist (declared license is MIT OR Apache-2.0)"
    );
}

#[test]
fn readme_exists_and_has_install_section() {
    let readme = std::fs::read_to_string(root().join("README.md")).expect("README.md must exist");
    assert!(readme.contains("## Install"), "README must have an Install section");
}
