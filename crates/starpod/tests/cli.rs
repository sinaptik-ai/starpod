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

    // Lifecycle files should be scaffolded
    for name in &["HEARTBEAT.md", "BOOT.md", "BOOTSTRAP.md"] {
        assert!(
            ws.path().join("agents/mybot").join(name).exists(),
            "{name} should be created by agent new"
        );
    }

    // Blueprint should NOT contain a users/ directory (users live in the instance, not the template)
    assert!(
        !ws.path().join("agents/mybot/users").exists(),
        "Blueprint should not scaffold a users/ directory"
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

// ── build command ────────────────────────────────────────────────────────

#[test]
fn build_creates_starpod_directory() {
    let tmp = TempDir::new().unwrap();

    // Create a minimal agent blueprint
    let agent_dir = tmp.path().join("my-agent");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(
        agent_dir.join("agent.toml"),
        "agent_name = \"TestBot\"\nmodel = \"claude-sonnet-4-6\"\n",
    ).unwrap();
    fs::write(
        agent_dir.join("SOUL.md"),
        "# Soul\n\nYou are TestBot.\n",
    ).unwrap();

    let output_dir = tmp.path().join("deploy");
    fs::create_dir_all(&output_dir).unwrap();

    starpod()
        .args([
            "build",
            "--agent", agent_dir.to_str().unwrap(),
            "--output", output_dir.to_str().unwrap(),
        ])
        .assert()
        .success();

    let sp = output_dir.join(".starpod");
    assert!(sp.join("agent.toml").is_file(), ".starpod/agent.toml should exist");
    assert!(sp.join("SOUL.md").is_file(), ".starpod/SOUL.md should exist");
    assert!(sp.join("db").is_dir(), ".starpod/db/ should exist");
    assert!(sp.join("users/admin/USER.md").is_file(), "admin user should be created");
}

#[test]
fn build_fails_without_agent_toml() {
    let tmp = TempDir::new().unwrap();

    let agent_dir = tmp.path().join("bad-agent");
    fs::create_dir_all(&agent_dir).unwrap();
    // No agent.toml

    starpod()
        .args([
            "build",
            "--agent", agent_dir.to_str().unwrap(),
        ])
        .assert()
        .failure();
}

#[test]
fn build_with_env_file() {
    let tmp = TempDir::new().unwrap();

    let agent_dir = tmp.path().join("my-agent");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.toml"), "agent_name = \"Bot\"\n").unwrap();

    let env_file = tmp.path().join("prod.env");
    fs::write(&env_file, "API_KEY=secret123\n").unwrap();

    let output_dir = tmp.path().join("deploy");
    fs::create_dir_all(&output_dir).unwrap();

    starpod()
        .args([
            "build",
            "--agent", agent_dir.to_str().unwrap(),
            "--output", output_dir.to_str().unwrap(),
            "--env", env_file.to_str().unwrap(),
        ])
        .assert()
        .success();

    let env_content = fs::read_to_string(output_dir.join(".starpod/.env")).unwrap();
    assert!(env_content.contains("API_KEY=secret123"));
}

#[test]
fn build_with_skills() {
    let tmp = TempDir::new().unwrap();

    let agent_dir = tmp.path().join("my-agent");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.toml"), "agent_name = \"Bot\"\n").unwrap();

    let skills_dir = tmp.path().join("skills");
    fs::create_dir_all(skills_dir.join("greet")).unwrap();
    fs::write(
        skills_dir.join("greet").join("SKILL.md"),
        "---\nname: greet\ndescription: Greet users.\n---\nSay hello.",
    ).unwrap();

    let output_dir = tmp.path().join("deploy");
    fs::create_dir_all(&output_dir).unwrap();

    starpod()
        .args([
            "build",
            "--agent", agent_dir.to_str().unwrap(),
            "--skills", skills_dir.to_str().unwrap(),
            "--output", output_dir.to_str().unwrap(),
        ])
        .assert()
        .success();

    assert!(
        output_dir.join(".starpod/skills/greet/SKILL.md").is_file(),
        "Skills should be copied"
    );
}
