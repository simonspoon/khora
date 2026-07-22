#!/usr/bin/env bash
# Khora end-to-end QA — exercises every CLI command against a test fixture.
# Usage: ./tests/e2e/qa.sh
set -uo pipefail

GREEN='\033[0;32m'
RED='\033[0;31m'
BOLD='\033[1m'
NC='\033[0m'

PASS=0
FAIL=0
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
FIXTURE="file://${SCRIPT_DIR}/fixture.html"
KHORA="${ROOT_DIR}/target/debug/khora"
SESSION=""
SCREENSHOT="/tmp/khora-qa-screenshot-$$.png"

# ── helpers ──────────────────────────────────────────────

assert_contains() {
  local label="$1" actual="$2" expected="$3"
  if echo "$actual" | grep -qF -- "$expected"; then
    printf "  ${GREEN}PASS${NC}  %s\n" "$label"
    ((PASS++))
  else
    printf "  ${RED}FAIL${NC}  %s\n" "$label"
    printf "       expected to contain: %s\n" "$expected"
    printf "       got: %s\n" "$actual"
    ((FAIL++))
  fi
}

assert_not_contains() {
  local label="$1" actual="$2" unexpected="$3"
  if echo "$actual" | grep -qF -- "$unexpected"; then
    printf "  ${RED}FAIL${NC}  %s\n" "$label"
    printf "       expected NOT to contain: %s\n" "$unexpected"
    printf "       got: %s\n" "$actual"
    ((FAIL++))
  else
    printf "  ${GREEN}PASS${NC}  %s\n" "$label"
    ((PASS++))
  fi
}

assert_exit() {
  local label="$1" actual="$2" expected="$3"
  if [[ "$actual" -eq "$expected" ]]; then
    printf "  ${GREEN}PASS${NC}  %s\n" "$label"
    ((PASS++))
  else
    printf "  ${RED}FAIL${NC}  %s\n" "$label"
    printf "       expected exit %s, got %s\n" "$expected" "$actual"
    ((FAIL++))
  fi
}

assert_file() {
  local label="$1" path="$2"
  if [[ -f "$path" ]]; then
    printf "  ${GREEN}PASS${NC}  %s\n" "$label"
    ((PASS++))
  else
    printf "  ${RED}FAIL${NC}  %s\n" "$label"
    printf "       file not found: %s\n" "$path"
    ((FAIL++))
  fi
}

assert_ge() {
  local label="$1" actual="$2" minimum="$3"
  if [[ "$actual" -ge "$minimum" ]]; then
    printf "  ${GREEN}PASS${NC}  %s\n" "$label"
    ((PASS++))
  else
    printf "  ${RED}FAIL${NC}  %s\n" "$label"
    printf "       expected >= %s, got %s\n" "$minimum" "$actual"
    ((FAIL++))
  fi
}

assert_process_gone() {
  local label="$1" pid="$2"
  if ! kill -0 "$pid" 2>/dev/null; then
    printf "  ${GREEN}PASS${NC}  %s\n" "$label"
    ((PASS++))
  else
    printf "  ${RED}FAIL${NC}  %s\n" "$label"
    printf "       Chrome process %s is still running\n" "$pid"
    ((FAIL++))
  fi
}

cleanup() {
  if [[ -n "$SESSION" ]]; then
    "$KHORA" kill "$SESSION" >/dev/null 2>&1 || true
  fi
  rm -f "$SCREENSHOT"
}
trap cleanup EXIT

# ── build ────────────────────────────────────────────────

printf "${BOLD}Building khora...${NC}\n"
if ! cargo build --manifest-path "$ROOT_DIR/Cargo.toml" 2>&1; then
  printf "${RED}Build failed${NC}\n"
  exit 1
fi
echo ""

# ── launch ───────────────────────────────────────────────

printf "${BOLD}▸ launch${NC}\n"
OUTPUT=$("$KHORA" launch 2>&1)
EC=$?
assert_exit "launch exits 0" "$EC" 0
SESSION=$(echo "$OUTPUT" | grep -oE 'Session: [a-f0-9]+' | head -1 | awk '{print $2}')
if [[ -z "$SESSION" ]]; then
  printf "  ${RED}FAIL${NC}  could not extract session ID from: %s\n" "$OUTPUT"
  exit 1
fi
printf "  ${GREEN}PASS${NC}  session ID extracted\n"
((PASS++))
printf "       session: %s\n" "$SESSION"

# ── status (alive) ──────────────────────────────────────

printf "\n${BOLD}▸ status${NC}\n"
OUTPUT=$("$KHORA" status "$SESSION" 2>&1)
EC=$?
assert_exit "status exits 0" "$EC" 0
assert_contains "session is alive" "$OUTPUT" "alive"

# Smoke check for mesa task 386: mouse_move on a never-painted page (the
# fresh about:blank tab a launch starts on, before any navigate) is where a
# ~5s cold-compositor hit-test tax was reported (mesa task 385), which
# would race khora's 5000ms default timeout. mouse_move now force-warms the
# compositor first as a preventive measure — but that tax did not
# reproduce live against this repo's current Chrome even without the fix,
# so this only confirms the command still works on a fresh tab, not that it
# catches the tax; see client.rs's force_compositor_frame call in
# mouse_move for the honest caveat.
OUTPUT=$("$KHORA" mouse-move "$SESSION" "10,10" 2>&1)
EC=$?
assert_exit "mouse-move on fresh about:blank exits 0" "$EC" 0

# ── navigate ─────────────────────────────────────────────

printf "\n${BOLD}▸ navigate${NC}\n"
OUTPUT=$("$KHORA" navigate "$SESSION" "$FIXTURE" 2>&1)
EC=$?
assert_exit "navigate exits 0" "$EC" 0
# Small settle time for page scripts to initialize
sleep 0.3

# ── navigate --no-cache ─────────────────────────────────

printf "\n${BOLD}▸ navigate --no-cache${NC}\n"
OUTPUT=$("$KHORA" navigate "$SESSION" "$FIXTURE" --no-cache 2>&1)
EC=$?
assert_exit "navigate --no-cache exits 0" "$EC" 0
assert_contains "navigate --no-cache reports bypass" "$OUTPUT" "cache bypassed"
sleep 0.3

