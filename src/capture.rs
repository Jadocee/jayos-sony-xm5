//! Captures: reading them out of a Bluetooth trace, recording them from the
//! daemon, and saying what one settles. Plan 23, part C1.
//!
//! WHY A CAPTURE PATH AT ALL. Every claim about a WH-1000XM5 in this package
//! rests on a capture of a different headset (a WF-1000XM5, the corpus in
//! `tests/fixtures/mdr/`), on the reference's header, and on other projects'
//! reports. The outstanding items in `docs/OUTSTANDING.md` are each settled by
//! one capture of the right headset, and this module is what turns that
//! capture into fixtures named exactly as the corpus is, so the decoders and
//! their tests take it as they took the first one.
//!
//! THREE WAYS IN:
//!
//! - [`import`]: a btsnoop file -- `btmon -w` (datalink 2001), an Android HCI
//!   snoop log (1002), or an unencapsulated one (1001) -- walked down through
//!   ACL reassembly, L2CAP to PSM 3, and RFCOMM UIH frames to the MDR byte
//!   stream on one DLCI.
//! - [`Recorder`]: `qs-sonyd --record=DIR`, the daemon writing both directions
//!   of its own link as it runs.
//! - A directory either of those wrote, read back by [`load`] for [`settle`].
//!
//! NOTHING IS REDACTED. A frame is written whole or not at all: `--exclude`
//! drops every frame of a named command, and the counter still counts it, so
//! the gap shows. A capture with a byte changed is not a capture.
//!
//! WHAT IS TESTED AND WHAT IS NOT. `tests/capture.rs` builds btsnoop files from
//! the corpus's own frames and requires every frame back, byte for byte and
//! named as the corpus names it. That round trip is CIRCULAR -- the input was
//! built by code that shares this module's reading of the formats -- and is
//! evidence that the walk is consistent, not that it matches a real trace.
//! The layouts are cited where they are read; none of this has read a trace a
//! real adapter wrote.

use crate::mdr::{frame, payload, DataType};
use std::io;
use std::path::{Path, PathBuf};

/// Which way a frame went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// From the headset: `-rx.` in a fixture name.
    Rx,
    /// To the headset: `-tx.`.
    Tx,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Rx => "rx",
            Direction::Tx => "tx",
        }
    }
}

// ── naming ──────────────────────────────────────────────────────────────────

