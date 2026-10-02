#!/usr/bin/env bash
#
# Regression tests for scripts/sony-daemon-process.sh.
#
#   tests/shell/test-daemon-process.sh
#
# Exit 0: every case passed. 1: a case failed. 2: could not run (no jq), which
# is not the same answer as "nothing is wrong" and must not be reported as one.
#
# The script decides three things a person can see: whether `stop` reaches
# somebody else's process, where the daemon's last words went, and whether a
# reader is left a document with no daemon behind it. All three are tested
# against a real fake binary and real processes -- none against an expected
# string, because an expected string only proves the script still does what it
# did.
#
# These cases were the process half of jayos's tools/test-sony-daemon.sh, which
# tested the whole lifecycle script before the script was split by ownership:
# the process half moved here with the daemon, the setting half stayed in
# jayos with the settings file. The cases that are new here are marked.
#
# WHY THE DAEMON IS FAKED AND NOT STUBBED OUT
#
# `start` and `stop` are the two verbs worth testing and both are about
# PROCESSES: whether one is left running, whether the pattern matches the right
# one. A stub that recorded "start was called" would test the case statement
# and nothing else. So the fake is a real executable that really sleeps, and
# the assertions are `pgrep` against the process table.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/scripts/sony-daemon-process.sh"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/sony-process-test.XXXXXX")"
cleanup() {
    # Anything this test started, whatever it did afterwards.
    pkill -u "$(id -u)" -f -- "^$WORK/bin/qs-sonyd" 2>/dev/null || true
    rm -rf "$WORK"
}
trap cleanup EXIT

PASS=0
FAIL=0
pass() { printf '  \033[32mok\033[0m   %s\n' "$1"; PASS=$((PASS + 1)); }
bad()  { printf '  \033[31mfail\033[0m %s\n' "$1"; FAIL=$((FAIL + 1)); }

command -v jq >/dev/null 2>&1 || {
    echo "$(basename "$0"): jq is not installed; these tests cannot run" >&2
    exit 2
}

mkdir -p "$WORK/bin"
# A daemon that does what a daemon does: says on stderr what it could not
# decode -- the sentence qs-sonyd's log() prints, with the numbers written in
# hex here where the real one prints them with {:?} -- then stays up until it
# is killed.
#
# `exec -a "$0"` is what makes it look like the real one on the process table.
# qs-sonyd is an ELF, so its command line STARTS with its path, which is what
# the script's pattern is anchored to. A shell script run through its shebang
# shows up as `bash /path/qs-sonyd` instead, and an anchored pattern would
# rightly not match it -- so without this the suite would be testing a process
# shape the real daemon never has.
cat > "$WORK/bin/qs-sonyd" <<'FAKE'
#!/usr/bin/env bash
echo "qs-sonyd: no decoder for command 0x66 kind 0x17 len 3" >&2
exec -a "$0" sleep 300
FAKE
chmod +x "$WORK/bin/qs-sonyd"

# Every run points the script at the fake and HOME/XDG at the work tree, so
# nothing here can touch a real daemon or a real log.
STATE="$WORK/state/sony-headphones"
run() { # run <verb...>
    XDG_STATE_HOME="$WORK/state" HOME="$WORK" SONY_DAEMON="$WORK/bin/qs-sonyd" \
        bash "$SCRIPT" "$@" 2>&1
}
field() { run --json | jq -r ".$1"; }
alive() { pgrep -u "$(id -u)" -f -- "^$WORK/bin/qs-sonyd" >/dev/null 2>&1; }
count() { pgrep -u "$(id -u)" -c -f -- "^$WORK/bin/qs-sonyd" 2>/dev/null || echo 0; }

echo "installed, and running, as two facts"

if [[ "$(field installed)" == "true" && "$(field running)" == "false" ]]; then
    pass "a binary that is there and not started is installed and not running"
else
    bad "--json did not report installed/running apart: $(run --json)"
fi

# A binary that is not there is not something to start. The settings page
# draws it grey, so `installed` must be false, and `start` must fail loudly
# rather than print "started" over nothing.
if [[ "$(XDG_STATE_HOME="$WORK/state" HOME="$WORK" SONY_DAEMON="$WORK/nothing" \
        bash "$SCRIPT" --json | jq -r .installed)" == "false" ]]; then
    pass "a missing binary is reported as not installed"
else
    bad "a missing binary was reported as installed"
fi
out=$(XDG_STATE_HOME="$WORK/state" HOME="$WORK" SONY_DAEMON="$WORK/nothing" bash "$SCRIPT" start 2>&1)
status=$?
if [[ "$status" == "1" && "$out" == *"no daemon at $WORK/nothing"* ]]; then
    pass "start with no binary exits 1 and says where it looked"
else
    bad "start with no binary answered '$out' with status $status"
fi

echo
echo "start and stop, against the process table"

run start >/dev/null
sleep 0.3
if alive; then
    pass "start starts it"
else
    bad "start did not start the daemon"
fi

if [[ "$(field running)" == "true" ]] && run running >/dev/null; then
    pass "a running daemon is reported running, and \`running\` exits 0"
else
    bad "a running daemon was not reported running"
fi

# Started twice is still one daemon. A second would take the socket lock,
# fail, and exit -- so the symptom of getting this wrong is a daemon that
# 'will not start' with a working one already up.
before=$(count)
run start >/dev/null
sleep 0.3
after=$(count)
if [[ "$before" == "1" && "$after" == "1" ]]; then
    pass "starting twice leaves one daemon, not two"
