//! The conversation, as a state machine with no I/O in it.
//!
//! Bytes and events go in, bytes and decisions come out. There is no socket
//! here, no clock, no file: time arrives as a `now` on every input, and the
//! engine answers [`Session::deadline`] -- "the earliest moment I want to be
//! stepped again" -- which a transport turns into a `poll` timeout. Wanting is
//! a decision and waiting is a syscall, and that is the whole seam.
//!
//! WHY IT IS SHAPED THIS WAY. This environment has no Bluetooth adapter and 70
//! frames a real WF-1000XM5 sent. A pure engine makes those 70 frames a
//! scripted headset: replay them in capture order and assert the exact bytes
//! the daemon would have transmitted and the exact document it ends holding.
//! Nothing is mocked -- the bytes are the device's. Everything not reachable
//! that way is pushed outward into the transport and named there.
//!
//! THE ONE INVARIANT WORTH READING BEFORE THE CODE
//!
//! [`Facts`] IS CLEARED WHEN THE LINK DROPS. `mdr::encode` is built on the rule
//! that every byte it emits is either named by the verb or echoed from a
//! reading the headset sent, and a SET is a whole-state message with no
//! "unchanged" sentinel. Carrying `Facts` across a reconnect would echo a mode,
//! an ambient level and a focus-on-voice setting from a reading THIS link never
//! produced -- into a message that overwrites all three. The headset may have
//! been used, reset or replaced in between.
//!
//! The corpus contains no disconnect, so no replay test can force that
//! question. It is an invariant rather than a discovery, and
//! `a_reconnect_forgets_what_the_last_link_said` is what holds it -- through
//! [`Session::forget`], which is the single place both ends of a link clear
//! it, so that removing the clearing is a change a test can see.

use super::command::{self, Command as Verb};
use super::state;
use crate::mdr::{encode, frame, payload, DataType};
use std::collections::VecDeque;

/// Milliseconds from an origin the caller chooses.
///
/// The engine never asks what time it is; it is told, so the same sequence of
/// tells gives the same run and a test needs no clock.
pub type Millis = u64;

/// Which client asked, so a reply can be routed back to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClientId(pub u64);

/// What the transport knows and the protocol does not.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Peer {
    /// The BlueZ alias. Never an address: an address is not a name and putting
    /// one on a bar looks like a fault.
    pub name: String,
    /// The MDR generation: 2 for the v2 service UUID, 1 for v1, 0 for neither.
    pub protocol: i32,
}

