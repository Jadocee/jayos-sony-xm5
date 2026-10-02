//! The daemon's public contract: the state document and the command language.
//!
//! Both are checked against the C++ implementation this replaces, because the
//! point of matching it is interoperability. `qs-sony` carries a schema-version
//! guard so that a reader works against either daemon; a document that differs
//! in key order, in a default, or in one escape rule defeats that guard while
//! still looking like valid JSON.
//!
//! The reference for the document is `serializeState` at
//! `native/src/state.cpp:63-141`, and the fixture below is its byte-exact
//! output for the state its own mock backend builds
//! (`native/src/mock_backend.cpp:10-38`) -- a WH-1000XM5, which is the device
//! this tree targets.

use sony_xm5::sony::command::{self, Command};
use sony_xm5::sony::session::{Input, Limits, Peer, Session};
use sony_xm5::sony::state::{self, Battery, State};

/// The reference's mock state, field for field.
fn mock_state() -> State {
    let mut s = State {
        revision: 1,
        phase: "ready".to_string(),
        connected: true,
        ..State::default()
    };
    s.device = state::Device {
        name: "Vyom's WH-1000XM5".to_string(),
        model: "WH-1000XM5".to_string(),
        firmware: "mock-2.3.1".to_string(),
        protocol: 2,
        codec: "ldac".to_string(),
    };
    // A WH-1000XM5 reports ONE battery. The other three stay absent, which is
    // present:false / level:-1 / charging:"unknown" -- and is exactly the case
    // the reader in this tree got wrong by treating -1 as a charge level.
    s.battery.main = Battery { present: true, level: 73, charging: "no".to_string() };

    let c = &mut s.capabilities;
    c.battery_single = true;
    c.playback_metadata = true;
    c.playback_control = true;
    c.playback_volume = true;
    c.noise_cancelling = true;
    c.ambient_sound = true;
    c.adaptive_ambient_sound = true;
    c.speak_to_chat = true;
    c.equalizer = true;
    c.dsee = true;
    c.connection_mode = true;
    c.auto_power_off = true;
    c.wearing_detection = true;
    c.auto_pause = true;

    s.noise = state::Noise {
        mode: "anc".to_string(),
        ambient_level: 10,
        focus_on_voice: false,
        adaptive: false,
        adaptive_sensitivity: "standard".to_string(),
    };
    s.speak_to_chat = state::SpeakToChat {
        enabled: false,
        sensitivity: "auto".to_string(),
        timeout: "medium".to_string(),
    };
    s.equalizer = state::Equalizer {
        preset: "bright".to_string(),
        available_presets: ["off", "bright", "bass", "treble", "speech"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        clear_bass: 2,
        bands: vec![-2, -1, 0, 1, 2],
        dsee_enabled: true,
        dsee_type: "ultimate".to_string(),
    };
    s.playback = state::Playback { status: "playing".to_string(), volume: 42 };
    s.connection = state::Connection { priority: "quality".to_string() };
    s
}

/// What `serializeState` produces for that state, character for character.
///
/// Written out in full rather than assembled, so that a change to the order or
/// to any default shows up here as a diff of the document a reader will
/// actually parse -- not as a passing test over a builder that changed with the
/// code it is checking.
const REFERENCE_DOCUMENT: &str = concat!(
    r#"{"schema_version":1,"daemon_version":"0.2.3","revision":1,"phase":"ready","connected":true"#,
    r#","device":{"name":"Vyom's WH-1000XM5","model":"WH-1000XM5","firmware":"mock-2.3.1","protocol":2,"codec":"ldac"}"#,
    r#","battery":{"main":{"present":true,"level":73,"charging":"no"}"#,
    r#","left":{"present":false,"level":-1,"charging":"unknown"}"#,
    r#","right":{"present":false,"level":-1,"charging":"unknown"}"#,
    r#","case":{"present":false,"level":-1,"charging":"unknown"}}"#,
    r#","capabilities":{"battery_single":true,"battery_left_right":false,"battery_case":false"#,
    r#","playback_metadata":true,"playback_control":true,"playback_volume":true"#,
    r#","noise_cancelling":true,"ambient_sound":true,"adaptive_ambient_sound":true"#,
    r#","speak_to_chat":true,"listening_mode":false,"equalizer":true,"dsee":true"#,
    r#","paired_device_management":false,"pairing_mode":false,"general_settings":false"#,
    r#","assignable_controls":false,"noise_control_button":false,"auto_power_off":true"#,
    r#","wearing_detection":true,"auto_pause":true,"head_gesture":false"#,
    r#","voice_guidance":false,"voice_guidance_volume":false,"shutdown":false"#,
    r#","connection_mode":true,"safe_listening":false}"#,
    r#","noise":{"mode":"anc","ambient_level":10,"focus_on_voice":false,"adaptive":false,"adaptive_sensitivity":"standard"}"#,
    r#","speak_to_chat":{"enabled":false,"sensitivity":"auto","timeout":"medium"}"#,
    r#","equalizer":{"preset":"bright","available_presets":["off","bright","bass","treble","speech"]"#,
    r#","clear_bass":2,"bands":[-2,-1,0,1,2],"dsee_enabled":true,"dsee_type":"ultimate"}"#,
    r#","playback":{"status":"playing","volume":42}"#,
    r#","connection":{"priority":"quality"}"#,
    r#","last_error":""}"#,
    "\n",
);

/// What this daemon adds, and the reference does not have.
///
/// ONE CONST, so the two assertions below can put it in and take it out. The
/// C++ daemon has no `GENERAL_SETTING_*` decoder at all, so there is no
/// reference output to match for this key -- it is ours, and being able to
/// subtract it is what keeps the byte-for-byte claim about everything else
/// honest.
const OUR_ADDITIONS: &str = concat!(
    r#","general_settings":[],"paired_devices":[]"#,
    // Plan 23, item 2: the range the headset stated for its ambient level,
    // or the C++ daemon's 1..=20 -- which is what a document with no
    // NCASM_RET_CAPABILITY behind it says, as this one has none.
    r#","ambient_range":{"min":1,"max":20,"step":1,"source":"compat"}"#,
    // Plan 23, item 10: whether DSEE can run now. `null` until the headset's
    // AUDIO_RET_STATUS at UPSCALING says; the mock state has not been told.
    r#","dsee_available":null"#,
    // Plan 23, item 11: table 2's voice guidance, safe listening and pairing
    // mode, every one `null` until the headset has said.
    r#","voice_guidance":{"enabled":null,"volume":null,"language":null}"#,
    r#","safe_listening":{"level":null,"cause":null},"pairing_mode":null"#,
);

#[test]
fn the_document_is_the_c_implementations_plus_the_keys_this_daemon_adds() {
    // "0.2.3" is passed explicitly: this daemon reports its own version in that
    // field, and the comparison here is about the SHAPE, not about pretending
    // to be the other implementation.
    //
    // THE NAME USED TO SAY "byte for byte" AND THE DOCUMENT IS NOW A SUPERSET.
    // That is a real change and it gets a real sentence rather than an updated
    // string: a reader written against the reference still reads this one,
    // because an unknown key is one a JSON parser skips -- but a reader doing
    // a byte comparison does not, and this test is the place that difference
    // is recorded. `schema_version` deliberately does NOT move for it: 1 is
    // shared with the C++ daemon and `qs-sony`'s version guard would then
    // reject that daemon's documents, which breaks interop in the direction
    // that actually matters.
    let got = state::serialize(&mock_state(), "0.2.3");

    // What we emit, in full. Fails if anything moves, including our own key.
    let want = REFERENCE_DOCUMENT
        .replace(r#","last_error":""}"#, &format!(r#"{OUR_ADDITIONS},"last_error":""}}"#));
    assert_eq!(got, want);

    // AND THE ORIGINAL PROPERTY, still checkable: take our additions back out
    // and what is left is the reference's document, character for character.
    // Without this, the assertion above degrades into "whatever the serialiser
    // currently produces" the moment somebody updates the string to make a
    // failure go away.
    assert_eq!(
        got.replace(OUR_ADDITIONS, ""),
        REFERENCE_DOCUMENT,
        "the document diverged from the reference somewhere other than our own keys"
    );
}

#[test]
fn it_is_one_line_of_valid_json_ending_in_a_newline() {
    let doc = state::serialize(&mock_state(), "0.2.3");
    assert!(doc.ends_with('\n'), "must end with a newline");
    assert_eq!(doc.matches('\n').count(), 1, "exactly one newline, at the end");
    // And it parses -- the hand-rolled serialiser is the risk this covers.
    let v: serde_json::Value = serde_json::from_str(doc.trim_end())
        .unwrap_or_else(|e| panic!("the document is not valid JSON: {e}"));
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["device"]["model"], "WH-1000XM5");
    // The nesting the reader in this tree got wrong: name and model live under
    // `device`, and there is nothing at the top level to find.
    assert!(v.get("name").is_none(), "there is no top-level name");
    assert!(v.get("model").is_none(), "there is no top-level model");
    // And a battery is an object, not a number.
    assert!(v["battery"]["main"].is_object(), "a battery is an object");
    assert_eq!(v["battery"]["case"]["present"], false);
    assert_eq!(v["battery"]["case"]["level"], -1);
}

#[test]
fn the_default_document_is_a_daemon_with_nothing_connected() {
    let doc = state::serialize(&State::default(), "qs-1.0.0");
    let v: serde_json::Value = serde_json::from_str(doc.trim_end()).expect("valid JSON");
    assert_eq!(v["connected"], false);
    assert_eq!(v["phase"], "idle");
    assert_eq!(v["revision"], 0);
    assert_eq!(v["device"]["protocol"], 0);
    // Every capability false, and all 27 present: a reader must be able to tell
    // "unsupported" from "this daemon does not know about it".
    let caps = v["capabilities"].as_object().expect("capabilities is an object");
    assert_eq!(caps.len(), 27);
    assert!(caps.values().all(|c| c == false), "nothing is supported until a device says so");
}

#[test]
fn the_generation_of_the_service_that_answered_reaches_the_document() {
    // The reference document above carries `"protocol":2` for a WH-1000XM5:
    // the C++ daemon writes `protocolGeneration()` of the service it
    // connected through, 2 for the XM5 UUID. The engine writes what the
    // dialler hands it, so a dialler that still wrote 0 for a found headset
    // is caught here through the same path.
    let mut session = Session::new("qs-test".to_string(), Limits::default());
    session.step(Input::LinkUp {
        now: 0,
        peer: Peer { name: "WH-1000XM5".to_string(), protocol: 2 },
    });
    let v: serde_json::Value = serde_json::from_str(&session.document()).expect("valid JSON");
    assert_eq!(v["device"]["protocol"], 2);
    assert!(session.document().contains(r#""protocol":2,"#), "the byte shape the reference uses");

    // And 0 is what a daemon with nothing connected says, not what a found
    // headset says.
    let idle = Session::new("qs-test".to_string(), Limits::default());
    let v: serde_json::Value = serde_json::from_str(&idle.document()).expect("valid JSON");
    assert_eq!(v["device"]["protocol"], 0);
}

#[test]
fn escaping_follows_the_reference_rather_than_serde() {
    // The two differ, and the difference is visible in the document. The
    // reference names seven escapes and writes \u00xx in LOWERCASE for anything
    // else below 0x20; from 0x20 up everything is raw, DEL included.
    assert_eq!(state::json_escape("a\"b\\c"), "a\\\"b\\\\c");
    assert_eq!(state::json_escape("\u{8}\u{c}\n\r\t"), "\\b\\f\\n\\r\\t");
    assert_eq!(state::json_escape("\u{1}\u{1f}"), "\\u0001\\u001f");
    assert_eq!(state::json_escape("\u{7f}"), "\u{7f}", "DEL is not escaped");
    // A name is whatever the peripheral advertised. An emoji survives as its
    // UTF-8 bytes rather than becoming a surrogate pair escape.
    assert_eq!(state::json_escape("Jay's 🎧"), "Jay's 🎧");

    // And the whole document still parses with one in it.
    let mut s = mock_state();
    s.device.name = "Line\nbreak \"quoted\" \u{1}".to_string();
    let doc = state::serialize(&s, "x");
    let v: serde_json::Value = serde_json::from_str(doc.trim_end())
        .unwrap_or_else(|e| panic!("a hostile device name broke the document: {e}"));
    assert_eq!(v["device"]["name"], "Line\nbreak \"quoted\" \u{1}");
}

// ── the command language ─────────────────────────────────────────────────────

#[test]
fn every_verb_parses_at_its_exact_arity() {
    assert_eq!(command::parse_line("status"), Ok(Command::Status));
    assert_eq!(command::parse_line("panel-open"), Ok(Command::PanelOpen));
    assert_eq!(command::parse_line("panel-close"), Ok(Command::PanelClose));
    assert_eq!(command::parse_line("refresh"), Ok(Command::Refresh));
    assert_eq!(command::parse_line("noise anc"), Ok(Command::Noise("anc".into())));
    assert_eq!(command::parse_line("ambient-level 20"), Ok(Command::AmbientLevel(20)));
    assert_eq!(command::parse_line("focus-on-voice on"), Ok(Command::FocusOnVoice(true)));
    assert_eq!(command::parse_line("speak-to-chat off"), Ok(Command::SpeakToChat(false)));
    assert_eq!(command::parse_line("dsee on"), Ok(Command::Dsee(true)));
    assert_eq!(command::parse_line("eq-preset bright"), Ok(Command::EqualizerPreset("bright".into())));
    // THE TWO VERBS THE C++ DAEMON DOES NOT HAVE. Extending the language is
    // allowed and narrowing it is not: the contract is that a client written
    // against the reference gets the same answer here, and a verb it never
    // sends cannot break that. Both take signed arguments, which no other verb
    // in this grammar does, so the sign is pinned in both directions.
    assert_eq!(command::parse_line("eq-band 3 -4"), Ok(Command::EqBand { index: 3, value: -4 }));
    assert_eq!(command::parse_line("eq-band 1 10"), Ok(Command::EqBand { index: 1, value: 10 }));
    assert_eq!(command::parse_line("clear-bass -10"), Ok(Command::ClearBass(-10)));
    assert_eq!(command::parse_line("clear-bass 0"), Ok(Command::ClearBass(0)));
    // The only verb whose word argument is NOT matched against a closed set,
    // because the headset names its own settings. The shape is bounded here
    // and the substance in `mdr::encode`.
    assert_eq!(
        command::parse_line("general multipoint off"),
        Ok(Command::GeneralSetting { key: "multipoint".into(), on: false })
    );
    assert_eq!(
        command::parse_line("general touch_panel on"),
        Ok(Command::GeneralSetting { key: "touch_panel".into(), on: true })
    );
    assert_eq!(command::parse_line("connection stable"), Ok(Command::ConnectionPriority("stable".into())));
    assert_eq!(command::parse_line("volume 0"), Ok(Command::Volume(0)));
    assert_eq!(command::parse_line("playback previous"), Ok(Command::Playback("previous".into())));

    // The trailing newline a socket client sends is whitespace, not an argument.
    assert_eq!(command::parse_line("noise anc\n"), Ok(Command::Noise("anc".into())));
    assert_eq!(command::parse_line("  status  \r\n"), Ok(Command::Status));
}

#[test]
fn the_usage_text_lists_every_verb_the_grammar_accepts() {
    // NOTHING TIED THESE TOGETHER. `usage()` is the verb list a person is
    // shown when a command is refused, and `parse_words` is the grammar --
    // two lists in one file, maintained by hand, exactly the shape
    // `EQ_PRESETS` has a paragraph about. Adding `eq-band` and `clear-bass`
    // meant touching both, and nothing would have said so if only one had
    // been touched.
    //
    // The grammar is enumerated by ASKING IT, not by reading it: every verb
    // below is probed at its own arity and must not answer "invalid command or
    // argument count". A verb that vanished from `parse_words` fails here
    // even though this list is hand-written, because the parser is what
    // decides.
    let verbs: [(&str, &str); 14] = [
        ("status", "status"),
        ("panel-open", "panel-open"),
        ("panel-close", "panel-close"),
        ("refresh", "refresh"),
        ("noise", "noise anc"),
        ("ambient-level", "ambient-level 10"),
        ("focus-on-voice", "focus-on-voice on"),
        ("speak-to-chat", "speak-to-chat on"),
        ("dsee", "dsee on"),
        ("eq-preset", "eq-preset rock"),
        ("eq-band", "eq-band 1 0"),
        ("clear-bass", "clear-bass 0"),
        ("general", "general multipoint on"),
        ("connection", "connection quality"),
    ];
    let text = command::usage();
    for (verb, line) in verbs {
        assert_ne!(
            command::parse_line(line),
            Err("invalid command or argument count".to_string()),
            "{line:?} is not accepted by the grammar this test claims to enumerate"
        );
        assert!(text.contains(verb), "usage() does not list {verb:?}:\n{text}");
    }
    // `volume` and `playback` are the two the loop cannot probe by prefix --
    // "volume" is a prefix of nothing and "playback" of nothing, but both
    // names appear inside another verb's argument list in the usage text, so
    // a substring test for them would pass on a text that never names them.
    // Asked for as whole lines instead.
    for line in ["  volume 0..100\n", "  playback play|pause|next|previous\n"] {
        assert!(text.contains(line), "usage() is missing the line {line:?}:\n{text}");
    }
    assert_eq!(command::parse_line("volume 50"), Ok(Command::Volume(50)));
    assert_eq!(command::parse_line("playback play"), Ok(Command::Playback("play".into())));
}

#[test]
fn the_refusal_messages_are_the_ones_the_reference_sends() {
    // Verbatim: these reach a person, and a client written against the C++
    // daemon must get the same answer from this one.
    let cases: [(&str, &str); 19] = [
        ("", "no command supplied"),
        ("   \n", "no command supplied"),
        ("noise loud", "noise must be off, anc, or ambient"),
        ("ambient-level 0", "ambient-level must be an integer from 1 to 20"),
        ("ambient-level 21", "ambient-level must be an integer from 1 to 20"),
        ("focus-on-voice yes", "focus-on-voice must be on or off"),
        ("eq-preset nope", "unknown equalizer preset"),
        ("connection fast", "connection must be quality or stable"),
        ("volume 101", "volume must be an integer from 0 to 100"),
        // These five are OURS rather than the reference's -- it has neither
        // verb -- so the wording is this daemon's to choose and is pinned here
        // for the same reason: it reaches a person. The bounds named are the
        // PROTOCOL'S widest, not the headset's; `mdr::encode` refuses the rest
        // with the device's own numbers, because the real bound depends on the
        // length of the array it last sent and no parser can know that.
        ("eq-band 0 0", "eq-band takes a band from 1 to 10 and a step from -10 to 10"),
        ("eq-band 11 0", "eq-band takes a band from 1 to 10 and a step from -10 to 10"),
        ("eq-band 1 11", "eq-band takes a band from 1 to 10 and a step from -10 to 10"),
        ("eq-band one 1", "eq-band takes a band from 1 to 10 and a step from -10 to 10"),
        ("clear-bass -11", "clear-bass must be an integer from -10 to 10"),
        // The shape bound, which is what keeps the module header's promise
        // that no value a caller sends becomes anything but a word.
        ("general MULTIPOINT off", "general takes a setting name and on or off"),
        ("general multi-point off", "general takes a setting name and on or off"),
        ("general ../../etc off", "general takes a setting name and on or off"),
        ("general a;b off", "general takes a setting name and on or off"),
        ("general multipoint yes", "general must be on or off"),
    ];
    for (line, want) in cases {
        assert_eq!(command::parse_line(line), Err(want.to_string()), "for {line:?}");
    }
    // Wrong arity for a real verb is not a wrong argument.
    // `general 'rm -rf' off` belongs HERE and not above: there is no quoting on
    // this socket, so the shell-looking argument splits into two words and the
    // line is a four-word `general`, which is an arity error. That is the
    // module header's "no value a caller sends becomes anything but a word"
    // demonstrated rather than asserted in prose.
    for line in [
        "status extra",
        "noise",
        "volume",
        "eq-preset",
        "nonsense",
        "eq-band 1",
        "clear-bass",
        "general",
        "general multipoint",
        "general 'rm -rf' off",
    ] {
        assert_eq!(
            command::parse_line(line),
            Err("invalid command or argument count".to_string()),
            "for {line:?}"
        );
    }
    // The size cap, which is the only thing standing between a socket and an
    // unbounded allocation upstream.
    let long = "status ".repeat(1000);
    assert!(long.len() > command::MAX_COMMAND);
    assert_eq!(command::parse_line(&long), Err("command exceeds 4096 bytes".to_string()));
}

#[test]
fn no_argument_can_be_anything_but_a_word() {
    // The whole input surface of a daemon holding a Bluetooth socket. Every
    // argument is matched against a closed set or parsed as an integer, so
    // nothing a caller sends becomes a path, a command, or a format string.
    for hostile in [
        "noise $(reboot)",
        "noise `id`",
        "eq-preset ../../etc/passwd",
        "connection quality; rm -rf /",
        "volume 1e9",
        "volume -0x1",
        "ambient-level 1.5",
        "playback play\0extra",
    ] {
        let got = command::parse_line(hostile);
        assert!(got.is_err(), "{hostile:?} was accepted as {got:?}");
    }
}

#[test]
fn the_volume_scale_round_trips_at_the_reference_fixed_points() {
    // The device has 31 steps and the document reports a percentage, so the
    // mapping is lossy in one direction and these are the points that pin it.
    for (level, percent) in [(0, 0), (1, 3), (15, 50), (29, 97), (30, 100)] {
        assert_eq!(command::mdr_volume_to_percent(level), percent, "level {level}");
    }
    for (percent, level) in [(1, 0), (2, 1), (20, 6), (50, 15), (99, 30), (100, 30)] {
        assert_eq!(command::percent_to_mdr_volume(percent), level, "percent {percent}");
    }

    // Out of range in either direction clamps rather than wrapping.
    assert_eq!(command::mdr_volume_to_percent(-5), 0);
    assert_eq!(command::mdr_volume_to_percent(999), 100);
    assert_eq!(command::percent_to_mdr_volume(-5), 0);
    assert_eq!(command::percent_to_mdr_volume(999), 30);

    // Most percentages are not reachable: asking for 51 sets the same step as
    // 50, so a caller that echoes the request shows a value the headset is not
    // at. canonical_volume_percent is what it should show instead.
    assert_eq!(command::canonical_volume_percent(51), 50);
    assert_eq!(command::canonical_volume_percent(50), 50);
    // And it is idempotent: canonicalising twice is canonicalising once.
    for p in 0..=100 {
        let once = command::canonical_volume_percent(p);
        assert_eq!(command::canonical_volume_percent(once), once, "percent {p}");
    }
}

#[test]
fn the_preset_table_is_the_parser_allowlist() {
    // One table. The reference keeps two -- a serialisation list and a parser
    // allowlist -- which can drift into a preset that can be reported but not
    // set, or set but not named.
    assert_eq!(command::EQ_PRESETS.len(), 30);
    for preset in command::EQ_PRESETS {
        assert_eq!(
            command::parse_line(&format!("eq-preset {preset}")),
            Ok(Command::EqualizerPreset(preset.to_string())),
            "{preset} is named but not accepted"
        );
    }
}
