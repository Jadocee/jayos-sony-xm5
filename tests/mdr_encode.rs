//! The write path, against the only evidence an `-rx`-only corpus can offer.
//!
//! THE ROUND TRIP IS NOT SELF-CONSISTENCY. For five of the seven device verbs
//! the SET struct is field-for-field identical to its RET twin. So: take a
//! frame a real WF-1000XM5 sent, decode it, ask the encoder to re-assert what
//! was decoded, and the payload must come back byte-identical to the fixture
//! with byte 0 swapped for the SET command. The decoder in the middle was
//! written from captures and agrees with the headset byte for byte, so the
//! chain reaches real hardware even though nothing here can transmit.
//!
//! WHAT IT CANNOT REACH is stated as loudly. `playback` has no field-identical
//! twin and is not round-tripped by anything. Its offset-2 byte is the one
//! byte in the module that is neither named by a verb nor echoed from a
//! reading, and no test here can tell you it is right.

use sony_xm5::mdr::encode::{self, Evidence, Facts, Refusal, Side};
use sony_xm5::mdr::payload::{self, Command, Reading};
use sony_xm5::mdr::{frame, DataType};
use sony_xm5::sony::command::{Command as Verb, EQ_PRESETS};
use std::fs;
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/mdr"))
}

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

/// The whole capture folded in, in the order it arrived.
fn captured_facts() -> Facts {
    let mut facts = Facts::blank();
    for f in corpus() {
        facts.observe(f.data_type, &f.payload);
    }
    facts
}

/// One fixture's payload, by counter.
fn payload_of(counter: &str) -> Vec<u8> {
    let needle = format!("-{counter}-");
    let name = fs::read_dir(fixture_dir())
        .expect("the fixture directory is there")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.ends_with(".bin") && n.contains(&needle))
        .unwrap_or_else(|| panic!("no fixture {counter}"));
    let bytes = fs::read(fixture_dir().join(name)).expect("readable");
    frame::unpack(&bytes).expect("a frame").payload
}

/// The fixture's bytes with byte 0 advanced from RET to SET.
fn as_set(counter: &str) -> Vec<u8> {
    let mut payload = payload_of(counter);
    payload[0] += 1;
    payload
}

#[test]
fn the_command_families_are_numbered_in_fours() {
    // GET = base, RET = base+1, SET = base+2, NTFY = base+3. Checked against
    // the reference over all 152 declared commands: it holds for every one of
    // the 91 derivable pairs. `Outgoing::ret_twin` is arithmetic because of
    // this, rather than a table that could fall out of step -- so pin the
    // instances this port depends on.
    for (get, ret, set, ntfy) in [
        (0x66u8, 0x67u8, 0x68u8, 0x69u8), // NCASM_*_PARAM
        (0x56, 0x57, 0x58, 0x59),         // EQEBB_*_PARAM
        (0xA6, 0xA7, 0xA8, 0xA9),         // PLAY_*_PARAM
        (0xA2, 0xA3, 0xA4, 0xA5),         // PLAY_*_STATUS
        (0xE6, 0xE7, 0xE8, 0xE9),         // AUDIO_*_PARAM
        (0xF6, 0xF7, 0xF8, 0xF9),         // SYSTEM_*_PARAM
        (0xFA, 0xFB, 0xFC, 0xFD),         // SYSTEM_*_EXT_PARAM
        (0x22, 0x23, 0x24, 0x25),         // POWER_*_STATUS
    ] {
        assert_eq!(ret, get + 1);
        assert_eq!(set, get + 2);
        assert_eq!(ntfy, get + 3);
        // And the mapping the decoder already relies on is the same law.
        assert_eq!(Command::from_byte(ntfy).canonical().to_byte(), ret, "0x{ntfy:02x}");
    }
}

#[test]
fn every_set_is_its_ret_with_one_byte_changed() {
    // The load-bearing test. Each verb re-asserts exactly what the fixture
    // said, so the emitted payload must equal the fixture's with byte 0
    // advanced by one. Goes red if any offset, length, polarity or echoed
    // byte drifts.
    let facts = captured_facts();

    // 000069: 67 17 01 00 01 00 14 -- settled, effect off, mode ASM, normal,
    // level 20. Re-asserting "off" reproduces it exactly.
    let noise = encode::encode(&facts, &Verb::Noise("off".into())).expect("a noise frame");
    assert_eq!(noise.payload, as_set("000069"));

    // 000081: f7 0c 01 01 -- both OnOffSettingValue OFF. Note the polarity:
    // asking for `speak-to-chat off` must write 0x01, not 0x00.
    let stc = encode::encode(&facts, &Verb::SpeakToChat(false)).expect("a speak-to-chat frame");
    assert_eq!(stc.payload, as_set("000081"));

    // 000101: e7 01 01 -- UpscalingTypeAutoOff AUTO, the ordinary way round,
    // so `dsee on` writes 0x01.
    let dsee = encode::encode(&facts, &Verb::Dsee(true)).expect("a dsee frame");
    assert_eq!(dsee.payload, as_set("000101"));

    // 000093: 57 00 a0 06 14 0b 0a 0d 09 0c -- preset CUSTOM and six steps.
    // The only write-direction exercise of an MDRPodArray in this tree: it
    // goes red if the count byte moves, stops counting, or the steps are
    // truncated or re-derived rather than echoed.
    let eq = encode::encode(&facts, &Verb::EqualizerPreset("custom".into())).expect("an eq frame");
    assert_eq!(eq.payload, as_set("000093"));

    // 000061: a7 20 0c -- MUSIC_VOLUME, 12 of 30. 40% round-trips to 12.
    let volume = encode::encode(&facts, &Verb::Volume(40)).expect("a volume frame");
    assert_eq!(volume.payload, as_set("000061"));

    // And all five say which fixture they stand on.
    for out in [&noise, &stc, &dsee, &eq, &volume] {
        assert_eq!(out.evidence.tier(), "mirrored-capture");
        assert_eq!(out.data_type, DataType::DataMdr);
        assert_eq!(out.ret_twin(), out.command().map(|c| c - 1));
    }
}

