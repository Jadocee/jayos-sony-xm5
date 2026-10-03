//! The SDP client, against records written out from the specification.
//!
//! WHERE THESE BYTES COME FROM, because this suite's provenance rule is
//! stricter than most and this file is the one exception in the port.
//!
//! Nothing here is captured. Every fixture is written by hand from the
//! Bluetooth Core Specification, Vol 3, Part B -- the data element encoding in
//! 3.3, the PDU formats in 4.4, the ProtocolDescriptorList in 5.1.5 -- and
//! annotated byte by byte with what the specification says each one is. That
//! is legitimate here and would not be for the MDR decoder next door: MDR is
//! undocumented and only a capture can say what it does, while SDP is
//! published and the document IS the authority. A fixture claiming to be a
//! recording of a Sony headset would be the same error in the other
//! direction.
//!
//! THE ADVERSARIAL HALF IS THE POINT. A service record arrives from a device
//! across a radio link before anything has authenticated it, so every length
//! in it is a claim. The cases below are the claims worth making: a length
//! past the end, a nesting that never stops, a sequence that declares four
//! gigabytes, an attribute count larger than the PDU. A parser that trusts any
//! of them is a parser that a hostile device can crash.

use sony_xm5::sony::sdp::{self, Element, SdpError, Uuid};

/// A ProtocolDescriptorList as a device would send it, one line per element.
///
/// This is the shape 5.1.5 describes for a service reached over RFCOMM, and it
/// is what the MDR service on a Sony headset returns: L2CAP carrying RFCOMM,
/// with the channel as the RFCOMM protocol's one parameter.
fn protocol_descriptor_list() -> Vec<u8> {
    vec![
        // Sequence, 1-byte length: the AttributeLists, holding one record.
        0x35, 0x16,
        //   Sequence, 1-byte length: one record's (id, value) pairs.
        0x35, 0x14,
        //     Unsigned int, 2 bytes: attribute id 0x0004,
        //     ProtocolDescriptorList.
        0x09, 0x00, 0x04,
        //     Sequence, 1-byte length: the protocol stack, outermost first.
        0x35, 0x0F,
        //       Sequence: L2CAP, and the PSM it is reached on.
        0x35, 0x06,
        //         UUID, 2 bytes: 0x0100, L2CAP.
        0x19, 0x01, 0x00,
        //         Unsigned int, 2 bytes: PSM 3.
        0x09, 0x00, 0x03,
        //       Sequence: RFCOMM, and the channel. THE ANSWER.
        0x35, 0x05,
        //         UUID, 2 bytes: 0x0003, RFCOMM.
        0x19, 0x00, 0x03,
        //         Unsigned int, 1 byte: channel 9.
        0x08, 0x09,
    ]
}

/// Wrap `attributes` in a ServiceSearchAttributeResponse (4.4, PDU 0x07).
fn respond(transaction: u16, attributes: &[u8], continuation: &[u8]) -> Vec<u8> {
    let mut params = Vec::new();
    params.extend_from_slice(&(attributes.len() as u16).to_be_bytes());
    params.extend_from_slice(attributes);
    params.push(continuation.len() as u8);
    params.extend_from_slice(continuation);

    let mut pdu = vec![0x07];
    pdu.extend_from_slice(&transaction.to_be_bytes());
    pdu.extend_from_slice(&(params.len() as u16).to_be_bytes());
    pdu.extend_from_slice(&params);
    pdu
}

#[test]
fn the_channel_is_read_out_of_a_real_shaped_record() {
    let bytes = protocol_descriptor_list();
    let (parsed, used) = sdp::element(&bytes).expect("it parses");
    assert_eq!(used, bytes.len(), "the record did not account for every byte");
    assert_eq!(sdp::rfcomm_channel(&parsed), Some(9));
}

