#!/usr/bin/env bash
# Hardware test of the text clipboard: AC-30 to AC-35, CLIP-6, CLIP-9.
#
# The test uses no click and no key. It uses the debug protocol
# (docs/debug-protocol.md) to start and stop sessions, and other programs
# (pbcopy, wl-copy, clip_copy) to copy, like a user.
#
# Run it on the Mac. The Linux peer is reached over SSH (BatchMode).
#
# Before the test:
# - Run both apps with the debug protocol and the same config. Use the
#   computer ids "mac" and "linux" (or set TS_MAC_ID and TS_LINUX_ID).
# - Build the example on Linux, in TS_LINUX_BUILD:
#   cargo build --release -p telescreen-platform-linux --example clip_copy
# - Accept that the test overwrites the clipboard of both computers, and
#   that each handoff runs the handoff steps of the config (the shared display
#   can switch its input).
#
# Environment:
#   TS_LINUX_HOST   SSH destination of the Linux peer (required)
#   TS_MAC_SOCK     debug socket of the Mac app (required)
#   TS_LINUX_SOCK   debug socket of the Linux app (default /run/user/<uid>/ts-debug.sock)
#   TS_MAC_LOG      log of the Mac app (default ~/Library/Logs/telescreen-mac.log)
#   TS_LINUX_LOG    log of the Linux app (default ~/.cache/telescreen-run.log)
#   TS_LINUX_BUILD  build directory on Linux (default ~/.cache/telescreen-build)
#   TS_WAYLAND      WAYLAND_DISPLAY on Linux (default wayland-1)
#   TS_DEVICE       device alias for SelectDevice, if a portal asks (default LG)
#
# Usage: scripts/hw-clipboard-test.sh [all | m2l | l2m | dbg mac|linux <json>]
#   all  runs the Mac to Linux part, then the Linux to Mac part (default).
#   m2l  runs only the Mac to Linux part (sessions S1 to S3).
#   l2m  runs only the Linux to Mac part (session S4).
# The exit status is 0 only if all checks pass.

set -u

: "${TS_LINUX_HOST:?set TS_LINUX_HOST to the SSH destination of the Linux peer}"
: "${TS_MAC_SOCK:?set TS_MAC_SOCK to the debug socket of the Mac app}"
TS_LINUX_SOCK=${TS_LINUX_SOCK:-'/run/user/$(id -u)/ts-debug.sock'}
TS_MAC_LOG=${TS_MAC_LOG:-$HOME/Library/Logs/telescreen-mac.log}
TS_LINUX_LOG=${TS_LINUX_LOG:-'$HOME/.cache/telescreen-run.log'}
TS_LINUX_BUILD=${TS_LINUX_BUILD:-'$HOME/.cache/telescreen-build'}
TS_WAYLAND=${TS_WAYLAND:-wayland-1}
TS_DEVICE=${TS_DEVICE:-LG}
TS_MAC_ID=${TS_MAC_ID:-mac}
TS_LINUX_ID=${TS_LINUX_ID:-linux}

LENV="export XDG_RUNTIME_DIR=/run/user/\$(id -u) WAYLAND_DISPLAY=$TS_WAYLAND;"
NONCE="tsclip-$(date +%s)-$$"
SETTLE=1.5      # wait for a negative check; the app tick is 100 ms
TIMEOUT=5       # max wait for a positive check, in seconds
CONCEALED_TYPE=org.nspasteboard.ConcealedType
EMPTY='<empty>'

PASSES=0
FAILS=0
RESULTS=()
CLIP_PIDS=()

say() { printf '%s\n' "$*" >&2; }

# ---- debug protocol -------------------------------------------------------

dbg_mac() { printf '%s\n' "$1" | nc -U "$TS_MAC_SOCK"; }
dbg_linux() { printf '%s\n' "$1" | ssh -o BatchMode=yes "$TS_LINUX_HOST" "socat - UNIX-CONNECT:$TS_LINUX_SOCK"; }
dbg() { if [ "$1" = mac ]; then dbg_mac "$2"; else dbg_linux "$2"; fi; }
intent() { dbg "$1" "{\"cmd\":\"intent\",\"intent\":$2}" >/dev/null; }