/// Table 1's `Command` enum (T1:375), transcribed whole, OUT_OF_RANGE aside.
const T1_COMMANDS: &[(u8, &str)] = &[
    (0x00, "CONNECT_GET_PROTOCOL_INFO"),
    (0x01, "CONNECT_RET_PROTOCOL_INFO"),
    (0x02, "CONNECT_GET_CAPABILITY_INFO"),
    (0x03, "CONNECT_RET_CAPABILITY_INFO"),
    (0x04, "CONNECT_GET_DEVICE_INFO"),
    (0x05, "CONNECT_RET_DEVICE_INFO"),
    (0x06, "CONNECT_GET_SUPPORT_FUNCTION"),
    (0x07, "CONNECT_RET_SUPPORT_FUNCTION"),
    (0x0F, "GET_TEST"),
    (0x10, "COMMON_GET_CAPABILITY"),
    (0x11, "COMMON_RET_CAPABILITY"),
    (0x12, "COMMON_GET_STATUS"),
    (0x13, "COMMON_RET_STATUS"),
    (0x15, "COMMON_NTFY_STATUS"),
    (0x18, "COMMON_SET_PARAM"),
    (0x19, "COMMON_NTFY_PARAM"),
    (0x20, "POWER_GET_CAPABILITY"),
    (0x21, "POWER_RET_CAPABILITY"),
    (0x22, "POWER_GET_STATUS"),
    (0x23, "POWER_RET_STATUS"),
    (0x24, "POWER_SET_STATUS"),
    (0x25, "POWER_NTFY_STATUS"),
    (0x26, "POWER_GET_PARAM"),
    (0x27, "POWER_RET_PARAM"),
    (0x28, "POWER_SET_PARAM"),
    (0x29, "POWER_NTFY_PARAM"),
    (0x30, "UPDT_GET_CAPABILITY"),
    (0x31, "UPDT_RET_CAPABILITY"),
    (0x32, "UPDT_GET_STATUS"),
    (0x33, "UPDT_RET_STATUS"),
    (0x34, "UPDT_SET_STATUS"),
    (0x35, "UPDT_NTFY_STATUS"),
    (0x36, "UPDT_GET_PARAM"),
    (0x37, "UPDT_RET_PARAM"),
    (0x38, "UPDT_SET_PARAM"),
    (0x39, "UPDT_NTFY_PARAM"),
    (0x3E, "UPDT_TRANSFER_DATA"),
    (0x3F, "UPDT_NTFY_MESSAGE"),
    (0x40, "LEA_GET_CAPABILITY"),
    (0x41, "LEA_RET_CAPABILITY"),
    (0x42, "LEA_GET_STATUS"),
    (0x43, "LEA_RET_STATUS"),
    (0x45, "LEA_NTFY_STATUS"),
    (0x46, "LEA_GET_PARAM"),
    (0x47, "LEA_RET_PARAM"),
    (0x48, "LEA_SET_PARAM"),
    (0x49, "LEA_NTFY_PARAM"),
    (0x4A, "LEA_GET_EXTENDED_PARAM"),
    (0x4B, "LEA_RET_EXTENDED_PARAM"),
    (0x4C, "LEA_SET_EXTENDED_PARAM"),
    (0x4D, "LEA_NTFY_EXTENDED_PARAM"),
    (0x50, "EQEBB_GET_CAPABILITY"),
    (0x51, "EQEBB_RET_CAPABILITY"),
    (0x52, "EQEBB_GET_STATUS"),
    (0x53, "EQEBB_RET_STATUS"),
    (0x55, "EQEBB_NTFY_STATUS"),
    (0x56, "EQEBB_GET_PARAM"),
    (0x57, "EQEBB_RET_PARAM"),
    (0x58, "EQEBB_SET_PARAM"),
    (0x59, "EQEBB_NTFY_PARAM"),
    (0x5A, "EQEBB_GET_EXTENDED_INFO"),
    (0x5B, "EQEBB_RET_EXTENDED_INFO"),
    (0x60, "NCASM_GET_CAPABILITY"),
    (0x61, "NCASM_RET_CAPABILITY"),
    (0x62, "NCASM_GET_STATUS"),
    (0x63, "NCASM_RET_STATUS"),
    (0x64, "NCASM_SET_STATUS"),
    (0x65, "NCASM_NTFY_STATUS"),
    (0x66, "NCASM_GET_PARAM"),
    (0x67, "NCASM_RET_PARAM"),
    (0x68, "NCASM_SET_PARAM"),
    (0x69, "NCASM_NTFY_PARAM"),
    (0x70, "SENSE_GET_CAPABILITY"),
    (0x71, "SENSE_RET_CAPABILITY"),
    (0x74, "SENSE_SET_STATUS"),
    (0x75, "SENSE_NTFY_STATUS"),
    (0x78, "SENSE_SET_PARAM"),
    (0x79, "SENSE_NTFY_PARAM"),
    (0x7A, "SENSE_GET_EXT_INFO"),
    (0x7B, "SENSE_RET_EXT_INFO"),
    (0x80, "OPT_GET_CAPABILITY"),
    (0x81, "OPT_RET_CAPABILITY"),
    (0x82, "OPT_GET_STATUS"),
    (0x83, "OPT_RET_STATUS"),
    (0x84, "OPT_SET_STATUS"),
    (0x85, "OPT_NTFY_STATUS"),
    (0x86, "OPT_GET_PARAM"),
    (0x87, "OPT_RET_PARAM"),
    (0x88, "OPT_SET_PARAM"),
    (0x89, "OPT_NTFY_PARAM"),
    (0x90, "ALERT_GET_CAPABILITY"),
    (0x91, "ALERT_RET_CAPABILITY"),
    (0x92, "ALERT_GET_STATUS"),
    (0x93, "ALERT_RET_STATUS"),
    (0x94, "ALERT_SET_STATUS"),
    (0x95, "ALERT_NTFY_STATUS"),
    (0x98, "ALERT_SET_PARAM"),
    (0x99, "ALERT_NTFY_PARAM"),
    (0xA0, "PLAY_GET_CAPABILITY"),
    (0xA1, "PLAY_RET_CAPABILITY"),
    (0xA2, "PLAY_GET_STATUS"),
    (0xA3, "PLAY_RET_STATUS"),
    (0xA4, "PLAY_SET_STATUS"),
    (0xA5, "PLAY_NTFY_STATUS"),
    (0xA6, "PLAY_GET_PARAM"),
    (0xA7, "PLAY_RET_PARAM"),
    (0xA8, "PLAY_SET_PARAM"),
    (0xA9, "PLAY_NTFY_PARAM"),
    (0xB0, "SAR_AUTO_PLAY_GET_CAPABILITY"),
    (0xB1, "SAR_AUTO_PLAY_RET_CAPABILITY"),
    (0xB2, "SAR_AUTO_PLAY_GET_STATUS"),
    (0xB3, "SAR_AUTO_PLAY_RET_STATUS"),
    (0xB5, "SAR_AUTO_PLAY_NTFY_STATUS"),
    (0xB6, "SAR_AUTO_PLAY_GET_PARAM"),
    (0xB7, "SAR_AUTO_PLAY_RET_PARAM"),
    (0xB8, "SAR_AUTO_PLAY_SET_PARAM"),
    (0xB9, "SAR_AUTO_PLAY_NTFY_PARAM"),
    (0xC4, "LOG_SET_STATUS"),
    (0xC9, "LOG_NTFY_PARAM"),
    (0xD0, "GENERAL_SETTING_GET_CAPABILITY"),
    (0xD1, "GENERAL_SETTING_RET_CAPABILITY"),
    (0xD2, "GENERAL_SETTING_GET_STATUS"),
    (0xD3, "GENERAL_SETTING_RET_STATUS"),
    (0xD5, "GENERAL_SETTING_NTFY_STATUS"),
    (0xD6, "GENERAL_SETTING_GET_PARAM"),
    (0xD7, "GENERAL_SETTING_RET_PARAM"),
    (0xD8, "GENERAL_SETTING_SET_PARAM"),
    (0xD9, "GENERAL_SETTING_NTNY_PARAM"),
    (0xE0, "AUDIO_GET_CAPABILITY"),
    (0xE1, "AUDIO_RET_CAPABILITY"),
    (0xE2, "AUDIO_GET_STATUS"),
    (0xE3, "AUDIO_RET_STATUS"),
    (0xE5, "AUDIO_NTFY_STATUS"),
    (0xE6, "AUDIO_GET_PARAM"),
    (0xE7, "AUDIO_RET_PARAM"),
    (0xE8, "AUDIO_SET_PARAM"),
    (0xE9, "AUDIO_NTFY_PARAM"),
    (0xF0, "SYSTEM_GET_CAPABILITY"),
    (0xF1, "SYSTEM_RET_CAPABILITY"),
    (0xF2, "SYSTEM_GET_STATUS"),
    (0xF3, "SYSTEM_RET_STATUS"),
    (0xF4, "SYSTEM_SET_STATUS"),
    (0xF5, "SYSTEM_NTFY_STATUS"),
    (0xF6, "SYSTEM_GET_PARAM"),
    (0xF7, "SYSTEM_RET_PARAM"),
    (0xF8, "SYSTEM_SET_PARAM"),
    (0xF9, "SYSTEM_NTFY_PARAM"),
    (0xFA, "SYSTEM_GET_EXT_PARAM"),
    (0xFB, "SYSTEM_RET_EXT_PARAM"),
    (0xFC, "SYSTEM_SET_EXT_PARAM"),
    (0xFD, "SYSTEM_NTFY_EXT_PARAM"),
    (0xFF, "UNKNOWN"),
];

