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

# The process script starts the daemon from where package() puts it unless
# SONY_DAEMON says otherwise, and every suite that runs the script says
# otherwise -- so nothing but this reads the default. It is the daemon among
# _helpers, under /usr/bin: the two were renamed from qs-sony and qs-sonyd on
# 2026-11-13, and a default left at the old name would start nothing.
daemon=$(tr ' ' '\n' <<< "$helpers" | grep -E 'd$' | head -1)
default=$(grep -vE '^[[:space:]]*#' "$ROOT/scripts/sony-daemon-process.sh" \
          | sed -n 's|^DAEMON="${SONY_DAEMON:-\([^}]*\)}"$|\1|p')
if [[ -n "$daemon" && "$default" == "/usr/bin/$daemon" ]]; then
    pass "the process script starts /usr/bin/$daemon, where package() installs it"
else
    bad "the process script's daemon is '${default:-none}', and package() installs /usr/bin/${daemon:-?}"
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
echo "build() and check(): mold links them"
# MOLD LINKS IT, through RUSTFLAGS: build() and check(), run as makepkg runs
# them, start cargo with the RUSTFLAGS makepkg.conf sets or with none, and
# mold's flag after it, once -- in one shell, where check() inherits build()'s
# export, and check() alone, where it must say mold itself. A stand-in cargo
# writes down its verb and the RUSTFLAGS it was started with.
if grep -qE "^makedepends=\(.*'mold'" "$RECIPE"; then
    pass "makedepends names mold"
else
    bad "makedepends does not name mold, which build() and check() link with"
fi
FAKEBIN="$WORK/fakebin"
mkdir -p "$FAKEBIN" "$WORK/msrc/target/release"
printf '#!/usr/bin/env bash\nprintf "%%s|%%s\\n" "$1" "${RUSTFLAGS-<unset>}" >> %q\nexit 0\n' "$WORK/cargo.flags" > "$FAKEBIN/cargo"
chmod +x "$FAKEBIN/cargo"
for h in jayos-sony jayos-sonyd; do printf '#!/bin/sh\n' > "$WORK/msrc/target/release/$h"; chmod +x "$WORK/msrc/target/release/$h"; done
mold_run() { # mold_run <functions> [VAR=value...]: the cargo calls they made, verb|RUSTFLAGS
    : > "$WORK/cargo.flags"
    ( env -u RUSTFLAGS PATH="$FAKEBIN:$PATH" srcdir="$WORK/msrc" "${@:2}" \
          bash -eE -c 'startdir="$1"; source "$2"; '"$1" _ "$ROOT/packaging/arch" "$RECIPE" ) >/dev/null 2>&1
    paste -sd' ' "$WORK/cargo.flags"
}
mold_case() { # mold_case <description> <expected> <functions> [VAR=value...]
    local got
    got=$(mold_run "${@:3}")
    if [[ "$got" == "$2" ]]; then pass "$1"; else bad "$1: got '$got', not '$2'"; fi
}
mold_case "build() and check() in one shell: makepkg's RUSTFLAGS and mold's flag after them, once" \
    "build|-Cforce-frame-pointers=yes -C link-arg=-fuse-ld=mold test|-Cforce-frame-pointers=yes -C link-arg=-fuse-ld=mold" \
    "build; check" RUSTFLAGS=-Cforce-frame-pointers=yes
mold_case "check() alone, in a shell of its own, says mold itself" \
    "test|-Cforce-frame-pointers=yes -C link-arg=-fuse-ld=mold" "check" RUSTFLAGS=-Cforce-frame-pointers=yes
mold_case "mold's flag alone where makepkg set none" "build|-C link-arg=-fuse-ld=mold" "build"
mold_case "and not a second time over a RUSTFLAGS that has it already" \
    "build|-C link-arg=-fuse-ld=mold" "build" RUSTFLAGS="-C link-arg=-fuse-ld=mold"

echo
echo ".cargo/config.toml and tools/check.sh: mold links the tests"
# The file's one setting, as a TOML reader reads it, which tools/check.sh's
# `cargo test`, run in the checkout, reads; and check.sh's fallback, read
# rather than run, since running it is the whole test build.
MOLD_CFG="$ROOT/.cargo/config.toml"
want='{"target": {"x86_64-unknown-linux-gnu": {"rustflags": ["-C", "link-arg=-fuse-ld=mold"]}}}'
if [[ ! -f "$MOLD_CFG" ]]; then
    bad ".cargo/config.toml is gone, and nothing links the tests with mold"
elif python3 -c 'import tomllib' 2>/dev/null; then
    got=$(python3 -I -c 'import json, sys, tomllib; print(json.dumps(tomllib.load(open(sys.argv[1], "rb")), sort_keys=True))' "$MOLD_CFG" 2>&1)
    if [[ "$got" == "$want" ]]; then pass "it holds one setting: mold, as the x86_64 linker's argument"; else bad ".cargo/config.toml reads as $got"; fi
else
    echo "  skip .cargo/config.toml as TOML not exercised: no python3 with tomllib"
fi
CHECK_CODE=$(grep -vE '^[[:space:]]*#' "$ROOT/tools/check.sh")
if grep -qxF '    run "cargo test" env "${link[@]}" cargo test --locked --quiet' <<<"$CHECK_CODE" \
   && grep -qxF '    if ! command -v ld.mold >/dev/null 2>&1; then' <<<"$CHECK_CODE" \
   && grep -qxF '        link=(RUSTFLAGS=)' <<<"$CHECK_CODE" \
   && grep -qxF '        SKIPPED+=("cargo test, linked with mold")' <<<"$CHECK_CODE"; then
    pass "tools/check.sh tests through it, falling back only with no ld.mold, listed as not exercised"
else
    bad "tools/check.sh's cargo test does not link through .cargo/config.toml's mold, falling back only without it"
fi

echo
printf '%d passed, %d failed\n' "$PASS" "$FAIL"
[[ $FAIL -eq 0 ]]
