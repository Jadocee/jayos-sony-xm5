//! The payload layer, against the frames a real headset sent.
//!
//! The frame tests next door prove that 70 captured packets unpack and re-pack
//! byte for byte. These take the payloads out of those same packets and ask
//! what they MEAN -- which is the layer where the reference header stops being
//! a reliable guide, for the reasons `fixtures/mdr/PROVENANCE.md` sets out.
//!
//! WHAT WOULD MAKE EACH OF THESE GO RED
//!
//! Not "the code changed": every assertion below is pinned to a value taken
//! out of a captured frame, so a decoder that starts reading a different byte
//! produces a different number and the test names it. The battery test in
//! particular fails for the exact mistake PROVENANCE describes -- reading a
//! threshold byte as a further battery level.

use sony_xm5::mdr::frame;
use sony_xm5::mdr::payload::{
    self, Charging, Codec, Command, NoiseMode, Reading, Sensitivity, Switch, Timeout, Upscaling,
};
use sony_xm5::mdr::DataType;
use std::fs;
use std::path::PathBuf;

/// Every payload in this file arrived on `DATA_MDR`, the ordinary command
/// channel, and the corpus filenames say so. Named once rather than spelled out
/// at forty call sites -- and it is spelled out at all because one message in
/// this protocol means different things on the two channels. See
/// `mdr_support_functions.rs`, which is where that matters.
const MDR: DataType = DataType::DataMdr;

fn fixture_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/mdr"))
}

/// Every capture whose filename names this command, decoded to its payload.
fn payloads_for(command: &str) -> Vec<Vec<u8>> {
    let mut names: Vec<String> = fs::read_dir(fixture_dir())
        .expect("the fixture directory is there")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".bin") && n.contains(&format!(".cmd-{command}-")))
        .collect();
    names.sort();
    names
        .iter()
        .filter_map(|n| fs::read(fixture_dir().join(n)).ok())
        .filter_map(|bytes| frame::unpack(&bytes).ok())
        .map(|f| f.payload)
        .filter(|p| !p.is_empty())
        .collect()
}

#[test]
fn the_command_byte_round_trips_through_the_enum() {
    // Every command this port names must survive a round trip, or a decoder
    // dispatching on the enum and a table keyed on the byte disagree.
    for byte in 0u8..=255 {
        assert_eq!(Command::from_byte(byte).to_byte(), byte, "byte 0x{byte:02x}");
    }
}

#[test]
fn a_notify_command_decodes_as_its_return_twin() {
    // The mapping the module relies on. Verified against the header family by
    // family; asserted here so a new entry cannot be added silently.
    assert_eq!(Command::from_byte(0x25).canonical(), Command::PowerRetStatus);
    assert_eq!(Command::from_byte(0x69).canonical(), Command::NcAsmRetParam);
    assert_eq!(Command::from_byte(0xA5).canonical(), Command::PlayRetStatus);
    // COMMON_NTFY_STATUS: CommonNotifyStatusAudioCodec is field for field
    // CommonRetStatusAudioCodec.
    assert_eq!(Command::from_byte(0x15).canonical(), Command::CommonRetStatus);
    // And a RET is already canonical.
    assert_eq!(Command::from_byte(0x23).canonical(), Command::PowerRetStatus);
    // An unmodelled byte stays itself rather than becoming something else.
    assert_eq!(Command::from_byte(0x77).canonical(), Command::Other(0x77));
}

#[test]
fn the_captured_battery_frames_decode_to_the_levels_the_device_reported() {
    let payloads = payloads_for("POWER_RET_STATUS");
    assert_eq!(payloads.len(), 2, "the corpus has two battery frames");

    let mut saw_lr = false;
    let mut saw_case = false;

    for payload in &payloads {
        match payload::decode(MDR, payload) {
            Reading::Battery(report) => {
                match payload.get(1) {
                    // LR_BATTERY_WITH_THRESHOLD. The whole capture is
                    //   23 09 | 00 00 64 00 64 64
                    // and the trap is reading byte 6 as a further battery.
                    Some(0x09) => {
                        saw_lr = true;
                        let left = report.left.expect("a left battery");
                        let right = report.right.expect("a right battery");
                        assert_eq!(left.level, 0);
                        assert_eq!(left.charging, Charging::No);
                        assert_eq!(left.threshold, Some(100));
                        assert_eq!(right.level, 100);
                        assert_eq!(right.charging, Charging::No);
                        assert_eq!(right.threshold, Some(100));
                        // The buds report no main battery, ever.
                        assert!(report.main.is_none(), "a WF has no main battery");
                        assert!(report.case.is_none(), "not in this message");
                    }
                    // CRADLE_BATTERY_WITH_THRESHOLD: 23 0a | 35 00 1e
                    Some(0x0A) => {
                        saw_case = true;
                        let case = report.case.expect("a case battery");
                        assert_eq!(case.level, 53);
                        assert_eq!(case.charging, Charging::No);
                        assert_eq!(case.threshold, Some(30));
                        assert!(report.left.is_none() && report.right.is_none());
                    }
                    other => panic!("an unexpected battery kind: {other:?}"),
                }
            }
            other => panic!("a battery frame decoded as {other:?}"),
        }
    }
    assert!(saw_lr && saw_case, "both battery kinds were exercised");

    // The document's words for BatteryChargingStatus are the C++ daemon's
    // chargingName: 0x00 NOT_CHARGING, 0x01 CHARGING, 0x02 UNKNOWN, 0x03
    // CHARGED. The page tests `=== "yes"`, so "charging" would never light it.
    assert_eq!(
        [0x00u8, 0x01, 0x02, 0x03, 0x7F].map(|b| Charging::from_byte(b).as_str()),
        ["no", "yes", "unknown", "complete", "unknown"]
    );
}

#[test]
fn the_over_ear_battery_forms_decode_from_their_header_layouts() {
    // HEADER ONLY: no capture in this corpus is a WH, so these bytes are built
    // from PowerRetStatusBattery (T1:6980) and PowerRetStatusBatteryThreshold
    // (T1:7010), whose offsets Gadgetbridge's handleBattery (level payload[2],
    // charging payload[3]) and libmdr agree on.
    //
    // 23 00 <level> <charging>: 0x49 = 73%, CHARGING.
    match payload::decode(MDR, &[0x23, 0x00, 0x49, 0x01]) {
        Reading::Battery(r) => {
            let main = r.main.expect("a main battery");
            assert_eq!(main.level, 73);
            assert_eq!(main.charging, Charging::Yes);
            assert_eq!(main.threshold, None);
            assert!(r.left.is_none() && r.right.is_none() && r.case.is_none());
        }
        other => panic!("decoded as {other:?}"),
    }
    // 23 08 <value1> <charging> <threshold>: 100%, NOT_CHARGING, threshold 20.
    match payload::decode(MDR, &[0x23, 0x08, 0x64, 0x00, 0x14]) {
        Reading::Battery(r) => {
            let main = r.main.expect("a main battery");
            assert_eq!(main.level, 100);
            assert_eq!(main.charging, Charging::No);
            assert_eq!(main.threshold, Some(20));
            assert!(r.case.is_none(), "0x08 is the MAIN battery, not the cradle");
        }
        other => panic!("decoded as {other:?}"),
    }
    // A length that fits neither struct is a different struct wearing the
    // key, and falls through rather than misreading a byte.
    assert!(matches!(
        payload::decode(MDR, &[0x23, 0x00, 0x49, 0x01, 0x14]),
        Reading::Unhandled { kind: Some(0x00), len: 5, .. }
    ));
    assert!(matches!(
        payload::decode(MDR, &[0x23, 0x08, 0x64, 0x00]),
        Reading::Unhandled { kind: Some(0x08), len: 4, .. }
    ));
}

#[test]
fn reading_a_threshold_as_a_battery_level_would_be_caught() {
    // The failure PROVENANCE describes, spelled out: the plain
    // LEFT_RIGHT_BATTERY layout is four bytes of payload after the type, and
    // the captured message carries six. Decoding the capture as the plain
    // variant must NOT succeed, because the length does not match -- that is
    // the whole reason the dispatch key includes it.
    let payloads = payloads_for("POWER_RET_STATUS");
    let lr = payloads.iter().find(|p| p.get(1) == Some(&0x09)).expect("the 0x09 capture");
    assert_eq!(lr.len(), 8, "type plus six bytes");

    // The same bytes relabelled as the plain kind: eight bytes where the plain
    // layout is six, so it must fall through rather than read two threshold
    // bytes as a third and fourth battery reading.
    let mut mislabelled = lr.clone();
    mislabelled[1] = 0x01;
    match payload::decode(MDR, &mislabelled) {
        Reading::Unhandled { command, kind, len } => {
            assert_eq!(command, 0x23);
            assert_eq!(kind, Some(0x01));
            assert_eq!(len, 8);
        }
        other => panic!("a length that matches no layout decoded as {other:?}"),
    }
}

