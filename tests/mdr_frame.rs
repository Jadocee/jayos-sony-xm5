//! The MDR codec against 70 packets captured from a real Sony headset.
//!
//! WHAT THIS PROVES, AND WHAT IT DOES NOT
//!
//! `fixtures/mdr/` is vendored from `mos9527/SonyHeadphonesClient` (MIT) at
//! commit db0ae2574d8f: 70 frames captured from a Sony WF-1000XM5 on firmware
//! 6.1.0. This environment has no Sony hardware, no Bluetooth adapter and no
//! BlueZ, so these bytes are the only ground truth available and the tests are
//! written to extract everything they can say.
//!
//! The central one is a round trip: decode each file, re-encode what came out,
//! and require the result to equal the file byte for byte. That is worth more
//! here than in most protocols, because the layers are chained -- the checksum
//! is computed over the length field and the escaping is applied over the
//! checksum -- so an error in any one of them changes the wire bytes and fails.
//!
//! It does NOT prove that anything this port TRANSMITS is accepted by a
//! headset: every fixture is `-rx.`. The one exception is the ACK, which
//! carries no payload and is therefore one of exactly two fixed 9-byte strings,
//! both of which are in the corpus. That much is checked for real.
//!
//! THE FILENAMES ARE ASSERTIONS
//!
//! Upstream's recorder derives each name from the decoded bytes, so
//! `.type-DATA_MDR-0c.seq-01.` is an independent statement about what the frame
//! contains. Checking the decode against the name is therefore a second opinion
//! rather than a restatement -- if this port's idea of the type byte or the
//! sequence number were wrong, the names would disagree.
//!
//! `[profile.test]` does not inherit `panic = "abort"` from the release
//! profile, so a fixture that would abort the daemon surfaces here as an
//! ordinary failing test. That is what these run for; it is not what they run
//! under.

use sony_xm5::mdr::frame::{self, Frame, FrameError, Scanner};
use sony_xm5::mdr::DataType;
use std::fs;
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mdr")
}

