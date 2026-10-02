# Captured MDR packets

70 frames, 1289 bytes, captured from a **Sony WF-1000XM5** on firmware
**6.1.0** over Bluetooth.

## Where they came from

Vendored verbatim from
[`mos9527/SonyHeadphonesClient`](https://github.com/mos9527/SonyHeadphonesClient)
at commit `db0ae2574d8f8e1407e8ea28a7ee87db9dde1c24`, path
`tests/WF-1000XM5-6.1.0/`. That project is MIT licensed (Copyright (c) 2026
mos9527, Amr Satrio and other contributors) and is the same library the
`sony-headphones-linux` daemon links as "libmdr".

They are copied **unmodified and unredacted**, and must stay that way. The
frames carry a checksum computed over the length field, and the escaping is
applied over the checksum — so editing any byte for tidiness changes the
checksum, and the round-trip test would then happily round-trip the edit. A
modified capture is not ground truth; it is a synthetic fixture wearing a
capture's name. `the_corpus_is_actually_there` pins the file count and the exact
byte total so that an edit fails the suite instead of passing it quietly.

The frames contain the capture device's Bluetooth address and model
identifiers, as upstream's own README warns. That is already public in the
MIT-licensed source above; nothing here was captured from this machine.

Three headers from that same commit are quoted throughout this file, and they
are three different files for a reason:

* `libmdr/include/mdr/ProtocolV2T1.hpp` — the table-1 messages, and the
  `*InquiredType` enums. This is what most of the port was transcribed from.
* `libmdr/include/mdr/ProtocolV2T2.hpp` — the table-2 messages: peripherals,
  voice guidance, safe listening.
* `libmdr/include/mdr/ProtocolV2.hpp` — the shared declarations the other two
  `#include`, among them `SupportFunction` and **both** `FunctionType` enums.
  Anything the T1 header uses but does not declare lives here, and reading only
  T1 is how a decoder ends up with one enum where the protocol has two.

## What they are

The filename encodes what upstream's recorder decoded from the bytes:

```
mdr-packet-<ms>-<counter>-rx.type-<TYPE>-<hex>.seq-<hex>.cmd-<COMMAND>-<hex>.bin
```

Because those fields are *derived from the bytes by another implementation*,
checking a decode against the filename is a second opinion rather than a
restatement — which is why `the_decode_agrees_with_what_the_recorder_named_the_file`
is worth running.

Every file is `-rx.`: captured **from** the headset. The counter has gaps
(1, 2, 5, 7, …) and those gaps are the transmitted half of the conversation,
which was not recorded. So the corpus can prove what this port *reads* and
almost nothing about what it *writes* — the exception being the ACK, which
carries no payload and is therefore one of exactly two fixed 9-byte strings,
both present here.

## Two things they do not cover

**The wrong ear.** The capture is a WF-1000XM5 (earbuds); this tree targets a
WH-1000XM5 (over-ear). Same protocol generation, different capabilities: the
buds advertise left, right and case batteries and no main battery, so the
over-ear's battery path has no fixture at all.

**Nothing below the frame.** There is no captured SDP exchange, no recorded
RFCOMM channel number, and no BlueZ reply. The transport is not tested by
anything here and cannot be, in an environment with no Bluetooth adapter.

## A correction worth keeping

The captured battery frames use the `*_WITH_THRESHOLD` variants, not the plain
ones, and a decoder written from the enum's first few names would mis-parse
every battery reading this device family sends.

`PowerInquiredType` (`libmdr/include/mdr/ProtocolV2T1.hpp:1323-1342`) lists
`BATTERY = 0x00`, `LEFT_RIGHT_BATTERY = 0x01`, `CRADLE_BATTERY = 0x02` — and
then, further down, `LR_BATTERY_WITH_THRESHOLD = 0x09` and
`CRADLE_BATTERY_WITH_THRESHOLD = 0x0A`. The WF-1000XM5 sends `0x09` and `0x0A`.

Decoded against `PowerNotifyStatusLeftRightBatteryThreshold`
(`ProtocolV2T1.hpp:6762-6783`), fixture `000131`:

```
23 09 | 00 00 64 00 64 64
       left level 0, left NOT_CHARGING, right level 100, right NOT_CHARGING,
       left threshold 100, right threshold 100
```

and against `PowerNotifyStatusCradleBatteryThreshold`
(`ProtocolV2T1.hpp:6709-6722`), fixture `000135`:

```
23 0a | 35 00 1e
       case level 53, NOT_CHARGING, threshold 30
```

Both are exactly the declared payload length. The plain variants carry four and
six bytes and would have read the threshold bytes as a further battery.

The lesson generalises: the payload layer must be transcribed per message from
`ProtocolV2T1.hpp` and checked against a fixture, not inferred from an enum
name. The frame layer could be ported from the framing rules alone; this layer
cannot.

### The charging byte: a published document disagrees with these fixtures

`delarosa1312/omarchy-sony-xm5` — the one tree with a real WH-1000XM5 — carries
this in `daemon/lib/mdr.py:78-82`:

```python
# mdr-c/Headphones.h
# Not a boolean: 1 means *not* charging. Reading it as truthy reports every
# idle headphone as charging.
CHARGING = {0: "unknown", 1: "no", 2: "yes", 3: "complete"}
```

`Charging::from_byte` in `mdr/payload.rs` has `{0: No, 1: Yes, 2: Unknown,
3: Charged}`. The two are **a rotation of each other**, and every value
disagrees. **Taken at face value that file would have us renumber a correct
decoder**, so it is recorded here rather than left for somebody to "fix" later.

The two fixtures above settle it. Both were captured from a headset that was
not charging, and both carry `0x00` in every charging position:

```
000131   23 09 | 00 00 64 00 64 64      left NOT_CHARGING, right NOT_CHARGING
                    ^^       ^^
000135   23 0a | 35 00 1e               case NOT_CHARGING
                       ^^
```

A device that was not charging sent `0x00`, so `0x00` is `No` and the mapping
here is right about the wire.

The disagreement is real but it is not about the wire, and their own first line
says so: `# mdr-c/Headphones.h`. Their daemon does not decode MDR — it calls
`libmdr` from SonyHeadphonesClient through ctypes and reads a **C struct
field**, so that table is libmdr's enum, not the protocol's. Ours is
transcribed from `ProtocolV2T1.hpp` and checked against the bytes above. Two
different numbering schemes, one name, and only one of them is what arrives on
the socket.

Nothing else in that tree is in doubt on this account — their
`XM5_SERVICE_UUID` at `daemon/lib/mdr.py:75` is `956C7B26-D49A-4BA8-B03F-`
`B17D393CB6E2`, byte for byte our `MDR_SERVICE_V2`.

Worth keeping for the shape as much as the fact: a protocol note written
against a library is not a protocol note, and the only thing that could tell
the two apart here was a captured packet whose ground truth was known.

## How a payload selects a struct, and where that goes wrong

The frame layer can be ported from the framing rules alone. The payload layer
cannot, and the reason is sharper than "read the header carefully": **the
header does not contain a dispatch table**. It contains 579 structs, each
carrying a `Command` default and most a second field naming an "inquired type",
and a decoder has to work out from those which struct a given payload is.

This was measured against `ProtocolV2T1.hpp` at commit `db0ae2574d8f` rather
than assumed.

### The type byte alone is not enough

`NcAsmInquiredType::NC_ON_OFF` (0x01) is the declared type of **five**
different `NcAsmRetParam*` structs, whose field counts are 4, 6, 8, 7 and 5.
They are not interchangeable: `NcAsmRetParamNcOnOff` has no ambient sound
level at all and `NcAsmRetParamModeNcModeSwitchAsmSeamless` has one at offset
7. A decoder keyed on `(command, type)` picks one of the five and is wrong four
times out of five.

Worse, the type defaults are not always the real discriminator. Both
`NcAsmRetParamModeNcModeSwitchAsmSeamless` and
`NcAsmRetParamNcModeSwitchAsmSeamless` declare `NC_ON_OFF`, which cannot be
true of both, while `NC_MODE_SWITCH_AND_ASM_SEAMLESS` (0x14) and
`MODE_NC_ASM_AUTO_NC_MODE_SWITCH_AND_ASM_SEAMLESS` (0x15) are declared by no
struct at all.

### Adding the payload length very nearly fixes it

Every field in a `MDR_DEFINE_TRIVIAL_SERIALIZATION` struct is one byte — the
file is under `#pragma pack(push, 1)` and every type in these structs is a
`UInt8` or a `UInt8`-backed enum — so the field count IS the payload length.

That is true of **444 of the 448** trivially-serialised structs. The four
exceptions, named so that the census below can be re-run and checked:

| struct | the field that is not one byte |
|--------|--------------------------------|
| `ConnectRetProtocolInfo` | `Int32BE protocolVersion` |
| `CommonSetParamConnectionEstablishedTime` | `Int16BE utcYear` |
| `UpdtNtfyParamTandemStartTransfer` | `Int32BE maxPacketSize`, `Int32BE offset` |
| `UpdtNtfyParamTandemExecuteFwUpdate` | `Int16BE requiredTime` |

None of the four is decoded by this port, and none is on a command the daemon
speaks, so the exceptions cost nothing here — but a length computed from a
field count is wrong for those four, and an encoder that computed one would be
wrong for them too.

A related convention worth stating, because it is easy to read the other way:
the `// 0xN` comment beside each field is a BYTE OFFSET, not a field index.
`ConnectRetProtocolInfo` is decisive — `Int32BE protocolVersion{}; // 0x2`
is followed by a field at `// 0x6`, four on, not one.

Keyed on `(command, type, length)` the 579 structs yield **409 distinct keys
with 37 collisions**. Of those 37:

* **16** are collisions in name only: every colliding struct has the same
  field types at the same offsets, so either decodes identically.
* **21** genuinely differ, and each one has to be decided rather than guessed.

### The 21, and which of them this port has to care about

Four are on commands the captured corpus carries:

```
0x67 / 0x69  MODE_NC_ASM_DUAL_NC_MODE_SWITCH_AND_ASM_SEAMLESS  len 7
    NcAsmRetParamModeNcDualModeSwitchAsmSeamless
    NcAsmRetParamNcNcssAsmModeNcDualModeSwitchAsmSeamless
    Differ at offset 4 only: NcAsmMode vs NcNcssAsmMode. Both are
    { NC = 0x00, ASM = 0x01, OUT_OF_RANGE = 0xFF }. HARMLESS -- and this is
    the message fixture 000069 carries, so the NC/ASM decode does not rest on
    picking between them.

0x68 / 0x69  NC_ON_OFF_AND_ASM_SEAMLESS  len 7
    NcAsmNtfyParamNcOnOffAsmOnOff     offset 6 is NcAsmOnOffValue
    NcAsmNtfyParamNcOnOffAsmSeamless  offset 6 is UInt8
    REAL. One reads the last byte as an on/off flag, the other as the ambient
    sound level. No capture settles it.

0xa3 / 0xa5  PLAYBACK_CONTROL_WITH_CALL_VOLUME_ADJUSTMENT  len 5
    RetPlayStatusPlaybackControlWithFunctionChange  offset 4 is EnableDisable
    RetPlayStatusPlaybackController                 offset 4 is MusicCallStatus
    REAL. No capture settles it.
```

The last pair was first written down the other way round -- each struct given
the other's offset-4 field. T1:7434-7446 and T1:7451-7463 say it as above.

**Plan 23 (2026-10-02) decided the play-status pair without settling it.** The
two structs agree field for field on offsets 2 and 3 (`EnableDisable status`,
`PlaybackStatus playbackStatus`), and those are all the state document needs.
`decode_play_status` reads exactly those two and never offset 4, and
`the_corpus_play_status_is_read_only_where_its_two_structs_agree` holds that
every value of offset 4 decodes the same. The collision itself stands.

The remaining seventeen are on firmware update (0x31–0x39), TWS topology
(0x43–0x49), SAR (0xb8/0xb9) and general settings (0xd7–0xd9) — none of which
this daemon speaks.

### What follows for the decoder

Dispatch on `(command, type, payload length)`, not on `(command, type)`. Where
that key still collides, either prove the two decode identically — as the NC/ASM
pair above does — or refuse the message and say which two structs it could be.
A decoder that silently picks one is the battery bug again with a longer fuse:
it produces a plausible number from the wrong byte, forever, with nothing to
say it is wrong.

## The support-function tables, and what they turn out to prove

Two frames — `000022` on `DATA_MDR` and `000026` on `DATA_MDR_NO2`, 54ms apart
in the same handshake — carry `CONNECT_RET_SUPPORT_FUNCTION`. They are the
device's own list of what it can do, and they turned out to be the most
checkable thing in the corpus.

### The layout, and how it was settled

`ConnectRetSupportFunction` holds an `MDRPodArray<SupportFunction>`, and
`SupportFunction` is declared in `ProtocolV2.hpp` — *not* in the `ProtocolV2T1.hpp`
this port was transcribed from — as exactly:

```cpp
struct SupportFunction { FunctionType functionType; UInt8 priority; };
```

So the payload is `command, type, count`, then `count` two-byte pairs. Both
captures are `3 + 2 * count` bytes to the byte: 95 for 46 functions, 25 for 11.

Which byte of the pair is the function is not left to the declaration order.
All 46 first bytes on table 1 are declared `t1::FunctionType` values and all 11
on table 2 are declared `t2::FunctionType` values. Read the other way round,
table 1 yields nine bytes no `t1::FunctionType` declares — 0x01, 0x08, 0x0D,
0x1D, 0x1F, 0x33, 0x3A, 0x3B, 0x3F — and table 2 yields 0x3D and 0x04, declared
by neither.

### The trap: one byte, two namespaces

`ProtocolV2.hpp` declares **two** `FunctionType` enums, `t1::` and `t2::`, and
the frame's data type is what selects between them. This capture contains the
collision twice over:

| code | table 1                                    | table 2                     |
|------|--------------------------------------------|-----------------------------|
| 0x42 | `CLASSIC_ONLY_LE_CLASSIC_SETTING`          | voice guidance, with volume |
| 0xF1 | `PLAYBACK_CONTROL_BY_WEARING_REMOVING_...` | `REPEAT_TAP_TRAINING_MODE`  |
| 0x53 | `PRESET_EQ_AND_ULT_MODE`                   | `SAFE_LISTENING_TWS_2`      |

Decoding the table-2 reply with the table-1 enum loses voice guidance, its
volume, safe listening, the paired-device list and pairing mode, and invents an
equaliser, an auto-pause, noise cancelling, ambient sound and Speak-to-Chat.
Every byte still checks out against the enum it was checked against. That is
the battery bug's shape exactly, and it is why `payload::decode` takes the
frame's data type as an argument rather than reaching for it later.

A second trap in the same message: **0xFF is a function code**, not a
terminator. On table 1 it is `HEAD_GESTURE_ON_OFF_TRAINING`, and this device
sends it — `ff 2d`, thirty-third of forty-six, with thirteen entries after it.
0xFF is *also* the priority value for a function Sony's app does not place, so
both bytes take it in different roles.

### The cross-check that makes this more than a transcription

The support tables predict which message families the rest of the capture
contains, and the prediction holds for all of them:

| the table says                              | the corpus contains                       |
|---------------------------------------------|-------------------------------------------|
| 0x29, 0x2A and **no** 0x20/0x28             | `POWER_RET_STATUS` at 0x09 and 0x0A only  |
| `AUTO_POWER_OFF_WITH_WEARING_DETECTION`     | `POWER_RET_PARAM` at 0x05                 |
| `PRESET_EQ`                                 | `EQEBB_RET_STATUS`/`_PARAM` at 0x00       |
| `MODE_NC_ASM_..._LEVEL_ADJUSTMENT` (0x6B)   | `NCASM_RET_PARAM` at 0x17                 |
| `PLAYBACK_CONTROLLER_WITH_CALL_VOLUME_...`  | `PLAY_RET_PARAM` at 0x01 and MUSIC_VOLUME |
| `UPSCALING_AUTO_OFF`                        | `AUDIO_RET_*` at `UPSCALING`              |
| four SYSTEM functions (0xF1,0xF3,0xFC,0xFF) | `SYSTEM_RET_PARAM` at 0x01,0x03,0x0C,0x0F |
| `GENERAL_SETTING_1` and `_2`, and no others | two `GENERAL_SETTING_RET_*` pairs         |
| t2 `PAIRING_DEVICE_MANAGEMENT_WITH_...`     | `PERI_RET_STATUS`/`_PARAM` at 0x02        |
| t2 voice guidance **with volume**           | `VOICE_GUIDANCE_RET_PARAM` at 0x01 AND 0x20 |
| t2 `SAFE_LISTENING_TWS_2` (0x53)            | `SAFE_LISTENING_RET_EXTENDED_PARAM` at 0x03 |

Two of those rows are sharper than they look. `SafeListeningInquiredType`
declares `SAFE_LISTENING_TWS_2 = 0x03`, and the advertised function is 0x53 —
the device answered at exactly the inquired type its function code names, which
is one more independent way of getting the same answer. And `PLAY_RET_PARAM` at
0x01 is `01 00 01 00 01 00 01 00`: four `PlaybackName` entries, each
`PlaybackNameStatus::NOTHING` with a zero-length string. Four track-metadata
slots, nothing playing — which is what a headset connected to an idle phone
should say, and which is why `playback_metadata` is derived from that function
rather than from a metadata function that does not exist.

Of the 26 payload frames captured after the handshake, every one belongs to an
advertised function. The one that maps to no capability flag is
`COMMON_RET_STATUS` at `AUDIO_CODEC` (fixture 000053, `13 02 02`, AAC), which
answers to t1 `CODEC_INDICATOR` (0x12) — the second entry of the captured
table 1. There is no frame the tables fail to explain — and that, not the
header, is what the support-function decoding rests on.

The check runs one way only. A partial capture is free to advertise a feature
and never mention it again, so an advertised function with no frame proves
nothing. A frame with no advertised function would be the falsifier.

### Where the mapping to the 27 capability flags stops

`sony::state::Capabilities::apply_support_functions` follows the function
names mechanically. Four places where it deliberately does not, each because a
name coincides with a different feature:

* `HEAD_TRACKER_*` (0xB5, 0xB7) is 360 Reality Audio, not head gestures. **This
  device advertises 0xB7 and 0xFF**, so the mistake is invisible on the only
  capture there is — a mutation that folded 0xB5/0xB7 into `head_gesture`
  passed every other test in the suite before
  `a_neighbouring_function_does_not_earn_a_flag_it_only_sounds_like` was added.
* `AUTO_POWER_OFF_WITH_WEARING_DETECTION` (0x25) says the timer may use the
  sensor, not that the headset reports wear.
* `CONNECTION_MODE_CLASSIC_AUDIO_LE_AUDIO` (0xE7) does not set
  `connection_mode` — but not because it is a different control. Its SET
  (`AudioSetParamConnectionModeClassicAudioLeAudio`, T1:3069) carries the same
  `PriorMode` at the same offset as the forms that do. What it also carries is
  a fourth byte the reference names only `value2`, with no comment anywhere,
  and no `AudioRetParam*` is pinned at its inquired type (0x05) to confirm a
  write. The flag would draw a control this port cannot send and cannot read
  back. One capture of a 0x05 exchange would settle it.
* `UPSCALING_INDICATOR` (0x13) is a display hint for Sony's app; the DSEE
  setting is `UPSCALING_AUTO_OFF` (0xE2).

One flag is never set from a support function at all. **`noise_control_button`**
has no counterpart: nothing in either table names a physical noise-control
button. It stays false rather than being mapped to something adjacent.

## A second correction worth keeping: which way round "on" is

`ProtocolV2.hpp:54-59`:

```cpp
enum class OnOffSettingValue : UInt8 { ON = 0x00, OFF = 0x01, OUT_OF_RANGE = 0xFF };
```

**ON is zero.** So is `EnableDisable::ENABLE`. And `NcAsmOnOffValue`, in the
T1 header, is `OFF = 0x00, ON = 0x01` — the ordinary way round, as is
`UpscalingTypeAutoOff`. The protocol uses both conventions, in enums with
nearly the same name, and nothing marks which is which except the declaration.

Fixture `000081` is `f7 0c 01 01` — `SystemRetParamSmartTalking` at
`SMART_TALKING_MODE_TYPE2`, `onOffValue` and `previewModeOnOffValue` both
0x01. **Speak-to-Chat is OFF on the captured headset.** A decoder written on
the obvious reading reports it as on, draws an enabled toggle for a disabled
feature, and nothing about the byte looks wrong.

That is why `Switch` has two named constructors — `from_on_off_setting` and
`from_auto_off` — rather than one `from_byte`. The polarity is a decision at
every use, not an assumption made once.

The rest of that message family, decoded:

```
fb 0c | 00 00   SystemRetExtParamSmartTalkingModeType2
                DetectSensitivity::AUTO, ModeOutTime::FAST
                — Sony's defaults, which is what an untouched setting reads as
e1 01 | 02      AudioRetCapabilityUpscaling, UpscalingType::DSEE_HX_AI
e7 01 | 01      AudioRetParamUpscaling, UpscalingTypeAutoOff::AUTO — DSEE on
```

The last two are three bytes each at the same inquired type, and byte 2 belongs
to a different enum in each: the COMMAND is the only discriminator. Read the
param byte as an `UpscalingType` and 0x01 becomes "DSEE" — a real upscaler
name, on a device whose upscaler is DSEE HX AI.

Two smaller notes from the same pass:

* `SystemRetParamSmartTalking` declares its inquired type as
  `SystemInquiredType::VIBRATOR` (0x00), which it plainly is not. Another
  instance of "the type defaults are not always the real discriminator", and
  another reason the dispatch key is `(command, type, length)` read off the
  payload rather than off the struct's defaults.
* `AUDIO_RET_STATUS` at `UPSCALING` (fixture `000097`, `e3 01 00`) was
  decoded by nothing here, on the argument that no struct declares that key
  because `AudioRetStatusCommon` defaults its type to CONNECTION_MODE. **That
  argument was wrong** (plan 23, 2026-10-02): the field is `CODEGEN Ignore`,
  not `EnumRange`, so the default is not a pin -- this file's own rule, below
  -- and libmdr reads exactly `AudioRetStatusCommon` at UPSCALING and keeps
  `status == ENABLE` as "upscaling available" (`HeadphonesV2T1.cpp:564-579`).
  It is decoded now, on reference-client grounds, and published as
  `dsee_available`.

## The command byte is table-scoped too, and that was a real bug

The support-function work established that the two tables are separate
FUNCTION-TYPE namespaces. The same is true one layer up, of the command bytes
themselves, and the port shipped without knowing it.

Table 1 declares 152 commands and table 2 declares 77. **They share 45 bytes and
disagree on all 45.** Five of the disagreements are on bytes this port names:

| byte | `ProtocolV2T1.hpp` | `ProtocolV2T2.hpp` |
|------|--------------------|--------------------|
| 0x53 | `EQEBB_RET_STATUS` | `SAFE_LISTENING_RET_STATUS` |
| 0x57 | `EQEBB_RET_PARAM`  | `SAFE_LISTENING_RET_PARAM` |
| 0x59 | `EQEBB_NTFY_PARAM` | `SAFE_LISTENING_NTFY_PARAM` |
| 0x67 | `NCASM_RET_PARAM`  | `LEA_RET_PARAM` |
| 0x69 | `NCASM_NTFY_PARAM` | `LEA_NTFY_PARAM` |

`decode` took the frame's data type as an argument and used it for exactly one
message. Everything else dispatched on the command byte alone. So a
`SAFE_LISTENING_RET_PARAM` arriving on `DATA_MDR_NO2` went to
`decode_equalizer`, and if its bytes happened to parse — preset byte, count
byte, that many steps, nothing left over — the panel showed an equaliser preset
nobody chose.

**This device sends those frames.** Its own table-2 support-function reply
advertises safe listening (0x53, 0x54) and LE Audio (0x60, 0x61, 0x63). The
corpus contains `SAFE_LISTENING_RET_EXTENDED_PARAM` (0x5B), which this port does
not name; a longer capture of the same session would very likely contain 0x57.

### A matching command name is not a matching message

0x23 is `POWER_RET_STATUS` in **both** enums, which looks safe and is not.
`ProtocolV2T2.hpp:373` declares table 2's own `PowerInquiredType`:

```
AUTO_STANDBY = 0x00,  CARING_CHARGE_WITH_THRESHOLD = 0x01,  USB_SUBMERSION = 0x02
```

where table 1 has `BATTERY`, `LEFT_RIGHT_BATTERY`, `CRADLE_BATTERY`. A table-2
0x23 at kind 0x00 is a standby setting, and `decode_battery` would have reported
its next byte as a charge level. `SystemInquiredType` diverges the same way
(`ProtocolV2T2.hpp:524`).

So the rule is a whitelist of one: **every decoder in this module reads table 1,
and only `DATA_MDR` is table 1.** `DATA_MDR_NO2` is table 2 and `DATA_COMMON` is
a third namespace again.

The single exception is the support-function reply, and it is exempt for a
stated reason rather than convenience: its HEADER is table-neutral — 0x07 is
`CONNECT_RET_SUPPORT_FUNCTION` and `ConnectInquiredType::FIXED_VALUE` is 0x00 in
both enums — while only its BODY is table-scoped, which is exactly what
`decode_support_functions` takes the table to read.

### How it was found, and what that says

Not by a test. Every test passed, before and after, because every hand-written
payload in the suite used `DataMdr` and every table-2 frame in the corpus
happens to carry a command byte this port does not name.

It was found by an adversarial reviewer asking, of the WRITE path, "are command
bytes table-scoped in both directions?" — and the answer turning out to matter
to the read path that was already shipped.

## The write path, and what an `-rx`-only corpus can still prove about it

Every fixture is received. Nothing this port SENDS has ever been accepted by a
headset, and nothing in this environment can change that. `mdr/encode.rs` is a
separate file for exactly that reason: so `payload.rs`'s claim — "a real
WF-1000XM5 sent these bytes and they decode to this" — is not diluted by a
module that can never say it.

### The mirror theorem

For five of the seven device verbs the SET struct is **field-for-field
identical to its RET twin** — same types, same byte offsets, differing only in
byte 0. Measured over the family rather than assumed: thirteen of the fourteen
`NcAsmSetParam*` structs match their `NcAsmRetParam*` twin exactly.

So an emitted SET, with byte 0 swapped back to the RET command, must decode
through the **existing** `payload::decode` to the thing the verb asked for.
That is not self-consistency. The decoder was written from captures and agrees
with a real headset byte for byte on all five of these messages, so the round
trip is transitive against real hardware:

| verb | SET | RET | fixture | the bytes |
|------|-----|-----|---------|-----------|
| `noise`, `ambient-level`, `focus-on-voice` | 0x68 | 0x67 | 000069 | `67 17 01 00 01 00 14` |
| `speak-to-chat` | 0xF8 | 0xF7 | 000081 | `f7 0c 01 01` |
| `dsee` | 0xE8 | 0xE7 | 000101 | `e7 01 01` |
| `volume` | 0xA8 | 0xA7 | 000061 | `a7 20 0c` |
| `eq-preset`, `eq-band`, `clear-bass` | 0x58 | 0x57 | 000093 | `57 00 a0 06 14 0b 0a 0d 09 0c` |

The last row is ONE message with three verbs pointed at it, and the bytes are
why: `57 00 a0 06 14 0b 0a 0d 09 0c` is the preset (`a0`, CUSTOM), a count
(`06`) and six steps — clear bass and the five bands in one
`MDRPodArray<UInt8>` with no separator and no short form. So every verb that
touches the equaliser sends all of it. `eq-preset` replaces `a0` and echoes the
six; `eq-band` and `clear-bass` replace one of the six and echo `a0`. A fourth
column would be a fourth way of saying the same frame.

The command arithmetic is not a table: within every family the reference
numbers GET, RET, SET and NTFY as base+0, +1, +2 and +3. That holds for all 91
derivable pairs across the 152 declared commands, with **zero** exceptions —
which is also why `Command::canonical` subtracts exactly 2.

### It found a real bug on its first run

`decode_noise` maps the total-effect byte and the mode byte together into a
single `NoiseMode`, collapsing to `Off` whenever the feature is switched off.
That is right for a panel and it **throws away the mode byte**. A SET
re-transmits every field, so an encoder re-deriving that byte from `NoiseMode`
would write NC for a headset sitting in ambient behind a disabled switch —
silently changing the mode the next time somebody turned it back on.

`NoiseReport` now carries `mode_byte` for that reason. The decoder reports
strictly more of what the device said than it did before, and the reason it
does is a test of the write path.

### The rule the module is built on

A SET is a **whole-state message**. None of the fourteen `NcAsmSetParam*`
structs has an "unchanged" sentinel in any field: the 0x17 payload carries the
change status, the total effect, the mode, the ambient mode AND the level,
every time. So `ambient-level 12` must re-transmit the mode and the
focus-on-voice setting, and `noise anc` must re-transmit the level.

Every byte the encoder emits is therefore exactly one of three things: named by
the verb, echoed from a reading the headset itself sent, or not emitted at all.
There is no fourth case and no struct-default fallback. An encoder filling the
other three NC/ASM bytes from the header's brace initialisers would emit
`68 17 01 01 00 00 00` for `noise anc` — zeroing the ambient level and
switching focus-on-voice off every time somebody changed their noise mode, and
no round trip would notice. `Facts` has no `Default` so that this cannot happen
by accident, and `nothing_is_emitted_without_the_state_it_must_echo` fails the
moment one appears.

The inquired type is echoed too, never inferred. The captured device answers
NC/ASM at 0x17, but the corpus is a WF-1000XM5 and this tree targets a WH —
"the wrong ear". Deriving 0x17 from support-function 0x6B would be a name
match, which is what this module exists not to trust.

### What cannot be mirrored, said plainly

`playback` has **no** field-identical twin. `SetPlayStatusPlaybackController`
(T1:8013) is four fields ending in a `PlaybackControl`;
`RetPlayStatusPlaybackController` (T1:7451) is five, ending in a
`PlaybackStatus` and a `MusicCallStatus`. The two enums share value names
across different bytes — `PAUSE` is 0x01 in one and 0x02 in the other — so
there is nothing to mirror it against. It was emitted at `HeaderOnly`; since
plan 23 it is `ReferenceClient`, because libmdr sends the same four bytes
(`HeadphonesV2.cpp:438-443`), and is gated on the support table and on the
headset's own play-status reply.

Its four transport words do rest on the strongest single citation in the
reference: `// CODEGEN EnumRange PlaybackControl::PAUSE PlaybackControl::TRACK_UP
PlaybackControl::TRACK_DOWN PlaybackControl::PLAY` restricts the field to
exactly those four, which settles `next`/`previous` as TRACK_UP/TRACK_DOWN and
not GROUP_UP/GROUP_DOWN. What the header does not settle is offset 2, an
`EnableDisable` with no EnumRange, no comment and an unpinned default — the
one byte in the module that is neither named by a verb nor echoed from a
reading. The reference client settles it: ENABLE, 0x00, on every write.

### The preset table, and the swap it avoids

`sony::command::EQ_PRESETS` is **not** in `EqPresetId` order. It lists heavy,
clear, hard, soft (0x30..=0x33) before gaming and fps_1..3 (0x20..=0x23); the
enum declares them the other way round. Mapping by index into the enum would
swap eight presets in four pairs, and every one of them is a real preset a
headset would happily apply. `EQ_PRESET_IDS` is indexed in parallel with
`EQ_PRESETS` and nothing else, so there is one table rather than two that can
drift.

### What is not built, and what would settle each

* **`connection quality|stable`** — refused by name on every device, until
  plan 23 (2026-10-02). It is written now to a headset that has answered at a
  connection type: `e8 02 <mode>` after a reply at 0x02 (header-only, a
  pinned field-identical RET/SET/NTFY triple) and `e8 00 <mode>` after one at
  0x00 (third party: mos9527's PR #64, on a WF-LC900). Still refused by name:
  a headset whose only connection function is t1 0xE7 -- the captured family
  -- because that SET's fourth byte is the `value2` the reference never
  explains. **Settled by**: one WH-1000XM5 support table plus an
  `AUDIO_RET_PARAM` at whichever connection type it answers, and one write
  that sticks.
* **Any NcAsm SET at an unpinned key** — `(0x68, 0x01)` with a 4-, 6-, 7- or
  8-byte body. Four structs carry `// CODEGEN Ignore OUT_OF_RANGE is expected`
  above their type field, so their `NC_ON_OFF = 0x01` is nothing but the enum's
  first value, and for three of them it contradicts their own name. Emitting
  one would name a message the header does not declare — the same error this
  file already records for the `(0x22, 6)` entry. **Settled by**: a device
  answering NCASM at 0x01, so the length picks the struct the way the read side
  does.
* **The `(0x68, 0x13, 7)` key** — both structs are pinned and read offset 6
  differently, one as an on/off value and one as the ambient level. The encoder
  refuses by name. This collision exists on the SET side and **not** on the
  read side, where the two RET structs are six and seven fields. **Settled by**:
  nothing in the reference.

### The pinned/unpinned rule, which is the general form of an old trap

This file already records that "the type defaults are not always the real
discriminator". The reference marks which is which, and the marker is
mechanical:

* `// CODEGEN EnumRange <Enum>::<VALUE>` above a field **pins** it. The value is
  a constraint.
* `// CODEGEN Ignore OUT_OF_RANGE is expected` above a field does **not**. The
  brace initialiser is just the enum's first declared value, and for several
  structs it contradicts the struct's own name.

`SystemRetParamSmartTalking` declaring `VIBRATOR`, `SetPlayParamPlaybackControllerVolume`
declaring `PLAYBACK_CONTROL_WITH_CALL_VOLUME_ADJUSTMENT` where the wire says
`MUSIC_VOLUME`, and four of the fourteen `NcAsmSetParam*` structs are all the
same phenomenon. A pin means "this byte is the type"; it does not mean "this
name matches this type".

### The queries, and a second one-way cross-check

Every GET is two bytes, `[command, inquired type]`, and every one is settled:
`ConnectGetProtocolInfo` (T1:3477), `ConnectGetCapabilityInfo` (T1:3455),
`ConnectGetDeviceInfo` (T1:3466) and `ConnectGetSupportFunction` (T1:3488) are
two fields each, and the per-feature GETs are the same shape.

`handshake()` sends them in the order the capture shows the app asking. The
transmitted half was never recorded, but the counter gaps ARE that half, and
the order of what came back is the order it was asked for. The support-function
query goes out **twice**, once per channel, and that is not an inference: the
corpus contains both replies, 54ms apart, on `DATA_MDR` and `DATA_MDR_NO2`.

`feature_gets()` asks only what the device's own tables justify, and the pair
of tests around it runs the support-function cross-check in **both**
directions:

* Every reply in the corpus must be something the queries would have asked
  for. This found three real omissions on its first run — `POWER_RET_PARAM` at
  0x05, `EQEBB_RET_STATUS` at 0x00 and `AUDIO_RET_STATUS` at 0x01 were all
  answered by the device and asked for by nothing.
* Every query must be something the tables advertise. This one is separate
  because the first would happily pass an encoder that asked for everything.
  The battery case is the sharpest: this device advertises 0x29 and 0x2A and
  not 0x20, so exactly two `POWER_GET_STATUS` queries go out, at 0x09 and
  0x0A, and none at 0x00.

The exceptions are named rather than filtered quietly: `SAFE_LISTENING`,
`PERI` and `VOICE_GUIDANCE` are table-2 families this port does not read — so
it does not ask for them either. `COMMON_RET_STATUS` is not an exception: it
answers to t1 `CODEC_INDICATOR` (0x12), which the captured table 1 advertises,
and the codec query is gated on exactly that function.

## What the corpus settles about a SESSION

The daemon has to hold a conversation, not just read one message, and most of
what that takes is not in an `-rx` capture. Three things are, and they are
worth separating from the many that are not.

**The acknowledgement rule.** Acknowledge an inbound frame of sequence S with
an ACK of sequence 1−S. `frame::ack_frame` does this, and both possible ACK
byte strings are in the corpus, so the rule is pinned against real bytes in
both directions of the flip:

* fixture `000005` is `3e 01 00 00 00 00 00 01 3c`
* fixture `000001` is `3e 01 01 00 00 00 00 02 3c`

**Our first frame carried sequence 0.** This is inferable from a capture that
contains none of our frames. The first thing the headset sent is fixture
`000001`, an ACK carrying sequence 1; an ACK carries 1 minus the sequence it
acknowledges; so the frame it was acknowledging — ours, unrecorded — carried
0.

**The headset's own alternation.** Its ACK and the DATA that follows share a
sequence, and consecutive pairs alternate:

```
000001 ACK 1   000002 DATA 1
000005 ACK 0   000007 DATA 0
000009 ACK 1   000011 DATA 1
000013 ACK 0   000015 DATA 0
000017 ACK 1   000019 DATA 1
000021 ACK 0   000022 DATA 0
000025 ACK 1   000026 DATA 1
```

### What it does NOT settle

**Our own counter's advance rule.** The recorder's counter gaps are the
transmitted half, and they do not decompose into one sent frame per reply:
between `000005` and `000007` there is exactly one gap, but between `000002`
and `000005` there are two. So the sequence a client uses for its *second*
query cannot be read off this capture, and anything asserting it would be
guessing.

**Everything a real socket does that a byte array does not.** No partial frame,
no frame split across two reads, no disconnect mid-message, no timeout, no
duplicate, no reordering, no garbage. The corpus is 70 clean frames in order.

That last point is why `Scanner` is tested against chunked delivery at every
chunk size, junk with no start marker, a peer that sends a start marker and
never finishes, and a span that will never decode — none of which the corpus
contains. A capture proves what the protocol says; it proves nothing about
what a socket does.
