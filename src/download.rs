//! On-demand download manager: per-OS/arch URL templating, mandatory sha256 verification against a
//! per-version pin (R14), archive extraction, and a lock-serialized replace into the bin cache. Used
//! only for `jbcentral` in v1.

use anyhow::{anyhow, bail};

/// JetBrains' public S3 bucket that hosts the `jbcentral` CLI assets.
pub const JBCENTRAL_S3_BASE: &str = "https://jetbrains-central-cli.s3.eu-west-1.amazonaws.com";

/// Build the download URL for a pinned `jbcentral` asset.
///
/// `os` is one of `darwin | linux | windows` (from `std::env::consts::OS` mapped to JetBrains'
/// naming) and `arch` is one of `x86_64 | arm64`. The extension is `zip` on Windows and `tar.gz`
/// everywhere else. There is **no** `windows-arm64` asset; that target returns a clear error.
pub fn jbcentral_asset_url(version: &str, os: &str, arch: &str) -> anyhow::Result<String> {
    let os = match os {
        "darwin" | "linux" | "windows" => os,
        other => bail!("unsupported jbcentral OS \"{other}\" (expected darwin|linux|windows)"),
    };
    let arch = match arch {
        "x86_64" | "arm64" => arch,
        other => bail!("unsupported jbcentral arch \"{other}\" (expected x86_64|arm64)"),
    };
    if os == "windows" && arch == "arm64" {
        bail!("jbcentral has no windows-arm64 asset; JB Central is unsupported on this target");
    }
    let ext = if os == "windows" { "zip" } else { "tar.gz" };
    Ok(format!(
        "{JBCENTRAL_S3_BASE}/jbcentral/{version}/jbcentral_{version}_{os}_{arch}.{ext}"
    ))
}

/// Map `std::env::consts::OS` to JetBrains' OS token (`macos` -> `darwin`).
pub fn host_os() -> anyhow::Result<&'static str> {
    match std::env::consts::OS {
        "macos" => Ok("darwin"),
        "linux" => Ok("linux"),
        "windows" => Ok("windows"),
        other => Err(anyhow!("unsupported host OS \"{other}\" for jbcentral")),
    }
}

/// Map `std::env::consts::ARCH` to JetBrains' arch token (`aarch64` -> `arm64`).
pub fn host_arch() -> anyhow::Result<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("x86_64"),
        "aarch64" => Ok("arm64"),
        other => Err(anyhow!("unsupported host arch \"{other}\" for jbcentral")),
    }
}

#[cfg(test)]
#[path = "download_tests.rs"]
mod download_tests;