# Counts the "too large" notices on the control screen and the portal (CLIP-6).
notice_count() {
  { state "$1" control; state "$1" portal; } | grep -o 'too large' | wc -l | tr -d ' '
}

# Prints one field of the state: control, portal, or windows.
state() {
  dbg "$1" '{"cmd":"state"}' | python3 -c '
import json, sys
d = json.load(sys.stdin)
print(json.dumps(d[sys.argv[1]]) if sys.argv[1] == "windows" else d[sys.argv[1]])
' "$2"
}

# Waits until the state field contains the pattern. Returns 1 at the timeout.
wait_state() {
  local peer=$1 field=$2 pattern=$3 limit=$4 i=0
  while [ "$i" -lt "$((limit * 4))" ]; do
    case "$(state "$peer" "$field")" in *"$pattern"*) return 0 ;; esac
    sleep 0.25
    i=$((i + 1))
  done
  return 1
}

# Activates the running Mac app (no click, no key).
activate_mac() {
  local app
  app=$(ps -axo command= | sed -n 's#^\(.*/Telescreen\.app\)/Contents/MacOS/.*#\1#p' | head -n 1)
  if [ -n "$app" ]; then open -a "$app"; sleep 1; else say "the Mac app is not running"; fi
}

portal_focused() {
  local peer=$1 limit=$2 i=0
  while [ "$i" -lt "$((limit * 4))" ]; do
    if state "$peer" windows | python3 -c '
import json, sys
w = json.load(sys.stdin)
sys.exit(0 if any(x["role"] == "portal" and x["focused"] for x in w) else 1)
'; then return 0; fi
    sleep 0.25
    i=$((i + 1))
  done
  return 1
}

other() { if [ "$1" = mac ]; then echo linux; else echo mac; fi; }
peer_id() { if [ "$1" = mac ]; then echo "$TS_MAC_ID"; else echo "$TS_LINUX_ID"; fi; }

# Starts a session from the source peer and waits for forwarding.
# On a failure, ends the session and closes the portal.
start_session() {
  if try_session "$1"; then return 0; fi
  intent "$1" '"EndSession"'
  intent "$1" '"ClosePortal"'
  return 1
}

try_session() {
  local src=$1 tgt
  tgt=$(other "$src")
  intent "$src" '"ClosePortal"'
  sleep 0.5
  intent "$src" '"OpenPortal"'
  sleep 1
  case "$(state "$src" portal)" in
    *"matching: Matched("*) ;;
    *) intent "$src" "{\"SelectDevice\":{\"alias\":\"$TS_DEVICE\"}}"; sleep 1 ;;
  esac
  intent "$src" "{\"StartHandoff\":{\"target\":\"$(peer_id "$tgt")\"}}"
  if ! wait_state "$src" portal 'WaitingForConfirmation' 40; then
    say "session $src->$tgt: no prompt; portal: $(state "$src" portal)"
    return 1
  fi
  # The confirmation needs the focused portal (AC-6). OpenPortal does not
  # activate the Mac app when another app is in front. Without a click,
  # activate the running app with `open -a`.
  if [ "$src" = mac ] && ! portal_focused mac 3; then
    say "the Mac portal is not focused: activate the Mac app"
    activate_mac
  fi
  if ! portal_focused "$src" 10; then
    say "session $src->$tgt: the portal is not focused; windows: $(state "$src" windows)"
    return 1
  fi
  say "confirm on $src: $(dbg "$src" '{"cmd":"confirm"}')"
  if ! wait_state "$src" portal 'phase: Forwarding' 10; then
    say "session $src->$tgt: no forwarding; portal: $(state "$src" portal)"
    return 1
  fi
  sleep 1
  say "session $src->$tgt: forwarding"
}

