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
- `client.rs` — `CdpClient` wrapping chromiumoxide `Browser`; per-launch user data dir + cleanup
- `session.rs` — Process liveness check (`is_process_alive`), `load_and_verify`, `reap_stale_sessions` (called automatically at CLI startup)

### khora-cli

CLI entry point:

- `main.rs` — clap Parser/Subcommand, dispatches to CdpClient methods

## Session lifecycle

1. `launch` — starts Chrome via chromiumoxide with a fresh per-session user data directory (tempdir), saves session JSON to `~/.khora/sessions/<id>.json`. The Chrome PID is recorded so dead sessions can be detected.
2. Auto-reap — every non-`reap` CLI invocation calls `reap_stale_sessions()` first, which removes session files whose recorded PID is no longer running and deletes their data dirs. Best-effort; never fails the caller.
3. Subsequent commands — `load_and_verify()` loads the session file and re-checks the PID; if dead, the session file is removed and `KhoraError::SessionDead` is returned. Otherwise reconnects via `Browser::connect()` and calls `fetch_targets()` to discover existing tabs.
4. `kill` / `reap` — closes the browser (if alive), removes the user data directory, and deletes the session file.

Session files contain: ID, WebSocket URL, PID, headless flag, created-at timestamp, and an optional `data_dir` path. `data_dir` is omitted from older session files (`#[serde(default)]`) so they continue to load.

## CDP integration

Uses [chromiumoxide](https://crates.io/crates/chromiumoxide) (v0.9) for Chrome DevTools Protocol communication:

- **Launch**: `Browser::launch()` with platform-specific Chrome path
- **Reconnect**: `Browser::connect()` + `fetch_targets()` to discover existing pages
- **Navigate**: `page.goto(url)` (CDP `Page.navigate`) with a 10 s timeout; falls back to JS `window.location.href` + readyState polling when lifecycle events don't fire (common on reconnected sessions). `--no-cache` sends `Network.setCacheDisabled` first.
- **Element operations**: JavaScript evaluation via `page.evaluate()` for rich element info (`find`, `click`, `type`, `text`, `attribute`). `type` sets the value through the native React-tracked property setter so framework `onChange` handlers fire, then dispatches a synthetic `input` event.
- **Trusted input events**: `drag`, `mouse-down`, `mouse-move`, `mouse-up`, `click-at`, `dblclick-at`, and `key` bypass JS evaluation entirely and dispatch native, OS-trusted CDP input events (`Input.dispatchMouseEvent`, `Input.dispatchKeyEvent`) directly at viewport coordinates or as raw key codes. This is required for handlers gated on `event.isTrusted` or `setPointerCapture`, which synthetic JS/DOM events cannot satisfy. `mouse-down`/`mouse-move`/`mouse-up` are separate CLI invocations that share no in-process state — persistence across the sequence works because Chrome's own input state (e.g. pointer capture) lives in the browser, not in khora.
- **Viewport**: `set-viewport` uses `Emulation.setDeviceMetricsOverride` to set arbitrary widths/heights (including phone widths headless Chrome rejects via `--window-size`), with `--mobile` toggling mobile emulation.
- **Screenshot**: `Page.captureScreenshot`; `--selector` first resolves the element's bounding box via `page.evaluate()`, then crops to it.
- **Page selection**: Prefers non-blank pages by CDP target URL, then falls back to JS `location.href` evaluation (handles stale target URLs after JS-based navigation)

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
