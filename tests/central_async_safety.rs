//! R5 guard: M8's blocking surface must be safe to call via spawn_blocking from a tokio runtime.
//! A regression that performed blocking I/O directly on the async executor would panic here.

use poverty_mode::central;

#[tokio::test(flavor = "multi_thread")]
async fn blocking_surface_is_spawn_blocking_safe() {
    // health: blocking GET against a dead port -> false, no panic on the executor.
    let healthy = tokio::task::spawn_blocking(|| central::health(1)).await.unwrap();
    assert!(!healthy, "nothing is listening on port 1");

    // locate_executable: blocking filesystem walk of PATH.
    let located = tokio::task::spawn_blocking(|| {
        central::locate_executable(std::path::Path::new("poverty-mode-no-such-central-xyz"))
    })
    .await
    .unwrap();
    assert_eq!(located, None);

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
}
