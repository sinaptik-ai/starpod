use std::env;
use std::path::Path;
use std::process::Command;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let npm = if cfg!(target_os = "windows") {
        "npm.cmd"
    } else {
        "npm"
    };

    // Skip web/docs builds when STARPOD_SKIP_WEB_BUILD=1 is set. Used when
    // the pre-built `static/dist` and `docs/.vitepress/dist` directories
    // have been provisioned externally and the Node toolchain isn't
    // necessarily usable (e.g. Node 16 on a worktree).
    if env::var("STARPOD_SKIP_WEB_BUILD").ok().as_deref() == Some("1") {
        println!("cargo:warning=STARPOD_SKIP_WEB_BUILD=1 — skipping npm build");
        return;
    }

    build_web_ui(&manifest_dir, npm);
    build_docs(&manifest_dir, npm);
}

fn build_web_ui(manifest_dir: &str, npm: &str) {
    // Rerun when web sources change OR when dist is missing
    println!("cargo:rerun-if-changed=../../web/src");
    println!("cargo:rerun-if-changed=../../web/index.html");
    println!("cargo:rerun-if-changed=../../web/vite.config.js");
    println!("cargo:rerun-if-changed=../../web/package.json");
    println!("cargo:rerun-if-changed=static/dist/index.html");

    let web_dir = Path::new(manifest_dir).join("../../web");
    let dist_dir = Path::new(manifest_dir).join("static/dist");

    if !web_dir.join("package.json").exists()
        || Command::new(npm).arg("--version").output().is_err()
    {
        eprintln!(
            "warning: web/ directory not found or npm not available — \
             web UI will not be included."
        );
        // Create a minimal placeholder so rust-embed doesn't fail,
        // but only if dist doesn't already have content (e.g. from package)
        if !dist_dir.join("index.html").exists() {
            std::fs::create_dir_all(&dist_dir).ok();
            std::fs::write(
                dist_dir.join("index.html"),
                "<html><body><h1>Web UI not built</h1><p>Run <code>npm run build</code> in the <code>web/</code> directory.</p></body></html>",
            ).ok();
        }
        return;
    }

    // Install dependencies
    let status = Command::new(npm)
        .arg("ci")
        .arg("--ignore-scripts")
        .current_dir(&web_dir)
        .status();
    let install_ok = match status {
        Ok(s) if s.success() => true,
        _ => {
            let s = Command::new(npm)
                .arg("install")
                .current_dir(&web_dir)
                .status()
                .expect("failed to run npm install");
            s.success()
        }
    };
    if !install_ok {
        panic!("npm install failed");
    }

    // Build the web UI
    let status = Command::new(npm)
        .arg("run")
        .arg("build")
        .current_dir(&web_dir)
        .status()
        .expect("failed to run npm run build");
    if !status.success() {
        panic!("npm run build failed");
    }
}

fn build_docs(manifest_dir: &str, npm: &str) {
    // Rerun when doc sources change
    println!("cargo:rerun-if-changed=../../docs/.vitepress/config.mts");
    println!("cargo:rerun-if-changed=../../docs/package.json");
    println!("cargo:rerun-if-changed=../../docs/.vitepress/dist/index.html");
    println!("cargo:rerun-if-changed=../../docs/index.md");
    println!("cargo:rerun-if-changed=../../docs/getting-started");
    println!("cargo:rerun-if-changed=../../docs/concepts");
    println!("cargo:rerun-if-changed=../../docs/crates");

    let docs_dir = Path::new(manifest_dir).join("../../docs");
    let dist_dir = docs_dir.join(".vitepress/dist");

    if !docs_dir.join("package.json").exists()
        || Command::new(npm).arg("--version").output().is_err()
    {
        eprintln!(
            "warning: docs/ directory not found or npm not available — \
             docs will not be included."
        );
        if !dist_dir.join("index.html").exists() {
            std::fs::create_dir_all(&dist_dir).ok();
            std::fs::write(
                dist_dir.join("index.html"),
                "<html><body><h1>Docs not built</h1><p>Run <code>npm run build</code> in the <code>docs/</code> directory.</p></body></html>",
            ).ok();
        }
        return;
    }

    // Install dependencies
    let status = Command::new(npm)
        .arg("ci")
        .arg("--ignore-scripts")
        .current_dir(&docs_dir)
        .status();
    let install_ok = match status {
        Ok(s) if s.success() => true,
        _ => {
            let s = Command::new(npm)
                .arg("install")
                .current_dir(&docs_dir)
                .status()
                .expect("failed to run npm install");
            s.success()
        }
    };
    if !install_ok {
        panic!("npm install failed in docs/");
    }

    // Build the docs
    let status = Command::new(npm)
        .arg("run")
        .arg("build")
        .current_dir(&docs_dir)
        .status()
        .expect("failed to run npm run build");
    if !status.success() {
        panic!("npm run build failed in docs/");
    }
}
