use thiserror::Error;

/// Errors that can occur during hook operations.
#[derive(Error, Debug)]
pub enum HookError {
    /// Invalid regex pattern in a hook matcher.
    #[error("Invalid hook matcher regex: {0}")]
    InvalidRegex(#[from] regex::Error),

    /// Hook callback returned an error.
    #[error("Hook callback failed: {0}")]
    CallbackFailed(String),

    /// Hook execution timed out.
    #[error("Hook timed out after {0}s")]
    Timeout(u64),

    /// Serialization/deserialization error.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Circuit breaker is open — hook is temporarily disabled.
    #[error("Circuit breaker open for hook '{0}'")]
    CircuitBreakerOpen(String),

    /// Hook eligibility check failed.
    #[error("Eligibility check failed: {0}")]
    Eligibility(String),

    /// Hook discovery error.
    #[error("Hook discovery error: {0}")]
    Discovery(String),

    /// Failed to parse a hook manifest file.
    #[error("Failed to parse hook manifest at {path}: {reason}")]
    ManifestParse { path: String, reason: String },

    /// Hook command execution failed.
    #[error("Hook command '{hook_name}' failed: {reason}")]
    CommandExecution { hook_name: String, reason: String },

    /// IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, HookError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_invalid_regex() {
        let err = HookError::InvalidRegex(regex::Regex::new("[invalid").unwrap_err());
        let msg = err.to_string();
        assert!(msg.contains("Invalid hook matcher regex"), "got: {}", msg);
    }

    #[test]
    fn display_callback_failed() {
        let err = HookError::CallbackFailed("connection reset".into());
        assert_eq!(err.to_string(), "Hook callback failed: connection reset");
    }

    #[test]
    fn display_timeout() {
        let err = HookError::Timeout(30);
        assert_eq!(err.to_string(), "Hook timed out after 30s");
    }

    #[test]
    fn from_regex_error() {
        let regex_err = regex::Regex::new("[bad").unwrap_err();
        let hook_err: HookError = regex_err.into();
        assert!(matches!(hook_err, HookError::InvalidRegex(_)));
    }

    #[test]
    fn from_serde_error() {
        let serde_err = serde_json::from_str::<String>("not json").unwrap_err();
        let hook_err: HookError = serde_err.into();
        assert!(matches!(hook_err, HookError::Serialization(_)));
    }

    #[test]
    fn display_circuit_breaker_open() {
        let err = HookError::CircuitBreakerOpen("my-hook".into());
        assert_eq!(err.to_string(), "Circuit breaker open for hook 'my-hook'");
    }

    #[test]
    fn display_eligibility() {
        let err = HookError::Eligibility("missing binary: eslint".into());
        assert_eq!(
            err.to_string(),
            "Eligibility check failed: missing binary: eslint"
        );
    }

    #[test]
    fn display_discovery() {
        let err = HookError::Discovery("bad glob".into());
        assert_eq!(err.to_string(), "Hook discovery error: bad glob");
    }

    #[test]
    fn display_manifest_parse() {
        let err = HookError::ManifestParse {
            path: "/hooks/bad/HOOK.md".into(),
            reason: "invalid toml".into(),
        };
        assert_eq!(
            err.to_string(),
            "Failed to parse hook manifest at /hooks/bad/HOOK.md: invalid toml"
        );
    }

    #[test]
    fn display_command_execution() {
        let err = HookError::CommandExecution {
            hook_name: "lint".into(),
            reason: "exit code 1".into(),
        };
        assert_eq!(err.to_string(), "Hook command 'lint' failed: exit code 1");
    }

    #[test]
    fn from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let hook_err: HookError = io_err.into();
        assert!(matches!(hook_err, HookError::Io(_)));
        assert!(hook_err.to_string().contains("not found"));
    }
}
