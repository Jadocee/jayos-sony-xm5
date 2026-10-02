//! The command line the daemon accepts, and the volume scale it speaks.
//!
//! One command per connection, at most 4096 bytes, answered with `ok`,
//! `ok <message>` or `error <message>` and a newline. There is no handshake and
//! no version on the wire: the only version a client can observe is
//! `schema_version` inside the state document.
//!
//! THE ERROR STRINGS ARE PART OF THE INTERFACE
//!
//! They are what a person sees when a command is refused, so they are
//! reproduced verbatim from `command.cpp:28-73` rather than reworded. A client
//! written against the C++ daemon must get the same answer from this one.
//!
//! WHAT IS DELIBERATELY NOT HERE
//!
//! No quoting, no escaping, no shell. The line is split on ASCII whitespace and
//! every argument is matched against a closed set or parsed as an integer, so
//! there is no value a caller can send that becomes anything but a word. That
//! is the whole input surface of a daemon holding a Bluetooth socket, and it is
//! deliberately this small.

/// The device's own volume scale. A percentage is what the document reports.
pub const MDR_VOLUME_MAX: i32 = 30;
pub const PERCENT_MAX: i32 = 100;

/// Device scale to percentage, rounding to nearest.
///
/// `+ MDR_VOLUME_MAX / 2` before the divide is the reference's rounding, and it
/// is integer arithmetic in both: 15 of 30 is 50%, not 49%.
pub fn mdr_volume_to_percent(level: i32) -> i32 {
    let clamped = level.clamp(0, MDR_VOLUME_MAX);
    (clamped * PERCENT_MAX + MDR_VOLUME_MAX / 2) / MDR_VOLUME_MAX
}

/// Percentage to device scale, rounding to nearest.
pub fn percent_to_mdr_volume(percent: i32) -> i32 {
    let clamped = percent.clamp(0, PERCENT_MAX);
    (clamped * MDR_VOLUME_MAX + PERCENT_MAX / 2) / PERCENT_MAX
}

/// The percentage a requested percentage actually becomes.
///
/// The device has 31 steps, so most percentages are not reachable: asking for
/// 51% sets the same step as 50% and the document then reports 50%. A caller
/// that echoes what it asked for rather than what this returns shows a value
/// the headset is not at.
pub fn canonical_volume_percent(percent: i32) -> i32 {
    mdr_volume_to_percent(percent_to_mdr_volume(percent))
}

/// Every equalizer preset the protocol names, in wire order.
///
/// Also the `eq-preset` allowlist. One table, because the reference keeps two
/// -- a serialisation table and a parser allowlist -- and they can drift.
pub const EQ_PRESETS: [&str; 30] = [
    "off",
    "rock",
    "pop",
    "jazz",
    "dance",
    "edm",
    "r_and_b_hip_hop",
    "acoustic",
    "bright",
    "excited",
    "mellow",
    "relaxed",
    "vocal",
    "treble",
    "bass",
    "speech",
    "heavy",
    "clear",
    "hard",
    "soft",
    "gaming",
    "fps_1",
    "fps_2",
    "fps_3",
    "custom",
    "user_1",
    "user_2",
    "user_3",
    "user_4",
    "user_5",
];

/// What a well-formed command line asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Status,
    PanelOpen,
    PanelClose,
    Refresh,
    /// `off`, `anc` or `ambient`.
    Noise(String),
    /// 1..=20. Not a percentage.
    AmbientLevel(i32),
    FocusOnVoice(bool),
    SpeakToChat(bool),
    Dsee(bool),
    EqualizerPreset(String),
    /// One equaliser band: a 1-based index and a signed step.
    EqBand { index: i32, value: i32 },
    /// The clear-bass shelf: a signed step.
    ClearBass(i32),
    /// One of the headset's OWN named on/off settings, by the name it gave.
    GeneralSetting { key: String, on: bool },
    /// `quality` or `stable`.
    ConnectionPriority(String),
    /// 0..=100.
    Volume(i32),
    /// `play`, `pause`, `next` or `previous`.
    Playback(String),
}

