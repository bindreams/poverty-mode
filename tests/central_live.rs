//! Live JB Central suite — REQUIRES an installed `central` on PATH + an interactive JetBrains login.
//! Excluded from the default test run via `#[ignore]`. Run deliberately with:
//!     cargo test --test central_live -- --ignored
//! Documented in tests/EMPIRICAL_GATES.md (R7). No skip-on-missing: when included, these must pass.

use poverty_mode::central;

#[test]
#[ignore = "live: requires an installed central, a pre-existing JetBrains login + daemon start"]
fn login_start_health_stop_round_trip() {
    // central is external-only: resolve the configured/default name the way the run does.
    let bin = central::central_executable(None);

    // Login is assumed (the run path no longer logs in). `start` never runs `config set`.
    let info = central::start(&bin, None).expect("start central daemon");
    assert!(info.port > 0, "expected a bound proxy port");
    assert!(!info.secret.is_empty(), "expected a proxy secret");

    assert!(central::health(info.port), "daemon should be healthy after start");

    // The wire upstream is well-formed and points at the bound port.
    let up = central::central_wire_upstream(&info).expect("wire upstream");
    assert_eq!(up.host_header(), format!("127.0.0.1:{}", info.port));

    // Assert the OUTCOME: `stop` reports rather than erroring, so `.expect()` alone could never
    // catch a failed or unspawnable stop.
    assert_eq!(
        central::stop(&bin).expect("spawn central proxy stop"),
        central::StopOutcome::Stopped
    );
}