#[derive(Debug)]
pub enum Input<'a> {
    LinkUp { now: Millis, peer: Peer },
    Bytes { now: Millis, bytes: &'a [u8] },
    LinkDown { now: Millis, why: Down },
    Command { now: Millis, id: ClientId, line: &'a str },
    /// Something outside the engine could not reach a headset, and this is
    /// what to say about it.
    ///
    /// A dial that found nothing is NOT an error state -- no headset is the
    /// ordinary case on most machines, most of the time -- so this records a
    /// reason without moving the phase. What it buys is the difference
    /// between a Headphones page that says "not connected" and one that says
    /// "bluetoothctl is not installed", which is the whole of what a person
    /// needs to fix it.
    Trouble { now: Millis, what: &'a str },
    Tick { now: Millis },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Down {
    /// The headset ended the link in an orderly way -- switched off, put in
    /// its case, walked out of range cleanly. `read` returned 0.
    ///
    /// NOT AN ERROR, and the distinction is the whole point of this enum: it
    /// leaves `last_error` empty and the phase `Idle`, which is what
    /// jayos's `readSony` (`ts/src/settings/Integration.ts`) draws as "no headset connected"
    /// rather than as a fault. The transport used to produce only `Failed`
    /// and this arm was reached by nothing outside the test suite.
    Closed,
    /// The link broke: a reset, a timeout, an adapter going away. The string
    /// is the sentence shown for it.
    Failed(&'static str),
    /// The transport shut the link because this engine asked it to with
    /// [`Effect::Hangup`]. The reason is already in `last_error`, written
    /// when the engine gave up, and this arm must not overwrite it with "the
    /// headset closed the connection" -- it did not.
    HungUp,
}

/// What the transport should do, in the order it should do it.
///
/// The ORDER is asserted rather than incidental: an ACK for an inbound frame is
/// emitted before anything that frame's contents make us send, because a
/// headset waiting on an acknowledgement will not read our next question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Put these bytes on the link, as they are. Already framed.
    Send(Vec<u8>),
    Reply { id: ClientId, line: String },
    /// The document differs from the last one published; [`Session::document`]
    /// is what to write.
    Publish,
    Note(Note),
    /// Drop the link: the headset has stopped acknowledging, or is sending
    /// frames this daemon cannot read. The transport shuts the socket so its
    /// reader returns, reports [`Down::HungUp`], and redials with its backoff
    /// -- what the C++ daemon's `failConnection` does (`closeTransport`, then
    /// `setError(error, wanted_)`, so it redials only while a panel is open;
    /// this daemon redials whenever the setting is on), and what
    /// Gadgetbridge's `initSendRunnable` does after its retries (`quit()`
    /// closes the socket).
    Hangup,
}

/// Things worth logging, as values rather than as strings, so the engine
/// decides what happened and the transport decides how to say it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    BadFrame(frame::FrameError),
    /// The dispatch key of a message no decoder claims -- exactly what somebody
    /// adding one needs.
    Unread { command: u8, kind: Option<u8>, len: usize },
    Ambiguous { command: u8, kind: Option<u8>, len: usize },
    Refused(encode::Refusal),
    /// An ACK arrived with nothing outstanding, or for a sequence we are not
    /// waiting on. Not fatal, and not silent either.
    StrayAck { seq: u8 },
    /// A client's command became a frame, and this is how well-founded that
    /// frame is.
    ///
    /// For the log, where somebody with a headset and a capture running reads
    /// which of this port's writes stand on a capture and which on a header
    /// or on another project's word -- the frames worth comparing first. Not
    /// in the reply, which stays the bare `ok` the C++ daemon sends: a client
    /// written against either reads `ok` and nothing else, and no reader of
    /// ours would do anything with the tier but print it.
    Sent { command: u8, kind: u8, tier: &'static str, set: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// How long a sent frame may go unacknowledged before it is resent, and
    /// -- once [`Limits::retries`] are spent -- before the link is dropped.
    ///
    /// STRICTLY UNDER the daemon socket's `DISPATCH_WAIT` (2s in qs-sonyd),
    /// which is itself under `qs-sony`'s 3s per-operation timeout, so that a
    /// client waiting on a frame hears the engine's answer at the FIRST
    /// expiry rather than the socket's own timeout. Equal would not do: the
    /// socket starts its timer before the engine has stamped the frame, so
    /// the engine's reply at exactly the same interval always arrives after
    /// the socket has given the client up. Gadgetbridge waits 1250ms before
    /// its resend (`SonyHeadphonesIoThread.scheduleInitRetry`); this is that
    /// with a little more slack for a headset over a busy link.
    pub ack_wait: Millis,
    /// How many times an unacknowledged frame is resent, at the SAME
    /// sequence, before the link is dropped. Gadgetbridge resends its init
    /// twice (`SonyHeadphonesIoThread.initSendRunnable`: `initRetries++ < 2`)
    /// and then closes the socket.
    pub retries: u8,
    /// Frames queued but not yet on the wire. A headset that stops reading must
    /// not grow this without bound.
    pub outbox: usize,
    /// Undecodable frames on one link before it is abandoned as unsynchronised.
    pub bad_frames: u32,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits { ack_wait: 1_500, retries: 2, outbox: 64, bad_frames: 32 }
    }
}

/// The document's `phase`, as its four strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Connecting,
    Ready,
    Error,
}

impl Phase {
    fn as_str(self) -> &'static str {
        match self {
            Phase::Idle => "idle",
            Phase::Connecting => "connecting",
            Phase::Ready => "ready",
            Phase::Error => "error",
        }
    }
}

/// A frame we have queued or sent, and who is waiting on it.
#[derive(Debug, Clone)]
struct Pending {
    out: encode::Outgoing,
    waiting: Option<ClientId>,
}

#[derive(Debug, Clone)]
struct InFlight {
    pending: Pending,
    seq: u8,
    sent_at: Millis,
    /// Retransmissions already made. `pump` starts it at 0.
    tries: u8,
}

