use super::*;

#[test]
fn not_implemented_displays_subcommand() {
    let e = Error::NotImplemented("run");
    assert_eq!(e.to_string(), "not yet implemented: run");
}

#[test]
fn wraps_anyhow_transparently() {
    let src = anyhow::anyhow!("disk on fire");
    let e: Error = src.into();
    assert_eq!(e.to_string(), "disk on fire");
}

#[test]
fn crate_result_alias_is_usable() {
    fn ok() -> Result<u8> {
        Ok(7)
    }
    fn boom() -> Result<u8> {
        Err(Error::NotImplemented("x"))
    }
    assert_eq!(ok().unwrap(), 7);
    assert!(boom().is_err());
}
