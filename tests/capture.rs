//! The capture path: btsnoop in, fixtures out, and what a capture settles.
//!
//! THE ROUND TRIP IS CIRCULAR, AND SAYS SO. The btsnoop files below are built
//! here, from the corpus's own 70 frames, by code that shares `capture`'s
//! reading of btsnoop, ACL, L2CAP and RFCOMM. Getting every frame back byte for
//! byte, named as the corpus names it, shows the walk is self-consistent and
//! survives the shapes a real trace has -- split ACL fragments, a frame split
//! across two UIH frames, credit bytes, signalling, other channels' traffic.
//! It does not show that a trace a real adapter wrote parses: none has been
//! read here. `docs/OUTSTANDING.md` says the same.

use sony_xm5::capture::{self, Direction, ImportError, Verdict, BTSNOOP_EPOCH_DELTA_US};
use sony_xm5::mdr::{frame, DataType};
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/mdr"))
}

/// The corpus as (ms, counter, name, wire bytes), in counter order.
fn corpus() -> Vec<(u64, u32, String, Vec<u8>)> {
    let mut out: Vec<(u64, u32, String, Vec<u8>)> = std::fs::read_dir(fixture_dir())
        .expect("the corpus")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".bin"))
        .map(|name| {
            let mut parts = name.split('-');
            let ms = parts.nth(2).and_then(|p| p.parse().ok()).expect("a timestamp");
            let counter = parts.next().and_then(|p| p.parse().ok()).expect("a counter");
            let wire = std::fs::read(fixture_dir().join(&name)).expect("it reads");
            (ms, counter, name, wire)
        })
        .collect();
    out.sort_by_key(|f| f.1);
    out
}

/// The corpus's frames and, at every counter it skips, a transmitted frame
/// standing in for the host's half the recorder did not keep.
fn conversation() -> Vec<(u64, Direction, Vec<u8>)> {
    let rx = corpus();
    let last = rx.last().map(|f| f.1).expect("frames");
    let mut out = Vec::new();
    let mut ms = rx[0].0;
    for counter in 1..=last {
        match rx.iter().find(|f| f.1 == counter) {
            Some((at, _, _, wire)) => {
                ms = *at;
                out.push((ms, Direction::Rx, wire.clone()));
            }
            None => out.push((ms, Direction::Tx, frame::pack(DataType::DataMdr, (counter % 2) as u8, &[0x00, 0x00]))),
        }
    }
    out
}

const HANDLE: u16 = 0x000B;
const HOST_CID: u16 = 0x0040;
const HEADSET_CID: u16 = 0x0041;
const CHANNEL: u8 = 9;

fn le(v: u16) -> [u8; 2] {
    v.to_le_bytes()
}

/// An RFCOMM UIH frame. `credit` adds the P/F bit and one credit byte.
fn uih(dlci: u8, info: &[u8], credit: Option<u8>) -> Vec<u8> {
    let mut out = vec![(dlci << 2) | 0b11, if credit.is_some() { 0xFF } else { 0xEF }];
    if info.len() < 128 {
        out.push(((info.len() as u8) << 1) | 1);
    } else {
        out.push(((info.len() & 0x7F) as u8) << 1);
        out.push((info.len() >> 7) as u8);
    }
    out.extend(credit);
    out.extend_from_slice(info);
    out.push(0x00);
    out
}

fn l2cap(cid: u16, payload: &[u8]) -> Vec<u8> {
    let mut out = le(payload.len() as u16).to_vec();
    out.extend(le(cid));
    out.extend_from_slice(payload);
    out
}

/// One L2CAP PDU as ACL packets, split in two at `split` when given.
fn acl(direction: Direction, pdu: &[u8], split: Option<usize>) -> Vec<Vec<u8>> {
    let first_pb = if direction == Direction::Rx { 0b10 } else { 0b00 };
    let pieces: Vec<&[u8]> = match split {
        Some(at) if at < pdu.len() => vec![&pdu[..at], &pdu[at..]],
        _ => vec![pdu],
    };
    pieces
        .iter()
        .enumerate()
        .map(|(i, piece)| {
            let pb: u16 = if i == 0 { first_pb } else { 0b01 };
            let mut out = le(HANDLE | (pb << 12)).to_vec();
            out.extend(le(piece.len() as u16));
            out.extend_from_slice(piece);
            out
        })
        .collect()
}