pub struct Session {
    version: String,
    limits: Limits,
    facts: encode::Facts,
    state: state::State,
    published: Option<String>,
    scanner: frame::Scanner,
    phase: Phase,
    tx_seq: u8,
    outbox: VecDeque<Pending>,
    inflight: Option<InFlight>,
    /// The moment of the step now running. `pump` needs a real one for the
    /// acknowledgement deadline, and the engine has no clock to ask.
    last_now: Millis,
    bad: u32,
    /// The support tables have arrived and the per-feature queries have not
    /// been asked yet. They cannot be queued when the tables land, because the
    /// handshake is still draining and `feature_gets` is derived from BOTH
    /// tables -- so it waits for the outbox.
    want_feature_gets: bool,
    /// They have now been asked, once, on this link.
    ///
    /// WITHOUT THIS THEY GO OUT TWICE, and `pump`'s comment claimed otherwise
    /// for as long as it existed. `want_feature_gets` is set by EVERY
    /// `SupportFunctions` reading, and [`encode::handshake`] asks for two of
    /// them -- one per support table, `DataMdr` and `DataMdrNo2`. The only
    /// thing that used to collapse the two into one batch was `pump`'s
    /// `outbox.is_empty()` guard happening to be false when the first reply
    /// landed, which is true when the second table's GET is still queued and
    /// false as soon as anything reorders the handshake. Replaying the
    /// captured corpus reorders it, and
    /// `no_question_is_asked_twice_on_one_link` caught six queries going out
    /// twice.
    ///
    /// Twice is not a correctness bug -- every one of them is an idempotent
    /// read -- but the link is stop-and-wait and the headset grants exactly one
    /// control session across every device paired with it, so six wasted round
    /// trips are six the phone spends locked out.
    ///
    /// `Verb::Refresh` deliberately re-queues the same batch and does not go
    /// through this flag: asking again because a person asked is the case this
    /// is not about.
    asked_feature_gets: bool,
    /// [`Effect::Hangup`] has been asked for and the transport has not yet
    /// reported the link down. Bytes still in the socket buffer arrive in
    /// that window, and a support-function reply among them would set the
    /// phase back to Ready and clear the reason the hang-up recorded -- so
    /// the link's remaining input is ignored until [`Session::link_down`] or
    /// [`Session::link_up`] closes the window.
    hanging_up: bool,
}

impl Session {
    pub fn new(version: String, limits: Limits) -> Session {
        Session {
            version,
            limits,
            facts: encode::Facts::blank(),
            state: state::State::default(),
            published: None,
            scanner: frame::Scanner::new(),
            phase: Phase::Idle,
            // OUR FIRST FRAME CARRIES SEQUENCE 0, and that is not a guess: the
            // first thing the captured headset sent is an ACK carrying 1, and
            // an ACK carries 1 minus the sequence it acknowledges. See
            // PROVENANCE.md, "What the corpus settles about a SESSION".
            tx_seq: 0,
            outbox: VecDeque::new(),
            inflight: None,
            last_now: 0,
            bad: 0,
            want_feature_gets: false,
            asked_feature_gets: false,
            hanging_up: false,
        }
    }

    /// The document as it now stands: `state::serialize`'s output, unchanged,
    /// so it ends in exactly the one newline the reference's file ends in.
    pub fn document(&self) -> String {
        state::serialize(&self.state, &self.version)
    }

    /// The earliest moment this session wants to be stepped again, or `None`
    /// when nothing is outstanding and the transport may block.
    pub fn deadline(&self) -> Option<Millis> {
        self.inflight.as_ref().map(|f| f.sent_at.saturating_add(self.limits.ack_wait))
    }

    /// The whole engine.
    ///
    /// Total: every input reaches an arm and nothing here can panic. Every read
    /// of device bytes goes through the decoders, which are themselves total.
    pub fn step(&mut self, input: Input<'_>) -> Vec<Effect> {
        let mut out = Vec::new();
        self.last_now = match &input {
            Input::LinkUp { now, .. }
            | Input::Bytes { now, .. }
            | Input::LinkDown { now, .. }
            | Input::Command { now, .. }
            | Input::Trouble { now, .. }
            | Input::Tick { now } => *now,
        };
        match input {
            Input::LinkUp { now, peer } => self.link_up(now, peer, &mut out),
            Input::Bytes { now, bytes } => self.bytes(now, bytes, &mut out),
            Input::LinkDown { now, why } => self.link_down(now, why, &mut out),
            Input::Command { now, id, line } => self.command(now, id, line, &mut out),
            Input::Trouble { now, what } => self.trouble(now, what),
            Input::Tick { now } => self.tick(now, &mut out),
        }
        self.pump(&mut out);
        self.publish_if_changed(&mut out);
        out
    }

    // ── the arms ─────────────────────────────────────────────────────────────

