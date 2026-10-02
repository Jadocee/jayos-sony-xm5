//! The state document, schema version 1.
//!
//! One JSON object per line, written atomically to
//! `$XDG_STATE_HOME/sony-headphones/status.json`, mode 0600 in a 0700
//! directory. Anything on the machine may read it; only this daemon writes it.
//!
//! WHY THE KEY ORDER IS FIXED AND THE SERIALISER IS HAND-ROLLED
//!
//! serde_json's map is a `BTreeMap`, so routing this through `Value` would sort
//! every key alphabetically -- `ambient_level` before `mode`, `case` before
//! `main` -- and the document would no longer match the C++ daemon's byte for
//! byte. That matters for one specific reason: `qs-sony` carries a schema
//! version guard so that this implementation and an already-installed C++ one
//! are interchangeable. A reader that works against one and not the other
//! defeats the guard.
//!
//! So the order here is the order in `state.cpp:63-141`, and there is a test
//! that pins the whole document against the byte-exact output of that function
//! for the same input.
//!
//! WHAT THIS IS NOT
//!
//! It is not the shape `qs-sony` was reading. That helper was written against a
//! different project and looked for `name` and `model` at the top level and a
//! bare number at `battery.main`; the documented daemon has never written
//! either. Fixing the reader is a separate commit -- the writer goes first,
//! because it is the thing that defines what there is to read.

use crate::mdr::payload::{SupportFunction, Table};
use std::fmt::Write as _;

/// The schema this writes, and the number a reader checks before trusting a
/// field. Bumping it is a promise that a reader written for 1 will refuse
/// rather than guess.
pub const SCHEMA_VERSION: i64 = 1;

/// One battery, as the document reports it.
///
/// `level` is -1 when the part is absent, and `present` is the field that says
/// so. Both are written, because a reader that looks only at the level has to
/// know that -1 is a sentinel -- and the one in this tree did not, so a headset
/// with no case battery reported "-1%".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Battery {
    pub present: bool,
    pub level: i32,
    pub charging: String,
}

impl Default for Battery {
    fn default() -> Battery {
        Battery { present: false, level: -1, charging: "unknown".to_string() }
    }
}

/// Which of the four the device actually has is a capability question, not a
/// battery question: a WH-1000XM5 reports `main` alone, a WF-1000XM5 reports
/// `left`, `right` and `case` and never `main`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Batteries {
    pub main: Battery,
    pub left: Battery,
    pub right: Battery,
    pub case: Battery,
}

/// The 27 capability flags, always all present in the document.
///
/// Always written, even when false, so a reader can tell "this device does not
/// support it" from "this daemon is too old to know". A panel gates its
/// controls on these: a control drawn for a capability the headset lacks is a
/// button that does nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub battery_single: bool,
    pub battery_left_right: bool,
    pub battery_case: bool,
    pub playback_metadata: bool,
    pub playback_control: bool,
    pub playback_volume: bool,
    pub noise_cancelling: bool,
    pub ambient_sound: bool,
    pub adaptive_ambient_sound: bool,
    pub speak_to_chat: bool,
    pub listening_mode: bool,
    pub equalizer: bool,
    pub dsee: bool,
    pub paired_device_management: bool,
    pub pairing_mode: bool,
    pub general_settings: bool,
    pub assignable_controls: bool,
    pub noise_control_button: bool,
    pub auto_power_off: bool,
    pub wearing_detection: bool,
    pub auto_pause: bool,
    pub head_gesture: bool,
    pub voice_guidance: bool,
    pub voice_guidance_volume: bool,
    pub shutdown: bool,
    pub connection_mode: bool,
    pub safe_listening: bool,
}