# ── find (single element) ───────────────────────────────

printf "\n${BOLD}▸ find${NC}\n"
OUTPUT=$("$KHORA" find "$SESSION" "#heading" 2>&1)
EC=$?
assert_exit "find #heading exits 0" "$EC" 0
assert_contains "find #heading returns h1" "$OUTPUT" "<h1>"

# ── find (multiple elements) ────────────────────────────

OUTPUT=$("$KHORA" find "$SESSION" ".item" 2>&1)
COUNT=$(echo "$OUTPUT" | grep -c "<li>" || true)
assert_ge "find .item returns >= 3 results" "$COUNT" 3

# ── text ─────────────────────────────────────────────────

printf "\n${BOLD}▸ text${NC}\n"
OUTPUT=$("$KHORA" text "$SESSION" "#greeting" 2>&1)
assert_contains "text #greeting" "$OUTPUT" "Hello, Khora!"

OUTPUT=$("$KHORA" text "$SESSION" "#heading" 2>&1)
assert_contains "text #heading" "$OUTPUT" "Khora Test Page"

# ── attribute ────────────────────────────────────────────

printf "\n${BOLD}▸ attribute${NC}\n"
OUTPUT=$("$KHORA" attribute "$SESSION" "#greeting" "data-testid" 2>&1)
assert_contains "attribute data-testid" "$OUTPUT" "hello"

OUTPUT=$("$KHORA" attribute "$SESSION" "#greeting" "data-role" 2>&1)
assert_contains "attribute data-role" "$OUTPUT" "primary"

# ── type ─────────────────────────────────────────────────

printf "\n${BOLD}▸ type${NC}\n"
OUTPUT=$("$KHORA" type "$SESSION" "#name-input" "khora-test" 2>&1)
EC=$?
assert_exit "type exits 0" "$EC" 0

# Verify via eval
OUTPUT=$("$KHORA" eval "$SESSION" "document.getElementById('name-input').value" 2>&1)
assert_contains "typed text persisted" "$OUTPUT" "khora-test"

# React-controlled input: onChange must actually fire, not just DOM .value
OUTPUT=$("$KHORA" type "$SESSION" "#react-input" "khora-react" 2>&1)
EC=$?
assert_exit "type into react-input exits 0" "$EC" 0

OUTPUT=$("$KHORA" text "$SESSION" "#react-input-result" 2>&1)
assert_contains "react onChange fired" "$OUTPUT" "onchange:1 value:khora-react"

# ── click ────────────────────────────────────────────────

printf "\n${BOLD}▸ click${NC}\n"
OUTPUT=$("$KHORA" click "$SESSION" "#counter-btn" 2>&1)
EC=$?
assert_exit "click exits 0" "$EC" 0

OUTPUT=$("$KHORA" text "$SESSION" "#counter-btn" 2>&1)
assert_contains "click updated button text" "$OUTPUT" "Clicked 1"

# ── drag ─────────────────────────────────────────────────

printf "\n${BOLD}▸ drag${NC}\n"
# Resolve drag-zone corners at runtime — fixture layout may shift
POINTS=$("$KHORA" eval "$SESSION" "var r=document.getElementById('drag-zone').getBoundingClientRect(); Math.round(r.x+10)+','+Math.round(r.y+10)+' '+Math.round(r.x+250)+','+Math.round(r.y+80)" 2>&1)
FROM="${POINTS% *}"
TO="${POINTS#* }"
OUTPUT=$("$KHORA" drag "$SESSION" "$FROM" "$TO" --steps 8 2>&1)
EC=$?
assert_exit "drag exits 0" "$EC" 0
assert_contains "drag reports path" "$OUTPUT" "8 steps"

OUTPUT=$("$KHORA" text "$SESSION" "#drag-result" 2>&1)
assert_contains "drag events are trusted" "$OUTPUT" "trusted:true"
assert_contains "drag dispatched all moves" "$OUTPUT" "moves:8"
assert_contains "drag released at target" "$OUTPUT" "->$TO"

# Pointer-capture-guarded drag (splitter pattern): only trusted input
# satisfies hasPointerCapture, so this proves drag drives real widgets.
POINTS=$("$KHORA" eval "$SESSION" "var r=document.getElementById('splitter').getBoundingClientRect(); Math.round(r.x+10)+','+Math.round(r.y+20)+' '+Math.round(r.x+250)+','+Math.round(r.y+20)" 2>&1)
FROM="${POINTS% *}"
TO="${POINTS#* }"
OUTPUT=$("$KHORA" drag "$SESSION" "$FROM" "$TO" --steps 6 2>&1)
EC=$?
assert_exit "capture drag exits 0" "$EC" 0

OUTPUT=$("$KHORA" text "$SESSION" "#splitter-result" 2>&1)
assert_contains "capture drag held pointer capture" "$OUTPUT" "captured:true"
assert_contains "capture drag moves seen under capture" "$OUTPUT" "capmoves:6"
assert_contains "capture drag events are trusted" "$OUTPUT" "trusted:true"

# ── mouse-down / mouse-move / mouse-up (step-wise drag) ─────

printf "\n${BOLD}▸ mouse-down/move/up${NC}\n"
# Same fixture as drag, but scripted as separate steps with a screenshot
# in between — proves mid-gesture state can be inspected without racing.
POINTS=$("$KHORA" eval "$SESSION" "var r=document.getElementById('drag-zone').getBoundingClientRect(); Math.round(r.x+10)+','+Math.round(r.y+10)+' '+Math.round(r.x+150)+','+Math.round(r.y+50)+' '+Math.round(r.x+250)+','+Math.round(r.y+80)" 2>&1)
FROM=$(echo "$POINTS" | cut -d' ' -f1)
MID=$(echo "$POINTS" | cut -d' ' -f2)
TO=$(echo "$POINTS" | cut -d' ' -f3)

OUTPUT=$("$KHORA" mouse-down "$SESSION" "$FROM" 2>&1)
EC=$?
assert_exit "mouse-down exits 0" "$EC" 0

