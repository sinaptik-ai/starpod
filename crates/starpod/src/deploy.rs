//! `starpod deploy` — package and upload the local `.starpod/` directory
//! to the platform, creating a remote instance directly.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use colored::Colorize;
use dialoguer::Confirm;
use flate2::write::GzEncoder;
use flate2::Compression;

use starpod_core::{detect_mode, load_agent_config, ResolvedPaths};
use starpod_instances::deploy::DeployClient;

use crate::auth;

/// Run the deploy flow: prompt, package, upload, wait for instance.
pub async fn run_deploy(
    name: Option<String>,
    zone: Option<String>,
    machine_type: String,
    skip_prompts: bool,
) -> anyhow::Result<()> {
    // 1. Load credentials
    let creds = auth::load_credentials().ok_or_else(|| {
        anyhow::anyhow!(
            "Not logged in. Run {} first.",
            "starpod auth login".bright_white()
        )
    })?;

    // 2. Detect .starpod/ from CWD
    let mode = detect_mode(None)?;
    let paths = ResolvedPaths::resolve(&mode)?;
    let agent_config = load_agent_config(&paths)?;
    let agent_name = &agent_config.agent_name;

    println!();
    println!(
        "  {} Deploying {} to {}",
        "▸".bright_cyan().bold(),
        agent_name.bright_white(),
        creds.backend_url.dimmed()
    );
    println!();

    // 3. Interactive prompts (or use defaults with --yes)
    let (include_secrets, include_memory, include_home) = if skip_prompts {
        (true, true, false)
    } else {
        let secrets = Confirm::new()
            .with_prompt("  Include secrets from vault?")
            .default(true)
            .interact()?;

        let memory = Confirm::new()
            .with_prompt("  Include memory?")
            .default(true)
            .interact()?;

        let home = Confirm::new()
            .with_prompt("  Include files in home/?")
            .default(false)
            .interact()?;

        (secrets, memory, home)
    };

    println!();

    // 4. Export secrets if requested
    let secrets = if include_secrets {
        export_vault_secrets(&paths).await?
    } else {
        None
    };

    // 5. Build tarball
    print!("  {} Packaging...", "▸".bright_cyan().bold());
    std::io::stdout().flush()?;

    let tarball = build_tarball(&paths, include_secrets, include_memory, include_home)?;

    println!(
        " {} ({:.1} KB)",
        "done".green(),
        tarball.len() as f64 / 1024.0
    );

    // 6. Upload
    println!("  {} Uploading to platform...", "▸".bright_cyan().bold());

    let client = DeployClient::new(&creds.backend_url, &creds.api_key)?;
    let deploy_resp = client
        .deploy_tarball(
            tarball,
            secrets,
            name.or_else(|| Some(agent_name.clone())),
            zone,
            Some(machine_type),
        )
        .await?;

    let instance_id = &deploy_resp.instance.id;
    println!(
        "  {} Instance {} created",
        "✓".green().bold(),
        &instance_id[..8].bright_white()
    );

    // 7. Poll until ready
    println!("  {} Provisioning...", "▸".bright_cyan().bold());

    let mut last_status = String::new();
    let result = client
        .wait_for_instance_ready(instance_id, Duration::from_secs(600), |inst| {
            if inst.status != last_status {
                last_status = inst.status.clone();
                print!(
                    "\r  {} Status: {}          ",
                    "…".dimmed(),
                    inst.status.bright_white()
                );
                let _ = std::io::stdout().flush();
            }
        })
        .await;

    println!(); // clear the status line

    match result {
        Ok(inst) => {
            println!();
            println!(
                "  {} Instance is {}!",
                "✓".green().bold(),
                "running".green().bold()
            );
            if let Some(url) = &inst.web_url {
                println!("  {} {}", "→".dimmed(), url.bright_white());
            }
            if let Some(url) = &inst.direct_url {
                println!("  {} {} (with auth)", "→".dimmed(), url.bright_white());
            }
            println!();
        }
        Err(e) => {
            println!();
            println!("  {} Deploy failed: {}", "✗".red().bold(), e);
            println!();
            return Err(e.into());
        }
    }

    Ok(())
}

