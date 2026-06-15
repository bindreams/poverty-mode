// EMPIRICAL VERIFICATION GATES (design §16/§17). NOT part of normal CI.
// Require the real `claude` binary on PATH, logged in. Run explicitly:
//   cargo test --test agent_empirical -- --ignored
// See tests/EMPIRICAL_GATES.md for the protocol, PASS criteria, and where the
// observed results are RECORDED.

mod common;

use std::time::Duration;

use common::stub::start_stub;
use poverty_mode::agent::claude::ClaudeAgent;
use poverty_mode::agent::Agent;
use url::Url;

// Minimal non-streaming Anthropic-shaped body so `claude` does not error on the
// response while we are only interested in WHICH upstream it reached.
const CANNED: &str = r#"{"id":"msg_stub","type":"message","role":"assistant","model":"stub","content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}"#;

// Human-surfaced failure bound (the SANCTIONED timeout exception, R8): its ONLY
// role is to turn a hung external `claude` process into a clear diagnostic +
// child kill. It does NOT synchronize anything — claude's exit is a genuine
// external event that might never happen (credential/network/login stall).
const CLAUDE_EXIT_DEADLINE: Duration = Duration::from_secs(120);

// EMPIRICAL VERIFICATION GATE (a): process-env vs settings.json env-block
// precedence for ANTHROPIC_BASE_URL.
//
// Method: point the process-env ANTHROPIC_BASE_URL at stub A and the
// --settings env-block ANTHROPIC_BASE_URL at stub B (two different ports). Drive
// a one-shot `claude --print`. Whichever stub records a /v1/messages request is
// the belt that won. The auth token is carried in BOTH belts (production-faithful);
// only the base URL differs, so it alone determines the winner.
#[tokio::test]
#[ignore = "requires installed claude; run with --ignored (empirical gate a)"]
async fn process_env_vs_settings_block_precedence() {
    let stub_proc = start_stub(CANNED);
    let stub_settings = start_stub(CANNED);

    let proc_url = Url::parse(&format!("http://127.0.0.1:{}", stub_proc.port)).unwrap();
    let settings_url = Url::parse(&format!("http://127.0.0.1:{}", stub_settings.port)).unwrap();

    // build_command sets BOTH belts to `settings_url` by design. To pit the two
    // belts against each other we override the PROCESS-env belt afterward to
    // `proc_url`, leaving the --settings JSON pointing at `settings_url`. The
    // auth token (carried in extra_env) lands in BOTH belts identically, so the
    // base URL is the only differing value.
    let mut cmd = ClaudeAgent.build_command(
        // Generic model (M6): argv[0] is the PROGRAM. Lead with "claude" so
        // build_command's split_first() yields program="claude" and inserts
        // `--settings <json>` between it and the user flags (argv[1..]). Omitting
        // the program here would resolve program="--print" and spawn a nonexistent
        // `--print` binary, never reaching the precedence assertions below.
        &[
            "claude".to_string(),
            "--print".to_string(),
            "say ok and stop".to_string(),
        ],
        &settings_url,
        &[
            ("ENABLE_TOOL_SEARCH".to_string(), "true".to_string()),
            ("ANTHROPIC_AUTH_TOKEN".to_string(), "empirical-dummy".to_string()),
        ],
    );
    cmd.env("ANTHROPIC_BASE_URL", proc_url.as_str());
    cmd.kill_on_drop(true);

    let mut child = cmd.spawn().expect(
        "claude must be installed on PATH for the empirical gate \
         (run with --ignored only when provisioned)",
    );

    // Human-surfaced failure bound only (R8 sanctioned exception): on expiry,
    // kill the child and fail with an actionable message. Not a sync primitive.
    let status = match tokio::time::timeout(CLAUDE_EXIT_DEADLINE, child.wait()).await {
        Ok(res) => res.expect("awaiting claude exit failed"),
        Err(_elapsed) => {
            let _ = child.kill().await;
            panic!(
                "claude did not exit within {}s — investigate a hang \
                 (login/credential/network stall or a prompt that never terminates)",
                CLAUDE_EXIT_DEADLINE.as_secs()
            );
        }
    };

    // Decide hits from the canonical stub's count(); confirm a /v1 request via
    // first_segment() so a non-/v1/messages probe does not count as a "hit".
    let proc_hit = stub_proc.count() > 0;
    let settings_hit = stub_settings.count() > 0;
    let proc_v1 = stub_proc.first_segment().as_deref() == Some("v1");
    let settings_v1 = stub_settings.first_segment().as_deref() == Some("v1");

    eprintln!(
        "EMPIRICAL(a): claude_exit_success={} process_env_hit={proc_hit}(v1={proc_v1}) \
         settings_block_hit={settings_hit}(v1={settings_v1}) \
         note=auth_token_identical_in_both_belts;base_url_is_the_only_differing_value",
        status.success()
    );

    // Distinguish the confusing case (missing-coverage #3): if NEITHER belt was
    // hit AND claude exited non-zero, claude errored BEFORE reaching any upstream
    // (login/network/Managed) — a real environment failure, not a precedence
    // result. Surface that distinctly rather than as an ambiguous xor failure.
    if !proc_hit && !settings_hit {
        panic!(
            "neither belt received a request (claude_exit_success={}) — claude never \
             reached an upstream; investigate login/credentials/network or a Managed \
             ANTHROPIC_BASE_URL policy. This is an environment failure, not a precedence result.",
            status.success()
        );
    }

    // Exactly one belt should have received the request.
    assert!(
        proc_hit ^ settings_hit,
        "exactly one belt must receive the request (proc={proc_hit}, settings={settings_hit})"
    );

    // The hit must be a /v1/messages-style request, not an incidental probe.
    let winner_v1 = if settings_hit { settings_v1 } else { proc_v1 };
    assert!(
        winner_v1,
        "the winning belt's recorded request was not a /v1/... request \
         (proc_v1={proc_v1}, settings_v1={settings_v1}); the hit may be an incidental probe"
    );

    // PASS criterion (design §8 working assumption): the --settings env block,
    // landing at CLI-arg precedence, wins. If this fails with proc_hit=true, the
    // design's belt-2 guarantee is wrong and §8 must be revisited. Record the
    // outcome in tests/EMPIRICAL_GATES.md regardless of pass/fail (Task M7.8).
    assert!(
        settings_hit,
        "EMPIRICAL FAIL: process-env belt won over the --settings env block; \
         re-examine design §8 precedence assumption and record in EMPIRICAL_GATES.md"
    );
}