#[test]
fn the_captured_noise_frame_says_the_feature_is_off() {
    let payloads = payloads_for("NCASM_RET_PARAM");
    assert_eq!(payloads.len(), 1, "the corpus has one NC/ASM frame");
    let payload = &payloads[0];
    assert_eq!(payload.as_slice(), &[0x67, 0x17, 0x01, 0x00, 0x01, 0x00, 0x14]);

    match payload::decode(MDR, payload) {
        Reading::Noise(noise) => {
            // ncAsmTotalEffect is OFF, and that beats the mode: the headset
            // reports ASM and a level of 20 while the whole feature is off,
            // and calling that "ambient" would put a bar on the panel for a
            // filter doing nothing.
            assert_eq!(noise.mode, NoiseMode::Off);
            assert_eq!(noise.ambient_level, Some(20));
            assert_eq!(noise.focus_on_voice, Some(false), "AmbientSoundMode::NORMAL");
            assert!(noise.settled, "ValueChangeStatus::CHANGED");
        }
        other => panic!("the NC/ASM frame decoded as {other:?}"),
    }
}

#[test]
fn the_total_effect_switch_decides_the_mode() {
    // Built from the captured frame by changing one byte at a time, so each
    // assertion differs from a real device's message in exactly one place.
    let base = [0x67u8, 0x17, 0x01, 0x00, 0x01, 0x00, 0x14];

    let mode_of = |effect: u8, mode: u8, ambient: u8| {
        let mut p = base;
        p[3] = effect;
        p[4] = mode;
        p[5] = ambient;
        match payload::decode(MDR, &p) {
            Reading::Noise(n) => (n.mode, n.focus_on_voice == Some(true)),
            other => panic!("decoded as {other:?}"),
        }
    };

    assert_eq!(mode_of(0x00, 0x01, 0x00).0, NoiseMode::Off, "off beats ASM");
    assert_eq!(mode_of(0x00, 0x00, 0x00).0, NoiseMode::Off, "off beats NC");
    assert_eq!(mode_of(0x01, 0x00, 0x00).0, NoiseMode::Anc);
    assert_eq!(mode_of(0x01, 0x01, 0x00).0, NoiseMode::Ambient);
    // AmbientSoundMode::VOICE is the speech filter, and it is a separate
    // switch from the mode.
    assert!(mode_of(0x01, 0x01, 0x01).1, "voice");
    assert!(!mode_of(0x01, 0x01, 0x00).1, "normal");
}

#[test]
fn the_headset_names_itself_and_its_firmware() {
    let payloads = payloads_for("CONNECT_RET_DEVICE_INFO");
    assert_eq!(payloads.len(), 3, "model name, firmware, series and colour");

    let mut model = String::new();
    let mut firmware = String::new();
    let mut undecoded = 0;

    for payload in &payloads {
        match payload::decode(MDR, payload) {
            Reading::Device(info) => {
                if !info.model.is_empty() {
                    model = info.model;
                }
                if !info.firmware.is_empty() {
                    firmware = info.firmware;
                }
            }
            // SERIES_AND_COLOR_INFO, deliberately not decoded: nothing in the
            // state document holds a series or a colour, and ModelColor is
            // declared in a header this port was not transcribed from.
            Reading::Unhandled { kind, .. } => {
                assert_eq!(kind, Some(0x03));
                undecoded += 1;
            }
            other => panic!("a device-info frame decoded as {other:?}"),
        }
    }

    // The device that produced this corpus, in its own words.
    assert_eq!(model, "WF-1000XM5");
    assert_eq!(firmware, "6.1.0");
    assert_eq!(undecoded, 1);
}

#[test]
fn a_length_prefix_that_lies_yields_nothing_rather_than_a_truncated_name() {
    // The prefix is ONE byte and the captures confirm it -- "WF-1000XM5"
    // arrives as 0a and ten bytes. A two-byte prefix would have produced a
    // model name starting with a NUL and nobody would have questioned it.
    let named = [0x05, 0x01, 0x0A, b'W', b'F', b'-', b'1', b'0', b'0', b'0', b'X', b'M', b'5'];
    match payload::decode(MDR, &named) {
        Reading::Device(info) => assert_eq!(info.model, "WF-1000XM5"),
        other => panic!("decoded as {other:?}"),
    }

    // A prefix claiming more bytes than are there. Truncating would report a
    // model name that is a prefix of the real one, which reads as a different
    // headset; refusing reports nothing, which reads as an unknown one.
    assert!(matches!(
        payload::decode(MDR, &[0x05, 0x01, 0x40, b'W', b'F']),
        Reading::Unhandled { .. }
    ));

    // An empty name is still a name-shaped message, and decodes to an empty
    // string rather than falling through.
    match payload::decode(MDR, &[0x05, 0x01, 0x00]) {
        Reading::Device(info) => assert_eq!(info.model, ""),
        other => panic!("decoded as {other:?}"),
    }

    // Two bytes is `ConnectRetDeviceInfo`, which carries no string at all --
    // the length is what tells it from ConnectRetDeviceInfoModelName, since
    // both declare MODEL_NAME.
    assert!(matches!(payload::decode(MDR, &[0x05, 0x01]), Reading::Unhandled { .. }));
}

#[test]
fn the_equaliser_frame_yields_a_preset_and_its_steps_unsplit() {
    let payloads = payloads_for("EQEBB_RET_PARAM");
    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0].as_slice(), &[0x57, 0x00, 0xA0, 0x06, 20, 11, 10, 13, 9, 12]);

    match payload::decode(MDR, &payloads[0]) {
        Reading::Equalizer(eq) => {
            assert_eq!(eq.preset, 0xA0, "EqPresetId::CUSTOM");
            // VERBATIM, in wire order: this array is what the eq-preset
            // encoder echoes back to the headset. The split into clear bass
            // and bands happens where the document is written, and
            // sony_session.rs pins it there.
            assert_eq!(eq.steps, vec![20, 11, 10, 13, 9, 12]);
        }
        other => panic!("the equaliser frame decoded as {other:?}"),
    }
}

#[test]
fn a_step_count_that_does_not_match_the_payload_is_refused() {
    // The count and the payload length have to agree exactly. They disagree
    // when the frame is a different structure wearing the same key:
    // EqParamUltMode puts a status byte before the array, so reading it as
    // EqParam leaves a byte the count never claimed.
    //
    // Trailing byte: the count says four, five follow.
    assert!(matches!(
        payload::decode(MDR, &[0x57, 0x00, 0xA0, 0x04, 1, 2, 3, 4, 5]),
        Reading::Unhandled { .. }
    ));
    // Short: the count says eight, four follow.
    assert!(matches!(
        payload::decode(MDR, &[0x57, 0x00, 0xA0, 0x08, 1, 2, 3, 4]),
        Reading::Unhandled { .. }
    ));
    // Exactly right.
    assert!(matches!(
        payload::decode(MDR, &[0x57, 0x00, 0xA0, 0x04, 1, 2, 3, 4]),
        Reading::Equalizer(_)
    ));
    // An empty array is a real answer: a preset with no adjustable bands.
    match payload::decode(MDR, &[0x57, 0x00, 0x10, 0x00]) {
        Reading::Equalizer(eq) => {
            assert_eq!(eq.preset, 0x10);
            assert!(eq.steps.is_empty());
        }
        other => panic!("decoded as {other:?}"),
    }
    // A kind other than PRESET_EQ carries a different structure entirely, and
    // the refusal has to be the KIND rather than an accident of length.
    //
    // The first draft of this test used `[0x57, 0x01, 0x05]`, which falls
    // through whether or not the kind is checked -- there is no count byte for
    // the array reader to find. A mutation removing the kind check passed it.
    // These bytes WOULD parse as an EqParam: preset 0xA0, count 2, two steps,
    // nothing left over. Only the kind stops them.
    assert!(
        matches!(payload::decode(MDR, &[0x57, 0x01, 0xA0, 0x02, 1, 2]), Reading::Unhandled { .. }),
        "EBB is not PRESET_EQ, however EqParam-shaped its bytes happen to be"
    );
    // And the same bytes under the right kind do parse, so the refusal above
    // is about the kind and nothing else.
    assert!(matches!(
        payload::decode(MDR, &[0x57, 0x00, 0xA0, 0x02, 1, 2]),
        Reading::Equalizer(_)
    ));
}