impl Capabilities {
    /// Every flag with its document name, in document order.
    ///
    /// The order is the contract -- the C++ daemon writes them in this order
    /// and `serialize` must match byte for byte -- so it is written once here
    /// rather than repeated at every place that iterates them.
    pub fn entries(&self) -> [(&'static str, bool); 27] {
        [
            ("battery_single", self.battery_single),
            ("battery_left_right", self.battery_left_right),
            ("battery_case", self.battery_case),
            ("playback_metadata", self.playback_metadata),
            ("playback_control", self.playback_control),
            ("playback_volume", self.playback_volume),
            ("noise_cancelling", self.noise_cancelling),
            ("ambient_sound", self.ambient_sound),
            ("adaptive_ambient_sound", self.adaptive_ambient_sound),
            ("speak_to_chat", self.speak_to_chat),
            ("listening_mode", self.listening_mode),
            ("equalizer", self.equalizer),
            ("dsee", self.dsee),
            ("paired_device_management", self.paired_device_management),
            ("pairing_mode", self.pairing_mode),
            ("general_settings", self.general_settings),
            ("assignable_controls", self.assignable_controls),
            ("noise_control_button", self.noise_control_button),
            ("auto_power_off", self.auto_power_off),
            ("wearing_detection", self.wearing_detection),
            ("auto_pause", self.auto_pause),
            ("head_gesture", self.head_gesture),
            ("voice_guidance", self.voice_guidance),
            ("voice_guidance_volume", self.voice_guidance_volume),
            ("shutdown", self.shutdown),
            ("connection_mode", self.connection_mode),
            ("safe_listening", self.safe_listening),
        ]
    }

    /// Fold one `CONNECT_RET_SUPPORT_FUNCTION` reply into these flags.
    ///
    /// ONLY EVER SETS FLAGS TRUE. A headset answers on both tables and neither
    /// answer is the whole picture -- the captured WF-1000XM5 gets its
    /// equaliser and its noise cancelling from table 1 and its voice guidance,
    /// its safe listening and its paired-device list from table 2. Clearing a
    /// flag here would mean the second reply erased the first.
    ///
    /// HOW A CODE BECOMES A FLAG, AND THE ONE RULE BEHIND IT
    ///
    /// The reference gives every function a name that states what it is, and
    /// those names are the mapping: a function whose name contains
    /// NOISE_CANCELLING sets `noise_cancelling`, one containing AMBIENT_SOUND
    /// sets `ambient_sound`, one containing VOICE_GUIDANCE sets
    /// `voice_guidance`, and so on. Applied mechanically so a reviewer can
    /// check every arm below against the enum in one pass, rather than
    /// re-deriving somebody's reasoning. Where a name pulls two ways it is
    /// spelled out at the arm.
    ///
    /// Four places where the mechanical answer is deliberately NOT taken, each
    /// because a name coincides with a different feature:
    ///
    ///   * `CONNECTION_MODE_CLASSIC_AUDIO_LE_AUDIO` (t1 0xE7) does not set
    ///     `connection_mode`, and NOT because it is a different control: it
    ///     carries the same `PriorMode` enum -- SOUND_QUALITY_PRIOR 0x00,
    ///     CONNECTION_QUALITY_PRIOR 0x01 -- at the same offset as the two
    ///     forms that do (T1:3076 against T1:3062). The reason is that this
    ///     port could not drive it if it claimed it. Its SET
    ///     (`AudioSetParamConnectionModeClassicAudioLeAudio`, T1:3069) carries
    ///     a FOURTH byte the reference names only `value2`, with no comment
    ///     anywhere -- one of five such unnamed placeholders in the header --
    ///     and no `AudioRetParam*` struct is pinned at that inquired type
    ///     (0x05) to confirm the write. So the flag would draw a control this
    ///     port cannot send correctly and could not read back.
    ///
    ///     The flag is therefore reachable only through
    ///     `CONNECTION_MODE_SOUND_QUALITY_CONNECTION_QUALITY` (0xE1) and its
    ///     LDAC twin (0xE3), and one capture of a 0x05 exchange would settle
    ///     whether it should widen.
    ///   * `HEAD_TRACKER_*` (t1 0xB5, 0xB7) does not set `head_gesture`. Head
    ///     tracking is 360 Reality Audio following where you look; head
    ///     gestures are nodding to answer a call. The captured device
    ///     advertises 0xB7 and 0xFF, so on this device the mistake would not
    ///     even show -- it would show on the next one.
    ///   * `AUTO_POWER_OFF_WITH_WEARING_DETECTION` (t1 0x25) does not set
    ///     `wearing_detection`. It says the auto-power-off timer can use the
    ///     wear sensor, not that the headset reports wear status; that is
    ///     `WEARING_STATUS_DETECTOR` (t1 0xF6) and `WEARING_STATUS_CHECKER`
    ///     (t2 0xF0).
    ///   * `UPSCALING_INDICATOR` (t1 0x13) does not set `dsee`. The 0x10..0x1B
    ///     block is what Sony's app should DISPLAY, not what the headset can be
    ///     told to do; the DSEE setting is `UPSCALING_AUTO_OFF` (0xE2).
    ///
    /// And one whole block is unmapped on purpose. t1 0x45..0x4F are the
    /// `*_CANT_BE_USED_WITH_LEA_CONNECTION` names, each a limitation ON a
    /// feature rather than the feature -- so 0x47, whose name begins
    /// `PAIRING_DEVICE_MANAGEMENT_`, sets nothing, and paired-device
    /// management is earned on table 2 or not at all.
    ///
    /// ONE FLAG IS NEVER SET HERE. `noise_control_button` has no counterpart:
    /// no function type in either table names a physical noise-control button.
    /// The nearest, `AMBIENT_SOUND_CONTROL_MODE_SELECT` (t1 0x69), is which
    /// modes the NC/ASM control cycles through, which is not the same claim.
    /// Left false rather than mapped to something adjacent -- an unset flag is
    /// a control the panel does not draw, and a wrongly set one is a control
    /// that does nothing.
    pub fn apply_support_functions(&mut self, table: Table, functions: &[SupportFunction]) {
        for function in functions {
            match table {
                Table::One => self.apply_table_one(function.code),
                Table::Two => self.apply_table_two(function.code),
            }
        }
    }

    /// `mdr::v2::t1::FunctionType`, in the enum's own order.
    fn apply_table_one(&mut self, code: u8) {
        match code {
            // BATTERY_LEVEL_INDICATOR, BATTERY_LEVEL_WITH_THRESHOLD.
            0x20 | 0x28 => self.battery_single = true,
            // LEFT_RIGHT_BATTERY_LEVEL_INDICATOR, LR_BATTERY_LEVEL_WITH_THRESHOLD.
            0x21 | 0x29 => self.battery_left_right = true,
            // CRADLE_BATTERY_LEVEL_INDICATOR, CRADLE_BATTERY_LEVEL_WITH_THRESHOLD.
            0x22 | 0x2A => self.battery_case = true,
            // POWER_OFF.
            0x23 => self.shutdown = true,
            // AUTO_POWER_OFF, AUTO_POWER_OFF_WITH_WEARING_DETECTION.
            0x24 | 0x25 => self.auto_power_off = true,
            // PRESET_EQ, EBB, PRESET_EQ_NON_CUSTOMIZABLE, PRESET_EQ_AND_ULT_MODE,
            // CUSTOM_EQ, TURN_KEY_EQ, PRESET_EQ_AND_ERRORCODE.
            //
            // 0x54 SOUND_EFFECT, 0x58 ULT_SOUND_EFFECT_ASSIGN and 0x59
            // CUSTOMIZABLE_SOUND_EFFECT are in the same block and are NOT the
            // equaliser: they travel on the same command family and carry a
            // different structure, and the document has no field for them.
            0x50..=0x53 | 0x55..=0x57 => self.equalizer = true,
            // NOISE_CANCELLING_ONOFF: cancelling and nothing else.
            0x61 => self.noise_cancelling = true,
            // The names carrying both NOISE_CANCELLING and AMBIENT_SOUND.
            0x62..=0x65 | 0x68 | 0x6A..=0x6D => {
                self.noise_cancelling = true;
                self.ambient_sound = true;
            }
            // AMBIENT_SOUND_MODE_ONOFF, AMBIENT_SOUND_MODE_LEVEL_ADJUSTMENT,
            // AMBIENT_SOUND_CONTROL_MODE_SELECT: ambient without cancelling in
            // the name.
            0x66 | 0x67 | 0x69 => self.ambient_sound = true,
            // AUTO_NCASM, ADAPTIVE_CONTROL_WITH_PARAMETER_NOTIFICATION:
            // Sony's Adaptive Sound Control.
            0x70 | 0x71 => self.adaptive_ambient_sound = true,
            // The playback controllers. All four carry track names --
            // `RetPlayParamPlaybackControllerName` is four `PlaybackName`
            // entries -- and all four take transport commands.
            //
            // Only the three whose names say CALL_VOLUME_ADJUSTMENT set
            // `playback_volume`. The document's `playback.volume` is MUSIC
            // volume, which no function type names at all; what settles it is
            // the capture, where a device advertising 0xA1 and nothing else in
            // this block answers `PLAY_RET_PARAM` at `MUSIC_VOLUME`.
            // PLAYBACK_CONTROLLER_WITH_FUNCTION_CHANGE (0xA4) drops the volume
            // clause from its name and has no capture, so it is left out.
            0xA1..=0xA3 => {
                self.playback_control = true;
                self.playback_metadata = true;
                self.playback_volume = true;
            }
            0xA4 => {
                self.playback_control = true;
                self.playback_metadata = true;
            }
            // GENERAL_SETTING_1..4. The device answers one
            // GENERAL_SETTING_RET_CAPABILITY per advertised slot.
            0xD1..=0xD4 => self.general_settings = true,
            // CONNECTION_MODE_SOUND_QUALITY_CONNECTION_QUALITY and its LDAC
            // twin. NOT 0xE7 -- see the note above.
            0xE1 | 0xE3 => self.connection_mode = true,
            // UPSCALING_AUTO_OFF, UPSCALING_AUTO_OFF_WITH_STATUS_DISABLE_REASON:
            // DSEE. Not 0x13 UPSCALING_INDICATOR, which is a display hint for
            // Sony's app and carries no setting.
            0xE2 | 0xED => self.dsee = true,
            // LISTENING_OPTION, LISTENING_OPTION_ASSIGN_CUSTOMIZABLE. The
            // BGM_MODE functions (0xE4, 0xEB) are a separate control with a
            // separate name and no field in this document.
            0xE6 | 0xEA => self.listening_mode = true,
            // PLAYBACK_CONTROL_BY_WEARING_REMOVING_HEADPHONE_ON_OFF.
            0xF1 => self.auto_pause = true,
            // SMART_TALKING_MODE_TYPE1, SMART_TALKING_MODE_TYPE2:
            // Speak-to-Chat.
            0xF2 | 0xFC => self.speak_to_chat = true,
            // ASSIGNABLE_SETTING, ASSIGNABLE_SETTING_WITH_LIMITATION.
            0xF3 | 0xFE => self.assignable_controls = true,
            // WEARING_STATUS_DETECTOR.
            0xF6 => self.wearing_detection = true,
            // HEAD_GESTURE_ON_OFF_TRAINING. 0xFF is a function code here and
            // not a terminator; see `payload::NO_PRIORITY`.
            0xFF => self.head_gesture = true,
            _ => {}
        }
    }

    /// `mdr::v2::t2::FunctionType`, in the enum's own order.
    fn apply_table_two(&mut self, code: u8) {
        match code {
            // PAIRING_DEVICE_MANAGEMENT_CLASSIC_BT and the two
            // WITH_BLUETOOTH_CLASS_OF_DEVICE forms.
            //
            // These set `pairing_mode` as well, and that is not a guess about a
            // second feature: `PeripheralSetStatusParingDeviceManagementCommon`
            // -- the set message these functions enable -- carries a
            // `PeripheralBluetoothMode` whose two values are NORMAL_MODE and
            // INQUIRY_SCAN_MODE. Putting the headset into pairing mode IS
            // writing that field. 0x31 SOURCE_SWITCH_CONTROL is a different
            // feature on the same command family and sets neither.
            0x30 | 0x32 | 0x33 => {
                self.paired_device_management = true;
                self.pairing_mode = true;
            }
            // The VOICE_GUIDANCE_SETTING_* forms that do not mention volume.
            0x40 | 0x41 | 0x44 | 0x45 => self.voice_guidance = true,
            // ..._SUPPORT_LANGUAGE_SWITCH_AND_VOLUME_ADJUSTMENT and
            // VOICE_GUIDANCE_VOLUME_SETTING_MTK_FIXED_TO_5_STEPS.
            0x42 | 0x43 => {
                self.voice_guidance = true;
                self.voice_guidance_volume = true;
            }
            // SAFE_LISTENING_{HBS,TWS}_{1,2} and SAFE_VOLUME_CONTROL. 0x55
            // MAX_VOLUME_LEVEL_LIMIT is a volume cap rather than the safe
            // listening report, and has no field here.
            0x50..=0x54 => self.safe_listening = true,
            // WEARING_STATUS_CHECKER.
            0xF0 => self.wearing_detection = true,
            _ => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// The BlueZ alias, then its name -- never a Bluetooth address. An address
    /// is not a name and putting one on a bar looks like a fault.
    pub name: String,
    pub model: String,
    pub firmware: String,
    /// The MDR generation: 2 for the v2 service UUID, 1 for v1, 0 for neither.
    /// An integer, not a string.
    ///
    /// The generation of the service the daemon's SDP query was answered on:
    /// `link::MDR_SERVICE_V2` is 2 and `link::MDR_SERVICE_V1` is 1, the two
    /// UUIDs libmdr's `Base.h` names and the numbers the C++ daemon's
    /// `protocolGeneration` gives them. 0 only while nothing is connected.
    pub protocol: i32,
    pub codec: String,
}

impl Default for Device {
    fn default() -> Device {
        Device {
            name: String::new(),
            model: String::new(),
            firmware: String::new(),
            protocol: 0,
            codec: "unknown".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Noise {
    /// `off`, `anc`, `ambient` or `unknown`.
    pub mode: String,
    /// The ambient sound level as the wire carries it. NOT a percentage: a
    /// slider drawn 0..100 sends values this scale has no meaning for.
    ///
    /// THE RANGE IS NOT 1..=20 BECAUSE THE PROTOCOL SAYS SO. `sony::command`
    /// refuses anything outside 1..=20 because the C++ daemon does, and that
    /// command language is a compatibility contract. The reference itself
    /// declares NO bound: none of the 32 `ambientSoundLevelValue` fields in
    /// `ProtocolV2T1.hpp` carries a `CODEGEN Range` comment.
    ///
    /// What the reference DOES say is that the bound is per-device and
    /// per-mode. `AmbientSoundModeLevelSet` (T1:2317) pairs an
    /// `AmbientSoundMode` with a `Range { min, max, step }`, and a headset
    /// delivers those in its NCASM_RET_CAPABILITY reply -- command 0x61, which
    /// no frame in this corpus carries. So a device whose ambient scale runs
    /// to 19 or to 32 would be clamped wrongly here, and the fix is one
    /// capture of a 0x61 frame rather than a different constant.
    pub ambient_level: i32,
    pub focus_on_voice: bool,
    pub adaptive: bool,
    /// `low`, `standard`, `high` or `unknown`.
    pub adaptive_sensitivity: String,
}

impl Default for Noise {
    fn default() -> Noise {
        Noise {
            mode: "unknown".to_string(),
            ambient_level: 1,
            focus_on_voice: false,
            adaptive: false,
            adaptive_sensitivity: "unknown".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeakToChat {
    pub enabled: bool,
    /// `auto`, `low`, `high` or `unknown`.
    pub sensitivity: String,
    /// `short`, `medium`, `long`, `manual` or `unknown`.
    pub timeout: String,
}

impl Default for SpeakToChat {
    fn default() -> SpeakToChat {
        SpeakToChat {
            enabled: false,
            sensitivity: "unknown".to_string(),
            timeout: "unknown".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Equalizer {
    pub preset: String,
    /// The write gate: `eq-preset` refuses anything not in here.
    ///
    /// Upstream never populates this in its MDR backend -- only in its mock --
    /// so on real hardware the verb is unconditionally rejected. That is a bug
    /// in the reference, not a contract to preserve, and this daemon fills it
    /// from the device's own capability reply.
    pub available_presets: Vec<String>,
    pub clear_bass: i32,
    pub bands: Vec<i32>,
    pub dsee_enabled: bool,
    /// `standard`, `hx`, `hx_ai`, `ultimate` or `unknown`: the C++ daemon's
    /// `dseeName` words, from `UpscalingType`.
    pub dsee_type: String,
}

impl Default for Equalizer {
    fn default() -> Equalizer {
        Equalizer {
            preset: "unknown".to_string(),
            available_presets: Vec::new(),
            clear_bass: 0,
            bands: Vec::new(),
            dsee_enabled: false,
            dsee_type: "unknown".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playback {
    /// `stopped`, `playing`, `paused`, `fast_forward`, `rewind` or `unknown`.
    pub status: String,
    /// A percentage, 0..=100. The device scale is 0..=30; see [`super::volume`].
    pub volume: i32,
}

impl Default for Playback {
    fn default() -> Playback {
        Playback { status: "unknown".to_string(), volume: 0 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    /// `quality`, `stable` or `unknown`.
    pub priority: String,
}

impl Default for Connection {
    fn default() -> Connection {
        Connection { priority: "unknown".to_string() }
    }
}

/// Everything the daemon publishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    /// Bumped on every published change, so a reader can tell a re-read that
    /// found nothing new from one that did.
    pub revision: u64,
    /// `idle`, `connecting`, `ready` or `error`.
    pub phase: String,
    pub connected: bool,
    pub device: Device,
    pub battery: Batteries,
    pub capabilities: Capabilities,
    pub noise: Noise,
    pub speak_to_chat: SpeakToChat,
    pub equalizer: Equalizer,
    pub playback: Playback,
    pub connection: Connection,
    /// The headset's own named on/off settings, in slot order.
    ///
    /// A LIST AND NOT NAMED FIELDS, because the device decides what is in it.
    /// A WF-1000XM5 answers sidetone and multipoint; a WH-1000XM5 answers the
    /// touch panel and multipoint. `general_settings[0].key` is not a fixed
    /// thing and a reader must match on `key` rather than on position.
    pub general_settings: Vec<GeneralSetting>,
    /// Every device this headset is paired with, and which of them it is
    /// currently connected to (when multipoint links are active).
    ///
    /// THE ADDRESS IS HERE AND THE HEADSET'S OWN ADDRESS IS NOT. Addresses
    /// identify entries in this list because names are not unique — the one
    /// capture carries two `KUWABARA` and two `KOMAHUANG-PC2`. The headset's
    /// own address would be redundant in a list of peers and is not sent.
    pub paired_devices: Vec<PairedDevice>,
    /// The last thing that went wrong, or empty. A page shows this instead of
    /// looking broken when there is nothing to show.
    pub last_error: String,
}

impl Default for State {
    fn default() -> State {
        State {
            revision: 0,
            phase: "idle".to_string(),
            connected: false,
            device: Device::default(),
            battery: Batteries::default(),
            capabilities: Capabilities::default(),
            noise: Noise::default(),
            speak_to_chat: SpeakToChat::default(),
            equalizer: Equalizer::default(),
            playback: Playback::default(),
            connection: Connection::default(),
            general_settings: Vec::new(),
            paired_devices: Vec::new(),
            last_error: String::new(),
        }
    }
}

/// One of the headset's named on/off settings, as the document reports it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GeneralSetting {
    /// The device's own name, lowercased: `sidetone`, `multipoint`,
    /// `touch_panel`. LOWERCASED AND SUFFIX-TRIMMED, because the wire carries
    /// `MULTIPOINT_SETTING` and a document key should read like the rest of
    /// this document -- but the trimming is only the `_SETTING` suffix, so a
    /// name that does not carry one survives whole rather than being guessed
    /// at.
    pub key: String,
    /// What the wire actually said, untouched: `MULTIPOINT_SETTING`. Kept
    /// beside the key because the key is derived and this is evidence.
    pub name: String,
    /// The longer key beside it, e.g.
    /// `MULTIPOINT_SETTING_SUMMARY_LDAC_AVAILABLE`.
    pub summary: String,
    /// The slot, 0xD1..=0xD4, which is what a write addresses.
    pub slot: u8,
    /// Whether it is on. `None` until the param reply has arrived -- the
    /// capability reply and the param reply are separate frames.
    pub enabled: Option<bool>,
}

/// One device this headset is paired with, and whether it is connected.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PairedDevice {
    /// The Bluetooth address as the wire carries it: `E8:9C:25:5D:B8:19`.
    /// Addresses identify entries because names are not unique.
    pub address: String,
    /// What the other machine calls itself.
    pub name: String,
    /// Non-zero when this headset is currently talking to this device
    /// (on a multipoint link). `0x01` or `0x02` observed connected, `0x00`
    /// when not. The exact meaning is not settled by one capture; read it as
    /// "0 means disconnected, non-zero means connected for now".
    pub status: u8,
    /// Three bytes of unread data: possibly codec, profile mask, or signal
    /// figure. Kept to round-trip the message without guessing their names.
    pub detail: [u8; 3],
}

/// Escape a string for the document.
///
/// Deliberately the reference's rules and not serde's, because the two differ
/// and the byte-for-byte test is the point: only the seven named escapes, then
/// `\u00xx` in LOWERCASE hex for anything else under 0x20. Everything from 0x20
/// up is emitted raw, including DEL and every UTF-8 continuation byte.
///
/// Iterates CHARS, not bytes, and the difference is not cosmetic. The C++
/// writes raw bytes to a stream, but a Rust `String` must be valid UTF-8, so
/// pushing each byte as a `char` reinterprets it as Latin-1 -- a device named
/// with an emoji came out as four mojibake characters, and the document then
/// carried a name nobody set.
///
/// The two agree for every input a `String` can hold: the bytes the reference
/// escapes are all below 0x20, and in UTF-8 those only ever appear as
/// standalone ASCII, never inside a multi-byte sequence. So escaping per char
/// and passing the rest through emits exactly the bytes the C++ would.
///
/// A BlueZ name that is not valid UTF-8 cannot reach here at all -- it was
/// already replaced on the way into a `String` -- which is a difference from
/// the reference this cannot close and does not pretend to.
pub fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

fn bool_json(v: bool) -> &'static str {
    if v {
        "true"
    } else {
        "false"
    }
}

fn append_battery(out: &mut String, b: &Battery) {
    let _ = write!(
        out,
        "{{\"present\":{},\"level\":{},\"charging\":\"{}\"}}",
        bool_json(b.present),
        b.level,
        json_escape(&b.charging)
    );
}

/// The document, with its trailing newline.
///
/// Key order is `state.cpp:63-141` exactly. A test pins the whole string.
pub fn serialize(state: &State, daemon_version: &str) -> String {
    let mut out = String::with_capacity(2048);

    let _ = write!(out, "{{\"schema_version\":{SCHEMA_VERSION}");
    let _ = write!(out, ",\"daemon_version\":\"{}\"", json_escape(daemon_version));
    let _ = write!(out, ",\"revision\":{}", state.revision);
    let _ = write!(out, ",\"phase\":\"{}\"", json_escape(&state.phase));
    let _ = write!(out, ",\"connected\":{}", bool_json(state.connected));

    let _ = write!(
        out,
        ",\"device\":{{\"name\":\"{}\",\"model\":\"{}\",\"firmware\":\"{}\",\"protocol\":{},\"codec\":\"{}\"}}",
        json_escape(&state.device.name),
        json_escape(&state.device.model),
        json_escape(&state.device.firmware),
        state.device.protocol,
        json_escape(&state.device.codec)
    );

    out.push_str(",\"battery\":{\"main\":");
    append_battery(&mut out, &state.battery.main);
    out.push_str(",\"left\":");
    append_battery(&mut out, &state.battery.left);
    out.push_str(",\"right\":");
    append_battery(&mut out, &state.battery.right);
    out.push_str(",\"case\":");
    append_battery(&mut out, &state.battery.case);

    out.push_str("},\"capabilities\":{");
    for (i, (name, value)) in state.capabilities.entries().into_iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "\"{name}\":{}", bool_json(value));
    }

    let _ = write!(
        out,
        "}},\"noise\":{{\"mode\":\"{}\",\"ambient_level\":{},\"focus_on_voice\":{},\"adaptive\":{},\"adaptive_sensitivity\":\"{}\"}}",
        json_escape(&state.noise.mode),
        state.noise.ambient_level,
        bool_json(state.noise.focus_on_voice),
        bool_json(state.noise.adaptive),
        json_escape(&state.noise.adaptive_sensitivity)
    );

    let _ = write!(
        out,
        ",\"speak_to_chat\":{{\"enabled\":{},\"sensitivity\":\"{}\",\"timeout\":\"{}\"}}",
        bool_json(state.speak_to_chat.enabled),
        json_escape(&state.speak_to_chat.sensitivity),
        json_escape(&state.speak_to_chat.timeout)
    );

    let _ = write!(out, ",\"equalizer\":{{\"preset\":\"{}\"", json_escape(&state.equalizer.preset));
    out.push_str(",\"available_presets\":[");
    for (i, p) in state.equalizer.available_presets.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "\"{}\"", json_escape(p));
    }
    let _ = write!(out, "],\"clear_bass\":{},\"bands\":[", state.equalizer.clear_bass);
    for (i, b) in state.equalizer.bands.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "{b}");
    }
    let _ = write!(
        out,
        "],\"dsee_enabled\":{},\"dsee_type\":\"{}\"}}",
        bool_json(state.equalizer.dsee_enabled),
        json_escape(&state.equalizer.dsee_type)
    );

    let _ = write!(
        out,
        ",\"playback\":{{\"status\":\"{}\",\"volume\":{}}}",
        json_escape(&state.playback.status),
        state.playback.volume
    );
    let _ = write!(
        out,
        ",\"connection\":{{\"priority\":\"{}\"}}",
        json_escape(&state.connection.priority)
    );
    // APPENDED AFTER `connection` AND BEFORE `last_error`, not at the end.
    //
    // The C++ daemon's `serializeState` closes with `last_error`, and
    // `sony_contract.rs`'s key-order assertions hold this document to the same
    // shape -- a reader that walks the keys in order sees the reference's
    // order with one new key inserted, rather than `last_error` moving. New
    // keys go before it, which is the rule the equaliser's `kev`-style
    // additions follow elsewhere in this tree.
    let _ = write!(out, ",\"general_settings\":[");
    for (i, g) in state.general_settings.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"key\":\"{}\",\"name\":\"{}\",\"summary\":\"{}\",\"slot\":{},\"enabled\":{}}}",
            json_escape(&g.key),
            json_escape(&g.name),
            json_escape(&g.summary),
            g.slot,
            // `null` until the param reply lands, because "we have not been
            // told" and "it is off" are different and the page draws them
            // differently.
            match g.enabled {
                Some(true) => "true",
                Some(false) => "false",
                None => "null",
            }
        );
    }
    out.push(']');
    let _ = write!(out, ",\"paired_devices\":[");
    for (i, d) in state.paired_devices.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"address\":\"{}\",\"name\":\"{}\",\"status\":{},\"detail\":[{},{},{}]}}",
            json_escape(&d.address),
            json_escape(&d.name),
            d.status,
            d.detail[0],
            d.detail[1],
            d.detail[2],
        );
    }
    out.push(']');
    let _ = write!(out, ",\"last_error\":\"{}\"}}", json_escape(&state.last_error));
    // The trailing newline is part of the document: the file ends in exactly
    // this one, the `"}\n"` the reference's `serializeState` (state.cpp)
    // closes with -- its `atomicWrite` adds nothing. The socket's
    // `status` reply is `ok ` plus this string with the newline trimmed,
    // because `serve` terminates every reply line itself.
    out.push('\n');

    out
}
