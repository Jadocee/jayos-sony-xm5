//! The support-function tables, and the capability flags they earn.
//!
//! Two frames in the corpus carry them, 54ms apart in the same handshake, and
//! the pair is unusually good evidence because they are the SAME message on
//! two different channels: `DATA_MDR` and `DATA_MDR_NO2`. Their function-code
//! bytes come from two different enums, so decoding both with one enum is a
//! mistake that produces a full, plausible, wrong list -- and
//! `the_wrong_table_produces_a_confident_wrong_answer` shows exactly what it
//! would produce.
//!
//! WHAT MAKES THIS MORE THAN A RESTATEMENT OF THE DECODER
//!
//! `the_advertised_functions_predict_the_rest_of_the_corpus` never looks at a
//! function code. It reads the other 26 payload frames -- captured AFTER the
//! handshake, by another implementation, from a real headset -- and asserts
//! that each one belongs to a capability this table turned on. The device is
//! the second opinion.

use sony_xm5::mdr::payload::{self, Reading, SupportFunction, Table};
use sony_xm5::mdr::{frame, DataType};
use sony_xm5::sony::state::Capabilities;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/mdr"))
}

/// Every capture, unpacked, in filename order.
fn corpus() -> Vec<frame::Frame> {
    let mut names: Vec<String> = fs::read_dir(fixture_dir())
        .expect("the fixture directory is there")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".bin"))
        .collect();
    names.sort();
    names
        .iter()
        .filter_map(|n| fs::read(fixture_dir().join(n)).ok())
        .filter_map(|bytes| frame::unpack(&bytes).ok())
        .collect()
}

/// The two support-function replies, decoded against the table their own frame
/// selects.
fn support_lists() -> Vec<(Table, Vec<SupportFunction>)> {
    corpus()
        .into_iter()
        .filter_map(|f| match payload::decode(f.data_type, &f.payload) {
            Reading::SupportFunctions { table, functions } => Some((table, functions)),
            _ => None,
        })
        .collect()
}

/// What the captured WF-1000XM5 says it can do, both tables folded together.
fn captured_capabilities() -> Capabilities {
    let mut caps = Capabilities::default();
    for (table, functions) in support_lists() {
        caps.apply_support_functions(table, &functions);
    }
    caps
}

fn flag(caps: &Capabilities, name: &str) -> bool {
    caps.entries()
        .into_iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("{name} is not a capability in the state document"))
        .1
}

#[test]
fn both_tables_decode_to_the_lists_the_device_actually_sent() {
    let lists = support_lists();
    assert_eq!(lists.len(), 2, "one support-function reply per table");

    // Byte for byte out of mdr-packet-…-000022 (DATA_MDR) and …-000026
    // (DATA_MDR_NO2). Pinned in wire order, priorities included, so a decoder
    // that reordered, dropped the priority or lost an entry fails here rather
    // than in a capability three layers up.
    let (table_one, one) = &lists[0];
    assert_eq!(*table_one, Table::One);
    let expected_one: Vec<(u8, u8)> = vec![
        (0x10, 0xFF), (0x12, 0xFF), (0x13, 0xFF), (0x14, 0xFF), (0x15, 0xFF), (0x16, 0xFF),
        (0x17, 0xFF), (0x23, 0xFF), (0x25, 0x36), (0x11, 0xFF), (0x27, 0xFF), (0x29, 0xFF),
        (0x2A, 0xFF), (0x92, 0xFF), (0x50, 0x11), (0x6B, 0x08), (0x70, 0x01), (0x90, 0xFF),
        (0xA1, 0x12), (0xC1, 0xFF), (0xC2, 0xFF), (0xE2, 0x1B), (0xF1, 0x38), (0xFC, 0x0D),
        (0x93, 0xFF), (0xF6, 0x33), (0xF9, 0x41), (0xD1, 0x3B), (0x94, 0xFF), (0x40, 0x2E),
        (0x42, 0x3A), (0x44, 0xFF), (0xFF, 0x2D), (0x19, 0xFF), (0xF3, 0x26), (0xE7, 0x1A),
        (0xD2, 0x20), (0xFD, 0x2B), (0xB6, 0x1D), (0xB0, 0x42), (0xB2, 0x43), (0xB8, 0x1F),
        (0xB7, 0x15), (0x34, 0x3F), (0xF4, 0x23), (0xF5, 0x24),
    ];
    assert_eq!(one.len(), 46, "the count byte said 0x2e");
    assert_eq!(
        one.iter().map(|f| (f.code, f.priority)).collect::<Vec<_>>(),
        expected_one
    );

    let (table_two, two) = &lists[1];
    assert_eq!(*table_two, Table::Two);
    let expected_two: Vec<(u8, u8)> = vec![
        (0x42, 0x3D), (0x53, 0xFF), (0x54, 0xFF), (0xF1, 0xFF), (0x60, 0xFF), (0x61, 0xFF),
        (0xF2, 0xFF), (0x63, 0xFF), (0xF8, 0x30), (0x32, 0x04), (0x31, 0xFF),
    ];
    assert_eq!(two.len(), 11, "the count byte said 0x0b");
    assert_eq!(
        two.iter().map(|f| (f.code, f.priority)).collect::<Vec<_>>(),
        expected_two
    );
}

