# The protocol, the document and the socket

What this package speaks to a headset, what it publishes, and what it takes on
its socket -- at the level of a map. The code is the detail: `src/mdr/frame.rs`
for the wire, `src/mdr/payload.rs` for what a payload means,
`src/mdr/encode.rs` for what this port is willing to send, `src/sony/state.rs`
for the document and `src/sony/command.rs` for the command language. Each
cites the reference by line. `tests/fixtures/mdr/PROVENANCE.md` is the corpus,
and `docs/EVIDENCE.md` is what every claim stands on.

Three sections below MOVED HERE from jayos when the integration did, verbatim,
from jayos commit `16acd81`: they are history, and are kept as written --
except where plan 23 found a statement in them wrong, which carries a
bracketed note saying so and what is true now, as `PROVENANCE.md` does.
**Paths in them are jayos's at that commit**: `rust/crates/qs-hw/src/X` there
is `src/X` here, a bare `sony/...` or `mdr/...` is under `src/`, and line
numbers are that commit's. jayos scripts they name (`sony-daemon.sh`,
`tools/test-*.sh`) are still jayos's.

## The wire, in one screen

A frame is `3e <type> <seq> <length:4, big-endian> <payload> <checksum> 3c`,
with `3c`, `3d` and `3e` inside it escaped as `3d` and the byte with bit 4
cleared. The checksum is the byte sum of type, seq, length and payload. An
ACK (type `01`) carries `1 - seq` of the frame it acknowledges and no payload,
and is never itself acknowledged. The link is stop-and-wait: one data frame in
flight, resent at the same sequence until it is acknowledged or given up on.

Two command tables share the payload's first byte and disagree on almost
every value: DATA_MDR (`0c`) is table 1, DATA_MDR_NO2 (`0e`) table 2. A
payload is `<command> <inquired type> <body>`. Within a family the commands
run GET, RET, SET, NTFY at base+0..3, and a decoder is chosen by `(command,
inquired type, length)` -- the type alone does not pick a struct, and where
that key still names two structs the decoder reads only what they agree on
or refuses by name.

A headset says what it can do in two support-function lists, one per table,
and this port asks only what those lists justify (`encode::feature_gets`).

## The document and the socket

`contract/` is the machine-checked form: a real document (`state.example.json`,
the corpus replayed through the daemon's own engine), every key path it can
carry (`keys.txt`), and the command language (`commands.txt`). jayos vendors it.

This daemon's additions to the C++ daemon's document come after `connection`
and before `last_error`, top-level, so the reference's own objects keep their
shape: `general_settings`, `paired_devices`, `ambient_range`,
`dsee_available`, `voice_guidance`, `safe_listening`, `pairing_mode` and
`alert`. `tests/sony_contract.rs` takes them out and holds what is left to the
C++ daemon's document byte for byte. The command language adds four verbs --
`eq-band`, `clear-bass`, `general` and `alert` -- and the rest is the C++
daemon's (`tests/sony_contract.rs` parses the first two as ours, and
`sony::command` marks `alert` so).

### What the daemon shares with `sony-headphones-linux`

`qs-sonyd` is written to be read by anything written against the C++ daemon,
and `rust/crates/qs-hw/tests/sony_contract.rs` holds it to that byte for byte.
It needs `bluetoothd`: `bluez` installed and `bluetooth.service` enabled, which
`bootstrap.d/80-desktop.sh` offers together, because `bluez-utils` alone gives
a `bluetoothctl` that finds no controller and every helper then sees a machine
with no radio.

* **The document**, `${XDG_STATE_HOME:-~/.local/state}/sony-headphones/status.json`,
  schema version 1, ending in exactly one newline. The words are the C++
  daemon's: a battery's `charging` is `no`, `yes`, `complete` or `unknown`, and
  the Headphones page lights the charging mark on `yes` alone; `dsee_type` is
  `standard`, `hx`, `hx_ai` or `ultimate`; `codec` is `sbc`, `aac`, `ldac`,
  `aptx`, `aptx_hd`, `lc3` or `unknown`; `clear_bass` and the five `bands` are
  signed, -10..10 with 0 flat, split the way libmdr splits the six steps a
  headset sends (the first is clear bass). `device.protocol` is the generation
  of the MDR service the daemon dialled: 2 for the one a WH-1000XM5 advertises,
  1 for the XM4-and-older one, 0 while nothing is connected.
