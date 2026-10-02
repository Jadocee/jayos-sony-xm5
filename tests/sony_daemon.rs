//! `qs-sonyd`, run as a process.
//!
//! The wiring is the one part of this daemon that cannot be reached by a unit
//! test: three threads, a mailbox, a socket and a dialler, and none of it
//! means anything until they are started together. So these tests start the
//! real binary, with `XDG_STATE_HOME` and `XDG_RUNTIME_DIR` pointed at a
//! temporary directory, and hold it to what `qs-sony` and the Headphones page
//! actually depend on.
//!
//! THERE IS NO BLUETOOTH ADAPTER HERE, and that is not a limitation of these
//! tests -- it is the case most machines are in most of the time, and the one
//! this daemon has to get right. A daemon that only behaves when a headset is
//! present is a daemon that spends its life misbehaving. So what is asserted
//! is the no-headset path: that the document exists and says so, that the
//! reason reaches it, that the socket still answers, that a second daemon
//! refuses, and that a one-shot run ends.

use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const DAEMON: &str = env!("CARGO_BIN_EXE_qs-sonyd");
const READER: &str = env!("CARGO_BIN_EXE_qs-sony");

struct Home {
    base: PathBuf,
}

impl Home {
    fn new(name: &str) -> Home {
        let base = std::env::temp_dir().join(format!("qs-sonyd-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("state")).expect("a state directory");
        std::fs::create_dir_all(base.join("run")).expect("a runtime directory");
        Home { base }
    }

    fn command(&self, verb: &str) -> Command {
        let mut command = Command::new(DAEMON);
        command
            .arg(verb)
            .env("XDG_STATE_HOME", self.base.join("state"))
            .env("XDG_RUNTIME_DIR", self.base.join("run"))
            // So the daemon's own fallbacks are never what is under test.
            .env("HOME", &self.base)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    /// `qs-sony <verb>` against this home, with a `qs-devices` in jayos's
    /// helpers' directory that lists one Sony headset -- the inventory the
    /// page's Devices tab reads.
    ///
    /// The helpers' directory and NOT `PATH`, which is left as the test runner
    /// has it: that is the session a display manager starts, where
    /// `~/.local/bin` is not on `PATH` and a bare-name spawn found nothing.
    /// The daemon is found beside `qs-sony` in cargo's target directory, which
    /// is where the package installs them too -- no variable points at it.
    fn reader(&self, verb: &str) -> std::process::Output {
        let bin = self.base.join("bin");
        std::fs::create_dir_all(&bin).expect("a bin directory");
        let fake = bin.join("qs-devices");
        std::fs::write(
            &fake,
            "#!/bin/sh\nprintf '%s\\n' '{\"devices\":[{\"name\":\"WH-1000XM5\",\"type\":\"headset\"}]}'\n",
        )
        .expect("a fake inventory");
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).expect("executable");
        Command::new(READER)
            .arg(verb)
            .env("XDG_STATE_HOME", self.base.join("state"))
            .env("XDG_RUNTIME_DIR", self.base.join("run"))
            .env("HOME", &self.base)
            .env("QS_BIN_DIR", &bin)
            .env_remove("QS_BIN")
            .output()
            .expect("qs-sony runs")
    }

    fn state_file(&self) -> PathBuf {
        self.base.join("state/sony-headphones/status.json")
    }

    fn socket(&self) -> PathBuf {
        self.base.join("run/sony-headphones/daemon.sock")
    }
}

/// A running daemon, killed when the test ends however the test ends.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_for(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if ready() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("{what} did not happen within ten seconds");
}

fn document(path: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(path).expect("the document is readable");
    // One JSON object per LINE, and one line: the file ends in exactly the
    // newline the reference's serializeState closes with (state.cpp, its
    // `"}\n"`; atomicWrite adds nothing), not a blank line after it.
    assert!(text.ends_with("}\n"), "the file must end with the object and one newline");
    assert_eq!(text.matches('\n').count(), 1, "exactly one newline, at the end");
    serde_json::from_str(&text).expect("the document is valid JSON")
}

/// One line to the socket, and the reply as the bytes came back.
fn ask_raw(socket: &Path, line: &str) -> std::io::Result<String> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\n")?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    Ok(reply)
}

