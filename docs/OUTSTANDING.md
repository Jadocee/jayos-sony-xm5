# What is still outstanding

Dated 2026-10-02. This is the successor of jayos's plan 23
(jayos: `docs/plans/23-sony-xm5-outstanding.md`), which listed fourteen things
the Sony integration lacked when it lived in jayos. The integration moved here
and the plan was implemented as far as a machine with no Bluetooth adapter and
no headset allows. Each item below says what is code now, what it stands on,
and what would settle the rest.

`qs-sony settle <capture-dir>` answers the last column mechanically for every
item a capture can settle; `tools/acceptance.sh` runs the session that produces
that capture; `docs/CAPTURE.md` explains the session step by step.

**Nothing here has spoken to a headset.** Every "done" below is code with
tests; every test is against the WF-1000XM5 corpus, the reference header, or
synthetic bytes, and says which. A WH-1000XM5 has not been seen by any of it.

## The fourteen, in one table

| # | What was missing | Now | Grounds | Settled by |
|---|---|---|---|---|
| 1 | The over-ear battery path | asked and published (`a2dd6e3`) | header only; 0x00 corroborated by libmdr and Gadgetbridge, 0x08 by libmdr | a WH's `POWER_RET_STATUS` at 0x00 or 0x08 |
| 2 | The ambient level's bound | the headset's own range, cut to 1..=20 and published as `ambient_range` (`a2dd6e3`) | header only | a WH's `NCASM_RET_CAPABILITY` (0x61) |
| 3 | NC/ASM at another type | read at six layouts, written at three (`a2dd6e3`) | captured (0x17), reference client (0x19), third party (0x15) | which type a WH answers at |
| 4 | Connection priority | read at 0x00 and 0x02, written in the form answered (`6ea5259`) | `stable`: header only (0x02), third party (0x00); `quality`: against the header | a WH's connection reply, and a write that changes the mode and is read back |
| 5 | Playback's offset 2 | reference client; play status read (`6ea5259`) | reference client; the status byte captured | a pause or play the headset's own status then shows, on the same link |
| 6 | General-setting polarity | inverted, the way the header and libmdr have it (`7f6e449`) | captured inference and reference client | multipoint read on while two peers are connected |
| 7 | The socket address | held to BlueZ's structs and functions (`c272127`) | header only; kernel check written and ignored | `cargo test -- --ignored` on a machine with Bluetooth |
| 8 | Unlink by path | by descriptor and inode (`841a20a`) | a test that swaps the directory | done |
| 9 | `noise_control_button` | documented as a placeholder (in jayos, before the move) | -- | done |
| 10 | `AUDIO_RET_STATUS` at UPSCALING | read, published as `dsee_available` (`6ea5259`) | captured bytes, reference client meaning | what DISABLE looks like |
| 11 | Table 2's replies beyond the pairing list | asked and read (`855bb1e`) | captured bytes, reference client meaning | the same four readings from a WH |
| 12 | A setting name two slots answer to | refused by name (`7b0aae2`) | -- | done |
| 13 | The page never drawn | a live document to draw it against | -- | a screenshot |
| 14 | A general-setting write persisting | alerts, so the write can be confirmed (`c955a5d`) | header only | a write that changed the setting, still there after a reconnect |

## Each, at more length

**1. The over-ear battery.** A WH-1000XM5 advertises a single battery, not the
left, right and case the corpus's earbuds report. The table's function now
chooses the question (`22 00` for 0x20, `22 08` for 0x28) and the answer lands
in `battery.main`; a volunteered `POWER_NTFY_STATUS` moves it. Both layouts are
the header's, with the level at offset 2 and charging at 3: 0x00 as libmdr and
Gadgetbridge read it, 0x08 as libmdr reads it (Gadgetbridge's
`decodeBatteryType` has no 0x08). `an_over_ear_headset_publishes_one_battery_
from_either_header_form` holds it with synthetic bytes.

**2. The ambient level's range.** `NCASM_RET_CAPABILITY` gives a minimum, a
maximum and a step per ambient mode. It is asked beside the NC/ASM query, read
when it is well formed (a zero step, an inverted range, an unknown or repeated
mode are all refused), and published as `ambient_range` with `source:
"device"` -- cut to the 1 to 20 the command language takes whatever a headset
states, so every level the document offers is one the daemon accepts
(`the_published_range_is_the_one_in_force` sends each); without it the range
is the C++ daemon's 1 to 20 with `source: "compat"`. An `ambient-level`
outside the stated range is refused before it is sent. No client asks for
this reply and no capture holds one.

**3. NC/ASM at another inquired type.** Three projects report three types for a
WH-1000XM5 (0x15, 0x17, 0x19). The decoder reads each layout by type and
length; the encoder writes only the three it has grounds for, in the layout
the headset itself last answered in, and refuses the rest by name. Which one a
WH-1000XM5 actually uses is the capture's to say.