#[test]
fn a_uuid_is_the_same_service_at_any_width() {
    // A device may answer a 16-bit query with a 128-bit record. Both are
    // legal and both name RFCOMM, so a comparison at the written width would
    // miss the channel on exactly the devices that write it out in full.
    let short = Uuid::Short(0x0003);
    let long = Uuid::Long([
        0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0x80, 0x5F, 0x9B, 0x34,
        0xFB,
    ]);
    assert_eq!(short.wide(), long.wide());
    assert_eq!(Uuid::Medium(0x0003).wide(), long.wide());
    // And a different service is still different.
    assert_ne!(Uuid::Short(0x0100).wide(), long.wide());

    // The whole record, written with the 128-bit form.
    let mut bytes = vec![
        0x35, 0x13, // Sequence
        0x1C, // UUID, 16 bytes
    ];
    bytes.extend_from_slice(&long.wide());
    bytes.extend_from_slice(&[0x08, 0x0B]); // Unsigned int, 1 byte: channel 11
    let (parsed, _) = sdp::element(&bytes).expect("it parses");
    assert_eq!(sdp::rfcomm_channel(&parsed), Some(11));
}

#[test]
fn a_record_without_rfcomm_answers_nothing_rather_than_guessing() {
    // L2CAP and OBEX, no RFCOMM. A daemon that fell back to a plausible
    // channel here would connect to whatever else the headset has on it.
    let bytes = vec![
        0x35, 0x0D, // Sequence
        0x35, 0x06, 0x19, 0x01, 0x00, 0x09, 0x00, 0x03, // L2CAP on PSM 3
        0x35, 0x03, 0x19, 0x00, 0x08, // OBEX, no parameters
    ];
    let (parsed, _) = sdp::element(&bytes).expect("it parses");
    assert_eq!(sdp::rfcomm_channel(&parsed), None);
}

#[test]
fn a_channel_outside_the_legal_range_is_not_a_channel() {
    // RFCOMM server channels are 1 to 30 (Vol 3, Part A, and RFCOMM 5.4). A
    // record naming 0 or 200 is malformed, and connecting to it would fail
    // with something less clear than "this device did not say where".
    for bad in [0u8, 31, 200] {
        let bytes = vec![0x35, 0x05, 0x19, 0x00, 0x03, 0x08, bad];
        let (parsed, _) = sdp::element(&bytes).expect("it parses");
        assert_eq!(sdp::rfcomm_channel(&parsed), None, "channel {bad} was accepted");
    }
    for good in [1u8, 9, 30] {
        let bytes = vec![0x35, 0x05, 0x19, 0x00, 0x03, 0x08, good];
        let (parsed, _) = sdp::element(&bytes).expect("it parses");
        assert_eq!(sdp::rfcomm_channel(&parsed), Some(good));
    }
}

#[test]
fn the_request_is_the_pdu_the_specification_describes() {
    let mdr = Uuid::Long([
        0x96, 0xCC, 0x20, 0x3E, 0x50, 0x68, 0x46, 0xAD, 0xB3, 0x2D, 0xE3, 0x16, 0xF5, 0xE0, 0x69,
        0xBA,
    ]);
    let pdu = sdp::search_request(0x1234, mdr, sdp::ATTR_PROTOCOL_DESCRIPTOR_LIST, &[]);

    assert_eq!(pdu[0], 0x06, "ServiceSearchAttributeRequest");
    assert_eq!(&pdu[1..3], &[0x12, 0x34], "the transaction id, big-endian");
    let declared = u16::from_be_bytes([pdu[3], pdu[4]]) as usize;
    assert_eq!(declared, pdu.len() - 5, "the parameter length must be the parameters' length");

    // ServiceSearchPattern: a sequence holding the 128-bit UUID.
    assert_eq!(&pdu[5..7], &[0x35, 0x11], "a 17-byte sequence: one 128-bit UUID element");
    assert_eq!(pdu[7], 0x1C, "UUID, 16 bytes");
    assert_eq!(&pdu[8..24], &mdr.wide());

    // MaximumAttributeByteCount, then the AttributeIDList.
    let max = u16::from_be_bytes([pdu[24], pdu[25]]);
    assert!(max > 0 && max <= 672, "the SDP MTU is 672; asking for {max} is not useful");
    assert_eq!(&pdu[26..29], &[0x35, 0x05, 0x0A], "a sequence holding one 4-byte unsigned int");
    assert_eq!(
        u32::from_be_bytes([pdu[29], pdu[30], pdu[31], pdu[32]]),
        0x0004_0004,
        "the range 0x0004..=0x0004, which is ProtocolDescriptorList and nothing else"
    );
    assert_eq!(pdu[33], 0, "no continuation on a first request");
    assert_eq!(pdu.len(), 34);

    // And a continuation is echoed back verbatim, which is the entire contract
    // for that field: it is the device's own bookmark and means nothing here.
    let again = sdp::search_request(0x1234, mdr, sdp::ATTR_PROTOCOL_DESCRIPTOR_LIST, &[0xAB, 0xCD]);
    assert_eq!(&again[33..], &[0x02, 0xAB, 0xCD]);
}