OUTPUT=$("$KHORA" mouse-move "$SESSION" "$MID" 2>&1)
EC=$?
assert_exit "mouse-move exits 0" "$EC" 0

MIDSHOT="/tmp/khora-qa-mid-drag-$$.png"
OUTPUT=$("$KHORA" screenshot "$SESSION" -o "$MIDSHOT" 2>&1)
EC=$?
assert_exit "screenshot mid-gesture exits 0" "$EC" 0
assert_file "mid-gesture screenshot written" "$MIDSHOT"
rm -f "$MIDSHOT"

OUTPUT=$("$KHORA" mouse-up "$SESSION" "$TO" 2>&1)
EC=$?
assert_exit "mouse-up exits 0" "$EC" 0

OUTPUT=$("$KHORA" text "$SESSION" "#drag-result" 2>&1)
assert_contains "step-wise drag events are trusted" "$OUTPUT" "trusted:true"
assert_contains "step-wise drag dispatched one move" "$OUTPUT" "moves:1"
assert_contains "step-wise drag released at target" "$OUTPUT" "->$TO"

# Same step-wise sequence against the pointer-capture-guarded splitter: each
# mouse-down/mouse-move/mouse-up is its own CLI process and CDP connection,
# so this proves the page (not the connection) holds pointer-capture state
# across steps — the premise "mouse-move carries over button state" depends on it.
POINTS=$("$KHORA" eval "$SESSION" "var r=document.getElementById('splitter').getBoundingClientRect(); Math.round(r.x+10)+','+Math.round(r.y+20)+' '+Math.round(r.x+250)+','+Math.round(r.y+20)" 2>&1)
FROM="${POINTS% *}"
TO="${POINTS#* }"

OUTPUT=$("$KHORA" mouse-down "$SESSION" "$FROM" 2>&1)
EC=$?
assert_exit "step-wise mouse-down on splitter exits 0" "$EC" 0

OUTPUT=$("$KHORA" mouse-move "$SESSION" "$TO" 2>&1)
EC=$?
assert_exit "step-wise mouse-move on splitter exits 0" "$EC" 0

OUTPUT=$("$KHORA" mouse-up "$SESSION" "$TO" 2>&1)
EC=$?
assert_exit "step-wise mouse-up on splitter exits 0" "$EC" 0

OUTPUT=$("$KHORA" text "$SESSION" "#splitter-result" 2>&1)
assert_contains "step-wise drag held pointer capture across connections" "$OUTPUT" "captured:true"
assert_contains "step-wise drag moves seen under capture" "$OUTPUT" "capmoves:1"
assert_contains "step-wise splitter events are trusted" "$OUTPUT" "trusted:true"

# ── click-at / dblclick-at ───────────────────────────────

printf "\n${BOLD}▸ click-at/dblclick-at${NC}\n"
# Point-based clicks hit whatever is at the pixel, not a resolved selector —
# resolve the target's center at runtime like the drag fixtures do.
POINT=$("$KHORA" eval "$SESSION" "var r=document.getElementById('point-target').getBoundingClientRect(); Math.round(r.x+r.width/2)+','+Math.round(r.y+r.height/2)" 2>&1)

OUTPUT=$("$KHORA" click-at "$SESSION" "$POINT" 2>&1)
EC=$?
assert_exit "click-at exits 0" "$EC" 0

OUTPUT=$("$KHORA" text "$SESSION" "#point-click-result" 2>&1)
assert_contains "click-at registered a click" "$OUTPUT" "click:1"
assert_contains "click-at event is trusted" "$OUTPUT" "trusted:true"

OUTPUT=$("$KHORA" dblclick-at "$SESSION" "$POINT" 2>&1)
EC=$?
assert_exit "dblclick-at exits 0" "$EC" 0

OUTPUT=$("$KHORA" text "$SESSION" "#point-click-result" 2>&1)
assert_contains "dblclick-at registered a dblclick" "$OUTPUT" "dblclick:1"
assert_contains "dblclick-at event is trusted" "$OUTPUT" "trusted:true"

# ── wheel ────────────────────────────────────────────────

printf "\n${BOLD}▸ wheel${NC}\n"
POINT=$("$KHORA" eval "$SESSION" "var r=document.getElementById('scroll-inner').getBoundingClientRect(); Math.round(r.x+r.width/2)+','+Math.round(r.y+r.height/2)" 2>&1)

OUTPUT=$("$KHORA" wheel "$SESSION" "$POINT" "0,1000" 2>&1)
EC=$?
assert_exit "wheel exits 0" "$EC" 0

OUTPUT=$("$KHORA" text "$SESSION" "#wheel-result" 2>&1)
assert_contains "wheel event is trusted" "$OUTPUT" "trusted:true"

INNER_SCROLL=$("$KHORA" eval "$SESSION" "document.getElementById('scroll-inner').scrollTop" 2>&1)
assert_ge "wheel drove real native scroll" "$INNER_SCROLL" 1

# overscroll-behavior:contain on the inner container means the remaining
# delta (1000px requested, only ~540px scrollable) doesn't chain to the
# outer container once inner hits its scroll bound — same topology as the
# task-382 sidebar bug this command was built to verify.
OUTER_SCROLL=$("$KHORA" eval "$SESSION" "document.getElementById('scroll-outer').scrollTop" 2>&1)
assert_contains "wheel scroll did not chain past contain boundary" "$OUTER_SCROLL" "0"

# ── key ──────────────────────────────────────────────────

printf "\n${BOLD}▸ key${NC}\n"
OUTPUT=$("$KHORA" key "$SESSION" "Cmd+D" 2>&1)
EC=$?
assert_exit "key Cmd+D exits 0" "$EC" 0

OUTPUT=$("$KHORA" text "$SESSION" "#key-result" 2>&1)
# Real Cmd+D (no Shift) reports lowercase e.key, matching what a page's own
# `if (e.metaKey && e.key === 'd')` shortcut handler checks for.
assert_contains "key reports lowercase key without shift" "$OUTPUT" "key:d"
assert_contains "key reports code" "$OUTPUT" "code:KeyD"
assert_contains "key reports meta modifier" "$OUTPUT" "meta:true"
assert_contains "key event is trusted" "$OUTPUT" "trusted:true"

