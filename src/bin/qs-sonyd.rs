//! The Sony headphone daemon.
//!
//!     qs-sonyd                    run in the foreground until stopped
//!     qs-sonyd once               dial, publish one document, exit
//!     qs-sonyd --alerts           also ask the headset for its confirmations
//!     qs-sonyd --record=DIR       also write every frame, both ways, as fixtures
//!
//! Speaks the MDR protocol to a connected Sony headset over RFCOMM, publishes
//! its state as JSON, and takes commands on a private Unix socket. It is a
//! native replacement for `sony-headphones-linux` and keeps that daemon's
//! contract on its own side: the same state path and socket path, the same
//! schema version and document words, one trailing newline on the document,
//! and the same socket verbs -- `status` answered with `ok ` and the document,
//! as the C++ `daemon_main` answers it. A reader written against either
//! daemon reads this one.
//!
//! WHAT IS WHERE
//!
//! Almost nothing is here. The protocol is in `mdr`, the state machine in
//! `sony::session`, the socket in `sony::serve`, the Bluetooth link in
//! `sony::link`. This file is the wiring, and the wiring is the part that
//! cannot be tested without a headset -- which is why it was kept this thin.
//!
//! THREE THREADS AND A MAILBOX
//!
//!   the engine     owns the `Session` and is the only thing that touches it.
//!                  Every input arrives as a `Msg` on one channel, so there is
//!                  no lock around the state machine and no question about
//!                  what order things happened in.
//!   the dialler    finds a headset, connects, and then READS from it until
//!                  the link ends -- one thread, not two, because a reader
//!                  with nothing to read is exactly a dialler waiting to
//!                  redial.
//!   the socket     accepts clients and forwards their lines into the mailbox,
//!                  waiting on a reply channel for the answer.
//!
//! The engine's own deadline closes the loop: `Session::deadline` says when
//! the headset will have failed to acknowledge, and the engine waits for a
//! message only that long.
//!
//! WHY THERE IS NO SIGNAL HANDLER
//!
//! A handler may do almost nothing that is safe, and everything that would
//! need doing here -- unlinking the socket, flushing the document -- is not on
//! that list. So SIGTERM ends the process the way it ends any process: the
//! kernel drops the flock and the socket file is left behind, which is
//! precisely the case `serve::bind` is built to clean up on the next start.
//! A handler that set a flag would also need something to notice it, and the
//! threads that would have to notice are all blocked on the kernel.

use sony_xm5::support::{cli::Args, emit};
use sony_xm5::sony::{
    self, link,
    serve::{self, Dispatch},
    session::{ClientId, Down, Effect, Input, Limits, Note, Peer, Session},
};
use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long a client's line may wait for the engine before it is told the
/// daemon is busy.
///
/// Under `qs-sony`'s own 3s per-operation timeout, so the client hears a
/// reason rather than a closed socket. And STRICTLY OVER the engine's
/// `Limits::ack_wait`, so an unanswering headset produces an error from the
/// engine and not a timeout here: this timer starts in `Mailbox::dispatch`
/// before the engine has received the line and stamped the frame, and the
/// engine's expiry is measured from that later stamp -- so with the two
/// equal, the engine's answer always arrived after the client had been given
/// up. `timing_lets_the_engine_answer_before_the_socket_gives_up` holds the
/// order.
const DISPATCH_WAIT: Duration = Duration::from_secs(2);

/// How long to wait for a headset to answer a connect or an SDP query.
const DIAL_TIMEOUT: Duration = Duration::from_secs(5);

/// How long `bluetoothctl` gets to list what is connected.
const INVENTORY_TIMEOUT: Duration = Duration::from_secs(5);

/// The longest a `once` run may take, whatever the headset does.
///
/// Generous next to a dial and a handshake, and short enough that a script
/// which runs this is not the reason a login hangs.
const ONCE_LONGEST: Duration = Duration::from_secs(30);

