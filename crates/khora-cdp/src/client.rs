use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use chromiumoxide::page::ScreenshotParams;
use chromiumoxide::Page;
use futures::StreamExt;
use khora_core::element::{BoundingBox, ConsoleMessage, ElementInfo};
use khora_core::error::{KhoraError, KhoraResult};
use khora_core::session::SessionInfo;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::chrome::find_chrome;

/// High-level CDP client wrapping chromiumoxide Browser.
pub struct CdpClient {
    browser: Browser,
    _handler_handle: tokio::task::JoinHandle<()>,
}

impl CdpClient {
    /// Launch a new Chrome instance and return a client + session info.
    pub async fn launch(headless: bool) -> KhoraResult<(Self, SessionInfo)> {
        let chrome_path = find_chrome()?;
        tracing::info!(?chrome_path, headless, "launching Chrome");

        let mut builder = BrowserConfig::builder()
            .chrome_executable(chrome_path)
            .arg("--disable-extensions")
            .arg("--disable-default-apps")
            .arg("--no-first-run")
            .arg("--disable-background-timer-throttling")
            .arg("--disable-backgrounding-occluded-windows");

        if !headless {
            builder = builder.with_head();
        }

        let config = builder
            .build()
            .map_err(|e| KhoraError::LaunchFailed(e.to_string()))?;

        let (browser, mut handler) = Browser::launch(config)
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

        // Get PID from the debug info (best effort — 0 means unknown)
        let pid = get_browser_pid(&ws_url);

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
        };

        let client = Self {
            browser,
            _handler_handle: handler_handle,
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
        })
    }

    /// Navigate to a URL in the current page.
    /// Uses JS-based navigation + readyState polling instead of CDP Page.navigate,
    /// because chromiumoxide's goto() waits for lifecycle events that may not fire
    /// on reconnected sessions.
    pub async fn navigate(&self, url: &str) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;
        let js = format!(
            "window.location.href = {}",
            serde_json::to_string(url).unwrap_or_default()
        );
        page.evaluate(js)
            .await
            .map_err(|e| KhoraError::NavigationFailed(e.to_string()))?;

        // Poll for document ready state
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
                const els = document.querySelectorAll({selector});
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
    pub async fn click(&self, selector: &str) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;
        let element = page
            .find_element(selector)
            .await
            .map_err(|e| KhoraError::ElementNotFound(format!("{selector}: {e}")))?;
        element
            .click()
            .await
            .map_err(|e| KhoraError::Cdp(format!("click failed: {e}")))?;
        Ok(())
    }

    /// Type text into an element matching a CSS selector.
    pub async fn type_text(&self, selector: &str, text: &str) -> KhoraResult<()> {
        let page = self.get_or_create_page().await?;
        let element = page
            .find_element(selector)
            .await
            .map_err(|e| KhoraError::ElementNotFound(format!("{selector}: {e}")))?;
        element
            .click()
            .await
            .map_err(|e| KhoraError::Cdp(format!("focus failed: {e}")))?;
        element
            .type_str(text)
            .await
            .map_err(|e| KhoraError::Cdp(format!("type failed: {e}")))?;
        Ok(())
    }

    /// Get text content of elements matching a CSS selector.
    pub async fn get_text(&self, selector: &str) -> KhoraResult<Vec<String>> {
        let page = self.get_or_create_page().await?;
        let js = format!(
            r#"
            (() => {{
                const els = document.querySelectorAll({selector});
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
                const el = document.querySelector({selector});
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
            r#"document.querySelector({selector}) !== null"#,
            selector = serde_json::to_string(selector).unwrap_or_default()
        );

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(timeout_ms);

        loop {
            let result = page
                .evaluate(js.as_str())
                .await
                .map_err(|e| KhoraError::Cdp(e.to_string()))?;

            if let Ok(found) = result.into_value::<bool>() {
                if found {
                    return Ok(());
                }
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
            r#"document.querySelector({selector}) === null"#,
            selector = serde_json::to_string(selector).unwrap_or_default()
        );

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(timeout_ms);

        loop {
            let result = page
                .evaluate(js.as_str())
                .await
                .map_err(|e| KhoraError::Cdp(e.to_string()))?;

            if let Ok(gone) = result.into_value::<bool>() {
                if gone {
                    return Ok(());
                }
            }

            if start.elapsed() >= timeout {
                return Err(KhoraError::Timeout(timeout_ms));
            }

            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Capture a full-page screenshot, returned as PNG bytes.
    pub async fn screenshot(&self) -> KhoraResult<Vec<u8>> {
        let page = self.get_or_create_page().await?;
        let png = page
            .screenshot(
                ScreenshotParams::builder()
                    .format(CaptureScreenshotFormat::Png)
                    .full_page(true)
                    .build(),
            )
            .await
            .map_err(|e| KhoraError::ScreenshotFailed(e.to_string()))?;
        Ok(png)
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

    /// Close the browser.
    pub async fn close(self) -> KhoraResult<()> {
        drop(self.browser);
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        self._handler_handle.abort();
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

        // Try to find a non-blank page first
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

        // Fall back to the first page
        Ok(pages.into_iter().next().unwrap())
    }
}

/// Extract browser PID from the WebSocket URL (best effort).
fn get_browser_pid(_ws_url: &str) -> u32 {
    // The PID isn't directly in the WS URL. We'll use 0 to indicate unknown.
    // The session file will still track the session for reconnection purposes.
    0
}