#[test]
fn a_named_setting_with_bytes_left_over_is_not_a_named_setting() {
    // The two length-prefixed strings have to account for the whole body, for
    // the same reason the equaliser's count does: a LIST-type general setting
    // carries its alternatives after the names, and reading one as a BOOLEAN
    // would produce a plausible name and a silently wrong meaning. The corpus
    // holds no LIST capture, so the length check is what keeps that case
    // arriving as `Unhandled` -- with its key, for whoever extends this --
    // rather than as a switch nobody can toggle.
    //
    // The captured shape, which must parse: 000031's body, trimmed.
    let ok = [0xD1, 0xD1, 0x00, 0x01, 0x02, b'h', b'i', 0x01, b'x'];
    match payload::decode(MDR, &ok) {
        Reading::GeneralSettingInfo(g) => {
            assert_eq!((g.slot, g.name.as_str(), g.summary.as_str()), (0xD1, "hi", "x"));
        }
        other => panic!("the captured shape decoded as {other:?}"),
    }
    // One byte past the second string, and it is a different structure.
    let mut trailing = ok.to_vec();
    trailing.push(0x00);
    assert!(
        matches!(payload::decode(MDR, &trailing), Reading::Unhandled { .. }),
        "a body the two names do not account for was read as a boolean setting"
    );
    // A prefix that lies the other way: the summary claims four bytes and one
    // follows. `prefixed_string` refuses it, which is the older guard, but
    // both edges belong in one place.
    let short = [0xD1, 0xD1, 0x00, 0x01, 0x02, b'h', b'i', 0x04, b'x'];
    assert!(matches!(payload::decode(MDR, &short), Reading::Unhandled { .. }));
}

#[test]
fn a_general_setting_value_that_is_neither_on_nor_off_is_refused() {
    // The corpus shows both of the two values this port accepts and no third.
    // ONE CITATION PER LINE, because `mdr_citations.rs` pairs a fixture
    // counter with the bytes beside it and a line naming two gets them
    // cross-matched -- which is exactly what the first draft of this comment
    // did, and the suite said so:
    //
    // - 000035 is `d7 d1 00 01`
    // - 000043 is `d7 d2 00 00`
    //
    // The last byte is a `GsSettingValue`, ON = 0x00 (T1:956): so 0x01 is OFF.
    // These two assertions said the opposite until the reference was read,
    // and `multipoint_is_on_where_two_hosts_are_connected_at_once` in
    // sony_session.rs is the corpus's own evidence that this way is right.
    assert!(matches!(
        payload::decode(MDR, &[0xD7, 0xD1, 0x00, 0x01]),
        Reading::GeneralSettingParam { slot: 0xD1, enabled: false }
    ));
    assert!(matches!(
        payload::decode(MDR, &[0xD7, 0xD2, 0x00, 0x00]),
        Reading::GeneralSettingParam { slot: 0xD2, enabled: true }
    ));
    // AND NOTHING ELSE FOLDED INTO A BOOL. The reference's on/off enums are
    // not uniformly 0=off -- Speak-to-Chat's is inverted, which
    // `speak_to_chat_is_off_on_the_captured_headset` pins -- so a value this
    // port has never seen must arrive as `Unhandled` with its key rather than
    // be rounded to the nearer of two answers.
    for byte in [0x02u8, 0x10, 0xFF] {
        assert!(
            matches!(payload::decode(MDR, &[0xD7, 0xD1, 0x00, byte]), Reading::Unhandled { .. }),
            "0x{byte:02x} was read as on or off"
        );
    }
    // And the length is fixed at two: a longer body is a different structure.
    assert!(matches!(
        payload::decode(MDR, &[0xD7, 0xD1, 0x00, 0x01, 0x00]),
        Reading::Unhandled { .. }
    ));
    // A LIST-TYPE setting is not a switch. Its setting-type byte is 0x01
    // (T1:952) and its body is `GsRetParamList`, which nothing here decodes;
    // read as a boolean, its first byte would be taken for on or off.
    assert!(
        matches!(payload::decode(MDR, &[0xD7, 0xD1, 0x01, 0x00]), Reading::Unhandled { .. }),
        "a list-type setting was read as a switch"
    );
}

#[test]
fn a_list_type_setting_is_not_offered_as_a_switch() {
    // The capability reply names the type before the strings. A LIST_TYPE
    // (0x01, T1:952) capability decodes to nothing, so no switch is drawn for
    // a setting whose param body this port cannot read or write.
    let boolean = [0xD1, 0xD1, 0x00, 0x01, 0x01, b'X', 0x01, b'S'];
    assert!(matches!(payload::decode(MDR, &boolean), Reading::GeneralSettingInfo(_)));
    let list = [0xD1, 0xD1, 0x01, 0x01, 0x01, b'X', 0x01, b'S'];
    assert!(matches!(payload::decode(MDR, &list), Reading::Unhandled { .. }));
    // And a string format the reference does not declare (T1:963 has two).
    let format = [0xD1, 0xD1, 0x00, 0x02, 0x01, b'X', 0x01, b'S'];
    assert!(matches!(payload::decode(MDR, &format), Reading::Unhandled { .. }));
}

#[test]
fn a_volunteered_setting_change_reads_the_same_as_an_answered_one() {
    // 0xD9 is GENERAL_SETTING_NTFY_PARAM: the same body, sent because somebody
    // pressed something on the headset rather than because we asked. It must
    // decode identically, or a setting toggled on the earbud shows in the
    // document only after the next reconnect.
    //
    // This is the same GET/RET/SET/NTFY arithmetic `mdr_encode.rs` pins over
    // all 91 derivable pairs -- 0xD6/0xD7/0xD8/0xD9 -- applied to the one
    // family this port added last.
    let ret = payload::decode(MDR, &[0xD7, 0xD2, 0x00, 0x01]);
    let ntfy = payload::decode(MDR, &[0xD9, 0xD2, 0x00, 0x01]);
    assert_eq!(ret, ntfy, "a volunteered change decoded differently from an answer");
    // 0x01 is GsSettingValue::OFF.
    assert!(matches!(ntfy, Reading::GeneralSettingParam { slot: 0xD2, enabled: false }));
}

#[test]
fn the_volume_frame_is_recognised_by_a_value_no_play_mode_could_be() {
    let payloads = payloads_for("PLAY_RET_PARAM");
    assert_eq!(payloads.len(), 2, "a volume and a longer frame this port skips");

    let volume = payloads.iter().find(|p| p.len() == 3).expect("the three-byte frame");
    assert_eq!(volume.as_slice(), &[0xA7, 0x20, 0x0C]);

    match payload::decode(MDR, volume) {
        Reading::Volume { device_level } => {
            assert_eq!(device_level, 12, "of a maximum of 30");
            // What the state document will report, through the existing
            // conversion rather than a second copy of it here.
            assert_eq!(sony_xm5::sony::command::mdr_volume_to_percent(i32::from(device_level)), 40);
        }
        other => panic!("the volume frame decoded as {other:?}"),
    }

    // The ten-byte frame at type 0x01 matches no struct in the header and is
    // reported as such rather than forced into one.
    let other = payloads.iter().find(|p| p.len() == 10).expect("the ten-byte frame");
    assert!(matches!(payload::decode(MDR, other), Reading::Unhandled { .. }));
}

#[test]
fn a_volume_the_device_cannot_produce_is_refused() {
    // The whole disambiguation rests on the byte being a plausible volume and
    // not a PlayMode, so the plausibility is enforced rather than assumed.
    // 30 is the maximum; 31 is a frame this port has misread.
    let thirty = payload::decode(MDR, &[0xA7, 0x20, 30]);
    assert!(matches!(thirty, Reading::Volume { device_level: 30 }));
    assert!(matches!(payload::decode(MDR, &[0xA7, 0x20, 31]), Reading::Unhandled { .. }));
    assert!(matches!(payload::decode(MDR, &[0xA7, 0x20, 200]), Reading::Unhandled { .. }));
    // Silence is a real volume.
    assert!(matches!(payload::decode(MDR, &[0xA7, 0x20, 0]), Reading::Volume { device_level: 0 }));

    // CALL_VOLUME is a different number for a different thing, and the state
    // document has nowhere to put it.
    assert!(matches!(payload::decode(MDR, &[0xA7, 0x21, 12]), Reading::Unhandled { .. }));
    // MUSIC_VOLUME_WITH_MUTE carries an extra byte and has no capture.
    assert!(matches!(payload::decode(MDR, &[0xA7, 0x30, 12, 0]), Reading::Unhandled { .. }));
    // And a four-byte frame under the right kind is not the three-byte struct.
    assert!(matches!(payload::decode(MDR, &[0xA7, 0x20, 12, 0]), Reading::Unhandled { .. }));
}

#[test]
fn an_ambiguous_key_is_refused_by_name_rather_than_guessed() {
    // NCASM_NTFY_PARAM, NC_ON_OFF_AND_ASM_SEAMLESS, seven bytes. Two structs
    // claim it and read byte 6 as different things -- an on/off value in one
    // and the ambient level in the other -- and no capture settles which, so a
    // decoder that picked one would produce a plausible value from the wrong
    // byte forever.
    //
    // It is in the NTFY family only: both NTFY structs are seven bytes while
    // the two RET structs are six and seven. So the same kind and length under
    // NCASM_RET_PARAM is NOT ambiguous, and refusing it would refuse a
    // decodable message.
    match payload::decode(MDR, &[0x69, 0x13, 0, 0, 0, 0, 0]) {
        Reading::Ambiguous { command, kind, len, between } => {
            assert_eq!((command, kind, len), (0x69, Some(0x13), 7));
            assert_eq!(between, ["NcAsmNtfyParamNcOnOffAsmOnOff", "NcAsmNtfyParamNcOnOffAsmSeamless"]);
        }
        other => panic!("an ambiguous key decoded as {other:?}"),
    }
    assert!(
        !matches!(payload::decode(MDR, &[0x67, 0x13, 0, 0, 0, 0, 0]), Reading::Ambiguous { .. }),
        "the RET family is not ambiguous at this key"
    );
}

