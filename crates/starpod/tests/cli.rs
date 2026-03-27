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

// ── init command ────────────────────────────────────────────────────────

#[test]
fn init_creates_starpod_directory() {
    let tmp = TempDir::new().unwrap();

    starpod()
        .current_dir(tmp.path())
        .args(["init"])
        .assert()
        .success();

    let sp = tmp.path().join(".starpod");
    let cfg = sp.join("config");
    assert!(cfg.join("agent.toml").is_file(), ".starpod/config/agent.toml should exist");
    assert!(cfg.join("SOUL.md").is_file(), ".starpod/config/SOUL.md should exist");
    assert!(cfg.join("frontend.toml").is_file(), ".starpod/config/frontend.toml should exist");
    assert!(cfg.join("HEARTBEAT.md").is_file(), "HEARTBEAT.md should exist");
    assert!(cfg.join("BOOT.md").is_file(), "BOOT.md should exist");
    assert!(cfg.join("BOOTSTRAP.md").is_file(), "BOOTSTRAP.md should exist");
    assert!(sp.join("db").is_dir(), ".starpod/db/ should exist");
    assert!(sp.join("skills").is_dir(), ".starpod/skills/ should exist");
    assert!(sp.join("users").is_dir(), ".starpod/users/ should exist");
    assert!(tmp.path().join("home").is_dir(), "home/ should exist");
}

