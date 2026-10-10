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
    acl_on(HANDLE, direction, pdu, split)
}

/// The same on another ACL handle.
fn acl_on(handle: u16, direction: Direction, pdu: &[u8], split: Option<usize>) -> Vec<Vec<u8>> {
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
            let mut out = le(handle | (pb << 12)).to_vec();
            out.extend(le(piece.len() as u16));
            out.extend_from_slice(piece);
            out
        })
        .collect()
}

/// What goes into a btsnoop record before the datalink wraps it.
enum Packet {
    Acl(Direction, Vec<u8>),
    /// An ACL packet on the Linux monitor's controller `index` -- what
    /// `btmon -w` writes for a second adapter. 2001 only.
    AclOn(u16, Direction, Vec<u8>),
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
            // BlueZ `src/shared/btsnoop.c`: `flags = index << 16 | opcode`.
            (2001, Packet::AclOn(index, Direction::Tx, b)) => ((u32::from(*index) << 16) | 4, b.clone()),
            (2001, Packet::AclOn(index, Direction::Rx, b)) => ((u32::from(*index) << 16) | 5, b.clone()),
            (_, Packet::AclOn(..)) => panic!("a controller index is the monitor's alone"),
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

// ── the shapes a real trace has ─────────────────────────────────────────────
//
// Each of these is a shape the corpus round trip above does not have, built
// small enough that what is lost or kept is one frame. Each was run against
// the mutation it names, which it fails.

const DLCI: u8 = CHANNEL << 1;

/// An L2CAP signalling command on `handle`, sent `direction`.
fn signal(handle: u16, direction: Direction, code: u8, body: &[u8]) -> Packet {
    let mut command = vec![code, 0x01];
    command.extend(le(body.len() as u16));
    command.extend_from_slice(body);
    Packet::Acl(direction, acl_on(handle, direction, &l2cap(0x0001, &command), None).remove(0))
}

/// A Connection Request for `psm` sent `direction`, naming `source`.
fn request(handle: u16, direction: Direction, psm: u16, source: u16) -> Packet {
    signal(handle, direction, 0x02, &[le(psm), le(source)].concat())
}

/// A successful Connection Response sent `direction`: its sender's channel
/// is `dest`, the requester's `source`.
fn response(handle: u16, direction: Direction, dest: u16, source: u16) -> Packet {
    signal(handle, direction, 0x03, &[le(dest), le(source), le(0), le(0)].concat())
}

/// One MDR payload as a whole frame in one UIH frame on `dlci`, to `cid`.
fn mdr(handle: u16, direction: Direction, cid: u16, dlci: u8, payload: &[u8]) -> Packet {
    let wire = frame::pack(DataType::DataMdr, 0, payload);
    Packet::Acl(direction, acl_on(handle, direction, &l2cap(cid, &uih(dlci, &wire, None)), None).remove(0))
}

/// The payloads of what an import kept, which way each went.
fn kept(file: &[u8]) -> Vec<(Direction, Vec<u8>)> {
    let imported = capture::import(file, None).unwrap_or_else(|e| panic!("{e}"));
    imported.frames.iter().map(|(_, d, w)| (*d, frame::unpack(w).map(|f| f.payload).unwrap_or_default())).collect()
}

#[test]
fn a_trace_started_late_across_a_reconnect_keeps_both_links() {
    // The documented session reconnects (docs/CAPTURE.md, step 5), so a
    // `btmon` started after the daemon dialled holds the SECOND link's PSM 3
    // signalling and not the first's. The first link's channels are named by
    // no signalling at all, so they are tried; this used to try them only
    // when the file held no PSM 3 signalling anywhere, and lost every frame
    // of the first link without a word.
    let first = [0xD8u8, 0xD2, 0x00, 0x01];
    let second = [0xD7u8, 0xD2, 0x00, 0x01];
    for (name, handle, host, headset) in [("new handle", 0x000Cu16, HOST_CID, HEADSET_CID), ("new channels", HANDLE, 0x0044, 0x0045)] {
        let file = btsnoop(
            2001,
            &[
                (1, mdr(HANDLE, Direction::Tx, HEADSET_CID, DLCI, &first)),
                (2, request(handle, Direction::Tx, 0x0003, host)),
                (3, response(handle, Direction::Rx, headset, host)),
                (4, mdr(handle, Direction::Rx, host, DLCI, RELINK)),
                (5, mdr(handle, Direction::Rx, host, DLCI, &second)),
            ],
        );
        assert_eq!(
            kept(&file),
            vec![(Direction::Tx, first.to_vec()), (Direction::Rx, RELINK.to_vec()), (Direction::Rx, second.to_vec())],
            "{name}"
        );
    }
}

#[test]
fn half_a_frame_at_the_end_of_a_link_is_not_spliced_onto_the_next() {
    // A link that drops mid-frame leaves half a frame in the scanner. The
    // next link's first frame used to close it -- one span that does not
    // unpack, and the frame lost with it. Each link gets a scanner of its
    // own now, and the half is counted as unfinished, not hidden.
    let whole = frame::pack(DataType::DataMdr, 0, &[0x23, 0x00, 0x49, 0x01]);
    let half = &frame::pack(DataType::DataMdr, 0, &[0x67, 0x15, 0x01, 0x01, 0x01, 0x01, 0x07])[..6];
    for (name, handle) in [("new handle", 0x000Cu16), ("same handle", HANDLE)] {
        let file = btsnoop(
            2001,
            &[
                (1, request(HANDLE, Direction::Tx, 0x0003, HOST_CID)),
                (2, response(HANDLE, Direction::Rx, HEADSET_CID, HOST_CID)),
                (3, Packet::Acl(Direction::Rx, acl(Direction::Rx, &l2cap(HOST_CID, &uih(DLCI, &whole, None)), None).remove(0))),
                (4, Packet::Acl(Direction::Rx, acl(Direction::Rx, &l2cap(HOST_CID, &uih(DLCI, half, None)), None).remove(0))),
                (5, request(handle, Direction::Tx, 0x0003, HOST_CID)),
                (6, response(handle, Direction::Rx, HEADSET_CID, HOST_CID)),
                (7, mdr(handle, Direction::Rx, HOST_CID, DLCI, RELINK)),
            ],
        );
        let imported = capture::import(&file, None).expect("imported");
        assert_eq!(
            kept(&file),
            vec![(Direction::Rx, vec![0x23, 0x00, 0x49, 0x01]), (Direction::Rx, RELINK.to_vec())],
            "{name}"
        );
        assert_eq!(imported.unfinished, 1, "{name}: the half frame was not counted");
    }
    // The same when nothing new is signalled: an RFCOMM SABM on DLCI 0 is a
    // new multiplexer session, and one on the MDR DLCI is a new data link on
    // the session still open -- what a redial looks like while another
    // profile keeps the session up.
    for (name, dlci) in [("new session", 0u8), ("new data link", DLCI)] {
        let sabm = l2cap(HEADSET_CID, &[(dlci << 2) | 0b11, 0x3F, 0x01, 0x00]);
        let file = btsnoop(
            2001,
            &[
                (1, request(HANDLE, Direction::Tx, 0x0003, HOST_CID)),
                (2, response(HANDLE, Direction::Rx, HEADSET_CID, HOST_CID)),
                (3, Packet::Acl(Direction::Rx, acl(Direction::Rx, &l2cap(HOST_CID, &uih(DLCI, half, None)), None).remove(0))),
                (4, Packet::Acl(Direction::Tx, acl(Direction::Tx, &sabm, None).remove(0))),
                (5, mdr(HANDLE, Direction::Rx, HOST_CID, DLCI, RELINK)),
            ],
        );
        assert_eq!(kept(&file), vec![(Direction::Rx, RELINK.to_vec())], "{name}");
    }
}

#[test]
fn a_span_that_does_not_unpack_is_counted_and_leaves_a_gap() {
    // A capture of an unknown headset exists to show what the decoder
    // rejects, so a span it rejects spends a counter, as the recorder's do.
    let good = |p: &[u8]| frame::pack(DataType::DataMdr, 0, p);
    let mut bad = good(&[0x67, 0x15, 0x01, 0x01, 0x01, 0x01, 0x07]);
    let at = bad.len() - 2;
    bad[at] ^= 0x01; // the checksum
    let mut records = vec![
        (1, request(HANDLE, Direction::Tx, 0x0003, HOST_CID)),
        (1, response(HANDLE, Direction::Rx, HEADSET_CID, HOST_CID)),
        // Another DLCI first, carrying a span that is frame-shaped and not a
        // frame: junk alone does not make a DLCI the MDR channel.
        (2, Packet::Acl(Direction::Rx, acl(Direction::Rx, &l2cap(HOST_CID, &uih(4 << 1, &[0x3E, 0x0C, 0x3C], None)), None).remove(0))),
    ];
    for (ms, wire) in [(10u64, good(&[0x23, 0x00, 0x49, 0x01])), (20, bad), (30, good(RELINK))] {
        records.push((ms, Packet::Acl(Direction::Rx, acl(Direction::Rx, &l2cap(HOST_CID, &uih(DLCI, &wire, None)), None).remove(0))));
    }
    let imported = capture::import(&btsnoop(2001, &records), None).expect("imported");
    assert_eq!(imported.dlci, DLCI, "a DLCI holding only junk was taken for the MDR one");
    assert_eq!((imported.frames.len(), imported.not_frames()), (3, 1));
    let dir = std::env::temp_dir().join(format!("sony-capture-junk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let names = capture::write_fixtures(&imported, &dir, &[]).expect("written");
    assert_eq!(names.len(), 2);
    assert!(names[0].contains("-000001-") && names[1].contains("-000003-"), "{names:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fragments_are_put_back_together_per_handle_and_direction() {
    let tx = frame::pack(DataType::DataMdr, 0, &[0xD8, 0xD2, 0x00, 0x01]);
    let rx = frame::pack(DataType::DataMdr, 1, &[0xD9, 0xD2, 0x00, 0x01]);
    let pieces = |handle: u16, direction: Direction, cid: u16, wire: &[u8]| {
        let pdu = l2cap(cid, &uih(DLCI, wire, None));
        acl_on(handle, direction, &pdu, Some(pdu.len() / 2))
    };
    // Sent and received fragments interleaved on ONE handle.
    let (t, r) = (pieces(HANDLE, Direction::Tx, HEADSET_CID, &tx), pieces(HANDLE, Direction::Rx, HOST_CID, &rx));
    let file = btsnoop(
        2001,
        &[
            (1, Packet::Acl(Direction::Tx, t[0].clone())),
            (2, Packet::Acl(Direction::Rx, r[0].clone())),
            (3, Packet::Acl(Direction::Tx, t[1].clone())),
            (4, Packet::Acl(Direction::Rx, r[1].clone())),
        ],
    );
    assert_eq!(kept(&file).len(), 2, "one direction's start dropped the other's partial");
    // The same direction on TWO handles, interleaved -- two headsets.
    let (a, b) = (pieces(HANDLE, Direction::Rx, HOST_CID, &rx), pieces(0x000C, Direction::Rx, HOST_CID, &tx));
    let file = btsnoop(
        2001,
        &[
            (1, Packet::Acl(Direction::Rx, a[0].clone())),
            (2, Packet::Acl(Direction::Rx, b[0].clone())),
            (3, Packet::Acl(Direction::Rx, a[1].clone())),
            (4, Packet::Acl(Direction::Rx, b[1].clone())),
        ],
    );
    assert_eq!(kept(&file).len(), 2, "one handle's start dropped the other's partial");
    // A start that is whole in one fragment drops a partial left unfinished
    // before it, rather than leaving it to take the next continuation and
    // come out after the frame that followed it.
    let whole = frame::pack(DataType::DataMdr, 0, &[0x23, 0x00, 0x49, 0x01]);
    let file = btsnoop(
        2001,
        &[
            (1, Packet::Acl(Direction::Rx, a[0].clone())),
            (2, Packet::Acl(Direction::Rx, acl(Direction::Rx, &l2cap(HOST_CID, &uih(DLCI, &whole, None)), None).remove(0))),
            (3, Packet::Acl(Direction::Rx, a[1].clone())),
        ],
    );
    assert_eq!(kept(&file), vec![(Direction::Rx, vec![0x23, 0x00, 0x49, 0x01])]);
}

#[test]
fn a_second_controller_is_still_read() {
    // `btmon -w` with no `-i` records every controller, and the monitor's
    // flags carry the controller's index above the opcode.
    let file = btsnoop(2001, &[(1, Packet::AclOn(1, Direction::Rx, match mdr(HANDLE, Direction::Rx, HOST_CID, DLCI, RELINK) {
        Packet::Acl(_, bytes) => bytes,
        _ => unreachable!(),
    }))]);
    assert_eq!(kept(&file), vec![(Direction::Rx, RELINK.to_vec())]);
}

#[test]
fn only_the_rfcomm_channel_is_read_as_rfcomm() {
    // Bytes that WOULD parse as a UIH frame carrying an MDR frame, on three
    // channels that are not the headset's RFCOMM one: the host's own CID
    // 0x0041, which signalling gives to AVDTP (PSM 25) while the headset's
    // RFCOMM channel is also 0x0041 the other way; a second device's channel
    // at the same CID numbers; and DLCI 0, the multiplexer's control channel.
    let stray = [0x99u8, 0x00, 0x07, 0x01];
    let file = btsnoop(
        2001,
        &[
            (1, request(HANDLE, Direction::Tx, 0x0003, HOST_CID)),
            (2, response(HANDLE, Direction::Rx, HEADSET_CID, HOST_CID)),
            (3, request(HANDLE, Direction::Tx, 0x0019, 0x0041)),
            (4, response(HANDLE, Direction::Rx, 0x0042, 0x0041)),
            (5, request(0x000C, Direction::Tx, 0x0019, HOST_CID)),
            (6, response(0x000C, Direction::Rx, HEADSET_CID, HOST_CID)),
            (7, mdr(HANDLE, Direction::Rx, 0x0041, DLCI, &stray)),
            (8, mdr(0x000C, Direction::Rx, HOST_CID, DLCI, &stray)),
            (9, mdr(HANDLE, Direction::Rx, HOST_CID, 0, &stray)),
            (10, mdr(HANDLE, Direction::Rx, HOST_CID, DLCI, RELINK)),
        ],
    );
    assert_eq!(kept(&file), vec![(Direction::Rx, RELINK.to_vec())]);
    // And a channel the HEADSET asked for is read the same way: its request
    // names its own CID, which the host's data is addressed to.
    let file = btsnoop(
        2001,
        &[
            (1, request(HANDLE, Direction::Rx, 0x0003, HEADSET_CID)),
            (2, response(HANDLE, Direction::Tx, HOST_CID, HEADSET_CID)),
            (3, mdr(HANDLE, Direction::Tx, HEADSET_CID, DLCI, &[0x00, 0x00])),
            (4, mdr(HANDLE, Direction::Rx, HOST_CID, DLCI, RELINK)),
        ],
    );
    assert_eq!(kept(&file), vec![(Direction::Tx, vec![0x00, 0x00]), (Direction::Rx, RELINK.to_vec())]);
}

#[test]
fn the_recorder_starts_each_link_clean_counts_what_is_not_a_frame_and_keeps_its_order() {
    let dir = std::env::temp_dir().join(format!("sony-capture-recorder-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut recorder = capture::Recorder::new(&dir).expect("a recorder");
    let names = |dir: &std::path::Path| -> Vec<String> {
        let mut n: Vec<String> = std::fs::read_dir(dir)
            .expect("the directory")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        n.sort();
        n
    };
    // Half a frame, then a new link: the half is not the start of the next.
    let cut = frame::pack(DataType::DataMdr, 0, &[0x23, 0x00, 0x49, 0x01]);
    recorder.received(1_000, &cut[..4]).expect("written");
    recorder.link_up();
    recorder.received(1_001, &frame::pack(DataType::DataMdr, 0, RELINK)).expect("written");
    // A span that does not unpack spends a counter and is not written.
    let mut bad = frame::pack(DataType::DataMdr, 1, &[0x23, 0x00, 0x49, 0x01]);
    let at = bad.len() - 2;
    bad[at] ^= 0x01;
    recorder.received(1_002, &bad).expect("written");
    recorder.received(1_003, &frame::pack(DataType::DataMdr, 0, &[0x23, 0x00, 0x48, 0x00])).expect("written");
    let got = names(&dir);
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(got[0].contains("-000001-rx.") && got[0].contains("CONNECT_RET_PROTOCOL_INFO"), "{got:?}");
    assert!(got[1].contains("-000003-rx."), "the span that is not a frame left no gap: {got:?}");
    // The wall clock stepping back between frames does not reorder them: the
    // counter is the order, and the time leads the name.
    recorder.received(9_000, &frame::pack(DataType::DataMdr, 0, &[0x23, 0x00, 0x46, 0x00])).expect("written");
    recorder.sent(8_500, &frame::pack(DataType::DataMdr, 0, &[0x22, 0x00])).expect("written");
    recorder.received(8_000, &frame::pack(DataType::DataMdr, 1, &[0x23, 0x00, 0x47, 0x00])).expect("written");
    let capture = capture::load(&dir).expect("it loads");
    let payloads: Vec<Vec<u8>> = capture.frames.iter().map(|(_, _, p)| p.clone()).collect();
    assert_eq!(payloads[2..], [vec![0x23, 0x00, 0x46, 0x00], vec![0x22, 0x00], vec![0x23, 0x00, 0x47, 0x00]]);
    assert_eq!(capture.frames[3].0, Direction::Tx);
    let replayed = sony_xm5::sony::replay::received_frames(&dir).expect("it reads");
    let last = frame::unpack(replayed.last().expect("frames")).map(|f| f.payload).ok();
    assert_eq!(last, Some(vec![0x23, 0x00, 0x47, 0x00]), "the replay took the frames in name order");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_paired_device_list_is_left_out_only_under_the_names_it_comes_under() {
    // The list comes as PERI_RET_PARAM when asked and PERI_NTFY_PARAM when
    // volunteered -- the same body (T2:4338), which `docs/CAPTURE.md` now
    // names both of.
    let list = corpus_payload(77);
    let mut volunteered = list.clone();
    volunteered[0] = 0x39;
    let imported = capture::Imported {
        dlci: DLCI,
        frames: vec![
            (1, Direction::Rx, frame::pack(DataType::DataMdrNo2, 0, &list)),
            (2, Direction::Rx, frame::pack(DataType::DataMdrNo2, 1, &volunteered)),
        ],
        unfinished: 0,
    };
    let dir = std::env::temp_dir().join(format!("sony-capture-peri-{}", std::process::id()));
    for (exclude, want) in [(vec!["PERI_RET_PARAM"], 1usize), (vec!["PERI_RET_PARAM", "PERI_NTFY_PARAM"], 0)] {
        let _ = std::fs::remove_dir_all(&dir);
        let exclude: Vec<String> = exclude.into_iter().map(str::to_string).collect();
        let names = capture::write_fixtures(&imported, &dir, &exclude).expect("written");
        assert_eq!(names.len(), want, "{exclude:?}: {names:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(capture::is_command_name("PERI_NTFY_PARAM") && capture::is_command_name("PERI_RET_PARAM"));
    assert!(!capture::is_command_name("PERI_RET_PARAMS") && !capture::is_command_name("peri_ret_param"));
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

/// One frame of a synthetic capture: received on table 1 or table 2, or sent.
enum F<'a> {
    Rx(&'a [u8]),
    Rx2(&'a [u8]),
    Tx(&'a [u8]),
}

/// A synthetic capture of a WH-1000XM5 that names itself and then says
/// `frames`, written by the real `Recorder` in the corpus's scheme, read back
/// by `load` and settled -- the path a real recording takes.
fn settled_by(name: &str, frames: &[F]) -> Vec<(u8, Verdict)> {
    settled_from("WH-1000XM5", name, frames)
}

/// The same from a headset that names itself `model`.
fn settled_from(model: &str, name: &str, frames: &[F]) -> Vec<(u8, Verdict)> {
    let dir = recorded(model, name, frames);
    let capture = capture::load(&dir).expect("it loads");
    assert_eq!(capture.model.as_deref(), Some(model));
    let _ = std::fs::remove_dir_all(&dir);
    capture::settle(&capture).into_iter().map(|(i, v)| (i.number, v)).collect()
}

/// A capture directory holding `frames` from a headset named `model`.
fn recorded(model: &str, name: &str, frames: &[F]) -> PathBuf {
    // A number of its own per call: the tests run in parallel in one
    // process, and two cases sharing a name shared a directory.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("sony-capture-settle-{name}-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut recorder = capture::Recorder::new(&dir).expect("a recorder");
    let mut named = vec![0x05, 0x01, model.len() as u8];
    named.extend_from_slice(model.as_bytes());
    recorder.received(1, &frame::pack(DataType::DataMdr, 0, &named)).expect("written");
    for f in frames {
        match f {
            F::Rx(p) => recorder.received(1, &frame::pack(DataType::DataMdr, 0, p)),
            F::Rx2(p) => recorder.received(1, &frame::pack(DataType::DataMdrNo2, 0, p)),
            F::Tx(p) => recorder.sent(1, &frame::pack(DataType::DataMdr, 0, p)),
        }
        .expect("written");
    }
    dir
}

fn item(verdicts: &[(u8, Verdict)], number: u8) -> &Verdict {
    &verdicts.iter().find(|(n, _)| *n == number).expect("an item").1
}

/// A corpus frame's payload, by counter.
fn corpus_payload(counter: u32) -> Vec<u8> {
    let (_, _, _, wire) = corpus().into_iter().find(|f| f.1 == counter).expect("in the corpus");
    frame::unpack(&wire).expect("it unpacks").payload
}

const RELINK: &[u8] = &[0x01, 0x00, 0x00, 0x00, 0x00, 0x02];

#[test]
fn a_capture_of_the_target_headset_settles_what_it_holds() {
    // Synthetic, written in the corpus's scheme: a WH-1000XM5 that names
    // itself and holds what each frame-settled item asks for. The general
    // settings and the paired-device list are the corpus's own bytes
    // (000039 names MULTIPOINT_SETTING in slot 0xD2, 000077 lists two peers
    // connected); the rest are built from the header.
    let multipoint = corpus_payload(39);
    let peers = corpus_payload(77);
    // `multipoint_reads` is the multipoint slot's byte before the write;
    // the write and what persists are the other one.
    let all = |multipoint_reads: u8| -> Vec<F<'static>> {
        let leak = |b: Vec<u8>| -> &'static [u8] { Box::leak(b.into_boxed_slice()) };
        let (multipoint, peers) = (leak(multipoint.clone()), leak(peers.clone()));
        let written = 0x01 - multipoint_reads;
        vec![
            F::Rx(&[0x23, 0x00, 0x49, 0x01]),
            F::Rx(&[0x61, 0x17, 0x01, 0x00, 0x01, 0x14, 0x01]),
            F::Rx(&[0x67, 0x15, 0x01, 0x01, 0x00, 0x00, 0x00]),
            // Item 4: stable, then a write to quality, read back as quality.
            F::Rx(&[0xE7, 0x02, 0x01]),
            F::Tx(&[0xE8, 0x02, 0x00]),
            F::Rx(&[0xE9, 0x02, 0x00]),
            // Item 5: playing, then PAUSE, then the status says PAUSE.
            F::Rx(&[0xA3, 0x01, 0x00, 0x01, 0x00]),
            F::Tx(&[0xA4, 0x01, 0x00, 0x01]),
            F::Rx(&[0xA5, 0x01, 0x00, 0x02, 0x00]),
            // Item 6: the multipoint slot read, two peers connected.
            F::Rx(multipoint),
            F::Rx(leak(vec![0xD7, 0xD2, 0x00, multipoint_reads])),
            F::Rx2(peers),
            // Item 10: the upscaler says DISABLE.
            F::Rx(&[0xE3, 0x01, 0x01]),
            // Item 11: table 2's four new readings.
            F::Rx2(&[0x47, 0x01, 0x00, 0x01]),
            F::Rx2(&[0x47, 0x20, 0x00]),
            F::Rx2(&[0x5B, 0x03, 0xFF, 0x00]),
            F::Rx2(&[0x33, 0x02, 0x00, 0x00]),
            // Item 14: multipoint switched over, the alert answered, a new
            // link, and multipoint still as it was written.
            F::Tx(leak(vec![0xD8, 0xD2, 0x00, written])),
            F::Rx(leak(vec![0xD9, 0xD2, 0x00, written])),
            F::Rx(&[0x99, 0x00, 0x07, 0x01]),
            F::Tx(&[0x98, 0x00, 0x07, 0x01]),
            F::Rx(RELINK),
            F::Rx(leak(vec![0xD7, 0xD2, 0x00, written])),
        ]
    };
    // THE COMMAND LINE'S 0, which nothing reached before: every item frames
    // can settle, settled. And its 1 for a contradiction alone -- multipoint
    // read off with two peers connected -- which is not settled and must not
    // read as it.
    for (multipoint_reads, code, state) in [(0x00u8, 0, "settled"), (0x01, 1, "contradicted")] {
        let dir = recorded("WH-1000XM5", &format!("cli-{multipoint_reads}"), &all(multipoint_reads));
        let (got, report) = qs_sony(&["settle", dir.to_str().expect("a path")]);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(got, code, "{report}");
        let six = report["items"].as_array().and_then(|items| items.iter().find(|i| i["item"] == 6)).cloned();
        assert_eq!(six.map(|i| i["state"].clone()), Some(serde_json::json!(state)), "{report}");
    }
    // Multipoint read ON (0x00) before the write: the same frames, settled.
    let v = settled_by("all", &all(0x00));
    let settled: Vec<u8> = v.iter().filter(|(_, v)| matches!(v, Verdict::Settled(_))).map(|(n, _)| *n).collect();
    assert_eq!(settled, vec![1, 2, 3, 4, 5, 6, 10, 11, 14], "{v:?}");
    assert_eq!(item(&v, 4), &Verdict::Settled("e7 02 01 then e8 02 00 then e9 02 00".to_string()));
    assert_eq!(item(&v, 5), &Verdict::Settled("a3 01 00 01 00 then a4 01 00 01 then a5 01 00 02 00".to_string()));
}

#[test]
fn a_connection_write_is_one_at_a_connection_type_that_changes_the_mode() {
    // ITEM 4. A DSEE write is AUDIO_SET_PARAM too, at UPSCALING (0x01): it
    // and its echo settle nothing -- the case `tools/acceptance.sh` makes
    // when `connection` is refused and `dsee on` follows.
    let dsee = [F::Rx(&[0xE7, 0x01, 0x00][..]), F::Tx(&[0xE8, 0x01, 0x01]), F::Rx(&[0xE9, 0x01, 0x01])];
    assert_eq!(item(&settled_by("dsee", &dsee), 4), &Verdict::Open);
    // A DSEE write FIRST does not hide a connection write after it.
    let both = [
        F::Rx(&[0xE7, 0x01, 0x00][..]),
        F::Rx(&[0xE7, 0x02, 0x01]),
        F::Tx(&[0xE8, 0x01, 0x01]),
        F::Rx(&[0xE9, 0x01, 0x01]),
        F::Tx(&[0xE8, 0x02, 0x00]),
        F::Rx(&[0xE9, 0x02, 0x00]),
    ];
    assert!(matches!(item(&settled_by("dsee-first", &both), 4), Verdict::Settled(_)));
    // Nor does a connection write that showed nothing hide a later one that
    // does: every write is looked at, not the first.
    let later = [
        F::Rx(&[0xE7, 0x02, 0x01][..]),
        F::Tx(&[0xE8, 0x02, 0x01]),
        F::Tx(&[0xE8, 0x02, 0x00]),
        F::Rx(&[0xE9, 0x02, 0x00]),
    ];
    assert!(matches!(item(&settled_by("later-write", &later), 4), Verdict::Settled(_)));
    // The plain form at 0x00 counts as well.
    let plain = [F::Rx(&[0xE7, 0x00, 0x00][..]), F::Tx(&[0xE8, 0x00, 0x01]), F::Rx(&[0xE7, 0x00, 0x01])];
    assert!(matches!(item(&settled_by("plain", &plain), 4), Verdict::Settled(_)));
    // Writing the mode already held shows nothing, nor does a write with no
    // reading before it, nor one read back the other way, nor a reply at
    // the other connection type.
    for (name, frames) in [
        ("noop", vec![F::Rx(&[0xE7, 0x02, 0x01][..]), F::Tx(&[0xE8, 0x02, 0x01]), F::Rx(&[0xE9, 0x02, 0x01])]),
        ("unread", vec![F::Tx(&[0xE8, 0x02, 0x00][..]), F::Rx(&[0xE9, 0x02, 0x00])]),
        ("refused", vec![F::Rx(&[0xE7, 0x02, 0x01][..]), F::Tx(&[0xE8, 0x02, 0x00]), F::Rx(&[0xE9, 0x02, 0x01])]),
        ("other-type", vec![F::Rx(&[0xE7, 0x02, 0x01][..]), F::Tx(&[0xE8, 0x02, 0x00]), F::Rx(&[0xE9, 0x00, 0x00])]),
    ] {
        assert_eq!(item(&settled_by(name, &frames), 4), &Verdict::Open, "{name}");
    }
}

#[test]
fn a_transport_command_counts_only_when_the_status_it_asks_for_follows_on_its_link() {
    // ITEM 5. PAUSE (0x01) asks for status PAUSE (0x02), PLAY (0x07) for
    // PLAY (0x01): PlaybackControl and PlaybackStatus put the names at
    // different bytes.
    let play = [F::Rx(&[0xA3, 0x01, 0x00, 0x02, 0x00][..]), F::Tx(&[0xA4, 0x01, 0x00, 0x07]), F::Rx(&[0xA3, 0x01, 0x00, 0x01, 0x00])];
    assert!(matches!(item(&settled_by("play", &play), 5), Verdict::Settled(_)));
    // Every transport write is looked at, not the first: a track change
    // first does not hide the pause after it.
    let after_track = [
        F::Rx(&[0xA3, 0x01, 0x00, 0x01, 0x00][..]),
        F::Tx(&[0xA4, 0x01, 0x00, 0x02]),
        F::Tx(&[0xA4, 0x01, 0x00, 0x01]),
        F::Rx(&[0xA5, 0x01, 0x00, 0x02, 0x00]),
    ];
    assert!(matches!(item(&settled_by("after-track", &after_track), 5), Verdict::Settled(_)));
    for (name, frames) in [
        // The status says the headset did NOT act.
        ("ignored", vec![F::Rx(&[0xA3, 0x01, 0x00, 0x01, 0x00][..]), F::Tx(&[0xA4, 0x01, 0x00, 0x01]), F::Rx(&[0xA3, 0x01, 0x00, 0x01, 0x00])]),
        // The status it asks for, but only on the NEXT link: the reply every
        // link-up asks for, not an answer to the write.
        (
            "relink",
            vec![
                F::Rx(&[0xA3, 0x01, 0x00, 0x01, 0x00][..]),
                F::Tx(&[0xA4, 0x01, 0x00, 0x01]),
                F::Rx(RELINK),
                F::Rx(&[0xA3, 0x01, 0x00, 0x02, 0x00]),
            ],
        ),
        // DISABLE in the control field: the headset will not take one.
        ("disabled", vec![F::Rx(&[0xA3, 0x01, 0x00, 0x01, 0x00][..]), F::Tx(&[0xA4, 0x01, 0x00, 0x01]), F::Rx(&[0xA3, 0x01, 0x01, 0x02, 0x00])]),
        // Already paused: a pause changes nothing anyone can see.
        ("already", vec![F::Rx(&[0xA3, 0x01, 0x00, 0x02, 0x00][..]), F::Tx(&[0xA4, 0x01, 0x00, 0x01]), F::Rx(&[0xA3, 0x01, 0x00, 0x02, 0x00])]),
        // A track change moves no status.
        ("track", vec![F::Rx(&[0xA3, 0x01, 0x00, 0x01, 0x00][..]), F::Tx(&[0xA4, 0x01, 0x00, 0x02]), F::Rx(&[0xA3, 0x01, 0x00, 0x02, 0x00])]),
        // The status after a SECOND write is that write's, not the first's.
        (
            "second",
            vec![
                F::Rx(&[0xA3, 0x01, 0x00, 0x01, 0x00][..]),
                F::Tx(&[0xA4, 0x01, 0x00, 0x01]),
                F::Tx(&[0xA4, 0x01, 0x00, 0x02]),
                F::Rx(&[0xA3, 0x01, 0x00, 0x02, 0x00]),
            ],
        ),
    ] {
        assert_eq!(item(&settled_by(name, &frames), 5), &Verdict::Open, "{name}");
    }
}

#[test]
fn polarity_is_settled_by_two_connected_peers_and_not_by_a_read_back() {
    // ITEM 6. A write read back cannot tell the polarities apart -- SET, RET
    // and NTFY share `GsSettingValue` -- so it settles nothing, whichever
    // byte it carries.
    for value in [0x00u8, 0x01] {
        let echo = [F::Tx(&[0xD8, 0xD2, 0x00, value][..]), F::Rx(&[0xD9, 0xD2, 0x00, value])];
        assert_eq!(item(&settled_by(&format!("echo-{value}"), &echo), 6), &Verdict::Open, "{value:#04x}");
    }
    let multipoint = corpus_payload(39);
    let peers = corpus_payload(77);
    // Multipoint read OFF while two peers are connected: the capture says
    // this package has the polarity backwards.
    let off = [F::Rx(&multipoint[..]), F::Rx(&[0xD7, 0xD2, 0x00, 0x01]), F::Rx2(&peers)];
    assert!(matches!(item(&settled_by("contradicted", &off), 6), Verdict::Contradicted(_)));
    // From another model that is evidence about that model, either way.
    assert_eq!(item(&settled_from("WF-1000XM5", "contradicted-wf", &off), 6), &Verdict::Open);
    // The reading after the list counts when none came before it.
    let after = [F::Rx(&multipoint[..]), F::Rx2(&peers), F::Rx(&[0xD7, 0xD2, 0x00, 0x00])];
    assert!(matches!(item(&settled_by("after", &after), 6), Verdict::Settled(_)));
    // Not from another slot, not from another link, and not with one peer.
    let mut one_peer = peers.clone();
    // 000077's second connected entry (`...:4A:9F 02`) set to not connected.
    let at = one_peer.windows(3).position(|w| w == [0x46, 0x02, 0x5A]).expect("the second peer's status") + 1;
    one_peer[at] = 0x00;
    for (name, frames) in [
        ("other-slot", vec![F::Rx(&multipoint[..]), F::Rx(&[0xD7, 0xD1, 0x00, 0x00]), F::Rx2(&peers)]),
        ("other-link", vec![F::Rx(&multipoint[..]), F::Rx(&[0xD7, 0xD2, 0x00, 0x00]), F::Rx(RELINK), F::Rx2(&peers)]),
        ("one-peer", vec![F::Rx(&multipoint[..]), F::Rx(&[0xD7, 0xD2, 0x00, 0x00]), F::Rx2(&one_peer)]),
    ] {
        assert_eq!(item(&settled_by(name, &frames), 6), &Verdict::Open, "{name}");
    }
}

#[test]
fn a_persisting_write_has_to_change_the_slot_and_be_read_back_after_a_new_link() {
    // ITEM 14.
    for (name, frames) in [
        // The value already held, written and read back after a reconnect:
        // it would be there whether or not the headset kept the write.
        (
            "noop",
            vec![F::Rx(&[0xD7, 0xD2, 0x00, 0x01][..]), F::Tx(&[0xD8, 0xD2, 0x00, 0x01]), F::Rx(RELINK), F::Rx(&[0xD7, 0xD2, 0x00, 0x01])],
        ),
        // Read back on ANOTHER slot.
        (
            "other-slot",
            vec![F::Rx(&[0xD7, 0xD2, 0x00, 0x00][..]), F::Tx(&[0xD8, 0xD2, 0x00, 0x01]), F::Rx(RELINK), F::Rx(&[0xD7, 0xD1, 0x00, 0x01])],
        ),
        // No new link: the alert alone settles nothing.
        (
            "no-relink",
            vec![F::Rx(&[0xD7, 0xD2, 0x00, 0x00][..]), F::Tx(&[0xD8, 0xD2, 0x00, 0x01]), F::Rx(&[0x99, 0x00, 0x07, 0x01]), F::Rx(&[0xD7, 0xD2, 0x00, 0x01])],
        ),
        // Back to what it was after the reconnect.
        (
            "reverted",
            vec![F::Rx(&[0xD7, 0xD2, 0x00, 0x00][..]), F::Tx(&[0xD8, 0xD2, 0x00, 0x01]), F::Rx(RELINK), F::Rx(&[0xD7, 0xD2, 0x00, 0x00])],
        ),
    ] {
        assert_eq!(item(&settled_by(name, &frames), 14), &Verdict::Open, "{name}");
    }
    // A later write that does change it settles it, though an earlier one
    // did not.
    let later = [
        F::Rx(&[0xD7, 0xD2, 0x00, 0x01][..]),
        F::Tx(&[0xD8, 0xD2, 0x00, 0x01]),
        F::Tx(&[0xD8, 0xD2, 0x00, 0x00]),
        F::Rx(RELINK),
        F::Rx(&[0xD7, 0xD2, 0x00, 0x00]),
    ];
    assert!(matches!(item(&settled_by("later", &later), 14), Verdict::Settled(_)));
}

#[test]
fn an_available_upscaler_says_nothing_about_an_unavailable_one() {
    // ITEM 10 wants DISABLE; the corpus's ENABLE (000097) is not it.
    assert_eq!(item(&settled_by("enable", &[F::Rx(&[0xE3, 0x01, 0x00])]), 10), &Verdict::Open);
    // ITEM 11 wants all four; three are not enough.
    let three = [F::Rx2(&[0x47, 0x01, 0x00, 0x01][..]), F::Rx2(&[0x47, 0x20, 0x00]), F::Rx2(&[0x5B, 0x03, 0xFF, 0x00])];
    assert_eq!(item(&settled_by("three", &three), 11), &Verdict::Open);
    // ITEM 2 wants a range that decodes.
    assert_eq!(item(&settled_by("no-range", &[F::Rx(&[0x61, 0x17, 0x00])]), 2), &Verdict::Open);
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
    // Nor did a directory that is not there, which used to answer 1 -- the
    // status that means "some items are open".
    let (code, report) = qs_sony(&["settle", "/nonexistent/sony-capture"]);
    assert_eq!(code, 2, "{report}");
    assert_eq!(report["ok"], false);

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
    // Every span counted, none of them junk, no link ended mid-frame.
    assert_eq!(summary["frames"], summary["written"]);
    assert_eq!((summary["not_frames"].as_u64(), summary["unfinished"].as_u64()), (Some(0), Some(0)));
    // An --exclude name neither table declares is refused before anything is
    // written: matching nothing, it would have written what it was typed to
    // keep out.
    let typo = base.join("typo");
    for name in ["PERI_RET_PARAMS", "peri_ret_param"] {
        let (code, said) = qs_sony(&[
            "import",
            trace_file.to_str().expect("a path"),
            &format!("--out={}", typo.display()),
            &format!("--exclude=PERI_NTFY_PARAM,{name}"),
        ]);
        assert_eq!(code, 1, "{name}: {said}");
        assert!(said["error"].as_str().is_some_and(|e| e.contains(name)), "{said}");
        assert!(!typo.exists(), "{name}: a refused import wrote fixtures");
    }
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