**4. Connection priority.** The plain form (0x00) and the LDAC form (0x02) are
read and written in the form answered; the LE Audio form (0x05) the captured
earbuds advertise stays refused over its unexplained `value2`. The 0x02 write
is a pinned, field-identical RET/SET/NTFY triple; the 0x00 write is mos9527's
PR #64, reported on a WF-LC900. Both forms pin the VALUE as well, to
CONNECTION_QUALITY_PRIOR, and libmdr's generated validation enforces it, so
`quality` is sent and read against the header -- tier `against-header` --
and `stable` alone stands where the forms do. Neither has been seen to stick.

**5. Playback.** The transport write sends what libmdr sends, ENABLE at offset
2. It is gated on the advertised controller and refused when the headset's own
play status says DISABLE. The play status is read at the two offsets both of
its colliding structs agree on, which also publishes `playback.status`; the
byte they disagree on is still never read.

**6. General-setting polarity.** `GsSettingValue` is ON = 0x00, and this port
read and wrote it the other way round. Fixture 000077's two connected peers
with multipoint read as "off" were the tell. Corrected both ways, with the
header and libmdr behind it. That tell, from a WH-1000XM5, is also what
settles it: a write read back cannot, because SET, RET and NTFY carry the same
`GsSettingValue` and a headset returns the byte it stored under either
reading.

**7. The socket address.** The byte-built addresses are held to the C structs
(offsets by `offset_of!`) and to transcriptions of BlueZ's `str2ba` and
`ba2str`. The one check that needs a kernel binds an RFCOMM socket and reads
the address back; it is written, marked `#[ignore]`, and fails here with
EAFNOSUPPORT.

**8. The socket directory.** Shutdown unlinks the daemon's socket through the
directory descriptor it bound under, and only if the inode is still its own; a
directory swapped after bind is not touched. jayos's `qs-video` had the same
shape and is fixed there.

**9.** Settled before the move: `noise_control_button` stays false and says why.

**10. Upscaling status.** `e3 01 00` (fixture 000097) is read as libmdr reads
it, ENABLE meaning DSEE can run, and published as `dsee_available`. The old
argument that no struct declares the key misread an unpinned default.

**11. Table 2.** No table-2 feature was ever asked about; only its support
list was. The table's functions now choose the questions, and all five
table-2 replies the corpus holds are read: the paired-device list, read
before plan 23, and the four readings it added -- voice guidance's switch and
language, its volume (-2 to 2), safe listening's raw level and cause, and
pairing mode. Each old refusal had an answer in the header or in libmdr; none
needed a second capture.

**12. Name collisions.** A setting name that more than one slot answers to,
by trimmed key or by full name, is refused with the slots listed.

**13. The page.** jayos's Headphones page has never been drawn on a screen.
`contract/state.example.json` is a real document to draw it against, and
`contract/state.alert.example.json` one with a question pending; the
screenshot itself needs a compositor.

**14. Persistence.** That a general-setting write survives a reconnect is
`delarosa1312/omarchy-sony-xm5`'s finding, not this package's. With
`qs-sonyd --alerts` the headset asks before a multipoint change, and `qs-sony
send alert yes|no` answers; jayos's page does not yet -- it reads no `alert`
until the jayos commit that re-vendors this contract. A drop of this link
while a yes to a disconnecting change is on the wire, or within five seconds
of the headset acknowledging it, is a clean close rather than an error; the
header does not say which link such a change drops, so a drop later than
that is what the transport says it was. The persistence itself is step 5 of
`docs/CAPTURE.md`.

## What a capture of the WH-1000XM5 settles

`qs-sony settle` needs the capture to name itself a WH-1000XM5 before it calls
anything settled: the same bytes from the corpus's earbuds are evidence about
the earbuds. Against the corpus it reports every frame item open, which is the
right answer. Against synthetic WH captures in `tests/capture.rs` it settles
exactly what each holds, and each item's check is also held to the captures
that must NOT settle it: a DSEE write for item 4, a status the headset would
send anyway for item 5, a write read back for item 6, a write of the value
already held for items 4 and 14. A capture that settles an item against this
package's reading -- multipoint read off with two peers connected -- is
reported `contradicted`, not settled.

## Run, and only read

Run here: `cargo test` (299 pass, 1 ignored), the shell suites in
`tests/shell/`, `tools/check.sh`, and `tools/acceptance.sh --dry-run`.

Read only: every frame this package sends to a headset, the RFCOMM and L2CAP
connections, `btmon`, the btsnoop importer against a trace a real adapter wrote
(its round trip is against traces built here), the kernel's `bind` and
`getsockname` on a Bluetooth socket, the page on a screen, and the package
installed by `makepkg`.
