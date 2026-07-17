<p align="center">
  <img src="icon.png" width="128" height="128" alt="khora">
</p>

# khora

Web app QA automation for agents. Navigate pages, find elements, click buttons, type text, take screenshots — all from the command line via Chrome DevTools Protocol.

Cross-platform. Built for CI/CD pipelines and agent workflows where you need to verify a web app actually works, not just that it builds.

## Install

```
brew install simonspoon/tap/khora
```

Or download from [Releases](https://github.com/simonspoon/khora/releases).

## Quick start

```bash
# Launch Chrome (headless by default)
khora launch
# Session: abc123

# Navigate to a page
khora navigate abc123 https://example.com

# Find elements
khora find abc123 "h1"
khora find abc123 "button.submit"

# Interact
khora click abc123 "button.submit"
khora type abc123 "input[name=email]" "user@example.com"

# Verify content
khora text abc123 "h1"
khora attribute abc123 "a" "href"
khora wait-for abc123 ".success-message"

# Screenshot (full page)
khora screenshot abc123 -o result.png

# Screenshot cropped to an element (errors if the selector matches nothing)
khora screenshot abc123 -o button.png --selector "#submit"

# Execute JavaScript
khora eval abc123 "document.title"

# Clean up
khora kill abc123
```

Use `--visible` to see the browser window:

```bash
khora launch --visible
```

Set a custom window size with `--window-size` (default `1920x1080`):

```bash
khora launch --window-size 1366x768
```

## Commands

| Command | Description |
|---------|-------------|
| `launch` | Start Chrome (headless by default, `--visible` for headed, `--window-size WxH`) |
| `navigate <session> <url>` | Go to a URL (`--no-cache` to bypass the browser cache) |
| `set-viewport <session> <WxH> [dpr]` | Override the viewport — phone widths headless Chrome won't allow via `--window-size` (`--mobile` for mobile emulation) |
| `find <session> <selector>` | Find elements by CSS selector |
| `click <session> <selector>` | Click an element |
| `type <session> <selector> <text>` | Type text into an element |
| `type-keys <session> <selector> <text>` | Type text with trusted per-character key events — for canvas/xterm.js-style widgets `type` can't reach |
| `drag <session> <from> <to>` | Drag with trusted mouse events between `X,Y` points (`--steps`, `--delay` ms) |
| `mouse-down <session> <at>` | Press the left mouse button at `X,Y` without releasing it |
| `mouse-move <session> <at>` | Move the mouse to `X,Y`, carrying over button state |
| `mouse-up <session> <at>` | Release the left mouse button at `X,Y` |
| `click-at <session> <at>` | Click whatever is at raw viewport point `X,Y` with a trusted mouse event |
| `dblclick-at <session> <at>` | Double-click at raw viewport point `X,Y` with trusted mouse events |
| `key <session> <combo>` | Press a trusted key combo, e.g. `Cmd+D`, `Ctrl+Shift+I`, `Escape` |
| `wheel <session> <at> <delta>` | Scroll with a trusted native wheel event at `X,Y` by `dX,dY` CSS pixels |
| `screenshot <session>` | Capture screenshot (full page, or `--selector` to crop to an element) |
| `text <session> <selector>` | Get text content of matching elements |
| `attribute <session> <selector> <attr>` | Get attribute value |
| `wait-for <session> <selector>` | Wait for element to appear |
| `wait-gone <session> <selector>` | Wait for element to disappear |
| `console <session>` | Read console messages |
| `eval <session> <js>` | Execute JavaScript, return result |
| `kill <session>` | Close browser and clean up (`--all` to kill every session) |
| `status [session]` | Check session status (or list all) |
| `network <session>` | List captured network requests (`fetch` and `XMLHttpRequest`) |
| `reap` | Remove sessions whose Chrome died (`--all` to also kill live sessions, `--older-than <dur>` for age-based cleanup, e.g. `30m`, `2h`, `24h`) |

## Output

All commands support `--format json` for structured output. Use `KHORA_FORMAT=json` to default to JSON.

Default timeout is 5000ms, override with `--timeout` or `KHORA_TIMEOUT`.

## Session management

Sessions persist across CLI invocations. `launch` starts Chrome and saves a session file to `~/.khora/sessions/`. Subsequent commands reconnect using the session ID. `kill` closes the browser and removes the session file.

Multiple concurrent sessions are supported — each gets a unique ID and a private Chrome user data directory.

Sessions whose Chrome process has died are auto-reaped on every CLI invocation. Use `khora reap` to clean up manually, `khora reap --all` to also close live sessions, or `khora reap --older-than 2h` to clear anything older than a given age.

## Requirements

- Chrome or Chromium installed (auto-detected per platform)
- Or set `CHROME_PATH` to a custom Chrome binary

## License

MIT