#[test]
fn the_emitted_frame_decodes_back_to_what_was_asked_for() {
    // The transitive half: swap byte 0 back and put it through the REAL
    // decoder. This is what makes the encoder's correctness rest on captures
    // rather than on itself.
    let facts = captured_facts();

    type Check = fn(&Reading) -> bool;
    let cases: Vec<(Verb, Check)> = vec![
        (Verb::Noise("anc".into()), |r| {
            matches!(r, Reading::Noise(n) if n.mode == payload::NoiseMode::Anc)
        }),
        (Verb::Noise("ambient".into()), |r| {
            matches!(r, Reading::Noise(n) if n.mode == payload::NoiseMode::Ambient)
        }),
        (Verb::Noise("off".into()), |r| {
            matches!(r, Reading::Noise(n) if n.mode == payload::NoiseMode::Off)
        }),
        (Verb::AmbientLevel(7), |r| {
            matches!(r, Reading::Noise(n) if n.ambient_level == Some(7))
        }),
        (Verb::FocusOnVoice(true), |r| {
            matches!(r, Reading::Noise(n) if n.focus_on_voice == Some(true))
        }),
        (Verb::SpeakToChat(true), |r| {
            matches!(r, Reading::SpeakToChat(s) if s.enabled == Some(payload::Switch::On))
        }),
        (Verb::SpeakToChat(false), |r| {
            matches!(r, Reading::SpeakToChat(s) if s.enabled == Some(payload::Switch::Off))
        }),
        (Verb::Dsee(true), |r| {
            matches!(r, Reading::Dsee(d) if d.enabled == Some(payload::Switch::On))
        }),
        (Verb::Dsee(false), |r| {
            matches!(r, Reading::Dsee(d) if d.enabled == Some(payload::Switch::Off))
        }),
        (Verb::EqualizerPreset("rock".into()), |r| {
            matches!(r, Reading::Equalizer(e) if e.preset == 0x01)
        }),
        (Verb::EqualizerPreset("gaming".into()), |r| {
            matches!(r, Reading::Equalizer(e) if e.preset == 0x20)
        }),
        (Verb::EqualizerPreset("heavy".into()), |r| {
            matches!(r, Reading::Equalizer(e) if e.preset == 0x30)
        }),
        (Verb::Volume(100), |r| matches!(r, Reading::Volume { device_level: 30 })),
        (Verb::Volume(0), |r| matches!(r, Reading::Volume { device_level: 0 })),
        // 000093 carries steps 14 0b 0a 0d 09 0c -- clear bass +10, then the
        // five bands +1, 0, +3, -1, +2, every one at bias ten. A write goes
        // out through the same decoder and must come back as the whole array
        // with ONE element moved.
        (Verb::EqBand { index: 3, value: 2 }, |r| {
            matches!(r, Reading::Equalizer(e)
                if e.steps == vec![0x14, 0x0b, 0x0a, 0x0c, 0x09, 0x0c] && e.preset == 0xa0)
        }),
        (Verb::EqBand { index: 1, value: -10 }, |r| {
            matches!(r, Reading::Equalizer(e)
                if e.steps == vec![0x14, 0x00, 0x0a, 0x0d, 0x09, 0x0c])
        }),
        (Verb::EqBand { index: 5, value: 10 }, |r| {
            matches!(r, Reading::Equalizer(e)
                if e.steps == vec![0x14, 0x0b, 0x0a, 0x0d, 0x09, 0x14])
        }),
        (Verb::ClearBass(-3), |r| {
            matches!(r, Reading::Equalizer(e)
                if e.steps == vec![0x07, 0x0b, 0x0a, 0x0d, 0x09, 0x0c])
        }),
        // The general settings are named by the DEVICE, so the round trip has
        // to go through a `Facts` that has heard the capability reply -- which
        // `captured_facts()` has, from fixtures 000031 and 000039.
        (Verb::GeneralSetting { key: "multipoint".into(), on: true }, |r| {
            matches!(r, Reading::GeneralSettingParam { slot: 0xD2, enabled: true })
        }),
        (Verb::GeneralSetting { key: "multipoint".into(), on: false }, |r| {
            matches!(r, Reading::GeneralSettingParam { slot: 0xD2, enabled: false })
        }),
        (Verb::GeneralSetting { key: "sidetone".into(), on: false }, |r| {
            matches!(r, Reading::GeneralSettingParam { slot: 0xD1, enabled: false })
        }),
        (Verb::ClearBass(0), |r| {
            matches!(r, Reading::Equalizer(e)
                if e.steps == vec![0x0a, 0x0b, 0x0a, 0x0d, 0x09, 0x0c])
        }),
    ];

    for (verb, wanted) in cases {
        let out = encode::encode(&facts, &verb).unwrap_or_else(|e| panic!("{verb:?}: {}", e.message()));
        let mut back = out.payload.clone();
        back[0] = out.ret_twin().expect("a twin");
        let reading = payload::decode(out.data_type, &back);
        assert!(wanted(&reading), "{verb:?} came back as {reading:?}");
    }
}

