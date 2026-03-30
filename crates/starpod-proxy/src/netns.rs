//! Linux network namespace isolation.
//!
//! Creates a persistent network namespace with a veth pair routed through
//! the proxy. Tool subprocesses enter this namespace via `setns()` in a
//! `pre_exec` hook — all their traffic is forced through the proxy.
//!
//! # Setup
//!
//! ```text
//! ┌──────────────────────────────────┐
//! │  Host namespace                  │
//! │                                  │
//! │  starpod-proxy (127.0.0.1:PORT) │
//! │         ▲                        │
//! │         │ iptables DNAT          │
//! │  ┌──────┴──────┐                 │
//! │  │ veth-host    │ 10.200.1.1/24  │
//! │  └──────┬──────┘                 │
//! │         │ veth pair              │
//! ├─────────┼────────────────────────┤
//! │  ┌──────┴──────┐                 │
//! │  │ veth-child   │ 10.200.1.2/24  │
//! │  └─────────────┘                 │
//! │  default route → 10.200.1.1      │
//! │                                  │
//! │  Child namespace (starpod-ns)    │
//! │  bash, curl, etc. live here      │
//! └──────────────────────────────────┘
//! ```

use std::process::Command;

use tracing::{debug, info, warn};

use starpod_core::{Result, StarpodError};

const NS_NAME: &str = "starpod-ns";
const VETH_HOST: &str = "sp-veth0";
const VETH_CHILD: &str = "sp-veth1";
const HOST_IP: &str = "10.200.1.1";
const CHILD_IP: &str = "10.200.1.2";
const SUBNET: &str = "10.200.1.0/24";

/// Handle to a created network namespace. Cleans up on drop.
pub struct NamespaceHandle {
    /// Path to the namespace (e.g. `/var/run/netns/starpod-ns`).
    pub ns_path: String,
}

impl NamespaceHandle {
    /// Create a pre_exec closure that enters this namespace.
    ///
    /// The returned closure is `Send + Sync + 'static` and can be stored
    /// in `ToolExecutor` to run in the child process after fork.
    pub fn pre_exec_fn(&self) -> Box<dyn Fn() -> std::io::Result<()> + Send + Sync> {
        let ns_path = self.ns_path.clone();
        Box::new(move || {
            use std::os::unix::io::AsRawFd;
            let file = std::fs::File::open(&ns_path).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to open netns {ns_path}: {e}"),
                )
            })?;
            nix::sched::setns(file.as_raw_fd(), nix::sched::CloneFlags::CLONE_NEWNET).map_err(
                |e| std::io::Error::new(std::io::ErrorKind::Other, format!("setns failed: {e}")),
            )?;
            Ok(())
        })
    }
}

impl Drop for NamespaceHandle {
    fn drop(&mut self) {
        debug!("Cleaning up network namespace {NS_NAME}");
        let _ = run_cmd("ip", &["netns", "delete", NS_NAME]);
        // veth pair is automatically removed when the namespace is deleted
    }
}

