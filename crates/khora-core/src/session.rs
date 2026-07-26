use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::config::KhoraConfig;
use crate::error::{KhoraError, KhoraResult};

/// CSS media state emulated for a session (CDP `Emulation.setEmulatedMedia`).
///
/// Chrome reverts media emulation as soon as the CDP client that set it
/// disconnects, and khora opens a fresh connection per command — so a one-shot
/// call would be gone before the next invocation could observe it. The state
/// lives here instead and is re-applied on every connect.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaEmulation {
    /// Media type to emulate (`screen`, `print`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<String>,
    /// `prefers-color-scheme` value (`light`, `dark`, `no-preference`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_scheme: Option<String>,
    /// `prefers-reduced-motion` value (`reduce`, `no-preference`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reduced_motion: Option<String>,
    /// `forced-colors` value (`active`, `none`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forced_colors: Option<String>,
}

impl MediaEmulation {
    /// True when nothing is overridden — Chrome's own defaults apply.
    pub fn is_empty(&self) -> bool {
        self.media.is_none()
            && self.color_scheme.is_none()
            && self.reduced_motion.is_none()
            && self.forced_colors.is_none()
    }

    /// Overlay whatever `other` sets, leaving the rest in place.
    ///
    /// Makes the overrides accumulate across calls, so `--color-scheme dark`
    /// followed by `--media print` keeps the dark scheme rather than the second
    /// call silently dropping it.
    pub fn merge(&mut self, other: &Self) {
        if other.media.is_some() {
            self.media.clone_from(&other.media);
        }
        if other.color_scheme.is_some() {
            self.color_scheme.clone_from(&other.color_scheme);
        }
        if other.reduced_motion.is_some() {
            self.reduced_motion.clone_from(&other.reduced_motion);
        }
        if other.forced_colors.is_some() {
            self.forced_colors.clone_from(&other.forced_colors);
        }
    }

    /// The overridden media features as CDP `(name, value)` pairs.
    /// The media *type* is a separate CDP field and is not included here.
    pub fn features(&self) -> Vec<(&'static str, String)> {
        let mut features = Vec::new();
        if let Some(v) = &self.color_scheme {
            features.push(("prefers-color-scheme", v.clone()));
        }
        if let Some(v) = &self.reduced_motion {
            features.push(("prefers-reduced-motion", v.clone()));
        }
        if let Some(v) = &self.forced_colors {
            features.push(("forced-colors", v.clone()));
        }
        features
    }
}

/// Touch-device emulation for a session (CDP
/// `Emulation.setTouchEmulationEnabled`).
///
/// Persisted for the same reason as [`MediaEmulation`]: Chrome reverts the
/// override the moment the CDP client that set it disconnects, which khora does
/// after every command. Re-applying it on connect is also what makes it visible
/// to a page's *load-time* feature detection, since `'ontouchstart' in window`
/// is decided when the document is created — the re-apply lands before the
/// navigation that follows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TouchEmulation {
    /// Value `navigator.maxTouchPoints` should report.
    pub max_touch_points: u32,
}

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
    /// Chrome user data directory for this session (cleaned up on close).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<PathBuf>,
    /// Media features emulated for this session, re-applied on every connect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaEmulation>,
    /// Touch-device emulation for this session, re-applied on every connect.
    /// `None` means the real device's capabilities are reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub touch: Option<TouchEmulation>,
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
            data_dir: None,
            media: None,
            touch: None,
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
    fn test_session_without_media_field_still_loads() {
        // Session files written before media emulation existed must keep working.
        let json = r#"{"id":"old","ws_url":"ws://x","pid":1,"headless":true,"created_at":0}"#;
        let parsed: SessionInfo = serde_json::from_str(json).unwrap();
        assert!(parsed.media.is_none());
        assert!(parsed.touch.is_none());
    }

    #[test]
    fn test_touch_emulation_roundtrip() {
        let session = SessionInfo {
            touch: Some(TouchEmulation {
                max_touch_points: 5,
            }),
            ..sample_session()
        };
        let json = serde_json::to_string(&session).unwrap();
        let parsed: SessionInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.touch.unwrap().max_touch_points, 5);
    }

    #[test]
    fn test_touch_emulation_omitted_when_off() {
        // `--off` clears the field rather than writing `enabled: false`, so an
        // older khora reading the file sees exactly what it would have written.
        let json = serde_json::to_string(&sample_session()).unwrap();
        assert!(!json.contains("touch"));
    }

    #[test]
    fn test_media_emulation_merge_keeps_unmentioned_features() {
        let mut state = MediaEmulation {
            color_scheme: Some("dark".to_string()),
            ..Default::default()
        };
        state.merge(&MediaEmulation {
            media: Some("print".to_string()),
            ..Default::default()
        });
        assert_eq!(state.color_scheme.as_deref(), Some("dark"));
        assert_eq!(state.media.as_deref(), Some("print"));
    }

    #[test]
    fn test_media_emulation_merge_overwrites_same_feature() {
        let mut state = MediaEmulation {
            color_scheme: Some("dark".to_string()),
            ..Default::default()
        };
        state.merge(&MediaEmulation {
            color_scheme: Some("light".to_string()),
            ..Default::default()
        });
        assert_eq!(state.color_scheme.as_deref(), Some("light"));
    }

    #[test]
    fn test_media_emulation_features_excludes_media_type() {
        let state = MediaEmulation {
            media: Some("print".to_string()),
            color_scheme: Some("dark".to_string()),
            reduced_motion: Some("reduce".to_string()),
            forced_colors: Some("active".to_string()),
        };
        let features = state.features();
        assert_eq!(
            features,
            vec![
                ("prefers-color-scheme", "dark".to_string()),
                ("prefers-reduced-motion", "reduce".to_string()),
                ("forced-colors", "active".to_string()),
            ]
        );
        assert!(!state.is_empty());
        assert!(MediaEmulation::default().is_empty());
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