#[test]
fn a_changed_value_moves_exactly_the_byte_it_should() {
    // An encoder that ignored its argument would pass the round trip above by
    // re-emitting the fixture. This pins which byte each verb owns.
    let facts = captured_facts();
    let base = as_set("000069");

    for (verb, offset) in [
        (Verb::Noise("anc".into()), 4usize),  // ncAsmMode, and effect at 3
        (Verb::FocusOnVoice(true), 5),        // ambientSoundMode
        (Verb::AmbientLevel(12), 6),          // ambientSoundLevelValue
    ] {
        let out = encode::encode(&facts, &verb).expect("a frame");
        let differing: Vec<usize> =
            (0..base.len()).filter(|i| out.payload[*i] != base[*i]).collect();
        assert!(
            differing.contains(&offset),
            "{verb:?} must move offset {offset}; it moved {differing:?}"
        );
        // `noise anc` legitimately moves the total-effect byte as well; the
        // other two must touch one byte and no more.
        let allowed: &[usize] = if offset == 4 { &[3, 4] } else { &[offset] };
        for i in &differing {
            assert!(allowed.contains(i), "{verb:?} also moved offset {i}");
        }
    }

    // The equaliser message is a different fixture and a different shape, so
    // it gets its own base. 000093 as a SET is
    //
    //     58 00 a0 06 14 0b 0a 0d 09 0c
    //      0  1  2  3  4  5  6  7  8  9
    //           |  |  \_ clear bass    \_ band 5
    //           |  \_ the count
    //           \_ the preset
    //
    // so clear bass owns offset 4 and band N owns offset 4 + N.
    let eq_base = as_set("000093");
    for (verb, offset) in [
        (Verb::ClearBass(-3), 4usize),
        (Verb::EqBand { index: 1, value: -10 }, 5),
        (Verb::EqBand { index: 2, value: 4 }, 6),
        (Verb::EqBand { index: 3, value: 2 }, 7),
        (Verb::EqBand { index: 4, value: 5 }, 8),
        (Verb::EqBand { index: 5, value: 10 }, 9),
    ] {
        let out = encode::encode(&facts, &verb).expect("an eq frame");
        let differing: Vec<usize> =
            (0..eq_base.len()).filter(|i| out.payload[*i] != eq_base[*i]).collect();
        assert_eq!(
            differing,
            vec![offset],
            "{verb:?} must move offset {offset} and nothing else; it moved {differing:?}"
        );
    }

    // AND THE PRESET IS NOT COLLATERAL. A band write that also rewrote the
    // preset byte would be two edits where one was asked for -- and since this
    // device family discards a preset write, it would be one edit and one
    // ignored one, which is worse than either. Offset 2 is the preset; the
    // loop above already forbids it moving, and this says why in a sentence a
    // failure will print.
    let out = encode::encode(&facts, &Verb::EqBand { index: 3, value: 2 }).expect("an eq frame");
    assert_eq!(out.payload[2], eq_base[2], "a band write changed the preset");
    assert_eq!(out.payload[3], eq_base[3], "a band write changed the band count");
}

#[test]
fn a_band_write_carries_the_whole_curve_because_the_message_has_no_short_form() {
    // THE TRAP THIS PORT CANNOT FALL INTO, recorded as an assertion because
    // somebody reading upstream's document will come looking for the
    // workaround and find none.
    //
    // delarosa1312/omarchy-sony-xm5 (MIT), docs/protocol.md, "Equalizer: bands
    // and clear bass are one message": clear bass is the first element of the
    // band array on the wire, and libmdr's staging model lets half a change go
    // out -- a clear-bass-only edit is committed locally and never sent, and a
    // bands-only edit ships whatever clear bass was last staged, so clear bass
    // slides a step every time the bands are touched. Their `write_eq()`
    // restates both every time to defeat it.
    //
    // Here `EqParam { EqPresetId; MDRPodArray<UInt8> }` has no short form, so
    // the whole array goes out or nothing does. This asserts that directly:
    // every step the verb did not name comes back byte-identical to what the
    // headset last sent.
    let facts = captured_facts();
    let captured = payload_of("000093");
    let steps = &captured[4..]; // past 57 00 <preset> <count>
    assert_eq!(steps.len(), 6, "the fixture is not the six-step shape this test is about");

    for (verb, moved) in [
        (Verb::ClearBass(-3), 0usize),
        (Verb::EqBand { index: 2, value: -6 }, 2),
        (Verb::EqBand { index: 5, value: 0 }, 5),
    ] {
        let out = encode::encode(&facts, &verb).expect("an eq frame");
        let sent = &out.payload[4..];
        assert_eq!(sent.len(), steps.len(), "{verb:?} changed the length of the curve");
        for i in 0..steps.len() {
            if i == moved {
                assert_ne!(sent[i], steps[i], "{verb:?} did not move step {i}");
            } else {
                assert_eq!(sent[i], steps[i], "{verb:?} disturbed step {i}, which it does not name");
            }
        }
    }
}

#[test]
fn the_equaliser_bounds_come_from_the_array_the_headset_sent() {
    // Not from a constant. The captured device sends six steps, so five bands
    // at -10..=+10 and a clear bass at the same -- and band 6 does not exist.
    // `sony::command` lets 1..=10 and -10..=10 through, because it cannot know
    // the shape at parse time; this layer is where the device's own answer is
    // applied.
    let facts = captured_facts();

    for verb in [
        Verb::EqBand { index: 6, value: 0 },
        Verb::EqBand { index: 10, value: 0 },
        Verb::EqBand { index: 0, value: 0 },
    ] {
        match encode::encode(&facts, &verb) {
            Err(Refusal::OutOfRange { bound, .. }) => {
                assert!(bound.contains("band"), "{verb:?} refused with {bound:?}");
            }
            other => panic!("{verb:?} was accepted on a five-band headset: {other:?}"),
        }
    }

    // The edges are legal, which is the half that stops "refuse everything"
    // from passing the test above.
    for verb in [
        Verb::EqBand { index: 1, value: -10 },
        Verb::EqBand { index: 5, value: 10 },
        Verb::ClearBass(-10),
        Verb::ClearBass(10),
    ] {
        encode::encode(&facts, &verb)
            .unwrap_or_else(|e| panic!("{verb:?} was refused at an edge: {}", e.message()));
    }
}