/// The largest command line accepted, per the spec.
pub const MAX_COMMAND: usize = 4096;

/// Parse one line.
///
/// `Err` carries the message that goes back as `error <message>`.
pub fn parse_line(line: &str) -> Result<Command, String> {
    if line.len() > MAX_COMMAND {
        return Err(format!("command exceeds {MAX_COMMAND} bytes"));
    }
    // Split on any run of ASCII whitespace, discarding empties -- which is what
    // `istringstream >> word` does, and which is why the trailing newline needs
    // no special handling.
    let words: Vec<&str> = line.split_ascii_whitespace().collect();
    parse_words(&words)
}

fn on_off(verb: &str, word: &str) -> Result<bool, String> {
    match word {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(format!("{verb} must be on or off")),
    }
}

/// Parse an already-split command.
///
/// Every verb is exact-arity: `noise` with no argument and `noise anc extra`
/// are both "invalid command or argument count" rather than one of them being
/// quietly accepted.
pub fn parse_words(words: &[&str]) -> Result<Command, String> {
    let Some(verb) = words.first() else {
        return Err("no command supplied".to_string());
    };
    let n = words.len();
    let arg = words.get(1).copied().unwrap_or_default();

    match (*verb, n) {
        ("status", 1) => Ok(Command::Status),
        ("panel-open", 1) => Ok(Command::PanelOpen),
        ("panel-close", 1) => Ok(Command::PanelClose),
        ("refresh", 1) => Ok(Command::Refresh),

        ("noise", 2) => match arg {
            "off" | "anc" | "ambient" => Ok(Command::Noise(arg.to_string())),
            _ => Err("noise must be off, anc, or ambient".to_string()),
        },

        // 1..=20 is the C++ daemon's bound, reproduced because this command
        // language is a compatibility contract -- NOT a fact about the
        // hardware. `ProtocolV2T1.hpp` declares no range on any of its 32
        // `ambientSoundLevelValue` fields; the real bound is per-device and
        // per-mode, delivered as a `Range { min, max, step }` in a
        // NCASM_RET_CAPABILITY reply. The daemon asks for that reply and
        // holds a write to it (`mdr::encode::encode_ncasm`), which can only
        // NARROW this: widening the grammar would change what `ambient-level
        // 21` answers, and that answer is part of the contract too
        // (`sony_contract.rs`). The document publishes the range in force as
        // `ambient_range`.
        ("ambient-level", 2) => match arg.parse::<i32>() {
            Ok(v) if (1..=20).contains(&v) => Ok(Command::AmbientLevel(v)),
            _ => Err("ambient-level must be an integer from 1 to 20".to_string()),
        },

        ("focus-on-voice", 2) => on_off(verb, arg).map(Command::FocusOnVoice),
        ("speak-to-chat", 2) => on_off(verb, arg).map(Command::SpeakToChat),
        ("dsee", 2) => on_off(verb, arg).map(Command::Dsee),

        // THE NEXT TWO ARE DEAD ENDS, BY DIFFERENT ROUTES, and an earlier
        // draft of this comment ran them together and was wrong about both.
        //
        // `eq-preset` goes all the way: it parses here, `encode.rs` builds a
        // well-formed SET, the headset ACKs it inside 25 ms -- and a fresh
        // session then reads the preset it had before. An ACK means the frame
        // ARRIVED, not that it was honoured, and no field anywhere in the
        // protocol distinguishes the two.
        //
        // `connection` never reaches the wire on the captured device family.
        // It parses here, and `encode`'s `encode_connection` refuses it for a
        // headset whose only connection function is the LE Audio form (t1
        // 0xE7) -- named rather than given a line number, because the line
        // moves -- because that SET's fourth byte is the `value2` the
        // reference never explains, and we do not guess at bytes. A headset
        // that answers at one of the two three-byte forms (0x02, 0x00) is
        // written in that form since plan 23; neither has been seen to stick.
        //
        // Both stay accepted at this layer deliberately: the error strings on
        // this socket are a contract with the C++ daemon (see the module
        // header), and refusing a command it accepts would break a client
        // written against either. For `eq-preset` that also keeps the one test
        // that could overturn the finding reachable:
        //
        //     qs-sony send eq-preset rock
        //     sony-daemon.sh off && sony-daemon.sh on   # a fresh session
        //     qs-sony status                            # did it keep it?
        //
        // (`sony-daemon.sh` is jayos's; there is no systemd unit for
        // qs-sonyd, whatever this example used to say.)
        //
        // What changed is what we CLAIM: jayos's HeadphonesPage.qml shows the preset
        // the headset reports and no longer draws buttons that would report
        // success and do nothing.
        //
        // NOT OUR FINDING. Nothing in this tree has spoken to a headset. It is
        // from delarosa1312/omarchy-sony-xm5 (MIT, Copyright (c) 2026
        // Delarosa1312), docs/protocol.md, which reports the same for the
        // auto-power-off timer -- a verb this daemon does not have, so there is
        // nothing here to change for it.
        ("eq-preset", 2) => {
            if EQ_PRESETS.contains(&arg) {
                Ok(Command::EqualizerPreset(arg.to_string()))
            } else {
                Err("unknown equalizer preset".to_string())
            }
        }

        // THE TWO THAT ACTUALLY TAKE. Everything the paragraph above says about
        // `eq-preset` is the reason these exist: the same device family that
        // acknowledges and discards a preset write keeps a band and clear-bass
        // write across a reconnect. Same source, same test, same document --
        // delarosa1312/omarchy-sony-xm5, docs/protocol.md, "Equalizer: bands
        // and clear bass are one message".
        //
        // THE BOUNDS HERE ARE THE PROTOCOL'S WIDEST, NOT THE DEVICE'S, and the
        // split is deliberate in the other direction from `ambient-level`
        // above. There the 1..=20 is a compatibility contract with the C++
        // daemon and no fact about hardware, so the encoder does not repeat it.
        // Here the real bound is narrower and the headset is the one that knows
        // it -- five bands run -10..=+10 and ten run -6..=+6, decided by the
        // length of the array the device last sent. That length is not
        // knowable at parse time, so this layer refuses only what NO shape
        // could accept and `mdr::encode::encode_eq_steps` refuses the rest with
        // the device's own numbers in the message.
        ("eq-band", 3) => {
            let index = words[1].parse::<i32>();
            let value = words[2].parse::<i32>();
            match (index, value) {
                (Ok(index), Ok(value)) if (1..=10).contains(&index) && (-10..=10).contains(&value) => {
                    Ok(Command::EqBand { index, value })
                }
                _ => Err("eq-band takes a band from 1 to 10 and a step from -10 to 10".to_string()),
            }
        }

        ("clear-bass", 2) => match arg.parse::<i32>() {
            Ok(v) if (-10..=10).contains(&v) => Ok(Command::ClearBass(v)),
            _ => Err("clear-bass must be an integer from -10 to 10".to_string()),
        },

        // THE DEVICE NAMES THESE, SO THERE IS NO ALLOWLIST HERE. Every other
        // word-taking verb in this grammar matches a closed set, and this one
        // cannot: the headset says what its settings are called, and the pair
        // differs by model -- a WF-1000XM5 answers sidetone and multipoint, a
        // WH-1000XM5 the touch panel and multipoint
        // (delarosa1312/omarchy-sony-xm5, docs/protocol.md). A table here
        // would be right on one ear and silently wrong on the other.
        //
        // SO THE BOUND IS A SHAPE, NOT A LIST. The argument is accepted only
        // if it looks like a key this daemon itself produces -- lowercase
        // ASCII, digits and underscores, and short -- which keeps the module
        // header's promise that no value a caller sends becomes anything but a
        // word. `mdr::encode` then refuses anything the headset has not
        // actually named, with `NotRead`, so an unknown setting is reported as
        // "this headset has not told us about that" rather than as a parse
        // error about a word that might be perfectly valid on another model.
        ("general", 3) => {
            let key = words[1];
            let fits = !key.is_empty()
                && key.len() <= 64
                && key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
            match (fits, on_off(verb, words[2])) {
                (true, Ok(on)) => Ok(Command::GeneralSetting { key: key.to_string(), on }),
                (true, Err(e)) => Err(e),
                (false, _) => Err("general takes a setting name and on or off".to_string()),
            }
        }

        ("connection", 2) => match arg {
            "quality" | "stable" => Ok(Command::ConnectionPriority(arg.to_string())),
            _ => Err("connection must be quality or stable".to_string()),
        },

        ("volume", 2) => match arg.parse::<i32>() {
            Ok(v) if (0..=100).contains(&v) => Ok(Command::Volume(v)),
            _ => Err("volume must be an integer from 0 to 100".to_string()),
        },

        ("playback", 2) => match arg {
            "play" | "pause" | "next" | "previous" => Ok(Command::Playback(arg.to_string())),
            _ => Err("playback must be play, pause, next, or previous".to_string()),
        },

        _ => Err("invalid command or argument count".to_string()),
    }
}

