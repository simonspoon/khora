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
  if echo "$actual" | grep -qF "$expected"; then
    printf "  ${GREEN}PASS${NC}  %s\n" "$label"
    ((PASS++))
  else
    printf "  ${RED}FAIL${NC}  %s\n" "$label"
    printf "       expected to contain: %s\n" "$expected"
    printf "       got: %s\n" "$actual"
    ((FAIL++))
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

# ── navigate ─────────────────────────────────────────────

printf "\n${BOLD}▸ navigate${NC}\n"
OUTPUT=$("$KHORA" navigate "$SESSION" "$FIXTURE" 2>&1)
EC=$?
assert_exit "navigate exits 0" "$EC" 0
# Small settle time for page scripts to initialize
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

# ── click ────────────────────────────────────────────────

printf "\n${BOLD}▸ click${NC}\n"
OUTPUT=$("$KHORA" click "$SESSION" "#counter-btn" 2>&1)
EC=$?
assert_exit "click exits 0" "$EC" 0

OUTPUT=$("$KHORA" text "$SESSION" "#counter-btn" 2>&1)
assert_contains "click updated button text" "$OUTPUT" "Clicked 1"

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

# ── JSON output format ──────────────────────────────────

printf "\n${BOLD}▸ JSON format${NC}\n"
OUTPUT=$("$KHORA" -f json text "$SESSION" "#heading" 2>&1)
assert_contains "json text has bracket" "$OUTPUT" "["
assert_contains "json text has content" "$OUTPUT" "Khora Test Page"

OUTPUT=$("$KHORA" -f json status "$SESSION" 2>&1)
assert_contains "json status has brace" "$OUTPUT" "{"
assert_contains "json status has alive" "$OUTPUT" "alive"

OUTPUT=$("$KHORA" -f json find "$SESSION" "#greeting" 2>&1)
assert_contains "json find has bracket" "$OUTPUT" "["

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

# ── summary ──────────────────────────────────────────────

printf "\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n"
TOTAL=$((PASS + FAIL))
printf "  ${GREEN}%d passed${NC}  ${RED}%d failed${NC}  %d total\n" "$PASS" "$FAIL" "$TOTAL"
printf "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n"

if [[ "$FAIL" -gt 0 ]]; then
  exit 1
fi