/// Table 2's `Command` enum (T2:22), transcribed whole, OUT_OF_RANGE aside.
const T2_COMMANDS: &[(u8, &str)] = &[
    (0x06, "CONNECT_GET_SUPPORT_FUNCTION"),
    (0x07, "CONNECT_RET_SUPPORT_FUNCTION"),
    (0x20, "POWER_GET_CAPABILITY"),
    (0x21, "POWER_RET_CAPABILITY"),
    (0x22, "POWER_GET_STATUS"),
    (0x23, "POWER_RET_STATUS"),
    (0x24, "POWER_SET_STATUS"),
    (0x25, "POWER_NTFY_STATUS"),
    (0x26, "POWER_GET_PARAM"),
    (0x27, "POWER_RET_PARAM"),
    (0x28, "POWER_SET_PARAM"),
    (0x29, "POWER_NTFY_PARAM"),
    (0x30, "PERI_GET_CAPABILITY"),
    (0x31, "PERI_RET_CAPABILITY"),
    (0x32, "PERI_GET_STATUS"),
    (0x33, "PERI_RET_STATUS"),
    (0x34, "PERI_SET_STATUS"),
    (0x35, "PERI_NTFY_STATUS"),
    (0x36, "PERI_GET_PARAM"),
    (0x37, "PERI_RET_PARAM"),
    (0x38, "PERI_SET_PARAM"),
    (0x39, "PERI_NTFY_PARAM"),
    (0x3C, "PERI_SET_EXTENDED_PARAM"),
    (0x3D, "PERI_NTFY_EXTENDED_PARAM"),
    (0x40, "VOICE_GUIDANCE_GET_CAPABILITY"),
    (0x41, "VOICE_GUIDANCE_RET_CAPABILITY"),
    (0x42, "VOICE_GUIDANCE_GET_STATUS"),
    (0x43, "VOICE_GUIDANCE_RET_STATUS"),
    (0x44, "VOICE_GUIDANCE_SET_STATUS"),
    (0x45, "VOICE_GUIDANCE_NTFY_STATUS"),
    (0x46, "VOICE_GUIDANCE_GET_PARAM"),
    (0x47, "VOICE_GUIDANCE_RET_PARAM"),
    (0x48, "VOICE_GUIDANCE_SET_PARAM"),
    (0x53, "SAFE_LISTENING_RET_STATUS"),
    (0x54, "SAFE_LISTENING_SET_STATUS"),
    (0x55, "SAFE_LISTENING_NTFY_STATUS"),
    (0x56, "SAFE_LISTENING_GET_PARAM"),
    (0x57, "SAFE_LISTENING_RET_PARAM"),
    (0x58, "SAFE_LISTENING_SET_PARAM"),
    (0x59, "SAFE_LISTENING_NTFY_PARAM"),
    (0x5A, "SAFE_LISTENING_GET_EXTENDED_PARAM"),
    (0x5B, "SAFE_LISTENING_RET_EXTENDED_PARAM"),
    (0x60, "LEA_GET_CAPABILITY"),
    (0x61, "LEA_RET_CAPABILITY"),
    (0x62, "LEA_GET_STATUS"),
    (0x63, "LEA_RET_STATUS"),
    (0x65, "LEA_NTFY_STATUS"),
    (0x66, "LEA_GET_PARAM"),
    (0x67, "LEA_RET_PARAM"),
    (0x68, "LEA_SET_PARAM"),
    (0x69, "LEA_NTFY_PARAM"),
    (0x70, "PARTY_GET_CAPABILITY"),
    (0x71, "PARTY_RET_CAPABILITY"),
    (0x72, "PARTY_GET_STATUS"),
    (0x73, "PARTY_RET_STATUS"),
    (0x74, "PARTY_SET_STATUS"),
    (0x75, "PARTY_NTFY_STATUS"),
    (0x76, "PARTY_GET_PARAM"),
    (0x77, "PARTY_RET_PARAM"),
    (0x78, "PARTY_SET_PARAM"),
    (0x79, "PARTY_NTFY_PARAM"),
    (0x7C, "PARTY_SET_EXTENDED_PARAM"),
    (0xF0, "SYSTEM_GET_CAPABILITY"),
    (0xF1, "SYSTEM_RET_CAPABILITY"),
    (0xF2, "SYSTEM_GET_STATUS"),
    (0xF3, "SYSTEM_RET_STATUS"),
    (0xF4, "SYSTEM_SET_STATUS"),
    (0xF5, "SYSTEM_NTFY_STATUS"),
    (0xF6, "SYSTEM_GET_PARAM"),
    (0xF7, "SYSTEM_RET_PARAM"),
    (0xF8, "SYSTEM_SET_PARAM"),
    (0xF9, "SYSTEM_NTFY_PARAM"),
    (0xFA, "SYSTEM_GET_EXTENDED_PARAM"),
    (0xFB, "SYSTEM_RET_EXTENDED_PARAM"),
    (0xFC, "SYSTEM_SET_EXTENDED_PARAM"),
    (0xFD, "SYSTEM_NTFY_EXTENDED_PARAM"),
    (0xFF, "UNKNOWN"),
];

