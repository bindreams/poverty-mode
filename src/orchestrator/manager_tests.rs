use super::*;

// Object-safety / dyn-callability guard (characterization: added with the trait;
// proves the seam is a real dyn-dispatchable abstraction, not a concrete type).
#[test]
fn proxy_manager_is_object_safe() {
    fn _takes_dyn(_m: &mut dyn ProxyManager) {}
    // If `ProxyManager` were not object-safe this fn would not compile.
}

#[test]
fn ephemeral_manager_constructs() {
    // `CARGO_BIN_EXE_*` is only set for integration tests under `tests/`, not for
    // `--lib` unit tests; the constructor only needs a path (it does not spawn or
    // validate it here), so the running test executable is a valid stand-in.
    let exe = std::env::current_exe().expect("current_exe");
    let _m = EphemeralManager::new(exe).expect("construct EphemeralManager");
}
