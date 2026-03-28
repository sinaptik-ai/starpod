use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

fn starpod() -> Command {
    Command::cargo_bin("starpod").unwrap()
}

fn starpod_json() -> Command {
    let mut cmd = starpod();
    cmd.args(["--format", "json"]);
    cmd
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
    starpod().args(["skill", "new"]).assert().failure();
}

#[test]
fn skill_new_accepts_description_flag() {
    let output = starpod()
        .args([
            "skill",
            "new",
            "pr-review",
            "--description",
            "Review PRs for security",
        ])
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
        .args([
            "skill",
            "new",
            "pr-review",
            "--prompt",
            "Focus on OWASP top 10",
        ])
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
            "skill",
            "new",
            "pr-review",
            "--description",
            "Review PRs for security",
            "--prompt",
            "Focus on OWASP top 10",
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
            "skill",
            "new",
            "pr-review",
            "-d",
            "Review PRs",
            "-p",
            "Focus on security",
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
    let agent_output = starpod().args(["agent", "new", "--help"]).output().unwrap();
    let agent_help = String::from_utf8_lossy(&agent_output.stdout);

    let skill_output = starpod().args(["skill", "new", "--help"]).output().unwrap();
    let skill_help = String::from_utf8_lossy(&skill_output.stdout);

    // Both should show NAME as a positional argument in usage
    // (required <NAME> or optional [NAME]).
    assert!(
        agent_help.contains("<NAME>")
            || agent_help.contains("<name>")
            || agent_help.contains("[NAME]")
            || agent_help.contains("[name]"),
        "agent new should have a positional NAME arg, got:\n{agent_help}"
    );
    assert!(
        skill_help.contains("<NAME>")
            || skill_help.contains("<name>")
            || skill_help.contains("[NAME]")
            || skill_help.contains("[name]"),
        "skill new should have a positional NAME arg, got:\n{skill_help}"
    );
}

#[test]
fn skill_new_help_mentions_ai() {
    let output = starpod().args(["skill", "new", "--help"]).output().unwrap();
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
    let output = starpod().args(["skill", "list"]).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unrecognized subcommand"),
        "skill list should be a valid subcommand, got: {stderr}"
    );
}

#[test]
fn skill_show_requires_name() {
    starpod().args(["skill", "show"]).assert().failure();
}

#[test]
fn skill_delete_requires_name() {
    starpod().args(["skill", "delete"]).assert().failure();
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
    assert!(
        frontend_path.exists(),
        "frontend.toml should be created by agent new"
    );
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
        .args([
            "agent",
            "new",
            "mybot",
            "--default",
            "--agent-name",
            "Jarvis",
        ])
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
    )
    .unwrap();
    fs::write(agent_dir.join("SOUL.md"), "# Soul\n\nYou are TestBot.\n").unwrap();

    // Provide .env with required secrets so build validation passes
    let env_file = tmp.path().join("test.env");
    fs::write(&env_file, "ANTHROPIC_API_KEY=sk-test-dummy\n").unwrap();

    let output_dir = tmp.path().join("deploy");
    fs::create_dir_all(&output_dir).unwrap();

    starpod()
        .args([
            "build",
            "--agent",
            agent_dir.to_str().unwrap(),
            "--output",
            output_dir.to_str().unwrap(),
            "--env",
            env_file.to_str().unwrap(),
        ])
        .assert()
        .success();

    let sp = output_dir.join(".starpod");
    let cfg = sp.join("config");
    assert!(
        cfg.join("agent.toml").is_file(),
        ".starpod/config/agent.toml should exist"
    );
    assert!(
        cfg.join("SOUL.md").is_file(),
        ".starpod/config/SOUL.md should exist"
    );
    assert!(sp.join("db").is_dir(), ".starpod/db/ should exist");
    assert!(sp.join("users").is_dir(), ".starpod/users/ should exist");
}

