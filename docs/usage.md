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

### Media features (dark mode, reduced motion, print)

`emulate-media` overrides what CSS media queries report (CDP
`Emulation.setEmulatedMedia`), so a system-following theme can be exercised
without touching the OS setting:

```bash
khora emulate-media "$S" --color-scheme dark
khora eval "$S" 'matchMedia("(prefers-color-scheme: dark)").matches'   # → true
khora emulate-media "$S" --color-scheme light                          # flip it back
khora emulate-media "$S" --reset                                       # Chrome's own defaults
```

| Flag | Values |
|------|--------|
| `--color-scheme` | `light`, `dark`, `no-preference` |
| `--reduced-motion` | `reduce`, `no-preference` |
| `--forced-colors` | `active`, `none` |
| `--media` | `screen`, `print` |
| `--reset` | clears every override above |

Flags accumulate across calls, so setting one feature never silently drops
another. `--reset` is the only way to clear them, and can't be combined with
the others:

```bash
khora emulate-media "$S" --color-scheme dark
khora emulate-media "$S" --media print        # still dark, now also print
```

**Why this fires real change events.** A page's own
`matchMedia('(prefers-color-scheme: dark)')` subscription can't be driven from
`eval`: `matchMedia()` returns a *fresh* `MediaQueryList` on every call, so a
`change` event dispatched from page JS never reaches the object the app
registered its listener on. Emulating the feature changes the real media state,
so the app's own listener fires and re-renders — which is what makes a
theme-switching UI testable end to end:

```bash
khora emulate-media "$S" --color-scheme dark
# the app's change handler has re-rendered by the time the next command lands
khora eval "$S" 'document.body.dataset.theme'    # → dark
khora screenshot "$S" -o dark.png
```

Assert the *result* of the flip (a class, a computed style, a screenshot)
rather than the event itself — delivery is a task on the page's event loop, so
an assertion in the same breath as the flip can outrun the re-render.

Unlike `set-viewport`, media emulation is **not** something Chrome keeps on its
own: it is reverted the moment the CDP client that set it disconnects, which
khora does at the end of every command. khora therefore records the state on
the session and re-applies it on each connect. The practical consequence is
that between two khora commands the page is momentarily back on Chrome's
defaults, so a page that *logs* or *counts* scheme changes will see extra
transitions; the state observed by any khora command is always the one you set.

`@media print` rules are covered by the same command — `--media print` applies
the print cascade to the live page, so `getComputedStyle` reports printed
values without exporting a PDF.

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

### When `type` isn't enough — `type-keys`

`type` sets the target element's `.value` via its native setter and fires
`input`/`change`. That covers ordinary form fields, including
React-controlled inputs (the native setter defeats React's value tracker, so
`onChange` does fire). It does **not** reproduce a real keystroke, and two
classes of target need one:

**1. Widgets that never read `.value`.** Canvas/WebGL-rendered widgets that
manage their own key handling off a hidden textarea (xterm.js and similar
terminal emulators) don't look at `.value` at all: `type` silently no-ops on
them — no error, no visible effect.

**2. Anything gated on focus or blur.** On a page nothing has interacted with
yet, `document.hasFocus()` is false — and in that state the JS `el.focus()`
that `type` performs moves `activeElement` **without the browser dispatching
a focus event**, so a later `blur()` dispatches nothing either. A
commit-on-blur or validate-on-blur handler never runs: the edit is never
saved and no request goes out. This one is nasty to diagnose, because the DOM
`.value` and the framework's own state both read back *correctly* — it looks
like an application bug rather than a harness artifact.

Worse, it's **stateful**. Any trusted input event — a `click`, a `key`, a
`type-keys` — gives the document focus, and from then on `type` *does* fire
focus/blur. So the identical `type` call works or silently doesn't depending
on what the session happened to do earlier. `navigate` resets `hasFocus` to
false again.

```
# fresh page (document.hasFocus() === false)
khora type      → ["input", "change"]                       (synthetic only)

# same call, after any trusted click/key
khora type      → ["focus", "focusin", "input", "change",
                   "blur", "focusout"]

# type-keys, either way
khora type-keys → ["focus", "focusin", "keydown", "input",
                   ..., "blur", "focusout"]                 (all trusted)
```

