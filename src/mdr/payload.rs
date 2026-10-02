//! What the bytes inside a frame mean.
//!
//! The frame layer above this one is the whole byte-level protocol and knows
//! nothing about content. This is the other half: a payload arrives, and this
//! decides which message it is and what it says.
//!
//! HOW A PAYLOAD SELECTS A MESSAGE, AND WHY IT IS NOT OBVIOUS
//!
//! The reference header has no dispatch table. It has 579 structs, each with a
//! `Command` default and most with a second field naming an "inquired type",
//! and a decoder has to work out from those which struct a payload is. Keying
//! on `(command, type)` does not work: `NcAsmInquiredType::NC_ON_OFF` is the
//! declared type of FIVE different `NcAsmRetParam*` structs whose lengths are
//! 4, 6, 8, 7 and 5 -- one has no ambient sound level at all and another has
//! one at offset 7.
//!
//! Adding the LENGTH very nearly fixes it, because the field count is almost
//! always the payload length: the header is packed, and 444 of the 448
//! `MDR_DEFINE_TRIVIAL_SERIALIZATION` structs hold nothing but `UInt8`s and
//! `UInt8`-backed enums. FOUR are not one byte per field, and they are named
//! here rather than left as a footnote for somebody to trip over:
//! `ConnectRetProtocolInfo` (an `Int32BE`),
//! `CommonSetParamConnectionEstablishedTime` (an `Int16BE`), and the two
//! `UpdtNtfyParamTandem*` firmware-update messages. None is decoded here.
//! Keyed on `(command, type, length)` the 579 structs give 409 distinct keys
//! with 37 collisions, 16 of which are collisions in name only.
//!
//! `tests/fixtures/mdr/PROVENANCE.md` carries the measurement and enumerates
//! the 21 that genuinely differ. Two of those are on messages this daemon
//! would speak, and no capture settles either. One -- the NC/ASM NTFY at
//! 0x13 -- is refused by name rather than guessed at ([`AMBIGUOUS`]). The
//! other, the play-status pair, is read only on the offsets both of its
//! structs agree on, and never on the byte they dispute
//! ([`decode_play_status`]); fixture 000065 is one.
//!
//! WHAT IS DECODED, AND WHAT IS NOT
//!
//! Each decoder says which grounds it stands on, and [`grounds`] says the same
//! to a program. Four kinds:
//!
//! CAPTURED means a frame a real WF-1000XM5 sent was decoded byte by byte and
//! the values came out sensible: the two threshold battery forms (0x09, 0x0A),
//! the NC/ASM variant at 0x17, the model name and firmware version, the
//! equaliser preset with its band steps, the music volume, the A2DP codec,
//! both support-function tables, both halves of DSEE, both halves of
//! Speak-to-Chat, the named general settings and the paired-device list.
//! [`CAPTURED`] lists the keys, held to the corpus by a test.
//!
//! REFERENCE CLIENT means the meaning is libmdr's: it decodes the key this
//! way, and people run it against real headsets. THIRD PARTY means another
//! project reports a real headset sending it. HEADER ONLY means the layout is
//! transcribed and nothing has been seen sending it: the four plain battery
//! forms, which are the ones an over-ear reports and which no capture in this
//! corpus covers, and the ambient level's capability reply.
//!
//! Everything else is [`Reading::Unhandled`]. That is a panel showing "unknown"
//! rather than a plausible-looking number, and it is recoverable by capturing
//! one frame from the device in question.
//!
//! The battery correction in PROVENANCE.md is what the rule exists for: a
//! decoder written from an enum's first few names read a threshold byte as a
//! further battery level, on every reading, with nothing to say it was wrong.
//! `OnOffSettingValue` is the same trap on one bit -- `ON = 0x00` and
//! `OFF = 0x01`, the opposite way round from `NcAsmOnOffValue` a few hundred
//! lines away in the same header. The captured headset has Speak-to-Chat off,
//! and the obvious reading says it is on.
//!
//! NOTHING HERE PANICS. Same reason as the frame layer: `panic = "abort"` in
//! release turns a bad packet from a Bluetooth socket into a SIGABRT with the
//! backtrace stripped. Every read is bounds-checked and every function that
//! reads bytes is total.

/// The commands this daemon speaks. Everything else is [`Command::Other`].
///
/// Deliberately partial. The header declares over two hundred; carrying the
/// ones this port has no use for would be two hundred names nothing reads and
/// a claim to understand messages it has never seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    ConnectRetProtocolInfo,
    ConnectRetCapabilityInfo,
    ConnectRetDeviceInfo,
    ConnectRetSupportFunction,
    CommonRetStatus,
    CommonNtfyStatus,
    PowerRetStatus,
    PowerNtfyStatus,
    PowerRetParam,
    PowerNtfyParam,
    EqEbbRetStatus,
    EqEbbRetParam,
    EqEbbNtfyParam,
    /// `NCASM_RET_CAPABILITY` (0x61): the ambient level's range, per mode.
    /// TABLE ONE ONLY -- on table 2 the same byte is `LEA_RET_CAPABILITY`, and
    /// the table gate in [`decode`] keeps the two apart.
    NcAsmRetCapability,
    NcAsmRetParam,
    NcAsmNtfyParam,
    PlayRetStatus,
    PlayNtfyStatus,
    PlayRetParam,
    PlayNtfyParam,
    AudioRetCapability,
    AudioRetStatus,
    /// `AUDIO_NTFY_STATUS` (0xE5), the status reply volunteered.
    AudioNtfyStatus,
    AudioRetParam,
    AudioNtfyParam,
    SystemRetParam,
    SystemNtfyParam,
    SystemRetExtParam,
    SystemNtfyExtParam,
    /// `GENERAL_SETTING_RET_CAPABILITY` (0xD1): which named on/off setting a
    /// slot holds, and what the device calls it.
    GsRetCapability,
    /// `GENERAL_SETTING_RET_PARAM` (0xD7): whether that setting is on.
    GsRetParam,
    /// `GENERAL_SETTING_NTFY_PARAM` (0xD9), the same body volunteered.
    GsNtfyParam,
    /// `ALERT_NTFY_PARAM` (0x99): the headset asking the person something.
    /// Volunteered only, and only after `ALERT_SET_STATUS` enabled it; there
    /// is no GET or RET in this family's PARAM half.
    AlertNtfyParam,
    /// A command byte this port does not model.
    Other(u8),
}

impl Command {
    pub fn from_byte(byte: u8) -> Command {
        match byte {
            0x01 => Command::ConnectRetProtocolInfo,
            0x03 => Command::ConnectRetCapabilityInfo,
            0x05 => Command::ConnectRetDeviceInfo,
            0x07 => Command::ConnectRetSupportFunction,
            0x13 => Command::CommonRetStatus,
            0x15 => Command::CommonNtfyStatus,
            0x23 => Command::PowerRetStatus,
            0x25 => Command::PowerNtfyStatus,
            0x27 => Command::PowerRetParam,
            0x29 => Command::PowerNtfyParam,
            0x53 => Command::EqEbbRetStatus,
            0x57 => Command::EqEbbRetParam,
            0x59 => Command::EqEbbNtfyParam,
            0x61 => Command::NcAsmRetCapability,
            0x67 => Command::NcAsmRetParam,
            0x69 => Command::NcAsmNtfyParam,
            0xA3 => Command::PlayRetStatus,
            0xA5 => Command::PlayNtfyStatus,
            0xA7 => Command::PlayRetParam,
            0xA9 => Command::PlayNtfyParam,
            0xE1 => Command::AudioRetCapability,
            0xE3 => Command::AudioRetStatus,
            0xE5 => Command::AudioNtfyStatus,
            0xE7 => Command::AudioRetParam,
            0xE9 => Command::AudioNtfyParam,
            0xF7 => Command::SystemRetParam,
            0xFB => Command::SystemRetExtParam,
            0xFD => Command::SystemNtfyExtParam,
            0xF9 => Command::SystemNtfyParam,
            0xD1 => Command::GsRetCapability,
            0xD7 => Command::GsRetParam,
            0xD9 => Command::GsNtfyParam,
            0x99 => Command::AlertNtfyParam,
            other => Command::Other(other),
        }
    }

    pub fn to_byte(self) -> u8 {
        match self {
            Command::ConnectRetProtocolInfo => 0x01,
            Command::ConnectRetCapabilityInfo => 0x03,
            Command::ConnectRetDeviceInfo => 0x05,
            Command::ConnectRetSupportFunction => 0x07,
            Command::CommonRetStatus => 0x13,
            Command::CommonNtfyStatus => 0x15,
            Command::PowerRetStatus => 0x23,
            Command::PowerNtfyStatus => 0x25,
            Command::PowerRetParam => 0x27,
            Command::PowerNtfyParam => 0x29,
            Command::EqEbbRetStatus => 0x53,
            Command::EqEbbRetParam => 0x57,
            Command::EqEbbNtfyParam => 0x59,
            Command::NcAsmRetCapability => 0x61,
            Command::NcAsmRetParam => 0x67,
            Command::NcAsmNtfyParam => 0x69,
            Command::PlayRetStatus => 0xA3,
            Command::PlayNtfyStatus => 0xA5,
            Command::PlayRetParam => 0xA7,
            Command::PlayNtfyParam => 0xA9,
            Command::AudioRetCapability => 0xE1,
            Command::AudioRetStatus => 0xE3,
            Command::AudioNtfyStatus => 0xE5,
            Command::AudioRetParam => 0xE7,
            Command::AudioNtfyParam => 0xE9,
            Command::SystemRetParam => 0xF7,
            Command::SystemNtfyParam => 0xF9,
            Command::SystemRetExtParam => 0xFB,
            Command::SystemNtfyExtParam => 0xFD,
            Command::GsRetCapability => 0xD1,
            Command::GsRetParam => 0xD7,
            Command::GsNtfyParam => 0xD9,
            Command::AlertNtfyParam => 0x99,
            Command::Other(byte) => byte,
        }
    }

    /// A NTFY command as its RET twin, where the two carry the same payload.
    ///
    /// A headset answers a query with RET and volunteers the same information
    /// later with NTFY, and the struct families are USUALLY identical but for
    /// the command byte -- so one decoder covers both and the table below
    /// carries one entry rather than two.
    ///
    /// USUALLY, not always, and the exception was found by comparing the two
    /// families field for field rather than by assuming:
    /// `NcAsmRetParamNcOnOffAsmOnOff` has six fields and
    /// `NcAsmNtfyParamNcOnOffAsmOnOff` has seven. That kind (0x13) is not
    /// decoded here, so the mapping is sound for everything this module reads
    /// -- but it is not a general truth about the protocol and must not be
    /// extended to a new message without the same comparison.
    ///
    /// Every `PowerRetStatus*` struct and its `PowerNotifyStatus*` twin agree,
    /// so the battery mapping is unconditional. The NcAsm families agree for
    /// every variant except the one named above.
    ///
    /// `SystemNotifyParamSmartTalking` and the `SYSTEM_NTFY_EXT_PARAM` twin of
    /// `SystemRetExtParamSmartTalkingModeType2` were compared the same way and
    /// agree field for field, which is what lets 0xF9 and 0xFD carry
    /// Speak-to-Chat. So were `CommonNotifyStatusAudioCodec` (T1:3229) and
    /// `CommonRetStatusAudioCodec` (T1:3355): command, inquired type, codec.
    /// And `AudioNotifyStatusCommon` (T1:2498) against `AudioRetStatusCommon`
    /// (T1:2930): command, inquired type, one `EnableDisable`.
    pub fn canonical(self) -> Command {
        match self {
            Command::CommonNtfyStatus => Command::CommonRetStatus,
            Command::PowerNtfyStatus => Command::PowerRetStatus,
            Command::PowerNtfyParam => Command::PowerRetParam,
            Command::EqEbbNtfyParam => Command::EqEbbRetParam,
            Command::NcAsmNtfyParam => Command::NcAsmRetParam,
            Command::PlayNtfyStatus => Command::PlayRetStatus,
            Command::PlayNtfyParam => Command::PlayRetParam,
            Command::AudioNtfyStatus => Command::AudioRetStatus,
            Command::AudioNtfyParam => Command::AudioRetParam,
            Command::SystemNtfyParam => Command::SystemRetParam,
            Command::SystemNtfyExtParam => Command::SystemRetExtParam,
            Command::GsNtfyParam => Command::GsRetParam,
            other => other,
        }
    }
}

/// A payload, split into the three things that select a message.
///
/// `kind` is the "inquired type" byte at offset 1. Almost every message in
/// this protocol has one; the handful that do not are single-byte payloads
/// where `kind` is absent and `body` is empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Message<'a> {
    pub command: Command,
    pub kind: Option<u8>,
    /// Everything after the command and kind bytes.
    pub body: &'a [u8],
    /// The whole payload's length, which is half of the dispatch key.
    pub len: usize,
}

impl<'a> Message<'a> {
    /// Split a payload. `None` only for an empty one, which is not a message.
    pub fn split(payload: &'a [u8]) -> Option<Message<'a>> {
        let command = Command::from_byte(*payload.first()?);
        let kind = payload.get(1).copied();
        let body = payload.get(2..).unwrap_or(&[]);
        Some(Message { command, kind, body, len: payload.len() })
    }
}

/// How charged, and whether it is charging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    /// 0..=100. The header's `CODEGEN Range 0 100`.
    pub level: u8,
    pub charging: Charging,
    /// The level below which the headset warns, when the message carries one.
    pub threshold: Option<u8>,
}

/// `BatteryChargingStatus`, verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charging {
    No,
    Yes,
    Unknown,
    Charged,
    /// A value the header does not name.
    Other(u8),
}

impl Charging {
    pub fn from_byte(byte: u8) -> Charging {
        match byte {
            0x00 => Charging::No,
            0x01 => Charging::Yes,
            0x02 => Charging::Unknown,
            0x03 => Charging::Charged,
            other => Charging::Other(other),
        }
    }

    /// The word the state document uses: the C++ daemon's `chargingName`,
    /// which is what the Headphones page compares against (`=== "yes"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Charging::No => "no",
            Charging::Yes => "yes",
            Charging::Charged => "complete",
            Charging::Unknown | Charging::Other(_) => "unknown",
        }
    }
}

/// Which batteries one message reported.
///
/// A device reports one shape or the other, never both: the over-ears have a
/// main battery and no case, the buds have left, right and case and no main.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BatteryReport {
    pub main: Option<Cell>,
    pub left: Option<Cell>,
    pub right: Option<Cell>,
    pub case: Option<Cell>,
}

/// Off, cancelling, or letting the room in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoiseMode {
    Off,
    /// Noise cancelling.
    Anc,
    /// Ambient sound.
    Ambient,
}

impl NoiseMode {
    /// The word the state document uses.
    pub fn as_str(self) -> &'static str {
        match self {
            NoiseMode::Off => "off",
            NoiseMode::Anc => "anc",
            NoiseMode::Ambient => "ambient",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoiseReport {
    pub mode: NoiseMode,
    /// `NcAsmMode` verbatim -- 0x00 NC, 0x01 ASM -- or `None` on a variant
    /// that has no mode byte at all.
    ///
    /// KEPT BECAUSE [`NoiseReport::mode`] LOSES IT. `mode` collapses to `Off`
    /// whenever the total effect is switched off, which is what a panel needs
    /// and which throws away a byte the headset sent. A SET message
    /// re-transmits every field, so an encoder that re-derived this from
    /// `mode` would write NC for a headset that was sitting in ambient with
    /// the feature off -- silently changing the mode the next time somebody
    /// turned it back on.
    ///
    /// Found by the write path's round trip on its first run, which is what
    /// that test is for.
    pub mode_byte: Option<u8>,
    /// The ambient sound level as the wire carries it, NOT a percentage, or
    /// `None` on the one variant that has no level (NC on/off only). See
    /// [`AmbientRange`] for where the level's real bound comes from.
    pub ambient_level: Option<u8>,
    /// `AmbientSoundMode::VOICE`: the filter that keeps speech and drops the
    /// rest. `None` where the variant has no ambient sound mode.
    pub focus_on_voice: Option<bool>,
    /// `ValueChangeStatus::CHANGED`. False means the headset is mid-change and
    /// this reading is about to be superseded.
    pub settled: bool,
    /// The noise-adaptation variant (0x19) only: `ncAsmOnOffValue` at offset
    /// 7, which libmdr reads as automatic ambient sound
    /// (`mNcAsmAutoAsmEnabled`, `HeadphonesV2T1.cpp:131-152`).
    pub adaptive: Option<bool>,
    /// The noise-adaptation variant only: `NoiseAdaptiveSensitivity` at
    /// offset 8 (T1:1203: STANDARD 0x00, HIGH 0x01, LOW 0x02), verbatim.
    pub adaptive_sensitivity: Option<u8>,
}

/// One ambient sound mode's level range, as a headset's
/// `NCASM_RET_CAPABILITY` reply states it.
///
/// `AmbientSoundModeLevelSet` (T1:2317): an `AmbientSoundMode` (NORMAL 0x00,
/// VOICE 0x01) and a `Range { min, max, step }` (`ProtocolV2.hpp:93`). This is
/// where the ambient level's bound actually lives -- per device and per mode
/// -- and why `ambient-level 1..20` in the command language is a
/// compatibility contract with the C++ daemon rather than a fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AmbientRange {
    /// `AmbientSoundMode::VOICE`: this range applies with focus on voice on.
    pub voice: bool,
    pub min: u8,
    pub max: u8,
    pub step: u8,
}

impl AmbientRange {
    /// Whether `level` is one a headset with this range accepts.
    pub fn admits(self, level: i32) -> bool {
        let (min, max, step) = (i32::from(self.min), i32::from(self.max), i32::from(self.step));
        (min..=max).contains(&level) && (level - min) % step == 0
    }

