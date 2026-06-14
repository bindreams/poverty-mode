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
    assert_eq!(std.get_program(), std::ffi::OsStr::new("/usr/bin/claude"));
    let args: Vec<_> = std.get_args().collect();
    assert_eq!(
        args,
        vec![std::ffi::OsStr::new("--print"), std::ffi::OsStr::new("hi")]
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
