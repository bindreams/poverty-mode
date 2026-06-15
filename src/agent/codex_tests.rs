use super::*;

fn args_of(cmd: &tokio::process::Command) -> Vec<String> {
    cmd.as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

fn env_of(cmd: &tokio::process::Command, key: &str) -> Option<String> {
    cmd.as_std()
        .get_envs()
        .find(|(k, _)| *k == std::ffi::OsStr::new(key))
        .and_then(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
}

#[test]
fn name_and_wire_client_and_requires_central() {
    assert_eq!(CodexAgent.name(), "codex");
    assert_eq!(CodexAgent.wire_client_path(), "codex/openai");
    assert!(CodexAgent.requires_central());
}

#[test]
fn overrides_precede_subcommand_and_user_args_preserved() {
    let base = Url::parse("http://127.0.0.1:4100/codex/openai").unwrap();
    let argv = vec![
        "/opt/bin/codex".to_string(),
        "exec".to_string(),
        "--json".to_string(),
    ];
    let cmd = CodexAgent.build_command(&argv, &base, &[]);
    assert_eq!(cmd.as_std().get_program(), std::ffi::OsStr::new("/opt/bin/codex"));
    let args = args_of(&cmd);
    // The injected `-c` overrides sit at top level BEFORE the `exec` subcommand.
    let last_c = args.iter().rposition(|a| a == "-c").expect("a -c flag");
    let exec_pos = args.iter().position(|a| a == "exec").expect("exec present");
    assert!(last_c < exec_pos, "all -c overrides must precede `exec`: {args:?}");
    assert_eq!(&args[args.len() - 2..], &["exec".to_string(), "--json".to_string()]);
}

#[test]
fn injects_self_contained_provider_pointing_at_base_url() {
    let base = Url::parse("http://127.0.0.1:4100/codex/openai").unwrap();
    let cmd = CodexAgent.build_command(&["codex".to_string()], &base, &[]);
    let args = args_of(&cmd);
    assert_eq!(args.iter().filter(|a| *a == "-c").count(), 4);
    assert!(args.contains(&"model_provider=\"povertymode\"".to_string()));
    assert!(args.contains(&"model_providers.povertymode.name=\"poverty-mode\"".to_string()));
    assert!(args.contains(
        &"model_providers.povertymode.base_url=\"http://127.0.0.1:4100/codex/openai\"".to_string()
    ));
    assert!(args.contains(&"model_providers.povertymode.wire_api=\"responses\"".to_string()));
}

#[test]
fn mirrors_only_poverty_proxy_env_not_anthropic_or_tool_search() {
    let base = Url::parse("http://127.0.0.1:4100/codex/openai").unwrap();
    let extra = vec![
        ("POVERTY_PROXY_CHAIN".to_string(), "pino,central".to_string()),
        ("POVERTY_PROXY_HEAD".to_string(), "http://127.0.0.1:4100/".to_string()),
        ("ENABLE_TOOL_SEARCH".to_string(), "true".to_string()),
        ("ANTHROPIC_AUTH_TOKEN".to_string(), "wire-proxy".to_string()),
    ];
    let cmd = CodexAgent.build_command(&["codex".to_string()], &base, &extra);
    assert_eq!(env_of(&cmd, "POVERTY_PROXY_CHAIN"), Some("pino,central".to_string()));
    assert_eq!(env_of(&cmd, "POVERTY_PROXY_HEAD"), Some("http://127.0.0.1:4100/".to_string()));
    // Codex reads its base URL from `-c`, never ANTHROPIC_BASE_URL; and the Claude-
    // specific keys are not propagated to codex (avoids any provider-fallback risk).
    assert_eq!(env_of(&cmd, "ANTHROPIC_BASE_URL"), None);
    assert_eq!(env_of(&cmd, "ANTHROPIC_AUTH_TOKEN"), None);
    assert_eq!(env_of(&cmd, "ENABLE_TOOL_SEARCH"), None);
}

#[test]
fn empty_argv_falls_back_to_codex_program() {
    let base = Url::parse("http://127.0.0.1:4100/codex/openai").unwrap();
    let cmd = CodexAgent.build_command(&[], &base, &[]);
    assert_eq!(cmd.as_std().get_program(), std::ffi::OsStr::new("codex"));
}