#[test]
fn every_capture_decodes_to_something_rather_than_panicking() {
    // The whole corpus through the decoder. Most are Unhandled, which is the
    // point: an undecoded message is a state a daemon lives in, not a crash.
    let mut decoded = 0;
    let mut ambiguous = 0;
    let mut unhandled = 0;
    let mut total = 0;

    for entry in fs::read_dir(fixture_dir()).expect("the fixture directory") {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".bin") {
            continue;
        }
        let Ok(bytes) = fs::read(entry.path()) else { continue };
        let Ok(frame) = frame::unpack(&bytes) else { continue };
        total += 1;
        match payload::decode(frame.data_type, &frame.payload) {
            Reading::Unhandled { .. } => unhandled += 1,
            Reading::Ambiguous { .. } => ambiguous += 1,
            _ => decoded += 1,
        }
    }
    assert_eq!(total, 70, "the whole corpus");
    // Twenty-five: two battery frames, one NC/ASM, two device-info strings,
    // the equaliser, the volume, the codec, the two support-function tables,
    // the two DSEE halves, the two Speak-to-Chat halves, the paired-device
    // list, the four general setting frames -- two capability replies naming
    // SIDETONE_SETTING and MULTIPOINT_SETTING and two param replies saying
    // whether each is on -- and the six plan 23 added: the play status
    // (000065), the upscaling status (000097), and table 2's two voice
    // guidance replies, safe-listening figure and pairing status.
    //
    // PINNED, so adding a decoder without a test to go with it fails here, and
    // so does deleting one. This number moved 14 -> 18 when the general
    // settings stopped being asked for and dropped: `feature_gets` had queried
    // them since the support tables were decoded, and nothing read the answers.
    // Then 18 -> 19 when table-two paired devices (fixture 000077) was added.
    // Then 19 -> 21 for plan 23: the play status (000065), read only where its
    // two structs agree, and the upscaling status (000097), read the way the
    // reference client reads it. Then 21 -> 25 for item 11: table 2's voice
    // guidance switch and volume, safe-listening figure and pairing status.
    assert_eq!(decoded, 25, "captures this port currently understands");
    // The one frame a real device sent onto an ambiguous key is now read on
    // the half both structs agree on; see below.
    assert_eq!(ambiguous, 0);
    assert_eq!(unhandled, 45);
}

#[test]
fn the_corpus_play_status_is_read_only_where_its_two_structs_agree() {
    // The strongest evidence that the (command, type, LENGTH) analysis was
    // worth doing: the ambiguity is not a theoretical property of the header.
    // A real WF-1000XM5 sent this frame, and two structs in the reference both
    // claim its key.
    let payloads = payloads_for("PLAY_RET_STATUS");
    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0].as_slice(), &[0xA3, 0x01, 0x00, 0x02, 0x00]);

    // And the capture does NOT settle it. Byte 4 is 0x00, which is
    // MusicCallStatus::MUSIC under one reading and EnableDisable::ENABLE under
    // the other -- both entirely plausible for an idle device. A second frame
    // with a 1 there would settle it; this corpus has no second frame.
    assert_eq!(payloads[0][4], 0x00, "the byte the two structs disagree about");

    // What the two DO agree on: offsets 0 through 3 are Command,
    // PlayInquiredType, EnableDisable, PlaybackStatus in both. So byte 3 is
    // PlaybackStatus::PAUSE (0x02) whichever struct this is -- plan 23, item 5
    // -- and that half is what is read.
    let want = Reading::PlayStatus(payload::PlayStatusReport {
        control: Switch::On,
        status: payload::PlaybackStatus::Pause,
    });
    assert_eq!(payload::decode(MDR, &payloads[0]), want);
    assert_eq!(payload::PlaybackStatus::Pause.as_str(), "paused");

    // THE DISPUTED BYTE IS NEVER READ: every value at offset 4 decodes to the
    // same thing. A decoder that picked a struct would fail here on the first
    // value its enum does not name.
    for byte in 0u8..=255 {
        assert_eq!(payload::decode(MDR, &[0xA3, 0x01, 0x00, 0x02, byte]), want, "offset 4 = {byte:#04x}");
    }
    // The NTFY reads as the RET does; the two families agree at offsets 0..=3.
    assert_eq!(payload::decode(MDR, &[0xA5, 0x01, 0x00, 0x02, 0x01]), want);

    // The other fields: DISABLE at offset 2 is the headset saying it will not
    // take a transport command, and PlaybackStatus's four values are the C++
    // daemon's words, with UNSETTLED and anything unnamed as `unknown`.
    match payload::decode(MDR, &[0xA3, 0x01, 0x01, 0x01, 0x00]) {
        Reading::PlayStatus(r) => {
            assert_eq!(r.control, Switch::Off);
            assert_eq!(r.status.as_str(), "playing");
        }
        other => panic!("decoded as {other:?}"),
    }
    let word = |byte: u8| match payload::decode(MDR, &[0xA3, 0x01, 0x00, byte, 0x00]) {
        Reading::PlayStatus(r) => r.status.as_str(),
        other => panic!("decoded as {other:?}"),
    };
    assert_eq!([0x00u8, 0x01, 0x02, 0x03, 0xFF].map(word), ["unknown", "playing", "paused", "stopped", "unknown"]);

    // Six bytes is the other RET at this type, which no capture holds.
    assert!(matches!(payload::decode(MDR, &[0xA3, 0x01, 0, 2, 0, 0]), Reading::Unhandled { .. }));
}

#[test]
fn the_decoder_survives_anything() {
    // It runs on bytes off a Bluetooth socket in a process built with
    // panic = "abort", where a panic is a SIGABRT with the backtrace stripped.
    payload::decode(MDR, &[]);
    payload::decode(MDR, &[0x23]);
    payload::decode(MDR, &[0x23, 0x09]);
    payload::decode(MDR, &[0x67, 0x17]);
    for byte in 0u8..=255 {
        payload::decode(MDR, &[byte]);
        payload::decode(MDR, &[0x23, byte]);
        payload::decode(MDR, &[0x67, byte, 0, 0, 0, 0, 0]);
        payload::decode(MDR, &[0x23, 0x09, byte, byte, byte, byte, byte, byte]);
    }
    let long = vec![0x23u8; 4096];
    payload::decode(MDR, &long);
}

#[test]
fn the_dsee_frames_say_which_upscaler_and_whether_it_is_on() {
    // Two commands at the same inquired type and the same length, and the byte
    // at offset 2 belongs to a different enum in each. Both are captured.
    let capability = payloads_for("AUDIO_RET_CAPABILITY");
    assert_eq!(capability.len(), 1);
    assert_eq!(capability[0], vec![0xE1, 0x01, 0x02]);
    match payload::decode(MDR, &capability[0]) {
        Reading::Dsee(report) => {
            // UpscalingType::DSEE_HX_AI, which is what a WF-1000XM5 has, in
            // the C++ daemon's dseeName word.
            assert_eq!(report.upscaling, Some(Upscaling::DseeHxAi));
            assert_eq!(report.upscaling.expect("a type").as_str(), "hx_ai");
            assert_eq!(report.enabled, None, "the capability carries no setting");
        }
        other => panic!("decoded as {other:?}"),
    }

    let param = payloads_for("AUDIO_RET_PARAM");
    assert_eq!(param.len(), 1);
    assert_eq!(param[0], vec![0xE7, 0x01, 0x01]);
    match payload::decode(MDR, &param[0]) {
        Reading::Dsee(report) => {
            // UpscalingTypeAutoOff::AUTO, the ordinary way round.
            assert_eq!(report.enabled, Some(Switch::On));
            assert_eq!(report.upscaling, None, "the setting carries no type");
        }
        other => panic!("decoded as {other:?}"),
    }

    // The mistake this arrangement invites: the SAME byte, 0x01, means AUTO on
    // the param command and DSEE on the capability command. Nothing but the
    // command tells them apart, so pin that they do not agree.
    let as_capability = payload::decode(MDR, &[0xE1, 0x01, 0x01]);
    let as_param = payload::decode(MDR, &[0xE7, 0x01, 0x01]);
    assert_ne!(as_capability, as_param);
    assert_eq!(as_capability, Reading::Dsee(payload::DseeReport {
        upscaling: Some(Upscaling::Dsee),
        enabled: None,
    }));

    // AUDIO_RET_STATUS arrives at the same inquired type, and is read as
    // libmdr reads it: `AudioRetStatusCommon`, whose type is unpinned, with
    // ENABLE (0x00) meaning DSEE can run. Plan 23, item 10. The NTFY (0xE5)
    // is `AudioNotifyStatusCommon`, the same three fields.
    let status = payloads_for("AUDIO_RET_STATUS");
    assert_eq!(status[0], vec![0xE3, 0x01, 0x00]);
    assert_eq!(payload::decode(MDR, &status[0]), Reading::UpscalingStatus { available: Switch::On });
    assert_eq!(payload::decode(MDR, &[0xE5, 0x01, 0x01]), Reading::UpscalingStatus { available: Switch::Off });
    assert_eq!(payload::grounds(MDR, &status[0]), Some(payload::Grounds::ReferenceClient));
    // At any other type it is not this message.
    assert!(matches!(payload::decode(MDR, &[0xE3, 0x02, 0x00]), Reading::Unhandled { .. }));

    // A NOTIFY of the setting decodes as the RET does; the header's two
    // structs were compared field for field.
    assert_eq!(payload::decode(MDR, &[0xE9, 0x01, 0x01]), as_param);

    // The document's words for UpscalingType, 0x00..=0x03, are the C++
    // daemon's: DSEE on the wire is `standard` in the document, and none of
    // them carries a `dsee_` prefix.
    let word = |byte: u8| match payload::decode(MDR, &[0xE1, 0x01, byte]) {
        Reading::Dsee(r) => r.upscaling.expect("a type").as_str(),
        other => panic!("decoded as {other:?}"),
    };
    assert_eq!([0x00u8, 0x01, 0x02, 0x03, 0xFF].map(word), ["hx", "standard", "hx_ai", "ultimate", "unknown"]);
}