else
    bad "starting twice left $after daemons (was $before)"
fi

run stop >/dev/null
sleep 0.4
if ! alive; then
    pass "stop stops it"
else
    bad "stop left the daemon running"
fi

if ! run running >/dev/null; then
    pass "and \`running\` then exits non-zero"
else
    bad "\`running\` exited 0 with no daemon"
fi

echo
echo "the pattern is the binary, not the word"

# `pkill -f qs-sonyd` would match a person's editor open on the source, and
# matching it would mean `stop` kills their editor. The pattern is the full
# path to this user's own binary, so a bystander that merely mentions the name
# must survive.
#
# THE BYSTANDER IS TRACKED BY PID, NEVER BY PATTERN. An earlier version of this
# test found it with `pgrep -f` and killed it with `pkill -f`, which also
# matched the SHELL RUNNING THE TEST -- its command line contained the pattern
# because the pattern is written in this file. It killed its own parent.
run start >/dev/null
sleep 0.3
bash -c "exec -a 'vim qs-sonyd.rs' sleep 30" &
named=$!
# New here: a bystander whose command line holds the daemon's FULL PATH as an
# argument -- a pager or an editor on the binary itself. jayos's pattern was
# the path unanchored, which this command line contains, so `stop` killed it.
bash -c "exec -a 'less $WORK/bin/qs-sonyd' sleep 30" &
pathed=$!
sleep 0.2
run stop >/dev/null
sleep 0.4
if kill -0 "$named" 2>/dev/null; then
    pass "stop leaves a process that merely mentions the daemon's name alone"
else
    bad "stop killed an unrelated process whose name contains 'qs-sonyd'"
fi
if kill -0 "$pathed" 2>/dev/null; then
    pass "and one whose arguments name the daemon's full path"
else
    bad "stop killed a process that only had the daemon's path as an argument"
fi
if ! alive; then
    pass "while still stopping the daemon itself"
else
    bad "the daemon survived a stop that spared the bystanders"
fi
kill "$named" "$pathed" 2>/dev/null || true
wait "$named" "$pathed" 2>/dev/null || true

echo
echo "the daemon's stderr is kept, one run back"

# The daemon's only outlet for a message it has no decoder for is stderr, and
# that line is what somebody extending the port for an untested headset needs.
rm -rf "$WORK/state"
run start >/dev/null
sleep 0.3
if grep -q 'no decoder for command 0x66' "$STATE/daemon.log" 2>/dev/null; then
    pass "start keeps the daemon's stderr in daemon.log"
else
    bad "daemon.log does not hold the daemon's stderr: $(cat "$STATE/daemon.log" 2>/dev/null)"
fi

# A crash at login is followed by a restart, and the restart must not truncate
# the notes that explain the crash. One previous run is kept as .1; the new
# log holds only the new run.
run stop >/dev/null
sleep 0.4
run start >/dev/null
sleep 0.3
if grep -q 'no decoder for command 0x66' "$STATE/daemon.log.1" 2>/dev/null; then
    pass "a second start keeps the first run's log as daemon.log.1"
else
    bad "the first run's log was not kept as daemon.log.1"
fi
# `grep -c` prints 0 AND exits 1 on no match, so `|| echo 0` would make this
# "0\n0"; the empty case is a missing file, handled by the default instead.
lines=$(grep -c 'no decoder for command 0x66' "$STATE/daemon.log" 2>/dev/null)
lines=${lines:-0}
if [[ "$lines" == "1" ]]; then
    pass "and daemon.log holds the second run alone"
else
    bad "daemon.log holds $lines 'no decoder' lines after a restart, not 1"
fi

echo
echo "the document goes with the daemon, and only with it"

# The daemon leaves its document behind on purpose and has no signal handler,
# so after `stop` the file still says whatever the headset was doing. A reader
# that trusts the file alone would draw a headset nobody is talking to.
printf '{"schema_version":1,"connected":true}\n' > "$STATE/status.json"
run stop >/dev/null
sleep 0.4
if [[ ! -e "$STATE/status.json" ]]; then
    pass "stop removes the status.json the daemon leaves behind"
else
    bad "stop left status.json in place -- readers will see a connected headset with no daemon"
fi

# New here: the C++ daemon publishes at the same path. A `stop` that found no
# qs-sonyd to stop has no business deleting the document something else is
# keeping current.
printf '{"schema_version":1,"connected":true}\n' > "$STATE/status.json"
run stop >/dev/null
if [[ -e "$STATE/status.json" ]]; then
    pass "a stop with nothing to stop leaves another daemon's document alone"
else
    bad "a stop that stopped nothing deleted status.json"
fi

if [[ "$(field log)" == "$STATE/daemon.log" ]]; then
    pass "--json names the log"
else
    bad "--json did not name the log: $(run --json)"
fi

echo
echo "everything it prints is what a reader can parse"

if run --json | jq -e . >/dev/null 2>&1; then
    pass "--json is valid JSON"
else
    bad "--json is not valid JSON: $(run --json)"
fi

# 2, not 1: `running` answers 1 for "not running", and a typo must not read as
# that answer.
out=$(run nonsense)
status=$?
if [[ "$status" == "2" && "$out" == *"usage:"* ]]; then
    pass "an unknown verb is refused with a usage line and status 2"
else
    bad "an unknown verb answered '$out' with status $status"
fi

echo
printf '%d passed, %d failed\n' "$PASS" "$FAIL"
[[ $FAIL -eq 0 ]]
