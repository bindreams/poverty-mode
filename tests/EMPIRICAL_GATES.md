# Empirical verification gates (Claude agent) + live-central suite

This file documents the test gates that drive **real external dependencies** and
are therefore **not run in normal CI**, and it is the single place their observed
results are **recorded** (design spec §8 / §16 / §17; reconciliation R7 / R8).

## 1. Claude agent empirical gates

Located in `tests/agent_empirical.rs`. They drive the real, installed `claude`
binary against the canonical in-process stub (`tests/common/stub.rs`) to confirm
two design assumptions:

- **(a) process-env vs settings.json `env`-block precedence** for
  `ANTHROPIC_BASE_URL` — which belt Claude honors when they conflict.
- **(b) subagent endpoint inheritance** — that a Claude Code subagent reaches the
  same proxy endpoint as the main loop.

### Why they are gated (no skip-on-missing)

Both spawn `claude`, which must be installed on `PATH` and logged in. We never
silently skip on a missing dependency: the gates carry `#[ignore]`, so `cargo test` excludes them and a normal run is green without `claude`. They become a
**hard failure** (not a skip) the moment you opt in and the binary is missing
(the spawn `.expect(...)` panics with an actionable message).

### Running them (only when `claude` is provisioned)

```sh
cargo test --test agent_empirical -- --ignored
```

Single gate:

```sh
cargo test --test agent_empirical process_env_vs_settings_block_precedence -- --ignored
cargo test --test agent_empirical subagent_inherits_chain_endpoint -- --ignored
```

### PASS criteria

- **(a)** Exactly one belt receives a `/v1/messages` request, and it is the
  `--settings` env block (CLI-arg precedence wins over process env). The test
  prints `EMPIRICAL(a): ... settings_block_hit=true(v1=true)`. The auth token is
  carried in **both** belts (production-faithful); only the base URL differs, so
  it alone determines the winner. If neither belt is hit and claude exited
  non-zero, that is an **environment failure** (login/network/Managed), reported
  distinctly — not a precedence result.
- **(b)** The stub records `>= 2` requests (a single stub on a single loopback
  port, so every reached request shares the host by construction) — the main loop
  plus the subagent both reached our endpoint. The test prints
  `EMPIRICAL(b): ... requests=2 ...`. Zero requests + non-zero exit is reported as
  an environment failure, not a subagent result.

### Cross-platform note (spec §12, Windows)

`claude --settings '<json>'` is passed via the `std::process` argument **vector**,
not a shell, on every platform — so Rust performs OS-level argument quoting and
there is no shell-escaping difference between Unix and Windows. The JSON-level
escaping (quotes/backslashes/newlines) is handled by `serde_json` and unit-tested
(`settings_value_escapes_special_characters`). When running gate (a)/(b) on
Windows with a provisioned `claude`, confirm the same PASS output as Unix and
record it below; no separate Windows code path exists.

### Remote / cloud execution bypass (spec §8)

Claude's cloud/remote execution path (scheduled routines, `RemoteTrigger`) runs
server-side, not in this process, so it inherits neither our process env nor our
`--settings` argument and inherently **bypasses** the local proxy chain. This is
expected and documented (mirrored in `agent::claude::REMOTE_EXECUTION_NOTE`):
only locally-spawned `claude` (main loop + in-process subagents, gate (b)) is
routed through poverty-mode. No gate is needed for the remote path because it is
out of our process boundary by construction.

## 2. Recorded results

Record each run here (date, OS/arch, `claude` version, exit + the `EMPIRICAL(...)`
line). This is the authoritative record the design's §8/§17 open verifications
refer to; update it whenever a gate is run on a new platform or `claude` version.

| Date                            | OS/arch | claude version | Gate | EMPIRICAL line | PASS? |
| ------------------------------- | ------- | -------------- | ---- | -------------- | ----- |
| _pending first provisioned run_ |         |                | (a)  |                |       |
| _pending first provisioned run_ |         |                | (b)  |                |       |

### Belt load-bearing decision (fill in after gate (a) resolves) — R8 follow-up

The design assumes belt 2 (`--settings`) wins at CLI-arg precedence and belt 1
(process env) is retained as **genuine redundancy** (some `claude` code paths and
spawned child tools — MCP servers, shell tools — read `ANTHROPIC_BASE_URL` from
the process env directly). Once gate (a) is run, record the measured outcome and
the decision here:

- **Measured winner:** _pending_ (expected: `--settings` / belt 2).
- **Decision:** _pending_ — if belt 2 is confirmed authoritative, KEEP belt 1 as
  justified redundancy for process-env-reading subprocesses (state the concrete
  paths verified). If gate (a) ever shows belt 1 can win on some path, document
  which path and why both belts remain necessary. If belt 1 is provably never the
  winner AND no subprocess reads process env, reframe it as a contract/debug
  guard or remove it — and surface this to the user rather than silently keeping
  dead code.

## 3. Live JB Central suite (forward reference — R7)

The live-central integration tests (added in M8) also require an external,
human-provisioned dependency: a logged-in `jbcentral` with a JetBrains AI Pro
subscription. There is **no** non-interactive `central login --token`; CI never
logs in. Those tests are likewise `#[ignore]`d and run only when a human
provisions the dependency. Record their results in this file's section 2 table
(add `central:<name>` rows) so all human-provisioned gate outcomes live in one
place.

## 4. CI

If/when CI provisions `claude` (logged-in, with credentials), add a dedicated job
that runs `cargo test --test agent_empirical -- --ignored`. Do **not** enable it
on the default test job — the gates are excluded there by design (R7: default
`cargo test` runs only non-`#[ignore]` tests; no central login in CI).

## Central live suite

The JB Central integration's genuinely external actions (download, interactive login, daemon
start/health/stop) live in `tests/central_live.rs`, gated behind `#[ignore]` so the default
`cargo test` never blocks on a network download or a browser-OAuth flow.

**Prerequisites (human-provisioned; NOT run in CI — see R7):**

- Network access to JetBrains' public S3 (`jetbrains-central-cli.s3.eu-west-1.amazonaws.com`).
- A JetBrains **AI Pro** subscription and an interactive browser login (`jbcentral login`).

**Run the central live suite deliberately:**

```
cargo test --test central_live -- --ignored
```

This installs `jbcentral` (default version `0.2.9`), performs the interactive login,
starts the singleton daemon, asserts `/health`, and stops it. There is **no** skip-on-missing: when
run, every test must pass (fail loudly otherwise). CI runs only the default `cargo test` and therefore
never touches this suite (R7).

**Where results are recorded:** record each manual run's date, host OS/arch, resolved `jbcentral`
version, and pass/fail outcome in the table below.

| Date | Host (os/arch) | jbcentral version | Result | Notes |
| ---- | -------------- | ----------------- | ------ | ----- |
|      |                |                   |        |       |
