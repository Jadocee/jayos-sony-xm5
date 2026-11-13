# Sony WH-1000XM5: what the integration still lacks — **moved to jayos-sony-xm5, partly done**

**Moved here from JayOS on 2026-11-13**, by its owner's call: "Move the
Sony-xm5-outstanding plan to jayos-sony-xm5 repo". It was JayOS's
`docs/development/plans/23-sony-xm5-outstanding.md`, plan 23 of that tree's
index, and is kept below as it stood there, word for word after this note.
`docs/OUTSTANDING.md` is its successor and says where each item stands now;
this is the analysis that list was worked from, with its own corrections.

**Its paths are JayOS's**, as it was written there: `tools/test-page-layout.sh`,
`sony-daemon.sh`, `HeadphonesPage.qml` and the rest name files in
Jadocee/jayos. `tools/acceptance.sh`, `qs-sony` and `qs-sonyd` are this
repository's. It is not among the documents `tests/mdr_citations.rs` holds to
the corpus: its quotes are the record of what was read then, and the
corrections below say which were wrong.

**The programs it names were renamed the same day**, by the owner's call
("Rename the qs-sony items"): `qs-sony` is `jayos-sony` and `qs-sonyd` is
`jayos-sonyd`. The plan keeps the names it was written with.

**Moved, then settled as far as a machine with no Bluetooth allows — 2026-10-02.**
The Sony integration left this tree. The MDR protocol, `qs-sonyd`, `qs-sony`,
their tests and the 70-frame corpus are the package `jayos-sony-xm5`
(<https://github.com/Jadocee/jayos-sony-xm5>), which this tree builds from
`pkgbuilds/jayos-sony-xm5/PKGBUILD` and finds in `/usr/bin`. What stayed is the
shell's half: the Headphones and Devices pages, the `sonyDaemon` setting and the
setting verbs of `sony-daemon.sh`, the doctor, and the generic Bluetooth,
WirePlumber and BlueZ configuration. A contract the package generates and this
tree vendors (`tools/fixtures/jayos-sony-xm5-contract/`) holds the two halves
together.

Every item below was then worked in the package. Its `docs/OUTSTANDING.md` is
where each now stands; in short:

| | what it was | what it is now |
|---|---|---|
| 1 | over-ear battery path | **code** — asked by the support table, published as `battery.main`; awaits a WH capture |
| 2 | ambient level bound to 1..=20 | **code** — the headset's stated range cut to the command language's, published as `ambient_range`, which the page draws its steppers from; awaits a WH capture |
| 3 | NC/ASM SET at unpinned types | **code** — six layouts read and three written, each on stated grounds; awaits which type a WH answers at |
| 4 | connection priority refused | **code** — read and written at 0x00 and 0x02, `quality` labelled against the header; the LE Audio form stays refused; awaits a write that sticks |
| 5 | playback's offset 2 | **code** — libmdr's ENABLE, gated on the headset's own play status; awaits a command the headset acts on |
| 6 | general setting's third byte | **fixed, and not where this said** — the value's polarity was inverted both ways; awaits a write read back |
| 7 | no transport testing | **code** — held to BlueZ's structs; the kernel check is written and needs a Bluetooth stack |
| 8 | TOCTOU | **done** before the move; the socket's shutdown now unlinks by descriptor too, the package's and `jayos-video`'s here |
| 9 | `noise_control_button` | **done** before the move |
| 10 | `AUDIO_RET_STATUS` at UPSCALING | **decoded**, as libmdr reads it; published as `dsee_available` |
| 11 | three table-2 families | **asked and decoded** — each refusal had an answer in the header or in libmdr |
| 12 | first match wins | **refused by name** when two slots answer |
| 13 | page never rendered | **drawn, but not on a screen** — `tools/test-page-layout.sh` builds it connected to the package's documents, with and without a question pending, under a `qmltestrunner` stand-in here; a screenshot still awaits a compositor |
| 14 | write persistence | **alerts**, so a write can be confirmed: the page shows the headset's question and answers it; the persistence check is a step of the package's capture session |

**Where this document was wrong**, left below as written and corrected here, so that
its analysis is kept. Item 6's third byte is `GsSettingType` -- the
reference names it, and 0x00 is BOOLEAN_TYPE -- and the defect was the fourth:
`GsSettingValue` is ON = 0x00, which this tree read and wrote the other way
round. Item 10's "no struct in the reference declares the key" misread an
unpinned default (`CODEGEN Ignore`, not `EnumRange`). Item 11's "suggests five"
cites t2 0x43, which the captured table does not carry; it carries 0x42. "36
Rust test files under `rust/crates/jayos-hw/tests/`" was not true even then: at
`16acd81` that directory held 13, twelve of them the Sony suites that moved,
and it holds one. And every `mdr/…` and `sony/…` path below, with its line
numbers, is where that code was under `rust/crates/jayos-hw/src/` until `16acd81`
-- the package's `src/mdr/` and `src/sony/` now; the citations into
`docs/reference/DEPENDENCIES.md` and `docs/REVIEW.md` had drifted before then, and the
prose they meant went with the code to the package's `docs/PROTOCOL.md`.

Nothing in this note has run on hardware either. The package's
`tools/acceptance.sh` is the session that would, and its `qs-sony settle` says
which of these a capture closes.

## The ask

Catalogue every outstanding item in the Sony headphone integration so the gaps
are visible in one place rather than scattered across `docs/REVIEW.md`, source
comments, and the test-fixture provenance notes. Most of what follows shares a
single root cause: the entire capture corpus is from a **WF-1000XM5** (earbuds),
and there is no Bluetooth adapter or WH-1000XM5 in the build environment.

## What already exists

The integration is extensive. It is enumerated here so a reader knows what the
list below is the remainder of, not a fresh start:

- **MDR protocol**, natively in Rust: `rust/crates/jayos-hw/src/mdr/` — frame,
  payload (19 of 70 corpus frames decoded), encoder (eight verbs, four
  refusals). 70 captured frames from a real WF-1000XM5.
- **Sony daemon** (`qs-sonyd`): `rust/crates/jayos-hw/src/sony/` — RFCOMM
  session, state publisher, command parser, socket server. Off by default;
  lifecycle via `sony-daemon.sh`.
- **CLI** (`qs-sony`): status, devices, send, available.
- **HeadphonesPage.qml**: noise control, equaliser, battery, DSEE,
  Speak-to-Chat, general settings, paired devices. Every control gated on 27
  capability flags.
- **WirePlumber**: three configs — LDAC/AAC codec list, autoswitch off, A2DP
  auto-connect, LDAC quality auto on the card, 21 ms buffer.
- **BlueZ**: `Experimental = true`, fast reconnect and dual controller mode,
  in `system/bluetooth/main.conf`, which `install.d/60-system.sh` installs
  whole as `/etc/bluetooth/main.conf`. This line used to name a drop-in at
  `/etc/bluetooth/main.conf.d/50-quickshell.conf`, and that was not in place
  in any useful sense: bluetoothd reads `main.conf` alone and has no
  `main.conf.d` (read in BlueZ's `src/main.c`, `load_config`), so none of the
  three was ever applied.
- **Bootstrap/installer**: `libldac` offered when missing, `bluetooth.service`
  enabled.
- **Doctor**: two Bluetooth sections checking autoswitch, HSP, LDAC placement,
  codec library, buffer size, USB autosuspend, Wi-Fi band.
- **Tests**: `tools/test-bluetooth.sh`, `tools/test-sony-daemon.sh`,
  `tools/test-headphones-page.sh`, plus 36 Rust test files under
  `rust/crates/jayos-hw/tests/`.
- **Documentation**: `docs/features/AUDIO.md`, `docs/reference/DEPENDENCIES.md:171-215`,
  `docs/REVIEW.md:116-255`.

## Items needing hardware captures

These cannot be settled by reading references or writing code. Each needs one
or more captured MDR frames from real hardware, and the item says which.

1. **Over-ear battery path has no fixture.** The four plain battery decode
   paths (`POWER_RET_STATUS` at 0x00 and 0x08) in `mdr/payload.rs:37-44` are
   marked `HEADER ONLY` — transcribed from the reference with nothing seen
   sending them. These are the variants a WH-1000XM5 (single main battery)
   uses; the corpus has only the `*_WITH_THRESHOLD` variants (0x09, 0x0A) from
   the WF-1000XM5 (left/right/case). **Settled by:** one `POWER_RET_STATUS` at
   0x00 or 0x08 from a WH-1000XM5.

2. **Ambient-level bound hardcoded 1..=20.** `sony/command.rs:165-174` clamps
   the ambient sound level to the C++ daemon's compatibility range. The comment
   says: "the real bound is per-device and per-mode, delivered as a `Range {
   min, max, step }` in a `NCASM_RET_CAPABILITY` reply no capture in this
   project contains". A device whose scale runs to 19 or 32 would be clamped
   incorrectly. **Settled by:** one `NCASM_RET_CAPABILITY` (command 0x61)
   frame.

3. **NcAsm SET at unpinned inquired types.** `mdr/encode.rs:514-528` refuses
   with `Refusal::Ambiguous` at type 0x13 (two structs claim the key and read
   offset 6 differently) and with `Refusal::UnknownVariant` at anything other
   than 0x17 (the one the WF-1000XM5 corpus carries). A device answering NCASM
   at a different type would be unable to have its noise mode changed.
   **Settled by:** a device answering NCASM at 0x01, so the length
   disambiguates the struct.

4. **Connection priority encoder refuses.** `mdr/encode.rs:474-483` returns
   `Refusal::Undocumented`: the SET's fourth byte (`value2` in
   `AudioSetParamConnectionModeClassicAudioLeAudio`) is unnamed in the
   reference, and no `AudioRetParam*` is pinned at its inquired type to confirm
   a write. **Settled by:** one WH-1000XM5 support table plus an
   `AUDIO_RET_PARAM` at the advertised connection type.

5. **Playback encoder offset 2.** `mdr/encode.rs:957-994` sends
   `SetPlayStatusPlaybackController` at `Evidence::HeaderOnly`. Offset 2 is an
   `EnableDisable status` with no `EnumRange`, no comment and no RET twin to
   mirror against. The encoder sends 0x00 (ENABLE, inverted convention). This
   is the one byte in the module that is neither named by a verb nor echoed from
   a reading. **Settled by:** a playback control sent and observed on hardware.

6. **General setting third byte literal 0x00.** `mdr/encode.rs:729-737` — the
   frame's third byte is written as 0x00 rather than echoed from the reading,
   because `Facts` keeps only whether the setting is on. Both captures carry
   0x00 and the reference names no field there. A device answering with a
   different byte would be written back as 0x00. **Settled by:** any capture
   showing a non-0x00 byte at that position.

7. **No Bluetooth transport testing.** `sony/link.rs:14-19` — `connect` against
   a Bluetooth address has never been tested against a real adapter. The byte
   order in `rfcomm_sockaddr` is the one thing that cannot be checked without
   one. Everything either side of the transport — address encoding, timeout,
   read/write behaviour, shutdown — is tested over Unix sockets. **Settled
   by:** one successful RFCOMM connection to a real device.

## Items that are code changes

These can be addressed without hardware. **Both have since been settled — see
the Status section at the end**, which is where the twelve remaining items comes
from. The present tense below is to the tree before that change and is left
alone: the analysis is what is worth keeping, and rewriting it into the past
tense would lose the reasoning that produced the fix. A reader starting from the
top of this section would otherwise begin work that is done, which is the
failure this note exists to prevent.

8. **TOCTOU in `ensure_private_dir`** — **settled; see "Item 8 is done" under
   Status.** `sony/mod.rs:100-108` — the
   O_DIRECTORY|O_NOFOLLOW check and fchmod are not atomic with the path-based
   `atomic_write` and `serve::bind` calls that follow. The current fix closes
   the plant-it-first case (a shared `/tmp`); it does not close a swap in the
   microseconds afterwards by something already running as the same user.
   Upstream's answer is to hold the directory descriptor and do every later
   create, replace and unlink with `dir_fd`-relative operations. That is a
   larger change deferred from the fix that closed the initial hole.

9. **`noise_control_button` capability flag permanently false** — **settled;
   see "Item 9 is already addressed" under Status. Of the two endings this item
   proposes, the second was taken: documented as a placeholder, because removing
   it would break the document schema contract.**
   `sony/state.rs:92` — this flag exists in the `Capabilities` struct and is
   published in every document, but no function type in either support-function
   table sets it. It will stay `false` on every device until a table entry is
   found that maps to it, and no reference names one. It should be either
   removed (breaking the document schema) or documented as a placeholder.

## Protocol knowledge gaps

Items where a frame exists in the corpus but the decoder does not understand it.

10. **`AUDIO_RET_STATUS` at `UPSCALING` decoded by nothing.**
    `mdr/payload.rs:1306-1312` — fixture `000097` (`e3 01 00`) arrives as
    `Reading::Unhandled`. No struct in the reference declares the key; the
    third byte is an `EnableDisable` by shape and nothing else.

11. **Three undecoded table-2 families.** `mdr/payload.rs:1115-1139` — the
    corpus carries frames from `VOICE_GUIDANCE`, `SAFE_LISTENING` and `PERI`,
    and each is a different reason it stays `Unhandled`:
    - `VOICE_GUIDANCE_RET_PARAM` at 0x01: a single capture in a single state
      cannot determine which of two bytes is the switch, or its polarity.
    - `VOICE_GUIDANCE_RET_PARAM` at 0x20: a volume on an unbounded scale.
    - `SAFE_LISTENING_RET_EXTENDED_PARAM` at 0x03: 0xFF is real in one
      protocol context and a sentinel in another; one frame cannot tell which.
    - `PERI_RET_STATUS` at 0x02: two bytes the state document has no field for.

12. **General setting key collision: first match wins, no tie-break.**
    `mdr/encode.rs:746-752` — if two slots had colliding keys (`X_SETTING`
    beside bare `X`), `general x` would silently address the lower slot. No
    capture, no reference and no device contains such a pair.

## Acceptance evidence gaps

Items that work but have never been confirmed end-to-end on hardware.

13. **HeadphonesPage has never been rendered on screen.** `docs/REVIEW.md:242-248`
    — there is no compositor in the build environment. Evidence is limited to
    `tools/test-qml.sh` compiling it and the grep suite. The PAIRED DEVICES
    section, the EQ sliders and the general-setting toggles have never been seen
    drawing.

14. **General setting writes rest on read evidence from another project.**
    `mdr/encode.rs:739-743` — that the write sticks is asserted by
    `delarosa1312/omarchy-sony-xm5`'s `docs/protocol.md`, not by this tree's
    own testing. The frame shape is verified by captures, but persistence of the
    write after a reconnect is their claim.

## What would close the gaps

Most items share one answer: **a WH-1000XM5 on a desk running `btmon`**, which
produces the captures the seven hardware items need and settles items 13 and 14
as a side effect. A capture session would produce:

| Capture | Items it closes |
|---|---|
| `POWER_RET_STATUS` at 0x00 or 0x08 | 1 (over-ear battery) |
| `NCASM_RET_CAPABILITY` (0x61) | 2 (ambient-level bounds) |
| NCASM at a second inquired type | 3 (unpinned NcAsm SET) |
| `AUDIO_RET_PARAM` at the connection type | 4 (connection priority) |
| Playback command issued and observed | 5 (playback offset 2) |
| A general setting frame with byte 2 != 0x00 | 6 (third-byte literal) |
| A successful RFCOMM connection | 7 (transport test) |
| Observed on the headphones page | 13, 14 (acceptance) |

The three protocol-knowledge items (10, 11, 12) do not need hardware; they need
a second capture of the same frame in a different state.

## Status

**Item 8 is done.** `ensure_private_dir` now returns the verified directory
descriptor, and `atomic_write` uses `openat`/`renameat`/`unlinkat` relative to
it; `serve::bind` uses `fstatat`/`unlinkat`/`fchmodat` relative to it and binds
the socket through `/proc/self/fd/<N>` so the kernel resolves the directory from
the descriptor, not the pathname. The same fix was applied to
`jayos_video::control`. All existing tests pass.

**Item 9 is already addressed.** `noise_control_button` is documented as
permanently false with its reason at `state.rs:201-207`. Removing it would break
the document schema contract pinned in `sony_contract.rs:101`. No action needed.

The remaining twelve items need hardware captures or a second frame in a
different state. **Nothing in this document has been run on hardware** — every
claim is read from the source files cited, the test fixtures, and
`docs/REVIEW.md`. The integration works against its own corpus and contract
tests; what it has not done is speak to a headset.