/// Create a network namespace with a veth pair routing all traffic through the proxy.
///
/// Requires `CAP_NET_ADMIN`. Returns a handle that cleans up on drop.
pub fn create_namespace(proxy_port: u16) -> Result<NamespaceHandle> {
    // Clean up any stale namespace from a previous run
    let _ = run_cmd("ip", &["netns", "delete", NS_NAME]);

    // 1. Create the network namespace
    run_cmd("ip", &["netns", "add", NS_NAME])
        .map_err(|e| StarpodError::Proxy(format!("Failed to create netns: {e}")))?;
    info!("Created network namespace {NS_NAME}");

    // 2. Create veth pair
    run_cmd(
        "ip",
        &[
            "link", "add", VETH_HOST, "type", "veth", "peer", "name", VETH_CHILD,
        ],
    )
    .map_err(|e| StarpodError::Proxy(format!("Failed to create veth pair: {e}")))?;

    // 3. Move child end into the namespace
    run_cmd("ip", &["link", "set", VETH_CHILD, "netns", NS_NAME])
        .map_err(|e| StarpodError::Proxy(format!("Failed to move veth to netns: {e}")))?;

    // 4. Configure host-side veth
    run_cmd(
        "ip",
        &["addr", "add", &format!("{HOST_IP}/24"), "dev", VETH_HOST],
    )
    .map_err(|e| StarpodError::Proxy(format!("Failed to configure host veth: {e}")))?;
    run_cmd("ip", &["link", "set", VETH_HOST, "up"])
        .map_err(|e| StarpodError::Proxy(format!("Failed to bring up host veth: {e}")))?;

    // 5. Configure child-side veth (inside the namespace)
    run_ns_cmd(&format!("{CHILD_IP}/24"), "addr", &["add"], VETH_CHILD)
        .map_err(|e| StarpodError::Proxy(format!("Failed to configure child veth: {e}")))?;
    run_cmd(
        "ip",
        &[
            "netns", "exec", NS_NAME, "ip", "link", "set", VETH_CHILD, "up",
        ],
    )
    .map_err(|e| StarpodError::Proxy(format!("Failed to bring up child veth: {e}")))?;
    run_cmd(
        "ip",
        &["netns", "exec", NS_NAME, "ip", "link", "set", "lo", "up"],
    )
    .map_err(|e| StarpodError::Proxy(format!("Failed to bring up child loopback: {e}")))?;

    // 6. Set default route inside namespace → through host veth
    run_cmd(
        "ip",
        &[
            "netns", "exec", NS_NAME, "ip", "route", "add", "default", "via", HOST_IP,
        ],
    )
    .map_err(|e| StarpodError::Proxy(format!("Failed to set default route: {e}")))?;

    // 7. Enable IP forwarding on the host
    let _ = std::fs::write("/proc/sys/net/ipv4/ip_forward", "1");

    // 8. NAT: masquerade traffic from the namespace
    run_cmd(
        "iptables",
        &[
            "-t",
            "nat",
            "-A",
            "POSTROUTING",
            "-s",
            SUBNET,
            "-j",
            "MASQUERADE",
        ],
    )
    .map_err(|e| StarpodError::Proxy(format!("Failed to set up NAT: {e}")))?;

    // 9. DNAT: redirect all traffic from namespace destined for port 80/443
    //    to the proxy on the host
    let proxy_dest = format!("{HOST_IP}:{proxy_port}");
    for port in &["80", "443"] {
        run_cmd(
            "iptables",
            &[
                "-t",
                "nat",
                "-A",
                "PREROUTING",
                "-s",
                SUBNET,
                "-p",
                "tcp",
                "--dport",
                port,
                "-j",
                "DNAT",
                "--to-destination",
                &proxy_dest,
            ],
        )
        .map_err(|e| StarpodError::Proxy(format!("Failed to set up DNAT for port {port}: {e}")))?;
    }

    let ns_path = format!("/var/run/netns/{NS_NAME}");
    info!(
        ns_path = %ns_path,
        proxy_port = proxy_port,
        "Network namespace isolation active"
    );

    Ok(NamespaceHandle { ns_path })
}

/// Run a command, returning Ok(()) on success or Err with stderr.
fn run_cmd(program: &str, args: &[&str]) -> std::result::Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "{program} {} exited {}: {}",
            args.join(" "),
            output.status,
            stderr.trim()
        ))
    }
}

/// Run `ip netns exec {NS_NAME} ip addr add {addr} dev {dev}` style commands.
fn run_ns_cmd(
    addr: &str,
    subcmd: &str,
    args: &[&str],
    dev: &str,
) -> std::result::Result<(), String> {
    let mut cmd_args = vec!["netns", "exec", NS_NAME, "ip", subcmd];
    cmd_args.extend_from_slice(args);
    cmd_args.push(addr);
    cmd_args.push("dev");
    cmd_args.push(dev);
    run_cmd("ip", &cmd_args)
}