# Discriminating case: Shift flips e.key to uppercase while code/vk (physical
# key identity) stay the same — proves the combo isn't just echoing input case.
OUTPUT=$("$KHORA" key "$SESSION" "Cmd+Shift+D" 2>&1)
EC=$?
assert_exit "key Cmd+Shift+D exits 0" "$EC" 0

OUTPUT=$("$KHORA" text "$SESSION" "#key-result" 2>&1)
assert_contains "key reports uppercase key with shift" "$OUTPUT" "key:D"
assert_contains "key reports shift modifier" "$OUTPUT" "shift:true"
assert_contains "key still reports meta modifier" "$OUTPUT" "meta:true"

OUTPUT=$("$KHORA" key "$SESSION" "Escape" 2>&1)
EC=$?
assert_exit "key Escape exits 0" "$EC" 0

OUTPUT=$("$KHORA" text "$SESSION" "#key-result" 2>&1)
assert_contains "key with no modifier reports key" "$OUTPUT" "key:Escape"
assert_contains "key with no modifier has no meta" "$OUTPUT" "meta:false"

# Regression for mesa task 384: a key press that reaches the renderer used
# to leave headless Chrome in a state where the next `wheel` call never
# acked, hanging for chromiumoxide's full 30s internal request timeout.
# key_press() now forces a compositor frame after dispatch, so this must
# stay fast (well under the 30s hang) rather than just eventually exit 0.
START=$(date +%s)
OUTPUT=$("$KHORA" wheel "$SESSION" "$POINT" "0,10" 2>&1)
EC=$?
ELAPSED=$(($(date +%s) - START))
assert_exit "wheel after key exits 0" "$EC" 0
if [ "$ELAPSED" -lt 10 ]; then
  printf "  ${GREEN}PASS${NC}  wheel after key stayed fast (${ELAPSED}s)\n"
  ((PASS++))
else
  printf "  ${RED}FAIL${NC}  wheel after key took ${ELAPSED}s (expected <10s, was hanging 30s pre-fix)\n"
  ((FAIL++))
fi

# ── type-keys ────────────────────────────────────────────

# Regression for mesa task 400: `type` sets .value via JS, which xterm.js-style
# widgets (a hidden textarea that only reacts to real keydown/keypress) never
# read — silently no-ops, no error. Confirm that first, then confirm
# type-keys' trusted per-char key dispatch reaches it where type can't.
printf "\n${BOLD}▸ type-keys${NC}\n"
OUTPUT=$("$KHORA" type "$SESSION" "#term-input" "ignored" 2>&1)
EC=$?
assert_exit "type into xterm-style textarea exits 0 (silently)" "$EC" 0

OUTPUT=$("$KHORA" text "$SESSION" "#term-result" 2>&1)
assert_not_contains "type has no visible effect on xterm-style widget" "$OUTPUT" "typed:"

# Mixed caps, space, digits, punctuation — exercises every char_key_info
# branch (letters preserve case, space, digits, and the Unicode/punctuation
# fallback), not just plain lowercase letters.
OUTPUT=$("$KHORA" type-keys "$SESSION" "#term-input" "Hi there-42" 2>&1)
EC=$?
assert_exit "type-keys exits 0" "$EC" 0

OUTPUT=$("$KHORA" text "$SESSION" "#term-result" 2>&1)
assert_contains "type-keys reaches xterm-style widget with mixed chars" "$OUTPUT" "typed:Hi there-42"
assert_contains "type-keys events are trusted" "$OUTPUT" "trusted:true"

OUTPUT=$("$KHORA" type-keys "$SESSION" "nonexistent-selector" "x" 2>&1)
EC=$?
assert_exit "type-keys on missing selector exits nonzero" "$EC" 1

# mesa task 464: text inserts at the caret, so editing a field that already
# has a value appends to it. Confirm that, then that --clear replaces.
"$KHORA" eval "$SESSION" 'document.querySelector("#focus-input").value = "Old"; "ok"' >/dev/null 2>&1
"$KHORA" type-keys "$SESSION" "#focus-input" "New" >/dev/null 2>&1
OUTPUT=$("$KHORA" eval "$SESSION" 'document.querySelector("#focus-input").value' 2>&1)
assert_contains "type-keys without --clear appends to the existing value" "$OUTPUT" "OldNew"

"$KHORA" eval "$SESSION" 'document.querySelector("#focus-input").value = "Old"; "ok"' >/dev/null 2>&1
OUTPUT=$("$KHORA" type-keys "$SESSION" "#focus-input" "New" --clear 2>&1)
EC=$?
assert_exit "type-keys --clear exits 0" "$EC" 0
OUTPUT=$("$KHORA" eval "$SESSION" 'document.querySelector("#focus-input").value' 2>&1)
assert_not_contains "type-keys --clear leaves none of the old value" "$OUTPUT" "Old"
assert_contains "type-keys --clear replaces the existing value" "$OUTPUT" "New"

# An already-empty field is the degenerate case: the Backspace has nothing to
# delete and must not eat a character or fail.
"$KHORA" eval "$SESSION" 'document.querySelector("#focus-input").value = ""; "ok"' >/dev/null 2>&1
OUTPUT=$("$KHORA" type-keys "$SESSION" "#focus-input" "Fresh" --clear 2>&1)
EC=$?
assert_exit "type-keys --clear on an empty field exits 0" "$EC" 0
OUTPUT=$("$KHORA" eval "$SESSION" 'document.querySelector("#focus-input").value' 2>&1)
assert_contains "type-keys --clear on an empty field types the full text" "$OUTPUT" "Fresh"

