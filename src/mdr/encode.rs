//! What this daemon would SAY to a headset.
//!
//! NOTHING IN THIS MODULE HAS EVER BEEN ACCEPTED BY A HEADSET, and nothing in
//! this environment can change that. Every one of the 70 captured frames is
//! `-rx` -- received -- and there is no Bluetooth adapter and no BlueZ here.
//! `payload.rs` can say "a real WF-1000XM5 sent these bytes and they decode to
//! this"; this module can never say anything of the kind about a byte it
//! emits. It is a separate file for that reason, so the first claim is not
//! diluted by the second.
//!
//! WHAT CAN BE PROVEN WITHOUT HARDWARE, AND IT IS MORE THAN NOTHING
//!
//! For five of the seven device verbs the SET struct is field-for-field
//! identical to its RET twin -- same types, same offsets, differing only in
//! byte 0. So an emitted SET, with byte 0 swapped for the RET command, must
//! decode through the EXISTING [`super::payload::decode`] to the very thing
//! the verb asked for. That is not self-consistency: the decoder was written
//! from captures and agrees with a real headset byte for byte on all five of
//! those messages, so the round trip is transitive against real hardware.
//!
//! The five, with the fixture each stands on:
//!
//! | verb | SET | RET | fixture |
//! |------|-----|-----|---------|
//! | `noise`, `ambient-level`, `focus-on-voice` | 0x68 | 0x67 | 000069 |
//! | `speak-to-chat` | 0xF8 | 0xF7 | 000081 |
//! | `dsee` | 0xE8 | 0xE7 | 000101 |
//! | `volume` | 0xA8 | 0xA7 | 000061 |
//! | `eq-preset`, `eq-band`, `clear-bass` | 0x58 | 0x57 | 000093 |
//!
//! The last row is one message, not three. `EqEbbSetParamEq` carries an
//! `EqParam { EqPresetId; MDRPodArray<UInt8> }` and there is no short form, so
//! every one of those verbs sends the preset AND the whole band array: the
//! preset verb replaces the first field and echoes the array, and the two band
//! verbs replace one element of the array and echo the preset.
//!
//! WHAT NO TEST HERE CAN REACH. `playback` has no field-identical twin --
//! `SetPlayStatusPlaybackController` (T1:8013) is four fields ending in a
//! `PlaybackControl`, while `RetPlayStatusPlaybackController` (T1:7451) is
//! five ending in a `PlaybackStatus` and a `MusicCallStatus`. `PlaybackControl`
//! and `PlaybackStatus` share value NAMES across different bytes, so there is
//! nothing to mirror it against. It is emitted at
//! [`Evidence::ReferenceClient`] -- libmdr sends the same four bytes -- and
//! says so. `connection` has a pinned twin at one type and only another
//! project's word at the other; see [`encode_connection`].
//!
//! A SET IS A WHOLE-STATE MESSAGE, WHICH IS THE RULE THIS MODULE IS BUILT ON
//!
//! None of the fourteen `NcAsmSetParam*` structs has an "unchanged" sentinel
//! in any field. The 0x17 payload carries the change status, the total effect,
//! the mode, the ambient mode AND the level, every time. So `ambient-level 12`
//! must re-transmit the mode and the focus-on-voice setting, and `noise anc`
//! must re-transmit the level.
//!
//! Every byte this module emits is therefore exactly one of three things:
//!
//!   1. named by the verb,
//!   2. echoed from a reading the headset itself sent, or
//!   3. not emitted at all, because the verb is refused.
//!
//! THERE IS NO FOURTH CASE. In particular there is no struct-default fallback:
//! an encoder filling the other three NC/ASM bytes from the header's brace
//! initialisers would emit `68 17 01 01 00 00 00` for `noise anc`, zeroing the
//! ambient level and switching focus-on-voice off every time somebody changed
//! their noise mode. [`Facts`] has no `Default` for that reason, and
//! `nothing_is_emitted_without_the_state_it_must_echo` fails the moment one
//! appears.
//!
//! THE INQUIRED TYPE IS ECHOED, NEVER INFERRED. The captured device answers
//! NC/ASM at 0x17, but the corpus is a WF-1000XM5 and this tree targets a
//! WH-1000XM5 -- "the wrong ear", as PROVENANCE.md puts it -- and the
//! WH-1000XM5 has been reported at 0x15, 0x17 and 0x19 by three different
//! projects. So the encoder writes at whatever type, and in whatever layout,
//! the attached headset answered with, and refuses when it has not answered.
//! The support table chooses only the type a QUESTION is asked at
//! (`ncasm_type`): a GET carries no value, so a wrong guess there costs a
//! round trip, where a wrong guess in a SET would write the wrong byte.

use super::payload::{
    self, AmbientRange, Command, DseeReport, EqReport, GeneralSetting, NoiseMode, NoiseReport,
    PlayStatusReport, PriorMode, Reading, SpeakToChatReport, SupportFunction, Switch, Table,
};
use super::DataType;
use crate::sony::command::{Command as Verb, EQ_PRESETS, MDR_VOLUME_MAX};

/// `EqPresetId` (T1:753), in the same order as [`EQ_PRESETS`].
///
/// ONE TABLE, INDEXED IN PARALLEL, because two tables of the same thing drift.
/// `encode` resolves a name only by its position in `EQ_PRESETS` and never by
/// matching a string of its own.
///
/// THE ORDER IS NOT THE ENUM'S ORDER, and that is the trap. `EQ_PRESETS` lists
/// heavy, clear, hard, soft (0x30..=0x33) BEFORE gaming and fps_1..3
/// (0x20..=0x23); the enum declares them the other way round. Mapping by index
/// into the enum instead of by name would swap eight presets in four pairs,
/// and every one of them is a real preset a headset would happily apply.
const EQ_PRESET_IDS: [u8; 30] = [
    0x00, // off               EqPresetId::OFF
    0x01, // rock              ROCK
    0x02, // pop               POP
    0x03, // jazz              JAZZ
    0x04, // dance             DANCE
    0x05, // edm               EDM
    0x06, // r_and_b_hip_hop   R_AND_B_HIP_HOP
    0x07, // acoustic          ACOUSTIC
    0x10, // bright            BRIGHT
    0x11, // excited           EXCITED
    0x12, // mellow            MELLOW
    0x13, // relaxed           RELAXED
    0x14, // vocal             VOCAL
    0x15, // treble            TREBLE
    0x16, // bass              BASS
    0x17, // speech            SPEECH
    0x30, // heavy             HEAVY      <- the enum has GAMING_EQ at 0x20 here
    0x31, // clear             CLEAR
    0x32, // hard              HARD
    0x33, // soft              SOFT
    0x20, // gaming            GAMING_EQ  <- and HEAVY at 0x30 here
    0x21, // fps_1             FPS_1
    0x22, // fps_2             FPS_2
    0x23, // fps_3             FPS_3
    0xA0, // custom            CUSTOM
    0xA1, // user_1            USER_SETTING1
    0xA2, // user_2            USER_SETTING2
    0xA3, // user_3            USER_SETTING3
    0xA4, // user_4            USER_SETTING4
    0xA5, // user_5            USER_SETTING5
];

/// How well-founded an emitted frame is.
///
/// The write-side form of the CAPTURED / HEADER ONLY distinction `payload.rs`
/// draws in prose, carried on the value so a `--dry-run` and a log can say
/// which of this port's frames somebody with a real headset should compare
/// against a capture first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    /// The SET struct is field-for-field identical to a RET struct, AND the
    /// corpus holds a frame of that RET. The round trip through
    /// [`super::payload::decode`] therefore reaches real captured bytes.
    MirroredCapture {
        /// The fixture counter, e.g. `"000069"`.
        fixture: &'static str,
        set: &'static str,
        ret: &'static str,
    },
    /// The bytes the reference CLIENT sends -- libmdr, which people run
    /// against real headsets -- for the same request, cited by file and line.
    ///
    /// Stronger than [`Evidence::HeaderOnly`], which is the header's word for
    /// a struct's layout: this is a program that has been run against
    /// hardware choosing those bytes. Weaker than a capture, because it is
    /// still read, not observed here, and libmdr's own test corpus is the same
    /// WF-1000XM5 this port's is.
    ReferenceClient { set: &'static str, cite: &'static str },
    /// The bytes another project sends to a real headset, and reports
    /// working, for a request the reference does not settle. Named so a
    /// reader knows the claim is theirs.
    ThirdParty { set: &'static str, source: &'static str },
    /// Transcribed from the header, with no frame in either direction and no
    /// twin to mirror it against.
    HeaderOnly { set: &'static str, unverified: &'static str },
}