#[test]
fn a_function_code_of_0xff_is_a_feature_and_not_the_end_of_the_list() {
    // Entry 33 of table 1 is `ff 2d`: HEAD_GESTURE_ON_OFF_TRAINING with a real
    // priority. A parser that stopped at 0xFF -- or that skipped it as "no
    // function" -- would drop head gestures AND the thirteen entries after it,
    // and the count byte would be the only thing that noticed.
    let lists = support_lists();
    let (_, one) = &lists[0];
    let at = one.iter().position(|f| f.code == 0xFF).expect("the 0xFF entry");
    assert_eq!(at, 32, "thirty-third of forty-six");
    assert_eq!(one[at].priority, 0x2D);
    assert_eq!(one.len() - at - 1, 13, "entries after it that a sentinel would lose");
    assert!(flag(&captured_capabilities(), "head_gesture"));
}

#[test]
fn the_captured_headset_earns_exactly_these_flags() {
    // All 27, named, so a mapping change has to say which flag it moved.
    let caps = captured_capabilities();
    let expected: &[(&str, bool)] = &[
        // The buds report left, right and case and never a main battery, and
        // the table says so before any battery frame arrives.
        ("battery_single", false),
        ("battery_left_right", true),
        ("battery_case", true),
        ("playback_metadata", true),
        ("playback_control", true),
        ("playback_volume", true),
        ("noise_cancelling", true),
        ("ambient_sound", true),
        ("adaptive_ambient_sound", true),
        ("speak_to_chat", true),
        // No LISTENING_OPTION on this model.
        ("listening_mode", false),
        ("equalizer", true),
        ("dsee", true),
        ("paired_device_management", true),
        ("pairing_mode", true),
        ("general_settings", true),
        ("assignable_controls", true),
        // Never set from a support function; see apply_support_functions.
        ("noise_control_button", false),
        ("auto_power_off", true),
        ("wearing_detection", true),
        ("auto_pause", true),
        ("head_gesture", true),
        ("voice_guidance", true),
        ("voice_guidance_volume", true),
        ("shutdown", true),
        // The device advertises CONNECTION_MODE_CLASSIC_AUDIO_LE_AUDIO (0xE7)
        // and NOT the sound-quality/connection-quality control the document's
        // `connection.priority` means. Sharing a word is not sharing a control.
        ("connection_mode", false),
        ("safe_listening", true),
    ];
    assert_eq!(caps.entries().len(), expected.len());
    for (name, want) in expected {
        assert_eq!(flag(&caps, name), *want, "{name}");
    }
}

