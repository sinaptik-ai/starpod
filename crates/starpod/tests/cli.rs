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

// ── skill new command ─────────────────────────────────────────────────────

#[test]
fn skill_new_subcommand_exists() {
    // `skill new` should be recognized (will fail because no API key / no .starpod dir,
    // but the error should NOT be about an unrecognized subcommand).
    let output = starpod()
        .args(["skill", "new", "pr-review"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unrecognized subcommand"),
        "skill new should be a valid subcommand, got: {stderr}"
    );
}

#[test]
fn skill_new_requires_name() {
    // `skill new` without a name should fail with a usage error.
    starpod()
        .args(["skill", "new"])
        .assert()
        .failure();
}

#[test]
fn skill_new_accepts_description_flag() {
    let output = starpod()
        .args(["skill", "new", "pr-review", "--description", "Review PRs for security"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected argument"),
        "skill new should accept --description, got: {stderr}"
    );
}

#[test]
fn skill_new_accepts_prompt_flag() {
    let output = starpod()
        .args(["skill", "new", "pr-review", "--prompt", "Focus on OWASP top 10"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected argument"),
        "skill new should accept --prompt, got: {stderr}"
    );
}

#[test]
fn skill_new_accepts_description_and_prompt_flags() {
    let output = starpod()
        .args([
            "skill", "new", "pr-review",
            "--description", "Review PRs for security",
            "--prompt", "Focus on OWASP top 10",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unrecognized subcommand") && !stderr.contains("unexpected argument"),
        "skill new should accept --description and --prompt flags, got: {stderr}"
    );
}

#[test]
fn skill_new_accepts_short_flags() {
    let output = starpod()
        .args([
            "skill", "new", "pr-review",
            "-d", "Review PRs",
            "-p", "Focus on security",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected argument"),
        "skill new should accept -d and -p short flags, got: {stderr}"
    );
}

#[test]
fn skill_new_rejects_old_body_flag() {
    // The old --body flag should no longer be accepted.
    starpod()
        .args(["skill", "new", "pr-review", "--body", "some body"])
        .assert()
        .failure();
}

#[test]
fn skill_new_rejects_old_file_flag() {
    // The old --file flag should no longer be accepted.
    starpod()
        .args(["skill", "new", "pr-review", "--file", "some-file.md"])
        .assert()
        .failure();
}

#[test]
fn skill_create_subcommand_removed() {
    // `skill create` should not exist as a subcommand.
    let output = starpod()
        .args(["skill", "create", "test-skill"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unrecognized subcommand") || !output.status.success(),
        "skill create should not be a valid subcommand, got: {stderr}"
    );
}

#[test]
fn skill_new_mirrors_agent_new_pattern() {
    // Both `agent new` and `skill new` take a positional name argument.
    // This test verifies they follow the same CLI pattern.
    let agent_output = starpod()
        .args(["agent", "new", "--help"])
        .output()
        .unwrap();
    let agent_help = String::from_utf8_lossy(&agent_output.stdout);

    let skill_output = starpod()
        .args(["skill", "new", "--help"])
        .output()
        .unwrap();
    let skill_help = String::from_utf8_lossy(&skill_output.stdout);

    // Both should show NAME as a positional argument in usage
    // (required <NAME> or optional [NAME]).
    assert!(
        agent_help.contains("<NAME>") || agent_help.contains("<name>")
            || agent_help.contains("[NAME]") || agent_help.contains("[name]"),
        "agent new should have a positional NAME arg, got:\n{agent_help}"
    );
    assert!(
        skill_help.contains("<NAME>") || skill_help.contains("<name>")
            || skill_help.contains("[NAME]") || skill_help.contains("[name]"),
        "skill new should have a positional NAME arg, got:\n{skill_help}"
    );
}

#[test]
fn skill_new_help_mentions_ai() {
    let output = starpod()
        .args(["skill", "new", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("AI") || stdout.contains("generat"),
        "skill new help should mention AI generation, got:\n{stdout}"
    );
}

#[test]
fn skill_parent_accepts_agent_flag() {
    // `starpod skill --agent mybot list` should be recognized.
    let output = starpod()
        .args(["skill", "--agent", "mybot", "list"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected argument"),
        "skill should accept --agent flag, got: {stderr}"
    );
}

// ── skill list / show / delete ────────────────────────────────────────────

#[test]
fn skill_list_subcommand_exists() {
    let output = starpod()
        .args(["skill", "list"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unrecognized subcommand"),
        "skill list should be a valid subcommand, got: {stderr}"
    );
}

#[test]
fn skill_show_requires_name() {
    starpod()
        .args(["skill", "show"])
        .assert()
        .failure();
}

#[test]
fn skill_delete_requires_name() {
    starpod()
        .args(["skill", "delete"])
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

    // frontend.toml should be scaffolded with defaults
    let frontend_path = ws.path().join("agents/mybot/frontend.toml");
    assert!(frontend_path.exists(), "frontend.toml should be created by agent new");
    let frontend_content = fs::read_to_string(&frontend_path).unwrap();
    assert!(
        frontend_content.contains("prompts"),
        "frontend.toml should contain default prompts"
    );

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
        "agent_name = \"TestBot\"\nmodel = \"claude-haiku-4-5\"\n",
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
    let cfg = sp.join("config");
    assert!(cfg.join("agent.toml").is_file(), ".starpod/config/agent.toml should exist");
    assert!(cfg.join("SOUL.md").is_file(), ".starpod/config/SOUL.md should exist");
    assert!(sp.join("db").is_dir(), ".starpod/db/ should exist");
    assert!(sp.join("users/admin/USER.md").is_file(), "admin user should be created");
    assert!(sp.join("users/user/USER.md").is_file(), "default user should be created");
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
