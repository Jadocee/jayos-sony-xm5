//! The MDR wire frame: markers, escaping, checksum, pack and unpack.
//!
//! This is the whole byte-level protocol and nothing else. It performs no I/O,
//! holds no state and knows nothing about what a payload means -- so the 70
//! captured packets in `tests/fixtures/mdr/` can be fed to it directly, and the
//! only thing that can make those tests pass is a codec that agrees with a real
//! Sony headset byte for byte.
//!
//! THE FRAME
//!
//! ```text
//! 0x3E  escape( type | seq | len:u32be | payload | checksum )  0x3C
//! ```
//!
//! - `len` counts the UNESCAPED payload only. Not the header, not the checksum,
//!   not the markers, and not the post-escape wire length.
//! - The checksum is an 8-bit wrapping sum over type + seq + len + payload,
//!   computed BEFORE escaping and then escaped along with everything else.
//! - The markers are outside the escaped region and are never themselves
//!   escaped, which is what makes them unambiguous frame boundaries.
//!
//! Minimum frame is 9 bytes: two markers, a six-byte header and a checksum,
//! with no payload. Every ACK in the corpus is exactly that.
//!
//! WHY NOTHING HERE PANICS
//!
//! The release profile sets `panic = "abort"`, so a panic in a daemon reading
//! from a Bluetooth socket is a SIGABRT with the backtrace stripped -- the
//! headset goes quiet and nothing says why. Every function that reads bytes
//! returns a `Result`, and the module denies the lints that would let an index
//! or an unwrap back in.
//!
//! This is where the reference implementation is at its weakest and the port
//! deliberately differs: `MDRUnescape` in the C++ signals failure by returning
//! an empty vector, and `MDRUnpackCommand` never checks for it before indexing
//! it. Here those are typed errors.
//!
//! THE CORPUS DOES NOT REACH THAT PATH, and this paragraph used to claim two
//! frames did. Counted: of the 70 captured frames, exactly ONE contains an
//! escape sentry at all, and NONE carries a malformed escape -- which is what
//! `MDRUnescape` returns empty for. `every_captured_frame_decodes` proves the
//! second half (all 70 decode) and
//! `exactly_one_captured_frame_contains_an_escape` holds the first, so neither
//! number can go stale the way the sentence they replace did.
//!
//! So this is a latent defect closed, not an observed crash fixed, and it is
//! worth saying which: a capture of ordinary traffic is exactly where a
//! malformed frame would NOT appear. The `FrameError` arms are reached by
//! hand-built inputs in `mdr_frame.rs`, not by the corpus.

use super::DataType;

/// Frame boundaries. Outside the escaped region, so a scanner can split a
/// stream on them without decoding anything.
pub const START: u8 = 0x3E;
pub const END: u8 = 0x3C;

/// The escape sentry, and the three bytes that follow it.
///
/// Exactly three byte values are escaped -- the two markers and the sentry
/// itself -- and every other byte passes through untouched. The escapee is the
/// original minus 0x10, which is a coincidence of the constants rather than a
/// rule: they are spelled out so a reader does not infer arithmetic that the
/// reference does not do.
pub const SENTRY: u8 = 0x3D;
pub const ESC_END: u8 = 0x2C;
pub const ESC_SENTRY: u8 = 0x2D;
pub const ESC_START: u8 = 0x2E;

/// type(1) + seq(1) + len(4).
pub const HEADER_LEN: usize = 6;
/// The header plus the checksum: the smallest body a frame can carry.
pub const MIN_BODY: usize = HEADER_LEN + 1;
/// The body plus both markers.
pub const MIN_FRAME: usize = MIN_BODY + 2;

/// What a frame carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub data_type: DataType,
    pub seq: u8,
    pub payload: Vec<u8>,
}

