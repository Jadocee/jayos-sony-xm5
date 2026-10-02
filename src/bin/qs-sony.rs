//! Sony headphone status and control, for the shell.
//!
//!     qs-sony status              the daemon's state, plus what we derive
//!     qs-sony devices             connected headsets, and what to offer for each
//!     qs-sony send <command...>   one command over the daemon's socket
//!     qs-sony available           exit 0 if the daemon is reachable
//!     qs-sony replay <dir> [--peer=NAME]   the document a capture produces
//!
//! A thin wrapper over the headphone daemon: something that speaks the Sony |
//! Sound Connect protocol, publishes JSON state and accepts commands on a
//! private Unix socket.
//!
//! WHICH DAEMON, AND WHY THIS DOES NOT CARE
//!
//! `qs-sonyd`, in this tree, or sony-headphones-linux, the C++ project it
//! replaces. They publish the same document at the same path and take the same
//! socket commands, deliberately -- so nothing here asks which one is running,
//! and someone who already installed the C++ one keeps working.
//!
//! WHY A WRAPPER AND NOT DIRECT ACCESS
//!
//! Two reasons. The state file has a schema version and the spec says a reader
//! MUST reject a newer one rather than guess -- that check belongs in one
//! place, not in every caller. And the socket is a line protocol with a size
//! limit and an `ok`/`error` reply, which is more than a QML Process should be
//! reimplementing.
//!
//! WHAT THIS DOES NOT DO
//!
//! It does not start the daemon. A shell that opened a Bluetooth connection to
//! somebody's headphones because a panel was looked at is not a shell.
//!
//! No daemon is running on most machines, which is the ordinary case and not a
//! failure: every verb answers "not available" with a reason, and nothing here
//! waits on it without a deadline.

use sony_xm5::support::{cli::Args, emit, proc};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
#[cfg(test)]
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The schema this understands, and the longest command line the daemon
/// accepts. The spec is explicit that a reader must refuse a newer schema
/// rather than guess at it -- a field that changed meaning is worse than a
/// field that is missing.
///
/// Imported, not restated. Both were literal copies here (`= 1`, `= 4096`)
/// beside the library's own, which is two numbers for one fact; now that the
/// reader and the writer ship in one package there is no reason for the copy.
use sony_xm5::sony::command::MAX_COMMAND;
use sony_xm5::sony::state::SCHEMA_VERSION;
use sony_xm5::sony::SONY_PREFIXES;

/// Per socket operation, as the Python's `settimeout` was.
const TIMEOUT: Duration = Duration::from_secs(3);

/// For the device inventory subprocess. Longer than the socket timeout because
/// it talks to upower, which is slow to answer on a machine that has just
/// resumed.
const INVENTORY_TIMEOUT: Duration = Duration::from_secs(15);

/// The project that provides the daemon. Named, not fetched: this is a
/// separate codebase and the shell should say where it is rather than pull it
/// silently.
/// The daemon in this package, which is the one to prefer when both are present.
const NATIVE_DAEMON: &str = "qs-sonyd";

/// jayos's device inventory, which this reads and does not own. Its output is
/// part of the contract between the two repositories -- see
/// `contract/qs-devices.example.json`.
const INVENTORY: &str = "qs-devices";

const DAEMON_PROJECT: &str = "sony-headphones-linux";
/// The project this contract comes from.
///
/// This said `ykhmelevskiy/sony-headphones-linux`, which is not where the
/// daemon lives -- and the settings page puts this string behind the button
/// that offers to install it, so the one action the page could take sent people
/// to a repository that is not the one the rest of this file was written
/// against. Worse, the shape it reads was not that project's either: the state
/// document nests `name` under `device` and reports a battery as an object, and
/// this helper was looking for both at the top level.
const DAEMON_URL: &str = "https://github.com/VyomJain6904/sony-headphones-linux";

// ── output shapes ────────────────────────────────────────────────────────────

/// The few values the bar and the overview tile need.
///
/// Kept separate from the raw state so a schema addition does not have to be
/// mirrored here -- the page reads `raw` and shows whatever is in it.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Summary {
    connected: bool,
    name: String,
    battery: Option<i64>,
    battery_left: Option<i64>,
    battery_right: Option<i64>,
    battery_case: Option<i64>,
    noise_mode: String,
}

/// The `status` envelope.
///
/// Generic over the summary so it can be a real [`Summary`] when there is one
/// and an empty object when there is not. Both without going through `Value`:
/// serde_json's map is a BTreeMap here, so anything routed through `Value`
/// comes out in alphabetical order, and the Python emitted the summary in the
/// order it built it. Nothing iterates those keys today, but a struct costs
/// nothing and keeps the output the same shape it has always been.
///
/// `raw` is the exception and cannot be helped: it is the daemon's own object,
/// so it is reordered alphabetically on the way through. Only `status` prints
/// it, and its readers -- the doctor check, and the settings page's field
/// lookups -- read by key.
#[derive(Debug, Serialize)]
struct Status<S: Serialize> {
    available: bool,
    error: String,
    /// Which kind of "no state", as a stable key. Absent when there IS state.
    ///
    /// The settings page decides its colour from this and never from `error`:
    /// no daemon running is not a fault and is drawn grey, while a state file
    /// this wrapper cannot read is a fault and is drawn as one. A reader that
    /// took that from the sentence would reclassify the day it is reworded.
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    summary: S,
    /// Absent, not null, when there is no state to report -- the Python built
    /// the unavailable envelope without the key at all.
    #[serde(skip_serializing_if = "Option::is_none")]
    raw: Option<Value>,
}

/// The `summary` of an unavailable status: `{}`, and nothing else.
///
/// A unit struct rather than `json!({})` so it serialises as an empty object
/// without the map that would reorder a populated one.
#[derive(Debug, Serialize)]
struct NoSummary {}

/// The `send` reply. A struct rather than `json!`, for the same reason
/// [`Status`] is one: a `Value` is a sorted map and would print `message`
/// before `ok`.
#[derive(Debug, Serialize)]
struct Reply {
    ok: bool,
    message: String,
}

#[derive(Debug, Serialize, PartialEq)]
struct Entry {
    name: String,
    /// Passed through from the inventory exactly as it came, including a null
    /// for a device that has not reported a level.
    battery: Value,
    sony: bool,
    /// What the button should do, decided here so the page does not have to
    /// reason about it. See [`action`].
    action: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Devices {
    devices: Vec<Entry>,
    daemon_installed: bool,
    /// A daemon is up: it holds the socket lock, or answers on the socket.
    /// Apart from `daemon_reporting` because the two come apart whenever the
    /// headset is off -- the daemon keeps redialling and the document says
    /// `connected:false` -- and a page that read only the second offered
    /// "Start daemon" against a daemon already running.
    daemon_running: bool,
    daemon_reporting: bool,
    project: &'static str,
    url: &'static str,
    /// The argv that starts whichever daemon is installed, or empty.
    ///
    /// DECIDED HERE RATHER THAN IN THE PAGE. The page used to run `systemctl
    /// --user start sony-headphones` -- a unit that only the C++ project
    /// installs -- so the one action it offered did nothing on a machine
    /// running `qs-sonyd`, and did nothing quietly. Which daemon is present is
    /// a question this program already answers for `daemon_installed`, and
    /// answering it twice in two languages is how the two got out of step.
    /// The page runs this only for the C++ project's unit; when `project` is
    /// `qs-sonyd` it starts the daemon through jayos's `sony-daemon.sh`
    /// instead, so the daemon's stderr lands in the log (see jayos's
    /// `DevicesPage.startSony`).
    start: Vec<String>,
}

// ── the daemon's files ───────────────────────────────────────────────────────

/// The daemon's own answers, so the reader and the daemon cannot disagree.
///
/// This used to build the two paths itself, and its socket fallback without
/// `XDG_RUNTIME_DIR` was `/tmp/sony-headphones/daemon.sock` -- while both
/// daemons bind `/tmp/sony-headphones-<uid>/daemon.sock` (qs-sonyd through
/// `sony::runtime_dir`, the C++ one through its `runtimeDirectory`). So
/// `qs-sony send` from a shell with no session said "the daemon socket is
/// not there" against a running daemon.
fn state_path() -> PathBuf {
    sony_xm5::sony::state_file()
}

fn socket_path() -> PathBuf {
    sony_xm5::sony::socket_path()
}

/// jayos's helpers' directory: `QS_BIN_DIR`, then `QS_BIN`, then
/// `~/.local/bin` -- the precedence jayos's paths.sh gives them
/// (`QS_BIN="${QS_BIN_DIR:-${QS_BIN:-$HOME/.local/bin}}"`).
///
/// ONLY FOR `qs-devices`, which is jayos's and is installed there. It used to
/// find `qs-sonyd` too, when both were built from jayos's workspace into that
/// directory. They are not any more: this package installs both of its
/// binaries side by side, and a `qs-sonyd` left in `~/.local/bin` by a jayos
/// build from before the move is stale -- found first, it would be the daemon
/// `devices` offered to start. See [`native_daemon`].
fn bin_dir() -> PathBuf {
    ["QS_BIN_DIR", "QS_BIN"]
        .iter()
        .find_map(|k| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from))
        .unwrap_or_else(|| sony_xm5::sony::home().join(".local/bin"))
}

