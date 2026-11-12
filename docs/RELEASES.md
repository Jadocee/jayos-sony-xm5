# Releases

A jayos-sony-xm5 release is a signed tag, `vMAJOR.MINOR.PATCH`, on `main`,
whose `VERSION` file says `MAJOR.MINOR.PATCH` -- and so does `Cargo.toml`'s
`[package]` version, which the daemon publishes as `qs-<version>`, and the
checkout recipe's `pkgver` (`tests/shell/test-release.sh` holds all three to
each other). The first is `1.0.0`, the crate's version since it moved out of
JayOS.

JayOS builds a release and nothing else, by the owner's call on 2026-11-12:
"Add releases to jayos-bluetooth and jayos-sony-xm5". Its recipe,
`pkgbuilds/jayos-sony-xm5/PKGBUILD` there, pins one by tag and by commit, and
its suites hold the Headphones page, `sony-daemon.sh` and the doctor to the
same release's `contract/` and `scripts/sony-daemon-process.sh`, which it
vendors as `tools/fixtures/jayos-sony-xm5-contract/`. Until then that recipe
built whatever `main` held at the moment of the build, so two machines that
built a day apart could hold two packages under one version, and JayOS's copy
of the contract could name a commit `main` had moved past.

The scripts are jayos-audio's (Jadocee/jayos-denoise, its `tools/release.sh`,
`tools/lib/release.sh` and `tools/publish-release.sh`), copied on 2026-11-12
with the package's name, so the owner signs this repository's releases,
jayos-audio's and JayOS's own with one key and one habit. What this copy adds
is the crate's version: a tag over a `Cargo.toml` that still names the
release before would install a daemon reporting another version than the one
JayOS pinned.

## Making one

On `main`, up to date with origin, with `VERSION` and `Cargo.toml`'s version
raised together and that commit pushed:

    tools/release.sh            # a dry run: the version, and every refusal
    tools/release.sh --apply    # the signed tag, checked and pushed

It refuses a branch other than `main`, a `main` that is not `origin/main`,
changes to tracked files, a `VERSION` that is not `MAJOR.MINOR.PATCH`, that
`Cargo.toml` does not repeat, or that is not newer than the newest release, a
commit already released, a commit whose `tools/check.sh` does not pass -- run
in a worktree of the commit itself, since this repository has no CI and that
check is the gate a release stands on -- signing that is not SSH, no signing
key, and a `.github/allowed_signers` on `origin/main` with no key in it. The
releases it counts are origin's tags, not the clone's: a tag made in a clone
and never pushed is no release. After the tag is made it is verified against
`origin/main`'s `.github/allowed_signers`, as the workflow will verify it,
and a tag that fails is deleted before anything is pushed.

`tools/check.sh` is the whole gate, and it is honest about it: exit 2, "not
exercised", where cargo or jq is missing, which the release refuses as it
refuses a failure.

`.github/workflows/release.yml` runs on the pushed tag. It holds no key and
makes no tag: `tools/publish-release.sh verify` asks again that the tag is
annotated, signed by a key `origin/main`'s `.github/allowed_signers` lists --
`main`'s, never the tag's own tree's, so a tag cannot bring the key that signs
it -- on `main`, and that `VERSION` at the tag is its version; `tools/check.sh`
runs at the tag; `notes` writes the release's notes, the commits since the
release before it, counting only tags that verify as releases; and `publish`
makes the GitHub release with them.

Then, in JayOS, `tools/package-release.sh jayos-sony-xm5 <version> --apply`
pins the release: it fetches the tag, verifies it against JayOS's own
`.github/allowed_signers`, writes its version and commit into the recipe, and
vendors its `contract/` and `sony-daemon-process` into
`tools/fixtures/jayos-sony-xm5-contract/` there, with their sums. That is a
change to JayOS, made and committed there, and its suites say whether the
release still fits.

## The owner's steps

Once, before the first release:

1. An SSH key that signs, the same one that signs JayOS's and jayos-audio's
   releases if there is one: `git config gpg.format ssh` and
   `git config user.signingkey <the public key's file>`.
2. Its public half in `.github/allowed_signers`, one line in ssh-keygen's
   ALLOWED SIGNERS format with `namespaces="git"`, committed and merged to
   `main`. The file is empty until then, and every release is refused.
3. The same line in JayOS's `.github/allowed_signers`, which its
   `tools/package-release.sh` verifies a release against.

For each release: raise `VERSION` and `Cargo.toml`'s version, merge to
`main`, then `tools/release.sh`, then `tools/release.sh --apply`, then JayOS's
`tools/package-release.sh jayos-sony-xm5 <version> --apply`.

## What has run, and what has only been read

RUN, by `tests/shell/test-release.sh` on 2026-11-12, 60 assertions against
scratch repositories, a bare origin, and two keys `ssh-keygen` (OpenSSH 9.6)
made for the run: every refusal above, a release made, signed, verified and
pushed, two more after it, a stranger's key refused with the tag deleted
unpushed, a `VERSION` raised without the crate's refused, a local tag that is
no release not counted, `verify` over a lightweight tag, an unsigned one, a
stranger's, one off `main` and one whose `VERSION` says otherwise, the notes
counted from the release before, and `publish`'s call of `gh release create`
against a stand-in; and, over this repository itself, that `VERSION`,
`Cargo.toml` and the checkout recipe agree. Five mutations of the scripts
were made, each restored afterwards -- the crate-version refusal removed, the
branch refusal removed, a stranger-signed tag kept rather than deleted, a
failing check let through, and `verify` letting a tag off `main` through --
and the suite failed on all five.

Read, not run: the workflow on GitHub's runners, `gh release create` against
GitHub, JayOS's `tools/package-release.sh` against this repository, and a
release of it, of which none exists yet.
