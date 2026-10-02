//! The session engine, driven by a real headset.
//!
//! THE CORPUS IS THE SCRIPT. 70 frames a Sony WF-1000XM5 actually sent, in
//! capture order, fed to the engine as though the socket had delivered them.
//! Nothing is mocked: the bytes are the device's, the decoder in the middle was
//! written from those same bytes, and the document that comes out is the one a
//! real machine would have published.
//!
//! WHAT THE SCRIPT CANNOT DO, and it is worth naming before the assertions:
//! it never disconnects, never drops a frame, never repeats one, never sends
//! garbage and never stops answering. So the tests that matter most here are
//! the ones the corpus CANNOT produce -- a reconnect, a stray ACK, a flood of
//! junk -- and those are built by hand below rather than replayed.
//!
//! The script is also OPEN-LOOP on purpose. It does not read what the engine
//! transmits and it never answers it. A fake that replied to our queries would
//! prove our queries were right, and every fixture is `-rx`: the corpus cannot
//! support that claim and this module must not make it.

use sony_xm5::mdr::{frame, DataType};
use sony_xm5::sony::session::{ClientId, Down, Effect, Input, Limits, Note, Peer, Session};
use std::fs;
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/mdr"))
}

/// Every capture, as raw wire bytes, in capture order.
fn script() -> Vec<Vec<u8>> {
    let mut names: Vec<String> = fs::read_dir(fixture_dir())
        .expect("the fixture directory is there")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".bin"))
        .collect();
    // The counter is the capture order, and it is zero-padded, so a plain sort
    // is chronological.
    names.sort();
    names.iter().filter_map(|n| fs::read(fixture_dir().join(n)).ok()).collect()
}

fn peer() -> Peer {
    Peer { name: "WF-1000XM5".to_string(), protocol: 2 }
}

fn fresh() -> Session {
    Session::new("qs-test".to_string(), Limits::default())
}

/// Run the whole script through a session, returning every effect in order.
fn replay(session: &mut Session, chunk: usize) -> Vec<Effect> {
    let mut all = Vec::new();
    all.extend(session.step(Input::LinkUp { now: 0, peer: peer() }));
    let mut now = 1u64;
    for wire in script() {
        for piece in wire.chunks(chunk.max(1)) {
            all.extend(session.step(Input::Bytes { now, bytes: piece }));
            now += 1;
        }
    }
    all
}

fn sent(effects: &[Effect]) -> Vec<Vec<u8>> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Send(bytes) => Some(bytes.clone()),
            _ => None,
        })
        .collect()
}

fn notes(effects: &[Effect]) -> Vec<&Note> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Note(n) => Some(n),
            _ => None,
        })
        .collect()
}

#[test]
fn the_whole_capture_produces_one_document_and_no_surprises() {
    let mut session = fresh();
    let effects = replay(&mut session, 4096);

    // Nothing in a clean capture is unreadable at the FRAME level.
    for note in notes(&effects) {
        assert!(!matches!(note, Note::BadFrame(_)), "a captured frame did not decode: {note:?}");
    }

    // STRAY ACKS ARE EXPECTED HERE, and saying why is the point of this
    // comment. The script is open-loop: its 36 ACKs were the headset
    // acknowledging the SONY APP's frames, not ours, and our query sequence is
    // our own. So an ACK arriving with nothing outstanding is the script
    // running ahead of us, not a defect -- and the engine noting it rather
    // than swallowing it is the behaviour under test.
    //
    // What must NOT happen is the reverse: an ACK credited to a frame it does
    // not acknowledge. `an_ack_is_credited_only_to_a_frame_in_flight` builds
    // that case by hand, because the corpus alternates perfectly and cannot.
    let strays = notes(&effects).iter().filter(|n| matches!(n, Note::StrayAck { .. })).count();
    let acks_in_script = script()
        .iter()
        .filter(|w| frame::unpack(w).map(|f| f.data_type == DataType::Ack).unwrap_or(false))
        .count();
    assert!(
        strays < acks_in_script,
        "every ACK in the script was stray, so none of our frames was ever acknowledged"
    );

    let document = session.document();
    assert!(document.ends_with("}\n"), "one JSON object per LINE");
    assert_eq!(document.matches('\n').count(), 1, "exactly one newline, at the end");
    let value: serde_json::Value =
        serde_json::from_str(&document).expect("the document is valid JSON");
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["connected"], true);
    assert_eq!(value["phase"], "ready");

    // Values a real WF-1000XM5 reported, end to end: frame, decode, project.
    assert_eq!(value["device"]["model"], "WF-1000XM5");
    assert_eq!(value["device"]["firmware"], "6.1.0");
    assert_eq!(value["device"]["name"], "WF-1000XM5");
    // Fixture 000053: `13 02 02`, COMMON_RET_STATUS at AUDIO_CODEC, AAC.
    assert_eq!(value["device"]["codec"], "aac");
    assert!(
        !notes(&effects).iter().any(|n| matches!(n, Note::Unread { command: 0x13, .. })),
        "the codec reply the corpus holds went unread"
    );
    // The buds: left, right and case, and never a main battery.
    assert_eq!(value["battery"]["left"]["present"], true);
    assert_eq!(value["battery"]["left"]["level"], 0);
    assert_eq!(value["battery"]["right"]["level"], 100);
    assert_eq!(value["battery"]["case"]["level"], 53);
    assert_eq!(value["battery"]["main"]["present"], false);
    assert_eq!(value["battery"]["main"]["level"], -1);
    // Both captured battery frames carry charging byte 0x00, and the document
    // says it in the C++ daemon's word -- the one the page compares against.
    assert_eq!(value["battery"]["left"]["charging"], "no");
    assert_eq!(value["battery"]["case"]["charging"], "no");
    assert_eq!(value["battery"]["main"]["charging"], "unknown", "absent, so unknown");
    // The NC/ASM frame said the whole feature is off.
    assert_eq!(value["noise"]["mode"], "off");
    assert_eq!(value["noise"]["ambient_level"], 20);
    // 12 of 30 is 40%.
    assert_eq!(value["playback"]["volume"], 40);
    // 57 00 a0 06 14 0b 0a 0d 09 0c: preset 0xA0 is CUSTOM; step 0 is clear
    // bass and steps 1-5 the bands, all biased +10 (libmdr HeadphonesV2T1.cpp,
    // the EqEbbRetParamEq arm; Gadgetbridge SonyProtocolImplV1.handleEqualizer).
    assert_eq!(value["equalizer"]["preset"], "custom");
    assert_eq!(value["equalizer"]["clear_bass"], 10);
    assert_eq!(value["equalizer"]["bands"], serde_json::json!([1, 0, 3, -1, 2]));
    assert!(
        value["equalizer"]["bands"].as_array().unwrap().iter().all(|b| (-10..=10).contains(&b.as_i64().unwrap())),
        "the contract's scale is -10..10"
    );
    assert_eq!(value["equalizer"]["dsee_enabled"], true);
    // e1 01 02: UpscalingType::DSEE_HX_AI, in the C++ daemon's word.
    assert_eq!(value["equalizer"]["dsee_type"], "hx_ai");
    // Speak-to-Chat is OFF on this device, and the polarity is inverted.
    assert_eq!(value["speak_to_chat"]["enabled"], false);
    assert_eq!(value["speak_to_chat"]["sensitivity"], "auto");
    assert_eq!(value["speak_to_chat"]["timeout"], "short");
    // The capability flags, from the device's own two support tables.
    assert_eq!(value["capabilities"]["battery_left_right"], true);
    assert_eq!(value["capabilities"]["battery_single"], false);
    assert_eq!(value["capabilities"]["connection_mode"], false);
    assert_eq!(value["capabilities"]["safe_listening"], true);
}

#[test]
fn every_data_frame_is_acknowledged_and_no_ack_ever_is() {
    // The acknowledgement rule, against the corpus's own ACK strings.
    let mut session = fresh();
    let effects = replay(&mut session, 4096);

    let mut expected = 0usize;
    for wire in script() {
        let f = frame::unpack(&wire).expect("a captured frame");
        if f.data_type.is_data() {
            expected += 1;
        }
    }
    assert!(expected > 0, "the corpus has no data frames; the script is broken");

    let acks: Vec<Vec<u8>> = sent(&effects)
        .into_iter()
        .filter(|w| frame::unpack(w).map(|f| f.data_type == DataType::Ack).unwrap_or(false))
        .collect();
    assert_eq!(acks.len(), expected, "one ACK per data frame, and not one more");

    // And every ACK we emit is byte-identical to one the headset itself sent,
    // which is a stronger statement than "it round-trips": there are exactly
    // two possible ACK frames and both are in the corpus.
    let theirs: Vec<Vec<u8>> = script()
        .into_iter()
        .filter(|w| frame::unpack(w).map(|f| f.data_type == DataType::Ack).unwrap_or(false))
        .collect();
    for ack in &acks {
        assert!(theirs.contains(ack), "emitted an ACK the headset never sent: {ack:02x?}");
    }
}