/// Where `qs-sonyd` is, as an absolute path, or `None`.
///
/// BESIDE THIS PROGRAM FIRST, then `PATH`. The two are built and installed
/// together -- `/usr/bin` from the package, `target/<profile>` from a checkout
/// -- so the daemon this client was built to talk to is the one in its own
/// directory. That holds even in a session started by a display manager, whose
/// `PATH` may not be the login shell's: the lookup does not depend on `PATH`
/// at all unless the sibling is missing. Absolute so that what the page hands
/// to `execDetached` does not depend on quickshell's `PATH` either.
///
/// NOT jayos's helpers' directory, which is where this looked while both were
/// built from jayos's workspace. A daemon still sitting there is a stale build
/// from before the move, and preferring it is how a page would start an old
/// daemon beside a current client.
fn native_daemon() -> Option<PathBuf> {
    let here = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
    native_daemon_beside(here.as_deref())
}

/// [`native_daemon`] with the program's directory passed in, so a test can
/// point it somewhere without moving the test binary.
fn native_daemon_beside(dir: Option<&Path>) -> Option<PathBuf> {
    dir.map(|d| d.join(NATIVE_DAEMON)).filter(|at| executable(at)).or_else(|| proc::which(NATIVE_DAEMON))
}

/// Where jayos's `qs-devices` is: its helpers' directory, then `PATH`.
///
/// It was spawned by bare name, which is a `PATH` search -- the very lookup
/// [`native_daemon`]'s comment explains fails in a session a display manager
/// started, where `~/.local/bin` is not on `PATH`. So `qs-sony devices` got an
/// empty inventory on exactly the machines the shell runs on, and the Devices
/// page drew no headsets. jayos installs it where [`bin_dir`] points.
fn inventory_program() -> Option<PathBuf> {
    Some(bin_dir().join(INVENTORY)).filter(|at| executable(at)).or_else(|| proc::which(INVENTORY))
}

/// The one test both lookups apply: a regular file someone may execute.
fn executable(at: &Path) -> bool {
    std::fs::metadata(at).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

/// Whether anything on this machine can serve the state file.
///
/// The names it probed -- `sony-headphones` and `sony-headphones-daemon` -- are
/// not installed by any version of that project, which ships
/// `sony-headphonesd` and `sony-headphonesctl`. So this answered false on a
/// machine where the daemon was installed and running, `action()` never reached
/// its `configure` or `start` branches, and the page offered to install
/// something that was already there.
///
/// `qs-sonyd` is this tree's own daemon and is checked first: where both exist,
/// the local one is the one being talked to.
fn daemon_installed() -> bool {
    native_daemon().is_some() || proc::have("sony-headphonesd") || proc::have("sony-headphonesctl")
}

/// Whether a daemon is alive behind the state file.
///
/// The test `qs-sonyd` itself makes before refusing to start a second copy:
/// `serve::bind` takes `flock(LOCK_EX|LOCK_NB)` on `daemon.lock` beside the
/// socket and holds it for the daemon's life, and the kernel releases it
/// however the process ends -- so a lock that can be taken is a daemon that
/// is gone, whatever its last document says. The C++ daemon holds no such
/// lock, so when the lock is free the socket is tried as well: a daemon that
/// is listening accepts the connection, and the socket file a dead one left
/// behind refuses it.
fn daemon_running() -> bool {
    let lock = sony_xm5::sony::runtime_dir().join("daemon.lock");
    if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&lock) {
        // SAFETY: flock's only precondition is a valid descriptor, which
        // `file` owns for the duration of the call.
        let taken = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0
            && std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock;
        if taken {
            return true;
        }
    }
    UnixStream::connect(socket_path()).is_ok()
}

/// Why there is no state to report.
///
/// A KEY AS WELL AS A SENTENCE, and the key is the point. The sentence is for a
/// person; a reader that decided from it would silently reclassify the day
/// somebody rewords one. The settings page needs the three apart -- nothing
/// running is not a fault and must be grey, while a file this wrapper cannot
/// read is a fault and must not be -- so the distinction the comment below has
/// always claimed is now carried in the structure rather than only in prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Missing {
    /// No state file. Nothing has started a daemon, which on most machines is
    /// the ordinary case rather than a fault.
    Absent,
    /// A file that is there and is not a state document.
    Unreadable,
    /// A document written by a daemon newer than this wrapper.
    Newer,
    /// A readable document with no daemon behind it. `qs-sonyd` leaves its
    /// last document in place on purpose (`qs-sonyd once` exists to do
    /// exactly that) and installs no signal handler, so a daemon killed by
    /// anything but `sony-daemon-process stop` (which jayos's
    /// `sony-daemon.sh stop` calls) leaves a file that may still say
    /// `connected:true`; read alone, that drew a headset nobody was talking
    /// to and told the doctor a daemon was "reporting".
    Stale,
}

impl Missing {
    fn key(self) -> &'static str {
        match self {
            Missing::Absent => "absent",
            Missing::Unreadable => "unreadable",
            Missing::Newer => "newer",
            Missing::Stale => "stale",
        }
    }
}

/// The daemon's state, or a reason it is not available.
///
/// Every failure is distinguished, because they want different responses: no
/// daemon is a service to start, a file that will not parse is one to look at,
/// and a newer schema is this wrapper needing an update.
fn read_state() -> Result<Value, String> {
    read_state_reason().map_err(|(_, message)| message)
}

fn read_state_reason() -> Result<Value, (Missing, String)> {
    let path = state_path();
    if !path.is_file() {
        return Err((Missing::Absent, "no state file; the daemon is not running".to_string()));
    }

    let data = match std::fs::read_to_string(&path) {
        Ok(text) => interpret_state_reason(&text)?,
        Err(e) => return Err((Missing::Unreadable, format!("unreadable state file: {e}"))),
    };
    // After the shape checks, not before: a file this wrapper cannot read is
    // that reason whether or not a daemon is behind it.
    if !daemon_running() {
        return Err((
            Missing::Stale,
            "state file is left over from a daemon that is not running".to_string(),
        ));
    }
    Ok(data)
}

/// The version and shape checks, separated from the file so they can be tested
/// without one.
#[cfg(test)]
fn interpret_state(text: &str) -> Result<Value, String> {
    interpret_state_reason(text).map_err(|(_, message)| message)
}

fn interpret_state_reason(text: &str) -> Result<Value, (Missing, String)> {
    let data: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        // The Python caught the read error and the parse error together and
        // said the same thing about both, because the answer is the same: the
        // file the daemon writes is not readable, so there is no state.
        Err(e) => return Err((Missing::Unreadable, format!("unreadable state file: {e}"))),
    };

    if !data.is_object() {
        return Err((Missing::Unreadable, "state file is not an object".to_string()));
    }

    let version = match data.get("schema_version") {
        None | Some(Value::Null) => {
            return Err((Missing::Unreadable, "state file has no schema_version".to_string()))
        }
        Some(v) => v,
    };

    match version.as_f64() {
        Some(n) if n > SCHEMA_VERSION as f64 => {
            // Refused, not guessed. A v2 file may have redefined a field this
            // understands, and rendering it as if it were v1 shows a confident
            // wrong answer.
            Err((
                Missing::Newer,
                format!(
                    "state schema v{version} is newer than this shell understands \
                     (v{SCHEMA_VERSION})"
                ),
            ))
        }
        Some(_) => Ok(data),
        // A version that is not a number is the same class of problem: this
        // cannot tell whether it is older or newer, so it refuses rather than
        // reading the file anyway. (The Python raised a TypeError here and
        // printed nothing at all.)
        None => Err((
            Missing::Unreadable,
            format!("state file has an unreadable schema_version: {version}"),
        )),
    }
}