#[test]
fn an_unnamed_band_array_is_refused_rather_than_guessed_at() {
    // `sony::session` shows an array of some other length as bands at bias
    // ten, by analogy, because a curve that might be off by a constant beats
    // no curve. Writing one back is the other thing entirely: the bias is a
    // guess and the wire would carry a curve nobody asked for. `EqShape` is
    // the one table that says which lengths are named, and this is the writer
    // half of that rule.
    let mut facts = captured_facts();
    // A seven-step reply: nothing in libmdr or Gadgetbridge names that shape.
    facts.observe(
        DataType::DataMdr,
        &[0x57, 0x00, 0xa0, 0x07, 0x0a, 0x0a, 0x0a, 0x0a, 0x0a, 0x0a, 0x0a],
    );
    for verb in [Verb::EqBand { index: 1, value: 1 }, Verb::ClearBass(1)] {
        match encode::encode(&facts, &verb) {
            Err(Refusal::UnknownVariant { observed, .. }) => {
                assert_eq!(observed, 7, "{verb:?} named the wrong length");
            }
            other => panic!("{verb:?} wrote a curve at a guessed bias: {other:?}"),
        }
    }

    // A ten-step reply IS named -- ten bands at bias six, and no clear bass at
    // all -- so the bands are writable there and clear bass is refused as a
    // thing this headset does not have, rather than written into band one.
    let mut ten = captured_facts();
    ten.observe(
        DataType::DataMdr,
        &[0x57, 0x00, 0xa0, 0x0a, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06],
    );
    let out = encode::encode(&ten, &Verb::EqBand { index: 10, value: -6 }).expect("a ten-band frame");
    assert_eq!(out.payload[13], 0x00, "band ten at -6 is byte zero when the bias is six");
    // BOTH EDGES, and the positive one is the load-bearing half. A mutation
    // that replaced the device's bound with a constant -10..=10 was caught on
    // the negative edge ALONE, by the `u8::try_from(value + bias)` further
    // down: -10 at bias 6 is -4, which is not a byte. +10 at bias 6 is 16,
    // which is a perfectly good byte and four steps past the maximum this
    // device accepts -- so it went out, and every test passed. This is that
    // hole.
    for value in [-10, -7, 7, 10] {
        match encode::encode(&ten, &Verb::EqBand { index: 10, value }) {
            Err(Refusal::OutOfRange { bound, .. }) => assert!(bound.contains("-6"), "{bound:?}"),
            other => panic!("a ten-band headset accepted a step of {value}: {other:?}"),
        }
    }
    // The six-step device is the negative control for that: the same values
    // are legal there, so "refuse anything past six" would fail here.
    //
    // A FRESH `captured_facts()`, not the `facts` above -- that one has had a
    // seven-step reply folded into it and is no longer the five-band device.
    // The first draft of this reused it and failed on the unmutated tree,
    // which is the friendlier half of that mistake.
    let five = captured_facts();
    for value in [-10, -7, 7, 10] {
        encode::encode(&five, &Verb::EqBand { index: 5, value })
            .unwrap_or_else(|e| panic!("a five-band headset refused {value}: {}", e.message()));
    }
    match encode::encode(&ten, &Verb::ClearBass(1)) {
        Err(Refusal::NoEncoder { why, .. }) => assert!(why.contains("clear bass"), "{why:?}"),
        other => panic!("clear bass was written to a headset whose array has none: {other:?}"),
    }
}

#[test]
fn nothing_is_emitted_without_the_state_it_must_echo() {
    // The mechanical form of the no-defaults rule. A SET is a whole-state
    // message, so an encoder that filled the unnamed fields from the header's
    // struct defaults would emit `68 17 01 01 00 00 00` for `noise anc` --
    // zeroing the ambient level and switching focus-on-voice off every time
    // somebody changed their noise mode -- and no other test would notice.
    let blank = Facts::blank();
    for verb in [
        Verb::Noise("anc".into()),
        Verb::AmbientLevel(10),
        Verb::FocusOnVoice(true),
        Verb::SpeakToChat(true),
        Verb::Dsee(true),
        Verb::EqualizerPreset("rock".into()),
        Verb::Volume(50),
    ] {
        match encode::encode(&blank, &verb) {
            Err(Refusal::NotRead { .. }) | Err(Refusal::NotAdvertised { .. }) => {}
            other => panic!("{verb:?} produced {other:?} from a headset that has said nothing"),
        }
    }
    // `playback` needs no echo -- it carries no state -- so it is the one
    // device verb that encodes from nothing, and that is stated rather than
    // accidental.
    assert!(encode::encode(&blank, &Verb::Playback("play".into())).is_ok());
}

#[test]
fn the_inquired_type_is_never_a_constant() {
    // Hardcoding 0x17 would be a guess about a device nobody here has: the
    // corpus is a WF-1000XM5 and this tree targets a WH. Feed a headset that
    // answers NC/ASM at other declared types and require a refusal that names
    // what it saw.
    for kind in [0x11u8, 0x12, 0x16, 0x19, 0x21] {
        let mut facts = Facts::blank();
        // A NCASM_RET_PARAM at `kind` that this port cannot decode. The kind
        // is still recorded, which is what turns "you have not asked" into
        // "your headset answered at a type I have no layout for".
        facts.observe(DataType::DataMdr, &[0x67, kind, 0, 0, 0, 0, 0]);
        match encode::encode(&facts, &Verb::Noise("anc".into())) {
            Err(Refusal::UnknownVariant { observed, .. }) => assert_eq!(observed, kind),
            other => panic!("type 0x{kind:02x} produced {other:?}"),
        }
    }

    // 0x13 is different: BOTH SET structs are seven bytes and they read
    // offset 6 differently, so it is a genuine reference collision -- and one
    // the READ side does not have, because there the two RET structs are six
    // and seven fields. Refused by name, never guessed.
    let mut facts = Facts::blank();
    facts.observe(DataType::DataMdr, &[0x67, 0x13, 0, 0, 0, 0, 0]);
    match encode::encode(&facts, &Verb::Noise("anc".into())) {
        Err(Refusal::Ambiguous { between, command, kind, len, .. }) => {
            assert_eq!((command, kind, len), (0x68, 0x13, 7));
            assert_eq!(between, ["NcAsmSetParamNcOnOffAsmOnOff", "NcAsmSetParamNcOnOffAsmSeamless"]);
        }
        other => panic!("the 0x13 collision produced {other:?}"),
    }
}

#[test]
fn no_emitted_payload_decodes_as_a_reading_without_the_twin_swap() {
    // A SET command byte must not collide with a RET one. If it did, a frame
    // this port sent could be mistaken for a reply from the headset.
    let facts = captured_facts();
    for verb in every_verb() {
        let Ok(out) = encode::encode(&facts, &verb) else { continue };
        let reading = payload::decode(out.data_type, &out.payload);
        assert!(
            matches!(reading, Reading::Unhandled { .. }),
            "{verb:?} emits bytes the decoder reads as {reading:?}"
        );
    }
}

