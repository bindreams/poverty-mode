use super::*;

#[test]
fn build_command_uses_argv_program_and_args() {
    let agent = ClaudeAgent;
    let base = Url::parse("http://127.0.0.1:4100/").unwrap();
    let argv = vec![
        "/usr/bin/claude".to_string(),
        "--print".to_string(),
        "hi".to_string(),
    ];
    let cmd = agent.build_command(&argv, &base, &[]);
    let std = cmd.as_std();
    // Generic model (M6): program is argv[0].
    assert_eq!(std.get_program(), std::ffi::OsStr::new("/usr/bin/claude"));
    // M7.2/M7.4 insert `--settings <json>` between the program and argv[1..]; the
    // user's args (argv[1..]) follow verbatim and in order. (We assert the flag's
    // position and the trailing user args here; the JSON value's *contents* are
    // characterized by the M7.4 `settings_*` tests, so we do not hardcode them.)
    let args: Vec<_> = std
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(args[0], "--settings");
    assert_eq!(&args[2..], &["--print".to_string(), "hi".to_string()]);
}

#[test]
fn build_command_exports_base_url_and_extra_env() {
    let agent = ClaudeAgent;
    let base = Url::parse("http://127.0.0.1:5000/").unwrap();
    let argv = vec!["true".to_string()];
    let extra = vec![
        ("POVERTY_PROXY_CHAIN".to_string(), "pino".to_string()),
        ("ENABLE_TOOL_SEARCH".to_string(), "true".to_string()),
    ];
    let cmd = agent.build_command(&argv, &base, &extra);
    let std = cmd.as_std();
    let envs: std::collections::HashMap<_, _> = std
        .get_envs()
        .filter_map(|(k, v)| v.map(|v| (k.to_owned(), v.to_owned())))
        .collect();
    assert_eq!(
        envs.get(std::ffi::OsStr::new("ANTHROPIC_BASE_URL"))
            .map(|v| v.as_os_str()),
        Some(std::ffi::OsStr::new("http://127.0.0.1:5000/"))
    );
    assert_eq!(
        envs.get(std::ffi::OsStr::new("POVERTY_PROXY_CHAIN"))
            .map(|v| v.as_os_str()),
        Some(std::ffi::OsStr::new("pino"))
    );
    assert_eq!(
        envs.get(std::ffi::OsStr::new("ENABLE_TOOL_SEARCH"))
            .map(|v| v.as_os_str()),
        Some(std::ffi::OsStr::new("true"))
    );
}

#[test]
fn name_is_claude() {
    assert_eq!(ClaudeAgent.name(), "claude");
}

// M7.2 (characterization of the M6 generic model + the `--settings` insertion).
//
// RECONCILIATION: M6 implemented `build_command` with the GENERIC model —
// program = argv[0], its args = argv[1..], with ANTHROPIC_BASE_URL + every
// extra_env entry mirrored into the child env. M7.2 LOCKS that model and adds
// belt 2: a single `--settings <json>` pair inserted immediately AFTER the
// program (argv[0]) and BEFORE argv[1..]. (The original M7.2 task asserted a
// hardcoded program=="claude" and the whole user argv after `--settings`; that
// is incompatible with the committed M6 model, so per the controller decision the
// assertions are amended to: program==argv[0], `--settings` between program and
// argv[1..]. The JSON contents are characterized in M7.4.)