    /// The levels this range admits that also lie in `lo..=hi`, as a range on
    /// the same steps -- or `None` when there are none.
    ///
    /// For the document: the command language admits 1..=20 whatever a
    /// headset states (`sony::command`), so the levels a page can actually
    /// set are the ones BOTH admit. The result admits a level exactly when
    /// this range and `lo..=hi` both do, which `the_published_range_is_the_one_in_force`
    /// holds over every level.
    pub fn within(self, lo: u8, hi: u8) -> Option<AmbientRange> {
        let (min, max, step) = (u32::from(self.min), u32::from(self.max), u32::from(self.step).max(1));
        let (lo, hi) = (u32::from(lo), u32::from(hi));
        // The first level on this range's steps at or above `lo`, and the
        // last at or below both tops.
        let first = if min >= lo { min } else { min + (lo - min).div_ceil(step) * step };
        let top = max.min(hi);
        if top < min {
            return None;
        }
        let last = min + (top - min) / step * step;
        if first > last {
            return None;
        }
        Some(AmbientRange { voice: self.voice, min: u8::try_from(first).ok()?, max: u8::try_from(last).ok()?, step: self.step })
    }
}

/// A string as the wire carries it: one length byte, then that many bytes.
///
/// `MDRPrefixedString` in the reference. The prefix is ONE byte, which the
/// captures confirm -- "WF-1000XM5" arrives as `0a` and ten bytes, "6.1.0" as
/// `05` and five. A two-byte prefix would have produced a model name starting
/// with a NUL and nobody would have questioned it.
///
/// Decoded lossily. These are shown to a person, and a headset whose name
/// carries a byte outside UTF-8 is better reported with a replacement
/// character than not reported.
fn prefixed_string(body: &[u8], at: usize) -> Option<String> {
    let len = usize::from(*body.get(at)?);
    let start = at.checked_add(1)?;
    let end = start.checked_add(len)?;
    let bytes = body.get(start..end)?;
    Some(String::from_utf8_lossy(bytes).into_owned())
}

/// What the headset says it is.
///
/// One message carries one of these, so the other is empty: the model name and
/// the firmware version arrive as separate frames.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeviceInfo {
    pub model: String,
    pub firmware: String,
}

/// The equaliser, as the wire carries it.
///
/// `steps` is the band-step array VERBATIM, not split into clear bass and
/// bands, and that is deliberate -- see [`decode_equalizer`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EqReport {
    /// `EqPresetId`. 0xA0 is CUSTOM, 0x00 is OFF, 0xA1..=0xA5 are the five user
    /// slots; the rest are Sony's named presets.
    pub preset: u8,
    /// One byte per step, in wire order. Six for a 1000X.
    pub steps: Vec<u8>,
}

/// One of the headset's named on/off settings, as its capability reply
/// describes it.
///
/// THE DEVICE NAMES THESE, WE DO NOT. There is no fixed list: the support
/// tables advertise up to four `GENERAL_SETTING_n` slots and the device says
/// what each one is, in ASCII, in its own capability reply. A WF-1000XM5
/// answers `SIDETONE_SETTING` and `MULTIPOINT_SETTING`; a WH-1000XM5 answers
/// `TOUCH_PANEL_SETTING` and `MULTIPOINT_SETTING`
/// (`delarosa1312/omarchy-sony-xm5`, `docs/protocol.md`). So a port that hard
/// coded the pair would be right on one of the two and silently wrong on the
/// other, and this carries the strings through instead.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GeneralSetting {
    /// The slot this arrived at -- 0xD1..=0xD4 -- which is also the inquired
    /// type a GET or a SET for it must carry.
    pub slot: u8,
    /// `SIDETONE_SETTING`, `MULTIPOINT_SETTING`, `TOUCH_PANEL_SETTING`.
    pub name: String,
    /// The longer key beside it, e.g.
    /// `MULTIPOINT_SETTING_SUMMARY_LDAC_AVAILABLE`. Carried because it is the
    /// only place the device says WHICH variant of a setting it has -- the LDAC
    /// suffix above is the device telling us the multipoint switch trades
    /// against the codec.
    pub summary: String,
}

impl GeneralSetting {
    /// The device's own name as a document key: lowercased, `_SETTING` trimmed.
    ///
    /// ONE COPY, because two would drift. The state document writes this into
    /// `general_settings[].key` and `mdr::encode` matches a `general` verb's
    /// argument against it, so a reader and a writer that each lowercased in
    /// their own way would disagree about which setting `multipoint` names.
    ///
    /// TRIMMED ONLY WHERE THE SUFFIX IS THERE. `MULTIPOINT_SETTING` becomes
    /// `multipoint` and `TOUCH_PANEL_SETTING` becomes `touch_panel`, but a
    /// name without the suffix survives whole -- guessing at a shape no
    /// capture contains is the alternative, and `name` carries the untouched
    /// original beside it either way.
    pub fn key(&self) -> String {
        self.name.strip_suffix("_SETTING").unwrap_or(&self.name).to_ascii_lowercase()
    }
}

/// One entry of the headset's own paired-device list.
///
/// THE ADDRESS IS HERE AND THE HEADSET'S OWN ADDRESS IS NOT, and the two are
/// different decisions rather than an inconsistency. `state::Device` says
/// "never a Bluetooth address" about the headset the bar is naming, because
/// there is exactly one of it and an address where a name should be reads as a
/// fault. This list has eight entries in the one capture and **two pairs of
/// them share a name** -- two `KUWABARA` and two `KOMAHUANG-PC2`. A name is
/// not an identity here, so the address travels with it or the list cannot say
/// which row is which.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PairedDevice {
    /// `E8:9C:25:5D:B8:19`, as seventeen ASCII bytes on the wire -- NOT
    /// length-prefixed, unlike every other string in this protocol.
    pub address: String,
    /// What the other machine calls itself, length-prefixed.
    pub name: String,
    /// The first of the four bytes between the address and the name.
    ///
    /// READ, NAMED CAUTIOUSLY, AND NOT INTERPRETED HERE. In the one capture it
    /// is 0x01 on the first entry, 0x02 on the second and 0x00 on the other
    /// six -- and a multipoint headset holds exactly two links. So "non-zero
    /// means the headset is talking to this one" fits, and the document
    /// derives `connected` from it on that basis, with this raw byte published
    /// beside the derivation so a reader can check the inference rather than
    /// inherit it. What the corpus CANNOT rule out is that 0x01 and 0x02 name
    /// a profile or a slot index rather than a state; a second capture with
    /// one device disconnected would settle it.
    pub status: u8,
    /// The other three bytes, unread.
    ///
    /// `2a 41 04` and `5a 02 0c` on the two entries whose `status` is
    /// non-zero, `ff ff ff` on the six where it is zero. All-0xFF beside a
    /// zero status reads as "not applicable", which makes these look like
    /// details of a live link -- a codec, a profile mask, a signal figure.
    /// One capture cannot say which, so they are carried and not named; a
    /// decoder that called them a codec would be inventing a label the corpus
    /// does not support.
    pub detail: [u8; 3],
}

/// What a band-step array of a given length means.
///
/// ONE TABLE, BECAUSE THERE WERE ABOUT TO BE TWO. `sony::session` has read the
/// step array since the decoder existed, splitting it by length and subtracting
/// a bias; `mdr::encode` now has to put one back together for `eq-band` and
/// `clear-bass`, and a reader and a writer that each carry their own copy of
/// "six steps means clear bass plus five bands at bias ten" is the drift
/// `command::EQ_PRESETS` already has a paragraph about. So the split lives
/// here, next to the `EqReport` it describes, and both call it.
///
/// libmdr's `HeadphonesV2T1.cpp`, the `EqEbbRetParamEq` arm
/// (`switch (bandSteps.size())`), is where the two named shapes come from;
/// Gadgetbridge's `SonyProtocolImplV1.handleEqualizer` reads the six the same
/// way. Neither names any other count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EqShape {
    /// Six steps: clear bass, then the five bands 400 Hz / 1 k / 2.5 k / 6.3 k
    /// / 16 k. Every one biased +10, so the wire runs 0..=20 for -10..=+10.
    ClearBassAndFive,
    /// Ten steps: ten bands biased +6, and no clear bass at all. The narrower
    /// range is the device's, not a choice -- upstream's `BAND_LIMITS` agrees
    /// (`{5: 10, 10: 6}`), reading it out of libmdr's own validation.
    TenBands,
    /// Any other count, including zero.
    ///
    /// A READER MAY GUESS HERE AND A WRITER MAY NOT, which is the whole reason
    /// this is a third variant rather than an `Option`. `sony::session` treats
    /// an unknown count as bands at bias ten by analogy and leaves clear bass
    /// alone, because showing a curve that might be off by a constant beats
    /// showing nothing. `mdr::encode` refuses: putting a guessed bias back on
    /// the wire writes a curve nobody asked for.
    Unknown,
}

impl EqShape {
    pub fn of(steps: usize) -> EqShape {
        match steps {
            6 => EqShape::ClearBassAndFive,
            10 => EqShape::TenBands,
            _ => EqShape::Unknown,
        }
    }

    /// What to subtract from a wire byte to get the signed step, and to add
    /// going the other way.
    ///
    /// `Unknown` answers 10 because that is what the reader has always applied
    /// by analogy to the shapes that are named; it is not a claim about a
    /// device, and the writer never asks.
    pub fn bias(self) -> i32 {
        match self {
            EqShape::TenBands => 6,
            EqShape::ClearBassAndFive | EqShape::Unknown => 10,
        }
    }

    /// The furthest a band may be moved either way, which is the bias: a wire
    /// byte of 0 is the bias below flat and 2x the bias is the bias above it.
    pub fn bound(self) -> i32 {
        self.bias()
    }

    /// Where clear bass sits in the array, when the shape has one.
    pub fn clear_bass_at(self) -> Option<usize> {
        match self {
            EqShape::ClearBassAndFive => Some(0),
            EqShape::TenBands | EqShape::Unknown => None,
        }
    }

    /// How many bands the shape has, not counting clear bass.
    pub fn bands(self) -> usize {
        match self {
            EqShape::ClearBassAndFive => 5,
            EqShape::TenBands => 10,
            EqShape::Unknown => 0,
        }
    }
}

/// A setting the wire reports as on, off, or something else.
///
/// Three states rather than a `bool`, because these enums all declare
/// `OUT_OF_RANGE = 0xFF` and a headset that sends it is saying "no value",
/// which is not the same as "off". A panel can show "unknown"; a `bool` cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    On,
    Off,
    /// A byte neither value names, `OUT_OF_RANGE` included.
    Unknown(u8),
}

impl Switch {
    /// `OnOffSettingValue`, WHICH IS THE OTHER WAY ROUND: `ON = 0x00` and
    /// `OFF = 0x01`.
    ///
    /// This is the battery trap wearing different clothes. The obvious reading
    /// -- 0 is off, 1 is on -- inverts every setting that uses this enum, and
    /// the captured WF-1000XM5 sends `f7 0c 01 01`, so Speak-to-Chat would be
    /// reported ON when the device says OFF. Nothing about the byte would look
    /// wrong.
    ///
    /// The protocol uses BOTH conventions, in enums a paragraph apart:
    /// `NcAsmOnOffValue` and `UpscalingTypeAutoOff` are the ordinary way round,
    /// `OnOffSettingValue` and `EnableDisable` are inverted. So the polarity is
    /// named at every use rather than assumed once.
    pub fn from_on_off_setting(byte: u8) -> Switch {
        match byte {
            0x00 => Switch::On,
            0x01 => Switch::Off,
            other => Switch::Unknown(other),
        }
    }

    /// `EnableDisable` (`ProtocolV2.hpp:14`): `ENABLE = 0x00`, `DISABLE =
    /// 0x01` -- inverted like [`Switch::from_on_off_setting`], and a separate
    /// constructor so each call site names the enum it is reading rather than
    /// borrowing another's polarity because the bytes happen to agree.
    pub fn from_enable_disable(byte: u8) -> Switch {
        match byte {
            0x00 => Switch::On,
            0x01 => Switch::Off,
            other => Switch::Unknown(other),
        }
    }

    /// `UpscalingTypeAutoOff`: `OFF = 0x00`, `AUTO = 0x01`. The ordinary way
    /// round, and adjacent in the same header to one that is not.
    pub fn from_auto_off(byte: u8) -> Switch {
        match byte {
            0x00 => Switch::Off,
            0x01 => Switch::On,
            other => Switch::Unknown(other),
        }
    }

    /// `Some` only when the wire actually said one or the other.
    pub fn as_bool(self) -> Option<bool> {
        match self {
            Switch::On => Some(true),
            Switch::Off => Some(false),
            Switch::Unknown(_) => None,
        }
    }

    /// The inverse of [`Switch::from_on_off_setting`], and INVERTED the same
    /// way: `On` is 0x00.
    ///
    /// `None` for [`Switch::Unknown`], because there is no byte that means "no
    /// value". A caller with an unknown switch has nothing to write and must
    /// refuse rather than pick one.
    pub fn to_on_off_setting(self) -> Option<u8> {
        match self {
            Switch::On => Some(0x00),
            Switch::Off => Some(0x01),
            Switch::Unknown(_) => None,
        }
    }

    /// The inverse of [`Switch::from_auto_off`]: `Off` is 0x00, the ordinary
    /// way round.
    pub fn to_auto_off(self) -> Option<u8> {
        match self {
            Switch::Off => Some(0x00),
            Switch::On => Some(0x01),
            Switch::Unknown(_) => None,
        }
    }
}

/// `PriorMode` (T1:1399): what the headset favours on the radio link.
///
/// The document's words are the C++ daemon's `connection.priority`: `quality`
/// for SOUND_QUALITY_PRIOR and `stable` for CONNECTION_QUALITY_PRIOR. The third
/// value, LOW_LATENCY_PRIOR_BETA, has no word there and no verb here, so it is
/// published as `unknown` rather than given a name the page would not draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorMode {
    SoundQuality,
    ConnectionQuality,
    LowLatencyBeta,
    Other(u8),
}

impl PriorMode {
    pub fn from_byte(byte: u8) -> PriorMode {
        match byte {
            0x00 => PriorMode::SoundQuality,
            0x01 => PriorMode::ConnectionQuality,
            0x02 => PriorMode::LowLatencyBeta,
            other => PriorMode::Other(other),
        }
    }

    pub fn to_byte(self) -> u8 {
        match self {
            PriorMode::SoundQuality => 0x00,
            PriorMode::ConnectionQuality => 0x01,
            PriorMode::LowLatencyBeta => 0x02,
            PriorMode::Other(byte) => byte,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            PriorMode::SoundQuality => "quality",
            PriorMode::ConnectionQuality => "stable",
            PriorMode::LowLatencyBeta | PriorMode::Other(_) => "unknown",
        }
    }
}

/// `PlaybackStatus` (T1:1307): what the source is doing.
///
/// NOT `PlaybackControl` (T1:1284), which is what a SET sends and shares value
/// NAMES with this enum at different bytes -- PAUSE is 0x02 here and 0x01
/// there. The two are separate types so no byte can cross between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackStatus {
    Unsettled,
    Play,
    Pause,
    Stop,
    Other(u8),
}

impl PlaybackStatus {
    pub fn from_byte(byte: u8) -> PlaybackStatus {
        match byte {
            0x00 => PlaybackStatus::Unsettled,
            0x01 => PlaybackStatus::Play,
            0x02 => PlaybackStatus::Pause,
            0x03 => PlaybackStatus::Stop,
            other => PlaybackStatus::Other(other),
        }
    }

    /// The C++ daemon's `playback.status` words. Its `fast_forward` and
    /// `rewind` have no value in this enum, so they are never produced here.
    pub fn as_str(self) -> &'static str {
        match self {
            PlaybackStatus::Play => "playing",
            PlaybackStatus::Pause => "paused",
            PlaybackStatus::Stop => "stopped",
            PlaybackStatus::Unsettled | PlaybackStatus::Other(_) => "unknown",
        }
    }
}

