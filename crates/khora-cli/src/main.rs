use clap::{Parser, Subcommand};
use khora_cdp::CdpClient;
use khora_core::error::KhoraError;
use khora_core::session::SessionInfo;
use khora_core::OutputFormat;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Clone, Debug, PartialEq, Eq)]
struct WindowSize {
    width: u32,
    height: u32,
}

impl std::str::FromStr for WindowSize {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (w, h) = s
            .split_once('x')
            .ok_or_else(|| format!("expected WxH (e.g. 1920x1080), got: {s}"))?;
        let width: u32 = w.parse().map_err(|_| format!("invalid width: {w}"))?;
        let height: u32 = h.parse().map_err(|_| format!("invalid height: {h}"))?;
        if width == 0 || height == 0 {
            return Err(format!("dimensions must be > 0, got: {width}x{height}"));
        }
        Ok(WindowSize { width, height })
    }
}

impl std::fmt::Display for WindowSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Point {
    x: f64,
    y: f64,
}

impl std::str::FromStr for Point {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (x, y) = s
            .split_once(',')
            .ok_or_else(|| format!("expected X,Y (e.g. 100,250), got: {s}"))?;
        let x: f64 = x.trim().parse().map_err(|_| format!("invalid x: {x}"))?;
        let y: f64 = y.trim().parse().map_err(|_| format!("invalid y: {y}"))?;
        if !x.is_finite() || !y.is_finite() {
            return Err(format!("coordinates must be finite, got: {s}"));
        }
        Ok(Point { x, y })
    }
}

impl std::fmt::Display for Point {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{},{}", self.x, self.y)
    }
}

