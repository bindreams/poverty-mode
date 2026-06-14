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
    // M7.2 inserts `--settings <json>` between the program and argv[1..]; the
    // user's args (argv[1..]) follow verbatim and in order.
    let args: Vec<_> = std.get_args().collect();
    assert_eq!(
        args,
        vec![
            std::ffi::OsStr::new("--settings"),
            std::ffi::OsStr::new("{}"),
            std::ffi::OsStr::new("--print"),
            std::ffi::OsStr::new("hi"),
        ]
    );
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
