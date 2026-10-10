//! The link to the headset: a Bluetooth socket, and how one is found.
//!
//! THERE IS NO TRANSPORT TRAIT HERE, DELIBERATELY.
//!
//! An RFCOMM socket and a `socketpair` are both file descriptors that read,
//! write, dup and shut down through the same four syscalls; the only thing
//! that differs is the address passed to `connect`. So the type below wraps a
//! descriptor rather than abstracting over one, and a test builds one from
//! `socketpair`. The fake is a REAL SOCKET taking the real code path, rather
//! than an implementation of a trait that only the tests use -- which is the
//! usual way a transport layer comes to be tested and the daemon still fails
//! the first time it meets a socket.
//!
//! WHAT IS NOT TESTED HERE, and cannot be: `connect` against a Bluetooth
//! address. There is no adapter on the machine this is built on, and a mocked
//! `connect` would assert that this file calls the function it obviously
//! calls. What IS tested is everything either side of it -- the address
//! encoding, the timeout, the read and write behaviour, the shutdown -- over
//! sockets that behave identically.
//!
//! TWO SERVICE UUIDS, FROM libmdr's `Base.h`: [`MDR_SERVICE_V2`]
//! (`MDR_SERVICE_UUID_XM5`, "XM5s and newer") and [`MDR_SERVICE_V1`]
//! (`MDR_SERVICE_UUID_LEGACY`, "XM4s and older"). A WH-1000XM5 advertises
//! ONLY the V2 one -- a public `bluetoothctl info` dump of one (Arch forum
//! post 2298135, Modalias usb:v054Cp0DF0d0251) lists 956c7b26-... and not
//! 96cc203e-..., and the C++ daemon's docs/COMPATIBILITY.md records the same
//! device connecting over "MDR v2" -- so a daemon that knew the legacy UUID
//! alone never found it. The daemon dials only what a device advertises, V2
//! first when both are there -- the order Gadgetbridge's `getUuidToConnect`
//! and the C++ daemon's `startAttempt` use -- and a device that advertises
//! neither is REPORTED as not advertising MDR rather than probed on a likely
//! channel, so a person is told something they can act on instead of the
//! daemon opening connections to whatever else is on the headset.

use super::sdp::{self, SdpError, Uuid};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::time::Duration;

/// libmdr `Base.h`: `MDR_SERVICE_UUID_XM5`, "XM5s and newer".
///
/// The one a WH-1000XM5 advertises. The C++ daemon's `protocolGeneration`
/// reports 2 for it, and so does [`mdr_services`].
pub const MDR_SERVICE_V2: Uuid = Uuid::Long([
    0x95, 0x6C, 0x7B, 0x26, 0xD4, 0x9A, 0x4B, 0xA8, 0xB0, 0x3F, 0xB1, 0x7D, 0x39, 0x3C, 0xB6, 0xE2,
]);

/// libmdr `Base.h`: `MDR_SERVICE_UUID_LEGACY`, "XM4s and older". Generation 1.
pub const MDR_SERVICE_V1: Uuid = Uuid::Long([
    0x96, 0xCC, 0x20, 0x3E, 0x50, 0x68, 0x46, 0xAD, 0xB3, 0x2D, 0xE3, 0x16, 0xF5, 0xE0, 0x69, 0xBA,
]);

/// An MDR service a device advertises, and the generation the state
/// document's `device.protocol` reports for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mdr {
    pub service: Uuid,
    pub generation: i32,
}

/// The MDR services that `bluetoothctl info` output advertises, newest first.
///
/// Empty for a device that is not a Sony headset. The order is the dialling
/// order: V2 before V1, as Gadgetbridge's `getUuidToConnect` (`if (hasV2) ...
/// else if (hasV1)`) and the C++ daemon's candidate list (`MDR_SERVICE_UUID_XM5`
/// pushed before `MDR_SERVICE_UUID_LEGACY`) both settle it.
pub fn mdr_services(info: &str) -> Vec<Mdr> {
    [(MDR_SERVICE_V2, 2), (MDR_SERVICE_V1, 1)]
        .into_iter()
        .filter(|(service, _)| advertises(info, *service))
        .map(|(service, generation)| Mdr { service, generation })
        .collect()
}