* **One key the C++ daemon does not have**, `general_settings`, written between
  `connection` and `last_error` so a reader walking the reference's keys in
  order sees them in the reference's order with one appended rather than
  `last_error` moving. It is an ARRAY and not named fields, because the headset
  decides what is in it: each entry is
  `{key, name, summary, slot, enabled}` where `name` is the device's own word
  (`MULTIPOINT_SETTING`), `key` is that word lowercased with `_SETTING` trimmed
  (`multipoint`), `slot` is 0xD1..=0xD4 as a number, and `enabled` is `true`,
  `false` or **`null`** — null meaning the capability reply has named the
  setting and the param reply has not yet said whether it is on, which is a
  real state and not the same as off. The entries are in slot order, never
  arrival order, so a reader may bind to a position. A WF-1000XM5 fills it with
  sidetone and multipoint; a WH-1000XM5 with a touch panel and multipoint, so
  match on `key` rather than on index.
* **A second key the C++ daemon does not have**, `paired_devices`, written
  between `general_settings` and `last_error` for the same reason and in the
  same place — appended, so a reader walking the reference's keys still sees
  them in the reference's order. It is the headset's own list of the machines it
  is paired with, read off MDR table 2, and present only where the device
  advertises `paired_device_management`. Each entry is
  `{address, name, status, detail}`: `address` is the peer's Bluetooth address
  (`E8:9C:25:5D:B8:19`) and identifies the entry, because names are not unique —
  one capture carries two `KUWABARA`; `name` is what that machine calls itself;
  `status` is 0 when the headset is not connected to that peer and non-zero
  (`0x01` or `0x02` observed) when it is, which on a multipoint headset is how
  the active links are told from the merely paired. `detail` is three bytes the
  decoder keeps to round-trip the message and has named none of — possibly
  codec, profile mask or signal figure — so the Headphones page draws the name,
  the address and the connected state and **nothing** from `detail`, the way
  its own header forbids a field that claims a meaning the daemon does not have.
* **The socket.** `status` answers `ok ` and the document on one line, as the
  C++ `daemon_main` does. `panel-open` and `panel-close` are accepted and
  ignored; the C++ daemon dials only after `panel-open`, so the Headphones page
  sends it when it opens and `panel-close` when it is hidden or closed.
* **Three verbs the C++ daemon does not have**, `eq-band <1..10> <-10..10>`,
  `clear-bass <-10..10>` and `general <setting> on|off`. *[Four since plan 23
  added `alert yes|no`.]* Extending the command
  language is allowed where
  narrowing it is not: the contract is that a client written against the
  reference gets the same answer here, and a verb it never sends cannot break
  that.

  The first two are the one sound control anybody has shown to survive a
  reconnect on this device family — unlike `eq-preset`, `connection` and the
  auto-power-off timer, which are acknowledged and discarded (read evidence,
  from a second Sony implementation's `docs/protocol.md`; nothing here has
  spoken to a headset). Both send the SAME message, `EqEbbSetParamEq`, which
  carries the preset and the whole band array with no short form — so a band
  edit restates clear bass and a clear-bass edit restates the bands, not by
  discipline but because the encoder can do nothing else. The bounds are the
  device's, taken from the length of the array it last sent: five bands run
  -10..10 and ten run -6..6, and a length neither libmdr nor Gadgetbridge names
  is refused rather than written at a guessed bias. The Headphones page shows
  the curve and offers no control, which is a design decision its own comment
  argues; these verbs are how it is changed.

  `general` is the third, and the ONLY verb in the grammar whose word argument
  is not matched against a closed set — the headset names its own settings, so
  the parser bounds the shape (lowercase ASCII, digits and underscores, at most
  64 bytes) and `mdr::encode` refuses anything the device has not actually
  named, as `NotRead` rather than `NotAdvertised`: "your headset has not told
  us about that" is a different answer from "your headset does not have that".
  The name to use is a `general_settings[].key`, so `general multipoint off` on
  the earbuds and `general touch_panel off` on the over-ears. Unlike the curve,
  this one HAS a control: the Headphones page draws one switch per entry,
  labelled from the device's own word, greyed while `enabled` is `null`.
