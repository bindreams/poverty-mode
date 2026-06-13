use super::*;

use std::io::Read;

use tracing::subscriber::with_default;
use tracing_subscriber::fmt::MakeWriter;

/// A `MakeWriter` that appends to a shared `Vec<u8>`, so we can assert on log
/// output synchronously (no sleeps, no polling).
#[derive(Clone, Default)]
struct VecWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for VecWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for VecWriter {
    type Writer = VecWriter;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[test]
fn build_subscriber_to_writer_captures_a_log_line() {
    let sink = VecWriter::default();
    let subscriber = build_subscriber(sink.clone(), false);
    with_default(subscriber, || {
        tracing::info!("hello from test");
    });
    let bytes = sink.0.lock().unwrap().clone();
    let text = String::from_utf8(bytes).unwrap();
    assert!(
        text.contains("hello from test"),
        "log output should contain the message, got: {text:?}"
    );
    // ANSI disabled => no escape sequences.
    assert!(!text.contains('\u{1b}'), "expected no ANSI escapes: {text:?}");
}

#[test]
fn build_subscriber_with_ansi_emits_escapes() {
    let sink = VecWriter::default();
    let subscriber = build_subscriber(sink.clone(), true);
    with_default(subscriber, || {
        tracing::info!("colored line");
    });
    let text = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    assert!(text.contains("colored line"), "got: {text:?}");
    assert!(
        text.contains('\u{1b}'),
        "ANSI enabled should emit escape sequences, got: {text:?}"
    );
}

/// The ONLY test that installs the process-global subscriber (set_global_default
/// is one-shot). Keep exactly one such test in this file.
#[test]
fn init_tracing_to_file_creates_parent_and_writes() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("nested").join("run.log");

    init_tracing(Some(log_path.as_path())).unwrap();
    tracing::info!("file target line");

    // The file `MakeWriter` opens-append-writes per event, so the bytes are on
    // disk the instant `info!` returns — read it back with no synchronization.
    let mut contents = String::new();
    std::fs::File::open(&log_path)
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert!(
        contents.contains("file target line"),
        "log file should contain the message, got: {contents:?}"
    );
}
