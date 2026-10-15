//! Raw source code in a `tool_result` must reach the upstream byte-for-byte.
//!
//! Upstream headroom compresses source code lossily (function bodies elided, no
//! retrieval marker). poverty-mode keeps that arm off by running the headroom
//! proxy child with `source_code` in `HEADROOM_LIVE_ZONE_DISABLE_ARMS`. The
//! switch is process-global and latched at first dispatch, so this drives the
//! real `proxy headroom` child.

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};

use common::stub::start_stub;
use poverty_mode::proxy::ReadyLine;
use serde_json::json;

fn python_source() -> String {
    (0..60)
        .map(|i| {
            format!(
                "def handler_{i}(request, context):\n    \"\"\"Handle request {i}.\"\"\"\n    \
                 value = request.get('field_{i}')\n    if value is None:\n        \
                 raise ValueError('missing field_{i}')\n    result = transform_{i}(value, context)\n    \
                 context.log('handled {i}', result)\n    return result\n\n"
            )
        })
        .collect()
}

#[test]
fn headroom_child_forwards_source_code_tool_result_unchanged() {
    let stub = start_stub(r#"{"ok":true}"#);
    let body = serde_json::to_vec(&json!({
        "model": "claude-sonnet-4-6",
        "max_tokens": 64,
        "messages": [{
            "role": "user",
            "content": [{ "type": "tool_result", "tool_use_id": "toolu_src", "content": python_source() }],
        }],
    }))
    .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_poverty-mode"))
        .args([
            "proxy",
            "headroom",
            "--listen",
            "127.0.0.1:0",
            "--upstream",
            &format!("http://127.0.0.1:{}/", stub.port),
            "--run-id",
            "01J0SRCCODE",
        ])
        .env_remove("HEADROOM_LIVE_ZONE_DISABLE_ARMS")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn headroom proxy child");

    let mut reader = BufReader::new(child.stdout.take().expect("child stdout"));
    let mut line = String::new();
    reader.read_line(&mut line).expect("read READY line");
    let ready: ReadyLine = serde_json::from_str(line.trim()).expect("parse ReadyLine");

    let mut conn = TcpStream::connect(("127.0.0.1", ready.port)).expect("connect proxy");
    write!(
        conn,
        "POST /v1/messages HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    conn.write_all(&body).unwrap();
    let mut response = Vec::new();
    conn.read_to_end(&mut response).expect("read response to EOF");

    let _ = child.kill();
    let _ = child.wait();

    let captured = stub.last().expect("upstream received the request");
    assert!(captured.body == body, "source-code tool_result must not be rewritten");
}
