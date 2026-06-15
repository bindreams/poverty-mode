use super::*;

const BASE: &str = "https://jetbrains-central-cli.s3.eu-west-1.amazonaws.com";

#[test]
fn linux_x86_64_is_tar_gz() {
    let got = jbcentral_asset_url("0.2.9", "linux", "x86_64").unwrap();
    assert_eq!(
        got,
        format!("{BASE}/jbcentral/0.2.9/jbcentral_0.2.9_linux_x86_64.tar.gz")
    );
}

#[test]
fn linux_arm64_is_tar_gz() {
    let got = jbcentral_asset_url("0.2.9", "linux", "arm64").unwrap();
    assert_eq!(
        got,
        format!("{BASE}/jbcentral/0.2.9/jbcentral_0.2.9_linux_arm64.tar.gz")
    );
}

#[test]
fn darwin_x86_64_is_tar_gz() {
    let got = jbcentral_asset_url("0.2.9", "darwin", "x86_64").unwrap();
    assert_eq!(
        got,
        format!("{BASE}/jbcentral/0.2.9/jbcentral_0.2.9_darwin_x86_64.tar.gz")
    );
}

#[test]
fn darwin_arm64_is_tar_gz() {
    let got = jbcentral_asset_url("0.2.9", "darwin", "arm64").unwrap();
    assert_eq!(
        got,
        format!("{BASE}/jbcentral/0.2.9/jbcentral_0.2.9_darwin_arm64.tar.gz")
    );
}

#[test]
fn windows_x86_64_is_zip() {
    let got = jbcentral_asset_url("0.2.9", "windows", "x86_64").unwrap();
    assert_eq!(
        got,
        format!("{BASE}/jbcentral/0.2.9/jbcentral_0.2.9_windows_x86_64.zip")
    );
}

#[test]
fn windows_arm64_is_an_error() {
    let err = jbcentral_asset_url("0.2.9", "windows", "arm64").unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("windows") && msg.contains("arm64"),
        "error should name the unsupported target: {msg}"
    );
}

#[test]
fn unknown_os_is_an_error() {
    let err = jbcentral_asset_url("0.2.9", "plan9", "x86_64").unwrap_err();
    assert!(err.to_string().contains("plan9"), "{err}");
}

#[test]
fn unknown_arch_is_an_error() {
    let err = jbcentral_asset_url("0.2.9", "linux", "riscv64").unwrap_err();
    assert!(err.to_string().contains("riscv64"), "{err}");
}

// extract_archive =====

use std::fs;
use std::io::Write as _;

/// Build a small `.tar.gz` in memory containing one file `file_rel` with known contents.
fn make_tar_gz_fixture(file_rel: &str, contents: &[u8]) -> Vec<u8> {
    use flate2::write::GzEncoder;
    use flate2::Compression;

    let mut tar_bytes: Vec<u8> = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append_data(&mut header, file_rel, contents).unwrap();
        builder.finish().unwrap();
    }
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&tar_bytes).unwrap();
    gz.finish().unwrap()
}

/// Build a small `.zip` in memory containing one file `file_rel` with known contents.
fn make_zip_fixture(file_rel: &str, contents: &[u8]) -> Vec<u8> {
    use std::io::Cursor;
    use zip::write::SimpleFileOptions;

    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        writer.start_file(file_rel, opts).unwrap();
        writer.write_all(contents).unwrap();
        writer.finish().unwrap();
    }
    cursor.into_inner()
}

#[test]
fn extract_archive_tar_gz_writes_files() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("out");
    let bytes = make_tar_gz_fixture("bin/jbcentral", b"#!/bin/sh\necho hi\n");

    extract_archive(&bytes, "thing.tar.gz", &dest).unwrap();

    let extracted = dest.join("bin").join("jbcentral");
    let got = fs::read(&extracted).unwrap();
    assert_eq!(got, b"#!/bin/sh\necho hi\n");
}

#[test]
fn extract_archive_zip_writes_files() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("out");
    let bytes = make_zip_fixture("bin/jbcentral.exe", b"MZ-fake-exe");

    extract_archive(&bytes, "thing.zip", &dest).unwrap();

    let extracted = dest.join("bin").join("jbcentral.exe");
    let got = fs::read(&extracted).unwrap();
    assert_eq!(got, b"MZ-fake-exe");
}

#[test]
fn extract_archive_unknown_suffix_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("out");
    let err = extract_archive(b"junk", "thing.rar", &dest).unwrap_err();
    assert!(err.to_string().contains("thing.rar"), "{err}");
}

// pin lookup + verify + replace =====

