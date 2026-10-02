#!/usr/bin/env bash
#
# Source guards: two properties of the Rust that no Rust test can hold.
#
#   tests/shell/test-source-guards.sh
#
# Exit 0: every guard holds. 1: one does not.
#
# Both were in jayos's tools/test-sony-daemon.sh, as greps over the Rust files
# that lived in jayos then, and moved here with those files. They are greps
# rather than tests for reasons each one gives below: the first is about which
# tests hold a lock, which a test cannot see from inside; the second is about a
# binary's private functions, which no integration test can call.
#
# Comments are stripped before every grep. A comment that names the thing a
# check looks for would otherwise satisfy the check over a tree where the code
# was deleted.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

PASS=0
FAIL=0
pass() { printf '  \033[32mok\033[0m   %s\n' "$1"; PASS=$((PASS + 1)); }
bad()  { printf '  \033[31mfail\033[0m %s\n' "$1"; FAIL=$((FAIL + 1)); }

# ── every environment-touching qs-sony test is serialised, spawners included ──
#
# WHY THIS IS HERE, in a shell suite, about a Rust test module.
#
# `a_daemon_is_running_while_it_holds_the_lock_or_answers_the_socket` failed
# roughly twice in eight `cargo test --workspace` runs and passed in isolation
# every time. It was reported as unexplained in three pull requests.
#
# The cause is not in that test. `Command::spawn` reads the whole `environ`
# array to build a child's copy of it, and glibc's `setenv`/`unsetenv`
# reallocate that array in place -- so the three `capture` tests, which set no
# variable at all, are readers racing the writers. A torn read makes
# XDG_RUNTIME_DIR come back absent, `src/sony/mod.rs`'s `runtime_dir` falls back to the
# machine-global /tmp/sony-headphones-<uid>, and the failing test's probes
# leave its own scratch directory.
#
# The fix is three `Env::hold()` calls. Nothing about them looks load-bearing,
# they cost nothing to delete, and the symptom of deleting them is an
# intermittent failure in a DIFFERENT test -- which is how it went undiagnosed
# the first time. So: a check, rather than trusting the comment.
#
# COMMENTS STRIPPED, or the long note explaining Env::hold would satisfy a
# grep for Env::hold.
QS_SONY="$ROOT/src/bin/qs-sony.rs"
if [[ ! -f "$QS_SONY" ]]; then
    bad "src/bin/qs-sony.rs is missing"
else
    # SCOPED TO THE TEST MODULE, and this is not fussiness: the first version
    # of the count below matched qs-sony.rs:627, the PRODUCTION call to
    # capture(), and reported four call sites where three are tests. A check
    # that counts the thing it is not about is a check that fails for the wrong
    # reason -- which is the better half of that mistake; the other half is
    # passing for the wrong reason.
    SONY_CODE=$(sed -n '/^mod tests {/,$p' "$QS_SONY" | grep -vE '^[[:space:]]*(//|/\*|\*)')

    # One per subprocess-spawning test. Counted rather than merely present, so
    # removing two of the three is caught.
    HOLDS=$(grep -cE 'let _env = Env::hold\(\);' <<<"$SONY_CODE" || true)
    if [[ "${HOLDS:-0}" == "3" ]]; then
        pass "all three subprocess tests hold the environment lock"
    else
        bad "subprocess tests holding the environment lock: want 3, got ${HOLDS:-0}"
    fi

    # And that there are still exactly three of them to hold it. If a fourth
    # capture() test arrives without the lock, the count above stays at 3 and
    # says nothing -- this is the half that notices.
    CAPTURES=$(grep -cE '^[[:space:]]*(let [a-z_]+ = )?capture\(' <<<"$SONY_CODE" || true)
    if [[ "${CAPTURES:-0}" == "3" ]]; then
        pass "and there are still exactly three of them"
    else
        bad "capture() call sites in tests: want 3, got ${CAPTURES:-0} -- a new one needs Env::hold()"
    fi

    if grep -qE 'fn hold\(\) -> Env' <<<"$SONY_CODE"; then
        pass "Env::hold exists rather than each test open-coding the lock"
    else
        bad "Env::hold is gone; the lock is being taken some other way"
    fi
fi

# ── an ended link is two different things, and only one is a fault ───────────
#
# WHY THIS IS A GREP AND NOT A TEST.
#
# `read_until_gone` and the engine's mailbox live in `bin/qs-sonyd.rs`, which
# is a binary: no integration test can call either, and `sony_daemon.rs` can
# only start the process on a machine with no Bluetooth adapter, where no link
# is ever established and so none ever ends. The behaviour is a two-line
# mapping and the mapping is what goes stale, so the mapping is what is pinned.
#
# What it is about: `read` returning 0 is an orderly close -- the headphones
# switched off -- and every other error is a fault. Both used to arrive at the
# engine as a sentence, which wrapped them in `Down::Failed` without asking,
# so the ordinary end of a session published `phase: "error"` and jayos's
# `readSony` (`ts/src/settings/Integration.ts`) drew a red chip for it.
# `Down::Closed` existed the whole time and reached `Phase::Idle` correctly
# (`src/sony/session.rs`, `link_down`); nothing outside the test suite ever
# produced one.
SONYD="$ROOT/src/bin/qs-sonyd.rs"
if [[ ! -f "$SONYD" ]]; then
    bad "src/bin/qs-sonyd.rs is missing"
else
    # COMMENTS STRIPPED. The comments above both edits name `Down::Failed` and
    # `Down::Closed` in prose, at length, which is exactly the shape that
    # satisfies a grep written to find the code.
    SONYD_CODE="$(grep -vE '^[[:space:]]*(//|/\*|\*)' "$SONYD")"

    if grep -qE 'fn read_until_gone\(.*\) -> Down' <<<"$SONYD_CODE"; then
        pass "the reader answers with a reason the engine can tell apart"
    else
        bad "read_until_gone answers something other than Down; EOF and a fault are one thing again"
    fi

    if grep -qE 'Ok\(0\) => return Down::Closed' <<<"$SONYD_CODE"; then
        pass "and an orderly end-of-file is a close, not a failure"
    else
        bad "the end-of-file arm no longer answers Down::Closed"
    fi

    # THE ENGINE MUST NOT RE-WRAP. Scoped to the LinkDown arm: `Down::Failed`
    # is correct twice inside `read_until_gone` and wrong here, so a file-wide
    # grep would answer the opposite of the question.
    ARM="$(awk '/Ok\(Msg::LinkDown\(why\)\) => \{/ { a = 1 }
                a { print } a && /^            \}/ { exit }' <<<"$SONYD_CODE")"
    if [[ -z "$ARM" ]]; then
        bad "the engine's LinkDown arm could not be found; this check examined nothing"
    elif grep -q 'Down::Failed' <<<"$ARM"; then
        bad "the engine wraps every ended link in Down::Failed again"
    elif grep -q 'Down::HungUp' <<<"$ARM"; then
        pass "and the engine keeps only its own hang-up, passing the rest through"
    else
        bad "the engine's LinkDown arm no longer distinguishes its own hang-up"
    fi
fi

echo
printf '%d passed, %d failed\n' "$PASS" "$FAIL"
[[ $FAIL -eq 0 ]]