fn ask(socket: &Path, line: &str) -> std::io::Result<String> {
    Ok(ask_raw(socket, line)?.trim().to_string())
}

#[test]
fn a_daemon_with_no_headset_publishes_a_document_that_says_so() {
    // THE FIRST THING ANY READER NEEDS. `qs-sony available` and the Headphones
    // page both read this file, and a file that is not there cannot be told
    // from a daemon that has not started -- so it exists before anything is
    // connected, and it is honest about there being nothing.
    let home = Home::new("nodevice");
    let _daemon = Running(home.command("run").spawn().expect("the daemon starts"));

    wait_for("the document to appear", || home.state_file().exists());
    let value = document(&home.state_file());
    assert_eq!(value["schema_version"], 1, "the version qs-sony gates on");
    assert_eq!(value["connected"], false);
    assert_eq!(value["phase"], "idle");
    assert!(
        value["daemon_version"].as_str().unwrap_or("").starts_with("qs-"),
        "this daemon must not claim to be the C++ one: {}",
        value["daemon_version"]
    );

    // The modes a private document depends on, from the kernel.
    let mode = |p: &Path| std::fs::metadata(p).expect("it exists").permissions().mode() & 0o777;
    assert_eq!(mode(&home.state_file()), 0o600);
    assert_eq!(mode(home.state_file().parent().expect("a parent")), 0o700);
}

#[test]
fn the_reason_a_headset_could_not_be_reached_reaches_the_document() {
    // There is no adapter here, so the dial fails -- and the point of this
    // test is that it fails OUT LOUD. "not connected" and "bluetoothctl is not
    // installed" send a person to two different places.
    let home = Home::new("reason");
    let _daemon = Running(home.command("run").spawn().expect("the daemon starts"));

    wait_for("a reason to be published", || {
        home.state_file().exists()
            && !document(&home.state_file())["last_error"].as_str().unwrap_or("").is_empty()
    });
    let value = document(&home.state_file());
    assert_eq!(value["connected"], false);
    assert_eq!(value["phase"], "idle", "no headset is not a broken daemon");
    let reason = value["last_error"].as_str().expect("a string");
    assert!(!reason.is_empty(), "the dial failed silently");
}

#[test]
fn the_socket_answers_while_no_headset_is_connected() {
    // A daemon that only answered with hardware attached would make every
    // panel that polls it look broken on a machine with no headphones.
    let home = Home::new("socket");
    let _daemon = Running(home.command("run").spawn().expect("the daemon starts"));
    wait_for("the socket to appear", || home.socket().exists());

    // The verbs the daemon answers itself, with or without a headset.
    for verb in ["panel-open", "panel-close"] {
        wait_for("the socket to answer", || matches!(ask(&home.socket(), verb).as_deref(), Ok("ok")));
    }
    // `status` is `ok `, the document, and the one newline the socket ends
    // every reply line with -- exactly what the C++ daemon_main sends.
    wait_for("the socket to answer", || ask(&home.socket(), "status").is_ok());
    let reply = ask_raw(&home.socket(), "status").expect("a reply");
    assert!(reply.starts_with("ok {\"schema_version\":1,"), "{reply}");
    assert!(reply.ends_with("}\n"), "{reply:?}");
    assert_eq!(reply.matches('\n').count(), 1, "one line: {reply:?}");
    let value: serde_json::Value = serde_json::from_str(&reply[3..]).expect("ok then JSON");
    assert_eq!(value["connected"], false);
    // And one that needs a headset says so rather than pretending.
    let reply = ask(&home.socket(), "noise anc").expect("a reply");
    assert!(reply.starts_with("error"), "a command was accepted with no headset: {reply}");
    assert!(reply.contains("no headset"), "the refusal does not say why: {reply}");

    // A command the language does not have is refused by the parser, not by
    // the headset -- which is why it gets an answer here at all.
    let reply = ask(&home.socket(), "nonsense").expect("a reply");
    assert!(reply.starts_with("error"), "{reply}");

    // The socket is private, and so is the directory holding it.
    let mode = |p: &Path| std::fs::metadata(p).expect("it exists").permissions().mode() & 0o777;
    assert_eq!(mode(&home.socket()), 0o600);
    assert_eq!(mode(home.socket().parent().expect("a parent")), 0o700);
}