/// How long a `once` run waits for the headset to go quiet before believing it.
///
/// NOTHING OUTSTANDING IS NOT THE SAME AS FINISHED, and this daemon used to
/// treat it as though it were. `Session::deadline` reports only the frame in
/// FLIGHT, so it is `None` the instant an acknowledgement is credited and the
/// outbox is empty -- whether or not the device's RET frames have landed. The
/// first `Effect::Publish` fires on LinkUp, which sets `connected`, the name
/// and `phase: connecting`, so `published_once` is true long before anything
/// has been decoded. The two conditions together said "done" at the first
/// quiet moment of the handshake, and a one-shot run could leave behind a
/// document reporting a connected headset with no battery in it.
///
/// EXECUTED, against the captured session:
/// `nothing_outstanding_does_not_mean_the_headset_has_finished_answering` in
/// `sony_session.rs` walks the 70 captured frames and finds the gap is
/// reachable -- at least one moment with nothing outstanding, `connected:
/// true`, and no battery decoded -- while the final document has one. So the
/// window is real in this port; how WIDE it is on hardware is not measurable
/// here.
///
/// The number is theirs: `delarosa1312/omarchy-sony-xm5` (MIT, Copyright (c)
/// 2026 Delarosa1312), `docs/protocol.md` -- "the handshake completing does not
/// mean the state has arrived. Reading immediately gives zeroes; values land
/// over the following second or two. So `open()` now issues a sync and pumps
/// events for a couple of seconds before returning. This was the cause of the
/// '0% battery' that first looked like a struct bug." 1.5s is the lower end of
/// their range, because `ONCE_LONGEST` bounds the whole run anyway and a
/// diagnostic that takes longer than it needs to is its own cost.
///
/// AND IT IS ALSO THE WAIT, which is the part that is easy to get wrong. A
/// settle applied to the exit condition ALONE would leave a machine with no
/// headset blocked in `recv_timeout` for the full hour below: the dial fails,
/// `Trouble` publishes, and then nothing else ever arrives to bring the loop
/// round to re-check. So a one-shot run with nothing outstanding waits THIS,
/// not an hour, and the timeout itself is what ends the run.
const ONCE_SETTLE: Duration = Duration::from_millis(1_500);

/// The gap between dial attempts, and the gap it grows to.
///
/// A headset that is off is the ordinary case, not a fault, and the daemon
/// should not spend a core on it. It starts short so that switching a headset
/// on feels immediate, and backs off so that a machine with no Sony hardware
/// at all costs one `bluetoothctl` a minute.
const REDIAL_FIRST: Duration = Duration::from_secs(2);
const REDIAL_LONGEST: Duration = Duration::from_secs(60);

/// What the engine thread is told about.
enum Msg {
    LinkUp { peer: Peer, writer: link::Channel },
    Bytes(Vec<u8>),
    /// Why the link ended, as `read_until_gone` saw it -- NOT as a sentence.
    ///
    /// It used to be a `&'static str` that the engine wrapped in
    /// `Down::Failed` unconditionally, which threw away the one distinction
    /// the reader had already made: a clean end-of-file is the headset
    /// switching off, and every other read error is a fault. Switching a
    /// headset off is the ordinary way a session ends, and it left the
    /// daemon in `phase: "error"` until something redialled, so the
    /// Headphones integration drew a red fault for the normal case
    /// (jayos's `readSony`, `ts/src/settings/Integration.ts`).
    LinkDown(Down),
    Command { id: ClientId, line: String, reply: Sender<String> },
    /// The dialler could not reach a headset, and this is what to say about
    /// it in the document.
    Trouble(String),
}

/// Milliseconds since the daemon started.
///
/// The engine is told the time rather than asking, so one clock reads it and
/// everything downstream is a pure function of the sequence.
fn now(began: Instant) -> u64 {
    began.elapsed().as_millis() as u64
}

