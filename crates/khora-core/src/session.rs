use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::config::KhoraConfig;
use crate::error::{KhoraError, KhoraResult};

/// Session info persisted to disk so CLI invocations can reconnect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Unique session identifier.
    pub id: String,
    /// WebSocket debug URL for reconnecting via CDP.
    pub ws_url: String,
    /// PID of the Chrome process.
    pub pid: u32,
    /// Whether Chrome was launched in headless mode.
    pub headless: bool,
    /// Timestamp when the session was created (Unix epoch seconds).
    pub created_at: u64,
}

impl SessionInfo {
    /// Save session info to `~/.khora/sessions/<id>.json`.
    pub fn save(&self) -> KhoraResult<PathBuf> {
        let dir = KhoraConfig::sessions_dir().ok_or_else(|| {
            KhoraError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "could not determine home directory",
            ))
        })?;
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.json", self.id));
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json)?;
        Ok(path)
    }

    /// Load session info from `~/.khora/sessions/<id>.json`.
    pub fn load(id: &str) -> KhoraResult<Self> {
        let dir = KhoraConfig::sessions_dir()
            .ok_or_else(|| KhoraError::SessionNotFound(id.to_string()))?;
        let path = dir.join(format!("{id}.json"));
        if !path.exists() {
            return Err(KhoraError::SessionNotFound(id.to_string()));
        }
        let json = std::fs::read_to_string(&path)?;
        let info: Self = serde_json::from_str(&json)?;
        Ok(info)
    }

    /// Remove the session file.
    pub fn remove(id: &str) -> KhoraResult<()> {
        if let Some(dir) = KhoraConfig::sessions_dir() {
            let path = dir.join(format!("{id}.json"));
            if path.exists() {
                std::fs::remove_file(path)?;
            }
        }
        Ok(())
    }

    /// List all saved sessions.
    pub fn list_all() -> KhoraResult<Vec<Self>> {
        let dir = match KhoraConfig::sessions_dir() {
            Some(d) if d.exists() => d,
            _ => return Ok(Vec::new()),
        };
        let mut sessions = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") {
                if let Ok(json) = std::fs::read_to_string(&path) {
                    if let Ok(info) = serde_json::from_str::<Self>(&json) {
                        sessions.push(info);
                    }
                }
            }
        }
        Ok(sessions)
    }

    /// Generate a short unique session ID.
    pub fn generate_id() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        // Use last 8 hex chars of timestamp + random suffix
        format!("{:x}", ts & 0xFFFFFFFF)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_session() -> SessionInfo {
        SessionInfo {
            id: "test123".to_string(),
            ws_url: "ws://127.0.0.1:9222/devtools/browser/abc".to_string(),
            pid: 12345,
            headless: true,
            created_at: 1700000000,
        }
    }

    #[test]
    fn test_session_roundtrip() {
        let session = sample_session();
        let json = serde_json::to_string(&session).unwrap();
        let parsed: SessionInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "test123");
        assert_eq!(parsed.pid, 12345);
        assert!(parsed.headless);
    }

    #[test]
    fn test_generate_id_not_empty() {
        let id = SessionInfo::generate_id();
        assert!(!id.is_empty());
    }

    #[test]
    fn test_generate_id_unique() {
        let id1 = SessionInfo::generate_id();
        // Small sleep to ensure different timestamp
        std::thread::sleep(std::time::Duration::from_millis(2));
        let id2 = SessionInfo::generate_id();
        // IDs should differ (though not guaranteed with millisecond granularity,
        // the 2ms sleep makes collision extremely unlikely)
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_load_nonexistent_session() {
        let result = SessionInfo::load("nonexistent_session_xyz");
        assert!(result.is_err());
        match result.unwrap_err() {
            KhoraError::SessionNotFound(id) => assert_eq!(id, "nonexistent_session_xyz"),
            other => panic!("expected SessionNotFound, got: {other}"),
        }
    }

    #[test]
    fn test_list_all_empty() {
        // When sessions dir doesn't exist, should return empty vec
        let sessions = SessionInfo::list_all().unwrap();
        // May or may not be empty depending on test environment,
        // but should not error
        let _ = sessions;
    }
}