#[test]
fn the_codec_frame_names_the_codec_and_nothing_else_on_that_command() {
    // Fixture 000053, the corpus's own COMMON_RET_STATUS: 13 02 02, which is
    // AUDIO_CODEC and AudioCodec::AAC. The captured table 1 advertises
    // CODEC_INDICATOR (0x12), the function this reply answers to.
    let captured = payloads_for("COMMON_RET_STATUS");
    assert_eq!(captured, vec![vec![0x13, 0x02, 0x02]]);
    assert_eq!(payload::decode(MDR, &captured[0]), Reading::Codec(Codec::Aac));

    // The rest of AudioCodec (T1:228), and the C++ daemon's codecName words.
    for (byte, codec, word) in [
        (0x00u8, Codec::Unsettled, "unknown"),
        (0x01, Codec::Sbc, "sbc"),
        (0x02, Codec::Aac, "aac"),
        (0x10, Codec::Ldac, "ldac"),
        (0x20, Codec::AptX, "aptx"),
        (0x21, Codec::AptXHd, "aptx_hd"),
        (0x30, Codec::Lc3, "lc3"),
        (0xFF, Codec::Other(0xFF), "unknown"),
    ] {
        assert_eq!(payload::decode(MDR, &[0x13, 0x02, byte]), Reading::Codec(codec), "0x{byte:02x}");
        assert_eq!(codec.as_str(), word);
    }
    assert_eq!(Codec::from_byte(0x7E), Codec::Other(0x7E));

    // COMMON_NTFY_STATUS is the same message volunteered: a codec change
    // mid-session (LDAC to SBC on a bad link) arrives this way.
    assert_eq!(payload::decode(MDR, &[0x15, 0x02, 0x01]), Reading::Codec(Codec::Sbc));

    // Only AUDIO_CODEC at exactly three bytes. CONNECTION_STATUS (0x01) is a
    // four-byte struct with two connection states; a codec with a byte too
    // few or too many is not this struct; and table 2 is another namespace.
    assert!(matches!(payload::decode(MDR, &[0x13, 0x01, 0x00, 0x00]), Reading::Unhandled { .. }));
    assert!(matches!(payload::decode(MDR, &[0x13, 0x02]), Reading::Unhandled { .. }));
    assert!(matches!(payload::decode(MDR, &[0x13, 0x02, 0x02, 0x00]), Reading::Unhandled { .. }));
    assert!(matches!(payload::decode(DataType::DataMdrNo2, &[0x13, 0x02, 0x02]), Reading::Unhandled { .. }));
}

#[test]
fn speak_to_chat_is_off_on_the_captured_headset() {
    // THE TRAP. OnOffSettingValue is ON = 0x00 and OFF = 0x01, so the obvious
    // reading of this frame inverts it -- and 0x01 is what the device sent.
    let params = payloads_for("SYSTEM_RET_PARAM");
    let smart = params
        .iter()
        .find(|p| p.get(1) == Some(&0x0C))
        .expect("the SMART_TALKING_MODE_TYPE2 frame");
    assert_eq!(*smart, vec![0xF7, 0x0C, 0x01, 0x01]);
    match payload::decode(MDR, smart) {
        Reading::SpeakToChat(report) => {
            assert_eq!(report.enabled, Some(Switch::Off), "0x01 is OFF, not ON");
            assert_eq!(report.enabled.expect("a switch").as_bool(), Some(false));
            assert_eq!(report.preview, Some(Switch::Off));
            assert_eq!(report.sensitivity, None, "not in this message");
            assert_eq!(report.timeout, None);
        }
        other => panic!("decoded as {other:?}"),
    }

    // The extended reply: DetectSensitivity::AUTO and ModeOutTime::FAST, which
    // are Sony's defaults and what an untouched setting should read as.
    let ext = payloads_for("SYSTEM_RET_EXT_PARAM");
    assert_eq!(ext.len(), 1);
    assert_eq!(ext[0], vec![0xFB, 0x0C, 0x00, 0x00]);
    match payload::decode(MDR, &ext[0]) {
        Reading::SpeakToChat(report) => {
            assert_eq!(report.sensitivity, Some(Sensitivity::Auto));
            assert_eq!(report.timeout, Some(Timeout::Short));
            assert_eq!(report.sensitivity.expect("a value").as_str(), "auto");
            assert_eq!(report.timeout.expect("a value").as_str(), "short");
            assert_eq!(report.enabled, None, "not in this message");
        }
        other => panic!("decoded as {other:?}"),
    }
    // ModeOutTime::NONE is the document's "manual" -- a rename, and the one
    // that took a decision. Exercised through a payload rather than the
    // constructor, so the offsets are part of the assertion.
    match payload::decode(MDR, &[0xFB, 0x0C, 0x02, 0x03]) {
        Reading::SpeakToChat(report) => {
            assert_eq!(report.sensitivity.expect("a value").as_str(), "low");
            assert_eq!(report.timeout, Some(Timeout::Manual));
            assert_eq!(report.timeout.expect("a value").as_str(), "manual");
        }
        other => panic!("decoded as {other:?}"),
    }
    // A byte neither enum names is reported as unknown rather than as the
    // nearest thing that parses.
    match payload::decode(MDR, &[0xFB, 0x0C, 0xFF, 0xFF]) {
        Reading::SpeakToChat(report) => {
            assert_eq!(report.sensitivity, Some(Sensitivity::Other(0xFF)));
            assert_eq!(report.sensitivity.expect("a value").as_str(), "unknown");
            assert_eq!(report.timeout.expect("a value").as_str(), "unknown");
        }
        other => panic!("decoded as {other:?}"),
    }
    // And an out-of-range switch is neither on nor off.
    match payload::decode(MDR, &[0xF7, 0x0C, 0xFF, 0x00]) {
        Reading::SpeakToChat(report) => {
            assert_eq!(report.enabled, Some(Switch::Unknown(0xFF)));
            assert_eq!(report.enabled.expect("a switch").as_bool(), None);
            assert_eq!(report.preview, Some(Switch::On));
        }
        other => panic!("decoded as {other:?}"),
    }
    assert_eq!(Timeout::Manual.as_str(), "manual");
}

#[test]
fn the_other_system_replies_are_left_alone() {
    // SMART_TALKING_MODE_TYPE1 (0x02) shares the RET_PARAM struct but NOT the
    // extended one: TYPE1's is five bytes with a voiceFocus in the middle, so
    // accepting both would read a timeout out of a voice-focus byte.
    assert!(matches!(
        payload::decode(MDR, &[0xF7, 0x02, 0x01, 0x01]),
        Reading::Unhandled { .. }
    ));
    assert!(matches!(
        payload::decode(MDR, &[0xFB, 0x02, 0x00, 0x01, 0x00]),
        Reading::Unhandled { .. }
    ));

    // The other three SYSTEM_RET_PARAM frames in the corpus -- auto-pause,
    // assignable settings and head gestures -- are three bytes at inquired
    // types this decoder does not claim, and the state document has no field
    // for any of them beyond the capability flag.
    for payload in payloads_for("SYSTEM_RET_PARAM") {
        let handled = matches!(payload::decode(MDR, &payload), Reading::SpeakToChat(_));
        assert_eq!(handled, payload.get(1) == Some(&0x0C), "{payload:02x?}");
    }
}

