//! Build-time generation of fake `jbcentral` executables for the tests that run a real
//! `<bin> ...` and classify its behavior (`src/central_tests.rs`, `tests/diagnostics.rs`).
//!
//! Those tests need a real executable; writing one inside the test and exec'ing it races a
//! concurrent in-process `fork()` (any other test that spawns) which inherits the fresh file's
//! write fd, intermittently failing the exec with `ETXTBSY` ("Text file busy") under parallel
//! load. Generating the scripts here — at build time, before any test thread runs — means no
//! thread ever holds them open for write during a fork, so the write-then-exec window cannot
//! occur. Paths reach the tests via `cargo:rustc-env`, which is injected for every target of this
//! package (unit tests AND integration tests) as a compile-time `env!`.

use std::path::{Path, PathBuf};

/// Write `body` to `dir/name`, mark it executable on unix, and return the path.
fn write_exe(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    p
}

/// A fake `jbcentral` that ignores its arguments, prints `stdout`, and exits with `code` — for
/// `run_status_classified`, which runs `<bin> status`. `.bat` on Windows, `#!/bin/sh` elsewhere.
fn write_ignoring_args(dir: &Path, stem: &str, stdout: &str, code: i32) -> PathBuf {
    if cfg!(windows) {
        // `@echo off` so the command line itself is not echoed into stdout.
        let body = format!("@echo off\r\necho {stdout}\r\nexit /b {code}\r\n");
        write_exe(dir, &format!("{stem}.bat"), &body)
    } else {
        write_exe(dir, stem, &format!("#!/bin/sh\necho '{stdout}'\nexit {code}\n"))
    }
}

/// A fake `jbcentral` whose `--version` prints `version_line` (exit 0) and whose other args exit
/// `other_code` — what External-mode `status` reporting probes. The `.bat` uses a single-line
/// `if not ... exit` (no parenthesized block) so a `)` inside `version_line` (e.g. "(fake)")
/// cannot close the block early and truncate the echoed line.
fn write_versioned(dir: &Path, stem: &str, version_line: &str, other_code: i32) -> PathBuf {
    if cfg!(windows) {
        let body = format!(
            "@echo off\r\n\
             if not \"%1\"==\"--version\" exit /b {other_code}\r\n\
             echo {version_line}\r\n\
             exit /b 0\r\n"
        );
        write_exe(dir, &format!("{stem}.bat"), &body)
    } else {
        let body = format!(
            "#!/bin/sh\n\
             if [ \"$1\" = \"--version\" ]; then echo '{version_line}'; exit 0; fi\n\
             exit {other_code}\n"
        );
        write_exe(dir, stem, &body)
    }
}

fn main() {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR set by cargo"));

    // `run_status_classified` wiring tests (src/central_tests.rs).
    let logged_in = write_ignoring_args(&out, "fake-jbcentral-logged-in", "Logged in as user@example.com", 0);
    let logged_out = write_ignoring_args(
        &out,
        "fake-jbcentral-logged-out",
        "not logged in; run jbcentral login",
        1,
    );
    // External-mode `status` reporting test (tests/diagnostics.rs).
    let version = write_versioned(&out, "fake-jbcentral-version", "jbcentral 9.9.9 (fake)", 1);

    println!("cargo:rustc-env=PM_FAKE_JBCENTRAL_LOGGED_IN={}", logged_in.display());
    println!("cargo:rustc-env=PM_FAKE_JBCENTRAL_LOGGED_OUT={}", logged_out.display());
    println!("cargo:rustc-env=PM_FAKE_JBCENTRAL_VERSION={}", version.display());
    println!("cargo:rerun-if-changed=build.rs");
}
