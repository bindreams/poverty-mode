# Contributing to poverty-mode

`poverty-mode` runs an AI coding agent (v1: `claude`) behind a user-chosen chain of local HTTP proxies. For install and usage, see [README.md](README.md). This file is for working *on* the code — human or AI.

## Build, test, and the gate

Requires a Rust toolchain (developed on stable 1.93). The full gate every change must pass — and CI enforces — is:

```sh
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all --check
```

`cargo test` runs ~500 tests. Tests marked `#[ignore]` are live/empirical gates that need provisioned dependencies (a logged-in `jbcentral`, an installed `claude`) — they are excluded by default and **not** run in CI. To run them with deps in place: `cargo test -- --ignored`. See [tests/EMPIRICAL_GATES.md](tests/EMPIRICAL_GATES.md) for the protocol and recorded results.

## Architecture

One multiplexed lib+bin. The binary (`src/main.rs`) is a thin shim over `lib::run`; everything lives in modules declared in `src/lib.rs`. The CLI (`src/cli.rs`) dispatches subcommands: `run` (orchestrate a chain + agent), `proxy` (run one first-party proxy in the foreground — the debugging entry point), `central`, `config`, `status`, `doctor`, `clean`.

```
src/cli.rs            clap definitions + dispatch (where each subcommand is wired)
src/orchestrator.rs   resolve the chain, build it back-to-front, run the agent, tear down
  orchestrator/manager.rs    ProxyManager seam + EphemeralManager (spawns first-party hops)
  orchestrator/teardown.rs   cross-platform child group that survives parent death (unsafe FFI)
src/proxy.rs          the shared async reverse-proxy engine (forward, /__pm/health, drain)
  proxy/pino.rs       transform: prompt-cache breakpoint injection, drop-tools, etc.
  proxy/headroom.rs   transform: context compression via the vendored headroom-core
src/agent.rs          Agent trait (generic over the wrapped tool); agent/claude.rs is v1
src/central.rs        JB Central (jbcentral): download, login, lifecycle; download.rs is generic
src/config.rs         the $XDG_CONFIG_HOME/poverty-mode.yaml model + chain resolution
src/paths.rs          dirs, run-ids, atomic writes, advisory file locks
src/tui.rs            interactive picker; tui/reducer.rs is the pure, headless-tested state
src/status.rs doctor.rs clean.rs   diagnostics/maintenance commands
vendor/headroom-core  vendored, feature-trimmed copy of hybloid/headroom (Apache-2.0)
```

**How a chain works.** Each proxy is `(inbound 127.0.0.1 port) + (outbound upstream URL)`. The orchestrator allocates a port per hop and wires `upstream[i] → 127.0.0.1:port[i+1]`; the agent points at the head. The two first-party proxies are the *same* engine differing only in their body transform — adding a v2 proxy means adding a transform, not a server.

## Conventions

- **TDD.** Write the failing test first; it must fail for the right reason before you implement.
- **Module/test layout.** `foo.rs` + `foo/` submodules (never `mod.rs`). Unit tests in a sibling `foo_tests.rs`, included with `#[cfg(test)] #[path = "foo_tests.rs"] mod foo_tests;`. Integration tests in `tests/`; the one shared HTTP stub is `tests/common/stub.rs`.
- **No data races, no time-based synchronization.** Readiness is a blocking READY-line read plus a health probe; shutdown drains in-flight work. The only permitted numeric timeout is a human-surfaced failure bound on an external event (e.g. `READINESS_DEADLINE`).
- **Byte-fidelity matters.** The proxy must not gratuitously re-serialize request bodies — that re-canonicalizes the cache-hot zone and defeats the prompt cache. A transform that changes nothing forwards the original bytes verbatim.
- **Cross-platform.** Targets Windows, macOS, Linux. Some teardown/permission tests are `#[cfg(unix)]` and only run on POSIX (CI's matrix covers them). No OS service installation.
- **License.** Dual MIT OR Apache-2.0. Keep `vendor/headroom-core`'s `LICENSE`/`NOTICE` (Apache-2.0 attribution) intact; do not pull its ONNX/embedding dependencies back in (they are feature-gated out).

Commit messages are single-line.