fn main() -> ExitCode {
    let args = Args::from_env();
    let once = match args.command_or("run") {
        "run" => false,
        "once" => true,
        other => return emit::unknown_command("qs-sonyd", other, &["run", "once"]),
    };

    let socket = sony::socket_path();
    let bound = match serve::bind(&socket) {
        Ok(bound) => bound,
        Err(serve::BindError::AlreadyRunning) => {
            // Not a failure to work around. The daemon that holds the lock is
            // serving clients correctly, and a second one is not wanted.
            return emit::failure("another sony daemon is already running");
        }
        Err(e) => return emit::failure(format!("cannot take the daemon socket: {e}")),
    };

    let began = Instant::now();
    let (post, mailbox) = mpsc::channel::<Msg>();

    // The document exists before anything is connected. A reader that finds no
    // file cannot tell a daemon that has not started from one that has nothing
    // to say, and `qs-sony available` is the difference.
    // Off unless asked for: a headset with alerts on waits for an answer, and
    // only a page that shows the question can give one. `encode::alerts_on`
    // has the long form.
    let session = Session::new(sony::DAEMON_VERSION.to_string(), Limits::default())
        .with_alerts(args.has("alerts"));
    // Opened before the link exists, so a directory that cannot be made fails
    // the start rather than the first frame. `capture::Recorder` says what
    // the files are and why the directory is private.
    let recorder = match args.value("record") {
        None => None,
        Some(dir) => match sony_xm5::capture::Recorder::new(std::path::Path::new(dir)) {
            Ok(r) => Some(r),
            Err(e) => return emit::failure(format!("cannot record into {dir}: {e}")),
        },
    };
    publish(&session);

    let stopper = bound.stopper();
    let dialler_post = post.clone();
    let dialler = std::thread::Builder::new()
        .name("qs-sonyd-dial".to_string())
        .spawn(move || dial_forever(dialler_post, once))
        .expect("the dialler thread starts");

    let socket_post = post.clone();
    let server = std::thread::Builder::new()
        .name("qs-sonyd-socket".to_string())
        .spawn(move || {
            bound.serve(Arc::new(Mailbox { post: socket_post }), serve::Limits::default())
        })
        .expect("the socket thread starts");

    // This thread is the engine.
    let outcome = run_engine(session, began, mailbox, once, recorder);

    stopper.stop();
    let _ = server.join();
    // The dialler is blocked reading a Bluetooth socket and cannot be joined
    // without one more handle on it than is worth threading through here. The
    // process is exiting; the kernel closes it.
    drop(dialler);

    outcome
}

/// The socket's view of the engine: hand a line over, wait for the answer.
struct Mailbox {
    post: Sender<Msg>,
}

impl Dispatch for Mailbox {
    fn dispatch(&self, id: ClientId, line: &str) -> String {
        let (reply, answer) = mpsc::channel();
        if self.post.send(Msg::Command { id, line: line.to_string(), reply }).is_err() {
            return "error this daemon is shutting down".to_string();
        }
        match answer.recv_timeout(DISPATCH_WAIT) {
            Ok(line) => line,
            // The engine has its own, shorter deadline on a headset, so this
            // one expiring means the ENGINE is wedged rather than the headset.
            // Said differently for exactly that reason.
            Err(RecvTimeoutError::Timeout) => "error this daemon did not answer in time".to_string(),
            Err(RecvTimeoutError::Disconnected) => {
                "error this daemon is shutting down".to_string()
            }
        }
    }
}