impl Evidence {
    pub fn tier(self) -> &'static str {
        match self {
            Evidence::MirroredCapture { .. } => "mirrored-capture",
            Evidence::ReferenceClient { .. } => "reference-client",
            Evidence::ThirdParty { .. } => "third-party",
            Evidence::HeaderOnly { .. } => "header-only",
        }
    }

    /// The struct the frame was built as, whatever the tier.
    pub fn set(self) -> &'static str {
        match self {
            Evidence::MirroredCapture { set, .. }
            | Evidence::ReferenceClient { set, .. }
            | Evidence::ThirdParty { set, .. }
            | Evidence::HeaderOnly { set, .. } => set,
        }
    }
}

/// One frame this port is willing to send.
///
/// The data type is carried rather than assumed, and that is not decoration:
/// the two tables share 45 command bytes and disagree on all of them. 0x58 is
/// `EQEBB_SET_PARAM` on table 1 and `SAFE_LISTENING_SET_PARAM` on table 2, so
/// an equaliser write put on the wrong channel is a safe-listening write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub data_type: DataType,
    pub payload: Vec<u8>,
    pub evidence: Evidence,
}

impl Outgoing {
    fn new(data_type: DataType, evidence: Evidence, command: u8, kind: u8, body: &[u8]) -> Outgoing {
        let mut payload = Vec::with_capacity(2 + body.len());
        payload.push(command);
        payload.push(kind);
        payload.extend_from_slice(body);
        Outgoing { data_type, payload, evidence }
    }

    pub fn command(&self) -> Option<u8> {
        self.payload.first().copied()
    }

    pub fn kind(&self) -> Option<u8> {
        self.payload.get(1).copied()
    }

    /// `(command, kind, len)` -- the same triple [`super::payload::decode`]
    /// dispatches on, PRODUCED here rather than measured there.
    pub fn key(&self) -> Option<(u8, u8, usize)> {
        Some((self.command()?, self.kind()?, self.payload.len()))
    }

    /// The command byte of the reply that would confirm this write.
    ///
    /// Within a family the reference numbers GET, RET, SET and NTFY as
    /// base+0, +1, +2 and +3. That holds for all 91 derivable pairs across the
    /// 152 declared commands with no exceptions --
    /// `the_command_families_are_numbered_in_fours` checks it -- so the twin
    /// is arithmetic rather than a table that could fall out of step.
    pub fn ret_twin(&self) -> Option<u8> {
        self.command()?.checked_sub(1)
    }
}

/// Which side of the conversation a refusal is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// This port cannot do it. Maps to the tree's exit 2: could not run.
    Port,
    /// The headset cannot do it, or has not said enough yet. Exit 1: a
    /// definite answer about the world that is not what was asked for.
    Device,
}

/// Why a verb produced no bytes.
///
/// Every variant NAMES the thing it could not settle, in the same style as
/// [`Reading::Unhandled`] and [`Reading::Ambiguous`]: a refusal that does not
/// say which is a refusal nobody can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The headset has not answered for this family yet, so there is nothing
    /// to echo into the fields the verb does not name.
    NotRead { verb: &'static str, needs: &'static str },
    /// It answered at an inquired type this port has no SET layout for.
    UnknownVariant { verb: &'static str, family: &'static str, observed: u8 },
    /// It did not advertise the feature in either support table.
    NotAdvertised { verb: &'static str, feature: &'static str },
    /// Two structs in the reference claim this key and read a byte
    /// differently. Refused, exactly as the read side refuses.
    Ambiguous { verb: &'static str, command: u8, kind: u8, len: usize, between: [&'static str; 2] },
    /// The message carries a field the reference declares and never explains.
    Undocumented { verb: &'static str, field: &'static str, struct_name: &'static str },
    /// A value this port will not put on the wire.
    OutOfRange { verb: &'static str, value: i32, bound: &'static str },
    /// A value outside the range the headset itself stated, or outside the
    /// compatibility range when it has stated none.
    OutOfDeviceRange { verb: &'static str, value: i32, min: u8, max: u8, step: u8 },
    /// The headset has the feature and said, in its own last reply, that it
    /// will not take it now.
    Unavailable { verb: &'static str, said: &'static str },
    /// A verb with no device message behind it at all.
    NoEncoder { verb: &'static str, why: &'static str },
}

impl Refusal {
    pub fn side(self) -> Side {
        match self {
            // "I have not asked yet" and "your headset lacks it" are both
            // answers about the device.
            Refusal::NotRead { .. }
            | Refusal::NotAdvertised { .. }
            | Refusal::UnknownVariant { .. }
            | Refusal::OutOfRange { .. }
            | Refusal::OutOfDeviceRange { .. }
            | Refusal::Unavailable { .. } => Side::Device,
            // These three are this port's gaps, not the headset's.
            Refusal::Ambiguous { .. }
            | Refusal::Undocumented { .. }
            | Refusal::NoEncoder { .. } => Side::Port,
        }
    }

    /// One line, suitable for the daemon's `error <message>` reply.
    ///
    /// Every port-side message says "this daemon" and every device-side one
    /// says "this headset", so the distinction survives into the text a person
    /// actually reads.
    pub fn message(&self) -> String {
        match self {
            Refusal::NotRead { verb, needs } => {
                format!("{verb}: this headset has not answered {needs} yet")
            }
            Refusal::UnknownVariant { verb, family, observed } => format!(
                "{verb}: this headset answered {family} at type 0x{observed:02x}, \
                 which this daemon has no layout for"
            ),
            Refusal::NotAdvertised { verb, feature } => {
                format!("{verb}: this headset does not advertise {feature}")
            }
            Refusal::Ambiguous { verb, command, kind, len, between } => format!(
                "{verb}: this daemon refuses 0x{command:02x}/0x{kind:02x} at {len} bytes -- \
                 {} and {} both claim it and read a byte differently",
                between[0], between[1]
            ),
            Refusal::Undocumented { verb, field, struct_name } => format!(
                "{verb}: this daemon will not send {struct_name}, whose `{field}` \
                 the reference never explains"
            ),
            Refusal::OutOfRange { verb, value, bound } => {
                format!("{verb}: this headset takes {bound}, not {value}")
            }
            Refusal::OutOfDeviceRange { verb, value, min, max, step } if *step == 1 => {
                format!("{verb}: this headset takes {min} to {max}, not {value}")
            }
            Refusal::OutOfDeviceRange { verb, value, min, max, step } => {
                format!("{verb}: this headset takes {min} to {max} in steps of {step}, not {value}")
            }
            Refusal::NoEncoder { verb, why } => {
                format!("{verb}: this daemon sends no frame for it -- {why}")
            }
            Refusal::Unavailable { verb, said } => {
                format!("{verb}: this headset says it is unavailable now -- {said}")
            }
        }
    }
}

/// A reply the headset sent: the inquired type it answered at, and what this
/// port could make of the body.
///
/// The kind is recorded even when the body could not be decoded, and that is
/// deliberate: it is the difference between "you have not asked" and "your
/// headset answered at a type I cannot write".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed<T> {
    pub kind: u8,
    /// The reply's length, command and type included. The type alone does not
    /// choose a struct in this protocol -- five declare NC_ON_OFF at five
    /// lengths -- so an encoder that mirrors a reply needs both.
    pub len: usize,
    pub value: Option<T>,
}

/// What the headset has actually told us.
///
/// NO `Default`, and that is the whole discipline in one line. Everything here
/// was decoded from a frame the device sent; a `Facts` that could be conjured
/// out of nothing is an encoder that can emit a struct default by accident.
/// The only ways to get one are [`Facts::blank`], which reads as an assertion
/// at the call site, and [`Facts::observe`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    table_one: Vec<SupportFunction>,
    table_two: Vec<SupportFunction>,
    noise: Option<Observed<NoiseReport>>,
    /// The ambient level's ranges, at the inquired type they were reported
    /// for. See [`Facts::ambient_range`].
    noise_capability: Option<(u8, Vec<AmbientRange>)>,
    speak_to_chat: Option<Observed<SpeakToChatReport>>,
    dsee: Option<Observed<DseeReport>>,
    equalizer: Option<Observed<EqReport>>,
    volume: Option<Observed<u8>>,
    /// The link priority, at the type it was reported at -- which chooses the
    /// SET. See [`encode_connection`].
    connection: Option<Observed<PriorMode>>,
    /// Whether the headset will take a transport command. See
    /// [`encode_playback`].
    play_status: Option<Observed<PlayStatusReport>>,
    /// The headset's named on/off settings, by slot.
    ///
    /// KEYED BY SLOT AND NOT BY NAME, because the slot is what a GET or a SET
    /// carries as its inquired type -- the name is for a person. A
    /// `BTreeMap` rather than a `Vec` so the order a document lists them in is
    /// the device's slot order and not the order the replies happened to
    /// arrive in, which differs between a fresh link and a notification.
    general: std::collections::BTreeMap<u8, (Option<GeneralSetting>, Option<bool>)>,
}

