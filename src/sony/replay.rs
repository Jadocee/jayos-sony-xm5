//! A capture, played back through the daemon's own engine.
//!
//! WHY THIS IS IN THE LIBRARY
//!
//! The state document is a contract with a program in another repository:
//! jayos's Headphones page reads it, field by field, and that page is not here
//! to be tested against. What can be handed across is a real document -- not
//! one written by hand to look like one, which would only ever contain the
//! fields its author remembered -- and the only way to get a real document
//! without a headset is to feed captured frames to the same `Session` the
//! daemon runs. `qs-sony replay` does that for a person, and the contract test
//! does it to produce `contract/state.example.json`.
//!
//! DETERMINISTIC, BECAUSE IT IS COMPARED BYTE FOR BYTE. The frames are taken
//! in the order of the counter in their names, which is capture order
//! (`capture::capture_order`), and the clock is a counter rather than the wall: one
//! millisecond per frame. Nothing in a replayed document may depend on when
//! it was replayed, or the contract check fails for reasons that are not a
//! change.
//!
//! RECEIVED FRAMES ONLY. A capture directory may also hold the frames the host
//! sent (`-tx.` in the name, as `qs-sonyd --record` writes them). Feeding those
//! to a session would be feeding it its own questions as if the headset had
//! asked them.

use super::session::{Input, Limits, Peer, Session};
use std::io;
use std::path::Path;

/// Every received frame in a capture directory, as raw wire bytes, in capture
/// order.
///
/// A file counts when its name ends `.bin` and contains `-rx.`. An unreadable
/// file is an error rather than a gap, because a replay that silently skipped
/// a frame produces a document that is wrong in a way nothing reports.
pub fn received_frames(dir: &Path) -> io::Result<Vec<Vec<u8>>> {
    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".bin") && n.contains("-rx."))
        .collect();
    // By counter, not by name: the name leads with a wall-clock time, which
    // a recording can see step backwards. `capture::fixture_name` says more.
    crate::capture::capture_order(&mut names);
    names.iter().map(|n| std::fs::read(dir.join(n))).collect()
}

/// The document a daemon would publish after receiving `frames` from a headset
/// called `peer` over a link that came up at time zero.
///
/// `version` is what the document names as its daemon. The contract passes
/// [`super::DAEMON_VERSION`]; a test that wants a document independent of the
/// package version passes something fixed.
pub fn document(frames: &[Vec<u8>], peer: &str, version: &str) -> String {
    let mut session = Session::new(version.to_string(), Limits::default());
    session.step(Input::LinkUp { now: 0, peer: Peer { name: peer.to_string(), protocol: 2 } });
    for (now, wire) in (1u64..).zip(frames) {
        session.step(Input::Bytes { now, bytes: wire });
    }
    session.document()
}
