#!/usr/bin/env bash
#
# tools/acceptance.sh, without a headset.
#
#   tests/shell/test-acceptance.sh
#
# Exit 0: every case passed. 1: a case failed.
#
# The session itself needs a WH-1000XM5 and a person; what can be held here is
# everything around it. --dry-run must name every step and run none of them --
# a dry run that ran `btmon` or wrote the capture directory would be the
# opposite of what it says. And the real run must refuse with exit 2, not 1
# and not 0, when it cannot run: a missing tool or a missing headset is "could
# not run", and reporting it as items left open would read as a capture that
# happened.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/acceptance.sh"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/sony-acceptance-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

PASS=0
FAIL=0
pass() { printf '  \033[32mok\033[0m   %s\n' "$1"; PASS=$((PASS + 1)); }
bad()  { printf '  \033[31mfail\033[0m %s\n' "$1"; FAIL=$((FAIL + 1)); }

command -v jq >/dev/null 2>&1 || { echo "  skip acceptance not exercised: no jq"; exit 2; }

# Stand-ins for every tool the session runs, each of which writes its name and
# arguments to a log when it is run at all.
mkdir -p "$WORK/bin"
for tool in qs-sony qs-sonyd btmon bluetoothctl cargo; do
    cat > "$WORK/bin/$tool" <<EOF
#!/usr/bin/env bash
echo "$tool \$*" >> "$WORK/ran"
if [[ "$tool" == qs-sony && "\${1:-}" == devices ]]; then
    printf '%s\n' '{"devices":[{"name":"WF-1000XM5","sony":true}]}'
fi
EOF
    chmod +x "$WORK/bin/$tool"
done
with_fakes() { PATH="$WORK/bin:$PATH" "$@"; }

echo "--dry-run"

out=$(with_fakes bash "$SCRIPT" --dry-run --out="$WORK/capture" 2>&1)
status=$?
if [[ "$status" == "0" ]]; then
    pass "it exits 0"
else
    bad "it exited $status: $out"
fi
if [[ ! -e "$WORK/ran" ]]; then
    pass "and runs nothing"
else
    bad "it ran: $(cat "$WORK/ran")"
fi
if [[ ! -e "$WORK/capture" ]]; then
    pass "and creates nothing"
else
    bad "it created $WORK/capture"
fi
# Every step, by the command that carries it. A step dropped from the script
# fails here even though nothing ran.
missing=""
for want in "btmon -w $WORK/capture/trace.btsnoop" \
            "qs-sonyd run --alerts --record=$WORK/capture/record" \
            "qs-sony send connection stable" \
            "qs-sony send playback pause" \
            "qs-sony send general multipoint off" \
            "qs-sony send alert yes" \
            "disconnect the headset and connect it again" \
            "screenshot" \
            "--test sony_link -- --ignored" \
            "qs-sony import $WORK/capture/trace.btsnoop --out=$WORK/capture/imported" \
            "qs-sony settle $WORK/capture/record"; do
    [[ "$out" == *"$want"* ]] || missing+=$'\n    '"$want"
done
if [[ -z "$missing" ]]; then
    pass "it names every step of the session"
else
    bad "the dry run does not say:$missing"
fi

echo
echo "a real run that cannot run says so with 2"

rm -f "$WORK/ran"
out=$(QS_SONY="$WORK/nothing" bash "$SCRIPT" --out="$WORK/capture" 2>&1)
status=$?
if [[ "$status" == "2" && "$out" == *"cannot run"* ]]; then
    pass "a missing tool is exit 2"
else
    bad "a missing tool answered $status: $out"
fi

# Every tool present, and the only headset connected is the wrong one.
out=$(with_fakes bash "$SCRIPT" --out="$WORK/capture" 2>&1 </dev/null)
status=$?
if [[ "$status" == "2" && "$out" == *"no WH-1000XM5 is connected"* ]]; then
    pass "no WH-1000XM5 connected is exit 2"
else
    bad "the wrong headset answered $status: $out"
fi
if [[ "$(cat "$WORK/ran" 2>/dev/null)" == "qs-sony devices" ]]; then
    pass "and it asked which headset before it started anything"
else
    bad "before refusing it ran: $(cat "$WORK/ran" 2>/dev/null)"
fi

out=$(bash "$SCRIPT" --nonsense 2>&1)
status=$?
if [[ "$status" == "2" && "$out" == *"usage:"* ]]; then
    pass "an unknown argument is refused with a usage line and status 2"
else
    bad "an unknown argument answered $status: $out"
fi

echo
printf '%d passed, %d failed\n' "$PASS" "$FAIL"
[[ $FAIL -eq 0 ]]