impl Facts {
    /// A headset that has said nothing. Every device verb refuses.
    pub fn blank() -> Facts {
        Facts {
            table_one: Vec::new(),
            table_two: Vec::new(),
            noise: None,
            noise_capability: None,
            speak_to_chat: None,
            dsee: None,
            equalizer: None,
            volume: None,
            connection: None,
            play_status: None,
            general: std::collections::BTreeMap::new(),
        }
    }

    /// What the headset called the setting in `slot`, and whether it is on.
    pub fn general_setting(&self, slot: u8) -> Option<(&GeneralSetting, Option<bool>)> {
        let (info, on) = self.general.get(&slot)?;
        Some((info.as_ref()?, *on))
    }

    /// Every slot the headset has described, in slot order.
    pub fn general_settings(&self) -> impl Iterator<Item = (u8, &GeneralSetting, Option<bool>)> {
        self.general.iter().filter_map(|(slot, (info, on))| Some((*slot, info.as_ref()?, *on)))
    }

    /// Fold one received frame in.
    ///
    /// Takes the raw payload rather than a [`Reading`], and decodes it here,
    /// because the inquired-type byte is the encoder's most load-bearing input
    /// and `Reading` does not carry it. Reading it off the payload at the one
    /// place that has the payload makes losing it impossible rather than
    /// merely unlikely.
    pub fn observe(&mut self, data_type: DataType, payload: &[u8]) {
        let Some(kind) = payload.get(1).copied() else { return };
        let reading = payload::decode(data_type, payload);
        let command = payload.first().copied().unwrap_or(0);
        match reading {
            Reading::SupportFunctions { table: Table::One, functions } => self.table_one = functions,
            Reading::SupportFunctions { table: Table::Two, functions } => self.table_two = functions,
            Reading::Noise(report) => self.noise = Some(Observed { kind, len: payload.len(), value: Some(report) }),
            Reading::NoiseCapability { kind, ranges } => self.noise_capability = Some((kind, ranges)),
            Reading::Dsee(report) => {
                // The capability reply and the param reply are different
                // messages at the same inquired type, and each fills half.
                let merged = match self.dsee.as_ref().and_then(|o| o.value) {
                    Some(before) => DseeReport {
                        upscaling: report.upscaling.or(before.upscaling),
                        enabled: report.enabled.or(before.enabled),
                    },
                    None => report,
                };
                self.dsee = Some(Observed { kind, len: payload.len(), value: Some(merged) });
            }
            Reading::SpeakToChat(report) => {
                let merged = match self.speak_to_chat.as_ref().and_then(|o| o.value) {
                    Some(before) => SpeakToChatReport {
                        enabled: report.enabled.or(before.enabled),
                        preview: report.preview.or(before.preview),
                        sensitivity: report.sensitivity.or(before.sensitivity),
                        timeout: report.timeout.or(before.timeout),
                    },
                    None => report,
                };
                self.speak_to_chat = Some(Observed { kind, len: payload.len(), value: Some(merged) });
            }
            Reading::Equalizer(report) => {
                self.equalizer = Some(Observed { kind, len: payload.len(), value: Some(report) })
            }
            Reading::Volume { device_level } => {
                self.volume = Some(Observed { kind, len: payload.len(), value: Some(device_level) })
            }
            Reading::Connection { kind, prior } => {
                self.connection = Some(Observed { kind, len: payload.len(), value: Some(prior) })
            }
            Reading::PlayStatus(report) => {
                self.play_status = Some(Observed { kind, len: payload.len(), value: Some(report) })
            }
            // TWO MESSAGES, ONE SLOT. The capability reply says what the
            // setting IS and the param reply says whether it is on; they arrive
            // as separate frames at the same inquired type, and a SET needs
            // both -- the slot to address it and the name to have refused an
            // unknown one. Merged the way `dsee` above merges its pair.
            Reading::GeneralSettingInfo(info) => {
                let slot = info.slot;
                self.general.entry(slot).or_default().0 = Some(info);
            }
            Reading::GeneralSettingParam { slot, enabled } => {
                self.general.entry(slot).or_default().1 = Some(enabled);
            }
            // A reply this port could not read still tells us the headset
            // answered, and at which type. That is what turns "you have not
            // asked" into "your headset answered at 0x13, which is one of the
            // reference's genuine collisions".
            Reading::Unhandled { .. } | Reading::Ambiguous { .. } => {
                if Table::of(data_type) != Some(Table::One) {
                    return;
                }
                let slot = match Command::from_byte(command).canonical() {
                    Command::NcAsmRetParam => &mut self.noise,
                    _ => return,
                };
                if slot.is_none() {
                    *slot = Some(Observed { kind, len: payload.len(), value: None });
                }
            }
            // No verb writes any of these, so the encoder keeps nothing.
            Reading::Device(_)
            | Reading::Battery(_)
            | Reading::Codec(_)
            | Reading::PairedDevices(_)
            | Reading::UpscalingStatus { .. } => {}
        }
    }

    /// The ambient level's range in one ambient sound mode, as the headset
    /// reported it -- but only when it reported it at the inquired type its
    /// noise reading came from. A range stated for one NC/ASM variant is not
    /// evidence about another.
    pub fn ambient_range(&self, voice: bool) -> Option<AmbientRange> {
        let (kind, ranges) = self.noise_capability.as_ref()?;
        if self.noise.as_ref().map(|o| o.kind) != Some(*kind) {
            return None;
        }
        ranges.iter().find(|r| r.voice == voice).copied()
    }

    pub(crate) fn advertises_one(&self, codes: &[u8]) -> bool {
        self.table_one.iter().any(|f| codes.contains(&f.code))
    }
}

// ── the verbs ────────────────────────────────────────────────────────────────

/// Turn one command-language verb into one frame, or say why not.
///
/// Total: every verb and every legal argument reaches an arm, and no arm can
/// panic. `Err` is an ordinary answer, not a fault.
pub fn encode(facts: &Facts, verb: &Verb) -> Result<Outgoing, Refusal> {
    match verb {
        // These four are the daemon talking to itself. They are not refusals
        // in the sense of a gap -- there is simply no wire message.
        Verb::Status => Err(Refusal::NoEncoder { verb: "status", why: "it reads the state document" }),
        Verb::PanelOpen => {
            Err(Refusal::NoEncoder { verb: "panel-open", why: "it is a shell-side request" })
        }
        Verb::PanelClose => {
            Err(Refusal::NoEncoder { verb: "panel-close", why: "it is a shell-side request" })
        }
        Verb::Refresh => {
            Err(Refusal::NoEncoder { verb: "refresh", why: "it re-sends the opening queries" })
        }

        Verb::Noise(mode) => encode_ncasm(facts, NcAsmEdit::Mode(mode)),
        Verb::AmbientLevel(level) => encode_ncasm(facts, NcAsmEdit::Level(*level)),
        Verb::FocusOnVoice(on) => encode_ncasm(facts, NcAsmEdit::FocusOnVoice(*on)),
        Verb::SpeakToChat(on) => encode_speak_to_chat(facts, *on),
        Verb::Dsee(on) => encode_dsee(facts, *on),
        Verb::EqualizerPreset(name) => encode_eq_preset(facts, name),
        Verb::EqBand { index, value } => {
            encode_eq_steps(facts, "eq-band", EqEdit::Band { index: *index, value: *value })
        }
        Verb::ClearBass(value) => encode_eq_steps(facts, "clear-bass", EqEdit::ClearBass(*value)),
        Verb::GeneralSetting { key, on } => encode_general_setting(facts, key, *on),
        Verb::Volume(percent) => encode_volume(facts, *percent),
        Verb::Playback(word) => encode_playback(facts, word),
        Verb::ConnectionPriority(word) => encode_connection(facts, word),
    }
}

enum NcAsmEdit<'a> {
    Mode(&'a str),
    Level(i32),
    FocusOnVoice(bool),
}

impl NcAsmEdit<'_> {
    fn verb(&self) -> &'static str {
        match self {
            NcAsmEdit::Mode(_) => "noise",
            NcAsmEdit::Level(_) => "ambient-level",
            NcAsmEdit::FocusOnVoice(_) => "focus-on-voice",
        }
    }
}

/// The ambient level's bound when the headset has not stated one: the C++
/// daemon's 1..=20 in steps of one, which is also what libmdr and Gadgetbridge
/// enforce. A compatibility choice, not a fact; see [`AmbientRange`].
pub const COMPAT_AMBIENT_RANGE: AmbientRange = AmbientRange { voice: false, min: 1, max: 20, step: 1 };

