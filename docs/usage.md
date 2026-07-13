# Usage Guide

## Installation

### Homebrew

```bash
brew install simonspoon/tap/khora
```

### From source

```bash
git clone https://github.com/simonspoon/khora.git
cd khora
cargo install --path crates/khora-cli
```

## Chrome setup

Khora auto-detects Chrome/Chromium:

| Platform | Search locations |
|----------|-----------------|
| macOS | `/Applications/Google Chrome.app`, Chromium, Brave, Edge |
| Linux | `/usr/bin/google-chrome`, `/usr/bin/chromium`, snap |
| Windows | Program Files (Chrome, Edge, Brave) |

Override with `CHROME_PATH=/path/to/chrome`.

## Session workflow

Every interaction starts with `launch` and ends with `kill`:

```bash
SESSION=$(khora --format json launch | jq -r .id)
khora navigate "$SESSION" "https://your-app.com"
# ... interact and verify ...
khora kill "$SESSION"
```

### Headed mode

```bash
khora launch --visible
```

### Window size

The launched Chrome window defaults to `1920x1080`. Override with `--window-size`:

```bash
khora launch --window-size 1366x768
khora launch --visible --window-size 800x600
```

Dimensions must be `WxH` with positive integers.

### Phone-width viewports

Headless Chrome clamps the window to a ~500px minimum inner width, so
`launch --window-size 390x844` can't produce a phone-width page. Use
`set-viewport` instead — it overrides the page's viewport metrics directly
(CDP `Emulation.setDeviceMetricsOverride`):

```bash
khora set-viewport "$S" 390x844              # iPhone 14 Pro sized viewport
khora set-viewport "$S" 390x844 3 --mobile   # + device pixel ratio 3, mobile emulation
khora eval "$S" "window.innerWidth"          # → 390
```

The size override sticks for the life of the Chrome session, across later
khora commands and navigations. The `dpr` and `--mobile` parts of the
override are applied but can reset once the invocation disconnects —
rely on the width/height, not on `devicePixelRatio`, in later commands.

### Multiple sessions

```bash
SESSION1=$(khora --format json launch | jq -r .id)
SESSION2=$(khora --format json launch | jq -r .id)
khora navigate "$SESSION1" "https://app1.com"
khora navigate "$SESSION2" "https://app2.com"
khora status "$SESSION1"   # check a single session (alive/dead)
khora status               # lists all sessions
khora kill --all           # kill every active session at once
```

Each session uses its own Chrome user data directory (a unique temp dir), so concurrent sessions can't corrupt each other's profile.

### Reaping stale sessions

Sessions whose Chrome process exited (crash, manual kill, `pkill chrome`) leave a stale session file behind. Khora auto-reaps these on every CLI invocation, but you can also clean up explicitly:

```bash
# Remove all sessions whose Chrome process is dead
khora reap

# Remove all sessions older than a duration (s/m/h)
khora reap --older-than 2h
khora reap --older-than 0s   # remove every session

# Also close live sessions (same as kill --all, but also reaps dead ones)
khora reap --all
```

`--older-than` accepts suffixes `s`, `m`, `h` (e.g. `30s`, `15m`, `24h`).

## Common patterns

### Verify page content

```bash
khora navigate "$S" "https://app.com/dashboard"
khora text "$S" "h1"  # Check heading
khora wait-for "$S" ".data-loaded"  # Wait for dynamic content
khora text "$S" ".user-count"  # Read a value
```

### Fresh assets after a rebuild

Chrome's cache can keep serving an old `index.html` (and its old hashed
bundles) after you rebuild your app. Pass `--no-cache` to bypass the browser
cache for that navigation (CDP `Network.setCacheDisabled`) instead of
resorting to `?bust=$RANDOM` query params:

```bash
khora navigate "$S" "http://localhost:5173" --no-cache
```

### Form interaction

```bash
khora navigate "$S" "https://app.com/login"
khora type "$S" "input[name=email]" "test@example.com"
khora type "$S" "input[name=password]" "password123"
khora click "$S" "button[type=submit]"
khora wait-for "$S" ".dashboard"
```

### Drag interactions

`drag` dispatches trusted native mouse events (CDP `Input.dispatchMouseEvent`):
press at the start point, evenly spaced moves along the line, release at the
end point. Use it for interactions that JS-synthesized events can't drive —
crop marquees, sliders, drag handles — because pages can check `isTrusted`.
Coordinates are viewport CSS pixels; get them from `find`'s bounding box or
`eval` with `getBoundingClientRect()`.

```bash
khora find "$S" ".crop-area"            # read the bounding box
khora drag "$S" 100,150 300,400          # marquee-select from 100,150 to 300,400
khora drag "$S" 50,200 250,200 --steps 25 --delay 30   # slower, finer drag
```

`--steps` (default 10) sets how many intermediate move events fire; `--delay`
(default 16 ms, one frame) is the pause between events so frameworks that
batch on animation frames see the motion.

For inspecting mid-gesture state — e.g. screenshotting a marquee halfway
through a drag — `drag` isn't enough: it runs the whole press-move-release
sequence in one call, so there's no way to pause it without backgrounding the
process and racing a separate `screenshot` against it. `mouse-down`,
`mouse-move`, and `mouse-up` expose the same trusted CDP mouse events as
individual steps so a script can interleave a `screenshot` between them:

```bash
khora mouse-down "$S" 100,150
khora mouse-move "$S" 200,275
khora screenshot "$S" -o mid-drag.png    # inspect state with the button still down
khora mouse-move "$S" 300,400
khora mouse-up "$S" 300,400
```

### Point clicks

`click` resolves a CSS selector; `click-at`/`dblclick-at` instead hit
whatever is actually at a raw viewport point, using the same trusted mouse
events as `drag`. Use them for pixel-precise hit-target verification a
selector can't express — overlapping elements, canvas-drawn UI, confirming
the topmost element at a point is the one that receives the click.

```bash
khora click-at "$S" 150,220
khora dblclick-at "$S" 150,220
```

`dblclick-at` fires two press/release pairs at the point, the second pair
carrying `clickCount: 2` — the signal Chromium needs to dispatch `dblclick`
instead of two independent `click` events.

### Key shortcuts

`key` dispatches a trusted native key event to the page (CDP
`Input.dispatchKeyEvent`: rawKeyDown then keyUp, modifier bits set on both).
Use it for a page's own modifier-key shortcuts — `e.metaKey`/`e.ctrlKey`
listeners a web app defines itself — that page-level `KeyboardEvent` dispatch
can't simulate as trusted. This targets the page's event handlers, not
Chrome's own accelerators (bookmark dialog, devtools) — those live in the
browser process, outside what `Input.dispatchKeyEvent` reaches.

The combo is `+`-separated; the last segment is the key (a single
letter/digit, or a named key: `Enter`, `Escape`, `Tab`, `Backspace`,
`Delete`, `Space`, `ArrowUp`/`Down`/`Left`/`Right`, `Home`, `End`,
`PageUp`/`PageDown`), everything before it a modifier (`Cmd`/`Meta`/`Command`,
`Ctrl`/`Control`, `Alt`/`Option`, `Shift`).

```bash
khora key "$S" "Cmd+S"           # app's own save handler (page calls preventDefault)
khora key "$S" "Cmd+Shift+D"     # app-defined shortcut
khora key "$S" "Escape"          # no modifier — just the key, e.g. dismiss a modal
```

### Screenshot comparison

```bash
khora navigate "$S" "https://app.com"
khora screenshot "$S" -o before.png
# ... make changes ...
khora screenshot "$S" -o after.png
```

Pass `--selector` to crop the shot to a single element's bounding box instead of
the full page. The element is scrolled into view first. If the selector matches
nothing (or the element has no visible area) the command errors with exit code 1
rather than silently falling back to a full-page shot.

```bash
khora screenshot "$S" -o submit-button.png --selector "#submit"
```

### JavaScript evaluation

```bash
# Get computed values
khora eval "$S" "window.innerWidth"
khora eval "$S" "document.querySelectorAll('li').length"

# Check application state
khora eval "$S" "JSON.stringify(window.__APP_STATE__)"
```

### Element inspection

```bash
# Find all matching elements (JSON for parsing)
khora --format json find "$S" "button"

# Get specific attributes
khora attribute "$S" "img.logo" "src"
khora attribute "$S" "a.nav-link" "href"
```

### Network request tracking

```bash
# After navigating and interacting, check what requests were made
khora network "$S"
# METHOD STATUS TYPE         URL
# GET    200    fetch        https://api.example.com/users
# POST   201    xhr          https://api.example.com/submit
```

Captures `fetch()` and `XMLHttpRequest` calls made by page JavaScript. Useful for verifying that API calls happened and returned expected status codes.

**What it captures:** programmatic requests (`fetch`, `XMLHttpRequest`) with method, status code, and URL.

**What it doesn't capture:** browser-initiated resource loads (images, stylesheets, scripts, fonts). For those, use `eval` with the Performance API:

```bash
khora eval "$S" "JSON.stringify(performance.getEntriesByType('resource').map(e => ({url: e.name, type: e.initiatorType})))"
```

### Console messages

```bash
khora console "$S"
# [error] Uncaught TypeError: cannot read property 'foo' of undefined
# [log] user clicked submit
```

Captures `console.log/warn/error/info` calls via a page-installed shim, buffered from page load. Useful for catching JS errors a UI test wouldn't otherwise surface.

### Wait patterns

```bash
# Wait for element to appear (default 5s timeout)
khora wait-for "$S" ".success-banner"

# Custom timeout
khora wait-for "$S" ".slow-content" --timeout 15000

# Wait for element to disappear
khora wait-gone "$S" ".loading-spinner"
```

## JSON output

All commands support `--format json`. Set `KHORA_FORMAT=json` for agent workflows:

```bash
export KHORA_FORMAT=json
khora find "$S" "button"
# Returns structured JSON with tag_name, text, bounding_box, etc.
```

## Timeouts

Default: 5000ms. Override per-command with `--timeout` or globally with `KHORA_TIMEOUT`:

```bash
khora --timeout 10000 wait-for "$S" ".slow-element"
# or
export KHORA_TIMEOUT=10000
```

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error |
| 2 | Chrome not found |
| 3 | Timeout |
| 4 | Session not found or dead |