If a field's save-on-blur seems broken, check `khora eval "$S"
'document.hasFocus()'` before concluding it's the app.

`type-keys` dispatches the same trusted keydown/keypress/keyup sequence a real
keyboard produces (CDP `Input.dispatchKeyEvent`, one rawKeyDown + char + keyUp
per character), driving the browser's real input pipeline — which establishes
real focus, so both the keystrokes and a subsequent blur behave as they would
for a user.

```bash
khora type-keys "$S" ".xterm-helper-textarea" "ls -la"
khora key "$S" "Enter"           # or embed a newline in the text
```

Each character is sent with the `code` and virtual-key code of the physical
US-layout key that produces it — `-` is `Minus`/189, `!` is `Digit1`/49, a
newline is `Enter`/13. That matters for terminal emulators specifically:
xterm.js decides what to write to the PTY from `keyCode`, not `key`, so a
character carrying the wrong virtual key is delivered as an escape sequence
rather than as itself. (Up to khora 0.3.17, unmapped characters reported
their Unicode codepoint as the virtual key, which put `-` on 45 = Insert;
`stty -echo` reached the shell as `stty <ESC>[2~echo`.) Characters with no
US-layout key — accented letters, CJK, emoji — still arrive as text via the
`char` event.

Verifying a field that saves on blur — type with `type-keys`, then `blur` and
check the app actually committed:

```bash
khora type-keys "$S" "#title" "New title" --clear
khora blur "$S"
khora network "$S" | grep PATCH  # the commit-on-blur request should be there
```

See [`blur`](#blur) for why that beats the older `key Tab` recipe.

`type-keys` inserts at the caret rather than replacing, so typing into a field
that already has a value appends to it — pass `--clear` to wipe the field
first, which is what you want when editing an existing value (renaming a
title, correcting a form). `--clear` selects the current content and deletes it
with a trusted Backspace, so the clear goes through the same real-input path as
the typing; it is a no-op on an already-empty field, and on canvas-backed
widgets that own their own buffer (xterm.js), which have no value to select.

It doesn't handle control characters (Enter, Tab, Backspace, arrows, ...) —
send those individually with `key`.

### Blur

`blur` fires `blur`/`focusout` on the focused element, which is what a
commit-on-blur or validate-on-blur handler is waiting for:

```bash
khora blur "$S"            # blurs document.activeElement
khora blur "$S" "#title"   # blurs that element specifically
```

The two older ways of getting there both have a catch. `khora key "$S" Tab`
blurs the field but *also* moves focus onto the next element in the tab order,
firing whatever handlers it owns — so a failure downstream is hard to attribute
to the field you were testing. `khora eval "$S" 'document.activeElement.blur()'`
avoids that but reaches past the CLI into raw JS, and silently succeeds when
nothing was focused.

`blur` errors instead of no-opping when the target isn't the focused element —
either because nothing is focused, or because the selector names some other
element:

```
$ khora blur "$S" "#some-other-field"
nothing to blur: #some-other-field is not the focused element (focused: input#title)
```

That's deliberate. `el.blur()` on a non-focused element dispatches no events at
all, so reporting success would read as "the commit fired" when nothing
happened — the exact misdiagnosis this command exists to prevent.

The same document-focus caveat as `type` applies: on a page nothing has
interacted with, the browser dispatches no focus events, so there is nothing
focused to blur. Establish real focus first with `click`, `key`, or `type-keys`
(not `type` — see above).

### Scrolling

`wheel` dispatches a trusted native wheel event (CDP `Input.dispatchMouseEvent`,
type `mouseWheel`). Use it instead of `eval`-based scrolling (e.g.
`element.scrollTop = ...` or a synthetic `WheelEvent`) to verify scroll
chaining and `overscroll-behavior` — a synthetic `WheelEvent` never reaches
Chromium's scroll pipeline, so it fires listeners but never actually scrolls
or chains between containers.

```bash
khora wheel "$S" 100,150 0,300     # scroll down 300px at 100,150
khora wheel "$S" 100,150 0,-150    # scroll up
```

### Touch gestures

`touch`, `swipe` and `long-press` dispatch trusted touch events (CDP
`Input.dispatchTouchEvent`). They are not a stylistic alternative to the mouse
commands — they reach behavior the mouse path structurally cannot:

- **`touch-action` is a touch-only property.** It governs which panning
  gestures the compositor may take over, and only ever consults *touch*. A
  trusted `wheel` therefore cannot verify it, and a synthetic `TouchEvent`
  dispatched from `eval` never enters the scroll pipeline at all — it fires
  listeners while `scrollTop` stays put.
- **Touch-only handlers.** A component listening for `touchstart`/`touchmove`
  (mobile carousels, pull-to-refresh, dnd-kit's `TouchSensor`) sees nothing
  from a click.
- **`pointerType`.** Pointer events derived from a tap report
  `pointerType: "touch"`; the mouse path reports `"mouse"`, which gesture
  libraries routinely branch on.

```bash
khora touch "$S" 150,220                        # tap
khora swipe "$S" 200,600 200,200                # swipe up (--steps, --delay)
khora long-press "$S" 150,220 --duration 800    # press and hold, then release
```

A tap produces the full sequence a phone does — `touchstart`/`touchend`,
`pointerdown`/`pointerup`, the compatibility `mousedown`/`mouseup`, then
`click` — all trusted. `long-press` holds for real time, so it trips the
`setTimeout` armed on `touchstart` that drag-activation delays and context
menus use, which a tap cancels.

Verifying a phone layout end to end usually means pairing these with
`set-viewport`:

```bash
khora set-viewport "$S" 390x844 3 --mobile
khora swipe "$S" 200,600 200,200
khora eval "$S" 'document.querySelector(".board").scrollTop'
```

**Coordinates are raw viewport points**, so a target below the fold receives a
dispatch into empty space — and that still exits 0, leaving the page untouched
while every later assertion reads as a silent failure. Scroll the element into
view and measure it in the *same* `eval`, since a later scroll invalidates an
earlier measurement:

```bash
P=$(khora eval "$S" '(function(){var e=document.querySelector(".card");
  e.scrollIntoView({block:"center"});var r=e.getBoundingClientRect();
  return Math.round(r.x+r.width/2)+","+Math.round(r.y+r.height/2)})()')