/// One NC/ASM write, built in the layout the headset itself last answered in.
///
/// All three NC/ASM verbs land here, and each names ONE byte. The other value
/// bytes are echoed from the headset's own last reading, because the struct
/// has no way to say "leave this alone".
///
/// WHICH LAYOUT IS THE HEADSET'S CHOICE, NOT OURS. It answered NCASM_GET_PARAM
/// at some inquired type and some length, and that pair names the SET. Three
/// pairs are written, each on its own grounds:
///
///   (0x17, 7)  `NcAsmSetParamModeNcDualModeSwitchAsmSeamless` (T1:5614),
///              field-for-field the RET twin fixture 000069 carries. MIRRORED
///              CAPTURE.
///   (0x19, 9)  `NcAsmSetParamModeNcDualModeSwitchAsmSeamlessNa` (T1:5634):
///              the same five bytes, then the adaptive switch and its
///              sensitivity, echoed. REFERENCE CLIENT: libmdr sends exactly
///              this struct for a headset advertising noise adaptation
///              (`HeadphonesV2.cpp` `RequestCommitV2`).
///   (0x15, 7)  The 0x17 layout at MODE_NC_ASM_AUTO_NC_MODE_SWITCH_AND_ASM_
///              SEAMLESS, which no struct in the header declares. THIRD
///              PARTY: Gadgetbridge writes its WH-1000XM5 this way
///              (`SonyProtocolImplV2.setAmbientSoundControl`).
///
/// The other layouts the decoder reads are refused by name. At 0x13 both SET
/// structs are seven bytes and read offset 6 differently (`Refusal::Ambiguous`,
/// the SET half of the collision PROVENANCE.md records). At 0x22 the reference
/// client contradicts itself -- it reads `NcAsmRetParamAsmSeamless` at 0x22 and
/// writes `NcAsmSetParamAsmSeamless` with the type left at its default, 0x01 --
/// and at 0x01 nothing this daemon asks for ever answers. A write in a layout
/// with no evidence behind it is a write that may do something else.
fn encode_ncasm(facts: &Facts, edit: NcAsmEdit) -> Result<Outgoing, Refusal> {
    let verb = edit.verb();
    let Some(observed) = facts.noise.as_ref() else {
        return Err(Refusal::NotRead { verb, needs: "NCASM_GET_PARAM" });
    };
    if observed.kind == 0x13 {
        return Err(Refusal::Ambiguous {
            verb,
            command: 0x68,
            kind: 0x13,
            len: 7,
            between: ["NcAsmSetParamNcOnOffAsmOnOff", "NcAsmSetParamNcOnOffAsmSeamless"],
        });
    }
    let evidence = match (observed.kind, observed.len) {
        (0x17, 7) => Evidence::MirroredCapture {
            fixture: "000069",
            set: "NcAsmSetParamModeNcDualModeSwitchAsmSeamless",
            ret: "NcAsmRetParamModeNcDualModeSwitchAsmSeamless",
        },
        (0x19, 9) => Evidence::ReferenceClient {
            set: "NcAsmSetParamModeNcDualModeSwitchAsmSeamlessNa",
            cite: "libmdr HeadphonesV2.cpp RequestCommitV2, the noise-adaptation branch",
        },
        (0x15, 7) => Evidence::ThirdParty {
            set: "NcAsmSetParam at MODE_NC_ASM_AUTO_NC_MODE_SWITCH_AND_ASM_SEAMLESS",
            source: "Gadgetbridge SonyProtocolImplV2.setAmbientSoundControl, WH-1000XM5",
        },
        _ => return Err(Refusal::UnknownVariant { verb, family: "NC/ASM", observed: observed.kind }),
    };
    let Some(now) = observed.value else {
        return Err(Refusal::NotRead { verb, needs: "a NCASM_RET_PARAM this port could read" });
    };

    // Start from what the headset last said, then move exactly one thing.
    let mut effect = if now.mode == NoiseMode::Off { 0x00 } else { 0x01 };
    // NcAsmMode as the wire carried it. NOT re-derived from `now.mode`, which
    // collapses to Off when the total effect is off and so cannot say whether
    // the headset is sitting in NC or ASM behind a disabled switch. Re-deriving
    // it would move a headset from ambient to cancelling the first time
    // somebody turned the feature off and on again.
    let (Some(mut asm_mode), Some(focus_on_voice), Some(mut level)) =
        (now.mode_byte, now.focus_on_voice, now.ambient_level)
    else {
        return Err(Refusal::NotRead { verb, needs: "a NC/ASM reply carrying a mode, a voice and a level" });
    };
    let mut voice = u8::from(focus_on_voice);

    match edit {
        NcAsmEdit::Mode("off") => effect = 0x00,
        NcAsmEdit::Mode("anc") => {
            effect = 0x01;
            asm_mode = 0x00;
        }
        NcAsmEdit::Mode("ambient") => {
            effect = 0x01;
            asm_mode = 0x01;
        }
        // `parse_words` accepts only those three, so this is unreachable from
        // the command language -- and it refuses rather than assuming.
        NcAsmEdit::Mode(other) => {
            return Err(Refusal::NoEncoder { verb, why: leaked(other) });
        }
        NcAsmEdit::Level(value) => {
            // THE HEADSET'S RANGE, WHEN IT HAS STATED ONE, for the ambient
            // mode the level will apply in. Only a level edit is held to it:
            // turning focus-on-voice on with a level the voice range does not
            // reach is the headset's to clamp, and refusing it would make the
            // switch unusable at levels the person never chose.
            let range = facts.ambient_range(focus_on_voice).unwrap_or(COMPAT_AMBIENT_RANGE);
            if !range.admits(value) {
                return Err(Refusal::OutOfDeviceRange {
                    verb,
                    value,
                    min: range.min,
                    max: range.max,
                    step: range.step,
                });
            }
            // `admits` bounds it to 0..=255 already: min and max are bytes.
            level = value as u8;
        }
        NcAsmEdit::FocusOnVoice(on) => voice = u8::from(on),
    }

    // ValueChangeStatus at offset 2: CHANGED (0x01), always. It used to be
    // echoed from the headset's last reading, which made a reading caught
    // mid-change -- UNDER_CHANGING, as a headset reports while its own slider
    // is moving -- into a write that said it was still moving. libmdr sends
    // CHANGED for every NC/ASM write (`HeadphonesV2.cpp` `RequestCommitV2`)
    // and Gadgetbridge sends 0x01; fixture 000069 carries 0x01 as well.
    let change = 0x01;
    let mut body = vec![change, effect, asm_mode, voice, level];
    if observed.kind == 0x19 {
        // The two noise-adaptation fields, echoed: no verb moves them.
        let (Some(adaptive), Some(sensitivity)) = (now.adaptive, now.adaptive_sensitivity) else {
            return Err(Refusal::NotRead { verb, needs: "a noise-adaptation reply carrying both its fields" });
        };
        body.extend_from_slice(&[u8::from(adaptive), sensitivity]);
    }
    Ok(Outgoing::new(DataType::DataMdr, evidence, 0x68, observed.kind, &body))
}

/// A word the command language cannot produce. Kept as a named function so the
/// refusal reads the same everywhere and carries no caller-supplied text.
fn leaked(_word: &str) -> &'static str {
    "the argument is not one this daemon's command language accepts"
}

/// `SystemSetParamSmartTalking` (T1:9039), identical to the RET twin fixture
/// 000081 carries.
///
/// Four bytes, and the second value byte is `previewModeOnOffValue` -- a field
/// the state document has nowhere to publish. It is echoed, not defaulted: a
/// person who turned Sony's preview on and then toggled Speak-to-Chat from
/// here would otherwise have it silently turned off.
fn encode_speak_to_chat(facts: &Facts, on: bool) -> Result<Outgoing, Refusal> {
    let verb = "speak-to-chat";
    if !facts.advertises_one(&[0xF2, 0xFC]) {
        return Err(Refusal::NotAdvertised { verb, feature: "Speak-to-Chat" });
    }
    let Some(observed) = facts.speak_to_chat.as_ref() else {
        return Err(Refusal::NotRead { verb, needs: "SYSTEM_GET_PARAM at SMART_TALKING_MODE_TYPE2" });
    };
    if observed.kind != 0x0C {
        return Err(Refusal::UnknownVariant { verb, family: "Speak-to-Chat", observed: observed.kind });
    }
    let Some(now) = observed.value else {
        return Err(Refusal::NotRead { verb, needs: "a SYSTEM_RET_PARAM this port could read" });
    };
    let Some(preview) = now.preview.and_then(Switch::to_on_off_setting) else {
        return Err(Refusal::NotRead { verb, needs: "a preview-mode value to echo back" });
    };
    // OnOffSettingValue is INVERTED: ON is 0x00.
    let enabled = if on { Switch::On } else { Switch::Off };
    let Some(enabled) = enabled.to_on_off_setting() else {
        return Err(Refusal::NoEncoder { verb, why: "no byte means \"no value\"" });
    };
    Ok(Outgoing::new(
        DataType::DataMdr,
        Evidence::MirroredCapture {
            fixture: "000081",
            set: "SystemSetParamSmartTalking",
            ret: "SystemRetParamSmartTalking",
        },
        0xF8,
        0x0C,
        &[enabled, preview],
    ))
}

