use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

fn starpod() -> Command {
    Command::cargo_bin("starpod").unwrap()
}

/// Create a temp workspace with a starpod.toml so `agent new` can find it.
fn workspace() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("starpod.toml"), "# workspace\n").unwrap();
    dir
}

#[test]
fn skill_new_subcommand_exists() {
    // `skill new` should be recognized (will fail because no .starpod dir, but
    // the error should NOT be about an unrecognized subcommand).
    let output = starpod()
        .args(["skill", "new", "test-skill", "-d", "A test skill", "-b", "body"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unrecognized subcommand"),
        "skill new should be a valid subcommand, got: {stderr}"
    );
}

#[test]
fn skill_create_subcommand_removed() {
    // `skill create` should no longer be recognized.
    starpod()
        .args(["skill", "create", "test-skill", "-d", "desc", "-b", "body"])
        .assert()
        .failure();
}

#[test]
fn agent_new_uses_name_as_display_name() {
    let ws = workspace();
    starpod()
        .current_dir(ws.path())
        .args(["agent", "new", "mybot", "--default"])
        .assert()
        .success();

    let agent_toml = fs::read_to_string(ws.path().join("agents/mybot/agent.toml")).unwrap();
    assert!(
        agent_toml.contains(r#"agent_name = "mybot""#),
        "agent_name should default to the positional name arg, got:\n{agent_toml}"
    );

    let soul = fs::read_to_string(ws.path().join("agents/mybot/SOUL.md")).unwrap();
    assert!(
        soul.contains("You are mybot"),
        "SOUL.md should use the name as the agent identity, got:\n{soul}"
    );
}

#[test]
fn agent_new_agent_name_flag_overrides() {
    let ws = workspace();
    starpod()
        .current_dir(ws.path())
        .args(["agent", "new", "mybot", "--default", "--agent-name", "Jarvis"])
        .assert()
        .success();

    let agent_toml = fs::read_to_string(ws.path().join("agents/mybot/agent.toml")).unwrap();
    assert!(
        agent_toml.contains(r#"agent_name = "Jarvis""#),
        "--agent-name should override the default, got:\n{agent_toml}"
    );

    let soul = fs::read_to_string(ws.path().join("agents/mybot/SOUL.md")).unwrap();
    assert!(
        soul.contains("You are Jarvis"),
        "SOUL.md should use the --agent-name value, got:\n{soul}"
    );
}
