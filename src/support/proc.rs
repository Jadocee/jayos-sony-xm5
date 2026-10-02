//! Running other programs.
//!
//! Most of these helpers are, at heart, a wrapper around a command-line tool
//! that already knows the answer -- `nmcli`, `wpctl`, `pacman`, `hyprctl`. The
//! Python did this with `subprocess.run(..., capture_output=True, text=True)`
//! and then ignored the exit status about half the time, which is why a
//! missing tool produced an empty widget rather than an error.
//!
//! The shape here makes the status impossible to ignore by accident: `output`
//! gives you all three parts, and `stdout_of` is explicitly the "I only care
//! whether it worked" form.
//!
//! `output_within` is `output` with a deadline, and it is the one to reach for
//! whenever the program on the other end talks to a daemon. Everything in
//! `output`'s family waits for as long as the child takes, which is for ever
//! when the child is `nmcli` against a NetworkManager that has accepted the
//! call and stopped replying.

use std::ffi::OsStr;
use std::io;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// What a command produced.
#[derive(Debug, Clone)]
pub struct Output {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    /// Exit status zero.
    pub fn ok(&self) -> bool {
        self.status == 0
    }

    /// Stdout with trailing newlines removed, which is what every caller
    /// actually wanted.
    pub fn trimmed(&self) -> &str {
        self.stdout.trim_end_matches(['\n', '\r'])
    }
}

