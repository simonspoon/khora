use clap::{Parser, Subcommand};
use khora_cdp::CdpClient;
use khora_core::error::KhoraError;
use khora_core::session::SessionInfo;
use khora_core::OutputFormat;
use std::path::PathBuf;
use std::process::ExitCode;

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
    },

    /// Navigate to a URL
    Navigate {
        /// Session ID
        session: String,
        /// URL to navigate to
        url: String,
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

    /// Capture a screenshot
    Screenshot {
        /// Session ID
        session: String,
        /// Output file path (default: khora-screenshot.png)
        #[arg(long, short)]
        output: Option<String>,
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

    /// List network requests (not yet implemented)
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
        /// Session ID
        session: String,
    },

    /// Check if a session is still alive
    Status {
        /// Session ID (omit to list all sessions)
        session: Option<String>,
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

async fn run(cli: &Cli) -> Result<String, KhoraError> {
    match &cli.command {
        Command::Launch { visible } => {
            let (client, session) = CdpClient::launch(!visible).await?;
            session.save()?;

            // Install console hook for this session
            if let Err(e) = client.install_console_hook().await {
                tracing::warn!("failed to install console hook: {e}");
            }

            // Keep the browser alive by leaking the client
            // (the handler task keeps Chrome running)
            std::mem::forget(client);

            Ok(khora_core::output::format_session(&session, cli.format))
        }

        Command::Navigate { session, url } => {
            let session_info = khora_cdp::load_and_verify(session)?;
            let client = CdpClient::connect(&session_info).await?;
            client.navigate(url).await?;

            match cli.format {
                OutputFormat::Text => Ok(format!("Navigated to: {url}")),
                OutputFormat::Json => Ok(serde_json::to_string_pretty(
                    &serde_json::json!({ "action": "navigate", "url": url }),
                )
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

        Command::Screenshot { session, output } => {
            let session_info = khora_cdp::load_and_verify(session)?;
            let client = CdpClient::connect(&session_info).await?;
            let png_bytes = client.screenshot().await?;

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

        Command::Network { session: _ } => match cli.format {
            OutputFormat::Text => Ok("Network request tracking not yet implemented.".to_string()),
            OutputFormat::Json => Ok(serde_json::to_string_pretty(
                &serde_json::json!({ "error": "not implemented" }),
            )
            .unwrap()),
        },

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

        Command::Kill { session } => {
            let session_info = khora_cdp::load_and_verify(session)?;
            let client = CdpClient::connect(&session_info).await?;
            client.close().await?;
            SessionInfo::remove(&session_info.id)?;

            match cli.format {
                OutputFormat::Text => Ok(format!("Killed session: {}", session_info.id)),
                OutputFormat::Json => Ok(serde_json::to_string_pretty(
                    &serde_json::json!({ "killed": session_info.id }),
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
    }
}