#[test]
fn the_wrong_table_produces_a_confident_wrong_answer() {
    // The whole reason `decode` takes a data type. Read the table-2 reply as
    // though it had arrived on DATA_MDR -- one wrong argument, no error, a full
    // list of declared function codes -- and compare what it claims.
    let two = corpus()
        .into_iter()
        .find(|f| f.data_type == DataType::DataMdrNo2 && f.payload.first() == Some(&0x07))
        .expect("the table-2 support-function frame");

    let Reading::SupportFunctions { table, functions } =
        payload::decode(DataType::DataMdr, &two.payload)
    else {
        panic!("it decodes happily, which is the problem");
    };
    assert_eq!(table, Table::One, "the mistake, made deliberately");

    let mut wrong = Capabilities::default();
    wrong.apply_support_functions(table, &functions);

    // Five features this device has, gone. 0x42 is voice guidance with volume
    // on table 2 and CLASSIC_ONLY_LE_CLASSIC_SETTING on table 1; 0x53/0x54 are
    // safe listening; 0x32 is the paired-device list.
    for lost in [
        "voice_guidance",
        "voice_guidance_volume",
        "safe_listening",
        "paired_device_management",
        "pairing_mode",
    ] {
        assert!(!flag(&wrong, lost), "{lost} would be lost");
    }

    // And five it does not have, invented -- each from a byte that is a
    // perfectly valid t1::FunctionType. 0x53 reads as PRESET_EQ_AND_ULT_MODE,
    // 0xF1 as auto-pause, 0x61/0x63 as noise cancelling, 0xF2 as Speak-to-Chat.
    for invented in [
        "equalizer",
        "auto_pause",
        "noise_cancelling",
        "ambient_sound",
        "speak_to_chat",
    ] {
        assert!(flag(&wrong, invented), "{invented} would be invented");
    }
}