/// `AudioSetParamUpscaling` (T1:3148), identical to the RET twin fixture
/// 000101 carries. `UpscalingTypeAutoOff` is the ORDINARY way round.
fn encode_dsee(facts: &Facts, on: bool) -> Result<Outgoing, Refusal> {
    let verb = "dsee";
    if !facts.advertises_one(&[0xE2, 0xED]) {
        return Err(Refusal::NotAdvertised { verb, feature: "DSEE" });
    }
    let Some(observed) = facts.dsee.as_ref() else {
        return Err(Refusal::NotRead { verb, needs: "AUDIO_GET_PARAM at UPSCALING" });
    };
    if observed.kind != 0x01 {
        return Err(Refusal::UnknownVariant { verb, family: "upscaling", observed: observed.kind });
    }
    let setting = if on { Switch::On } else { Switch::Off };
    let Some(byte) = setting.to_auto_off() else {
        return Err(Refusal::NoEncoder { verb, why: "no byte means \"no value\"" });
    };
    Ok(Outgoing::new(
        DataType::DataMdr,
        Evidence::MirroredCapture {
            fixture: "000101",
            set: "AudioSetParamUpscaling",
            ret: "AudioRetParamUpscaling",
        },
        0xE8,
        0x01,
        &[byte],
    ))
}

/// `EqEbbSetParamEq` (T1:9800): the preset AND the band-step array.
///
/// The steps are echoed unchanged, and that is forced rather than chosen. The
/// message carries an `EqParam { EqPresetId; MDRPodArray<UInt8> }` with no
/// short form, and the reference names no neutral curve to send instead -- so
/// the only array this port can honestly put on the wire is the one the
/// headset last sent it. Sending an empty array would be a claim that a
/// zero-length curve is legal, which nothing here establishes.
fn encode_eq_preset(facts: &Facts, name: &str) -> Result<Outgoing, Refusal> {
    let verb = "eq-preset";
    if !facts.advertises_one(&[0x50, 0x51, 0x52, 0x53, 0x55, 0x56, 0x57]) {
        return Err(Refusal::NotAdvertised { verb, feature: "an equaliser" });
    }
    let Some(index) = EQ_PRESETS.iter().position(|p| *p == name) else {
        return Err(Refusal::NoEncoder { verb, why: leaked(name) });
    };
    let Some(preset) = EQ_PRESET_IDS.get(index).copied() else {
        return Err(Refusal::NoEncoder { verb, why: "the preset tables are out of step" });
    };
    let Some(observed) = facts.equalizer.as_ref() else {
        return Err(Refusal::NotRead { verb, needs: "EQEBB_GET_PARAM at PRESET_EQ" });
    };
    if observed.kind != 0x00 {
        return Err(Refusal::UnknownVariant { verb, family: "equaliser", observed: observed.kind });
    }
    let Some(now) = observed.value.as_ref() else {
        return Err(Refusal::NotRead { verb, needs: "an EQEBB_RET_PARAM this port could read" });
    };
    let Ok(count) = u8::try_from(now.steps.len()) else {
        return Err(Refusal::NoEncoder { verb, why: "the observed band array is longer than a byte" });
    };
    let mut body = Vec::with_capacity(2 + now.steps.len());
    body.push(preset);
    body.push(count);
    body.extend_from_slice(&now.steps);
    Ok(Outgoing::new(
        DataType::DataMdr,
        Evidence::MirroredCapture {
            fixture: "000093",
            set: "EqEbbSetParamEq",
            ret: "EqEbbRetParamEq",
        },
        0x58,
        0x00,
        &body,
    ))
}

/// `GeneralSettingSetParam` (0xD8): switch one of the headset's own named
/// settings.
///
/// ADDRESSED BY THE DEVICE'S NAME, NOT BY A SLOT NUMBER, because the slot is
/// not stable across models: a WF-1000XM5 puts sidetone in 0xD1 and a
/// WH-1000XM5 puts the touch panel there. A person typing
/// `general multipoint off` means the setting the headset CALLS multipoint,
/// and this looks it up in what the headset said rather than in a table here.
///
/// MIRRORED CAPTURE, on the RET twin. Fixtures 000035 (`d7 d1 00 01`) and
/// 000043 (`d7 d2 00 00`) are `GeneralSettingRetParam`, and the SET is the
/// same four bytes with the command advanced by one -- the same
/// RET-is-the-SET property the other five verbs stand on, and the reason the
/// round-trip test can put this frame back through the real decoder.
///
/// THE THIRD BYTE IS `GsSettingType::BOOLEAN_TYPE`, NAMED, NOT A LITERAL
/// NOBODY COULD EXPLAIN. `GsSetParamBoolean` (T1:4323) declares it, and the
/// decoder refuses a setting of any other type, so a setting this encodes a
/// write for is a boolean by construction. The fourth is a `GsSettingValue`,
/// whose ON is 0x00 -- written the other way round until the decoder's
/// evidence for that (`decode_general_setting_param`) was found, which is why
/// `general multipoint on` used to send what the headset reads as OFF.
///
/// READ EVIDENCE FOR THE WRITE ITSELF, and theirs, not ours:
/// `delarosa1312/omarchy-sony-xm5` (MIT, Copyright (c) 2026 Delarosa1312),
/// `docs/protocol.md` -- "on the WH-1000XM5 there are exactly two, both
/// writable: `TOUCH_PANEL_SETTING` and `MULTIPOINT_SETTING`". Nothing in this
/// tree has spoken to a headset, so whether the write sticks is their claim.
/// What IS ours is the frame's shape, which the capture settles.
///
/// THE FIRST MATCH WINS, and the corpus cannot say whether that is ever a
/// choice. Two slots whose keys collided -- `X_SETTING` beside a bare `X` --
/// would make `general x` address the lower slot silently. No capture and no
/// reference list contains such a pair, and inventing a tie-break for a shape
/// nobody has seen would be a guess wearing a rule; what the document does
/// instead is publish both rows with their slots, so the collision is visible
/// to a reader even though this verb cannot express it.
fn encode_general_setting(facts: &Facts, key: &str, on: bool) -> Result<Outgoing, Refusal> {
    let verb = "general";
    let Some(slot) =
        facts.general_settings().find(|(_, info, _)| info.key() == key).map(|(slot, _, _)| slot)
    else {
        // NOT `NotAdvertised`, which would say the headset lacks the FEATURE.
        // It may have general settings and simply not this one -- a WF-1000XM5
        // has no touch panel -- and it may not have answered yet. `NotRead`
        // says the honest thing: we have not been told about it.
        return Err(Refusal::NotRead {
            verb,
            needs: "a GENERAL_SETTING_RET_CAPABILITY naming that setting",
        });
    };
    Ok(Outgoing::new(
        DataType::DataMdr,
        Evidence::MirroredCapture {
            fixture: "000035",
            set: "GeneralSettingSetParam",
            ret: "GeneralSettingRetParam",
        },
        0xD8,
        slot,
        &[payload::GS_BOOLEAN_TYPE, if on { payload::GS_ON } else { payload::GS_OFF }],
    ))
}

/// Which part of the curve a write is moving.
///
/// The band index is 1-BASED, as `command`'s grammar numbers it, because the
/// person typing `eq-band 1 -3` is talking about the first band and not about
/// an array offset. The array offset it becomes depends on the shape: with
/// clear bass present, band 1 is `steps[1]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EqEdit {
    Band { index: i32, value: i32 },
    ClearBass(i32),
}

