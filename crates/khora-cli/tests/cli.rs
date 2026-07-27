use assert_cmd::Command;
use predicates::prelude::*;

// The version-string renderer `build.rs` stamps into the binary. A build script
// cannot depend on its own crate, so both sides `include!` the same file.
include!("../build_version.rs");

fn khora() -> Command {
    Command::cargo_bin("khora").unwrap()
}

/// `git describe --tags --always --long` for the package, as `build.rs` ran it,
/// or `None` when this is not a checkout (a tarball build).
fn git_describe() -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["describe", "--tags", "--always", "--long"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!s.is_empty()).then_some(s)
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
        .stdout(predicate::str::starts_with(format!(
            "khora {}",
            env!("CARGO_PKG_VERSION")
        )));
}

/// The binary's stamp must match the git state it was built from — that is the
/// whole claim, and it is what makes a version read a usable capability signal.
#[test]
fn test_version_matches_git_describe() {
    let Some(describe) = git_describe() else {
        return; // not a checkout; nothing to compare the stamp against
    };
    let expected = build_version(env!("CARGO_PKG_VERSION"), Some(&describe));
    khora()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::eq(format!("khora {expected}\n")));
}

/// A build made past the release tag must not report the bare package version.
/// This is the understatement the marker exists to prevent: `khora 0.3.17` on a
/// binary carrying thirteen commits' worth of extra commands.
#[test]
fn test_version_marks_post_tag_builds() {
    let Some(describe) = git_describe() else {
        return;
    };
    let Some((commits, _)) = parse_describe_long(&describe) else {
        return; // no tag reachable from HEAD (a shallow CI checkout)
    };
    if commits == 0 {
        return; // exactly on a release tag: a bare version is correct
    }
    let out = khora().arg("--version").output().unwrap();
    let reported = String::from_utf8(out.stdout).unwrap().trim().to_string();
    assert_ne!(
        reported,
        format!("khora {}", env!("CARGO_PKG_VERSION")),
        "{commits} commits past the tag, but --version reports the bare package version"
    );
    assert!(
        reported.contains(&format!("+{commits}.g")),
        "expected a +<commits>.g<sha> marker, got: {reported}"
    );
}

#[test]
fn test_build_version_on_tag_is_bare() {
    assert_eq!(
        build_version("0.3.17", Some("v0.3.17-0-g830d5cb")),
        "0.3.17"
    );
}

#[test]
fn test_build_version_past_tag_carries_marker() {
    assert_eq!(
        build_version("0.3.17", Some("v0.3.17-13-g6114093")),
        "0.3.17+13.g6114093"
    );
}

#[test]
fn test_build_version_tolerates_hyphenated_tag() {
    assert_eq!(
        build_version("1.0.0", Some("v1.0.0-rc1-3-gabc1234")),
        "1.0.0+3.gabc1234"
    );
}

#[test]
fn test_build_version_without_tags_uses_bare_sha() {
    assert_eq!(build_version("0.3.17", Some("2153033")), "0.3.17+2153033");
}

