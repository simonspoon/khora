use crate::element::{BrowserTarget, ConsoleMessage, ElementInfo, NetworkRequest};
use crate::session::{MediaEmulation, SessionInfo, TouchEmulation, ViewportEmulation};

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
}