/// `EqEbbSetParamEq` again -- the same message [`encode_eq_preset`] sends, with
/// the STEPS replaced instead of the preset.
///
/// THE ONE SOUND CONTROL ANYBODY HAS SHOWN TO STICK. Presets, connection
/// priority and the auto-power-off timer are acknowledged and discarded on this
/// device family; the bands and clear bass survive a reconnect.
/// `delarosa1312/omarchy-sony-xm5` (MIT, Copyright (c) 2026 Delarosa1312),
/// `docs/protocol.md`, established both by writing, restarting the daemon and
/// reading from a fresh session. Read evidence, theirs: nothing in this tree
/// has spoken to a headset.
///
/// THEIR BIGGEST TRAP CANNOT HAPPEN HERE, and that is worth saying because
/// somebody reading their document will come looking for the workaround. They
/// found that clear bass is the first element of the band array on the wire,
/// and that libmdr's staging model therefore lets you send half a change: a
/// clear-bass-only edit is committed locally and never sent, and a bands-only
/// edit ships whatever clear bass was last staged, so clear bass drifts a step
/// every time you touch the bands. Their `write_eq()` restates both every time
/// to defeat it. This port has no staging: the message carries `EqParam
/// { EqPresetId; MDRPodArray<UInt8> }` with no short form, so the WHOLE array
/// goes out or nothing does, and restating both is not a discipline here but
/// the only thing the encoder can do.
///
/// The bounds are the DEVICE'S, taken from the shape of the array it last sent
/// rather than from a constant: five bands run -10..=+10 and ten run -6..=+6.
/// That is the opposite of `ambient-level`, whose 1..=20 is a compatibility
/// contract with the C++ daemon and no fact about any headset -- see
/// `sony::command`. Here the headset has told us, so we use what it said.
fn encode_eq_steps(facts: &Facts, verb: &'static str, edit: EqEdit) -> Result<Outgoing, Refusal> {
    if !facts.advertises_one(&[0x50, 0x51, 0x52, 0x53, 0x55, 0x56, 0x57]) {
        return Err(Refusal::NotAdvertised { verb, feature: "an equaliser" });
    }
    let Some(observed) = facts.equalizer.as_ref() else {
        return Err(Refusal::NotRead { verb, needs: "EQEBB_GET_PARAM at PRESET_EQ" });
    };
    if observed.kind != 0x00 {
        return Err(Refusal::UnknownVariant { verb, family: "equaliser", observed: observed.kind });
    }
    let Some(now) = observed.value.as_ref() else {
        return Err(Refusal::NotRead { verb, needs: "an EQEBB_RET_PARAM this port could read" });
    };

    let shape = payload::EqShape::of(now.steps.len());
    // A READER MAY GUESS AT AN UNNAMED COUNT AND A WRITER MAY NOT.
    // `sony::session` shows an array of some other length as bands at bias ten
    // by analogy, because a curve that might be off by a constant beats no
    // curve. Sending one back would write a curve nobody asked for, at a bias
    // no reference names. `EqShape::Unknown` also covers the empty array, which
    // is a preset-only reply and carries no curve to edit.
    if shape == payload::EqShape::Unknown {
        return Err(Refusal::UnknownVariant {
            verb,
            family: "an equaliser band array",
            // The LENGTH is what is unrecognised, and it is what somebody
            // adding a layout for this device would need. `observed` is a byte
            // and every named shape is under 256 steps.
            observed: u8::try_from(now.steps.len()).unwrap_or(u8::MAX),
        });
    }

    let (at, value, bound) = match edit {
        EqEdit::ClearBass(value) => {
            let Some(at) = shape.clear_bass_at() else {
                return Err(Refusal::NoEncoder {
                    verb,
                    why: "this headset's band array carries no clear bass",
                });
            };
            (at, value, "a clear bass step from -10 to 10")
        }
        EqEdit::Band { index, value } => {
            let bands = i32::try_from(shape.bands()).unwrap_or(i32::MAX);
            if index < 1 || index > bands {
                return Err(Refusal::OutOfRange {
                    verb,
                    value: index,
                    bound: match shape {
                        payload::EqShape::TenBands => "a band from 1 to 10",
                        _ => "a band from 1 to 5",
                    },
                });
            }
            // 1-based to an array offset, past clear bass when the shape has
            // one. `clear_bass_at` is `Some(0)` there, so the shift is its
            // length rather than a literal 1.
            let offset = shape.clear_bass_at().map_or(0, |_| 1);
            let at = (index as usize - 1) + offset;
            let bound = match shape {
                payload::EqShape::TenBands => "a step from -6 to 6",
                _ => "a step from -10 to 10",
            };
            (at, value, bound)
        }
    };

    if value < -shape.bound() || value > shape.bound() {
        return Err(Refusal::OutOfRange { verb, value, bound });
    }
    let mut steps = now.steps.clone();
    // The bias back on. In range by the check above, so this cannot wrap: the
    // widest case is +10 on a bias of 10, which is 20.
    let Some(slot) = steps.get_mut(at) else {
        return Err(Refusal::NoEncoder { verb, why: "the band array is shorter than its shape" });
    };
    let Ok(byte) = u8::try_from(value + shape.bias()) else {
        return Err(Refusal::OutOfRange { verb, value, bound });
    };
    *slot = byte;

    let Ok(count) = u8::try_from(steps.len()) else {
        return Err(Refusal::NoEncoder { verb, why: "the observed band array is longer than a byte" });
    };
    // THE PRESET IS ECHOED, not dropped. The message names both, and sending
    // a different preset alongside a band change would be two edits where one
    // was asked for -- and the preset half would be discarded anyway.
    let mut body = Vec::with_capacity(2 + steps.len());
    body.push(now.preset);
    body.push(count);
    body.extend_from_slice(&steps);
    Ok(Outgoing::new(
        DataType::DataMdr,
        Evidence::MirroredCapture {
            fixture: "000093",
            set: "EqEbbSetParamEq",
            ret: "EqEbbRetParamEq",
        },
        0x58,
        0x00,
        &body,
    ))
}

/// `SetPlayParamPlaybackControllerVolume` (T1:7989), identical to the RET twin
/// fixture 000061 carries.
///
/// WRITTEN AT THE TYPE THE HEADSET ANSWERED AT, which is 0x20 (MUSIC_VOLUME)
/// on the captured device and NOT the 0x01 the struct declares. The struct's
/// type field is annotated `CODEGEN Ignore OUT_OF_RANGE is expected` rather
/// than `CODEGEN EnumRange`, so its default is not a pin -- the third instance
/// in this protocol of a struct whose declared type contradicts the wire.
fn encode_volume(facts: &Facts, percent: i32) -> Result<Outgoing, Refusal> {
    let verb = "volume";
    if !(0..=100).contains(&percent) {
        return Err(Refusal::OutOfRange { verb, value: percent, bound: "0 to 100" });
    }
    let Some(observed) = facts.volume.as_ref() else {
        return Err(Refusal::NotRead { verb, needs: "PLAY_GET_PARAM at MUSIC_VOLUME" });
    };
    let level = crate::sony::command::percent_to_mdr_volume(percent);
    let Ok(level) = u8::try_from(level.clamp(0, MDR_VOLUME_MAX)) else {
        return Err(Refusal::OutOfRange { verb, value: level, bound: "a single byte" });
    };
    Ok(Outgoing::new(
        DataType::DataMdr,
        Evidence::MirroredCapture {
            fixture: "000061",
            set: "SetPlayParamPlaybackControllerVolume",
            ret: "RetPlayParamPlaybackControllerVolume",
        },
        0xA8,
        observed.kind,
        &[level],
    ))
}

