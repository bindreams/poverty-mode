//! R5 guard: M8's blocking surface must be safe to call via spawn_blocking from a tokio runtime.
//! A regression that performed blocking I/O directly on the async executor would panic here.

use poverty_mode::{central, download};

#[tokio::test(flavor = "multi_thread")]
async fn blocking_surface_is_spawn_blocking_safe() {
    // resolve_version_from: blocking GET against a dead base -> falls back to DEFAULT (no panic).
    let v = tokio::task::spawn_blocking(|| central::resolve_version_from(None, "http://127.0.0.1:1"))
        .await
        .unwrap();
    assert_eq!(v, central::DEFAULT_JBCENTRAL_VERSION);

    // Pure parsers/classifiers are trivially safe but are exercised through spawn_blocking too, to
    // document the uniform contract the orchestrator follows for the whole module.
    let info =
        tokio::task::spawn_blocking(|| central::parse_wire_config(r#"{ "proxy_port": 4321, "proxy_secret": "s" }"#))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(info.port, 4321);

    let up = tokio::task::spawn_blocking(move || central::central_wire_upstream(&info))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(up.host_header(), "127.0.0.1:4321");

    let state = tokio::task::spawn_blocking(|| central::classify_login_status(Some(0), "Logged in", ""))
        .await
        .unwrap();
    assert_eq!(state, central::CentralLoginState::LoggedIn);

    let has_pin = tokio::task::spawn_blocking(|| {
        download::pinned_sha256(central::DEFAULT_JBCENTRAL_VERSION, "linux", "x86_64").is_some()
    })
    .await
    .unwrap();
    assert!(has_pin, "default-version linux/x86_64 pin must exist after M8.12");
}
