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

From source (needs [Rust](https://rustup.rs)):

```bash
git clone https://github.com/simonspoon/khora && cd khora
./install.sh                     # -> ~/.cargo/bin/khora
./install.sh --prefix /usr/local # -> /usr/local/bin/khora
```

### Which build am I running?

```bash
khora --version
# khora 0.3.17              <- a release build, exactly on its tag
# khora 0.3.17+13.g6114093  <- built 13 commits past v0.3.17, at 6114093
```

A build made past the last release tag says so, so a version read never
understates what the binary can do. Use it to tell a source build apart from
the release it was cut from; `khora --help` and `khora <cmd> --help` remain the
authority on the exact command surface.

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

# Screenshot (whole scrollable page)
khora screenshot abc123 -o result.png

# Screenshot of just the visible viewport
khora screenshot abc123 -o fold.png --viewport

# Screenshot cropped to an element (errors if the selector matches nothing)
khora screenshot abc123 -o button.png --selector "#submit"

# Screenshot of an explicit page region, X,Y,WxH in CSS pixels
khora screenshot abc123 -o overlay.png --clip 0,0,1920x2500

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
| `set-viewport <session> <WxH> [dpr]` | Override the viewport — phone widths headless Chrome won't allow via `--window-size` (`--mobile` for mobile emulation, `--reset` to clear). Persists on the session and is re-applied before every later command, so it survives `navigate` |
| `emulate-media <session>` | Emulate CSS media features — `--color-scheme`, `--reduced-motion`, `--forced-colors`, `--media` (`screen`/`print`), `--reset`. Sticks to the session and fires real `matchMedia` change events |
| `find <session> <selector>` | Find elements by CSS selector |
| `click <session> <selector>` | Click an element |
| `type <session> <selector> <text>` | Type text into an element |
| `type-keys <session> <selector> <text>` | Type text with trusted per-character key events — for xterm.js-style widgets `type` can't reach, and for anything gated on focus/blur (`type` fires no focus events, so commit-on-blur never runs). Inserts at the caret; `--clear` wipes the field first |
| `blur <session> [selector]` | Blur the focused element (default `document.activeElement`), firing blur/focusout so commit-on-blur runs — without `key Tab`'s side effect of focusing the next element. Errors if the target isn't focused |
| `drag <session> <from> <to>` | Drag with trusted mouse events between `X,Y` points (`--steps`, `--delay` ms) |
| `mouse-down <session> <at>` | Press the left mouse button at `X,Y` without releasing it |
| `mouse-move <session> <at>` | Move the mouse to `X,Y`, carrying over button state |
| `mouse-up <session> <at>` | Release the left mouse button at `X,Y` |
| `click-at <session> <at>` | Click whatever is at raw viewport point `X,Y` with a trusted mouse event |
| `dblclick-at <session> <at>` | Double-click at raw viewport point `X,Y` with trusted mouse events |
| `key <session> <combo>` | Press a trusted key combo, e.g. `Cmd+D`, `Ctrl+Shift+I`, `Escape` |
| `wheel <session> <at> <delta>` | Scroll with a trusted native wheel event at `X,Y` by `dX,dY` CSS pixels |
| `touch <session> <at>` | Tap at raw viewport point `X,Y` with trusted touch events — real `TouchEvent`s and `pointerType: touch`, which `click-at` never produces |
| `swipe <session> <from> <to>` | Swipe between `X,Y` points with trusted touch events (`--steps`, `--delay` ms). The only way to exercise `touch-action`, which governs touch panning exclusively — `wheel` can't reach it |
| `pinch <session> <at> --from <gap> --to <gap>` | Pinch two fingers around `X,Y`, spreading or squeezing the gap between them in CSS pixels (`--steps`, `--delay` ms). The only two-contact gesture — `swipe` dispatches a single one, so pinch-zoom is out of its reach |
| `long-press <session> <at>` | Hold a touch at `X,Y` then release (`--duration` ms, default 600) — for context menus and drag-activation delays |
| `emulate-touch <session>` | Report a touch device (`navigator.maxTouchPoints`, `pointer: coarse`, `ontouchstart`) so load-time feature detection registers touch handlers — run it **before** `navigate`. `--max-points N`, `--off`. Sticks to the session |
| `emulate-keyboard <session> <px>` | Emulate an on-screen keyboard `px` tall: shrinks `visualViewport.height` **only**, leaving the layout viewport (and so `dvh`/`vh`/`fixed`) alone, and fires `resize` on the page's own `VisualViewport`. `--off`. Sticks to the session |
| `screenshot <session>` | Capture screenshot — whole page by default, `--viewport` for the visible area, `--selector` to crop to an element, `--clip X,Y,WxH` for an explicit region |
| `text <session> <selector>` | Get text content of matching elements |
| `attribute <session> <selector> <attr>` | Get attribute value |
| `wait-for <session> <selector>` | Wait for element to appear |
| `wait-gone <session> <selector>` | Wait for element to disappear |
| `targets <session>` (alias `pages`) | List the browser's open targets — type, url, title, which tab is `active`, and which one khora drives |
| `console <session>` | Read console messages |
| `eval <session> <js>` | Execute JavaScript, return result |
| `kill <session>` | Close browser and clean up (`--all` to kill every session) |
| `status [session] [--check]` | Check session status, probe whether the browser still responds, and report its CDP WebSocket URL (or list all) |
| `network <session>` | List captured network requests (`fetch` and `XMLHttpRequest`) |
| `reap` | Remove sessions whose Chrome died (`--all` to also kill live sessions, `--older-than <dur>` for age-based cleanup, e.g. `30m`, `2h`, `24h`, `--purge-data-dirs` to also delete orphaned Chrome profiles) |

## Output

All commands support `--format json` for structured output. Use `KHORA_FORMAT=json` to default to JSON.

Default timeout is 5000ms, override with `--timeout` or `KHORA_TIMEOUT`.

## Session management

Sessions persist across CLI invocations. `launch` starts Chrome and saves a session file to `~/.khora/sessions/`. Subsequent commands reconnect using the session ID. `kill` closes the browser and removes the session file.

Multiple concurrent sessions are supported — each gets a unique ID and a private Chrome user data directory. That directory is deleted when the session ends, on every path: `kill`, `reap`, and auto-reap alike.

Sessions whose Chrome process has died are auto-reaped on every CLI invocation. Use `khora reap` to clean up manually, `khora reap --all` to also close live sessions, or `khora reap --older-than 2h` to clear anything older than a given age.

Chrome profiles orphaned by earlier khora versions (which leaked them on the graceful `kill`/`reap` path, up to and including 0.3.17) can be swept with `khora reap --purge-data-dirs`. It deletes `khora-chrome-*` directories in the system temp dir that no current session owns, and reports how much disk it reclaimed.

## Requirements

- Chrome or Chromium installed (auto-detected per platform)
- Or set `CHROME_PATH` to a custom Chrome binary

## License

MIT