end_on_source() {
  intent "$1" '"EndSession"'
  wait_state "$1" portal 'phase: Ended' 10 || say "end: portal: $(state "$1" portal)"
  sleep 1
}

# Stops the incoming session on the target peer (StopSession).
stop_on_target() {
  local id
  id=$(state "$1" control | sed -n 's/.*IncomingRow { session: SessionId(\([0-9]*\)).*/\1/p')
  if [ -z "$id" ]; then
    say "stop: no incoming session on $1"
    return 1
  fi
  intent "$1" "{\"StopSession\":{\"session\":$id}}"
  sleep 1.5
}

set_switch() { intent "$1" "{\"SetClipboard\":{\"on\":$2}}"; sleep 0.5; }

# ---- clipboards -----------------------------------------------------------

copy_mac() { printf '%s' "$1" | pbcopy; }

copy_mac_concealed() {
  osascript -l JavaScript -e '
ObjC.import("AppKit");
function run(argv) {
  var pb = $.NSPasteboard.generalPasteboard;
  pb.clearContents;
  pb.setStringForType($(argv[0]), $.NSPasteboardTypeString);
  pb.setStringForType($(""), $("org.nspasteboard.ConcealedType"));
  return "";
}' "$1" >/dev/null
}

clear_mac() {
  osascript -l JavaScript -e 'ObjC.import("AppKit"); $.NSPasteboard.generalPasteboard.clearContents; ""' >/dev/null
}

paste_mac() {
  local text
  text=$(pbpaste)
  if [ -z "$text" ]; then echo "$EMPTY"; else printf '%s' "$text"; fi
}

types_mac() {
  osascript -l JavaScript -e '
ObjC.import("AppKit");
var t = $.NSPasteboard.generalPasteboard.types;
var out = [];
if (t) { for (var i = 0; i < t.count; i++) out.push(ObjC.unwrap(t.objectAtIndex(i))); }
out.join(",")'
}

count_mac() {
  osascript -l JavaScript -e 'ObjC.import("AppKit"); String($.NSPasteboard.generalPasteboard.changeCount)'
}

lsh() { ssh -o BatchMode=yes "$TS_LINUX_HOST" "$LENV $1"; }

copy_linux() { printf '%s' "$1" | lsh 'wl-copy >/dev/null 2>&1'; }

paste_linux() {
  local out rc
  out=$(lsh "wl-paste --no-newline 2>/dev/null || printf '%s' '$EMPTY'")
  rc=$?
  if [ "$rc" -eq 255 ]; then say "ssh failed"; exit 2; fi
  printf '%s' "$out"
}

types_linux() { lsh 'wl-paste --list-types 2>/dev/null | paste -sd, -'; }
hint_linux() { lsh 'wl-paste --type x-kde-passwordManagerHint 2>/dev/null'; }

# Starts clip_copy on Linux (a password manager) and prints its PID.
clip_copy_linux() {
  lsh "cd $TS_LINUX_BUILD && { setsid nohup target/release/examples/clip_copy $1 >/dev/null 2>&1 </dev/null & echo \$!; }"
}

kill_linux() { lsh "kill $1 2>/dev/null"; }

# Counts selection changes on Linux for some seconds, while a command runs.
# The first change is the current selection at the start of the watch.
watch_linux_start() {
  lsh "rm -f /tmp/ts-clip-watch.\$(id -u); { setsid nohup timeout $1 wl-paste --watch sh -c 'echo x >> /tmp/ts-clip-watch.\$(id -u)' >/dev/null 2>&1 </dev/null & }"
  sleep 1
}
watch_linux_count() { lsh 'wc -l < /tmp/ts-clip-watch.$(id -u) 2>/dev/null || echo 0' | tr -d ' '; }

# Waits until the getter prints the expected text. Prints the last value.
wait_for() {
  local getter=$1 want=$2 got i=0
  while :; do
    got=$($getter)
    if [ "$got" = "$want" ] || [ "$i" -ge "$((TIMEOUT * 4))" ]; then break; fi
    sleep 0.25
    i=$((i + 1))
  done
  printf '%s' "$got"
}