#[test]
fn the_captured_device_gates_the_verbs_it_should() {
    let facts = captured_facts();
    // Everything the WF-1000XM5's own support tables say it can do.
    for verb in [
        Verb::Noise("anc".into()),
        Verb::AmbientLevel(10),
        Verb::FocusOnVoice(true),
        Verb::SpeakToChat(true),
        Verb::Dsee(true),
        Verb::EqualizerPreset("rock".into()),
        Verb::Volume(50),
        Verb::Playback("next".into()),
    ] {
        assert!(encode::encode(&facts, &verb).is_ok(), "{verb:?} should encode");
    }

    // And the one that is refused by name, on this device and every other.
    match encode::encode(&facts, &Verb::ConnectionPriority("quality".into())) {
        Err(Refusal::Undocumented { field, struct_name, .. }) => {
            assert_eq!(field, "value2");
            assert_eq!(struct_name, "AudioSetParamConnectionModeClassicAudioLeAudio");
        }
        other => panic!("connection produced {other:?}"),
    }

    // A headset that answered but advertised nothing gets a different
    // refusal from one that advertised but has not answered -- the two are
    // not the same problem and must not read the same.
    let mut mute = Facts::blank();
    mute.observe(DataType::DataMdr, &payload_of("000101"));
    assert!(matches!(
        encode::encode(&mute, &Verb::Dsee(true)),
        Err(Refusal::NotAdvertised { .. })
    ));
}

#[test]
fn a_refusal_says_which_side_the_gap_is_on() {
    let facts = captured_facts();
    let blank = Facts::blank();
    let mut seen: Vec<String> = Vec::new();
    for source in [&facts, &blank] {
        for verb in every_verb() {
            let Err(refusal) = encode::encode(source, &verb) else { continue };
            let text = refusal.message();
            match refusal.side() {
                Side::Port => assert!(text.contains("this daemon"), "{text}"),
                Side::Device => assert!(text.contains("this headset"), "{text}"),
            }
            seen.push(text);
        }
    }
    assert!(seen.len() >= 8, "only {} refusals exercised", seen.len());
}

#[test]
fn every_verb_gets_an_answer() {
    // Exhaustive rather than fuzzed: the verb vocabulary is closed. Every
    // verb, against three kinds of headset, must return without panicking.
    let mut nonsense = Facts::blank();
    nonsense.observe(DataType::DataMdr, &[0x67, 0xAB, 9, 9, 9, 9, 9]);
    let sources = [Facts::blank(), captured_facts(), nonsense];
    let mut answered = 0;
    for facts in &sources {
        for verb in every_verb() {
            let _ = encode::encode(facts, &verb);
            answered += 1;
        }
    }
    assert!(answered >= 60, "only {answered} cases");
}

#[test]
fn an_emitted_payload_survives_the_frame_layer() {
    // Nothing hand-builds a frame: everything the encoder emits goes through
    // `frame::pack` and must come back out of `frame::unpack` unchanged.
    let facts = captured_facts();
    let mut packed = 0;
    for verb in every_verb() {
        let Ok(out) = encode::encode(&facts, &verb) else { continue };
        for seq in [0u8, 1] {
            let wire = frame::pack(out.data_type, seq, &out.payload);
            let back = frame::unpack(&wire).expect("its own frame unpacks");
            assert_eq!(back.payload, out.payload);
            assert_eq!(back.data_type, out.data_type);
            assert_eq!(back.seq, seq);
            packed += 1;
        }
    }
    assert!(packed >= 40, "only {packed} frames packed");

    // AND THE ESCAPE PATH IS NOT EXERCISED HERE, which is worth stating rather
    // than leaving to be noticed. No frame this encoder can currently emit
    // contains 0x3C, 0x3D or 0x3E, in its payload or in its checksum: the
    // value spaces are too small. Working the arithmetic for the widest case,
    // `ambient-level N` gives a checksum of 0x94 + seq + N, so it would take
    // N = 169 to land on the sentry -- far outside the 1..=20 the command
    // language accepts.
    //
    // That is a fact about today's verbs, not a guarantee, so it is asserted
    // rather than assumed. When it stops being true this test says so, and
    // the round trip above already covers the case. The escape path itself is
    // tested next door in `mdr_frame.rs`, against a real fixture that contains
    // one.
    let mut escaping = Vec::new();
    for verb in every_verb() {
        let Ok(out) = encode::encode(&facts, &verb) else { continue };
        for seq in [0u8, 1] {
            let wire = frame::pack(out.data_type, seq, &out.payload);
            if wire.len() != out.payload.len() + frame::MIN_FRAME {
                escaping.push((verb.clone(), seq));
            }
        }
    }
    assert!(
        escaping.is_empty(),
        "an emitted frame now escapes: {escaping:?} -- the comment above is stale, \
         and the round trip above is what actually covers it"
    );
}

#[test]
fn the_write_gate_is_one_table() {
    // Two tables of the same thing drift. The encoder resolves a preset only
    // by its position in `sony::command::EQ_PRESETS`, so the two must stay
    // the same length -- and every name must produce a frame.
    let facts = captured_facts();
    let mut ids = Vec::new();
    for name in EQ_PRESETS {
        let out = encode::encode(&facts, &Verb::EqualizerPreset(name.to_string()))
            .unwrap_or_else(|e| panic!("{name}: {}", e.message()));
        ids.push(out.payload[2]);
    }
    assert_eq!(ids.len(), 30);
    // No two names may share a preset byte.
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), 30, "two preset names share a byte");

    // The four pairs an index-into-the-enum map would swap. `EQ_PRESETS`
    // lists heavy..soft before gaming..fps_3; `EqPresetId` declares them the
    // other way round.
    for (name, want) in
        [("heavy", 0x30u8), ("clear", 0x31), ("hard", 0x32), ("soft", 0x33), ("gaming", 0x20),
         ("fps_1", 0x21), ("fps_2", 0x22), ("fps_3", 0x23)]
    {
        let at = EQ_PRESETS.iter().position(|p| *p == name).expect("a known preset");
        assert_eq!(ids[at], want, "{name}");
    }
}