khora touch "$S" "$P"
```

**Known limitation.** These commands do not turn on CDP touch *emulation*
(`Emulation.setTouchEmulationEnabled`), because it buys nothing here:
dispatch, native panning and `pointerType` are identical with and without it,
and Chrome reverts the override when the CDP client disconnects — which khora
does after every command — so it could never survive to be read by the next
one anyway. The consequence is that a page's **load-time feature detection**
still reports a non-touch device: `'ontouchstart' in window` is `false`,
`navigator.maxTouchPoints` is `0`, and `(pointer: coarse)` does not match. An
app that registers touch handlers only behind such a check won't have any for
these events to reach. Apps that always attach touch listeners (the common
case, including dnd-kit) are unaffected.

### Screenshot comparison

```bash
khora navigate "$S" "https://app.com"
khora screenshot "$S" -o before.png
# ... make changes ...
khora screenshot "$S" -o after.png
```

Shots cover the whole scrollable page by default, however far it extends below
the fold. `--full-page` names that default explicitly; `--viewport` opts out and
captures only the visible area.

```bash
khora screenshot "$S" -o whole-page.png --full-page   # same as no flag
khora screenshot "$S" -o above-the-fold.png --viewport
```

Whole-page capture clips to the document's content size and renders beyond the
viewport in place (CDP `captureBeyondViewport`), so it neither reflows the page
nor disturbs a viewport set earlier by [`set-viewport`](#phone-width-viewports).
`position: fixed`, `sticky` and `vh`-sized elements therefore appear at the size
they have on screen, anchored to the top of the shot.

Content that overflows a viewport-sized document — a tall `position: fixed`
overlay on a page pinned to `height: 100vh` — is *not* part of that content size
and gets cut off. Crop to the overlay with `--selector` to capture it whole, or
name the region directly with `--clip X,Y,WxH` when no single element wraps it:

```bash
khora screenshot "$S" -o overlay.png --clip 0,0,1920x2500
```

`--clip` takes page CSS pixels (the same coordinate space as `window.scrollX +
getBoundingClientRect()`) and captures exactly that rectangle, whatever the
document content size says. Measure the region first when you don't know it:

```bash
khora eval "$S" "var r=document.querySelector('.drawer').getBoundingClientRect(); \
  [r.x+scrollX, r.y+scrollY, r.width, r.height].join()"
```

Pass `--selector` to crop the shot to a single element's bounding box instead of
the page. The element is scrolled into view first, and the crop covers its full
bounding box even when that is taller than the viewport. If the selector matches
nothing (or the element has no visible area) the command errors with exit code 1
rather than silently falling back to a whole-page shot.

```bash
khora screenshot "$S" -o submit-button.png --selector "#submit"
khora screenshot "$S" -o tall-modal.png --selector "[role=dialog]"
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

### Open targets

```bash
khora targets "$S"        # `pages` is an alias
# TYPE             STATE          TITLE                          URL
# page             active,driven  My App                         http://localhost:3000/
```

Lists every open browser target (tabs, workers) with two independent flags:

- **`active`** — Chrome's foreground tab, read from `document.visibilityState`.
- **`driven`** — the target every *other* khora command operates on. Exactly one.

They normally coincide. When they don't, khora is driving a background tab —
clicks and `type` still "succeed" against an invisible page, and trusted `key`
dispatch silently arrives as `key:Unidentified` with an empty `code`. That
split is the first thing to check when a command reports success but the page
you're looking at didn't change:

```bash
khora targets "$S" | grep -q 'active,driven' || echo "khora is driving a background tab"
```

`targets` is read-only — unlike every other command it never creates a tab when
the browser has none, since manufacturing the thing you asked it to report
would defeat the point. Use `--format json` for `target_id`, `type`, `url`,
`title`, `active` and `driven` per target; `active` is `null` for targets that
can't be probed (workers, or a page whose evaluate timed out).

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

`--timeout` also bounds reconnecting to an existing session (used by every command, including `kill`/`status`), so a wedged Chrome fails fast instead of hanging indefinitely.

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error |
| 2 | Chrome not found |
| 3 | Timeout |
| 4 | Session not found or dead |
