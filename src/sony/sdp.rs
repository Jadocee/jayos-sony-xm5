//! Just enough Service Discovery Protocol to ask a headset which RFCOMM
//! channel its MDR service is on.
//!
//! WHY THIS EXISTS AT ALL
//!
//! An RFCOMM connection needs a channel number, and the channel is not fixed:
//! it is whatever the device's own service record says, and it differs between
//! Sony models and between firmware versions on one model. The usual way to
//! ask is `libbluetooth`'s `sdp_service_search_attr_req`, which is a C library
//! this project does not link and would have to. The alternative -- probing
//! channels 1 to 30 and keeping whichever accepts -- means opening connections
//! to hands-free and remote-control profiles on the user's headset to see what
//! happens, which is not a thing to do to somebody's hardware.
//!
//! So the query is written out here. It is about two hundred lines because SDP
//! is a small protocol and only one question is being asked of it.
//!
//! PROVENANCE, AND HOW IT DIFFERS FROM THE REST OF THIS PORT
//!
//! The MDR decoder next door was written from captured frames, and its prose
//! cites the capture for every claim. NOTHING HERE IS FROM A CAPTURE. SDP is a
//! published protocol -- Bluetooth Core Specification, Vol 3, Part B -- and
//! every byte below is the specification's encoding, cited by section. The
//! test fixtures are constructed from that specification rather than recorded
//! from a device, and they say so. Treating a spec-derived encoding as though
//! it were evidence from hardware would be the same mistake in the other
//! direction: this is the one part of the port where the document IS the
//! authority.
//!
//! WHAT A RESPONSE IS
//!
//! Bytes from a device across a radio link, before anything has authenticated
//! it. Every length in it is a claim, every nesting is a claim, and the parser
//! below treats them that way: nothing is allocated on a declared size that
//! has not been checked against what is actually there, the nesting depth is
//! bounded, and so is the number of elements. A malformed record is an error,
//! never a panic.

/// A UUID as SDP carries it: 16, 32 or 128 bits (Vol 3, Part B, 3.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Uuid {
    Short(u16),
    Medium(u32),
    Long([u8; 16]),
}

/// The Bluetooth Base UUID (Vol 3, Part B, 2.5.1). A 16- or 32-bit UUID is
/// shorthand for this with the value in the first four bytes.
const BASE: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0x80, 0x5F, 0x9B, 0x34, 0xFB,
];

impl Uuid {
    /// The 128-bit form, so two UUIDs written at different widths compare.
    ///
    /// A device may answer a 16-bit query with a 128-bit record or the other
    /// way round; both are legal and mean the same service.
    pub fn wide(self) -> [u8; 16] {
        match self {
            Uuid::Long(bytes) => bytes,
            Uuid::Short(v) => Uuid::Medium(u32::from(v)).wide(),
            Uuid::Medium(v) => {
                let mut out = BASE;
                out[..4].copy_from_slice(&v.to_be_bytes());
                out
            }
        }
    }
}

/// The RFCOMM protocol identifier (Assigned Numbers, Protocol Identifiers).
const RFCOMM: Uuid = Uuid::Short(0x0003);

/// One SDP data element (Vol 3, Part B, 3.3).
#[derive(Debug, Clone, PartialEq)]
pub enum Element {
    Nil,
    Uint(u128),
    Int(i128),
    Uuid(Uuid),
    /// Kept as bytes. SDP text is not required to be UTF-8 and nothing here
    /// reads it; decoding it would be inventing a failure mode.
    Text(Vec<u8>),
    Bool(bool),
    Sequence(Vec<Element>),
    Alternative(Vec<Element>),
    Url(Vec<u8>),
}

#[derive(Debug, PartialEq, Eq)]
pub enum SdpError {
    /// The bytes ran out inside an element that said it was longer.
    Truncated,
    /// A type or size combination the specification does not define.
    Malformed,
    /// Nested past [`MAX_DEPTH`], or more elements than [`MAX_ELEMENTS`].
    TooDeep,
    /// A response that is not the one this request asked for.
    Unexpected(&'static str),
    /// The device answered with an SDP_ErrorResponse (PDU 0x01).
    Refused(u16),
}

impl std::fmt::Display for SdpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SdpError::Truncated => write!(f, "the service record ended mid-element"),
            SdpError::Malformed => write!(f, "the service record is not valid SDP"),
            SdpError::TooDeep => write!(f, "the service record nests further than is plausible"),
            SdpError::Unexpected(what) => write!(f, "the device answered with {what}"),
            SdpError::Refused(code) => write!(f, "the device refused the query (SDP error {code})"),
        }
    }
}

/// How far a record may nest.
///
/// A real ProtocolDescriptorList is three deep and the deepest thing SDP
/// defines is not much more. This is not a limit on legal records; it is a
/// limit on a remote device sending a thousand open sequences and taking the
/// daemon's stack with it.
const MAX_DEPTH: usize = 16;

/// How many elements one record may contain, in total across all nesting.
///
/// Same reason. A two-byte header can declare a sequence, so a short response
/// can otherwise ask for a great many small allocations.
const MAX_ELEMENTS: usize = 4096;