#[test]
fn the_advertised_functions_predict_the_rest_of_the_corpus() {
    // The second opinion. Every entry is (frame data type, command, inquired
    // type, the flag that message belongs to) -- read off the fixtures, not off
    // the support tables. Each row asserts BOTH that the corpus really contains
    // that message and that the capability derived from a completely different
    // frame turned the matching flag on.
    const FAMILIES: &[(u8, u8, u8, &str)] = &[
        // POWER_RET_STATUS at LR_BATTERY_WITH_THRESHOLD and
        // CRADLE_BATTERY_WITH_THRESHOLD -- and never at BATTERY, which is why
        // battery_single is false below.
        (0x0C, 0x23, 0x09, "battery_left_right"),
        (0x0C, 0x23, 0x0A, "battery_case"),
        // POWER_RET_PARAM at AUTO_POWER_OFF_WEARING_DETECTION.
        (0x0C, 0x27, 0x05, "auto_power_off"),
        // EQEBB_RET_STATUS and EQEBB_RET_PARAM at PRESET_EQ.
        (0x0C, 0x53, 0x00, "equalizer"),
        (0x0C, 0x57, 0x00, "equalizer"),
        // NCASM_RET_PARAM at MODE_NC_ASM_DUAL_NC_MODE_SWITCH_AND_ASM_SEAMLESS,
        // which is both halves of the feature.
        (0x0C, 0x67, 0x17, "noise_cancelling"),
        (0x0C, 0x67, 0x17, "ambient_sound"),
        // PLAY_RET_PARAM at PLAYBACK_CONTROL_WITH_CALL_VOLUME_ADJUSTMENT (four
        // empty track names) and at MUSIC_VOLUME; PLAY_RET_STATUS for the
        // transport.
        (0x0C, 0xA7, 0x01, "playback_metadata"),
        (0x0C, 0xA7, 0x20, "playback_volume"),
        (0x0C, 0xA3, 0x01, "playback_control"),
        // The AUDIO family at UPSCALING, which is DSEE.
        (0x0C, 0xE1, 0x01, "dsee"),
        (0x0C, 0xE3, 0x01, "dsee"),
        (0x0C, 0xE7, 0x01, "dsee"),
        // SYSTEM_RET_PARAM at four different inquired types, each one a
        // separate flag.
        (0x0C, 0xF7, 0x01, "auto_pause"),
        (0x0C, 0xF7, 0x03, "assignable_controls"),
        (0x0C, 0xF7, 0x0C, "speak_to_chat"),
        (0x0C, 0xFB, 0x0C, "speak_to_chat"),
        (0x0C, 0xF7, 0x0F, "head_gesture"),
        // GENERAL_SETTING_RET_CAPABILITY and _RET_PARAM, once each for
        // GENERAL_SETTING_1 and GENERAL_SETTING_2 -- exactly the two the table
        // advertises, and no third.
        (0x0C, 0xD1, 0xD1, "general_settings"),
        (0x0C, 0xD1, 0xD2, "general_settings"),
        (0x0C, 0xD7, 0xD1, "general_settings"),
        (0x0C, 0xD7, 0xD2, "general_settings"),
        // Table 2's own families.
        (0x0E, 0x33, 0x02, "paired_device_management"),
        (0x0E, 0x37, 0x02, "paired_device_management"),
        (0x0E, 0x47, 0x01, "voice_guidance"),
        (0x0E, 0x47, 0x20, "voice_guidance_volume"),
        (0x0E, 0x5B, 0x03, "safe_listening"),
    ];

    // The handshake itself. These answer to no feature, and listing them here
    // is what makes the sweep below a real check rather than an empty one.
    const NOT_A_FEATURE: &[u8] = &[0x01, 0x03, 0x05, 0x07];

    // COMMON_RET_STATUS at AUDIO_CODEC answers to a table-1 function the
    // state document has no flag for: CODEC_INDICATOR, t1 0x12. It is
    // explained by the table rather than exempted from it, so the assertion
    // is that the captured table 1 advertises that function.
    const ADVERTISED_WITHOUT_A_FLAG: &[(u8, u8, u8, u8)] = &[(0x0C, 0x13, 0x02, 0x12)];

    let present: BTreeSet<(u8, u8, u8)> = corpus()
        .iter()
        .filter(|f| f.payload.len() >= 2)
        .map(|f| (f.data_type.to_wire(), f.payload[0], f.payload[1]))
        .collect();

    let caps = captured_capabilities();
    for (data_type, command, kind, name) in FAMILIES {
        assert!(
            present.contains(&(*data_type, *command, *kind)),
            "the corpus no longer has {command:#04x}/{kind:#04x} on frame type {data_type:#04x}"
        );
        assert!(flag(&caps, name), "{name}, for {command:#04x}/{kind:#04x}");
    }
    let lists = support_lists();
    let (_, table_one) = lists.iter().find(|(t, _)| *t == Table::One).expect("table 1");
    for (data_type, command, kind, function) in ADVERTISED_WITHOUT_A_FLAG {
        assert!(present.contains(&(*data_type, *command, *kind)));
        assert!(
            table_one.iter().any(|f| f.code == *function),
            "table 1 does not advertise {function:#04x}, which {command:#04x}/{kind:#04x} answers to"
        );
    }

    // The falsifier: a message family with no advertised function behind it.
    // There is not one, and this is the assertion that would find it.
    let unexplained: Vec<(u8, u8, u8)> = present
        .iter()
        .copied()
        .filter(|(_, command, _)| !NOT_A_FEATURE.contains(command))
        .filter(|(d, c, k)| !FAMILIES.iter().any(|(fd, fc, fk, _)| (fd, fc, fk) == (d, c, k)))
        .filter(|(d, c, k)| !ADVERTISED_WITHOUT_A_FLAG.iter().any(|(fd, fc, fk, _)| (fd, fc, fk) == (d, c, k)))
        .collect();
    assert!(unexplained.is_empty(), "messages no advertised function explains: {unexplained:02x?}");

    // The other direction of the battery evidence: no BATTERY or
    // BATTERY_WITH_THRESHOLD frame anywhere, and the flag off.
    assert!(!present.contains(&(0x0C, 0x23, 0x00)));
    assert!(!present.contains(&(0x0C, 0x23, 0x08)));
    assert!(!flag(&caps, "battery_single"));
}