#[test]
fn the_same_bytes_on_the_other_table_are_not_the_same_message() {
    // The command byte does not say which table a frame is on, and the two
    // enums disagree on 45 bytes. Six of them are bytes this port names
    // (`decode` lists them: 0x53, 0x57, 0x59, 0x61, 0x67, 0x69).
    //
    // These are the captured equaliser and NC/ASM payloads, unchanged, put on
    // a DATA_MDR_NO2 frame. On table 2 they are SAFE_LISTENING_RET_PARAM and
    // LEA_RET_PARAM -- messages this port does not read. Decoding them as an
    // equaliser preset and a noise mode would put a preset nobody chose and a
    // noise mode nobody set onto the panel, from a device that really does
    // send both families: the captured WF-1000XM5 advertises safe listening
    // (t2 0x53, 0x54) and LE Audio (t2 0x60, 0x61, 0x63) in its own table-2
    // support-function reply.
    let equaliser = [0x57u8, 0x00, 0xA0, 0x06, 0x14, 0x0B, 0x0A, 0x0D, 0x09, 0x0C];
    let noise = [0x67u8, 0x17, 0x01, 0x00, 0x01, 0x00, 0x14];

    for payload in [&equaliser[..], &noise[..]] {
        // Table 1: read, as the captures prove they should be.
        assert!(
            !matches!(payload::decode(MDR, payload), Reading::Unhandled { .. }),
            "{payload:02x?} is a table-1 message and must still decode"
        );
        // Table 2, and the third namespace: refused, with the key intact so a
        // daemon can log the gap.
        for elsewhere in [DataType::DataMdrNo2, DataType::DataCommon] {
            assert!(
                matches!(payload::decode(elsewhere, payload), Reading::Unhandled { .. }),
                "{payload:02x?} decoded as a table-1 message on {elsewhere:?}"
            );
        }
    }

    // A matching command NAME is not a matching message either. 0x23 is
    // POWER_RET_STATUS on both tables, but ProtocolV2T2.hpp:373 declares its
    // own PowerInquiredType: 0x00 is AUTO_STANDBY there and BATTERY here. The
    // captured left/right battery frame on a table-2 wrapper is a standby
    // setting, and reporting a charge level for it would be a number invented
    // out of the wrong enum.
    let battery = [0x23u8, 0x09, 0x00, 0x00, 0x64, 0x00, 0x64, 0x64];
    assert!(matches!(payload::decode(MDR, &battery), Reading::Battery(_)));
    assert!(matches!(
        payload::decode(DataType::DataMdrNo2, &battery),
        Reading::Unhandled { .. }
    ));

    // The support-function reply is the one exception, and it is exempt
    // because its HEADER is table-neutral -- 0x07 and FIXED_VALUE are the same
    // in both enums -- while its body is table-scoped, which is exactly what
    // decode_support_functions takes the table for.
    let functions = [0x07u8, 0x00, 0x01, 0x50, 0x11];
    for (data_type, want) in
        [(MDR, payload::Table::One), (DataType::DataMdrNo2, payload::Table::Two)]
    {
        match payload::decode(data_type, &functions) {
            Reading::SupportFunctions { table, .. } => assert_eq!(table, want),
            other => panic!("the support-function reply must decode on both tables, got {other:?}"),
        }
    }
}

#[test]
fn the_captured_keys_are_exactly_the_ones_the_corpus_decodes() {
    // `payload::CAPTURED` is the read side's claim of capture, and a claim is
    // only worth what holds it. Both directions: a key there that no fixture
    // decodes is a capture nobody made, and a decoded fixture key missing
    // there would be reported as header-only when a headset was seen sending
    // it. Keys decoded on other grounds are listed in OTHER_GROUNDS instead,
    // and may hold a fixture's bytes -- that is the point of the distinction.
    use std::collections::BTreeSet;
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/mdr");
    let mut decoded = BTreeSet::new();
    for entry in std::fs::read_dir(dir).expect("the corpus") {
        let path = entry.expect("an entry").path();
        if path.extension().is_none_or(|x| x != "bin") {
            continue;
        }
        let f = frame::unpack(&std::fs::read(&path).expect("it reads")).expect("it unpacks");
        if f.payload.len() < 2 || payload::grounds(f.data_type, &f.payload).is_none() {
            continue;
        }
        // The key `grounds` itself looks up, so the two cannot canonicalise
        // differently.
        decoded.insert(payload::key(f.data_type, &f.payload).expect("two bytes at least"));
    }
    let other: BTreeSet<payload::Key> = payload::OTHER_GROUNDS.iter().map(|(k, _, _)| *k).collect();
    let captured: BTreeSet<payload::Key> = payload::CAPTURED.iter().copied().collect();
    let want: BTreeSet<payload::Key> = decoded.difference(&other).copied().collect();
    assert_eq!(captured, want, "CAPTURED and the corpus disagree");
    assert!(captured.is_disjoint(&other), "a key is claimed both captured and otherwise grounded");
}

#[test]
fn a_frame_nothing_decodes_has_no_grounds_and_a_header_only_decode_says_so() {
    // The plain single-battery form: decoded, never captured.
    assert_eq!(payload::grounds(MDR, &[0x23, 0x00, 0x49, 0x01]), Some(payload::Grounds::HeaderOnly));
    // A captured key.
    assert_eq!(payload::grounds(MDR, &[0x23, 0x09, 0x46, 0x00, 0x46, 0x00, 0x1e, 0x1e]).map(|g| g.as_str()), Some("captured"));
    // Nothing decodes this.
    assert_eq!(payload::grounds(MDR, &[0x01, 0x00]), None);
}

#[test]
fn the_ambient_level_range_is_read_from_the_capability_reply_and_nothing_less() {
    // PLAN 23, ITEM 2. HEADER ONLY: no capture holds an NCASM_RET_CAPABILITY,
    // so these are built from `AmbientSoundModeLevelSet` (T1:2317) in an
    // `MDRPodArray` (a count byte, then packed elements, Protocol.hpp:409).
    // 61 17 02 | 00 01 14 01 | 01 01 0a 01: NORMAL 1..=20 by 1, VOICE 1..=10 by 1.
    let good = [0x61, 0x17, 0x02, 0x00, 0x01, 0x14, 0x01, 0x01, 0x01, 0x0A, 0x01];
    match payload::decode(MDR, &good) {
        Reading::NoiseCapability { kind, ranges } => {
            assert_eq!(kind, 0x17);
            assert_eq!(ranges.len(), 2);
            assert!(!ranges[0].voice && ranges[0].min == 1 && ranges[0].max == 20 && ranges[0].step == 1);
            assert!(ranges[1].voice && ranges[1].max == 10);
            assert!(ranges[0].admits(20) && !ranges[0].admits(21) && !ranges[0].admits(0));
        }
        other => panic!("decoded as {other:?}"),
    }
    // A step is honoured, not just the ends.
    let stepped = payload::AmbientRange { voice: false, min: 2, max: 20, step: 2 };
    assert!(stepped.admits(4) && !stepped.admits(5) && !stepped.admits(1));

    // Everything a range must not be is refused whole.
    let refused: [(&str, Vec<u8>); 8] = [
        ("a length that is not the count's", vec![0x61, 0x17, 0x02, 0x00, 0x01, 0x14, 0x01]),
        ("a zero step", vec![0x61, 0x17, 0x01, 0x00, 0x01, 0x14, 0x00]),
        ("min above max", vec![0x61, 0x17, 0x01, 0x00, 0x14, 0x01, 0x01]),
        ("a mode that is neither NORMAL nor VOICE", vec![0x61, 0x17, 0x01, 0x02, 0x01, 0x14, 0x01]),
        ("a mode listed twice", vec![0x61, 0x17, 0x02, 0x00, 0x01, 0x14, 0x01, 0x00, 0x01, 0x0A, 0x01]),
        ("no ranges at all", vec![0x61, 0x17, 0x00]),
        ("a type outside the seamless family", vec![0x61, 0x01, 0x01, 0x00, 0x01, 0x14, 0x01]),
        // 0x13 has a second struct with one-byte modes and no ranges: 3 + n.
        ("the 0x13 on/off form", vec![0x61, 0x13, 0x02, 0x00, 0x01]),
    ];
    for (why, bytes) in refused {
        assert!(matches!(payload::decode(MDR, &bytes), Reading::Unhandled { .. }), "{why} was read as a range");
    }
    // And on table 2 the same byte is LEA_RET_CAPABILITY.
    assert!(matches!(payload::decode(DataType::DataMdrNo2, &good), Reading::Unhandled { .. }));
}

