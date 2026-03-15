//! Eligibility requirements for hooks — binary, env var, and OS checks.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Requirements that must be satisfied for a hook to be eligible to run.
///
/// All conditions within a field are ANDed: every `bins` entry must exist,
/// every `env` var must be set, and the current OS must be in `os`.
/// The `any_bins` field is an OR: at least one must exist.
///
/// Empty vectors mean "no requirement" for that field.
///
/// # Example
///
/// ```
/// use starpod_hooks::HookRequirements;
///
/// let req = HookRequirements {
///     bins: vec!["sh".into()],
///     os: vec!["macos".into(), "linux".into()],
///     ..Default::default()
/// };
///
/// // On macOS/Linux with sh available, this passes:
/// // assert!(req.check().is_ok());
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HookRequirements {
    /// All listed binaries must exist on PATH.
    #[serde(default)]
    pub bins: Vec<String>,

    /// At least one of these binaries must exist on PATH.
    #[serde(default)]
    pub any_bins: Vec<String>,

    /// All listed environment variables must be set (non-empty).
    #[serde(default)]
    pub env: Vec<String>,

    /// Current OS must be one of these. Values: "macos", "linux", "windows".
    #[serde(default)]
    pub os: Vec<String>,
}

/// Error returned when an eligibility check fails.
#[derive(Debug)]
pub enum EligibilityError {
    /// A required binary was not found on PATH.
    MissingBinary(String),
    /// None of the alternative binaries were found on PATH.
    NoMatchingBinary(Vec<String>),
    /// A required environment variable is not set.
    MissingEnvVar(String),
    /// The current OS is not in the required list.
    UnsupportedOs {
        current: String,
        required: Vec<String>,
    },
}

impl fmt::Display for EligibilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingBinary(bin) => write!(f, "required binary not found: {}", bin),
            Self::NoMatchingBinary(bins) => {
                write!(f, "none of the required binaries found: {}", bins.join(", "))
            }
            Self::MissingEnvVar(var) => write!(f, "required env var not set: {}", var),
            Self::UnsupportedOs { current, required } => {
                write!(
                    f,
                    "unsupported OS '{}', required one of: {}",
                    current,
                    required.join(", ")
                )
            }
        }
    }
}

impl std::error::Error for EligibilityError {}

impl HookRequirements {
    /// Check all requirements against the current environment.
    ///
    /// Returns `Ok(())` if all requirements are met, or the first
    /// `EligibilityError` encountered.
    pub fn check(&self) -> Result<(), EligibilityError> {
        // Check required binaries (all must exist)
        for bin in &self.bins {
            if which::which(bin).is_err() {
                return Err(EligibilityError::MissingBinary(bin.clone()));
            }
        }

        // Check alternative binaries (at least one must exist)
        if !self.any_bins.is_empty() {
            let found = self.any_bins.iter().any(|bin| which::which(bin).is_ok());
            if !found {
                return Err(EligibilityError::NoMatchingBinary(self.any_bins.clone()));
            }
        }

        // Check environment variables
        for var in &self.env {
            if std::env::var(var).is_err() {
                return Err(EligibilityError::MissingEnvVar(var.clone()));
            }
        }

        // Check OS
        if !self.os.is_empty() {
            let current = current_os();
            if !self.os.iter().any(|o| o == &current) {
                return Err(EligibilityError::UnsupportedOs {
                    current,
                    required: self.os.clone(),
                });
            }
        }

        Ok(())
    }
}

/// Map `std::env::consts::OS` to our canonical names.
fn current_os() -> String {
    match std::env::consts::OS {
        "macos" => "macos".to_string(),
        "linux" => "linux".to_string(),
        "windows" => "windows".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_requirements_always_pass() {
        let req = HookRequirements::default();
        assert!(req.check().is_ok());
    }

    #[test]
    fn missing_binary_fails() {
        let req = HookRequirements {
            bins: vec!["__nonexistent_binary_xyz__".to_string()],
            ..Default::default()
        };
        let err = req.check().unwrap_err();
        assert!(matches!(err, EligibilityError::MissingBinary(_)));
        assert!(err.to_string().contains("__nonexistent_binary_xyz__"));
    }

    #[test]
    fn existing_binary_passes() {
        // `sh` should exist on all unix systems
        #[cfg(unix)]
        {
            let req = HookRequirements {
                bins: vec!["sh".to_string()],
                ..Default::default()
            };
            assert!(req.check().is_ok());
        }
    }

    #[test]
    fn any_bins_at_least_one_must_exist() {
        let req = HookRequirements {
            any_bins: vec![
                "__nonexistent_a__".to_string(),
                "__nonexistent_b__".to_string(),
            ],
            ..Default::default()
        };
        let err = req.check().unwrap_err();
        assert!(matches!(err, EligibilityError::NoMatchingBinary(_)));
    }

    #[cfg(unix)]
    #[test]
    fn any_bins_passes_if_one_exists() {
        let req = HookRequirements {
            any_bins: vec!["__nonexistent__".to_string(), "sh".to_string()],
            ..Default::default()
        };
        assert!(req.check().is_ok());
    }

    #[test]
    fn missing_env_var_fails() {
        let req = HookRequirements {
            env: vec!["__STARPOD_TEST_NONEXISTENT_VAR__".to_string()],
            ..Default::default()
        };
        let err = req.check().unwrap_err();
        assert!(matches!(err, EligibilityError::MissingEnvVar(_)));
    }

    #[test]
    fn existing_env_var_passes() {
        std::env::set_var("__STARPOD_TEST_ELIG_VAR__", "1");
        let req = HookRequirements {
            env: vec!["__STARPOD_TEST_ELIG_VAR__".to_string()],
            ..Default::default()
        };
        assert!(req.check().is_ok());
        std::env::remove_var("__STARPOD_TEST_ELIG_VAR__");
    }

    #[test]
    fn os_mismatch_fails() {
        let req = HookRequirements {
            os: vec!["__fakeos__".to_string()],
            ..Default::default()
        };
        let err = req.check().unwrap_err();
        assert!(matches!(err, EligibilityError::UnsupportedOs { .. }));
    }

    #[test]
    fn current_os_passes() {
        let req = HookRequirements {
            os: vec![current_os()],
            ..Default::default()
        };
        assert!(req.check().is_ok());
    }

    #[test]
    fn serde_roundtrip() {
        let req = HookRequirements {
            bins: vec!["node".into()],
            any_bins: vec!["npm".into(), "yarn".into()],
            env: vec!["HOME".into()],
            os: vec!["macos".into(), "linux".into()],
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: HookRequirements = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.bins, req.bins);
        assert_eq!(parsed.any_bins, req.any_bins);
        assert_eq!(parsed.env, req.env);
        assert_eq!(parsed.os, req.os);
    }
}