/// The half of a play-status reply both colliding structs agree on.
///
/// `RetPlayStatusPlaybackController` (T1:7451) and
/// `RetPlayStatusPlaybackControlWithFunctionChange` (T1:7434) are both five
/// bytes at PLAYBACK_CONTROL_WITH_CALL_VOLUME_ADJUSTMENT and differ at offset
/// 4 alone -- a `MusicCallStatus` in one and an `EnableDisable` in the other.
/// Offsets 2 and 3 are the same field of the same type in both, so those two
/// are read and offset 4 is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayStatusReport {
    /// Offset 2, `EnableDisable status`: whether the headset will take a
    /// transport command now. ENABLE is 0x00.
    pub control: Switch,
    /// Offset 3.
    pub status: PlaybackStatus,
}

/// What one voice-guidance reply said. Each message fills its own fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VoiceGuidanceReport {
    pub enabled: Option<Switch>,
    /// `VoiceGuidanceLanguage` (T2:620), raw; [`voice_guidance_language`]
    /// names it.
    pub language: Option<u8>,
    /// -2..=2, zero centred.
    pub volume: Option<i8>,
}

/// `VoiceGuidanceLanguage` (T2:620) as a lowercase word, `unknown` for a byte
/// it does not name.
pub fn voice_guidance_language(byte: u8) -> &'static str {
    match byte {
        0x00 => "undefined",
        0x01 => "english",
        0x02 => "french",
        0x03 => "german",
        0x04 => "spanish",
        0x05 => "italian",
        0x06 => "portuguese",
        0x07 => "dutch",
        0x08 => "swedish",
        0x09 => "finnish",
        0x0A => "russian",
        0x0B => "japanese",
        0x0D => "brazilian_portuguese",
        0x0F => "korean",
        0x10 => "turkish",
        0xF0 => "chinese",
        _ => "unknown",
    }
}

/// `SafeListeningRetExtendedParam`'s two value bytes, unread beyond naming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SafeListeningReport {
    /// `levelPerPeriod`, `CODEGEN Range 0 255`.
    pub level: u8,
    /// `SafeListeningErrorCause` (T2:424), raw.
    pub cause: u8,
}

impl SafeListeningReport {
    /// The cause as a lowercase word: `not_playing`, `in_call`, `detached`,
    /// or `unknown`.
    pub fn cause_word(self) -> &'static str {
        match self.cause {
            0x00 => "not_playing",
            0x01 => "in_call",
            0x02 => "detached",
            _ => "unknown",
        }
    }
}

/// `UpscalingType`: which DSEE this headset has.
///
/// The variants carry the header's names; the document's words come from
/// [`Upscaling::as_str`] and are the C++ daemon's, which drop the `dsee`
/// prefix -- `DSEE` on the wire is the document's `standard`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upscaling {
    DseeHx,
    Dsee,
    DseeHxAi,
    DseeUltimate,
    Other(u8),
}

impl Upscaling {
    fn from_byte(byte: u8) -> Upscaling {
        match byte {
            0x00 => Upscaling::DseeHx,
            0x01 => Upscaling::Dsee,
            0x02 => Upscaling::DseeHxAi,
            0x03 => Upscaling::DseeUltimate,
            other => Upscaling::Other(other),
        }
    }

    /// The word the state document uses: the C++ daemon's `dseeName`.
    pub fn as_str(self) -> &'static str {
        match self {
            Upscaling::DseeHx => "hx",
            Upscaling::Dsee => "standard",
            Upscaling::DseeHxAi => "hx_ai",
            Upscaling::DseeUltimate => "ultimate",
            Upscaling::Other(_) => "unknown",
        }
    }
}

/// `AudioCodec` (T1:228): what the A2DP link is carrying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// `UNSETTLED`: the headset has not decided yet.
    Unsettled,
    Sbc,
    Aac,
    Ldac,
    AptX,
    AptXHd,
    Lc3,
    /// A byte the header does not name, `OTHER` (0xFF) included.
    Other(u8),
}

impl Codec {
    pub fn from_byte(byte: u8) -> Codec {
        match byte {
            0x00 => Codec::Unsettled,
            0x01 => Codec::Sbc,
            0x02 => Codec::Aac,
            0x10 => Codec::Ldac,
            0x20 => Codec::AptX,
            0x21 => Codec::AptXHd,
            0x30 => Codec::Lc3,
            other => Codec::Other(other),
        }
    }

    /// The word the state document uses: the C++ daemon's `codecName`.
    pub fn as_str(self) -> &'static str {
        match self {
            Codec::Sbc => "sbc",
            Codec::Aac => "aac",
            Codec::Ldac => "ldac",
            Codec::AptX => "aptx",
            Codec::AptXHd => "aptx_hd",
            Codec::Lc3 => "lc3",
            Codec::Unsettled | Codec::Other(_) => "unknown",
        }
    }
}

/// What one AUDIO message said about upscaling.
///
/// One message carries one half, so the other is `None`: the TYPE comes from
/// `AUDIO_RET_CAPABILITY` and the on/off from `AUDIO_RET_PARAM`. `None` here
/// means "not in this message"; a value the wire did not name is
/// [`Switch::Unknown`] or [`Upscaling::Other`], never `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DseeReport {
    pub upscaling: Option<Upscaling>,
    pub enabled: Option<Switch>,
}

/// `DetectSensitivity`: how readily Speak-to-Chat decides you are talking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sensitivity {
    Auto,
    High,
    Low,
    Other(u8),
}

impl Sensitivity {
    fn from_byte(byte: u8) -> Sensitivity {
        match byte {
            0x00 => Sensitivity::Auto,
            0x01 => Sensitivity::High,
            0x02 => Sensitivity::Low,
            other => Sensitivity::Other(other),
        }
    }

    /// The word the state document uses.
    pub fn as_str(self) -> &'static str {
        match self {
            Sensitivity::Auto => "auto",
            Sensitivity::High => "high",
            Sensitivity::Low => "low",
            Sensitivity::Other(_) => "unknown",
        }
    }
}

/// `ModeOutTime`: how long Speak-to-Chat waits before it ends by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timeout {
    Short,
    Medium,
    Long,
    /// `ModeOutTime::NONE`: it does not end by itself.
    Manual,
    Other(u8),
}

impl Timeout {
    fn from_byte(byte: u8) -> Timeout {
        match byte {
            0x00 => Timeout::Short,
            0x01 => Timeout::Medium,
            0x02 => Timeout::Long,
            0x03 => Timeout::Manual,
            other => Timeout::Other(other),
        }
    }

    /// The word the state document uses.
    ///
    /// A RENAME, not a transcription: the reference calls these FAST, MID,
    /// SLOW and NONE, and the document calls them short, medium, long and
    /// manual. The first three line up in order and in meaning. `NONE` to
    /// `manual` is the one that needed a decision -- a mode that never times
    /// out is one you end yourself -- and it is recorded here so that nobody
    /// later reads "manual" as a fifth value the protocol has.
    pub fn as_str(self) -> &'static str {
        match self {
            Timeout::Short => "short",
            Timeout::Medium => "medium",
            Timeout::Long => "long",
            Timeout::Manual => "manual",
            Timeout::Other(_) => "unknown",
        }
    }
}

/// What one SYSTEM message said about Speak-to-Chat.
///
/// Two messages, two halves, same rule as [`DseeReport`]: the switches come
/// from `SYSTEM_RET_PARAM` and the tuning from `SYSTEM_RET_EXT_PARAM`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SpeakToChatReport {
    pub enabled: Option<Switch>,
    /// `previewModeOnOffValue`: Sony's "preview" of the effect while you set
    /// it up. The state document has nowhere to put it; carried because the
    /// message carries it and dropping a field on the way past is how a
    /// re-encode stops round-tripping.
    pub preview: Option<Switch>,
    pub sensitivity: Option<Sensitivity>,
    pub timeout: Option<Timeout>,
}

/// Which of the two command channels a payload arrived on.
///
/// THIS IS PART OF THE MESSAGE'S IDENTITY, not decoration. A headset answers
/// `CONNECT_GET_SUPPORT_FUNCTION` twice, once per channel, and the two replies
/// are identical in shape while their function-code bytes come from two
/// DIFFERENT enums. The captured WF-1000XM5 sends 0x42 and 0xF1 on BOTH
/// channels, meaning four different features:
///
/// | code | table 1                                    | table 2                                  |
/// |------|--------------------------------------------|------------------------------------------|
/// | 0x42 | `CLASSIC_ONLY_LE_CLASSIC_SETTING`          | voice guidance, with volume adjustment   |
/// | 0xF1 | `PLAYBACK_CONTROL_BY_WEARING_REMOVING_...` | `REPEAT_TAP_TRAINING_MODE`               |
/// | 0x53 | `PRESET_EQ_AND_ULT_MODE`                   | `SAFE_LISTENING_TWS_2`                   |
///
/// So a decoder that read one enum for both channels would report this device
/// as having no voice guidance and an auto-pause it does not have, and would
/// read its safe-listening support as an equaliser. Every byte would still be
/// a declared value of the enum it was checked against.
///
/// `ConnectRetProtocolInfo` carries `supportTable1Value` and
/// `supportTable2Value`, which is the reference saying the same thing: the two
/// tables are separate namespaces that a device opts into separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Table {
    /// `DATA_MDR`, and `mdr::v2::t1::FunctionType`.
    One,
    /// `DATA_MDR_NO2`, and `mdr::v2::t2::FunctionType`.
    Two,
}

impl Table {
    /// The table a frame's payload is keyed to, or `None` for a frame type
    /// that carries no support-function namespace.
    ///
    /// Only the two DATA types map. The `SHOT_*` twins are not included: no
    /// capture shows a support-function list arriving as a shot, and a name
    /// that looks like it ought to work is not evidence. An unmapped type
    /// leaves the message [`Reading::Unhandled`], which a daemon can log and
    /// somebody can fix with one capture -- the recoverable failure, rather
    /// than a plausible list of the wrong features.
    pub fn of(data_type: super::DataType) -> Option<Table> {
        match data_type {
            super::DataType::DataMdr => Some(Table::One),
            super::DataType::DataMdrNo2 => Some(Table::Two),
            _ => None,
        }
    }
}

/// One element of `MDRPodArray<SupportFunction>`: a feature the headset has.
///
/// `SupportFunction` in `ProtocolV2.hpp` is exactly two fields, and this is
/// them verbatim. `code` is a `t1::FunctionType` or a `t2::FunctionType`
/// depending on the [`Table`], which is why the two never travel apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SupportFunction {
    /// The `FunctionType` byte, in the table's own namespace.
    pub code: u8,
    /// The reference declares this field as a bare `UInt8 priority` and says
    /// nothing else about it. In both captures the non-0xFF values are all
    /// DISTINCT -- 26 of them across 46 functions on table 1 -- which is what
    /// an ordering looks like and not what a flag or a version looks like, but
    /// no capture settles it and nothing here reads it. Carried anyway:
    /// dropping a field on the way past is how a re-encode stops round-tripping.
    pub priority: u8,
}

/// The priority byte for a function Sony's app does not place.
///
/// 0xFF IS ALSO A VALID FUNCTION CODE -- `HEAD_GESTURE_ON_OFF_TRAINING` on
/// table 1 and `WEARING_POSITION_WITHOUT_FITTING_SUPPORTER` on table 2 -- so
/// this constant must never be used to test `code`, and a parser that treated
/// 0xFF as an end-of-list sentinel would silently drop the head-gesture
/// feature. The captured WF-1000XM5 carries `ff 2d`: code 0xFF with a real
/// priority, thirty-third of forty-six.
pub const NO_PRIORITY: u8 = 0xFF;