// ── deriving the answer ──────────────────────────────────────────────────────

/// Python's notion of truth, which the original leaned on in three places.
///
/// It matters because `connected` is read two different ways deliberately:
/// loosely here, for "is the daemon reporting a headset at all", and strictly
/// in [`summarise`], where only a real `true` counts.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// What `str()` did to a value out of the state file.
///
/// Only a string is realistic here -- the daemon writes names and modes -- but
/// a malformed file must not lose the field, so anything else is rendered as
/// the JSON it is rather than dropped.
fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A truthy field, or nothing. `data.get(k) or ...` in the original.
fn field<'a>(data: &'a Value, key: &str) -> Option<&'a Value> {
    data.get(key).filter(|v| truthy(Some(v)))
}

/// Whether a device name looks like a Sony headset.
///
/// Substring rather than a true prefix match, as the Python was: the vendor
/// string is often in front of the model ("Sony WH-1000XM4", "JBL WI-..." is
/// not a thing), and a name that carries the model anywhere in it is the one
/// the daemon can talk to.
fn is_sony(name: &str) -> bool {
    let n = name.trim();
    if n.is_empty() {
        return false;
    }
    let lower = n.to_lowercase();
    // "Sony" in the vendor field, or a model prefix in the name.
    lower.contains("sony") || SONY_PREFIXES.iter().any(|p| lower.contains(p))
}

/// What the settings page's button should offer for a device.
///
///   configure  the daemon is running (headset or not; the Headphones page
///              says which)
///   start      installed but not running
///   install    a Sony headset with no daemon
///   none       not a Sony device; nothing to offer
fn action(sony: bool, installed: bool, running: bool) -> &'static str {
    if sony && installed && running {
        "configure"
    } else if sony && installed {
        "start"
    } else if sony {
        "install"
    } else {
        "none"
    }
}

fn summarise(data: &Value) -> Summary {
    let battery = field(data, "battery");

    // A part is `{"present":bool,"level":int,"charging":string}`, and a level of
    // -1 means the part is not there.
    //
    // Both halves were wrong here. The flat form (`"left": 80`) is accepted
    // because an older reader may still see one, but the daemon has never
    // written it -- and `present` was never read at all, so a WH-1000XM5, which
    // reports a main battery and no sides or case, published -1 as a charge
    // level for three of the four. The bar then showed "-1%".
    //
    // `present` decides; the -1 is refused as well, because a hand-written or
    // half-populated file can carry one without the flag.
    let level = |key: &str| -> Option<i64> {
        let v = battery?.get(key)?;
        if v.is_object() {
            if v.get("present") == Some(&Value::Bool(false)) {
                return None;
            }
            let level = v.get("level")?.as_i64()?;
            return (level >= 0).then_some(level);
        }
        let level = v.as_i64()?;
        (level >= 0).then_some(level)
    };

    let (main, left, right) = (level("main"), level("left"), level("right"));
    // The lowest of the buds, because that is the one that ends the session.
    // A single-battery model reports `main` and no sides.
    let battery_level = [main, left, right].into_iter().flatten().min();

    let noise = field(data, "noise");
    let mode = match noise {
        Some(v) if v.is_object() => v.get("mode"),
        other => other,
    };

    Summary {
        // Strictly `true`, not merely truthy: this drives the "connected" dot,
        // and a state file that reports the field as a string or a count
        // should not light it.
        connected: data.get("connected") == Some(&Value::Bool(true)),
        // Under `device`, which is where the daemon writes them.
        //
        // This read them at the TOP level, where nothing has ever put them --
        // this helper was written against a different project -- so the name
        // always fell through to the "Headphones" placeholder however much the
        // daemon reported. The flat spellings are still tried afterwards so an
        // older or hand-written file is not made worse.
        name: field(data, "device")
            .and_then(|d| d.get("name").filter(|v| truthy(Some(v))))
            .or_else(|| field(data, "device").and_then(|d| d.get("model").filter(|v| truthy(Some(v)))))
            .or_else(|| field(data, "name"))
            .or_else(|| field(data, "model"))
            .map(text_of)
            .unwrap_or_else(|| "Headphones".to_string())
            // Characters, not bytes: a name is user-set and routinely has
            // emoji in it, and cutting one in half would produce invalid text
            // where the Python produced a short name.
            .chars()
            .take(48)
            .collect(),
        battery: battery_level,
        battery_left: left,
        battery_right: right,
        battery_case: level("case"),
        noise_mode: mode
            .filter(|v| truthy(Some(v)))
            .map(text_of)
            .unwrap_or_else(|| "unknown".to_string()),
    }
}

/// The headsets in a device inventory, with what can be offered for each.
fn collect(inventory: &Value, installed: bool, running: bool) -> Vec<Entry> {
    let mut found: Vec<Entry> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let list = |key: &str| -> Vec<Value> {
        inventory.get(key).and_then(Value::as_array).cloned().unwrap_or_default()
    };

    for d in list("devices") {
        let name = d.get("name").and_then(Value::as_str).unwrap_or("");
        // Headsets only. A mouse has a battery too and does not belong on a
        // headphones page. A device with no type at all is skipped as well:
        // the empty string is what the sysfs source reports for "unknown",
        // and a missing key is a source that did not answer the question.
        let kind = d.get("type").and_then(Value::as_str);
        if !matches!(kind, Some("headset" | "headphones" | "")) || name.is_empty() {
            continue;
        }
        if !seen.insert(name.to_lowercase()) {
            continue;
        }
        let sony = is_sony(name);
        found.push(Entry {
            name: name.to_string(),
            battery: d.get("percent").cloned().unwrap_or(Value::Null),
            sony,
            action: action(sony, installed, running),
        });
    }

    // Bluetooth cards with no battery report still count: a headset that has
    // not reported a level yet is still connected, and leaving it out would
    // show an empty page while music was playing.
    for c in list("cards") {
        let name = c.get("name").and_then(Value::as_str).unwrap_or("");
        if name.is_empty() || !seen.insert(name.to_lowercase()) {
            continue;
        }
        let sony = is_sony(name);
        found.push(Entry {
            name: name.to_string(),
            battery: Value::Null,
            sony,
            action: action(sony, installed, running),
        });
    }

    found
}

// ── the device inventory ─────────────────────────────────────────────────────

/// Every peripheral the machine knows about.
///
/// From the devices helper rather than a second implementation: it already
/// merges upower and sysfs and reports a type, and two detectors for one
/// question is two things to keep in step.
///
/// The Rust port wins when it is installed and the Python script is the
/// fallback, because the two are being replaced one at a time and this has to
/// work in a tree where only one of them is there. Either way a failure is an
/// empty inventory, which renders as a page with no headsets rather than an
/// error -- the same silence the Python's bare `except` produced.
fn inventory() -> Value {
    // qs-devices or nothing.
    //
    // The other arm ran `python3 scripts/hypr/devices.py`, which does not exist
    // anywhere in jayos and could not have since the port -- so on a
    // machine without qs-devices this spawned a missing interpreter against a
    // missing script and returned the same empty inventory it would have
    // returned by not trying. There is no Python here to fall back to.
    let text = inventory_program()
        .and_then(|program| capture(&program, &["list".to_string()], INVENTORY_TIMEOUT));

    text.and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

/// A subprocess's stdout, or nothing if it failed or ran past its deadline.
///
/// `proc::output` has no deadline, and this one needs one: the inventory shells
/// out to upower, and this runs on the settings page's open path where a child
/// that never exits is a page that never fills in.
///
/// The pipe is drained on another thread so that a child which stops writing
/// without exiting can still be given up on -- reading it here and then waiting
/// would block in the read with no way out.
fn capture(program: impl AsRef<std::ffi::OsStr>, args: &[String], limit: Duration) -> Option<String> {
    let deadline = Instant::now() + limit;

    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        // Discarded rather than captured: the Python read only stdout, and a
        // child whose stderr fills a pipe nobody reads would wedge.
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let Some(mut out) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        // Lossy, like proc::output: a device named from someone's phone is not
        // always valid UTF-8, and that should not lose the whole inventory.
        let mut bytes = Vec::new();
        let _ = out.read_to_end(&mut bytes);
        text.push_str(&String::from_utf8_lossy(&bytes));
        let _ = tx.send(text);
    });

    match rx.recv_timeout(limit) {
        Ok(text) => {
            reap(&mut child, deadline);
            Some(text)
        }
        Err(_) => {
            // The reader thread is left behind; it dies with the process,
            // which is about to exit anyway.
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    }
}

