//! Isolation tier detection.
//!
//! Automatically selects the strongest network isolation available:
//! - **Tier 1 (NetNamespace)**: Linux + CAP_NET_ADMIN → kernel-enforced isolation
//! - **Tier 0 (EnvProxy)**: All platforms → env var proxy injection

use tracing::info;

/// Network isolation tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationTier {
    /// Tier 0: HTTP_PROXY env vars. Tools _can_ ignore them.
    EnvProxy,
    /// Tier 1: Network namespace. All traffic forced through proxy at kernel level.
    #[cfg(all(target_os = "linux", feature = "netns"))]
    NetNamespace,
}

/// Detect and log the best available isolation tier.
pub fn detect_and_log() -> IsolationTier {
    #[cfg(all(target_os = "linux", feature = "netns"))]
    {
        if has_cap_net_admin() {
            info!("proxy: network namespace isolation (linux/netns)");
            return IsolationTier::NetNamespace;
        }
        info!("proxy: env var mode (netns unavailable: missing CAP_NET_ADMIN)");
        return IsolationTier::EnvProxy;
    }

    #[cfg(not(all(target_os = "linux", feature = "netns")))]
    {
        #[cfg(target_os = "linux")]
        info!("proxy: env var mode (netns feature not compiled)");
        #[cfg(not(target_os = "linux"))]
        info!("proxy: env var mode (not linux)");
        IsolationTier::EnvProxy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_returns_valid_tier() {
        let tier = detect_and_log();
        // On macOS or Linux without CAP_NET_ADMIN, should be EnvProxy
        #[cfg(not(all(target_os = "linux", feature = "netns")))]
        assert_eq!(tier, IsolationTier::EnvProxy);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn has_cap_net_admin_does_not_panic() {
        // Just verify it doesn't panic — actual capability depends on environment
        let _ = has_cap_net_admin();
    }
}

/// Check if the current process has CAP_NET_ADMIN.
///
/// Reads `/proc/self/status` and checks the effective capabilities bitmask.
/// CAP_NET_ADMIN is bit 12 (value 0x1000).
#[cfg(target_os = "linux")]
fn has_cap_net_admin() -> bool {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return false;
    };
    for line in status.lines() {
        if let Some(hex) = line.strip_prefix("CapEff:\t") {
            let Ok(caps) = u64::from_str_radix(hex.trim(), 16) else {
                return false;
            };
            // CAP_NET_ADMIN = bit 12
            return caps & (1 << 12) != 0;
        }
    }
    false
}