/// What one message said, in the state document's own terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    Battery(BatteryReport),
    Noise(NoiseReport),
    Device(DeviceInfo),
    Equalizer(EqReport),
    /// Music volume, on the DEVICE scale of 0..=30 rather than as a
    /// percentage. Converting is `sony::command::mdr_volume_to_percent`, and it
    /// happens where the state document is written rather than here: this layer
    /// reports what the headset said.
    Volume { device_level: u8 },
    /// The A2DP codec in use. CAPTURED: fixture 000053 is `13 02 02`,
    /// `CommonRetStatusAudioCodec` at AUDIO_CODEC, `AudioCodec::AAC`.
    Codec(Codec),
    /// DSEE: which upscaler the headset has, or whether it is switched on.
    Dsee(DseeReport),
    /// Whether DSEE can run now: `AUDIO_RET_STATUS` at UPSCALING, read as
    /// `AudioRetStatusCommon`'s `EnableDisable` the way libmdr reads it.
    /// CAPTURED: fixture 000097 is `e3 01 00`, ENABLE. See
    /// [`decode_upscaling_status`].
    UpscalingStatus { available: Switch },
    /// The radio-link priority, at the inquired type the headset answered at:
    /// CONNECTION_MODE (0x00) or CONNECTION_MODE_WITH_LDAC_STATUS (0x02). The
    /// type is carried because it chooses the SET. See [`decode_connection`].
    Connection { kind: u8, prior: PriorMode },
    /// Whether the source is playing, and whether the headset will take a
    /// transport command. CAPTURED: fixture 000065 is `a3 01 00 02 00`. See
    /// [`decode_play_status`].
    PlayStatus(PlayStatusReport),
    /// Speak-to-Chat: the switches, or the sensitivity and timeout.
    SpeakToChat(SpeakToChatReport),
    /// What one of the headset's named on/off settings IS.
    ///
    /// CAPTURED, both of them -- one citation per line, because
    /// `mdr_citations.rs` pairs a fixture counter with the bytes beside it and
    /// two captures in one paragraph get cross-matched (which is how the first
    /// draft of this comment failed):
    ///
    /// - 000031: `d1 d1 00 01 10` then `SIDETONE_SETTING`, `18` then
    ///   `SIDETONE_SETTING_SUMMARY`
    /// - 000039: `d1 d2 00 01 12` then `MULTIPOINT_SETTING`, `29` then
    ///   `MULTIPOINT_SETTING_SUMMARY_LDAC_AVAILABLE`
    GeneralSettingInfo(GeneralSetting),
    /// The ambient level's range per ambient sound mode, at the inquired type
    /// the headset answered. HEADER ONLY: no capture holds an
    /// `NCASM_RET_CAPABILITY`. See [`decode_noise_capability`].
    NoiseCapability { kind: u8, ranges: Vec<AmbientRange> },
    /// The headset's spoken prompts: on or off, in which language, how loud.
    /// TABLE TWO -- see [`decode_voice_guidance`].
    VoiceGuidance(VoiceGuidanceReport),
    /// Safe listening's sound-pressure figure, raw. TABLE TWO -- see
    /// [`decode_safe_listening`].
    SafeListening(SafeListeningReport),
    /// Whether the headset is discoverable for pairing. TABLE TWO -- see
    /// [`decode_pairing_mode`].
    PairingMode { on: bool },
    /// The headset asking for a confirmation, at FIXED_MESSAGE: what about,
    /// and what kind of answer it wants. Both bytes raw; see
    /// [`decode_alert`].
    Alert { message: u8, action: u8 },
    /// Every device this headset is paired with, and which of them it is
    /// talking to. TABLE TWO -- see [`decode_paired_devices`].
    ///
    /// CAPTURED: 000077, `37 02` then 262 bytes.
    PairedDevices(Vec<PairedDevice>),
    /// Whether one of them is on.
    ///
    /// CAPTURED, one per line for the reason above:
    ///
    /// - 000035: `d7 d1 00 01`
    /// - 000043: `d7 d2 00 00`
    ///
    /// So on the captured WF-1000XM5, sidetone (slot 0xD1) is OFF and
    /// multipoint (slot 0xD2) is ON. The last byte is a `GsSettingValue`,
    /// whose ON is 0x00 -- see [`decode_general_setting_param`] for the
    /// evidence, and for the years this said the opposite.
    GeneralSettingParam {
        slot: u8,
        enabled: bool,
    },
    /// Every feature the headset says it has, on one of the two tables.
    ///
    /// A device sends two of these, one per table, and the two lists must be
    /// folded together: neither is the whole answer. See
    /// [`crate::sony::state::Capabilities::apply_support_functions`], which is
    /// where the codes become the state document's flags.
    SupportFunctions { table: Table, functions: Vec<SupportFunction> },
    /// Recognised as a message and not decoded.
    ///
    /// Carries what it was, so a daemon can log the gap rather than discard it
    /// silently: `command`, the kind byte, and the payload length are exactly
    /// the dispatch key, which is what somebody adding a decoder needs.
    Unhandled { command: u8, kind: Option<u8>, len: usize },
    /// The dispatch key is one of the collisions PROVENANCE.md enumerates.
    ///
    /// REFUSED, not guessed. Two structs in the reference claim this key and
    /// they read the same byte as different things; no capture settles which,
    /// and a decoder that picked one would produce a plausible number from the
    /// wrong byte forever.
    Ambiguous { command: u8, kind: Option<u8>, len: usize, between: [&'static str; 2] },
}

/// The `(command, kind, len)` keys where the reference is genuinely ambiguous
/// AND this daemon might see the message.
///
/// The full list of 21 is in PROVENANCE.md; this is the one on a command this
/// port speaks that it still refuses. The rest are firmware update, TWS
/// topology, SAR and general settings, none of which this daemon touches.
///
/// THE PLAY-STATUS PAIR USED TO BE HERE AND IS NOT ANY MORE, and not because
/// the collision went away. `(0xA3|0xA5, 0x01, 5)` is still two structs that
/// read offset 4 differently -- but they agree, field for field, on offsets 2
/// and 3, and those two are all the document has a place for. So
/// [`decode_play_status`] reads exactly the agreed half and never the byte in
/// dispute: a partial decode is not a guess when the part it declines is the
/// only part the two candidates disagree on.
/// Matched against the RAW command byte, before any RET/NTFY canonicalising:
/// the NcAsm collision below exists in the NTFY family and NOT in the RET one,
/// because the two RET structs are six and seven bytes while both NTFY structs
/// are seven. Canonicalising first would refuse a decodable RET message and let
/// the genuinely ambiguous NTFY one through.
const AMBIGUOUS: &[(u8, u8, usize, [&str; 2])] = &[
    // NCASM_NTFY_PARAM, NC_ON_OFF_AND_ASM_SEAMLESS (0x13). Offset 6 is
    // NcAsmOnOffValue in one and the ambient sound level in the other.
    (0x69, 0x13, 7, ["NcAsmNtfyParamNcOnOffAsmOnOff", "NcAsmNtfyParamNcOnOffAsmSeamless"]),
];

/// Decode one payload, in the context of the frame that carried it.
///
/// Total: any sequence of bytes goes in and a `Reading` comes out. An empty
/// payload is `Unhandled` rather than an error, because an empty payload is
/// what an ACK carries and an ACK is not a fault.
///
/// WHY THE FRAME'S DATA TYPE IS AN ARGUMENT. Every decoder below ignores it
/// but one, and that one cannot be written without it: a support-function list
/// is a list of bytes from a namespace the DATA TYPE selects, and the same byte
/// names a different feature on each. See [`Table`]. Passing it here rather
/// than reaching for it later means the type system asks the question at every
/// call site, which is the only place the answer is actually known.
pub fn decode(data_type: super::DataType, payload: &[u8]) -> Reading {
    let Some(message) = Message::split(payload) else {
        return Reading::Unhandled { command: 0, kind: None, len: 0 };
    };
    let unhandled = Reading::Unhandled {
        command: message.command.to_byte(),
        kind: message.kind,
        len: message.len,
    };

    // WHICH TABLE, BEFORE ANYTHING ELSE.
    //
    // Every decoder below this line reads a TABLE 1 message, and a command
    // byte alone does not say which table a frame is on. The two enums overlap
    // on 45 bytes; six of the ones this port names mean something else
    // entirely on table 2:
    //
    //     byte   this port calls it     table 2 declares it
    //     0x53   EQEBB_RET_STATUS       SAFE_LISTENING_RET_STATUS
    //     0x57   EQEBB_RET_PARAM        SAFE_LISTENING_RET_PARAM
    //     0x59   EQEBB_NTFY_PARAM       SAFE_LISTENING_NTFY_PARAM
    //     0x61   NCASM_RET_CAPABILITY   LEA_RET_CAPABILITY
    //     0x67   NCASM_RET_PARAM        LEA_RET_PARAM
    //     0x69   NCASM_NTFY_PARAM       LEA_NTFY_PARAM
    //
    // The captured WF-1000XM5 advertises safe listening AND LE Audio on table
    // 2, so this is a device that sends 0x57 and 0x67 frames meaning neither
    // of the things this port would call them. Ungated, a safe-listening reply
    // decodes as an equaliser preset and lands on the panel as a preset
    // nobody chose.
    //
    // AND A MATCHING NAME IS NOT A MATCHING MESSAGE. 0x23 is POWER_RET_STATUS
    // on both tables -- but `ProtocolV2T2.hpp:373` declares its OWN
    // `PowerInquiredType` reading AUTO_STANDBY = 0x00,
    // CARING_CHARGE_WITH_THRESHOLD = 0x01, USB_SUBMERSION = 0x02, where table
    // 1 has BATTERY, LEFT_RIGHT_BATTERY and CRADLE_BATTERY. A table-2 0x23 at
    // kind 0x00 is a standby setting, and `decode_battery` would report its
    // next byte as a charge level. `SystemInquiredType` diverges the same way.
    //
    // So this is a whitelist of ONE: only DATA_MDR is table 1. DATA_MDR_NO2 is
    // table 2 and DATA_COMMON is a third namespace again.
    //
    // The single exception is the support-function reply, which is genuinely
    // table-neutral in its header -- 0x07 is CONNECT_RET_SUPPORT_FUNCTION in
    // both enums and `ConnectInquiredType::FIXED_VALUE` is 0x00 in both -- and
    // table-scoped only in its BODY, which `decode_support_functions` already
    // takes the table to read.
    let table = Table::of(data_type);
    if message.command == Command::ConnectRetSupportFunction {
        return match table {
            Some(table) => decode_support_functions(table, &message).unwrap_or(unhandled),
            None => unhandled,
        };
    }
    // TABLE TWO IS READ BY ITS OWN FUNCTION, DISPATCHING ON THE RAW BYTE.
    //
    // This used to be a bare `return unhandled`, and the paragraph above
    // explains why it had to be: a table-2 frame put through the table-1 match
    // produces a plausible reading of the wrong message. That argument is
    // about the NAMES, though, not about the channel -- the fix is a second
    // decoder, not a permanent silence. The corpus contains five table-2
    // frames beyond the support list. The paired-device list (000077) was
    // read before plan 23; the daemon logged the other four as undecodable
    // until then, and now reads all five.
    //
    // `decode_table_two` matches on `message.command.to_byte()` and never on a
    // `Command` variant, so no table-1 name can leak across. The round trip
    // through the enum is total -- `the_command_byte_round_trips_through_the_enum`
    // pins all 256 bytes -- so the byte it matches on is the byte off the wire.
    if table == Some(Table::Two) {
        return decode_table_two(&message).unwrap_or(unhandled);
    }
    if table != Some(Table::One) {
        return unhandled;
    }

    // Against the RAW command, before canonicalising: see the note on
    // AMBIGUOUS for why the order matters.
    let raw = message.command.to_byte();
    if let Some(kind) = message.kind {
        for (cmd, k, len, between) in AMBIGUOUS {
            if raw == *cmd && kind == *k && message.len == *len {
                return Reading::Ambiguous {
                    command: message.command.to_byte(),
                    kind: message.kind,
                    len: message.len,
                    between: *between,
                };
            }
        }
    }

    match message.command.canonical() {
        Command::PowerRetStatus => decode_battery(&message).unwrap_or(unhandled),
        Command::NcAsmRetParam => decode_noise(&message).unwrap_or(unhandled),
        Command::NcAsmRetCapability => decode_noise_capability(&message).unwrap_or(unhandled),
        Command::ConnectRetDeviceInfo => decode_device_info(&message).unwrap_or(unhandled),
        Command::EqEbbRetParam => decode_equalizer(&message).unwrap_or(unhandled),
        Command::GsRetCapability => decode_general_setting_info(&message).unwrap_or(unhandled),
        Command::GsRetParam => decode_general_setting_param(&message).unwrap_or(unhandled),
        Command::PlayRetParam => decode_volume(&message).unwrap_or(unhandled),
        Command::PlayRetStatus => decode_play_status(&message).unwrap_or(unhandled),
        Command::CommonRetStatus => decode_codec(&message).unwrap_or(unhandled),
        // Two commands, one feature: the capability reply carries which DSEE
        // and the param reply carries whether it is on. The param reply at
        // the two connection types is the link priority instead -- the
        // inquired type, not the command, separates them.
        Command::AudioRetCapability => decode_dsee(&message).unwrap_or(unhandled),
        Command::AudioRetParam => match message.kind {
            Some(0x00 | 0x02) => decode_connection(&message),
            _ => decode_dsee(&message),
        }
        .unwrap_or(unhandled),
        Command::AudioRetStatus => decode_upscaling_status(&message).unwrap_or(unhandled),
        Command::AlertNtfyParam => decode_alert(&message).unwrap_or(unhandled),
        // Same shape again: the param reply carries the switches and the
        // extended param reply carries the sensitivity and the timeout.
        Command::SystemRetParam | Command::SystemRetExtParam => {
            decode_speak_to_chat(&message).unwrap_or(unhandled)
        }
        _ => unhandled,
    }
}

/// Table 2's messages, which are a different namespace with the same bytes.
///
/// FOUR FAMILIES, ALL FIVE OF THE CORPUS'S TABLE-2 REPLIES. This read one of
/// them for a long time and refused the other four on stated grounds; plan 23
/// (item 11) went back to each ground with the reference client in hand, and
/// each turned out to be answerable without a second capture:
///
/// - `VOICE_GUIDANCE_RET_PARAM` (0x47) at 0x01, `00 01`. Refused because one
///   capture in one state cannot fix a polarity. The header can: the struct
///   (`VoiceGuidanceRetParamSettingMtkSupportLangSwitchAndSupportLangSwitch`)
///   names the byte an `OnOffSettingValue`, and libmdr reads it `== ON`
///   (`HeadphonesV2T2.cpp:36-40`). So `00` is ON and `01` is ENGLISH.
/// - The same at 0x20, `00`. Refused as "a volume on a scale nothing bounds",
///   citing an advertised `..._MTK_FIXED_TO_5_STEPS` (t2 0x43) that the
///   captured table does not in fact carry -- it advertises 0x42, the
///   VOLUME_ADJUSTMENT form. `VoiceGuidanceRetParamVolume` bounds it:
///   `// CODEGEN Range -2 2` on an `Int8`. Five steps, signed, zero centred.
/// - `SAFE_LISTENING_RET_EXTENDED_PARAM` (0x5B) at 0x03, `ff 00`. Refused
///   because 0xFF is a sentinel in some places. Here it is `UInt8
///   levelPerPeriod` with `// CODEGEN Range 0 255` -- a real value -- next to
///   `SafeListeningErrorCause::NOT_PLAYING`. Published raw, both of them:
///   what a level of 255 while not playing MEANS is not something this port
///   decides.
/// - `PERI_RET_STATUS` (0x33) at 0x02, `00 00`. Refused because the document
///   had nowhere to put it. It has now: libmdr reads these two bytes as
///   pairing mode, `btMode == INQUIRY_SCAN_MODE && status == ENABLE`
///   (`HeadphonesV2T2.cpp:66-74`), and so does this.
///
/// Every one of these is [`Grounds::ReferenceClient`] and not CAPTURED: the
/// bytes were captured, the MEANING is the header's and libmdr's.
fn decode_table_two(message: &Message) -> Option<Reading> {
    match message.command.to_byte() {
        // RET, and NTFY canonicalised onto it by the same arithmetic table 1
        // uses -- GET, RET, SET, NTFY at base+0..3. That law is PINNED on
        // table 1 over all 91 derivable pairs
        // (`the_command_families_are_numbered_in_fours`) and READ on table 2
        // from the one block the two tables both name: 0x53/0x57/0x59 is
        // EQEBB_RET_STATUS/_PARAM/NTFY_PARAM on table 1 and
        // SAFE_LISTENING_RET_STATUS/_PARAM/NTFY_PARAM on table 2, the same
        // three offsets apart. The corpus contains no table-2 NTFY frame, so
        // 0x39 is accepted on that reading and nothing more.
        //
        // It matters because the interesting case for this list is exactly the
        // volunteered one: a second phone connects and the headset says so
        // without being asked.
        0x37 | 0x39 => decode_paired_devices(message),
        // PERI_RET_STATUS and PERI_NTFY_STATUS: `PeripheralNotifyStatus
        // ParingDeviceManagementCommon` (T2:1685) and its RET twin (T2:1774)
        // are field-identical, and libmdr reads both the same way.
        0x33 | 0x35 => decode_pairing_mode(message),
        // RET only. Table 2 declares no VOICE_GUIDANCE_NTFY_PARAM and no
        // SAFE_LISTENING_NTFY_EXTENDED_PARAM, so there is no twin to accept.
        0x47 => decode_voice_guidance(message),
        0x5B => decode_safe_listening(message),
        _ => None,
    }
}

/// `VoiceGuidanceRetParam*` (T2:4071-4125): the spoken prompts.
///
/// Read by `(type, length)`, the way every table-1 decoder here is:
///
///   (0x00..=0x02, 3)  `...SettingMtk`: the switch alone.
///   (0x01|0x02, 4)    `...SupportLangSwitchAndSupportLangSwitch`: the switch,
///                     then the language. CAPTURED BYTES: `47 01 00 01`.
///   (0x03, 3)         `...SettingOnOff`, pinned: the switch alone.
///   (0x20, 3)         `...Volume`: an `Int8`, `CODEGEN Range -2 2`. CAPTURED
///                     BYTES: `47 20 00`. Out of range is not read.
///
/// The switch is `OnOffSettingValue`, INVERTED: 0x00 is on.
fn decode_voice_guidance(message: &Message) -> Option<Reading> {
    let kind = message.kind?;
    let mut report = VoiceGuidanceReport::default();
    match (kind, message.len) {
        (0x00..=0x03, 3) | (0x01 | 0x02, 4) => {
            report.enabled = Some(Switch::from_on_off_setting(*message.body.first()?));
            if message.len == 4 {
                report.language = Some(*message.body.get(1)?);
            }
        }
        (0x20, 3) => {
            let volume = i8::from_ne_bytes([*message.body.first()?]);
            if !(-2..=2).contains(&volume) {
                return None;
            }
            report.volume = Some(volume);
        }
        _ => return None,
    }
    Some(Reading::VoiceGuidance(report))
}

/// `SafeListeningRetExtendedParam` (T2:2398): the sound-pressure figure and
/// why it might not mean anything. CAPTURED BYTES: `5b 03 ff 00`.
///
/// Four bytes at any of the four safe-listening types -- the struct's type is
/// `Ignore`, and libmdr asks at whichever one the table advertises and reads
/// every reply as this struct (`HeadphonesV2.cpp:274-292`,
/// `HeadphonesV2T2.cpp:217-222`). 0x04 and 0x05 are SAFE_VOLUME_CONTROL and
/// MAX_VOL_LV_LIMIT, other settings with no capture, and are not read.
fn decode_safe_listening(message: &Message) -> Option<Reading> {
    if message.kind? > 0x03 || message.len != 4 {
        return None;
    }
    Some(Reading::SafeListening(SafeListeningReport {
        level: *message.body.first()?,
        cause: *message.body.get(1)?,
    }))
}

/// `PeripheralRetStatusPairingDeviceManagementCommon` (T2:1774) at
/// PAIRING_DEVICE_MANAGEMENT_WITH_BLUETOOTH_CLASS_OF_DEVICE: whether the
/// headset is discoverable for pairing. CAPTURED BYTES: `33 02 00 00` --
/// NORMAL_MODE, ENABLE, so not pairing.
///
/// Only 0x02, the type the captured family answers at and libmdr reads
/// (`HeadphonesV2T2.cpp:61-75`). A byte either enum does not name is not read:
/// "not pairing" would be a claim the headset did not make.
fn decode_pairing_mode(message: &Message) -> Option<Reading> {
    if message.kind? != 0x02 || message.len != 4 {
        return None;
    }
    let inquiry_scan = match *message.body.first()? {
        0x00 => false,
        0x01 => true,
        _ => return None,
    };
    let enabled = Switch::from_enable_disable(*message.body.get(1)?).as_bool()?;
    Some(Reading::PairingMode { on: inquiry_scan && enabled })
}

/// `PeripheralRetParam` at PAIRING_DEVICE_MANAGEMENT: who else this headset
/// is paired with.
///
/// CAPTURED: 000077, `37 02` then a 262-byte body. The largest payload in the
/// corpus by a factor of three, and the only one carrying a list of records.
///
/// THE LAYOUT, READ OFF THE BYTES:
///
/// ```text
///   08                     how many devices follow
///   "E8:9C:25:5D:B8:19"    seventeen raw ASCII bytes, no length prefix
///   01 2a 41 04            status, then three bytes this port does not read
///   0d "KOMAHUANG-PC2"     length-prefixed, like every other string here
///   ... seven more ...
///   01                     one byte left over
/// ```
///
/// WHY THAT SPLIT AND NOT ANOTHER. A record shape is a guess until it is
/// forced, and this one is forced by arithmetic: eight records of
/// `17 + 1 + 3 + 1 + n` consume 260 bytes, the count byte makes 261, and the
/// body is 262. Every one of the eight `n` values lands a clean printable
/// ASCII name, and the eighth ends one byte short of the end. Shift the
/// unexplained block to three bytes or move it before the address and the
/// second name is already garbage. So the only freedom left is what that last
/// byte is, which is the next paragraph.
///
/// THE TRAILING BYTE IS READ AND NOT NAMED. It is 0x01. `state::Capabilities`
/// records that the SET message this family enables carries a
/// `PeripheralBluetoothMode` of NORMAL_MODE or INQUIRY_SCAN_MODE, so a RET
/// that reported the current mode beside the list would be an unremarkable
/// design and 0x01 would be one of the two. So would "how many of these are
/// connected", which is also plausible and is NOT 0x01 in this capture --
/// two entries carry a non-zero status. One capture cannot choose, and the
/// decoder that guesses would put a wrong word in a document. What it does
/// instead is REQUIRE the byte: a body that does not end exactly one byte
/// after the last name is a different structure and is refused, so the day
/// this shape is wrong the port says so rather than publishing a short list.
///
/// AND NOTHING IS TRUSTED ABOUT THE COUNT. `count` is a byte off the wire and
/// the loop is bounded by the body, not by it: a device claiming 255 devices
/// in a 262-byte body runs out of bytes and the whole frame is refused, rather
/// than yielding however many records happened to fit.
fn decode_paired_devices(message: &Message) -> Option<Reading> {
    // PAIRING_DEVICE_MANAGEMENT. The other PeripheralInquiredType values carry
    // other structures and have no capture.
    if message.kind? != 0x02 {
        return None;
    }
    let body = message.body;
    let count = usize::from(*body.first()?);
    let mut at = 1usize;
    let mut devices = Vec::with_capacity(count.min(16));
    for _ in 0..count {
        let address = body.get(at..at.checked_add(17)?)?;
        at = at.checked_add(17)?;
        let status = *body.get(at)?;
        let detail = [*body.get(at + 1)?, *body.get(at + 2)?, *body.get(at + 3)?];
        at = at.checked_add(4)?;
        let len = usize::from(*body.get(at)?);
        let name = body.get(at.checked_add(1)?..at.checked_add(1)?.checked_add(len)?)?;
        at = at.checked_add(1)?.checked_add(len)?;
        devices.push(PairedDevice {
            address: String::from_utf8_lossy(address).into_owned(),
            name: String::from_utf8_lossy(name).into_owned(),
            status,
            detail,
        });
    }
    // Exactly one byte left, for the reason above.
    if at.checked_add(1)? != body.len() {
        return None;
    }
    Some(Reading::PairedDevices(devices))
}

/// `PLAY_RET_PARAM` at MUSIC_VOLUME: how loud the headset is.
///
/// CAPTURED: `a7 20 0c` -- MUSIC_VOLUME, level 12 of 30.
///
/// THE VALUE IS WHAT SETTLES WHICH STRUCT THIS IS, which is worth spelling out
/// because it is the only place in this module where that happens.
///
/// The type byte does not settle it: `MUSIC_VOLUME` (0x20) is declared by NO
/// struct in the header, so the usual index is empty here. Two structs are
/// three bytes long and could take this key --
/// `RetPlayParamPlaybackControllerVolume`, whose third byte is a `UInt8`
/// volume, and `RetPlayParamPlayMode`, whose third byte is a `PlayMode`.
///
/// `PlayMode` runs 0x00..=0x05 with 0xFF for out of range. The captured byte
/// is 0x0C, which is not any of them. So this frame cannot be a play mode, and
/// the volume reading is the only one left.
///
/// That reasoning is only as good as the byte, so it is enforced rather than
/// assumed: a level above the device's maximum of 30 is refused. A headset
/// reporting 200 is a headset this port has misread, and saying nothing is
/// better than putting 200 on a bar drawn 0..30.
fn decode_volume(message: &Message) -> Option<Reading> {
    // MUSIC_VOLUME. CALL_VOLUME (0x21) is a different number for a different
    // thing and the state document has nowhere to put it; the two *_WITH_MUTE
    // forms (0x30, 0x31) carry an extra byte and have no capture.
    if message.kind? != 0x20 || message.len != 3 {
        return None;
    }
    let level = *message.body.first()?;
    if i32::from(level) > crate::sony::command::MDR_VOLUME_MAX {
        return None;
    }
    Some(Reading::Volume { device_level: level })
}

/// `CommonRetStatusAudioCodec` (T1:3355) and its NTFY twin
/// `CommonNotifyStatusAudioCodec` (T1:3229): the A2DP codec.
///
/// CAPTURED: fixture 000053, `13 02 02` -- COMMON_RET_STATUS at AUDIO_CODEC,
/// `AudioCodec::AAC`. It is the answer to `COMMON_GET_STATUS` at AUDIO_CODEC
/// (`12 02`), the GET Gadgetbridge's `SonyProtocolImplV2.getAudioCodec` sends
/// and its `handleAudioCodec` reads at exactly three bytes with byte 1 == 0x02
/// -- the same guard as here.
///
/// Only AUDIO_CODEC (0x02). The other `CommonInquiredType`s on this command
/// are different structs: CONNECTION_STATUS (0x01) is four bytes with a left
/// and a right connection status (`CommonRetStatusConnection`, T1:3380), and
/// UPSCALING_EFFECT carries an effect type and a status. Both stay Unhandled.
fn decode_codec(message: &Message) -> Option<Reading> {
    if message.kind? != 0x02 || message.len != 3 {
        return None;
    }
    Some(Reading::Codec(Codec::from_byte(*message.body.first()?)))
}

/// `AudioRetCapabilityUpscaling` and `AudioRetParamUpscaling`: DSEE.
///
/// CAPTURED, both halves, from the same handshake:
///
///   e1 01 02   AUDIO_RET_CAPABILITY, UPSCALING, `UpscalingType::DSEE_HX_AI`
///   e7 01 01   AUDIO_RET_PARAM,      UPSCALING, `UpscalingTypeAutoOff::AUTO`
///
/// THE SAME OFFSET, THE SAME LENGTH, TWO DIFFERENT ENUMS. Both messages are
/// three bytes at `AudioInquiredType::UPSCALING`, and byte 2 is an
/// `UpscalingType` in one and an `UpscalingTypeAutoOff` in the other. The
/// COMMAND is the whole discriminator, and getting it wrong is silent: read
/// the param byte as an `UpscalingType` and 0x01 becomes "DSEE" -- a real
/// upscaler name, on a device whose upscaler is DSEE HX AI -- while reading
/// the capability byte as an `UpscalingTypeAutoOff` turns 0x02 into
/// out-of-range and loses the type altogether.
///
/// `AUDIO_RET_STATUS` at UPSCALING (`e3 01 00`, fixture 000097) is the third
/// DSEE message, and [`decode_upscaling_status`] reads it.
fn decode_dsee(message: &Message) -> Option<Reading> {
    // AudioInquiredType::UPSCALING. The other upscaling type, 0x0B, carries a
    // status and a reason list and has no capture.
    if message.kind? != 0x01 || message.len != 3 {
        return None;
    }
    let byte = *message.body.first()?;
    let mut out = DseeReport::default();
    match message.command.canonical() {
        Command::AudioRetCapability => out.upscaling = Some(Upscaling::from_byte(byte)),
        Command::AudioRetParam => out.enabled = Some(Switch::from_auto_off(byte)),
        _ => return None,
    }
    Some(Reading::Dsee(out))
}

/// `AudioRetStatusCommon` (T1:2930) at UPSCALING: whether DSEE can run now.
///
/// CAPTURED: fixture 000097, `e3 01 00` -- ENABLE, so available.
///
/// THIS WAS LEFT UNREAD FOR A WHILE, ON AN ARGUMENT THAT DID NOT HOLD. The note
/// on [`decode_dsee`] used to say no struct declares this key, because
/// `AudioRetStatusCommon`'s type defaults to CONNECTION_MODE. But that field
/// is annotated `CODEGEN Ignore OUT_OF_RANGE is expected` and not `EnumRange`,
/// which in this header means the default is not a pin -- the same reading
/// that lets [`decode_volume`] answer at MUSIC_VOLUME. And the reference client
/// settles it: libmdr deserializes exactly `AudioRetStatusCommon` (and
/// `AudioNotifyStatusCommon` for 0xE5) at UPSCALING and stores
/// `status == ENABLE` as `mUpscalingAvailable` (`HeadphonesV2T1.cpp:564-579`).
/// So the grounds are the captured byte and a reference client that reads it
/// the same way, which is [`Grounds::ReferenceClient`] and no stronger: what
/// DISABLE looks like has not been seen.
fn decode_upscaling_status(message: &Message) -> Option<Reading> {
    if message.kind? != 0x01 || message.len != 3 {
        return None;
    }
    let available = Switch::from_enable_disable(*message.body.first()?);
    Some(Reading::UpscalingStatus { available })
}

/// `AudioRetParamConnection` (T1:2801) and
/// `AudioRetParamConnectionWithLdacStatus` (T1:2814): the link priority.
///
/// TWO TYPES, ONE LAYOUT, DIFFERENT GROUNDS:
///
///   0x02  CONNECTION_MODE_WITH_LDAC_STATUS. The RET, the SET (T1:3084) and
///         the NTFY (T1:2637) all PIN the type with `CODEGEN EnumRange` and
///         are field-identical: command, type, `PriorMode`. HEADER ONLY.
///   0x00  CONNECTION_MODE. The RET's type is `Ignore`, not pinned, and the
///         NTFY twin (`AudioNtfyParamConnection`, T1:2609) declares a
///         different default type altogether. What makes 0x00 readable is the
///         reference client: libmdr asks `e6 00` and reads the reply as this
///         struct when a headset advertises t1 0xE1
///         (`HeadphonesV2.cpp:179-181`, `HeadphonesV2T1.cpp:598-613`).
///
/// AND THE VALUE IS PINNED, which those grounds cover only half of. Every
/// connection struct carries `CODEGEN EnumRange PriorMode::CONNECTION_QUALITY_
/// PRIOR` on `settingValue` (T1:2807, 2820 for the RETs), and libmdr's
/// generated validation rejects any other value as it deserializes
/// (`ProtocolV2T1Validation.cpp:470,479`) -- so libmdr reads `stable` here and
/// drops a reply saying anything else. This reads SOUND_QUALITY_PRIOR as
/// `quality` anyway, because `PriorMode` (T1:1399) names it and a headset
/// sending it is a fact worth showing; [`grounds`] reports such a reading as
/// [`Grounds::AgainstHeader`], not as the key's tier. `encode_connection`
/// says why the write side does the same.
///
/// No capture holds either: the WF-1000XM5 advertises neither 0xE1 nor 0xE3.
/// Three bytes at either type and nothing else. 0x05, the LE Audio form, has
/// no RET in the header at all, and its two NTFYs -- three bytes
/// (`AudioNtfyParamConnection`, T1:2609) and four ending in a
/// `SwitchingStream` (T1:2622) -- are not read; the `value2` the encoder
/// refuses over is the SET's (T1:3069).
fn decode_connection(message: &Message) -> Option<Reading> {
    let kind = message.kind?;
    if !matches!(kind, 0x00 | 0x02) || message.len != 3 {
        return None;
    }
    Some(Reading::Connection { kind, prior: PriorMode::from_byte(*message.body.first()?) })
}

/// `RetPlayStatusPlaybackController` and its colliding twin, offsets 2 and 3.
///
/// CAPTURED: fixture 000065, `a3 01 00 02 00` -- ENABLE, PAUSE. See
/// [`PlayStatusReport`] for why offset 4 is not read and [`AMBIGUOUS`] for why
/// this key is no longer refused whole. libmdr reads the same message as
/// `RetPlayStatusPlaybackController` and keeps `playbackStatus`
/// (`HeadphonesV2T1.cpp:428-447`) -- the field this reads, and not the one in
/// dispute.
fn decode_play_status(message: &Message) -> Option<Reading> {
    // PLAYBACK_CONTROL_WITH_CALL_VOLUME_ADJUSTMENT, five bytes. The
    // `...WithCallVolumeAdjustmentAndFunctionChange` RET at the same type is
    // six, and is not read: no capture holds one.
    if message.kind? != 0x01 || message.len != 5 {
        return None;
    }
    let control = Switch::from_enable_disable(*message.body.first()?);
    let status = PlaybackStatus::from_byte(*message.body.get(1)?);
    Some(Reading::PlayStatus(PlayStatusReport { control, status }))
}

/// `AlertNotifyParamFixedMessage` (T1:2080): the headset asking something.
///
/// `99 00 <AlertMessageType> <AlertActionType>`. HEADER ONLY for the layout --
/// the type is pinned, both value fields are `Ignore` -- and REFERENCE CLIENT
/// for the reading: libmdr deserializes exactly this when a headset
/// advertises FIXED_MESSAGE (t1 0x90) and treats POSITIVE_NEGATIVE as a
/// question to put to the person (`HeadphonesV2T1.cpp:918-940`). No capture
/// holds one, because the headset asked nothing while the corpus was
/// recorded -- NOT because alerts were off. The recorder very likely enabled
/// them: libmdr sends `94 00 00` to a headset advertising FIXED_MESSAGE
/// (`HeadphonesV2.cpp:88-90`), this one advertises it (fixture 000022), and
/// fixture 000049 is an ACK with no reply after it at the point in libmdr's
/// order where that request falls. Read evidence; the host's half was not
/// recorded.
///
/// Both bytes are carried raw. [`alert_message_name`] and
/// [`alert_action_word`] name them where the document is written, because a
/// message type this port has no name for is still a question the headset is
/// waiting on.
fn decode_alert(message: &Message) -> Option<Reading> {
    if message.kind? != 0x00 || message.len != 4 {
        return None;
    }
    Some(Reading::Alert { message: *message.body.first()?, action: *message.body.get(1)? })
}

/// `AlertMessageType` (T1:98-207), transcribed whole. What the headset is
/// asking about.
const ALERT_MESSAGES: &[(u8, &str)] = &[
    (0x00, "DISCONNECT_CAUSED_BY_CONNECTION_MODE_CHANGE"),
    (0x01, "DISCONNECT_CAUSED_BY_CHANGING_KEY_ASSIGN"),
    (0x02, "NEED_DISCONNECTION_FOR_UPDATING_FIRMWARE"),
    (0x03, "GOOGLE_ASSISTANT_IS_NOW_AVAILABLE"),
    (0x05, "DUAL_ASSIGN_OF_VOICE_ASSISTANT_IS_UNAVAILABLE"),
    (0x06, "DISCONNECT_CAUSED_BY_CHANGING_MULTIPOINT_LDAC_DISABLE"),
    (0x07, "DISCONNECT_CAUSED_BY_CHANGING_MULTIPOINT"),
    (0x08, "BATTERY_CONSUMPTION_INCREASE_DUE_TO_EQ_AND_UPSCALING"),
    (0x09, "CAUTION_FOR_DUAL_ASSIGNMENT_OF_PLAYBACK_CONTROL"),
    (0x0A, "CAUTION_FOR_DISABLE_TOUCH_SENSOR_PANEL"),
    (0x0B, "CAUTION_FOR_DISABLE_TOUCH_SENSOR_PANEL_AND_RECONNECTION"),
    (0x0C, "CAUTION_FOR_ABLE_TOUCH_SENSOR_PANEL"),
    (0x0D, "CAUTION_FOR_ABLE_TOUCH_SENSOR_PANEL_AND_RECONNECTION"),
    (0x0E, "CAUTION_FOR_DISABLE_VOICE_ASSISTANT_ASSIGNABLE_BUTTON"),
    (0x0F, "CAUTION_FOR_DISABLE_VOICE_ASSISTANT_ASSIGNABLE_SENSOR"),
    (0x10, "CAUTION_FOR_DISABLE_VOICE_ASSISTANT_AND_RECONNECTION_ASSIGNABLE_BUTTON"),
    (0x11, "CAUTION_FOR_DISABLE_VOICE_ASSISTANT_AND_RECONNECTION_ASSIGNABLE_SENSOR"),
    (0x12, "CAUTION_FOR_ABLE_VOICE_ASSISTANT_ASSIGNABLE_BUTTON"),
    (0x13, "CAUTION_FOR_ABLE_VOICE_ASSISTANT_ASSIGNABLE_SENSOR"),
    (0x14, "CAUTION_FOR_ABLE_VOICE_ASSISTANT_AND_RECONNECTION_ASSIGNABLE_BUTTON"),
    (0x15, "CAUTION_FOR_ABLE_VOICE_ASSISTANT_AND_RECONNECTION_ASSIGNABLE_SENSOR"),
    (0x16, "CAUTION_FOR_CHANGE_VOICE_ASSISTANT"),
    (0x17, "CAUTION_FOR_CHANGE_VOICE_ASSISTANT_TOUCH_SENSOR_PANEL"),
    (0x18, "CAUTION_FOR_CHANGE_VOICE_ASSISTANT_TOUCH_SENSOR_PANEL_MOBILE_OR_SIRI"),
    (0x19, "FOREGROUND_CAUTION_NEED_DISCONNECTION_FOR_ENABLING_WAKE_WORD"),
    (0x1A, "FOREGROUND_CAUTION_WAKE_WORD_IS_AVAILABLE_ONLY_IN_USE_OF_ALEXA"),
    (0x1B, "CAUTION_FOR_NOT_CONNECTED_COMPASS_MOUNTING_SIDE_RIGHT"),
    (0x1C, "CAUTION_FOR_CONNECTION_MODE_SOUND_QUALITY_PRIOR"),
    (0x1D, "CAUTION_FOR_FW_UPDATE_IN_PROGRESS"),
    (0x1E, "CAUTION_FOR_DISABLE_SAR_GM1"),
    (0x1F, "CAUTION_FOR_GATT_DISCONNECTION_FOR_SAR"),
    (0x20, "CAUTION_FOR_GATT_TO_ON"),
    (0x21, "CANT_ASSIGN_MS_AND_GOOGLE_ASSISTANT_AT_THE_SAME_TIME"),
    (0x22, "CAUTION_FOR_GATT_FOR_MS"),
    (0x23, "CAUTION_FOR_GATT_FOR_QUICK_ACCESS_SERVICE"),
    (0x24, "DISCONNECT_CAUSED_BY_GATT_ON"),
    (0x25, "DISCONNECT_AND_CHANGE_KEY_ASSIGN_CAUSED_BY_GATT_ON"),
    (0x26, "DISCONNECT_CAUSED_BY_CHANGING_KEY_ASSIGN_AND_CHANGE_GATT_TO_OFF"),
    (0x27, "DISCONNECT_CAUSED_BY_CHANGING_VOICE_ASSISTANT_AND_CHANGE_GATT_TO_OFF"),
    (0x28, "DISCONNECT_AND_CHANGE_VOICE_ASSISTANT_TO_SIRI_CAUSED_BY_GATT_ON"),
    (0x29, "CANT_USE_LDAC_WHILE_GATT_IS_ON"),
    (0x2A, "CANT_USE_LDAC_IN_SOUND_QUALITY_PRIOR"),
    (0x2B, "BATTERY_CONSUMPTION_INCREASE_DUE_TO_SOUND_FUNCTION_1"),
    (0x2C, "DISCONNECT_CAUSED_BY_CHANGING_CONNECTION_STANDBY_MODE_CLASSIC_ONLY"),
    (0x2D, "DISCONNECT_CAUSED_BY_CHANGING_CONNECTION_STANDBY_MODE_LE_AUDIO_CLASSIC"),
    (0x2E, "DISCONNECT_CAUSED_BY_ENTER_PAIRING_MODE_CLASSIC"),
    (0x2F, "DISCONNECT_CAUSED_BY_ENTER_PAIRING_MODE_WITH_FOLLOWING_FOR_LE_AUDIO_LIMITATION_VAS_CONNECTION_MODE"),
    (0x30, "DISCONNECT_CAUSED_BY_CHANGE_CONNECTION_CLASSIC"),
    (0x31, "DISCONNECT_CAUSED_BY_CHANGE_CONNECTION_WITH_FOLLOWING_FOR_LE_AUDIO_LIMITATION_VAS_CONNECTION_MODE"),
    (0x32, "BISTO_LCH_ASSIGNMENT_AND_GATT_ACTIVATION_IS_IMPOSSIBLE"),
    (0x33, "TEAMS_MUST_BE_ASSIGNED_TO_EITHER_LEFT_OR_RIGHT"),
    (0x34, "DISCONNECT_CAUSED_BY_ENTER_PAIRING_MODE_WITH_FOLLOWING_FOR_LE_AUDIO_LIMITATION_ASSIGNABLE_SETTINGS_VA_QA_CONNECTION_MODE_QA"),
    (0x35, "DISCONNECT_CAUSED_BY_CHANGE_CONNECTION_WITH_FOLLOWING_FOR_LE_AUDIO_LIMITATION_ASSIGNABLE_SETTINGS_VA_QA_CONNECTION_MODE_QA"),
    (0x36, "DISCONNECT_CAUSED_BY_ENTER_PAIRING_MODE_WITH_FOLLOWING_FOR_LE_AUDIO_LIMITATION_VAS_CONNECTION_MODE_PDM"),
    (0x37, "DISCONNECT_CAUSED_BY_CHANGE_CONNECTION_WITH_FOLLOWING_FOR_LE_AUDIO_LIMITATION_VAS_CONNECTION_MODE_PDM"),
    (0x38, "DISCONNECT_CAUSED_BY_ENTER_PAIRING_MODE_WITH_FOLLOWING_FOR_LE_AUDIO_LIMITATION_VAS_CONNECTION_MODE_QA"),
    (0x39, "DISCONNECT_CAUSED_BY_CHANGE_CONNECTION_WITH_FOLLOWING_FOR_LE_AUDIO_LIMITATION_VAS_CONNECTION_MODE_QA"),
    (0x40, "DISCONNECT_CAUSED_BY_ENTER_PAIRING_MODE_WITH_FOLLOWING_FOR_LE_AUDIO_LIMITATION_VAS_CONNECTION_MODE_QA_PDM"),
    (0x41, "DISCONNECT_CAUSED_BY_CHANGE_CONNECTION_WITH_FOLLOWING_FOR_LE_AUDIO_LIMITATION_VAS_CONNECTION_MODE_QA_PDM"),
    (0x50, "CANT_USE_LDAC_WHILE_GATT_IS_ON_NO_RIGHT_SIDE_LIMITATION"),
    (0x51, "CAUTION_FOR_CHANGING_VOICE_ASSISTANT_TO_SIRI"),
    (0x52, "CAUTION_FOR_GATT_TO_OFF_CAUSED_BY_CHANGING_VOICE_ASSISTANT"),
    (0x53, "CAUTION_FOR_LDAC_990"),
    (0x54, "CAUTION_FOR_GATT_TO_ON_FOR_QA_EASY_SETTING"),
    (0x55, "CAUTION_FOR_FW_UPDATE_IN_PROGRESS_FOR_QA_EASY_SETTING"),
    (0x56, "DISCONNECT_CAUSED_BY_GATT_ON_FOR_EASY_SETTING"),
    (0x57, "DISCONNECT_AND_CHANGE_KEY_ASSIGN_CAUSED_BY_GATT_ON_FOR_QA_EASY_SETTING"),
    (0x58, "DISCONNECT_AND_CHANGE_VOICE_ASSISTANT_TO_SIRI_CAUSED_BY_GATT_ON_FOR_QA_EASY_SETTING"),
    (0x59, "CAUTION_FOR_AUTO_VOLUME_OPTIMIZER_TO_ON"),
    (0x5A, "CAUTION_FOR_AUTO_VOLUME_TO_ON_SVC_IS_ON"),
    (0x5B, "CAUTION_FOR_AUTO_VOLUME_TO_ON_SVC_VOLUME_LIMITATION_IS_ON"),
    (0x5C, "CAUTION_FOR_EXCLUSIVE_EQ_CHANGING_AND_BGM_MODE"),
    (0x5D, "CAUTION_FOR_CHANGE_KEY_ASSIGN_TO_OTHER_VOICE_ASSISTANT"),
    (0x5E, "CAUTION_FOR_CHANGE_KEY_ASSIGN_TO_PLAYBACK_CONTROL_L"),
    (0x5F, "CAUTION_FOR_CHANGE_KEY_ASSIGN_TO_PLAYBACK_CONTROL_R"),
    (0x60, "CAUTION_FOR_CHANGE_KEY_ASSIGN_TO_NC_ASM_QUICK_ACCESS_L"),
    (0x61, "CAUTION_FOR_CHANGE_KEY_ASSIGN_TO_NC_ASM_QUICK_ACCESS_R"),
    (0x62, "CAUTION_FOR_CHANGE_KEY_ASSIGN_TO_VOICE_ASSISTANT_TO_NOT_SVA"),
    (0x63, "CAUTION_FOR_CAN_USE_GOOGLE_HOT_WORD"),
    (0x64, "CAUTION_FOR_CALLING_WHEN_SVA_TRAINING"),
    (0x65, "CAUTION_FOR_CHANGE_KEY_ASSIGN_TO_BE_ENABLE_REPEAT_TAP"),
    (0x66, "CAUTION_FOR_CHANGE_KEY_ASSIGN_TO_BE_DISABLE_REPEAT_TAP"),
    (0x67, "CAUTION_FOR_CHANGE_HEART_RATE_PROFILE_TO_ON"),
    (0x68, "CAUTION_FOR_CHANGE_HEART_RATE_SENSOR_GREEN_LIGHT_TO_OFF"),
    (0x70, "CAUTION_FOR_ENABLING_2_DEVICES_CONNECTION_WITH_LDAC"),
    (0x71, "CAUTION_FOR_CHANGING_TO_QUALITY_PRIOR_CONNECTION_MODE_WITH_2_DEVICES_CONNECTION"),
    (0x72, "CAUTION_FOR_CONNECTED_2_DEVICES_IN_BACKGROUND_WITH_LDAC"),
    (0x73, "WARNING_FOR_CHANGING_TO_LDAC_990_WITH_2_DEVICES_CONNECTION"),
    (0x74, "DISCONNECT_CAUSED_BY_CHANGE_CLASSIC_AUDIO_SOUND_QUALITY_PRIOR_FROM_LE_AUDIO"),
    (0x75, "DISCONNECT_CAUSED_BY_CHANGE_CLASSIC_AUDIO_CONNECTION_QUALITY_PRIOR_FROM_LE_AUDIO"),
    (0x76, "DISCONNECT_CAUSED_BY_CHANGE_CLASSIC_AUDIO_SOUND_QUALITY_PRIOR"),
    (0x77, "DISCONNECT_CAUSED_BY_CHANGE_CLASSIC_AUDIO_CONNECTION_QUALITY_PRIOR"),
    (0x80, "CAUTION_FOR_TV_SOUND_BOOSTER_SETTING_ON"),
    (0x81, "CAUTION_FOR_TV_SOUND_BOOSTER_SETTING_OFF"),
    (0x82, "CAUTION_FOR_SETTINGS_STAMINA_FROM_ON_TO_OFF"),
    (0x83, "CAUTION_FOR_SETTINGS_STAMINA_FROM_ON_TO_OFF_NO_LIGHTING_MODE"),
    (0x84, "CAUTION_FOR_SETTINGS_STAMINA_FROM_OFF_TO_ON"),
    (0x85, "CAUTION_FOR_SETTINGS_STAMINA_FROM_OFF_TO_ON_NO_LIGHTING_MODE"),
    (0x86, "CAUTION_FOR_SETTINGS_SOUND_EFFECT_ON_STAMINA_ON"),
    (0x87, "CAUTION_FOR_SETTINGS_SOUND_EFFECT_ON_STAMINA_ON_NO_LIGHTING"),
    (0x88, "CAUTION_FOR_SETTINGS_LIGHTING_MODE_ON_WHEN_STAMINA_ON"),
    (0x90, "CAUTION_FOR_DIABLING_BGM_MODE"),
    (0x91, "CAUTION_FOR_USB_SUBMERSION_MONITOR_DURING_NOT_CHARGING"),
    (0x92, "CAUTION_FOR_USB_SUBMERSION_MONITOR_DURING_CASE_IN"),
    (0x93, "TURN_KEY_EQ_SUCCESS"),
    (0x94, "CAUTION_FOR_USB_OVERHEAT_DETECTION_DURING_NOT_CHARGING"),
    (0x95, "CAUTION_FOR_USB_OVERHEAT_DETECTION_DURING_CASE_IN"),
];

/// The header's name for an `AlertMessageType`, or `None` for a byte it does
/// not declare.
pub fn alert_message_name(byte: u8) -> Option<&'static str> {
    ALERT_MESSAGES.iter().find(|(b, _)| *b == byte).map(|(_, name)| *name)
}

