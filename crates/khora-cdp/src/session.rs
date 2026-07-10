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

/// Terminate a Chrome process, and any child processes it spawned (Chrome's
/// renderer/GPU/zygote helpers), by PID: SIGTERM, then SIGKILL if still alive
/// after a short grace period. No-op if pid is 0 (unknown) or already dead.
/// Returns true if the process tree was confirmed gone (or was never alive).
pub async fn kill_process(pid: u32) -> bool {
    if pid == 0 || !is_process_alive(pid) {
        return true;
    }

    // Discover descendants before signaling: once the parent dies, its
    // children are reparented and no longer found by their original ppid.
    let targets = process_tree(pid);

    send_signal(&targets, false);
    if wait_for_exit(&targets).await {
        return true;
    }

    send_signal(&targets, true);
    wait_for_exit(&targets).await
}

/// Collect `pid` and all of its descendant process IDs.
#[cfg(unix)]
fn process_tree(pid: u32) -> Vec<u32> {
    let mut tree = vec![pid];
    let mut frontier = vec![pid];
    while let Some(parent) = frontier.pop() {
        let children: Vec<u32> = std::process::Command::new("pgrep")
            .args(["-P", &parent.to_string()])
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter_map(|l| l.trim().parse().ok())
                    .collect()
            })
            .unwrap_or_default();
        frontier.extend(&children);
        tree.extend(children);
    }
    tree
}

/// Windows' `taskkill /T` below kills the whole process tree in one call.
#[cfg(not(unix))]
fn process_tree(pid: u32) -> Vec<u32> {
    vec![pid]
}

#[cfg(unix)]
fn send_signal(pids: &[u32], force: bool) {
    let sig = if force { libc::SIGKILL } else { libc::SIGTERM };
    for &pid in pids {
        unsafe {
            libc::kill(pid as i32, sig);
        }
    }
}

#[cfg(not(unix))]
fn send_signal(pids: &[u32], force: bool) {
    for &pid in pids {
        let mut args = vec!["/FI".to_string(), format!("PID eq {pid}"), "/T".to_string()];
        if force {
            args.push("/F".to_string());
        }
        let _ = std::process::Command::new("taskkill").args(&args).output();
    }
}

async fn wait_for_exit(pids: &[u32]) -> bool {
    for _ in 0..10 {
        if pids.iter().all(|&p| !is_process_alive(p)) {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    pids.iter().all(|&p| !is_process_alive(p))
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

    #[tokio::test]
    async fn kill_process_terminates_live_process() {
        // A real long-running child stands in for an orphaned Chrome process.
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let pid = child.id();
        assert!(is_process_alive(pid), "child should start alive");

        // In production the process is orphaned (its `khora launch` parent has
        // already exited, so init reaps it as soon as it dies). Here we still
        // hold the child handle, so a signaled-but-unreaped process would look
        // "alive" via kill(pid, 0) as a zombie — reap concurrently to match.
        let reaper = std::thread::spawn(move || {
            child.wait().ok();
        });

        assert!(
            kill_process(pid).await,
            "kill_process should confirm termination"
        );
        assert!(
            !is_process_alive(pid),
            "process should be dead after kill_process"
        );

        reaper.join().ok();
    }

    #[tokio::test]
    async fn kill_process_noop_for_zero_pid() {
        assert!(kill_process(0).await);
    }

    #[tokio::test]
    async fn kill_process_noop_for_already_dead_pid() {
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("spawn true");
        let dead_pid = child.id();
        child.wait().ok();

        assert!(kill_process(dead_pid).await);
    }

    #[tokio::test]
    async fn kill_process_terminates_child_process_too() {
        // Simulate Chrome's parent-plus-renderer process shape: a parent shell
        // that spawns a child `sleep`, both still running when killed.
        let mut parent = std::process::Command::new("sh")
            .arg("-c")
            .arg("sleep 30 & wait")
            .spawn()
            .expect("spawn parent shell");
        let parent_pid = parent.id();

        // Give the child time to spawn and register under the parent's pid.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let children = process_tree(parent_pid);
        assert!(
            children.len() > 1,
            "expected the shell's sleep child to be discoverable, got {children:?}"
        );

        let reaper = std::thread::spawn(move || {
            parent.wait().ok();
        });

        assert!(kill_process(parent_pid).await);
        for pid in children {
            assert!(!is_process_alive(pid), "pid {pid} should be dead");
        }

        reaper.join().ok();
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
