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
        builder
            .append_data(&mut header, file_rel, contents)
            .unwrap();
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
        let opts =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
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
