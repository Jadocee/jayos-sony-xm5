//! Sony's MDR protocol, as spoken by WH-/WF-1000X headphones over Bluetooth.
//!
//! WHAT THIS IS AND WHERE IT CAME FROM
//!
//! The byte-level reference is `mos9527/SonyHeadphonesClient` at commit
//! `db0ae2574d8f8e1407e8ea28a7ee87db9dde1c24`, MIT licensed -- the library the
//! `sony-headphones-linux` daemon calls "libmdr" and links against. This is a
//! port, not a binding: nothing here links C++, and the C++ is a reference to
//! be checked against rather than a thing to transliterate.
//!
//! HOW IT IS KNOWN TO BE RIGHT
//!
//! `tests/fixtures/mdr/` holds 70 packets captured from a real Sony WF-1000XM5
//! on firmware 6.1.0, vendored from that project's own test corpus. They are
//! the ground truth: this environment has no Sony hardware, no Bluetooth
//! adapter and no BlueZ, so a claim about the protocol that the corpus cannot
//! settle is a claim that is not being made.
//!
//! Every one of the 70 must decode, and re-encoding what was decoded must
//! reproduce the file byte for byte. That is a strong statement for this
//! protocol in particular: the checksum covers the length field and the
//! escaping covers the checksum, so an error anywhere reaches the wire.
//!
//! WHAT THE CORPUS CANNOT SETTLE
//!
//! Every fixture is `-rx.` -- captured from the headset. Nothing this code
//! would TRANSMIT was recorded, with one exception that matters: an ACK carries
//! no payload, so the two possible ACK frames are fixed 9-byte strings, and
//! both appear in the corpus. Those are checked. Anything else this port sends
//! is checked against itself and against the reference's source, which proves
//! self-consistency and not that a headset accepts it.
//!
//! The capture is a WF-1000XM5 -- earbuds. The device this tree is written for
//! is a WH-1000XM5 -- over-ear. They speak the same protocol generation, but
//! they do not advertise the same capabilities: the buds report left, right and
//! case batteries and no main battery, while the over-ears report a main
//! battery and none of the others. So the corpus exercises the battery decoding
//! that the target device does NOT use. Capability handling is written from the
//! support tables rather than from the fixtures for exactly that reason.

pub mod frame;
pub mod encode;
pub mod payload;

/// The frame's type byte.
///
/// Total, and deliberately so: a headset that speaks a type this port does not
/// know should decode as `Unknown` and be ignored, not abort the daemon. The
/// values are from the reference's `MDRDataType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    Data,
    Ack,
    DataMcNo1,
    DataIcd,
    DataEv,
    /// Table 1: the ordinary command channel.
    DataMdr,
    DataCommon,
    /// Table 2: the second command channel, used by newer models for the
    /// functions that did not fit table 1.
    DataMdrNo2,
    Shot,
    ShotMcNo1,
    ShotIcd,
    ShotEv,
    ShotMdr,
    ShotCommon,
    ShotMdrNo2,
    /// Anything else. Carries the byte so a report can name it.
    Unknown(u8),
}

impl DataType {
    pub fn from_wire(b: u8) -> DataType {
        match b {
            0x00 => DataType::Data,
            0x01 => DataType::Ack,
            0x02 => DataType::DataMcNo1,
            0x09 => DataType::DataIcd,
            0x0A => DataType::DataEv,
            0x0C => DataType::DataMdr,
            0x0D => DataType::DataCommon,
            0x0E => DataType::DataMdrNo2,
            0x10 => DataType::Shot,
            0x12 => DataType::ShotMcNo1,
            0x19 => DataType::ShotIcd,
            0x1A => DataType::ShotEv,
            0x1C => DataType::ShotMdr,
            0x1D => DataType::ShotCommon,
            0x1E => DataType::ShotMdrNo2,
            other => DataType::Unknown(other),
        }
    }

    pub fn to_wire(self) -> u8 {
        match self {
            DataType::Data => 0x00,
            DataType::Ack => 0x01,
            DataType::DataMcNo1 => 0x02,
            DataType::DataIcd => 0x09,
            DataType::DataEv => 0x0A,
            DataType::DataMdr => 0x0C,
            DataType::DataCommon => 0x0D,
            DataType::DataMdrNo2 => 0x0E,
            DataType::Shot => 0x10,
            DataType::ShotMcNo1 => 0x12,
            DataType::ShotIcd => 0x19,
            DataType::ShotEv => 0x1A,
            DataType::ShotMdr => 0x1C,
            DataType::ShotCommon => 0x1D,
            DataType::ShotMdrNo2 => 0x1E,
            DataType::Unknown(b) => b,
        }
    }

    /// Whether this type carries a command payload that must be acknowledged.
    ///
    /// An ACK is never acknowledged -- doing so is an infinite exchange, and it
    /// is the mistake a port makes when it treats every inbound frame alike.
    pub fn is_data(self) -> bool {
        matches!(self, DataType::DataMdr | DataType::DataMdrNo2 | DataType::DataCommon)
    }
}
