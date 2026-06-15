//! Live JB Central suite — REQUIRES network + an interactive JetBrains AI Pro login.
//! Excluded from the default test run via `#[ignore]`. Run deliberately with:
//!     cargo test --test central_live -- --ignored
//! Documented in tests/EMPIRICAL_GATES.md (R7). No skip-on-missing: when included, these must pass.

use poverty_mode::central;

#[test]
#[ignore = "live: downloads jbcentral over the network"]
fn ensure_installed_downloads_resolved_version() {
    let version = central::resolve_version(None);
    let bin = central::ensure_installed(&version).expect("install jbcentral");
    assert!(bin.is_file(), "binary should exist at {}", bin.display());
    // Idempotent second call returns the SAME resolved path without re-downloading (flat or nested).
    let bin2 = central::ensure_installed(&version).expect("second ensure_installed");
    assert_eq!(bin, bin2);
}

#[test]
#[ignore = "live: requires a pre-existing JetBrains login + daemon start"]
fn login_start_health_stop_round_trip() {
    let version = central::resolve_version(None);
    let bin = central::ensure_installed(&version).expect("install jbcentral");

    // Login is assumed (the run path no longer logs in). `start` no longer takes a
    // version — it never runs `config set`.
    let info = central::start(&bin, None).expect("start central daemon");
    assert!(info.port > 0, "expected a bound proxy port");
    assert!(!info.secret.is_empty(), "expected a proxy secret");

    assert!(central::health(info.port), "daemon should be healthy after start");

    // The wire upstream is well-formed and points at the bound port.
    let up = central::central_wire_upstream(&info).expect("wire upstream");
    assert_eq!(up.host_header(), format!("127.0.0.1:{}", info.port));

    central::stop(&bin).expect("stop central daemon");
}