/// Drain the mailbox into the session, and act on what comes out.
fn run_engine(
    mut session: Session,
    began: Instant,
    mailbox: mpsc::Receiver<Msg>,
    once: bool,
    mut recorder: Option<sony_xm5::capture::Recorder>,
) -> ExitCode {
    let mut writer: Option<link::Channel> = None;
    // Set when the ENGINE asked for the link to be shut, so the LinkDown the
    // reader then reports is not written up as the headset hanging up.
    let mut hung_up = false;
    let mut waiting: HashMap<ClientId, Sender<String>> = HashMap::new();
    let mut published_once = false;
    // When the document last changed. `ONCE_SETTLE` is measured from here, so
    // every reply that decodes into something new restarts the window.
    let mut last_publish = Instant::now();

    loop {
        // The engine's own deadline: how long until the headset will have
        // failed to acknowledge. `None` means nothing is outstanding and the
        // wait is open-ended.
        let wait = match session.deadline() {
            Some(at) => Duration::from_millis(at.saturating_sub(now(began)).max(1)),
            // A one-shot run with nothing outstanding is in its settle window
            // rather than finished. `ONCE_SETTLE` says why this is not the
            // hour below: without it, a machine with no headset would block
            // here until something arrived, and nothing ever would.
            None if once => ONCE_SETTLE,
            None => Duration::from_secs(3600),
        };

        let effects = match mailbox.recv_timeout(wait) {
            Err(RecvTimeoutError::Disconnected) => break,
            Ok(Msg::LinkUp { peer, writer: channel }) => {
                if let Some(r) = recorder.as_mut() {
                    r.link_up();
                }
                writer = Some(channel);
                hung_up = false;
                session.step(Input::LinkUp { now: now(began), peer })
            }
            Ok(Msg::Bytes(bytes)) => {
                if let Some(r) = recorder.as_mut() {
                    note_record(r.received(wall_ms(), &bytes));
                }
                session.step(Input::Bytes { now: now(began), bytes: &bytes })
            }
            Ok(Msg::LinkDown(why)) => {
                writer = None;
                // A hang-up this engine asked for outranks whatever the
                // reader saw, because `Effect::Hangup` shuts the socket and
                // the reader's end-of-file is that shutdown, not the headset.
                let why = if std::mem::take(&mut hung_up) { Down::HungUp } else { why };
                session.step(Input::LinkDown { now: now(began), why })
            }
            Ok(Msg::Command { id, line, reply }) => {
                waiting.insert(id, reply);
                session.step(Input::Command { now: now(began), id, line: &line })
            }
            Ok(Msg::Trouble(what)) => {
                session.step(Input::Trouble { now: now(began), what: &what })
            }
            Err(RecvTimeoutError::Timeout) => session.step(Input::Tick { now: now(began) }),
        };

        for effect in effects {
            match effect {
                Effect::Send(bytes) => {
                    // With no link, the frame is dropped and the engine's own
                    // acknowledgement deadline reports it -- which is the
                    // right answer: a frame that could not be sent was not
                    // acknowledged, and there is one place that says so.
                    if let Some(link) = writer.as_ref() {
                        // Recorded only once it is on the wire: a frame that
                        // never left is not part of the conversation.
                        let written = link.write_all(&bytes);
                        if let (Ok(()), Some(r)) = (&written, recorder.as_mut()) {
                            note_record(r.sent(wall_ms(), &bytes));
                        }
                        if written.is_err() {
                            // The link is gone. Reported here rather than left
                            // to the reader, which may be blocked for as long
                            // as the headset stays silent.
                            link.shutdown();
                            writer = None;
                        }
                    }
                }
                Effect::Reply { id, line } => {
                    if let Some(client) = waiting.remove(&id) {
                        let _ = client.send(line);
                    }
                }
                Effect::Publish => {
                    publish(&session);
                    published_once = true;
                    last_publish = Instant::now();
                }
                Effect::Note(note) => log(&note),
                Effect::Hangup => {
                    // SHUT_RDWR wakes the reader parked in `read` -- it returns
                    // 0 -- so `read_until_gone` returns, the dialler posts
                    // LinkDown and redials with its backoff. Nothing else can
                    // reach that thread: it deliberately has no read deadline.
                    if let Some(link) = writer.take() {
                        link.shutdown();
                        hung_up = true;
                    }
                }
            }
        }

        // `once` is for a person checking the daemon works, and for the
        // installer: dial, say what was found, exit. It stops at the first
        // published document with nothing outstanding, so it takes as long as
        // the headset takes and no longer -- and after ONCE_LONGEST whatever
        // happens, because a headset that keeps sending notifications would
        // otherwise keep a one-shot running forever.
        if once
            && (began.elapsed() > ONCE_LONGEST
                || (published_once
                    && session.deadline().is_none()
                    && last_publish.elapsed() >= ONCE_SETTLE))
        {
            break;
        }
    }
    ExitCode::SUCCESS
}

/// Unix milliseconds, for a recorded frame's name. The engine's own clock
/// (`now`) counts from the daemon's start, which would give every recording
/// session the same names.
fn wall_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// A frame that could not be recorded is said on stderr and the daemon goes
/// on: a full disk must not cost the person their headset.
fn note_record(result: std::io::Result<()>) {
    if let Err(e) = result {
        eprintln!("qs-sonyd: cannot record a frame: {e}");
    }
}

