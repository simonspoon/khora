use crate::element::{ConsoleMessage, ElementInfo, NetworkRequest};
use crate::session::SessionInfo;

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
                        let truncated = if text.len() > 60 {
                            format!("{}...", &text[..57])
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
        }
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
