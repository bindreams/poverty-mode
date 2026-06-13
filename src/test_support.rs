//! Test-only helpers shared across `paths_tests` and `config_tests`.
//!
//! Declared in `src/lib.rs` as
//! `#[cfg(test)] #[path = "test_support.rs"] pub(crate) mod test_support;`
//! so both sibling test modules reach it as `crate::test_support` without
//! reaching into each other's private `*_tests` submodules (R13).

use std::ffi::OsString;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

/// Process-global guard serializing env-mutating tests under the default
/// multi-threaded test runner. A real lock (not a timer): the suite is correct
/// regardless of thread scheduling.
pub(crate) static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Holds `ENV_LOCK` and sets `XDG_CONFIG_HOME` to a chosen value (or removes it)
/// for the guard's lifetime, restoring the prior value on drop.
pub(crate) struct XdgConfigGuard {
    prev: Option<OsString>,
    _lock: MutexGuard<'static, ()>,
}

impl XdgConfigGuard {
    /// Set `XDG_CONFIG_HOME` to `value` (or remove it when `None`).
    pub(crate) fn set(value: Option<&Path>) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var_os("XDG_CONFIG_HOME");
        match value {
            Some(p) => std::env::set_var("XDG_CONFIG_HOME", p),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        XdgConfigGuard { prev, _lock: lock }
    }
}

impl Drop for XdgConfigGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }
}

/// Holds `ENV_LOCK` and sets one or more env vars to chosen values (or removes
/// them) for the guard's lifetime, restoring each prior value on drop. Used by
/// `paths_tests` to exercise the `POVERTY_STATE_DIR`/`POVERTY_CACHE_DIR` overrides
/// (R23j) under the same serialization as the XDG guard. `ENV_LOCK` is a plain
/// (non-reentrant) `Mutex`, so a single guard must own ALL the vars a test sets at
/// once — use `set_pair` rather than two `set` calls (which would deadlock).
pub(crate) struct EnvVarGuard {
    saved: Vec<(&'static str, Option<OsString>)>,
    _lock: MutexGuard<'static, ()>,
}

impl EnvVarGuard {
    /// Set `key` to `value` (or remove it when `None`).
    pub(crate) fn set(key: &'static str, value: Option<&Path>) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var_os(key);
        apply_env(key, value);
        EnvVarGuard {
            saved: vec![(key, prev)],
            _lock: lock,
        }
    }

    /// Set two env vars under a SINGLE `ENV_LOCK` acquisition (avoids the deadlock
    /// of nesting two `set` guards).
    pub(crate) fn set_pair(
        a: (&'static str, Option<&Path>),
        b: (&'static str, Option<&Path>),
    ) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev_a = std::env::var_os(a.0);
        let prev_b = std::env::var_os(b.0);
        apply_env(a.0, a.1);
        apply_env(b.0, b.1);
        EnvVarGuard {
            saved: vec![(a.0, prev_a), (b.0, prev_b)],
            _lock: lock,
        }
    }
}

/// Set `key` to `value` (or remove it when `None`).
fn apply_env(key: &str, value: Option<&Path>) {
    match value {
        Some(p) => std::env::set_var(key, p),
        None => std::env::remove_var(key),
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        // Restore in reverse order so paired sets unwind cleanly.
        for (key, prev) in self.saved.iter().rev() {
            match prev {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
}

/// Holds `ENV_LOCK` and points `XDG_CONFIG_HOME` at a fresh `TempDir` for the
/// guard's lifetime, so config reads/writes are fully isolated per test. Restores
/// the prior value on drop; the temp dir is removed when the guard drops.
pub(crate) struct ConfigHomeGuard {
    prev: Option<OsString>,
    _lock: MutexGuard<'static, ()>,
    dir: tempfile::TempDir,
}

// `ConfigHomeGuard` lets `config_tests` isolate the config path without reaching
// into a sibling test module (R13). `config_tests::load_or_create_*` are its callers.
impl ConfigHomeGuard {
    pub(crate) fn new() -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = tempfile::TempDir::new().unwrap();
        let prev = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", dir.path());
        ConfigHomeGuard {
            prev,
            _lock: lock,
            dir,
        }
    }

    /// The `poverty-mode.yaml` path inside the isolated config home.
    pub(crate) fn config_file(&self) -> std::path::PathBuf {
        self.dir.path().join("poverty-mode.yaml")
    }
}

impl Drop for ConfigHomeGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }
}
