//! Sony WH-1000XM5 headphones over Bluetooth: the MDR protocol, and the daemon
//! and client built on it.

/// Sony's MDR protocol. See the module docs for what the captured corpus in
/// `tests/fixtures/mdr/` does and does not prove.
pub mod mdr;

/// The Sony headphone daemon's contract: the state document it publishes and
/// the command language it accepts.
pub mod sony;

/// What the two binaries share that is not about headphones: argument
/// parsing, JSON on stdout, and running a subprocess with a deadline. The
/// module docs say why it is a copy.
pub mod support;

/// Captures: imported from a Bluetooth trace, recorded by the daemon, and
/// measured against what is still outstanding.
pub mod capture;
