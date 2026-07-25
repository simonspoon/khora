#!/usr/bin/env bash
# Soak harness for the intermittent trusted-`key` wedge (mesa task 551).
#
# Reported symptom, ~1 in 4 runs of qa.sh on macOS: `key Cmd+D` exits 0, but
# the page's keydown listener reports `key:Unidentified` with an EMPTY `code`
# (modifier + trusted flags correct), and from that point every subsequent
# khora command in the session times out at the 5000ms default.
#
# An isolated 60-dispatch loop against a freshly-navigated page did NOT
# reproduce it, so this harness replays qa.sh's full mutating preamble
# (navigate x2, type, click, drag x2, step-wise mouse-down/move/screenshot/up
# x2, click-at, dblclick-at, wheel) before the first key dispatch — the state
# the real failure was observed under — then dispatches repeatedly.
#
# It separates the two halves of the symptom, because they may not share a
# cause:
#   BAD-KEY  the page saw a keydown whose key/code are not what khora sent
#   WEDGE    a command after the key dispatch took >= WEDGE_MS
#
# ROOT CAUSE (found with this harness, fixed in resolve_page): when Chrome's
# initial Target.targetCreated had not arrived yet, pages() came back empty and
# khora created its own tab via Target.createTarget, then drove that while
# Chrome's startup tab — which resolves to chrome://newtab/ — stayed the active
# one. Trusted key dispatch into the resulting background tab keeps its modifier
# bits but loses key/code. Hence the `pages=N ntp=N` column: every BAD-KEY
# observed had ntp=1, every clean run had ntp=0 (17/17 vs 0/58).
#
# To reproduce deterministically rather than at ~7% per session, widen the race
# by cutting the post-fetch_targets() guard in CdpClient::launch() to zero:
#   tokio::time::sleep(Duration::from_millis(0)).await;
# With that and the resolve_page fix reverted: 15/15 double-tab, 13/15 BAD-KEY.
# With the fix in place: 0/15 and 0/15.
#
# NOT explained by the above: the WEDGE half of the original report (every
# later command timing out at 5000ms). It never reproduced here — 0 wedges in
# 272 dispatches across 55 sessions, before or after the fix — so the ticket's
# assumption that both halves share one cause is unconfirmed. The WEDGE arm of
# this harness is kept precisely because that half is still unaccounted for.
#
# On either, it dumps the raw CDP params khora sent (RUST_LOG=debug), what
# the page actually observed, per-command timings, and browser-level state
# (/json/version + /json/list over the DevTools HTTP port, which does not go
# through the wedged page target), then leaves that Chrome alive for manual
# probing if --keep is passed.
#
# Usage: ./tests/e2e/key-wedge-soak.sh [ITERATIONS] [--keep]
#   ITERATIONS  sessions to run (default 12)
#   --keep      do not kill Chrome after a failing iteration
#   LOAD=N      run N CPU spinners alongside (default 0). The original report
#               came from a busy machine running an agent session; a
#               frame-timing flake is exactly the kind that only appears when
#               the compositor is competing for cores, so this makes that
#               variable settable instead of assumed.

set -uo pipefail

ITERATIONS="${1:-12}"
[[ "$ITERATIONS" =~ ^[0-9]+$ ]] || ITERATIONS=12
KEEP=0
for arg in "$@"; do [[ "$arg" == "--keep" ]] && KEEP=1; done

