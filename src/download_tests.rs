use super::*;

const BASE: &str = "https://jetbrains-central-cli.s3.eu-west-1.amazonaws.com";

#[test]
fn linux_x86_64_is_tar_gz() {
    let got = jbcentral_asset_url("0.2.9", "linux", "x86_64").unwrap();
    assert_eq!(
        got,
        format!("{BASE}/jbcentral/0.2.9/jbcentral_0.2.9_linux_x86_64.tar.gz")
    );
}

#[test]
fn linux_arm64_is_tar_gz() {
    let got = jbcentral_asset_url("0.2.9", "linux", "arm64").unwrap();
    assert_eq!(
        got,
        format!("{BASE}/jbcentral/0.2.9/jbcentral_0.2.9_linux_arm64.tar.gz")
    );
}

#[test]
fn darwin_x86_64_is_tar_gz() {
    let got = jbcentral_asset_url("0.2.9", "darwin", "x86_64").unwrap();
    assert_eq!(
        got,
        format!("{BASE}/jbcentral/0.2.9/jbcentral_0.2.9_darwin_x86_64.tar.gz")
    );
}

#[test]
fn darwin_arm64_is_tar_gz() {
    let got = jbcentral_asset_url("0.2.9", "darwin", "arm64").unwrap();
    assert_eq!(
        got,
        format!("{BASE}/jbcentral/0.2.9/jbcentral_0.2.9_darwin_arm64.tar.gz")
    );
}

#[test]
fn windows_x86_64_is_zip() {
    let got = jbcentral_asset_url("0.2.9", "windows", "x86_64").unwrap();
    assert_eq!(
        got,
        format!("{BASE}/jbcentral/0.2.9/jbcentral_0.2.9_windows_x86_64.zip")
    );
}

#[test]
fn windows_arm64_is_an_error() {
    let err = jbcentral_asset_url("0.2.9", "windows", "arm64").unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("windows") && msg.contains("arm64"),
        "error should name the unsupported target: {msg}"
    );
}

#[test]
fn unknown_os_is_an_error() {
    let err = jbcentral_asset_url("0.2.9", "plan9", "x86_64").unwrap_err();
    assert!(err.to_string().contains("plan9"), "{err}");
}

#[test]
fn unknown_arch_is_an_error() {
    let err = jbcentral_asset_url("0.2.9", "linux", "riscv64").unwrap_err();
    assert!(err.to_string().contains("riscv64"), "{err}");
}
