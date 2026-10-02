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
//! nothing to mirror it against. It is emitted at [`Evidence::HeaderOnly`] and
//! says so.
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
//! WH-1000XM5 -- "the wrong ear", as PROVENANCE.md puts it. So the encoder
//! writes at whatever type the attached headset answered at, and refuses when
//! it has not answered. It does NOT derive the type from the support table:
//! t1 `FunctionType` 0x6B corresponds to `NcAsmInquiredType` 0x17 by a name
//! match, and a name match is what this module exists not to trust.

use super::payload::{
    self, Command, DseeReport, EqReport, GeneralSetting, NoiseMode, NoiseReport, Reading,
    SpeakToChatReport, SupportFunction, Switch, Table,
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
    /// Transcribed from the header, with no frame in either direction and no
    /// twin to mirror it against.
    HeaderOnly { set: &'static str, unverified: &'static str },
}

impl Evidence {
    pub fn tier(self) -> &'static str {
        match self {
            Evidence::MirroredCapture { .. } => "mirrored-capture",
            Evidence::HeaderOnly { .. } => "header-only",
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
            | Refusal::OutOfRange { .. } => Side::Device,
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
            Refusal::NoEncoder { verb, why } => {
                format!("{verb}: this daemon sends no frame for it -- {why}")
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
    speak_to_chat: Option<Observed<SpeakToChatReport>>,
    dsee: Option<Observed<DseeReport>>,
    equalizer: Option<Observed<EqReport>>,
    volume: Option<Observed<u8>>,
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
            speak_to_chat: None,
            dsee: None,
            equalizer: None,
            volume: None,
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
            Reading::Noise(report) => self.noise = Some(Observed { kind, value: Some(report) }),
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
                self.dsee = Some(Observed { kind, value: Some(merged) });
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
                self.speak_to_chat = Some(Observed { kind, value: Some(merged) });
            }
            Reading::Equalizer(report) => {
                self.equalizer = Some(Observed { kind, value: Some(report) })
            }
            Reading::Volume { device_level } => {
                self.volume = Some(Observed { kind, value: Some(device_level) })
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
                    *slot = Some(Observed { kind, value: None });
                }
            }
            // No verb writes any of these, so the encoder keeps nothing.
            Reading::Device(_) | Reading::Battery(_) | Reading::Codec(_) | Reading::PairedDevices(_) => {}
        }
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
        Verb::Playback(word) => encode_playback(word),

        // See `sony::state::Capabilities::apply_support_functions` for the
        // long form. The short of it: the only connection function the
        // captured device family advertises leads to a SET whose fourth byte
        // the reference names `value2` and never explains, and no
        // `AudioRetParam*` is pinned at its inquired type to confirm a write.
        Verb::ConnectionPriority(_) => Err(Refusal::Undocumented {
            verb: "connection",
            field: "value2",
            struct_name: "AudioSetParamConnectionModeClassicAudioLeAudio",
        }),
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

/// `NcAsmSetParamModeNcDualModeSwitchAsmSeamless` (T1:5614), the seven-byte
/// form, field-for-field identical to the RET twin fixture 000069 carries.
///
/// All three NC/ASM verbs land here, and each names ONE byte. The other three
/// value bytes are echoed from the headset's own last reading, because the
/// struct has no way to say "leave this alone".
fn encode_ncasm(facts: &Facts, edit: NcAsmEdit) -> Result<Outgoing, Refusal> {
    let verb = edit.verb();
    let Some(observed) = facts.noise.as_ref() else {
        return Err(Refusal::NotRead { verb, needs: "NCASM_GET_PARAM" });
    };
    // The SET half of the collision PROVENANCE.md records. It exists here and
    // NOT on the read side: the two RET structs are six and seven fields, so
    // only the NTFY pair collides there, while both SET structs are seven.
    if observed.kind == 0x13 {
        return Err(Refusal::Ambiguous {
            verb,
            command: 0x68,
            kind: 0x13,
            len: 7,
            between: ["NcAsmSetParamNcOnOffAsmOnOff", "NcAsmSetParamNcOnOffAsmSeamless"],
        });
    }
    if observed.kind != 0x17 {
        return Err(Refusal::UnknownVariant { verb, family: "NC/ASM", observed: observed.kind });
    }
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
    let Some(mut asm_mode) = now.mode_byte else {
        return Err(Refusal::NotRead { verb, needs: "a NC/ASM reply carrying a mode byte" });
    };
    let mut voice = u8::from(now.focus_on_voice);
    let mut level = now.ambient_level;

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
            if !(0..=255).contains(&value) {
                return Err(Refusal::OutOfRange { verb, value, bound: "a single byte" });
            }
            level = value as u8;
        }
        NcAsmEdit::FocusOnVoice(on) => voice = u8::from(on),
    }

    // ValueChangeStatus at offset 2. CHANGED (0x01) is what the captured RET
    // carries and what a settled value means; the reference does not say what
    // a client sends, so this echoes the headset's own last word rather than
    // inventing one. `now.settled` IS that byte, decoded.
    let change = u8::from(now.settled);
    Ok(Outgoing::new(
        DataType::DataMdr,
        Evidence::MirroredCapture {
            fixture: "000069",
            set: "NcAsmSetParamModeNcDualModeSwitchAsmSeamless",
            ret: "NcAsmRetParamModeNcDualModeSwitchAsmSeamless",
        },
        0x68,
        0x17,
        &[change, effect, asm_mode, voice, level],
    ))
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

/// `SetPlayStatusPlaybackController` (T1:8013). HEADER ONLY.
///
/// The four transport words rest on the strongest single citation in the
/// reference: `// CODEGEN EnumRange PlaybackControl::PAUSE
/// PlaybackControl::TRACK_UP PlaybackControl::TRACK_DOWN PlaybackControl::PLAY`
/// -- the field is restricted to exactly those four, which settles that `next`
/// and `previous` are TRACK_UP and TRACK_DOWN and not GROUP_UP and GROUP_DOWN.
///
/// What it does NOT settle is offset 2, an `EnableDisable status` with no
/// EnumRange, no comment and an unpinned default. ENABLE is 0x00 -- the
/// inverted convention -- and this port sends it, which is the one byte in
/// this module that is neither named by a verb nor echoed from a reading. It
/// is called out here rather than buried because it is the exception to the
/// module's own rule, and because there is no RET twin to mirror it against:
/// `RetPlayStatusPlaybackController` (T1:7451) is five fields ending in a
/// `PlaybackStatus` and a `MusicCallStatus`.
fn encode_playback(word: &str) -> Result<Outgoing, Refusal> {
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
    Ok(Outgoing::new(
        DataType::DataMdr,
        Evidence::HeaderOnly {
            set: "SetPlayStatusPlaybackController",
            unverified: "offset 2, an EnableDisable with no EnumRange and no RET twin to mirror",
        },
        0xA4,
        0x01,
        &[0x00, control],
    ))
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
    if facts.advertises_one(&[0x6B]) {
        out.push(get(DataType::DataMdr, 0x66, 0x17, "NcAsmGetParam"));
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
    if facts.advertises_one(&[0xE2, 0xED]) {
        out.push(get(DataType::DataMdr, 0xE0, 0x01, "AudioGetCapability"));
        out.push(get(DataType::DataMdr, 0xE2, 0x01, "AudioGetStatus"));
        out.push(get(DataType::DataMdr, 0xE6, 0x01, "AudioGetParam"));
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
