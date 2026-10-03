#!/usr/bin/env bash
#
# Everything this repository checks, in one run.
#
#   tools/check.sh
#
# Exit 0: everything ran and passed. 1: something failed. 2: nothing failed but
# something could not run -- no cargo, no jq -- which is a different answer
# from "clean" and is reported as one. A check that examined nothing must not
# be counted as a check that found nothing.
#
# The same three statuses as jayos's suites, because jayos's installer runs its
# own copy of the contract check against this repository's output and reads
# them the same way.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || exit 2

FAILED=()
SKIPPED=()

step() { printf '\n\033[1m%s\033[0m\n' "$1"; }

# run <name> <command...>: 0 passes, 2 is "could not run", anything else fails.
run() {
    local name="$1"
    shift
    "$@"
    case $? in
        0) ;;
        2) SKIPPED+=("$name") ;;
        *) FAILED+=("$name") ;;
    esac
}

step "shell syntax"
syntax() {
    local f bad=0
    while IFS= read -r f; do
        bash -n "$f" || { echo "  syntax error in $f"; bad=1; }
    done < <(git ls-files '*.sh' 'packaging/arch/PKGBUILD' 2>/dev/null)
    return "$bad"
}
run "shell syntax" syntax

step "cargo test"
if command -v cargo >/dev/null 2>&1; then
    run "cargo test" cargo test --locked --quiet
else
    echo "  cargo is not installed"
    SKIPPED+=("cargo test")
fi

for suite in tests/shell/test-*.sh; do
    step "$suite"
    run "$suite" bash "$suite"
done

echo
if (( ${#FAILED[@]} )); then
    printf 'FAILED: %s\n' "${FAILED[@]}"
fi
if (( ${#SKIPPED[@]} )); then
    printf 'not exercised: %s\n' "${SKIPPED[@]}"
fi
if (( ${#FAILED[@]} )); then
    exit 1
elif (( ${#SKIPPED[@]} )); then
    exit 2
fi
echo "ok, nothing to fix"