    /// Everything the last link accumulated, dropped.
    ///
    /// THE INVARIANT, and it lives here rather than at each end of the link so
    /// that it is one line to hold and one line to break. Written twice --
    /// once in `link_up`, once in `link_down` -- either copy would hold it on
    /// its own, so neither could be shown to be doing the work and a test
    /// could not tell a live invariant from a redundant one.
    ///
    /// `facts` is the reason (see the module header): a SET echoes fields from
    /// a reading, and a reading from the previous headset would be written
    /// into this one. `state` is the visible half of the same thing -- a
    /// document still showing the last device's battery. `scanner` holds half
    /// a frame that will never be finished, and would corrupt the first frame
    /// of the new link by prefixing it. `bad` is a count of a link's junk and
    /// is not the next link's debt.
    fn forget(&mut self) {
        self.facts = encode::Facts::blank();
        self.state = state::State::default();
        self.scanner = frame::Scanner::new();
        self.bad = 0;
    }

    fn link_up(&mut self, _now: Millis, peer: Peer, _out: &mut Vec<Effect>) {
        // A fresh link is a fresh conversation, whether or not a LinkDown came
        // first: a transport that redials may report the new link without
        // having reported the old one closing.
        self.forget();
        self.outbox.clear();
        self.inflight = None;
        self.want_feature_gets = false;
        self.asked_feature_gets = false;
        self.hanging_up = false;
        self.tx_seq = 0;

        self.state.device.name = peer.name;
        self.state.device.protocol = peer.protocol;
        self.state.connected = true;
        self.phase = Phase::Connecting;

        for out in encode::handshake() {
            self.queue(out, None);
        }
    }

    fn link_down(&mut self, _now: Millis, why: Down, out: &mut Vec<Effect>) {
        // Anyone still waiting is told, rather than left on a deadline.
        self.abandon_queued("error this headset disconnected before it answered", out);
        // `forget` resets `last_error` with the rest of the state, so the
        // reason a hang-up recorded is carried across it.
        let hung_up_for = self.state.last_error.clone();
        self.forget();
        self.hanging_up = false;
        self.phase = match why {
            Down::Closed => Phase::Idle,
            Down::Failed(reason) => {
                self.state.last_error = reason.to_string();
                Phase::Error
            }
            Down::HungUp => {
                self.state.last_error = hung_up_for;
                Phase::Error
            }
        };
        self.state.connected = false;
    }

    /// Drop everything queued and in flight, answering `line` to any client
    /// still waiting on one of those frames.
    fn abandon_queued(&mut self, line: &str, out: &mut Vec<Effect>) {
        for pending in self.outbox.drain(..).chain(self.inflight.take().map(|f| f.pending)) {
            if let Some(id) = pending.waiting {
                out.push(Effect::Reply { id, line: line.to_string() });
            }
        }
        // Otherwise `pump` would queue the per-feature queries onto a link
        // that is being torn down.
        self.want_feature_gets = false;
    }

    /// Give the link up: record `reason`, answer `client_line` to every
    /// client still queued, clear the queue, and ask the transport to close
    /// the socket. The two lines differ because the two paths do: a headset
    /// that stopped acknowledging did not answer the client's frame, while a
    /// headset sending unreadable frames may have acknowledged everything
    /// and is being dropped because THIS daemon cannot read it.
    ///
    /// ONE hang-up per link, held by `hanging_up`: `bytes` and `tick` do
    /// nothing more until the transport reports the link down.
    fn hang_up(&mut self, reason: &str, client_line: &str, out: &mut Vec<Effect>) {
        self.abandon_queued(client_line, out);
        self.state.last_error = reason.to_string();
        self.phase = Phase::Error;
        self.hanging_up = true;
        out.push(Effect::Hangup);
    }

    fn bytes(&mut self, now: Millis, bytes: &[u8], out: &mut Vec<Effect>) {
        if self.hanging_up {
            return;
        }
        self.scanner.push(bytes);
        while let Some(result) = self.scanner.next_frame() {
            match result {
                Err(e) => {
                    self.bad = self.bad.saturating_add(1);
                    out.push(Effect::Note(Note::BadFrame(e)));
                    if self.bad == self.limits.bad_frames {
                        self.hang_up(
                            "this headset is sending frames this daemon cannot read",
                            "error this headset is sending frames this daemon cannot read",
                            out,
                        );
                        // Nothing more is read from this link: `hanging_up`
                        // is set, and the junk still in the scanner would
                        // only be counted against a link already given up.
                        return;
                    }
                }
                Ok(f) => self.frame(now, f, out),
            }
        }
    }