/// Bluetooth socket protocols, from `include/net/bluetooth/bluetooth.h`.
///
/// Written out because the `libc` crate does not carry them. They are kernel
/// ABI: the numbers cannot change without breaking every Bluetooth program on
/// Linux.
const BTPROTO_L2CAP: libc::c_int = 0;
const BTPROTO_RFCOMM: libc::c_int = 3;

/// The SDP protocol/service multiplexer (Vol 3, Part A, 4.2).
const PSM_SDP: u16 = 0x0001;

/// A Bluetooth device address.
///
/// STORED IN WRITTEN ORDER -- `AA:BB:CC:DD:EE:FF` is `[0xAA, .., 0xFF]` -- and
/// reversed only where the kernel wants it. `bdaddr_t` is little-endian, which
/// is the single most reliable way to get a Bluetooth program subtly wrong:
/// the connection fails with ENODEV or, worse, reaches a different device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mac(pub [u8; 6]);

impl Mac {
    pub fn parse(text: &str) -> Option<Mac> {
        let mut out = [0u8; 6];
        let mut seen = 0usize;
        for (n, part) in text.split(':').enumerate() {
            if n >= 6 || part.len() != 2 {
                return None;
            }
            out[n] = u8::from_str_radix(part, 16).ok()?;
            seen += 1;
        }
        (seen == 6).then_some(Mac(out))
    }

    /// The kernel's byte order: written order, reversed.
    fn bdaddr(self) -> [u8; 6] {
        let mut out = self.0;
        out.reverse();
        out
    }
}

impl std::fmt::Display for Mac {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [a, b, c, d, e, g] = self.0;
        write!(f, "{a:02X}:{b:02X}:{c:02X}:{d:02X}:{e:02X}:{g:02X}")
    }
}

/// A socket address, laid out by hand.
///
/// WHY BYTES AND NOT A `#[repr(C)]` STRUCT. Both Bluetooth address structs
/// have a trailing pad byte, and the padding in a Rust struct is uninitialised
/// -- reading it to hand to a syscall is undefined, and handing the kernel
/// whatever was on the stack is a small leak into a structure it may echo
/// back. Writing the layout out zeroes the padding and, more usefully, makes
/// the layout itself something a test can assert rather than something a
/// reader has to trust.
///
/// Aligned to 2 because that is `sockaddr`'s alignment, and the pointer is
/// cast to one.
#[repr(C, align(2))]
struct RawAddr<const N: usize>([u8; N]);

impl<const N: usize> RawAddr<N> {
    fn as_sockaddr(&self) -> *const libc::sockaddr {
        self.0.as_ptr().cast::<libc::sockaddr>()
    }
}

/// `struct sockaddr_rc`, from `include/net/bluetooth/rfcomm.h`:
/// `sa_family_t` (2), `bdaddr_t` (6), `uint8_t` channel (1), then one byte of
/// padding to the struct's alignment.
///
/// Public so the byte order below can be asserted. It is the only thing in
/// this file that cannot be checked without a Bluetooth adapter and the only
/// one that fails by reaching the wrong device rather than by failing.
pub fn rfcomm_sockaddr(mac: Mac, channel: u8) -> [u8; 10] {
    let mut out = [0u8; 10];
    out[0..2].copy_from_slice(&(libc::AF_BLUETOOTH as u16).to_ne_bytes());
    out[2..8].copy_from_slice(&mac.bdaddr());
    out[8] = channel;
    out
}

/// `struct sockaddr_l2`, from `include/net/bluetooth/l2cap.h`:
/// `sa_family_t` (2), PSM (2), `bdaddr_t` (6), CID (2), address type (1), pad.
///
/// The PSM and the CID are LITTLE-ENDIAN -- BlueZ writes them through
/// `htobs()` -- while the family is host order like any socket address. Three
/// fields, three different rules, in fourteen bytes.
pub fn l2cap_sockaddr(mac: Mac, psm: u16) -> [u8; 14] {
    let mut out = [0u8; 14];
    out[0..2].copy_from_slice(&(libc::AF_BLUETOOTH as u16).to_ne_bytes());
    out[2..4].copy_from_slice(&psm.to_le_bytes());
    out[4..10].copy_from_slice(&mac.bdaddr());
    // The CID is zero when connecting by PSM, and the address type is
    // BDADDR_BREDR: classic Bluetooth, which is what MDR runs over.
    out
}