#[test]
fn a_second_daemon_refuses_rather_than_stealing_the_first_ones_socket() {
    let home = Home::new("single");
    let _daemon = Running(home.command("run").spawn().expect("the daemon starts"));
    wait_for("the socket to appear", || home.socket().exists());

    let second = home.command("run").output().expect("the second runs");
    assert!(!second.status.success(), "a second daemon started alongside the first");
    let said = String::from_utf8_lossy(&second.stdout) + String::from_utf8_lossy(&second.stderr);
    assert!(said.contains("already running"), "the refusal does not say why: {said}");

    // And the first is still serving.
    assert!(ask(&home.socket(), "status").expect("a reply").starts_with("ok {"));
}

#[test]
fn a_one_shot_run_ends_by_itself() {
    // For a person checking the daemon works, and for the installer. A one
    // shot that did not end would be a hung login script.
    let home = Home::new("once");
    let began = Instant::now();
    let out = home.command("once").output().expect("it runs");
    assert!(out.status.success(), "a one-shot run failed: {out:?}");
    assert!(began.elapsed() < Duration::from_secs(35), "it took {:?}", began.elapsed());

    // AND IT COSTS ONE SETTLE, NOT THIRTY SECONDS. The bound above is
    // `ONCE_LONGEST` plus slack, which is the backstop rather than the
    // expectation -- it passes over a run that reached the backstop, and a run
    // that reached the backstop is a run that hung.
    //
    // There is no headset here, so the whole of it is: fail to dial, publish
    // the reason, wait `ONCE_SETTLE` (1.5s), exit. This bounds it at five
    // seconds, which leaves room for a slow `bluetoothctl` and none for a
    // settle that quietly grew.
    //
    // WHAT IT CATCHES AND WHAT IT DOES NOT, because the difference is the
    // whole reason the constant is written the way it is. Applying the settle
    // to the exit condition and NOT to the wait leaves this daemon blocked in
    // `recv_timeout` for the full hour: the dial fails, `Trouble` publishes,
    // and nothing further ever arrives to bring the loop round. MEASURED --
    // mutated to exactly that shape, this suite ran past 120 seconds where it
    // otherwise finishes in 1.5. But it ran past it by HANGING, not by
    // failing: `cargo test` has no per-test timeout, so that regression
    // surfaces as a suite that never returns rather than as a red assertion.
    // What the line below actually catches is the milder shape -- a settle
    // that grew, or a second one added -- and the hang is caught by somebody
    // noticing the suite has stopped.
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "a one-shot run on a machine with no headset took {:?} -- it should cost \
         one dial and one settle, so this is the wait blocking rather than expiring",
        began.elapsed()
    );

    // It published what it found, which here is that it found nothing.
    let value = document(&home.state_file());
    assert_eq!(value["connected"], false);

    // And it left nothing behind for the next daemon to clean up.
    assert!(!home.socket().exists(), "the one-shot left its socket in place");
}

#[test]
fn a_one_shot_run_says_whether_it_asks_for_alerts() {
    // PLAN 23, ITEM 14. `--alerts` reaching the session has no other
    // witness without a headset: the enable it causes is sent only to one
    // that advertises FIXED_MESSAGE. The log line is read back from the
    // session, so a flag that stopped reaching it stops the line too.
    const LINE: &str = "--alerts: the headset will be asked for its confirmations";
    let home = Home::new("alerts");
    let with = home.command("once").arg("--alerts").output().expect("it runs");
    assert!(with.status.success(), "{with:?}");
    assert!(String::from_utf8_lossy(&with.stderr).contains(LINE), "{}", String::from_utf8_lossy(&with.stderr));
    let without = home.command("once").output().expect("it runs");
    assert!(without.status.success(), "{without:?}");
    assert!(!String::from_utf8_lossy(&without.stderr).contains(LINE), "alerts were asked for without --alerts");
}

