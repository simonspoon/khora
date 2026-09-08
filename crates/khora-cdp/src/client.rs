use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::browser::GetVersionParams;
use chromiumoxide::cdp::browser_protocol::emulation::{
    ClearDeviceMetricsOverrideParams, MediaFeature, SetDeviceMetricsOverrideParams,
    SetEmulatedMediaParams, SetTouchEmulationEnabledParams,
};
use chromiumoxide::cdp::browser_protocol::input::{
    DispatchKeyEventParams, DispatchKeyEventType, DispatchMouseEventParams, DispatchMouseEventType,
    DispatchTouchEventReturns, DispatchTouchEventType, InsertTextParams, MouseButton, TouchPoint,
};
use chromiumoxide::cdp::browser_protocol::network::{
    EnableParams as NetworkEnableParams, SetCacheDisabledParams,
};
use chromiumoxide::cdp::browser_protocol::page::{
    AddScriptToEvaluateOnNewDocumentParams, CaptureScreenshotFormat,
    EnableParams as PageEnableParams, Viewport,
};
use chromiumoxide::cdp::browser_protocol::target::GetTargetsParams;
use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
use chromiumoxide::page::ScreenshotParams;
use chromiumoxide::Page;
use futures::StreamExt;
use khora_core::element::{
    BoundingBox, BrowserTarget, ConsoleMessage, ElementInfo, HealthCheck, NetworkRequest,
    SessionHealth,
};
use khora_core::error::{KhoraError, KhoraResult};
use khora_core::session::{
    KeyboardEmulation, MediaEmulation, SessionInfo, TouchEmulation, ViewportEmulation,
};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::chrome::find_chrome;

/// `Input.dispatchTouchEvent` params that always put `touchPoints` on the
/// wire.
///
/// chromiumoxide's generated `DispatchTouchEventParams` marks that field
/// `skip_serializing_if = "Vec::is_empty"`, but CDP requires the parameter
/// and defines `touchEnd`/`touchCancel` as carrying *no* points — so the
/// release closing every gesture serialized with the field missing entirely
/// and came back `-32602 Invalid parameters` (measured against Chrome, not
/// inferred: the touchStart and touchMoves of the same swipe all landed and
/// scrolled the page, only the release failed). Reusing the generated event
/// type and response keeps this to a serialization override.
#[derive(Debug, serde::Serialize)]
struct DispatchTouchEvent {
    #[serde(rename = "type")]
    kind: DispatchTouchEventType,
    #[serde(rename = "touchPoints")]
    touch_points: Vec<TouchPoint>,
}

impl DispatchTouchEvent {
    const IDENTIFIER: &'static str = "Input.dispatchTouchEvent";
}

impl chromiumoxide::Method for DispatchTouchEvent {
    fn identifier(&self) -> chromiumoxide::types::MethodId {
        Self::IDENTIFIER.into()
    }
}

impl chromiumoxide::MethodType for DispatchTouchEvent {
    fn method_id() -> chromiumoxide::types::MethodId {
        Self::IDENTIFIER.into()
    }
}

impl chromiumoxide::Command for DispatchTouchEvent {
    type Response = DispatchTouchEventReturns;
}

/// Where an element command runs its JS and dispatches its input.
///
/// A frame reached with `--frame` is one of two things, and they need
/// different CDP handling.
///
/// A cross-origin frame is an out-of-process iframe (OOPIF) — its own CDP
/// target, which chromiumoxide will not drive: its target type is `iframe`,
/// `Target::is_page()` is false, and `Handler` skips polling non-page targets
/// entirely, so a `Page` obtained via `Browser::get_page()` for one queues
/// commands that are never sent and every call on it hangs forever. Chrome
/// lists the OOPIF in `/json/list` with its own `webSocketDebuggerUrl`
/// though, so khora opens a second connection straight to that target and
/// talks to it with no session routing at all.
///
/// A same-origin frame is not a target at all — it shares the page's. CDP
/// would reach it by evaluating in its execution context, but chromiumoxide
/// only learns child frames from live `Page.frameAttached` events, and khora
/// reconnects to an already-loaded page, so its frame list holds nothing but
/// the main frame and `frame_execution_context()` has nothing to return.
/// Reaching it through the page's own JS instead costs nothing: the frame is
/// same-origin, so `contentDocument` is readable, and the element JS runs
/// unchanged against a rebound `document`/`window`.
struct FrameContext {
    /// The page target, always. Input dispatched at the browser level and
    /// the post-input compositor nudge go here even for an OOPIF.
    page: Page,
    /// The frame match, when the frame is a same-origin one reached through
    /// the page's JS.
    same_origin: Option<String>,
    /// A direct connection to an OOPIF target, when the frame is one.
    frame: Option<FrameConnection>,
    /// The frame match this was resolved from, for error messages.
    label: Option<String>,
}

/// A dedicated connection to an OOPIF's own DevTools endpoint, plus the task
/// pumping its event handler — which must outlive the commands sent on it.
struct FrameConnection {
    browser: Browser,
    _handler_handle: tokio::task::JoinHandle<()>,
}

impl FrameContext {
    /// Open a dedicated connection to an OOPIF's own DevTools endpoint.
    ///
    /// Bounded for the same reason `CdpClient::connect` is: a wedged Chrome
    /// leaves the socket open but unresponsive, and an unbounded handshake
    /// here would hang a command that was given an explicit `--timeout`.
    async fn connect(ws_url: &str, timeout_ms: u64) -> KhoraResult<FrameConnection> {
        let (browser, mut handler) = tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            Browser::connect(ws_url),
        )
        .await
        .map_err(|_| KhoraError::Timeout(timeout_ms))?
        .map_err(|e| KhoraError::Cdp(format!("{ws_url}: {e}")))?;
        let handler_handle = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                if event.is_err() {
                    break;
                }
            }
        });
        Ok(FrameConnection {
            browser,
            _handler_handle: handler_handle,
        })
    }

    /// A JS expression for the same-origin frame window whose URL contains
    /// `needle`, or `null`. Cross-origin children are skipped — reading their
    /// location throws, and they are reached as OOPIF targets instead.
    fn frame_window_js(needle: &str) -> String {
        format!(
            r#"
            (() => {{
                const needle = {needle};
                const search = (w) => {{
                    for (const child of Array.from(w.frames)) {{
                        let href;
                        try {{ href = child.location.href; }} catch (e) {{ continue; }}
                        if (href.includes(needle)) return child;
                        const found = search(child);
                        if (found) return found;
                    }}
                    return null;
                }};
                return search(window);
            }})()
            "#,
            needle = serde_json::to_string(needle).unwrap_or_default()
        )
    }

    /// Evaluate `js` in this frame and decode the result, the way every
    /// element op here expects it.
    async fn evaluate(&self, js: String) -> KhoraResult<serde_json::Value> {
        // A same-origin frame runs the caller's JS unchanged, with `document`
        // and `window` bound to the frame's — the element ops reach for both.
        let js = match &self.same_origin {
            Some(needle) => format!(
                r#"
                (() => {{
                    const frame = {finder};
                    if (!frame) return {{ found: false, error: "frame is gone" }};
                    // Rebound as parameters, not `const`s: the finder above
                    // reads the page's own `window`, which a `const window`
                    // in the same scope would put in the temporal dead zone.
                    return ((window, document) => {js})(frame, frame.document);
                }})()
                "#,
                finder = Self::frame_window_js(needle)
            ),
            None => js,
        };

        let params = EvaluateParams::builder()
            .expression(js)
            .return_by_value(true)
            .build()
            .map_err(KhoraError::Cdp)?;

        let value = match &self.frame {
            Some(frame) => frame
                .browser
                .execute(params)
                .await
                .map_err(|e| KhoraError::Cdp(e.to_string()))?
                .result
                .result
                .value
                .clone(),
            // `evaluate_expression`, not `evaluate`: the latter first awaits
            // the page's own execution context, a round trip we don't need.
            None => self
                .page
                .evaluate_expression(params)
                .await
                .map_err(|e| KhoraError::Cdp(e.to_string()))?
                .object()
                .value
                .clone(),
        };

        Ok(value.unwrap_or(serde_json::Value::Null))
    }

    /// Send a CDP command to this frame, discarding the response.
    ///
    /// Input goes to the OOPIF's own target when there is one, and to the
    /// page otherwise — including for a same-origin frame, whose focused
    /// element the page's input pipeline already routes to.
    async fn execute<T: chromiumoxide::Command>(&self, cmd: T) -> KhoraResult<()> {
        match &self.frame {
            Some(frame) => frame
                .browser
                .execute(cmd)
                .await
                .map(|_| ())
                .map_err(|e| KhoraError::Cdp(e.to_string())),
            None => self
                .page
                .execute(cmd)
                .await
                .map(|_| ())
                .map_err(|e| KhoraError::Cdp(e.to_string())),
        }
    }

    /// A selector description for errors, naming the frame when there is one.
    fn describe(&self, selector: &str) -> String {
        match &self.label {
            Some(frame) => format!("{selector} (in frame {frame})"),
            None => selector.to_string(),
        }
    }
}

/// High-level CDP client wrapping chromiumoxide Browser.
pub struct CdpClient {
    browser: Browser,
    _handler_handle: tokio::task::JoinHandle<()>,
    /// The browser-level DevTools websocket URL this client is connected on.
    /// Kept so `frame_context()` can derive a per-target endpoint from it.
    ws_url: String,
    /// Chrome user data directory owned by this session. Carried by `connect()`
    /// as well as `launch()` so that `close()` can remove it on every path —
    /// `kill`/`reap` always reach Chrome via `connect()`, and leaving this
    /// `None` there made `close()`'s cleanup a silent no-op that leaked the
    /// whole profile (tens of MB per launch) on the graceful path.
    ///
    /// Only `close()` acts on it, and only `kill`/`reap` call `close()`, so an
    /// ordinary command's client can hold the path and drop it harmlessly.
    data_dir: Option<PathBuf>,
    pid: u32,
    /// Media emulation carried by the session, re-applied to each page this
    /// client resolves — Chrome drops it when the previous command disconnected.
    media: Option<MediaEmulation>,
    /// Touch emulation carried by the session, re-applied for the same reason.
    touch: Option<TouchEmulation>,
    /// Viewport metrics carried by the session, re-applied for the same reason.
    viewport: Option<ViewportEmulation>,
    /// On-screen-keyboard emulation carried by the session, re-applied for the
    /// same reason — its page-side install rides on a CDP-session-scoped
    /// `Page.addScriptToEvaluateOnNewDocument`, which dies with the connection.
    keyboard: Option<KeyboardEmulation>,
}