/// Whether answering yes to this alert makes the headset drop a link.
///
/// BY NAME, because the names are the only statement of it the reference
/// makes: libmdr's `HandleAlertParamT1` (`HeadphonesV2T1.cpp:918-940`) stores
/// the message and acts on none. So the rule is what the name SAYS, wherever
/// in the name it says it -- not a prefix, which is what this was, and which
/// missed `CAUTION_FOR_DISABLE_TOUCH_SENSOR_PANEL_AND_RECONNECTION` (0x0B), the
/// one a WH-1000XM5's own touch-panel setting would raise:
///
/// - `DISCONNECT` anywhere: every `DISCONNECT_*` message,
///   `NEED_DISCONNECTION_FOR_UPDATING_FIRMWARE` (0x02), and
///   `FOREGROUND_CAUTION_NEED_DISCONNECTION_FOR_ENABLING_WAKE_WORD` (0x19) --
///   which the header pins to the APP_BECOMES_FOREGROUND form this port does
///   not read (T1:2065-2076), and which says the same thing if it ever
///   arrives as a fixed message, whose message field takes any value.
/// - `RECONNECTION` anywhere: the touch-panel and voice-assistant cautions
///   that end `..._AND_RECONNECTION` (0x0B, 0x0D, 0x10, 0x11, 0x14, 0x15).
/// - EXCEPT `GATT_DISCONNECTION` (0x1F, `CAUTION_FOR_GATT_DISCONNECTION_FOR_SAR`):
///   GATT is the LE link, not the RFCOMM one this daemon holds.
///
/// `an_alert_is_read_raw_and_named_from_the_header` pins the whole set over
/// all 256 bytes, so a change to the rule is a change a test sees.
pub fn alert_disconnects(byte: u8) -> bool {
    alert_message_name(byte).is_some_and(|n| {
        (n.contains("DISCONNECT") || n.contains("RECONNECTION")) && !n.contains("GATT_DISCONNECTION")
    })
}