fn publish(session: &Session) {
    let path = sony::state_file();
    if let Err(e) = sony::atomic_write(&path, &session.document()) {
        eprintln!("qs-sonyd: cannot write {}: {e}", path.display());
    }
}

/// What the engine could not make sense of, on stderr.
///
/// Notes are not errors. An unread message is a gap in this port, not a fault
/// in the headset, and the shape of it -- command, kind, length -- is exactly
/// what a person would need to extend the decoder. They go to stderr because
/// stdout is not a log: nothing reads this daemon's stdout, but making it a
/// stream of notes would still be wrong the day something does.
fn log(note: &Note) {
    match note {
        Note::BadFrame(e) => eprintln!("qs-sonyd: unreadable frame: {e:?}"),
        Note::Unread { command, kind, len } => {
            eprintln!("qs-sonyd: no decoder for command {command:?} kind {kind:?} len {len}");
        }
        Note::Ambiguous { command, kind, len } => {
            eprintln!("qs-sonyd: ambiguous message: command {command:?} kind {kind:?} len {len}");
        }
        Note::Refused(refusal) => eprintln!("qs-sonyd: refused: {}", refusal.message()),
        Note::StrayAck { seq } => eprintln!("qs-sonyd: an acknowledgement for seq {seq}, unasked"),
        Note::Sent { command, kind, tier, set } => {
            eprintln!("qs-sonyd: sending {set} (command {command:#04x} kind {kind:#04x}), {tier}");
        }
    }
}

// ── the dialler ──────────────────────────────────────────────────────────────

/// Find a headset, connect, read until the link ends, repeat.
fn dial_forever(post: Sender<Msg>, once: bool) {
    let mut wait = REDIAL_FIRST;
    loop {
        match dial() {
            Ok((device, generation, channel)) => {
                wait = REDIAL_FIRST;
                let reader = match channel.try_clone() {
                    Ok(reader) => reader,
                    Err(e) => {
                        let _ = post.send(Msg::Trouble(format!("cannot use the link: {e}")));
                        return;
                    }
                };
                // The generation of the MDR service that answered the SDP
                // query: 2 or 1, as the C++ daemon's `protocolGeneration`
                // reports for the same two UUIDs.
                let peer = Peer { name: device.name.clone(), protocol: generation };
                if post.send(Msg::LinkUp { peer, writer: channel }).is_err() {
                    return;
                }
                let why = read_until_gone(&reader, &post);
                if post.send(Msg::LinkDown(why)).is_err() {
                    return;
                }
            }
            Err(e) => {
                if post.send(Msg::Trouble(e.to_string())).is_err() {
                    return;
                }
            }
        }
        if once {
            return;
        }
        std::thread::sleep(wait);
        wait = (wait * 2).min(REDIAL_LONGEST);
    }
}