/// Every fixture, in filename order.
///
/// Sorted, and that is load-bearing rather than tidiness: the six-digit
/// zero-padded counter in each name sorts lexically into the order the packets
/// arrived, and decoding is stateful further up the stack. A harness that read
/// the directory in whatever order the filesystem returned would pass or fail
/// depending on the machine.
fn corpus() -> Vec<(String, Vec<u8>)> {
    let dir = fixture_dir();
    let mut out: Vec<(String, Vec<u8>)> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "bin"))
        .map(|p| {
            let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let bytes = fs::read(&p).unwrap_or_else(|e| panic!("cannot read {name}: {e}"));
            (name, bytes)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The type byte and sequence number the recorder put in the filename.
fn declared(name: &str) -> (u8, u8) {
    let ty = name
        .split(".type-")
        .nth(1)
        .and_then(|s| s.split('.').next())
        .and_then(|s| s.rsplit('-').next())
        .and_then(|s| u8::from_str_radix(s, 16).ok())
        .unwrap_or_else(|| panic!("no type in {name}"));
    let seq = name
        .split(".seq-")
        .nth(1)
        .and_then(|s| s.split('.').next())
        .and_then(|s| u8::from_str_radix(s, 16).ok())
        .unwrap_or_else(|| panic!("no seq in {name}"));
    (ty, seq)
}

#[test]
fn the_corpus_is_actually_there() {
    // A guard on every test below. A missing fixture directory would otherwise
    // make an empty corpus vacuously pass every loop in this file -- the
    // failure mode where a suite reports success having checked nothing.
    let files = corpus();
    assert_eq!(files.len(), 70, "expected 70 captured packets, found {}", files.len());
    // The exact byte count, so an edit to a fixture is a failing test rather
    // than a quietly different corpus. These are captures: the moment one is
    // "cleaned up" it stops being ground truth and starts being a synthetic
    // fixture wearing a capture's name, and every round-trip below would still
    // pass because it would be round-tripping the edit.
    let total: usize = files.iter().map(|(_, b)| b.len()).sum();
    assert_eq!(total, 1289, "corpus bytes changed; fixtures must stay exactly as captured");
}

#[test]
fn every_captured_frame_decodes() {
    for (name, bytes) in corpus() {
        match frame::unpack(&bytes) {
            Ok(_) => {}
            Err(e) => panic!("{name}: {e}"),
        }
    }
}

#[test]
fn every_captured_frame_re_encodes_byte_for_byte() {
    // The whole point. Decode, re-encode, compare to the file.
    for (name, bytes) in corpus() {
        let f = frame::unpack(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let again = frame::pack(f.data_type, f.seq, &f.payload);
        assert_eq!(
            again, bytes,
            "{name}: re-encoding what was decoded did not reproduce the captured bytes"
        );
    }
}

#[test]
fn the_decode_agrees_with_what_the_recorder_named_the_file() {
    // A second opinion: the names were derived from the bytes by another
    // implementation, so agreeing with them is agreeing with that decoder.
    for (name, bytes) in corpus() {
        let (want_type, want_seq) = declared(&name);
        let f = frame::unpack(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(f.data_type.to_wire(), want_type, "{name}: data type");
        assert_eq!(f.seq, want_seq, "{name}: sequence number");
    }
}

#[test]
fn the_declared_length_is_the_unescaped_payload_length() {
    // The field that a port gets wrong by counting the wrong thing: the wire
    // length, or the body including the header, or the post-escape size. One
    // fixture in this corpus contains an escape, so a port that counted escaped
    // bytes would pass 69 of 70 -- and one that read only the low byte of the
    // four would also pass 69, because exactly one payload is over 255 bytes.
    let mut with_escape = 0;
    let mut over_255 = 0;
    for (name, bytes) in corpus() {
        let f = frame::unpack(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let escaped_body_len = bytes.len().saturating_sub(2);
        let unescaped_body_len = frame::MIN_BODY + f.payload.len();
        if escaped_body_len != unescaped_body_len {
            with_escape += 1;
        }
        if f.payload.len() > 255 {
            over_255 += 1;
        }
    }
    assert!(with_escape >= 1, "no fixture exercises escaping; the escape path is untested");
    assert!(over_255 >= 1, "no fixture exceeds 255 payload bytes; the length field is untested above one byte");
}

#[test]
fn exactly_one_captured_frame_contains_an_escape() {
    // THE NUMBER, not "at least one", because `frame.rs`'s module comment now
    // states it -- and the sentence it replaced stated a different one. It
    // claimed two frames reached the path where the C++ `MDRUnescape` returns
    // an empty vector and `MDRUnpackCommand` indexes it anyway. None does:
    // `every_captured_frame_decodes` is the other half of that, since a
    // malformed escape is exactly what would fail there.
    //
    // Counted over the BODY, between the start and end markers, because the
    // markers themselves are never escaped -- that is the whole reason they
    // can bound a frame.
    let mut with_sentry = Vec::new();
    for (name, bytes) in corpus() {
        let body = match (bytes.first(), bytes.last()) {
            (Some(&frame::START), Some(&frame::END)) => &bytes[1..bytes.len() - 1],
            _ => panic!("{name}: a captured frame is not bounded by the markers"),
        };
        if body.contains(&frame::SENTRY) {
            with_sentry.push(name);
        }
    }
    assert_eq!(
        with_sentry.len(),
        1,
        "the corpus no longer contains exactly one escaped frame; frame.rs says it does: {with_sentry:?}"
    );
}

#[test]
fn the_two_acks_this_port_sends_are_in_the_corpus() {
    // The only bytes an RX-only capture can say anything about on the TX side.
    // An ACK carries no payload, so there are exactly two of them, and the
    // headset sent both.
    let files = corpus();
    let acks: Vec<&Vec<u8>> = files
        .iter()
        .filter(|(n, _)| n.contains(".type-ACK-01."))
        .map(|(_, b)| b)
        .collect();
    assert!(!acks.is_empty(), "no ACK fixtures");

    let seq0 = vec![0x3E, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x3C];
    let seq1 = vec![0x3E, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x02, 0x3C];
    assert!(acks.iter().any(|a| **a == seq0), "the seq-0 ACK is not in the corpus");
    assert!(acks.iter().any(|a| **a == seq1), "the seq-1 ACK is not in the corpus");

    // `1 - seq`: acknowledging a frame that arrived as 1 is sent as 0.
    assert_eq!(frame::ack_frame(1), seq0, "the ACK for an inbound seq 1");
    assert_eq!(frame::ack_frame(0), seq1, "the ACK for an inbound seq 0");

    // And every ACK in the corpus is one of those two, i.e. the headset's own
    // ACKs are the same two byte strings this port emits.
    for (name, bytes) in files.iter().filter(|(n, _)| n.contains(".type-ACK-01.")) {
        assert!(*bytes == seq0 || *bytes == seq1, "{name} is an ACK this port would not produce");
    }
}

#[test]
fn a_stream_split_at_every_byte_boundary_reassembles_identically() {
    // A Bluetooth read returns whatever arrived, not a frame. This concatenates
    // the whole corpus and feeds it to the scanner one chunk at a time at every
    // possible split, which is the failure a live socket produces and a
    // fixture-per-call test never does.
    let files = corpus();
    let whole: Vec<u8> = files.iter().flat_map(|(_, b)| b.clone()).collect();
    let expected: Vec<Frame> = files
        .iter()
        .map(|(n, b)| frame::unpack(b).unwrap_or_else(|e| panic!("{n}: {e}")))
        .collect();

    for chunk in [1usize, 2, 3, 7, 13, 64, 255, 4096] {
        let mut scanner = Scanner::new();
        let mut got = Vec::new();
        for piece in whole.chunks(chunk) {
            scanner.push(piece);
            while let Some(f) = scanner.next_frame() {
                got.push(f.unwrap_or_else(|e| panic!("chunk {chunk}: {e}")));
            }
        }
        assert_eq!(got.len(), expected.len(), "chunk size {chunk}: frame count");
        assert_eq!(got, expected, "chunk size {chunk}: frames differ");
        assert_eq!(scanner.buffered(), 0, "chunk size {chunk}: bytes left over");
    }
}

#[test]
fn the_scanner_does_not_hoard_bytes_that_cannot_begin_a_frame() {
    // A socket connected to the wrong service, line noise, or a device
    // speaking something else entirely: none of it contains a start marker, so
    // none of it can ever become a frame. Holding it is holding rubbish for the
    // life of the daemon.
    let mut scanner = Scanner::new();
    for _ in 0..1000 {
        scanner.push(&[0x00; 64]);
        assert!(scanner.next_frame().is_none());
    }
    assert_eq!(scanner.buffered(), 0, "junk with no start marker was retained");

    // Junk followed by a real frame still yields the frame: resynchronising is
    // the point, not just discarding.
    let (name, bytes) = corpus().into_iter().next().expect("a fixture");
    let expected = frame::unpack(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
    scanner.push(b"garbage before the marker");
    scanner.push(&bytes);
    let got = scanner.next_frame().expect("a frame after the junk");
    assert_eq!(got.as_ref(), Ok(&expected));
    assert_eq!(scanner.buffered(), 0);

    // A start marker whose frame never arrives is held -- it is half a frame,
    // and the rest may still be in flight -- but not without limit.
    //
    // Pushed in one go rather than in a loop on `buffered()`: crossing the cap
    // makes next_frame DROP the buffer, so a loop conditioned on the buffer
    // still being under the cap never ends.
    let mut stuck = Scanner::new();
    stuck.push(&[frame::START]);
    stuck.push(&vec![0x41u8; frame::MAX_BUFFER + 4096]);
    assert!(stuck.next_frame().is_none(), "an unterminated frame is not a frame");
    assert!(
        stuck.buffered() <= frame::MAX_BUFFER,
        "an unterminated frame grew past the cap: {} bytes held",
        stuck.buffered()
    );

    // Under the cap it IS held, because the rest of the frame may still be
    // coming -- dropping it there would break every read that lands mid-frame.
    let mut partial = Scanner::new();
    partial.push(&[frame::START, 0x0C, 0x00]);
    assert!(partial.next_frame().is_none());
    assert_eq!(partial.buffered(), 3, "half a frame must be kept");
}

#[test]
fn a_single_flipped_bit_is_caught() {
    // The checksum's job. Every bit of a mid-sized fixture is flipped in turn
    // and the result must not decode to the same frame: either it fails, or --
    // where the flip landed in a marker and produced a different but valid
    // framing -- it decodes to something else. Silently decoding to the
    // ORIGINAL frame would mean a corrupted byte reached the state machine.
    let (name, bytes) = corpus()
        .into_iter()
        .find(|(n, _)| n.contains("CONNECT_RET_DEVICE_INFO"))
        .expect("a device-info fixture");
    let original = frame::unpack(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));

    let mut caught = 0usize;
    let mut differed = 0usize;
    for i in 0..bytes.len() {
        for bit in 0..8u32 {
            let mut mutant = bytes.clone();
            if let Some(b) = mutant.get_mut(i) {
                *b ^= 1 << bit;
            }
            match frame::unpack(&mutant) {
                Err(_) => caught += 1,
                Ok(f) if f != original => differed += 1,
                Ok(_) => panic!("{name}: flipping bit {bit} of byte {i} still decoded to the same frame"),
            }
        }
    }
    assert!(caught > 0, "no mutant was rejected");
    assert!(caught + differed == bytes.len() * 8);
}

#[test]
fn the_failures_the_reference_indexes_out_of_bounds_are_typed_errors_here() {
    // `MDRUnescape` in the C++ returns an empty vector for both of these and
    // `MDRUnpackCommand` indexes it without checking. Under `panic = "abort"`
    // the equivalent mistake here would be a SIGABRT in a daemon holding a
    // Bluetooth socket, so each is a named error instead.
    assert_eq!(
        frame::unescape(&[0x01, frame::SENTRY]),
        Err(FrameError::TrailingSentry),
        "a sentry with nothing after it"
    );
    assert_eq!(
        frame::unescape(&[frame::SENTRY, 0x41]),
        Err(FrameError::BadEscapee(0x41)),
        "a sentry followed by a byte that is not an escapee"
    );

    // And the whole-frame paths, none of which may panic.
    assert_eq!(frame::unpack(&[]), Err(FrameError::TooShort));
    assert_eq!(frame::unpack(&[0x3E, 0x3C]), Err(FrameError::TooShort));
    assert_eq!(
        frame::unpack(&[0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x3C]),
        Err(FrameError::NoStart)
    );
    assert_eq!(
        frame::unpack(&[0x3E, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00]),
        Err(FrameError::NoEnd)
    );
    // A length field that claims more payload than is present: the read a
    // decoder performs before checking is the out-of-bounds one.
    let mut body = vec![0x0C, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x41];
    let ck = frame::checksum(&body);
    body.push(ck);
    let mut lying = vec![0x3E];
    frame::escape_into(&body, &mut lying);
    lying.push(0x3C);
    assert!(
        matches!(frame::unpack(&lying), Err(FrameError::LengthMismatch { .. })),
        "a length field that overruns the payload"
    );
}

#[test]
fn escaping_round_trips_over_every_byte_value() {
    // Exhaustive over the alphabet, which is 256 values -- so there is no
    // reason to sample. Includes the three that expand and the sentry runs
    // that a naive decoder mis-pairs.
    let all: Vec<u8> = (0..=255u8).collect();
    let mut escaped = Vec::new();
    frame::escape_into(&all, &mut escaped);
    assert_eq!(frame::unescape(&escaped).as_deref(), Ok(all.as_slice()));

    for &b in &[frame::START, frame::END, frame::SENTRY] {
        let run = vec![b; 8];
        let mut esc = Vec::new();
        frame::escape_into(&run, &mut esc);
        assert_eq!(esc.len(), 16, "{b:#04x} must expand to two bytes each");
        assert_eq!(frame::unescape(&esc).as_deref(), Ok(run.as_slice()));
    }

    // A frame whose payload is nothing but bytes that must be escaped: the
    // markers still delimit it, because they are outside the escaped region.
    let payload = vec![frame::END, frame::SENTRY, frame::START];
    let packed = frame::pack(DataType::DataMdr, 1, &payload);
    let back = frame::unpack(&packed).expect("a frame of nothing but escapees");
    assert_eq!(back.payload, payload);
    assert_eq!(back.data_type, DataType::DataMdr);
    assert_eq!(back.seq, 1);
}

#[test]
fn an_unknown_data_type_survives_the_round_trip() {
    // A headset speaking something this port does not know must not be able to
    // stop it. The byte is carried through rather than normalised away.
    let f = frame::unpack(&frame::pack(DataType::Unknown(0x7F), 0, &[1, 2, 3]))
        .expect("an unknown type is still a frame");
    assert_eq!(f.data_type, DataType::Unknown(0x7F));
    assert!(!f.data_type.is_data(), "an unknown type is not acknowledged");
    assert!(!DataType::Ack.is_data(), "an ACK is never acknowledged");
}
