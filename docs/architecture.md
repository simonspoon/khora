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
- `config.rs` — `KhoraConfig`, currently used only for `sessions_dir()`. Its `~/.khora/config.json` loader is not wired into any command: timeouts come solely from `--timeout`/`KHORA_TIMEOUT` and format from `--format`/`KHORA_FORMAT`.
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
3. Subsequent commands — `load_and_verify()` loads the session file and re-checks the PID; if dead, the session file is removed and `KhoraError::SessionDead` is returned. Otherwise reconnects via `Browser::connect()` and calls `fetch_targets()` to discover existing tabs, both bounded by `--timeout`/`KHORA_TIMEOUT` so a wedged Chrome fails fast with `KhoraError::Timeout` instead of hanging.
4. `kill` / `reap` — closes the browser (if alive), removes the user data directory, and deletes the session file. If `connect()` times out (wedged but not confirmed dead), the PID is signaled directly (SIGTERM/SIGKILL) rather than treated as already-gone, since a `SessionDead` websocket refusal and a timed-out handshake mean different things — the resulting `Timeout` and `SessionDead` errors are handled by distinct match arms.

Session files contain: ID, WebSocket URL, PID, headless flag, created-at timestamp, and an optional `data_dir` path. `data_dir` is omitted from older session files (`#[serde(default)]`) so they continue to load.

## CDP integration

Uses [chromiumoxide](https://crates.io/crates/chromiumoxide) (v0.9) for Chrome DevTools Protocol communication:

- **Launch**: `Browser::launch()` with platform-specific Chrome path
- **Reconnect**: `Browser::connect()` + `fetch_targets()` to discover existing pages, both wrapped in `tokio::time::timeout` so a wedged Chrome can't hang a command indefinitely
- **Navigate**: `page.goto(url)` (CDP `Page.navigate`) with a 10 s timeout; falls back to JS `window.location.href` + readyState polling when lifecycle events don't fire (common on reconnected sessions). `--no-cache` sends `Network.setCacheDisabled` first.
- **Element operations**: JavaScript evaluation via `page.evaluate()` for rich element info (`find`, `click`, `type`, `text`, `attribute`, `blur`, `wait-for`, `wait-gone`). `type` sets the value through the native React-tracked property setter so framework `onChange` handlers fire, then dispatches a synthetic `input` event. `blur` calls the element's native `blur()` so commit-on-blur handlers run without needing a trusted event.
- **Trusted input events**: `drag`, `mouse-down`, `mouse-move`, `mouse-up`, `click-at`, `dblclick-at`, `wheel`, `key`, and `type-keys` bypass JS evaluation entirely and dispatch native, OS-trusted CDP input events (`Input.dispatchMouseEvent`, `Input.dispatchKeyEvent`) directly at viewport coordinates or as raw key codes. This is required for handlers gated on `event.isTrusted` or `setPointerCapture`, which synthetic JS/DOM events cannot satisfy — `wheel` specifically must be trusted because a synthetic `WheelEvent` never reaches Chromium's scroll pipeline at all, so it fires listeners without driving any actual scrolling or scroll chaining. `mouse-down`/`mouse-move`/`mouse-up` are separate CLI invocations that share no in-process state — persistence across the sequence works because Chrome's own input state (e.g. pointer capture) lives in the browser, not in khora.
- **Compositor-frame nudge**: a page with no pending compositor frame — after a trusted key event consumes it, or on a never-painted page like a fresh `about:blank` tab — makes the next `mouseMoved` pay a ~5s hit-test tax and the next `mouseWheel` hang for chromiumoxide's full 30s internal timeout. `key_press` forces a cheap 1x1 screenshot after dispatch to fix the desync it causes; `drag`/`mouse-move` force one before their first move to pre-warm a possibly-cold compositor, since that tax otherwise races khora's default 5000ms timeout.
- **Console & network capture**: rather than subscribing to CDP events (which would need a long-lived listener khora doesn't have between invocations), `launch` injects JS shims that patch `console.log/warn/error/info` and `fetch`/`XMLHttpRequest`, buffering entries into `window.__khora_console` / `window.__khora_network`. The `console` and `network` commands read those buffers back out. Navigation replaces the page context and wipes them, so `navigate` reinstalls both hooks after load. Both installs are best-effort — a failure warns rather than failing the command.
- **Viewport**: `set-viewport` uses `Emulation.setDeviceMetricsOverride` to set arbitrary widths/heights (including phone widths headless Chrome rejects via `--window-size`), with `--mobile` toggling mobile emulation.
- **Screenshot**: `Page.captureScreenshot` with `captureBeyondViewport`, always clipped explicitly — to the element's bounding box for `--selector` (resolved via `page.evaluate()`), to the user-supplied rectangle for `--clip X,Y,WxH`, or to `Page.getLayoutMetrics`' content size for a whole-page shot. `--clip` is the escape hatch for content the content size does not cover — a tall `position: fixed` overlay on a document pinned to `height: 100vh` contributes nothing to it, so a whole-page shot cuts the overlay off. Deliberately not chromiumoxide's `ScreenshotParams::full_page`, which captures whole pages by overriding device metrics to the content height and clearing the override afterwards: that reflows the page (`fixed`/`sticky`/`vh` layout renders at sizes the user never sees) and the trailing clear wipes any override `set-viewport` installed. Clipping touches no emulation state, so screenshots and `set-viewport` stay independent.
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