    fn frame(&mut self, now: Millis, f: frame::Frame, out: &mut Vec<Effect>) {
        if f.data_type == DataType::Ack {
            self.acknowledged(f.seq, out);
            return;
        }
        // The ACK first, before anything this frame's contents make us send: a
        // headset waiting to be acknowledged will not read the next question.
        if f.data_type.is_data() {
            out.push(Effect::Send(frame::ack_frame(f.seq)));
        }
        let reading = payload::decode(f.data_type, &f.payload);
        match &reading {
            payload::Reading::Unhandled { command, kind, len } => {
                out.push(Effect::Note(Note::Unread { command: *command, kind: *kind, len: *len }));
            }
            payload::Reading::Ambiguous { command, kind, len, .. } => {
                out.push(Effect::Note(Note::Ambiguous {
                    command: *command,
                    kind: *kind,
                    len: *len,
                }));
            }
            _ => {}
        }
        self.facts.observe(f.data_type, &f.payload);
        self.project(&reading);
        let _ = now;
    }

    /// An ACK arrived.
    ///
    /// CREDITED ONLY TO A FRAME ACTUALLY IN FLIGHT, and only at the sequence
    /// that frame would be acknowledged at. A session that cleared whatever was
    /// pending on any ACK would pass every replay of this corpus -- the capture
    /// is perfectly alternating and never loses one -- and would silently drop
    /// a frame the moment a real link did.
    fn acknowledged(&mut self, seq: u8, out: &mut Vec<Effect>) {
        // ack_frame(s) carries 1 - s, so an ACK of `seq` acknowledges 1 - seq.
        let acknowledges = 1u8.wrapping_sub(seq);
        match self.inflight.as_ref() {
            Some(f) if f.seq == acknowledges => {
                let done = self.inflight.take();
                if let Some(f) = done {
                    if let Some(id) = f.pending.waiting {
                        out.push(Effect::Reply { id, line: "ok".to_string() });
                    }
                }
                self.tx_seq = 1u8.wrapping_sub(self.tx_seq);
            }
            _ => out.push(Effect::Note(Note::StrayAck { seq })),
        }
    }

    fn trouble(&mut self, _now: Millis, what: &str) {
        // Ignored while a headset is answering. A dialler that has not yet
        // noticed the link came up is the ordinary source of one of these, and
        // its news is older than the link.
        if self.state.connected {
            return;
        }
        self.state.last_error = what.to_string();
    }

    fn tick(&mut self, now: Millis, out: &mut Vec<Effect>) {
        if self.hanging_up {
            return;
        }
        let Some(f) = self.inflight.as_mut() else { return };
        if now.saturating_sub(f.sent_at) < self.limits.ack_wait {
            return;
        }
        // A client is answered at the FIRST expiry and not kept for the
        // retries: the daemon's socket gives the client up after
        // DISPATCH_WAIT and `qs-sony` after 3s, so an answer that waited for
        // the second resend would reach nobody -- and `ack_wait` is under
        // DISPATCH_WAIT so that even this first answer arrives in time. The
        // frame itself goes on being retried for the link's sake, with
        // nobody waiting on it.
        if let Some(id) = f.pending.waiting.take() {
            out.push(Effect::Reply {
                id,
                line: "error this headset did not acknowledge the command".to_string(),
            });
        }
        if f.tries < self.limits.retries {
            // THE IDENTICAL FRAME AT THE SAME SEQUENCE. Gadgetbridge's
            // `initSendRunnable` re-sends `encodeInit()`, which encodes with
            // the `sequenceNumber` field untouched -- that field changes only
            // when an ACK is decoded (`SonyHeadphonesProtocol.decodeResponse`,
            // the ACK branch, before `handleAck` is called). So an ACK for
            // the resend is credited to this frame by `acknowledged`, which
            // keys on the sequence.
            f.tries += 1;
            f.sent_at = now;
            out.push(Effect::Send(frame::pack(
                f.pending.out.data_type,
                f.seq,
                &f.pending.out.payload,
            )));
            return;
        }
        self.hang_up(
            "this headset stopped acknowledging",
            "error this headset did not acknowledge the command",
            out,
        );
    }

