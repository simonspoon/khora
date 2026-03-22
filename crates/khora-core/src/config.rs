use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Configuration for Khora.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KhoraConfig {
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

fn default_timeout() -> u64 {
    5000
}

impl Default for KhoraConfig {
    fn default() -> Self {
        Self {
            timeout_ms: default_timeout(),
        }
    }
}

impl KhoraConfig {
    /// Load config from `~/.khora/config.json`, falling back to defaults.
    pub fn load() -> Self {
        Self::config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn config_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".khora").join("config.json"))
    }

    /// Directory for session files.
    pub fn sessions_dir() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".khora").join("sessions"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_timeout() {
        let config = KhoraConfig::default();
        assert_eq!(config.timeout_ms, 5000);
    }

    #[test]
    fn test_load_falls_back_to_default() {
        let config = KhoraConfig::load();
        assert_eq!(config.timeout_ms, 5000);
    }

    #[test]
    fn test_deserialize_with_timeout() {
        let json = r#"{"timeout_ms": 10000}"#;
        let config: KhoraConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.timeout_ms, 10000);
    }

    #[test]
    fn test_deserialize_empty_uses_default() {
        let json = "{}";
        let config: KhoraConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.timeout_ms, 5000);
    }

    #[test]
    fn test_serialize_roundtrip() {
        let config = KhoraConfig { timeout_ms: 7500 };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: KhoraConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.timeout_ms, 7500);
    }

    #[test]
    fn test_sessions_dir() {
        let dir = KhoraConfig::sessions_dir();
        assert!(dir.is_some());
        let path = dir.unwrap();
        assert!(path.ends_with(".khora/sessions"));
    }
}
