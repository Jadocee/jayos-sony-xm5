//! The link layer, over real sockets.
//!
//! There is no Bluetooth adapter on the machine this is built on, and the one
//! thing that genuinely needs one -- `connect` to a device address -- is not
//! faked here. Everything else is a stream socket doing what a stream socket
//! does, so a `socketpair` exercises the same code the headset will: the same
//! `read`, the same `write_all`, the same `shutdown`, the same descriptor
//! ownership.
//!
//! THE SDP CONVERSATION IS DRIVEN FROM THE OTHER END. A test plays the device:
//! it reads the request this code sends, checks it, and answers with a record
//! built from the specification (see `sony_sdp.rs` on why that is legitimate
//! here). That reaches the continuation loop and both of its bounds, which is
//! where a device gets to choose how long this code runs and how much it
//! holds.

use sony_xm5::sony::link::{self, Channel, Mac};
use sony_xm5::sony::sdp::{self, Uuid};
use std::time::Duration;

fn protocol_descriptor_list(channel: u8) -> Vec<u8> {
    vec![
        0x35, 0x16, // Sequence: the attribute lists
        0x35, 0x14, //   Sequence: one record
        0x09, 0x00, 0x04, //     attribute 0x0004, ProtocolDescriptorList
        0x35, 0x0F, //     Sequence: the protocol stack
        0x35, 0x06, 0x19, 0x01, 0x00, 0x09, 0x00, 0x03, //       L2CAP on PSM 3
        0x35, 0x05, 0x19, 0x00, 0x03, 0x08, channel, //       RFCOMM on `channel`
    ]
}

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

/// Read one SDP request off `device` and return its transaction id and the
/// continuation it echoed.
fn take_request(device: &Channel) -> (u16, Vec<u8>) {
    let mut buf = vec![0u8; 1024];
    let n = device.read(&mut buf).expect("the request arrives");
    assert!(n >= 5, "a request too short to be a PDU: {n} bytes");
    assert_eq!(buf[0], 0x06, "not a ServiceSearchAttributeRequest");
    let transaction = u16::from_be_bytes([buf[1], buf[2]]);
    let length = usize::from(u16::from_be_bytes([buf[3], buf[4]]));
    assert_eq!(length, n - 5, "the request's declared length is not its length");
    let params = &buf[5..n];

    // Walked forward through the three fields before the continuation, rather
    // than counted back from the end: the continuation is a length byte
    // followed by that many bytes, so its own last byte is indistinguishable
    // from its length if you start at the wrong end.
    let mut at = 0usize;
    for field in ["the service search pattern", "the attribute id list"] {
        assert_eq!(params[at], 0x35, "{field} is not a one-byte-length sequence");
        at += 2 + usize::from(params[at + 1]);
        // The MaximumAttributeByteCount sits between the two sequences.
        if field.contains("search") {
            at += 2;
        }
    }
    let continuation_len = usize::from(params[at]);
    assert_eq!(at + 1 + continuation_len, params.len(), "the continuation is not the whole tail");
    (transaction, params[at + 1..].to_vec())
}