#[test]
fn each_nc_asm_layout_reads_its_own_offsets() {
    // PLAN 23, ITEM 3. Built on the captured 0x17 frame's values where the
    // layouts share them. See `decode_noise` for what each stands on.
    let noise = |bytes: &[u8]| match payload::decode(MDR, bytes) {
        Reading::Noise(n) => n,
        other => panic!("{bytes:02x?} decoded as {other:?}"),
    };

    // 0x15, Gadgetbridge's WH-1000XM5: the 0x17 offsets. On, ASM, voice, 7.
    let n = noise(&[0x67, 0x15, 0x01, 0x01, 0x01, 0x01, 0x07]);
    assert_eq!((n.mode, n.focus_on_voice, n.ambient_level), (NoiseMode::Ambient, Some(true), Some(7)));

    // 0x19, noise adaptation: the same five, then the adaptive switch and its
    // sensitivity at the END -- the level stays at offset 6.
    let n = noise(&[0x67, 0x19, 0x01, 0x01, 0x00, 0x00, 0x0C, 0x01, 0x02]);
    assert_eq!((n.mode, n.ambient_level), (NoiseMode::Anc, Some(12)));
    assert_eq!((n.adaptive, n.adaptive_sensitivity), (Some(true), Some(0x02)));

    // 0x22, ambient only: no mode byte, so on means ambient. The ambient
    // mode and the level are given values that cannot stand in for each
    // other -- VOICE (0x01) and 9 -- so reading either at the other's offset
    // changes an answer. `NcAsmRetParamAsmSeamless` (T1:5289): ambient mode
    // at frame byte 4, level at 5.
    let n = noise(&[0x67, 0x22, 0x01, 0x01, 0x00, 0x05]);
    assert_eq!((n.mode, n.mode_byte, n.ambient_level), (NoiseMode::Ambient, None, Some(5)));
    let n = noise(&[0x67, 0x22, 0x01, 0x01, 0x01, 0x09]);
    assert_eq!((n.focus_on_voice, n.ambient_level), (Some(true), Some(9)));

    // 0x13 at seven bytes: an NC switch where the others have a mode byte.
    // `NcAsmRetParamNcOnOffAsmSeamless` (T1:5505): NC switch at frame byte 4,
    // ambient mode at 5, level at 6 -- distinct values again.
    assert_eq!(noise(&[0x67, 0x13, 0x01, 0x01, 0x01, 0x00, 0x09]).mode, NoiseMode::Anc);
    assert_eq!(noise(&[0x67, 0x13, 0x01, 0x01, 0x00, 0x00, 0x09]).mode, NoiseMode::Ambient);
    let n = noise(&[0x67, 0x13, 0x01, 0x01, 0x00, 0x01, 0x09]);
    assert_eq!((n.mode, n.focus_on_voice, n.ambient_level), (NoiseMode::Ambient, Some(true), Some(9)));
    let n = noise(&[0x67, 0x13, 0x01, 0x01, 0x00, 0x00, 0x01]);
    assert_eq!((n.focus_on_voice, n.ambient_level), (Some(false), Some(1)));

    // 0x01 at five: cancelling or nothing, and no level to publish.
    let n = noise(&[0x67, 0x01, 0x01, 0x01, 0x01]);
    assert_eq!((n.mode, n.ambient_level, n.focus_on_voice), (NoiseMode::Anc, None, None));
    assert_eq!(noise(&[0x67, 0x01, 0x01, 0x01, 0x00]).mode, NoiseMode::Off);

    // Total effect off beats every layout's mode.
    assert_eq!(noise(&[0x67, 0x19, 0x01, 0x00, 0x00, 0x00, 0x0C, 0x01, 0x02]).mode, NoiseMode::Off);

    // A length no struct at that type has is not read.
    for bytes in [&[0x67u8, 0x15, 0x01, 0x01, 0x01, 0x01][..], &[0x67, 0x19, 0x01, 0x01, 0x00, 0x00, 0x0C][..]] {
        assert!(matches!(payload::decode(MDR, bytes), Reading::Unhandled { .. }), "{bytes:02x?}");
    }
    // And the 0x13 NOTIFY at seven bytes is still the reference's collision.
    assert!(matches!(
        payload::decode(MDR, &[0x69, 0x13, 0x01, 0x01, 0x01, 0x00, 0x09]),
        Reading::Ambiguous { .. }
    ));
}

#[test]
fn each_nc_asm_layout_reports_what_it_stands_on() {
    use payload::Grounds;
    let g = |bytes: &[u8]| payload::grounds(MDR, bytes);
    assert_eq!(g(&[0x67, 0x17, 0x01, 0x00, 0x01, 0x00, 0x14]), Some(Grounds::Captured));
    assert_eq!(g(&[0x67, 0x15, 0x01, 0x01, 0x01, 0x01, 0x07]), Some(Grounds::ThirdParty));
    assert_eq!(g(&[0x67, 0x19, 0x01, 0x01, 0x00, 0x00, 0x0C, 0x01, 0x02]), Some(Grounds::ReferenceClient));
    assert_eq!(g(&[0x67, 0x22, 0x01, 0x01, 0x00, 0x05]), Some(Grounds::ReferenceClient));
    assert_eq!(g(&[0x67, 0x01, 0x01, 0x01, 0x01]), Some(Grounds::HeaderOnly));
    assert_eq!(g(&[0x61, 0x17, 0x01, 0x00, 0x01, 0x14, 0x01]), Some(Grounds::HeaderOnly));
    // A volunteered change stands where its reply does.
    assert_eq!(g(&[0x69, 0x15, 0x01, 0x01, 0x01, 0x01, 0x07]), Some(Grounds::ThirdParty));
}

#[test]
fn the_connection_mode_is_read_at_both_of_its_types_and_no_other() {
    // PLAN 23, ITEM 4. HEADER ONLY at 0x02, where the RET, SET and NTFY all
    // pin the type; REFERENCE CLIENT at 0x00, where libmdr reads exactly this.
    // No capture holds either -- the WF-1000XM5 advertises neither 0xE1 nor
    // 0xE3 -- so these bytes are synthetic.
    use payload::PriorMode;
    for (bytes, kind, prior, word) in [
        (&[0xE7u8, 0x02, 0x00][..], 0x02u8, PriorMode::SoundQuality, "quality"),
        (&[0xE7, 0x02, 0x01], 0x02, PriorMode::ConnectionQuality, "stable"),
        (&[0xE9, 0x02, 0x01], 0x02, PriorMode::ConnectionQuality, "stable"),
        (&[0xE7, 0x00, 0x00], 0x00, PriorMode::SoundQuality, "quality"),
        (&[0xE9, 0x00, 0x01], 0x00, PriorMode::ConnectionQuality, "stable"),
        // LOW_LATENCY_PRIOR_BETA has no word in the C++ daemon's document.
        (&[0xE7, 0x02, 0x02], 0x02, PriorMode::LowLatencyBeta, "unknown"),
    ] {
        assert_eq!(payload::decode(MDR, bytes), Reading::Connection { kind, prior }, "{bytes:02x?}");
        assert_eq!(prior.as_str(), word);
    }
    assert_eq!(payload::grounds(MDR, &[0xE7, 0x02, 0x01]), Some(payload::Grounds::HeaderOnly));
    assert_eq!(payload::grounds(MDR, &[0xE7, 0x00, 0x01]), Some(payload::Grounds::ReferenceClient));
    // Every connection struct pins `settingValue` to CONNECTION_QUALITY_PRIOR
    // (T1:2807, 2820) and libmdr's validation drops anything else, so a
    // reading of any other value stands on less than its key: against the
    // header, at either type and volunteered or asked.
    for bytes in [&[0xE7u8, 0x02, 0x00][..], &[0xE7, 0x00, 0x00], &[0xE9, 0x02, 0x00], &[0xE7, 0x02, 0x02]] {
        assert_eq!(payload::grounds(MDR, bytes), Some(payload::Grounds::AgainstHeader), "{bytes:02x?}");
    }
    assert_eq!(payload::Grounds::AgainstHeader.as_str(), "against-header");

    // The LE Audio form (0x05) is four bytes with the unexplained `value2`,
    // and is not read; nor is a three-byte reply at any other type, nor the
    // same bytes on table 2.
    for bytes in [&[0xE7u8, 0x05, 0x00, 0x00][..], &[0xE7, 0x03, 0x00], &[0xE7, 0x02, 0x00, 0x00]] {
        assert!(matches!(payload::decode(MDR, bytes), Reading::Unhandled { .. }), "{bytes:02x?}");
    }
    assert!(matches!(payload::decode(DataType::DataMdrNo2, &[0xE7, 0x02, 0x00]), Reading::Unhandled { .. }));
    // And the DSEE param reply at UPSCALING is still DSEE: the type, not the
    // command, separates the two.
    assert!(matches!(payload::decode(MDR, &[0xE7, 0x01, 0x01]), Reading::Dsee(_)));
}