* **The log.** The daemon's stderr goes to `daemon.log` beside `status.json`;
  `sony-daemon.sh start` first moves the previous run to `daemon.log.1`. The
  lines worth reading are `qs-sonyd: no decoder for command … kind … len …` —
  a message the port has no decoder for, which is what extending it for an
  untested headset starts from. The Headphones page names the file when the
  daemon is switched on and not running.
* **Recovery.** A frame the headset does not acknowledge within 1.5 s is
  resent twice at the same sequence — Gadgetbridge's init-retry rule
  (`initSendRunnable`, 1250 ms between sends) — then the link is dropped and
  redialled, as the C++ daemon's `failConnection` does after libmdr's 3 s ACK
  wait. A client waiting on that frame is answered at the first expiry, which
  is inside the daemon socket's 2 s dispatch wait and `qs-sony`'s 3 s
  per-operation timeout.

  **This used to end "(libmdr itself never retransmits)", which is wrong.**
  A second Sony implementation's `docs/protocol.md` runs libmdr
  against real XM5s and reports the opposite from its own logs: "a command that
  goes unanswered is resent with the sequence bit flipped, and the library waits
  a full second before each retry" — the `FIXME-ACK Timeout` lines in its
  journal, which it counts and publishes as `retransmits`. So the retry rule
  above is not a layer libmdr lacks; it is a **second** one, with a shorter
  deadline and the same sequence number rather than a flipped one. That is
  still the right rule for this daemon — 1.5 s is inside the client timeouts
  where 3 s is not — but the reason for it is "sooner", not "at all". Their
  observation that every retransmission they saw succeeded on the second
  attempt points at a sequence-number desync rather than a flaky link, and is
  unexplained by either implementation.

Two things are not shared. The C++ daemon removes `status.json` when it exits;
`qs-sonyd` leaves its last document behind (`qs-sonyd once` exists to do
exactly that), so `sony-daemon.sh stop` removes the file itself, and a daemon
killed any other way leaves a file behind — which `qs-sony` reports as
`"reason":"stale"` rather than as the last thing the daemon said, because it
first checks that a daemon is there: the socket lock `qs-sonyd` holds for its
life, or (for the C++ daemon, which takes no lock) the socket answering.
And the Headphones page's lifecycle strip goes through `sony-daemon.sh`, which
knows only `qs-sonyd`: with the C++ daemon alone the page reports no daemon
installed and reads nothing, while `qs-sony` and the doctor recognise
`sony-headphonesd` and read whichever daemon's document is there.

## History

### What a second Sony implementation's own notes were worth

A second Sony implementation is a Python daemon around `libmdr` from
`mos9527/SonyHeadphonesClient`. Measured on 2026-09-14 — 1,476 lines of Python,
2,925 counting its QML, shell and PKGBUILD, against 8,198 lines under
`qs-hw/src/{sony,mdr}` and the two binaries and 5,709 lines of tests for them.
**No code was taken and none was worth taking.** Those are dated measurements,
not enforced counts: `tools/test-counts.sh` deliberately holds no number about
another repository. What it has that this tree cannot have is a WH-1000XM5 and a
WF-1000XM5 on a desk, so its `docs/protocol.md` and `docs/hardening.md` are
empirical where everything here is read. Five things came out of it, in
descending order of how much they mattered: *[six are numbered below; the
sixth, the paired-device list, was added after this sentence was written.]*

1. **Two holes in this tree's own filesystem handling**, from
   `docs/hardening.md`, "The runtime directory" — reported there against their
   daemon and present in ours. `sony::ensure_private_dir` followed a symlink:
   `DirBuilder::recursive(true)` treats EEXIST as success after a `stat` that
   resolves the link, and `fs::set_permissions` then chmods the link's target.
   **Executed**, not read: a 0755 directory reached through a symlink came back
   0700, and the daemon would have published `status.json` inside it.
   `qs_video::control` had the identical two lines. And `serve::bind` unlinked
   whatever wore the socket's name rather than only a socket. Both fixed, both
   with a test that fails against the previous code —
   `a_symlinked_directory_is_refused_rather_than_chmodded` (twice, once per
   crate), `something_at_the_socket_path_that_is_not_a_socket_is_not_deleted`,
   `only_a_socket_is_cleared_from_the_socket_path`. **What is not fixed**: the
   check is not atomic with the path-based calls that follow it; upstream holds
   the directory descriptor and does every later operation `dir_fd`-relative,
   and that is a larger change. This closes the plant-it-first case, which is
   the one `runtime_dir`'s documented `/tmp` fallback actually offers.