# Regression for mesa task 385: trusted-input commands used to ignore
# --timeout/KHORA_TIMEOUT entirely, inheriting chromiumoxide's hardcoded 30s
# internal request timeout instead. click-at now bounds its CDP round-trip
# with the same timeout goto()/wait-for use, so an unreasonably short
# --timeout must fail fast (exit 3) rather than hang toward 30s.
#
# Uses 0ms rather than 1ms: on fast/warm hardware the real CDP round-trip
# can complete in under 1ms, so a 1ms bound is a race the op sometimes wins
# (verified independently — the timeout mechanism itself is not at fault).
# A 0ms tokio::time::timeout always elapses on first poll, so this is
# deterministic.
START=$(date +%s)
OUTPUT=$("$KHORA" --timeout 0 click-at "$SESSION" "$POINT" 2>&1)
EC=$?
ELAPSED=$(($(date +%s) - START))
assert_exit "click-at --timeout 0 exits 3" "$EC" 3
assert_contains "click-at --timeout 0 reports timeout" "$OUTPUT" "timed out after 0ms"
if [ "$ELAPSED" -lt 10 ]; then
  printf "  ${GREEN}PASS${NC}  click-at --timeout 0 failed fast (${ELAPSED}s)\n"
  ((PASS++))
else
  printf "  ${RED}FAIL${NC}  click-at --timeout 0 took ${ELAPSED}s (expected <10s)\n"
  ((FAIL++))
fi

# ── console ──────────────────────────────────────────────

printf "\n${BOLD}▸ console${NC}\n"
OUTPUT=$("$KHORA" console "$SESSION" 2>&1)
assert_contains "console captured click log" "$OUTPUT" "counter:1"

# ── eval ─────────────────────────────────────────────────

printf "\n${BOLD}▸ eval${NC}\n"
OUTPUT=$("$KHORA" eval "$SESSION" "window.khoraTestValue" 2>&1)
assert_contains "eval returns window value" "$OUTPUT" "42"

OUTPUT=$("$KHORA" eval "$SESSION" "2 + 2" 2>&1)
assert_contains "eval arithmetic" "$OUTPUT" "4"

OUTPUT=$("$KHORA" eval "$SESSION" "document.title" 2>&1)
assert_contains "eval document.title" "$OUTPUT" "Khora QA Fixture"

OUTPUT=$("$KHORA" eval "$SESSION" "undefined" 2>&1)
EC=$?
assert_exit "eval undefined exits 0" "$EC" 0
assert_contains "eval undefined prints null" "$OUTPUT" "null"

OUTPUT=$("$KHORA" eval "$SESSION" "void 0" 2>&1)
EC=$?
assert_exit "eval void expr exits 0" "$EC" 0
assert_contains "eval void expr prints null" "$OUTPUT" "null"

OUTPUT=$("$KHORA" eval "$SESSION" "null" 2>&1)
assert_contains "eval null prints null" "$OUTPUT" "null"

# ── screenshot ───────────────────────────────────────────

printf "\n${BOLD}▸ screenshot${NC}\n"
rm -f "$SCREENSHOT"
OUTPUT=$("$KHORA" screenshot "$SESSION" -o "$SCREENSHOT" 2>&1)
EC=$?
assert_exit "screenshot exits 0" "$EC" 0
assert_file "screenshot file created" "$SCREENSHOT"

# Check file is a valid PNG (starts with PNG magic bytes)
if file "$SCREENSHOT" | grep -q "PNG"; then
  printf "  ${GREEN}PASS${NC}  screenshot is valid PNG\n"
  ((PASS++))
else
  printf "  ${RED}FAIL${NC}  screenshot is not valid PNG\n"
  ((FAIL++))
fi

# --selector crops to the element bounding box (smaller than full page)
FULL_SIZE=$(wc -c <"$SCREENSHOT")
rm -f "$SCREENSHOT"
OUTPUT=$("$KHORA" screenshot "$SESSION" -o "$SCREENSHOT" --selector "#heading" 2>&1)
EC=$?
assert_exit "screenshot --selector exits 0" "$EC" 0
assert_file "screenshot --selector file created" "$SCREENSHOT"
CROP_SIZE=$(wc -c <"$SCREENSHOT")
if [ "$CROP_SIZE" -lt "$FULL_SIZE" ]; then
  printf "  ${GREEN}PASS${NC}  cropped screenshot smaller than full page\n"
  ((PASS++))
else
  printf "  ${RED}FAIL${NC}  cropped screenshot not smaller than full page\n"
  ((FAIL++))
fi

# --selector with no match errors (exit 1), does not fall back to full page
OUTPUT=$("$KHORA" screenshot "$SESSION" -o /tmp/khora-qa-nomatch-$$.png --selector "#does-not-exist" 2>&1)
EC=$?
assert_exit "screenshot --selector missing errors" "$EC" 1
assert_contains "screenshot --selector missing message" "$OUTPUT" "element not found"

# --full-page vs --viewport. The fixture is exactly viewport-height, so grow it
# past the fold for this check and shrink it back, leaving later cases alone.
"$KHORA" eval "$SESSION" \
  "document.body.insertAdjacentHTML('beforeend', '<div id=\"tall-spacer\" style=\"height:3000px\"></div>')" \
  >/dev/null 2>&1
rm -f "$SCREENSHOT"
OUTPUT=$("$KHORA" screenshot "$SESSION" -o "$SCREENSHOT" --full-page 2>&1)
EC=$?
assert_exit "screenshot --full-page exits 0" "$EC" 0
TALL_SIZE=$(wc -c <"$SCREENSHOT")
rm -f "$SCREENSHOT"
OUTPUT=$("$KHORA" screenshot "$SESSION" -o "$SCREENSHOT" --viewport 2>&1)
EC=$?
assert_exit "screenshot --viewport exits 0" "$EC" 0
VIEW_SIZE=$(wc -c <"$SCREENSHOT")
if [ "$VIEW_SIZE" -lt "$TALL_SIZE" ]; then
  printf "  ${GREEN}PASS${NC}  --viewport shot smaller than --full-page shot\n"
  ((PASS++))
else
  printf "  ${RED}FAIL${NC}  --viewport shot not smaller than --full-page shot\n"
  ((FAIL++))
fi
"$KHORA" eval "$SESSION" "document.getElementById('tall-spacer').remove()" >/dev/null 2>&1