#[test]
fn the_polarity_inverses_are_literal() {
    // Not a round-trip property -- that passes for a doubly inverted pair --
    // but the bytes themselves.
    use payload::Switch;
    assert_eq!(Switch::On.to_on_off_setting(), Some(0x00));
    assert_eq!(Switch::Off.to_on_off_setting(), Some(0x01));
    assert_eq!(Switch::On.to_auto_off(), Some(0x01));
    assert_eq!(Switch::Off.to_auto_off(), Some(0x00));
    assert_eq!(Switch::Unknown(0xFF).to_on_off_setting(), None);
    assert_eq!(Switch::Unknown(0xFF).to_auto_off(), None);
    // And each is the inverse of the constructor it names.
    for byte in [0x00u8, 0x01] {
        assert_eq!(Switch::from_on_off_setting(byte).to_on_off_setting(), Some(byte));
        assert_eq!(Switch::from_auto_off(byte).to_auto_off(), Some(byte));
    }
}

#[test]
fn the_header_only_verb_says_so() {
    // `playback` is the one verb with no field-identical twin, so nothing can
    // mirror it. It must not claim otherwise.
    let out = encode::encode(&Facts::blank(), &Verb::Playback("pause".into())).expect("a frame");
    assert_eq!(out.evidence.tier(), "header-only");
    assert!(matches!(out.evidence, Evidence::HeaderOnly { .. }));
    // PlaybackControl: PAUSE = 0x01, PLAY = 0x07, TRACK_UP = 0x02,
    // TRACK_DOWN = 0x03. The reference restricts the field to exactly those
    // four, which is what settles next/previous as TRACK_* and not GROUP_*.
    for (word, want) in [("play", 0x07u8), ("pause", 0x01), ("next", 0x02), ("previous", 0x03)] {
        let out = encode::encode(&Facts::blank(), &Verb::Playback(word.to_string())).expect("a frame");
        assert_eq!(out.payload, vec![0xA4, 0x01, 0x00, want], "{word}");
    }
}

/// Every verb the command language can produce, with a representative
/// argument for each.
fn every_verb() -> Vec<Verb> {
    let mut out = vec![
        Verb::Status,
        Verb::PanelOpen,
        Verb::PanelClose,
        Verb::Refresh,
        Verb::FocusOnVoice(true),
        Verb::FocusOnVoice(false),
        Verb::SpeakToChat(true),
        Verb::SpeakToChat(false),
        Verb::Dsee(true),
        Verb::Dsee(false),
    ];
    for word in ["off", "anc", "ambient"] {
        out.push(Verb::Noise(word.to_string()));
    }
    for level in [1, 10, 20] {
        out.push(Verb::AmbientLevel(level));
    }
    for percent in [0, 40, 100] {
        out.push(Verb::Volume(percent));
    }
    for word in ["play", "pause", "next", "previous"] {
        out.push(Verb::Playback(word.to_string()));
    }
    for word in ["quality", "stable"] {
        out.push(Verb::ConnectionPriority(word.to_string()));
    }
    for name in EQ_PRESETS {
        out.push(Verb::EqualizerPreset(name.to_string()));
    }
    out
}

#[test]
fn every_get_has_the_shape_its_struct_declares_and_asks_at_a_real_type() {
    let facts = captured_facts();
    let mut all = encode::handshake();
    all.extend(encode::feature_gets(&facts));
    assert!(all.len() >= 20, "only {} queries", all.len());
    for query in &all {
        // Two bytes, command and inquired type -- except GsGetCapability
        // (T1:4129), which also says which language to name the setting in,
        // and asks for ENGLISH (0x01) as the reference client does.
        if query.command() == Some(0xD0) {
            assert_eq!(query.payload.len(), 3, "{query:?}");
            assert_eq!(query.payload[2], 0x01, "GsGetCapability asks in English: {query:?}");
        } else {
            assert_eq!(query.payload.len(), 2, "{query:?}");
        }
        // Every GET command is even, and its reply is the odd byte above it:
        // GET = base, RET = base+1. So a GET whose command were odd would be
        // asking with a reply's byte.
        let command = query.command().expect("a command");
        assert_eq!(command % 2, 0, "0x{command:02x} is not a GET");
        // A GET's reply is command + 1. `ret_twin` is a SET concept -- the
        // reply to a SET is command - 1 -- and does not apply here.
        assert!(command.checked_add(1).is_some(), "0x{command:02x} has no reply byte");
    }
    // The support-function query goes out on BOTH channels, because the two
    // tables are separate namespaces and the device answers each separately.
    let support: Vec<DataType> =
        all.iter().filter(|q| q.command() == Some(0x06)).map(|q| q.data_type).collect();
    assert_eq!(support, vec![DataType::DataMdr, DataType::DataMdrNo2]);
}

#[test]
fn the_queries_ask_for_every_family_the_corpus_answers() {
    // The cross-check that makes this more than a transcription, run in the
    // OPPOSITE direction to `mdr_support_functions.rs`. That test asserts every
    // frame in the corpus belongs to an advertised function; this one asserts
    // the queries derived from those same tables would have asked for it.
    //
    // A reply in the corpus that nothing here asks for is the falsifier.
    let facts = captured_facts();
    let mut asked: Vec<(u8, u8, DataType)> = Vec::new();
    for query in encode::handshake().into_iter().chain(encode::feature_gets(&facts)) {
        let (Some(command), Some(kind)) = (query.command(), query.kind()) else { continue };
        // The reply's command is the query's plus one.
        asked.push((command + 1, kind, query.data_type));
    }

    // Everything the corpus contains a reply for, with the exceptions named.
    let mut unasked: Vec<(u8, u8)> = Vec::new();
    for frame in corpus() {
        let (Some(&command), Some(&kind)) = (frame.payload.first(), frame.payload.get(1)) else {
            continue;
        };
        // ACKs carry no payload; NTFY frames are volunteered, not asked for.
        if frame.data_type == DataType::Ack {
            continue;
        }
        // SAFE_LISTENING, PERI and VOICE_GUIDANCE are table 2 families this
        // port does not read, so it does not ask for them either. Nothing on
        // table 1 is exempt: COMMON_RET_STATUS answers to CODEC_INDICATOR and
        // the codec query below must cover it.
        if [0x5Bu8, 0x33, 0x37, 0x47].contains(&command) {
            continue;
        }
        if !asked.iter().any(|(c, k, t)| *c == command && *k == kind && *t == frame.data_type) {
            unasked.push((command, kind));
        }
    }
    unasked.sort_unstable();
    unasked.dedup();
    assert!(
        unasked.is_empty(),
        "the corpus answers messages nothing asks for: {unasked:02x?}"
    );
}