/// `MDRDataType` (`Protocol.hpp:38`) by name.
pub fn data_type_name(data_type: DataType) -> &'static str {
    match data_type {
        DataType::Data => "DATA",
        DataType::Ack => "ACK",
        DataType::DataMcNo1 => "DATA_MC_NO1",
        DataType::DataIcd => "DATA_ICD",
        DataType::DataEv => "DATA_EV",
        DataType::DataMdr => "DATA_MDR",
        DataType::DataCommon => "DATA_COMMON",
        DataType::DataMdrNo2 => "DATA_MDR_NO2",
        DataType::Shot => "SHOT",
        DataType::ShotMcNo1 => "SHOT_MC_NO1",
        DataType::ShotIcd => "SHOT_ICD",
        DataType::ShotEv => "SHOT_EV",
        DataType::ShotMdr => "SHOT_MDR",
        DataType::ShotCommon => "SHOT_COMMON",
        DataType::ShotMdrNo2 => "SHOT_MDR_NO2",
        DataType::Unknown(_) => "UNKNOWN",
    }
}

/// The command's name in the table its data type selects.
///
/// `NO_COMMAND` for an empty payload -- an ACK -- which is what the corpus's
/// recorder wrote and is in neither enum. `UNKNOWN` for a byte the table does
/// not declare, and for a data type that is neither table: the byte is in the
/// name beside it either way.
pub fn command_name(data_type: DataType, payload: &[u8]) -> &'static str {
    let Some(&byte) = payload.first() else { return "NO_COMMAND" };
    let table = match data_type {
        DataType::DataMdr => T1_COMMANDS,
        DataType::DataMdrNo2 => T2_COMMANDS,
        _ => return "UNKNOWN",
    };
    table.iter().find(|(b, _)| *b == byte).map_or("UNKNOWN", |(_, n)| n)
}

/// The corpus's file name for one wire frame:
/// `mdr-packet-<ms>-<counter>-<rx|tx>.type-<NAME>-<hex>.seq-<hex>.cmd-<NAME>-<hex>.bin`.
///
/// `None` for bytes that are not a frame. The counter is six digits, so the
/// names sort in capture order, which is what `sony::replay` relies on.
pub fn fixture_name(ms: u64, counter: u32, direction: Direction, wire: &[u8]) -> Option<String> {
    let f = frame::unpack(wire).ok()?;
    let command = f.payload.first().copied().unwrap_or(0);
    Some(format!(
        "mdr-packet-{ms}-{counter:06}-{}.type-{}-{:02x}.seq-{:02x}.cmd-{}-{command:02x}.bin",
        direction.as_str(),
        data_type_name(f.data_type),
        f.data_type.to_wire(),
        f.seq,
        command_name(f.data_type, &f.payload),
    ))
}

// ── btsnoop ─────────────────────────────────────────────────────────────────

/// Why a file could not be imported. Every arm names what was wrong with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    /// It does not begin `btsnoop\0`, version 1.
    NotBtsnoop,
    /// A datalink this does not read.
    Datalink(u32),
    /// A record runs past the end of the file, at this byte offset.
    Truncated(usize),
    /// No RFCOMM stream on the channel asked for, or none that carries MDR.
    NoStream,
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::NotBtsnoop => write!(f, "not a btsnoop file (version 1)"),
            ImportError::Datalink(d) => {
                write!(f, "btsnoop datalink {d} is not one this reads (1001, 1002, 2001)")
            }
            ImportError::Truncated(at) => write!(f, "a record at byte {at} runs past the end of the file"),
            ImportError::NoStream => write!(f, "no RFCOMM stream in it carries MDR frames"),
        }
    }
}

/// Microseconds from 0 AD to the Unix epoch: btsnoop's timestamp origin (the
/// btsnoop format note, "Timestamp"; BlueZ `src/shared/btsnoop.c` uses the
/// same constant).
pub const BTSNOOP_EPOCH_DELTA_US: u64 = 0x00dc_ddb3_0f2f_8000;