/// Parse one data element, returning it and how many bytes it used.
pub fn element(bytes: &[u8]) -> Result<(Element, usize), SdpError> {
    let mut budget = MAX_ELEMENTS;
    parse_at(bytes, 0, &mut budget)
}

fn parse_at(bytes: &[u8], depth: usize, budget: &mut usize) -> Result<(Element, usize), SdpError> {
    if depth > MAX_DEPTH {
        return Err(SdpError::TooDeep);
    }
    if *budget == 0 {
        return Err(SdpError::TooDeep);
    }
    *budget -= 1;

    let header = *bytes.first().ok_or(SdpError::Truncated)?;
    let kind = header >> 3;
    let size_index = header & 0x07;
    let rest = &bytes[1..];

    // The size index means different things per type (Vol 3, Part B, 3.3):
    // for the fixed-width types it selects a width, and 5/6/7 always mean a
    // length that follows in 1, 2 or 4 bytes.
    let (len, header_extra) = match size_index {
        0 => (if kind == 0 { 0 } else { 1 }, 0),
        1 => (2, 0),
        2 => (4, 0),
        3 => (8, 0),
        4 => (16, 0),
        5 => (usize::from(*rest.first().ok_or(SdpError::Truncated)?), 1),
        6 => {
            let raw: [u8; 2] = rest.get(..2).ok_or(SdpError::Truncated)?.try_into().unwrap();
            (usize::from(u16::from_be_bytes(raw)), 2)
        }
        7 => {
            let raw: [u8; 4] = rest.get(..4).ok_or(SdpError::Truncated)?.try_into().unwrap();
            (u32::from_be_bytes(raw) as usize, 4)
        }
        _ => return Err(SdpError::Malformed),
    };

    let body = rest.get(header_extra..header_extra + len).ok_or(SdpError::Truncated)?;
    let used = 1 + header_extra + len;

    let value = match kind {
        0 => Element::Nil,
        1 => Element::Uint(be_uint(body)?),
        2 => Element::Int(be_int(body)?),
        3 => Element::Uuid(match body.len() {
            2 => Uuid::Short(u16::from_be_bytes([body[0], body[1]])),
            4 => Uuid::Medium(u32::from_be_bytes([body[0], body[1], body[2], body[3]])),
            16 => Uuid::Long(body.try_into().unwrap()),
            _ => return Err(SdpError::Malformed),
        }),
        4 => Element::Text(body.to_vec()),
        5 => Element::Bool(*body.first().ok_or(SdpError::Truncated)? != 0),
        6 | 7 => {
            let mut items = Vec::new();
            let mut at = 0usize;
            while at < body.len() {
                let (item, n) = parse_at(&body[at..], depth + 1, budget)?;
                // A zero-width element would be an infinite loop rather than
                // an error, and only Nil is legitimately zero-width -- which
                // still uses its own header byte, so n is never 0. Belt and
                // braces, against a parser change rather than against input.
                if n == 0 {
                    return Err(SdpError::Malformed);
                }
                at += n;
                items.push(item);
            }
            if kind == 6 {
                Element::Sequence(items)
            } else {
                Element::Alternative(items)
            }
        }
        8 => Element::Url(body.to_vec()),
        _ => return Err(SdpError::Malformed),
    };
    Ok((value, used))
}

fn be_uint(bytes: &[u8]) -> Result<u128, SdpError> {
    if bytes.is_empty() || bytes.len() > 16 {
        return Err(SdpError::Malformed);
    }
    Ok(bytes.iter().fold(0u128, |acc, b| (acc << 8) | u128::from(*b)))
}

fn be_int(bytes: &[u8]) -> Result<i128, SdpError> {
    let raw = be_uint(bytes)?;
    let bits = bytes.len() * 8;
    // Sign-extend from the declared width. `1i128 << 127` would overflow the
    // shift for a 16-byte integer, so that width is handled as itself.
    if bits == 128 {
        return Ok(raw as i128);
    }
    let sign = 1u128 << (bits - 1);
    Ok(if raw & sign != 0 { (raw as i128) - ((1i128) << bits) } else { raw as i128 })
}

/// The RFCOMM channel in a ProtocolDescriptorList, if there is one.
///
/// The shape being looked for (Vol 3, Part B, 5.1.5) is a sequence whose first
/// element is the RFCOMM protocol UUID and whose second is the channel:
///
/// ```text
/// Sequence [ Sequence [ Uuid(L2CAP), Uint(0x0100) ],
///            Sequence [ Uuid(RFCOMM), Uint(9) ] ]
/// ```
///
/// Searched recursively rather than at a fixed path, because a record may wrap
/// it: a device offering the service over more than one transport answers with
/// an Alternative of these lists, and the attribute itself arrives inside the
/// attribute-list sequences. The first match wins, which is the same choice
/// `sdp_get_proto_port` makes.
pub fn rfcomm_channel(element: &Element) -> Option<u8> {
    if let Element::Sequence(items) | Element::Alternative(items) = element {
        if let [Element::Uuid(id), Element::Uint(channel), ..] = items.as_slice() {
            if id.wide() == RFCOMM.wide() && *channel >= 1 && *channel <= 30 {
                return Some(*channel as u8);
            }
        }
        for item in items {
            if let Some(found) = rfcomm_channel(item) {
                return Some(found);
            }
        }
    }
    None
}

