use serde::{Deserialize, Serialize};

/// Information about a DOM element found via CSS selector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementInfo {
    /// CSS selector used to find this element.
    pub selector: String,
    /// Tag name (e.g., "div", "button", "input").
    pub tag_name: String,
    /// Inner text content, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Element attributes as key-value pairs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attributes: Option<serde_json::Value>,
    /// Bounding box in viewport coordinates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounding_box: Option<BoundingBox>,
    /// Whether the element is visible.
    #[serde(default)]
    pub visible: bool,
    /// Number of elements matching the selector.
    pub match_count: usize,
    /// Index of this element among matches (0-based).
    pub match_index: usize,
}

/// Bounding box of an element in viewport coordinates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundingBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A captured network request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkRequest {
    pub url: String,
    pub method: String,
    pub status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<String>,
}

/// A console message from the browser.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleMessage {
    pub level: String,
    pub text: String,
}

/// An open browser target (tab, worker, ...) as reported by `Target.getTargets`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserTarget {
    /// CDP target id.
    pub target_id: String,
    /// Target type: "page", "iframe", "service_worker", "worker", ...
    #[serde(rename = "type")]
    pub target_type: String,
    pub url: String,
    pub title: String,
    /// Whether this is the foreground tab of its window
    /// (`document.visibilityState === "visible"`). `None` for targets that
    /// cannot be probed — non-page types, and pages whose evaluate timed out.
    pub active: Option<bool>,
    /// Whether khora's other commands would operate on this target. Exactly
    /// one target is driven; a `driven` that isn't `active` is the shape of
    /// the background-tab bug fixed in 1f24769.
    pub driven: bool,
}

/// One layer's answer in a session health probe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheck {
    /// `Some(true)` answered, `Some(false)` did not, `None` not applicable —
    /// e.g. a browser with no page open, which is idle rather than wedged and
    /// must not be reported as unhealthy.
    pub ok: Option<bool>,
    /// Wall-clock the probe took, in milliseconds. A layer can answer *and*
    /// be sick; the number is what shows it.
    pub ms: u64,
    /// Why `ok` is not `Some(true)`: the CDP error, "timed out after Nms", or
    /// the reason the layer was skipped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl HealthCheck {
    /// A layer that was never probed, and so cannot condemn the session.
    pub fn skipped(reason: &str) -> Self {
        Self {
            ok: None,
            ms: 0,
            detail: Some(reason.to_string()),
        }
    }
}

/// How responsive a session's browser is, layer by layer.
///
/// Three layers rather than one because the incident this answers (mesa task
/// 557 / 568) wedged exactly one of them: `eval`, `click-at` and `wait-for`
/// all worked on the degraded session while `wheel` timed out at both 5s and
/// 15s. A single `Runtime.evaluate` health check — the obvious one — would
/// have reported that session green. The layers are ordered by how much of
/// the browser they need, so the first `false` names the deepest thing still
/// working.
///
/// How sharply they actually separate, measured rather than assumed:
///
/// - **protocol vs the rest** — reproduced. A page busy-looping its main
///   thread leaves `Browser.getVersion` answering in ~0ms while both
///   page-scoped layers time out.
/// - **renderer vs compositor** — *not* reproduced, and they collapse on a
///   JS-thread wedge, because resolving a page reaches into the renderer and
///   blocks before `evaluate` runs. The split still earns its place: it is
///   the only shape that can express the reported incident, where `eval`
///   answered and only the frame-producing path was gone. Chromium's
///   compositor runs on its own thread, so the state is real; khora has
///   simply never reproduced it on demand — 40 timed cold launches in mesa
///   task 569 never did either.
///
/// Read a `renderer` failure, therefore, as "the renderer could not be
/// reached", not as "`evaluate` specifically was tried and failed" — the
/// `detail` string says which.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionHealth {
    /// The browser process answers CDP at all (`Browser.getVersion`). This is
    /// the only layer that needs no page, so it stays `Some` even on a
    /// browser with no tabs.
    pub protocol: HealthCheck,
    /// The driven page's JS thread answers (`Runtime.evaluate`). A page
    /// busy-looping its main thread fails here while `protocol` stays green.
    pub renderer: HealthCheck,
    /// The compositor still produces frames (a 1x1 `Page.captureScreenshot`,
    /// the same nudge the trusted-input commands use). This is the layer
    /// `wheel` needs and `eval` does not — see the type-level note.
    pub compositor: HealthCheck,
}

impl SessionHealth {
    /// True unless some layer was probed and failed. A `None` layer is
    /// "nothing to probe", never a verdict.
    pub fn healthy(&self) -> bool {
        [&self.protocol, &self.renderer, &self.compositor]
            .iter()
            .all(|c| c.ok != Some(false))
    }