/// `AlertActionType` (T1:47-53) as a lowercase word: what kind of answer the
/// headset wants.
pub fn alert_action_word(byte: u8) -> &'static str {
    match byte {
        0x00 => "confirmation_only",
        0x01 => "positive_negative",
        0x02 => "positive_confirmation_with_reply",
        _ => "unknown",
    }
}

/// `SystemRetParamSmartTalking` and `SystemRetExtParamSmartTalkingModeType2`.
///
/// CAPTURED, both halves:
///
///   f7 0c 01 01   SMART_TALKING_MODE_TYPE2, onOff and previewMode
///   fb 0c 00 00   SMART_TALKING_MODE_TYPE2, sensitivity and timeout
///
/// AND THE FIRST OF THOSE MEANS OFF. `OnOffSettingValue` is `ON = 0x00`,
/// `OFF = 0x01`; the captured device has Speak-to-Chat switched off, and a
/// decoder that read 1 as on would put an enabled toggle on the panel for a
/// disabled feature. See [`Switch::from_on_off_setting`], which is where that
/// polarity is named. The extended reply decodes to `DetectSensitivity::AUTO`
/// and `ModeOutTime::FAST` -- Sony's defaults, which is what an untouched
/// setting should read as.
///
/// TWO THINGS THE HEADER GETS WRONG HERE, both harmless once known.
/// `SystemRetParamSmartTalking` declares its inquired type as
/// `SystemInquiredType::VIBRATOR` (0x00), which it plainly is not; the
/// struct's name is right and its type default is junk, exactly as
/// PROVENANCE.md records for the NcAsm family. And the same struct serves
/// SMART_TALKING_MODE_TYPE1 (0x02) as well as TYPE2 (0x0C). Only 0x0C is
/// decoded, because only 0x0C has a capture and because the two types differ
/// on the EXTENDED side: the TYPE1 extended reply is five bytes with a
/// `voiceFocus` in the middle, so a decoder that accepted both would read its
/// timeout out of a voice-focus byte.
fn decode_speak_to_chat(message: &Message) -> Option<Reading> {
    // SMART_TALKING_MODE_TYPE2.
    if message.kind? != 0x0C || message.len != 4 {
        return None;
    }
    let first = *message.body.first()?;
    let second = *message.body.get(1)?;
    let mut out = SpeakToChatReport::default();
    match message.command.canonical() {
        Command::SystemRetParam => {
            out.enabled = Some(Switch::from_on_off_setting(first));
            out.preview = Some(Switch::from_on_off_setting(second));
        }
        Command::SystemRetExtParam => {
            out.sensitivity = Some(Sensitivity::from_byte(first));
            out.timeout = Some(Timeout::from_byte(second));
        }
        _ => return None,
    }
    Some(Reading::SpeakToChat(out))
}