# ---- results --------------------------------------------------------------

short() { local s=$1; if [ "${#s}" -gt 80 ]; then printf '%s...(%d bytes)' "${s:0:60}" "${#s}"; else printf '%s' "$s"; fi; }

record() {
  local verdict=$1 id=$2 what=$3 want=$4 got=$5
  if [ "$verdict" = PASS ]; then PASSES=$((PASSES + 1)); else FAILS=$((FAILS + 1)); fi
  RESULTS+=("$verdict  $id  $what")
  say "$verdict  $id  $what"
  if [ "$verdict" = FAIL ]; then
    say "      expected: $(short "$want")"
    say "      actual:   $(short "$got")"
  fi
}

check_eq() { if [ "$4" = "$5" ]; then record PASS "$1" "$2" "$4" "$5"; else record FAIL "$1" "$2" "$4" "$5"; fi; }
check_ne() { if [ "$4" != "$5" ]; then record PASS "$1" "$2" "not $4" "$5"; else record FAIL "$1" "$2" "not $4" "$5"; fi; }
check_has() { case "$5" in *"$4"*) record PASS "$1" "$2" "$4" "$5" ;; *) record FAIL "$1" "$2" "contains $4" "$5" ;; esac; }
check_lacks() { case "$5" in *"$4"*) record FAIL "$1" "$2" "no $4" "$5" ;; *) record PASS "$1" "$2" "no $4" "$5" ;; esac; }

t() { printf '%s-%s' "$NONCE" "$1"; }

cleanup() {
  local pid
  for pid in "${CLIP_PIDS[@]+"${CLIP_PIDS[@]}"}"; do kill_linux "$pid"; done
  CLIP_PIDS=()
}
trap cleanup EXIT

# ---- Mac to Linux ---------------------------------------------------------

