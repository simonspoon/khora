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