/// Run a command and capture its output.
///
/// Output is decoded lossily. These read tool output, device names and file
/// paths, and a byte sequence that is not UTF-8 -- a Bluetooth device named by
/// someone's phone, most often -- should degrade to a replacement character
/// rather than failing the call.
pub fn output<S: AsRef<OsStr>>(program: S, args: &[&str]) -> io::Result<Output> {
    let out = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()?;
    Ok(Output {
        status: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// Run a command, returning its trimmed stdout only when it succeeded.
///
/// The common case: "ask this tool, and if it is missing or fails, act as
/// though there is no answer".
pub fn stdout_of<S: AsRef<OsStr>>(program: S, args: &[&str]) -> Option<String> {
    match output(program, args) {
        Ok(o) if o.ok() => Some(o.trimmed().to_string()),
        _ => None,
    }
}

/// Run a command with something on its stdin.
pub fn with_stdin<S: AsRef<OsStr>>(program: S, args: &[&str], input: &str) -> io::Result<Output> {
    use std::io::Write;

    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Written before waiting. Writing after `wait()` deadlocks as soon as the
    // input exceeds the pipe buffer, because the child is blocked reading and
    // we are blocked waiting for it to exit.
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.as_bytes())?;
    }

    let out = child.wait_with_output()?;
    Ok(Output {
        status: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// One deadline shared by every call a run makes.
///
/// The companion to [`output_within`], and the reason it takes a `Duration`
/// rather than holding a clock of its own: a panel that asks a daemon four
/// questions wants a budget for the ANSWER, not four independent deadlines.
/// Four two-second deadlines is an eight-second poll, which is worse than one
/// long one in the way that matters -- long enough that the panel still looks
/// frozen, and reached only when something is already wrong.
///
/// Spend it with [`Budget::left`]: once it is gone every later call expires
/// without spawning anything, which is the right answer. Once the daemon has
/// stopped replying there is nothing left to ask it.
///
/// It is a type rather than a bare `Instant` because the obvious way to write
/// `left` -- `deadline - Instant::now()` -- panics on a deadline that has
/// already passed, and that is a crash reached only on a machine that is
/// already having a bad day.
#[derive(Debug)]
pub struct Budget(Instant);

impl Budget {
    pub fn new(total: Duration) -> Self {
        Budget(Instant::now() + total)
    }

    /// What is left, and `Duration::ZERO` once it is spent. Zero expires; it
    /// does not mean "no limit".
    pub fn left(&self) -> Duration {
        self.0.saturating_duration_since(Instant::now())
    }
}

/// Run a command and capture its output, giving up after `timeout`.
///
/// [`output`] with a deadline, and the same return type, so a call site that
/// grows a timeout changes one line rather than its shape.
///
/// WHY THIS IS NOT OPTIONAL FOR ANYTHING THAT TALKS TO A DAEMON
///
/// A daemon that accepts the connection and never answers is the common
/// failure, not an exotic one: NetworkManager, bluetoothd, upowerd and dockerd
/// all reach it, and `nmcli`/`bluetoothctl`/`upower`/`docker` all wait for a
/// reply with no deadline of their own. These helpers are spawned from a QML
/// poller, so a call that never returns is a panel frozen on stale data with
/// no way back until the shell restarts, and nothing anywhere says why. Three
/// binaries had each written their own version of this before it was shared.
///
/// The expiry is `ErrorKind::TimedOut`, which is what lets a caller tell "it
/// did not answer" from "it answered and said no" -- the second is `Ok` with a
/// non-zero `status`, and a caller that cannot tell them apart cannot write a
/// message worth reading. A missing program is still `ErrorKind::NotFound`,
/// as it is from [`output`].
///
/// Partial output from a call that expired is discarded rather than returned.
/// Half of a listing parses as a shorter listing, and a panel cannot tell that
/// from a machine that really has fewer networks on it.
///
/// An expiry returns within a short fixed grace of `timeout`, never later:
/// killing a child does not always end it, and this returns either way. See
/// `kill_and_reap` for what that costs.
pub fn output_within<S: AsRef<OsStr>>(
    program: S,
    args: &[&str],
    timeout: Duration,
) -> io::Result<Output> {
    let (status, stdout, stderr) = bytes_within(program, args, timeout)?;
    Ok(Output {
        status,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

/// [`output_within`] with variables added to the child's environment.
///
/// For a tool whose output is parsed as text that it translates: `pactl list
/// cards` prints "Active Profile:" through gettext, so in a German session it
/// is "Aktives Profil:" and in a French one the card's "Name:" is "Nom :",
/// and a parser matching the English labels finds a card with no profile, or
/// no card at all. qs-devices runs it with `("LC_ALL", "C")`.
///
/// A parameter on the one runner rather than `env LC_ALL=C pactl ...`: that
/// works, since env execs in place and the deadline's kill still reaches the
/// tool, but the expiry would then name `env` rather than the program it was
/// waiting on, and the call site would stop saying what it runs.
pub fn output_within_env<S: AsRef<OsStr>>(
    program: S,
    args: &[&str],
    env: &[(&str, &str)],
    timeout: Duration,
) -> io::Result<Output> {
    let (status, stdout, stderr) = run_within(program, args, env, timeout)?;
    Ok(Output {
        status,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

/// The same bounded run as [`output_within`], before anything decodes it.
///
/// This is the runner; `output_within` is this plus the lossy decode every
/// caller but one wants. Reach for it only if you have your own decoder:
/// qs-wifi-scan does, because it must break an invalid byte run where jq
/// breaks it so that the path the panel runs and the path the differential
/// harness checks agree, and that is not where `String::from_utf8_lossy`
/// breaks it.
pub fn bytes_within<S: AsRef<OsStr>>(
    program: S,
    args: &[&str],
    timeout: Duration,
) -> io::Result<(i32, Vec<u8>, Vec<u8>)> {
    run_within(program, args, &[], timeout)
}

/// The runner behind [`bytes_within`] and [`output_within_env`]: one body, so
/// the deadline, the draining and the reaping cannot differ between them.
fn run_within<S: AsRef<OsStr>>(
    program: S,
    args: &[&str],
    env: &[(&str, &str)],
    timeout: Duration,
) -> io::Result<(i32, Vec<u8>, Vec<u8>)> {
    // Taken before the name is moved into `Command`, so the expiry can say
    // which program it was waiting on -- "timed out" with no subject is a line
    // that sends the next person reading stderr back to the source.
    let name = program.as_ref().to_string_lossy().into_owned();

    // A spent [`Budget`] hands this zero, and it has to give up BEFORE the
    // spawn. `qs-eth-status status` reaches that: on a live wired link, `nmcli
    // connection show --active` wedging spends the run's whole two seconds, and
    // the IP4.ADDRESS call behind it was still forking, exec'ing and SIGKILLing
    // an nmcli two hundred microseconds later -- one more D-Bus client per
    // poll, every one to three seconds, handed to the NetworkManager that had
    // already stopped answering, and gone again mid-handshake.
    //
    // It also answered the wrong question. `spawn` reports a missing program as
    // NotFound, so a spent budget against an uninstalled tool came back
    // NotFound rather than TimedOut -- and qs-container-status branches on exactly
    // that, so it would have blamed "docker is not installed" for a daemon that
    // had merely run out of time and sent the reader off to pacman.
    if timeout.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("{name} was not run: the deadline was already spent"),
        ));
    }

    let mut child = Command::new(program)
        .args(args)
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let deadline = Instant::now() + timeout;

    // Both pipes are drained on their own threads, and drained BEFORE the wait.
    // `.output()` gets this right; spawning and then reading one pipe and
    // waiting afterwards does not -- it deadlocks as soon as the other fills
    // its 64 KiB buffer, because the child is blocked writing and we are
    // blocked waiting for it to exit. `docker ps --all --no-trunc` on a busy
    // machine and `pactl list cards` are both past that.
    let out_rx = drain(child.stdout.take());
    let err_rx = drain(child.stderr.take());

    let (Some(stdout), Some(stderr)) = (collect(&out_rx, deadline), collect(&err_rx, deadline))
    else {
        return Err(expired(child, &name, timeout));
    };

    // EOF on both pipes almost always means the child has exited, so this
    // settles on the first pass. The deadline is still enforced, because a
    // program that closes its output and keeps running is what a timeout is
    // for -- `bluetoothctl` holding its D-Bus connection open is that.
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok((status.code().unwrap_or(-1), stdout, stderr)),
            Ok(None) => {
                if Instant::now() >= deadline {
                    return Err(expired(child, &name, timeout));
                }
                thread::sleep(Duration::from_millis(2));
            }
            Err(e) => {
                kill_and_reap(child, REAP_GRACE);
                return Err(e);
            }
        }
    }
}

/// Read a pipe to EOF on another thread.
///
/// The thread outlives a timed-out call only for as long as the pipe stays
/// open, which killing the child ends -- unless the child left a grandchild
/// holding the write end, and a helper that spawns a daemon is not something
/// any caller here does.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    match pipe {
        Some(mut p) => {
            thread::spawn(move || {
                let mut buf = Vec::new();
                let _ = p.read_to_end(&mut buf);
                let _ = tx.send(buf);
            });
        }
        // No pipe is not a failure; it is nothing to read.
        None => {
            let _ = tx.send(Vec::new());
        }
    }
    rx
}

/// A drained pipe's contents, or None if the deadline passed first.
fn collect(rx: &mpsc::Receiver<Vec<u8>>, deadline: Instant) -> Option<Vec<u8>> {
    rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).ok()
}

/// How long a killed child gets to actually die before we stop watching it.
///
/// Not a second deadline for the work: the child has already been signalled
/// and the answer has already been given up on, so this is only "long enough
/// that a process which CAN die has died". SIGKILL on a process that is
/// runnable is delivered in microseconds; the rest is headroom for a machine
/// under enough load that the runner missed its deadline in the first place.
///
/// A run that shares a [`Budget`] pays it at most once, because the first
/// expiry spends the budget and every call after that gives up before the
/// spawn. A caller that hands each call its own fixed timeout can pay it per
/// expiry, which is why it is this short rather than a comfortable second.
const REAP_GRACE: Duration = Duration::from_millis(200);

/// Kill a child that ran out of time, and turn that into the caller's error.
fn expired(child: Child, name: &str, timeout: Duration) -> io::Error {
    kill_and_reap(child, REAP_GRACE);
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!("{name} did not answer within {:.1}s", timeout.as_secs_f32()),
    )
}

/// Kill a child and reap it, waiting at most `grace` for the kill to land.
///
/// WHY IT IS REAPED AT ALL
///
/// A killed child that is never waited for stays a zombie in our process table
/// until we exit, and the callers that poll -- one process per tick, several
/// ticks a second between three panels -- would accumulate one per wedged call
/// for as long as the shell runs. So "kill and return" is not the fix here,
/// even though it is the one-liner: it trades a hang for a leak.
///
/// WHY THE WAIT IS BOUNDED
///
/// SIGKILL is not delivered to a process in uninterruptible sleep until the
/// syscall it is inside returns. `docker ps --all` against an overlay or
/// NFS-backed graph driver whose mount has gone away is that, and so is
/// `upower -i` on a wedged USB controller. `kill` reports success, the process
/// stays in D state, and an unbounded `wait` blocks for as long as the mount
/// does -- so the deadline that exists to stop a panel freezing becomes the
/// freeze, in the one case it was written for.
///
/// It is worse there than no deadline at all: `qs-container-status list` never
/// prints its JSON, so ContainersPage's StdioCollector never fires, and because
/// `Process.running` stays true the QML poller never restarts it. The page is
/// stuck on stale data with no way back until the shell restarts, and nothing
/// anywhere says why.
///
/// WHAT THE HAND-OFF COSTS
///
/// A child that has not died by `grace` is moved onto a thread that waits on
/// it, and the caller returns. The thread is spawned only on that path, so an
/// ordinary expiry -- a child that dies the instant it is signalled -- costs
/// nothing at all.
///
/// The residual, when it is reached, is the pid, the thread waiting on it,
/// and -- for as long as the child holds our pipes open -- the two draining
/// them, all of them stuck until the syscall the child is in returns. That is
/// the trade, made deliberately: holding threads is survivable, holding the
/// CALLER is not, and only the caller has a panel waiting on it. Every caller
/// today is a one-shot helper that runs, prints JSON and exits -- the kernel
/// reparents the child to init, which reaps it, so nothing outlives the
/// process. A session-long daemon keeps the thread until the syscall returns,
/// which is why this is a detached thread rather than a `wait` on the way out.
///
/// Not one shared reaper thread: that costs a thread and a channel in every
/// process that ever calls this, including the one-shot helpers that never
/// reach this path, to save threads only in the case where a daemon is polling
/// a mount that has been dead for hours -- and in that case it is already
/// leaking one wedged pid per poll no matter who waits on it.
fn kill_and_reap(mut child: Child, grace: Duration) {
    let _ = child.kill();

    let deadline = Instant::now() + grace;
    loop {
        match child.try_wait() {
            // Reaped, or unwaitable and so not ours to reap: either way there
            // is nothing left to hold.
            Ok(Some(_)) | Err(_) => return,
            Ok(None) => {
                if Instant::now() >= deadline {
                    break;
                }
                thread::sleep(Duration::from_millis(2));
            }
        }
    }

    // `Builder` rather than `thread::spawn` because the release profile is
    // panic = "abort", where a spawn that fails at the thread or pid limit
    // aborts the helper mid-answer -- and a machine out of threads is exactly
    // where this path gets reached. On failure the closure is dropped with the
    // child inside it, which leaves the zombie this function exists to prevent;
    // one zombie until exit beats no output at all. Named so that `ps -L` on a
    // helper that is taking too long says which thread is stuck and why.
    let _ = thread::Builder::new().name("qs-reap".into()).spawn(move || {
        let _ = child.wait();
    });
}

/// Start a program and do not wait for it.
///
/// stdio is detached from ours. A child that inherits our stdout writes into
/// whatever is parsing our JSON, and one that inherits our stdin can steal a
/// terminal.
pub fn detach<S: AsRef<OsStr>>(program: S, args: &[&str]) -> io::Result<u32> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|c| c.id())
}