/// One ACL packet, which way it went, and when, in Unix milliseconds.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Acl {
    ms: u64,
    direction: Direction,
    bytes: Vec<u8>,
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

/// The ACL packets in a btsnoop file, in file order.
///
/// Per datalink: 1001 is unencapsulated HCI, with bit 0 of the flags the
/// direction and bit 1 set for commands and events (skipped); 1002 is H4, the
/// first byte the packet type (0x02 ACL) and the flags as for 1001; 2001 is
/// the Linux monitor, whose flags are `index << 16 | opcode` with opcode 4 an
/// ACL packet sent and 5 one received (BlueZ `src/shared/btsnoop.h`,
/// `BTSNOOP_OPCODE_ACL_TX_PKT` / `_RX_PKT`).
fn acl_packets(file: &[u8]) -> Result<Vec<Acl>, ImportError> {
    if file.get(..8) != Some(b"btsnoop\0") || be32(file, 8) != Some(1) {
        return Err(ImportError::NotBtsnoop);
    }
    let datalink = be32(file, 12).ok_or(ImportError::NotBtsnoop)?;
    if !matches!(datalink, 1001 | 1002 | 2001) {
        return Err(ImportError::Datalink(datalink));
    }
    let mut out = Vec::new();
    let mut at = 16usize;
    while at < file.len() {
        let header = file.get(at..at + 24).ok_or(ImportError::Truncated(at))?;
        let included = be32(header, 4).ok_or(ImportError::Truncated(at))? as usize;
        let flags = be32(header, 8).ok_or(ImportError::Truncated(at))?;
        let stamp = u64::from_be_bytes(header[16..24].try_into().map_err(|_| ImportError::Truncated(at))?);
        let packet = file.get(at + 24..at + 24 + included).ok_or(ImportError::Truncated(at))?;
        at += 24 + included;
        let ms = stamp.saturating_sub(BTSNOOP_EPOCH_DELTA_US) / 1000;
        let (direction, bytes) = match datalink {
            2001 => match flags & 0xFFFF {
                4 => (Direction::Tx, packet),
                5 => (Direction::Rx, packet),
                _ => continue,
            },
            _ => {
                if flags & 0b10 != 0 {
                    continue;
                }
                let direction = if flags & 1 == 0 { Direction::Tx } else { Direction::Rx };
                if datalink == 1002 {
                    match packet.split_first() {
                        Some((0x02, rest)) => (direction, rest),
                        _ => continue,
                    }
                } else {
                    (direction, packet)
                }
            }
        };
        out.push(Acl { ms, direction, bytes: bytes.to_vec() });
    }
    Ok(out)
}

/// One whole L2CAP PDU, reassembled.
#[derive(Debug, Clone)]
struct Pdu {
    ms: u64,
    direction: Direction,
    handle: u16,
    cid: u16,
    payload: Vec<u8>,
}

/// ACL fragments back into L2CAP PDUs, per (handle, direction).
///
/// The packet boundary flag (bits 12-13 of the handle field) says which a
/// fragment is: 0b01 continues the PDU in progress, anything else starts one.
/// The L2CAP basic header's length says when it is whole. A continuation with
/// nothing in progress, or a start while one is unfinished, is a lost
/// fragment, and the partial PDU is dropped rather than spliced to the next.
fn reassemble(packets: &[Acl]) -> Vec<Pdu> {
    use std::collections::HashMap;
    let mut open: HashMap<(u16, bool), (u64, Vec<u8>)> = HashMap::new();
    let mut out = Vec::new();
    for p in packets {
        let (Some(field), Some(len)) = (le16(&p.bytes, 0), le16(&p.bytes, 2)) else { continue };
        let Some(data) = p.bytes.get(4..4 + usize::from(len)) else { continue };
        let handle = field & 0x0FFF;
        let key = (handle, p.direction == Direction::Rx);
        let buf = if (field >> 12) & 0b11 == 0b01 {
            let Some((_, buf)) = open.get_mut(&key) else { continue };
            buf.extend_from_slice(data);
            open.remove(&key).map(|(_, b)| b).unwrap_or_default()
        } else {
            data.to_vec()
        };
        let Some(want) = le16(&buf, 0) else {
            open.insert(key, (p.ms, buf));
            continue;
        };
        let whole = 4 + usize::from(want);
        if buf.len() < whole {
            open.insert(key, (p.ms, buf));
            continue;
        }
        let cid = le16(&buf, 2).unwrap_or(0);
        out.push(Pdu { ms: p.ms, direction: p.direction, handle, cid, payload: buf[4..whole].to_vec() });
    }
    out
}

