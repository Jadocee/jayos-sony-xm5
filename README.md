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
| `src/bin/qs-sonyd.rs` | The daemon. Dials the connected headset, publishes `$XDG_STATE_HOME/sony-headphones/status.json`, answers commands on a private Unix socket. |
| `src/bin/qs-sony.rs` | The client: `status`, `devices`, `send <command…>`, `available`, `replay <dir>`. |
| `src/support/` | Argument parsing, JSON on stdout and bounded subprocesses -- a copy of three jayos files; `src/support/mod.rs` says why a copy. |
| `scripts/sony-daemon-process.sh` | Starts and stops the daemon, keeps its stderr, removes its document on stop. Knows nothing about whether it *should* run; that is the shell's setting. |
| `contract/` | What JayOS tests its side against: a real state document, every key it carries, the command language, the model prefixes. Generated; see `contract/README.md`. |
| `packaging/arch/PKGBUILD` | Builds and installs this checkout. |

## Installing

On Arch, from a checkout:

    cd packaging/arch && makepkg -si

That installs `qs-sonyd` and `qs-sony` to `/usr/bin` and the process script to
`/usr/lib/jayos-sony-xm5/sony-daemon-process`. JayOS carries a recipe of its
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

## Licence

MIT; see `LICENSE`. The captured frames and the protocol reference are
mos9527's, under MIT; `THIRD_PARTY.md` lists everything that came from
elsewhere and what is only cited.