part_m2l() {
  local pid c1 n big notices before

  say "== S1: Mac to Linux"
  set_switch mac true
  set_switch linux true
  copy_mac "$(t ac31-mac)"
  copy_linux "$(t ac31-linux)"
  start_session mac || { record FAIL S1 "session Mac to Linux starts" forwarding "no forwarding"; return; }
  sleep "$SETTLE"
  check_eq AC-31 "Mac text from before the session is not sent" "" "$(t ac31-linux)" "$(paste_linux)"
  check_eq AC-31 "Linux text from before the session is not sent" "" "$(t ac31-mac)" "$(paste_mac)"

  copy_mac "$(t ac30-m2l)"
  c1=$(count_mac)
  check_eq AC-30 "copy on the source (Mac) arrives on the target (Linux)" "" "$(t ac30-m2l)" "$(wait_for paste_linux "$(t ac30-m2l)")"
  sleep "$SETTLE"
  check_eq AC-33 "Mac clipboard is not set again after its copy (changeCount)" "" "$c1" "$(count_mac)"
  check_eq AC-33 "Mac clipboard keeps its own copy" "" "$(t ac30-m2l)" "$(paste_mac)"

  watch_linux_start 6
  copy_linux "$(t ac30-l2m)"
  check_eq AC-30 "copy on the target (Linux) arrives on the source (Mac)" "" "$(t ac30-l2m)" "$(wait_for paste_mac "$(t ac30-l2m)")"
  sleep 4
  n=$(watch_linux_count)
  check_eq AC-33 "Linux selection changes only for its own copy (watch: start + copy)" "" "2" "$n"
  check_eq AC-33 "Linux clipboard keeps its own copy" "" "$(t ac30-l2m)" "$(paste_linux)"

  say "-- AC-34 Mac password manager"
  copy_mac_concealed "$(t ac34-mac-pw1)"
  check_eq AC-34 "concealed Mac item arrives on Linux" "" "$(t ac34-mac-pw1)" "$(wait_for paste_linux "$(t ac34-mac-pw1)")"
  check_has AC-34 "Linux item has the concealed mark type" "" "x-kde-passwordManagerHint" "$(types_linux)"
  check_eq AC-34 "Linux mark value is secret" "" "secret" "$(hint_linux)"
  clear_mac
  check_eq AC-34 "Linux clears when the Mac password manager clears" "" "$EMPTY" "$(wait_for paste_linux "$EMPTY")"

  copy_mac_concealed "$(t ac34-mac-pw2)"
  wait_for paste_linux "$(t ac34-mac-pw2)" >/dev/null
  copy_linux "$(t ac34-linux-user)"
  wait_for paste_mac "$(t ac34-linux-user)" >/dev/null
  clear_mac
  sleep "$SETTLE"
  check_eq AC-34 "a Linux user copy after the concealed item stays when the Mac clears" "" "$(t ac34-linux-user)" "$(paste_linux)"
  say "      note: the Mac now holds: $(paste_mac)"

  say "-- AC-34 Linux password manager"
  pid=$(clip_copy_linux "--concealed $(t ac34-linux-pw1)")
  CLIP_PIDS+=("$pid")
  check_eq AC-34 "concealed Linux item arrives on the Mac" "" "$(t ac34-linux-pw1)" "$(wait_for paste_mac "$(t ac34-linux-pw1)")"
  check_has AC-34 "Mac item has $CONCEALED_TYPE" "" "$CONCEALED_TYPE" "$(types_mac)"
  kill_linux "$pid"
  check_eq AC-34 "Mac clears when the Linux password manager clears" "" "$EMPTY" "$(wait_for paste_mac "$EMPTY")"
  check_eq AC-34 "Mac pasteboard has no types after the clear" "" "" "$(types_mac)"

  pid=$(clip_copy_linux "--concealed $(t ac34-linux-pw2)")
  CLIP_PIDS+=("$pid")
  wait_for paste_mac "$(t ac34-linux-pw2)" >/dev/null
  copy_mac "$(t ac34-mac-user)"
  wait_for paste_linux "$(t ac34-mac-user)" >/dev/null
  kill_linux "$pid"
  sleep "$SETTLE"
  check_eq AC-34 "a Mac user copy after the concealed item stays when Linux clears" "" "$(t ac34-mac-user)" "$(paste_mac)"
  say "      note: Linux now holds: $(paste_linux)"
  cleanup

  say "-- AC-32 switch off on the sender"
  set_switch mac false
  copy_mac "$(t ac32-sender-off)"
  sleep "$SETTLE"
  check_ne AC-32 "Mac switch off: the Mac copy is not sent" "" "$(t ac32-sender-off)" "$(paste_linux)"
  set_switch mac true
  sleep "$SETTLE"
  check_ne AC-32 "Mac switch on again: the copy from the off time is not sent" "" "$(t ac32-sender-off)" "$(paste_linux)"

  say "-- AC-32 switch off on the receiver"
  set_switch linux false
  copy_mac "$(t ac32-receiver-off)"
  sleep "$SETTLE"
  check_ne AC-32 "Linux switch off: Linux does not apply the Mac copy" "" "$(t ac32-receiver-off)" "$(paste_linux)"
  set_switch linux true
  sleep "$SETTLE"
  check_ne AC-32 "Linux switch on again: the copy from the off time is not applied" "" "$(t ac32-receiver-off)" "$(paste_linux)"
  copy_mac "$(t ac32-after-on)"
  check_eq AC-32 "after both switches are on again, a new copy syncs" "" "$(t ac32-after-on)" "$(wait_for paste_linux "$(t ac32-after-on)")"

  say "-- AC-35 switch off on the receiver"
  copy_linux "$(t ac35-off-mac-rx)"
  wait_for paste_mac "$(t ac35-off-mac-rx)" >/dev/null
  set_switch mac false
  sleep "$SETTLE"
  check_eq AC-35 "Mac switch off: the Mac clears the received text" "" "$EMPTY" "$(paste_mac)"
  check_eq AC-35 "Mac switch off: the Linux user copy stays" "" "$(t ac35-off-mac-rx)" "$(paste_linux)"
  set_switch mac true
  sleep 1
  copy_mac "$(t ac35-off-linux-rx)"
  wait_for paste_linux "$(t ac35-off-linux-rx)" >/dev/null
  set_switch linux false
  sleep "$SETTLE"
  check_eq AC-35 "Linux switch off: Linux clears the received text" "" "$EMPTY" "$(paste_linux)"
  check_eq AC-35 "Linux switch off: the Mac user copy stays" "" "$(t ac35-off-linux-rx)" "$(paste_mac)"
  set_switch linux true
  sleep 1
  copy_linux "$(t ac35-off-rx-1)"
  wait_for paste_mac "$(t ac35-off-rx-1)" >/dev/null
  copy_mac "$(t ac35-off-user)"
  wait_for paste_linux "$(t ac35-off-user)" >/dev/null
  set_switch mac false
  sleep "$SETTLE"
  check_eq AC-35 "Mac switch off: a later Mac user copy stays" "" "$(t ac35-off-user)" "$(paste_mac)"
  set_switch mac true
  sleep 1

  say "-- CLIP-6 text larger than 256 KiB"
  before=$(paste_linux)
  n=$(notice_count mac)
  big="$(t clip6-mac)-$(head -c 300000 /dev/zero | tr '\0' 'x')"
  copy_mac "$big"
  sleep 3
  check_eq CLIP-6 "Mac text of 300 kB is not sent" "" "$before" "$(paste_linux)"
  check_eq CLIP-6 "Mac shows one more too-large notice" "" "$((n + 1))" "$(notice_count mac)"
  notices="$(state mac control) $(state mac portal)"
  check_lacks CLIP-6 "Mac notice does not contain the text" "" "$NONCE" "$notices"
  before=$(paste_mac)
  n=$(notice_count linux)
  big="$(t clip6-linux)-$(head -c 300000 /dev/zero | tr '\0' 'y')"
  copy_linux "$big"
  sleep 3
  check_eq CLIP-6 "Linux text of 300 kB is not sent" "" "$before" "$(paste_mac)"
  check_eq CLIP-6 "Linux shows one more too-large notice" "" "$((n + 1))" "$(notice_count linux)"
  notices="$(state linux control) $(state linux portal)"
  check_lacks CLIP-6 "Linux notice does not contain the text" "" "$NONCE" "$notices"

  say "-- AC-35 EndSession on the source"
  copy_mac "$(t ac35-end-user-mac)"
  wait_for paste_linux "$(t ac35-end-user-mac)" >/dev/null
  copy_linux "$(t ac35-end-mac-rx)"
  wait_for paste_mac "$(t ac35-end-mac-rx)" >/dev/null
  end_on_source mac
  check_eq AC-35 "EndSession on the Mac: the Mac clears the received text" "" "$EMPTY" "$(paste_mac)"
  check_eq AC-35 "EndSession on the Mac: the Linux user copy stays" "" "$(t ac35-end-mac-rx)" "$(paste_linux)"

  say "-- AC-32 after the session end"
  copy_mac "$(t ac32-after-end-mac)"
  copy_linux "$(t ac32-after-end-linux)"
  sleep "$SETTLE"
  check_eq AC-32 "after the end, the Mac copy is not sent" "" "$(t ac32-after-end-linux)" "$(paste_linux)"
  check_eq AC-32 "after the end, the Linux copy is not sent" "" "$(t ac32-after-end-mac)" "$(paste_mac)"

  say "== S2: Mac to Linux, StopSession on the target, Mac receives"
  start_session mac || { record FAIL S2 "session Mac to Linux starts" forwarding "no forwarding"; return; }
  sleep "$SETTLE"
  check_eq AC-31 "S2: Mac copy from the end time is not sent at the new session" "" "$(t ac32-after-end-linux)" "$(paste_linux)"
  copy_linux "$(t ac35-stop-mac-rx)"
  wait_for paste_mac "$(t ac35-stop-mac-rx)" >/dev/null
  stop_on_target linux
  check_eq AC-35 "StopSession on Linux: the Mac clears the received text" "" "$EMPTY" "$(paste_mac)"
  check_eq AC-35 "StopSession on Linux: the Linux user copy stays" "" "$(t ac35-stop-mac-rx)" "$(paste_linux)"

  say "== S3: Mac to Linux, StopSession on the target, Linux receives"
  start_session mac || { record FAIL S3 "session Mac to Linux starts" forwarding "no forwarding"; return; }
  copy_mac "$(t ac35-stop-linux-rx)"
  wait_for paste_linux "$(t ac35-stop-linux-rx)" >/dev/null
  stop_on_target linux
  check_eq AC-35 "StopSession on Linux: Linux clears the received text" "" "$EMPTY" "$(paste_linux)"
  check_eq AC-35 "StopSession on Linux: the Mac user copy stays" "" "$(t ac35-stop-linux-rx)" "$(paste_mac)"
  intent mac '"ClosePortal"'
}