/// The (handle, direction, CID) triples that carry PSM 3, from the L2CAP
/// signalling channel (CID 0x0001).
///
/// A Connection Request (0x02) for PSM 3 names its sender's channel as its
/// source CID, and data TO that sender carries it -- so it marks the opposite
/// direction. A successful Connection Response (0x03) names its sender's
/// channel as its destination CID, which marks the opposite direction too.
/// (Bluetooth Core, Vol 3 Part A, 4.2 and 4.3.)
fn rfcomm_channels(pdus: &[Pdu]) -> Vec<(u16, Direction, u16)> {
    let flip = |d: Direction| if d == Direction::Rx { Direction::Tx } else { Direction::Rx };
    let mut asked: Vec<(u16, Direction, u16)> = Vec::new();
    let mut out = Vec::new();
    for p in pdus.iter().filter(|p| p.cid == 0x0001) {
        let mut at = 0;
        while let (Some(&code), Some(len)) = (p.payload.get(at), le16(&p.payload, at + 2)) {
            let body = p.payload.get(at + 4..at + 4 + usize::from(len)).unwrap_or(&[]);
            match code {
                0x02 if le16(body, 0) == Some(0x0003) => {
                    if let Some(source) = le16(body, 2) {
                        out.push((p.handle, flip(p.direction), source));
                        asked.push((p.handle, p.direction, source));
                    }
                }
                0x03 if le16(body, 4) == Some(0x0000) => {
                    let (Some(dest), Some(source)) = (le16(body, 0), le16(body, 2)) else { break };
                    if asked.iter().any(|(h, d, s)| *h == p.handle && *d == flip(p.direction) && *s == source) {
                        out.push((p.handle, flip(p.direction), dest));
                    }
                }
                _ => {}
            }
            at += 4 + usize::from(len);
        }
    }
    out
}

/// One RFCOMM UIH frame's DLCI and information field, or `None` for any other
/// frame or a malformed one.
///
/// TS 07.10 as RFCOMM uses it: an address byte (EA, C/R, then the DLCI in
/// bits 2-7), a control byte (UIH is 0xEF, 0xFF with the P/F bit), a length of
/// one byte or two (EA in bit 0 of the first), then -- in a UIH frame with P/F
/// set under credit-based flow control -- ONE CREDIT BYTE that is not data,
/// then the information field and a one-byte FCS, which is not checked.
fn uih(pdu: &[u8]) -> Option<(u8, &[u8])> {
    let address = *pdu.first()?;
    let control = *pdu.get(1)?;
    if control & !0x10 != 0xEF {
        return None;
    }
    let first = *pdu.get(2)?;
    let (len, mut at) = if first & 1 == 1 {
        (usize::from(first >> 1), 3)
    } else {
        (usize::from(first >> 1) | usize::from(*pdu.get(3)?) << 7, 4)
    };
    if control == 0xFF {
        at += 1;
    }
    let info = pdu.get(at..at + len)?;
    pdu.get(at + len)?;
    Some((address >> 2, info))
}

/// What [`import`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    /// The DLCI the frames were read from. The RFCOMM server channel is this
    /// shifted right by one.
    pub dlci: u8,
    /// Every MDR frame in both directions, in the order they crossed the link,
    /// each with the time of the record that completed it.
    pub frames: Vec<(u64, Direction, Vec<u8>)>,
}

/// One direction's piece of an RFCOMM stream: when, which way, what bytes.
type Piece = (u64, Direction, Vec<u8>);

/// A btsnoop file's MDR frames.
///
/// `channel` picks the RFCOMM server channel; without it, the first DLCI (in
/// order of first appearance) whose stream unpacks to at least one MDR frame
/// is taken. Without L2CAP signalling in the file -- a trace started after the
/// link came up -- every dynamic channel (CID 0x0040 and up) is tried as RFCOMM.
pub fn import(file: &[u8], channel: Option<u8>) -> Result<Imported, ImportError> {
    let pdus = reassemble(&acl_packets(file)?);
    let channels = rfcomm_channels(&pdus);
    let rfcomm = |p: &Pdu| {
        if channels.is_empty() {
            p.cid >= 0x0040
        } else {
            channels.iter().any(|(h, d, c)| *h == p.handle && *d == p.direction && *c == p.cid)
        }
    };
    let mut streams: Vec<(u8, Vec<Piece>)> = Vec::new();
    for p in pdus.iter().filter(|p| rfcomm(p)) {
        let Some((dlci, info)) = uih(&p.payload) else { continue };
        if dlci == 0 || info.is_empty() {
            continue;
        }
        match streams.iter_mut().find(|(d, _)| *d == dlci) {
            Some((_, s)) => s.push((p.ms, p.direction, info.to_vec())),
            None => streams.push((dlci, vec![(p.ms, p.direction, info.to_vec())])),
        }
    }
    for (dlci, pieces) in streams {
        if channel.is_some_and(|c| c != dlci >> 1) {
            continue;
        }
        let frames = split(&pieces);
        if !frames.is_empty() {
            return Ok(Imported { dlci, frames });
        }
    }
    Err(ImportError::NoStream)
}

/// Two byte streams, one per direction, into frames in wire order.
fn split(pieces: &[Piece]) -> Vec<Piece> {
    let mut rx = frame::Scanner::new();
    let mut tx = frame::Scanner::new();
    let mut out = Vec::new();
    for (ms, direction, bytes) in pieces {
        let scanner = if *direction == Direction::Rx { &mut rx } else { &mut tx };
        scanner.push(bytes);
        while let Some(raw) = scanner.next_raw() {
            if frame::unpack(&raw).is_ok() {
                out.push((*ms, *direction, raw));
            }
        }
    }
    out
}

