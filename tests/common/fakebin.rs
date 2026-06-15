//! Shared cross-platform fake-`jbcentral` builder for integration tests (R3 — single copy,
//! reused via `mod common;`). The unit-test helper in `src/central_tests.rs` always exits with
//! a single code regardless of args; the External-mode status path needs a binary whose
//! `--version` and `status` behave differently, so this helper takes both.
//!
//! `allow(dead_code)`: each integration-test crate includes the whole `common` module but uses
//! only a subset; the unused-in-this-crate items must not trip `dead_code` under `-D warnings`.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// Write a fake `jbcentral` into `dir`:
/// - `<exe> --version` prints `version_line` and exits 0;
/// - any other argument (e.g. `status`) exits `other_code` with no stdout.
///
/// Cross-platform: a `.bat` on Windows (executed via its full path) and a `chmod +x` shell
/// script elsewhere. Returns the executable's path.
pub fn write_fake_jbcentral(dir: &Path, version_line: &str, other_code: i32) -> PathBuf {
    if cfg!(windows) {
        let p = dir.join("jbcentral.bat");
        // `@echo off` so the command line itself is not echoed into stdout. We must NOT wrap the
        // `echo` in a parenthesized `if (...)` block: a `)` inside `version_line` (e.g. "(fake)")
        // would close the block early and truncate the output. A single-line `if not ... exit`
        // guard avoids any block, so the echoed line is emitted verbatim.
        std::fs::write(
            &p,
            format!(
                "@echo off\r\n\
                 if not \"%1\"==\"--version\" exit /b {other_code}\r\n\
                 echo {version_line}\r\n\
                 exit /b 0\r\n"
            ),
        )
        .unwrap();
        p
    } else {
        let p = dir.join("jbcentral");
        std::fs::write(
            &p,
            format!(
                "#!/bin/sh\n\
                 if [ \"$1\" = \"--version\" ]; then echo '{version_line}'; exit 0; fi\n\
                 exit {other_code}\n"
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }
}
