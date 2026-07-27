use crate::element::{BrowserTarget, ConsoleMessage, ElementInfo, NetworkRequest, SessionHealth};
use crate::session::{
    KeyboardEmulation, MediaEmulation, SessionInfo, TouchEmulation, ViewportEmulation,
};

/// Output format for CLI results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

/// Format session info for display.
pub fn format_session(session: &SessionInfo, format: OutputFormat) -> String {
    match format {
        OutputFormat::Text => {
            format!(
                "Session: {}\nPID: {}\nHeadless: {}\nWebSocket: {}",
                session.id, session.pid, session.headless, session.ws_url
            )
        }
        OutputFormat::Json => serde_json::to_string_pretty(session).unwrap_or_default(),
    }
}

/// Format the media emulation now in effect for a session.
pub fn format_media_emulation(media: &MediaEmulation, format: OutputFormat) -> String {
    match format {
        OutputFormat::Text => {
            if media.is_empty() {
                return "Media emulation cleared.".to_string();
            }
            let mut parts = Vec::new();
            if let Some(v) = &media.media {
                parts.push(format!("media={v}"));
            }
            for (name, value) in media.features() {
                parts.push(format!("{name}={value}"));
            }
            format!("Media emulated: {}", parts.join(" "))
        }
        OutputFormat::Json => serde_json::to_string_pretty(&serde_json::json!({
            "action": "emulate_media",
            "media": media.media,
            "color_scheme": media.color_scheme,
            "reduced_motion": media.reduced_motion,
            "forced_colors": media.forced_colors,
        }))
        .unwrap_or_default(),
    }
}

/// Format the touch emulation now in effect for a session.
///
/// `ontouchstart_pending` is set when this call actually changed the state, and
/// so the document currently loaded predates it and still reports the old
/// `'ontouchstart' in window`. It is surfaced because that is the signal
/// callers came for and the only one that needs a navigation to follow.
pub fn format_touch_emulation(
    touch: Option<&TouchEmulation>,
    ontouchstart_pending: bool,
    format: OutputFormat,
) -> String {
    match format {
        OutputFormat::Text => {
            let head = match touch {
                Some(t) => format!(
                    "Touch emulated: maxTouchPoints={} (pointer: coarse)",
                    t.max_touch_points
                ),
                None => "Touch emulation cleared.".to_string(),
            };
            if ontouchstart_pending {
                format!("{head}\nNavigate or reload for 'ontouchstart' in window to follow.")
            } else {
                head
            }
        }
        OutputFormat::Json => serde_json::to_string_pretty(&serde_json::json!({
            "action": "emulate_touch",
            "enabled": touch.is_some(),
            "max_touch_points": touch.map(|t| t.max_touch_points),
            "ontouchstart_pending": ontouchstart_pending,
        }))
        .unwrap_or_default(),
    }
}

/// Format the outcome of an `emulate-keyboard` call. `None` means the keyboard
/// was retracted.
///
/// `visual_height` is read back from the page rather than computed here: it is
/// the number the app's own listener just saw, and the only one that proves the
/// shadow reached the document. Reporting it beside the inset also removes the
/// ambiguity in the argument — `PX` is the keyboard's height, not the
/// viewport's.
pub fn format_keyboard_emulation(
    keyboard: Option<&KeyboardEmulation>,
    visual_height: f64,
    format: OutputFormat,
) -> String {
    match format {
        OutputFormat::Text => match keyboard {
            Some(k) => format!(
                "Keyboard emulated: inset={}px visualViewport.height={} \
                 (layout viewport unchanged)",
                k.inset, visual_height
            ),
            None => format!("Keyboard emulation cleared. visualViewport.height={visual_height}"),
        },
        OutputFormat::Json => serde_json::to_string_pretty(&serde_json::json!({
            "action": "emulate_keyboard",
            "enabled": keyboard.is_some(),
            "inset": keyboard.map(|k| k.inset),
            "visual_viewport_height": visual_height,
        }))
        .unwrap_or_default(),
    }
}