#[test]
fn a_response_gives_up_its_attributes_and_its_continuation() {
    let attributes = protocol_descriptor_list();
    let pdu = respond(0x1234, &attributes, &[0x00, 0x2A]);
    let response = sdp::read_response(&pdu, 0x1234).expect("it reads");
    assert_eq!(response.attributes, attributes);
    assert_eq!(response.continuation, vec![0x00, 0x2A]);

    let done = respond(0x1234, &attributes, &[]);
    assert!(sdp::read_response(&done, 0x1234).expect("it reads").continuation.is_empty());
}

#[test]
fn an_answer_to_a_different_question_is_not_this_answer() {
    // SDP multiplexes on the transaction id. A daemon that ignored it would
    // read a stale or unrelated response as its own -- and on a link where the
    // other end chooses the timing, that is not a hypothetical.
    let pdu = respond(0x1234, &protocol_descriptor_list(), &[]);
    assert_eq!(
        sdp::read_response(&pdu, 0x0001).expect_err("it must refuse"),
        SdpError::Unexpected("a reply to a different question")
    );
}

#[test]
fn an_error_response_is_reported_as_the_device_refusing() {
    // PDU 0x01 with error 0x0003, "invalid request syntax". Reported rather
    // than parsed as a record, so a daemon logs what the device said instead
    // of "malformed".
    let pdu = vec![0x01, 0x12, 0x34, 0x00, 0x02, 0x00, 0x03];
    assert_eq!(sdp::read_response(&pdu, 0x1234).expect_err("it must refuse"), SdpError::Refused(3));
}

#[test]
fn every_length_in_a_response_is_checked_against_what_arrived() {
    // The three a device controls, each set past the end of what it sent.
    let attributes = protocol_descriptor_list();

    // The PDU's own parameter length.
    let mut pdu = respond(0x1234, &attributes, &[]);
    pdu[3] = 0xFF;
    pdu[4] = 0xFF;
    assert_eq!(sdp::read_response(&pdu, 0x1234).expect_err("checked"), SdpError::Truncated);

    // The attribute-list byte count, inside a parameter length that is honest.
    let mut pdu = respond(0x1234, &attributes, &[]);
    pdu[5] = 0xFF;
    pdu[6] = 0xFF;
    assert_eq!(sdp::read_response(&pdu, 0x1234).expect_err("checked"), SdpError::Truncated);

    // The continuation length.
    let mut pdu = respond(0x1234, &attributes, &[]);
    let last = pdu.len() - 1;
    pdu[last] = 0xFF;
    assert_eq!(sdp::read_response(&pdu, 0x1234).expect_err("checked"), SdpError::Truncated);

    // And a PDU too short to have a header at all.
    for n in 0..5 {
        assert_eq!(
            sdp::read_response(&vec![0u8; n], 0x1234).expect_err("checked"),
            SdpError::Truncated
        );
    }
}