    /// The layers in probe order, for formatting.
    pub fn layers(&self) -> [(&'static str, &HealthCheck); 3] {
        [
            ("protocol", &self.protocol),
            ("renderer", &self.renderer),
            ("compositor", &self.compositor),
        ]
    }

    /// One-line reason the session is unhealthy, naming every failed layer.
    /// `None` when nothing failed.
    pub fn failure_summary(&self) -> Option<String> {
        let failed: Vec<String> = self
            .layers()
            .iter()
            .filter(|(_, c)| c.ok == Some(false))
            .map(|(name, c)| match &c.detail {
                Some(d) => format!("{name} unresponsive ({d})"),
                None => format!("{name} unresponsive"),
            })
            .collect();
        if failed.is_empty() {
            None
        } else {
            Some(failed.join("; "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(ok: Option<bool>) -> HealthCheck {
        HealthCheck {
            ok,
            ms: 3,
            detail: ok.map_or(Some("skipped".to_string()), |v| {
                (!v).then(|| "timed out after 5000ms".to_string())
            }),
        }
    }

    fn health(
        protocol: Option<bool>,
        renderer: Option<bool>,
        compositor: Option<bool>,
    ) -> SessionHealth {
        SessionHealth {
            protocol: check(protocol),
            renderer: check(renderer),
            compositor: check(compositor),
        }
    }

    #[test]
    fn test_health_all_green_is_healthy() {
        let h = health(Some(true), Some(true), Some(true));
        assert!(h.healthy());
        assert_eq!(h.failure_summary(), None);
    }

    #[test]
    fn test_health_compositor_only_failure_is_unhealthy() {
        // The incident this exists for: the JS thread answered fine and only
        // the compositor was gone, so a probe that stopped at `renderer`
        // would have called this session healthy.
        let h = health(Some(true), Some(true), Some(false));
        assert!(!h.healthy());
        let why = h.failure_summary().unwrap();
        assert!(why.contains("compositor unresponsive"), "got {why}");
        assert!(!why.contains("renderer"), "got {why}");
    }

    #[test]
    fn test_health_unprobed_layer_is_not_a_failure() {
        // A browser with no page open is idle, not sick — `None` must never
        // condemn the session, or `--check` fails a session a `navigate`
        // would revive.
        let h = health(Some(true), None, None);
        assert!(h.healthy());
        assert_eq!(h.failure_summary(), None);
    }

    #[test]
    fn test_health_summary_names_every_failed_layer() {
        let h = health(Some(true), Some(false), Some(false));
        let why = h.failure_summary().unwrap();
        assert!(why.contains("renderer unresponsive"), "got {why}");
        assert!(why.contains("compositor unresponsive"), "got {why}");
        // The detail is what makes the exit code actionable rather than just
        // a red light.
        assert!(why.contains("timed out after 5000ms"), "got {why}");
    }

    #[test]
    fn test_health_layers_are_in_probe_order() {
        let names: Vec<&str> = health(Some(true), Some(true), Some(true))
            .layers()
            .iter()
            .map(|(n, _)| *n)
            .collect();
        assert_eq!(names, ["protocol", "renderer", "compositor"]);
    }

    #[test]
    fn test_element_info_roundtrip() {
        let el = ElementInfo {
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
        };
        let json = serde_json::to_string(&el).unwrap();
        let parsed: ElementInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.selector, "button.submit");
        assert_eq!(parsed.tag_name, "button");
        assert_eq!(parsed.text.as_deref(), Some("Submit"));
        assert!(parsed.visible);
    }

    #[test]
    fn test_element_info_omits_none_fields() {
        let el = ElementInfo {
            selector: "div".to_string(),
            tag_name: "div".to_string(),
            text: None,
            attributes: None,
            bounding_box: None,
            visible: false,
            match_count: 1,
            match_index: 0,
        };
        let json = serde_json::to_string(&el).unwrap();
        assert!(!json.contains("text"));
        assert!(!json.contains("attributes"));
        assert!(!json.contains("bounding_box"));
    }

    #[test]
    fn test_network_request_roundtrip() {
        let req = NetworkRequest {
            url: "https://example.com/api".to_string(),
            method: "GET".to_string(),
            status: Some(200),
            resource_type: Some("fetch".to_string()),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: NetworkRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.status, Some(200));
    }

    #[test]
    fn test_console_message_roundtrip() {
        let msg = ConsoleMessage {
            level: "error".to_string(),
            text: "Uncaught TypeError".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: ConsoleMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.level, "error");
    }

    #[test]
    fn test_bounding_box_roundtrip() {
        let bb = BoundingBox {
            x: 10.5,
            y: 20.0,
            width: 800.0,
            height: 600.0,
        };
        let json = serde_json::to_string(&bb).unwrap();
        let parsed: BoundingBox = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.x, 10.5);
        assert_eq!(parsed.width, 800.0);
    }
}