use sha2::{Digest, Sha256};

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    let mut s = String::with_capacity(digest.len() * 2);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[test]
fn pinned_sha256_returns_some_for_a_known_entry() {
    // The real 0.2.9 entries are populated in Task M8.12; this asserts the lookup MECHANISM using the
    // host target. While the table is empty (M8.3 → M8.12) the inner `if let Some` is skipped — this
    // is a labelled invariant guard (R12), not a red→green for behavior added later. After M8.12 it
    // asserts the real host pin is 64-char lowercase hex. The default version is referenced as a
    // string literal here (NOT `central::DEFAULT_JBCENTRAL_VERSION`) because `src/central.rs` does not
    // exist yet at this point in the build order; M8.5 creates the module and introduces the constant.
    if let (Ok(os), Ok(arch)) = (host_os(), host_arch()) {
        if let Some(sum) = pinned_sha256("0.2.9", os, arch) {
            assert_eq!(sum.len(), 64, "sha256 hex must be 64 chars: {sum}");
            assert!(
                sum.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
                "pin must be lowercase hex: {sum}"
            );
        }
    }
}

#[test]
fn pinned_sha256_none_for_unknown_version() {
    assert!(pinned_sha256("0.0.0-does-not-exist", "linux", "x86_64").is_none());
}

#[test]
fn verify_and_extract_accepts_matching_sha256() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("dest");
    let bytes = make_tar_gz_fixture("bin/jbcentral", b"payload-A");
    let sum = sha256_hex(&bytes);

    verify_and_extract_bytes(&bytes, "asset.tar.gz", Some(&sum), &dest).unwrap();

    let got = std::fs::read(dest.join("bin").join("jbcentral")).unwrap();
    assert_eq!(got, b"payload-A");
}

#[test]
fn verify_and_extract_rejects_wrong_sha256_and_leaves_no_dest() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("dest");
    let bytes = make_tar_gz_fixture("bin/jbcentral", b"payload-B");
    let wrong = "0".repeat(64);

    let err = verify_and_extract_bytes(&bytes, "asset.tar.gz", Some(&wrong), &dest).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("checksum") || msg.contains("sha256"), "{msg}");
    assert!(!dest.exists(), "dest must not exist after a checksum failure");
}

#[test]
fn verify_and_extract_seam_allows_none_checksum() {
    // The low-level SEAM treats `None` as "no checksum supplied" and extracts. Fail-closed (R14) is
    // enforced one level up by `ensure_installed`, which always passes a `Some(pin)` for a known
    // target and errors when no pin exists (asserted in Task M8.8/M8.12). This test only proves the
    // seam's None branch works for the test path.
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("dest");
    let bytes = make_tar_gz_fixture("bin/jbcentral", b"payload-N");
    verify_and_extract_bytes(&bytes, "asset.tar.gz", None, &dest).unwrap();
    let got = std::fs::read(dest.join("bin").join("jbcentral")).unwrap();
    assert_eq!(got, b"payload-N");
}

#[test]
fn verify_and_extract_is_case_insensitive_on_hex() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("dest");
    let bytes = make_zip_fixture("bin/jbcentral.exe", b"payload-C");
    let sum = sha256_hex(&bytes).to_uppercase();

    verify_and_extract_bytes(&bytes, "asset.zip", Some(&sum), &dest).unwrap();

    let got = std::fs::read(dest.join("bin").join("jbcentral.exe")).unwrap();
    assert_eq!(got, b"payload-C");
}

#[test]
fn verify_and_extract_replaces_existing_dest_without_stale_leftovers() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("dest");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(dest.join("STALE"), b"old").unwrap();

    let bytes = make_tar_gz_fixture("bin/jbcentral", b"payload-E");
    verify_and_extract_bytes(&bytes, "asset.tar.gz", None, &dest).unwrap();

    assert!(!dest.join("STALE").exists(), "stale file must be gone after replace");
    let got = std::fs::read(dest.join("bin").join("jbcentral")).unwrap();
    assert_eq!(got, b"payload-E");
}

// pin coverage (R14) =====

#[test]
fn pin_table_covers_all_supported_targets_for_default_version() {
    let v = crate::central::DEFAULT_JBCENTRAL_VERSION;
    // Every supported (os, arch) EXCEPT windows-arm64 (no asset) must have a pin for the default ver.
    let supported = [
        ("darwin", "x86_64"),
        ("darwin", "arm64"),
        ("linux", "x86_64"),
        ("linux", "arm64"),
        ("windows", "x86_64"),
    ];
    for (os, arch) in supported {
        let sum = pinned_sha256(v, os, arch).unwrap_or_else(|| panic!("missing sha256 pin for {v} {os}/{arch}"));
        assert_eq!(sum.len(), 64, "pin for {os}/{arch} must be 64 hex chars: {sum}");
        assert!(
            sum.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "pin for {os}/{arch} must be lowercase hex: {sum}"
        );
    }
    // windows-arm64 has no asset, hence no pin.
    assert!(pinned_sha256(v, "windows", "arm64").is_none());
}