/// Format the outcome of a `set-viewport` call. `None` means the override was
/// cleared.
pub fn format_viewport_emulation(
    viewport: Option<&ViewportEmulation>,
    format: OutputFormat,
) -> String {
    match format {
        OutputFormat::Text => match viewport {
            Some(v) => {
                let mut msg = format!("Viewport set: {}x{}", v.width, v.height);
                if v.dpr > 0.0 {
                    msg.push_str(&format!(" dpr={}", v.dpr));
                }
                if v.mobile {
                    msg.push_str(" (mobile)");
                }
                msg
            }
            None => "Viewport override cleared.".to_string(),
        },
        OutputFormat::Json => serde_json::to_string_pretty(&serde_json::json!({
            "action": "set_viewport",
            "enabled": viewport.is_some(),
            "width": viewport.map(|v| v.width),
            "height": viewport.map(|v| v.height),
            "dpr": viewport.map(|v| v.dpr),
            "mobile": viewport.map(|v| v.mobile),
        }))
        .unwrap_or_default(),
    }
}

/// Format a list of sessions for display.
pub fn format_sessions(sessions: &[SessionInfo], format: OutputFormat) -> String {
    match format {
        OutputFormat::Text => {
            if sessions.is_empty() {
                return "No active sessions.".to_string();
            }
            let mut lines = Vec::with_capacity(sessions.len() + 1);
            lines.push(format!(
                "{:<12} {:<8} {:<10} {}",
                "SESSION", "PID", "HEADLESS", "WEBSOCKET"
            ));
            for s in sessions {
                lines.push(format!(
                    "{:<12} {:<8} {:<10} {}",
                    s.id, s.pid, s.headless, s.ws_url
                ));
            }
            lines.join("\n")
        }
        OutputFormat::Json => serde_json::to_string_pretty(sessions).unwrap_or_default(),
    }
}

/// Format element info for display.
pub fn format_elements(elements: &[ElementInfo], format: OutputFormat) -> String {
    match format {
        OutputFormat::Text => {
            if elements.is_empty() {
                return "No elements found.".to_string();
            }
            let mut lines = Vec::with_capacity(elements.len());
            for el in elements {
                let mut parts = vec![format!("<{}>", el.tag_name)];
                if let Some(ref text) = el.text {
                    if !text.is_empty() {
                        let truncated = if text.chars().count() > 60 {
                            let cut: String = text.chars().take(57).collect();
                            format!("{cut}...")
                        } else {
                            text.clone()
                        };
                        parts.push(format!("\"{truncated}\""));
                    }
                }
                if el.match_count > 1 {
                    parts.push(format!("[{}/{}]", el.match_index + 1, el.match_count));
                }
                if let Some(ref bb) = el.bounding_box {
                    parts.push(format!(
                        "({:.0}x{:.0} at {:.0},{:.0})",
                        bb.width, bb.height, bb.x, bb.y
                    ));
                }
                lines.push(parts.join(" "));
            }
            lines.join("\n")
        }
        OutputFormat::Json => serde_json::to_string_pretty(elements).unwrap_or_default(),
    }
}

/// Format text content for display.
pub fn format_text(texts: &[String], format: OutputFormat) -> String {
    match format {
        OutputFormat::Text => texts.join("\n"),
        OutputFormat::Json => serde_json::to_string_pretty(texts).unwrap_or_default(),
    }
}

/// Format console messages for display.
pub fn format_console(messages: &[ConsoleMessage], format: OutputFormat) -> String {
    match format {
        OutputFormat::Text => {
            if messages.is_empty() {
                return "No console messages.".to_string();
            }
            messages
                .iter()
                .map(|m| format!("[{}] {}", m.level, m.text))
                .collect::<Vec<_>>()
                .join("\n")
        }
        OutputFormat::Json => serde_json::to_string_pretty(messages).unwrap_or_default(),
    }
}