/// A byte pipe to the headset.
///
/// Owns its descriptor and closes it on drop. Reads and writes take `&self`
/// rather than `&mut self` on purpose: the daemon has a thread blocked in
/// `read` while the engine thread writes, and a socket is safe for exactly
/// that. A `&mut` interface would force a lock around the write and the reader
/// would hold it for as long as the headset stayed quiet.
pub struct Channel {
    fd: OwnedFd,
}

impl Channel {
    /// # Safety
    ///
    /// `fd` must be an open, connected socket that nothing else owns.
    pub fn from_fd(fd: OwnedFd) -> Channel {
        Channel { fd }
    }

    /// A second handle on the same connection.
    ///
    /// `dup`, not a new connection: the reader thread and the writer must be
    /// on the same socket, and closing one half must not close the other.
    pub fn try_clone(&self) -> io::Result<Channel> {
        Ok(Channel { fd: self.fd.try_clone()? })
    }

    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        // SAFETY: `buf` is a live slice of `buf.len()` bytes and the
        // descriptor is owned by `self` for the call.
        let n = unsafe {
            libc::read(self.fd.as_raw_fd(), buf.as_mut_ptr().cast::<libc::c_void>(), buf.len())
        };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(n as usize)
    }

    pub fn write_all(&self, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            // SAFETY: as `read`, with a shared slice.
            let n = unsafe {
                libc::write(self.fd.as_raw_fd(), bytes.as_ptr().cast::<libc::c_void>(), bytes.len())
            };
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            if n == 0 {
                return Err(io::Error::new(io::ErrorKind::WriteZero, "the link took no bytes"));
            }
            bytes = &bytes[n as usize..];
        }
        Ok(())
    }

    pub fn set_read_timeout(&self, limit: Duration) -> io::Result<()> {
        set_timeout(self.fd.as_raw_fd(), libc::SO_RCVTIMEO, limit)
    }

    pub fn set_write_timeout(&self, limit: Duration) -> io::Result<()> {
        set_timeout(self.fd.as_raw_fd(), libc::SO_SNDTIMEO, limit)
    }

    /// Break the connection, waking anything blocked on it.
    ///
    /// A reader thread parked in `read` cannot be told to stop; it can only be
    /// given something to return from. `shutdown` is that, and it works from
    /// another thread, which `close` does not: closing a descriptor another
    /// thread is reading from is a use-after-free waiting for the number to be
    /// reused.
    pub fn shutdown(&self) {
        // SAFETY: the descriptor is owned by `self`; shutdown on an already
        // shut-down or unconnected socket is an error code, not a hazard.
        unsafe {
            libc::shutdown(self.fd.as_raw_fd(), libc::SHUT_RDWR);
        }
    }
}