/// Where a program is, if it is on `PATH`.
///
/// This is `command -v`, and it is here because guarding on it is a decision
/// the project has already made deliberately: a tool is guarded when a
/// fallback exists, and left unguarded when its absence should be loud. See
/// docs/DEPENDENCIES.md. Use it for the first case only.
pub fn which(program: &str) -> Option<PathBuf> {
    if program.contains('/') {
        let p = PathBuf::from(program);
        return is_executable(&p).then_some(p);
    }

    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        let candidate = dir.join(program);
        is_executable(&candidate).then_some(candidate)
    })
}

/// Whether a program is available at all.
pub fn have(program: &str) -> bool {
    which(program).is_some()
}

/// Die quietly when the reader goes away, instead of aborting.
///
/// Rust sets SIGPIPE to SIG_IGN before `main`, so a write to a closed pipe
/// comes back as `EPIPE` and the printing macros panic on it. Under this
/// workspace's `panic = "abort"` that is a SIGABRT: `qs-check all ~/.config |
/// head` printed the first few lines, then
///
/// ```text
/// thread 'main' panicked at library/std/src/io/stdio.rs:
/// failed printing to stdout: Broken pipe (os error 32)
/// ```
///
/// and left 134 -- which is not one of the three statuses this tree's exit
/// codes are a contract about, so a wrapper reading it learns nothing. Piping
/// into `head` or quitting `less` early is the ordinary way to read output
/// this long, and it should mean what it means for every other Unix program.
///
/// Only for programs whose stdout is a stream a person reads. The helpers that
/// answer a QML `Process` print one small envelope that fits the pipe buffer,
/// so they never see EPIPE, and their callers read to the end regardless.
pub fn die_on_closed_stdout() {
    // SAFETY: setting a signal disposition to SIG_DFL is async-signal-safe and
    // is what the process would have had if Rust had not changed it.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

/// Have the kernel end this process when the one that started it ends.
///
/// For a daemon the shell owns. `shell-restart.sh` ends Quickshell with a
/// signal it has no handler for, so the cleanup that stops the Processes it
/// started never runs and every one of them outlives it. A daemon that only
/// writes files never finds out: nothing it writes goes to the dead parent, so
/// there is no SIGPIPE to end it, and it keeps running beside the copy the next
/// shell starts -- two writers for one file, for the rest of the session.
///
/// SIGTERM, sent however the parent ended, including `kill -9`. Returns whether
/// the kernel took the request; a caller that gets false is running the way it
/// would have without this. `qs-osk-controller` keeps its own copy, which also
/// refuses to start once its parent has already gone.
pub fn die_with_parent() -> bool {
    // SAFETY: prctl(PR_SET_PDEATHSIG) reads two integers and writes nothing;
    // there is no pointer argument to get wrong. The signal goes as the
    // unsigned long the variadic call reads it as.
    unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM as libc::c_ulong) == 0 }
}

fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Long enough that a slow machine cannot make a test about something else
    /// fail on the deadline. The tests that are about the deadline set their
    /// own, in milliseconds.
    const TWENTY: Duration = Duration::from_secs(20);

    #[test]
    fn captures_status_and_streams() {
        let o = output("sh", &["-c", "printf out; printf err >&2; exit 3"]).unwrap();
        assert_eq!(o.status, 3);
        assert!(!o.ok());
        assert_eq!(o.stdout, "out");
        assert_eq!(o.stderr, "err");
    }

    #[test]
    fn stdout_of_is_none_on_failure() {
        assert_eq!(stdout_of("sh", &["-c", "echo hi"]).as_deref(), Some("hi"));
        assert_eq!(stdout_of("sh", &["-c", "echo hi; exit 1"]), None);
        assert_eq!(stdout_of("definitely-not-a-real-program-qs", &[]), None);
    }

    #[test]
    fn stdin_larger_than_a_pipe_buffer_does_not_deadlock() {
        // 1 MiB, comfortably past the 64 KiB pipe buffer that made the naive
        // write-then-wait version hang.
        let big = "x".repeat(1024 * 1024);
        let o = with_stdin("wc", &["-c"], &big).unwrap();
        assert!(o.ok());
        assert_eq!(o.trimmed().trim(), big.len().to_string());
    }

    // ── output_within ────────────────────────────────────────────────────────
    //
    // These spawn real children, which nothing else in this tree's tests does.
    // They earn it: what is under test is the interaction between a process
    // that will not exit, a pipe with a finite buffer and a deadline, and none
    // of the three can be faked without testing the fake instead.

    #[test]
    fn a_child_that_never_exits_times_out_rather_than_hanging() {
        let started = Instant::now();
        let e = output_within("sh", &["-c", "sleep 30"], Duration::from_millis(150))
            .expect_err("a child that outlives its deadline must not return Ok");

        assert_eq!(e.kind(), io::ErrorKind::TimedOut);
        // The whole point. If this assertion is what fails, the test suite has
        // just spent 30 seconds proving the deadline is not enforced.
        assert!(started.elapsed() < Duration::from_secs(5), "the deadline was not enforced");
    }

    #[test]
    fn a_child_holding_its_pipes_open_still_times_out() {
        // The harder case: the child has written nothing and closed nothing, so
        // both drain threads are blocked in `read_to_end` and only the pipe
        // deadline can end the call. `sleep` alone inherits nothing to hold, so
        // this keeps a subshell alive with the pipes still attached.
        let started = Instant::now();
        let e = output_within("sh", &["-c", "exec sleep 30"], Duration::from_millis(150))
            .expect_err("a child holding both pipes open must not return Ok");

        assert_eq!(e.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(5), "the deadline was not enforced");
    }

    #[test]
    fn stdout_larger_than_a_pipe_buffer_does_not_deadlock() {
        // 200 KB, comfortably past the 64 KiB pipe buffer that wedges a runner
        // which waits on the child before draining it. The trailing `sleep`
        // keeps the child alive after the last byte, so the wait really does
        // overlap the write rather than finding an already-exited process.
        let o = output_within(
            "sh",
            &["-c", "yes abcdefghij | head -c 200000; sleep 0.2"],
            Duration::from_secs(20),
        )
        .expect("a child that writes past the pipe buffer must still complete");

        assert!(o.ok());
        assert_eq!(o.stdout.len(), 200_000);
    }

    #[test]
    fn both_pipes_past_the_buffer_at_once_does_not_deadlock() {
        // Draining stdout and then stderr in sequence passes the test above and
        // still deadlocks here: whichever stream is read second fills its
        // buffer and stops the child before the first reaches EOF.
        let o = output_within(
            "sh",
            &["-c", "yes a | head -c 200000 & yes b | head -c 200000 >&2; wait"],
            Duration::from_secs(20),
        )
        .expect("a child that fills both pipes must still complete");

        assert_eq!(o.stdout.len(), 200_000);
        assert_eq!(o.stderr.len(), 200_000);
    }

    #[test]
    fn a_failure_is_told_apart_from_an_expiry() {
        // The distinction the callers branch on: qs-container-status reports 124
        // and "did not respond" for one and docker's own words for the other,
        // and it cannot do that from a single "it did not work".
        let ran = output_within("sh", &["-c", "printf no >&2; exit 3"], TWENTY)
            .expect("a child that exits non-zero ran, and is not an error");
        assert_eq!(ran.status, 3);
        assert_eq!(ran.stderr, "no");

        let missing = output_within("definitely-not-a-real-program-qs", &[], TWENTY)
            .expect_err("a missing program is an error");
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn partial_output_from_an_expired_call_is_not_returned() {
        // A killed `bluetoothctl info` that got half a block out must not have
        // that half parsed as state, which is what `|| info=""` meant in the
        // shell this replaced.
        let e = output_within(
            "sh",
            &["-c", "printf partial; sleep 30"],
            Duration::from_millis(150),
        )
        .expect_err("an expired call must not return its partial output");
        assert_eq!(e.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn a_spent_budget_expires_rather_than_meaning_no_limit() {
        // What the second and later calls of a run whose first call wedged are
        // handed. `Duration::ZERO` has to expire: reading it as "no limit"
        // would put the unbounded wait back exactly where the deadline was
        // supposed to have removed it.
        let spent = Budget::new(Duration::ZERO);
        assert_eq!(spent.left(), Duration::ZERO);

        let started = Instant::now();
        let e = output_within("sh", &["-c", "sleep 30"], spent.left())
            .expect_err("a spent budget must not run the child anyway");
        assert_eq!(e.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(5), "a spent budget still waited");
    }

    #[test]
    fn a_spent_deadline_gives_up_before_the_spawn() {
        // "Expires" is not enough on its own -- it has to expire without
        // forking. A spent budget that still spawns hands an already-wedged
        // daemon a fresh client every poll, and it reports the wrong cause: the
        // spawn's own NotFound reaches the caller instead of TimedOut, and
        // qs-container-status turns that into "<runtime> is not installed".
        //
        // A missing program is what makes this observable either way: a runner
        // that spawns can only answer NotFound, and one that does not can only
        // answer TimedOut.
        let e = output_within("definitely-not-a-real-program-qs", &[], Duration::ZERO)
            .expect_err("a spent deadline must not return Ok");
        assert_eq!(
            e.kind(),
            io::ErrorKind::TimedOut,
            "a spent deadline reached the spawn: {e}"
        );

        // And nothing ran. `touch` is the cheapest observable side effect a
        // spawned child can have; the file must not exist because there was no
        // child, not because we killed it before it got that far.
        let marker = std::env::temp_dir().join(format!("qs-spent-deadline-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        for _ in 0..50 {
            let _ = output_within("touch", &[marker.to_str().unwrap()], Duration::ZERO);
        }
        assert!(!marker.exists(), "a spent deadline ran the child anyway");
    }

    #[test]
    fn a_timed_out_child_is_reaped_rather_than_left_a_zombie() {
        // Killing without waiting leaves a zombie in OUR table until we exit.
        // The callers poll -- one process per tick, several ticks a second
        // across three panels -- so a wedged daemon would accumulate one per
        // call for as long as the shell runs, and the shell runs for days.
        //
        // Counted as a delta rather than an absolute, because `proc` is the
        // only thing in this crate that spawns anything and its other tests run
        // alongside this one.
        let before = zombie_children();
        for _ in 0..5 {
            let e = output_within("sh", &["-c", "exec sleep 30"], Duration::from_millis(80))
                .expect_err("the child outlives its deadline");
            assert_eq!(e.kind(), io::ErrorKind::TimedOut);
        }
        assert!(
            zombie_children() <= before,
            "a killed child was not reaped: {before} zombies before, {} after",
            zombie_children()
        );
    }

    #[test]
    fn a_child_that_closes_both_pipes_and_keeps_running_still_expires() {
        // The other expiry path, and the one the pipe deadline cannot reach:
        // both pipes hit EOF straight away, so `collect` succeeds and the only
        // thing left holding the deadline is the `try_wait` loop. A program that
        // closes its output and keeps its D-Bus connection open is what that
        // loop is for -- returning here would mean an empty answer reported as
        // a real one.
        let started = Instant::now();
        let e = output_within(
            "sh",
            &["-c", "exec 1>&- 2>&-; exec sleep 30"],
            Duration::from_millis(150),
        )
        .expect_err("a child that closed its pipes but is still running must not return Ok");

        assert_eq!(e.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(5), "the deadline was not enforced");
    }

    #[test]
    fn a_kill_that_does_not_land_does_not_hold_the_caller() {
        // The D-state case -- `docker ps` on a graph driver whose mount has
        // gone away -- cannot be provoked without a hostile mount, so what is
        // tested is the mechanism rather than the symptom: with no grace at
        // all, the reap must give up on a child that is still there and return
        // anyway. A child that is merely alive stands in for one that cannot
        // be killed; the code path taken is the same, and it is the one that
        // used to block for as long as the mount stayed dead.
        let child = Command::new("sh").args(["-c", "exec sleep 30"]).spawn().unwrap();
        let pid = child.id();

        let started = Instant::now();
        kill_and_reap(child, Duration::ZERO);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "the reap waited for the child rather than handing it off"
        );

        // Handed to a thread, not abandoned. The zombie this function exists
        // to prevent must still not be there a moment later -- the difference
        // is only that nobody waited for it here.
        assert!(
            reaped_within(pid, Duration::from_secs(5)),
            "the handed-off child was left a zombie: nothing waited on {pid}"
        );
    }

    #[test]
    fn an_ordinary_expiry_reaps_before_it_returns() {
        // The common path, which must not have got slower or leakier for the
        // sake of the one above: a child that dies when it is signalled is
        // reaped inside the grace, no thread is spawned, and the caller still
        // gets TimedOut.
        let child = Command::new("sh").args(["-c", "exec sleep 30"]).spawn().unwrap();
        let pid = child.id();

        let started = Instant::now();
        let e = expired(child, "sleep", Duration::from_millis(150));
        assert_eq!(e.kind(), io::ErrorKind::TimedOut);
        assert!(e.to_string().contains("sleep"), "the expiry does not name the program: {e}");
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "killing a killable child should not take the whole grace"
        );
        assert!(reaped_within(pid, Duration::from_secs(5)), "a killed child was left a zombie");
    }

    /// Whether a pid stops existing within `limit` -- which for one of our own
    /// children means it was waited for, since an unreaped child stays in
    /// /proc as a zombie until we exit.
    fn reaped_within(pid: u32, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        loop {
            if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Our own children sitting unreaped, counted out of /proc.
    ///
    /// Linux only, which is what this shell targets. Field 3 of `stat` is the
    /// state and field 4 the parent, both read from after the final `)` because
    /// the command name in between can contain spaces and brackets.
    fn zombie_children() -> usize {
        let me = std::process::id();
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return 0;
        };
        entries
            .flatten()
            .filter(|e| {
                let Ok(stat) = std::fs::read_to_string(e.path().join("stat")) else {
                    return false;
                };
                let Some(close) = stat.rfind(')') else { return false };
                let fields: Vec<&str> = stat[close + 1..].split_whitespace().collect();
                fields.first() == Some(&"Z")
                    && fields.get(1).and_then(|p| p.parse::<u32>().ok()) == Some(me)
            })
            .count()
    }

    #[test]
    fn a_budget_is_spent_by_the_calls_that_use_it() {
        // The property the panels rely on: four calls against one budget cost
        // one budget between them, not four.
        let budget = Budget::new(Duration::from_millis(400));
        let started = Instant::now();

        for _ in 0..4 {
            let _ = output_within("sh", &["-c", "sleep 30"], budget.left());
        }

        assert_eq!(budget.left(), Duration::ZERO);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "each call took its own deadline rather than a share of one"
        );
    }

    #[test]
    fn bytes_within_hands_back_the_undecoded_stream() {
        // `ED A0 80` is a CESU-8 surrogate: std's decoder emits three
        // replacement characters for it and jq emits one, so qs-wifi-scan needs
        // the bytes rather than either answer. See its `jq_lossy`.
        let (status, out, _) =
            bytes_within("printf", &["\\355\\240\\200"], TWENTY).unwrap();
        assert_eq!(status, 0);
        assert_eq!(out, b"\xed\xa0\x80");
    }

    #[test]
    fn output_within_env_sets_the_childs_environment() {
        // The child sees the variable, which is what pins pactl's locale.
        let o = output_within_env(
            "sh",
            &["-c", "printf '%s' \"$LC_ALL\""],
            &[("LC_ALL", "C")],
            TWENTY,
        )
        .unwrap();
        assert_eq!(o.stdout, "C");
        // And only the child's: a plain run after it does not inherit it, so a
        // helper that pins one call does not change the next.
        let probe = ["-c", "printf '%s' \"${QS_PROC_ENV_PROBE-unset}\""];
        let pinned =
            output_within_env("sh", &probe, &[("QS_PROC_ENV_PROBE", "set")], TWENTY).unwrap();
        let plain = output_within("sh", &probe, TWENTY).unwrap();
        assert_eq!((pinned.stdout.as_str(), plain.stdout.as_str()), ("set", "unset"));
    }

    #[test]
    fn output_within_env_keeps_the_deadline_and_names_the_program() {
        let err = output_within_env("sleep", &["30"], &[("LC_ALL", "C")], Duration::from_millis(100))
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(err.to_string().contains("sleep"), "the expiry names the program: {err}");
    }

    #[test]
    fn which_finds_and_rejects() {
        assert!(have("sh"));
        assert!(!have("definitely-not-a-real-program-qs"));
        // A directory is not an executable, even though it has the bit set.
        assert_eq!(which("/tmp"), None);
    }
}
