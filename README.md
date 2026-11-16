# jayos-sony-xm5

Sony WH-1000XM5 headphones on Linux: the MDR protocol Sony's app speaks over
Bluetooth, a daemon that holds the connection and publishes what the headset
says, and the client JayOS's control room calls. Built for
[JayOS](https://github.com/Jadocee/jayos); usable without it.

Nothing here has spoken to a headset. The protocol is a port of
`mos9527/SonyHeadphonesClient`'s `libmdr` (MIT), checked byte for byte
against 70 frames captured from a **WF-1000XM5** -- the earbuds, not the
over-ears this is written for. What that does and does not prove is set out
in `tests/fixtures/mdr/PROVENANCE.md`, and every claim in the code says
which kind of evidence stands behind it.

## What is here

| | |
|---|---|
| `src/mdr/` | The protocol: framing (`frame`), what a payload means (`payload`), and the frames this port is willing to send (`encode`). Pure: no I/O. |
| `src/sony/` | The daemon's contract: the state document (`state`), the command language (`command`), the engine (`session`), RFCOMM and SDP (`link`, `sdp`), the control socket (`serve`), and playback of a capture (`replay`). |
| `src/capture.rs` | Captures: a btsnoop trace read down to its MDR frames (`import`), the daemon's own link written as fixtures (`Recorder`), and which outstanding items a capture settles (`settle`). |
| `src/bin/jayos-sonyd.rs` | The daemon. Dials the connected headset, publishes `$XDG_STATE_HOME/sony-headphones/status.json`, answers commands on a private Unix socket. `--alerts` asks the headset for its confirmations; `--record=DIR` writes every frame, both ways. |
| `src/bin/jayos-sony.rs` | The client: `status`, `devices`, `send <command…>`, `available`, `replay <dir>`, and for a capture `decode`, `import` and `settle`. |
| `src/support/` | Argument parsing, JSON on stdout and bounded subprocesses -- a copy of three jayos files; `src/support/mod.rs` says why a copy. |
| `scripts/sony-daemon-process.sh` | Starts and stops the daemon, keeps its stderr, removes its document on stop. Knows nothing about whether it *should* run; that is the shell's setting. |
| `contract/` | What JayOS tests its side against: a real state document, every key it carries, the command language, the model prefixes. Generated; see `contract/README.md`. |
| `packaging/arch/PKGBUILD` | Builds and installs this checkout. |
| `docs/` | `PROTOCOL.md`, the map; `EVIDENCE.md`, what each claim stands on; `OUTSTANDING.md`, what a WH-1000XM5 capture would still settle; `PLAN.md`, jayos's plan 23 that list was worked from, moved here; `CAPTURE.md`, how to make one; `RELEASES.md`, how a release is made. |
| `tools/acceptance.sh` | That capture session, scripted. `--dry-run` says every step and runs none. |
| `tools/release.sh`, `tools/publish-release.sh` | A release: the owner's signed tag, and what `.github/workflows/release.yml` checks and publishes. |

## Installing

On Arch, from a checkout:

    cd packaging/arch && makepkg -si

That installs `jayos-sonyd` and `jayos-sony` to `/usr/bin` and the process script to
`/usr/lib/jayos-sony-xm5/sony-daemon-process`. The two were `qs-sonyd` and
`qs-sony`, the names they had in JayOS, until the owner's call of 2026-11-13
renamed them before the first release; nothing reads the old names. JayOS carries a recipe of its
own that fetches this repository, and offers it from its Headphones and
Devices pages when a Sony headset is connected and this is not installed.

The daemon is off until something starts it. In JayOS that is the
"Headphone daemon" switch on the Headphones page; without JayOS,
`/usr/lib/jayos-sony-xm5/sony-daemon-process start`.

## Checking

    tools/check.sh

Runs `cargo test --locked`, the shell suites in `tests/shell/`, and a syntax
check over every tracked script. Exit 0 is clean, 1 is a failure, 2 is "could
not run" (no cargo, no jq) -- which is not the same answer as clean and is not
reported as one.

No test reaches the machine's Bluetooth, so the package's `check()` is safe on
a machine with a headset connected: the daemons the tests start are given a
`PATH` holding only a `bluetoothctl` of the suite's own, which lists nothing
connected, and a test fails if one asks anything else. Until 2026-11-06 they
inherited the build's `PATH` and dialled whatever was connected;
`tests/sony_daemon.rs`'s header has what that cost.

## Linking

The Rust is linked with [mold](https://github.com/rui314/mold), by JayOS's
owner's call of 2026-11-13. `packaging/arch/PKGBUILD`'s `build()` and
`check()` append `-C link-arg=-fuse-ld=mold` to `RUSTFLAGS`, after makepkg's
own and once, and `makedepends` names `mold`. `RUSTFLAGS` and not a cargo
config, because makepkg sets `RUSTFLAGS`, which cargo takes in place of every
config's flags. `.cargo/config.toml` gives cargo run in this tree the same
flag, and `tools/check.sh` links with Rust's own LLD where mold is not
installed, and lists that as not exercised. `tests/shell/test-packaging.sh`
holds each of these.

Run on 2026-11-13 under rustc 1.97.0 and mold 2.30.0: build() in 34 s and
check() in 21 s, run as makepkg runs them, `jayos-sony` and `jayos-sonyd` both
carrying `mold 2.30.0` in their `.comment` section, and `tools/check.sh`
found nothing to fix. Not run: Arch's makepkg.conf and Arch's mold 3.0.0.

## Releases

A release is a signed tag, `vMAJOR.MINOR.PATCH`, on `main`, made by the
owner with `tools/release.sh`; JayOS builds the release its recipe pins, not
`main`. `docs/RELEASES.md` has how one is made, what is refused, and the
owner's steps before the first. None exists yet: `.github/allowed_signers`
holds no key until the owner adds one.

## Licence

MIT; see `LICENSE`. The captured frames and the protocol reference are
mos9527's, under MIT; `THIRD_PARTY.md` lists everything that came from
elsewhere and what is only cited.