// Collect the built command's args as owned Strings for assertion.
fn args_of(cmd: &tokio::process::Command) -> Vec<String> {
    cmd.as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn program_is_argv0() {
    // Generic model (M6): the program is argv[0], NOT a hardcoded "claude".
    let base = Url::parse("http://127.0.0.1:4100").unwrap();
    let argv = vec!["/opt/bin/claude".to_string(), "--print".to_string()];
    let cmd = ClaudeAgent.build_command(&argv, &base, &[]);
    assert_eq!(
        cmd.as_std().get_program(),
        std::ffi::OsStr::new("/opt/bin/claude")
    );
}

#[test]
fn empty_argv_program_falls_back_to_name() {
    // Empty argv has no program; the agent falls back to its `name()` binary.
    let base = Url::parse("http://127.0.0.1:4100").unwrap();
    let cmd = ClaudeAgent.build_command(&[], &base, &[]);
    assert_eq!(cmd.as_std().get_program(), std::ffi::OsStr::new("claude"));
}

#[test]
fn settings_flag_inserted_between_program_and_user_args() {
    // argv = [program, user_arg1, user_arg2]: the program becomes get_program();
    // the emitted args are `--settings <json>` followed by argv[1..] in order.
    let base = Url::parse("http://127.0.0.1:4100").unwrap();
    let argv = vec![
        "claude".to_string(),
        "--print".to_string(),
        "do a thing".to_string(),
    ];
    let cmd = ClaudeAgent.build_command(&argv, &base, &[]);
    let args = args_of(&cmd);

    // Exactly one --settings flag, with a value immediately after it.
    let pos = args
        .iter()
        .position(|a| a == "--settings")
        .expect("--settings present");
    assert!(
        pos + 1 < args.len(),
        "--settings must be followed by a value"
    );
    assert_eq!(args.iter().filter(|a| *a == "--settings").count(), 1);

    // `--settings` lands at the very front of the arg vector (right after the
    // program, before any user args).
    assert_eq!(pos, 0, "--settings must precede the user's args");

    // User argv[1..] comes strictly after the settings flag+value.
    let user_start = pos + 2;
    assert_eq!(
        &args[user_start..],
        &["--print".to_string(), "do a thing".to_string()]
    );
}

#[test]
fn empty_user_args_still_emit_settings() {
    // argv = [program] only (no user args after the program): we still emit the
    // `--settings <json>` pair and nothing else.
    let base = Url::parse("http://127.0.0.1:4100").unwrap();
    let argv = vec!["claude".to_string()];
    let cmd = ClaudeAgent.build_command(&argv, &base, &[]);
    let args = args_of(&cmd);
    assert_eq!(args.len(), 2, "exactly --settings + json, no user args");
    assert_eq!(args[0], "--settings");
}

// M7.3 (characterization of the M6 process-env belt — belt 1 of the R8 dual belt).
//
// RECONCILIATION (R12 labeling): the env-mirroring behavior these tests inspect —
// `ANTHROPIC_BASE_URL` set from `base_url`, plus every `extra_env` pair copied
// into the child env — was implemented and committed in M6 (it is what the
// orchestrator's env half relies on). The original M7 task scripted these as
// red→green (env unwired until M7.3), but under the controller decision M6 already
// wired belt 1, so these are CHARACTERIZATION guards added after the behavior
// exists: they LOCK belt 1 so an accidental change to the env wiring is caught.
// They are NOT dressed as red→green.

// Read a single env var off the built command, if present and set.
fn env_of(cmd: &tokio::process::Command, key: &str) -> Option<String> {
    cmd.as_std()
        .get_envs()
        .find(|(k, _)| *k == std::ffi::OsStr::new(key))
        .and_then(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
}

#[test]
fn process_env_sets_base_url() {
    let base = Url::parse("http://127.0.0.1:4100").unwrap();
    let cmd = ClaudeAgent.build_command(&[], &base, &[]);
    assert_eq!(
        env_of(&cmd, "ANTHROPIC_BASE_URL"),
        Some("http://127.0.0.1:4100/".to_string())
    );
}

#[test]
fn process_env_includes_every_extra_env_entry() {
    let base = Url::parse("http://127.0.0.1:4100").unwrap();
    let extra = vec![
        (
            "POVERTY_PROXY_CHAIN".to_string(),
            "pino,headroom".to_string(),
        ),
        ("ENABLE_TOOL_SEARCH".to_string(), "true".to_string()),
    ];
    let cmd = ClaudeAgent.build_command(&[], &base, &extra);

    assert_eq!(
        env_of(&cmd, "POVERTY_PROXY_CHAIN"),
        Some("pino,headroom".to_string())
    );
    assert_eq!(env_of(&cmd, "ENABLE_TOOL_SEARCH"), Some("true".to_string()));
    // base url is still set even when extra_env does not carry it.
    assert_eq!(
        env_of(&cmd, "ANTHROPIC_BASE_URL"),
        Some("http://127.0.0.1:4100/".to_string())
    );
}

#[test]
fn central_tail_auth_token_lands_in_process_env() {
    // Orchestrator marks a central tail by adding ANTHROPIC_AUTH_TOKEN=wire-proxy.
    let base = Url::parse("http://127.0.0.1:4100").unwrap();
    let extra = vec![
        ("ENABLE_TOOL_SEARCH".to_string(), "true".to_string()),
        ("ANTHROPIC_AUTH_TOKEN".to_string(), "wire-proxy".to_string()),
    ];
    let cmd = ClaudeAgent.build_command(&[], &base, &extra);
    assert_eq!(
        env_of(&cmd, "ANTHROPIC_AUTH_TOKEN"),
        Some("wire-proxy".to_string())
    );
}

#[test]
fn anthropic_tail_has_no_auth_token_override() {
    // No central tail => orchestrator does NOT add ANTHROPIC_AUTH_TOKEN, so the
    // child inherits the user's real Anthropic auth verbatim (we set nothing).
    // (extra_env still carries ENABLE_TOOL_SEARCH from the orchestrator.)
    let base = Url::parse("http://127.0.0.1:4100").unwrap();
    let extra = vec![("ENABLE_TOOL_SEARCH".to_string(), "true".to_string())];
    let cmd = ClaudeAgent.build_command(&[], &base, &extra);
    assert_eq!(env_of(&cmd, "ANTHROPIC_AUTH_TOKEN"), None);
}

// M7.4 (the real new work: belt 2's `--settings {"env":{...}}` contents) =====
//
// The `--settings` JSON env block must carry exactly belt 1's pairs:
// ANTHROPIC_BASE_URL plus every extra_env entry — so the two belts cannot
// disagree (design §8). These tests parse the JSON back and assert per-field.

use serde_json::Value;

// Extract and parse the JSON value passed to `--settings`.
fn settings_json(cmd: &tokio::process::Command) -> Value {
    let args = args_of(cmd);
    let pos = args
        .iter()
        .position(|a| a == "--settings")
        .expect("--settings present");
    let raw = &args[pos + 1];
    serde_json::from_str(raw).expect("--settings value must be valid JSON")
}

#[test]
fn settings_env_block_carries_base_url() {
    let base = Url::parse("http://127.0.0.1:4100").unwrap();
    let v = settings_json(&ClaudeAgent.build_command(&[], &base, &[]));
    assert_eq!(
        v["env"]["ANTHROPIC_BASE_URL"],
        Value::String("http://127.0.0.1:4100/".to_string())
    );
}

#[test]
fn settings_env_mirrors_extra_env_exactly() {
    let base = Url::parse("http://127.0.0.1:4100").unwrap();
    let extra = vec![
        (
            "POVERTY_PROXY_CHAIN".to_string(),
            "pino,central".to_string(),
        ),
        ("ENABLE_TOOL_SEARCH".to_string(), "true".to_string()),
        ("ANTHROPIC_AUTH_TOKEN".to_string(), "wire-proxy".to_string()),
    ];
    let v = settings_json(&ClaudeAgent.build_command(&[], &base, &extra));
    let env = v["env"].as_object().expect("env is an object");

    // Exactly base_url + the three extra entries: 4 keys, no more.
    assert_eq!(env.len(), 4, "env block has exactly base_url + extra_env");
    assert_eq!(
        env["ANTHROPIC_BASE_URL"],
        Value::String("http://127.0.0.1:4100/".to_string())
    );
    assert_eq!(
        env["POVERTY_PROXY_CHAIN"],
        Value::String("pino,central".to_string())
    );
    assert_eq!(env["ENABLE_TOOL_SEARCH"], Value::String("true".to_string()));
    assert_eq!(
        env["ANTHROPIC_AUTH_TOKEN"],
        Value::String("wire-proxy".to_string())
    );
}

#[test]
fn anthropic_tail_settings_omits_auth_token() {
    // No central tail => no ANTHROPIC_AUTH_TOKEN in extra_env => absent from the
    // settings env block too (user auth flows through verbatim).
    let base = Url::parse("http://127.0.0.1:4100").unwrap();
    let extra = vec![("ENABLE_TOOL_SEARCH".to_string(), "true".to_string())];
    let v = settings_json(&ClaudeAgent.build_command(&[], &base, &extra));
    let env = v["env"].as_object().unwrap();
    assert!(!env.contains_key("ANTHROPIC_AUTH_TOKEN"));
    assert_eq!(env.len(), 2); // base_url + ENABLE_TOOL_SEARCH
}

#[test]
fn settings_env_matches_process_env_for_every_key() {
    // The dual-belt invariant: every key in the settings env block has the same
    // value as the process env, and vice versa.
    let base = Url::parse("http://127.0.0.1:4100").unwrap();
    let extra = vec![
        ("POVERTY_PROXY_CHAIN".to_string(), "headroom".to_string()),
        ("ENABLE_TOOL_SEARCH".to_string(), "true".to_string()),
        ("ANTHROPIC_AUTH_TOKEN".to_string(), "wire-proxy".to_string()),
    ];
    let cmd = ClaudeAgent.build_command(&[], &base, &extra);
    let v = settings_json(&cmd);
    let env = v["env"].as_object().unwrap();

    for (k, val) in env {
        let from_proc = env_of(&cmd, k);
        let expected = val.as_str().unwrap().to_string();
        assert_eq!(
            from_proc,
            Some(expected),
            "settings key {k} must equal the process-env value"
        );
    }
    // ANTHROPIC_BASE_URL is in process env too even though the orchestrator
    // does not include it in extra_env.
    assert_eq!(
        env_of(&cmd, "ANTHROPIC_BASE_URL"),
        Some("http://127.0.0.1:4100/".to_string())
    );
}

#[test]
fn settings_value_escapes_special_characters() {
    // A value with quotes/backslashes/newlines must round-trip through JSON,
    // proving we serialize with serde_json rather than string concatenation.
    // Note: cross-platform safety (spec 12) — the arg is passed via the
    // std::process arg vector (no shell), so Rust handles OS-level quoting on
    // both Unix and Windows; only JSON-level escaping is our concern here.
    let base = Url::parse("http://127.0.0.1:4100").unwrap();
    let extra = vec![
        ("ENABLE_TOOL_SEARCH".to_string(), "true".to_string()),
        ("WEIRD".to_string(), "a\"b\\c\nd".to_string()),
    ];
    let v = settings_json(&ClaudeAgent.build_command(&[], &base, &extra));
    assert_eq!(v["env"]["WEIRD"], Value::String("a\"b\\c\nd".to_string()));
}

// M7.5 (ENABLE_TOOL_SEARCH cross-check, pinned flag/key constants, Managed +
// remote-exec notes) =====
//
// These lock the four M7.5 contract gaps: the `--settings` flag and env-key
// names are pinned so a silent rename can't break precedence; the cross-check
// debug-assert catches an orchestrator (M6) regression that drops
// ENABLE_TOOL_SEARCH; and two machine-checked constants document the Managed
// policy ceiling and the remote-execution chain bypass (spec §8).

use crate::agent::claude::{
    ENV_BASE_URL, ENV_ENABLE_TOOL_SEARCH, MANAGED_POLICY_NOTE, REMOTE_EXECUTION_NOTE, SETTINGS_FLAG,
};

#[test]
fn settings_flag_constant_matches_emitted_arg() {
    let base = Url::parse("http://127.0.0.1:4100").unwrap();
    let args = args_of(&ClaudeAgent.build_command(&[], &base, &[]));
    assert!(args.iter().any(|a| a == SETTINGS_FLAG));
    assert_eq!(SETTINGS_FLAG, "--settings");
}

#[test]
fn env_key_constants_are_the_names_we_emit() {
    assert_eq!(ENV_BASE_URL, "ANTHROPIC_BASE_URL");
    assert_eq!(ENV_ENABLE_TOOL_SEARCH, "ENABLE_TOOL_SEARCH");
    // The base-URL key is actually emitted (guards against a silent rename).
    let base = Url::parse("http://127.0.0.1:4100").unwrap();
    let v = settings_json(&ClaudeAgent.build_command(&[], &base, &[]));
    assert!(v["env"].as_object().unwrap().contains_key(ENV_BASE_URL));
}

#[test]
fn managed_policy_note_documents_the_one_layer_we_cannot_beat() {
    // The note must name Managed and must NOT claim we override it.
    assert!(MANAGED_POLICY_NOTE.contains("Managed"));
    assert!(
        !MANAGED_POLICY_NOTE
            .to_lowercase()
            .contains("override managed"),
        "we must not claim to beat Managed policy"
    );
}

#[test]
fn remote_execution_bypass_is_documented() {
    // Spec §8: cloud/remote execution (scheduled routines, RemoteTrigger) runs
    // server-side and inherently bypasses the local chain — documented as such.
    let lower = REMOTE_EXECUTION_NOTE.to_lowercase();
    assert!(lower.contains("remote") || lower.contains("cloud"));
    assert!(lower.contains("bypass"));
}

#[test]
fn tool_search_cross_check_passes_when_present() {
    // Orchestrator-shaped call: ENABLE_TOOL_SEARCH present => no panic.
    let base = Url::parse("http://127.0.0.1:4100").unwrap();
    let extra = vec![
        ("POVERTY_PROXY_CHAIN".to_string(), "pino".to_string()),
        ("ENABLE_TOOL_SEARCH".to_string(), "true".to_string()),
    ];
    // Must not panic.
    let _ = ClaudeAgent.build_command(&[], &base, &extra);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "ENABLE_TOOL_SEARCH")]
fn tool_search_cross_check_fires_when_orchestrator_drops_it() {
    // Non-empty extra_env without ENABLE_TOOL_SEARCH is an M6 contract breach:
    // the debug-assert must fire so a regression is caught in CI, not shipped.
    let base = Url::parse("http://127.0.0.1:4100").unwrap();
    let extra = vec![("POVERTY_PROXY_CHAIN".to_string(), "pino".to_string())];
    let _ = ClaudeAgent.build_command(&[], &base, &extra);
}