/// Collect a child that has closed its output, without waiting past the
/// deadline for one that has closed it and not exited.
fn reap(child: &mut std::process::Child, deadline: Instant) {
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

// ── the socket ───────────────────────────────────────────────────────────────

/// One command line to the daemon. Returns (ok, message).
fn send(command: &[String]) -> (bool, String) {
    let sock = socket_path();
    if !sock.exists() {
        return (false, "the daemon socket is not there".to_string());
    }

    let line = match prepare_line(command) {
        Ok(line) => line,
        Err(why) => return (false, why.to_string()),
    };

    match talk(&sock, &line) {
        Ok(reply) => interpret_reply(&reply),
        Err(e) => (false, format!("could not reach the daemon: {}", describe(&e))),
    }
}

/// What went wrong, in the words the Python used.
///
/// A read that runs past `SO_RCVTIMEO` surfaces as EAGAIN -- "Resource
/// temporarily unavailable" -- which describes the syscall rather than the
/// situation. Python's `socket.timeout` said "timed out", and that is the one
/// a person reading the message needs.
fn describe(e: &std::io::Error) -> String {
    use std::io::ErrorKind::{TimedOut, WouldBlock};
    match e.kind() {
        TimedOut | WouldBlock => "timed out".to_string(),
        _ => e.to_string(),
    }
}

/// The command as one line, or why it cannot be sent.
fn prepare_line(command: &[String]) -> Result<String, &'static str> {
    let line = command.join(" ").trim().to_string();
    if line.is_empty() {
        return Err("empty command");
    }
    // Bounded before sending rather than after: the daemon rejects an
    // over-long line, and finding that out over the wire is slower and less
    // clear than not sending it. Bytes, not characters, because that is what
    // the limit is measured in.
    if line.len() > MAX_COMMAND {
        return Err("command too long");
    }
    Ok(line)
}

fn talk(sock: &Path, line: &str) -> std::io::Result<String> {
    let stream = connect(sock, TIMEOUT)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;

    let mut stream = stream;
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\n")?;

    // One read of one reply, as the protocol is one command per connection.
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf[..n]).trim().to_string())
}

/// Connect, with a deadline.
///
/// `UnixStream` has no `connect_timeout`, and a Unix connect is not always
/// instant: it blocks when the listener's accept backlog is full, which is
/// exactly the wedged-daemon case worth bounding. The Python got this for free
/// from `settimeout` before `connect`.
///
/// The connect runs on its own thread so the wait can be abandoned. A thread
/// stuck in connect costs nothing here -- this process answers and exits.
fn connect(sock: &Path, limit: Duration) -> std::io::Result<UnixStream> {
    let path = sock.to_path_buf();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(UnixStream::connect(&path));
    });

    match rx.recv_timeout(limit) {
        Ok(result) => result,
        Err(_) => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "timed out connecting to the daemon socket",
        )),
    }
}

/// The daemon's `ok`/`error` reply, split into a result and what it said.
fn interpret_reply(reply: &str) -> (bool, String) {
    if let Some(rest) = reply.strip_prefix("ok") {
        return (true, rest.trim().to_string());
    }
    if let Some(rest) = reply.strip_prefix("error") {
        return (false, rest.trim().to_string());
    }
    // Anything else is passed through whole rather than guessed at, so the
    // page shows what the daemon actually said.
    (false, reply.to_string())
}

// ── verbs ────────────────────────────────────────────────────────────────────

fn status() -> ExitCode {
    match read_state_reason() {
        Err((missing, err)) => emit::json(&Status {
            available: false,
            error: err,
            reason: Some(missing.key()),
            // An object, not null: the page reads `summary.battery` without
            // checking, and a null there is a binding error per field.
            summary: NoSummary {},
            raw: None,
        }),
        Ok(data) => emit::json(&Status {
            available: true,
            error: String::new(),
            reason: None,
            summary: summarise(&data),
            raw: Some(data),
        }),
    }
}

fn devices() -> ExitCode {
    let native = native_daemon();
    let installed = daemon_installed();
    // The button says "Settings" or "Start daemon" on whether a daemon is UP,
    // not on whether its document names a headset: a running daemon with the
    // headphones in their case says `connected:false`, and a "Start daemon"
    // for it would start a second copy that refuses the socket lock and exits
    // on a stderr nobody reads.
    let running = daemon_running();
    // Truthy, not strictly true: anything the daemon says about a connection
    // counts as reporting one.
    let reporting = running && read_state().map(|s| truthy(s.get("connected"))).unwrap_or(false);

    // This package's daemon, when it is there, is not something to go and
    // install: it arrived in the same package as this program. Offering a
    // project page for it would send someone to a repository to fetch what
    // they already have.
    let (project, url) =
        if native.is_some() { (NATIVE_DAEMON, "") } else { (DAEMON_PROJECT, DAEMON_URL) };

    emit::json(&Devices {
        devices: collect(&inventory(), installed, running),
        daemon_installed: installed,
        daemon_running: running,
        daemon_reporting: reporting,
        project,
        url,
        start: start_command(native.as_deref(), installed),
    })
}

/// How to start the daemon that is actually here.
///
/// This package's one is its absolute path -- a foreground process the page
/// detaches, which is how every other daemon in jayos's shell is started. The
/// C++ one is a systemd user unit, which is how ITS project ships it. Neither
/// installed means there is nothing to offer, and an empty list is what the
/// page checks rather than a command that would fail.
fn start_command(native: Option<&Path>, installed: bool) -> Vec<String> {
    if let Some(path) = native {
        vec![path.to_string_lossy().into_owned()]
    } else if installed {
        ["systemctl", "--user", "start", "sony-headphones"].iter().map(|s| s.to_string()).collect()
    } else {
        Vec::new()
    }
}

