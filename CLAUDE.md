# CLAUDE.md

Guidance for AI agents working on this repo. **Read [CONTRIBUTING.md](CONTRIBUTING.md) first** (architecture, layout, conventions) and [README.md](README.md) (usage). This file only adds what's easy to get wrong.

## Before claiming done

Run the full gate (CONTRIBUTING.md → "the gate"), all four green:

```sh
cargo build && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --all --check
```

`#[cfg(unix)]` tests not running on a Windows host is expected, not a failure.

## Easy to get wrong

- **lib+bin:** declare modules with `pub mod x;` in `src/lib.rs`, never `mod x` in `main.rs`. Subcommand dispatch lives in `src/cli.rs::dispatch`, not `main.rs`.
- **Sibling unit tests** need the path attribute: `#[cfg(test)] #[path = "foo_tests.rs"] mod foo_tests;` — a bare `mod foo_tests;` makes rustc look in `foo/`.
- **One engine, two transforms.** pino and headroom share `src/proxy.rs`; per-proxy behavior belongs in `proxy/pino.rs` / `proxy/headroom.rs`, not the engine.
- **Byte-fidelity.** Don't re-serialize a body the transform didn't change — forward the original bytes. Re-canonicalizing the cache-hot zone defeats the prompt cache the proxy exists to protect.
- **`Agent` is generic** (`argv[0]` is the program to spawn); don't hardcode `claude`. Central is always the last hop.
- **Central is external-by-default.** `central::central_source` is the single External-vs-Download decision point: a non-blank `CentralSettings.executable` (default `jbcentral`) is used as-is; blank/unset downloads the latest `jbcentral` unpinned. poverty-mode never writes `~/.wire` config, never runs `jbcentral config set`, and never drives login — the user is assumed logged in. There is no `central` subcommand and no SHA pin.
- **No sleeps / time-based sync** — drain-on-signal + the READY-line handshake; the only numeric timeout is the human-surfaced readiness deadline. No data races.
- **Vendored `headroom-core`** is a feature-trimmed path dep; never re-add its ONNX/embedding dependencies.
- **Live tests** (`central_live`, `agent_empirical`) are `#[ignore]`; they assume a pre-existing JetBrains login and never run in CI.

## Identity

Never invent the user's name or email for a copyright line, Cargo `authors`, a LICENSE, or any attribution — read it from the "About the user" section of the user's `~/.claude/CLAUDE.md`, or ask.

Deeper detail lives in [CONTRIBUTING.md](CONTRIBUTING.md), [tests/EMPIRICAL_GATES.md](tests/EMPIRICAL_GATES.md), and the module docs in `src/proxy.rs` / `src/orchestrator.rs`.