#[test]
fn init_with_name_flag() {
    let tmp = TempDir::new().unwrap();

    starpod()
        .current_dir(tmp.path())
        .args(["init", "--name", "Jarvis"])
        .assert()
        .success();

    let agent_toml = fs::read_to_string(
        tmp.path().join(".starpod/config/agent.toml"),
    ).unwrap();
    assert!(
        agent_toml.contains(r#"agent_name = "Jarvis""#),
        "agent_name should be Jarvis, got:\n{agent_toml}"
    );

    let soul = fs::read_to_string(
        tmp.path().join(".starpod/config/SOUL.md"),
    ).unwrap();
    assert!(
        soul.contains("You are Jarvis"),
        "SOUL.md should reference Jarvis, got:\n{soul}"
    );
}

#[test]
fn init_with_model_flag() {
    let tmp = TempDir::new().unwrap();

    starpod()
        .current_dir(tmp.path())
        .args(["init", "--model", "openai/gpt-4o"])
        .assert()
        .success();

    let agent_toml = fs::read_to_string(
        tmp.path().join(".starpod/config/agent.toml"),
    ).unwrap();
    assert!(
        agent_toml.contains("openai/gpt-4o"),
        "model should be openai/gpt-4o, got:\n{agent_toml}"
    );
}

#[test]
fn init_default_values() {
    let tmp = TempDir::new().unwrap();

    starpod()
        .current_dir(tmp.path())
        .args(["init"])
        .assert()
        .success();

    let agent_toml = fs::read_to_string(
        tmp.path().join(".starpod/config/agent.toml"),
    ).unwrap();
    assert!(
        agent_toml.contains(r#"agent_name = "Nova""#),
        "default agent_name should be Nova, got:\n{agent_toml}"
    );
    assert!(
        agent_toml.contains("anthropic/claude-haiku-4-5"),
        "default model should be anthropic/claude-haiku-4-5, got:\n{agent_toml}"
    );
}

#[test]
fn init_fails_if_already_initialized() {
    let tmp = TempDir::new().unwrap();

    // First init succeeds
    starpod()
        .current_dir(tmp.path())
        .args(["init"])
        .assert()
        .success();

    // Second init fails
    starpod()
        .current_dir(tmp.path())
        .args(["init"])
        .assert()
        .failure();
}

#[test]
fn init_creates_gitignore() {
    let tmp = TempDir::new().unwrap();

    starpod()
        .current_dir(tmp.path())
        .args(["init"])
        .assert()
        .success();

    let gitignore = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
    assert!(gitignore.contains(".starpod/db/"), ".gitignore should contain .starpod/db/");
    assert!(gitignore.contains("home/"), ".gitignore should contain home/");
}

#[test]
fn init_with_env_flag() {
    let tmp = TempDir::new().unwrap();

    starpod()
        .current_dir(tmp.path())
        .args(["init", "--env", "TEST_KEY=test_value"])
        .assert()
        .success();

    // Vault should exist after seeding env vars
    assert!(
        tmp.path().join(".starpod/db/vault.db").is_file(),
        "vault.db should be created when --env is used"
    );
}

// ── init: combined flags and edge cases ───────────────────────────────

#[test]
fn init_with_name_and_model() {
    let tmp = TempDir::new().unwrap();

    starpod()
        .current_dir(tmp.path())
        .args(["init", "--name", "Ada", "--model", "openai/gpt-4o"])
        .assert()
        .success();

    let agent_toml = fs::read_to_string(
        tmp.path().join(".starpod/config/agent.toml"),
    ).unwrap();
    assert!(
        agent_toml.contains(r#"agent_name = "Ada""#),
        "agent_name should be Ada, got:\n{agent_toml}"
    );
    assert!(
        agent_toml.contains("openai/gpt-4o"),
        "model should be openai/gpt-4o, got:\n{agent_toml}"
    );

    let soul = fs::read_to_string(
        tmp.path().join(".starpod/config/SOUL.md"),
    ).unwrap();
    assert!(
        soul.contains("You are Ada"),
        "SOUL.md should reference Ada, got:\n{soul}"
    );
}

#[test]
fn init_with_multiple_env_flags() {
    let tmp = TempDir::new().unwrap();

    starpod()
        .current_dir(tmp.path())
        .args([
            "init",
            "--env", "KEY_A=val_a",
            "--env", "KEY_B=val_b",
        ])
        .assert()
        .success();

    assert!(
        tmp.path().join(".starpod/db/vault.db").is_file(),
        "vault.db should be created with multiple --env flags"
    );
}

#[test]
fn init_creates_home_subdirectories() {
    let tmp = TempDir::new().unwrap();

    starpod()
        .current_dir(tmp.path())
        .args(["init"])
        .assert()
        .success();

    for dir in &["desktop", "documents", "projects", "downloads"] {
        assert!(
            tmp.path().join("home").join(dir).is_dir(),
            "home/{dir} should exist"
        );
    }
}

#[test]
fn init_appends_to_existing_gitignore() {
    let tmp = TempDir::new().unwrap();

    // Pre-existing .gitignore
    fs::write(tmp.path().join(".gitignore"), "node_modules/\n").unwrap();

    starpod()
        .current_dir(tmp.path())
        .args(["init"])
        .assert()
        .success();

    let gitignore = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
    assert!(gitignore.contains("node_modules/"), "should preserve existing entries");
    assert!(gitignore.contains(".starpod/db/"), "should add .starpod/db/");
    assert!(gitignore.contains("home/"), "should add home/");
}

#[test]
fn init_frontend_toml_references_agent_name() {
    let tmp = TempDir::new().unwrap();

    starpod()
        .current_dir(tmp.path())
        .args(["init", "--name", "Jarvis"])
        .assert()
        .success();

    let frontend = fs::read_to_string(
        tmp.path().join(".starpod/config/frontend.toml"),
    ).unwrap();
    assert!(
        frontend.contains("Jarvis"),
        "frontend.toml should reference the agent name, got:\n{frontend}"
    );
}

#[test]
fn init_lifecycle_files_are_empty() {
    let tmp = TempDir::new().unwrap();

    starpod()
        .current_dir(tmp.path())
        .args(["init"])
        .assert()
        .success();

    let cfg = tmp.path().join(".starpod/config");
    for file in &["HEARTBEAT.md", "BOOT.md", "BOOTSTRAP.md"] {
        let content = fs::read_to_string(cfg.join(file)).unwrap();
        assert!(content.is_empty(), "{file} should be empty, got: {content:?}");
    }
}

// ── deploy stub ───────────────────────────────────────────────────────

#[test]
fn deploy_prints_coming_soon() {
    let output = starpod()
        .args(["deploy"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("coming soon") || stdout.contains("Coming soon"),
        "deploy should print coming soon, got:\n{stdout}"
    );
}

// ── subcommand help text ──────────────────────────────────────────────

#[test]
fn dev_help_shows_port_flag() {
    let output = starpod()
        .args(["dev", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("--port"),
        "dev --help should list --port flag, got:\n{stdout}"
    );
}

#[test]
fn chat_help_shows_message_arg() {
    let output = starpod()
        .args(["chat", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("message") || stdout.contains("MESSAGE"),
        "chat --help should describe the message argument, got:\n{stdout}"
    );
}

#[test]
fn init_help_shows_all_flags() {
    let output = starpod()
        .args(["init", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    for flag in &["--name", "--model", "--env"] {
        assert!(
            stdout.contains(flag),
            "init --help should list '{flag}' flag, got:\n{stdout}"
        );
    }
}

// ── --format json global flag ──────────────────────────────────────────

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

// ── auth commands ──────────────────────────────────────────────────────

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

#[test]
fn auth_status_produces_json() {
    let output = starpod_json()
        .args(["auth", "status"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("auth status --format json should produce valid JSON, got: {stdout}\nerror: {e}");
    });
    // Should have a logged_in field (value depends on environment)
    assert!(
        parsed.get("logged_in").is_some(),
        "auth status JSON should contain logged_in field, got: {parsed}"
    );
}

// ── help output ────────────────────────────────────────────────────────

#[test]
fn help_shows_all_commands() {
    let output = starpod()
        .args(["--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    for cmd in &["init", "dev", "serve", "deploy", "repl", "chat", "auth"] {
        assert!(
            stdout.contains(cmd),
            "help should list '{cmd}' command, got:\n{stdout}"
        );
    }
}

#[test]
fn old_commands_removed() {
    // These commands should no longer exist
    for cmd in &["agent", "instance", "secret", "build", "memory", "sessions", "skill", "cron"] {
        let output = starpod()
            .args([cmd, "--help"])
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "'{cmd}' should no longer be a valid command"
        );
    }
}
