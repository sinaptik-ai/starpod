//! Host allow-list matching for opaque token host binding.

/// Check if `target_host` matches any of the `allowed_hosts`.
///
/// Rules:
/// - Empty `allowed_hosts` = unrestricted (always matches)
/// - Exact match: `"api.github.com"` matches `"api.github.com"`
/// - Glob prefix: `"*.github.com"` matches `"api.github.com"` but NOT `"github.com"`
/// - Case-insensitive
///
/// ```
/// use starpod_proxy::host_match::host_matches;
///
/// assert!(host_matches("api.github.com", &[])); // unrestricted
/// assert!(host_matches("api.github.com", &["api.github.com".into()]));
/// assert!(host_matches("api.github.com", &["*.github.com".into()]));
/// assert!(!host_matches("evil.com", &["api.github.com".into()]));
/// assert!(!host_matches("github.com", &["*.github.com".into()]));
/// ```
pub fn host_matches(target_host: &str, allowed_hosts: &[String]) -> bool {
    if allowed_hosts.is_empty() {
        return true;
    }

    let target = target_host.to_lowercase();

    allowed_hosts.iter().any(|pattern| {
        let pattern = pattern.to_lowercase();
        if let Some(suffix) = pattern.strip_prefix("*.") {
            // Glob: *.github.com matches api.github.com but not github.com
            target.ends_with(&format!(".{suffix}"))
        } else {
            target == pattern
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_hosts_is_unrestricted() {
        assert!(host_matches("anything.com", &[]));
    }

    #[test]
    fn exact_match() {
        assert!(host_matches("api.github.com", &["api.github.com".into()]));
    }

    #[test]
    fn exact_mismatch() {
        assert!(!host_matches("evil.com", &["api.github.com".into()]));
    }

    #[test]
    fn glob_match() {
        assert!(host_matches("api.github.com", &["*.github.com".into()]));
        assert!(host_matches("raw.github.com", &["*.github.com".into()]));
    }

    #[test]
    fn glob_no_match_bare_domain() {
        // *.github.com should NOT match github.com itself
        assert!(!host_matches("github.com", &["*.github.com".into()]));
    }

    #[test]
    fn glob_no_match_different_domain() {
        assert!(!host_matches("evil.com", &["*.github.com".into()]));
    }

    #[test]
    fn case_insensitive() {
        assert!(host_matches("API.GitHub.COM", &["api.github.com".into()]));
        assert!(host_matches("api.github.com", &["*.GitHub.COM".into()]));
    }

    #[test]
    fn multiple_hosts_any_matches() {
        let hosts = vec!["api.github.com".into(), "api.stripe.com".into()];
        assert!(host_matches("api.github.com", &hosts));
        assert!(host_matches("api.stripe.com", &hosts));
        assert!(!host_matches("evil.com", &hosts));
    }

    #[test]
    fn mixed_exact_and_glob() {
        let hosts = vec!["exact.com".into(), "*.wildcard.com".into()];
        assert!(host_matches("exact.com", &hosts));
        assert!(host_matches("sub.wildcard.com", &hosts));
        assert!(!host_matches("other.com", &hosts));
    }

    #[test]
    fn deeply_nested_subdomain_matches_glob() {
        assert!(host_matches("a.b.c.github.com", &["*.github.com".into()]));
    }
}