/// What goes into a btsnoop record before the datalink wraps it.
enum Packet {
    Acl(Direction, Vec<u8>),
    Event(Vec<u8>),
}

fn btsnoop(datalink: u32, records: &[(u64, Packet)]) -> Vec<u8> {
    let mut out = b"btsnoop\0".to_vec();
    out.extend(1u32.to_be_bytes());
    out.extend(datalink.to_be_bytes());
    for (ms, packet) in records {
        let (flags, bytes): (u32, Vec<u8>) = match (datalink, packet) {
            (2001, Packet::Acl(Direction::Tx, b)) => (4, b.clone()),
            (2001, Packet::Acl(Direction::Rx, b)) => (5, b.clone()),
            (2001, Packet::Event(b)) => (3, b.clone()),
            (1002, Packet::Acl(d, b)) => (u32::from(*d == Direction::Rx), [&[0x02][..], b].concat()),
            (1002, Packet::Event(b)) => (0b11, [&[0x04][..], b].concat()),
            (_, Packet::Acl(d, b)) => (u32::from(*d == Direction::Rx), b.clone()),
            (_, Packet::Event(b)) => (0b11, b.clone()),
        };
        out.extend((bytes.len() as u32).to_be_bytes());
        out.extend((bytes.len() as u32).to_be_bytes());
        out.extend(flags.to_be_bytes());
        out.extend(0u32.to_be_bytes());
        out.extend((ms * 1000 + BTSNOOP_EPOCH_DELTA_US).to_be_bytes());
        out.extend(bytes);
    }
    out
}

/// The whole conversation as a trace, with every awkward shape in it.
fn trace(datalink: u32, signalling: bool) -> Vec<u8> {
    let dlci = CHANNEL << 1;
    let start = conversation()[0].0;
    let mut records: Vec<(u64, Packet)> = Vec::new();
    let to_cid = |d: Direction| if d == Direction::Tx { HEADSET_CID } else { HOST_CID };
    // An HCI event, which is not ACL and must be passed over.
    records.push((start, Packet::Event(vec![0x0E, 0x04, 0x01, 0x05, 0x0C, 0x00])));
    if signalling {
        // Connection Request for PSM 3 from the host (source CID 0x0040),
        // and the headset's success (destination CID 0x0041).
        let mut req = vec![0x02, 0x01];
        req.extend(le(4));
        req.extend(le(0x0003));
        req.extend(le(HOST_CID));
        records.push((start, Packet::Acl(Direction::Tx, acl(Direction::Tx, &l2cap(0x0001, &req), None).remove(0))));
        let mut resp = vec![0x03, 0x01];
        resp.extend(le(8));
        resp.extend(le(HEADSET_CID));
        resp.extend(le(HOST_CID));
        resp.extend(le(0));
        resp.extend(le(0));
        records.push((start, Packet::Acl(Direction::Rx, acl(Direction::Rx, &l2cap(0x0001, &resp), None).remove(0))));
    }
    // DLCI 0, the multiplexer: a SABM (not UIH) and a UIH there, both to be
    // ignored -- and another channel carrying bytes that are not MDR.
    let sabm = vec![0x03, 0x3F, 0x01, 0x00];
    records.push((start, Packet::Acl(Direction::Tx, acl(Direction::Tx, &l2cap(HEADSET_CID, &sabm), None).remove(0))));
    records.push((start, Packet::Acl(Direction::Tx, acl(Direction::Tx, &l2cap(HEADSET_CID, &uih(0, &[0xE3, 0x05, 0x13, 0x8D], None)), None).remove(0))));
    records.push((start, Packet::Acl(Direction::Rx, acl(Direction::Rx, &l2cap(HOST_CID, &uih(4 << 1, b"AT+BRSF=0\r", None)), None).remove(0))));

    for (i, (ms, direction, wire)) in conversation().into_iter().enumerate() {
        // Every seventh frame crosses two UIH frames; every third UIH carries
        // a credit; every fifth PDU is split into two ACL fragments.
        let halves: Vec<&[u8]> = if i % 7 == 3 && wire.len() > 4 {
            vec![&wire[..wire.len() / 2], &wire[wire.len() / 2..]]
        } else {
            vec![&wire]
        };
        for (j, half) in halves.into_iter().enumerate() {
            let credit = ((i + j) % 3 == 0).then_some(0x05);
            let pdu = l2cap(to_cid(direction), &uih(dlci, half, credit));
            let split = (i % 5 == 1).then_some(pdu.len() / 2);
            for fragment in acl(direction, &pdu, split) {
                records.push((ms, Packet::Acl(direction, fragment)));
            }
        }
    }
    btsnoop(datalink, &records)
}