2. **Two controls we were offering that the device discards, and one we were
   not offering that it keeps.** Their reconnect test — write, restart the
   daemon, read from a fresh session — established that an equaliser preset
   write is acknowledged within 25 ms and then ignored, as are connection
   priority and the auto-power-off timer, while the five bands and clear bass
   survive. **Read evidence, theirs, unreproducible here.**
   `HeadphonesPage.qml` no longer draws preset buttons, and
   `qs-sony send eq-band` / `send clear-bass` now exist — the same
   `EqEbbSetParamEq` message the preset verb already sent, with the steps
   replaced instead of the preset. Their hardest-won workaround is not needed
   in this port and the encoder says so: libmdr's staging lets half an
   equaliser change go out, so their `write_eq()` restates bands and clear bass
   every time; here the message has no short form, so the whole array goes or
   nothing does. **Executed**, against the captured curve: five mutations on
   the encoder and two on the page check, each caught, including one that
   escaped its first attempt — replacing the device's bound with a constant was
   caught on the negative edge alone, by an unrelated `u8::try_from`, while
   +10 at a bias of six sailed through as a legal byte four steps past the
   maximum.
3. **One place their document would have made this tree worse.** Their
   `CHARGING` enum is a rotation of ours, annotated in their own source with
   the C header it came from — `libmdr`'s struct field, not the wire byte. Two
   captured packets in `qs-hw/tests/fixtures/mdr/` settle it our way, and
   `PROVENANCE.md` now says so, so that a later reader who finds their document
   does not "correct" a working decoder into a bug.
4. **Two bugs of ours their shape made visible**, neither of which they
   report. An orderly disconnect published `phase: "error"` — their "a session
   object is not a link" section is about the opposite failure, their library
   answering reads from cache long after the headphones are off, and reading it
   is what sent us to look at what ours does when the link ends. And
   `qs-sonyd once` treated "nothing awaiting an acknowledgement" as "the
   headset has finished answering", so a one-shot run could publish a document
   saying `connected: true` with no battery in it. Their sentence is the one
   that names the window — "the handshake completing does not mean the state
   has arrived … values land over the following second or two" — and the
   number in `ONCE_SETTLE` is theirs. **Executed**: the gap is reachable
   against our own 70 captured frames
   (`nothing_outstanding_does_not_mean_the_headset_has_finished_answering`),
   and mutating the fix into the obvious-but-wrong shape — settle on the exit
   condition, not on the wait — made the daemon hang past 120 seconds where
   the suite otherwise finishes in 1.5.