#[test]
fn test_build_version_without_git_is_bare() {
    assert_eq!(build_version("0.3.17", None), "0.3.17");
    assert_eq!(build_version("0.3.17", Some("  ")), "0.3.17");
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
fn test_touch_help() {
    khora()
        .args(["touch", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("trusted touch events"));
}

#[test]
fn test_touch_nonexistent_session() {
    khora()
        .args(["touch", "nonexistent_xyz", "10,20"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
}

#[test]
fn test_touch_rejects_invalid_point() {
    khora()
        .args(["touch", "nonexistent_xyz", "abc"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("X,Y"));
}

#[test]
fn test_swipe_help() {
    khora()
        .args(["swipe", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("touch-action"));
}

#[test]
fn test_swipe_nonexistent_session() {
    khora()
        .args(["swipe", "nonexistent_xyz", "10,20", "10,5"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
}

#[test]
fn test_swipe_rejects_invalid_point() {
    khora()
        .args(["swipe", "nonexistent_xyz", "abc", "10,5"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("X,Y"));
}

/// Swiping content into view above the fold means a negative Y endpoint, and
/// clap parses a leading `-` as a flag unless the argument opts out. Asserted
/// at the CLI layer on purpose: a parser-level test would pass while the
/// shipped command still died at argument parsing.
#[test]
fn test_swipe_accepts_negative_coordinates() {
    khora()
        .args(["swipe", "nonexistent_xyz", "200,600", "-40,-100"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
}

#[test]
fn test_pinch_help() {
    khora()
        .args(["pinch", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("two fingers"));
}

#[test]
fn test_pinch_nonexistent_session() {
    khora()
        .args([
            "pinch",
            "nonexistent_xyz",
            "400,300",
            "--from",
            "100",
            "--to",
            "300",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
}

/// Both gaps are required — a pinch with only one end of the range named has
/// no motion to interpolate, and defaulting the other would invent a gesture
/// the caller did not ask for.
#[test]
fn test_pinch_requires_both_gaps() {
    khora()
        .args(["pinch", "nonexistent_xyz", "400,300", "--from", "100"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--to"));
}

/// Asserted at the CLI layer, not against `parse_gap`: clap decides how
/// `--from -10` is tokenized before any value parser sees it, so only a
/// process-level test proves a negative gap is actually unreachable.
#[test]
fn test_pinch_rejects_negative_gap() {
    khora()
        .args([
            "pinch",
            "nonexistent_xyz",
            "400,300",
            "--from",
            "-10",
            "--to",
            "300",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found").not());
}

#[test]
fn test_pinch_rejects_invalid_gap() {
    khora()
        .args([
            "pinch",
            "nonexistent_xyz",
            "400,300",
            "--from",
            "abc",
            "--to",
            "300",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--from"));
}

#[test]
fn test_pinch_rejects_zero_steps() {
    khora()
        .args([
            "pinch",
            "nonexistent_xyz",
            "400,300",
            "--from",
            "100",
            "--to",
            "300",
            "--steps",
            "0",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--steps"));
}

#[test]
fn test_long_press_help() {
    khora()
        .args(["long-press", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("trusted touch events"));
}

#[test]
fn test_long_press_nonexistent_session() {
    khora()
        .args(["long-press", "nonexistent_xyz", "10,20"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
}

#[test]
fn test_long_press_rejects_invalid_duration() {
    khora()
        .args([
            "long-press",
            "nonexistent_xyz",
            "10,20",
            "--duration",
            "abc",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--duration"));
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
fn test_status_check_flag_documented() {
    khora()
        .args(["status", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--check"))
        .stdout(predicate::str::contains("unresponsive"));
}

#[test]
fn test_status_check_on_nonexistent_session_exits_4() {
    // `--check` must not turn a missing session into a different failure than
    // the bare command: both are "stop driving this session", exit 4.
    khora()
        .args(["status", "nonexistent_xyz", "--check"])
        .assert()
        .code(4)
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
fn test_targets_help() {
    khora()
        .args(["targets", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("open targets"));
}

#[test]
fn test_targets_nonexistent_session() {
    khora()
        .args(["targets", "nonexistent_xyz"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
}

/// `pages` is the name the DevTools endpoint uses; keep it reachable so the
/// obvious guess works.
#[test]
fn test_pages_alias_resolves_to_targets() {
    khora()
        .args(["pages", "nonexistent_xyz"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
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
fn test_set_viewport_help_documents_persistence_and_reset() {
    // The command's whole contract changed — an override now outlives the
    // invocation — so --help has to say so, and has to offer the way back out.
    khora()
        .args(["set-viewport", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("re-applied"))
        .stdout(predicate::str::contains("--reset"));
}

#[test]
fn test_set_viewport_requires_size_or_reset() {
    // Guards the clap layer, not the helper below it: `size` became optional
    // so --reset could stand alone, and nothing under clap would ever see a
    // call that named neither.
    khora()
        .args(["set-viewport", "nonexistent_xyz"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn test_set_viewport_reset_conflicts_with_size() {
    khora()
        .args(["set-viewport", "nonexistent_xyz", "390x844", "--reset"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn test_set_viewport_reset_reaches_the_session_lookup() {
    // Proves --reset is accepted by the parser and gets as far as resolving a
    // session — otherwise the two conflict tests above could both pass while
    // --reset was itself unparseable.
    khora()
        .args(["set-viewport", "nonexistent_xyz", "--reset"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
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
fn test_emulate_media_help() {
    khora()
        .args(["emulate-media", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--color-scheme"))
        .stdout(predicate::str::contains("--reduced-motion"))
        .stdout(predicate::str::contains("--forced-colors"))
        .stdout(predicate::str::contains("--reset"));
}

#[test]
fn test_emulate_media_nonexistent_session() {
    khora()
        .args(["emulate-media", "nonexistent_xyz", "--color-scheme", "dark"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
}

#[test]
fn test_emulate_media_requires_a_flag() {
    khora()
        .args(["emulate-media", "nonexistent_xyz"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--color-scheme"));
}

#[test]
fn test_emulate_media_rejects_unknown_color_scheme() {
    khora()
        .args([
            "emulate-media",
            "nonexistent_xyz",
            "--color-scheme",
            "sepia",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("sepia"));
}

#[test]
fn test_emulate_media_reset_conflicts_with_overrides() {
    khora()
        .args([
            "emulate-media",
            "nonexistent_xyz",
            "--reset",
            "--color-scheme",
            "dark",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn test_emulate_touch_help() {
    khora()
        .args(["emulate-touch", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--max-points"))
        .stdout(predicate::str::contains("--off"))
        // The load-time ordering is the whole reason the command exists, so the
        // help must say it — a caller who runs it after `navigate` gets a page
        // whose 'ontouchstart' check already ran.
        .stdout(predicate::str::contains("BEFORE `navigate`"));
}

#[test]
fn test_emulate_touch_nonexistent_session() {
    khora()
        .args(["emulate-touch", "nonexistent_xyz"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
}

#[test]
fn test_emulate_touch_off_conflicts_with_max_points() {
    khora()
        .args([
            "emulate-touch",
            "nonexistent_xyz",
            "--off",
            "--max-points",
            "3",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn test_emulate_touch_rejects_out_of_range_max_points() {
    // Chrome caps maxTouchPoints at 16; reject at the CLI rather than letting
    // CDP fail after a session has already been resolved.
    khora()
        .args(["emulate-touch", "nonexistent_xyz", "--max-points", "99"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("1..=16"));
}

#[test]
fn test_emulate_touch_rejects_zero_max_points() {
    khora()
        .args(["emulate-touch", "nonexistent_xyz", "--max-points", "0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("1..=16"));
}

#[test]
fn test_emulate_keyboard_help() {
    khora()
        .args(["emulate-keyboard", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--off"))
        // The layout/visual split is the entire reason the command exists, and
        // the help is the only place a caller reaching for `set-viewport`
        // instead will be told why that produces a green result on a real bug.
        .stdout(predicate::str::contains(
            "LAYOUT viewport is left untouched",
        ))
        .stdout(predicate::str::contains("set-viewport"))
        // The rig cannot reproduce offsetTop, so the help says so rather than
        // letting a fully green run imply an offsetTop translate was verified.
        .stdout(predicate::str::contains("offsetTop"));
}

#[test]
fn test_emulate_keyboard_nonexistent_session() {
    khora()
        .args(["emulate-keyboard", "nonexistent_xyz", "300"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("session not found"));
}

#[test]
fn test_emulate_keyboard_requires_inset_or_off() {
    // Neither given: the command has no sensible default — a bare invocation
    // must not be read as "retract", which is what an Option<u32> with no
    // group would silently do.
    khora()
        .args(["emulate-keyboard", "nonexistent_xyz"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn test_emulate_keyboard_off_conflicts_with_inset() {
    khora()
        .args(["emulate-keyboard", "nonexistent_xyz", "300", "--off"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn test_emulate_keyboard_rejects_zero_inset() {
    // A zero-pixel keyboard is a retraction wearing a number, and it would
    // persist as `Some(0)` — a session that claims to be emulating a keyboard
    // while every later command re-applies a no-op shadow.
    khora()
        .args(["emulate-keyboard", "nonexistent_xyz", "0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not in 1..=10000"));
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

#[test]
fn test_reap_help_documents_purge_data_dirs() {
    khora()
        .args(["reap", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--purge-data-dirs"));
}

#[test]
fn test_reap_without_purge_reports_no_data_dir_sweep() {
    // The sweep is opt-in, so the default JSON must not carry its keys —
    // existing callers parse this output.
    khora()
        .args(["--format", "json", "reap"])
        .assert()
        .success()
        .stdout(predicate::str::contains("purged_data_dirs").not());
}

#[test]
#[cfg(unix)]
fn test_rust_log_diagnostics_go_to_stderr_not_stdout() {
    // stdout is the command's result and callers pipe it (`khora eval | jq`),
    // so tracing output must never land there. A planted session whose PID is
    // dead makes auto-reap emit a log line without needing Chrome, which is
    // what makes this assert on real diagnostic output rather than on silence.
    //
    // Unix-only because the setup redirects the sessions dir via $HOME, and
    // `dirs::home_dir()` honours $HOME only on Linux/macOS — on Windows it
    // reads the profile folder through SHGetKnownFolderPath, so the planted
    // session would never be found and the assertion would fail on the
    // windows-latest CI leg. The behaviour under test (which writer the
    // subscriber uses) is platform-independent; only this harness is not.
    let home = std::env::temp_dir().join(format!("khora-log-test-{}", std::process::id()));
    let dir = home.join(".khora").join("sessions");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("deadbeef.json"),
        r#"{"id":"deadbeef","ws_url":"ws://127.0.0.1:1/devtools/browser/x","pid":999999,"headless":true,"created_at":0}"#,
    )
    .unwrap();

    khora()
        .env("HOME", &home)
        .env("RUST_LOG", "info")
        .args(["status", "deadbeef"])
        .assert()
        .stderr(predicate::str::contains(
            "auto-reaped stale session deadbeef",
        ))
        .stdout(predicate::str::contains("auto-reaped").not());

    std::fs::remove_dir_all(&home).ok();
}