#[test]
fn a_trace_of_the_corpus_gives_the_corpus_back_byte_for_byte_and_by_name() {
    let corpus = corpus();
    assert_eq!(corpus.len(), 70, "the whole corpus");
    let fillers = conversation().iter().filter(|f| f.1 == Direction::Tx).count();
    for datalink in [2001u32, 1002, 1001] {
        let imported = capture::import(&trace(datalink, true), None).unwrap_or_else(|e| panic!("{datalink}: {e}"));
        assert_eq!(imported.dlci, CHANNEL << 1, "{datalink}: the MDR channel, not the AT one");
        let rx: Vec<&Vec<u8>> =
            imported.frames.iter().filter(|f| f.1 == Direction::Rx).map(|f| &f.2).collect();
        let want: Vec<&Vec<u8>> = corpus.iter().map(|f| &f.3).collect();
        assert_eq!(rx, want, "{datalink}: the received frames, byte for byte, in order");
        assert_eq!(imported.frames.len(), 70 + fillers, "{datalink}: and the host's half");

        // Written out, the received half is named exactly as the corpus is:
        // the same timestamp, counter, type, sequence and command name.
        let dir = std::env::temp_dir().join(format!("sony-capture-{datalink}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let names = capture::write_fixtures(&imported, &dir, &[]).expect("written");
        let rx_names: Vec<&String> = names.iter().filter(|n| n.contains("-rx.")).collect();
        let want_names: Vec<&String> = corpus.iter().map(|f| &f.2).collect();
        assert_eq!(rx_names, want_names, "{datalink}: the names");
        for (name, (_, _, _, wire)) in rx_names.iter().zip(&corpus) {
            assert_eq!(&std::fs::read(dir.join(name)).expect("it reads"), wire);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn without_signalling_every_dynamic_channel_is_tried_and_a_channel_can_be_named() {
    // A trace started after the link came up has no Connection Request.
    let file = trace(2001, false);
    let imported = capture::import(&file, None).expect("found by trying");
    assert_eq!(imported.dlci, CHANNEL << 1);
    assert_eq!(capture::import(&file, Some(CHANNEL)).expect("named").dlci, CHANNEL << 1);
    // The AT channel carries no MDR frame, so naming it finds nothing.
    assert_eq!(capture::import(&file, Some(4)), Err(ImportError::NoStream));
}

#[test]
fn a_file_that_is_not_a_trace_says_why() {
    assert_eq!(capture::import(b"not a trace at all", None), Err(ImportError::NotBtsnoop));
    let mut wrong = trace(2001, true);
    wrong[12..16].copy_from_slice(&1003u32.to_be_bytes());
    assert_eq!(capture::import(&wrong, None), Err(ImportError::Datalink(1003)));
    let mut short = trace(2001, true);
    short.truncate(short.len() - 3);
    assert!(matches!(capture::import(&short, None), Err(ImportError::Truncated(_))));
    assert_eq!(capture::import(&btsnoop(2001, &[]), None), Err(ImportError::NoStream));
}

#[test]
fn an_excluded_command_leaves_a_gap_and_nothing_is_redacted() {
    let imported = capture::import(&trace(2001, true), None).expect("imported");
    let dir = std::env::temp_dir().join(format!("sony-capture-exclude-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let names = capture::write_fixtures(&imported, &dir, &["PERI_RET_PARAM".to_string()]).expect("written");
    assert!(!names.iter().any(|n| n.contains("PERI_RET_PARAM")), "the paired-device list was written");
    // The counter it would have had is not reused.
    let paired = corpus().into_iter().find(|f| f.2.contains("PERI_RET_PARAM")).expect("in the corpus");
    assert!(!names.iter().any(|n| n.contains(&format!("-{:06}-", paired.1))), "the counter was reused");
    assert_eq!(names.len(), imported.frames.len() - 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_scanner_hands_back_the_wire_bytes_unchanged() {
    // `next_raw` is what a capture writes, and `next_frame` is it plus an
    // unpack: the same span either way, escapes and all.
    let mut scanner = frame::Scanner::new();
    let all: Vec<u8> = corpus().iter().flat_map(|f| f.3.clone()).collect();
    for chunk in all.chunks(13) {
        scanner.push(chunk);
    }
    let mut spans = Vec::new();
    while let Some(span) = scanner.next_raw() {
        spans.push(span);
    }
    assert_eq!(spans, corpus().into_iter().map(|f| f.3).collect::<Vec<_>>());
}

#[test]
fn the_recorder_names_both_directions_on_one_counter() {
    let dir = std::env::temp_dir().join(format!("sony-capture-record-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut recorder = capture::Recorder::new(&dir).expect("a recorder");
    let conversation = conversation();
    for (ms, direction, wire) in &conversation {
        match direction {
            // Received bytes arrive in whatever pieces a read returns.
            Direction::Rx => {
                for piece in wire.chunks(5) {
                    recorder.received(*ms, piece).expect("written");
                }
            }
            Direction::Tx => recorder.sent(*ms, wire).expect("written"),
        }
    }
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("the directory")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names.len(), conversation.len());
    let rx: Vec<&String> = names.iter().filter(|n| n.contains("-rx.")).collect();
    let want: Vec<String> = corpus().into_iter().map(|f| f.2).collect();
    assert_eq!(rx, want.iter().collect::<Vec<_>>(), "named as the corpus is");
    let mode = std::fs::metadata(&dir).map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o777);
    assert_eq!(mode.ok(), Some(0o700), "a capture names every device the headset knows");
    // And a replay of what it recorded is the replay of the corpus.
    let frames = sony_xm5::sony::replay::received_frames(&dir).expect("it reads");
    let corpus_frames = sony_xm5::sony::replay::received_frames(&fixture_dir()).expect("it reads");
    assert_eq!(frames, corpus_frames);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_corpus_settles_nothing_because_it_is_the_wrong_headset() {
    let capture = capture::load(&fixture_dir()).expect("it loads");
    assert_eq!(capture.model.as_deref(), Some("WF-1000XM5"));
    for (item, verdict) in capture::settle(&capture) {
        match item.number {
            7 | 13 => assert!(matches!(verdict, Verdict::NotFromFrames(_)), "item {}", item.number),
            n => assert_eq!(verdict, Verdict::Open, "item {n} settled by a WF-1000XM5 capture"),
        }
    }
}

#[test]
fn a_capture_of_the_target_headset_settles_what_it_holds() {
    // Synthetic, written in the corpus's scheme: a WH-1000XM5 that names
    // itself, answers its single battery and NC/ASM, takes a general-setting
    // write and reads it back, and asks and is answered an alert.
    let dir = std::env::temp_dir().join(format!("sony-capture-settle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut recorder = capture::Recorder::new(&dir).expect("a recorder");
    let mut model = vec![0x05, 0x01, 10];
    model.extend_from_slice(b"WH-1000XM5");
    let rx = |r: &mut capture::Recorder, p: &[u8]| r.received(1, &frame::pack(DataType::DataMdr, 0, p)).expect("written");
    let tx = |r: &mut capture::Recorder, p: &[u8]| r.sent(1, &frame::pack(DataType::DataMdr, 0, p)).expect("written");
    rx(&mut recorder, &model);
    rx(&mut recorder, &[0x23, 0x00, 0x49, 0x01]);
    rx(&mut recorder, &[0x67, 0x15, 0x01, 0x01, 0x00, 0x00, 0x00]);
    tx(&mut recorder, &[0xD8, 0xD2, 0x00, 0x01]);
    rx(&mut recorder, &[0xD9, 0xD2, 0x00, 0x01]);
    rx(&mut recorder, &[0x99, 0x00, 0x07, 0x01]);
    tx(&mut recorder, &[0x98, 0x00, 0x07, 0x01]);
    let capture = capture::load(&dir).expect("it loads");
    assert_eq!(capture.model.as_deref(), Some("WH-1000XM5"));
    let verdicts: Vec<(u8, Verdict)> = capture::settle(&capture).into_iter().map(|(i, v)| (i.number, v)).collect();
    let settled: Vec<u8> = verdicts.iter().filter(|(_, v)| matches!(v, Verdict::Settled(_))).map(|(n, _)| *n).collect();
    assert_eq!(settled, vec![1, 3, 6, 14], "{verdicts:?}");
    // A write read back in the OTHER polarity does not settle item 6.
    let _ = std::fs::remove_dir_all(&dir);
    let mut recorder = capture::Recorder::new(&dir).expect("a recorder");
    rx(&mut recorder, &model);
    tx(&mut recorder, &[0xD8, 0xD2, 0x00, 0x01]);
    rx(&mut recorder, &[0xD9, 0xD2, 0x00, 0x00]);
    let capture = capture::load(&dir).expect("it loads");
    assert!(capture::settle(&capture).iter().all(|(i, v)| i.number != 6 || *v == Verdict::Open));
    let _ = std::fs::remove_dir_all(&dir);
}

// ── the command line ────────────────────────────────────────────────────────

const READER: &str = env!("CARGO_BIN_EXE_qs-sony");
const DAEMON: &str = env!("CARGO_BIN_EXE_qs-sonyd");

fn qs_sony(args: &[&str]) -> (i32, serde_json::Value) {
    let out = std::process::Command::new(READER).args(args).output().expect("qs-sony runs");
    let value = serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
    (out.status.code().unwrap_or(-1), value)
}

#[test]
fn qs_sony_decodes_imports_and_settles_from_the_command_line() {
    let corpus = fixture_dir();
    let corpus = corpus.to_str().expect("a path");

    // decode: every frame, with grounds where something decodes it -- the
    // same 25 `mdr_payload.rs` pins.
    let (code, frames) = qs_sony(&["decode", corpus]);
    assert_eq!(code, 0);
    let frames = frames.as_array().expect("an array");
    assert_eq!(frames.len(), 70);
    assert_eq!(frames.iter().filter(|f| !f["grounds"].is_null()).count(), 25);

    // settle: the corpus is the wrong headset, so items stay open -- exit 1;
    // an empty directory examined nothing -- exit 2, not 0 and not 1.
    let (code, report) = qs_sony(&["settle", corpus]);
    assert_eq!(code, 1, "{report}");
    assert_eq!(report["model"], "WF-1000XM5");
    let empty = std::env::temp_dir().join(format!("sony-capture-empty-{}", std::process::id()));
    std::fs::create_dir_all(&empty).expect("a directory");
    assert_eq!(qs_sony(&["settle", empty.to_str().expect("a path")]).0, 2);

    // import, then replay what it wrote: the document the corpus produces.
    let base = std::env::temp_dir().join(format!("sony-capture-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).expect("a directory");
    let trace_file = base.join("trace.btsnoop");
    std::fs::write(&trace_file, trace(2001, true)).expect("written");
    let out = base.join("fixtures");
    let (code, summary) = qs_sony(&[
        "import",
        trace_file.to_str().expect("a path"),
        &format!("--out={}", out.display()),
        &format!("--channel={CHANNEL}"),
    ]);
    assert_eq!(code, 0, "{summary}");
    assert_eq!(summary["channel"], CHANNEL);
    let replayed = |dir: &str| {
        let out = std::process::Command::new(READER).args(["replay", dir]).output().expect("it runs");
        String::from_utf8(out.stdout).expect("utf-8")
    };
    assert_eq!(replayed(out.to_str().expect("a path")), replayed(corpus));
    let (code, _) = qs_sony(&["import", trace_file.to_str().expect("a path"), "--out=/dev/null/x", "--channel=40"]);
    assert_eq!(code, 1, "a channel past 30 is refused");
    let _ = std::fs::remove_dir_all(&base);
    let _ = std::fs::remove_dir_all(&empty);
}

#[test]
fn a_recording_directory_that_cannot_be_made_stops_the_daemon_at_start() {
    // Read evidence for the rest of --record: the daemon only records a live
    // link, and there is none here. This is the part that runs without one.
    let base = std::env::temp_dir().join(format!("sony-capture-daemon-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("run")).expect("a runtime directory");
    let mut child = std::process::Command::new(DAEMON)
        .args(["once", "--record=/dev/null/capture"])
        .env("XDG_STATE_HOME", base.join("state"))
        .env("XDG_RUNTIME_DIR", base.join("run"))
        .env("HOME", &base)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("qs-sonyd starts");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().expect("it can be waited on") {
            break Some(status);
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let status = status.expect("the daemon ran on with a recording directory it could not make");
    assert_eq!(status.code(), Some(1));
    let mut err = String::new();
    std::io::Read::read_to_string(child.stderr.as_mut().expect("stderr"), &mut err).expect("it reads");
    assert!(err.contains("cannot record into /dev/null/capture"), "{err}");
    let _ = std::fs::remove_dir_all(&base);
}