#[test]
fn the_stream_is_read_the_same_at_every_chunk_size() {
    // A socket returns what it returns. The engine must not care whether a
    // frame arrives whole, in two pieces, or one byte at a time -- including a
    // split inside an escape sequence, which one fixture in this corpus has.
    let whole = {
        let mut s = fresh();
        let e = replay(&mut s, 4096);
        (s.document(), sent(&e))
    };
    for chunk in [1usize, 2, 3, 7, 13, 64, 512] {
        let mut s = fresh();
        let e = replay(&mut s, chunk);
        assert_eq!(s.document(), whole.0, "chunk size {chunk} produced a different document");
        assert_eq!(sent(&e), whole.1, "chunk size {chunk} transmitted different bytes");
    }
}

#[test]
fn a_reconnect_forgets_what_the_last_link_said() {
    // THE INVARIANT THE CORPUS CANNOT TEST, because it never disconnects.
    //
    // encode.rs emits every byte either named by the verb or echoed from a
    // reading the headset sent, and a SET is a whole-state message with no
    // "unchanged" sentinel. Carrying Facts across a redial would echo a mode,
    // an ambient level and a focus-on-voice setting from a reading THIS link
    // never produced -- into a message that overwrites all three.
    let mut session = fresh();
    replay(&mut session, 4096);
    // Mid-session the device verbs are ACCEPTED, because the headset has
    // answered. Not necessarily transmitted in the same step: this is
    // stop-and-wait, and a query may still be in flight, so the test for
    // acceptance is the absence of a refusal rather than the presence of bytes.
    let out = session.step(Input::Command { now: 100, id: ClientId(1), line: "noise anc" });
    let refusals: Vec<&str> = out
        .iter()
        .filter_map(|e| match e {
            Effect::Reply { line, .. } if line.starts_with("error") => Some(line.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        refusals.is_empty(),
        "a headset that has answered refused a command it should take: {refusals:?}"
    );
    session.step(Input::LinkDown { now: 200, why: Down::Closed });
    session.step(Input::LinkUp { now: 300, peer: peer() });

    // After a redial and before the new headset has said anything, the same
    // command must be refused rather than answered from the old link's memory.
    let out = session.step(Input::Command { now: 400, id: ClientId(2), line: "noise anc" });
    let replies: Vec<&str> = out
        .iter()
        .filter_map(|e| match e {
            Effect::Reply { line, .. } => Some(line.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        replies.iter().any(|r| r.starts_with("error")),
        "a fresh link answered from the previous link's Facts: {replies:?}"
    );
    // And the document has forgotten the old device's readings too.
    let value: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
    assert_eq!(value["device"]["model"], "", "a redial kept the last headset's model");
    assert_eq!(value["battery"]["left"]["present"], false);
}

#[test]
fn an_ack_is_credited_only_to_a_frame_in_flight() {
    // The corpus alternates perfectly and never loses an ACK, so an engine that
    // cleared whatever was pending on ANY ack would replay it cleanly and drop
    // a frame the first time a real link misbehaved.
    let mut session = fresh();
    session.step(Input::LinkUp { now: 0, peer: peer() });

    // Nothing is in flight but the handshake's first frame, sent at seq 0 and
    // acknowledged by an ACK carrying 1. An ACK carrying 0 acknowledges seq 1,
    // which is not what is outstanding.
    let out = session.step(Input::Bytes { now: 1, bytes: &frame::ack_frame(1) });
    assert!(
        notes(&out).iter().any(|n| matches!(n, Note::StrayAck { .. })),
        "an ACK for a sequence nothing is waiting on was accepted"
    );

    // The right one is credited, and the next queued frame goes out.
    let out = session.step(Input::Bytes { now: 2, bytes: &frame::ack_frame(0) });
    assert!(!sent(&out).is_empty(), "a correct ACK did not release the next frame");
}

#[test]
fn the_engine_survives_what_the_corpus_never_sends() {
    // Junk, a truncated frame, a frame with a bad checksum, and a flood. None
    // of these appears in a capture; all of them appear on a socket.
    let mut session = fresh();
    session.step(Input::LinkUp { now: 0, peer: peer() });

    session.step(Input::Bytes { now: 1, bytes: &[0u8; 4096] });
    session.step(Input::Bytes { now: 2, bytes: b"not a frame at all" });
    // A start marker and nothing else, forever.
    for i in 0..64 {
        session.step(Input::Bytes { now: 3 + i, bytes: &[0x3E; 1024] });
    }
    // A well-formed frame whose checksum is wrong.
    let mut corrupt = frame::pack(DataType::DataMdr, 0, &[0x67, 0x17, 1, 0, 1, 0, 20]);
    let n = corrupt.len();
    corrupt[n - 2] ^= 0xFF;
    session.step(Input::Bytes { now: 200, bytes: &corrupt });
    // A real frame carrying an empty device-controlled array.
    session.step(Input::Bytes {
        now: 201,
        bytes: &frame::pack(DataType::DataMdr, 0, &[0x57, 0x00, 0xA0, 0x00]),
    });

    // Still answering, still serialising.
    let document = session.document();
    let _: serde_json::Value = serde_json::from_str(&document).expect("still valid JSON");
    let out = session.step(Input::Command { now: 300, id: ClientId(9), line: "status" });
    assert!(out.iter().any(|e| matches!(e, Effect::Reply { line, .. } if line.starts_with("ok {"))));
}

#[test]
fn the_four_self_verbs_are_answered_rather_than_refused() {
    // encode::encode returns Refusal::NoEncoder for these, which is true and
    // useless: they are not gaps in the port, they are simply not wire
    // messages. The engine answers them itself.
    let mut session = fresh();
    for (n, verb) in ["status", "panel-open", "panel-close"].iter().enumerate() {
        let out = session.step(Input::Command {
            now: n as u64,
            id: ClientId(n as u64),
            line: verb,
        });
        let replies: Vec<&str> = out
            .iter()
            .filter_map(|e| match e {
                Effect::Reply { line, .. } => Some(line.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(replies.len(), 1, "{verb} was not answered exactly once: {replies:?}");
        if *verb == "status" {
            // `ok ` and the document, as the C++ daemon_main answers -- with
            // no newline of its own, because the socket adds the line's.
            let line = replies[0];
            assert!(line.starts_with("ok {\"schema_version\":1,"), "{line}");
            assert!(!line.ends_with('\n'));
            assert_eq!(line.matches('\n').count(), 0);
            let v: serde_json::Value = serde_json::from_str(&line[3..]).expect("ok then JSON");
            assert_eq!(v["phase"], "idle");
        } else {
            assert_eq!(replies, vec!["ok"], "{verb} was not answered");
        }
    }
    // Asked again with nothing changed in between, the reply IS the document
    // -- the same bytes a reader of the file sees, minus its newline.
    let before = session.document();
    let out = session.step(Input::Command { now: 5, id: ClientId(5), line: "status" });
    let line = out
        .iter()
        .find_map(|e| match e {
            Effect::Reply { line, .. } => Some(line.clone()),
            _ => None,
        })
        .expect("a reply");
    assert_eq!(line, format!("ok {}", before.trim_end()));
    assert_eq!(session.document(), before, "answering status changed nothing");
    // refresh needs a headset, and says so rather than pretending.
    let out = session.step(Input::Command { now: 9, id: ClientId(9), line: "refresh" });
    assert!(out.iter().any(|e| matches!(e, Effect::Reply { line, .. } if line.starts_with("error"))));
}

#[test]
fn a_command_the_language_does_not_have_is_refused_by_the_parser() {
    let mut session = fresh();
    session.step(Input::LinkUp { now: 0, peer: peer() });
    for line in ["", "nonsense", "noise sideways", "volume 400", "ambient-level 0", "noise"] {
        let out = session.step(Input::Command { now: 1, id: ClientId(1), line });
        let replies: Vec<&str> = out
            .iter()
            .filter_map(|e| match e {
                Effect::Reply { line, .. } => Some(line.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(replies.len(), 1, "{line:?} produced {replies:?}");
        assert!(replies[0].starts_with("error"), "{line:?} was accepted: {replies:?}");
    }
}

#[test]
fn the_document_is_published_when_it_changes_and_not_when_it_does_not() {
    // The revision counts CHANGES, not frames: a reader uses it to tell a
    // re-read that found something new from one that did not.
    let mut session = fresh();
    let effects = replay(&mut session, 4096);
    let publishes = effects.iter().filter(|e| matches!(e, Effect::Publish)).count();
    let frames = script().len();
    assert!(publishes > 1, "nothing was ever published");
    assert!(publishes < frames, "every frame published, so the revision counts frames");

    // Feeding the same frame twice changes nothing the second time.
    let before = session.document();
    let repeat = frame::pack(DataType::DataMdr, 0, &[0xA7, 0x20, 0x0C]);
    let out = session.step(Input::Bytes { now: 900, bytes: &repeat });
    assert!(
        !out.iter().any(|e| matches!(e, Effect::Publish)),
        "an unchanged reading was published as news"
    );
    assert_eq!(session.document(), before);
}

#[test]
fn an_unacknowledged_frame_is_resent_at_the_same_sequence_then_the_link_is_dropped() {
    // The corpus never loses a frame, so this is built by hand. The rule is
    // Gadgetbridge's: resend the identical bytes at the same sequence
    // (initSendRunnable re-encodes with sequenceNumber untouched), twice, and
    // then close the socket -- which here is Effect::Hangup, the transport's
    // cue to shut the link so its reader returns and the dialler redials.
    let mut session = fresh();
    let up = session.step(Input::LinkUp { now: 0, peer: peer() });
    let first = sent(&up).remove(0);
    assert_eq!(frame::unpack(&first).expect("a frame").seq, 0, "the first frame carries seq 0");
    let retries = Limits::default().retries;
    assert!(retries > 0, "the retry rule is under test, so there must be retries");

    // The first deadline is what a client would see, and it stays under
    // qs-sony's own 3s per-operation timeout.
    let mut t = session.deadline().expect("the first handshake frame is in flight");
    assert!(t < 3_000, "the daemon must answer before its client gives up");

    for n in 1..=retries {
        let out = session.step(Input::Tick { now: t - 1 });
        assert!(sent(&out).is_empty(), "retry {n} went out before the deadline");
        let out = session.step(Input::Tick { now: t + 1 });
        assert_eq!(sent(&out), vec![first.clone()], "retry {n}: not the identical bytes and sequence");
        assert!(!out.iter().any(|e| matches!(e, Effect::Hangup)), "hung up before the retries were spent");
        let v: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
        assert_ne!(v["phase"], "error", "gave up before the retries were spent");
        let next = session.deadline().expect("the retransmission is in flight");
        assert_eq!(next, t + 1 + Limits::default().ack_wait, "the deadline moves to sent_at + ack_wait");
        t = next;
    }

    // The next expiry: no further copy, exactly one hang-up, nothing else on
    // the wire in that step -- the rest of the handshake must not be pumped
    // onto a link that is being torn down -- and no deadline, because
    // nothing is in flight any more.
    let out = session.step(Input::Tick { now: t + 1 });
    assert_eq!(out.iter().filter(|e| matches!(e, Effect::Hangup)).count(), 1, "{out:?}");
    assert!(sent(&out).is_empty(), "sent on a link being torn down: {out:?}");
    assert_eq!(session.deadline(), None);
    let v: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
    assert_eq!(v["phase"], "error");
    assert!(v["last_error"].as_str().unwrap_or("").contains("acknowledg"), "{}", v["last_error"]);

    // A further tick asks nothing more of the transport.
    let out = session.step(Input::Tick { now: t + 5_000 });
    assert!(!out.iter().any(|e| matches!(e, Effect::Hangup | Effect::Send(_))), "{out:?}");

    // The transport reports the drop it was asked for. The reason the engine
    // recorded survives, rather than being overwritten with a story about
    // the headset closing the connection.
    session.step(Input::LinkDown { now: t + 5_001, why: Down::HungUp });
    let v: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
    assert_eq!(v["connected"], false);
    assert_eq!(v["phase"], "error");
    assert!(v["last_error"].as_str().unwrap_or("").contains("acknowledg"), "{}", v["last_error"]);

    // And the redial starts clean at seq 0 with the same first frame.
    let again = sent(&session.step(Input::LinkUp { now: t + 7_000, peer: peer() }));
    assert_eq!(again, vec![first]);
}

/// Switching the headphones off is not a fault, and the difference is the
/// whole of what the Headphones page draws.
///
/// jayos's `readSony` (`ts/src/settings/Integration.ts`) turns `phase:
/// "error"` into a red chip with `last_error` as its sentence, and a headset that is
/// simply not there into a green "no headset connected". Which of the two an
/// ended link becomes is decided by the transport -- `qs-sonyd.rs`'s
/// `read_until_gone` answers `Down::Closed` for an orderly end-of-file and
/// `Down::Failed` for a read error. It used to answer a sentence, which the
/// engine wrapped in `Down::Failed` unconditionally, so a headset switched off
/// drew a fault for the ordinary case. This holds both destinations.
#[test]
fn a_headset_switched_off_is_idle_and_a_broken_link_is_an_error() {
    let mut session = fresh();
    session.step(Input::LinkUp { now: 0, peer: peer() });
    session.step(Input::LinkDown { now: 100, why: Down::Closed });
    let v: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
    assert_eq!(v["connected"], false);
    assert_eq!(v["phase"], "idle", "an orderly close drew a fault");
    assert_eq!(v["last_error"], "", "an orderly close left a sentence for the page to show");

    // The other half, so that the assertion above cannot be satisfied by an
    // engine which calls every ended link idle.
    let mut session = fresh();
    session.step(Input::LinkUp { now: 0, peer: peer() });
    let why = Down::Failed("the connection to the headset failed");
    session.step(Input::LinkDown { now: 100, why });
    let v: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
    assert_eq!(v["connected"], false);
    assert_eq!(v["phase"], "error", "a broken link was reported as no headset");
    assert_eq!(v["last_error"], "the connection to the headset failed");
}

#[test]
fn a_reply_that_arrives_while_hanging_up_does_not_reopen_the_link() {
    // Between Effect::Hangup and the transport's LinkDown, bytes already in
    // the socket buffer still reach the engine. A support-function reply
    // among them used to set the phase back to Ready, clear the reason the
    // hang-up recorded, and re-arm the feature queries -- so the LinkDown
    // then carried no reason, and the link could be hung up on twice. The
    // window closes only when the transport reports the link down or up.
    let mut session = fresh();
    session.step(Input::LinkUp { now: 0, peer: peer() });
    let mut out = Vec::new();
    for _ in 0..=Limits::default().retries {
        let t = session.deadline().expect("the first handshake frame is in flight");
        out = session.step(Input::Tick { now: t + 1 });
    }
    assert_eq!(out.iter().filter(|e| matches!(e, Effect::Hangup)).count(), 1, "{out:?}");
    let before: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
    assert_eq!(before["phase"], "error");

    let tables: Vec<Vec<u8>> = script()
        .into_iter()
        .filter(|w| frame::unpack(w).map(|f| f.payload.first() == Some(&0x07)).unwrap_or(false))
        .collect();
    assert_eq!(tables.len(), 2, "both support-function replies");
    let mut now = 20_000u64;
    for wire in &tables {
        let out = session.step(Input::Bytes { now, bytes: wire });
        // Not even the acknowledgement: nothing goes out on a link being
        // torn down, and nothing is asked of the transport twice.
        assert!(!out.iter().any(|e| matches!(e, Effect::Send(_) | Effect::Hangup)), "{out:?}");
        now += 1;
    }
    let after: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
    assert_eq!(after["phase"], "error", "a late reply reopened a link being torn down");
    assert_eq!(after["last_error"], before["last_error"], "a late reply cleared the hang-up's reason");
    assert!(!session.step(Input::Tick { now: now + 10_000 }).iter().any(|e| matches!(e, Effect::Hangup)));

    // The transport's LinkDown carries the reason through, and the next link
    // starts clean -- the window is closed, so its first reply is read.
    session.step(Input::LinkDown { now: now + 10_001, why: Down::HungUp });
    let down: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
    assert!(down["last_error"].as_str().unwrap_or("").contains("acknowledg"), "{}", down["last_error"]);
    session.step(Input::LinkUp { now: now + 12_000, peer: peer() });
    let out = session.step(Input::Bytes { now: now + 12_001, bytes: &tables[0] });
    assert!(!sent(&out).is_empty(), "the new link's first reply was not acknowledged");
}

#[test]
fn an_ack_for_a_retransmitted_frame_is_credited_and_the_next_frame_takes_its_sequence() {
    let mut session = fresh();
    session.step(Input::LinkUp { now: 0, peer: peer() });
    let t = session.deadline().expect("in flight");
    let out = session.step(Input::Tick { now: t + 1 });
    assert_eq!(sent(&out).len(), 1, "one retransmission");

    // ack_frame(0) carries 1 and acknowledges seq 0 -- the sequence the frame
    // was resent at. Credited, not stray; and the next frame goes out at 1.
    let out = session.step(Input::Bytes { now: t + 2, bytes: &frame::ack_frame(0) });
    assert!(
        !notes(&out).iter().any(|n| matches!(n, Note::StrayAck { .. })),
        "the ACK for a retransmitted frame was called stray"
    );
    let next = sent(&out);
    assert_eq!(next.len(), 1, "the ACK did not release the next frame");
    assert_eq!(frame::unpack(&next[0]).expect("a frame").seq, 1, "the next frame must carry the ACK's sequence");
}

/// Acknowledge every data frame the engine just sent, and keep doing so for
/// whatever that releases, until nothing is in flight. The one closed-loop
/// helper in this file, used only to get a command to the head of the queue.
fn acknowledge_all(session: &mut Session, effects: Vec<Effect>, now: &mut u64) {
    let mut pending = effects;
    loop {
        let data: Vec<u8> = sent(&pending)
            .into_iter()
            .filter_map(|w| frame::unpack(&w).ok())
            .filter(|f| f.data_type.is_data())
            .map(|f| f.seq)
            .collect();
        let Some(seq) = data.first().copied() else { return };
        pending = session.step(Input::Bytes { now: *now, bytes: &frame::ack_frame(seq) });
        *now += 1;
    }
}

#[test]
fn a_client_is_answered_at_the_first_deadline_and_the_frame_is_still_retried() {
    // qs-sony gives up after 3s and the daemon's socket after 2s, so a client
    // told at the third expiry would be told nothing. It hears the refusal at
    // the first, and the frame keeps being retried with nobody waiting on it.
    let mut session = fresh();
    let mut now = 1u64;
    let up = session.step(Input::LinkUp { now: 0, peer: peer() });
    acknowledge_all(&mut session, up, &mut now);
    for wire in script() {
        let out = session.step(Input::Bytes { now, bytes: &wire });
        now += 1;
        acknowledge_all(&mut session, out, &mut now);
    }
    assert_eq!(session.deadline(), None, "everything the replay sent was acknowledged");

    let out = session.step(Input::Command { now, id: ClientId(7), line: "noise anc" });
    let command = sent(&out);
    assert_eq!(command.len(), 1, "the command went out at once: {out:?}");
    let t = session.deadline().expect("the command is in flight");

    let out = session.step(Input::Tick { now: t + 1 });
    let replies: Vec<&str> = out
        .iter()
        .filter_map(|e| match e {
            Effect::Reply { id, line } if *id == ClientId(7) => Some(line.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(replies, vec!["error this headset did not acknowledge the command"]);
    assert_eq!(sent(&out), command, "the frame is still retried after the client was answered");

    // Nobody is answered twice: the second expiry retries in silence.
    let t = session.deadline().expect("the retry is in flight");
    let out = session.step(Input::Tick { now: t + 1 });
    assert!(!out.iter().any(|e| matches!(e, Effect::Reply { .. })), "{out:?}");

    // And a client queued BEHIND the frame that gave up is told at the
    // hang-up rather than left on a deadline that will never come.
    let queued = session.step(Input::Command { now: t + 2, id: ClientId(8), line: "noise off" });
    assert!(sent(&queued).is_empty(), "stop-and-wait: nothing goes out behind a frame in flight");
    let t = session.deadline().expect("the last retry is in flight");
    let out = session.step(Input::Tick { now: t + 1 });
    assert_eq!(out.iter().filter(|e| matches!(e, Effect::Hangup)).count(), 1);
    assert!(
        out.iter().any(|e| matches!(e, Effect::Reply { id, line } if *id == ClientId(8) && line.starts_with("error"))),
        "the queued client was left waiting: {out:?}"
    );
}

#[test]
fn a_headset_that_answers_after_a_retry_is_ready_with_no_error_left_behind() {
    // The handshake's first frame is lost once, then everything is answered.
    // The document ends "ready" and last_error is empty: a headset that
    // recovered is not still reported as having stopped acknowledging.
    let mut session = fresh();
    session.step(Input::LinkUp { now: 0, peer: peer() });
    let t = session.deadline().expect("in flight");
    let out = session.step(Input::Tick { now: t + 1 });
    assert_eq!(sent(&out).len(), 1, "one retransmission");
    session.step(Input::Bytes { now: t + 2, bytes: &frame::ack_frame(0) });

    let value: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
    assert_eq!(value["phase"], "connecting", "still in the handshake");

    let mut now = t + 3;
    for wire in script() {
        session.step(Input::Bytes { now, bytes: &wire });
        now += 1;
    }
    let value: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
    assert_eq!(value["phase"], "ready");
    assert_eq!(value["last_error"], "", "a recovered headset kept a stale error");
}

#[test]
fn a_hang_up_mid_handshake_does_not_queue_the_feature_queries() {
    // The support tables arrive while the handshake is still draining -- the
    // engine notes it wants the per-feature queries once the outbox empties.
    // Then the frame in flight goes unanswered until the link is dropped.
    // Clearing the outbox empties it, and a hang-up that forgot to withdraw
    // that note would have `pump` fill it again and put a query on the wire
    // in the very step that asked the transport to tear the link down.
    let mut session = fresh();
    session.step(Input::LinkUp { now: 0, peer: peer() });
    let tables: Vec<Vec<u8>> = script()
        .into_iter()
        .filter(|w| frame::unpack(w).map(|f| f.payload.first() == Some(&0x07)).unwrap_or(false))
        .collect();
    assert_eq!(tables.len(), 2, "both support-function replies");
    let mut now = 1u64;
    for wire in &tables {
        session.step(Input::Bytes { now, bytes: wire });
        now += 1;
    }
    let value: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
    assert_eq!(value["phase"], "ready", "the tables bring the phase to ready");

    let mut out = Vec::new();
    for _ in 0..=Limits::default().retries {
        let t = session.deadline().expect("the first handshake frame is still in flight");
        out = session.step(Input::Tick { now: t + 1 });
    }
    assert_eq!(out.iter().filter(|e| matches!(e, Effect::Hangup)).count(), 1, "{out:?}");
    assert!(sent(&out).is_empty(), "a query went out on a link being torn down: {out:?}");
    assert_eq!(session.deadline(), None);
}

#[test]
fn a_link_that_sends_only_junk_is_hung_up_on_exactly_once() {
    // The twin of the acknowledgement path: the bad-frame limit also ends in
    // one Effect::Hangup, and the junk that keeps arriving after it does not
    // ask the transport twice. A client queued on this link is told THIS
    // reason -- the headset may have acknowledged everything it was sent, and
    // is being dropped because the daemon cannot read what it sends back.
    let mut session = ready_session();
    let out = session.step(Input::Command { now: 5_000, id: ClientId(9), line: "noise anc" });
    assert!(!out.iter().any(|e| matches!(e, Effect::Reply { .. })), "the command was answered at once: {out:?}");
    let mut corrupt = frame::pack(DataType::DataMdr, 0, &[0x67, 0x17, 0x01, 0x00, 0x01, 0x00, 0x14]);
    let n = corrupt.len();
    corrupt[n - 2] ^= 0xFF;

    let mut hangups = 0usize;
    let mut sends_after = 0usize;
    let mut told = Vec::new();
    let limit = Limits::default().bad_frames;
    for i in 0..(limit * 2) {
        let out = session.step(Input::Bytes { now: 5_001 + u64::from(i), bytes: &corrupt });
        let here = out.iter().filter(|e| matches!(e, Effect::Hangup)).count();
        if hangups > 0 {
            sends_after += sent(&out).len();
        }
        hangups += here;
        if here > 0 {
            assert!(sent(&out).is_empty(), "sent on a link being torn down: {out:?}");
            assert_eq!(i + 1, limit, "hung up at the wrong count");
        }
        told.extend(out.iter().filter_map(|e| match e {
            Effect::Reply { id, line } if *id == ClientId(9) => Some(line.clone()),
            _ => None,
        }));
    }
    assert_eq!(hangups, 1, "the transport was asked to drop the link {hangups} times");
    assert_eq!(sends_after, 0);
    assert_eq!(session.deadline(), None, "nothing in flight on a dropped link");
    let value: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
    assert_eq!(value["phase"], "error");
    assert!(value["last_error"].as_str().unwrap_or("").contains("cannot read"));
    assert_eq!(told.len(), 1, "the queued client was answered {} times: {told:?}", told.len());
    assert!(told[0].starts_with("error"), "{told:?}");
    assert!(told[0].contains("cannot read"), "the client was given the other path's reason: {told:?}");
    assert!(!told[0].contains("acknowledge"), "{told:?}");
}

#[test]
fn a_second_link_without_a_disconnect_starts_just_as_fresh() {
    // The other half of the reconnect invariant, and the half a mutation
    // showed was unheld. `link_up` and `link_down` both used to clear Facts,
    // so either copy alone kept every test green: deleting one changed
    // nothing a test could see, which means neither could be shown to be
    // doing the work. Both now call `forget`, and this drives the path that
    // reaches it WITHOUT a LinkDown -- a transport that redials and reports
    // the new link before it reported the old one closing.
    let mut session = fresh();
    replay(&mut session, 4096);
    let before: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
    assert_eq!(before["battery"]["right"]["level"], 100, "the script did not run");

    session.step(Input::LinkUp { now: 500, peer: peer() });

    // Nothing this headset has said, so nothing to echo into a SET.
    let out = session.step(Input::Command { now: 501, id: ClientId(3), line: "noise anc" });
    let replies: Vec<&str> = out
        .iter()
        .filter_map(|e| match e {
            Effect::Reply { line, .. } => Some(line.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        replies.iter().any(|r| r.starts_with("error")),
        "a relink answered from the previous link's Facts: {replies:?}"
    );
    let after: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
    assert_eq!(after["battery"]["right"]["present"], false, "a relink kept the old readings");
    assert_eq!(after["device"]["firmware"], "");
}

#[test]
fn the_acknowledgement_is_the_first_thing_out_of_every_data_frame() {
    // ORDER, not just presence. `every_data_frame_is_acknowledged_and_no_ack
    // _ever_is` counts ACKs and would pass just as happily with the ACK
    // pushed last -- a mutation moving it after the frame's own effects
    // survived the whole suite. The engine's reason for the order is in
    // `frame`: "a headset waiting to be acknowledged will not read the next
    // question", so an ACK queued behind a 200-byte query is a stall on a
    // link where the peer is the one holding the token.
    let mut session = fresh();
    session.step(Input::LinkUp { now: 0, peer: peer() });

    let mut acknowledged = 0usize;
    let mut had_more_after = 0usize;
    let mut now = 1u64;
    for wire in script() {
        let f = frame::unpack(&wire).expect("a captured frame");
        let effects = session.step(Input::Bytes { now, bytes: &wire });
        now += 1;
        if !f.data_type.is_data() {
            continue;
        }
        let expected = frame::ack_frame(f.seq);
        match effects.first() {
            Some(Effect::Send(bytes)) if *bytes == expected => acknowledged += 1,
            other => panic!(
                "frame {:02x?} produced {other:?} before its acknowledgement {expected:02x?}",
                f.payload
            ),
        }
        if effects.len() > 1 {
            had_more_after += 1;
        }
    }
    assert!(acknowledged > 0, "the corpus has no data frames; the script is broken");
    // And the assertion has teeth: on some of those frames there really was
    // something else to emit, so "first" is a claim about order and not a
    // restatement of "only".
    assert!(
        had_more_after > 0,
        "every data frame produced the ACK and nothing else, so the ordering was never tested"
    );

    // The strongest case is one the corpus cannot make: a frame the decoder
    // cannot read still gets acknowledged FIRST, before the note saying so.
    // Table 2 carries its own command namespace, so this is a well-formed
    // frame the table-1 decoder is right to refuse.
    let unreadable = frame::pack(DataType::DataMdrNo2, 1, &[0x67, 0x17, 0x01]);
    let effects = session.step(Input::Bytes { now, bytes: &unreadable });
    assert!(
        matches!(effects.first(), Some(Effect::Send(bytes)) if *bytes == frame::ack_frame(1)),
        "an unreadable frame was noted before it was acknowledged: {effects:?}"
    );
    assert!(
        notes(&effects).iter().any(|n| matches!(n, Note::Unread { .. })),
        "a table-2 frame decoded as a table-1 message"
    );
}

#[test]
fn a_new_link_inherits_neither_half_a_frame_nor_the_last_links_junk() {
    // The other two things `forget` clears, and two more mutations that
    // survived until this test existed. Both are about state that is
    // meaningful only WITHIN one link:
    //
    //   the scanner   holds bytes read but not yet framed. Kept across a
    //                 redial, the tail of the old link's last frame becomes
    //                 the head of the new link's first one, and the frame
    //                 that comes out belongs to neither.
    //   the bad count is how unsynchronised THIS link is. Kept, a link that
    //                 saw 31 junk frames hands the next one a single-strike
    //                 budget, and a healthy headset is abandoned for the
    //                 previous one's noise.
    let mut session = fresh();
    session.step(Input::LinkUp { now: 0, peer: peer() });

    // Junk, stopping one short of the limit.
    let limit = Limits::default().bad_frames;
    let mut corrupt = frame::pack(DataType::DataMdr, 0, &[0x67, 0x17, 0x01, 0x00, 0x01, 0x00, 0x14]);
    let n = corrupt.len();
    corrupt[n - 2] ^= 0xFF;
    let mut seen = 0u32;
    let mut now = 1u64;
    while seen < limit - 1 {
        let out = session.step(Input::Bytes { now, bytes: &corrupt });
        now += 1;
        seen += notes(&out).iter().filter(|x| matches!(x, Note::BadFrame(_))).count() as u32;
    }
    let value: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
    assert_eq!(value["phase"], "connecting", "one short of the limit is not yet a bad link");

    // Then a well-formed frame, cut in half, with only the first half
    // delivered -- LAST, so it is what the scanner is still holding when the
    // link drops. Sent first it would be thrown away by the resynchronisation
    // on the next start marker, and this test would prove nothing.
    let whole = frame::pack(DataType::DataMdr, 0, &[0x67, 0x17, 0x01, 0x00, 0x01, 0x00, 0x14]);
    let cut = whole.len() / 2;
    session.step(Input::Bytes { now, bytes: &whole[..cut] });

    session.step(Input::LinkDown { now: 100, why: Down::Closed });
    session.step(Input::LinkUp { now: 200, peer: peer() });

    // The rest of that frame, on the new link. It must be junk here: those
    // bytes are the tail of a message the previous headset was sending.
    let out = session.step(Input::Bytes { now: 201, bytes: &whole[cut..] });
    let acks: Vec<Vec<u8>> = sent(&out)
        .into_iter()
        .filter(|w| frame::unpack(w).map(|f| f.data_type == DataType::Ack).unwrap_or(false))
        .collect();
    assert!(
        acks.is_empty(),
        "a frame was assembled from bytes belonging to two different links: {acks:02x?}"
    );

    // And one more piece of junk is one, not the thirty-second.
    session.step(Input::Bytes { now: 202, bytes: &corrupt });
    let value: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
    assert_ne!(value["phase"], "error", "the new link was abandoned for the old link's junk");

    // The limit itself still bites, so the assertion above is about WHOSE
    // junk it is and not about the limit having been quietly removed.
    let mut seen = 1u32;
    let mut now = 203u64;
    while seen < limit {
        let out = session.step(Input::Bytes { now, bytes: &corrupt });
        now += 1;
        seen += notes(&out).iter().filter(|x| matches!(x, Note::BadFrame(_))).count() as u32;
    }
    let value: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
    assert_eq!(value["phase"], "error", "a link that sent nothing readable was not abandoned");
}

#[test]
fn a_dial_that_found_nothing_says_why_without_calling_it_an_error() {
    // Most machines have no Sony headset, most of the time. That is the
    // ordinary case and not a fault, so it must not put the daemon in an error
    // phase -- but the REASON still has to reach the document, because the
    // difference between "not connected" and "bluetoothctl is not installed"
    // is the whole of what a person needs to fix it.
    let mut session = fresh();
    let out = session.step(Input::Trouble { now: 0, what: "bluetoothctl is not installed" });
    assert!(out.iter().any(|e| matches!(e, Effect::Publish)), "the reason was not published");

    let value: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
    assert_eq!(value["phase"], "idle", "a machine with no headset is not a broken daemon");
    assert_eq!(value["connected"], false);
    assert_eq!(value["last_error"], "bluetoothctl is not installed");

    // And it is ignored once a headset is answering: a dialler that has not
    // noticed the link came up is the ordinary source of one of these, and its
    // news is older than the link.
    replay(&mut session, 4096);
    let before = session.document();
    session.step(Input::Trouble { now: 1000, what: "no headset found" });
    assert_eq!(session.document(), before, "a stale dial failure overwrote a live link's state");
}

#[test]
fn a_device_name_reaches_the_document_as_data_and_not_as_json() {
    // THE NAME IS NOT OURS. It is whatever a device advertises, read out of
    // `bluetoothctl` by the dialler and handed straight to `LinkUp` -- so
    // anyone within Bluetooth range chooses this string. `state::json_escape`
    // is what makes that safe, and `sony_contract.rs` pins the escaping
    // itself; this pins the PATH, because an escape that is never reached is
    // not protecting anything.
    let mut session = fresh();
    let hostile = "\",\"connected\":true,\"x\":\"\n\u{1}\\";
    session.step(Input::LinkUp {
        now: 0,
        peer: Peer { name: hostile.to_string(), protocol: 2 },
    });

    let document = session.document();
    let value: serde_json::Value = serde_json::from_str(&document)
        .unwrap_or_else(|e| panic!("a device name broke the document: {e}\n{document}"));
    assert_eq!(value["device"]["name"], hostile, "the name came back changed");
    // And the key it tried to forge is still the one the engine set.
    assert_eq!(value["connected"], true);
    assert!(value.get("x").is_none(), "a device name introduced a key");
}

/// A session brought to Ready by the replay, and the sequence its next frame
/// from the headset should carry -- the replay's last data frame alternates
/// with ours, so the test frames continue the alternation.
fn ready_session() -> Session {
    let mut session = fresh();
    replay(&mut session, 4096);
    let value: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
    assert_eq!(value["phase"], "ready");
    session
}

#[test]
fn no_question_is_asked_twice_on_one_link() {
    // `pump`'s comment said the per-feature queries go out "Once". They did
    // not. `want_feature_gets` is set by EVERY SupportFunctions reading, and
    // `encode::handshake` asks for two of them -- one per support table,
    // DataMdr and DataMdrNo2. The only thing collapsing the two into one batch
    // was `pump`'s `outbox.is_empty()` guard happening to be false when the
    // first reply landed. Replaying the corpus is enough to reorder that, and
    // the first run of this test found SIX queries going out twice:
    // [22, 09], [22, 0a], [26, 05], [50, 00], [66, 17] and the batch's own
    // shape. `asked_feature_gets` is what makes "once" true rather than
    // likely.
    //
    // Idempotent reads, so not a correctness bug -- but the link is
    // stop-and-wait and the headset grants one control session across every
    // device paired with it, so the wasted round trips are round trips the
    // phone spends locked out.
    //
    // Stated as "no question is sent twice", which covers the batch and every
    // other repetition: nothing ticks in `replay`, so there are no
    // retransmissions to confuse with one.
    //
    // THE KEY IS (data type, payload), NOT THE PAYLOAD. The first version of
    // this test keyed on the payload alone and reported `[06, 00]` twice --
    // which is `ConnectGetSupportFunction`, asked once on `DataMdr` and once
    // on `DataMdrNo2`, deliberately, because that is how both support tables
    // are requested. Keying on the payload made the handshake look like the
    // defect it was written to find.
    let mut session = fresh();
    let out = replay(&mut session, 4096);
    let value: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
    assert_eq!(value["phase"], "ready", "the corpus did not reach a ready link");

    let mut seen: std::collections::HashMap<(u8, Vec<u8>), usize> = std::collections::HashMap::new();
    for wire in sent(&out) {
        let f = frame::unpack(&wire).expect("this session's own frames unpack");
        // ACKs carry no payload and are one per inbound frame by design.
        if f.data_type == DataType::Ack {
            continue;
        }
        *seen.entry((f.data_type.to_wire(), f.payload.clone())).or_default() += 1;
    }
    assert!(!seen.is_empty(), "nothing was asked at all; this test examined nothing");
    let twice: Vec<(String, usize)> = seen
        .iter()
        .filter(|(_, n)| **n > 1)
        .map(|((t, p), n)| (format!("type {t:#04x} {p:02x?}"), *n))
        .collect();
    assert!(twice.is_empty(), "a question went out more than once on one link: {twice:?}");
}

#[test]
fn the_two_equaliser_writes_reach_the_wire_and_the_preset_write_still_does_not() {
    // The whole engine, not just the encoder: parse, route, encode, emit.
    //
    // `eq-preset` is here as the CONTRAST rather than as an oversight. It is
    // still accepted and still sent, because the error strings on this socket
    // are a contract with the C++ daemon (`sony::command`'s header) -- what
    // changed is that the page no longer draws buttons for it. The three verbs
    // share one message, so a test that could not tell them apart would pass
    // over an engine that routed all three to the same edit.
    let mut session = ready_session();
    let mut now = 6_000u64;
    let mut frames: Vec<Vec<u8>> = Vec::new();
    for line in ["eq-band 3 2", "clear-bass -3", "eq-preset rock"] {
        let out = session.step(Input::Command { now, id: ClientId(1), line });
        let refusals: Vec<&str> = out
            .iter()
            .filter_map(|e| match e {
                Effect::Reply { line, .. } if line.starts_with("error") => Some(line.as_str()),
                _ => None,
            })
            .collect();
        assert!(refusals.is_empty(), "{line:?} was refused: {refusals:?}");
        // Stop-and-wait: only the first goes out at once, so the frame is
        // collected after each step and the queue drained by an ACK.
        let sent = sent(&out);
        if let Some(wire) = sent.first() {
            frames.push(wire.clone());
        }
        now += 1;
        // Acknowledge whatever is in flight so the next verb can be pumped.
        let seq = frame::unpack(frames.last().expect("a frame")).expect("a frame").seq;
        session.step(Input::Bytes {
            now,
            bytes: &frame::pack(DataType::Ack, 1 - seq, &[]),
        });
        now += 1;
    }
    assert_eq!(frames.len(), 3, "one frame per verb: {frames:02x?}");

    let payloads: Vec<Vec<u8>> = frames
        .iter()
        .map(|w| frame::unpack(w).expect("a frame").payload)
        .collect();
    for (line, p) in ["eq-band", "clear-bass", "eq-preset"].iter().zip(&payloads) {
        assert_eq!(p[0], 0x58, "{line} did not send EQEBB_SET_PARAM: {p:02x?}");
        assert_eq!(p[1], 0x00, "{line} did not send it at PRESET_EQ: {p:02x?}");
    }
    // And the three edits are three DIFFERENT frames, each moving its own
    // byte: the band at offset 7, clear bass at 4, the preset at 2.
    assert_ne!(payloads[0], payloads[1]);
    assert_ne!(payloads[0], payloads[2]);
    assert_ne!(payloads[1], payloads[2]);
    assert_eq!(payloads[0][2], payloads[1][2], "a band write moved the preset byte");
    assert_ne!(payloads[2][2], payloads[0][2], "the preset write did not move the preset byte");
}

#[test]
fn nothing_outstanding_does_not_mean_the_headset_has_finished_answering() {
    // THE PROPERTY `qs-sonyd once` USED TO MISTAKE FOR "done".
    //
    // Its exit was `published_once && session.deadline().is_none()`, and
    // `deadline` reports only the frame in flight -- so it is `None` the
    // instant an ACK is credited and the outbox is empty, whether or not the
    // device's RET frames have landed. `Effect::Publish` fires on LinkUp, so
    // `published_once` is already true by then. The two together say "quiet",
    // and quiet mid-handshake is not finished.
    //
    // This walks the captured session frame by frame and asserts the gap is
    // REACHABLE: there is at least one moment where the engine has nothing
    // outstanding, has published, says `connected: true` -- and has decoded no
    // battery at all. That document is what a one-shot run would have left
    // behind for the Headphones page to read.
    //
    // Their hardware is where the number for the fix comes from:
    // delarosa1312/omarchy-sony-xm5 (MIT), docs/protocol.md -- "the handshake
    // completing does not mean the state has arrived. Reading immediately
    // gives zeroes; values land over the following second or two." Nothing
    // here can measure that window; what it CAN do is prove the window exists.
    let mut session = fresh();
    let mut quiet_and_empty = 0;
    let mut ever_quiet = 0;
    let mut now = 1u64;
    session.step(Input::LinkUp { now: 0, peer: peer() });

    for wire in script() {
        session.step(Input::Bytes { now, bytes: &wire });
        now += 1;
        if session.deadline().is_some() {
            continue;
        }
        ever_quiet += 1;
        let v: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
        // ANY battery, not `battery.main`. The captured device is a
        // WF-1000XM5 -- earbuds -- so it answers at the LEFT_RIGHT and CRADLE
        // types and never fills `main`. A test that looked only at `main`
        // would call the FINAL document empty too, which is how the first
        // draft of this failed.
        if v["connected"] == true && !any_battery(&v) {
            quiet_and_empty += 1;
        }
    }

    assert!(ever_quiet > 0, "the replay never went quiet; this test examined nothing");
    assert!(
        quiet_and_empty > 0,
        "no quiet moment carried an empty document -- if this is genuinely \
         unreachable now, the settle in qs-sonyd.rs is dead code and should go, \
         but do not delete this test to make it pass"
    );

    // And the other half: by the end of the capture the document is NOT empty,
    // so the assertion above is about timing rather than about a decoder that
    // never works.
    let v: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
    assert!(
        any_battery(&v),
        "the capture never produced a battery reading at all: {}",
        v["battery"]
    );
}

/// Did the document decode a battery of any kind?
///
/// FOUR PLACES, because the device decides which. A WH-1000XM5 answers at
/// BATTERY and fills `main`; the captured WF-1000XM5 is earbuds and answers at
/// LEFT_RIGHT and CRADLE, so `main` stays absent forever and `left`, `right`
/// and `case` carry everything. A check that looked at one of them would call
/// a perfectly good document empty.
fn any_battery(v: &serde_json::Value) -> bool {
    ["main", "left", "right", "case"]
        .iter()
        .any(|side| v["battery"][side]["level"].as_i64().unwrap_or(-1) > 0)
}

#[test]
fn the_headsets_own_named_settings_reach_the_document() {
    // FOUR CAPTURED FRAMES THAT USED TO BE DROPPED. `feature_gets` has asked
    // for these since the support tables were decoded -- a
    // GENERAL_SETTING_GET_CAPABILITY and a GET_PARAM per advertised slot --
    // and nothing read the replies, so all four arrived as `Note::Unread` and
    // the daemon published nothing about them.
    //
    // The values below are the captured device's, not a fixture of ours:
    // 000031/000035 say sidetone is off, 000039/000043 say multipoint is on.
    // (The other way round until the value byte was read as the reference
    // declares it -- see `multipoint_is_on_where_two_hosts_are_connected_at_once`.)
    let mut session = fresh();
    let out = replay(&mut session, 4096);

    let dropped: Vec<(u8, Option<u8>)> = notes(&out)
        .iter()
        .filter_map(|n| match n {
            Note::Unread { command, kind, .. } if *command == 0xD1 || *command == 0xD7 => {
                Some((*command, *kind))
            }
            _ => None,
        })
        .collect();
    assert!(dropped.is_empty(), "a general-setting reply went unread again: {dropped:02x?}");

    let v: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
    let rows = v["general_settings"].as_array().expect("an array");
    assert_eq!(rows.len(), 2, "the corpus holds exactly two: {}", v["general_settings"]);

    // SLOT ORDER, not arrival order -- 0xD1 before 0xD2 whatever order the
    // replies landed in, because a document whose array reorders under the
    // reader is one nothing can bind to.
    assert_eq!(rows[0]["slot"], 0xD1);
    assert_eq!(rows[1]["slot"], 0xD2);

    assert_eq!(rows[0]["key"], "sidetone");
    assert_eq!(rows[0]["name"], "SIDETONE_SETTING", "the wire's own word, untouched");
    assert_eq!(rows[0]["summary"], "SIDETONE_SETTING_SUMMARY");
    assert_eq!(rows[0]["enabled"], false);

    assert_eq!(rows[1]["key"], "multipoint");
    assert_eq!(rows[1]["name"], "MULTIPOINT_SETTING");
    // The device telling us WHICH multipoint it has: the suffix says the
    // switch trades against LDAC on this model.
    assert_eq!(rows[1]["summary"], "MULTIPOINT_SETTING_SUMMARY_LDAC_AVAILABLE");
    assert_eq!(rows[1]["enabled"], true);

    // AND NOT A TOUCH PANEL. Upstream's WH-1000XM5 answers TOUCH_PANEL_SETTING
    // in the slot this WF-1000XM5 puts sidetone in, which is the whole reason
    // the names travel from the device instead of from a table here.
    assert!(
        !session.document().contains("touch_panel"),
        "the captured earbuds do not have a touch panel"
    );
}

#[test]
fn a_later_slot_arriving_first_does_not_reorder_the_array() {
    // ARRIVAL ORDER IS NOT SLOT ORDER, and the document must publish the
    // second. The capability replies come back in the order the handshake
    // asked for them, but a `GENERAL_SETTING_NTFY_PARAM` can touch any slot at
    // any time -- somebody presses the earbud -- so a list that pushed on first
    // sight would reorder itself the first time a setting was toggled on the
    // headset. A reader binding `general_settings[1]` to a row would then be
    // drawing a different setting's state under the same control.
    //
    // The corpus has filled 0xD1 and 0xD2 already. Feed 0xD4 and then 0xD3, so
    // arrival order and slot order disagree by construction.
    let mut session = ready_session();
    let mut seq = 0u8;
    let mut feed = |session: &mut Session, now: u64, payload: &[u8]| {
        session.step(Input::Bytes { now, bytes: &frame::pack(DataType::DataMdr, seq, payload) });
        seq = 1 - seq;
    };
    // 0xD4 first: capability, then its param.
    feed(&mut session, 7_000, &[0xD1, 0xD4, 0x00, 0x01, 0x02, b'z', b'z', 0x01, b'-']);
    // 0x00 is GsSettingValue::ON (T1:956).
    feed(&mut session, 7_010, &[0xD7, 0xD4, 0x00, 0x00]);
    // 0xD3 after it, and DELIBERATELY WITHOUT A PARAM REPLY.
    feed(&mut session, 7_020, &[0xD1, 0xD3, 0x00, 0x01, 0x02, b'a', b'a', 0x01, b'-']);

    let v: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
    let rows = v["general_settings"].as_array().expect("an array");
    let slots: Vec<u64> = rows.iter().map(|r| r["slot"].as_u64().unwrap_or(0)).collect();
    assert_eq!(slots, vec![0xD1, 0xD2, 0xD3, 0xD4], "the array followed arrival order");

    // AND `null` RATHER THAN `false` FOR THE ONE THAT NEVER ANSWERED. The
    // capability reply and the param reply are separate frames, so between
    // them there is a real state where the daemon knows a setting exists and
    // not whether it is on. Publishing `false` there draws a switch in the off
    // position for something that may well be on, which is worse than drawing
    // nothing -- the page can grey out a `null` and cannot grey out a `false`.
    assert!(rows[2]["enabled"].is_null(), "an unanswered setting was published as off");
    assert_eq!(rows[3]["enabled"], true, "0xD4 did answer");
    // And the row is otherwise complete, so the null is about the param reply
    // alone and not about a half-built row.
    assert_eq!(rows[2]["name"], "aa");
}

#[test]
fn a_write_to_a_setting_this_headset_never_named_is_refused() {
    // The device names its own settings, so the daemon cannot know at parse
    // time whether `general touch_panel off` is meaningful -- on a WH-1000XM5
    // it is, on the captured WF-1000XM5 it is not. The parser lets the shape
    // through and the encoder refuses the substance, with a reason that says
    // which of the two it is.
    let mut session = ready_session();

    let out = session.step(Input::Command {
        now: 6_000,
        id: ClientId(1),
        line: "general touch_panel off",
    });
    let replies: Vec<&str> = out
        .iter()
        .filter_map(|e| match e {
            Effect::Reply { line, .. } => Some(line.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        replies.iter().any(|r| r.starts_with("error") && r.contains("has not answered")),
        "a setting this headset never named was not refused as unknown: {replies:?}"
    );
    assert!(sent(&out).is_empty(), "a frame went out for a setting the headset never named");

    // And the one it DID name is accepted and reaches the wire, so the
    // refusal above is about the name rather than about the verb.
    let out =
        session.step(Input::Command { now: 6_100, id: ClientId(2), line: "general multipoint on" });
    let refusals: Vec<&str> = out
        .iter()
        .filter_map(|e| match e {
            Effect::Reply { line, .. } if line.starts_with("error") => Some(line.as_str()),
            _ => None,
        })
        .collect();
    assert!(refusals.is_empty(), "`general multipoint on` was refused: {refusals:?}");
    let wire = sent(&out);
    let p = frame::unpack(wire.first().expect("a frame")).expect("a frame").payload;
    // GENERAL_SETTING_SET_PARAM at the slot the device put multipoint in:
    // BOOLEAN_TYPE, then GsSettingValue::ON, which is 0x00 (T1:956). This
    // asserted 0x01 -- OFF -- for as long as `general multipoint on` sent it.
    assert_eq!(p, vec![0xD8, 0xD2, 0x00, 0x00], "{p:02x?}");
}

#[test]
fn multipoint_is_on_where_two_hosts_are_connected_at_once() {
    // THE CORPUS'S OWN EVIDENCE FOR THE POLARITY, independent of any decoder
    // of the setting itself. The paired-device list in the same capture
    // (fixture 000077) shows two peers connected at the same moment, and a
    // headset with multipoint off holds one connection. So whatever the
    // general-setting byte means, the multipoint reading of this capture must
    // be ON -- and it read OFF while the value was taken as 0 = off.
    let mut session = fresh();
    replay(&mut session, 4096);
    let v: serde_json::Value = serde_json::from_str(&session.document()).expect("json");

    let connected = v["paired_devices"]
        .as_array()
        .expect("an array")
        .iter()
        // Non-zero is connected: `state.rs`'s PairedDevice says 0x01 and 0x02
        // were observed on live links and 0x00 on the rest.
        .filter(|d| d["status"].as_u64().is_some_and(|s| s != 0))
        .count();
    assert!(connected >= 2, "the capture no longer shows two live connections: {}", v["paired_devices"]);

    let multipoint = v["general_settings"]
        .as_array()
        .expect("an array")
        .iter()
        .find(|g| g["key"] == "multipoint")
        .expect("the capture names multipoint");
    assert_eq!(
        multipoint["enabled"], true,
        "two hosts are connected and multipoint reads off: the value byte is read inverted"
    );
}

#[test]
fn an_equaliser_write_before_the_headset_has_answered_is_refused_not_guessed() {
    // A SET is a whole-state message and the band array has no short form, so
    // there is no honest frame to send before the headset has sent its curve.
    // The daemon must say so rather than invent a flat one.
    let mut session = fresh();
    session.step(Input::LinkUp { now: 0, peer: peer() });
    for line in ["eq-band 1 0", "clear-bass 0"] {
        let out = session.step(Input::Command { now: 10, id: ClientId(2), line });
        let replies: Vec<&str> = out
            .iter()
            .filter_map(|e| match e {
                Effect::Reply { line, .. } => Some(line.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            replies.iter().any(|r| r.starts_with("error")),
            "{line:?} was accepted from a headset that has sent no curve: {replies:?}"
        );
        assert!(sent(&out).is_empty(), "{line:?} put a guessed curve on the wire");
    }
}

fn equalizer_after(session: &mut Session, now: u64, seq: u8, payload: &[u8]) -> serde_json::Value {
    session.step(Input::Bytes { now, bytes: &frame::pack(DataType::DataMdr, seq, payload) });
    let value: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
    value["equalizer"].clone()
}

#[test]
fn the_gadgetbridge_preset_frames_put_clear_bass_first() {
    // Two NOTIFY frames a V2 device sent, from Gadgetbridge's
    // SonyProtocolImplV2Test, fed as DATA_MDR frames. BASS_BOOST moves only the
    // first step (+7); TREBLE_BOOST leaves it flat and rises to the last.
    // Read from the tail, treble boost would be a +10 clear bass and bass
    // boost a lone 400Hz bump.
    let mut session = ready_session();
    let eq = equalizer_after(&mut session, 5_000, 0, &[0x59, 0x00, 0x16, 0x06, 0x11, 0x0a, 0x0a, 0x0a, 0x0a, 0x0a]);
    assert_eq!(eq["preset"], "bass");
    assert_eq!(eq["clear_bass"], 7);
    assert_eq!(eq["bands"], serde_json::json!([0, 0, 0, 0, 0]));

    let eq = equalizer_after(&mut session, 5_001, 1, &[0x59, 0x00, 0x15, 0x06, 0x0a, 0x0a, 0x0a, 0x0c, 0x10, 0x14]);
    assert_eq!(eq["preset"], "treble");
    assert_eq!(eq["clear_bass"], 0);
    assert_eq!(eq["bands"], serde_json::json!([0, 0, 2, 6, 10]));

    // A count neither reference names carries its steps whole and leaves
    // clear bass as the last six-step reading set it: libmdr's `default`
    // arm touches neither, and clear bass is not invented from a shape that
    // has no slot for it.
    let eq = equalizer_after(&mut session, 5_002, 0, &[0x59, 0x00, 0x16, 0x06, 0x11, 0x0a, 0x0a, 0x0a, 0x0a, 0x0a]);
    assert_eq!(eq["clear_bass"], 7);
    let eq = equalizer_after(&mut session, 5_003, 1, &[0x57, 0x00, 0xa0, 0x03, 0x0a, 0x0c, 0x08]);
    assert_eq!(eq["clear_bass"], 7, "an odd-count frame overwrote clear bass");
    assert_eq!(eq["bands"], serde_json::json!([0, 2, -2]));
}

#[test]
fn a_ten_band_or_odd_count_equaliser_is_not_split() {
    // libmdr's ten-step branch: no clear bass, ten bands biased +6 (31Hz to
    // 16kHz). Six 0x06 steps would be flat; these are not, so a mutant that
    // applied the +10 bias or the six-step split to ten steps fails on value.
    let mut session = ready_session();
    let eq = equalizer_after(
        &mut session,
        5_000,
        0,
        &[0x57, 0x00, 0xa0, 0x0a, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x05, 0x04, 0x03],
    );
    assert_eq!(eq["clear_bass"], 0);
    assert_eq!(eq["bands"], serde_json::json!([0, 1, 2, 3, 4, 5, 6, -1, -2, -3]));

    // No steps at all is a preset change alone (libmdr's `case 0`: the
    // preset is written and nothing else is touched). The last known clear
    // bass and bands stand rather than being wiped by a frame that did not
    // carry them.
    let eq = equalizer_after(&mut session, 5_001, 1, &[0x57, 0x00, 0x00, 0x00]);
    assert_eq!(eq["preset"], "off");
    assert_eq!(eq["clear_bass"], 0);
    assert_eq!(eq["bands"], serde_json::json!([0, 1, 2, 3, 4, 5, 6, -1, -2, -3]), "a zero-step frame wiped the bands");

    // A count neither reference names travels whole, biased +10, and clear
    // bass is not invented from it.
    let eq = equalizer_after(&mut session, 5_002, 0, &[0x57, 0x00, 0xa0, 0x03, 0x0a, 0x0c, 0x08]);
    assert_eq!(eq["clear_bass"], 0);
    assert_eq!(eq["bands"], serde_json::json!([0, 2, -2]));
}

#[test]
fn every_frame_a_command_becomes_is_logged_with_its_evidence() {
    // The daemon's log is where somebody with a headset and a capture running
    // reads which writes to compare first. A command that reaches the wire
    // without saying how well-founded its frame is would leave them guessing.
    let mut session = ready_session();
    let out = session.step(Input::Command { now: 9_000, id: ClientId(4), line: "noise anc" });
    let sent_notes: Vec<&Note> =
        notes(&out).into_iter().filter(|n| matches!(n, Note::Sent { .. })).collect();
    assert_eq!(sent_notes.len(), 1, "one frame, one note: {sent_notes:?}");
    let Note::Sent { command, kind, tier, set } = sent_notes[0] else { unreachable!() };
    assert_eq!((*command, *kind), (0x68, 0x17), "NCASM_SET_PARAM at the type the headset answered at");
    assert_eq!(*tier, "mirrored-capture");
    assert!(set.starts_with("NcAsmSetParam"), "{set}");

    // And a refusal is not a send.
    let out = session.step(Input::Command { now: 9_100, id: ClientId(4), line: "connection stable" });
    assert!(
        !notes(&out).iter().any(|n| matches!(n, Note::Sent { .. })),
        "a refused command was logged as sent"
    );
}

#[test]
fn an_over_ear_headset_publishes_one_battery_from_either_header_form() {
    // PLAN 23, ITEM 1, the publishing half, end to end through the session.
    // HEADER ONLY -- no capture in the corpus is a WH -- with the layouts
    // corroborated by libmdr's decoder and Gadgetbridge's (level at offset 2,
    // charging at offset 3). The decode itself is pinned in mdr_payload.rs;
    // what this pins is that the reading reaches `battery.main` and only
    // there, that the support table makes it a single-battery headset, and
    // that a volunteered change (POWER_NTFY_STATUS, 0x25) moves it.
    // (function, answer, volunteered change, level, charging, then both again)
    type Case<'a> = (u8, &'a [u8], &'a [u8], i64, &'a str, i64, &'a str);
    let cases: [Case; 2] = [
        // BATTERY_LEVEL_INDICATOR: 73% charging, then 74% not.
        (0x20, &[0x23, 0x00, 0x49, 0x01], &[0x25, 0x00, 0x4a, 0x00], 73, "yes", 74, "no"),
        // BATTERY_LEVEL_WITH_THRESHOLD: 100% not charging, then 99% charging.
        (0x28, &[0x23, 0x08, 0x64, 0x00, 0x14], &[0x25, 0x08, 0x63, 0x01, 0x14], 100, "no", 99, "yes"),
    ];
    for (function, answer, notify, level, charging, later, later_charging) in cases {
        let mut session = fresh();
        session.step(Input::LinkUp { now: 0, peer: Peer { name: "WH-1000XM5".to_string(), protocol: 2 } });
        let mut seq = 0u8;
        let mut feed = |session: &mut Session, now: u64, payload: &[u8]| {
            session.step(Input::Bytes { now, bytes: &frame::pack(DataType::DataMdr, seq, payload) });
            seq = 1 - seq;
        };
        feed(&mut session, 10, &[0x07, 0x00, 0x01, function, 0xFF]);
        feed(&mut session, 20, answer);

        let v: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
        assert_eq!(v["capabilities"]["battery_single"], true, "function {function:#04x}");
        assert_eq!(v["capabilities"]["battery_left_right"], false);
        assert_eq!(v["battery"]["main"]["present"], true, "{}", v["battery"]);
        assert_eq!(v["battery"]["main"]["level"], level);
        assert_eq!(v["battery"]["main"]["charging"], charging);
        for part in ["left", "right", "case"] {
            assert_eq!(v["battery"][part]["present"], false, "an over-ear headset grew a {part} battery");
        }

        feed(&mut session, 30, notify);
        let v: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
        assert_eq!(v["battery"]["main"]["level"], later, "the volunteered change did not land");
        assert_eq!(v["battery"]["main"]["charging"], later_charging);
    }
}

#[test]
fn the_document_publishes_the_ambient_range_the_headset_stated() {
    // PLAN 23, ITEM 2, end to end: the corpus has no capability reply, so the
    // range is the compatibility one; a headset that states one is believed,
    // and focus-on-voice moves which of its ranges applies.
    let mut session = ready_session();
    let range = |session: &Session| -> serde_json::Value {
        let v: serde_json::Value = serde_json::from_str(&session.document()).expect("json");
        v["ambient_range"].clone()
    };
    assert_eq!(range(&session), serde_json::json!({"min":1,"max":20,"step":1,"source":"compat"}));

    let mut seq = 0u8;
    let mut feed = |session: &mut Session, now: u64, payload: &[u8]| {
        session.step(Input::Bytes { now, bytes: &frame::pack(DataType::DataMdr, seq, payload) });
        seq = 1 - seq;
    };
    // At 0x17, the type the corpus's NC/ASM reading came from.
    feed(&mut session, 8_000, &[0x61, 0x17, 0x02, 0x00, 0x01, 0x0C, 0x01, 0x01, 0x02, 0x0A, 0x02]);
    assert_eq!(range(&session), serde_json::json!({"min":1,"max":12,"step":1,"source":"device"}));

    // Focus on voice, volunteered by the headset: the VOICE range now.
    feed(&mut session, 8_010, &[0x69, 0x17, 0x01, 0x01, 0x01, 0x01, 0x04]);
    assert_eq!(range(&session), serde_json::json!({"min":2,"max":10,"step":2,"source":"device"}));
}