#[test]
fn build_fails_without_agent_toml() {
    let tmp = TempDir::new().unwrap();

    let agent_dir = tmp.path().join("bad-agent");
    fs::create_dir_all(&agent_dir).unwrap();
    // No agent.toml

    starpod()
        .args(["build", "--agent", agent_dir.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn build_with_env_file() {
    let tmp = TempDir::new().unwrap();

    let agent_dir = tmp.path().join("my-agent");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.toml"), "agent_name = \"Bot\"\n").unwrap();

    // Include required ANTHROPIC_API_KEY alongside custom vars
    let env_file = tmp.path().join("prod.env");
    fs::write(
        &env_file,
        "ANTHROPIC_API_KEY=sk-test-dummy\nAPI_KEY=secret123\n",
    )
    .unwrap();

    let output_dir = tmp.path().join("deploy");
    fs::create_dir_all(&output_dir).unwrap();

    starpod()
        .args([
            "build",
            "--agent",
            agent_dir.to_str().unwrap(),
            "--output",
            output_dir.to_str().unwrap(),
            "--env",
            env_file.to_str().unwrap(),
        ])
        .assert()
        .success();

    // .env is NOT copied into the instance (secrets go into vault at serve time).
    // Verify the build succeeded by checking the output structure exists.
    assert!(output_dir.join(".starpod/config/agent.toml").is_file());
    assert!(output_dir.join(".starpod/db").is_dir());
}

#[test]
fn build_with_skills() {
    let tmp = TempDir::new().unwrap();

    let agent_dir = tmp.path().join("my-agent");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("agent.toml"), "agent_name = \"Bot\"\n").unwrap();

    // Provide .env with required secrets so build validation passes
    let env_file = tmp.path().join("test.env");
    fs::write(&env_file, "ANTHROPIC_API_KEY=sk-test-dummy\n").unwrap();

    let skills_dir = tmp.path().join("skills");
    fs::create_dir_all(skills_dir.join("greet")).unwrap();
    fs::write(
        skills_dir.join("greet").join("SKILL.md"),
        "---\nname: greet\ndescription: Greet users.\n---\nSay hello.",
    )
    .unwrap();

    let output_dir = tmp.path().join("deploy");
    fs::create_dir_all(&output_dir).unwrap();

    starpod()
        .args([
            "build",
            "--agent",
            agent_dir.to_str().unwrap(),
            "--skills",
            skills_dir.to_str().unwrap(),
            "--output",
            output_dir.to_str().unwrap(),
            "--env",
            env_file.to_str().unwrap(),
        ])
        .assert()
        .success();

    assert!(
        output_dir.join(".starpod/skills/greet/SKILL.md").is_file(),
        "Skills should be copied"
    );
}

// ── P0: Renamed instance commands ─────────────────────────────────────────

#[test]
fn instance_destroy_subcommand_exists() {
    let output = starpod()
        .args(["instance", "destroy", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Permanently destroy") || stdout.contains("destroy"),
        "instance destroy should exist, got:\n{stdout}"
    );
}

#[test]
fn instance_stop_subcommand_exists() {
    let output = starpod()
        .args(["instance", "stop", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Stop") || stdout.contains("stop"),
        "instance stop should exist, got:\n{stdout}"
    );
}

#[test]
fn instance_start_subcommand_exists() {
    let output = starpod()
        .args(["instance", "start", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Start") || stdout.contains("start"),
        "instance start should exist, got:\n{stdout}"
    );
}

#[test]
fn instance_restart_subcommand_exists() {
    let output = starpod()
        .args(["instance", "restart", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Restart") || stdout.contains("restart"),
        "instance restart should exist, got:\n{stdout}"
    );
}

#[test]
fn instance_kill_removed() {
    // `instance kill` should no longer be a valid subcommand.
    let output = starpod()
        .args(["instance", "kill", "abc123"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "instance kill should no longer exist"
    );
    assert!(
        stderr.contains("unrecognized subcommand") || stderr.contains("invalid"),
        "instance kill should be unrecognized, got: {stderr}"
    );
}

#[test]
fn instance_pause_removed() {
    let output = starpod()
        .args(["instance", "pause", "abc123"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "instance pause should no longer exist"
    );
}

#[test]
fn instance_destroy_has_yes_flag() {
    let output = starpod()
        .args(["instance", "destroy", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("--yes"),
        "instance destroy should accept --yes flag, got:\n{stdout}"
    );
}

// ── P0: --agent flag placement (global) ──────────────────────────────────

#[test]
fn skill_agent_flag_after_subcommand() {
    // `starpod skill list --agent mybot` should work (global arg)
    let output = starpod()
        .args(["skill", "list", "--agent", "mybot"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected argument"),
        "skill --agent should work after subcommand, got: {stderr}"
    );
}

#[test]
fn memory_agent_flag_after_subcommand() {
    let output = starpod()
        .args(["memory", "search", "test", "--agent", "mybot"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected argument"),
        "memory --agent should work after subcommand, got: {stderr}"
    );
}

#[test]
fn cron_agent_flag_after_subcommand() {
    let output = starpod()
        .args(["cron", "list", "--agent", "mybot"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected argument"),
        "cron --agent should work after subcommand, got: {stderr}"
    );
}

#[test]
fn sessions_agent_flag_after_subcommand() {
    let output = starpod()
        .args(["sessions", "list", "--agent", "mybot"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected argument"),
        "sessions --agent should work after subcommand, got: {stderr}"
    );
}

// ── P1: --format json global flag ──────────────────────────────────────

#[test]
fn format_json_flag_accepted_globally() {
    let output = starpod()
        .args(["--format", "json", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success(), "--format json should be accepted");
}

#[test]
fn format_invalid_rejected() {
    starpod()
        .args(["--format", "yaml", "--help"])
        .assert()
        .failure();
}

#[test]
fn agent_list_json_output() {
    let ws = workspace();
    // Create an agent so we get non-empty output
    fs::create_dir_all(ws.path().join("agents/test-bot")).unwrap();
    fs::write(
        ws.path().join("agents/test-bot/agent.toml"),
        "agent_name = \"Bot\"\n",
    )
    .unwrap();

    let output = starpod_json()
        .current_dir(ws.path())
        .args(["agent", "list"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should be valid JSON
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("agent list --format json should produce valid JSON, got: {stdout}\nerror: {e}");
    });
    assert!(parsed.is_array(), "Should be a JSON array");
    let arr = parsed.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], "test-bot");
    assert_eq!(arr[0]["has_config"], true);
}

#[test]
fn agent_list_json_empty() {
    let ws = workspace();
    let output = starpod_json()
        .current_dir(ws.path())
        .args(["agent", "list"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("agent list --format json should produce valid JSON for empty list, got: {stdout}\nerror: {e}");
    });
    assert!(parsed.is_array());
    assert!(parsed.as_array().unwrap().is_empty());
}

// ── P1: agent diff subcommand ──────────────────────────────────────────

#[test]
fn agent_diff_subcommand_exists() {
    let output = starpod()
        .args(["agent", "diff", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("diff") || stdout.contains("Diff"),
        "agent diff should be a valid subcommand, got:\n{stdout}"
    );
}

#[test]
fn agent_diff_requires_name() {
    starpod().args(["agent", "diff"]).assert().failure();
}

// ── P1: push --dry-run ────────────────────────────────────────────────

#[test]
fn agent_push_dry_run_flag_exists() {
    let output = starpod()
        .args(["agent", "push", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("--dry-run"),
        "agent push should accept --dry-run flag, got:\n{stdout}"
    );
}

#[test]
fn agent_push_yes_flag_exists() {
    let output = starpod()
        .args(["agent", "push", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("--yes"),
        "agent push should accept --yes flag, got:\n{stdout}"
    );
}

// ── P1: auth --api-key ────────────────────────────────────────────────

#[test]
fn auth_login_api_key_flag_exists() {
    let output = starpod()
        .args(["auth", "login", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("--api-key"),
        "auth login should accept --api-key flag, got:\n{stdout}"
    );
}

#[test]
fn auth_login_email_flag_exists() {
    let output = starpod()
        .args(["auth", "login", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("--email"),
        "auth login should accept --email flag, got:\n{stdout}"
    );
}

// ── P0: secret delete confirmation ────────────────────────────────────

#[test]
fn secret_delete_has_yes_flag() {
    let output = starpod()
        .args(["secret", "delete", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("--yes"),
        "secret delete should accept --yes flag, got:\n{stdout}"
    );
}
