# Architecture

## Crate structure

Khora is a 3-crate Rust workspace:

```
khora/
├── crates/
│   ├── khora-core/     # Types, config, errors, output formatting
│   ├── khora-cdp/      # Chrome discovery, CDP client, session lifecycle
│   └── khora-cli/      # clap commands, JSON output, entry point
```

### khora-core

Platform-agnostic types shared by all crates:

- `error.rs` — `KhoraError` enum with exit codes (thiserror)
- `config.rs` — `KhoraConfig` loaded from `~/.khora/config.json`
- `element.rs` — `ElementInfo`, `BoundingBox`, `NetworkRequest`, `ConsoleMessage`
- `session.rs` — `SessionInfo` with save/load/remove/list operations
- `output.rs` — `OutputFormat` enum (Text/Json), formatting functions

### khora-cdp

Chrome DevTools Protocol integration:

- `chrome.rs` — Chrome binary discovery per platform (macOS, Linux, Windows)
- `client.rs` — `CdpClient` wrapping chromiumoxide `Browser`
- `session.rs` — Process liveness check, session verification

### khora-cli

CLI entry point:

- `main.rs` — clap Parser/Subcommand, dispatches to CdpClient methods

## Session lifecycle

1. `launch` — starts Chrome via chromiumoxide, saves session JSON to `~/.khora/sessions/<id>.json`
2. Subsequent commands — loads session file, reconnects via `Browser::connect()`, calls `fetch_targets()` to discover existing tabs
3. `kill` — closes browser, removes session file

Session files contain: ID, WebSocket URL, PID, headless flag, timestamp.

## CDP integration

Uses [chromiumoxide](https://crates.io/crates/chromiumoxide) (v0.9) for Chrome DevTools Protocol communication:

- **Launch**: `Browser::launch()` with platform-specific Chrome path
- **Reconnect**: `Browser::connect()` + `fetch_targets()` to discover existing pages
- **Navigate**: `browser.new_page(url)` creates a tab at the target URL
- **Element operations**: JavaScript evaluation via `page.evaluate()` for rich element info
- **Page selection**: Prefers non-blank pages when multiple tabs exist

## Error handling

All errors flow through `KhoraError` with specific exit codes:
- 1: General error
- 2: Chrome not found
- 3: Timeout
- 4: Session not found or dead

## Output format

Two modes controlled by `--format` or `KHORA_FORMAT`:
- `text` — human-readable (default)
- `json` — structured JSON for agent consumption