# Key dispatches per session after the preamble. The reported failure was on
# the first one; the rest are free samples off an expensive preamble.
KEYS_PER_SESSION="${KEYS_PER_SESSION:-8}"
# A command slower than this counts as wedged. khora's default timeout is
# 5000ms, so a wedged command exits non-zero at ~5s; 4000 catches it plus the
# ~5s hit-test tax that does not quite time out.
WEDGE_MS="${WEDGE_MS:-4000}"

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[0;33m'
BOLD='\033[1m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
FIXTURE="file://${SCRIPT_DIR}/fixture.html"
KHORA="${ROOT_DIR}/target/debug/khora"
OUTDIR="${OUTDIR:-/tmp/khora-key-wedge-$$}"
SESSION=""
LOAD="${LOAD:-0}"
LOAD_PIDS=()

BAD_KEY=0
WEDGES=0
DISPATCHES=0

mkdir -p "$OUTDIR"

cleanup() {
  if [[ -n "$SESSION" && "$KEEP" -eq 0 ]]; then
    "$KHORA" kill "$SESSION" >/dev/null 2>&1 || true
  fi
  # Spinners are backgrounded from this same shell, so they are killable by
  # PID here; they would otherwise outlive the run and peg the machine.
  for p in ${LOAD_PIDS+"${LOAD_PIDS[@]}"}; do kill "$p" 2>/dev/null; done
}
trap cleanup EXIT

for ((l = 0; l < LOAD; l++)); do
  yes > /dev/null 2>&1 &
  LOAD_PIDS+=($!)
done

if [[ ! -x "$KHORA" ]]; then
  printf "${RED}no khora binary at %s — run: cargo build${NC}\n" "$KHORA"
  exit 1
fi

# Summarise the browser's open targets over the DevTools HTTP port. Reported
# per dispatch because a stray extra page target turned out to be the state
# that distinguishes a corrupted key event from a clean one, and knowing
# whether it predates the dispatch or is created by it is the whole question.
target_summary() {
  local port targets
  port=$(grep -oE '127\.0\.0\.1:[0-9]+' "$HOME/.khora/sessions/${SESSION}.json" 2>/dev/null | head -1 | cut -d: -f2)
  [[ -z "$port" ]] && { echo "pages=? ntp=?"; return; }
  targets=$(curl -s -m 5 "http://127.0.0.1:${port}/json/list" 2>/dev/null)
  printf "pages=%s ntp=%s" \
    "$(echo "$targets" | grep -c '"type": "page"')" \
    "$(echo "$targets" | grep -c 'chrome://newtab')"
}

# Run a khora command, recording wall-clock ms and exit code.
# Sets: T_MS, T_EC, T_OUT
run_timed() {
  local start end
  start=$(python3 -c 'import time;print(int(time.time()*1000))')
  T_OUT=$("$KHORA" "$@" 2>&1)
  T_EC=$?
  end=$(python3 -c 'import time;print(int(time.time()*1000))')
  T_MS=$((end - start))
}

# Everything qa.sh does to the session before its `key` block. Kept in the
# same order and with the same arguments — the point of this harness is that
# the wedge did not reproduce without this accumulated state.
replay_preamble() {
  "$KHORA" status "$SESSION" >/dev/null 2>&1
  "$KHORA" mouse-move "$SESSION" "10,10" >/dev/null 2>&1
  "$KHORA" navigate "$SESSION" "$FIXTURE" >/dev/null 2>&1
  sleep 0.3
  "$KHORA" navigate "$SESSION" "$FIXTURE" --no-cache >/dev/null 2>&1
  sleep 0.3
  "$KHORA" find "$SESSION" "#heading" >/dev/null 2>&1
  "$KHORA" find "$SESSION" ".item" >/dev/null 2>&1
  "$KHORA" text "$SESSION" "#greeting" >/dev/null 2>&1
  "$KHORA" attribute "$SESSION" "#greeting" "data-testid" >/dev/null 2>&1
  "$KHORA" type "$SESSION" "#name-input" "khora-test" >/dev/null 2>&1
  "$KHORA" type "$SESSION" "#react-input" "khora-react" >/dev/null 2>&1
  "$KHORA" click "$SESSION" "#counter-btn" >/dev/null 2>&1

  local points from to mid
  points=$("$KHORA" eval "$SESSION" "var r=document.getElementById('drag-zone').getBoundingClientRect(); Math.round(r.x+10)+','+Math.round(r.y+10)+' '+Math.round(r.x+250)+','+Math.round(r.y+80)" 2>&1)
  from="${points% *}"; to="${points#* }"
  "$KHORA" drag "$SESSION" "$from" "$to" --steps 8 >/dev/null 2>&1

  points=$("$KHORA" eval "$SESSION" "var r=document.getElementById('splitter').getBoundingClientRect(); Math.round(r.x+10)+','+Math.round(r.y+20)+' '+Math.round(r.x+250)+','+Math.round(r.y+20)" 2>&1)
  from="${points% *}"; to="${points#* }"
  "$KHORA" drag "$SESSION" "$from" "$to" --steps 6 >/dev/null 2>&1

  # Step-wise gesture with a screenshot mid-flight — a real compositor frame
  # lands in the middle of a held mouse button here, which is exactly the
  # kind of state the compositor-frame fix interacts with.
  points=$("$KHORA" eval "$SESSION" "var r=document.getElementById('drag-zone').getBoundingClientRect(); Math.round(r.x+10)+','+Math.round(r.y+10)+' '+Math.round(r.x+150)+','+Math.round(r.y+50)+' '+Math.round(r.x+250)+','+Math.round(r.y+80)" 2>&1)
  from=$(echo "$points" | cut -d' ' -f1)
  mid=$(echo "$points" | cut -d' ' -f2)
  to=$(echo "$points" | cut -d' ' -f3)
  "$KHORA" mouse-down "$SESSION" "$from" >/dev/null 2>&1
  "$KHORA" mouse-move "$SESSION" "$mid" >/dev/null 2>&1
  "$KHORA" screenshot "$SESSION" -o "$OUTDIR/midshot.png" >/dev/null 2>&1
  "$KHORA" mouse-up "$SESSION" "$to" >/dev/null 2>&1

  points=$("$KHORA" eval "$SESSION" "var r=document.getElementById('splitter').getBoundingClientRect(); Math.round(r.x+10)+','+Math.round(r.y+20)+' '+Math.round(r.x+250)+','+Math.round(r.y+20)" 2>&1)
  from="${points% *}"; to="${points#* }"
  "$KHORA" mouse-down "$SESSION" "$from" >/dev/null 2>&1
  "$KHORA" mouse-move "$SESSION" "$to" >/dev/null 2>&1
  "$KHORA" mouse-up "$SESSION" "$to" >/dev/null 2>&1

  local point
  point=$("$KHORA" eval "$SESSION" "var r=document.getElementById('point-target').getBoundingClientRect(); Math.round(r.x+r.width/2)+','+Math.round(r.y+r.height/2)" 2>&1)
  "$KHORA" click-at "$SESSION" "$point" >/dev/null 2>&1
  "$KHORA" dblclick-at "$SESSION" "$point" >/dev/null 2>&1

  # The wheel immediately before the key block: a 1000px delta into a
  # container with only ~540px of scroll, so a smooth-scroll animation may
  # still be settling when the key dispatch lands.
  point=$("$KHORA" eval "$SESSION" "var r=document.getElementById('scroll-inner').getBoundingClientRect(); Math.round(r.x+r.width/2)+','+Math.round(r.y+r.height/2)" 2>&1)
  WHEEL_POINT="$point"
  "$KHORA" wheel "$SESSION" "$point" "0,1000" >/dev/null 2>&1
}

# Dump everything knowable about a failing session, including browser-level
# state fetched over plain HTTP so it does not depend on the page target.
dump_failure() {
  local iter="$1" kind="$2" detail="$3"
  local dir="$OUTDIR/fail-${iter}-${kind}"
  mkdir -p "$dir"

  printf "%s\n" "$detail" > "$dir/detail.txt"
  cp "$OUTDIR/key-dispatch.log" "$dir/key-dispatch.log" 2>/dev/null

  local sess_file="$HOME/.khora/sessions/${SESSION}.json"
  cp "$sess_file" "$dir/session.json" 2>/dev/null

  local port
  port=$(grep -oE '127\.0\.0\.1:[0-9]+' "$sess_file" 2>/dev/null | head -1 | cut -d: -f2)
  if [[ -n "$port" ]]; then
    # Exit codes go to a sidecar file: appending them to the payload would
    # leave json-list.txt unparseable by any JSON reader.
    curl -s -m 5 "http://127.0.0.1:${port}/json/version" > "$dir/json-version.json" 2>&1
    echo "/json/version curl exit: $?" > "$dir/curl-exit.txt"
    curl -s -m 5 "http://127.0.0.1:${port}/json/list" > "$dir/json-list.json" 2>&1
    echo "/json/list curl exit: $?" >> "$dir/curl-exit.txt"
  fi

  local pid
  pid=$(grep -oE '"pid":[[:space:]]*[0-9]+' "$sess_file" 2>/dev/null | grep -oE '[0-9]+')
  if [[ -n "$pid" ]]; then
    ps -o pid,stat,%cpu,rss,etime -p "$pid" > "$dir/chrome-ps.txt" 2>&1
    # Every renderer/GPU child too — a wedge in one of those looks identical
    # from the CLI but is a different bug.
    pgrep -P "$pid" | while read -r child; do
      ps -o pid,stat,%cpu,rss,command -p "$child" >> "$dir/chrome-ps.txt" 2>&1
    done
  fi

  # Probe each layer separately: browser-level (version) responds even when a
  # page target is stuck, so this discriminates "renderer wedged" from
  # "whole browser wedged".
  run_timed status "$SESSION"
  printf "status: %sms ec=%s\n%s\n" "$T_MS" "$T_EC" "$T_OUT" > "$dir/probe-status.txt"
  run_timed eval "$SESSION" "1+1"
  printf "eval: %sms ec=%s\n%s\n" "$T_MS" "$T_EC" "$T_OUT" > "$dir/probe-eval.txt"
  run_timed text "$SESSION" "#key-result"
  printf "text: %sms ec=%s\n%s\n" "$T_MS" "$T_EC" "$T_OUT" > "$dir/probe-text.txt"

  printf "       ${YELLOW}dumped to %s${NC}\n" "$dir"
}

printf "${BOLD}khora key-wedge soak${NC}  iterations=%s keys/session=%s wedge-threshold=%sms load=%s\n" \
  "$ITERATIONS" "$KEYS_PER_SESSION" "$WEDGE_MS" "$LOAD"
printf "output: %s\n\n" "$OUTDIR"

for ((i = 1; i <= ITERATIONS; i++)); do
  printf "${BOLD}[%s/%s]${NC} " "$i" "$ITERATIONS"

  OUTPUT=$("$KHORA" launch 2>&1)
  SESSION=$(echo "$OUTPUT" | grep -oE 'Session: [a-f0-9]+' | head -1 | awk '{print $2}')
  if [[ -z "$SESSION" ]]; then
    printf "${RED}launch failed${NC}: %s\n" "$OUTPUT"
    continue
  fi

  replay_preamble
  iter_bad=0
  PRE_TARGETS=$(target_summary)
  printf "%s " "$PRE_TARGETS"

  for ((k = 1; k <= KEYS_PER_SESSION; k++)); do
    # Clear the recorder so a stale value from the previous dispatch can
    # never be read as this dispatch's result.
    "$KHORA" eval "$SESSION" "document.getElementById('key-result').textContent=''" >/dev/null 2>&1

    # RUST_LOG=debug makes key_press log the exact serialized
    # Input.dispatchKeyEvent params it handed to Chrome, so a bad observed
    # event can be attributed to sender or receiver rather than guessed at.
    start=$(python3 -c 'import time;print(int(time.time()*1000))')
    KEY_OUT=$(RUST_LOG=khora_cdp=debug "$KHORA" key "$SESSION" "Cmd+D" 2>"$OUTDIR/key-dispatch.log")
    KEY_EC=$?
    end=$(python3 -c 'import time;print(int(time.time()*1000))')
    KEY_MS=$((end - start))
    ((DISPATCHES++))

    run_timed text "$SESSION" "#key-result"
    OBSERVED="$T_OUT"

    POST_TARGETS=$(target_summary)

    if ! echo "$OBSERVED" | grep -qF "key:d code:KeyD"; then
      ((BAD_KEY++)); iter_bad=1
      printf "\n  ${RED}BAD-KEY${NC} dispatch %s: exit=%s in %sms\n    before-preamble-end: %s\n    after-dispatch:      %s\n    page saw: %s\n" \
        "$k" "$KEY_EC" "$KEY_MS" "$PRE_TARGETS" "$POST_TARGETS" "$OBSERVED"
      dump_failure "$i" "badkey-$k" \
        "key exit=$KEY_EC ms=$KEY_MS observed=$OBSERVED pre=[$PRE_TARGETS] post=[$POST_TARGETS]"
      break
    fi

    # The wedge half: a command issued after the key dispatch that should be
    # fast. wheel is the dispatch type with no frame-ack fallback, so it is
    # the most sensitive probe available (see the 1f332ae regression note).
    run_timed wheel "$SESSION" "$WHEEL_POINT" "0,10"
    if [[ "$T_MS" -ge "$WEDGE_MS" || "$T_EC" -ne 0 ]]; then
      ((WEDGES++)); iter_bad=1
      printf "\n  ${RED}WEDGE${NC} after dispatch %s: wheel took %sms exit=%s\n" "$k" "$T_MS" "$T_EC"
      dump_failure "$i" "wedge-$k" "post-key wheel ms=$T_MS ec=$T_EC; key exit=$KEY_EC ms=$KEY_MS observed=$OBSERVED"
      break
    fi
  done

  if [[ "$iter_bad" -eq 0 ]]; then
    printf "${GREEN}ok${NC} (%s dispatches clean)\n" "$KEYS_PER_SESSION"
  fi

  if [[ "$iter_bad" -eq 1 && "$KEEP" -eq 1 ]]; then
    printf "  ${YELLOW}--keep: leaving session %s alive${NC}\n" "$SESSION"
    SESSION=""
    break
  fi

  "$KHORA" kill "$SESSION" >/dev/null 2>&1
  SESSION=""
done

printf "\n${BOLD}── summary ──${NC}\n"
printf "  dispatches:  %s\n" "$DISPATCHES"
printf "  bad-key:     %s\n" "$BAD_KEY"
printf "  wedges:      %s\n" "$WEDGES"
printf "  artifacts:   %s\n" "$OUTDIR"

if [[ "$BAD_KEY" -eq 0 && "$WEDGES" -eq 0 ]]; then
  printf "  ${GREEN}no reproduction in %s dispatches${NC}\n" "$DISPATCHES"
  exit 0
fi
exit 1
