use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams;
use chromiumoxide::cdp::browser_protocol::input::{
    DispatchKeyEventParams, DispatchKeyEventType, DispatchMouseEventParams, DispatchMouseEventType,
    MouseButton,
};
use chromiumoxide::cdp::browser_protocol::network::{
    EnableParams as NetworkEnableParams, SetCacheDisabledParams,
};
use chromiumoxide::cdp::browser_protocol::page::{CaptureScreenshotFormat, Viewport};
use chromiumoxide::page::ScreenshotParams;
use chromiumoxide::Page;
use futures::StreamExt;
use khora_core::element::{BoundingBox, ConsoleMessage, ElementInfo, NetworkRequest};
use khora_core::error::{KhoraError, KhoraResult};
use khora_core::session::SessionInfo;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::chrome::find_chrome;

/// High-level CDP client wrapping chromiumoxide Browser.
pub struct CdpClient {
    browser: Browser,
    _handler_handle: tokio::task::JoinHandle<()>,
    data_dir: Option<PathBuf>,
    pid: u32,
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
        };

        let client = Self {
            browser,
            _handler_handle: handler_handle,
            data_dir: Some(data_dir),
            pid,
        };

        Ok((client, session))
    }

    /// Connect to an existing Chrome instance using session info.
    /// Fetches existing targets so that pages from previous sessions are visible.
    pub async fn connect(session: &SessionInfo) -> KhoraResult<Self> {
        tracing::info!(session_id = %session.id, ws_url = %session.ws_url, "connecting to Chrome");

        let (mut browser, mut handler) = Browser::connect(&session.ws_url)
            .await
            .map_err(|e| KhoraError::SessionDead(format!("{}: {e}", session.id)))?;

        let handler_handle = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                if event.is_err() {
                    break;
                }
            }
        });

        // Fetch existing targets so pages from previous connections are visible
        let _ = browser.fetch_targets().await;
        // Give a moment for pages to be registered internally
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        Ok(Self {
            browser,
            _handler_handle: handler_handle,
            data_dir: None,
            pid: session.pid,
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
    pub async fn find_elements(&self, selector: &str) -> KhoraResult<Vec<ElementInfo>> {
        let page = self.get_or_create_page().await?;

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

        let result = page
            .evaluate(js)
            .await
            .map_err(|e| KhoraError::Cdp(e.to_string()))?;

        let value = result
            .into_value::<serde_json::Value>()
            .map_err(|e| KhoraError::Cdp(e.to_string()))?;

        let elements: Vec<serde_json::Value> = match value {
            serde_json::Value::Array(arr) => arr,
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

        if elements.is_empty() {
            return Err(KhoraError::ElementNotFound(selector.to_string()));
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
    pub async fn click(&self, selector: &str) -> KhoraResult<()> {
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
                el.click();
                return {{ found: true }};
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

        if value.get("found").and_then(|v| v.as_bool()) != Some(true) {
            if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
                return Err(KhoraError::JavaScriptError(format!(
                    "invalid selector {selector:?}: {err}"
                )));
            }
            return Err(KhoraError::ElementNotFound(selector.to_string()));
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
    pub async fn type_text(&self, selector: &str, text: &str) -> KhoraResult<()> {
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
        Ok(())
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
    /// With no `selector`, captures the full page. With a `selector`, scrolls
    /// the first matching element into view and crops the shot to its bounding
    /// box; returns [`KhoraError::ElementNotFound`] if nothing matches or the
    /// element has no visible area, rather than falling back to a full-page shot.
    pub async fn screenshot(&self, selector: Option<&str>) -> KhoraResult<Vec<u8>> {
        let page = self.get_or_create_page().await?;
        let mut builder = ScreenshotParams::builder().format(CaptureScreenshotFormat::Png);
        builder = match selector {
            // capture_beyond_viewport renders the whole clip even when the
            // element is taller/wider than the viewport; without it CDP only
            // paints the viewport-visible part and the rest comes out blank.
            Some(sel) => builder
                .clip(self.element_clip(&page, sel).await?)
                .capture_beyond_viewport(true),
            None => builder.full_page(true),
        };
        let png = page
            .screenshot(builder.build())
            .await
            .map_err(|e| KhoraError::ScreenshotFailed(e.to_string()))?;
        Ok(png)
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
    /// Shared by [`Self::drag`] and the step-wise [`Self::mouse_down`] /
    /// [`Self::mouse_move`] / [`Self::mouse_up`] primitives. `buttons` is set
    /// to 1 (left button held) for every event type, matching the state a
    /// real press-move-release gesture reports throughout.
    async fn dispatch_mouse_event(
        &self,
        page: &Page,
        kind: DispatchMouseEventType,
        x: f64,
        y: f64,
    ) -> KhoraResult<()> {
        let event = DispatchMouseEventParams::builder()
            .r#type(kind)
            .x(x)
            .y(y)
            .button(MouseButton::Left)
            .buttons(1)
            .click_count(1)
            .build()
            .map_err(KhoraError::Cdp)?;
        page.execute(event)
            .await
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
    ) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;
        let delay = std::time::Duration::from_millis(delay_ms);

        self.dispatch_mouse_event(&page, DispatchMouseEventType::MousePressed, from.0, from.1)
            .await?;

        for i in 1..=steps {
            tokio::time::sleep(delay).await;
            let t = f64::from(i) / f64::from(steps);
            let x = from.0 + (to.0 - from.0) * t;
            let y = from.1 + (to.1 - from.1) * t;
            self.dispatch_mouse_event(&page, DispatchMouseEventType::MouseMoved, x, y)
                .await?;
        }

        tokio::time::sleep(delay).await;
        self.dispatch_mouse_event(&page, DispatchMouseEventType::MouseReleased, to.0, to.1)
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
    pub async fn mouse_down(&self, at: (f64, f64)) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;
        self.dispatch_mouse_event(&page, DispatchMouseEventType::MousePressed, at.0, at.1)
            .await
    }

    /// Move the mouse to a point with a trusted event, carrying over
    /// whatever button state a prior [`Self::mouse_down`] established.
    pub async fn mouse_move(&self, at: (f64, f64)) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;
        self.dispatch_mouse_event(&page, DispatchMouseEventType::MouseMoved, at.0, at.1)
            .await
    }

    /// Release the left mouse button at a point with a trusted event,
    /// completing a gesture started with [`Self::mouse_down`].
    pub async fn mouse_up(&self, at: (f64, f64)) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;
        self.dispatch_mouse_event(&page, DispatchMouseEventType::MouseReleased, at.0, at.1)
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
    pub async fn key_press(&self, combo: &str) -> KhoraResult<()> {
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
        page.execute(down)
            .await
            .map_err(|e| KhoraError::Cdp(e.to_string()))?;

        let up = builder
            .r#type(DispatchKeyEventType::KeyUp)
            .build()
            .map_err(KhoraError::Cdp)?;
        page.execute(up)
            .await
            .map_err(|e| KhoraError::Cdp(e.to_string()))?;
        Ok(())
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

    /// Override the page viewport via CDP Emulation.setDeviceMetricsOverride.
    ///
    /// Headless Chrome clamps `launch --window-size` to a ~500px minimum inner
    /// width, so phone-width QA (375-430px) needs a metrics override instead.
    /// `dpr` 0.0 keeps the current device scale factor; `mobile` enables
    /// mobile emulation (meta-viewport handling, mobile UA hints).
    pub async fn set_viewport(
        &self,
        width: u32,
        height: u32,
        dpr: f64,
        mobile: bool,
    ) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;
        let params = SetDeviceMetricsOverrideParams::builder()
            .width(width as i64)
            .height(height as i64)
            .device_scale_factor(dpr)
            .mobile(mobile)
            .build()
            .map_err(KhoraError::Cdp)?;
        page.execute(params)
            .await
            .map_err(|e| KhoraError::Cdp(e.to_string()))?;
        Ok(())
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

        let value = result
            .into_value::<serde_json::Value>()
            .map_err(|e| KhoraError::JavaScriptError(e.to_string()))?;

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

    /// Get the active page, preferring non-blank pages.
    /// After connect(), fetch_targets() has been called so pages from previous
    /// connections are visible via pages().
    async fn get_or_create_page(&self) -> KhoraResult<Page> {
        let pages = self
            .browser
            .pages()
            .await
            .map_err(|e| KhoraError::Cdp(e.to_string()))?;

        if pages.is_empty() {
            return self
                .browser
                .new_page("about:blank")
                .await
                .map_err(|e| KhoraError::Cdp(e.to_string()));
        }

        // Try to find a non-blank page by CDP target URL first
        for page in &pages {
            if let Ok(Some(ref u)) = page.url().await {
                let url_str = u.as_str();
                if url_str != "about:blank"
                    && !url_str.is_empty()
                    && !url_str.starts_with("chrome://")
                {
                    return Ok(page.clone());
                }
            }
        }

        // Fallback: check each page's actual location via JS, in case the
        // target URL is stale (can happen after JS-based navigation).
        for page in &pages {
            if let Ok(result) = page.evaluate("location.href").await {
                if let Ok(url) = result.into_value::<String>() {
                    if url != "about:blank" && !url.is_empty() && !url.starts_with("chrome://") {
                        return Ok(page.clone());
                    }
                }
            }
        }
        // Fall back to the first page
        Ok(pages.into_iter().next().unwrap())
    }
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