fn available() -> ExitCode {
    let (ok, error) = match read_state() {
        Ok(data) => (truthy(data.get("connected")), String::new()),
        Err(err) => (false, err),
    };

    // The Python printed nothing here and answered with its exit status alone.
    // The status is what callers use and it is unchanged, but stdout is JSON
    // in every one of these programs, so there is something to read for a
    // caller that reads it.
    let _ = emit::json(&json!({ "available": ok, "error": error }));
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn main() -> ExitCode {
    let args = Args::from_env();

    match args.command_or("status") {
        "status" => status(),
        "devices" => devices(),
        "available" => available(),
        "replay" => replay(&args),
        "send" => {
            // Taken raw rather than from `Args`, because everything after the
            // verb belongs to the daemon's protocol and not to ours: a flag it
            // defines would otherwise be parsed as one of ours and dropped.
            // The verb is argv[1], as it was in the Python.
            let command: Vec<String> = std::env::args_os()
                .skip(2)
                .map(|a| a.to_string_lossy().into_owned())
                .collect();

            if command.is_empty() {
                return emit::failure("send needs a command");
            }

            let (ok, message) = send(&command);
            let _ = emit::json(&Reply { ok, message });
            if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        other => emit::unknown_command(
            "qs-sony",
            other,
            &["status", "devices", "send", "available", "replay"],
        ),
    }
}

/// The document a daemon would publish after receiving a capture.
///
/// For a person with a capture directory -- a corpus here, or what
/// `qs-sonyd --record` wrote -- who wants to see what the daemon makes of it
/// without a headset in reach. The same function produces the contract's
/// `state.example.json`; see `sony::replay` for why the playback is
/// deterministic and why only received frames are played.
///
/// Printed as the daemon writes it: one compact line. `| jq .` is the way to
/// read it, and printing it differently here would make it a different
/// document from the one the shell reads.
fn replay(args: &Args) -> ExitCode {
    if let Some(code) = args.reject_unknown(&["peer"]) {
        return code;
    }
    let Some(dir) = args.positional(0) else {
        return emit::failure("replay needs a capture directory");
    };
    let frames = match sony_xm5::sony::replay::received_frames(Path::new(dir)) {
        Ok(frames) => frames,
        Err(e) => return emit::failure(&format!("cannot read {dir}: {e}")),
    };
    // A directory with no received frame in it is a wrong path far more often
    // than a capture of nothing, and a document of defaults would look like an
    // answer.
    if frames.is_empty() {
        return emit::failure(&format!("{dir} holds no received frame (*-rx.*.bin)"));
    }
    let peer = args.value("peer").unwrap_or("WF-1000XM5");
    print!("{}", sony_xm5::sony::replay::document(&frames, peer, sony_xm5::sony::DAEMON_VERSION));
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sony_names_are_matched_by_vendor_or_model() {
        assert!(is_sony("Sony WH-1000XM4"));
        assert!(is_sony("WH-1000XM5"));
        assert!(is_sony("wf-1000xm4"));
        assert!(is_sony("LinkBuds S"));
        assert!(is_sony("MDR-ZX110"));
        // The vendor field alone, in any case.
        assert!(is_sony("SONY Corporation Headset"));
        // The two lines with no model code, from the C++ daemon's list.
        assert!(is_sony("ULT WEAR"));
        assert!(is_sony("INZONE Buds"));

        assert!(!is_sony("Logitech MX Master 3S"));
        assert!(!is_sony("Jabra Elite 75t"));
        assert!(!is_sony(""));
        assert!(!is_sony("   "));
    }

    #[test]
    fn the_offer_depends_on_the_daemon_and_the_device() {
        assert_eq!(action(true, true, true), "configure");
        assert_eq!(action(true, true, false), "start");
        assert_eq!(action(true, false, false), "install");
        // Installed but not a Sony device is still nothing to offer.
        assert_eq!(action(false, true, true), "none");
        assert_eq!(action(false, false, false), "none");
    }

    #[test]
    fn a_missing_or_older_schema_is_read_and_a_newer_one_is_refused() {
        let ok = interpret_state(r#"{"schema_version": 1, "connected": true}"#);
        assert!(ok.is_ok());

        assert_eq!(
            interpret_state("not json").unwrap_err().split(':').next(),
            Some("unreadable state file")
        );
        assert_eq!(
            interpret_state("[1, 2]").unwrap_err(),
            "state file is not an object"
        );
        assert_eq!(
            interpret_state(r#"{"connected": true}"#).unwrap_err(),
            "state file has no schema_version"
        );
        assert_eq!(
            interpret_state(r#"{"schema_version": null}"#).unwrap_err(),
            "state file has no schema_version"
        );
        assert_eq!(
            interpret_state(r#"{"schema_version": 2}"#).unwrap_err(),
            "state schema v2 is newer than this shell understands (v1)"
        );
        // A version this cannot compare is refused rather than read.
        assert!(interpret_state(r#"{"schema_version": "1"}"#)
            .unwrap_err()
            .starts_with("state file has an unreadable schema_version"));
    }

    fn summary_of(text: &str) -> Summary {
        let data: Value = serde_json::from_str(text).expect("test fixture is valid JSON");
        summarise(&data)
    }

    /// The shape the daemon ACTUALLY writes: name under `device`, each battery
    /// an object with `present`, absent parts carrying level -1.
    ///
    /// Every test below used the flat shape, which no version of this daemon
    /// has ever emitted -- so the suite was self-consistent and wrong, and it
    /// went on passing while the name fell through to a placeholder and a
    /// WH-1000XM5 reported -1%.
    #[test]
    fn the_summary_reads_the_document_the_daemon_writes() {
        let s = summary_of(
            r#"{"schema_version": 1, "connected": true,
                "device": {"name": "Jay's WH-1000XM5", "model": "WH-1000XM5"},
                "battery": {
                  "main":  {"present": true,  "level": 73, "charging": "no"},
                  "left":  {"present": false, "level": -1, "charging": "unknown"},
                  "right": {"present": false, "level": -1, "charging": "unknown"},
                  "case":  {"present": false, "level": -1, "charging": "unknown"}},
                "noise": {"mode": "anc", "ambient_level": 10}}"#,
        );
        assert_eq!(
            s,
            Summary {
                connected: true,
                name: "Jay's WH-1000XM5".to_string(),
                // The over-ear reports one battery. -1 is absence, not a charge.
                battery: Some(73),
                battery_left: None,
                battery_right: None,
                battery_case: None,
                noise_mode: "anc".to_string(),
            }
        );
    }

    #[test]
    fn an_absent_part_is_absent_however_it_says_so() {
        // `present: false` is the flag; the -1 is refused as well, because a
        // half-populated or hand-written file can carry one without the flag.
        let s = summary_of(
            r#"{"battery": {"main": {"present": true, "level": 40},
                            "left": {"present": false, "level": -1},
                            "right": {"level": -1},
                            "case": {"present": true, "level": 55}}}"#,
        );
        assert_eq!(s.battery, Some(40));
        assert_eq!(s.battery_left, None);
        assert_eq!(s.battery_right, None, "a bare -1 is not a level either");
        assert_eq!(s.battery_case, Some(55));
    }

    #[test]
    fn the_buds_still_report_the_lowest_of_the_two() {
        // A WF-1000XM5 reports left and right and no main; the lower one is
        // what ends the session, so it is what the bar shows.
        let s = summary_of(
            r#"{"device": {"name": "WF-1000XM5"},
                "battery": {"main":  {"present": false, "level": -1},
                            "left":  {"present": true,  "level": 70},
                            "right": {"present": true,  "level": 55},
                            "case":  {"present": true,  "level": 90}}}"#,
        );
        assert_eq!(s.battery, Some(55));
        assert_eq!(s.battery_left, Some(70));
        assert_eq!(s.battery_case, Some(90));
    }

    #[test]
    fn the_name_falls_back_to_the_model_and_then_to_a_placeholder() {
        assert_eq!(summary_of(r#"{"device": {"model": "WH-1000XM5"}}"#).name, "WH-1000XM5");
        assert_eq!(summary_of(r#"{"device": {"name": ""}}"#).name, "Headphones");
        assert_eq!(summary_of("{}").name, "Headphones");
        // The flat spellings are still read, so an older or hand-written file
        // is not made worse by the fix.
        assert_eq!(summary_of(r#"{"name": "Legacy"}"#).name, "Legacy");
    }

    #[test]
    fn a_flat_level_is_still_accepted() {
        // Not a shape the daemon writes, but a reader that refused it would
        // make a hand-written file worse rather than better.
        let s = summary_of(r#"{"battery": {"main": 42}}"#);
        assert_eq!(s.battery, Some(42));
        assert_eq!(s.battery_left, None);
    }

    #[test]
    fn only_whole_non_negative_numbers_are_levels() {
        let s = summary_of(
            r#"{"battery": {"left": {"present": true, "level": 80, "charging": "no"},
                            "right": {"present": true, "charging": "no"},
                            "main": 55.5}}"#,
        );
        assert_eq!(s.battery_left, Some(80));
        // An object with no level is no level.
        assert_eq!(s.battery_right, None);
        // A float is not a level.
        assert_eq!(s.battery, Some(80));
    }

    #[test]
    fn a_battery_that_is_not_an_object_reports_no_levels() {
        // The Python raised AttributeError here and printed no JSON at all.
        let s = summary_of(r#"{"battery": 55}"#);
        assert_eq!(s.battery, None);
        assert_eq!(s.battery_left, None);
    }

    #[test]
    fn connected_is_only_a_real_true() {
        assert!(summary_of(r#"{"connected": true}"#).connected);
        assert!(!summary_of(r#"{"connected": 1}"#).connected);
        assert!(!summary_of(r#"{"connected": "yes"}"#).connected);
        assert!(!summary_of("{}").connected);
    }

    #[test]
    fn the_name_falls_back_and_is_cut_to_forty_eight_characters() {
        assert_eq!(summary_of(r#"{"model": "WF-1000XM5"}"#).name, "WF-1000XM5");
        // An empty name is not a name.
        assert_eq!(summary_of(r#"{"name": "", "model": "WH-CH720N"}"#).name, "WH-CH720N");
        assert_eq!(summary_of("{}").name, "Headphones");

        let long = "x".repeat(60);
        assert_eq!(summary_of(&format!(r#"{{"name": "{long}"}}"#)).name.len(), 48);

        // Characters, not bytes: fifty emoji are 200 bytes and must not be cut
        // through one.
        let emoji = "\u{1f3a7}".repeat(50);
        let s = summary_of(&format!(r#"{{"name": "{emoji}"}}"#));
        assert_eq!(s.name.chars().count(), 48);
    }

    #[test]
    fn the_noise_mode_reads_either_shape_and_defaults() {
        assert_eq!(summary_of(r#"{"noise": {"mode": "off"}}"#).noise_mode, "off");
        // A bare string where an object was expected is still the mode.
        assert_eq!(summary_of(r#"{"noise": "ambient"}"#).noise_mode, "ambient");
        assert_eq!(summary_of(r#"{"noise": {}}"#).noise_mode, "unknown");
        assert_eq!(summary_of("{}").noise_mode, "unknown");
    }

    #[test]
    fn only_headsets_are_listed_and_names_are_deduplicated() {
        let inv = json!({
            "devices": [
                {"name": "Sony WH-1000XM4", "type": "headset", "percent": 80},
                {"name": "MX Master 3S", "type": "mouse", "percent": 50},
                {"name": "hid-ab-battery", "type": "", "percent": 30},
                {"name": "No Type At All", "percent": 30},
                {"name": "", "type": "headset", "percent": 10},
                {"name": "sony wh-1000xm4", "type": "headphones", "percent": 79}
            ],
            "cards": [
                {"name": "Sony WH-1000XM4"},
                {"name": "JBL Go"}
            ]
        });

        let found = collect(&inv, true, true);
        assert_eq!(
            found,
            vec![
                Entry {
                    name: "Sony WH-1000XM4".to_string(),
                    battery: json!(80),
                    sony: true,
                    action: "configure",
                },
                Entry {
                    name: "hid-ab-battery".to_string(),
                    battery: json!(30),
                    sony: false,
                    action: "none",
                },
                // The card with no battery report still counts.
                Entry {
                    name: "JBL Go".to_string(),
                    battery: Value::Null,
                    sony: false,
                    action: "none",
                },
            ]
        );
    }

    #[test]
    fn a_device_with_no_battery_report_carries_a_null() {
        let inv = json!({"devices": [{"name": "WF-1000XM4", "type": "headset"}]});
        let found = collect(&inv, false, false);
        assert_eq!(found[0].battery, Value::Null);
        assert_eq!(found[0].action, "install");
    }

    #[test]
    fn an_empty_inventory_is_an_empty_list_not_a_failure() {
        assert!(collect(&json!({}), true, true).is_empty());
        assert!(collect(&json!({"devices": null, "cards": null}), true, true).is_empty());
    }

    #[test]
    fn the_reply_is_split_on_ok_and_error() {
        assert_eq!(interpret_reply("ok"), (true, String::new()));
        assert_eq!(interpret_reply("ok anc enabled"), (true, "anc enabled".to_string()));
        assert_eq!(interpret_reply("error no device"), (false, "no device".to_string()));
        // Anything else comes back whole.
        assert_eq!(interpret_reply("what?"), (false, "what?".to_string()));
        // The original sliced two characters off anything starting with "ok",
        // which this keeps: the protocol has no other word beginning that way.
        assert_eq!(interpret_reply("okay then"), (true, "ay then".to_string()));
    }

    #[test]
    fn the_send_envelope_puts_ok_first() {
        let text = serde_json::to_string(&Reply {
            ok: true,
            message: "anc enabled".to_string(),
        })
        .expect("the envelope serialises");
        assert_eq!(text, r#"{"ok":true,"message":"anc enabled"}"#);
    }

    #[test]
    fn a_socket_deadline_reads_as_a_timeout_not_an_errno() {
        use std::io::{Error, ErrorKind};
        assert_eq!(describe(&Error::from(ErrorKind::WouldBlock)), "timed out");
        assert_eq!(describe(&Error::from(ErrorKind::TimedOut)), "timed out");
        // Everything else is passed through as the system described it.
        assert_eq!(
            describe(&Error::new(ErrorKind::NotFound, "no such file")),
            "no such file"
        );
    }

    #[test]
    fn a_command_is_joined_trimmed_and_bounded() {
        let words = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();

        assert_eq!(prepare_line(&words(&["noise", "anc"])), Ok("noise anc".to_string()));
        assert_eq!(prepare_line(&words(&[" ", ""])), Err("empty command"));
        assert_eq!(prepare_line(&[]), Err("empty command"));

        let long = vec!["x".repeat(MAX_COMMAND + 1)];
        assert_eq!(prepare_line(&long), Err("command too long"));
        // Exactly at the limit still goes.
        let edge = vec!["x".repeat(MAX_COMMAND)];
        assert!(prepare_line(&edge).is_ok());
    }

    #[test]
    fn truthiness_matches_the_python_it_replaces() {
        assert!(!truthy(None));
        assert!(!truthy(Some(&Value::Null)));
        assert!(!truthy(Some(&json!(false))));
        assert!(!truthy(Some(&json!(0))));
        assert!(!truthy(Some(&json!(""))));
        assert!(!truthy(Some(&json!({}))));
        assert!(!truthy(Some(&json!([]))));
        assert!(truthy(Some(&json!(true))));
        assert!(truthy(Some(&json!(1))));
        assert!(truthy(Some(&json!("off"))));
        assert!(truthy(Some(&json!({"mode": "anc"}))));
    }

    #[test]
    fn an_unavailable_status_still_carries_an_object_summary() {
        let text = serde_json::to_string(&Status {
            available: false,
            error: "no state file; the daemon is not running".to_string(),
            reason: Some(Missing::Absent.key()),
            summary: NoSummary {},
            raw: None,
        })
        .expect("the envelope serialises");

        // Key order and shape are the contract: the doctor greps this text and
        // the settings page reads the keys by name.
        assert_eq!(
            text,
            r#"{"available":false,"error":"no state file; the daemon is not running","reason":"absent","summary":{}}"#
        );
    }

    #[test]
    fn each_way_of_having_no_state_is_a_different_reason() {
        // WHY THIS IS STRUCTURE AND NOT PROSE. The settings page draws "no
        // daemon is running" grey -- it is the ordinary state of most machines
        // and not a fault -- and draws a state file it cannot read as a fault.
        // Both arrive here as `available:false` with a sentence, and a reader
        // that told them apart by the sentence would silently reclassify one
        // the day somebody rewords it.
        assert_eq!(interpret_state_reason("not json").expect_err("refused").0, Missing::Unreadable);
        assert_eq!(interpret_state_reason("[]").expect_err("refused").0, Missing::Unreadable);
        assert_eq!(interpret_state_reason("{}").expect_err("refused").0, Missing::Unreadable);
        assert_eq!(
            interpret_state_reason(r#"{"schema_version":"one"}"#).expect_err("refused").0,
            Missing::Unreadable
        );
        // A daemon newer than this wrapper is the one case that is neither
        // "nothing there" nor "corrupt": the file is fine and this program is
        // the thing that needs updating.
        assert_eq!(
            interpret_state_reason(r#"{"schema_version":2}"#).expect_err("refused").0,
            Missing::Newer
        );
        assert!(interpret_state_reason(r#"{"schema_version":1}"#).is_ok());

        // And the keys are what the page matches on, so they are pinned.
        assert_eq!(Missing::Absent.key(), "absent");
        assert_eq!(Missing::Unreadable.key(), "unreadable");
        assert_eq!(Missing::Newer.key(), "newer");
        assert_eq!(Missing::Stale.key(), "stale");
    }

    // Environment variables are process-global, so every test that sets one
    // holds this for its whole run and puts back what it found.
    //
    // AND EVERY TEST THAT SPAWNS A SUBPROCESS HOLDS IT TOO, which is not
    // obvious and is why this comment is long.
    //
    // `Command::spawn` walks the whole `environ` array to build the child's
    // copy of it. glibc's `setenv`/`unsetenv` reallocate and compact that same
    // array in place. So a test that only READS the environment -- by spawning
    // anything at all -- races a test that writes one, and the reader can walk
    // an array that is being moved out from under it. That is precisely why
    // `std::env::set_var` is `unsafe`; the hazard is the shared array, not the
    // particular variable.
    //
    // The symptom was `a_daemon_is_running_while_it_holds_the_lock_or_answers_the_socket`
    // failing roughly twice in eight `cargo test --workspace` runs and passing
    // in isolation every single time. The mechanism: a torn read makes
    // `XDG_RUNTIME_DIR` come back absent, and `sony/mod.rs:60-66` then falls
    // back to `/tmp/sony-headphones-<uid>` -- machine-global and shared by
    // every process for that uid. The test's lock probe and `UnixStream`
    // connect leave its private scratch directory and land on a path other
    // tests are using, so the four states it walks stop being its own.
    //
    // It was reported as "not root-caused, and not being called a flake" in
    // three pull requests before anyone read `capture`'s callers.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct Env {
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
        _held: std::sync::MutexGuard<'static, ()>,
    }

    impl Env {
        fn take(keys: &[&'static str]) -> Env {
            let held = ENV.lock().unwrap_or_else(|e| e.into_inner());
            Env { saved: keys.iter().map(|k| (*k, std::env::var_os(k))).collect(), _held: held }
        }
        /// Take the lock without saving or changing anything.
        ///
        /// For a test that spawns a subprocess: it writes no variable, but
        /// `Command::spawn` reads the whole environment block, so it is a
        /// reader that has to be serialised against the writers. See the note
        /// on `ENV`.
        fn hold() -> Env {
            Env::take(&[])
        }

        fn set(&self, key: &str, value: Option<&Path>) {
            // SAFETY: `ENV` is held for the whole lifetime of this `Env`, and
            // every test in this module that either writes the environment or
            // spawns a subprocess (which reads all of it) holds it too -- so
            // no thread can be walking `environ` while this call reallocates
            // it. An earlier version of this comment claimed "nothing else in
            // this process reads these variables concurrently", which was
            // about the wrong thing: the three `capture` tests below read the
            // whole block without touching these keys at all.
            unsafe {
                match value {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    impl Drop for Env {
        fn drop(&mut self) {
            for (key, value) in self.saved.drain(..) {
                // SAFETY: as in `set`.
                unsafe {
                    match value {
                        Some(v) => std::env::set_var(key, v),
                        None => std::env::remove_var(key),
                    }
                }
            }
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qs-sony-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    #[test]
    fn the_reader_uses_the_daemons_own_paths() {
        // The failure this replaces: without XDG_RUNTIME_DIR this dialled
        // /tmp/sony-headphones/daemon.sock while both daemons bind
        // /tmp/sony-headphones-<uid>/daemon.sock, so `qs-sony send` from a
        // bare shell said the socket was not there against a running daemon.
        let env = Env::take(&["XDG_RUNTIME_DIR", "XDG_STATE_HOME", "HOME"]);
        env.set("XDG_RUNTIME_DIR", None);
        env.set("XDG_STATE_HOME", None);
        env.set("HOME", Some(Path::new("/home/tester")));
        // SAFETY: getuid cannot fail and has no preconditions.
        let uid = unsafe { libc::getuid() };
        assert_eq!(socket_path(), PathBuf::from(format!("/tmp/sony-headphones-{uid}/daemon.sock")));
        assert_eq!(socket_path(), sony_xm5::sony::socket_path());
        assert_eq!(state_path(), PathBuf::from("/home/tester/.local/state/sony-headphones/status.json"));
        assert_eq!(state_path(), sony_xm5::sony::state_file());

        env.set("XDG_RUNTIME_DIR", Some(Path::new("/run/user/tester")));
        env.set("XDG_STATE_HOME", Some(Path::new("/var/tester")));
        assert_eq!(socket_path(), PathBuf::from("/run/user/tester/sony-headphones/daemon.sock"));
        assert_eq!(socket_path(), sony_xm5::sony::socket_path());
        assert_eq!(state_path(), PathBuf::from("/var/tester/sony-headphones/status.json"));
    }

    #[test]
    fn a_daemon_is_running_while_it_holds_the_lock_or_answers_the_socket() {
        let env = Env::take(&["XDG_RUNTIME_DIR"]);
        let run = scratch("running");
        env.set("XDG_RUNTIME_DIR", Some(&run));
        assert!(!daemon_running(), "nothing has started, so nothing is running");

        // qs-sonyd: the lock serve::bind takes, held for the daemon's life.
        let held = sony_xm5::sony::serve::bind(&sony_xm5::sony::socket_path()).expect("bind");
        assert!(daemon_running(), "a daemon holding the lock was not seen");
        drop(held);
        // `bind` leaves its socket file for the next daemon to clear, so this
        // is also the dead-daemon case: a file that refuses the connection.
        assert!(sony_xm5::sony::socket_path().exists(), "the fixture left no socket file");
        assert!(!daemon_running(), "a released lock still counted as a daemon");

        // The C++ daemon: no lock, but a listener on the same socket path.
        std::fs::remove_file(sony_xm5::sony::socket_path()).expect("the debris is removable");
        let listener = UnixListener::bind(sony_xm5::sony::socket_path()).expect("a listener");
        assert!(daemon_running(), "a daemon that answers the socket without the lock was not seen");
        drop(listener);
        // The socket file a dead daemon leaves behind refuses connections.
        assert!(sony_xm5::sony::socket_path().exists(), "the fixture left no socket file");
        assert!(!daemon_running(), "a dead daemon's socket file counted as a daemon");
        let _ = std::fs::remove_dir_all(&run);
    }

    #[test]
    fn a_document_with_no_daemon_behind_it_is_stale() {
        // The daemon leaves its last document behind on purpose and has no
        // signal handler, so after a crash or a kill the file may still say
        // connected:true. A reader that trusted the file alone drew a headset
        // nobody was talking to; this one checks for the daemon first.
        let env = Env::take(&["XDG_RUNTIME_DIR", "XDG_STATE_HOME"]);
        let run = scratch("stale-run");
        let state = scratch("stale-state");
        env.set("XDG_RUNTIME_DIR", Some(&run));
        env.set("XDG_STATE_HOME", Some(&state));

        assert_eq!(read_state_reason().expect_err("no file").0, Missing::Absent);

        std::fs::create_dir_all(state.join("sony-headphones")).expect("the state directory");
        std::fs::write(state_path(), "{\"schema_version\":1,\"connected\":true}\n").expect("a document");
        let (why, said) = read_state_reason().expect_err("a document with no daemon behind it");
        assert_eq!(why, Missing::Stale);
        assert!(said.contains("not running"), "{said}");

        let held = sony_xm5::sony::serve::bind(&sony_xm5::sony::socket_path()).expect("bind");
        let data = read_state_reason().expect("the same document, with a daemon behind it");
        assert_eq!(data["connected"], true);
        drop(held);
        assert_eq!(read_state_reason().expect_err("gone again").0, Missing::Stale);

        // A file that cannot be read is that reason whether or not a daemon
        // is behind it: the shape is checked first.
        std::fs::write(state_path(), "not json").expect("a broken document");
        assert_eq!(read_state_reason().expect_err("refused").0, Missing::Unreadable);
        let _ = std::fs::remove_dir_all(&run);
        let _ = std::fs::remove_dir_all(&state);
    }

    #[test]
    fn the_inventory_shape_in_the_contract_is_read_as_jayos_writes_it() {
        // `qs-devices list` is jayos's, and this reads it across a repository
        // boundary. contract/qs-devices.example.json is the shape jayos's
        // qs-devices writes -- jayos holds its own output to the same file --
        // so a field renamed on either side fails a test on that side.
        let inventory: Value =
            serde_json::from_str(include_str!("../../contract/qs-devices.example.json")).expect("valid JSON");
        let found = collect(&inventory, true, false);
        let names: Vec<&str> = found.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["WH-1000XM5", "WF-1000XM5"], "a mouse is not a headset, and a card with no battery still is");
        assert_eq!(found[0].battery, json!(80), "the battery is read from `percent`");
        assert!(found[0].sony && found[1].sony, "both are Sony models");
        assert_eq!(found[0].action, "start", "installed and not running offers a start");
    }

    #[test]
    fn the_native_daemon_is_the_one_beside_this_program_before_path() {
        // Built and installed together, so the sibling is the daemon this
        // client was built against. jayos's helpers' directory is NOT looked
        // in: a qs-sonyd there is a build from before the move.
        let env = Env::take(&["QS_BIN_DIR", "QS_BIN", "HOME"]);
        let dir = scratch("native");
        let daemon = dir.join(NATIVE_DAEMON);
        std::fs::write(&daemon, "#!/bin/sh\n").expect("a fake daemon");
        std::fs::set_permissions(&daemon, std::fs::Permissions::from_mode(0o755)).expect("executable");
        let stale = scratch("native-stale");
        std::fs::copy(&daemon, stale.join(NATIVE_DAEMON)).expect("a stale copy");

        // A stale copy in every place jayos used to look is not preferred.
        env.set("QS_BIN_DIR", Some(&stale));
        env.set("QS_BIN", Some(&stale));
        env.set("HOME", Some(&stale));
        assert_eq!(native_daemon_beside(Some(&dir)), Some(daemon.clone()), "the sibling was not preferred");
        assert_eq!(
            start_command(native_daemon_beside(Some(&dir)).as_deref(), true),
            vec![daemon.to_string_lossy().into_owned()]
        );

        // No sibling: PATH, whatever it holds here -- and still not the
        // helpers' directory, which proc::which does not search.
        let empty = scratch("native-empty");
        assert_eq!(native_daemon_beside(Some(&empty)), proc::which(NATIVE_DAEMON));
        assert_eq!(native_daemon_beside(None), proc::which(NATIVE_DAEMON));

        // A sibling that is there but not executable is not a daemon.
        std::fs::set_permissions(&daemon, std::fs::Permissions::from_mode(0o644)).expect("not executable");
        assert_eq!(native_daemon_beside(Some(&dir)), proc::which(NATIVE_DAEMON));
        for d in [&dir, &stale, &empty] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn the_inventory_is_found_in_the_helpers_directory_before_path() {
        // qs-devices is jayos's, installed in jayos's helpers' directory, and
        // a session a display manager started may not have that on PATH. The
        // precedence is paths.sh's: QS_BIN_DIR, then QS_BIN, then ~/.local/bin.
        let env = Env::take(&["QS_BIN_DIR", "QS_BIN", "HOME"]);
        let dir = scratch("inventory");
        let program = dir.join(INVENTORY);
        std::fs::write(&program, "#!/bin/sh\n").expect("a fake inventory");
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).expect("executable");
        let nowhere = scratch("inventory-nowhere");

        env.set("HOME", Some(&nowhere));
        env.set("QS_BIN_DIR", Some(&dir));
        env.set("QS_BIN", Some(&nowhere));
        assert_eq!(inventory_program(), Some(program.clone()), "QS_BIN_DIR was not looked in first");

        // An empty QS_BIN_DIR is not an override, and QS_BIN is next.
        env.set("QS_BIN_DIR", Some(Path::new("")));
        env.set("QS_BIN", Some(&dir));
        assert_eq!(inventory_program(), Some(program.clone()), "QS_BIN was not looked in second");

        // Neither: ~/.local/bin.
        env.set("QS_BIN_DIR", None);
        env.set("QS_BIN", None);
        let home = scratch("inventory-home");
        std::fs::create_dir_all(home.join(".local/bin")).expect("a bin directory");
        std::fs::copy(&program, home.join(".local/bin").join(INVENTORY)).expect("a copy");
        env.set("HOME", Some(&home));
        assert_eq!(inventory_program(), Some(home.join(".local/bin").join(INVENTORY)));

        // Not in any of the three: only PATH is left, whatever it holds here.
        env.set("HOME", Some(&nowhere));
        assert_eq!(inventory_program(), proc::which(INVENTORY));
        for d in [&dir, &nowhere, &home] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn an_available_status_keeps_the_summary_in_the_order_the_python_built_it() {
        let data: Value = serde_json::from_str(
            r#"{"schema_version": 1, "connected": true, "name": "WH-1000XM4",
                "battery": {"main": 61}, "noise": {"mode": "off"}}"#,
        )
        .expect("test fixture is valid JSON");

        let text = serde_json::to_string(&Status {
            available: true,
            error: String::new(),
            // Absent when there IS state: a reason with no missing state would
            // be a key the page has to learn to ignore.
            reason: None,
            summary: summarise(&data),
            // Left out of this fixture: `raw` is the daemon's object and
            // serde_json sorts it, which is the one ordering this cannot keep.
            raw: None,
        })
        .expect("the envelope serialises");

        assert_eq!(
            text,
            r#"{"available":true,"error":"","summary":{"connected":true,"name":"WH-1000XM4","battery":61,"batteryLeft":null,"batteryRight":null,"batteryCase":null,"noiseMode":"off"}}"#
        );
    }

    #[test]
    fn the_devices_envelope_keeps_its_keys() {
        let text = serde_json::to_string(&Devices {
            devices: vec![],
            daemon_installed: false,
            daemon_running: false,
            daemon_reporting: false,
            project: DAEMON_PROJECT,
            url: DAEMON_URL,
            start: vec![],
        })
        .expect("the envelope serialises");

        assert!(text.starts_with(
            r#"{"devices":[],"daemonInstalled":false,"daemonRunning":false,"daemonReporting":false"#
        ));
        assert!(text.contains(r#""project":"sony-headphones-linux""#));
        // The project the contract in this file was actually written against.
        // The button on the settings page puts this string in front of a
        // person, so a wrong one is the one action the page can take, sending
        // them somewhere that is not it.
        assert!(text.contains(r#""url":"https://github.com/VyomJain6904/sony-headphones-linux""#));
        assert!(text.contains(r#""start":[]"#), "the page checks this key: {text}");
    }

    #[test]
    fn the_start_button_runs_the_daemon_that_is_actually_installed() {
        // The failure this replaces: the page ran `systemctl --user start
        // sony-headphones` unconditionally, which is a unit only the C++
        // project installs. On a machine running the in-tree daemon the one
        // action the page offered did nothing, and did nothing quietly.
        let native = Path::new("/opt/qs/bin/qs-sonyd");
        // Absolute, as found: what the page hands to execDetached must not
        // depend on quickshell's PATH any more than finding it did.
        assert_eq!(start_command(Some(native), true), vec!["/opt/qs/bin/qs-sonyd"]);
        // The native one wins even when both are installed: it is the one this
        // tree ships, tests and can fix.
        assert_eq!(start_command(Some(native), true), start_command(Some(native), false));
        assert_eq!(
            start_command(None, true),
            vec!["systemctl", "--user", "start", "sony-headphones"]
        );
        // Neither: an EMPTY list, not a command that would fail. The page
        // checks the list rather than running something and hoping.
        assert!(start_command(None, false).is_empty());
    }

    #[test]
    fn a_subprocess_that_never_exits_is_given_up_on() {
        // Holds ENV: spawning reads the whole environment block. See `ENV`.
        let _env = Env::hold();
        let start = Instant::now();
        let out = capture(
            "sh",
            &["-c".to_string(), "sleep 30".to_string()],
            Duration::from_millis(200),
        );
        assert_eq!(out, None);
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_subprocess_that_answers_is_read() {
        // Holds ENV: spawning reads the whole environment block. See `ENV`.
        let _env = Env::hold();
        let out = capture(
            "sh",
            &["-c".to_string(), "printf '{\"devices\":[]}'".to_string()],
            Duration::from_secs(5),
        );
        assert_eq!(out.as_deref(), Some(r#"{"devices":[]}"#));
    }

    #[test]
    fn a_missing_program_is_no_inventory() {
        // Holds ENV: spawning reads the whole environment block. See `ENV`.
        let _env = Env::hold();
        assert_eq!(
            capture("definitely-not-a-real-program-qs", &[], Duration::from_secs(1)),
            None
        );
    }
}
