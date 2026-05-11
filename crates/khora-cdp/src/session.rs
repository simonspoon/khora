use khora_core::error::{KhoraError, KhoraResult};
use khora_core::session::SessionInfo;

/// Check if a session's Chrome process is still alive.
pub fn is_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return true; // Unknown PID, assume alive
    }

    #[cfg(unix)]
    {
        // Use libc kill(pid, 0) to check if process exists without sending a signal
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }

    #[cfg(not(unix))]
    {
        // On Windows, try tasklist
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
            .unwrap_or(true)
    }
}

/// Reap sessions whose Chrome process has died.
/// Called automatically at CLI startup — best effort, never errors the caller.
pub fn reap_stale_sessions() {
    let sessions = match SessionInfo::list_all() {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("auto-reap: failed to list sessions: {e}");
            return;
        }
    };
    for info in sessions {
        if info.pid > 0 && !is_process_alive(info.pid) {
            if let Some(ref dir) = info.data_dir {
                crate::client::cleanup_data_dir(dir);
            }
            let _ = SessionInfo::remove(&info.id);
            tracing::info!("auto-reaped stale session {}", info.id);
        }
    }
}

/// Load a session and verify it's still valid.
pub fn load_and_verify(session_id: &str) -> KhoraResult<SessionInfo> {
    let session = SessionInfo::load(session_id)?;

    if session.pid > 0 && !is_process_alive(session.pid) {
        // Clean up stale session file
        let _ = SessionInfo::remove(&session.id);
        return Err(KhoraError::SessionDead(session_id.to_string()));
    }

    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn make_session(id: &str, pid: u32) -> SessionInfo {
        SessionInfo {
            id: id.to_string(),
            ws_url: "ws://127.0.0.1:9999/devtools/browser/test".to_string(),
            pid,
            headless: true,
            created_at: now_secs(),
            data_dir: None,
        }
    }

    #[test]
    fn is_process_alive_self() {
        assert!(is_process_alive(std::process::id()));
    }

    #[test]
    fn is_process_alive_fake_high_pid() {
        assert!(!is_process_alive(9_999_999));
    }

    #[test]
    fn is_process_alive_zero_assumes_alive() {
        assert!(is_process_alive(0));
    }

    #[test]
    fn auto_reap_removes_dead_pid_session() {
        // Spawn a short-lived process, wait for it to exit, then its PID is dead.
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("spawn true");
        let dead_pid = child.id();
        child.wait().ok();

        let id = format!("test-dead-reap-{}", now_secs());
        let session = make_session(&id, dead_pid);
        if session.save().is_err() {
            // Sessions dir not available in this environment — skip.
            return;
        }

        reap_stale_sessions();

        assert!(
            SessionInfo::load(&id).is_err(),
            "dead-pid session should have been reaped"
        );
    }

    #[test]
    fn auto_reap_preserves_live_pid_session() {
        // Use our own PID — definitely alive.
        let live_pid = std::process::id();
        let id = format!("test-live-reap-{}", now_secs());
        let session = make_session(&id, live_pid);
        if session.save().is_err() {
            return;
        }

        reap_stale_sessions();

        assert!(
            SessionInfo::load(&id).is_ok(),
            "live-pid session should be preserved"
        );
        // Cleanup
        let _ = SessionInfo::remove(&id);
    }
}
