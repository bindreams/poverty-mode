//! Integration test for download::download_verify_extract against an in-process hyper server.
//! Reuses the shared raw-byte fixtures from `tests/common/fixtures.rs` (R3).

mod common;

use common::fixtures::{make_tar_gz_fixture, serve_bytes};

#[tokio::test(flavor = "multi_thread")]
async fn download_verify_extract_fetches_and_unpacks() {
    let archive = make_tar_gz_fixture("bin/jbcentral", b"hello-from-stub");
    let port = serve_bytes(archive).await;
    let url = format!("http://127.0.0.1:{port}/jbcentral/1.0.0/jbcentral_1.0.0_linux_x86_64.tar.gz");

    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("jbcentral").join("1.0.0");

    // R5: drive the blocking download off the async runtime via spawn_blocking.
    let dest2 = dest.clone();
    tokio::task::spawn_blocking(move || {
        poverty_mode::download::download_verify_extract(&url, &dest2).unwrap();
    })
    .await
    .unwrap();

    let got = std::fs::read(dest.join("bin").join("jbcentral")).unwrap();
    assert_eq!(got, b"hello-from-stub");
}