#[test]
fn an_address_is_written_one_way_and_sent_the_other() {
    // `bdaddr_t` is little-endian, which is the single most reliable way to
    // get a Bluetooth program subtly wrong: it does not fail to compile, it
    // fails to find the device -- or finds a different one.
    let mac = Mac::parse("AA:BB:CC:DD:EE:FF").expect("it parses");
    assert_eq!(mac.0, [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
    assert_eq!(mac.to_string(), "AA:BB:CC:DD:EE:FF", "it comes back out as it went in");

    // Case and spacing as `bluetoothctl` and a person write them.
    assert_eq!(Mac::parse("aa:bb:cc:dd:ee:ff").expect("lowercase"), mac);

    // And the shapes that are not addresses.
    for bad in ["", "AA:BB:CC:DD:EE", "AA:BB:CC:DD:EE:FF:00", "AA-BB-CC-DD-EE-FF", "AA:BB:CC:DD:EE:GG", "A:B:C:D:E:F"] {
        assert!(Mac::parse(bad).is_none(), "{bad:?} was accepted as an address");
    }
}

#[test]
fn the_socket_addresses_are_laid_out_the_way_the_kernel_reads_them() {
    // THE ONE THING HERE THAT NEEDS AN ADAPTER TO FAIL, so it is asserted at
    // the byte level instead. Every field below has a different rule and
    // getting any of them wrong compiles, runs, and either finds nothing or
    // finds the wrong device.
    let mac = Mac::parse("AA:BB:CC:DD:EE:FF").expect("it parses");

    let rc = link::rfcomm_sockaddr(mac, 9);
    assert_eq!(rc.len(), 10, "sockaddr_rc is 9 bytes of fields padded to 10");
    assert_eq!(u16::from_ne_bytes([rc[0], rc[1]]), 31, "AF_BLUETOOTH, in host order");
    assert_eq!(
        &rc[2..8],
        &[0xFF, 0xEE, 0xDD, 0xCC, 0xBB, 0xAA],
        "bdaddr_t is little-endian: the written address, reversed"
    );
    assert_eq!(rc[8], 9, "the RFCOMM channel");
    assert_eq!(rc[9], 0, "the pad byte must be zero and not whatever was on the stack");

    let l2 = link::l2cap_sockaddr(mac, 0x0001);
    assert_eq!(l2.len(), 14, "sockaddr_l2 is 13 bytes of fields padded to 14");
    assert_eq!(u16::from_ne_bytes([l2[0], l2[1]]), 31, "AF_BLUETOOTH, in host order");
    assert_eq!(u16::from_le_bytes([l2[2], l2[3]]), 1, "the PSM is little-endian, unlike the family");
    assert_eq!(&l2[4..10], &[0xFF, 0xEE, 0xDD, 0xCC, 0xBB, 0xAA]);
    assert_eq!(u16::from_le_bytes([l2[10], l2[11]]), 0, "no CID when connecting by PSM");
    assert_eq!(l2[12], 0, "BDADDR_BREDR: classic Bluetooth, which is what MDR runs over");
    assert_eq!(l2[13], 0, "the pad byte");

    // The reversal is a reversal and not a coincidence of a palindromic test
    // address: a different address must come out differently.
    let other = Mac::parse("01:02:03:04:05:06").expect("it parses");
    assert_eq!(&link::rfcomm_sockaddr(other, 1)[2..8], &[0x06, 0x05, 0x04, 0x03, 0x02, 0x01]);
}

#[test]
fn a_channel_carries_bytes_both_ways_and_survives_being_cloned() {
    let (ours, theirs) = link::pair().expect("a socket pair");
    ours.write_all(b"hello").expect("written");
    let mut buf = [0u8; 16];
    let n = theirs.read(&mut buf).expect("read");
    assert_eq!(&buf[..n], b"hello");

    // The clone is the same connection, not a new one: the daemon reads on one
    // handle while the engine writes on another.
    let reader = theirs.try_clone().expect("a second handle");
    ours.write_all(b"again").expect("written");
    let n = reader.read(&mut buf).expect("read");
    assert_eq!(&buf[..n], b"again");

    // And dropping the clone does not close the original.
    drop(reader);
    ours.write_all(b"still").expect("written");
    let n = theirs.read(&mut buf).expect("read");
    assert_eq!(&buf[..n], b"still");
}

#[test]
fn a_shutdown_wakes_a_reader_that_is_parked_on_the_link() {
    // The daemon's reader thread blocks in `read` and cannot be told to stop;
    // it can only be given something to return from. Closing the descriptor
    // from another thread would be a use-after-free the moment the number is
    // reused, so it must be a shutdown -- and this is the assertion that it
    // actually wakes the reader rather than merely not crashing.
    let (ours, theirs) = link::pair().expect("a socket pair");
    let reader = theirs.try_clone().expect("a second handle");
    // REPORTED THROUGH A CHANNEL RATHER THAN A JOIN. A shutdown that does not
    // wake the reader leaves that thread blocked forever, and a `join` on it
    // would hang the whole suite with no message -- which is the worst way for
    // a test to fail, because nobody can tell it from a slow one.
    let (done, woken) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = [0u8; 16];
        let _ = done.send(reader.read(&mut buf));
    });

    // Long enough that the thread is certainly inside `read`, short enough
    // that a failure is a failure and not a slow machine.
    std::thread::sleep(Duration::from_millis(50));
    theirs.shutdown();

    match woken.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(0)) => {}
        Ok(other) => panic!("a shutdown should read as end of input, not {other:?}"),
        Err(_) => panic!("the reader was still parked three seconds after the shutdown"),
    }
    drop(ours);
}

