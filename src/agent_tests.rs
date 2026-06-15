use crate::agent::select_agent;
use crate::agent::Agent;
use url::Url;

// A minimal Agent implementation used only to lock the trait shape.
struct FakeAgent;

impl Agent for FakeAgent {
    fn name(&self) -> &str {
        "fake"
    }

    fn build_command(
        &self,
        argv: &[String],
        base_url: &Url,
        extra_env: &[(String, String)],
    ) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new("fake-binary");
        cmd.arg(base_url.as_str());
        for a in argv {
            cmd.arg(a);
        }
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        cmd
    }
}

#[test]
fn agent_is_object_safe_and_name_works() {
    let agent: Box<dyn Agent> = Box::new(FakeAgent);
    assert_eq!(agent.name(), "fake");
}

#[test]
fn build_command_through_trait_object_carries_inputs() {
    let agent: &dyn Agent = &FakeAgent;
    let base = Url::parse("http://127.0.0.1:4100").unwrap();
    let argv = vec!["--print".to_string(), "hello".to_string()];
    let env = vec![("FOO".to_string(), "bar".to_string())];

    let cmd = agent.build_command(&argv, &base, &env);
    let std_cmd = cmd.as_std();

    assert_eq!(std_cmd.get_program(), "fake-binary");

    let args: Vec<String> = std_cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
    assert_eq!(
        args,
        vec![
            "http://127.0.0.1:4100/".to_string(),
            "--print".to_string(),
            "hello".to_string(),
        ]
    );

    let foo = std_cmd
        .get_envs()
        .find(|(k, _)| *k == std::ffi::OsStr::new("FOO"))
        .map(|(_, v)| v.unwrap().to_string_lossy().into_owned());
    assert_eq!(foo, Some("bar".to_string()));
}

#[test]
fn select_agent_picks_codex_by_basename() {
    assert_eq!(select_agent(&["codex".to_string()]).name(), "codex");
    assert_eq!(select_agent(&["/usr/local/bin/codex".to_string()]).name(), "codex");
    assert_eq!(select_agent(&["codex.exe".to_string()]).name(), "codex");
    assert_eq!(select_agent(&[r"C:\tools\codex.EXE".to_string()]).name(), "codex");
}

#[test]
fn select_agent_defaults_to_claude() {
    assert_eq!(select_agent(&["claude".to_string()]).name(), "claude");
    assert_eq!(select_agent(&["/opt/claude".to_string()]).name(), "claude");
    assert_eq!(select_agent(&["something-else".to_string()]).name(), "claude");
    assert_eq!(select_agent(&[]).name(), "claude");
}