5. **A capability we had been asking for and then dropping on the floor.**
   `encode::feature_gets` has sent a `GENERAL_SETTING_GET_CAPABILITY` and a
   `GET_PARAM` per advertised slot since the support tables were decoded, and
   nothing read the four replies in the corpus — all four arrived as
   `Note::Unread`. Their `docs/protocol.md` is what named the family and said
   which of its settings are writable. The decoders, the `general_settings`
   array in the document, the `general <setting> on|off` verb and the rows on
   `HeadphonesPage.qml` are this tree's.

   **The device names these and we do not**, which is the whole shape of the
   feature: our captured WF-1000XM5 answers `SIDETONE_SETTING` and
   `MULTIPOINT_SETTING`, and their WH-1000XM5 answers `TOUCH_PANEL_SETTING` in
   the slot ours puts sidetone in. A port with a table of names would be right
   on one model and silently wrong on the other, so the strings travel from the
   wire into the document and the page labels its switches from them.

   **Executed**, against the four captures: fixtures 000031/000035 (sidetone,
   on) and 000039/000043 (multipoint, off) decode by hand to what the decoders
   produce *[and the polarity was backwards: `GsSettingValue` is ON = 0x00
   (T1:956), so 000035 is sidetone OFF and 000043 multipoint ON -- which
   000077's two connected peers had said all along. Plan 23 inverted both the
   read and the write (`7f6e449`); `docs/OUTSTANDING.md`, item 6]*, and
   twenty-two mutations were run — thirteen on the Rust and nine
   on `tools/test-sony-daemon.sh` — each caught in the end. Five of the
   thirteen escaped their first attempt and are the reason four tests exist
   that otherwise would not (the last two share one): a body the two names do
   not account for, a value that is neither 0x00 nor 0x01, an NTFY not
   canonicalised to its RET, a slot arriving out of order, and `enabled`
   published as `false` rather than `null` before the param reply lands. One
   of the nine escaped too — a switch drawn from the click rather than from
   the daemon's reading went unnoticed — and fixing that added the two
   assertions about how a row is bound. **Read evidence, theirs**: that the write is honoured. The
   frame's shape is ours — 000035 is the SET's field-identical RET twin, so the
   emitted frame decodes back through the real decoder to what was asked.

   **What is weaker than the rest of the encoder**, said where it is done: the
   frame's third byte is written as a literal `0x00` rather than echoed from
   the reading, because `Facts` keeps only whether the setting is on. Both
   captures carry `0x00` and the reference names no field there, so the corpus
   cannot tell a constant from an echo. *[It does name one:
   `GsSetParamBoolean` (T1:4323) declares the byte `GsSettingType settingType`,
   BOOLEAN_TYPE = 0x00 (T1:951). The encoder now writes it by that name, and
   the decoder refuses a setting of any other type.]*

6. **The paired-device list, decoded and then shown.** MDR table 2 carries the
   headset's own list of the machines it is paired with; fixture 000077 held it
   and nothing read it — it arrived as `Note::Unread`, the general settings'
   position one family earlier. The decoder, the `paired_devices` array in the
   document (each entry `{address, name, status, detail}`), and the read-only
   PAIRED DEVICES section on `HeadphonesPage.qml` are this tree's, gated on the
   `paired_device_management` capability the same table advertises.

   **Executed**, against the capture: fixture 000077 decodes through the real
   decoder to the entries the document publishes, `mdr_payload.rs` pins the
   corpus at 19 understood and 50 unhandled with the list among the understood
   *[25 and 45 since plan 23]*,
   and `sony_contract.rs` holds the new key in the document's byte-for-byte
   shape. `tools/test-headphones-page.sh` asserts the page reads the field,
   gates on the capability, and draws nothing from the `detail` bytes; three
   mutations were run against it — the gate removed, a `detail` read added, the
   field read removed — each caught.

   *The page half of this item -- whether the Headphones page draws the list --
   stays in jayos's `docs/REVIEW.md`, where the page is.*

**Declined:** a finding that our discovery gate would never dial a real XM5.
`link::MDR_SERVICE_V2` (`sony/link.rs:44`) is
`956C7B26-D49A-4BA8-B03F-B17D393CB6E2`, byte for byte the UUID their
`docs/protocol.md` says the device answers on and nothing else does.
Acting on it would have meant probing RFCOMM channels on arbitrary Bluetooth
devices.

### The `qs-sony` environment race

Moved from jayos's `docs/REVIEW.md`, where it was the third of three things
read and not executed. The three `Env::hold()` calls it describes, and the
greps that fail if one goes, came here with the code: the greps are
`tests/shell/test-source-guards.sh`.

3. **The `qs-sony` environment race was diagnosed from the source and has not
   been reproduced on demand.** `a_daemon_is_running_while_it_holds_the_lock_or_answers_the_socket`
   failed about twice in eight workspace runs and passed in isolation every
   time; the reading is that `Command::spawn` walks `environ` while another
   thread's `setenv` reallocates it, and `sony/mod.rs:60-66` then falls back to
   a machine-global path. Three `Env::hold()` calls serialise the readers
   against the writers, and `tools/test-sony-daemon.sh` now fails if one goes.
   **The suite passing afterwards is not proof**: it passed most of the time
   before. What is solid is that the SAFETY comment on `Env::set` asserted
   something false — it claimed nothing else read those variables concurrently,
   which was about the wrong object, since the hazard is the shared array and
   the three `capture` tests read all of it. That claim is now correct.
