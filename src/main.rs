//! Thin binary shim. All logic lives in the `poverty_mode` library (R1).

fn main() -> anyhow::Result<()> {
    poverty_mode::run()
}