/// Why a sequence of bytes is not a frame.
///
/// Every variant names something a real stream can contain, and each is
/// distinguishable: a caller retrying a truncated read must be able to tell
/// "not all here yet" from "these bytes will never be a frame".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// Fewer than [`MIN_FRAME`] bytes.
    TooShort,
    /// The first byte is not [`START`].
    NoStart,
    /// The last byte is not [`END`].
    NoEnd,
    /// A sentry was the final byte of the escaped region, so its escapee is
    /// missing. The reference returns an empty buffer here and then indexes it.
    TrailingSentry,
    /// A sentry was followed by a byte that is not one of the three escapees.
    /// The reference returns an empty buffer here too.
    BadEscapee(u8),
    /// The unescaped body is shorter than a header plus a checksum.
    BodyTooShort,
    /// The length field disagrees with how many payload bytes are present.
    /// `declared` is what the header said; `actual` is what was there.
    LengthMismatch { declared: usize, actual: usize },
    /// The trailing byte is not the sum of everything before it.
    BadChecksum { declared: u8, computed: u8 },
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::TooShort => write!(f, "shorter than the {MIN_FRAME}-byte minimum frame"),
            FrameError::NoStart => write!(f, "does not begin with the start marker"),
            FrameError::NoEnd => write!(f, "does not end with the end marker"),
            FrameError::TrailingSentry => write!(f, "escape sentry with nothing after it"),
            FrameError::BadEscapee(b) => write!(f, "escape sentry followed by {b:#04x}"),
            FrameError::BodyTooShort => write!(f, "body is shorter than a header and a checksum"),
            FrameError::LengthMismatch { declared, actual } => {
                write!(f, "header declares {declared} payload bytes, {actual} present")
            }
            FrameError::BadChecksum { declared, computed } => {
                write!(f, "checksum {declared:#04x}, computed {computed:#04x}")
            }
        }
    }
}

/// The 8-bit wrapping sum the protocol uses.
///
/// No seed, no complement, no carry fold. Written as a wrapping add over `u8`
/// rather than a wide sum truncated at the end: the two are identical, but the
/// intent is that the arithmetic is modular, and a reader should not have to
/// work out whether a `u32` accumulator was meant to be masked later.
pub fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |acc, &b| acc.wrapping_add(b))
}

/// Escape the three special bytes, appending to `out`.
pub fn escape_into(src: &[u8], out: &mut Vec<u8>) {
    for &b in src {
        match b {
            END => out.extend_from_slice(&[SENTRY, ESC_END]),
            SENTRY => out.extend_from_slice(&[SENTRY, ESC_SENTRY]),
            START => out.extend_from_slice(&[SENTRY, ESC_START]),
            _ => out.push(b),
        }
    }
}

/// Undo [`escape_into`].
///
/// A `Result`, unlike the reference, which returns an empty vector for both
/// failure modes and leaves the caller to index it.
pub fn unescape(src: &[u8]) -> Result<Vec<u8>, FrameError> {
    let mut out = Vec::with_capacity(src.len());
    let mut iter = src.iter().copied();
    while let Some(b) = iter.next() {
        if b != SENTRY {
            out.push(b);
            continue;
        }
        match iter.next() {
            None => return Err(FrameError::TrailingSentry),
            Some(ESC_END) => out.push(END),
            Some(ESC_SENTRY) => out.push(SENTRY),
            Some(ESC_START) => out.push(START),
            Some(other) => return Err(FrameError::BadEscapee(other)),
        }
    }
    Ok(out)
}

/// Build a frame.
///
/// The payload length is written as `u32` big-endian. A payload longer than
/// `u32::MAX` cannot be expressed and cannot occur -- the transport reads in
/// 2 KiB chunks and nothing in this tree queues a payload over that -- but the
/// cast is saturating rather than wrapping so that an impossible input would
/// produce an over-long declared length (which the peer rejects) rather than a
/// short one (which would resynchronise the stream in the middle of a payload).
pub fn pack(data_type: DataType, seq: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + (MIN_BODY + payload.len()) * 2);
    pack_into(data_type, seq, payload, &mut out);
    out
}

/// [`pack`], appending to a buffer the caller owns.
pub fn pack_into(data_type: DataType, seq: u8, payload: &[u8], out: &mut Vec<u8>) {
    let declared = u32::try_from(payload.len()).unwrap_or(u32::MAX);

    let mut body = Vec::with_capacity(MIN_BODY + payload.len());
    body.push(data_type.to_wire());
    body.push(seq);
    body.extend_from_slice(&declared.to_be_bytes());
    body.extend_from_slice(payload);
    body.push(checksum(&body));

    out.push(START);
    escape_into(&body, out);
    out.push(END);
}

/// The ACK for a frame that arrived with sequence number `seq`.
///
/// `1 - seq`, which is the reference's rule and is why the sequence number
/// behaves as a single alternating bit rather than a counter. An ACK carries no
/// payload, so this is always the same two 9-byte strings -- and both of them
/// appear in the corpus, which is what lets an RX-only capture verify something
/// this port TRANSMITS.
pub fn ack_frame(seq: u8) -> Vec<u8> {
    pack(DataType::Ack, 1u8.wrapping_sub(seq), &[])
}