#[test]
fn a_list_whose_count_does_not_claim_the_payload_is_refused() {
    // 07 00 02 followed by one and a half entries, and by three.
    let short = [0x07, 0x00, 0x02, 0x10, 0xFF, 0x12];
    let long = [0x07, 0x00, 0x02, 0x10, 0xFF, 0x12, 0xFF, 0x13, 0xFF];
    for bad in [&short[..], &long[..]] {
        assert!(
            matches!(payload::decode(DataType::DataMdr, bad), Reading::Unhandled { .. }),
            "a list built from the first n pairs of something else is the failure to avoid"
        );
    }

    // An empty list is a real answer: a device that supports nothing on a
    // table says so, and that is not the same as a malformed reply.
    assert_eq!(
        payload::decode(DataType::DataMdr, &[0x07, 0x00, 0x00]),
        Reading::SupportFunctions { table: Table::One, functions: Vec::new() }
    );
}

#[test]
fn a_frame_type_with_no_table_leaves_the_list_undecoded() {
    // DATA_COMMON carries commands but no support-function namespace. Guessing
    // a table here would be a coin toss over a hundred features, so the message
    // comes back Unhandled with its key intact for a daemon to log.
    let payload = [0x07, 0x00, 0x01, 0x50, 0x11];
    assert_eq!(
        payload::decode(DataType::DataCommon, &payload),
        Reading::Unhandled { command: 0x07, kind: Some(0x00), len: 5 }
    );
    // And the same bytes on a frame type that does have one.
    assert!(matches!(
        payload::decode(DataType::DataMdr, &payload),
        Reading::SupportFunctions { table: Table::One, .. }
    ));
}

#[test]
fn an_inquired_type_the_message_does_not_have_is_refused() {
    // ConnectInquiredType::FIXED_VALUE is 0x00 and is the only one this
    // message declares. A payload claiming another one is a different message
    // wearing this command byte.
    assert!(matches!(
        payload::decode(DataType::DataMdr, &[0x07, 0x01, 0x01, 0x50, 0x11]),
        Reading::Unhandled { .. }
    ));
}

#[test]
fn applying_a_table_never_clears_a_flag_another_table_set() {
    // The two replies arrive separately and neither is the whole answer. If
    // this folded rather than accumulated, whichever arrived second would win.
    let lists = support_lists();
    let mut forwards = Capabilities::default();
    for (table, functions) in &lists {
        forwards.apply_support_functions(*table, functions);
    }
    let mut backwards = Capabilities::default();
    for (table, functions) in lists.iter().rev() {
        backwards.apply_support_functions(*table, functions);
    }
    assert_eq!(forwards, backwards, "order must not matter");
    assert_eq!(forwards, captured_capabilities());

    // And applying one twice changes nothing.
    let mut twice = forwards.clone();
    for (table, functions) in &lists {
        twice.apply_support_functions(*table, functions);
    }
    assert_eq!(twice, forwards);
}

/// A device that advertises exactly these codes on this table, and nothing else.
fn only(table: Table, codes: &[u8]) -> Capabilities {
    let functions: Vec<SupportFunction> = codes
        .iter()
        .map(|code| SupportFunction { code: *code, priority: payload::NO_PRIORITY })
        .collect();
    let mut caps = Capabilities::default();
    caps.apply_support_functions(table, &functions);
    caps
}