    fn command(&mut self, _now: Millis, id: ClientId, line: &str, out: &mut Vec<Effect>) {
        let verb = match command::parse_line(line) {
            Ok(v) => v,
            Err(message) => {
                out.push(Effect::Reply { id, line: format!("error {message}") });
                return;
            }
        };
        // The four the daemon answers itself. Routing these into `encode` would
        // get `Refusal::NoEncoder`, which is true and useless: they are not
        // gaps, they are simply not wire messages.
        match &verb {
            Verb::Status => {
                // `ok ` and the document, as the reference's `daemon_main`
                // answers: `"ok " + serializeState(...)`. The document's own
                // newline is trimmed because `serve` terminates every reply
                // line itself; kept, the client would read a blank line.
                let document = self.document();
                out.push(Effect::Reply { id, line: format!("ok {}", document.trim_end()) });
                return;
            }
            Verb::PanelOpen | Verb::PanelClose => {
                out.push(Effect::Reply { id, line: "ok".to_string() });
                return;
            }
            Verb::Refresh => {
                if self.phase == Phase::Idle || !self.state.connected {
                    out.push(Effect::Reply {
                        id,
                        line: "error no headset is connected".to_string(),
                    });
                    return;
                }
                for query in encode::feature_gets(&self.facts) {
                    self.queue(query, None);
                }
                out.push(Effect::Reply { id, line: "ok".to_string() });
                return;
            }
            _ => {}
        }

        if !self.state.connected {
            out.push(Effect::Reply { id, line: "error no headset is connected".to_string() });
            return;
        }
        match encode::encode(&self.facts, &verb) {
            Ok(outgoing) => {
                if self.outbox.len() >= self.limits.outbox {
                    out.push(Effect::Reply {
                        id,
                        line: "error this daemon has too many commands outstanding".to_string(),
                    });
                    return;
                }
                if let Some((command, kind, _)) = outgoing.key() {
                    out.push(Effect::Note(Note::Sent {
                        command,
                        kind,
                        tier: outgoing.evidence.tier(),
                        set: outgoing.evidence.set(),
                    }));
                }
                self.queue(outgoing, Some(id));
            }
            Err(refusal) => {
                out.push(Effect::Reply { id, line: format!("error {}", refusal.message()) });
                out.push(Effect::Note(Note::Refused(refusal)));
            }
        }
    }

    // ── the plumbing ─────────────────────────────────────────────────────────

    fn queue(&mut self, out: encode::Outgoing, waiting: Option<ClientId>) {
        if self.outbox.len() >= self.limits.outbox {
            return;
        }
        self.outbox.push_back(Pending { out, waiting });
    }

    /// Put the next queued frame on the wire, if nothing is in flight.
    ///
    /// Stop-and-wait, which is what the capture shows: every reply is
    /// acknowledged before the next question goes out.
    fn pump(&mut self, out: &mut Vec<Effect>) {
        if self.inflight.is_some() || self.hanging_up {
            return;
        }
        // The handshake has drained and the device has said what it supports,
        // so ask for the state of each thing it supports. Once per link: the
        // corpus shows the app doing exactly this, and re-asking on every reply
        // would be a loop.
        //
        // "Once" IS `asked_feature_gets`, NOT THIS GUARD, and that field says
        // why at length -- the guard alone let the whole batch go out twice,
        // which `no_question_is_asked_twice_on_one_link` caught. What
        // `outbox.is_empty()` is for is the other half: `feature_gets` is
        // derived from BOTH support tables, so it must not be built while the
        // handshake that fetches the second one is still draining.
        if self.outbox.is_empty() && self.want_feature_gets {
            self.want_feature_gets = false;
            self.asked_feature_gets = true;
            for query in encode::feature_gets(&self.facts) {
                self.queue(query, None);
            }
        }
        let Some(pending) = self.outbox.pop_front() else { return };
        let seq = self.tx_seq;
        let wire = frame::pack(pending.out.data_type, seq, &pending.out.payload);
        out.push(Effect::Send(wire));
        // sent_at is filled by the caller's clock on the next step; the
        // deadline uses it, so it must be a real moment. `pump` runs at the end
        // of `step`, which has one, so it is threaded through `last_now`.
        self.inflight = Some(InFlight { pending, seq, sent_at: self.last_now, tries: 0 });
    }

    /// The ambient level's range for the mode the headset is in now, as it
    /// stated it, or the compatibility range when it has not.
    ///
    /// From `facts` rather than from the reading in hand, so the document and
    /// the encoder answer the same question the same way: a level the page
    /// offers is a level the encoder accepts. Re-derived on a noise reading as
    /// well as on a capability reply, because focus-on-voice moves which of
    /// the two ranges applies.
    fn refresh_ambient_range(&mut self) {
        let voice = self.state.noise.focus_on_voice;
        self.state.ambient_range = match self.facts.ambient_range(voice) {
            Some(r) => state::AmbientRange { min: r.min, max: r.max, step: r.step, from_device: true },
            None => state::AmbientRange::default(),
        };
    }