#[test]
fn nothing_is_asked_that_the_device_did_not_advertise() {
    // The other direction, and it needs saying separately: the test above
    // catches a reply nothing asks for, and would happily pass an encoder that
    // asked for everything. A query the device never advertised is a round
    // trip spent to be told nothing -- and a reply this port would then have
    // to decide how to read.
    assert!(
        encode::feature_gets(&Facts::blank()).is_empty(),
        "a headset that has said nothing is asked nothing"
    );

    let facts = captured_facts();
    let gets = encode::feature_gets(&facts);

    // The battery case is the sharpest. This device advertises
    // LR_BATTERY_LEVEL_WITH_THRESHOLD (0x29) and
    // CRADLE_BATTERY_LEVEL_WITH_THRESHOLD (0x2A) and NOT
    // BATTERY_LEVEL_INDICATOR (0x20) -- so exactly two POWER_GET_STATUS
    // queries go out, at 0x09 and 0x0A, and none at 0x00.
    let mut battery: Vec<u8> =
        gets.iter().filter(|q| q.command() == Some(0x22)).filter_map(|q| q.kind()).collect();
    battery.sort_unstable();
    assert_eq!(battery, vec![0x09, 0x0A], "the battery queries the tables justify");

    // And the same shape for the system settings: one query per advertised
    // function, at the SystemInquiredType it answers at -- which is not the
    // function code.
    let mut system: Vec<u8> =
        gets.iter().filter(|q| q.command() == Some(0xF6)).filter_map(|q| q.kind()).collect();
    system.sort_unstable();
    assert_eq!(system, vec![0x01, 0x03, 0x06, 0x0C, 0x0F], "auto-pause, assignable, wear, S2C, gesture");

    // The codec: one COMMON_GET_STATUS, at AUDIO_CODEC, on table 1, because
    // this device advertises CODEC_INDICATOR (t1 0x12).
    let codec: Vec<(u8, DataType)> =
        gets.iter().filter(|q| q.command() == Some(0x12)).map(|q| (q.kind().unwrap(), q.data_type)).collect();
    assert_eq!(codec, vec![(0x02, DataType::DataMdr)]);
    // Gated on that function and not on a neighbour: a table 1 carrying only
    // 0x12 is asked, one carrying only UPSCALING_INDICATOR (0x13) is not.
    assert!(
        encode::feature_gets(&only_table_one(&[0x12])).iter().any(|q| q.command() == Some(0x12) && q.kind() == Some(0x02)),
        "CODEC_INDICATOR alone did not earn the codec query"
    );
    assert!(
        encode::feature_gets(&only_table_one(&[0x13])).iter().all(|q| q.command() != Some(0x12)),
        "UPSCALING_INDICATOR is not the codec gate"
    );

    // No query is asked twice.
    let mut keys: Vec<(u8, u8)> =
        gets.iter().filter_map(|q| Some((q.command()?, q.kind()?))).collect();
    let before = keys.len();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), before, "a query is asked twice");
}

/// Facts from a synthetic table-1 support list carrying only `codes`.
fn only_table_one(codes: &[u8]) -> Facts {
    let mut payload = vec![0x07, 0x00, codes.len() as u8];
    for code in codes {
        payload.extend_from_slice(&[*code, 0xFF]);
    }
    let mut facts = Facts::blank();
    facts.observe(DataType::DataMdr, &payload);
    facts
}

#[test]
fn an_over_ear_headset_is_asked_for_its_one_battery_and_not_for_three() {
    // PLAN 23, ITEM 1, the asking half. A WH-1000XM5 advertises a single
    // battery (Gadgetbridge's coordinator says `BatterySingle`; libmdr's
    // RequestSyncV2 asks BATTERY for BATTERY_LEVEL_INDICATOR and
    // BATTERY_WITH_THRESHOLD for BATTERY_LEVEL_WITH_THRESHOLD). Whichever of
    // the two it advertises, POWER_GET_STATUS goes out at that type and at no
    // earbud type -- the corpus's two batteries answer at 0x09 and 0x0A, and a
    // headband that was asked for those would be asked about parts it has not
    // got.
    for (function, kind) in [(0x20u8, 0x00u8), (0x28, 0x08)] {
        let gets = encode::feature_gets(&only_table_one(&[function]));
        let power: Vec<Vec<u8>> =
            gets.iter().filter(|g| g.command() == Some(0x22)).map(|g| g.payload.clone()).collect();
        assert_eq!(power, vec![vec![0x22, kind]], "function {function:#04x}: {power:02x?}");
    }
}

/// Facts from a support table and the NC/ASM frames a headset answered with.
fn ncasm_facts(functions: &[u8], frames: &[&[u8]]) -> Facts {
    let mut facts = only_table_one(functions);
    for frame in frames {
        facts.observe(DataType::DataMdr, frame);
    }
    facts
}