/// Export all secrets from the local vault as a HashMap.
async fn export_vault_secrets(
    paths: &ResolvedPaths,
) -> anyhow::Result<Option<HashMap<String, String>>> {
    let vault_path = paths.db_dir.join("vault.db");
    if !vault_path.exists() {
        return Ok(None);
    }

    let master_key = starpod_vault::derive_master_key(&paths.db_dir)?;
    let vault = starpod_vault::Vault::new(&vault_path, &master_key).await?;

    let keys = vault.list_keys().await?;
    if keys.is_empty() {
        return Ok(None);
    }

    let mut map = HashMap::new();
    for key in keys {
        if let Some(val) = vault.get(&key, None).await? {
            map.insert(key, val);
        }
    }

    if map.is_empty() {
        Ok(None)
    } else {
        Ok(Some(map))
    }
}

/// Build a gzipped tarball of the `.starpod/` directory, respecting exclusions.
fn build_tarball(
    paths: &ResolvedPaths,
    include_secrets: bool,
    include_memory: bool,
    include_home: bool,
) -> anyhow::Result<Vec<u8>> {
    let mut compressed = Vec::new();
    let encoder = GzEncoder::new(&mut compressed, Compression::fast());
    let mut archive = tar::Builder::new(encoder);

    let starpod_dir = &paths.agent_home; // .starpod/
    let project_root = &paths.project_root;

    // Add .starpod/ contents
    add_directory_to_tar(&mut archive, starpod_dir, ".starpod", &|rel_path: &str| {
        // Always exclude core.db (instance creates its own)
        if rel_path == ".starpod/db/core.db"
            || rel_path == ".starpod/db/core.db-wal"
            || rel_path == ".starpod/db/core.db-shm"
        {
            return false;
        }

        // Exclude secrets if not requested
        if !include_secrets
            && (rel_path == ".starpod/db/vault.db"
                || rel_path == ".starpod/db/vault.db-wal"
                || rel_path == ".starpod/db/vault.db-shm"
                || rel_path == ".starpod/db/.vault_key")
        {
            return false;
        }

        // Exclude memory if not requested
        if !include_memory {
            if rel_path == ".starpod/db/memory.db"
                || rel_path == ".starpod/db/memory.db-wal"
                || rel_path == ".starpod/db/memory.db-shm"
            {
                return false;
            }
            // users/*/MEMORY.md and users/*/memory/
            if rel_path.starts_with(".starpod/users/") {
                let after_users = &rel_path[".starpod/users/".len()..];
                if let Some(after_id) = after_users.split('/').nth(1) {
                    if after_id == "MEMORY.md"
                        || after_id == "memory"
                        || after_id.starts_with("memory/")
                    {
                        return false;
                    }
                }
            }
        }

        true
    })?;

    // Add home/ contents if requested
    if include_home {
        let home_dir = project_root.join("home");
        if home_dir.exists() {
            add_directory_to_tar(&mut archive, &home_dir, "home", &|_| true)?;
        }
    }

    archive.finish()?;
    drop(archive);

    Ok(compressed)
}

/// Recursively add a directory to a tar archive.
fn add_directory_to_tar<W: Write>(
    archive: &mut tar::Builder<W>,
    dir: &Path,
    prefix: &str,
    filter: &dyn Fn(&str) -> bool,
) -> anyhow::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    walk_dir_recursive(archive, dir, dir, prefix, filter)
}

fn walk_dir_recursive<W: Write>(
    archive: &mut tar::Builder<W>,
    root: &Path,
    current: &Path,
    prefix: &str,
    filter: &dyn Fn(&str) -> bool,
) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let rel = path.strip_prefix(root)?;
        let archive_path = format!("{}/{}", prefix, rel.display());

        if !filter(&archive_path) {
            continue;
        }

        if path.is_dir() {
            walk_dir_recursive(archive, root, &path, prefix, filter)?;
        } else if path.is_file() {
            let mut file = std::fs::File::open(&path)?;
            let metadata = file.metadata()?;
            let mut header = tar::Header::new_gnu();
            header.set_size(metadata.len());
            header.set_mode(0o644);
            header.set_cksum();
            archive.append_data(&mut header, &archive_path, &mut file)?;
        }
    }
    Ok(())
}
