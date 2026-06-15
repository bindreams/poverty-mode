mod common;

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use poverty_mode::proxy::ReadyLine;

#[test]
fn proxy_subcommand_binds_prints_ready_line_with_real_port() {
    // Unreachable upstream is fine: we only read the READY line (health/bind are
    // local; we send no traffic).
    let upstream = "http://127.0.0.1:1/";

    let mut child = Command::new(env!("CARGO_BIN_EXE_poverty-mode"))
        .args([
            "proxy",
            "pino",
            "--listen",
            "127.0.0.1:0",
            "--upstream",
            upstream,
            "--run-id",
            "01J0CLIRUN",
            "--auto-cache",
            "--main-ttl",
            "1h",
            "--no-strip-ansi",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn proxy child");

    // Blocking pipe read of the first stdout line == real synchronization.
    let stdout = child.stdout.take().expect("child stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read READY line");

    let ready: ReadyLine = serde_json::from_str(line.trim()).expect("parse ReadyLine");
    assert!(ready.ready);
    assert_eq!(ready.proxy, "pino");
    assert_eq!(ready.run_id, "01J0CLIRUN");
    assert_ne!(
        ready.port, 0,
        "READY must report the real bound ephemeral port"
    );

    // Confirm the port is actually bound by connecting to it.
    let addr = format!("127.0.0.1:{}", ready.port);
    assert!(
        std::net::TcpStream::connect(&addr).is_ok(),
        "reported port must be connectable"
    );

    child.kill().expect("kill child");
    let _ = child.wait();
}
