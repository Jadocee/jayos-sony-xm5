#!/usr/bin/env bash
#
# The package recipe, without makepkg.
#
#   tests/shell/test-packaging.sh
#
# Exit 0: every case passed. 1: a case failed.
#
# makepkg is an Arch tool and this suite has to run where it is not. So the
# recipe is sourced and its functions are called directly, the way makepkg
# calls them, against a scratch $srcdir and $pkgdir -- with stand-ins for the
# built binaries rather than a release build, because what is under test is
# what package() installs and where, not whether cargo works. What this does
# NOT prove: that makepkg accepts the file (no namcap here), and that a real
# build produces what check() asserts. Those are read, not run.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RECIPE="$ROOT/packaging/arch/PKGBUILD"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/sony-packaging-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

PASS=0
FAIL=0
pass() { printf '  \033[32mok\033[0m   %s\n' "$1"; PASS=$((PASS + 1)); }
bad()  { printf '  \033[31mfail\033[0m %s\n' "$1"; FAIL=$((FAIL + 1)); }

echo "the recipe"

if bash -n "$RECIPE"; then
    pass "it parses"
else
    bad "it does not parse"
fi

# Sourced in a subshell for each question, so nothing it sets leaks into the
# next case.
recipe() { ( set +u; startdir="$ROOT/packaging/arch"; source "$RECIPE"; eval "$1" ); }

# THE VERSION IS THE CRATE'S. Written out rather than computed (the recipe
# says why), so it is held here instead.
crate_version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$ROOT/Cargo.toml" | head -1)
if [[ -n "$crate_version" && "$(recipe 'printf %s "$pkgver"')" == "$crate_version" ]]; then
    pass "pkgver is the crate's version ($crate_version)"
else
    bad "pkgver '$(recipe 'printf %s "$pkgver"')' is not the crate's version '$crate_version'"
fi

# THE HELPERS ARE THE BINARIES. One bin target per src/bin/<name>.rs, and no
# [[bin]] table in Cargo.toml -- so the directory is the list, and _helpers
# must be exactly it. A helper missing from _helpers is built and not
# installed; an extra one fails check() on a real build.
bins=$(cd "$ROOT/src/bin" && ls -1 ./*.rs | sed 's|^\./||; s/\.rs$//' | sort | tr '\n' ' ')
helpers=$(recipe 'printf "%s\n" "${_helpers[@]}"' | sort | tr '\n' ' ')
if [[ "$bins" == "$helpers" ]]; then
    pass "_helpers is exactly the binaries src/bin defines ($helpers)"
else
    bad "_helpers is '$helpers' and src/bin defines '$bins'"
fi
if grep -q '^\[\[bin\]\]' "$ROOT/Cargo.toml"; then
    bad "Cargo.toml declares [[bin]], so src/bin is no longer the whole list"
else
    pass "and no [[bin]] table makes src/bin an incomplete list"
fi

# One line, because jayos's integration check reads it with a one-line
# pattern, and a multi-line array would read as empty there.
if grep -qE '^_helpers=\([^)]*\)$' "$RECIPE"; then
    pass "_helpers is declared on one line"
else
    bad "_helpers is not a one-line array"
fi

echo
echo "package(), into a scratch pkgdir"

srcdir="$WORK/src"
pkgdir="$WORK/pkg"
mkdir -p "$srcdir/target/release" "$pkgdir"
for h in $helpers; do
    printf '#!/bin/sh\n' > "$srcdir/target/release/$h"
    chmod 755 "$srcdir/target/release/$h"
done
if ( set +u; startdir="$ROOT/packaging/arch"; source "$RECIPE"; \
     srcdir="$srcdir" pkgdir="$pkgdir"; package ) 2>"$WORK/err"; then
    pass "package() runs"
else
    bad "package() failed: $(cat "$WORK/err")"
fi

mode() { stat -c %a "$1" 2>/dev/null; }
for h in $helpers; do
    if [[ "$(mode "$pkgdir/usr/bin/$h")" == "755" ]]; then
        pass "/usr/bin/$h is installed, executable"
    else
        bad "/usr/bin/$h is missing or not 755"
    fi
done
if [[ "$(mode "$pkgdir/usr/lib/jayos-sony-xm5/sony-daemon-process")" == "755" ]] \
   && cmp -s "$pkgdir/usr/lib/jayos-sony-xm5/sony-daemon-process" "$ROOT/scripts/sony-daemon-process.sh"; then
    pass "the process script is installed where jayos looks for it"
else
    bad "/usr/lib/jayos-sony-xm5/sony-daemon-process is missing, not 755, or not the script"
fi
for f in LICENSE THIRD_PARTY.md; do
    if cmp -s "$pkgdir/usr/share/licenses/jayos-sony-xm5/$f" "$ROOT/$f"; then
        pass "$f is installed with the licences"
    else
        bad "$f is not installed under /usr/share/licenses/jayos-sony-xm5"
    fi
done
missing=""
for f in "$ROOT"/contract/*; do
    cmp -s "$f" "$pkgdir/usr/share/jayos-sony-xm5/contract/${f##*/}" || missing+=" ${f##*/}"
done
if [[ -z "$missing" ]]; then
    pass "every contract file is installed"
else
    bad "contract files not installed:$missing"
fi

echo
printf '%d passed, %d failed\n' "$PASS" "$FAIL"
[[ $FAIL -eq 0 ]]