    fn publish_if_changed(&mut self, out: &mut Vec<Effect>) {
        self.state.phase = self.phase.as_str().to_string();
        let now = self.document();
        if self.published.as_deref() != Some(now.as_str()) {
            // The revision counts CHANGES, not frames: a reader uses it to tell
            // a re-read that found something new from one that did not, and a
            // headset repeating an unchanged reading is not news.
            self.state.revision = self.state.revision.saturating_add(1);
            let now = self.document();
            self.published = Some(now);
            out.push(Effect::Publish);
        }
    }

    /// One decoded reading into the document.
    fn project(&mut self, reading: &payload::Reading) {
        match reading {
            payload::Reading::Battery(report) => {
                let put = |slot: &mut state::Battery, cell: Option<payload::Cell>| {
                    if let Some(c) = cell {
                        slot.present = true;
                        slot.level = i32::from(c.level);
                        slot.charging = c.charging.as_str().to_string();
                    }
                };
                put(&mut self.state.battery.main, report.main);
                put(&mut self.state.battery.left, report.left);
                put(&mut self.state.battery.right, report.right);
                put(&mut self.state.battery.case, report.case);
            }
            payload::Reading::Noise(report) => {
                self.state.noise.mode = report.mode.as_str().to_string();
                // Only what this variant carries. NC_ON_OFF has no level and
                // no ambient mode, and publishing a zero for them would draw a
                // level nobody set.
                if let Some(level) = report.ambient_level {
                    self.state.noise.ambient_level = i32::from(level);
                }
                if let Some(voice) = report.focus_on_voice {
                    self.state.noise.focus_on_voice = voice;
                }
                if let Some(adaptive) = report.adaptive {
                    self.state.noise.adaptive = adaptive;
                }
                if let Some(byte) = report.adaptive_sensitivity {
                    // NoiseAdaptiveSensitivity (T1:1203).
                    self.state.noise.adaptive_sensitivity = match byte {
                        0x00 => "standard",
                        0x01 => "high",
                        0x02 => "low",
                        _ => "unknown",
                    }
                    .to_string();
                }
                self.refresh_ambient_range();
            }
            payload::Reading::NoiseCapability { .. } => self.refresh_ambient_range(),
            payload::Reading::Device(info) => {
                if !info.model.is_empty() {
                    self.state.device.model = info.model.clone();
                }
                if !info.firmware.is_empty() {
                    self.state.device.firmware = info.firmware.clone();
                }
            }
            payload::Reading::Equalizer(report) => {
                self.state.equalizer.preset = preset_name(report.preset).to_string();
                // THE SPLIT IS `payload::EqShape`, not a copy of it, since
                // `mdr::encode` has to invert exactly this to build an
                // `eq-band` or `clear-bass` write. That type carries the
                // libmdr and Gadgetbridge citations and the reason a reader
                // may guess at an unnamed count where a writer may not.
                //
                // NO steps (`case 0`) is a preset change alone, and the last
                // known clear bass and bands stand -- which is why the empty
                // slice is matched before the shape is asked at all rather
                // than falling into `Unknown`, whose reading would replace the
                // bands with an empty array.
                let signed = |steps: &[u8], bias: i32| -> Vec<i32> {
                    steps.iter().map(|s| i32::from(*s) - bias).collect()
                };
                let eq = &mut self.state.equalizer;
                let steps = report.steps.as_slice();
                if !steps.is_empty() {
                    let shape = payload::EqShape::of(steps.len());
                    match shape.clear_bass_at() {
                        Some(0) => {
                            eq.clear_bass = i32::from(steps[0]) - shape.bias();
                            eq.bands = signed(&steps[1..], shape.bias());
                        }
                        // No shape puts clear bass anywhere but first, and a
                        // future one that did would need its own arm rather
                        // than silently landing here.
                        Some(_) => {}
                        None => {
                            if shape == payload::EqShape::TenBands {
                                eq.clear_bass = 0;
                            }
                            eq.bands = signed(steps, shape.bias());
                        }
                    }
                }
            }
            // TWO FRAMES, ONE ROW. The capability reply says what the slot is
            // and the param reply says whether it is on, so each finds or
            // creates the row and fills its own half. Keyed on the SLOT, which
            // is what the device addresses it by; the name is for a person and
            // could in principle repeat.
            payload::Reading::GeneralSettingInfo(info) => {
                let row = general_row(&mut self.state.general_settings, info.slot);
                row.key = info.key();
                row.name = info.name.clone();
                row.summary = info.summary.clone();
            }
            payload::Reading::GeneralSettingParam { slot, enabled } => {
                general_row(&mut self.state.general_settings, *slot).enabled = Some(*enabled);
            }
            payload::Reading::Volume { device_level } => {
                self.state.playback.volume =
                    command::mdr_volume_to_percent(i32::from(*device_level));
            }
            payload::Reading::Codec(codec) => {
                self.state.device.codec = codec.as_str().to_string();
            }
            payload::Reading::Connection { prior, .. } => {
                self.state.connection.priority = prior.as_str().to_string();
            }
            payload::Reading::PlayStatus(report) => {
                self.state.playback.status = report.status.as_str().to_string();
            }
            payload::Reading::UpscalingStatus { available } => {
                // `null` stays `null` for a byte EnableDisable does not name:
                // "the headset said something else" is not "unavailable".
                self.state.dsee_available = available.as_bool();
            }
            payload::Reading::Dsee(report) => {
                if let Some(switch) = report.enabled {
                    if let Some(on) = switch.as_bool() {
                        self.state.equalizer.dsee_enabled = on;
                    }
                }
                if let Some(kind) = report.upscaling {
                    self.state.equalizer.dsee_type = kind.as_str().to_string();
                }
            }
            payload::Reading::SpeakToChat(report) => {
                if let Some(switch) = report.enabled {
                    if let Some(on) = switch.as_bool() {
                        self.state.speak_to_chat.enabled = on;
                    }
                }
                if let Some(s) = report.sensitivity {
                    self.state.speak_to_chat.sensitivity = s.as_str().to_string();
                }
                if let Some(t) = report.timeout {
                    self.state.speak_to_chat.timeout = t.as_str().to_string();
                }
            }
            payload::Reading::SupportFunctions { table, functions } => {
                self.state.capabilities.apply_support_functions(*table, functions);
                // The write gate for `eq-preset`. Filled from the device's own
                // capability reply rather than left empty as the reference
                // leaves it, which made the verb unconditionally rejected.
                if self.state.capabilities.equalizer {
                    self.state.equalizer.available_presets =
                        command::EQ_PRESETS.iter().map(|p| (*p).to_string()).collect();
                }
                self.phase = Phase::Ready;
                // A headset that answered after a retransmission has
                // recovered; the reason recorded when it went quiet would
                // otherwise sit under "ready" for the rest of the link.
                self.state.last_error.clear();
                // Once per link. `asked_feature_gets` says why this is not
                // simply `= true`.
                if !self.asked_feature_gets {
                    self.want_feature_gets = true;
                }
            }
            payload::Reading::PairedDevices(devices) => {
                self.state.paired_devices = devices
                    .iter()
                    .map(|d| state::PairedDevice {
                        address: d.address.clone(),
                        name: d.name.clone(),
                        status: d.status,
                        detail: d.detail,
                    })
                    .collect();
            }
            payload::Reading::Unhandled { .. } | payload::Reading::Ambiguous { .. } => {}
        }
    }
}

/// `EqPresetId` as the document's word, or `unknown`.
///
/// The inverse of `encode`'s table, and deliberately NOT a second copy of it:
/// it resolves through the same `EQ_PRESET_IDS` ordering by asking the encoder
/// which name produces which byte.
fn preset_name(id: u8) -> &'static str {
    for (i, name) in command::EQ_PRESETS.iter().enumerate() {
        if encode::preset_id_at(i) == Some(id) {
            return name;
        }
    }
    "unknown"
}

/// Find or create the row for `slot`, keeping the list in slot order.
///
/// SLOT ORDER AND NOT ARRIVAL ORDER. The capability replies arrive in the
/// order the handshake asked for them, but a later NTFY can touch any slot, so
/// a push-on-first-sight list would reorder itself the first time somebody
/// toggled the second setting. A document whose array reorders under the
/// reader is one nothing can bind to by index.
fn general_row(rows: &mut Vec<state::GeneralSetting>, slot: u8) -> &mut state::GeneralSetting {
    let at = match rows.binary_search_by_key(&slot, |g| g.slot) {
        Ok(at) => at,
        Err(at) => {
            rows.insert(at, state::GeneralSetting { slot, ..Default::default() });
            at
        }
    };
    &mut rows[at]
}