/// `SetPlayStatusPlaybackController` (T1:8013), the four bytes libmdr sends.
///
/// The four transport words rest on the strongest single citation in the
/// reference: `// CODEGEN EnumRange PlaybackControl::PAUSE
/// PlaybackControl::TRACK_UP PlaybackControl::TRACK_DOWN PlaybackControl::PLAY`
/// -- the field is restricted to exactly those four, which settles that `next`
/// and `previous` are TRACK_UP and TRACK_DOWN and not GROUP_UP and GROUP_DOWN.
///
/// OFFSET 2 IS THE ONE BYTE IN THIS MODULE NEITHER NAMED NOR ECHOED, and what
/// stands behind it is the reference client rather than the header: libmdr
/// fills `status = EnableDisable::ENABLE` (0x00) on every transport write
/// (`HeadphonesV2.cpp:438-443`), and people drive real headsets with it. The
/// header alone gives the field no EnumRange and no comment. This used to call
/// itself header-only and say so; with the client's bytes in hand it is
/// [`Evidence::ReferenceClient`], and still not a mirror: there is no RET twin
/// to round-trip through.
///
/// GATED ON WHAT THE HEADSET SAID, in two ways. It must advertise a playback
/// controller (t1 0xA1..=0xA4), and when its last play-status reply put
/// DISABLE in the same offset-2 field, the write is refused: the headset has
/// said it will not take one now. Offset 2 of that reply is a field both
/// colliding RET structs agree on -- see `payload::PlayStatusReport` -- and
/// fixture 000065 carries ENABLE there. No reply yet is not a refusal: the
/// write echoes nothing from it, so it has nothing to wait for.
fn encode_playback(facts: &Facts, word: &str) -> Result<Outgoing, Refusal> {
    let verb = "playback";
    // PlaybackControl (T1:1284). PAUSE = 0x01 and PLAY = 0x07; PlaybackStatus
    // is a DIFFERENT enum with overlapping names at different bytes, and
    // reading one for the other is how "pause" becomes 0x02.
    let control = match word {
        "play" => 0x07u8,
        "pause" => 0x01,
        "next" => 0x02,     // TRACK_UP
        "previous" => 0x03, // TRACK_DOWN
        other => return Err(Refusal::NoEncoder { verb, why: leaked(other) }),
    };
    if !facts.advertises_one(&[0xA1, 0xA2, 0xA3, 0xA4]) {
        return Err(Refusal::NotAdvertised { verb, feature: "a playback controller (t1 0xA1..0xA4)" });
    }
    if let Some(report) = facts.play_status.as_ref().and_then(|o| o.value) {
        if report.control == Switch::Off {
            return Err(Refusal::Unavailable {
                verb,
                said: "PLAY_RET_STATUS put DISABLE in its control field",
            });
        }
    }
    Ok(Outgoing::new(
        DataType::DataMdr,
        Evidence::ReferenceClient {
            set: "SetPlayStatusPlaybackController",
            cite: "libmdr HeadphonesV2.cpp:438-443, status = EnableDisable::ENABLE",
        },
        0xA4,
        0x01,
        &[ENABLE, control],
    ))
}

/// `EnableDisable::ENABLE` (`ProtocolV2.hpp:16`). Inverted: 0x00.
const ENABLE: u8 = 0x00;

/// The link-priority write, in the form the headset last answered in.
///
/// THE TYPE IS THE HEADSET'S, as for NC/ASM, and the two it can answer at
/// stand on different grounds:
///
///   0x02  `AudioSetParamConnectionWithLdacStatus` (T1:3084). The RET
///         (T1:2814), this SET and the NTFY (T1:2637) all pin the type with
///         `CODEGEN EnumRange` and are field-identical: command, type,
///         `PriorMode`. HEADER ONLY -- no capture either way -- but a pinned
///         triple is the strongest header-only there is.
///   0x00  `AudioSetParamConnection` (T1:3056) at CONNECTION_MODE, which is
///         NOT what the header declares: that struct pins its type to 0x05,
///         the LE Audio form, and libmdr sends it unchanged -- `e8 05 <mode>`
///         (`HeadphonesV2.cpp:664-673`), three bytes at a type whose own SET
///         is four. mos9527/SonyHeadphonesClient PR #64 changed it to
///         `e8 00 <mode>` and reports that working on a WF-LC900. THIRD PARTY,
///         and on a different headset; it is sent only to a headset that has
///         itself answered at 0x00, so the type is at least its own.
///
/// WHAT IS STILL REFUSED. A headset that has not answered gets `NotRead` when
/// it advertises t1 0xE1 or 0xE3 (the GET is in [`feature_gets`]). One that
/// advertises only 0xE7, the LE Audio form -- the captured WF-1000XM5 -- gets
/// the old refusal by name: that SET's fourth byte is the `value2` the
/// reference never explains (`sony::state` `apply_support_functions` has the
/// long form). Nothing else is a connection mode this port knows.
fn encode_connection(facts: &Facts, word: &str) -> Result<Outgoing, Refusal> {
    let verb = "connection";
    let prior = match word {
        "quality" => PriorMode::SoundQuality,
        "stable" => PriorMode::ConnectionQuality,
        other => return Err(Refusal::NoEncoder { verb, why: leaked(other) }),
    };
    match facts.connection.as_ref() {
        Some(Observed { kind: 0x02, len: 3, .. }) => Ok(Outgoing::new(
            DataType::DataMdr,
            Evidence::HeaderOnly {
                set: "AudioSetParamConnectionWithLdacStatus",
                unverified: "no frame either way; RET, SET and NTFY pin the type and agree field for field",
            },
            0xE8,
            0x02,
            &[prior.to_byte()],
        )),
        Some(Observed { kind: 0x00, len: 3, .. }) => Ok(Outgoing::new(
            DataType::DataMdr,
            Evidence::ThirdParty {
                set: "AudioSetParamConnection",
                source: "mos9527/SonyHeadphonesClient PR #64: e8 00 <mode>, reported on a WF-LC900",
            },
            0xE8,
            0x00,
            &[prior.to_byte()],
        )),
        Some(observed) => Err(Refusal::UnknownVariant {
            verb,
            family: "AUDIO_RET_PARAM's connection mode",
            observed: observed.kind,
        }),
        None if facts.advertises_one(&[0xE1, 0xE3]) => {
            Err(Refusal::NotRead { verb, needs: "AUDIO_GET_PARAM at its connection mode" })
        }
        None if facts.advertises_one(&[0xE7]) => Err(Refusal::Undocumented {
            verb,
            field: "value2",
            struct_name: "AudioSetParamConnectionModeClassicAudioLeAudio",
        }),
        None => Err(Refusal::NotAdvertised { verb, feature: "a connection mode (t1 0xE1, 0xE3 or 0xE7)" }),
    }
}

// ── asking ───────────────────────────────────────────────────────────────────

/// A GET: the command, then the inquired type.
///
/// Two bytes for every GET this sends but one -- `ConnectGetProtocolInfo`
/// (T1:3477), `ConnectGetCapabilityInfo` (T1:3455), `ConnectGetDeviceInfo`
/// (T1:3466) and `ConnectGetSupportFunction` (T1:3488) are two fields each, and
/// so are the per-feature GETs. The exception is [`gs_get_capability`]. A GET
/// carries no state, so none of the echo machinery above applies to it.
fn get(data_type: DataType, command: u8, kind: u8, cite: &'static str) -> Outgoing {
    Outgoing::new(
        data_type,
        Evidence::HeaderOnly { set: cite, unverified: "nothing -- a GET carries no value" },
        command,
        kind,
        &[],
    )
}

/// The queries that bring a headset from "just connected" to "it has told us
/// what it is".
///
/// IN THE ORDER THE CAPTURE SHOWS THE APP ASKING. Every fixture is `-rx`, so
/// the transmitted half was never recorded -- but the counter gaps ARE the
/// transmitted half, and the order of what came back is the order it was
/// asked for. Frames 000002, 000007, 000011, 000015, 000019, 000022 and
/// 000026 are the replies, in that sequence.
///
/// The support-function query is asked TWICE, once per channel, because the
/// two tables are separate namespaces and a device answers each separately.
/// That is not an inference: the corpus contains both replies, 54ms apart, one
/// on `DATA_MDR` and one on `DATA_MDR_NO2`.
pub fn handshake() -> Vec<Outgoing> {
    vec![
        get(DataType::DataMdr, 0x00, 0x00, "ConnectGetProtocolInfo"),
        get(DataType::DataMdr, 0x02, 0x00, "ConnectGetCapabilityInfo"),
        // DeviceInfoType: MODEL_NAME, FW_VERSION, SERIES_AND_COLOR_INFO. The
        // third is asked for and its reply deliberately not decoded -- see
        // `decode_device_info`.
        get(DataType::DataMdr, 0x04, 0x01, "ConnectGetDeviceInfo"),
        get(DataType::DataMdr, 0x04, 0x02, "ConnectGetDeviceInfo"),
        get(DataType::DataMdr, 0x04, 0x03, "ConnectGetDeviceInfo"),
        get(DataType::DataMdr, 0x06, 0x00, "ConnectGetSupportFunction"),
        get(DataType::DataMdrNo2, 0x06, 0x00, "ConnectGetSupportFunction"),
    ]
}

