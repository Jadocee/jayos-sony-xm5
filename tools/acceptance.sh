#!/usr/bin/env bash
#
# The capture session that settles what docs/OUTSTANDING.md leaves open.
#
#   tools/acceptance.sh [--out=DIR]      run it, against a connected WH-1000XM5
#   tools/acceptance.sh --dry-run        say every step, run none of them
#
# Exit 0: every item a capture can settle is settled. 1: some are still open,
# or contradicted -- `qs-sony settle` lists which. 2: could not run -- a tool is
# missing, no WH-1000XM5 is connected, or nothing was captured -- which is not
# the same answer as either and is not reported as one. --dry-run exits 0
# having run nothing.
#
# WHY A SCRIPT. The session is a dozen steps in a fixed order -- the trace must
# be running before the daemon dials, the alert must be answered before the
# reconnect, the reconnect before the read-back -- and docs/CAPTURE.md says
# why each is there. A person following prose skips one; this does not. It
# needs a person anyway: two steps are a physical reconnect and a screenshot,
# and it waits for each.
#
# WHAT IT HAS BEEN RUN AGAINST: nothing but --dry-run, by
# tests/shell/test-acceptance.sh. There is no adapter and no headset where it
# was written. Every command it runs is the package's own or BlueZ's, named
# in docs/CAPTURE.md, and none of them has been run here against a headset.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DRY=0
OUT="${XDG_STATE_HOME:-$HOME/.local/state}/sony-headphones/capture-$(date +%Y%m%d-%H%M%S)"
for arg in "$@"; do
    case "$arg" in
        --dry-run) DRY=1 ;;
        --out=*) OUT="${arg#--out=}" ;;
        *) echo "usage: tools/acceptance.sh [--dry-run] [--out=DIR]" >&2; exit 2 ;;
    esac
done

# The package's binaries, wherever this checkout's PATH finds them. A checkout
# that has not been installed points these at target/release.
QS_SONY="${QS_SONY:-qs-sony}"
QS_SONYD="${QS_SONYD:-qs-sonyd}"
RECORD="$OUT/record"
TRACE="$OUT/trace.btsnoop"
IMPORTED="$OUT/imported"

say() { printf '\n== %s\n' "$*"; }
run() {
    printf '  $ %s\n' "$*"
    [[ $DRY -eq 1 ]] && return 0
    "$@"
}
# A step that needs a person: said, then waited for.
ask() {
    printf '  -> %s\n' "$*"
    [[ $DRY -eq 1 ]] && return 0
    read -r -p "     press Enter when done " _ </dev/tty
}
cannot() { echo "acceptance: cannot run: $*" >&2; exit 2; }

say "what this needs"
for tool in "$QS_SONY" "$QS_SONYD" btmon bluetoothctl cargo jq; do
    if command -v "$tool" >/dev/null 2>&1; then
        printf '  have %s\n' "$tool"
    elif [[ $DRY -eq 1 ]]; then
        printf '  would need %s\n' "$tool"
    else
        cannot "$tool is not on PATH"
    fi
done
if [[ $DRY -eq 0 ]]; then
    # The model, from the inventory qs-sony itself reads. Another Sony headset
    # would produce a capture, and settle would rightly settle nothing with it.
    "$QS_SONY" devices | jq -e '.devices[]? | select(.name | test("WH-1000XM5"))' >/dev/null 2>&1 \
        || cannot "no WH-1000XM5 is connected (qs-sony devices)"
fi

say "1. trace the link from before the daemon dials (items 1-7, 10, 11)"
printf '  in another terminal, as root:\n  $ btmon -w %s\n' "$TRACE"
ask "start btmon as above"
# Item 6 is the polarity of the general settings, and the one frame that tells
# the two readings apart is the multipoint slot read while TWO peers are
# connected: the paired-device list the daemon asks at link-up then shows both.
ask "with multipoint on, connect a second device (a phone) to the headset as well (item 6)"

say "2. the daemon, recording, with alerts on (items 6, 14)"
# Off first: the shell's daemon would hold the socket and the headset's one
# control channel, and this one would refuse to start.
run sh -c "command -v sony-daemon.sh >/dev/null && sony-daemon.sh off || true"
run mkdir -p "$RECORD"
printf '  $ %s run --alerts --record=%s &\n' "$QS_SONYD" "$RECORD"
if [[ $DRY -eq 0 ]]; then
    "$QS_SONYD" run --alerts --record="$RECORD" 2>"$OUT/daemon.log" &
    DAEMON_PID=$!
    trap 'kill "$DAEMON_PID" 2>/dev/null' EXIT
    sleep 8
fi
run "$QS_SONY" status

say "3. every verb, once (items 2-6)"
for verb in "noise anc" "noise ambient" "ambient-level 10" "focus-on-voice on" "noise anc" \
            "connection stable" "connection quality" "playback pause" "playback play" \
            "dsee on" "volume 30"; do
    # shellcheck disable=SC2086  # the verb is words on purpose
    run "$QS_SONY" send $verb
done

say "4. a general-setting write, its alert, and the answer (item 14)"
run "$QS_SONY" send general multipoint off
ask "if the headset asked (qs-sony status shows .alert), answer it: qs-sony send alert yes"
run "$QS_SONY" status

say "5. disconnect and reconnect, then read it back (item 14)"
ask "disconnect the headset and connect it again (bluetoothctl disconnect, then connect)"
[[ $DRY -eq 0 ]] && sleep 10
run "$QS_SONY" status
run "$QS_SONY" send general multipoint on

say "6. the page, drawn against the live document (item 13)"
ask "open the Headphones page in jayos and take a screenshot into $OUT"

say "7. the kernel's half of the socket address (item 7)"
run cargo test --manifest-path "$ROOT/Cargo.toml" --test sony_link -- --ignored

say "8. stop, import the trace, and settle"
if [[ $DRY -eq 0 ]]; then
    kill "$DAEMON_PID" 2>/dev/null
    wait "$DAEMON_PID" 2>/dev/null
    trap - EXIT
fi
ask "stop btmon (Ctrl-C in its terminal)"
run "$QS_SONY" import "$TRACE" --out="$IMPORTED"
run "$QS_SONY" settle "$IMPORTED"
printf '  $ %s settle %s\n' "$QS_SONY" "$RECORD"
if [[ $DRY -eq 1 ]]; then
    printf '\ndry run: nothing was run\n'
    exit 0
fi
"$QS_SONY" settle "$RECORD"
status=$?
cat <<EOF

The capture is in $OUT. docs/CAPTURE.md says what to commit from it and where:
tests/fixtures/<model>-<firmware>/, with a PROVENANCE.md of its own.
EOF
exit "$status"