// EMPIRICAL VERIFICATION GATE (b): subagent endpoint inheritance.
//
// Method: point claude at a SINGLE canonical stub via both belts (the production
// wiring, auth token in both belts), then prompt it to spawn a subagent (Task
// tool). If subagents inherit the resolved endpoint, BOTH the main loop and the
// subagent reach our one stub: the stub records >1 request. Since there is a
// single stub on a single loopback port, any reached request shares the same
// host/port by construction — count() is the discriminator.
#[tokio::test]
#[ignore = "requires installed claude; run with --ignored (empirical gate b)"]
async fn subagent_inherits_chain_endpoint() {
    let stub = start_stub(CANNED);
    let base = Url::parse(&format!("http://127.0.0.1:{}", stub.port)).unwrap();

    let prompt = "Use the Task tool to spawn one general-purpose subagent that \
                  replies with the single word ok. Then stop.";

    let mut cmd = ClaudeAgent.build_command(
        // Generic model (M6): argv[0] is the PROGRAM. Lead with "claude" so
        // build_command's split_first() yields program="claude" and inserts
        // `--settings <json>` between it and the user flags (argv[1..]). Omitting
        // the program here would resolve program="--print" and spawn a nonexistent
        // `--print` binary, never reaching the inheritance assertions below.
        &["claude".to_string(), "--print".to_string(), prompt.to_string()],
        &base,
        &[
            ("ENABLE_TOOL_SEARCH".to_string(), "true".to_string()),
            ("ANTHROPIC_AUTH_TOKEN".to_string(), "empirical-dummy".to_string()),
        ],
    );
    cmd.kill_on_drop(true);

    let mut child = cmd.spawn().expect(
        "claude must be installed on PATH for the empirical gate \
         (run with --ignored only when provisioned)",
    );

    // Human-surfaced failure bound only (R8 sanctioned exception): a subagent
    // prompt that never terminates would otherwise hang CI forever.
    let status = match tokio::time::timeout(CLAUDE_EXIT_DEADLINE, child.wait()).await {
        Ok(res) => res.expect("awaiting claude exit failed"),
        Err(_elapsed) => {
            let _ = child.kill().await;
            panic!(
                "claude did not exit within {}s — investigate a hang \
                 (login/credential/network stall or a subagent prompt that never terminates)",
                CLAUDE_EXIT_DEADLINE.as_secs()
            );
        }
    };

    let count = stub.count();
    let last_host = stub.last().and_then(|c| c.host);
    let first_seg = stub.first_segment();

    eprintln!(
        "EMPIRICAL(b): claude_exit_success={} requests={count} \
         last_host={last_host:?} first_segment={first_seg:?}",
        status.success()
    );

    // Distinguish the confusing case (missing-coverage #3): zero requests + a
    // non-zero exit means claude never reached the stub (login/network/Managed),
    // not "subagent diverged". Surface that distinctly.
    if count == 0 {
        panic!(
            "no request reached the stub (claude_exit_success={}) — claude never \
             reached an upstream; investigate login/credentials/network or a Managed \
             ANTHROPIC_BASE_URL policy. This is an environment failure, not a subagent result.",
            status.success()
        );
    }

    assert!(count >= 1, "the main loop must reach our stub at minimum");

    // PASS criterion: the subagent reached the SAME (single) stub as the main
    // loop, so the stub recorded a second request. Record the outcome in
    // tests/EMPIRICAL_GATES.md regardless of pass/fail (Task M7.8).
    assert!(
        count >= 2,
        "EMPIRICAL FAIL: only {count} request(s) reached the stub — the subagent did \
         not inherit the chain endpoint; re-examine design §8 subagent assumption \
         and record in EMPIRICAL_GATES.md"
    );
}

// Characterization guard (R12): asserts the recorded-results doc exists and
// carries the load-bearing sections. Not a red→green behavior test.
#[test]
fn empirical_gates_doc_has_required_sections() {
    let doc = include_str!("EMPIRICAL_GATES.md");
    assert!(doc.contains("--ignored"), "must document the opt-in run flag");
    assert!(doc.contains("agent_empirical"), "must name the test target");
    assert!(
        doc.contains("not run in normal CI") || doc.contains("not part of normal CI"),
        "must state the gates are excluded from normal CI"
    );
    assert!(
        doc.contains("Recorded results"),
        "must have a place where observed results are recorded (R8)"
    );
    assert!(
        doc.to_lowercase().contains("load-bearing") || doc.to_lowercase().contains("belt"),
        "must record which belt is authoritative and why the other is kept (R8 follow-up)"
    );
    assert!(
        doc.to_lowercase().contains("remote") || doc.to_lowercase().contains("cloud"),
        "must document the remote/cloud execution bypass (spec §8)"
    );
    assert!(
        doc.to_lowercase().contains("windows"),
        "must confirm Windows inline-JSON arg passing (spec §12)"
    );
    assert!(
        doc.to_lowercase().contains("central"),
        "must forward-reference the live-central suite (R7)"
    );
}