/// The per-feature queries a headset's own support tables justify.
///
/// Asked for only what it advertised. A GET for a feature a device does not
/// have is a round trip spent to be told nothing, and -- worse -- a reply this
/// port would then have to decide how to read.
///
/// The battery queries are the clearest case: this device advertises
/// `LR_BATTERY_LEVEL_WITH_THRESHOLD` (0x29) and
/// `CRADLE_BATTERY_LEVEL_WITH_THRESHOLD` (0x2A) and NOT
/// `BATTERY_LEVEL_INDICATOR` (0x20), and the corpus contains replies at
/// exactly 0x09 and 0x0A and none at 0x00. The table predicts the query.
pub fn feature_gets(facts: &Facts) -> Vec<Outgoing> {
    let mut out = Vec::new();
    // POWER_GET_STATUS at the battery type each advertised function implies.
    for (function, kind) in [(0x20u8, 0x00u8), (0x28, 0x08), (0x21, 0x01), (0x29, 0x09), (0x22, 0x02), (0x2A, 0x0A)] {
        if facts.advertises_one(&[function]) {
            out.push(get(DataType::DataMdr, 0x22, kind, "PowerGetStatus"));
        }
    }
    // POWER_GET_PARAM for the auto-power-off timer. The advertised function
    // names the wearing-detection form; the plain form answers at 0x04.
    for (function, kind) in [(0x24u8, 0x04u8), (0x25, 0x05)] {
        if facts.advertises_one(&[function]) {
            out.push(get(DataType::DataMdr, 0x26, kind, "PowerGetParam"));
        }
    }
    // NCASM_GET_PARAM. The type is the one the device will answer at, and this
    // port does not know it until it has -- so the query goes out at the type
    // its advertised function names, and the REPLY is what the encoder then
    // echoes. This is the one place a function-to-type correspondence is used,
    // and it is used to ask a question rather than to write a value.
    if let Some(kind) = ncasm_type(facts) {
        out.push(get(DataType::DataMdr, 0x66, kind, "NcAsmGetParam"));
        // And the ambient level's range at the same type: NcAsmGetCapability
        // (T1:4842), a plain two-byte GET. Neither reference client asks for
        // it; this does because it is the one place a headset says what its
        // ambient levels are. See `payload::decode_noise_capability`.
        out.push(get(DataType::DataMdr, 0x60, kind, "NcAsmGetCapability"));
    }
    if facts.advertises_one(&[0x50, 0x51, 0x52, 0x53, 0x55, 0x56, 0x57]) {
        out.push(get(DataType::DataMdr, 0x50, 0x00, "EqEbbGetCapability"));
        out.push(get(DataType::DataMdr, 0x52, 0x00, "EqEbbGetStatus"));
        out.push(get(DataType::DataMdr, 0x56, 0x00, "EqEbbGetParam"));
    }
    if facts.advertises_one(&[0xA1, 0xA2, 0xA3, 0xA4]) {
        out.push(get(DataType::DataMdr, 0xA0, 0x01, "PlayGetCapability"));
        out.push(get(DataType::DataMdr, 0xA6, 0x01, "PlayGetParam"));
        out.push(get(DataType::DataMdr, 0xA6, 0x20, "PlayGetParam"));
        out.push(get(DataType::DataMdr, 0xA2, 0x01, "PlayGetStatus"));
    }
    // AUDIO_*_* at UPSCALING (0x01), on UPSCALING_AUTO_OFF (0xE2) ALONE. This
    // also asked for 0xED once, and 0xED's messages live at a different type
    // -- UPSCALING_AUTO_OFF_WITH_STATUS_DISABLE_REASON, 0x0B, a status and a
    // reason list -- so a headset advertising only that one was asked at a
    // type it has no reason to answer. libmdr gates the same three on 0xE2
    // (`HeadphonesV2.cpp:83-86,184-188`).
    if facts.advertises_one(&[0xE2]) {
        out.push(get(DataType::DataMdr, 0xE0, 0x01, "AudioGetCapability"));
        out.push(get(DataType::DataMdr, 0xE2, 0x01, "AudioGetStatus"));
        out.push(get(DataType::DataMdr, 0xE6, 0x01, "AudioGetParam"));
    }
    // AUDIO_GET_PARAM at the connection mode, and only one: the LDAC form
    // (0x02) when 0xE3 is advertised, since its SET is the pinned one, else
    // the plain form (0x00) on 0xE1, as libmdr asks (`HeadphonesV2.cpp:
    // 179-181`). The reply's type is what `encode_connection` then writes at.
    if facts.advertises_one(&[0xE3]) {
        out.push(get(DataType::DataMdr, 0xE6, 0x02, "AudioGetParam"));
    } else if facts.advertises_one(&[0xE1]) {
        out.push(get(DataType::DataMdr, 0xE6, 0x00, "AudioGetParam"));
    }
    // COMMON_GET_STATUS at AUDIO_CODEC, gated on t1 CODEC_INDICATOR (0x12,
    // `ProtocolV2.hpp`), which the captured table 1 advertises. Where the
    // recorder asked it too: fixture 000053 answers `13 02 02` right after the
    // AUDIO_RET_CAPABILITY reply.
    if facts.advertises_one(&[0x12]) {
        out.push(get(DataType::DataMdr, 0x12, 0x02, "CommonGetStatus"));
    }
    // SYSTEM_GET_PARAM, one per advertised system function. The inquired type
    // is `SystemInquiredType`, which is NOT the function code -- 0xF1 is the
    // function and 0x01 is the type it answers at.
    for (function, kind) in
        [(0xF1u8, 0x01u8), (0xF3, 0x03), (0xF2, 0x02), (0xFC, 0x0C), (0xFF, 0x0F), (0xF6, 0x06)]
    {
        if facts.advertises_one(&[function]) {
            out.push(get(DataType::DataMdr, 0xF6, kind, "SystemGetParam"));
        }
    }
    // The extended reply carries the sensitivity and the timeout.
    if facts.advertises_one(&[0xFC]) {
        out.push(get(DataType::DataMdr, 0xFA, 0x0C, "SystemGetExtParam"));
    }
    // GENERAL_SETTING_GET_CAPABILITY, one per advertised slot. The inquired
    // type here IS the function code, which is unusual and is why the corpus
    // shows GENERAL_SETTING_RET_CAPABILITY arriving at 0xD1 and 0xD2.
    for slot in [0xD1u8, 0xD2, 0xD3, 0xD4] {
        if facts.advertises_one(&[slot]) {
            out.push(gs_get_capability(slot));
            out.push(get(DataType::DataMdr, 0xD6, slot, "GsGetParam"));
        }
    }
    out
}

/// The inquired type to ask NC/ASM at, from the function the headset
/// advertises, or `None` when it advertises none this port reads.
///
/// libmdr's order (`HeadphonesV2.cpp` `RequestInitV2`): 0x6B
/// (MODE_NC_ASM_..._LEVEL_ADJUSTMENT) asks 0x17, 0x6D (the noise-adaptation
/// variant) asks 0x19, 0x67 (AMBIENT_SOUND_MODE_LEVEL_ADJUSTMENT) asks 0x22.
/// Then 0x68 (MODE_NC_ASM_..._DUAL_AUTO_...), which libmdr does not ask about
/// at all, asks 0x15 -- the type its name pairs with, and where Gadgetbridge
/// drives a WH-1000XM5. Asking is safe where writing might not be: a GET
/// carries no value, and whatever type the reply comes back at is what the
/// encoder then mirrors.
fn ncasm_type(facts: &Facts) -> Option<u8> {
    [(0x6Bu8, 0x17u8), (0x6D, 0x19), (0x67, 0x22), (0x68, 0x15)]
        .into_iter()
        .find(|(function, _)| facts.advertises_one(&[*function]))
        .map(|(_, kind)| kind)
}

/// `GsGetCapability` (T1:4129): THREE bytes, not two.
///
/// The struct is `command, GsInquiredType type, DisplayLanguage
/// displayLanguage` -- the headset is asked what a setting is called AND in
/// which language to say it. This sent two bytes for as long as it existed,
/// the shape every other GET has, which is a short struct: what a headset
/// makes of a GET missing its last field is not something any capture shows.
/// The reference client sends `ENGLISH` (0x01, T1:636; libmdr
/// `HeadphonesV2.cpp` `RequestInitV2`, every slot), so this does too. The names
/// the corpus carries are `GsStringFormat::ENUM_NAME` identifiers
/// (`MULTIPOINT_SETTING`), so the language changes nothing this port reads
/// today; it is sent because the struct has the field.
fn gs_get_capability(slot: u8) -> Outgoing {
    Outgoing::new(
        DataType::DataMdr,
        Evidence::HeaderOnly { set: "GsGetCapability", unverified: "nothing -- a GET carries no value" },
        0xD0,
        slot,
        &[DISPLAY_LANGUAGE_ENGLISH],
    )
}

/// `DisplayLanguage::ENGLISH` (T1:636).
const DISPLAY_LANGUAGE_ENGLISH: u8 = 0x01;

/// The `EqPresetId` byte for the name at `index` in [`EQ_PRESETS`].
///
/// Exposed so the READ side can turn a preset byte back into the document's
/// word without a second table. Two tables of the same mapping drift, and this
/// one has an order that invites exactly that: `EQ_PRESETS` lists heavy..soft
/// before gaming..fps_3 while `EqPresetId` declares them the other way round.
pub fn preset_id_at(index: usize) -> Option<u8> {
    EQ_PRESET_IDS.get(index).copied()
}