/// `ConnectRetSupportFunction`: every feature the headset claims.
///
/// CAPTURED, both tables, from the same WF-1000XM5 handshake 54ms apart:
///
///   07 00 2e | 10 ff | 12 ff | ... | ff 2d | ...    46 functions, table 1
///   07 00 0b | 42 3d | 53 ff | ... | 31 ff          11 functions, table 2
///
/// The layout is `MDRPodArray<SupportFunction>`: one count byte, then that
/// many two-byte elements. Nothing about that is guessed -- `ProtocolV2.hpp`
/// declares `SupportFunction` as exactly `{ FunctionType functionType; UInt8
/// priority; }`, and both captures are `3 + 2 * count` bytes to the byte.
///
/// HOW IT IS KNOWN THE PAIRING IS THE RIGHT WAY ROUND. Every one of the 46
/// first bytes is a declared `t1::FunctionType` and every one of the 11 is a
/// declared `t2::FunctionType`. Read the pairs the other way round -- priority
/// first -- and table 1 yields nine bytes no `t1::FunctionType` declares
/// (0x01, 0x08, 0x0D, 0x1D, 0x1F, 0x33, 0x3A, 0x3B, 0x3F) while table 2 yields
/// 0x3D and 0x04, declared by neither enum.
///
/// A stricter check than the header, because it comes from the device rather
/// than from a transcription of Sony's own code: the corpus holds 26 further
/// payload frames, captured after this handshake, across eleven command
/// families -- and every one of them belongs to a feature this list
/// advertises. `POWER_RET_STATUS` arrives at `LR_BATTERY_WITH_THRESHOLD` and
/// `CRADLE_BATTERY_WITH_THRESHOLD` and never at `BATTERY`, matching a table
/// carrying 0x29 and 0x2A and not 0x20; `GENERAL_SETTING_RET_CAPABILITY` and
/// `GENERAL_SETTING_RET_PARAM` each arrive exactly twice, for a table
/// advertising `GENERAL_SETTING_1` and `GENERAL_SETTING_2` and no others.
/// `tests/mdr_support_functions.rs` asserts that agreement family by family.
///
/// `COMMON_RET_STATUS` at AUDIO_CODEC answers to t1 `CODEC_INDICATOR` (0x12,
/// `ProtocolV2.hpp`), which this table advertises second: `12 ff`.
///
/// The check only runs one way: a device is free to advertise a feature and
/// never mention it again in a partial capture, so an advertised function with
/// no frame proves nothing. A FRAME with no advertised function would be the
/// falsifier, and there is not one.
fn decode_support_functions(table: Table, message: &Message) -> Option<Reading> {
    // ConnectInquiredType::FIXED_VALUE. The command has no other inquired type
    // -- `ConnectRetSupportFunction` declares this one and the enum's other
    // members belong to the device-info and capability replies.
    if message.kind? != 0x00 {
        return None;
    }
    let count = usize::from(*message.body.first()?);
    let mut functions = Vec::with_capacity(count);
    let mut at = 1usize;
    for _ in 0..count {
        let code = *message.body.get(at)?;
        let priority = *message.body.get(at.checked_add(1)?)?;
        functions.push(SupportFunction { code, priority });
        at = at.checked_add(2)?;
    }
    // The count must claim the payload exactly. A trailing byte means this is
    // not the message it says it is, and a list built from the first n pairs
    // of something else is the failure mode this whole module exists to avoid.
    if at != message.body.len() {
        return None;
    }
    Some(Reading::SupportFunctions { table, functions })
}

/// `EqEbbRetParamEq`: the preset and its band steps.
///
/// CAPTURED: `57 00 a0 06 14 0b 0a 0d 09 0c` -- PRESET_EQ, preset 0xA0
/// (CUSTOM), then an `MDRPodArray<UInt8>` of six steps: 20, 11, 10, 13, 9, 12.
///
/// THE STEPS STAY RAW HERE AND ARE SPLIT WHERE THE DOCUMENT IS WRITTEN.
///
/// `ProtocolV2T1.hpp` declares the field as a bare `MDRPodArray<UInt8>` with
/// no names for the positions, and this one capture cannot say which end is
/// clear bass. Two hardware-tested implementations can, and agree:
///
///   * libmdr `HeadphonesV2T1.cpp`, the EqEbbRetParamEq arm: for six steps
///     `mEqClearBass = bandSteps[0] - 10` and the bands are `[1..5] - 10`,
///     labelled 400, 1k, 2.5k, 6.3k, 16k; for ten steps there is no clear
///     bass and the bands are `- 6`.
///   * Gadgetbridge `SonyProtocolImplV1.handleEqualizer`, which the XM5's V2
///     class inherits: `clearBass = payload[4] - 10` and each band is
///     `payload[5+i] - 10`. And its `SonyProtocolImplV2Test` holds NOTIFY
///     frames a V2 device sent: BASS_BOOST is `59 00 16 06 11 0a 0a 0a 0a 0a`
///     (only the first step moves) while TREBLE_BOOST is
///     `59 00 15 06 0a 0a 0a 0c 10 14` (rising to the last). Read from the
///     tail, treble boost would be a +10 clear bass.
///
/// So step 0 is clear bass and 0x0A is flat. The split lives in
/// `sony::session::Session::project` rather than here because `EqReport.steps`
/// is what `encode_eq_preset` echoes back to the headset, and an echo must be
/// the bytes the device sent.
fn decode_equalizer(message: &Message) -> Option<Reading> {
    // PRESET_EQ. The other EqEbbInquiredType values -- EBB, SOUND_EFFECT,
    // CUSTOM_EQ and the rest -- carry different structures and have no capture.
    if message.kind? != 0x00 {
        return None;
    }
    let preset = *message.body.first()?;
    let count = usize::from(*message.body.get(1)?);
    let start = 2usize;
    let end = start.checked_add(count)?;
    let steps = message.body.get(start..end)?.to_vec();
    // A trailing byte the count did not claim means this is a different
    // structure wearing the same key -- EqParamUltMode puts a status byte
    // before the array, so its count would land short.
    if end != message.body.len() {
        return None;
    }
    Some(Reading::Equalizer(EqReport { preset, steps }))
}

/// `GENERAL_SETTING_RET_CAPABILITY`: what the setting in this slot is called.
///
/// CAPTURED, both of the corpus's two:
///
///   d1 d1 00 01 10 "SIDETONE_SETTING"   18 "SIDETONE_SETTING_SUMMARY"
///   d1 d2 00 01 12 "MULTIPOINT_SETTING" 29 "MULTIPOINT_SETTING_SUMMARY_LDAC_AVAILABLE"
///    |  |  |  |  \_ a length-prefixed string, then another
///    |  |  |  \_ GsStringFormat: ENUM_NAME (0x01) on both
///    |  |  \_ GsSettingType: BOOLEAN_TYPE (0x00) on both
///    |  \_ the slot, which is also the inquired type
///    \_ the command
///
/// THE TWO BYTES ARE NAMED, AND ONE OF THEM DECIDES WHAT FOLLOWS. These were
/// once left undecoded on the grounds that both captures carry the same two
/// bytes, so the corpus cannot tell a field from a constant. The reference
/// can: `GsRetCapability` (T1:4220) declares offset 2 a `GsSettingType`
/// (T1:949: BOOLEAN_TYPE 0x00, LIST_TYPE 0x01) and offset 3 a
/// `GsStringFormat` (T1:963: RAW_NAME 0x00, ENUM_NAME 0x01). The corpus fixed
/// the VALUES and the header supplies the NAMES; the decode needs both.
///
/// ONLY A BOOLEAN IS READ. A LIST-TYPE setting has a different param body
/// (`GsRetParamList`) that nothing here decodes, and reading its capability
/// as if it were a switch would put a switch on the page that writes the
/// wrong frame. It arrives as `Unhandled` with its key instead. A string
/// format other than the two declared is refused the same way.
fn decode_general_setting_info(message: &Message) -> Option<Reading> {
    let slot = message.kind?;
    if *message.body.first()? != GS_BOOLEAN_TYPE || *message.body.get(1)? > 0x01 {
        return None;
    }
    let name = prefixed_string(message.body, 2)?;
    let summary = prefixed_string(message.body, 2 + 1 + name.len())?;
    // Nothing may be left over: a trailing byte means this is a different
    // structure wearing the same key, exactly as `decode_equalizer` reasons.
    if 2 + 1 + name.len() + 1 + summary.len() != message.body.len() {
        return None;
    }
    Some(Reading::GeneralSettingInfo(GeneralSetting { slot, name, summary }))
}

/// `GENERAL_SETTING_RET_PARAM`: whether the setting in this slot is on.
///
/// CAPTURED: `d7 d1 00 01` and `d7 d2 00 00` -- so on the captured
/// WF-1000XM5 sidetone is OFF and multipoint is ON.
///
/// THE VALUE IS A `GsSettingValue`, AND ITS ON IS 0x00. `GsRetParamBoolean`
/// (T1:4267) is `command, type, GsSettingType settingType, GsSettingValue
/// value`, and `GsSettingValue` (T1:956) declares ON = 0x00, OFF = 0x01 --
/// the inverted convention `OnOffSettingValue` uses, which `decode_speak_to_chat`
/// already has a paragraph about. This read it the other way for as long as
/// it existed, on the strength of "0 is off" and a byte the corpus showed in
/// both positions, and the encoder made the same mistake in mirror, so every
/// round-trip test passed over a write that did the opposite of what was
/// asked. Three things say this way round is right, independently:
///
/// - the reference CLIENT reads `res.value == GsSettingValue::ON` and writes
///   `submitted ? ON : OFF` (libmdr `HeadphonesV2T1.cpp:501`, V2:804-811);
/// - Gadgetbridge's V2 touch-sensor write sends 0x00 for enabled
///   (`SonyProtocolImplV2.setWideAreaTap`, "this is reversed on V2");
/// - and the corpus itself: fixture 000077 is the paired-device list with
///   TWO peers connected at once, which a headset with multipoint off cannot
///   do. Read the old way, the same capture said multipoint was off.
///
/// THE SETTING TYPE MUST BE BOOLEAN, and the value one of the two declared.
/// Anything else is `Unhandled` with its key, not rounded to the nearer of two
/// answers.
fn decode_general_setting_param(message: &Message) -> Option<Reading> {
    let slot = message.kind?;
    if message.body.len() != 2 || message.body[0] != GS_BOOLEAN_TYPE {
        return None;
    }
    let enabled = match message.body[1] {
        GS_ON => true,
        GS_OFF => false,
        _ => return None,
    };
    Some(Reading::GeneralSettingParam { slot, enabled })
}

/// `GsSettingType::BOOLEAN_TYPE` (T1:951).
pub const GS_BOOLEAN_TYPE: u8 = 0x00;
/// `GsSettingValue::ON` (T1:958). Yes, zero.
pub const GS_ON: u8 = 0x00;
/// `GsSettingValue::OFF` (T1:959).
pub const GS_OFF: u8 = 0x01;

/// `ConnectRetDeviceInfo*`: what the headset calls itself.
///
/// The one family in this module where the payload is NOT a fixed size, so the
/// dispatch key's length half cannot select a struct on its own.
/// `ConnectRetDeviceInfo` (two bytes, no string) and
/// `ConnectRetDeviceInfoModelName` (variable, at least three) both declare
/// MODEL_NAME; here the string reader separates them, because a two-byte
/// payload has no length byte for it to read.
///
/// CAPTURED, all three that this corpus contains:
///   05 01 0a 57 46 2d 31 30 30 30 58 4d 35   MODEL_NAME "WF-1000XM5"
///   05 02 05 36 2e 31 2e 30                  FW_VERSION "6.1.0"
///   05 03 30 03                              SERIES_AND_COLOR_INFO
///
/// The third is NOT decoded, and that is a decision rather than an oversight.
/// It parses cleanly -- `ModelSeries::PREMIUM` (0x30), which is right for a
/// 1000X, and a colour byte -- but `ModelColor` is declared in `ProtocolV2.hpp`
/// rather than the header this port was transcribed from, so the colour cannot
/// be named here. More to the point, nothing in the state document holds either
/// field, so decoding them would be untested code in a daemon that must not
/// panic. It goes in when something displays it.
fn decode_device_info(message: &Message) -> Option<Reading> {
    let kind = message.kind?;
    let mut out = DeviceInfo::default();
    match kind {
        // MODEL_NAME is declared by two structs: `ConnectRetDeviceInfoModelName`
        // and the bare two-byte `ConnectRetDeviceInfo`, which carries no string
        // at all. Nothing here tests the length for that, because
        // `prefixed_string` already refuses it: a two-byte payload has an empty
        // body, so there is no length byte to read.
        //
        // An explicit `message.len > 2` guard was here and was removed after a
        // mutation test showed that deleting it changed no behaviour and failed
        // no test. A line that reads as a safety check and is not one is worse
        // than no line: the next person to touch this trusts it.
        0x01 => out.model = prefixed_string(message.body, 0)?,
        0x02 => out.firmware = prefixed_string(message.body, 0)?,
        _ => return None,
    }
    Some(Reading::Device(out))
}

/// `PowerRetStatus*`, the battery variants with a capture behind them.
///
/// THE CORRECTION THIS FUNCTION EXISTS TO CARRY. `PowerInquiredType` lists
/// `BATTERY = 0x00`, `LEFT_RIGHT_BATTERY = 0x01`, `CRADLE_BATTERY = 0x02` and
/// then, further down, `BATTERY_WITH_THRESHOLD = 0x08`,
/// `LR_BATTERY_WITH_THRESHOLD = 0x09` and `CRADLE_BATTERY_WITH_THRESHOLD =
/// 0x0A`. The captured device sends 0x09 and 0x0A. A decoder written from the
/// first three names reads a threshold byte as a further battery level.
///
/// The four plain variants are decoded too -- the same fields without the
/// trailing threshold, and the length tells them apart -- but only the two
/// threshold forms have a capture. The plain ones are HEADER ONLY, and they
/// are the ones that matter for an over-ear: a WH-1000XM5 reports a main
/// battery and no case, and nothing in this corpus is a WH.
///
/// Each of `BATTERY`, `LEFT_RIGHT_BATTERY`, `BATTERY_WITH_THRESHOLD` is
/// claimed by two structs (a plain one and a `*Base` one) at the same length.
/// Those pairs are collisions in name only -- same field types at the same
/// offsets -- so either decodes identically, which is why they are not in
/// `AMBIGUOUS`.
fn decode_battery(message: &Message) -> Option<Reading> {
    let kind = message.kind?;
    let body = message.body;
    let cell = |at: usize, threshold: Option<usize>| -> Option<Cell> {
        Some(Cell {
            level: *body.get(at)?,
            charging: Charging::from_byte(*body.get(at + 1)?),
            threshold: match threshold {
                Some(t) => Some(*body.get(t)?),
                None => None,
            },
        })
    };

    let mut out = BatteryReport::default();
    match (kind, message.len) {
        // BATTERY: level, charging.
        (0x00, 4) => out.main = cell(0, None),
        // LEFT_RIGHT_BATTERY: left level, left charging, right level, right
        // charging.
        (0x01, 6) => {
            out.left = cell(0, None);
            out.right = cell(2, None);
        }
        // CRADLE_BATTERY: level, charging.
        (0x02, 4) => out.case = cell(0, None),
        // BATTERY_WITH_THRESHOLD: level, charging, threshold.
        (0x08, 5) => out.main = cell(0, Some(2)),
        // LR_BATTERY_WITH_THRESHOLD: left level, left charging, right level,
        // right charging, left threshold, right threshold. CAPTURED.
        (0x09, 8) => {
            out.left = Some(Cell {
                level: *body.first()?,
                charging: Charging::from_byte(*body.get(1)?),
                threshold: Some(*body.get(4)?),
            });
            out.right = Some(Cell {
                level: *body.get(2)?,
                charging: Charging::from_byte(*body.get(3)?),
                threshold: Some(*body.get(5)?),
            });
        }
        // CRADLE_BATTERY_WITH_THRESHOLD: level, charging, threshold. CAPTURED.
        (0x0A, 5) => out.case = cell(0, Some(2)),
        _ => return None,
    }
    Some(Reading::Battery(out))
}