#[test]
fn nc_asm_is_asked_at_the_type_the_advertised_function_names() {
    // PLAN 23, ITEM 3, the asking half. libmdr's order, then 0x68 -> 0x15,
    // the type Gadgetbridge drives a WH-1000XM5 at. The capability GET goes
    // out at the same type (item 2).
    let asked = |functions: &[u8]| -> Vec<Vec<u8>> {
        encode::feature_gets(&only_table_one(functions))
            .into_iter()
            .filter(|g| matches!(g.command(), Some(0x66) | Some(0x60)))
            .map(|g| g.payload)
            .collect()
    };
    assert_eq!(asked(&[0x6B]), vec![vec![0x66, 0x17], vec![0x60, 0x17]]);
    assert_eq!(asked(&[0x6D]), vec![vec![0x66, 0x19], vec![0x60, 0x19]]);
    assert_eq!(asked(&[0x67]), vec![vec![0x66, 0x22], vec![0x60, 0x22]]);
    assert_eq!(asked(&[0x68]), vec![vec![0x66, 0x15], vec![0x60, 0x15]]);
    // Both 0x6B and 0x6D: libmdr asks 0x17 first, and so does this.
    assert_eq!(asked(&[0x6D, 0x6B]), vec![vec![0x66, 0x17], vec![0x60, 0x17]]);
    // Nothing NC/ASM advertised: nothing asked.
    assert!(asked(&[0x20]).is_empty());
}

#[test]
fn an_nc_asm_write_mirrors_the_layout_the_headset_answered_in() {
    // PLAN 23, ITEM 3, the writing half: three layouts, each on its own
    // grounds, and the rest refused by name.
    let write = |facts: &Facts, verb: Verb| encode::encode(facts, &verb);

    // 0x19, noise adaptation: five bytes, then the two adaptation fields
    // echoed. ValueChangeStatus is CHANGED even though the reading was
    // caught mid-change (0x00 at offset 2).
    let facts = ncasm_facts(&[0x6D], &[&[0x67, 0x19, 0x00, 0x01, 0x01, 0x00, 0x0C, 0x01, 0x02]]);
    let out = write(&facts, Verb::Noise("anc".into())).expect("it encodes");
    assert_eq!(out.payload, vec![0x68, 0x19, 0x01, 0x01, 0x00, 0x00, 0x0C, 0x01, 0x02]);
    assert_eq!(out.evidence.tier(), "reference-client");

    // 0x15, Gadgetbridge's WH-1000XM5 layout: the 0x17 offsets at 0x15.
    let facts = ncasm_facts(&[0x68], &[&[0x67, 0x15, 0x01, 0x01, 0x00, 0x00, 0x0A]]);
    let out = write(&facts, Verb::FocusOnVoice(true)).expect("it encodes");
    assert_eq!(out.payload, vec![0x68, 0x15, 0x01, 0x01, 0x00, 0x01, 0x0A]);
    assert_eq!(out.evidence.tier(), "third-party");

    // 0x17, the captured layout, is unchanged except that a mid-change
    // reading no longer becomes a mid-change write.
    let facts = ncasm_facts(&[0x6B], &[&[0x67, 0x17, 0x00, 0x01, 0x01, 0x00, 0x14]]);
    let out = write(&facts, Verb::AmbientLevel(5)).expect("it encodes");
    assert_eq!(out.payload, vec![0x68, 0x17, 0x01, 0x01, 0x01, 0x00, 0x05]);
    assert_eq!(out.evidence.tier(), "mirrored-capture");

    // 0x22 and 0x01 are read, and not written.
    for frame in [&[0x67u8, 0x22, 0x01, 0x01, 0x00, 0x05][..], &[0x67, 0x01, 0x01, 0x01, 0x01][..]] {
        let facts = ncasm_facts(&[0x67], &[frame]);
        assert!(
            matches!(write(&facts, Verb::Noise("off".into())), Err(Refusal::UnknownVariant { .. })),
            "{frame:02x?} was written"
        );
    }
}

#[test]
fn the_ambient_level_is_held_to_the_range_the_headset_stated() {
    // PLAN 23, ITEM 2. A headset that says its ambient level runs 1..=12 in
    // NORMAL and 2..=10 in steps of two with focus on voice is held to that.
    let reading: &[u8] = &[0x67, 0x17, 0x01, 0x01, 0x01, 0x00, 0x05];
    let capability: &[u8] = &[0x61, 0x17, 0x02, 0x00, 0x01, 0x0C, 0x01, 0x01, 0x02, 0x0A, 0x02];
    let facts = ncasm_facts(&[0x6B], &[reading, capability]);

    assert!(encode::encode(&facts, &Verb::AmbientLevel(12)).is_ok());
    let refused = encode::encode(&facts, &Verb::AmbientLevel(15)).expect_err("15 is past the headset's 12");
    assert!(matches!(&refused, Refusal::OutOfDeviceRange { min: 1, max: 12, step: 1, .. }), "{refused:?}");
    assert_eq!(refused.clone().side(), Side::Device);
    assert!(refused.message().contains("1 to 12"), "{}", refused.message());

    // With focus on voice the VOICE range applies, step and all.
    let voiced = ncasm_facts(&[0x6B], &[&[0x67, 0x17, 0x01, 0x01, 0x01, 0x01, 0x04], capability]);
    assert!(encode::encode(&voiced, &Verb::AmbientLevel(6)).is_ok());
    let odd = encode::encode(&voiced, &Verb::AmbientLevel(5)).expect_err("5 is off the step of two");
    assert!(odd.message().contains("in steps of 2"), "{}", odd.message());

    // A range stated at another type is not evidence about this one, and
    // with no range the compatibility bound applies.
    let other = ncasm_facts(&[0x6B], &[reading, &[0x61, 0x19, 0x01, 0x00, 0x01, 0x05, 0x01]]);
    assert!(encode::encode(&other, &Verb::AmbientLevel(20)).is_ok(), "a 0x19 range bound a 0x17 write");
    let none = ncasm_facts(&[0x6B], &[reading]);
    assert!(encode::encode(&none, &Verb::AmbientLevel(20)).is_ok());
    assert!(matches!(
        encode::encode(&none, &Verb::AmbientLevel(21)),
        Err(Refusal::OutOfDeviceRange { min: 1, max: 20, .. })
    ));
}