#[test]
fn a_verb_the_daemon_does_not_have_is_refused_before_anything_starts() {
    let home = Home::new("verbs");
    let out = home.command("dance").output().expect("it runs");
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stdout) + String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("dance"), "the refusal does not name the verb: {said}");
    // Nothing was started, so nothing was left behind.
    assert!(!home.socket().exists(), "a refused verb still bound the socket");
}

#[test]
fn a_flag_the_daemon_does_not_take_is_refused_before_anything_starts() {
    // PLAN 23. Both of the daemon's flags shape a capture session, and each
    // of these used to run one without it, exiting 0: a typo ran with alerts
    // off, a bare --record recorded nothing, and --alerts=no turned alerts
    // ON. Refused with 2 -- nothing was attempted -- and nothing bound,
    // published or created.
    let home = Home::new("flags");
    let record = home.base.join("record");
    for (flag, says) in [
        ("--alertz", "--alertz"),
        ("--alert", "--alert"),
        ("--record", "--record needs a directory"),
        ("--record=", "--record needs a directory"),
        ("--alerts=no", "--alerts takes no value"),
    ] {
        let out = home.command("once").arg(flag).output().expect("it runs");
        let said = String::from_utf8_lossy(&out.stdout) + String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{flag}: {said}");
        assert!(said.contains(says), "{flag}: the refusal does not say why: {said}");
        assert!(!home.socket().exists(), "{flag} still bound the socket");
        assert!(!home.state_file().exists(), "{flag} still published a document");
        assert!(!record.exists(), "{flag} made a recording directory");
    }
}

#[test]
fn the_reader_tells_a_running_daemon_from_the_document_it_left_behind() {
    // `qs-sony` is what the Devices tab, the Headphones page and the doctor
    // read. Two things it must get right that the file alone cannot tell it:
    // a daemon that is UP with no headset (the document says connected:false,
    // and "Start daemon" would start a second copy that refuses the lock),
    // and a daemon that is GONE behind a document that still says
    // connected:true (there is no signal handler, so a kill leaves it).
    let home = Home::new("reader");
    let daemon = Running(home.command("run").spawn().expect("the daemon starts"));
    wait_for("the socket to appear", || home.socket().exists());
    wait_for("the document to appear", || home.state_file().exists());

    let out = home.reader("devices");
    let devices: serde_json::Value =
        serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("devices printed JSON: {out:?}"));
    assert_eq!(devices["daemonRunning"], true, "{devices}");
    assert_eq!(devices["daemonReporting"], false, "no headset is connected here: {devices}");
    assert_eq!(devices["project"], "qs-sonyd", "{devices}");
    assert_eq!(devices["devices"][0]["name"], "WH-1000XM5", "{devices}");
    assert_eq!(devices["devices"][0]["action"], "configure", "a running daemon was offered a start: {devices}");
    let out = home.reader("status");
    let status: serde_json::Value = serde_json::from_slice(&out.stdout).expect("status printed JSON");
    assert_eq!(status["available"], true, "{status}");

    // Killed without ceremony, as a crash or a kill -9 would; the document
    // is rewritten as the last thing a connected daemon might have said.
    drop(daemon);
    std::fs::write(home.state_file(), "{\"schema_version\":1,\"connected\":true,\"phase\":\"ready\"}\n")
        .expect("a stale document");

    let out = home.reader("available");
    assert!(!out.status.success(), "a document with no daemon behind it was called available");
    let said: serde_json::Value = serde_json::from_slice(&out.stdout).expect("available printed JSON");
    assert_eq!(said["available"], false, "{said}");
    let out = home.reader("status");
    let status: serde_json::Value = serde_json::from_slice(&out.stdout).expect("status printed JSON");
    assert_eq!(status["available"], false, "{status}");
    assert_eq!(status["reason"], "stale", "{status}");
    let out = home.reader("devices");
    let devices: serde_json::Value = serde_json::from_slice(&out.stdout).expect("devices printed JSON");
    assert_eq!(devices["daemonRunning"], false, "{devices}");
    assert_eq!(devices["daemonReporting"], false, "a dead daemon's document was called reporting: {devices}");
    assert_eq!(devices["devices"][0]["action"], "start", "{devices}");
}