# ---- Linux to Mac ---------------------------------------------------------

part_l2m() {
  local c1
  say "== S4: Linux to Mac"
  set_switch mac true
  set_switch linux true
  copy_linux "$(t s4-pre-linux)"
  copy_mac "$(t s4-pre-mac)"
  start_session linux || { record FAIL S4 "session Linux to Mac starts" forwarding "no forwarding"; return; }
  sleep "$SETTLE"
  check_eq AC-31 "S4: Linux text from before the session is not sent" "" "$(t s4-pre-mac)" "$(paste_mac)"
  check_eq AC-31 "S4: Mac text from before the session is not sent" "" "$(t s4-pre-linux)" "$(paste_linux)"
  copy_linux "$(t s4-ac30-src)"
  check_eq AC-30 "copy on the source (Linux) arrives on the target (Mac)" "" "$(t s4-ac30-src)" "$(wait_for paste_mac "$(t s4-ac30-src)")"
  copy_mac "$(t s4-ac30-tgt)"
  c1=$(count_mac)
  check_eq AC-30 "copy on the target (Mac) arrives on the source (Linux)" "" "$(t s4-ac30-tgt)" "$(wait_for paste_linux "$(t s4-ac30-tgt)")"
  sleep "$SETTLE"
  check_eq AC-33 "S4: Mac clipboard is not set again after its copy (changeCount)" "" "$c1" "$(count_mac)"
  copy_linux "$(t s4-end-mac-rx)"
  wait_for paste_mac "$(t s4-end-mac-rx)" >/dev/null
  end_on_source linux
  check_eq AC-35 "EndSession on Linux: the Mac clears the received text" "" "$EMPTY" "$(paste_mac)"
  check_eq AC-35 "EndSession on Linux: the Linux user copy stays" "" "$(t s4-end-mac-rx)" "$(paste_linux)"
  intent linux '"ClosePortal"'
}

# ---- CLIP-9 ---------------------------------------------------------------

check_logs() {
  local hits
  hits=$(grep -c -- "$NONCE" "$TS_MAC_LOG" 2>/dev/null)
  check_eq CLIP-9 "Mac log has no test text" "" "0" "${hits:-0}"
  hits=$(lsh "grep -c -- '$NONCE' $TS_LINUX_LOG 2>/dev/null")
  check_eq CLIP-9 "Linux log has no test text" "" "0" "${hits:-0}"
}

# ---- main -----------------------------------------------------------------

case "${1:-all}" in
  dbg) dbg "$2" "$3"; exit ;;
  m2l) part_m2l ;;
  l2m) part_l2m ;;
  all) part_m2l; part_l2m ;;
  *) say "usage: $0 [all | m2l | l2m | dbg mac|linux <json>]"; exit 2 ;;
esac
check_logs
set_switch mac true
set_switch linux true

say ""
say "run $NONCE: $PASSES passed, $FAILS failed"
for line in "${RESULTS[@]}"; do say "  $line"; done
[ "$FAILS" -eq 0 ]