/// Read one complete frame.
///
/// The input must be exactly one frame: the first byte is the start marker, the
/// last is the end marker, and everything between them is its escaped body.
/// Splitting a stream into those spans is [`Scanner`]'s job.
pub fn unpack(bytes: &[u8]) -> Result<Frame, FrameError> {
    if bytes.len() < MIN_FRAME {
        return Err(FrameError::TooShort);
    }
    let (Some(&first), Some(&last)) = (bytes.first(), bytes.last()) else {
        return Err(FrameError::TooShort);
    };
    if first != START {
        return Err(FrameError::NoStart);
    }
    if last != END {
        return Err(FrameError::NoEnd);
    }

    // Both markers are known present, so this cannot underflow.
    let escaped = bytes.get(1..bytes.len() - 1).ok_or(FrameError::TooShort)?;
    let body = unescape(escaped)?;

    if body.len() < MIN_BODY {
        return Err(FrameError::BodyTooShort);
    }

    // The checksum covers everything before it, and is compared before the
    // payload is trusted: a length field is as corruptible as a payload byte,
    // and acting on a bad length first is how a decoder reads out of bounds.
    let (summed, tail) = body.split_at(body.len().saturating_sub(1));
    let declared_sum = tail.first().copied().unwrap_or(0);
    let computed = checksum(summed);
    if declared_sum != computed {
        return Err(FrameError::BadChecksum { declared: declared_sum, computed });
    }

    let header = summed.get(..HEADER_LEN).ok_or(FrameError::BodyTooShort)?;
    let data_type = DataType::from_wire(header.first().copied().unwrap_or(0));
    let seq = header.get(1).copied().unwrap_or(0);

    let len_bytes: [u8; 4] = header.get(2..6).and_then(|s| s.try_into().ok()).ok_or(FrameError::BodyTooShort)?;
    let declared = u32::from_be_bytes(len_bytes) as usize;

    let payload = summed.get(HEADER_LEN..).unwrap_or(&[]);
    if payload.len() != declared {
        return Err(FrameError::LengthMismatch { declared, actual: payload.len() });
    }

    Ok(Frame { data_type, seq, payload: payload.to_vec() })
}

/// Splits a byte stream into frames.
///
/// A Bluetooth read returns whatever arrived, which is not a frame: it can be
/// half of one, several, or a frame plus the start of the next. The scanner
/// holds the remainder between reads.
///
/// It resynchronises rather than giving up. Anything before the next start
/// marker is discarded -- a byte outside a frame has no meaning, and a decoder
/// that stops at the first one stays stopped for the rest of the session.
#[derive(Debug, Default)]
pub struct Scanner {
    buf: Vec<u8>,
}

/// The largest the scanner's buffer may grow before it is abandoned.
///
/// Without a cap a peer that sends a start marker and then never an end marker
/// grows this without bound, which on a long-lived daemon is the whole session's
/// memory. 64 KiB is far past the largest frame in the corpus (283 bytes) and
/// past any payload the protocol defines.
pub const MAX_BUFFER: usize = 64 * 1024;

impl Scanner {
    pub fn new() -> Scanner {
        Scanner::default()
    }

    /// Bytes held from previous reads.
    pub fn buffered(&self) -> usize {
        self.buf.len()
    }

    /// Add what a read returned.
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Take the next complete frame, if there is one.
    ///
    /// `Some(Err(..))` is a frame-shaped span that did not decode: the span is
    /// consumed, so the caller can report it and carry on rather than seeing it
    /// again on every call.
    pub fn next_frame(&mut self) -> Option<Result<Frame, FrameError>> {
        let Some(start) = self.buf.iter().position(|&b| b == START) else {
            // Not one byte here can begin a frame, so none of it is worth
            // keeping. Without this a peer that sends anything except a start
            // marker -- a wrong-service connection, line noise, a device
            // speaking something else entirely -- grows this buffer for the
            // life of the daemon, and the MAX_BUFFER guard below never sees it
            // because that path is only reached once a start marker exists.
            self.buf.clear();
            return None;
        };
        if start > 0 {
            // Bytes before a start marker are not part of any frame.
            self.buf.drain(..start);
        }

        // Search from index 1: a start marker is never escaped, so the first
        // end marker after it closes this frame.
        let Some(end) = self.buf.iter().skip(1).position(|&b| b == END).map(|i| i + 1) else {
            // A start marker with no end yet. Ordinarily that is half a frame
            // and the rest is still in flight; past MAX_BUFFER it is a peer
            // that will never finish one.
            if self.buf.len() > MAX_BUFFER {
                self.buf.clear();
            }
            return None;
        };

        // Consumed whether or not it decodes, so a span that will never be a
        // frame is reported once rather than returned again on every call.
        let span: Vec<u8> = self.buf.drain(..=end).collect();
        Some(unpack(&span))
    }
}
