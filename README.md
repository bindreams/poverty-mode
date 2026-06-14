# poverty-mode

Run an AI coding agent (v1: `claude`) behind a user-chosen, ordered chain of
local HTTP proxies — pino + headroom compiled into the binary, JB Central
downloaded on demand — wiring each proxy's inbound port to the next hop's
outbound upstream.

## Install

Prebuilt binaries for win-x64, mac-x64, mac-arm64, linux-x64, and linux-arm64 are
attached to each [GitHub release](https://github.com/bindreams/poverty-mode/releases).
Download the archive for your platform, verify the `.sha256` sidecar
(`shasum -a 256 -c <file>.sha256`), extract, and put `poverty-mode` on your `PATH`.

Or build from source with Cargo (pure Rust; no system OpenSSL needed):

```sh
cargo install --git https://github.com/bindreams/poverty-mode
# or, from a checkout:
cargo install --path .
```

This installs a single self-contained `poverty-mode` binary. The first-party `pino`
and `headroom` proxies are compiled in; `jbcentral` is downloaded on demand only when
you enable the `central` proxy.

## Testing

CI runs the default `cargo test` on five target triples. Some tests are `#[ignore]`-d
because they need a human-provisioned dependency (a JetBrains AI Pro subscription, an
installed `claude`, interactive login). CI never runs them. To run them locally see
[tests/EMPIRICAL_GATES.md](tests/EMPIRICAL_GATES.md).

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.