/// One form of the command language, and lines that are instances of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Form {
    /// The form as [`usage`] writes it, e.g. `noise off|anc|ambient`.
    pub form: &'static str,
    /// Lines a client may send, every one of which [`parse_words`] accepts.
    pub examples: &'static [&'static str],
}

/// The command language as data, for the contract.
///
/// `contract/commands.txt` is generated from this, and jayos holds its
/// Headphones page to that file: every verb the page sends must be a verb
/// here. A list a test could not run would be a second prose description of
/// the grammar, free to drift from the parser -- so `tests/contract.rs` sends
/// every example through [`parse_words`], requires every [`Command`] variant to
/// be reached by one, and requires [`usage`] to be exactly these forms. The
/// first catches a form the parser no longer accepts, the second a verb added
/// to the parser and not here, the third a help text that says otherwise.
pub const GRAMMAR: &[Form] = &[
    Form { form: "status", examples: &["status"] },
    Form { form: "panel-open", examples: &["panel-open"] },
    Form { form: "panel-close", examples: &["panel-close"] },
    Form { form: "refresh", examples: &["refresh"] },
    Form { form: "noise off|anc|ambient", examples: &["noise off", "noise anc", "noise ambient"] },
    Form { form: "ambient-level 1..20", examples: &["ambient-level 1", "ambient-level 20"] },
    Form { form: "focus-on-voice on|off", examples: &["focus-on-voice on", "focus-on-voice off"] },
    Form { form: "speak-to-chat on|off", examples: &["speak-to-chat on", "speak-to-chat off"] },
    Form { form: "dsee on|off", examples: &["dsee on", "dsee off"] },
    Form { form: "eq-preset <preset>", examples: &["eq-preset off", "eq-preset bright"] },
    Form { form: "eq-band <1..10> <-10..10>", examples: &["eq-band 1 -10", "eq-band 10 10"] },
    Form { form: "clear-bass <-10..10>", examples: &["clear-bass -10", "clear-bass 10"] },
    Form { form: "general <setting> on|off", examples: &["general multipoint on", "general touch_panel off"] },
    Form { form: "connection quality|stable", examples: &["connection quality", "connection stable"] },
    Form { form: "volume 0..100", examples: &["volume 0", "volume 100"] },
    Form {
        form: "playback play|pause|next|previous",
        examples: &["playback play", "playback pause", "playback next", "playback previous"],
    },
];

/// The verb list, for `--help` and for a rejected command.
pub fn usage() -> &'static str {
    concat!(
        "commands:\n",
        "  status\n  panel-open\n  panel-close\n  refresh\n",
        "  noise off|anc|ambient\n  ambient-level 1..20\n",
        "  focus-on-voice on|off\n  speak-to-chat on|off\n  dsee on|off\n",
        "  eq-preset <preset>\n  eq-band <1..10> <-10..10>\n  clear-bass <-10..10>\n",
        "  general <setting> on|off\n",
        "  connection quality|stable\n",
        "  volume 0..100\n  playback play|pause|next|previous\n",
    )
}