# --clip reaches past the document content size. A `position: fixed` overlay on
# a non-scrolling document contributes nothing to content size, so --full-page
# (which clips to it) cuts the overlay off; --clip names the region directly.
"$KHORA" eval "$SESSION" \
  "document.body.style.overflow='hidden'; document.body.insertAdjacentHTML('beforeend', '<div id=\"tall-fixed\" style=\"position:fixed;left:0;top:0;width:400px;height:2500px;background:linear-gradient(#08f,#f80)\"></div>')" \
  >/dev/null 2>&1
rm -f "$SCREENSHOT"
OUTPUT=$("$KHORA" screenshot "$SESSION" -o "$SCREENSHOT" --full-page 2>&1)
assert_not_contains "--full-page cannot reach a fixed overlay" \
  "$(file "$SCREENSHOT")" "x 2500"
rm -f "$SCREENSHOT"
OUTPUT=$("$KHORA" screenshot "$SESSION" -o "$SCREENSHOT" --clip 0,0,400x2500 2>&1)
EC=$?
assert_exit "screenshot --clip exits 0" "$EC" 0
assert_contains "--clip captures the full 400x2500 region" \
  "$(file "$SCREENSHOT")" "400 x 2500"
"$KHORA" eval "$SESSION" \
  "document.getElementById('tall-fixed').remove(); document.body.style.overflow=''" \
  >/dev/null 2>&1

# --clip rejects a malformed region before touching Chrome
OUTPUT=$("$KHORA" screenshot "$SESSION" -o /tmp/khora-qa-badclip-$$.png --clip 0,0,1920 2>&1)
EC=$?
assert_exit "screenshot --clip malformed rejected" "$EC" 2
assert_contains "screenshot --clip malformed message" "$OUTPUT" "X,Y,WxH"

# --full-page and --viewport are mutually exclusive
OUTPUT=$("$KHORA" screenshot "$SESSION" -o /tmp/khora-qa-conflict-$$.png --full-page --viewport 2>&1)
EC=$?
assert_exit "screenshot --full-page --viewport conflicts" "$EC" 2

# --clip and --selector are mutually exclusive
OUTPUT=$("$KHORA" screenshot "$SESSION" -o /tmp/khora-qa-conflict-$$.png --clip 0,0,10x10 --selector "#heading" 2>&1)
EC=$?
assert_exit "screenshot --clip --selector conflicts" "$EC" 2

# ── wait-for ─────────────────────────────────────────────

printf "\n${BOLD}▸ wait-for${NC}\n"
# Click button that appends #appeared to DOM after 500ms
"$KHORA" click "$SESSION" "#show-btn" >/dev/null 2>&1
OUTPUT=$("$KHORA" wait-for "$SESSION" "#appeared" --timeout 5000 2>&1)
EC=$?
assert_exit "wait-for #appeared" "$EC" 0

# Verify the element has correct content
OUTPUT=$("$KHORA" text "$SESSION" "#appeared" 2>&1)
assert_contains "appeared element has text" "$OUTPUT" "I appeared!"

# ── wait-gone ────────────────────────────────────────────

printf "\n${BOLD}▸ wait-gone${NC}\n"
# Click button that removes #ephemeral from DOM after 500ms
"$KHORA" click "$SESSION" "#hide-btn" >/dev/null 2>&1
OUTPUT=$("$KHORA" wait-gone "$SESSION" "#ephemeral" --timeout 5000 2>&1)
EC=$?
assert_exit "wait-gone #ephemeral" "$EC" 0

# ── network ──────────────────────────────────────────────

printf "\n${BOLD}▸ network${NC}\n"
# Trigger fetch + XHR via eval (click can hang on some elements)
"$KHORA" eval "$SESSION" "var b=URL.createObjectURL(new Blob(['ok'])); fetch(b); var x=new XMLHttpRequest(); x.open('POST',b); x.send('hi'); 'ok'" >/dev/null 2>&1
sleep 0.5  # let fetch + XHR complete
OUTPUT=$("$KHORA" network "$SESSION" 2>&1)
EC=$?
assert_exit "network exits 0" "$EC" 0
assert_contains "network captured fetch" "$OUTPUT" "fetch"
assert_contains "network captured xhr" "$OUTPUT" "xhr"
assert_contains "network has POST method" "$OUTPUT" "POST"

# ── set-viewport ─────────────────────────────────────────

printf "\n${BOLD}▸ set-viewport${NC}\n"
OUTPUT=$("$KHORA" set-viewport "$SESSION" 390x844 3 --mobile 2>&1)
EC=$?
assert_exit "set-viewport exits 0" "$EC" 0
assert_contains "set-viewport reports size" "$OUTPUT" "390x844"

OUTPUT=$("$KHORA" eval "$SESSION" "window.innerWidth" 2>&1)
assert_contains "viewport width applied (below headless ~500px clamp)" "$OUTPUT" "390"

# Size override persists across navigation
"$KHORA" navigate "$SESSION" "$FIXTURE" >/dev/null 2>&1
OUTPUT=$("$KHORA" eval "$SESSION" "window.innerWidth" 2>&1)
assert_contains "viewport persists across navigate" "$OUTPUT" "390"

# Regression: a full-page screenshot used to be implemented as
# setDeviceMetricsOverride -> capture -> clearDeviceMetricsOverride, so taking
# one silently reset the override installed here.
"$KHORA" screenshot "$SESSION" -o "$SCREENSHOT" >/dev/null 2>&1
OUTPUT=$("$KHORA" eval "$SESSION" "window.innerWidth" 2>&1)
assert_contains "viewport survives a screenshot" "$OUTPUT" "390"

# ── JSON output format ──────────────────────────────────

printf "\n${BOLD}▸ JSON format${NC}\n"
OUTPUT=$("$KHORA" -f json text "$SESSION" "#heading" 2>&1)
assert_contains "json text has bracket" "$OUTPUT" "["
assert_contains "json text has content" "$OUTPUT" "Khora Test Page"