/// Format network requests for display.
pub fn format_network(requests: &[NetworkRequest], format: OutputFormat) -> String {
    match format {
        OutputFormat::Text => {
            if requests.is_empty() {
                return "No network requests.".to_string();
            }
            let mut lines = Vec::with_capacity(requests.len() + 1);
            lines.push(format!(
                "{:<6} {:<6} {:<12} {}",
                "METHOD", "STATUS", "TYPE", "URL"
            ));
            for r in requests {
                lines.push(format!(
                    "{:<6} {:<6} {:<12} {}",
                    r.method,
                    r.status.map_or("-".to_string(), |s| s.to_string()),
                    r.resource_type.as_deref().unwrap_or("-"),
                    r.url
                ));
            }
            lines.join("\n")
        }
        OutputFormat::Json => serde_json::to_string_pretty(requests).unwrap_or_default(),
    }
}

/// Format the browser's open targets for display.
pub fn format_targets(targets: &[BrowserTarget], format: OutputFormat) -> String {
    match format {
        OutputFormat::Text => {
            if targets.is_empty() {
                return "No targets.".to_string();
            }
            let mut lines = Vec::with_capacity(targets.len() + 1);
            lines.push(format!(
                "{:<16} {:<14} {:<30} {}",
                "TYPE", "STATE", "TITLE", "URL"
            ));
            for t in targets {
                // Two independent bits in one column: `active` is Chrome's
                // foreground tab, `driven` is the one khora acts on. They
                // normally coincide, and the whole point of this command is
                // seeing the case where they don't.
                let mut state = Vec::new();
                if t.active == Some(true) {
                    state.push("active");
                }
                if t.driven {
                    state.push("driven");
                }
                let state = if state.is_empty() {
                    "-".to_string()
                } else {
                    state.join(",")
                };
                let title = truncate(&t.title, 30);
                lines.push(format!(
                    "{:<16} {:<14} {:<30} {}",
                    t.target_type, state, title, t.url
                ));
            }
            lines.join("\n")
        }
        OutputFormat::Json => serde_json::to_string_pretty(targets).unwrap_or_default(),
    }
}

/// Render a single session's `status`, including the per-layer health probe.
///
/// `alive` and `health` answer different questions and both are reported:
/// `alive` is whether the Chrome process is still there, `health` is whether
/// it still answers. The failure this exists for is the pair
/// `alive: true, healthy: false` — a browser that is running and no longer
/// usable, which every other command discovers one timeout at a time.
pub fn format_status(
    info: &SessionInfo,
    alive: bool,
    health: &SessionHealth,
    format: OutputFormat,
) -> String {
    let session = &info.id;
    let ws_url = &info.ws_url;
    match format {
        OutputFormat::Text => {
            let state = if !alive {
                "dead"
            } else if health.healthy() {
                "alive"
            } else {
                "degraded"
            };
            let mut lines = vec![format!("Session {session} is {state}")];
            for (name, check) in health.layers() {
                let verdict = match check.ok {
                    Some(true) => "ok",
                    Some(false) => "UNRESPONSIVE",
                    None => "-",
                };
                let detail = match &check.detail {
                    Some(d) => format!(" ({d})"),
                    None => String::new(),
                };
                lines.push(format!(
                    "  {name:<12} {verdict:<13} {ms}ms{detail}",
                    ms = check.ms
                ));
            }
            // The WebSocket URL carries the browser's CDP port, which is what
            // anyone dropping to raw CDP for something khora has no command
            // for needs; without it here the only source is the session JSON
            // on disk.
            lines.push(format!("WebSocket: {ws_url}"));
            lines.join("\n")
        }
        OutputFormat::Json => serde_json::to_string_pretty(&serde_json::json!({
            "session": session,
            "alive": alive,
            // `healthy` is the flat one-bit answer a script wants
            // (`jq -e .healthy`); `health` is the per-layer detail that says
            // which part of the browser stopped.
            "healthy": health.healthy(),
            "health": health,
            "pid": info.pid,
            "headless": info.headless,
            "ws_url": ws_url,
        }))
        .unwrap_or_default(),
    }
}