fn set_timeout(fd: RawFd, which: libc::c_int, limit: Duration) -> io::Result<()> {
    let tv = libc::timeval {
        tv_sec: limit.as_secs() as libc::time_t,
        tv_usec: libc::suseconds_t::from(limit.subsec_micros()),
    };
    // SAFETY: `tv` is a live timeval and the length matches its type.
    let rc = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            which,
            (&raw const tv).cast::<libc::c_void>(),
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Two connected channels, for tests and for nothing else.
///
/// A `socketpair` is a stream socket like an RFCOMM one, so everything above
/// behaves identically across it -- including the shutdown, which is the part
/// that is easy to get wrong and impossible to check by reading.
pub fn pair() -> io::Result<(Channel, Channel)> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` is a live array of two ints, which is what socketpair
    // fills in.
    let rc = unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr()) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both descriptors are fresh, open, and owned by nothing else.
    unsafe { Ok((Channel::from_fd(OwnedFd::from_raw_fd(fds[0])), Channel::from_fd(OwnedFd::from_raw_fd(fds[1])))) }
}

/// Connect an RFCOMM socket to `channel` on `mac`.
pub fn connect_rfcomm(mac: Mac, channel: u8, limit: Duration) -> io::Result<Channel> {
    let addr = RawAddr(rfcomm_sockaddr(mac, channel));
    let fd = socket(BTPROTO_RFCOMM)?;
    connect_within(fd.as_raw_fd(), addr.as_sockaddr(), addr.0.len() as libc::socklen_t, limit)?;
    Ok(Channel { fd })
}

/// Connect an L2CAP socket to the SDP multiplexer on `mac`.
fn connect_sdp(mac: Mac, limit: Duration) -> io::Result<Channel> {
    let addr = RawAddr(l2cap_sockaddr(mac, PSM_SDP));
    let fd = socket(BTPROTO_L2CAP)?;
    connect_within(fd.as_raw_fd(), addr.as_sockaddr(), addr.0.len() as libc::socklen_t, limit)?;
    Ok(Channel { fd })
}

fn socket(protocol: libc::c_int) -> io::Result<OwnedFd> {
    // SOCK_CLOEXEC in the call rather than an fcntl afterwards: the daemon
    // spawns `bluetoothctl`, and a descriptor that leaks into it is a socket
    // the headset sees held open by a process nobody is tracking.
    // SAFETY: socket() has no pointer arguments.
    let fd = unsafe {
        libc::socket(libc::AF_BLUETOOTH, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, protocol)
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a fresh descriptor from socket(), owned by nothing else.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Connect, giving up after `limit`.
///
/// A blocking `connect` to a Bluetooth device that is switched off takes as
/// long as the kernel's page timeout, which is measured in tens of seconds. A
/// daemon that waited would be indistinguishable from a hung one for that
/// whole time, so the socket is put in non-blocking mode for the connect and
/// put back afterwards.
fn connect_within(
    fd: RawFd,
    addr: *const libc::sockaddr,
    len: libc::socklen_t,
    limit: Duration,
) -> io::Result<()> {
    // SAFETY: fcntl with F_GETFL/F_SETFL takes no pointers.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: as above.
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: `addr` points at a live socket address of `len` bytes, built by
    // the caller and outliving this call.
    let rc = unsafe { libc::connect(fd, addr, len) };
    let pending = if rc == 0 {
        false
    } else {
        let e = io::Error::last_os_error();
        match e.raw_os_error() {
            Some(libc::EINPROGRESS) | Some(libc::EALREADY) => true,
            _ => return Err(e),
        }
    };

    if pending {
        let mut poller =
            libc::pollfd { fd, events: libc::POLLOUT, revents: 0 };
        // Milliseconds, and saturating: a limit longer than i32 milliseconds
        // is a caller mistake, and rounding it to "forever" would be the one
        // outcome this function exists to prevent.
        let ms = i32::try_from(limit.as_millis()).unwrap_or(i32::MAX);
        // SAFETY: one live pollfd, and a count of one.
        let ready = unsafe { libc::poll(&raw mut poller, 1, ms) };
        if ready < 0 {
            return Err(io::Error::last_os_error());
        }
        if ready == 0 {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "the headset did not answer"));
        }
        // POLLOUT alone does not mean success: a refused connection is also
        // writable, and the reason is in SO_ERROR.
        let mut err: libc::c_int = 0;
        let mut size = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        // SAFETY: `err` and `size` are live and correctly sized.
        let rc = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                (&raw mut err).cast::<libc::c_void>(),
                &raw mut size,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        if err != 0 {
            return Err(io::Error::from_raw_os_error(err));
        }
    }

    // SAFETY: as the earlier fcntl calls.
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Why a headset could not be reached.
/// `EBUSY`. Named rather than written as 16 at the match site, because a bare
/// number there reads as a magic constant and this one carries a meaning worth
/// stating: the headset's single control session is already taken.
const EBUSY: i32 = 16;

#[derive(Debug)]
pub enum LinkError {
    /// No connected device advertises either MDR service.
    NoDevice,
    /// The device advertised an MDR service and its SDP record has no RFCOMM
    /// channel under it.
    NotSupported(Mac),
    Sdp(SdpError),
    Io(io::Error),
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkError::NoDevice => write!(f, "no connected headset speaks this protocol"),
            LinkError::NotSupported(mac) => {
                write!(f, "{mac} offers no RFCOMM channel for its Sony MDR service")
            }
            LinkError::Sdp(e) => write!(f, "{e}"),

            // EBUSY AND A CONNECT TIMEOUT USUALLY MEAN YOUR PHONE.
            //
            // "Device or resource busy" is what the kernel says and it sends a
            // person looking at their computer, which is the wrong end. The
            // headset grants exactly ONE MDR control session and shares it with
            // everything it is paired to, so with multipoint on and a phone
            // attached the connect fails while A2DP audio keeps playing
            // perfectly -- which makes it look like our bug rather than a
            // session already taken. Sony's app does not need to be open for
            // the phone to be holding it.
            //
            // Observed on real hardware by delarosa1312/omarchy-sony-xm5 (MIT,
            // Copyright (c) 2026 Delarosa1312), docs/protocol.md: "the connect
            // can fail with either EBUSY or 'Timed out Connecting to remote
            // device' while A2DP audio keeps working perfectly". The errno is
            // kept in the text, because the guess is a guess and somebody
            // debugging needs the real one.
            LinkError::Io(e)
                if e.raw_os_error() == Some(EBUSY) || e.kind() == io::ErrorKind::TimedOut =>
            {
                write!(
                    f,
                    "{e} -- the headset allows one control session across every \
                     device it is paired with, so another one (usually a phone) \
                     may be holding it"
                )
            }

            LinkError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<io::Error> for LinkError {
    fn from(e: io::Error) -> LinkError {
        LinkError::Io(e)
    }
}

impl From<SdpError> for LinkError {
    fn from(e: SdpError) -> LinkError {
        LinkError::Sdp(e)
    }
}

/// Ask `mac` which RFCOMM channel `service` is on.
///
/// One SDP conversation: connect, ask, follow continuations, disconnect. The
/// continuation loop is bounded because the device chooses when it ends, and
/// "until the device says stop" is not a loop condition to accept from
/// hardware across a radio link.
///
/// One service per call. The caller holds the list [`mdr_services`] built and
/// moves on to the next entry on [`LinkError::NotSupported`], which is how the
/// first service the device actually answers for is the one dialled.
pub fn mdr_channel(mac: Mac, service: Uuid, limit: Duration) -> Result<u8, LinkError> {
    let link = connect_sdp(mac, limit)?;
    link.set_read_timeout(limit)?;
    link.set_write_timeout(limit)?;
    ask_for_channel(&link, service)?.ok_or(LinkError::NotSupported(mac))
}

/// The SDP conversation, over a channel that is already connected.
///
/// SPLIT FROM THE CONNECT so it can be driven over a `socketpair` with a test
/// answering as the device. Everything interesting about this function is in
/// what it does with the answers -- the continuation loop, the two bounds on
/// it, assembling a record across responses -- and none of that is reachable
/// through a function that opens its own Bluetooth socket first.
///
/// `Ok(None)` means the device answered and its record has no RFCOMM channel,
/// which is a different thing from the query failing and is reported
/// differently.
pub fn ask_for_channel(link: &Channel, service: Uuid) -> Result<Option<u8>, SdpError> {
    let transaction = 0x4D44u16; // "MD", so a stray reply is obvious in a log.
    let mut continuation: Vec<u8> = Vec::new();
    let mut attributes: Vec<u8> = Vec::new();

    for _ in 0..MAX_CONTINUATIONS {
        let request =
            sdp::search_request(transaction, service, sdp::ATTR_PROTOCOL_DESCRIPTOR_LIST, &continuation);
        link.write_all(&request).map_err(|_| SdpError::Unexpected("a link that stopped taking bytes"))?;

        // One read per response. SDP is a request/response protocol over a
        // sequential-packet-ish L2CAP channel, and a response is one PDU.
        let mut buf = vec![0u8; SDP_MTU];
        let n = link
            .read(&mut buf)
            .map_err(|_| SdpError::Unexpected("a link that stopped answering"))?;
        if n == 0 {
            return Err(SdpError::Truncated);
        }
        let response = sdp::read_response(&buf[..n], transaction)?;
        attributes.extend_from_slice(&response.attributes);
        if response.continuation.is_empty() {
            let (parsed, _) = sdp::element(&attributes)?;
            return Ok(sdp::rfcomm_channel(&parsed));
        }
        continuation = response.continuation;
    }
    Err(SdpError::Unexpected("a device that never finished answering"))
}

/// One SDP read. The specification's required MTU (Vol 3, Part B, 7.1).
const SDP_MTU: usize = 672;

/// How many times a device may say "there is more".
///
/// A real ProtocolDescriptorList fits in one response. This is generous enough
/// that a chatty device is not cut off and small enough that one which never
/// finishes is.
///
/// IT IS ALSO THE MEMORY BOUND, and that is why there is not a second one. A
/// response carries at most `SDP_MTU` bytes because that is all one read
/// takes, so the record being assembled cannot exceed `MAX_CONTINUATIONS *
/// SDP_MTU` -- about ten kilobytes. A separate cap on the assembled length
/// looks like defence in depth and is not: it can only ever fire in cases this
/// one has already caught, which makes it a line no test can show is doing
/// anything.
const MAX_CONTINUATIONS: usize = 16;

// ── finding the headset ──────────────────────────────────────────────────────

/// A connected Bluetooth device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub mac: Mac,
    pub name: String,
}