#[test]
fn a_read_deadline_is_a_deadline_and_not_a_suggestion() {
    // On a thread, for the same reason as the shutdown test above: a deadline
    // that is not set means this read never returns, and a suite that hangs
    // reports nothing at all.
    let (ours, theirs) = link::pair().expect("a socket pair");
    theirs.set_read_timeout(Duration::from_millis(120)).expect("set");
    let (done, answered) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let began = std::time::Instant::now();
        let mut buf = [0u8; 16];
        let out = theirs.read(&mut buf);
        let _ = done.send((format!("{out:?}"), out.is_err(), began.elapsed()));
    });

    match answered.recv_timeout(Duration::from_secs(3)) {
        Ok((shown, failed, waited)) => {
            assert!(failed, "a read with nothing to read returned {shown}");
            assert!(
                waited >= Duration::from_millis(100),
                "it returned in {waited:?}, before its deadline"
            );
            assert!(waited < Duration::from_secs(2), "it waited {waited:?}, past its deadline");
        }
        Err(_) => panic!("the read never came back, so no deadline was set on it"),
    }
    drop(ours);
}

#[test]
fn the_channel_comes_back_from_a_device_that_answers_in_one_go() {
    let (ours, theirs) = link::pair().expect("a socket pair");
    let device = std::thread::spawn(move || {
        let (transaction, continuation) = take_request(&theirs);
        assert!(continuation.is_empty(), "the first request carried a continuation");
        theirs.write_all(&respond(transaction, &protocol_descriptor_list(9), &[])).expect("answered");
    });

    let found = link::ask_for_channel(&ours, link::MDR_SERVICE_V1).expect("the query succeeds");
    assert_eq!(found, Some(9));
    device.join().expect("the device thread finished");
}

#[test]
fn a_record_split_across_continuations_is_assembled_before_it_is_read() {
    // A device is allowed to answer in pieces and say "there is more" with a
    // continuation, which this code must echo back verbatim. The record only
    // parses once all the pieces are together, so a client that tried to read
    // each response on its own would find nothing and report the headset as
    // not supporting MDR.
    // EVERY SPLIT POINT, not one. A device chooses where to break, and most
    // choices leave a tail that happens to parse on its own: split this record
    // after its seventh byte and the remainder is a well-formed
    // ProtocolDescriptorList holding the right channel, so a client that threw
    // away everything but the last response would pass. The offsets that catch
    // that are the ones landing mid-element, and there is no reason to guess
    // which those are when all 23 can be run.
    let whole = protocol_descriptor_list(11);
    for at in 1..whole.len() {
        let first = whole[..at].to_vec();
        let second = whole[at..].to_vec();

        let (ours, theirs) = link::pair().expect("a socket pair");
        let device = std::thread::spawn(move || {
            let (transaction, continuation) = take_request(&theirs);
            assert!(continuation.is_empty());
            theirs.write_all(&respond(transaction, &first, &[0xC0, 0xDE])).expect("answered");

            let (transaction, continuation) = take_request(&theirs);
            assert_eq!(continuation, vec![0xC0, 0xDE], "the continuation was not echoed back");
            theirs.write_all(&respond(transaction, &second, &[])).expect("answered");
        });

        assert_eq!(
            link::ask_for_channel(&ours, link::MDR_SERVICE_V1).expect("succeeds"),
            Some(11),
            "a record split after byte {at} was not put back together"
        );
        device.join().expect("the device thread finished");
    }
}