OUTPUT=$("$KHORA" -f json status "$SESSION" 2>&1)
assert_contains "json status has brace" "$OUTPUT" "{"
assert_contains "json status has alive" "$OUTPUT" "alive"
CHROME_PID=$(echo "$OUTPUT" | grep -oE '"pid": [0-9]+' | awk '{print $2}')

OUTPUT=$("$KHORA" -f json find "$SESSION" "#greeting" 2>&1)
assert_contains "json find has bracket" "$OUTPUT" "["

# Regression for mesa task 386: CdpClient::connect() used to have no
# timeout bound at all, so a wedged Chrome could leave `khora kill` (or any
# other command routed through connect(), like `status`) hanging
# indefinitely. connect() now bounds its handshake with
# --timeout/KHORA_TIMEOUT, so an unreasonably short --timeout must resolve
# fast rather than hang, instead of blocking.
#
# Unlike the click-at case above, the alive/dead *verdict* here is NOT
# deterministic and must not be asserted. click-at bounds a single CDP
# round-trip, so a 0ms timeout always elapses it. `status` calls
# `Browser::connect()`, whose future can resolve on its first poll (the
# websocket is driven by the spawned handler task), in which case
# timeout(0ms) never gets to fire and `is_alive()` — a PID check, not a
# connection check — reports alive. Measured 5 alive / 1 dead over 6 runs of
# the same binary. What connect()'s timeout bound actually guarantees is that
# the command *resolves* rather than hangs, so assert that and accept either
# verdict.
START=$(date +%s)
OUTPUT=$("$KHORA" --timeout 0 status "$SESSION" 2>&1)
EC=$?
ELAPSED=$(($(date +%s) - START))
assert_exit "status --timeout 0 exits 0" "$EC" 0
if echo "$OUTPUT" | grep -qE "is (alive|dead)"; then
  printf "  ${GREEN}PASS${NC}  status --timeout 0 returns a verdict, not a hang\n"
  ((PASS++))
else
  printf "  ${RED}FAIL${NC}  status --timeout 0 gave no alive/dead verdict: ${OUTPUT}\n"
  ((FAIL++))
fi
if [ "$ELAPSED" -lt 10 ]; then
  printf "  ${GREEN}PASS${NC}  status --timeout 0 resolved fast (${ELAPSED}s)\n"
  ((PASS++))
else
  printf "  ${RED}FAIL${NC}  status --timeout 0 took ${ELAPSED}s (expected <10s)\n"
  ((FAIL++))
fi

# Session must still be usable after a connect()-level timeout — it only
# bounded that one call, it didn't tear anything down.
OUTPUT=$("$KHORA" status "$SESSION" 2>&1)
assert_contains "session still alive after connect timeout" "$OUTPUT" "alive"

# ── type vs type-keys: focus and blur events ─────────────
#
# Regression for mesa task 450. A page the user has never interacted with has
# `document.hasFocus() === false`, and in that state the JS `el.focus()` that
# `type` performs moves `activeElement` WITHOUT the browser dispatching a
# focus event — so a later `blur()` dispatches nothing either, and anything
# gated on focus/blur (commit-on-blur, validate-on-blur) silently never runs.
# The DOM value and any framework state derived from it still read back
# correctly, so it presents as an app bug; that cost several turns to rule
# out during mesa task 448's QA.
#
# The trap is that this is *stateful*, which is what makes it so confusing:
# any trusted input event (a click, a key, a type-keys) gives the document
# focus, after which `type` DOES fire focus/blur — so the same `type` call
# works or silently doesn't depending on what the session happened to do
# earlier. `navigate` resets `hasFocus` to false, which is what makes the
# no-focus arm below deterministic rather than order-dependent.
#
# `type-keys` drives the real input pipeline, which establishes real focus
# itself, so it fires focus and allows the subsequent blur in either state.
printf "\n${BOLD}▸ type vs type-keys focus/blur${NC}\n"

"$KHORA" navigate "$SESSION" "$FIXTURE" >/dev/null 2>&1
OUTPUT=$("$KHORA" eval "$SESSION" "document.hasFocus()" 2>&1)
assert_contains "fresh navigate leaves document unfocused" "$OUTPUT" "false"

"$KHORA" type "$SESSION" "#focus-input" "abc" >/dev/null 2>&1
"$KHORA" eval "$SESSION" 'document.querySelector("#focus-input").blur(); "ok"' >/dev/null 2>&1
OUTPUT=$("$KHORA" text "$SESSION" "#focus-result" 2>&1)
assert_not_contains "type on an unfocused document fires no focus event" "$OUTPUT" "focus,"
assert_not_contains "type on an unfocused document leaves blur unfired" "$OUTPUT" "blur,"

"$KHORA" type-keys "$SESSION" "#focus-input" "abc" >/dev/null 2>&1
"$KHORA" eval "$SESSION" 'document.querySelector("#focus-input").blur(); "ok"' >/dev/null 2>&1
OUTPUT=$("$KHORA" text "$SESSION" "#focus-result" 2>&1)
assert_contains "type-keys fires a real focus event regardless" "$OUTPUT" "focus,"
assert_contains "type-keys lets the subsequent blur fire" "$OUTPUT" "blur,"

# The blur above is driven by an explicit eval. docs/usage.md and the khora
# skill instead prescribe `key Tab` as the blur step for verifying a
# commit-on-blur field, since that's what a user actually does — so exercise
# that exact recipe rather than only the eval form.
"$KHORA" navigate "$SESSION" "$FIXTURE" >/dev/null 2>&1
"$KHORA" type-keys "$SESSION" "#focus-input" "abc" >/dev/null 2>&1
"$KHORA" key "$SESSION" "Tab" >/dev/null 2>&1
OUTPUT=$("$KHORA" text "$SESSION" "#focus-result" 2>&1)
assert_contains "type-keys then key Tab blurs the field" "$OUTPUT" "blur,"
# ...but Tab also lands focus on the next element, firing its handlers. That
# collateral is exactly what `blur` exists to avoid, so pin it here — the
# assertion below is only meaningful against this one.
assert_contains "key Tab also focuses the next element" "$OUTPUT" "next-focus,"

