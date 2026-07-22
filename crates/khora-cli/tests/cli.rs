use assert_cmd::Command;
use predicates::prelude::*;

fn khora() -> Command {
    Command::cargo_bin("khora").unwrap()
}

#[test]
fn test_help() {
    khora()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Web app QA automation"));
}

#[test]
fn test_version() {
    khora()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("khora"));
}

#[test]
fn test_launch_help() {
    khora()
        .args(["launch", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Start Chrome"))
        .stdout(predicate::str::contains("--window-size"))
        .stdout(predicate::str::contains("1920x1080"));
}

#[test]
fn test_launch_rejects_invalid_window_size_abc() {
    khora()
        .args(["launch", "--window-size", "abc"])
        .assert()
        .failure();
}

#[test]
fn test_launch_rejects_invalid_window_size_missing_dim() {
    khora()
        .args(["launch", "--window-size", "1920"])
        .assert()
        .failure();
}

#[test]
fn test_launch_rejects_invalid_window_size_partial_alpha() {
    khora()
        .args(["launch", "--window-size", "1920xabc"])
        .assert()
        .failure();
}

#[test]
fn test_launch_rejects_invalid_window_size_zero() {
    khora()
        .args(["launch", "--window-size", "0x0"])
        .assert()
        .failure();
}

#[test]
fn test_navigate_help() {
    khora()
        .args(["navigate", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Navigate to a URL"))
        .stdout(predicate::str::contains("--no-cache"));
}

#[test]
fn test_find_help() {
    khora()
        .args(["find", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("CSS selector"));
}

#[test]
fn test_click_help() {
    khora()
        .args(["click", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Click an element"));
}

#[test]
fn test_type_help() {
    khora()
        .args(["type", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Type text"));
}

#[test]
fn test_type_keys_help() {
    khora()
        .args(["type-keys", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("trusted per-character key events"))
        .stdout(predicate::str::contains("--clear"));
}

#[test]
fn test_type_keys_nonexistent_session() {
    khora()
        .args(["type-keys", "nonexistent_xyz", "#input", "hello"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
}

#[test]
fn test_blur_help() {
    khora()
        .args(["blur", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Blur the focused element"))
        .stdout(predicate::str::contains("document.activeElement"));
}

#[test]
fn test_blur_selector_is_optional() {
    // No selector at all still parses — it defaults to document.activeElement,
    // so the only failure below is the missing session, not a missing arg.
    khora()
        .args(["blur", "nonexistent_xyz"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
}

#[test]
fn test_blur_nonexistent_session() {
    khora()
        .args(["blur", "nonexistent_xyz", "#input"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
}

#[test]
fn test_drag_help() {
    khora()
        .args(["drag", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Drag"))
        .stdout(predicate::str::contains("--steps"))
        .stdout(predicate::str::contains("--delay"));
}

#[test]
fn test_drag_nonexistent_session() {
    khora()
        .args(["drag", "nonexistent_xyz", "0,0", "10,10"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
}

#[test]
fn test_drag_rejects_invalid_point() {
    khora()
        .args(["drag", "nonexistent_xyz", "abc", "10,10"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("X,Y"));
}

#[test]
fn test_drag_rejects_zero_steps() {
    khora()
        .args(["drag", "nonexistent_xyz", "0,0", "10,10", "--steps", "0"])
        .assert()
        .failure();
}

#[test]
fn test_mouse_down_help() {
    khora()
        .args(["mouse-down", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Press the left mouse button"));
}

#[test]
fn test_mouse_down_nonexistent_session() {
    khora()
        .args(["mouse-down", "nonexistent_xyz", "0,0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
}

#[test]
fn test_mouse_move_help() {
    khora()
        .args(["mouse-move", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Move the mouse"));
}

#[test]
fn test_mouse_move_nonexistent_session() {
    khora()
        .args(["mouse-move", "nonexistent_xyz", "0,0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
}

#[test]
fn test_mouse_up_help() {
    khora()
        .args(["mouse-up", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Release the left mouse button"));
}

#[test]
fn test_mouse_up_nonexistent_session() {
    khora()
        .args(["mouse-up", "nonexistent_xyz", "0,0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
}

#[test]
fn test_click_at_help() {
    khora()
        .args(["click-at", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Click at a raw viewport point"));
}

#[test]
fn test_click_at_nonexistent_session() {
    khora()
        .args(["click-at", "nonexistent_xyz", "0,0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
}

#[test]
fn test_click_at_rejects_invalid_point() {
    khora()
        .args(["click-at", "nonexistent_xyz", "abc"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("X,Y"));
}

#[test]
fn test_dblclick_at_help() {
    khora()
        .args(["dblclick-at", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Double-click at a raw viewport point",
        ));
}

#[test]
fn test_dblclick_at_nonexistent_session() {
    khora()
        .args(["dblclick-at", "nonexistent_xyz", "0,0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
}

#[test]
fn test_dblclick_at_rejects_invalid_point() {
    khora()
        .args(["dblclick-at", "nonexistent_xyz", "abc"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("X,Y"));
}

#[test]
fn test_wheel_help() {
    khora()
        .args(["wheel", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("trusted native wheel event"));
}

#[test]
fn test_wheel_nonexistent_session() {
    khora()
        .args(["wheel", "nonexistent_xyz", "0,0", "0,300"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
}

#[test]
fn test_wheel_rejects_invalid_point() {
    khora()
        .args(["wheel", "nonexistent_xyz", "abc", "0,300"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("X,Y"));
}

#[test]
fn test_wheel_rejects_invalid_delta() {
    khora()
        .args(["wheel", "nonexistent_xyz", "0,0", "abc"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("X,Y"));
}

#[test]
fn test_key_help() {
    khora()
        .args(["key", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("trusted key event"));
}

#[test]
fn test_key_nonexistent_session() {
    khora()
        .args(["key", "nonexistent_xyz", "Cmd+D"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
}

#[test]
fn test_mouse_move_rejects_invalid_point() {
    khora()
        .args(["mouse-move", "nonexistent_xyz", "abc"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("X,Y"));
}

#[test]
fn test_screenshot_help() {
    khora()
        .args(["screenshot", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("screenshot"))
        .stdout(predicate::str::contains("--selector"))
        .stdout(predicate::str::contains("--full-page"))
        .stdout(predicate::str::contains("--viewport"))
        .stdout(predicate::str::contains("--clip"));
}

#[test]
fn test_screenshot_full_page_and_viewport_conflict() {
    khora()
        .args(["screenshot", "abc123", "--full-page", "--viewport"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn test_screenshot_clip_and_selector_conflict() {
    khora()
        .args([
            "screenshot",
            "abc123",
            "--clip",
            "0,0,100x100",
            "--selector",
            "body",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn test_screenshot_rejects_malformed_clip() {
    khora()
        .args(["screenshot", "abc123", "--clip", "0,0,1920"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("X,Y,WxH"));
}

#[test]
fn test_eval_help() {
    khora()
        .args(["eval", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("JavaScript"));
}

#[test]
fn test_kill_help() {
    khora()
        .args(["kill", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Close browser"));
}

#[test]
fn test_status_no_sessions() {
    khora().args(["status"]).assert().success();
}

#[test]
fn test_broken_pipe_exits_quietly() {
    use std::process::{Command as StdCommand, Stdio};
    // `status` always prints ("No active sessions." at minimum). Closing our
    // end of the pipe before the child writes forces EPIPE on its stdout.
    let mut child = StdCommand::new(assert_cmd::cargo::cargo_bin("khora"))
        .arg("status")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("panicked") && !stderr.contains("Broken pipe"),
        "khora panicked on EPIPE: {stderr}"
    );
    assert!(out.status.success(), "expected quiet success, got {out:?}");
}

#[test]
fn test_status_nonexistent_session() {
    khora()
        .args(["status", "nonexistent_xyz"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
}

#[test]
fn test_navigate_nonexistent_session() {
    khora()
        .args(["navigate", "nonexistent_xyz", "https://example.com"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
}

#[test]
fn test_format_json_flag() {
    khora()
        .args(["--format", "json", "status"])
        .assert()
        .success();
}

#[test]
fn test_invalid_command() {
    khora().args(["notacommand"]).assert().failure();
}

#[test]
fn test_wait_for_help() {
    khora()
        .args(["wait-for", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Wait for an element"));
}

#[test]
fn test_wait_gone_help() {
    khora()
        .args(["wait-gone", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("disappear"));
}

#[test]
fn test_console_help() {
    khora()
        .args(["console", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("console"));
}

#[test]
fn test_text_help() {
    khora()
        .args(["text", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("text content"));
}

#[test]
fn test_attribute_help() {
    khora()
        .args(["attribute", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("attribute"));
}

#[test]
fn test_set_viewport_help() {
    khora()
        .args(["set-viewport", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("viewport"))
        .stdout(predicate::str::contains("--mobile"));
}

#[test]
fn test_set_viewport_nonexistent_session() {
    khora()
        .args(["set-viewport", "nonexistent_xyz", "390x844"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
}

#[test]
fn test_set_viewport_rejects_invalid_size() {
    khora()
        .args(["set-viewport", "nonexistent_xyz", "390"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("WxH"));
}

#[test]
fn test_set_viewport_rejects_negative_dpr() {
    khora()
        .args(["set-viewport", "nonexistent_xyz", "390x844", "--", "-1"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("dpr"));
}

#[test]
fn test_reap_no_sessions() {
    khora().args(["reap"]).assert().success();
}

#[test]
fn test_reap_help() {
    khora()
        .args(["reap", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Reap"));
}

#[test]
fn test_reap_json_format() {
    khora()
        .args(["--format", "json", "reap"])
        .assert()
        .success()
        .stdout(predicate::str::contains("reaped"));
}