// ── the request ──────────────────────────────────────────────────────────────

/// PDU identifiers (Vol 3, Part B, 4.4).
const PDU_ERROR: u8 = 0x01;
const PDU_SEARCH_ATTR_REQUEST: u8 = 0x06;
const PDU_SEARCH_ATTR_RESPONSE: u8 = 0x07;

/// The ProtocolDescriptorList attribute (Vol 3, Part B, 5.1.5).
pub const ATTR_PROTOCOL_DESCRIPTOR_LIST: u16 = 0x0004;

/// How many bytes of attributes to accept in one response.
///
/// The L2CAP MTU SDP is required to support is 672 (Vol 3, Part B, 7.1), and
/// asking for more than the peer can send in one PDU just means continuing.
const MAX_ATTRIBUTE_BYTES: u16 = 640;

/// A ServiceSearchAttributeRequest for one service's one attribute.
///
/// `continuation` is what a previous response asked to be echoed back, or
/// empty on the first request.
pub fn search_request(transaction: u16, service: Uuid, attribute: u16, continuation: &[u8]) -> Vec<u8> {
    let mut params = Vec::with_capacity(64);

    // ServiceSearchPattern: a sequence holding the one UUID.
    let uuid = data_uuid(service);
    params.push(0x35); // Sequence, 1-byte length
    params.push(uuid.len() as u8);
    params.extend_from_slice(&uuid);

    params.extend_from_slice(&MAX_ATTRIBUTE_BYTES.to_be_bytes());

    // AttributeIDList: a sequence holding one 32-bit range, low..=high. A
    // range rather than a bare id because a device is allowed to answer a
    // range with nothing, and asking for `attr..=attr` is how one attribute is
    // requested without a second code path.
    let range = (u32::from(attribute) << 16) | u32::from(attribute);
    params.push(0x35);
    params.push(5);
    params.push(0x0A); // Unsigned int, 4 bytes
    params.extend_from_slice(&range.to_be_bytes());

    params.push(continuation.len() as u8);
    params.extend_from_slice(continuation);

    let mut pdu = Vec::with_capacity(5 + params.len());
    pdu.push(PDU_SEARCH_ATTR_REQUEST);
    pdu.extend_from_slice(&transaction.to_be_bytes());
    pdu.extend_from_slice(&(params.len() as u16).to_be_bytes());
    pdu.extend_from_slice(&params);
    pdu
}

fn data_uuid(uuid: Uuid) -> Vec<u8> {
    match uuid {
        Uuid::Short(v) => {
            let mut out = vec![0x19];
            out.extend_from_slice(&v.to_be_bytes());
            out
        }
        Uuid::Medium(v) => {
            let mut out = vec![0x1A];
            out.extend_from_slice(&v.to_be_bytes());
            out
        }
        Uuid::Long(v) => {
            let mut out = vec![0x1C];
            out.extend_from_slice(&v);
            out
        }
    }
}

/// One response: the attribute bytes it carried, and the continuation to send
/// back if there is more.
#[derive(Debug)]
pub struct Response {
    pub attributes: Vec<u8>,
    pub continuation: Vec<u8>,
}

/// Read a ServiceSearchAttributeResponse.
///
/// Every length is checked against what actually arrived, including the two
/// that a device could use to make this allocate: the PDU's parameter length
/// and the attribute-list byte count.
pub fn read_response(pdu: &[u8], transaction: u16) -> Result<Response, SdpError> {
    let header = pdu.get(..5).ok_or(SdpError::Truncated)?;
    let id = header[0];
    let seen = u16::from_be_bytes([header[1], header[2]]);
    let length = usize::from(u16::from_be_bytes([header[3], header[4]]));
    let params = pdu.get(5..5 + length).ok_or(SdpError::Truncated)?;

    if seen != transaction {
        return Err(SdpError::Unexpected("a reply to a different question"));
    }
    if id == PDU_ERROR {
        let code = params.get(..2).ok_or(SdpError::Truncated)?;
        return Err(SdpError::Refused(u16::from_be_bytes([code[0], code[1]])));
    }
    if id != PDU_SEARCH_ATTR_RESPONSE {
        return Err(SdpError::Unexpected("a message that answers a different request"));
    }

    let count = usize::from(u16::from_be_bytes([
        *params.first().ok_or(SdpError::Truncated)?,
        *params.get(1).ok_or(SdpError::Truncated)?,
    ]));
    let attributes = params.get(2..2 + count).ok_or(SdpError::Truncated)?.to_vec();

    let tail = &params[2 + count..];
    let continuation_len = usize::from(*tail.first().ok_or(SdpError::Truncated)?);
    // The field is one byte, so the specification's own maximum is 255; a
    // device claiming more than the PDU holds is caught here rather than by an
    // allocation.
    let continuation = tail.get(1..1 + continuation_len).ok_or(SdpError::Truncated)?.to_vec();

    Ok(Response { attributes, continuation })
}
