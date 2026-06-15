//! On-demand download manager: per-OS/arch URL templating, archive extraction, and a lock-serialized
//! replace into the bin cache. Used only for `jbcentral` in v1.

use std::fs;
use std::io::Cursor;
use std::path::Path;

use anyhow::{anyhow, bail, Context};

use crate::paths;

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
/// `dest_dir` is created if absent. Both `tar` and `zip` sanitize path traversal internally.
pub fn extract_archive(bytes: &[u8], name: &str, dest_dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dest_dir).with_context(|| format!("creating extract dir {}", dest_dir.display()))?;

    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        let gz = flate2::read::GzDecoder::new(Cursor::new(bytes));
        let mut archive = tar::Archive::new(gz);
        archive
            .unpack(dest_dir)
            .with_context(|| format!("unpacking tar.gz into {}", dest_dir.display()))?;
        Ok(())
    } else if lower.ends_with(".zip") {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).context("opening zip archive")?;
        archive
            .extract(dest_dir)
            .with_context(|| format!("extracting zip into {}", dest_dir.display()))?;
        Ok(())
    } else {
        bail!("unsupported archive type for \"{name}\" (expected .tar.gz/.tgz or .zip)")
    }
}

/// Replace `dest_dir` with a fresh extraction of `bytes`.
///
/// **Replace semantics (R20, accurate wording):** this is a *lock-serialized* replace, NOT atomic at
/// the filesystem level. Extraction happens in a sibling temp dir; an existing `dest_dir` is first
/// renamed aside (`<dest>.old-<pid>`), then the staged dir is renamed into place, then the old dir is
/// removed. The window in which `dest_dir` is momentarily absent is closed against *writers* by the
/// advisory file lock held in `download_verify_extract`; *readers* (status/clean/orchestrator
/// `is_installed`) must take the same lock for a consistent view. This is the network-free seam used
/// by both `download_verify_extract` and the unit tests.
pub fn verify_and_extract_bytes(bytes: &[u8], name: &str, dest_dir: &Path) -> anyhow::Result<()> {
    let parent = dest_dir
        .parent()
        .ok_or_else(|| anyhow!("destination {} has no parent dir", dest_dir.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("creating parent dir {}", parent.display()))?;

    // Stage extraction in a temp dir in the SAME parent so the final rename is on one filesystem.
    let staging = tempfile::Builder::new()
        .prefix(".pm-extract-")
        .tempdir_in(parent)
        .with_context(|| format!("creating staging dir in {}", parent.display()))?;
    extract_archive(bytes, name, staging.path())?;
    let staged = staging.keep(); // TempDir::keep() returns the PathBuf and disarms drop (into_path is deprecated in tempfile 3.x)

    // Rename any existing dest aside first (shrinks the absent-window vs. remove-then-rename), then
    // move the staged dir into place, then drop the old dir.
    let mut renamed_old: Option<std::path::PathBuf> = None;
    if dest_dir.exists() {
        let aside = parent.join(format!(
            ".pm-old-{}-{}",
            dest_dir.file_name().and_then(|s| s.to_str()).unwrap_or("dest"),
            std::process::id()
        ));
        if let Err(e) = fs::rename(dest_dir, &aside) {
            let _ = fs::remove_dir_all(&staged); // don't leak the staged extract on this error path
            return Err(e).with_context(|| format!("renaming old dest {} aside", dest_dir.display()));
        }
        renamed_old = Some(aside);
    }

    if let Err(e) = fs::rename(&staged, dest_dir) {
        // Best-effort rollback: restore the old dir so we never leave dest missing.
        if let Some(old) = &renamed_old {
            let _ = fs::rename(old, dest_dir);
        }
        let _ = fs::remove_dir_all(&staged);
        return Err(e).with_context(|| format!("renaming staged extract into {}", dest_dir.display()));
    }

    if let Some(old) = renamed_old {
        fs::remove_dir_all(&old).with_context(|| format!("removing old dest {}", old.display()))?;
    }
    Ok(())
}

/// Download `url` and replace `dest_dir` with the extraction.
///
/// **R5 contract:** this is a synchronous `reqwest::blocking` GET. Callers in an async context MUST
/// invoke it via `tokio::task::spawn_blocking`.
///
/// The whole operation is serialized by an advisory file lock keyed beside `dest_dir`, so concurrent
/// runs racing the first download cooperate (the loser re-extracts into a fresh staging dir, which is
/// cheap and correct). The blocking client uses reqwest's native-roots TLS (M1, R2) — no
/// rustls-platform-verifier.
pub fn download_verify_extract(url: &str, dest_dir: &Path) -> anyhow::Result<()> {
    let parent = dest_dir
        .parent()
        .ok_or_else(|| anyhow!("destination {} has no parent dir", dest_dir.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("creating parent dir {}", parent.display()))?;

    let lock_name = format!(
        "{}.lock",
        dest_dir.file_name().and_then(|s| s.to_str()).unwrap_or("download")
    );
    let lock_path = parent.join(lock_name);

    paths::with_file_lock(&lock_path, || {
        let name = url
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("download.tar.gz")
            .to_string();

        let client = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("building reqwest blocking client")?;
        let resp = client
            .get(url)
            .send()
            .with_context(|| format!("GET {url}"))?
            .error_for_status()
            .with_context(|| format!("non-success status from {url}"))?;
        let bytes = resp.bytes().with_context(|| format!("reading body of {url}"))?;

        verify_and_extract_bytes(&bytes, &name, dest_dir)
    })
}

#[cfg(test)]
#[path = "download_tests.rs"]
mod download_tests;
