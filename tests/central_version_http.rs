//! Integration test for central::resolve_version_from against an in-process hyper server (R4 live
//! latest resolution). Reuses the shared raw-byte fixtures (R3).

mod common;

use common::fixtures::serve_bytes;

#[tokio::test(flavor = "multi_thread")]
async fn resolve_version_reads_latest_version_txt_when_unpinned() {
    let port = serve_bytes(b"0.4.2\n".to_vec()).await;
    let base = format!("http://127.0.0.1:{port}");

    let v = tokio::task::spawn_blocking(move || {
        poverty_mode::central::resolve_version_from(None, &base)
    })
    .await
    .unwrap();

    assert_eq!(v, "0.4.2");
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_version_prefers_explicit_pin_over_network() {
    // Server would return 0.4.2 but the explicit pin wins and the network is not consulted.
    let port = serve_bytes(b"0.4.2\n".to_vec()).await;
    let base = format!("http://127.0.0.1:{port}");

    let v = tokio::task::spawn_blocking(move || {
        poverty_mode::central::resolve_version_from(Some("7.7.7"), &base)
    })
    .await
    .unwrap();

    assert_eq!(v, "7.7.7");
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_version_falls_back_to_default_on_unreachable_base() {
    // A base that refuses connections must fall back to the compiled DEFAULT, not error.
    // Port 1 on loopback reliably refuses (no listener) — the GET fails and we fall back.
    let base = "http://127.0.0.1:1".to_string();
    let v = tokio::task::spawn_blocking(move || {
        poverty_mode::central::resolve_version_from(None, &base)
    })
    .await
    .unwrap();

    assert_eq!(v, poverty_mode::central::DEFAULT_JBCENTRAL_VERSION);
}
