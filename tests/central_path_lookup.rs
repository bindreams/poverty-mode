//! An empty `PATH` entry means "current directory" to POSIX. The advisory locator must refuse it:
//! resolving a bare name against the CWD would let whatever directory the user happens to be in
//! decide which `central` gets reported.
//!
//! This lives in its own integration binary because it sets the process CWD, which is global state.
//! One test per binary keeps that safe.

use std::path::Path;

#[test]
fn empty_path_entry_does_not_resolve_against_the_cwd() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("central"), "x").unwrap();
    std::env::set_current_dir(dir.path()).unwrap();

    // A PATH of a single empty entry: POSIX would search ".", which now holds `central`.
    let path_var = std::ffi::OsString::from("");
    assert_eq!(
        poverty_mode::central::locate_executable_in(Path::new("central"), Some(&path_var)),
        None,
        "an empty PATH entry must not resolve against the current directory"
    );
}
