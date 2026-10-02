# What each claim stands on

Every frame this package reads or writes carries, as a value, the kind of
evidence behind it. This document lists the kinds, where they show, what each
read and write stands on today, and the sources they cite. Dated 2026-10-02;
the tables were measured from the code on that date.

## The four kinds

Weakest last. The same four on both sides, named for what they are.

| Kind | Read side (`mdr::payload::Grounds`) | Write side (`mdr::encode::Evidence`) |
|---|---|---|
| Captured | a real headset sent this key, and it decoded to sensible values; the corpus holds it | `MirroredCapture`: the SET is field-for-field its RET twin, and the corpus holds that RET, so the round trip reaches captured bytes |
| Reference client | libmdr reads it this way, and people run libmdr against real headsets; the bytes may be captured, the meaning was not settled by them alone | `ReferenceClient`: libmdr sends these bytes for this request |
| Third party | another project reports a real headset sending or taking it | `ThirdParty`: another project sends these bytes and reports them working |
| Header only | transcribed from the reference header; nothing seen | `HeaderOnly`: transcribed from the header, no frame either way |

Where they show:

- `qs-sony decode <file|dir>` prints `grounds` beside every frame (`null` for a
  frame nothing decodes).
- `qs-sonyd` logs every frame a command becomes with its tier and the struct it
  was built as (`Note::Sent`), so someone with a headset knows which writes to
  compare against a capture first.
- `payload::CAPTURED` lists the captured keys and a test holds it to the corpus
  in both directions; `payload::OTHER_GROUNDS` lists the rest with a citation
  each. A key in neither is header only.

## The writes, against the captured headset

Measured by encoding every verb against the WF-1000XM5 corpus's facts:

| Verb | Tier | Struct | Bytes |
|---|---|---|---|
| `noise`, `ambient-level`, `focus-on-voice` | mirrored capture | `NcAsmSetParamModeNcDualModeSwitchAsmSeamless` | `68 17 ...` |
| `speak-to-chat` | mirrored capture | `SystemSetParamSmartTalking` | `f8 0c ...` |
| `dsee` | mirrored capture | `AudioSetParamUpscaling` | `e8 01 ...` |
| `eq-preset`, `eq-band`, `clear-bass` | mirrored capture | `EqEbbSetParamEq` | `58 00 ...` |
| `general` | mirrored capture | `GeneralSettingSetParam` | `d8 <slot> 00 <value>` |
| `volume` | mirrored capture | `SetPlayParamPlaybackControllerVolume` | `a8 20 ...` |
| `playback` | reference client | `SetPlayStatusPlaybackController` | `a4 01 00 <control>` |
| `connection` | refused on this headset (`value2`) | -- | -- |
| `alert` | refused until the headset asks | -- | -- |

On other headsets: NC/ASM at 0x19 is reference client and at 0x15 third party;
`connection` is header only at 0x02 and third party at 0x00; `alert yes|no` is
header only; the alerts opt-in `94 00 00` is reference client.

## The reads that are not captured

`payload::OTHER_GROUNDS`, ten keys:

| Key | Grounds | Because |
|---|---|---|
| t1 0x67 at 0x15 | third party | Gadgetbridge reads a WH-1000XM5's NC/ASM there |
| t1 0x67 at 0x19, 0x22 | reference client | libmdr's `HandleNcAsmParamT1` |
| t1 0xE3 at 0x01 | reference client | fixture 000097's bytes, libmdr's meaning |
| t1 0xE7 at 0x00 | reference client | libmdr's `HandleAudioParamT1` |
| t1 0x99 at 0x00 | reference client | libmdr's `HandleAlertParamT1` |
| t2 0x33 at 0x02, 0x47 at 0x01 and 0x20, 0x5B at 0x03 | reference client | the corpus's bytes, libmdr's `HandleCommandV2T2` meanings |

Everything else decodable and not captured -- the over-ear battery forms, the
ambient range, the connection mode at 0x02, NTFY twins with no capture -- is
header only.

## Sources

Cited as `T1:`, `T2:` and `V2:` with a line, they are the reference at
mos9527/SonyHeadphonesClient `db0ae2574d8f8e1407e8ea28a7ee87db9dde1c24`:
`ProtocolV2T1.hpp`, `ProtocolV2T2.hpp` and `HeadphonesV2.cpp` in its libmdr.
`ProtocolV2.hpp`, `Protocol.hpp`, `HeadphonesV2T1.cpp` and `HeadphonesV2T2.cpp`
are cited by name. MIT, and the corpus in `tests/fixtures/mdr/` is from the
same project; `THIRD_PARTY.md` carries the notice.

Cited for facts only, never copied -- their licences are not this package's:

- Gadgetbridge (AGPL-3.0): `SonyProtocolImplV2.java`, how it reads and writes a
  WH-1000XM5's NC/ASM at 0x15 and its battery.
- xm5-mode-switch (GPL): the NC/ASM type it observes on a WH-1000XM5, 0x17.
- XMDeck, pull request 5: 0x19 on a WH-1000XM5.
- mos9527/SonyHeadphonesClient pull request 64: the connection write at 0x00,
  on a WF-LC900.

`delarosa1312/omarchy-sony-xm5` (MIT) is quoted where it is cited, for which
writes persist on this family; `THIRD_PARTY.md` carries its notice.