impl CdpClient {
    /// Launch a new Chrome instance and return a client + session info.
    pub async fn launch(
        headless: bool,
        window_size: (u32, u32),
    ) -> KhoraResult<(Self, SessionInfo)> {
        let chrome_path = find_chrome()?;
        tracing::info!(?chrome_path, headless, ?window_size, "launching Chrome");

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let data_dir = std::env::temp_dir().join(format!("khora-chrome-{ts}"));

        std::fs::create_dir_all(&data_dir)
            .map_err(|e| KhoraError::LaunchFailed(format!("failed to create data dir: {e}")))?;

        let mut builder = BrowserConfig::builder()
            .chrome_executable(chrome_path)
            .user_data_dir(&data_dir)
            .window_size(window_size.0, window_size.1)
            .arg("--disable-extensions")
            .arg("--disable-default-apps")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-background-timer-throttling")
            .arg("--disable-backgrounding-occluded-windows");

        if !headless {
            builder = builder.with_head();
        }

        let config = builder
            .build()
            .map_err(|e| KhoraError::LaunchFailed(e.to_string()))?;

        let (mut browser, mut handler) = Browser::launch(config)
            .await
            .map_err(|e| KhoraError::LaunchFailed(e.to_string()))?;

        let ws_url = browser.websocket_address().to_string();

        let handler_handle = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                if event.is_err() {
                    break;
                }
            }
        });

        // Chrome's initial TargetCreated event may not have been received yet,
        // so pages() would return empty — causing get_or_create_page() to create
        // a second blank tab (the double-tab bug). fetch_targets() forces the
        // handler to sync its target list. Same pattern as connect().
        let _ = browser.fetch_targets().await;
        // Fragile time-based guard; see upstream chromiumoxide fetch_targets docs.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Capture Chrome's OS PID via the child handle (best effort — 0 means unknown)
        let pid = get_browser_pid(&mut browser);

        let session_id = SessionInfo::generate_id();
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let session = SessionInfo {
            id: session_id,
            ws_url,
            pid,
            headless,
            created_at,
            data_dir: Some(data_dir.clone()),
            media: None,
            touch: None,
            viewport: None,
            keyboard: None,
        };

        let client = Self {
            browser,
            _handler_handle: handler_handle,
            ws_url: session.ws_url.clone(),
            data_dir: Some(data_dir),
            pid,
            media: None,
            touch: None,
            viewport: None,
            keyboard: None,
        };

        Ok((client, session))
    }

    /// Connect to an existing Chrome instance using session info.
    /// Fetches existing targets so that pages from previous sessions are visible.
    ///
    /// A wedged Chrome (e.g. a hung compositor) can leave the websocket open
    /// but unresponsive, so both the initial handshake and the target fetch
    /// are bounded by `timeout_ms` — otherwise every command, including
    /// `kill`, could hang indefinitely trying to reach it.
    pub async fn connect(session: &SessionInfo, timeout_ms: u64) -> KhoraResult<Self> {
        tracing::info!(session_id = %session.id, ws_url = %session.ws_url, "connecting to Chrome");

        let timeout = std::time::Duration::from_millis(timeout_ms);

        let (mut browser, mut handler) =
            tokio::time::timeout(timeout, Browser::connect(&session.ws_url))
                .await
                .map_err(|_| KhoraError::Timeout(timeout_ms))?
                .map_err(|e| KhoraError::SessionDead(format!("{}: {e}", session.id)))?;

        let handler_handle = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                if event.is_err() {
                    break;
                }
            }
        });

        // Fetch existing targets so pages from previous connections are visible
        let _ = tokio::time::timeout(timeout, browser.fetch_targets()).await;
        // Give a moment for pages to be registered internally
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        Ok(Self {
            browser,
            _handler_handle: handler_handle,
            ws_url: session.ws_url.clone(),
            data_dir: session.data_dir.clone(),
            pid: session.pid,
            media: session.media.clone().filter(|m| !m.is_empty()),
            touch: session.touch,
            viewport: session.viewport,
            keyboard: session.keyboard,
        })
    }

    /// Navigate to a URL in the current page.
    /// Uses page.goto() which sends CDP Page.navigate, updating the target URL
    /// so subsequent connections can find the page. Falls back to JS-based
    /// navigation if goto() hangs (lifecycle events may not fire on reconnected
    /// sessions).
    pub async fn navigate(&self, url: &str, no_cache: bool) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;

        if no_cache {
            // setCacheDisabled only applies while the Network domain is enabled
            // on this CDP session, so enable it first. The state ends when this
            // invocation disconnects — enough to cover the navigation below.
            page.execute(NetworkEnableParams::default())
                .await
                .map_err(|e| KhoraError::Cdp(e.to_string()))?;
            page.execute(SetCacheDisabledParams::new(true))
                .await
                .map_err(|e| KhoraError::Cdp(e.to_string()))?;
        }

        // Try CDP Page.navigate via goto() with a timeout
        let goto_result =
            tokio::time::timeout(std::time::Duration::from_secs(10), page.goto(url)).await;

        match goto_result {
            Ok(Ok(_)) => {} // CDP navigation succeeded
            Ok(Err(e)) => {
                return Err(KhoraError::NavigationFailed(e.to_string()));
            }
            Err(_) => {
                // goto() timed out (lifecycle events didn't fire).
                // The page already navigated via CDP, but chromiumoxide is stuck
                // waiting for load events. Poll readyState ourselves.
                tracing::debug!("goto() timed out, polling readyState");
                let start = std::time::Instant::now();
                let timeout = std::time::Duration::from_secs(10);
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    let ready = page
                        .evaluate("document.readyState")
                        .await
                        .ok()
                        .and_then(|r| r.into_value::<String>().ok())
                        .unwrap_or_default();
                    if ready == "complete" || ready == "interactive" {
                        break;
                    }
                    if start.elapsed() >= timeout {
                        return Err(KhoraError::NavigationFailed(
                            "timed out waiting for page load".to_string(),
                        ));
                    }
                }
            }
        }
        // Reinstall hooks — navigation replaces the page context
        let _ = self.install_console_hook().await;
        let _ = self.install_network_hook().await;
        Ok(())
    }

    /// Find elements matching a CSS selector.
    pub async fn find_elements(
        &self,
        selector: &str,
        frame: Option<&str>,
        timeout_ms: u64,
    ) -> KhoraResult<Vec<ElementInfo>> {
        let ctx = self.frame_context(frame, timeout_ms).await?;

        // Use JavaScript to gather element info since chromiumoxide's Element
        // doesn't expose all the fields we need directly
        let js = format!(
            r#"
            (() => {{
                let els;
                try {{
                    els = document.querySelectorAll({selector});
                }} catch (e) {{
                    return {{ error: e.message }};
                }}
                const count = els.length;
                return Array.from(els).map((el, i) => {{
                    const rect = el.getBoundingClientRect();
                    return {{
                        tag_name: el.tagName.toLowerCase(),
                        text: el.textContent?.trim()?.substring(0, 200) || null,
                        visible: rect.width > 0 && rect.height > 0,
                        bounding_box: rect.width > 0 ? {{
                            x: rect.x,
                            y: rect.y,
                            width: rect.width,
                            height: rect.height
                        }} : null,
                        match_count: count,
                        match_index: i
                    }};
                }});
            }})()
            "#,
            selector = serde_json::to_string(selector).unwrap_or_default()
        );

        let value = ctx.evaluate(js).await?;

        let elements: Vec<serde_json::Value> = match value {
            serde_json::Value::Array(arr) => arr,
            serde_json::Value::Object(ref obj) => {
                if let Some(err) = obj.get("error").and_then(|v| v.as_str()) {
                    return Err(KhoraError::JavaScriptError(format!(
                        "invalid selector {selector:?}: {err}"
                    )));
                }
                return Err(KhoraError::ElementNotFound(ctx.describe(selector)));
            }
            _ => return Err(KhoraError::ElementNotFound(ctx.describe(selector))),
        };

        if elements.is_empty() {
            return Err(KhoraError::ElementNotFound(ctx.describe(selector)));
        }

        let mut result = Vec::with_capacity(elements.len());
        for el in elements {
            let bb = el.get("bounding_box").and_then(|v| {
                if v.is_null() {
                    None
                } else {
                    serde_json::from_value::<BoundingBox>(v.clone()).ok()
                }
            });

            result.push(ElementInfo {
                selector: selector.to_string(),
                tag_name: el["tag_name"].as_str().unwrap_or("unknown").to_string(),
                text: el["text"].as_str().map(|s| s.to_string()),
                attributes: None,
                bounding_box: bb,
                visible: el["visible"].as_bool().unwrap_or(false),
                match_count: el["match_count"].as_u64().unwrap_or(0) as usize,
                match_index: el["match_index"].as_u64().unwrap_or(0) as usize,
            });
        }

        Ok(result)
    }

    /// Click an element matching a CSS selector.
    /// Uses JS-based click to avoid chromiumoxide element methods hanging
    /// on reconnected sessions.
    pub async fn click(
        &self,
        selector: &str,
        frame: Option<&str>,
        timeout_ms: u64,
    ) -> KhoraResult<()> {
        let ctx = self.frame_context(frame, timeout_ms).await?;
        let js = format!(
            r#"
            (() => {{
                let el;
                try {{
                    el = document.querySelector({selector});
                }} catch (e) {{
                    return {{ found: false, error: e.message }};
                }}
                if (!el) return {{ found: false }};
                el.click();
                return {{ found: true }};
            }})()
            "#,
            selector = serde_json::to_string(selector).unwrap_or_default()
        );

        let value = ctx.evaluate(js).await?;

        if value.get("found").and_then(|v| v.as_bool()) != Some(true) {
            if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
                return Err(KhoraError::JavaScriptError(format!(
                    "invalid selector {selector:?}: {err}"
                )));
            }
            return Err(KhoraError::ElementNotFound(ctx.describe(selector)));
        }
        Ok(())
    }

    /// Type text into an element matching a CSS selector.
    /// Uses JS-based focus + value assignment to avoid chromiumoxide element
    /// methods hanging on reconnected sessions. Sets the value through the
    /// element's native prototype setter (not the plain `el.value =`
    /// assignment) so React's instance-level value tracker sees a real
    /// change and fires onChange, then dispatches input/change events so
    /// frameworks (React, Vue, etc.) pick up the value.
    ///
    /// This silently no-ops on canvas/WebGL-rendered widgets that manage
    /// their own key handling off a hidden textarea and never read `.value`
    /// (xterm.js and similar terminal emulators) — no error, no visible
    /// effect, since the `input`/`change` events dispatched here have no
    /// listener. Use [`Self::type_keys`] for those.
    ///
    /// On a page nothing has interacted with yet — `document.hasFocus()`
    /// false — it also dispatches **no focus events**: the `el.focus()`
    /// below moves `activeElement` without the browser firing
    /// `focus`/`focusin`, so a later `blur()` fires nothing either. Anything
    /// gated on focus/blur (commit-on-blur, validate-on-blur) never runs even
    /// though the value, and any framework state derived from it, reads back
    /// correctly. This is stateful: any trusted input event (`click`, `key`,
    /// `type_keys`) focuses the document, after which this method does fire
    /// focus/blur, while a `navigate` resets it. Use [`Self::type_keys`] when
    /// focus/blur behavior matters.
    pub async fn type_text(
        &self,
        selector: &str,
        text: &str,
        frame: Option<&str>,
        timeout_ms: u64,
    ) -> KhoraResult<()> {
        let ctx = self.frame_context(frame, timeout_ms).await?;
        let js = format!(
            r#"
            (() => {{
                let el;
                try {{
                    el = document.querySelector({selector});
                }} catch (e) {{
                    return {{ found: false, error: e.message }};
                }}
                if (!el) return {{ found: false }};
                el.focus();
                const proto = el.tagName === 'TEXTAREA'
                    ? window.HTMLTextAreaElement.prototype
                    : window.HTMLInputElement.prototype;
                const setter = Object.getOwnPropertyDescriptor(proto, 'value')?.set;
                if (setter) {{
                    setter.call(el, {text});
                }} else {{
                    el.value = {text};
                }}
                el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                return {{ found: true }};
            }})()
            "#,
            selector = serde_json::to_string(selector).unwrap_or_default(),
            text = serde_json::to_string(text).unwrap_or_default()
        );

        let value = ctx.evaluate(js).await?;

        if value.get("found").and_then(|v| v.as_bool()) != Some(true) {
            if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
                return Err(KhoraError::JavaScriptError(format!(
                    "invalid selector {selector:?}: {err}"
                )));
            }
            return Err(KhoraError::ElementNotFound(ctx.describe(selector)));
        }
        Ok(())
    }

    /// Type text into an element matching a CSS selector using trusted
    /// per-character key events (CDP `Input.dispatchKeyEvent`: rawKeyDown +
    /// char + keyUp per character), instead of the JS value-assignment
    /// [`Self::type_text`] uses.
    ///
    /// `type` only reaches elements backed by real form-control state that a
    /// framework listens to `input`/`change` on. Canvas/WebGL-rendered
    /// widgets that manage their own key handling off a hidden textarea
    /// (xterm.js and similar terminal emulators) never read `.value` — this
    /// dispatches the same trusted keydown/keypress/keyup sequence a real
    /// keyboard produces, which any keydown/keypress listener, including
    /// xterm.js's, picks up.
    ///
    /// Driving the browser's real input pipeline also establishes real focus,
    /// so unlike [`Self::type_text`] this produces trusted `focus`/`focusin`
    /// and lets a subsequent blur fire — required by commit-on-blur and
    /// validate-on-blur handlers. Text is inserted at the caret, so it
    /// appends to an existing value rather than replacing it — pass `clear`
    /// to wipe the field first (the common case when editing an existing
    /// value, e.g. renaming a title).
    ///
    /// `clear` selects the element's existing content (`select()` on form
    /// controls, a full-contents range on contenteditable) and deletes it
    /// with a trusted Backspace, so the same real-input path — and the
    /// `input` events frameworks listen to — covers the clear as well as the
    /// typing. It is a no-op on an already-empty field, and on canvas-backed
    /// widgets with no readable value (xterm.js), which own their own buffer.
    ///
    /// Control characters (Enter, Tab, Backspace, arrows, ...) aren't
    /// handled here — send those individually with `key`.
    pub async fn type_keys(
        &self,
        selector: &str,
        text: &str,
        clear: bool,
        frame: Option<&str>,
        timeout_ms: u64,
    ) -> KhoraResult<()> {
        let ctx = self.frame_context(frame, timeout_ms).await?;
        let js = format!(
            r#"
            (() => {{
                let el;
                try {{
                    el = document.querySelector({selector});
                }} catch (e) {{
                    return {{ found: false, error: e.message }};
                }}
                if (!el) return {{ found: false }};
                el.focus();
                if ({clear}) {{
                    if (typeof el.select === 'function') {{
                        el.select();
                    }} else if (el.isContentEditable) {{
                        const range = document.createRange();
                        range.selectNodeContents(el);
                        const sel = window.getSelection();
                        sel.removeAllRanges();
                        sel.addRange(range);
                    }}
                }}
                return {{ found: true }};
            }})()
            "#,
            selector = serde_json::to_string(selector).unwrap_or_default()
        );

        let value = ctx.evaluate(js).await?;

        if value.get("found").and_then(|v| v.as_bool()) != Some(true) {
            if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
                return Err(KhoraError::JavaScriptError(format!(
                    "invalid selector {selector:?}: {err}"
                )));
            }
            return Err(KhoraError::ElementNotFound(ctx.describe(selector)));
        }

        if clear {
            let builder = DispatchKeyEventParams::builder()
                .code("Backspace")
                .key("Backspace")
                .windows_virtual_key_code(8)
                .native_virtual_key_code(8);

            let down = builder
                .clone()
                .r#type(DispatchKeyEventType::RawKeyDown)
                .build()
                .map_err(KhoraError::Cdp)?;
            tokio::time::timeout(
                std::time::Duration::from_millis(timeout_ms),
                ctx.execute(down),
            )
            .await
            .map_err(|_| KhoraError::Timeout(timeout_ms))??;

            let up = builder
                .r#type(DispatchKeyEventType::KeyUp)
                .build()
                .map_err(KhoraError::Cdp)?;
            tokio::time::timeout(
                std::time::Duration::from_millis(timeout_ms),
                ctx.execute(up),
            )
            .await
            .map_err(|_| KhoraError::Timeout(timeout_ms))??;
        }

        for c in text.chars() {
            let (key, code, vk) = Self::char_key_info(c);
            let builder = DispatchKeyEventParams::builder()
                .code(code)
                .key(key.clone())
                .windows_virtual_key_code(vk)
                .native_virtual_key_code(vk);

            let down = builder
                .clone()
                .r#type(DispatchKeyEventType::RawKeyDown)
                .build()
                .map_err(KhoraError::Cdp)?;
            tokio::time::timeout(
                std::time::Duration::from_millis(timeout_ms),
                ctx.execute(down),
            )
            .await
            .map_err(|_| KhoraError::Timeout(timeout_ms))??;

            let char_event = builder
                .clone()
                .r#type(DispatchKeyEventType::Char)
                .text(key.clone())
                .unmodified_text(key.clone())
                .build()
                .map_err(KhoraError::Cdp)?;
            tokio::time::timeout(
                std::time::Duration::from_millis(timeout_ms),
                ctx.execute(char_event),
            )
            .await
            .map_err(|_| KhoraError::Timeout(timeout_ms))??;

            let up = builder
                .r#type(DispatchKeyEventType::KeyUp)
                .build()
                .map_err(KhoraError::Cdp)?;
            tokio::time::timeout(
                std::time::Duration::from_millis(timeout_ms),
                ctx.execute(up),
            )
            .await
            .map_err(|_| KhoraError::Timeout(timeout_ms))??;
        }

        self.force_compositor_frame(&ctx.page).await;
        Ok(())
    }

    /// Insert text into an element matching a CSS selector in one CDP
    /// `Input.insertText` call, the way a paste or an IME commit arrives.
    ///
    /// Unlike [`Self::type_keys`] this fires **no key events at all** — no
    /// keydown, keypress or keyup, and no per-character `beforeinput`. A
    /// field that only reads `input` events (the common case) sees the text;
    /// anything driven by key handling — a keystroke-counting masked input,
    /// an xterm.js-style widget, a keydown-gated autocomplete — does not.
    /// Use `type-keys` there. In exchange it is one round trip rather than
    /// three per character, which is what makes it usable for long values and
    /// for fields inside a cross-origin iframe.
    ///
    /// Text is inserted at the caret, so it appends to an existing value.
    pub async fn insert_text(
        &self,
        selector: &str,
        text: &str,
        frame: Option<&str>,
        timeout_ms: u64,
    ) -> KhoraResult<()> {
        let ctx = self.frame_context(frame, timeout_ms).await?;
        let js = format!(
            r#"
            (() => {{
                let el;
                try {{
                    el = document.querySelector({selector});
                }} catch (e) {{
                    return {{ found: false, error: e.message }};
                }}
                if (!el) return {{ found: false }};
                el.focus();
                return {{ found: true }};
            }})()
            "#,
            selector = serde_json::to_string(selector).unwrap_or_default()
        );

        let value = ctx.evaluate(js).await?;

        if value.get("found").and_then(|v| v.as_bool()) != Some(true) {
            if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
                return Err(KhoraError::JavaScriptError(format!(
                    "invalid selector {selector:?}: {err}"
                )));
            }
            return Err(KhoraError::ElementNotFound(ctx.describe(selector)));
        }

        let insert = InsertTextParams::builder()
            .text(text)
            .build()
            .map_err(KhoraError::Cdp)?;
        tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            ctx.execute(insert),
        )
        .await
        .map_err(|_| KhoraError::Timeout(timeout_ms))??;

        self.force_compositor_frame(&ctx.page).await;
        Ok(())
    }

    /// Blur the focused element, firing `blur`/`focusout` so commit-on-blur
    /// and validate-on-blur handlers run.
    ///
    /// `selector` `None` blurs `document.activeElement`; `Some` blurs that
    /// element. Unlike `key Tab` — the usual stand-in — this moves focus off
    /// the field without moving it *onto* the next one, so unrelated focus
    /// handlers downstream in the tab order don't fire.
    ///
    /// Errors rather than no-ops when the target isn't actually focused
    /// (nothing focused at all, or a selector naming some other element):
    /// `el.blur()` on a non-focused element dispatches nothing, and a silent
    /// success there reads as "the commit fired" when it didn't.
    ///
    /// Carries the same document-focus caveat as [`Self::type_text`]: on a
    /// page nothing has interacted with, the browser fires no focus events,
    /// so there is nothing focused to blur. Establish real focus first with a
    /// trusted input event ([`Self::click`], [`Self::key_press`],
    /// [`Self::type_keys`]).
    ///
    /// Returns a short descriptor of the blurred element (`input#name`).
    pub async fn blur(&self, selector: Option<&str>) -> KhoraResult<String> {
        let page = self.get_or_create_page().await?;
        // r##: the JS below contains `"#"`, which would close an r#" string.
        let js = format!(
            r##"
            (() => {{
                const describe = (el) => {{
                    if (!el) return "null";
                    let d = el.tagName.toLowerCase();
                    if (el.id) d += "#" + el.id;
                    else if (el.name) d += "[name=" + el.name + "]";
                    else if (el.className && typeof el.className === 'string') {{
                        const c = el.className.trim().split(/\s+/)[0];
                        if (c) d += "." + c;
                    }}
                    return d;
                }};
                const selector = {selector};
                const active = document.activeElement;
                const focused = active && active !== document.body ? active : null;
                let el = focused;
                if (selector !== null) {{
                    try {{
                        el = document.querySelector(selector);
                    }} catch (e) {{
                        return {{ status: "bad-selector", error: e.message }};
                    }}
                    if (!el) return {{ status: "not-found" }};
                    if (el !== focused) {{
                        return {{ status: "not-focused", focused: describe(focused) }};
                    }}
                }} else if (!el) {{
                    return {{ status: "nothing-focused" }};
                }}
                const desc = describe(el);
                el.blur();
                return {{ status: "ok", element: desc }};
            }})()
            "##,
            selector = selector
                .map(|s| serde_json::to_string(s).unwrap_or_default())
                .unwrap_or_else(|| "null".to_string())
        );

        let result = page
            .evaluate(js)
            .await
            .map_err(|e| KhoraError::Cdp(e.to_string()))?;

        let value = result
            .into_value::<serde_json::Value>()
            .map_err(|e| KhoraError::Cdp(e.to_string()))?;

        match value.get("status").and_then(|v| v.as_str()) {
            Some("ok") => Ok(value
                .get("element")
                .and_then(|v| v.as_str())
                .unwrap_or("element")
                .to_string()),
            Some("bad-selector") => {
                let err = value
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("invalid selector");
                Err(KhoraError::JavaScriptError(format!(
                    "invalid selector {:?}: {err}",
                    selector.unwrap_or_default()
                )))
            }
            Some("not-found") => Err(KhoraError::ElementNotFound(
                selector.unwrap_or_default().to_string(),
            )),
            Some("not-focused") => {
                let focused = value
                    .get("focused")
                    .and_then(|v| v.as_str())
                    .unwrap_or("null");
                Err(KhoraError::NotFocused(format!(
                    "{} is not the focused element (focused: {focused})",
                    selector.unwrap_or_default()
                )))
            }
            _ => Err(KhoraError::NotFocused(
                "no element is focused — click or type-keys into one first".to_string(),
            )),
        }
    }

    /// Get text content of elements matching a CSS selector.
    pub async fn get_text(&self, selector: &str) -> KhoraResult<Vec<String>> {
        let page = self.get_or_create_page().await?;
        let js = format!(
            r#"
            (() => {{
                let els;
                try {{
                    els = document.querySelectorAll({selector});
                }} catch (e) {{
                    return {{ error: e.message }};
                }}
                if (els.length === 0) return null;
                return Array.from(els).map(el => el.textContent?.trim() || "");
            }})()
            "#,
            selector = serde_json::to_string(selector).unwrap_or_default()
        );

        let result = page
            .evaluate(js)
            .await
            .map_err(|e| KhoraError::Cdp(e.to_string()))?;

        let value = result
            .into_value::<serde_json::Value>()
            .map_err(|e| KhoraError::Cdp(e.to_string()))?;

        match value {
            serde_json::Value::Array(arr) => Ok(arr
                .into_iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()),
            serde_json::Value::Null => Err(KhoraError::ElementNotFound(selector.to_string())),
            serde_json::Value::Object(ref obj) => {
                if let Some(err) = obj.get("error").and_then(|v| v.as_str()) {
                    return Err(KhoraError::JavaScriptError(format!(
                        "invalid selector {selector:?}: {err}"
                    )));
                }
                Err(KhoraError::ElementNotFound(selector.to_string()))
            }
            _ => Ok(vec![value.to_string()]),
        }
    }

    /// Get an attribute value from the first element matching a CSS selector.
    pub async fn get_attribute(
        &self,
        selector: &str,
        attribute: &str,
    ) -> KhoraResult<Option<String>> {
        let page = self.get_or_create_page().await?;
        let js = format!(
            r#"
            (() => {{
                let el;
                try {{
                    el = document.querySelector({selector});
                }} catch (e) {{
                    return {{ found: false, error: e.message }};
                }}
                if (!el) return {{ found: false }};
                const val = el.getAttribute({attribute});
                return {{ found: true, value: val }};
            }})()
            "#,
            selector = serde_json::to_string(selector).unwrap_or_default(),
            attribute = serde_json::to_string(attribute).unwrap_or_default()
        );

        let result = page
            .evaluate(js)
            .await
            .map_err(|e| KhoraError::Cdp(e.to_string()))?;

        let value = result
            .into_value::<serde_json::Value>()
            .map_err(|e| KhoraError::Cdp(e.to_string()))?;

        if value.get("found").and_then(|v| v.as_bool()) != Some(true) {
            if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
                return Err(KhoraError::JavaScriptError(format!(
                    "invalid selector {selector:?}: {err}"
                )));
            }
            return Err(KhoraError::ElementNotFound(selector.to_string()));
        }

        Ok(value
            .get("value")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()))
    }

    /// Wait for an element to appear (poll until found or timeout).
    pub async fn wait_for(&self, selector: &str, timeout_ms: u64) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;
        let js = format!(
            r#"
            (() => {{
                try {{
                    return {{ found: document.querySelector({selector}) !== null }};
                }} catch (e) {{
                    return {{ error: e.message }};
                }}
            }})()
            "#,
            selector = serde_json::to_string(selector).unwrap_or_default()
        );

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(timeout_ms);

        loop {
            let result = page
                .evaluate(js.as_str())
                .await
                .map_err(|e| KhoraError::Cdp(e.to_string()))?;

            let value: serde_json::Value = result
                .into_value()
                .map_err(|e| KhoraError::Cdp(e.to_string()))?;

            if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
                return Err(KhoraError::JavaScriptError(format!(
                    "invalid selector {selector:?}: {err}"
                )));
            }

            if value.get("found").and_then(|v| v.as_bool()) == Some(true) {
                return Ok(());
            }

            if start.elapsed() >= timeout {
                return Err(KhoraError::Timeout(timeout_ms));
            }

            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Wait for an element to disappear (poll until gone or timeout).
    pub async fn wait_gone(&self, selector: &str, timeout_ms: u64) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;
        let js = format!(
            r#"
            (() => {{
                try {{
                    return {{ gone: document.querySelector({selector}) === null }};
                }} catch (e) {{
                    return {{ error: e.message }};
                }}
            }})()
            "#,
            selector = serde_json::to_string(selector).unwrap_or_default()
        );

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(timeout_ms);

        loop {
            let result = page
                .evaluate(js.as_str())
                .await
                .map_err(|e| KhoraError::Cdp(e.to_string()))?;

            let value: serde_json::Value = result
                .into_value()
                .map_err(|e| KhoraError::Cdp(e.to_string()))?;

            if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
                return Err(KhoraError::JavaScriptError(format!(
                    "invalid selector {selector:?}: {err}"
                )));
            }

            if value.get("gone").and_then(|v| v.as_bool()) == Some(true) {
                return Ok(());
            }

            if start.elapsed() >= timeout {
                return Err(KhoraError::Timeout(timeout_ms));
            }

            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Capture a screenshot, returned as PNG bytes.
    ///
    /// A `selector` scrolls the first matching element into view and crops the
    /// shot to its bounding box; returns [`KhoraError::ElementNotFound`] if
    /// nothing matches or the element has no visible area, rather than falling
    /// back to a whole-page shot. A `clip` of `(x, y, width, height)` in page
    /// CSS pixels captures that region verbatim — including area the document
    /// content size does not cover, which is the only way to reach a tall
    /// `position: fixed` overlay on a document that does not scroll. With
    /// neither, `full_page` chooses between the whole scrollable document and
    /// the visible viewport.
    pub async fn screenshot(
        &self,
        selector: Option<&str>,
        clip: Option<(f64, f64, f64, f64)>,
        full_page: bool,
    ) -> KhoraResult<Vec<u8>> {
        let page = self.get_or_create_page().await?;
        let mut builder = ScreenshotParams::builder().format(CaptureScreenshotFormat::Png);
        // capture_beyond_viewport renders the whole clip even when it is
        // taller/wider than the viewport; without it CDP only paints the
        // viewport-visible part and the rest comes out blank.
        builder = match (selector, clip, full_page) {
            (Some(sel), _, _) => builder
                .clip(self.element_clip(&page, sel).await?)
                .capture_beyond_viewport(true),
            (None, Some((x, y, width, height)), _) => builder
                .clip(Viewport {
                    x,
                    y,
                    width,
                    height,
                    scale: 1.0,
                })
                .capture_beyond_viewport(true),
            (None, None, true) => builder
                .clip(self.page_clip(&page).await?)
                .capture_beyond_viewport(true),
            (None, None, false) => builder,
        };
        let png = page
            .screenshot(builder.build())
            .await
            .map_err(|e| KhoraError::ScreenshotFailed(e.to_string()))?;
        Ok(png)
    }

    /// Clip rectangle covering the whole scrollable document.
    ///
    /// Deliberately *not* chromiumoxide's `ScreenshotParams::full_page`, which
    /// implements whole-page capture as
    /// `Emulation.setDeviceMetricsOverride(width, contentHeight)` → capture →
    /// `clearDeviceMetricsOverride`. That has two visible defects: resizing the
    /// viewport to the content height reflows the page, so `position: fixed`,
    /// `sticky` and `vh`-sized layout render at sizes the user never sees (a
    /// `inset: 40px` modal 857px tall on screen came out 2920px tall); and the
    /// trailing *clear* wipes any override a previous `set-viewport` installed,
    /// silently resetting the session's viewport as a side effect of taking a
    /// picture. Sizing a clip from `Page.getLayoutMetrics` instead touches no
    /// emulation state and leaves layout alone.
    async fn page_clip(&self, page: &Page) -> KhoraResult<Viewport> {
        let metrics = page
            .layout_metrics()
            .await
            .map_err(|e| KhoraError::Cdp(e.to_string()))?;
        let content = metrics.css_content_size;
        Ok(Viewport {
            x: 0.0,
            y: 0.0,
            width: content.width,
            height: content.height,
            scale: 1.0,
        })
    }

    /// Resolve a selector to a screenshot clip rectangle in page coordinates.
    ///
    /// Done by JS evaluation (per project convention) so the clip is computed
    /// the same way as other element ops. Returns [`KhoraError::ElementNotFound`]
    /// if the selector matches nothing or the element has zero area.
    async fn element_clip(&self, page: &Page, selector: &str) -> KhoraResult<Viewport> {
        // Return an array ([] = no match) rather than null: chromiumoxide's
        // into_value() fails on a bare JS null, but parses arrays fine (see
        // find_elements). This is what makes the not-found case detectable.
        let js = format!(
            r#"
            (() => {{
                let el;
                try {{
                    el = document.querySelector({selector});
                }} catch (e) {{
                    return {{ error: e.message }};
                }}
                if (!el) return [];
                el.scrollIntoView({{ block: 'center', inline: 'center' }});
                const rect = el.getBoundingClientRect();
                if (rect.width <= 0 || rect.height <= 0) return [];
                return [{{
                    x: rect.left + window.scrollX,
                    y: rect.top + window.scrollY,
                    width: rect.width,
                    height: rect.height,
                }}];
            }})()
            "#,
            selector = serde_json::to_string(selector).unwrap_or_default()
        );

        let value = page
            .evaluate(js)
            .await
            .map_err(|e| KhoraError::Cdp(e.to_string()))?
            .into_value::<serde_json::Value>()
            .map_err(|e| KhoraError::Cdp(e.to_string()))?;

        let bb = match value {
            serde_json::Value::Array(arr) if !arr.is_empty() => {
                serde_json::from_value::<BoundingBox>(arr.into_iter().next().unwrap())
                    .map_err(|e| KhoraError::Cdp(e.to_string()))?
            }
            serde_json::Value::Object(ref obj) => {
                if let Some(err) = obj.get("error").and_then(|v| v.as_str()) {
                    return Err(KhoraError::JavaScriptError(format!(
                        "invalid selector {selector:?}: {err}"
                    )));
                }
                return Err(KhoraError::ElementNotFound(selector.to_string()));
            }
            _ => return Err(KhoraError::ElementNotFound(selector.to_string())),
        };

        Ok(Viewport {
            x: bb.x,
            y: bb.y,
            width: bb.width,
            height: bb.height,
            scale: 1.0,
        })
    }

    /// Dispatch a single trusted mouse event (CDP Input.dispatchMouseEvent).
    ///
    /// Shared by [`Self::drag`], the step-wise [`Self::mouse_down`] /
    /// [`Self::mouse_move`] / [`Self::mouse_up`] primitives, and
    /// [`Self::click_at`] / [`Self::dblclick_at`]. `buttons` is set to 1
    /// (left button held) for every event type, matching the state a real
    /// press-move-release gesture reports throughout. `click_count` tells
    /// Chromium which click in a sequence this is — 2 on the second
    /// press/release pair is what makes it register as a double-click
    /// rather than two separate clicks.
    async fn dispatch_mouse_event(
        &self,
        page: &Page,
        kind: DispatchMouseEventType,
        x: f64,
        y: f64,
        click_count: i64,
        timeout_ms: u64,
    ) -> KhoraResult<()> {
        let event = DispatchMouseEventParams::builder()
            .r#type(kind)
            .x(x)
            .y(y)
            .button(MouseButton::Left)
            .buttons(1)
            .click_count(click_count)
            .build()
            .map_err(KhoraError::Cdp)?;
        tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            page.execute(event),
        )
        .await
        .map_err(|_| KhoraError::Timeout(timeout_ms))?
        .map_err(|e| KhoraError::Cdp(e.to_string()))?;
        Ok(())
    }

    /// Drag from one viewport point to another with trusted mouse events
    /// (CDP Input.dispatchMouseEvent: press, interpolated moves, release).
    ///
    /// Unlike the JS-evaluation element ops, this must use native CDP input:
    /// drag interactions (crop marquees, sliders, drag handles) check
    /// `isTrusted` or track real pointer state, which synthetic JS events
    /// can't satisfy. `steps` mouseMoved events are spread evenly along the
    /// line, ending exactly at `to`; `delay_ms` sleeps between events give
    /// frameworks that batch on animation frames (e.g. React) a chance to
    /// observe the motion.
    pub async fn drag(
        &self,
        from: (f64, f64),
        to: (f64, f64),
        steps: u32,
        delay_ms: u64,
        timeout_ms: u64,
    ) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;
        let delay = std::time::Duration::from_millis(delay_ms);

        // A never-painted page (e.g. about:blank) has no pending compositor
        // frame, so the first MouseMoved dispatch below can pay a ~5s
        // hit-test tax that races khora's default 5000ms timeout (reported
        // against mesa task 385; not reproduced live against this repo's
        // current Chrome — timing is version/load-dependent). Force a frame
        // up front, once, so the interpolated moves that follow stay
        // fast — preventive, mirroring key_press's proven fix below.
        self.force_compositor_frame(&page).await;

        self.dispatch_mouse_event(
            &page,
            DispatchMouseEventType::MousePressed,
            from.0,
            from.1,
            1,
            timeout_ms,
        )
        .await?;

        for i in 1..=steps {
            tokio::time::sleep(delay).await;
            let t = f64::from(i) / f64::from(steps);
            let x = from.0 + (to.0 - from.0) * t;
            let y = from.1 + (to.1 - from.1) * t;
            self.dispatch_mouse_event(
                &page,
                DispatchMouseEventType::MouseMoved,
                x,
                y,
                1,
                timeout_ms,
            )
            .await?;
        }

        tokio::time::sleep(delay).await;
        self.dispatch_mouse_event(
            &page,
            DispatchMouseEventType::MouseReleased,
            to.0,
            to.1,
            1,
            timeout_ms,
        )
        .await?;
        Ok(())
    }

    /// Press the left mouse button at a point with a trusted event, without
    /// releasing it.
    ///
    /// Pairs with [`Self::mouse_move`] and [`Self::mouse_up`] to script a
    /// drag as separate CLI invocations, so mid-gesture state (e.g. a
    /// marquee mid-drag) can be inspected with an ordinary `screenshot` call
    /// between steps instead of racing a backgrounded `drag`.
    pub async fn mouse_down(&self, at: (f64, f64), timeout_ms: u64) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;
        self.dispatch_mouse_event(
            &page,
            DispatchMouseEventType::MousePressed,
            at.0,
            at.1,
            1,
            timeout_ms,
        )
        .await
    }

    /// Move the mouse to a point with a trusted event, carrying over
    /// whatever button state a prior [`Self::mouse_down`] established.
    pub async fn mouse_move(&self, at: (f64, f64), timeout_ms: u64) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;
        // Step-wise mouse-move is its own CLI invocation, so there's no
        // memory of whether this page has ever painted a frame. Nudge one
        // unconditionally — preventive, same reasoning as drag()'s comment.
        self.force_compositor_frame(&page).await;
        self.dispatch_mouse_event(
            &page,
            DispatchMouseEventType::MouseMoved,
            at.0,
            at.1,
            1,
            timeout_ms,
        )
        .await
    }

    /// Release the left mouse button at a point with a trusted event,
    /// completing a gesture started with [`Self::mouse_down`].
    pub async fn mouse_up(&self, at: (f64, f64), timeout_ms: u64) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;
        self.dispatch_mouse_event(
            &page,
            DispatchMouseEventType::MouseReleased,
            at.0,
            at.1,
            1,
            timeout_ms,
        )
        .await
    }

    /// Click at a raw viewport point with trusted mouse events (CDP
    /// Input.dispatchMouseEvent: press then release, both `clickCount: 1`).
    ///
    /// Unlike [`Self::click`], which resolves a CSS selector and clicks that
    /// element, this hits whatever is actually at `(x, y)` — needed to
    /// verify pixel-precise hit targets (overlapping elements, canvas-drawn
    /// UI, z-order) that a selector can't express.
    pub async fn click_at(&self, at: (f64, f64), timeout_ms: u64) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;
        self.dispatch_mouse_event(
            &page,
            DispatchMouseEventType::MousePressed,
            at.0,
            at.1,
            1,
            timeout_ms,
        )
        .await?;
        self.dispatch_mouse_event(
            &page,
            DispatchMouseEventType::MouseReleased,
            at.0,
            at.1,
            1,
            timeout_ms,
        )
        .await
    }

    /// Double-click at a raw viewport point with trusted mouse events: two
    /// press/release pairs at the same point, the second pair carrying
    /// `clickCount: 2` — the signal Chromium uses to fire `dblclick` instead
    /// of two separate `click` events.
    pub async fn dblclick_at(&self, at: (f64, f64), timeout_ms: u64) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;
        self.dispatch_mouse_event(
            &page,
            DispatchMouseEventType::MousePressed,
            at.0,
            at.1,
            1,
            timeout_ms,
        )
        .await?;
        self.dispatch_mouse_event(
            &page,
            DispatchMouseEventType::MouseReleased,
            at.0,
            at.1,
            1,
            timeout_ms,
        )
        .await?;
        self.dispatch_mouse_event(
            &page,
            DispatchMouseEventType::MousePressed,
            at.0,
            at.1,
            2,
            timeout_ms,
        )
        .await?;
        self.dispatch_mouse_event(
            &page,
            DispatchMouseEventType::MouseReleased,
            at.0,
            at.1,
            2,
            timeout_ms,
        )
        .await
    }

    /// Scroll with a trusted native wheel event at a viewport point (CDP
    /// Input.dispatchMouseEvent: type `mouseWheel`, with `deltaX`/`deltaY`).
    ///
    /// Unlike the JS-evaluation element ops, this must use native CDP input:
    /// a synthetic `WheelEvent` dispatched from JS never reaches Chromium's
    /// scroll pipeline, so it can't drive native scrolling, scroll chaining,
    /// or `overscroll-behavior` — it only fires listeners, leaving
    /// `scrollTop` untouched. This dispatches the real event Chromium's
    /// compositor scrolls in response to, at whatever element sits under
    /// `at`.
    pub async fn wheel(
        &self,
        at: (f64, f64),
        delta: (f64, f64),
        timeout_ms: u64,
    ) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;
        // `mouseWheel` is the strictest consumer of compositor frames: unlike
        // `mousePressed`/`mouseReleased` (which need none) and `mouseMoved`
        // (which degrades but retries), it has no fallback — dispatched into a
        // page with no pending frame it can fail to ack at all, hanging until
        // the caller's timeout. Every other trusted-input method already
        // nudges a frame; this one did not, which is the asymmetry mesa task
        // 569 reported. Preventive and best-effort, same reasoning as
        // `mouse_move()`/`drag()` — and, like theirs, defense-in-depth rather
        // than a fix for a locally reproduced hang: 40 cold launches (20
        // launch->navigate->wheel, 20 launch->wheel) were all green before
        // this call was added.
        self.force_compositor_frame(&page).await;
        let event = DispatchMouseEventParams::builder()
            .r#type(DispatchMouseEventType::MouseWheel)
            .x(at.0)
            .y(at.1)
            .delta_x(delta.0)
            .delta_y(delta.1)
            .build()
            .map_err(KhoraError::Cdp)?;
        tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            page.execute(event),
        )
        .await
        .map_err(|_| KhoraError::Timeout(timeout_ms))?
        .map_err(|e| KhoraError::Cdp(e.to_string()))?;
        Ok(())
    }

    /// Dispatch a single trusted touch event (CDP Input.dispatchTouchEvent).
    ///
    /// `points` carries the touch points still active *after* this event, so
    /// `TouchStart`/`TouchMove` need at least one (one per finger — a pinch
    /// sends two) and `TouchEnd`/`TouchCancel` must carry none, which is why
    /// releasing two fingers is still a single empty `TouchEnd`. CDP rejects
    /// the combination the other way round.
    async fn dispatch_touch_event(
        &self,
        page: &Page,
        kind: DispatchTouchEventType,
        points: Vec<TouchPoint>,
        timeout_ms: u64,
    ) -> KhoraResult<()> {
        let event = DispatchTouchEvent {
            kind,
            touch_points: points,
        };
        tracing::debug!(
            params = %serde_json::to_string(&event).unwrap_or_default(),
            "Input.dispatchTouchEvent"
        );
        tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            page.execute(event),
        )
        .await
        .map_err(|_| KhoraError::Timeout(timeout_ms))?
        .map_err(|e| KhoraError::Cdp(e.to_string()))?;
        Ok(())
    }

    /// One touch contact at a viewport point, tagged with `id`.
    ///
    /// The `id` is pinned so every event in a gesture describes the same
    /// touch source; Chromium tracks a finger across start/move/end by that
    /// id, and letting it default would make each move look like a new one.
    /// Multi-contact gestures ([`Self::pinch`]) need the ids to differ, which
    /// is what makes them two fingers rather than one teleporting one.
    fn contact(id: f64, at: (f64, f64)) -> TouchPoint {
        let mut point = TouchPoint::new(at.0, at.1);
        point.id = Some(id);
        point
    }

    /// The single finger the one-contact gestures dispatch.
    fn finger(at: (f64, f64)) -> TouchPoint {
        Self::contact(0.0, at)
    }

    /// Tap at a viewport point with trusted touch events (touchStart then
    /// touchEnd).
    ///
    /// Distinct from [`Self::click_at`]: a tap produces real `TouchEvent`s,
    /// so it drives touch-only handlers, and the pointer events it derives
    /// report `pointerType: "touch"` where the mouse path reports `mouse`.
    /// The full compatibility sequence a phone produces follows
    /// (`pointerdown`/`pointerup`, `mousedown`/`mouseup`, `click`), all
    /// `isTrusted` — verified live, see `tests/e2e/qa.sh`.
    ///
    /// The touch commands do not turn on CDP touch *emulation* themselves,
    /// because measurement showed it buys nothing for dispatch: the events,
    /// native panning and `pointerType` are identical with and without it.
    /// What it does change is what a page *believes* about the device, which
    /// matters before any of these events are sent — an app that registers
    /// touch handlers only behind a `'ontouchstart' in window` check has none
    /// for a trusted tap to reach. That is the separate, session-persisted
    /// [`Self::emulate_touch`]; run it before `navigate` so the page's
    /// load-time detection sees a touch device.
    pub async fn touch(&self, at: (f64, f64), timeout_ms: u64) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;
        self.dispatch_touch_event(
            &page,
            DispatchTouchEventType::TouchStart,
            vec![Self::finger(at)],
            timeout_ms,
        )
        .await?;
        self.dispatch_touch_event(&page, DispatchTouchEventType::TouchEnd, vec![], timeout_ms)
            .await
    }

    /// Swipe from one viewport point to another with trusted touch events
    /// (touchStart, interpolated touchMoves, touchEnd).
    ///
    /// This is the only way to exercise `touch-action`: that property governs
    /// *touch* panning exclusively, so a trusted `wheel` cannot reach it and
    /// synthetic `TouchEvent`s dispatched from `eval` never enter Chromium's
    /// scroll pipeline at all. Like [`Self::drag`], `steps` events are spread
    /// evenly along the line ending exactly at `to`, with `delay_ms` between
    /// them so frameworks batching on animation frames observe the motion —
    /// and gesture recognizers that discriminate by velocity see a plausible
    /// one.
    ///
    /// Chromium honors the axis restriction, not merely the motion: the same
    /// vertical swipe scrolls a `touch-action: pan-y` container and leaves an
    /// otherwise-identical `pan-x` one at `scrollTop` 0 (measured; both cases
    /// are asserted in `tests/e2e/qa.sh`).
    pub async fn swipe(
        &self,
        from: (f64, f64),
        to: (f64, f64),
        steps: u32,
        delay_ms: u64,
        timeout_ms: u64,
    ) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;
        let delay = std::time::Duration::from_millis(delay_ms);
        // Same never-painted-page hit-test tax the interpolated moves in
        // drag() pay; force a frame once up front for the same reason.
        self.force_compositor_frame(&page).await;

        self.dispatch_touch_event(
            &page,
            DispatchTouchEventType::TouchStart,
            vec![Self::finger(from)],
            timeout_ms,
        )
        .await?;

        for i in 1..=steps {
            tokio::time::sleep(delay).await;
            let t = f64::from(i) / f64::from(steps);
            let x = from.0 + (to.0 - from.0) * t;
            let y = from.1 + (to.1 - from.1) * t;
            self.dispatch_touch_event(
                &page,
                DispatchTouchEventType::TouchMove,
                vec![Self::finger((x, y))],
                timeout_ms,
            )
            .await?;
        }

        tokio::time::sleep(delay).await;
        self.dispatch_touch_event(&page, DispatchTouchEventType::TouchEnd, vec![], timeout_ms)
            .await
    }

    /// Pinch two fingers around a viewport point, from one separation to
    /// another (touchStart, interpolated touchMoves, touchEnd).
    ///
    /// The one gesture [`Self::swipe`] structurally cannot reach: it
    /// dispatches a single contact, and pinch-zoom is defined by the
    /// *distance between two* of them. The contacts are placed symmetrically
    /// on the horizontal axis, `gap / 2` either side of `at`, and the gap is
    /// interpolated from `from_gap` to `to_gap` across `steps` moves — so
    /// `--from 100 --to 300` is a 3x spread and the reverse is a 3x squeeze.
    ///
    /// Horizontal placement is not a limitation for handlers that key off
    /// `Math.hypot` of the two touches, which is the usual pan/zoom canvas
    /// shape (measured against one in `tests/e2e/qa.sh`: scale tracks the gap
    /// ratio). A handler that discriminates by axis would need its own
    /// command, not a flag on this one.
    pub async fn pinch(
        &self,
        at: (f64, f64),
        from_gap: f64,
        to_gap: f64,
        steps: u32,
        delay_ms: u64,
        timeout_ms: u64,
    ) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;
        let delay = std::time::Duration::from_millis(delay_ms);
        // Same never-painted-page hit-test tax swipe() pays.
        self.force_compositor_frame(&page).await;

        let contacts = |gap: f64| {
            let half = gap / 2.0;
            vec![
                Self::contact(0.0, (at.0 - half, at.1)),
                Self::contact(1.0, (at.0 + half, at.1)),
            ]
        };

        self.dispatch_touch_event(
            &page,
            DispatchTouchEventType::TouchStart,
            contacts(from_gap),
            timeout_ms,
        )
        .await?;

        for i in 1..=steps {
            tokio::time::sleep(delay).await;
            let t = f64::from(i) / f64::from(steps);
            let gap = from_gap + (to_gap - from_gap) * t;
            self.dispatch_touch_event(
                &page,
                DispatchTouchEventType::TouchMove,
                contacts(gap),
                timeout_ms,
            )
            .await?;
        }

        tokio::time::sleep(delay).await;
        self.dispatch_touch_event(&page, DispatchTouchEventType::TouchEnd, vec![], timeout_ms)
            .await
    }

    /// Hold a touch at a viewport point for `duration_ms`, then release
    /// (touchStart, wait, touchEnd).
    ///
    /// For the press-and-hold gestures a tap can't reach — context menus,
    /// dnd-kit's `TouchSensor` activation delay, reorder handles — which are
    /// implemented as a timer started on `touchstart` and cancelled by an
    /// early `touchend`, so the wait has to be real.
    pub async fn long_press(
        &self,
        at: (f64, f64),
        duration_ms: u64,
        timeout_ms: u64,
    ) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;
        self.dispatch_touch_event(
            &page,
            DispatchTouchEventType::TouchStart,
            vec![Self::finger(at)],
            timeout_ms,
        )
        .await?;
        tokio::time::sleep(std::time::Duration::from_millis(duration_ms)).await;
        self.dispatch_touch_event(&page, DispatchTouchEventType::TouchEnd, vec![], timeout_ms)
            .await
    }

    /// Press a `+`-separated key combo (e.g. `Cmd+D`, `Ctrl+Shift+I`,
    /// `Escape`) with a trusted key event (CDP Input.dispatchKeyEvent:
    /// rawKeyDown then keyUp, modifier bits set on both).
    ///
    /// Unlike the JS-evaluation element ops, this must use native CDP input:
    /// modifier shortcuts are handled by the browser/OS or by listeners
    /// checking `isTrusted`, which synthetic `KeyboardEvent` dispatch can't
    /// satisfy.
    pub async fn key_press(&self, combo: &str, timeout_ms: u64) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;
        let (modifiers, key_name) = Self::parse_key_combo(combo)?;
        let (key, code, vk) = Self::key_info(key_name).ok_or_else(|| {
            KhoraError::InvalidKeyCombo(format!("unsupported key {key_name:?} in {combo:?}"))
        })?;
        // key_info normalizes single letters to lowercase; only Shift should
        // report the uppercase `key` a real keyboard would produce (`code`
        // and the virtual-key code stay the same either way — physical key
        // identity, not the character it produces).
        let shifted = modifiers & 8 != 0;
        let key = if shifted && key.len() == 1 && key.starts_with(|c: char| c.is_ascii_alphabetic())
        {
            key.to_ascii_uppercase()
        } else {
            key
        };

        let builder = DispatchKeyEventParams::builder()
            .modifiers(modifiers)
            .code(code)
            .key(key)
            .windows_virtual_key_code(vk)
            .native_virtual_key_code(vk);

        let down = builder
            .clone()
            .r#type(DispatchKeyEventType::RawKeyDown)
            .build()
            .map_err(KhoraError::Cdp)?;
        // Log what was actually put on the wire, so a page that reports a
        // different `key`/`code` than requested can be attributed to Chrome's
        // side of the dispatch instead of guessed at (mesa task 551 — see
        // tests/e2e/key-wedge-soak.sh, which reads this back under RUST_LOG).
        tracing::debug!(
            params = %serde_json::to_string(&down).unwrap_or_default(),
            "Input.dispatchKeyEvent rawKeyDown"
        );
        tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            page.execute(down),
        )
        .await
        .map_err(|_| KhoraError::Timeout(timeout_ms))?
        .map_err(|e| KhoraError::Cdp(e.to_string()))?;

        let up = builder
            .r#type(DispatchKeyEventType::KeyUp)
            .build()
            .map_err(KhoraError::Cdp)?;
        tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            page.execute(up),
        )
        .await
        .map_err(|_| KhoraError::Timeout(timeout_ms))?
        .map_err(|e| KhoraError::Cdp(e.to_string()))?;

        self.force_compositor_frame(&page).await;
        Ok(())
    }

    /// Nudge Chromium into producing a fresh compositor frame, discarding
    /// the result.
    ///
    /// A page with no pending compositor frame — either because a trusted
    /// `Input.dispatchKeyEvent` just consumed the pending one, or because
    /// the page (e.g. `about:blank`) has never painted at all — makes the
    /// next `mouseMoved` dispatch pay a ~5s hit-test tax, and the next
    /// `mouseWheel` dispatch never ack at all (hangs the full 30s
    /// chromiumoxide request timeout), until *something* forces a frame to
    /// be produced. A 1x1 screenshot is the cheapest CDP call observed to
    /// force that frame (a real `captureScreenshot` inherently waits on
    /// one) — verified live: it also fixes the mouse-move degradation,
    /// confirming both symptoms share this one root cause. Callers use this
    /// both reactively (`key_press`, after dispatch, to undo the desync it
    /// caused) and proactively (`drag`, `mouse_move`, before dispatch, to
    /// pre-warm a possibly-cold compositor). Best-effort: swallow failures
    /// rather than fail the caller over a side-effect nudge, and cap the
    /// wait so a wedged nudge can't turn a fast call into a slow one.
    async fn force_compositor_frame(&self, page: &Page) {
        if let Err(e) =
            tokio::time::timeout(std::time::Duration::from_secs(3), capture_1x1(page)).await
        {
            tracing::debug!("force_compositor_frame timed out: {e}");
        }
    }

    /// Split a key combo into a CDP modifier bitfield (Alt=1, Ctrl=2,
    /// Meta=4, Shift=8) and the trailing key name, e.g. `"Cmd+D"` ->
    /// `(4, "D")`.
    fn parse_key_combo(combo: &str) -> KhoraResult<(i64, &str)> {
        let parts: Vec<&str> = combo
            .split('+')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        let (key_name, mod_names) = parts
            .split_last()
            .ok_or_else(|| KhoraError::InvalidKeyCombo(format!("empty combo: {combo:?}")))?;

        let mut modifiers = 0i64;
        for name in mod_names {
            let bit = match name.to_ascii_lowercase().as_str() {
                "alt" | "option" => 1,
                "ctrl" | "control" => 2,
                "cmd" | "command" | "meta" => 4,
                "shift" => 8,
                other => {
                    return Err(KhoraError::InvalidKeyCombo(format!(
                        "unknown modifier {other:?} in {combo:?}"
                    )))
                }
            };
            modifiers |= bit;
        }
        Ok((modifiers, key_name))
    }

    /// Look up the CDP `key`, `code`, and Windows virtual-key-code for a key
    /// name: single letters/digits, or a common named key (Enter, Escape,
    /// Tab, Backspace, Delete, Space, arrow keys).
    fn key_info(name: &str) -> Option<(String, String, i64)> {
        let mut chars = name.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            if c.is_ascii_alphabetic() {
                let upper = c.to_ascii_uppercase();
                // Unshifted `key` is lowercase regardless of the case typed in
                // the combo; key_press uppercases it back when Shift is set.
                return Some((
                    c.to_ascii_lowercase().to_string(),
                    format!("Key{upper}"),
                    i64::from(upper as u8),
                ));
            }
            if c.is_ascii_digit() {
                return Some((
                    c.to_string(),
                    format!("Digit{c}"),
                    i64::from(c as u8), // '0'..='9' == 0x30..=0x39, matching VK_0..VK_9
                ));
            }
        }
        let (key, code, vk) = match name.to_ascii_lowercase().as_str() {
            "enter" | "return" => ("Enter", "Enter", 13),
            "escape" | "esc" => ("Escape", "Escape", 27),
            "tab" => ("Tab", "Tab", 9),
            "backspace" => ("Backspace", "Backspace", 8),
            "delete" | "del" => ("Delete", "Delete", 46),
            "space" => (" ", "Space", 32),
            "home" => ("Home", "Home", 36),
            "end" => ("End", "End", 35),
            "pageup" => ("PageUp", "PageUp", 33),
            "pagedown" => ("PageDown", "PageDown", 34),
            "arrowup" | "up" => ("ArrowUp", "ArrowUp", 38),
            "arrowdown" | "down" => ("ArrowDown", "ArrowDown", 40),
            "arrowleft" | "left" => ("ArrowLeft", "ArrowLeft", 37),
            "arrowright" | "right" => ("ArrowRight", "ArrowRight", 39),
            _ => return None,
        };
        Some((key.to_string(), code.to_string(), vk))
    }

    /// Look up the CDP `key`, `code`, and Windows virtual-key-code to
    /// produce a given character via [`Self::type_keys`]. Unlike
    /// [`Self::key_info`], `key` preserves the character's actual case (a
    /// real Shift+H keydown reports `key: "H"`, not `"h"`) — `code` is the
    /// physical key position, which stays the same regardless of case.
    ///
    /// The virtual-key code matters as much as `key`: xterm.js and other
    /// terminal emulators dispatch off `keyCode`, not `key`. Reporting a
    /// character's codepoint as its virtual-key code (the old fallback)
    /// silently turned punctuation into navigation keys — `-` is U+002D = 45
    /// = `VK_INSERT`, so typing `stty -echo` delivered `ESC[2~` in place of
    /// every hyphen. Every ASCII printable therefore gets its real US-layout
    /// `code`/virtual-key pair here, and anything still unmapped reports
    /// virtual key 0 rather than a codepoint that could land in the
    /// editing/navigation range (`VK` 8..=46).
    fn char_key_info(c: char) -> (String, String, i64) {
        if c.is_ascii_alphabetic() {
            let upper = c.to_ascii_uppercase();
            return (c.to_string(), format!("Key{upper}"), i64::from(upper as u8));
        }
        if c.is_ascii_digit() {
            return (c.to_string(), format!("Digit{c}"), i64::from(c as u8));
        }
        // US-layout physical keys. Shifted characters report the same `code`
        // and virtual key as their unshifted twin — a real `!` keydown is
        // Shift+Digit1, i.e. `code: "Digit1"`, `keyCode: 49`. `key` carries
        // the shifted character, which is what listeners read for text.
        if let Some((code, vk)) = match c {
            ' ' => Some(("Space", 32)),
            '!' => Some(("Digit1", 49)),
            '@' => Some(("Digit2", 50)),
            '#' => Some(("Digit3", 51)),
            '$' => Some(("Digit4", 52)),
            '%' => Some(("Digit5", 53)),
            '^' => Some(("Digit6", 54)),
            '&' => Some(("Digit7", 55)),
            '*' => Some(("Digit8", 56)),
            '(' => Some(("Digit9", 57)),
            ')' => Some(("Digit0", 48)),
            ';' | ':' => Some(("Semicolon", 186)),
            '=' | '+' => Some(("Equal", 187)),
            ',' | '<' => Some(("Comma", 188)),
            '-' | '_' => Some(("Minus", 189)),
            '.' | '>' => Some(("Period", 190)),
            '/' | '?' => Some(("Slash", 191)),
            '`' | '~' => Some(("Backquote", 192)),
            '[' | '{' => Some(("BracketLeft", 219)),
            '\\' | '|' => Some(("Backslash", 220)),
            ']' | '}' => Some(("BracketRight", 221)),
            '\'' | '"' => Some(("Quote", 222)),
            '\n' | '\r' => Some(("Enter", 13)),
            '\t' => Some(("Tab", 9)),
            _ => None,
        } {
            return (c.to_string(), code.to_string(), vk);
        }
        // No physical key known. Non-ASCII characters keep the codepoint —
        // it is above the navigation range and terminals need a nonzero
        // keyCode to treat the event as printable — but control characters
        // report 0 rather than impersonating an editing key.
        let vk = if u32::from(c) >= 48 {
            i64::from(u32::from(c))
        } else {
            0
        };
        (c.to_string(), "Unidentified".to_string(), vk)
    }

    /// Override the page viewport via CDP `Emulation.setDeviceMetricsOverride`.
    /// `None` clears the override and restores the real window's metrics.
    ///
    /// Headless Chrome clamps `launch --window-size` to a ~500px minimum inner
    /// width, so phone-width QA (375-430px) needs a metrics override instead.
    /// `dpr` 0.0 keeps the current device scale factor; `mobile` enables
    /// mobile emulation (meta-viewport handling, mobile UA hints).
    ///
    /// Like media and touch emulation this dies with the CDP connection, so the
    /// caller persists it on the session and `connect()` reloads it. The loss is
    /// less obvious here because it is *staged*: an already-loaded document keeps
    /// reporting the emulated `innerWidth` after the connection that set it went
    /// away, and only the next document created — the next `navigate` or reload —
    /// comes up at the real window size. That is why a one-shot `set-viewport`
    /// looked like it persisted while silently failing across every navigation.
    pub async fn set_viewport(&self, viewport: Option<ViewportEmulation>) -> KhoraResult<()> {
        // resolve_page() rather than get_or_create_page(): re-applying the
        // session's *previous* metrics first would reflow the page at the old
        // width on the way to the one asked for, firing a spurious matchMedia
        // transition — same reason `emulate_touch` bypasses the re-apply.
        let page = self.resolve_page().await?;
        apply_viewport(&page, viewport).await
    }

    /// Read console messages from the page.
    pub async fn console_messages(&self) -> KhoraResult<Vec<ConsoleMessage>> {
        let page = self.get_or_create_page().await?;
        let js = r#"
            (() => {
                if (!window.__khora_console) return [];
                return window.__khora_console;
            })()
        "#;

        let result = page
            .evaluate(js)
            .await
            .map_err(|e| KhoraError::Cdp(e.to_string()))?;

        let value = result
            .into_value::<serde_json::Value>()
            .map_err(|e| KhoraError::Cdp(e.to_string()))?;

        match value {
            serde_json::Value::Array(arr) => {
                let messages: Vec<ConsoleMessage> = arr
                    .into_iter()
                    .filter_map(|v| serde_json::from_value(v).ok())
                    .collect();
                Ok(messages)
            }
            _ => Ok(Vec::new()),
        }
    }

    /// Read captured network requests from the page.
    pub async fn network_requests(&self) -> KhoraResult<Vec<NetworkRequest>> {
        let page = self.get_or_create_page().await?;
        let js = r#"
            (() => {
                if (!window.__khora_network) return [];
                return window.__khora_network;
            })()
        "#;

        let result = page
            .evaluate(js)
            .await
            .map_err(|e| KhoraError::Cdp(e.to_string()))?;

        let value = result
            .into_value::<serde_json::Value>()
            .map_err(|e| KhoraError::Cdp(e.to_string()))?;

        match value {
            serde_json::Value::Array(arr) => {
                let requests: Vec<NetworkRequest> = arr
                    .into_iter()
                    .filter_map(|v| serde_json::from_value(v).ok())
                    .collect();
                Ok(requests)
            }
            _ => Ok(Vec::new()),
        }
    }

    /// Install network request capture hook (monkey-patches fetch and XMLHttpRequest).
    pub async fn install_network_hook(&self) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;
        let js = r#"
            (() => {
                if (window.__khora_network) return;
                window.__khora_network = [];

                // Patch fetch
                const origFetch = window.fetch;
                window.fetch = function(input, init) {
                    const url = (typeof input === 'string') ? input : (input.url || String(input));
                    const method = (init && init.method) ? init.method.toUpperCase() : 'GET';
                    const entry = { url, method, status: null, resource_type: 'fetch' };
                    window.__khora_network.push(entry);
                    return origFetch.apply(this, arguments).then(resp => {
                        entry.status = resp.status;
                        return resp;
                    }).catch(err => {
                        entry.status = 0;
                        throw err;
                    });
                };

                // Patch XMLHttpRequest
                const origOpen = XMLHttpRequest.prototype.open;
                const origSend = XMLHttpRequest.prototype.send;
                XMLHttpRequest.prototype.open = function(method, url) {
                    this.__khora = { url: String(url), method: method.toUpperCase(), resource_type: 'xhr' };
                    return origOpen.apply(this, arguments);
                };
                XMLHttpRequest.prototype.send = function() {
                    if (this.__khora) {
                        const entry = { ...this.__khora, status: null };
                        window.__khora_network.push(entry);
                        this.addEventListener('loadend', () => {
                            entry.status = this.status || 0;
                        });
                    }
                    return origSend.apply(this, arguments);
                };
            })()
        "#;
        page.evaluate(js)
            .await
            .map_err(|e| KhoraError::Cdp(e.to_string()))?;
        Ok(())
    }

    /// Install console message capture hook.
    pub async fn install_console_hook(&self) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;
        let js = r#"
            (() => {
                if (window.__khora_console) return;
                window.__khora_console = [];
                const orig = {};
                ['log', 'warn', 'error', 'info'].forEach(level => {
                    orig[level] = console[level];
                    console[level] = function(...args) {
                        window.__khora_console.push({
                            level: level,
                            text: args.map(a => String(a)).join(' ')
                        });
                        orig[level].apply(console, args);
                    };
                });
            })()
        "#;
        page.evaluate(js)
            .await
            .map_err(|e| KhoraError::Cdp(e.to_string()))?;
        Ok(())
    }

    /// Execute JavaScript and return the result as a JSON value.
    pub async fn eval(&self, expression: &str) -> KhoraResult<serde_json::Value> {
        let page = self.get_or_create_page().await?;
        let result = page
            .evaluate(expression)
            .await
            .map_err(|e| KhoraError::JavaScriptError(e.to_string()))?;

        // CDP's RemoteObject omits `value` entirely for `undefined` results (no
        // exception involved), so treat a missing value as JSON null rather than
        // failing the command.
        let value = match result.value() {
            Some(v) => v.clone(),
            None => serde_json::Value::Null,
        };

        Ok(value)
    }

    /// Check if the browser process is still alive.
    pub fn is_alive(&self) -> bool {
        !self._handler_handle.is_finished()
    }

    /// Close the browser and clean up the Chrome user data directory.
    /// Sends a graceful CDP `Browser.close`, then falls back to signaling the
    /// OS process directly (SIGTERM/SIGKILL) since `drop`ping a `Browser`
    /// obtained via `connect()` has no child handle to kill on its own.
    pub async fn close(mut self) -> KhoraResult<()> {
        let _ =
            tokio::time::timeout(std::time::Duration::from_millis(500), self.browser.close()).await;
        drop(self.browser);
        self._handler_handle.abort();
        if !crate::session::kill_process(self.pid).await {
            tracing::warn!(pid = self.pid, "failed to confirm Chrome process exited");
        }
        if let Some(ref dir) = self.data_dir {
            cleanup_data_dir(dir);
        }
        Ok(())
    }

    /// Emulate CSS media state on the current page (CDP
    /// `Emulation.setEmulatedMedia`).
    ///
    /// Unlike a device-metrics override, media emulation does *not* survive the
    /// CDP client disconnecting, so the caller is expected to have persisted
    /// `media` into the session — `connect()` reloads it and every page this
    /// client hands out gets it re-applied. Pass an empty `MediaEmulation` to
    /// hand control back to Chrome's own defaults.
    ///
    /// The page's own `matchMedia()` listeners receive real `change` events
    /// from this, which page-side JS cannot synthesize: `matchMedia()` returns
    /// a fresh `MediaQueryList` each call, so a dispatched event never reaches
    /// the object the app registered on.
    pub async fn emulate_media(&self, media: &MediaEmulation) -> KhoraResult<()> {
        // resolve_page() rather than get_or_create_page(): the latter would
        // first re-apply the session's *previous* state, and the page would see
        // a spurious `change` to it on the way to the state being asked for.
        let page = self.resolve_page().await?;
        apply_media(&page, media).await
    }

    /// Emulate a touch device on the current page (CDP
    /// `Emulation.setTouchEmulationEnabled`). `None` restores the real
    /// device's capabilities.
    ///
    /// Like media emulation this dies with the CDP connection, so the caller
    /// persists it on the session and `connect()` reloads it — that re-apply is
    /// the whole point of the command, because it runs *before* the navigation
    /// in the same invocation and so lands in time for a page's load-time
    /// feature detection.
    ///
    /// The three signals do not turn on together (measured, headless Chrome
    /// 150 / chromiumoxide 0.9.1):
    ///
    /// | signal | when it flips |
    /// |---|---|
    /// | `navigator.maxTouchPoints` | immediately |
    /// | `(pointer: coarse)`, `(any-pointer: coarse)`, `(hover: none)` | immediately |
    /// | `'ontouchstart' in window` | only on the next document |
    ///
    /// `ontouchstart` is installed on the window when the document is created,
    /// so an already-loaded page keeps reporting the old value until it
    /// navigates or reloads. Notably no device-metrics override is needed for
    /// the pointer/hover queries — touch emulation alone moves them.
    pub async fn emulate_touch(&self, touch: Option<TouchEmulation>) -> KhoraResult<()> {
        // resolve_page() rather than get_or_create_page(): re-applying the
        // session's *previous* touch state first would push the page through a
        // spurious `(pointer: coarse)` transition on the way to the one asked
        // for, which any matchMedia listener would see.
        let page = self.resolve_page().await?;
        apply_touch(&page, touch).await
    }

    /// Emulate an on-screen keyboard covering `keyboard.inset` CSS pixels of
    /// the visual viewport, and deliver the `resize` the page is waiting for.
    /// `None` retracts it. Returns the resulting `visualViewport.height`.
    ///
    /// Chrome cannot raise a keyboard and CDP models no such thing, so unlike
    /// every neighbouring `emulate-*` this is page-side: the instance property
    /// `window.visualViewport.height` is shadowed with a getter that subtracts
    /// the inset from the real one. **The layout viewport is deliberately left
    /// alone**, which is the entire point — a real keyboard shrinks only the
    /// visual viewport, so `vh`/`dvh`/`%` and `position: fixed` keep believing
    /// they have the full height. `set-viewport` shrinks both, so a keyboard
    /// bug tested through it disappears into a green result.
    ///
    /// The dispatched `resize` is real delivery, not a synthetic stand-in:
    /// `window.visualViewport` is a **singleton**, so it is the very object the
    /// app registered its listener on. (Contrast `matchMedia()`, which hands
    /// back a fresh `MediaQueryList` per call — the reason media emulation
    /// needed a CDP command to be testable at all.)
    ///
    /// Not reproduced by this, and worth stating rather than assuming:
    /// `visualViewport.offsetTop`, which a real browser makes non-zero when it
    /// scrolls the layout viewport to keep a focused input above the keyboard.
    /// Nothing here is focused and nothing scrolls, so an `offsetTop`-based
    /// translate stays unverified after a fully green run.
    pub async fn emulate_keyboard(&self, keyboard: Option<KeyboardEmulation>) -> KhoraResult<f64> {
        // resolve_page() rather than get_or_create_page(): the latter would
        // re-apply the session's *previous* inset first, and the page's own
        // resize listener would see a spurious intermediate height on the way
        // to the one being asked for — the same reason emulate_media and
        // emulate_touch bypass the re-apply.
        let page = self.resolve_page().await?;
        apply_keyboard(&page, keyboard).await?;
        // Dispatch only from the command, never from the per-connect re-apply:
        // re-applying an unchanged inset must not look like a keyboard event.
        let height = page
            .evaluate(KEYBOARD_DISPATCH_JS)
            .await
            .map_err(|e| KhoraError::Cdp(e.to_string()))?
            .into_value::<f64>()
            .map_err(|e| {
                KhoraError::Cdp(format!(
                    "page reported no window.visualViewport height: {e}"
                ))
            })?;
        Ok(height)
    }

    /// Probe how responsive the session's browser actually is, layer by layer.
    ///
    /// `is_alive()` answers a different, weaker question — whether *this
    /// process* still has a live handler task — and so reports "alive" for a
    /// browser that has stopped answering. This puts three bounded round-trips
    /// on the wire instead, ordered by how much of the browser each needs, so
    /// the report names the deepest layer still working rather than just
    /// failing.
    ///
    /// The layering is the whole point and not defensive over-engineering: on
    /// the incident this was written for (mesa task 557), `eval`, `click-at`
    /// and `wait-for` all worked while `wheel` timed out at 5s and 15s. A
    /// single `Runtime.evaluate` probe would have called that session healthy.
    /// `wheel` is the command that needs a compositor frame, so the compositor
    /// gets its own probe — the same 1x1 capture the trusted-input commands
    /// already use to pre-warm it.
    ///
    /// Each layer is bounded by `budget` independently: a healthy session
    /// answers all three in well under one, and only a genuinely wedged one
    /// pays the wait — which is the diagnosis being asked for. Read-only, like
    /// `targets`: it uses `find_page()` and never manufactures a tab, since a
    /// fresh `about:blank` would be a page whose health says nothing about the
    /// one you were driving.
    pub async fn health(&self, budget: std::time::Duration) -> SessionHealth {
        async fn timed<F, T, E>(budget: std::time::Duration, f: F) -> HealthCheck
        where
            F: std::future::Future<Output = Result<T, E>>,
            E: std::fmt::Display,
        {
            let started = std::time::Instant::now();
            let outcome = tokio::time::timeout(budget, f).await;
            let ms = started.elapsed().as_millis() as u64;
            match outcome {
                Ok(Ok(_)) => HealthCheck {
                    ok: Some(true),
                    ms,
                    detail: None,
                },
                Ok(Err(e)) => HealthCheck {
                    ok: Some(false),
                    ms,
                    detail: Some(e.to_string()),
                },
                Err(_) => HealthCheck {
                    ok: Some(false),
                    ms,
                    detail: Some(format!("timed out after {}ms", budget.as_millis())),
                },
            }
        }

        let protocol = timed(budget, self.browser.execute(GetVersionParams::default())).await;

        // Only the protocol layer can be probed without a page. Resolving one
        // is itself a round-trip that reaches into the renderer — measured on
        // a page busy-looping its main thread, `find_page()` is what blocks,
        // before `evaluate` is ever reached. So a JS-thread wedge surfaces
        // here, and both page-scoped layers have to inherit that one verdict:
        // neither was reached, and calling either a healthy skip would be the
        // false green this command exists to prevent.
        let resolve_started = std::time::Instant::now();
        let resolved = tokio::time::timeout(budget, self.find_page()).await;
        let resolve_ms = resolve_started.elapsed().as_millis() as u64;
        let unreached = |detail: String| SessionHealth {
            protocol: protocol.clone(),
            renderer: HealthCheck {
                ok: Some(false),
                ms: resolve_ms,
                detail: Some(detail.clone()),
            },
            compositor: HealthCheck {
                ok: Some(false),
                ms: resolve_ms,
                detail: Some(detail),
            },
        };
        let page = match resolved {
            Ok(Ok(Some(page))) => Some(page),
            Ok(Ok(None)) => None,
            Ok(Err(e)) => return unreached(format!("could not resolve a page: {e}")),
            Err(_) => {
                return unreached(format!(
                    "resolving a page timed out after {}ms",
                    budget.as_millis()
                ))
            }
        };

        let Some(page) = page else {
            // A browser with no tab open is idle, not sick. Condemning it here
            // would make `--check` fail a session that a `navigate` would
            // revive perfectly.
            let reason = "browser has no page open";
            return SessionHealth {
                protocol,
                renderer: HealthCheck::skipped(reason),
                compositor: HealthCheck::skipped(reason),
            };
        };

        // `1` rather than a bare literal: the value is irrelevant, what is
        // being measured is whether the renderer's JS thread answers at all.
        let renderer = timed(budget, page.evaluate("1")).await;
        let compositor = timed(budget, capture_1x1(&page)).await;

        SessionHealth {
            protocol,
            renderer,
            compositor,
        }
    }

    /// List the browser's open targets, flagging the foreground tab and the
    /// one khora's other commands would drive.
    ///
    /// The `driven`/`active` split is the point: khora resolves its own target
    /// by URL heuristics, Chrome decides which tab is foregrounded, and when
    /// those disagree every command still "succeeds" while trusted input goes
    /// somewhere invisible (mesa task 551). This is read-only — unlike every
    /// other command it never creates a tab when the browser has none, since
    /// manufacturing the thing you asked it to report would defeat the point.
    pub async fn targets(&self) -> KhoraResult<Vec<BrowserTarget>> {
        let driven = self
            .find_page()
            .await?
            .map(|p| p.target_id().inner().clone());

        // `document.visibilityState` is the only signal for "foreground tab"
        // — CDP's TargetInfo has no active flag and neither does
        // /json/list. Probing costs a round-trip per page, so bound each one:
        // a wedged renderer must not hang the listing that would diagnose it.
        let mut visible = std::collections::HashMap::new();
        if let Ok(pages) = self.browser.pages().await {
            for page in pages {
                let id = page.target_id().inner().clone();
                let probe = tokio::time::timeout(
                    std::time::Duration::from_millis(1000),
                    page.evaluate("document.visibilityState"),
                )
                .await;
                if let Ok(Ok(result)) = probe {
                    if let Ok(state) = result.into_value::<String>() {
                        visible.insert(id, state == "visible");
                    }
                }
            }
        }

        let returns = self
            .browser
            .execute(GetTargetsParams::default())
            .await
            .map_err(|e| KhoraError::Cdp(e.to_string()))?;

        Ok(returns
            .result
            .target_infos
            .iter()
            // The browser target is the connection itself, not something a
            // command could ever drive; /json/list omits it too.
            .filter(|t| t.r#type != "browser")
            .map(|t| {
                let id = t.target_id.inner().clone();
                BrowserTarget {
                    active: visible.get(&id).copied(),
                    driven: driven.as_ref() == Some(&id),
                    target_id: id,
                    target_type: t.r#type.clone(),
                    url: t.url.clone(),
                    title: t.title.clone(),
                }
            })
            .collect())
    }

    /// Get the active page, preferring non-blank pages.
    /// After connect(), fetch_targets() has been called so pages from previous
    /// connections are visible via pages().
    async fn get_or_create_page(&self) -> KhoraResult<Page> {
        let page = self.resolve_page().await?;
        if let Some(media) = &self.media {
            apply_media(&page, media).await?;
        }
        if let Some(touch) = &self.touch {
            apply_touch(&page, Some(*touch)).await?;
        }
        if let Some(viewport) = &self.viewport {
            apply_viewport(&page, Some(*viewport)).await?;
        }
        // Last, so the shadow's live getter reads a visual viewport that any
        // device-metrics override above has already settled.
        if let Some(keyboard) = &self.keyboard {
            apply_keyboard(&page, Some(*keyboard)).await?;
        }
        Ok(page)
    }

    /// Resolve where an element command should run.
    ///
    /// `frame` is a substring matched against frame and target URLs. OOPIF
    /// targets are checked first, since a cross-origin frame is the case the
    /// page's own JS cannot reach.
    async fn frame_context(
        &self,
        frame: Option<&str>,
        timeout_ms: u64,
    ) -> KhoraResult<FrameContext> {
        let page = self.get_or_create_page().await?;
        let Some(needle) = frame else {
            return Ok(FrameContext {
                page,
                same_origin: None,
                frame: None,
                label: None,
            });
        };

        // Cross-origin: the frame is its own CDP target, of type "iframe".
        let targets = self
            .browser
            .execute(GetTargetsParams::default())
            .await
            .map_err(|e| KhoraError::Cdp(e.to_string()))?;
        let oopif = targets
            .result
            .target_infos
            .iter()
            .find(|t| t.r#type == "iframe" && t.url.contains(needle));
        if let Some(target) = oopif {
            let frame_ws = Self::target_ws_url(&self.ws_url, target.target_id.inner());
            tracing::debug!(url = %target.url, ws = %frame_ws, "frame is an OOPIF target");
            let connection = FrameContext::connect(&frame_ws, timeout_ms).await?;
            return Ok(FrameContext {
                page,
                same_origin: None,
                frame: Some(connection),
                label: Some(needle.to_string()),
            });
        }

        // Same origin: a frame in this page, reachable from the page's JS.
        let ctx = FrameContext {
            page,
            same_origin: None,
            frame: None,
            label: None,
        };
        let probe = format!("{} !== null", FrameContext::frame_window_js(needle));
        if ctx.evaluate(probe).await? == serde_json::Value::Bool(true) {
            tracing::debug!(frame = needle, "frame is same-origin");
            return Ok(FrameContext {
                same_origin: Some(needle.to_string()),
                label: Some(needle.to_string()),
                ..ctx
            });
        }

        Err(KhoraError::FrameNotFound(needle.to_string()))
    }

    /// The per-target DevTools endpoint for `target_id`, derived from the
    /// browser-level one. This is the same URL `/json/list` advertises as a
    /// target's `webSocketDebuggerUrl` — built here rather than fetched so
    /// frame resolution needs no HTTP round trip, and no knowledge of which
    /// interface Chrome's HTTP endpoint is bound to.
    fn target_ws_url(browser_ws_url: &str, target_id: &str) -> String {
        let base = browser_ws_url
            .split_once("/devtools/")
            .map(|(base, _)| base)
            .unwrap_or(browser_ws_url);
        format!("{base}/devtools/page/{target_id}")
    }

    /// Pick the page `get_or_create_page()` should operate on, creating one if
    /// the browser genuinely has none.
    async fn resolve_page(&self) -> KhoraResult<Page> {
        match self.find_page().await? {
            Some(page) => Ok(page),
            None => self
                .browser
                .new_page("about:blank")
                .await
                .map_err(|e| KhoraError::Cdp(e.to_string())),
        }
    }

    /// The page khora would drive, or `None` if the browser has no page at
    /// all. Split out of `resolve_page()` so `targets()` can ask the same
    /// question without the tab-creating fallback.
    async fn find_page(&self) -> KhoraResult<Option<Page>> {
        let mut pages = self
            .browser
            .pages()
            .await
            .map_err(|e| KhoraError::Cdp(e.to_string()))?;

        // An empty target list almost always means Chrome's initial
        // `Target.targetCreated` has not been received yet, not that the
        // browser has no tab. Creating one here is worse than merely wasteful:
        // khora then drives its own CDP-created tab while Chrome's startup tab
        // (which resolves to `chrome://newtab/`) stays the active one, and a
        // trusted `Input.dispatchKeyEvent` into a background tab arrives with
        // its modifier bits intact but `key`/`code` stripped to
        // `Unidentified`/`""` — the intermittent corruption in mesa task 551.
        // So re-poll before concluding there is nothing to attach to; only a
        // browser with genuinely no pages pays this bound.
        if pages.is_empty() {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
            while pages.is_empty() && std::time::Instant::now() < deadline {
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                pages = self
                    .browser
                    .pages()
                    .await
                    .map_err(|e| KhoraError::Cdp(e.to_string()))?;
            }
        }

        if pages.is_empty() {
            return Ok(None);
        }

        // Try to find a non-blank page by CDP target URL first
        for page in &pages {
            if let Ok(Some(ref u)) = page.url().await {
                let url_str = u.as_str();
                if url_str != "about:blank"
                    && !url_str.is_empty()
                    && !url_str.starts_with("chrome://")
                {
                    return Ok(Some(page.clone()));
                }
            }
        }

        // Fallback: check each page's actual location via JS, in case the
        // target URL is stale (can happen after JS-based navigation).
        for page in &pages {
            if let Ok(result) = page.evaluate("location.href").await {
                if let Ok(url) = result.into_value::<String>() {
                    if url != "about:blank" && !url.is_empty() && !url.starts_with("chrome://") {
                        return Ok(Some(page.clone()));
                    }
                }
            }
        }
        // Fall back to the first page
        Ok(pages.into_iter().next())
    }
}

/// Shadow `visualViewport.height` with the real getter minus `__INSET__`.
///
/// The shadow is an *own* property on the singleton, over the accessor
/// inherited from `VisualViewport.prototype`, so `delete` restores the genuine
/// getter exactly. Reading the prototype descriptor each time — rather than
/// caching a "real" height — keeps this composable with `set-viewport`: the
/// emulated keyboard tracks whatever the real visual viewport becomes, and
/// re-installing over an existing shadow cannot compound (the prototype getter
/// is never the shadowed one).
const KEYBOARD_INSTALL_JS: &str = r#"
(() => {
  const vv = window.visualViewport;
  if (!vv) return null;
  const real = Object.getOwnPropertyDescriptor(
    Object.getPrototypeOf(vv), 'height');
  if (!real || !real.get) return null;
  const get = real.get;
  Object.defineProperty(vv, 'height', {
    get: () => get.call(vv) - __INSET__,
    configurable: true,
  });
  return vv.height;
})()
"#;

/// Drop the shadow, restoring the prototype's own getter.
const KEYBOARD_CLEAR_JS: &str = r#"
(() => {
  const vv = window.visualViewport;
  if (!vv) return null;
  delete vv.height;
  return vv.height;
})()
"#;

/// Fire `resize` on the object the page subscribed to, and read back the height.
const KEYBOARD_DISPATCH_JS: &str = r#"
(() => {
  const vv = window.visualViewport;
  if (!vv) return null;
  vv.dispatchEvent(new Event('resize'));
  return vv.height;
})()
"#;

/// Capture a 1x1 PNG — the cheapest CDP call that makes the compositor
/// produce a frame, and therefore both the pre-warm nudge the trusted-input
/// commands need and the health probe's compositor layer. Unbounded on
/// purpose: the two callers want different bounds, and a wedged compositor is
/// exactly what one of them is trying to measure.
async fn capture_1x1(page: &Page) -> Result<Vec<u8>, chromiumoxide::error::CdpError> {
    let params = ScreenshotParams::builder()
        .format(CaptureScreenshotFormat::Png)
        .clip(Viewport {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
            scale: 1.0,
        })
        .build();
    page.screenshot(params).await
}

/// Push an on-screen-keyboard state to a page, without dispatching `resize`.
///
/// Two installs, deliberately, because the shadow has two lifetimes to cover:
///
/// - `evaluate` fixes up the document already loaded;
/// - `Page.addScriptToEvaluateOnNewDocument` puts it ahead of the *next*
///   document's own scripts, so an app reading `visualViewport.height` at mount
///   sees the keyboard rather than the full height.
///
/// The registration is scoped to this CDP session and dies with it, which is
/// why the state is persisted on the session and re-applied on every connect.
/// That also makes ordering automatic: `navigate` re-applies before it
/// navigates, so emulate-then-navigate needs no discipline from the caller.
async fn apply_keyboard(page: &Page, keyboard: Option<KeyboardEmulation>) -> KhoraResult<()> {
    match keyboard {
        Some(k) => {
            let js = KEYBOARD_INSTALL_JS.replace("__INSET__", &k.inset.to_string());
            // Chromium only installs the new-document hook while the Page
            // domain is enabled on this CDP session; without this the
            // registration is accepted and silently never fires.
            page.execute(PageEnableParams::default())
                .await
                .map_err(|e| KhoraError::Cdp(e.to_string()))?;
            let params = AddScriptToEvaluateOnNewDocumentParams::builder()
                .source(js.clone())
                .build()
                .map_err(KhoraError::Cdp)?;
            page.execute(params)
                .await
                .map_err(|e| KhoraError::Cdp(e.to_string()))?;
            page.evaluate(js)
                .await
                .map_err(|e| KhoraError::Cdp(e.to_string()))?;
        }
        None => {
            // Only the live document needs clearing. The new-document script
            // was registered on a CDP session that has since closed, so Chrome
            // has already dropped it — there is nothing to unregister, and not
            // re-registering it is what makes `--off` stick across navigations.
            page.evaluate(KEYBOARD_CLEAR_JS)
                .await
                .map_err(|e| KhoraError::Cdp(e.to_string()))?;
        }
    }
    Ok(())
}

/// Push a media-emulation state to a page.
///
/// `Emulation.setEmulatedMedia` replaces the whole state on every call rather
/// than adding to it, so the full desired state is always sent — an empty
/// `media` string and no features is how the override is released.
async fn apply_media(page: &Page, media: &MediaEmulation) -> KhoraResult<()> {
    let params = SetEmulatedMediaParams::builder()
        .media(media.media.clone().unwrap_or_default())
        .features(
            media
                .features()
                .into_iter()
                .map(|(name, value)| MediaFeature::new(name, value))
                .collect::<Vec<_>>(),
        )
        .build();
    page.execute(params)
        .await
        .map_err(|e| KhoraError::Cdp(e.to_string()))?;
    Ok(())
}

/// Push a touch-emulation state to a page. `None` disables the override.
///
/// Disabling is sent explicitly rather than skipped: the page may still be
/// showing a document created while emulation was on, and while that document's
/// `'ontouchstart' in window` cannot be taken back, `navigator.maxTouchPoints`
/// and the pointer media queries drop back immediately.
async fn apply_touch(page: &Page, touch: Option<TouchEmulation>) -> KhoraResult<()> {
    // Chrome rejects a zero. The CLI already bounds this to 1..=16, so the
    // clamp only covers a library caller building the struct directly.
    let max_touch_points = touch.map_or(1, |t| t.max_touch_points).max(1);
    let params = SetTouchEmulationEnabledParams::builder()
        .enabled(touch.is_some())
        .max_touch_points(i64::from(max_touch_points))
        .build()
        .map_err(KhoraError::Cdp)?;
    page.execute(params)
        .await
        .map_err(|e| KhoraError::Cdp(e.to_string()))?;
    Ok(())
}

/// Push a viewport override to a page. `None` clears it.
///
/// Clearing goes through `Emulation.clearDeviceMetricsOverride` rather than
/// setting the real window's numbers back: only the clear takes
/// `deviceScaleFactor` and `mobile` off with it, and re-asserting a size would
/// leave the session emulating metrics it claims not to be emulating.
async fn apply_viewport(page: &Page, viewport: Option<ViewportEmulation>) -> KhoraResult<()> {
    match viewport {
        Some(v) => {
            let params = SetDeviceMetricsOverrideParams::builder()
                .width(i64::from(v.width))
                .height(i64::from(v.height))
                .device_scale_factor(v.dpr)
                .mobile(v.mobile)
                .build()
                .map_err(KhoraError::Cdp)?;
            page.execute(params)
                .await
                .map_err(|e| KhoraError::Cdp(e.to_string()))?;
        }
        None => {
            page.execute(ClearDeviceMetricsOverrideParams::default())
                .await
                .map_err(|e| KhoraError::Cdp(e.to_string()))?;
        }
    }
    Ok(())
}

/// Extract the OS PID of the launched Chrome process (best effort — 0 means unknown).
fn get_browser_pid(browser: &mut Browser) -> u32 {
    let pid = browser
        .get_mut_child()
        .and_then(|child| child.inner.id())
        .unwrap_or(0);
    if pid == 0 {
        tracing::warn!("could not determine Chrome PID; auto-reap will not fire for this session");
    }
    pid
}

/// Remove a Chrome user data directory.
/// Called after close() or when Chrome is already dead, to prevent stale profile data.
pub fn cleanup_data_dir(dir: &std::path::Path) {
    if let Err(e) = std::fs::remove_dir_all(dir) {
        tracing::warn!(?dir, %e, "failed to remove Chrome data dir");
    }
}

/// Filename prefix of every khora-created Chrome user data directory.
/// `launch()` builds them as `<temp_dir>/khora-chrome-<nanos>`.
pub const DATA_DIR_PREFIX: &str = "khora-chrome-";

/// Total size in bytes of a directory tree. Best effort: entries that cannot be
/// read are skipped rather than failing the walk, since this only feeds a
/// human-facing "freed N MB" figure.
fn dir_size_bytes(dir: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            // Don't follow symlinks — a profile can contain them, and their
            // target is not disk this directory owns.
            Ok(ft) if ft.is_dir() => dir_size_bytes(&entry.path()),
            Ok(ft) if ft.is_file() => entry.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

/// Remove orphaned Chrome user data directories left in the system temp dir by
/// khora launches whose session record is already gone.
///
/// Only paths named `<temp_dir>/khora-chrome-*` are considered, and any
/// directory in `keep` (the data dirs of sessions that still exist) is skipped,
/// so a concurrent session's live profile is never deleted out from under it.
///
/// A live session recorded by a build old enough to omit `data_dir` cannot be
/// caught out by this: commit 9dfcc2d introduced the `khora-chrome-` prefix and
/// the `data_dir` field together, and before it chromiumoxide used a fixed
/// `chromiumoxide-runner/` directory that this prefix never matches. So a
/// session with no `data_dir` owns no directory the sweep can see.
///
/// Returns `(directories removed, bytes freed)`.
pub fn purge_orphan_data_dirs(keep: &[std::path::PathBuf]) -> (usize, u64) {
    purge_orphan_data_dirs_in(&std::env::temp_dir(), keep, PURGE_MIN_AGE)
}

/// How recently a data dir may have been touched and still be spared.
///
/// `launch()` creates the data dir *before* Chrome starts and before the
/// session JSON that names it is written, so a launch racing this sweep owns a
/// directory that appears in neither `keep` nor any session record. Skipping
/// anything freshly modified closes that window; orphans are minutes-to-days
/// old, so nothing worth reclaiming is missed.
const PURGE_MIN_AGE: std::time::Duration = std::time::Duration::from_secs(60);

/// `purge_orphan_data_dirs` against an explicit parent directory and age bound,
/// so the sweep can be tested without touching the real temp dir.
fn purge_orphan_data_dirs_in(
    parent: &std::path::Path,
    keep: &[std::path::PathBuf],
    min_age: std::time::Duration,
) -> (usize, u64) {
    let Ok(entries) = std::fs::read_dir(parent) else {
        tracing::warn!(dir = ?parent, "purge: failed to read temp dir");
        return (0, 0);
    };

    let mut removed = 0;
    let mut freed = 0;

    for entry in entries.flatten() {
        let path = entry.path();
        if !entry.file_type().is_ok_and(|ft| ft.is_dir()) {
            continue;
        }
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with(DATA_DIR_PREFIX)
        {
            continue;
        }
        if keep.iter().any(|k| k == &path) {
            continue;
        }
        let recently_touched = entry
            .metadata()
            .and_then(|m| m.modified())
            .and_then(|t| t.elapsed().map_err(std::io::Error::other))
            .is_ok_and(|age| age < min_age);
        if recently_touched {
            tracing::debug!(?path, "purge: skipping recently modified data dir");
            continue;
        }

        let size = dir_size_bytes(&path);
        match std::fs::remove_dir_all(&path) {
            Ok(()) => {
                removed += 1;
                freed += size;
            }
            Err(e) => tracing::warn!(?path, %e, "purge: failed to remove orphaned data dir"),
        }
    }

    (removed, freed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `khora-chrome-*`-style directory holding one file of `bytes`.
    fn make_data_dir(parent: &std::path::Path, name: &str, bytes: usize) -> std::path::PathBuf {
        let dir = parent.join(name);
        std::fs::create_dir_all(dir.join("Default")).unwrap();
        std::fs::write(dir.join("Default").join("Cookies"), vec![b'x'; bytes]).unwrap();
        dir
    }

    #[test]
    fn purge_removes_orphans_but_spares_live_and_foreign_dirs() {
        // A purge that swept the whole temp dir, or that ignored `keep`, would
        // delete a concurrently-running session's profile out from under it.
        let parent = std::env::temp_dir().join(format!(
            "khora-purge-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&parent).unwrap();

        let orphan_a = make_data_dir(&parent, "khora-chrome-1000", 512);
        let orphan_b = make_data_dir(&parent, "khora-chrome-2000", 512);
        let live = make_data_dir(&parent, "khora-chrome-3000", 512);
        let foreign = make_data_dir(&parent, "some-other-tool-4000", 512);

        let (removed, freed) = purge_orphan_data_dirs_in(
            &parent,
            std::slice::from_ref(&live),
            std::time::Duration::ZERO,
        );

        assert_eq!(removed, 2, "only the two orphaned khora dirs should go");
        assert_eq!(freed, 1024, "freed bytes should sum the removed trees");
        assert!(!orphan_a.exists());
        assert!(!orphan_b.exists());
        assert!(live.exists(), "a session's live profile must be spared");
        assert!(foreign.exists(), "non-khora dirs must never be touched");

        std::fs::remove_dir_all(&parent).ok();
    }

    #[test]
    fn purge_spares_a_freshly_created_data_dir() {
        // A launch racing the sweep has already created its data dir but has
        // not yet written the session JSON that would put it in `keep`, so age
        // is the only thing standing between it and deletion.
        let parent = std::env::temp_dir().join(format!(
            "khora-purge-race-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&parent).unwrap();
        let in_flight = make_data_dir(&parent, "khora-chrome-5000", 128);

        let (removed, freed) = purge_orphan_data_dirs_in(&parent, &[], PURGE_MIN_AGE);

        assert_eq!(removed, 0);
        assert_eq!(freed, 0);
        assert!(
            in_flight.exists(),
            "a data dir created seconds ago belongs to an in-flight launch"
        );

        std::fs::remove_dir_all(&parent).ok();
    }

    #[test]
    fn dir_size_bytes_sums_nested_files() {
        let parent = std::env::temp_dir().join(format!(
            "khora-size-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let dir = make_data_dir(&parent, "khora-chrome-1", 300);
        std::fs::write(dir.join("top-level"), vec![b'y'; 45]).unwrap();

        assert_eq!(dir_size_bytes(&dir), 345);

        std::fs::remove_dir_all(&parent).ok();
    }

    /// Virtual-key codes a terminal emulator reads as an editing or
    /// navigation key rather than as text: Backspace/Tab/Enter, Escape,
    /// PageUp..ArrowDown, Insert, Delete. A printable character that reports
    /// one of these is delivered as an escape sequence instead of itself.
    fn is_navigation_vk(vk: i64) -> bool {
        matches!(vk, 8 | 9 | 13 | 27 | 33..=46)
    }

    #[test]
    fn char_key_info_maps_hyphen_to_the_minus_key() {
        // Regression: '-' is U+002D = 45 = VK_INSERT. Reporting the codepoint
        // as the virtual key made xterm.js emit ESC[2~ for every hyphen, so
        // `type-keys $S SEL 'stty -echo'` reached the shell garbled.
        assert_eq!(
            CdpClient::char_key_info('-'),
            ("-".to_string(), "Minus".to_string(), 189)
        );
    }

    #[test]
    fn char_key_info_maps_plus_to_the_shifted_equal_key() {
        assert_eq!(
            CdpClient::char_key_info('+'),
            ("+".to_string(), "Equal".to_string(), 187)
        );
    }

    #[test]
    fn char_key_info_never_gives_punctuation_a_navigation_key_code() {
        // The collision class is much wider than '-': '!' is 33 (PageUp),
        // '&' is 38 (ArrowUp), '.' is 46 (Delete), '{' is 123 (F12)...
        for c in ('!'..='~').filter(|c| !c.is_ascii_alphanumeric()) {
            let (key, code, vk) = CdpClient::char_key_info(c);
            assert_eq!(key, c.to_string(), "key must be the character itself");
            assert_ne!(code, "Unidentified", "{c:?} has no physical key mapped");
            assert!(
                !is_navigation_vk(vk),
                "{c:?} maps to virtual key {vk}, an editing/navigation key"
            );
            assert!(
                !(112..=123).contains(&vk),
                "{c:?} maps to virtual key {vk}, a function key"
            );
            assert!(
                vk >= 48,
                "{c:?} maps to virtual key {vk}; terminals treat keyCode < 48 as non-printable"
            );
        }
    }

    #[test]
    fn char_key_info_keeps_alphanumerics_and_space_on_their_own_keys() {
        assert_eq!(
            CdpClient::char_key_info('h'),
            ("h".to_string(), "KeyH".to_string(), 72)
        );
        // `key` preserves case; `code` is the physical key either way.
        assert_eq!(
            CdpClient::char_key_info('H'),
            ("H".to_string(), "KeyH".to_string(), 72)
        );
        assert_eq!(
            CdpClient::char_key_info('7'),
            ("7".to_string(), "Digit7".to_string(), 55)
        );
        assert_eq!(
            CdpClient::char_key_info(' '),
            (" ".to_string(), "Space".to_string(), 32)
        );
    }

    #[test]
    fn char_key_info_maps_newline_and_tab_to_their_real_keys() {
        // U+000A/U+0009 as virtual keys would be 10 (unassigned) and 9 (Tab);
        // a newline in typed text should submit like Enter, not fire nothing.
        assert_eq!(
            CdpClient::char_key_info('\n'),
            ("\n".to_string(), "Enter".to_string(), 13)
        );
        assert_eq!(
            CdpClient::char_key_info('\t'),
            ("\t".to_string(), "Tab".to_string(), 9)
        );
    }

    #[test]
    fn char_key_info_falls_back_without_impersonating_an_editing_key() {
        // Non-ASCII keeps the codepoint — above the navigation range, and
        // terminals need a nonzero keyCode to treat the event as printable.
        let (key, code, vk) = CdpClient::char_key_info('é');
        assert_eq!(key, "é");
        assert_eq!(code, "Unidentified");
        assert_eq!(vk, 233);

        // Control characters report 0 rather than a navigation key: U+0008
        // would otherwise be Backspace, U+001B Escape.
        for c in ['\u{8}', '\u{1b}', '\u{7}'] {
            let (_, code, vk) = CdpClient::char_key_info(c);
            assert_eq!(code, "Unidentified");
            assert_eq!(vk, 0, "{c:?} must not impersonate an editing key");
        }
    }
}
