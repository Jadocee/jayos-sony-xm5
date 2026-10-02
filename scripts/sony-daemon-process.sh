#!/usr/bin/env bash
#
# The qs-sonyd process: start it, stop it, say whether it is up.
#
#   sony-daemon-process start     start it unless it is already running
#   sony-daemon-process start --alerts
#                                 the same, with the headset's confirmations on
#   sony-daemon-process stop      stop it, and remove the document it left
#   sony-daemon-process running   exit 0 if it is running, 1 if not
#   sony-daemon-process --json    {installed, running, log}, for a settings page
#
# Installed as /usr/lib/jayos-sony-xm5/sony-daemon-process. It knows nothing
# about whether the daemon SHOULD run: that is a person's setting, and the
# setting belongs to the shell that draws the switch. jayos's
# config/common/scripts/hypr/sony-daemon.sh owns `on`, `off` and the
# `sonyDaemon` key, and calls this for everything that touches a process. The
# split is by who owns what, and it is why this file has no settings path in
# it: a package that edited a desktop's settings file would be a package that
# breaks when the desktop moves the file.
#
# qs-sonyd opens an RFCOMM connection to a paired Sony headset and publishes
# its state. Three things follow from that and shape this script.
#
# NOT KILLED BY PATTERN ALONE. `pkill -f qs-sonyd` would also match a person's
# `less qs-sonyd.rs`, and more to the point it would match the OTHER user's
# daemon on a shared machine. The pattern is this user's processes only, and
# it is the daemon's absolute path ANCHORED AT THE START of the command line:
# unanchored, as it was in jayos, it still matched `less /usr/bin/qs-sonyd`,
# because that command line contains the path too.
#
# STDERR IS KEPT, in $XDG_STATE_HOME/sony-headphones/daemon.log beside the
# status.json the daemon publishes. The daemon's only outlet for a message it
# has no decoder for -- "no decoder for command X kind Y len Z" -- is stderr,
# and that line is the one thing needed to extend the port for a headset
# nobody has fixtures for. One previous run is kept as daemon.log.1.
#
# THE DOCUMENT GOES WITH THE DAEMON. qs-sonyd leaves its last document behind
# on purpose (`qs-sonyd once` exists to do exactly that) and installs no
# signal handler, so after a kill the file still says whatever the headset was
# doing. `stop` removes it, as the reference daemon does on exit
# (sony-headphones-linux daemon_main.cpp, end of main).

set -uo pipefail

# The packaged path, not a PATH search: this is started from a login hook and
# from a settings page, neither of which runs the shell startup files. A
# checkout points SONY_DAEMON at its own target/<profile>/qs-sonyd.
DAEMON="${SONY_DAEMON:-/usr/bin/qs-sonyd}"

# `:-` and not `-`: an empty XDG_STATE_HOME is treated as unset, which is what
# the daemon's own state_dir does (src/sony/mod.rs), so the log lands beside
# its status.json rather than under /sony-headphones.
LOG_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/sony-headphones"
LOG="$LOG_DIR/daemon.log"

# Derived from DAEMON, not written out beside it. A stale pattern here is worse
# than a stale path: `start` would add a second daemon, and the second one
# refuses the socket lock and exits -- so the symptom would be a daemon that
# "will not start" with a perfectly good one already running.
#
# pgrep and pkill take an extended regular expression, so the path is escaped:
# a `.` in it would otherwise match any character. And it ends at a space or
# the end of the line, so /usr/bin/qs-sonyd-old is not this daemon either.
PATTERN="^$(printf '%s' "$DAEMON" | sed 's/[][\.*^$+?(){}|]/\\&/g')( |$)"

installed() { [[ -x "$DAEMON" ]]; }

# -u: this user's processes only. On a shared machine another user's daemon is
# not ours to see and emphatically not ours to kill.
running() { pgrep -u "$(id -u)" -f -- "$PATTERN" >/dev/null 2>&1; }

# --alerts is passed through to the daemon and is the only argument start
# takes. Alerts are a capture-session switch (qs-sonyd's encode::alerts_on
# says why they are off by default), so an unknown word here is refused rather
# than handed to the daemon to refuse after it has detached.
start_it() {
    local extra=()
    case "${1:-}" in
        "") ;;
        --alerts) extra=(--alerts) ;;
        *) echo "start takes only --alerts, not $1" >&2; return 2 ;;
    esac
    installed || { echo "no daemon at $DAEMON" >&2; return 1; }
    running && { echo "already running"; return 0; }
    mkdir -p "$LOG_DIR" 2>/dev/null || { echo "cannot create $LOG_DIR" >&2; return 1; }
    # One previous run is kept: the daemon is started at login, and a crash
    # there must not have its notes overwritten by the restart that follows.
    [[ -f "$LOG" ]] && mv -f "$LOG" "$LOG.1"
    # setsid -f so it survives the shell that started it -- a settings page
    # launches this through a process that exits immediately. The daemon writes
    # nothing to stdout; stderr is its notes, so that one goes to the log.
    setsid -f "$DAEMON" "${extra[@]}" >/dev/null 2>"$LOG"
    echo "started"
}

stop_it() {
    running || { echo "not running"; return 0; }
    pkill -u "$(id -u)" -f -- "$PATTERN" 2>/dev/null || true
    # The daemon has no signal handler by design, so SIGTERM ends it at once
    # and the kernel releases its lock. The grace period and the second signal
    # are for the case where it is wedged in a syscall.
    sleep 0.2
    pkill -9 -u "$(id -u)" -f -- "$PATTERN" 2>/dev/null || true
    # Only when this stopped a daemon. The C++ daemon publishes at the same
    # path, and a `stop` with no qs-sonyd running must not delete the document
    # a different daemon is keeping current.
    rm -f "$LOG_DIR/status.json"
    echo "stopped"
}

case "${1:-}" in
    start)
        start_it "${2:-}"
        ;;
    stop)
        stop_it
        ;;
    running)
        running
        ;;
    --json)
        # Through jq rather than printf so a path with a quote in it stays JSON.
        jq -cn \
            --argjson installed "$(installed && echo true || echo false)" \
            --argjson running   "$(running   && echo true || echo false)" \
            --arg log "$LOG" \
            '{installed: $installed, running: $running, log: $log}'
        ;;
    *)
        echo "usage: sony-daemon-process start [--alerts]|stop|running|--json" >&2
        exit 2
        ;;
esac