#[test]
fn a_record_that_lies_about_its_size_is_an_error_and_not_an_allocation() {
    // A four-byte header declaring a sequence of four gigabytes. A parser that
    // reserved before reading would take the machine's memory on receipt of
    // five bytes.
    let bytes = vec![0x37, 0xFF, 0xFF, 0xFF, 0xFF];
    assert_eq!(sdp::element(&bytes).expect_err("checked"), SdpError::Truncated);

    // The same at every width the specification offers.
    assert_eq!(sdp::element(&[0x35, 0x40]).expect_err("checked"), SdpError::Truncated);
    assert_eq!(sdp::element(&[0x36, 0x10, 0x00]).expect_err("checked"), SdpError::Truncated);
    // A header with nothing after it.
    assert_eq!(sdp::element(&[]).expect_err("checked"), SdpError::Truncated);
    assert_eq!(sdp::element(&[0x35]).expect_err("checked"), SdpError::Truncated);
    // A truncated fixed-width element.
    assert_eq!(sdp::element(&[0x19, 0x00]).expect_err("checked"), SdpError::Truncated);
}

#[test]
fn a_record_that_nests_forever_is_refused_before_the_stack_is() {
    // 64 open sequences, each holding the next. Legal-looking at every level
    // and unbounded in total, which is the shape that takes a recursive
    // parser's stack rather than its memory.
    let mut bytes = Vec::new();
    for _ in 0..64 {
        bytes.push(0x35);
        bytes.push(0x00);
    }
    // Fix the lengths so each sequence really does contain the next.
    for depth in 0..64 {
        let at = depth * 2 + 1;
        bytes[at] = ((64 - depth - 1) * 2) as u8;
    }
    assert_eq!(sdp::element(&bytes).expect_err("checked"), SdpError::TooDeep);

    // And breadth, which costs no depth at all: thousands of one-byte
    // elements inside one sequence.
    let mut wide = vec![0x36, 0x30, 0x00];
    wide.extend(std::iter::repeat_n(0x00u8, 0x3000));
    assert_eq!(sdp::element(&wide).expect_err("checked"), SdpError::TooDeep);
}

#[test]
fn the_element_types_the_specification_defines_all_decode() {
    // Not because the daemon reads any of them -- it wants one channel -- but
    // because a record containing a type the parser cannot handle must not
    // stop it finding the channel further along. Text and URL in particular
    // appear in every real service record, carrying the service's name.
    let cases: Vec<(Vec<u8>, Element)> = vec![
        (vec![0x00], Element::Nil),
        (vec![0x08, 0xFF], Element::Uint(255)),
        (vec![0x09, 0x01, 0x00], Element::Uint(256)),
        (vec![0x0A, 0x00, 0x01, 0x00, 0x00], Element::Uint(65536)),
        (vec![0x10, 0xFF], Element::Int(-1)),
        (vec![0x11, 0xFF, 0xFE], Element::Int(-2)),
        (vec![0x10, 0x7F], Element::Int(127)),
        (vec![0x19, 0x11, 0x0B], Element::Uuid(Uuid::Short(0x110B))),
        (vec![0x25, 0x03, b'M', b'D', b'R'], Element::Text(b"MDR".to_vec())),
        (vec![0x28, 0x01], Element::Bool(true)),
        (vec![0x28, 0x00], Element::Bool(false)),
        (vec![0x45, 0x02, b'h', b'i'], Element::Url(b"hi".to_vec())),
    ];
    for (bytes, want) in cases {
        let (got, used) = sdp::element(&bytes).unwrap_or_else(|e| panic!("{bytes:02x?}: {e}"));
        assert_eq!(got, want, "{bytes:02x?}");
        assert_eq!(used, bytes.len(), "{bytes:02x?} left bytes unread");
    }

    // A record whose first attribute is a name and whose second is the
    // protocol list -- the ordinary case, and the one that proves a type the
    // daemon does not care about is stepped over rather than stopping it.
    // 0x20 bytes of body: the id and its text (8), then the whole 24-byte
    // protocol list including its own outer sequence.
    let mut record = vec![0x35, 0x20, 0x09, 0x01, 0x00, 0x25, 0x03, b'M', b'D', b'R'];
    record.extend_from_slice(&protocol_descriptor_list());
    assert_eq!(record.len(), 2 + 0x20, "the hand-written length is not the body's length");
    let (parsed, used) = sdp::element(&record).expect("it parses");
    assert_eq!(used, record.len());
    assert_eq!(sdp::rfcomm_channel(&parsed), Some(9));
}