#[test]
fn a_neighbouring_function_does_not_earn_a_flag_it_only_sounds_like() {
    // Every one of these is a code the mapping deliberately does NOT follow,
    // and each is checked in isolation because the captured headset advertises
    // the real function alongside the lookalike -- so on that device the
    // mistake is invisible. A mutation that added HEAD_TRACKER_ACCEL_TYPE to
    // the head-gesture arm passed every other test in this file.

    // HEAD_TRACKER_* is 360 Reality Audio following your head. Head gestures
    // are nodding to answer a call, and that is 0xFF alone.
    assert!(!flag(&only(Table::One, &[0xB5, 0xB7]), "head_gesture"));
    assert!(flag(&only(Table::One, &[0xFF]), "head_gesture"));

    // AUTO_POWER_OFF_WITH_WEARING_DETECTION says the timer may use the
    // sensor, not that the headset reports wear.
    let auto_off = only(Table::One, &[0x25]);
    assert!(flag(&auto_off, "auto_power_off"));
    assert!(!flag(&auto_off, "wearing_detection"));
    assert!(flag(&only(Table::One, &[0xF6]), "wearing_detection"));
    assert!(flag(&only(Table::Two, &[0xF0]), "wearing_detection"));

    // CONNECTION_MODE_CLASSIC_AUDIO_LE_AUDIO carries the SAME PriorMode as the
    // two forms that do set this flag; what it also carries is an undocumented
    // fourth byte, and its inquired type has no pinned reply struct. So the
    // flag is withheld because the port could not drive the control, not
    // because the control is a different one.
    assert!(!flag(&only(Table::One, &[0xE7]), "connection_mode"));
    assert!(flag(&only(Table::One, &[0xE1]), "connection_mode"));
    assert!(flag(&only(Table::One, &[0xE3]), "connection_mode"));

    // AMBIENT_SOUND_CONTROL_MODE_SELECT is which modes the control cycles
    // through, not a physical noise-control button -- and no code sets that
    // flag at all.
    let mode_select = only(Table::One, &[0x69]);
    assert!(flag(&mode_select, "ambient_sound"));
    assert!(!flag(&mode_select, "noise_control_button"));
    for code in 0u8..=255 {
        assert!(!flag(&only(Table::One, &[code]), "noise_control_button"), "t1 {code:#04x}");
        assert!(!flag(&only(Table::Two, &[code]), "noise_control_button"), "t2 {code:#04x}");
    }

    // UPSCALING_INDICATOR is a display hint for Sony's app; UPSCALING_AUTO_OFF
    // is the DSEE setting.
    assert!(!flag(&only(Table::One, &[0x13]), "dsee"));
    assert!(flag(&only(Table::One, &[0xE2]), "dsee"));

    // SOUND_EFFECT sits inside the equaliser's code block and is not one.
    assert!(!flag(&only(Table::One, &[0x54, 0x58, 0x59]), "equalizer"));
    assert!(flag(&only(Table::One, &[0x50]), "equalizer"));

    // BGM_MODE is not LISTENING_OPTION.
    assert!(!flag(&only(Table::One, &[0xE4, 0xEB]), "listening_mode"));
    assert!(flag(&only(Table::One, &[0xE6]), "listening_mode"));

    // MAX_VOLUME_LEVEL_LIMIT is a cap, not the safe-listening report.
    assert!(!flag(&only(Table::Two, &[0x55]), "safe_listening"));
    assert!(flag(&only(Table::Two, &[0x54]), "safe_listening"));

    // SOURCE_SWITCH_CONTROL shares the peripheral command family with the
    // paired-device list and is a different feature.
    let source_switch = only(Table::Two, &[0x31]);
    assert!(!flag(&source_switch, "paired_device_management"));
    assert!(!flag(&source_switch, "pairing_mode"));

    // PLAYBACK_CONTROLLER_WITH_FUNCTION_CHANGE drops the volume clause.
    let no_volume = only(Table::One, &[0xA4]);
    assert!(flag(&no_volume, "playback_control") && flag(&no_volume, "playback_metadata"));
    assert!(!flag(&no_volume, "playback_volume"));
    assert!(flag(&only(Table::One, &[0xA1]), "playback_volume"));

    // The whole 0x45..0x4F block names a LIMITATION on a feature, not the
    // feature. 0x47 begins PAIRING_DEVICE_MANAGEMENT_ and means the opposite
    // of having it.
    let lea = [0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x4B, 0x4C, 0x4D, 0x4E, 0x4F];
    assert_eq!(
        only(Table::One, &lea),
        Capabilities::default(),
        "a limitation is not a capability"
    );
}