# ── blur ─────────────────────────────────────────────────
#
# mesa task 465. `blur` fires blur/focusout on the focused element without
# moving focus anywhere, so a commit-on-blur handler runs while the next
# element in the tab order stays untouched.
printf "\n${BOLD}▸ blur${NC}\n"

"$KHORA" navigate "$SESSION" "$FIXTURE" >/dev/null 2>&1
"$KHORA" type-keys "$SESSION" "#focus-input" "abc" >/dev/null 2>&1
OUTPUT=$("$KHORA" blur "$SESSION" 2>&1)
assert_contains "blur reports the element it blurred" "$OUTPUT" "input#focus-input"
OUTPUT=$("$KHORA" text "$SESSION" "#focus-result" 2>&1)
assert_contains "blur fires a real blur event" "$OUTPUT" "blur,"
assert_not_contains "blur does not focus the next element" "$OUTPUT" "next-focus,"

# An explicit selector blurs that element, provided it is the focused one.
"$KHORA" navigate "$SESSION" "$FIXTURE" >/dev/null 2>&1
"$KHORA" type-keys "$SESSION" "#focus-input" "abc" >/dev/null 2>&1
OUTPUT=$("$KHORA" blur "$SESSION" "#focus-input" 2>&1)
assert_contains "blur accepts an explicit selector" "$OUTPUT" "input#focus-input"
OUTPUT=$("$KHORA" text "$SESSION" "#focus-result" 2>&1)
assert_contains "blur by selector fires a real blur event" "$OUTPUT" "blur,"

# Blurring a non-focused element dispatches nothing, so it errors rather than
# reporting a success the caller would read as "the commit fired".
"$KHORA" navigate "$SESSION" "$FIXTURE" >/dev/null 2>&1
"$KHORA" type-keys "$SESSION" "#focus-input" "abc" >/dev/null 2>&1
OUTPUT=$("$KHORA" blur "$SESSION" "#focus-next" 2>&1 || true)
assert_contains "blur errors on a non-focused element" "$OUTPUT" "nothing to blur"

# Nothing focused at all is the same failure, not a silent no-op.
"$KHORA" navigate "$SESSION" "$FIXTURE" >/dev/null 2>&1
OUTPUT=$("$KHORA" blur "$SESSION" 2>&1 || true)
assert_contains "blur errors when nothing is focused" "$OUTPUT" "no element is focused"

# A selector matching nothing is an element-not-found, distinct from the above.
"$KHORA" navigate "$SESSION" "$FIXTURE" >/dev/null 2>&1
"$KHORA" type-keys "$SESSION" "#focus-input" "abc" >/dev/null 2>&1
OUTPUT=$("$KHORA" blur "$SESSION" "#no-such-element" 2>&1 || true)
assert_contains "blur errors on an unmatched selector" "$OUTPUT" "element not found"

OUTPUT=$("$KHORA" blur "$SESSION" --format json 2>&1 || true)
assert_contains "blur json reports the action" "$OUTPUT" '"action": "blur"'


# ── kill ─────────────────────────────────────────────────

printf "\n${BOLD}▸ kill${NC}\n"
KILLED_SESSION="$SESSION"
OUTPUT=$("$KHORA" kill "$SESSION" 2>&1)
EC=$?
assert_exit "kill exits 0" "$EC" 0
SESSION=""  # prevent double-kill in cleanup

# Verify session is gone
OUTPUT=$("$KHORA" status "$KILLED_SESSION" 2>&1 || true)
assert_contains "session gone after kill" "$OUTPUT" "not found"

# Verify the underlying Chrome process actually died, not just the session file
if [[ -n "$CHROME_PID" && "$CHROME_PID" != "0" ]]; then
  assert_process_gone "Chrome process $CHROME_PID exited after kill" "$CHROME_PID"
fi

# Regression for mesa task 386: Kill's connect()-error handling used to
# assume any failure meant Chrome was already gone and skipped signaling
# the PID. Now that connect() can also fail with Timeout (Chrome alive but
# unresponsive, not confirmed dead), treating it the same way would trade
# the old 30+min hang for a *silent* Chrome process leak — worse, since it
# reports success. --timeout 0 deterministically forces that Timeout arm
# (status --timeout 0 above proves it; --timeout 1 is a race the real
# connect() sometimes wins on fast hardware), so drive it directly and
# confirm the PID is actually signaled rather than left running.
printf "\n${BOLD}▸ kill --timeout 0 (Timeout arm doesn't leak the process)${NC}\n"
LEAK_OUTPUT=$("$KHORA" launch 2>&1)
LEAK_SESSION=$(echo "$LEAK_OUTPUT" | grep -oE 'Session: [a-f0-9]+' | head -1 | awk '{print $2}')
LEAK_PID=$(echo "$LEAK_OUTPUT" | grep -oE 'PID: [0-9]+' | head -1 | awk '{print $2}')
if [[ -z "$LEAK_SESSION" || -z "$LEAK_PID" || "$LEAK_PID" == "0" ]]; then
  printf "  ${RED}FAIL${NC}  could not launch throwaway session for kill-timeout leak regression\n"
  ((FAIL++))
else
  OUTPUT=$("$KHORA" --timeout 0 kill "$LEAK_SESSION" 2>&1)
  EC=$?
  assert_exit "kill --timeout 0 exits 0" "$EC" 0
  # kill_process() itself already waits out its SIGTERM/SIGKILL grace period
  # before the CLI returns; this is just headroom for scheduling jitter.
  sleep 1
  assert_process_gone "kill --timeout 0 still terminates Chrome (no leak)" "$LEAK_PID"
  # Safety net: don't leave a leaked Chrome running if the assertion above failed.
  kill -9 "$LEAK_PID" 2>/dev/null || true
fi

# ── summary ──────────────────────────────────────────────

printf "\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n"
TOTAL=$((PASS + FAIL))
printf "  ${GREEN}%d passed${NC}  ${RED}%d failed${NC}  %d total\n" "$PASS" "$FAIL" "$TOTAL"
printf "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n"

if [[ "$FAIL" -gt 0 ]]; then
  exit 1
fi