#[derive(Parser)]
#[command(
    name = "khora",
    about = "Web app QA automation via Chrome DevTools Protocol",
    version
)]
struct Cli {
    #[arg(
        short,
        long,
        default_value = "text",
        global = true,
        env = "KHORA_FORMAT"
    )]
    format: OutputFormat,

    #[arg(
        short,
        long,
        default_value = "5000",
        global = true,
        env = "KHORA_TIMEOUT"
    )]
    timeout: u64,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start Chrome and create a new session
    Launch {
        /// Show browser window (default is headless)
        #[arg(long)]
        visible: bool,
        /// Chrome window size as WxH (e.g. 1920x1080)
        #[arg(long, default_value_t = WindowSize { width: 1920, height: 1080 })]
        window_size: WindowSize,
    },

    /// Navigate to a URL
    Navigate {
        /// Session ID
        session: String,
        /// URL to navigate to
        url: String,
        /// Bypass the browser cache for this navigation (CDP Network.setCacheDisabled)
        #[arg(long)]
        no_cache: bool,
    },

    /// Override the viewport size (CDP Emulation.setDeviceMetricsOverride)
    SetViewport {
        /// Session ID
        session: String,
        /// Viewport size as WxH (e.g. 390x844)
        size: WindowSize,
        /// Device pixel ratio (0 keeps the current value)
        #[arg(default_value_t = 0.0, value_parser = parse_dpr)]
        dpr: f64,
        /// Emulate a mobile device (meta-viewport handling, mobile UA hints)
        #[arg(long)]
        mobile: bool,
    },

    /// Find elements by CSS selector
    Find {
        /// Session ID
        session: String,
        /// CSS selector
        selector: String,
    },

    /// Click an element by CSS selector
    Click {
        /// Session ID
        session: String,
        /// CSS selector
        selector: String,
    },

    /// Type text into an element
    Type {
        /// Session ID
        session: String,
        /// CSS selector for the input element
        selector: String,
        /// Text to type
        text: String,
    },

    /// Drag from one point to another with trusted mouse events (CDP Input.dispatchMouseEvent)
    Drag {
        /// Session ID
        session: String,
        /// Start point as X,Y in viewport CSS pixels (e.g. 100,250)
        #[arg(allow_hyphen_values = true)]
        from: Point,
        /// End point as X,Y in viewport CSS pixels
        #[arg(allow_hyphen_values = true)]
        to: Point,
        /// Number of intermediate mouse-move events along the path
        #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u32).range(1..))]
        steps: u32,
        /// Delay between mouse events in milliseconds
        #[arg(long, default_value_t = 16)]
        delay: u64,
    },

    /// Capture a screenshot
    Screenshot {
        /// Session ID
        session: String,
        /// Output file path (default: khora-screenshot.png)
        #[arg(long, short)]
        output: Option<String>,
        /// CSS selector to crop the shot to; errors if it matches nothing
        #[arg(long, short)]
        selector: Option<String>,
    },

    /// Get text content of matching elements
    Text {
        /// Session ID
        session: String,
        /// CSS selector
        selector: String,
    },

    /// Get attribute value of an element
    Attribute {
        /// Session ID
        session: String,
        /// CSS selector
        selector: String,
        /// Attribute name
        attr: String,
    },

    /// Wait for an element to appear
    WaitFor {
        /// Session ID
        session: String,
        /// CSS selector
        selector: String,
        /// Timeout in milliseconds
        #[arg(long)]
        timeout: Option<u64>,
    },

    /// Wait for an element to disappear
    WaitGone {
        /// Session ID
        session: String,
        /// CSS selector
        selector: String,
        /// Timeout in milliseconds
        #[arg(long)]
        timeout: Option<u64>,
    },

    /// Read console messages
    Console {
        /// Session ID
        session: String,
    },

    /// List captured network requests (fetch and XHR)
    Network {
        /// Session ID
        session: String,
    },

    /// Execute JavaScript and return the result
    Eval {
        /// Session ID
        session: String,
        /// JavaScript expression
        js: String,
    },

    /// Close browser and clean up session
    Kill {
        /// Session ID (omit with --all to kill every session)
        #[arg(required_unless_present = "all", conflicts_with = "all")]
        session: Option<String>,
        /// Kill all active sessions
        #[arg(long)]
        all: bool,
    },

    /// Check if a session is still alive
    Status {
        /// Session ID (omit to list all sessions)
        session: Option<String>,
    },

    /// Reap stale (dead-process) sessions; optionally kill live or old sessions
    Reap {
        /// Also kill live sessions (same as kill --all)
        #[arg(long)]
        all: bool,
        /// Kill sessions older than this duration (e.g. 30m, 2h, 24h, 0s)
        #[arg(long)]
        older_than: Option<String>,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match run(&cli).await {
        Ok(output) => {
            if !output.is_empty() {
                println!("{output}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(e.exit_code() as u8)
        }
    }
}

/// Parse a Go-style duration string into a `std::time::Duration`.
/// Supported suffixes: `s` (seconds), `m` (minutes), `h` (hours).
fn parse_duration(s: &str) -> Result<std::time::Duration, String> {
    if s.is_empty() {
        return Err("empty duration".to_string());
    }
    let (num_str, secs_per_unit) = if let Some(n) = s.strip_suffix('s') {
        (n, 1u64)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60u64)
    } else if let Some(n) = s.strip_suffix('h') {
        (n, 3600u64)
    } else {
        return Err(format!("unsupported unit in {s:?} — use s, m, or h"));
    };
    let n: u64 = num_str
        .parse()
        .map_err(|_| format!("invalid number {num_str:?} in duration {s:?}"))?;
    Ok(std::time::Duration::from_secs(n * secs_per_unit))
}

/// Parse a device pixel ratio argument: a non-negative float (0 = keep current).
fn parse_dpr(s: &str) -> Result<f64, String> {
    let dpr: f64 = s.parse().map_err(|_| format!("invalid dpr: {s}"))?;
    if !dpr.is_finite() || dpr < 0.0 {
        return Err(format!("dpr must be >= 0, got: {s}"));
    }
    Ok(dpr)
}

async fn run(cli: &Cli) -> Result<String, KhoraError> {
    // Auto-reap dead sessions on every invocation — best effort.
    // Skip for `reap` itself: it handles cleanup and must report what it reaped.
    if !matches!(cli.command, Command::Reap { .. }) {
        khora_cdp::reap_stale_sessions();
    }

    match &cli.command {
        Command::Launch {
            visible,
            window_size,
        } => {
            let (client, session) =
                CdpClient::launch(!visible, (window_size.width, window_size.height)).await?;
            session.save()?;

            // Install console and network hooks for this session
            if let Err(e) = client.install_console_hook().await {
                tracing::warn!("failed to install console hook: {e}");
            }
            if let Err(e) = client.install_network_hook().await {
                tracing::warn!("failed to install network hook: {e}");
            }

            // Keep the browser alive by leaking the client
            // (the handler task keeps Chrome running)
            std::mem::forget(client);

            Ok(khora_core::output::format_session(&session, cli.format))
        }

        Command::Navigate {
            session,
            url,
            no_cache,
        } => {
            let session_info = khora_cdp::load_and_verify(session)?;
            let client = CdpClient::connect(&session_info).await?;
            client.navigate(url, *no_cache).await?;

            match cli.format {
                OutputFormat::Text => {
                    if *no_cache {
                        Ok(format!("Navigated to: {url} (cache bypassed)"))
                    } else {
                        Ok(format!("Navigated to: {url}"))
                    }
                }
                OutputFormat::Json => Ok(serde_json::to_string_pretty(
                    &serde_json::json!({ "action": "navigate", "url": url, "no_cache": no_cache }),
                )
                .unwrap()),
            }
        }

        Command::SetViewport {
            session,
            size,
            dpr,
            mobile,
        } => {
            let session_info = khora_cdp::load_and_verify(session)?;
            let client = CdpClient::connect(&session_info).await?;
            client
                .set_viewport(size.width, size.height, *dpr, *mobile)
                .await?;

            match cli.format {
                OutputFormat::Text => {
                    let mut msg = format!("Viewport set: {size}");
                    if *dpr > 0.0 {
                        msg.push_str(&format!(" dpr={dpr}"));
                    }
                    if *mobile {
                        msg.push_str(" (mobile)");
                    }
                    Ok(msg)
                }
                OutputFormat::Json => Ok(serde_json::to_string_pretty(&serde_json::json!({
                    "action": "set_viewport",
                    "width": size.width,
                    "height": size.height,
                    "dpr": dpr,
                    "mobile": mobile,
                }))
                .unwrap()),
            }
        }

        Command::Find { session, selector } => {
            let session_info = khora_cdp::load_and_verify(session)?;
            let client = CdpClient::connect(&session_info).await?;
            let elements = client.find_elements(selector).await?;
            Ok(khora_core::output::format_elements(&elements, cli.format))
        }

        Command::Click { session, selector } => {
            let session_info = khora_cdp::load_and_verify(session)?;
            let client = CdpClient::connect(&session_info).await?;
            client.click(selector).await?;

            match cli.format {
                OutputFormat::Text => Ok(format!("Clicked: {selector}")),
                OutputFormat::Json => Ok(serde_json::to_string_pretty(
                    &serde_json::json!({ "action": "click", "selector": selector }),
                )
                .unwrap()),
            }
        }

        Command::Type {
            session,
            selector,
            text,
        } => {
            let session_info = khora_cdp::load_and_verify(session)?;
            let client = CdpClient::connect(&session_info).await?;
            client.type_text(selector, text).await?;

            match cli.format {
                OutputFormat::Text => Ok(format!("Typed \"{text}\" into {selector}")),
                OutputFormat::Json => Ok(serde_json::to_string_pretty(&serde_json::json!({
                    "action": "type",
                    "selector": selector,
                    "text": text,
                }))
                .unwrap()),
            }
        }

        Command::Drag {
            session,
            from,
            to,
            steps,
            delay,
        } => {
            let session_info = khora_cdp::load_and_verify(session)?;
            let client = CdpClient::connect(&session_info).await?;
            client
                .drag((from.x, from.y), (to.x, to.y), *steps, *delay)
                .await?;

            match cli.format {
                OutputFormat::Text => Ok(format!("Dragged: {from} -> {to} ({steps} steps)")),
                OutputFormat::Json => Ok(serde_json::to_string_pretty(&serde_json::json!({
                    "action": "drag",
                    "from": { "x": from.x, "y": from.y },
                    "to": { "x": to.x, "y": to.y },
                    "steps": steps,
                    "delay_ms": delay,
                }))
                .unwrap()),
            }
        }

        Command::Screenshot {
            session,
            output,
            selector,
        } => {
            let session_info = khora_cdp::load_and_verify(session)?;
            let client = CdpClient::connect(&session_info).await?;
            let png_bytes = client.screenshot(selector.as_deref()).await?;

            let path = PathBuf::from(output.as_deref().unwrap_or("khora-screenshot.png"));
            std::fs::write(&path, &png_bytes)?;

            match cli.format {
                OutputFormat::Text => Ok(format!(
                    "Screenshot saved: {} ({} bytes)",
                    path.display(),
                    png_bytes.len()
                )),
                OutputFormat::Json => Ok(serde_json::to_string_pretty(&serde_json::json!({
                    "path": path.display().to_string(),
                    "format": "png",
                    "size": png_bytes.len(),
                }))
                .unwrap()),
            }
        }

        Command::Text { session, selector } => {
            let session_info = khora_cdp::load_and_verify(session)?;
            let client = CdpClient::connect(&session_info).await?;
            let texts = client.get_text(selector).await?;
            Ok(khora_core::output::format_text(&texts, cli.format))
        }

        Command::Attribute {
            session,
            selector,
            attr,
        } => {
            let session_info = khora_cdp::load_and_verify(session)?;
            let client = CdpClient::connect(&session_info).await?;
            let value = client.get_attribute(selector, attr).await?;

            match cli.format {
                OutputFormat::Text => Ok(value.unwrap_or_else(|| "(null)".to_string())),
                OutputFormat::Json => Ok(serde_json::to_string_pretty(&serde_json::json!({
                    "selector": selector,
                    "attribute": attr,
                    "value": value,
                }))
                .unwrap()),
            }
        }

        Command::WaitFor {
            session,
            selector,
            timeout,
        } => {
            let session_info = khora_cdp::load_and_verify(session)?;
            let client = CdpClient::connect(&session_info).await?;
            let t = timeout.unwrap_or(cli.timeout);
            client.wait_for(selector, t).await?;

            match cli.format {
                OutputFormat::Text => Ok(format!("Found: {selector}")),
                OutputFormat::Json => Ok(serde_json::to_string_pretty(
                    &serde_json::json!({ "status": "found", "selector": selector }),
                )
                .unwrap()),
            }
        }

        Command::WaitGone {
            session,
            selector,
            timeout,
        } => {
            let session_info = khora_cdp::load_and_verify(session)?;
            let client = CdpClient::connect(&session_info).await?;
            let t = timeout.unwrap_or(cli.timeout);
            client.wait_gone(selector, t).await?;

            match cli.format {
                OutputFormat::Text => Ok(format!("Gone: {selector}")),
                OutputFormat::Json => Ok(serde_json::to_string_pretty(
                    &serde_json::json!({ "status": "gone", "selector": selector }),
                )
                .unwrap()),
            }
        }

        Command::Console { session } => {
            let session_info = khora_cdp::load_and_verify(session)?;
            let client = CdpClient::connect(&session_info).await?;
            let messages = client.console_messages().await?;
            Ok(khora_core::output::format_console(&messages, cli.format))
        }

        Command::Network { session } => {
            let session_info = khora_cdp::load_and_verify(session)?;
            let client = CdpClient::connect(&session_info).await?;
            let requests = client.network_requests().await?;
            Ok(khora_core::output::format_network(&requests, cli.format))
        }

        Command::Eval { session, js } => {
            let session_info = khora_cdp::load_and_verify(session)?;
            let client = CdpClient::connect(&session_info).await?;
            let result = client.eval(js).await?;

            match cli.format {
                OutputFormat::Text => Ok(match &result {
                    serde_json::Value::String(s) => s.clone(),
                    other => serde_json::to_string_pretty(other).unwrap_or_default(),
                }),
                OutputFormat::Json => Ok(serde_json::to_string_pretty(&result).unwrap_or_default()),
            }
        }

        Command::Kill { session, all } => {
            let sessions_to_kill: Vec<_> = if *all {
                SessionInfo::list_all()?
            } else {
                let id = session.as_deref().unwrap();
                vec![khora_cdp::load_and_verify(id)?]
            };

            let mut killed = Vec::new();
            for info in &sessions_to_kill {
                match CdpClient::connect(info).await {
                    Ok(client) => {
                        let _ = client.close().await;
                    }
                    Err(_) => {
                        // Chrome already dead — clean up data dir manually
                        if let Some(ref dir) = info.data_dir {
                            khora_cdp::cleanup_data_dir(dir);
                        }
                    }
                }
                let _ = SessionInfo::remove(&info.id);
                killed.push(info.id.clone());
            }

            match cli.format {
                OutputFormat::Text => {
                    if killed.len() == 1 {
                        Ok(format!("Killed session: {}", killed[0]))
                    } else {
                        Ok(format!(
                            "Killed {} sessions: {}",
                            killed.len(),
                            killed.join(", ")
                        ))
                    }
                }
                OutputFormat::Json => Ok(serde_json::to_string_pretty(
                    &serde_json::json!({ "killed": killed }),
                )
                .unwrap()),
            }
        }

        Command::Status { session } => {
            if let Some(id) = session {
                match khora_cdp::load_and_verify(id) {
                    Ok(info) => {
                        let alive = match CdpClient::connect(&info).await {
                            Ok(client) => client.is_alive(),
                            Err(_) => false,
                        };

                        match cli.format {
                            OutputFormat::Text => {
                                if alive {
                                    Ok(format!("Session {} is alive", info.id))
                                } else {
                                    Ok(format!("Session {} is dead", info.id))
                                }
                            }
                            OutputFormat::Json => {
                                Ok(serde_json::to_string_pretty(&serde_json::json!({
                                    "session": info.id,
                                    "alive": alive,
                                    "pid": info.pid,
                                    "headless": info.headless,
                                }))
                                .unwrap())
                            }
                        }
                    }
                    Err(e) => Err(e),
                }
            } else {
                let sessions = SessionInfo::list_all()?;
                Ok(khora_core::output::format_sessions(&sessions, cli.format))
            }
        }

        Command::Reap { all, older_than } => {
            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            let age_threshold = if let Some(dur_str) = older_than {
                let dur = parse_duration(dur_str)
                    .map_err(|e| KhoraError::Cdp(format!("invalid --older-than: {e}")))?;
                Some(now_secs.saturating_sub(dur.as_secs()))
            } else {
                None
            };

            let sessions = SessionInfo::list_all()?;
            let mut reaped = Vec::new();

            for info in &sessions {
                let is_dead = info.pid > 0 && !khora_cdp::is_process_alive(info.pid);
                let is_old = age_threshold.is_some_and(|threshold| info.created_at <= threshold);
                let should_reap = is_dead || *all || is_old;

                if !should_reap {
                    continue;
                }

                if !is_dead {
                    // Live session: attempt graceful close.
                    match CdpClient::connect(info).await {
                        Ok(client) => {
                            let _ = client.close().await;
                        }
                        Err(_) => {
                            if let Some(ref dir) = info.data_dir {
                                khora_cdp::cleanup_data_dir(dir);
                            }
                        }
                    }
                } else {
                    if let Some(ref dir) = info.data_dir {
                        khora_cdp::cleanup_data_dir(dir);
                    }
                }

                let _ = SessionInfo::remove(&info.id);
                reaped.push(info.id.clone());
            }

            match cli.format {
                OutputFormat::Text => Ok(format!(
                    "reaped {} sessions: {}",
                    reaped.len(),
                    reaped.join(", ")
                )),
                OutputFormat::Json => Ok(serde_json::to_string_pretty(
                    &serde_json::json!({ "reaped": reaped }),
                )
                .unwrap()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn parse_duration_seconds() {
        assert_eq!(
            parse_duration("0s").unwrap(),
            std::time::Duration::from_secs(0)
        );
        assert_eq!(
            parse_duration("30s").unwrap(),
            std::time::Duration::from_secs(30)
        );
    }

    #[test]
    fn parse_duration_minutes() {
        assert_eq!(
            parse_duration("30m").unwrap(),
            std::time::Duration::from_secs(1800)
        );
    }

    #[test]
    fn parse_duration_hours() {
        assert_eq!(
            parse_duration("2h").unwrap(),
            std::time::Duration::from_secs(7200)
        );
        assert_eq!(
            parse_duration("24h").unwrap(),
            std::time::Duration::from_secs(86400)
        );
    }

    #[test]
    fn parse_duration_rejects_bad_input() {
        assert!(parse_duration("").is_err());
        assert!(parse_duration("30").is_err());
        assert!(parse_duration("abcm").is_err());
        assert!(parse_duration("30d").is_err());
    }

    #[test]
    fn parse_window_size_valid() {
        assert_eq!(
            WindowSize::from_str("1920x1080").unwrap(),
            WindowSize {
                width: 1920,
                height: 1080
            }
        );
        assert_eq!(
            WindowSize::from_str("800x600").unwrap(),
            WindowSize {
                width: 800,
                height: 600
            }
        );
        assert_eq!(
            WindowSize::from_str("1x1").unwrap(),
            WindowSize {
                width: 1,
                height: 1
            }
        );
    }

    #[test]
    fn parse_window_size_rejects_missing_x() {
        assert!(WindowSize::from_str("1920").is_err());
    }

    #[test]
    fn parse_window_size_rejects_non_numeric() {
        assert!(WindowSize::from_str("abc").is_err());
        assert!(WindowSize::from_str("1920xabc").is_err());
        assert!(WindowSize::from_str("abcx1080").is_err());
    }

    #[test]
    fn parse_window_size_rejects_empty() {
        assert!(WindowSize::from_str("").is_err());
    }

    #[test]
    fn parse_window_size_rejects_zero() {
        assert!(WindowSize::from_str("0x0").is_err());
        assert!(WindowSize::from_str("1920x0").is_err());
        assert!(WindowSize::from_str("0x1080").is_err());
    }

    #[test]
    fn parse_window_size_rejects_wrong_separator() {
        assert!(WindowSize::from_str("1920X1080").is_err()); // capital X
        assert!(WindowSize::from_str("1920*1080").is_err());
        assert!(WindowSize::from_str("1920,1080").is_err());
    }

    #[test]
    fn parse_dpr_valid() {
        assert_eq!(parse_dpr("0").unwrap(), 0.0);
        assert_eq!(parse_dpr("1").unwrap(), 1.0);
        assert_eq!(parse_dpr("2.5").unwrap(), 2.5);
        assert_eq!(parse_dpr("3").unwrap(), 3.0);
    }

    #[test]
    fn parse_dpr_rejects_bad_input() {
        assert!(parse_dpr("").is_err());
        assert!(parse_dpr("abc").is_err());
        assert!(parse_dpr("-1").is_err());
        assert!(parse_dpr("inf").is_err());
        assert!(parse_dpr("NaN").is_err());
    }

    #[test]
    fn parse_point_valid() {
        assert_eq!(
            Point::from_str("100,250").unwrap(),
            Point { x: 100.0, y: 250.0 }
        );
        assert_eq!(
            Point::from_str("10.5, 20.25").unwrap(),
            Point { x: 10.5, y: 20.25 }
        );
        assert_eq!(
            Point::from_str("-5,10").unwrap(),
            Point { x: -5.0, y: 10.0 }
        );
        assert_eq!(Point::from_str("0,0").unwrap(), Point { x: 0.0, y: 0.0 });
    }

    #[test]
    fn parse_point_rejects_bad_input() {
        assert!(Point::from_str("").is_err());
        assert!(Point::from_str("100").is_err());
        assert!(Point::from_str("abc,10").is_err());
        assert!(Point::from_str("10,abc").is_err());
        assert!(Point::from_str("10x20").is_err());
        assert!(Point::from_str("inf,10").is_err());
        assert!(Point::from_str("NaN,10").is_err());
    }

    #[test]
    fn display_point_round_trip() {
        let p = Point { x: 100.0, y: 250.5 };
        assert_eq!(Point::from_str(&p.to_string()).unwrap(), p);
    }

    #[test]
    fn display_window_size_round_trip() {
        let w = WindowSize {
            width: 1920,
            height: 1080,
        };
        assert_eq!(w.to_string(), "1920x1080");
        assert_eq!(WindowSize::from_str(&w.to_string()).unwrap(), w);
    }
}
