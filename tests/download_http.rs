//! Integration test for download::download_verify_extract against an in-process hyper server.
//! Reuses the shared raw-byte fixtures from `tests/common/fixtures.rs` (R3).

mod common;

use common::fixtures::{make_tar_gz_fixture, serve_bytes, sha256_hex};

#[tokio::test(flavor = "multi_thread")]
async fn download_verify_extract_fetches_and_unpacks() {
    let archive = make_tar_gz_fixture("bin/jbcentral", b"hello-from-stub");
    let sum = sha256_hex(&archive);
    let port = serve_bytes(archive).await;
    let url =
        format!("http://127.0.0.1:{port}/jbcentral/1.0.0/jbcentral_1.0.0_linux_x86_64.tar.gz");

    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("jbcentral").join("1.0.0");

    // R5: drive the blocking download off the async runtime via spawn_blocking.
    let dest2 = dest.clone();
    tokio::task::spawn_blocking(move || {
        poverty_mode::download::download_verify_extract(&url, Some(&sum), &dest2).unwrap();
    })
    .await
    .unwrap();

    let got = std::fs::read(dest.join("bin").join("jbcentral")).unwrap();
    assert_eq!(got, b"hello-from-stub");
}

#[tokio::test(flavor = "multi_thread")]
async fn download_verify_extract_rejects_bad_checksum() {
    let archive = make_tar_gz_fixture("bin/jbcentral", b"hello-2");
    let port = serve_bytes(archive).await;
    let url = format!("http://127.0.0.1:{port}/x.tar.gz");
    let wrong = "0".repeat(64);

    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("jbcentral").join("1.0.0");

    let dest2 = dest.clone();
    let res = tokio::task::spawn_blocking(move || {
        poverty_mode::download::download_verify_extract(&url, Some(&wrong), &dest2)
    })
    .await
    .unwrap();

    assert!(res.is_err(), "bad checksum must fail");
    assert!(
        !dest.exists(),
        "dest must not be created on checksum failure"
    );
}