/// `NcAsmRetParam*`: the noise-control reading, in each layout a WH-1000XM5
/// has been seen or credibly reported to answer with.
///
/// KEYED ON (TYPE, LENGTH), because the type alone does not choose a struct:
/// five declare `NC_ON_OFF` (0x01) with lengths 4, 6, 8, 7 and 5, and two
/// declare 0x13 with lengths 6 and 7. The length picks one.
///
/// WHY MORE THAN THE CAPTURED ONE. This decoded 0x17 alone for as long as the
/// corpus was the only evidence, on the principle that a layout read off the
/// header is a guess. Two things changed that. The reference CLIENT decodes
/// 0x19 and 0x22 (libmdr `HandleNcAsmParamT1`, `HeadphonesV2T1.cpp:95-180`),
/// and people run it against real headsets. And the WH-1000XM5 does not
/// answer at one type: Gadgetbridge drives it at 0x15
/// (`SonyProtocolImplV2.java`, `handleAmbientSoundControl`), a WH-1000XM5
/// was observed answering 0x17 (snizovtsev/xm5-mode-switch), and XMDeck added
/// 0x19 for it. A headset whose reading this refused is a headset whose noise
/// control the panel can neither show nor change, and that is the device this
/// port is for. Each row says what it stands on; `grounds()` says the same to
/// a program.
///
///   (0x17, 7)  MODE_NC_ASM_DUAL_NC_MODE_SWITCH_AND_ASM_SEAMLESS. CAPTURED:
///              fixture mdr-packet-…-000069, the corpus's only NCASM frame:
///              67 17 01 00 01 00 14 -- settled, total effect off, mode ASM,
///              normal (not voice), level 20. Two structs claim this key and
///              differ only in the NAME of the enum at offset 4 (`NcAsmMode`,
///              `NcNcssAsmMode`, both NC 0x00 / ASM 0x01), so they decode alike.
///   (0x15, 7)  MODE_NC_ASM_AUTO_NC_MODE_SWITCH_AND_ASM_SEAMLESS. THIRD PARTY:
///              no struct in the header declares 0x15; Gadgetbridge reads its
///              WH-1000XM5 here with the 0x17 offsets.
///   (0x19, 9)  ..._SEAMLESS_NA, noise adaptation. REFERENCE CLIENT. Its two
///              extra fields are at the END, offsets 7 and 8 -- a first draft
///              of this table moved the level to 7 and would have read it out
///              of an adaptive-sensitivity byte.
///   (0x22, 6)  ASM_SEAMLESS, ambient only. REFERENCE CLIENT: libmdr reads it
///              as `NcAsmRetParamAsmSeamless`, whose declared type is the
///              unpinned NC_ON_OFF, which is why an earlier note here called
///              the key a message that does not exist. libmdr asks for it at
///              0x22 and decodes what comes back.
///   (0x13, 7)  NC_ON_OFF_AND_ASM_SEAMLESS. HEADER ONLY, pinned (T1:5505):
///              total effect, then NC on or off, then the ambient mode and
///              level. The 0x13 NOTIFY collides at this length (`AMBIGUOUS`)
///              and is refused before it gets here.
///   (0x01, 5)  NC_ON_OFF, cancelling and nothing else. HEADER ONLY, pinned
///              (T1:5469): total effect, NC on or off. No ambient fields.
///
/// Only the first three are WRITTEN; see `mdr::encode::encode_ncasm`.
fn decode_noise(message: &Message) -> Option<Reading> {
    let kind = message.kind?;
    let body = message.body;

    // Offsets into the body (the payload after command and type).
    struct Layout {
        mode: Option<usize>,
        nc_switch: Option<usize>,
        ambient: Option<usize>,
        level: Option<usize>,
        adaptive: Option<usize>,
        sensitivity: Option<usize>,
    }
    let full = |adaptive, sensitivity| Layout {
        mode: Some(2),
        nc_switch: None,
        ambient: Some(3),
        level: Some(4),
        adaptive,
        sensitivity,
    };
    let layout = match (kind, message.len) {
        (0x17, 7) | (0x15, 7) => full(None, None),
        (0x19, 9) => full(Some(5), Some(6)),
        (0x22, 6) => Layout {
            mode: None,
            nc_switch: None,
            ambient: Some(2),
            level: Some(3),
            adaptive: None,
            sensitivity: None,
        },
        (0x13, 7) => Layout {
            mode: None,
            nc_switch: Some(2),
            ambient: Some(3),
            level: Some(4),
            adaptive: None,
            sensitivity: None,
        },
        (0x01, 5) => Layout {
            mode: None,
            nc_switch: Some(2),
            ambient: None,
            level: None,
            adaptive: None,
            sensitivity: None,
        },
        _ => return None,
    };

    let byte = |at: Option<usize>| -> Option<Option<u8>> {
        match at {
            Some(i) => Some(Some(*body.get(i)?)),
            None => Some(None),
        }
    };
    // ValueChangeStatus at 0 and NcAsmOnOffValue (the total effect) at 1, in
    // every layout.
    let settled = *body.first()? == 0x01;
    let on = *body.get(1)? == 0x01;
    let mode_byte = byte(layout.mode)?;
    let nc_switch = byte(layout.nc_switch)?;
    // AmbientSoundMode: NORMAL = 0x00, VOICE = 0x01.
    let focus_on_voice = byte(layout.ambient)?.map(|b| b == 0x01);
    let ambient_level = byte(layout.level)?;
    // NcAsmOnOffValue: OFF = 0x00, ON = 0x01 -- the uninverted convention.
    let adaptive = byte(layout.adaptive)?.map(|b| b == 0x01);
    let adaptive_sensitivity = byte(layout.sensitivity)?;

    // Which filter is running, from whichever byte this layout has for it.
    let cancelling = match (mode_byte, nc_switch) {
        // NcAsmMode: NC = 0x00, ASM = 0x01.
        (Some(mode), _) => mode == 0x00,
        // An NC switch: on is cancelling. With ambient fields beside it, off
        // is ambient sound; without them it is simply cancelling switched off.
        (None, Some(nc)) => nc == 0x01,
        // A variant that only does ambient sound.
        (None, None) => false,
    };
    let mode = if !on {
        // Total effect off beats everything: the headset reports a mode and a
        // level while the whole feature is switched off, and reporting that as
        // "ambient" would put a bar on the panel for a filter doing nothing.
        NoiseMode::Off
    } else if cancelling {
        NoiseMode::Anc
    } else if layout.ambient.is_some() {
        NoiseMode::Ambient
    } else {
        // NC_ON_OFF with the switch off: the effect is on and nothing is
        // running, which is off by any other name.
        NoiseMode::Off
    };

    Some(Reading::Noise(NoiseReport {
        mode,
        mode_byte,
        ambient_level,
        focus_on_voice,
        settled,
        adaptive,
        adaptive_sensitivity,
    }))
}

/// `NCASM_RET_CAPABILITY` (0x61): the ambient level's range, per mode.
///
/// `61 <type> <n> [mode min max step] × n` -- an `MDRPodArray` of
/// `AmbientSoundModeLevelSet` (T1:2317), whose wire form is a count byte and
/// then the packed elements (`Protocol.hpp:409-426`). The struct is pinned at
/// 0x16, 0x17 and 0x19 (T1:9882-9906), and at 0x13 (T1:9954); three more
/// declare the same body with an unpinned type (T1:9870, 9918, 9930).
///
/// HEADER ONLY. No capture holds one, and neither reference client asks for
/// one: libmdr and Gadgetbridge both hard-code a maximum of 20. This port asks
/// because the ambient level is the one control whose legal values the
/// command language had to guess, and a headset that says what they are
/// should be believed over the guess.
///
/// THE LENGTH MUST BE THE COUNT'S. At 0x13 a second struct
/// (`NcAsmRetCapabilityNcOnOffAsmOnOff`, T1:5243) carries one-byte modes and
/// no ranges; `3 + 4n` and `3 + n` differ for every n > 0, so the length says
/// which arrived. A reply that is not exactly `3 + 4n` is not read.
///
/// AND THE RANGES MUST BE RANGES. A mode other than NORMAL or VOICE, a mode
/// listed twice, `min > max` or a zero step is refused whole: a range this
/// would then enforce is one no level could satisfy, or one that admits
/// everything, and a headset that sent it is not one to take at its word.
fn decode_noise_capability(message: &Message) -> Option<Reading> {
    let kind = message.kind?;
    if !matches!(kind, 0x13..=0x19 | 0x22) {
        return None;
    }
    let body = message.body;
    let count = usize::from(*body.first()?);
    if count == 0 || body.len() != 1 + 4 * count {
        return None;
    }
    let mut ranges: Vec<AmbientRange> = Vec::with_capacity(count);
    for element in body[1..].chunks_exact(4) {
        let voice = match element[0] {
            0x00 => false,
            0x01 => true,
            _ => return None,
        };
        let (min, max, step) = (element[1], element[2], element[3]);
        if min > max || step == 0 || ranges.iter().any(|r| r.voice == voice) {
            return None;
        }
        ranges.push(AmbientRange { voice, min, max, step });
    }
    Some(Reading::NoiseCapability { kind, ranges })
}

// ── how well-founded a reading is ────────────────────────────────────────────

/// How well-founded a decoded reading is -- the read-side twin of
/// [`super::encode::Evidence`].
///
/// The module docs draw CAPTURED against HEADER ONLY in prose. That was
/// enough while every decoder was one or the other; it stopped being enough
/// when the reference CLIENT turned out to settle frames the corpus holds but
/// cannot explain on its own, and when other projects' observations of a
/// WH-1000XM5 turned out to settle frames the corpus does not hold at all. So
/// it is a value, and `qs-sony decode` prints it beside every frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grounds {
    /// A real headset sent this key -- [`Key`]'s, with a volunteered NTFY
    /// standing where its RET does on either table -- and it decoded to
    /// values that came out sensible. The corpus in `tests/fixtures/mdr/`
    /// holds it.
    Captured,
    /// The meaning is the reference client's: libmdr decodes this key the
    /// same way, and people run libmdr against real headsets. The bytes may
    /// be captured; the meaning was not settled by them alone.
    ReferenceClient,
    /// Another project reports a real headset sending this, laid out this
    /// way. Theirs, cited where it is used.
    ThirdParty,
    /// Transcribed from the reference header. Nothing has been seen sending
    /// it, and nothing but the header says what it means.
    HeaderOnly,
    /// A value the reference header's own `CODEGEN EnumRange` on the field
    /// excludes, read because the field's enum names it. Whatever its key
    /// stands on, this READING stands on less: the header says it will not
    /// come. The connection mode's SOUND_QUALITY_PRIOR is the case; see
    /// `decode_connection`. The write side's twin is
    /// `encode::Evidence::AgainstHeader`.
    AgainstHeader,
}

impl Grounds {
    pub fn as_str(self) -> &'static str {
        match self {
            Grounds::Captured => "captured",
            Grounds::ReferenceClient => "reference-client",
            Grounds::ThirdParty => "third-party",
            Grounds::HeaderOnly => "header-only",
            Grounds::AgainstHeader => "against-header",
        }
    }
}

/// `(data type, canonical command, inquired type)`.
///
/// CANONICAL ON BOTH TABLES: a NTFY is keyed as the RET it twins, because a
/// volunteered change is the same message as the reply to a question and a
/// headset seen sending one has been seen sending the other's layout. Table
/// 1 by [`Command::canonical`]; table 2 by the two twins `decode_table_two`
/// accepts, PERI_NTFY_STATUS (0x35) onto PERI_RET_STATUS (0x33) and
/// PERI_NTFY_PARAM (0x39) onto PERI_RET_PARAM (0x37). Table 2 used to be
/// keyed on the raw byte, so a pairing-list NTFY came out header-only while
/// the identical RET was captured.
pub type Key = (u8, u8, u8);

/// The [`Key`] a payload is grounded by, or `None` for one too short to
/// have a type.
pub fn key(data_type: super::DataType, payload: &[u8]) -> Option<Key> {
    let (&command, &kind) = (payload.first()?, payload.get(1)?);
    let wire = data_type.to_wire();
    let command = match wire {
        T1 => Command::from_byte(command).canonical().to_byte(),
        T2 => match command {
            0x35 => 0x33,
            0x39 => 0x37,
            other => other,
        },
        _ => command,
    };
    Some((wire, command, kind))
}

const T1: u8 = 0x0C;
const T2: u8 = 0x0E;

/// Every key the corpus holds that a decoder here reads, and nothing else.
///
/// HELD TO THE CORPUS BY A TEST, in both directions
/// (`the_captured_keys_are_exactly_the_ones_the_corpus_decodes`): a key here
/// that no fixture decodes would be a claim of capture with nothing behind
/// it, and a corpus key that decodes and is missing here would be reported
/// as header-only when a headset was seen sending it.
pub const CAPTURED: &[Key] = &[
    (T1, 0x05, 0x01),
    (T1, 0x05, 0x02),
    (T1, 0x07, 0x00),
    (T1, 0x13, 0x02),
    (T1, 0x23, 0x09),
    (T1, 0x23, 0x0A),
    (T1, 0x57, 0x00),
    (T1, 0x67, 0x17),
    (T1, 0xA3, 0x01),
    (T1, 0xA7, 0x20),
    (T1, 0xD1, 0xD1),
    (T1, 0xD1, 0xD2),
    (T1, 0xD7, 0xD1),
    (T1, 0xD7, 0xD2),
    (T1, 0xE1, 0x01),
    (T1, 0xE7, 0x01),
    (T1, 0xF7, 0x0C),
    (T1, 0xFB, 0x0C),
    (T2, 0x07, 0x00),
    (T2, 0x37, 0x02),
];

/// Keys decoded on grounds other than a capture of their own, with the
/// citation. Everything decodable and in neither list is
/// [`Grounds::HeaderOnly`].
pub const OTHER_GROUNDS: &[(Key, Grounds, &str)] = &[
    (
        (T1, 0x67, 0x15),
        Grounds::ThirdParty,
        "Gadgetbridge SonyProtocolImplV2.handleAmbientSoundControl, WH-1000XM5",
    ),
    (
        (T1, 0x67, 0x19),
        Grounds::ReferenceClient,
        "libmdr HeadphonesV2T1.cpp HandleNcAsmParamT1, MODE_NC_ASM_DUAL_NC_MODE_SWITCH_AND_ASM_SEAMLESS_NA",
    ),
    (
        (T1, 0x67, 0x22),
        Grounds::ReferenceClient,
        "libmdr HeadphonesV2T1.cpp HandleNcAsmParamT1, ASM_SEAMLESS",
    ),
    // Fixture 000097 holds these bytes; what the byte MEANS at this type is
    // libmdr's reading, which is why this is not in CAPTURED. See
    // `decode_upscaling_status`.
    (
        (T1, 0xE3, 0x01),
        Grounds::ReferenceClient,
        "libmdr HeadphonesV2T1.cpp HandleAudioStatusT1, UPSCALING as AudioRetStatusCommon",
    ),
    (
        (T1, 0xE7, 0x00),
        Grounds::ReferenceClient,
        "libmdr HeadphonesV2T1.cpp HandleAudioParamT1, CONNECTION_MODE as AudioRetParamConnection",
    ),
    // Plan 23 item 14. No capture: the headset asked nothing while the corpus
    // was recorded. See `decode_alert`.
    ((T1, 0x99, 0x00), Grounds::ReferenceClient, "libmdr HeadphonesV2T1.cpp HandleAlertParamT1, FIXED_MESSAGE"),
    // Table 2, plan 23 item 11. The corpus holds the bytes of all four; what
    // they mean is the header's types and libmdr's reading of them. See
    // `decode_table_two`.
    ((T2, 0x33, 0x02), Grounds::ReferenceClient, "libmdr HeadphonesV2T2.cpp HandlePeripheralStatusT2"),
    ((T2, 0x47, 0x01), Grounds::ReferenceClient, "libmdr HeadphonesV2T2.cpp HandleVoiceGuidanceParamT2"),
    ((T2, 0x47, 0x20), Grounds::ReferenceClient, "libmdr HeadphonesV2T2.cpp HandleVoiceGuidanceParamT2, VOLUME"),
    ((T2, 0x5B, 0x03), Grounds::ReferenceClient, "libmdr HeadphonesV2T2.cpp HandleSafeListeningExtendedParamT2"),
];

/// The grounds a decoded frame stands on, or `None` when nothing decodes it.
pub fn grounds(data_type: super::DataType, payload: &[u8]) -> Option<Grounds> {
    match decode(data_type, payload) {
        Reading::Unhandled { .. } | Reading::Ambiguous { .. } => return None,
        // The value, not the key: see `decode_connection`.
        Reading::Connection { prior, .. } if prior != PriorMode::ConnectionQuality => {
            return Some(Grounds::AgainstHeader);
        }
        _ => {}
    }
    let key = key(data_type, payload)?;
    if CAPTURED.contains(&key) {
        return Some(Grounds::Captured);
    }
    Some(OTHER_GROUNDS.iter().find(|(k, _, _)| *k == key).map_or(Grounds::HeaderOnly, |(_, g, _)| *g))
}