#[test]
fn a_device_that_never_stops_answering_is_stopped() {
    // The loop's condition is the device's own choice, which is not a
    // condition to accept from hardware across a radio link. Both bounds are
    // exercised: the number of responses, and the total assembled.
    let (ours, theirs) = link::pair().expect("a socket pair");
    let device = std::thread::spawn(move || {
        // Answers forever, always with more to come, until the other end
        // stops asking.
        loop {
            let mut buf = vec![0u8; 1024];
            match theirs.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let transaction = u16::from_be_bytes([buf[1], buf[2]]);
                    let _ = n;
                    if theirs.write_all(&respond(transaction, &[0x00], &[0x01])).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let out = link::ask_for_channel(&ours, link::MDR_SERVICE_V1);
    assert!(out.is_err(), "a device that never finished was followed: {out:?}");
    drop(ours);
    device.join().expect("the device thread finished");
}

#[test]
fn a_device_with_no_rfcomm_in_its_record_is_told_apart_from_one_that_failed() {
    // Two different outcomes that a single Result would collapse: the query
    // worked and the answer was "not this service", against the query itself
    // going wrong. The daemon reports them differently, because only one of
    // them is worth telling the user to check their headset about.
    let no_rfcomm = vec![
        0x35, 0x0A, // Sequence
        0x35, 0x08, 0x09, 0x00, 0x04, 0x35, 0x03, 0x19, 0x01, 0x00, // L2CAP only
    ];
    let (ours, theirs) = link::pair().expect("a socket pair");
    let device = std::thread::spawn(move || {
        let (transaction, _) = take_request(&theirs);
        theirs.write_all(&respond(transaction, &no_rfcomm, &[])).expect("answered");
    });
    assert_eq!(link::ask_for_channel(&ours, link::MDR_SERVICE_V1).expect("the query worked"), None);
    device.join().expect("the device thread finished");

    // And a device that hangs up mid-answer is an error, not a None.
    let (ours, theirs) = link::pair().expect("a socket pair");
    let device = std::thread::spawn(move || {
        let _ = take_request(&theirs);
        drop(theirs);
    });
    assert!(link::ask_for_channel(&ours, link::MDR_SERVICE_V1).is_err());
    device.join().expect("the device thread finished");
}

#[test]
fn the_request_that_goes_out_is_the_one_the_service_was_asked_for() {
    // The UUID travels from the caller to the wire unchanged. A port that
    // substituted its own would query for something the headset does not have
    // and report every device as unsupported.
    let service = Uuid::Short(0x1234);
    let (ours, theirs) = link::pair().expect("a socket pair");
    let device = std::thread::spawn(move || {
        let mut buf = vec![0u8; 1024];
        let n = theirs.read(&mut buf).expect("the request arrives");
        let request = buf[..n].to_vec();
        let (transaction, _) = (u16::from_be_bytes([buf[1], buf[2]]), ());
        theirs.write_all(&respond(transaction, &protocol_descriptor_list(3), &[])).expect("answered");
        request
    });

    assert_eq!(link::ask_for_channel(&ours, service).expect("succeeds"), Some(3));
    let request = device.join().expect("the device thread finished");
    let expected = sdp::search_request(
        u16::from_be_bytes([request[1], request[2]]),
        service,
        sdp::ATTR_PROTOCOL_DESCRIPTOR_LIST,
        &[],
    );
    assert_eq!(request, expected, "the request on the wire is not the one for this service");
}

// ── finding the headset ──────────────────────────────────────────────────────
//
// Against `bluetoothctl`'s real output shapes. There is no adapter here, so
// the text is the fixture -- which is the same arrangement `qs-bluetooth-
// devices` uses next door, and for the same reason.

#[test]
fn the_connected_devices_are_read_off_bluetoothctls_listing() {
    let listing = "\
Device AA:BB:CC:DD:EE:FF WH-1000XM4
Device 11:22:33:44:55:66 Keychron K3
";
    let found = link::connected_devices(listing);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].mac, Mac::parse("AA:BB:CC:DD:EE:FF").expect("parses"));
    assert_eq!(found[0].name, "WH-1000XM4");
    assert_eq!(found[1].name, "Keychron K3");
}

#[test]
fn a_device_name_is_the_devices_own_text_and_cannot_be_confused_for_a_field() {
    // A name is whatever the device advertises. These are the shapes that
    // break a parser which splits on whitespace and takes field two, or which
    // searches the line for something address-shaped.
    let listing = "\
Device AA:BB:CC:DD:EE:FF Bose QC 35 II
Device 11:22:33:44:55:66 Device 99:99:99:99:99:99
Device 22:33:44:55:66:77 
Agent registered
[bluetooth]# devices Connected
Device not-an-address Whatever
Device
";
    let found = link::connected_devices(listing);
    assert_eq!(found.len(), 3, "{found:?}");
    assert_eq!(found[0].name, "Bose QC 35 II", "a name with spaces was cut short");
    assert_eq!(
        found[1].mac,
        Mac::parse("11:22:33:44:55:66").expect("parses"),
        "a name that looks like an address was read as the address"
    );
    assert_eq!(found[1].name, "Device 99:99:99:99:99:99");
    assert_eq!(found[2].name, "", "a device with no name is still a device");
}

#[test]
fn a_service_is_recognised_in_an_info_block() {
    // `bluetoothctl info` as it actually prints: the description before the
    // parenthesis is a lookup table's opinion and says "Vendor specific" for
    // everything worth having.
    let info = "\
Device AA:BB:CC:DD:EE:FF (public)
\tName: WH-1000XM4
\tConnected: yes
\tUUID: Headset                   (00001108-0000-1000-8000-00805f9b34fb)
\tUUID: Audio Sink                (0000110b-0000-1000-8000-00805f9b34fb)
\tUUID: Vendor specific           (96cc203e-5068-46ad-b32d-e316f5e069ba)
\tBattery Percentage: 0x64 (100)
";
    assert!(link::advertises(info, link::MDR_SERVICE_V1));
    // And the short-form services in the same block, which is the check that
    // the comparison widens both sides rather than only the one it was given.
    assert!(link::advertises(info, Uuid::Short(0x110B)), "Audio Sink was not recognised");
    assert!(!link::advertises(info, Uuid::Short(0x111E)), "a service that is not there was found");

    // A device with no MDR service.
    let plain = "\
Device 11:22:33:44:55:66 (public)
\tName: Keychron K3
\tUUID: Human Interface Device    (00001124-0000-1000-8000-00805f9b34fb)
";
    assert!(!link::advertises(plain, link::MDR_SERVICE_V1));
}

#[test]
fn a_wh1000xm5_is_found_on_the_v2_service_and_an_xm4_on_v1() {
    // `bluetoothctl info` of a real WH-1000XM5 (Modalias usb:v054Cp0DF0d0251;
    // a public dump, Arch forum post 2298135) lists 956c7b26-... -- libmdr's
    // MDR_SERVICE_UUID_XM5 -- and does NOT list the legacy 96cc203e-... at
    // all; the C++ daemon's docs/COMPATIBILITY.md records the same device
    // connecting over MDR v2. A daemon that knew only the legacy UUID
    // reported "no connected headset speaks this protocol" for the one device
    // this tree targets.
    let xm5 = "\
Device AA:BB:CC:DD:EE:FF (public)
\tName: WH-1000XM5
\tConnected: yes
\tUUID: Audio Sink                (0000110b-0000-1000-8000-00805f9b34fb)
\tUUID: Vendor specific           (2573f2dd-a2f8-486c-8476-a63734909acb)
\tUUID: Vendor specific           (956c7b26-d49a-4ba8-b03f-b17d393cb6e2)
\tModalias: usb:v054Cp0DF0d0251
";
    let found = link::mdr_services(xm5);
    assert_eq!(found, vec![link::Mdr { service: link::MDR_SERVICE_V2, generation: 2 }]);
    assert_ne!(found[0].generation, 0, "a found headset has a generation");

    // The XM4-era block from the test above: only the legacy service.
    let xm4 = "\
Device AA:BB:CC:DD:EE:FF (public)
\tName: WH-1000XM4
\tUUID: Vendor specific           (96cc203e-5068-46ad-b32d-e316f5e069ba)
";
    assert_eq!(
        link::mdr_services(xm4),
        vec![link::Mdr { service: link::MDR_SERVICE_V1, generation: 1 }]
    );

    // Both advertised, in either textual order: V2 is listed first, because
    // it is dialled first -- Gadgetbridge's getUuidToConnect and the C++
    // daemon's candidate list both prefer it.
    let both = format!("{xm4}\tUUID: Vendor specific           (956c7b26-d49a-4ba8-b03f-b17d393cb6e2)\n");
    let want = vec![
        link::Mdr { service: link::MDR_SERVICE_V2, generation: 2 },
        link::Mdr { service: link::MDR_SERVICE_V1, generation: 1 },
    ];
    assert_eq!(link::mdr_services(&both), want);
    let reversed = "\tUUID: Vendor specific (956c7b26-d49a-4ba8-b03f-b17d393cb6e2)\n\
\tUUID: Vendor specific (96cc203e-5068-46ad-b32d-e316f5e069ba)\n";
    assert_eq!(link::mdr_services(reversed), want, "the order is the dialling order, not the listing's");

    // Neither: not a headset.
    let keychron = "\
Device 11:22:33:44:55:66 (public)
\tName: Keychron K3
\tUUID: Human Interface Device    (00001124-0000-1000-8000-00805f9b34fb)
";
    assert!(link::mdr_services(keychron).is_empty());
}

#[test]
fn the_search_pattern_carries_the_service_that_was_asked_for() {
    // Over a socket pair, the device side reads the ServiceSearchAttribute
    // request and checks the search pattern is the 128-bit UUID it was asked
    // for -- `0x1C` then sixteen bytes -- so a constant with one byte wrong,
    // or a query wired to the other service, is caught on the wire.
    let v2: [u8; 16] = [
        0x95, 0x6C, 0x7B, 0x26, 0xD4, 0x9A, 0x4B, 0xA8, 0xB0, 0x3F, 0xB1, 0x7D, 0x39, 0x3C, 0xB6, 0xE2,
    ];
    let v1: [u8; 16] = [
        0x96, 0xCC, 0x20, 0x3E, 0x50, 0x68, 0x46, 0xAD, 0xB3, 0x2D, 0xE3, 0x16, 0xF5, 0xE0, 0x69, 0xBA,
    ];
    for (service, bytes, other) in [(link::MDR_SERVICE_V2, v2, v1), (link::MDR_SERVICE_V1, v1, v2)] {
        let (ours, theirs) = link::pair().expect("a socket pair");
        let device = std::thread::spawn(move || {
            let mut buf = vec![0u8; 1024];
            let n = theirs.read(&mut buf).expect("the request arrives");
            let request = buf[..n].to_vec();
            let transaction = u16::from_be_bytes([buf[1], buf[2]]);
            theirs.write_all(&respond(transaction, &protocol_descriptor_list(9), &[])).expect("answered");
            request
        });
        assert_eq!(link::ask_for_channel(&ours, service).expect("succeeds"), Some(9));
        let request = device.join().expect("the device thread finished");

        let mut pattern = vec![0x1C];
        pattern.extend_from_slice(&bytes);
        assert!(
            request.windows(pattern.len()).any(|w| w == pattern.as_slice()),
            "the request for {service:?} does not carry its UUID: {request:02x?}"
        );
        assert!(
            !request.windows(other.len()).any(|w| w == other),
            "the request for {service:?} carries the OTHER service's UUID"
        );
    }
}

#[test]
fn a_uuid_that_is_not_one_is_not_read_as_one() {
    // The line is device-controlled text. None of these should be taken for a
    // service, and none should panic.
    for line in [
        "\tUUID: Vendor specific           (not-a-uuid)",
        "\tUUID: Vendor specific           (96cc203e-5068-46ad-b32d-e316f5e069b)",
        "\tUUID: Vendor specific           (96cc203e50684 6adb32de316f5e069ba)",
        "\tUUID: Vendor specific           ()",
        "\tUUID:",
        "\tName: UUID: (96cc203e-5068-46ad-b32d-e316f5e069ba)",
        "",
    ] {
        assert!(
            !link::advertises(line, link::MDR_SERVICE_V1),
            "{line:?} was read as advertising the service"
        );
    }

    // Including the one that is a real UUID on a line that is not a UUID line:
    // a device whose NAME contains the MDR uuid must not be taken for one that
    // offers the service.
    let spoof = "\tAlias: UUID: Vendor specific (96cc203e-5068-46ad-b32d-e316f5e069ba)";
    assert!(!link::advertises(spoof, link::MDR_SERVICE_V1));
}

// ── the one control session, and what its refusal should say ─────────────────

/// EBUSY carries a cause a person can act on, and the errno survives.
///
/// The kernel says "Device or resource busy", which sends somebody to look at
/// their computer. The real cause is almost always at the other end: the
/// headset grants ONE MDR control session across everything it is paired with,
/// so a phone on multipoint holds it while A2DP audio keeps playing --
/// observed on hardware by delarosa1312/omarchy-sony-xm5, docs/protocol.md.
///
/// The errno stays in the string because the guess is a guess, and whoever is
/// debugging a connect failure that is NOT this needs the real one.
#[test]
fn a_busy_link_explains_that_something_else_holds_the_session() {
    let err = link::LinkError::Io(std::io::Error::from_raw_os_error(16));
    let said = err.to_string();
    assert!(said.contains("one control session"), "explains the cause: {said}");
    assert!(said.contains("paired"), "names where to look: {said}");
    assert!(
        said.contains("busy") || said.contains("Busy"),
        "and keeps the errno: {said}"
    );
}

#[test]
fn a_connect_timeout_gets_the_same_explanation() {
    // The same cause reaches us as either errno, per their observation, so a
    // message on only one of the two would be right half the time.
    let err = link::LinkError::Io(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "Timed out Connecting to remote device",
    ));
    assert!(err.to_string().contains("one control session"));
}

#[test]
fn an_unrelated_io_error_is_left_alone() {
    // The negative control. A guess attached to every failure is not a guess,
    // it is noise -- ENODEV means the adapter is gone and has nothing to do
    // with another device holding a session.
    let err = link::LinkError::Io(std::io::Error::from_raw_os_error(19));
    assert!(!err.to_string().contains("one control session"));
}