/// Write `imported` as fixtures in `dir`, one file per frame, counting from 1.
///
/// A frame whose command NAME is in `exclude` is not written, and its counter
/// is still spent, so the gap says something was left out. Returns the names
/// written.
pub fn write_fixtures(imported: &Imported, dir: &Path, exclude: &[String]) -> io::Result<Vec<String>> {
    std::fs::create_dir_all(dir)?;
    let mut names = Vec::new();
    for (counter, (ms, direction, wire)) in (1u32..).zip(&imported.frames) {
        let Some(name) = fixture_name(*ms, counter, *direction, wire) else { continue };
        let f = frame::unpack(wire).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{e:?}")))?;
        if exclude.iter().any(|e| e == command_name(f.data_type, &f.payload)) {
            continue;
        }
        crate::sony::atomic_write_bytes(&dir.join(&name), wire)?;
        names.push(name);
    }
    Ok(names)
}

// ── recording ───────────────────────────────────────────────────────────────

/// `qs-sonyd --record=DIR`: both directions of the daemon's link, as fixtures.
///
/// Received bytes arrive in whatever pieces a `read` returned, so they go
/// through a scanner of their own; sent bytes are one whole frame per
/// `Effect::Send`. One counter for both, so the names interleave in the order
/// the frames crossed the link -- the corpus's own scheme, where the counter
/// gaps were the host's half.
pub struct Recorder {
    dir: PathBuf,
    counter: u32,
    rx: frame::Scanner,
}

impl Recorder {
    /// Records into `dir`, which is created private (0700) if it is not
    /// there: a capture holds the names of every device the headset knows.
    pub fn new(dir: &Path) -> io::Result<Recorder> {
        crate::sony::ensure_private_dir(dir)?;
        Ok(Recorder { dir: dir.to_path_buf(), counter: 0, rx: frame::Scanner::new() })
    }

    /// A new link: half a received frame from the last one is not the start
    /// of the next. The counter carries on.
    pub fn link_up(&mut self) {
        self.rx = frame::Scanner::new();
    }

    pub fn received(&mut self, ms: u64, bytes: &[u8]) -> io::Result<()> {
        self.rx.push(bytes);
        while let Some(raw) = self.rx.next_raw() {
            self.write(ms, Direction::Rx, &raw)?;
        }
        Ok(())
    }

    pub fn sent(&mut self, ms: u64, wire: &[u8]) -> io::Result<()> {
        self.write(ms, Direction::Tx, wire)
    }

    fn write(&mut self, ms: u64, direction: Direction, wire: &[u8]) -> io::Result<()> {
        self.counter += 1;
        // A span that is not a frame has no name in the scheme; it is counted,
        // so the gap is visible, and not written.
        let Some(name) = fixture_name(ms, self.counter, direction, wire) else { return Ok(()) };
        crate::sony::atomic_write_bytes(&self.dir.join(name), wire)
    }
}

// ── what a capture settles ──────────────────────────────────────────────────

/// A capture directory, read back: every frame in both directions, in name
/// order, and the model the headset said it was.
#[derive(Debug, Clone, Default)]
pub struct Capture {
    pub model: Option<String>,
    pub frames: Vec<(Direction, DataType, Vec<u8>)>,
}

/// Every `.bin` in `dir` whose name says `-rx.` or `-tx.`, in name order.
pub fn load(dir: &Path) -> io::Result<Capture> {
    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".bin") && (n.contains("-rx.") || n.contains("-tx.")))
        .collect();
    names.sort();
    let mut capture = Capture::default();
    for name in names {
        let Ok(f) = frame::unpack(&std::fs::read(dir.join(&name))?) else { continue };
        let direction = if name.contains("-rx.") { Direction::Rx } else { Direction::Tx };
        if direction == Direction::Rx {
            if let payload::Reading::Device(info) = payload::decode(f.data_type, &f.payload) {
                if !info.model.is_empty() {
                    capture.model = Some(info.model);
                }
            }
        }
        capture.frames.push((direction, f.data_type, f.payload));
    }
    Ok(capture)
}

/// What a capture says about one outstanding item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The capture holds what settles it; the string says what that was.
    Settled(String),
    /// It does not, yet.
    Open,
    /// Frames cannot settle it; the string says what does.
    NotFromFrames(&'static str),
}

/// One item of `docs/OUTSTANDING.md` and the frames that settle it.
pub struct Item {
    pub number: u8,
    pub title: &'static str,
    check: fn(&Capture) -> Verdict,
}

/// The model every frame-settled item needs the capture to come from. The
/// corpus is a WF-1000XM5, and a second capture of that would settle nothing
/// this package does not already stand on.
pub const TARGET_MODEL: &str = "WH-1000XM5";

fn rx(c: &Capture) -> impl Iterator<Item = (DataType, &[u8])> {
    c.frames.iter().filter(|(d, _, _)| *d == Direction::Rx).map(|(_, t, p)| (*t, p.as_slice()))
}

/// The index of the first tx frame matching `is`, and every rx frame after it.
fn after_tx(c: &Capture, is: impl Fn(&[u8]) -> bool) -> Option<(&[u8], Vec<&[u8]>)> {
    let at = c.frames.iter().position(|(d, t, p)| *d == Direction::Tx && *t == DataType::DataMdr && is(p))?;
    let sent = c.frames[at].2.as_slice();
    let later = c.frames[at + 1..]
        .iter()
        .filter(|(d, t, _)| *d == Direction::Rx && *t == DataType::DataMdr)
        .map(|(_, _, p)| p.as_slice())
        .collect();
    Some((sent, later))
}

