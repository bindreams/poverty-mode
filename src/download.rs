//! On-demand download manager: per-OS/arch URL templating, mandatory sha256 verification against a
//! per-version pin (R14), archive extraction, and a lock-serialized replace into the bin cache. Used
//! only for `jbcentral` in v1.

use std::fs;
use std::io::Cursor;
use std::path::Path;

use anyhow::{anyhow, bail, Context};

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

/// Extract an in-memory archive into `dest_dir`, dispatching on `name`'s suffix:
/// `*.tar.gz` / `*.tgz` -> gzip+tar, `*.zip` -> zip. Any other suffix is an error.
/// `dest_dir` is created if absent. These are our own pinned, sha256-verified JetBrains assets; both
/// `tar` and `zip` sanitize path traversal internally.
pub fn extract_archive(bytes: &[u8], name: &str, dest_dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dest_dir)
        .with_context(|| format!("creating extract dir {}", dest_dir.display()))?;

    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        let gz = flate2::read::GzDecoder::new(Cursor::new(bytes));
        let mut archive = tar::Archive::new(gz);
        archive
            .unpack(dest_dir)
            .with_context(|| format!("unpacking tar.gz into {}", dest_dir.display()))?;
        Ok(())
    } else if lower.ends_with(".zip") {
        let mut archive =
            zip::ZipArchive::new(Cursor::new(bytes)).context("opening zip archive")?;
        archive
            .extract(dest_dir)
            .with_context(|| format!("extracting zip into {}", dest_dir.display()))?;
        Ok(())
    } else {
        bail!("unsupported archive type for \"{name}\" (expected .tar.gz/.tgz or .zip)")
    }
}

#[cfg(test)]
#[path = "download_tests.rs"]
mod download_tests;