/// The devices in `bluetoothctl devices Connected` output.
///
/// The format is one device per line:
///
/// ```text
/// Device AA:BB:CC:DD:EE:FF WH-1000XM4
/// ```
///
/// Parsed rather than pattern-matched loosely, because a name is arbitrary
/// text the DEVICE chose: it may contain spaces, the word `Device`, or
/// something that looks like an address. Only the second field is an address
/// and everything after it is the name, so neither can be confused for the
/// other however the name is written.
///
/// A line without a valid address in that position is skipped rather than
/// guessed at -- `bluetoothctl` also prints warnings and prompts on stdout.
pub fn connected_devices(text: &str) -> Vec<Device> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut fields = line.trim().splitn(3, ' ');
        if fields.next() != Some("Device") {
            continue;
        }
        let Some(mac) = fields.next().and_then(Mac::parse) else { continue };
        let name = fields.next().unwrap_or("").trim().to_string();
        out.push(Device { mac, name });
    }
    out
}

/// Whether `bluetoothctl info <mac>` output lists `service`.
///
/// The line is `\tUUID: Vendor specific  (96cc203e-...)`, and only the
/// parenthesised value is the UUID -- the description before it is a lookup
/// table's opinion and says "Vendor specific" for everything interesting.
///
/// Compared at 128 bits, because `bluetoothctl` prints the long form for every
/// UUID including the ones with short aliases.
pub fn advertises(info: &str, service: Uuid) -> bool {
    let wanted = service.wide();
    info.lines().filter_map(uuid_on_line).any(|found| found.wide() == wanted)
}

fn uuid_on_line(line: &str) -> Option<Uuid> {
    let line = line.trim();
    if !line.starts_with("UUID:") {
        return None;
    }
    let open = line.rfind('(')?;
    let close = line.rfind(')')?;
    parse_uuid128(line.get(open + 1..close)?)
}

/// `8-4-4-4-12` hex, as every Bluetooth tool prints it.
fn parse_uuid128(text: &str) -> Option<Uuid> {
    let groups: Vec<&str> = text.trim().split('-').collect();
    if groups.len() != 5 || [8, 4, 4, 4, 12] != groups.iter().map(|g| g.len()).collect::<Vec<_>>()[..] {
        return None;
    }
    let flat: String = groups.concat();
    let mut out = [0u8; 16];
    for (n, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(flat.get(n * 2..n * 2 + 2)?, 16).ok()?;
    }
    Some(Uuid::Long(out))
}