fn decodes(c: &Capture, want: impl Fn(&payload::Reading) -> bool) -> Option<Vec<u8>> {
    rx(c).find(|(t, p)| want(&payload::decode(*t, p))).map(|(_, p)| p.to_vec())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

/// The items, in `docs/OUTSTANDING.md`'s order.
pub const ITEMS: &[Item] = &[
    Item { number: 1, title: "the over-ear battery reply", check: |c| {
        decodes(c, |r| matches!(r, payload::Reading::Battery(b) if b.main.is_some()))
            .map_or(Verdict::Open, |p| Verdict::Settled(hex(&p)))
    } },
    Item { number: 2, title: "the ambient level's range", check: |c| {
        decodes(c, |r| matches!(r, payload::Reading::NoiseCapability { .. }))
            .map_or(Verdict::Open, |p| Verdict::Settled(hex(&p)))
    } },
    Item { number: 3, title: "the NC/ASM type a WH-1000XM5 answers at", check: |c| {
        decodes(c, |r| matches!(r, payload::Reading::Noise(_)))
            .map_or(Verdict::Open, |p| Verdict::Settled(format!("NCASM at 0x{:02x}: {}", p[1], hex(&p))))
    } },
    Item { number: 4, title: "a connection-mode write that sticks", check: |c| {
        let Some((sent, later)) = after_tx(c, |p| p.first() == Some(&0xE8)) else { return Verdict::Open };
        match later.iter().find(|p| matches!(p.first(), Some(0xE7 | 0xE9)) && p.get(1) == sent.get(1)) {
            Some(back) if back.get(2) == sent.get(2) => Verdict::Settled(format!("{} then {}", hex(sent), hex(back))),
            _ => Verdict::Open,
        }
    } },
    Item { number: 5, title: "a transport command the headset acts on", check: |c| {
        let Some((sent, later)) = after_tx(c, |p| p.first() == Some(&0xA4)) else { return Verdict::Open };
        later.iter().find(|p| matches!(p.first(), Some(0xA3 | 0xA5)))
            .map_or(Verdict::Open, |back| Verdict::Settled(format!("{} then {}", hex(sent), hex(back))))
    } },
    Item { number: 6, title: "a general-setting write read back in the same polarity", check: |c| {
        let Some((sent, later)) = after_tx(c, |p| p.first() == Some(&0xD8)) else { return Verdict::Open };
        later.iter().find(|p| matches!(p.first(), Some(0xD7 | 0xD9)) && p.get(1) == sent.get(1))
            .filter(|back| back.get(3) == sent.get(3))
            .map_or(Verdict::Open, |back| Verdict::Settled(format!("{} then {}", hex(sent), hex(back))))
    } },
    Item { number: 7, title: "the kernel reads the socket address back", check: |_| {
        Verdict::NotFromFrames("cargo test -- --ignored on the capture machine")
    } },
    Item { number: 10, title: "what an unavailable upscaler says", check: |c| {
        decodes(c, |r| matches!(r, payload::Reading::UpscalingStatus { available: payload::Switch::Off }))
            .map_or(Verdict::Open, |p| Verdict::Settled(hex(&p)))
    } },
    Item { number: 11, title: "table 2's four families on a WH-1000XM5", check: |c| {
        let seen = |want: fn(&payload::Reading) -> bool| decodes(c, want).is_some();
        let all = seen(|r| matches!(r, payload::Reading::VoiceGuidance(v) if v.enabled.is_some()))
            && seen(|r| matches!(r, payload::Reading::VoiceGuidance(v) if v.volume.is_some()))
            && seen(|r| matches!(r, payload::Reading::SafeListening(_)))
            && seen(|r| matches!(r, payload::Reading::PairingMode { .. }));
        if all { Verdict::Settled("all four decode".to_string()) } else { Verdict::Open }
    } },
    Item { number: 13, title: "the page drawn against a live document", check: |_| {
        Verdict::NotFromFrames("a screenshot of the Headphones page, docs/CAPTURE.md")
    } },
    Item { number: 14, title: "an alert asked and answered", check: |c| {
        let Some(asked) = decodes(c, |r| matches!(r, payload::Reading::Alert { .. })) else { return Verdict::Open };
        let answered = c.frames.iter().any(|(d, _, p)| *d == Direction::Tx && p.first() == Some(&0x98));
        if answered { Verdict::Settled(format!("asked {}", hex(&asked))) } else { Verdict::Open }
    } },
];

/// Every item against `capture`.
///
/// A frame-settled item is `Open` unless the capture is of [`TARGET_MODEL`]:
/// the same bytes from another model are evidence about that model.
pub fn settle(capture: &Capture) -> Vec<(&'static Item, Verdict)> {
    let target = capture.model.as_deref().is_some_and(|m| m.contains(TARGET_MODEL));
    ITEMS
        .iter()
        .map(|item| {
            let verdict = (item.check)(capture);
            let verdict = match verdict {
                Verdict::Settled(_) if !target => Verdict::Open,
                other => other,
            };
            (item, verdict)
        })
        .collect()
}