/// Render a `reap` result: the sessions reaped, plus the orphaned data-dir
/// sweep when `--purge-data-dirs` ran (`purged` is `None` when it did not, so
/// the default output is unchanged).
pub fn format_reap(
    reaped: &[String],
    purged: Option<(usize, u64)>,
    format: OutputFormat,
) -> String {
    match format {
        OutputFormat::Text => {
            let mut out = format!("reaped {} sessions: {}", reaped.len(), reaped.join(", "));
            if let Some((dirs, bytes)) = purged {
                out.push_str(&format!(
                    "\npurged {} orphaned data dirs ({})",
                    dirs,
                    human_bytes(bytes)
                ));
            }
            out
        }
        OutputFormat::Json => {
            let mut value = serde_json::json!({ "reaped": reaped });
            if let Some((dirs, bytes)) = purged {
                value["purged_data_dirs"] = serde_json::json!(dirs);
                value["freed_bytes"] = serde_json::json!(bytes);
            }
            serde_json::to_string_pretty(&value).unwrap_or_default()
        }
    }
}

/// Format a byte count for humans (`16.0 GB`), for reporting reclaimed disk.
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Shorten `s` to at most `max` characters, on a char boundary.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(3)).collect();
    format!("{cut}...")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::BoundingBox;

    fn sample_session() -> SessionInfo {
        SessionInfo {
            id: "abc123".to_string(),
            ws_url: "ws://127.0.0.1:9222/devtools/browser/abc".to_string(),
            pid: 12345,
            headless: true,
            created_at: 1700000000,
            data_dir: None,
            media: None,
            touch: None,
            viewport: None,
            keyboard: None,
        }
    }

    #[test]
    fn test_format_touch_emulation_flags_the_pending_navigation() {
        let touch = TouchEmulation {
            max_touch_points: 5,
        };
        let out = format_touch_emulation(Some(&touch), true, OutputFormat::Text);
        assert!(out.contains("maxTouchPoints=5"));
        // The nudge is the whole usability of the command: without it a caller
        // reads "touch emulated" and wonders why the loaded page disagrees.
        assert!(out.contains("Navigate or reload"));

        // Re-asserting the state the session already has changes nothing, so
        // the loaded document is not stale and the nudge would be noise.
        let out = format_touch_emulation(Some(&touch), false, OutputFormat::Text);
        assert!(!out.contains("Navigate or reload"));
    }

    #[test]
    fn test_format_touch_emulation_json_off() {
        let out = format_touch_emulation(None, true, OutputFormat::Json);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["enabled"], serde_json::json!(false));
        assert_eq!(v["max_touch_points"], serde_json::Value::Null);
        assert_eq!(v["ontouchstart_pending"], serde_json::json!(true));
    }

    #[test]
    fn test_format_keyboard_emulation_text_reports_both_numbers() {
        let kb = KeyboardEmulation { inset: 300 };
        let out = format_keyboard_emulation(Some(&kb), 544.0, OutputFormat::Text);
        // Both numbers, because `300` alone is ambiguous between "the keyboard
        // is 300px tall" and "the viewport is now 300px tall".
        assert!(out.contains("inset=300px"), "{out}");
        assert!(out.contains("visualViewport.height=544"), "{out}");
        // The layout viewport not moving is the property callers came for and
        // the one that distinguishes this from set-viewport, so it is stated.
        assert!(out.contains("layout viewport unchanged"), "{out}");
    }

    #[test]
    fn test_format_keyboard_emulation_off_reports_restored_height() {
        let out = format_keyboard_emulation(None, 844.0, OutputFormat::Text);
        assert!(out.contains("cleared"), "{out}");
        // The restored height rides along on `--off` too: it is the read-back
        // that shows the shadow actually came off the live document.
        assert!(out.contains("844"), "{out}");
    }

    #[test]
    fn test_format_keyboard_emulation_json() {
        let kb = KeyboardEmulation { inset: 336 };
        let out = format_keyboard_emulation(Some(&kb), 508.0, OutputFormat::Json);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["action"], "emulate_keyboard");
        assert_eq!(v["enabled"], true);
        assert_eq!(v["inset"], 336);
        assert_eq!(v["visual_viewport_height"], 508.0);

        let off: serde_json::Value =
            serde_json::from_str(&format_keyboard_emulation(None, 844.0, OutputFormat::Json))
                .unwrap();
        assert_eq!(off["enabled"], false);
        assert_eq!(off["inset"], serde_json::Value::Null);
    }

    #[test]
    fn test_format_viewport_emulation_text() {
        let vp = ViewportEmulation {
            width: 390,
            height: 844,
            dpr: 3.0,
            mobile: true,
        };
        let out = format_viewport_emulation(Some(&vp), OutputFormat::Text);
        assert_eq!(out, "Viewport set: 390x844 dpr=3 (mobile)");

        // dpr 0 means "keep whatever Chrome has", so echoing it back as
        // `dpr=0` would report an override that was never asked for.
        let plain = ViewportEmulation {
            width: 390,
            height: 844,
            dpr: 0.0,
            mobile: false,
        };
        assert_eq!(
            format_viewport_emulation(Some(&plain), OutputFormat::Text),
            "Viewport set: 390x844"
        );

        assert_eq!(
            format_viewport_emulation(None, OutputFormat::Text),
            "Viewport override cleared."
        );
    }

    #[test]
    fn test_format_viewport_emulation_json_cleared() {
        let out = format_viewport_emulation(None, OutputFormat::Json);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["action"], serde_json::json!("set_viewport"));
        assert_eq!(v["enabled"], serde_json::json!(false));
        // Null rather than a stale number: a consumer must not read a width
        // off a response that says the override is gone.
        assert_eq!(v["width"], serde_json::Value::Null);
        assert_eq!(v["mobile"], serde_json::Value::Null);
    }

    #[test]
    fn test_format_session_text() {
        let output = format_session(&sample_session(), OutputFormat::Text);
        assert!(output.contains("abc123"));
        assert!(output.contains("12345"));
        assert!(output.contains("true"));
    }

    #[test]
    fn test_format_session_json() {
        let output = format_session(&sample_session(), OutputFormat::Json);
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["id"], "abc123");
    }

    #[test]
    fn test_format_sessions_empty() {
        assert_eq!(
            format_sessions(&[], OutputFormat::Text),
            "No active sessions."
        );
    }

    #[test]
    fn test_format_elements_text_empty() {
        assert_eq!(
            format_elements(&[], OutputFormat::Text),
            "No elements found."
        );
    }

    #[test]
    fn test_format_elements_truncates_multibyte_text_on_char_boundary() {
        let elements = vec![ElementInfo {
            selector: "a".to_string(),
            tag_name: "a".to_string(),
            text: Some("→".repeat(80)),
            attributes: None,
            bounding_box: None,
            visible: true,
            match_count: 1,
            match_index: 0,
        }];
        let output = format_elements(&elements, OutputFormat::Text);
        assert!(output.contains(&format!("{}...", "→".repeat(57))));
    }

    #[test]
    fn test_format_elements_text() {
        let elements = vec![ElementInfo {
            selector: "button.submit".to_string(),
            tag_name: "button".to_string(),
            text: Some("Submit".to_string()),
            attributes: None,
            bounding_box: Some(BoundingBox {
                x: 10.0,
                y: 20.0,
                width: 100.0,
                height: 40.0,
            }),
            visible: true,
            match_count: 1,
            match_index: 0,
        }];
        let output = format_elements(&elements, OutputFormat::Text);
        assert!(output.contains("<button>"));
        assert!(output.contains("\"Submit\""));
        assert!(output.contains("100x40 at 10,20"));
    }

    #[test]
    fn test_format_elements_json() {
        let elements = vec![ElementInfo {
            selector: "div".to_string(),
            tag_name: "div".to_string(),
            text: None,
            attributes: None,
            bounding_box: None,
            visible: false,
            match_count: 1,
            match_index: 0,
        }];
        let output = format_elements(&elements, OutputFormat::Json);
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert!(parsed.is_array());
        assert_eq!(parsed[0]["tag_name"], "div");
    }

    #[test]
    fn test_format_console_empty() {
        assert_eq!(
            format_console(&[], OutputFormat::Text),
            "No console messages."
        );
    }

    #[test]
    fn test_format_console_text() {
        let messages = vec![ConsoleMessage {
            level: "error".to_string(),
            text: "Uncaught TypeError".to_string(),
        }];
        let output = format_console(&messages, OutputFormat::Text);
        assert!(output.contains("[error]"));
        assert!(output.contains("Uncaught TypeError"));
    }

    #[test]
    fn test_format_network_empty() {
        assert_eq!(
            format_network(&[], OutputFormat::Text),
            "No network requests."
        );
    }

    #[test]
    fn test_format_network_text() {
        let requests = vec![NetworkRequest {
            url: "https://example.com/api".to_string(),
            method: "GET".to_string(),
            status: Some(200),
            resource_type: Some("fetch".to_string()),
        }];
        let output = format_network(&requests, OutputFormat::Text);
        assert!(output.contains("GET"));
        assert!(output.contains("200"));
        assert!(output.contains("https://example.com/api"));
    }

    fn sample_status_session() -> SessionInfo {
        SessionInfo {
            id: "abc123".to_string(),
            ws_url: "ws://127.0.0.1:9222/devtools/browser/x".to_string(),
            pid: 4242,
            headless: true,
            created_at: 0,
            data_dir: None,
            media: None,
            touch: None,
            viewport: None,
            keyboard: None,
        }
    }

    fn wedged_compositor() -> crate::element::SessionHealth {
        use crate::element::{HealthCheck, SessionHealth};
        let ok = |ms| HealthCheck {
            ok: Some(true),
            ms,
            detail: None,
        };
        SessionHealth {
            protocol: ok(4),
            renderer: ok(11),
            compositor: HealthCheck {
                ok: Some(false),
                ms: 5000,
                detail: Some("timed out after 5000ms".to_string()),
            },
        }
    }

    #[test]
    fn test_format_status_text_reports_degraded_not_alive() {
        // The pair this command exists to name: the process is up, so the old
        // `alive` bit says yes, while the browser has stopped answering.
        let out = format_status(
            &sample_status_session(),
            true,
            &wedged_compositor(),
            OutputFormat::Text,
        );
        assert!(out.contains("Session abc123 is degraded"), "got {out}");
        assert!(!out.contains("is alive"), "got {out}");
        assert!(out.contains("compositor"), "got {out}");
        assert!(out.contains("UNRESPONSIVE"), "got {out}");
        assert!(out.contains("timed out after 5000ms"), "got {out}");
        // Per-layer timings distinguish "answered" from "answered slowly".
        assert!(out.contains("11ms"), "got {out}");
        assert!(out.contains("WebSocket: ws://"), "got {out}");
    }

    #[test]
    fn test_format_status_json_keeps_preexisting_fields() {
        let out = format_status(
            &sample_status_session(),
            true,
            &wedged_compositor(),
            OutputFormat::Json,
        );
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        // `pid`, `headless` and `ws_url` predate the health probe; adding
        // health must not silently drop what scripts already read.
        assert_eq!(v["session"], "abc123");
        assert_eq!(v["pid"], 4242);
        assert_eq!(v["headless"], true);
        assert_eq!(v["ws_url"], "ws://127.0.0.1:9222/devtools/browser/x");
        assert_eq!(v["alive"], true);
        assert_eq!(v["healthy"], false);
        assert_eq!(v["health"]["compositor"]["ok"], false);
        assert_eq!(v["health"]["renderer"]["ok"], true);
        // A green layer carries no detail, so `jq` sees a clean object.
        assert!(v["health"]["renderer"].get("detail").is_none());
    }

    #[test]
    fn test_format_status_text_healthy_says_alive() {
        use crate::element::{HealthCheck, SessionHealth};
        let ok = || HealthCheck {
            ok: Some(true),
            ms: 2,
            detail: None,
        };
        let health = SessionHealth {
            protocol: ok(),
            renderer: ok(),
            compositor: ok(),
        };
        let out = format_status(&sample_status_session(), true, &health, OutputFormat::Text);
        assert!(out.contains("Session abc123 is alive"), "got {out}");
        assert!(!out.contains("UNRESPONSIVE"), "got {out}");
    }

    fn sample_target(title: &str, active: Option<bool>, driven: bool) -> BrowserTarget {
        BrowserTarget {
            target_id: format!("T-{title}"),
            target_type: "page".to_string(),
            url: format!("https://example.com/{title}"),
            title: title.to_string(),
            active,
            driven,
        }
    }

    #[test]
    fn test_format_targets_empty() {
        assert_eq!(format_targets(&[], OutputFormat::Text), "No targets.");
    }

    #[test]
    fn test_format_targets_text_marks_active_and_driven() {
        let targets = vec![
            sample_target("stray", Some(true), false),
            sample_target("app", Some(false), true),
        ];
        let output = format_targets(&targets, OutputFormat::Text);
        let lines: Vec<&str> = output.lines().collect();
        assert!(lines[0].contains("STATE"));
        assert!(lines[1].contains("active") && !lines[1].contains("driven"));
        assert!(lines[2].contains("driven") && !lines[2].contains("active"));
    }

    #[test]
    fn test_format_targets_text_combines_both_flags() {
        let output = format_targets(
            &[sample_target("app", Some(true), true)],
            OutputFormat::Text,
        );
        assert!(output.contains("active,driven"));
    }

    /// Non-page targets can't report visibility; they must not read as
    /// "backgrounded", which is a diagnosis, not an absence of data.
    #[test]
    fn test_format_targets_text_unprobeable_shows_dash() {
        let mut t = sample_target("sw", None, false);
        t.target_type = "service_worker".to_string();
        let output = format_targets(&[t], OutputFormat::Text);
        assert!(output.lines().nth(1).unwrap().contains(" - "));
    }

    #[test]
    fn test_format_targets_truncates_multibyte_title_on_char_boundary() {
        let mut t = sample_target("x", Some(true), false);
        t.title = "→".repeat(80);
        let output = format_targets(&[t], OutputFormat::Text);
        assert!(output.contains(&format!("{}...", "→".repeat(27))));
    }

    #[test]
    fn test_format_targets_json() {
        let output = format_targets(
            &[sample_target("app", Some(false), true)],
            OutputFormat::Json,
        );
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed[0]["type"], "page");
        assert_eq!(parsed[0]["active"], false);
        assert_eq!(parsed[0]["driven"], true);
    }

    #[test]
    fn test_format_text_items() {
        let texts = vec!["Hello".to_string(), "World".to_string()];
        assert_eq!(format_text(&texts, OutputFormat::Text), "Hello\nWorld");
    }

    #[test]
    fn test_format_text_json() {
        let texts = vec!["Hello".to_string()];
        let output = format_text(&texts, OutputFormat::Json);
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert!(parsed.is_array());
        assert_eq!(parsed[0], "Hello");
    }

    #[test]
    fn test_format_reap_without_purge_is_unchanged() {
        // --purge-data-dirs is opt-in: the default output must not grow a line
        // or a JSON key, since callers already parse this.
        let reaped = vec!["abc123".to_string()];
        assert_eq!(
            format_reap(&reaped, None, OutputFormat::Text),
            "reaped 1 sessions: abc123"
        );
        let parsed: serde_json::Value =
            serde_json::from_str(&format_reap(&reaped, None, OutputFormat::Json)).unwrap();
        assert_eq!(parsed["reaped"][0], "abc123");
        assert!(parsed.get("purged_data_dirs").is_none());
        assert!(parsed.get("freed_bytes").is_none());
    }

    #[test]
    fn test_format_reap_with_purge_reports_dirs_and_size() {
        let reaped: Vec<String> = vec![];
        let text = format_reap(&reaped, Some((283, 17_179_869_184)), OutputFormat::Text);
        assert_eq!(
            text,
            "reaped 0 sessions: \npurged 283 orphaned data dirs (16.0 GB)"
        );

        let parsed: serde_json::Value =
            serde_json::from_str(&format_reap(&reaped, Some((2, 1024)), OutputFormat::Json))
                .unwrap();
        assert_eq!(parsed["purged_data_dirs"], 2);
        assert_eq!(parsed["freed_bytes"], 1024);
    }

    #[test]
    fn test_human_bytes_scales_units() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(1024), "1.0 KB");
        assert_eq!(human_bytes(8_704_000), "8.3 MB");
        assert_eq!(human_bytes(17_179_869_184), "16.0 GB");
    }
}
