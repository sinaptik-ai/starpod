use std::env;
use std::path::Path;
use std::process::Command;

fn main() {
    // Rerun when web sources change OR when dist is missing
    println!("cargo:rerun-if-changed=../../web/src");
    println!("cargo:rerun-if-changed=../../web/index.html");
    println!("cargo:rerun-if-changed=../../web/vite.config.js");
    println!("cargo:rerun-if-changed=../../web/package.json");
    println!("cargo:rerun-if-changed=static/dist/index.html");

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let web_dir = Path::new(&manifest_dir).join("../../web");
    let dist_dir = Path::new(&manifest_dir).join("static/dist");

    // If dist already has index.html, skip (allows CI to pre-build)
    if dist_dir.join("index.html").exists() {
        return;
    }

    // Check if npm is available
    let npm = if cfg!(target_os = "windows") {
        "npm.cmd"
    } else {
        "npm"
    };

    if Command::new(npm).arg("--version").output().is_err() {
        eprintln!(
            "warning: npm not found — web UI will not be included. \
             Install Node.js and run `npm run build` in web/ manually."
        );
        // Create a minimal placeholder so rust-embed doesn't fail
        std::fs::create_dir_all(&dist_dir).ok();
        std::fs::write(
            dist_dir.join("index.html"),
            "<html><body><h1>Web UI not built</h1><p>Run <code>npm run build</code> in the <code>web/</code> directory.</p></body></html>",
        ).ok();
        return;
    }

    // Install dependencies
    let status = Command::new(npm)
        .arg("ci")
        .arg("--ignore-scripts")
        .current_dir(&web_dir)
        .status();
    // Fall back to npm install if npm ci fails (no lockfile)
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