#[test]
fn table_two_voice_guidance_safe_listening_and_pairing_read_as_the_reference_client_reads_them() {
    // PLAN 23, ITEM 11. The bytes are the corpus's; the meaning is the
    // header's types and libmdr's reading of them, so the grounds say
    // reference-client and not captured.
    const T2: DataType = DataType::DataMdrNo2;
    use payload::{SafeListeningReport, VoiceGuidanceReport};

    // Voice guidance at 0x01: OnOffSettingValue, INVERTED, then the language.
    let vg = payloads_for("VOICE_GUIDANCE_RET_PARAM");
    assert!(vg.contains(&vec![0x47, 0x01, 0x00, 0x01]), "{vg:02x?}");
    assert_eq!(
        payload::decode(T2, &[0x47, 0x01, 0x00, 0x01]),
        Reading::VoiceGuidance(VoiceGuidanceReport { enabled: Some(Switch::On), language: Some(0x01), volume: None })
    );
    assert_eq!(payload::voice_guidance_language(0x01), "english");
    assert_eq!(payload::voice_guidance_language(0xF0), "chinese");
    assert_eq!(payload::voice_guidance_language(0x0C), "unknown", "0x0C is a hole in the enum");
    // 0x01 in the switch is OFF -- the trap the old refusal was about.
    match payload::decode(T2, &[0x47, 0x01, 0x01, 0x0B]) {
        Reading::VoiceGuidance(r) => assert_eq!(r.enabled, Some(Switch::Off)),
        other => panic!("decoded as {other:?}"),
    }
    // The three-byte forms carry the switch alone, at 0x00..=0x03.
    for kind in 0x00u8..=0x03 {
        match payload::decode(T2, &[0x47, kind, 0x00]) {
            Reading::VoiceGuidance(r) => assert_eq!((r.enabled, r.language), (Some(Switch::On), None)),
            other => panic!("0x{kind:02x} decoded as {other:?}"),
        }
    }

    // The volume: an Int8 bounded -2..=2 by the header, zero centred.
    assert!(vg.contains(&vec![0x47, 0x20, 0x00]), "{vg:02x?}");
    for (byte, want) in [(0x00u8, 0i8), (0x02, 2), (0xFE, -2), (0xFF, -1)] {
        match payload::decode(T2, &[0x47, 0x20, byte]) {
            Reading::VoiceGuidance(r) => assert_eq!(r.volume, Some(want), "{byte:#04x}"),
            other => panic!("{byte:#04x} decoded as {other:?}"),
        }
    }
    for byte in [0x03u8, 0x7F, 0x80, 0xFD] {
        assert!(matches!(payload::decode(T2, &[0x47, 0x20, byte]), Reading::Unhandled { .. }), "{byte:#04x}");
    }

    // Safe listening: raw, both bytes, at the four safe-listening types only.
    let sl = payloads_for("SAFE_LISTENING_RET_EXTENDED_PARAM");
    assert_eq!(sl, vec![vec![0x5B, 0x03, 0xFF, 0x00]]);
    let want = SafeListeningReport { level: 0xFF, cause: 0x00 };
    assert_eq!(payload::decode(T2, &sl[0]), Reading::SafeListening(want));
    assert_eq!(want.cause_word(), "not_playing");
    assert!(matches!(payload::decode(T2, &[0x5B, 0x04, 0x00, 0x00]), Reading::Unhandled { .. }));

    // Pairing mode: INQUIRY_SCAN and ENABLE, both, and nothing else.
    let peri = payloads_for("PERI_RET_STATUS");
    assert_eq!(peri, vec![vec![0x33, 0x02, 0x00, 0x00]]);
    assert_eq!(payload::decode(T2, &peri[0]), Reading::PairingMode { on: false });
    assert_eq!(payload::decode(T2, &[0x33, 0x02, 0x01, 0x00]), Reading::PairingMode { on: true });
    assert_eq!(payload::decode(T2, &[0x35, 0x02, 0x01, 0x00]), Reading::PairingMode { on: true }, "NTFY");
    assert_eq!(payload::decode(T2, &[0x33, 0x02, 0x01, 0x01]), Reading::PairingMode { on: false });
    assert!(matches!(payload::decode(T2, &[0x33, 0x02, 0x02, 0x00]), Reading::Unhandled { .. }));
    assert!(matches!(payload::decode(T2, &[0x33, 0x02, 0x01, 0x07]), Reading::Unhandled { .. }));

    // A volunteered table-2 change stands where its reply does, as on table
    // 1: PERI_NTFY_STATUS on PERI_RET_STATUS's grounds, and the corpus's
    // paired-device list (000077, PERI_RET_PARAM, captured) sent as
    // PERI_NTFY_PARAM on the same grounds as the reply that was captured.
    assert_eq!(payload::grounds(T2, &[0x35, 0x02, 0x00, 0x00]), Some(payload::Grounds::ReferenceClient), "NTFY");
    let list = payloads_for("PERI_RET_PARAM");
    assert_eq!(list.len(), 1);
    assert_eq!(payload::grounds(T2, &list[0]), Some(payload::Grounds::Captured));
    let mut volunteered = list[0].clone();
    volunteered[0] = 0x39;
    assert!(matches!(payload::decode(T2, &volunteered), Reading::PairedDevices(_)));
    assert_eq!(payload::grounds(T2, &volunteered), Some(payload::Grounds::Captured), "PERI_NTFY_PARAM");
    assert_eq!(payload::key(T2, &volunteered), payload::key(T2, &list[0]));
    for bytes in [&[0x47u8, 0x01, 0x00, 0x01][..], &[0x47, 0x20, 0x00], &[0x5B, 0x03, 0xFF, 0x00], &[0x33, 0x02, 0x00, 0x00]] {
        assert_eq!(payload::grounds(T2, bytes), Some(payload::Grounds::ReferenceClient), "{bytes:02x?}");
        // TABLE ONE IS ANOTHER NAMESPACE: the same bytes there are not these.
        assert!(
            !matches!(
                payload::decode(MDR, bytes),
                Reading::VoiceGuidance(_) | Reading::SafeListening(_) | Reading::PairingMode { .. }
            ),
            "{bytes:02x?} on table 1"
        );
    }
}

#[test]
fn an_alert_is_read_raw_and_named_from_the_header() {
    // PLAN 23, ITEM 14. `99 00 <AlertMessageType> <AlertActionType>`, at
    // FIXED_MESSAGE only. No capture holds one: the headset asked nothing
    // while the corpus was recorded (`decode_alert` has why that, and not
    // alerts being off, is the likely reason).
    assert_eq!(payload::decode(MDR, &[0x99, 0x00, 0x07, 0x01]), Reading::Alert { message: 0x07, action: 0x01 });
    assert_eq!(payload::grounds(MDR, &[0x99, 0x00, 0x07, 0x01]), Some(payload::Grounds::ReferenceClient));
    // Another alert type, another length, or table 2: not this message.
    for (table, bytes) in [
        (MDR, &[0x99u8, 0x02, 0x07, 0x01][..]),
        (MDR, &[0x99, 0x00, 0x07]),
        (DataType::DataMdrNo2, &[0x99, 0x00, 0x07, 0x01]),
    ] {
        assert!(matches!(payload::decode(table, bytes), Reading::Unhandled { .. }), "{bytes:02x?}");
    }

    // The names are the header's, transcribed whole: 107 declared values.
    assert_eq!(payload::alert_message_name(0x07), Some("DISCONNECT_CAUSED_BY_CHANGING_MULTIPOINT"));
    assert_eq!(payload::alert_message_name(0x95), Some("CAUTION_FOR_USB_OVERHEAT_DETECTION_DURING_CASE_IN"));
    assert_eq!(payload::alert_message_name(0x04), None, "0x04 is a hole in the enum");
    assert_eq!((0u8..=255).filter(|b| payload::alert_message_name(*b).is_some()).count(), 107);
    assert_eq!(payload::alert_action_word(0x01), "positive_negative");
    assert_eq!(payload::alert_action_word(0x07), "unknown");

    // Which answers drop the link: every name that says DISCONNECT or
    // RECONNECTION anywhere in it, bar the one about the LE (GATT) link.
    assert!(payload::alert_disconnects(0x07));
    assert!(payload::alert_disconnects(0x02), "NEED_DISCONNECTION_FOR_UPDATING_FIRMWARE");
    assert!(
        payload::alert_disconnects(0x0B),
        "CAUTION_FOR_DISABLE_TOUCH_SENSOR_PANEL_AND_RECONNECTION -- a WH-1000XM5's touch panel"
    );
    assert!(payload::alert_disconnects(0x19), "FOREGROUND_CAUTION_NEED_DISCONNECTION_FOR_ENABLING_WAKE_WORD");
    assert!(!payload::alert_disconnects(0x1F), "CAUTION_FOR_GATT_DISCONNECTION_FOR_SAR is the LE link");
    assert!(!payload::alert_disconnects(0x08), "a battery caution is not a disconnect");
    assert!(!payload::alert_disconnects(0x04), "an undeclared byte promises nothing");
    // THE WHOLE SET, over every byte, so the rule cannot drift by one name
    // without this failing. Read off T1:98-207 by the rule above: 0x0B, 0x0D,
    // 0x10, 0x11, 0x14, 0x15 end `..._AND_RECONNECTION...`; 0x19 says
    // NEED_DISCONNECTION; the rest are DISCONNECT_* and NEED_DISCONNECTION_*.
    let want: Vec<u8> = vec![
        0x00, 0x01, 0x02, 0x06, 0x07, 0x0B, 0x0D, 0x10, 0x11, 0x14, 0x15, 0x19, 0x24, 0x25, 0x26, 0x27, 0x28,
        0x2C, 0x2D, 0x2E, 0x2F, 0x30, 0x31, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x40, 0x41, 0x56, 0x57, 0x58,
        0x74, 0x75, 0x76, 0x77,
    ];
    let got: Vec<u8> = (0u8..=255).filter(|b| payload::alert_disconnects(*b)).collect();
    assert_eq!(got, want);
}