/// Read from the headset until it stops, forwarding every byte to the engine.
///
/// END-OF-FILE IS NOT A FAULT, AND THE DIFFERENCE IS DECIDED HERE.
///
/// `read` returning 0 is an orderly close: the peer sent a disconnect and the
/// kernel has nothing more to hand up. For a headset that is somebody
/// switching it off, taking it out of a case, or walking it out of range
/// cleanly -- the ordinary end of a session, several times a day. Every other
/// error (a reset, a timeout, an adapter going away) is a fault. They used to
/// be the same sentence wrapped in the same `Down::Failed`, so the ordinary
/// case published `phase: "error"` and the Headphones integration showed a red
/// fault with "the headset stopped responding" until something redialled.
///
/// READ EVIDENCE, NOT OBSERVED: nothing in this tree has spoken to a headset,
/// so which of the two a real XM5 produces on a power-off has not been seen
/// here. The distinction is the kernel's, not the headset's -- `read` on any
/// `SOCK_STREAM` returns 0 only after an orderly shutdown -- and the mapping
/// is written so that the WRONG guess is the safe one: an orderly close
/// mistaken for a fault is the bug being fixed, and a fault arriving as an
/// error is what already happened.
fn read_until_gone(link: &link::Channel, post: &Sender<Msg>) -> Down {
    // A read buffer, not a frame buffer: the engine's scanner is written for
    // a stream and does not care where the pieces land, so this only has to be
    // large enough that a busy headset is not read a byte at a time.
    let mut buf = vec![0u8; 4096];
    loop {
        match link.read(&mut buf) {
            Ok(0) => return Down::Closed,
            Ok(n) => {
                if post.send(Msg::Bytes(buf[..n].to_vec())).is_err() {
                    // Nothing will read the answer -- the engine has gone --
                    // but it is a failure rather than a headset that closed.
                    return Down::Failed("the daemon is shutting down");
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return Down::Failed("the connection to the headset failed"),
        }
    }
}

/// One attempt: which headset, which channel, and a socket to it -- with the
/// generation of the MDR service the channel came from.
///
/// The services are tried in the order `find_headset` lists them, V2 first
/// -- the order the C++ daemon's `startAttempt` tries them in -- and the
/// first one whose SDP record names an RFCOMM channel is dialled. Only a
/// record without a channel moves to the next service; `startAttempt` moves
/// on after ANY failed connect, which this does not: an SDP query that timed
/// out is reported, and the redial tries again.
fn dial() -> Result<(link::Device, i32, link::Channel), link::LinkError> {
    let (device, services) = find_headset()?;
    let mut channel = None;
    for mdr in &services {
        match link::mdr_channel(device.mac, mdr.service, DIAL_TIMEOUT) {
            Ok(found) => {
                channel = Some((found, mdr.generation));
                break;
            }
            // Advertised, but no RFCOMM channel under it: try the next one.
            Err(link::LinkError::NotSupported(_)) => continue,
            Err(e) => return Err(e),
        }
    }
    let Some((channel, generation)) = channel else {
        return Err(link::LinkError::NotSupported(device.mac));
    };
    let socket = link::connect_rfcomm(device.mac, channel, DIAL_TIMEOUT)?;
    // No read deadline: the headset speaks when it has something to say, and
    // a reader that gave up after a timeout would tear down a working link
    // every time somebody stopped touching their headphones. The write
    // deadline is there because a write that blocks blocks the ENGINE.
    socket.set_write_timeout(DIAL_TIMEOUT)?;
    Ok((device, generation, socket))
}

/// The first connected device that advertises an MDR service, with the
/// services it advertises, newest first.
///
/// Via `bluetoothctl`, which is how everything in jayos's shell asks about
/// Bluetooth -- and which means the daemon needs no D-Bus client of its own.
fn find_headset() -> Result<(link::Device, Vec<link::Mdr>), link::LinkError> {
    let listing = sony_xm5::support::proc::output_within(
        "bluetoothctl",
        &["devices", "Connected"],
        INVENTORY_TIMEOUT,
    )
    .map_err(link::LinkError::Io)?;
    for device in link::connected_devices(&listing.stdout) {
        let mac = device.mac.to_string();
        let Ok(info) =
            sony_xm5::support::proc::output_within("bluetoothctl", &["info", &mac], INVENTORY_TIMEOUT)
        else {
            continue;
        };
        let services = link::mdr_services(&info.stdout);
        if !services.is_empty() {
            return Ok((device, services));
        }
    }
    Err(link::LinkError::NoDevice)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_lets_the_engine_answer_before_the_socket_gives_up() {
        // The three deadlines a client's command passes through, innermost
        // first: the engine's acknowledgement wait, this socket's dispatch
        // wait, qs-sony's per-operation timeout. Each must be strictly
        // inside the next, or the answer from the inner one reaches nobody.
        // The socket's timer starts before the engine stamps the frame, so
        // equal is not enough for the first pair.
        let engine = Duration::from_millis(Limits::default().ack_wait);
        assert!(engine < DISPATCH_WAIT, "the engine's answer would arrive after the socket gave up");
        assert!(DISPATCH_WAIT < Duration::from_secs(3), "the socket's answer would arrive after qs-sony gave up");
    }
}
