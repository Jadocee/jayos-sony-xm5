//! The client socket, against a real one.
//!
//! Everything here runs over an actual `AF_UNIX` listener in a temporary
//! directory, with a real client on the other end. Nothing is simulated,
//! because every property worth asserting about this module is a property of
//! the socket rather than of the code around it: the modes the kernel records,
//! what a peer sees when the cap is reached, whether a silent client keeps its
//! slot, whether a second daemon can steal the path.
//!
//! The dispatcher is the part that is faked, and only that: it is a closure
//! answering lines, so the socket's behaviour can be tested without a headset.
//!
//! ONE PROPERTY IS NOT ASSERTED HERE, and it is better said than left to be
//! noticed: the `SO_PEERCRED` check refusing another user. A test for it would
//! have to connect as a different uid, which means either running as root or
//! a user namespace, and a suite that needs one of those is a suite that gets
//! skipped in the environment where it matters. What IS asserted is the thing
//! that actually keeps other users out -- 0600 in a 0700 directory -- and the
//! peer check exists precisely for the case where that turns out not to hold.

use sony_xm5::sony::serve::{self, BindError, Bound, Limits};
use sony_xm5::sony::session::ClientId;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn work(name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("qs-sony-serve-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).expect("a working directory");
    base
}

fn quick() -> Limits {
    // Short, because two of these tests spend the idle timeout waiting for it.
    Limits { clients: 4, line: 4096, idle: Duration::from_millis(300) }
}

/// Start a server on its own thread and hand back the socket path and a stop.
fn start<D: serve::Dispatch>(bound: Bound, dispatch: D, limits: Limits) -> (PathBuf, impl FnOnce()) {
    let path = bound.path().to_path_buf();
    let stopper = bound.stopper();
    let joined = std::thread::spawn(move || bound.serve(Arc::new(dispatch), limits));
    (path, move || {
        stopper.stop();
        let _ = joined.join();
    })
}

/// One command, one reply, as `qs-sony` does it.
fn ask(path: &std::path::Path, line: &str) -> std::io::Result<String> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\n")?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    Ok(reply.trim().to_string())
}

#[test]
fn a_command_is_answered_and_the_connection_closed() {
    let base = work("roundtrip");
    let socket = base.join("daemon.sock");
    let bound = serve::bind(&socket).expect("it binds");
    let (path, stop) = start(bound, |_: ClientId, line: &str| format!("ok {line}"), quick());

    assert_eq!(ask(&path, "noise anc").expect("a reply"), "ok noise anc");
    // Closed, not held: a second read on a fresh connection is a fresh
    // conversation, which is what one-command-per-connection means.
    assert_eq!(ask(&path, "status").expect("a reply"), "ok status");
    stop();
}

#[test]
fn the_socket_is_private_and_so_is_the_directory_holding_it() {
    // The peer check below is defence in depth. THIS is the thing that keeps
    // another user off the socket, and it is a mode rather than a check, so it
    // is worth asserting against the kernel rather than reading the source.
    let base = work("modes");
    let dir = base.join("sony-headphones");
    let socket = dir.join("daemon.sock");
    let bound = serve::bind(&socket).expect("it binds");
    let (path, stop) = start(bound, |_: ClientId, _: &str| "ok".to_string(), quick());

    let mode = |p: &std::path::Path| {
        std::fs::metadata(p).expect("it exists").permissions().mode() & 0o777
    };
    assert_eq!(mode(&path), 0o600, "the socket is reachable by another user");
    assert_eq!(mode(&dir), 0o700, "the directory holding it is not private");
    stop();
}

#[test]
fn a_second_daemon_is_refused_rather_than_stealing_the_socket() {
    // The failure this prevents: daemon two decides daemon one's socket is
    // debris, unlinks it and binds its own. Both processes are then running,
    // daemon one is listening on a path nothing can reach, and every client
    // reaches daemon two -- silently, with no error anywhere.
    let base = work("single");
    let socket = base.join("daemon.sock");
    let first = serve::bind(&socket).expect("the first binds");
    match serve::bind(&socket) {
        Err(BindError::AlreadyRunning) => {}
        Err(e) => panic!("the second daemon failed for the wrong reason: {e}"),
        Ok(_) => panic!("two daemons bound the same socket"),
    }
    // And the first one's socket is still there and still its own.
    assert!(socket.exists(), "the refused daemon removed the running one's socket");
    drop(first);
}

#[test]
fn a_socket_left_behind_by_a_dead_daemon_is_replaced() {
    // The other half. Refusing a stale socket would mean a daemon killed with
    // SIGKILL can never start again without someone deleting a file by hand.
    let base = work("stale");
    let socket = base.join("daemon.sock");
    let first = serve::bind(&socket).expect("the first binds");
    assert!(socket.exists());
    // Dropped without serving: the lock goes, the socket file stays. That is
    // exactly the state a killed daemon leaves.
    drop(first);
    assert!(socket.exists(), "the test did not reproduce a stale socket");

    let second = serve::bind(&socket).expect("a stale socket is not a permanent lockout");
    let (path, stop) = start(second, |_: ClientId, _: &str| "ok".to_string(), quick());
    assert_eq!(ask(&path, "status").expect("a reply"), "ok");
    stop();
}

#[test]
fn something_at_the_socket_path_that_is_not_a_socket_is_not_deleted() {
    // The lock says no live daemon OF OURS is listening. It says nothing about
    // a regular file, a symlink or a fifo wearing the name, and `remove_file`
    // would have deleted any of them: the test above is the case the removal
    // exists for, and this is the case it must not reach. Refusing to start is
    // the conservative direction -- a daemon that will not start is visible,
    // and somebody else's file silently disappearing is not.
    //
    // Reported by delarosa1312/omarchy-sony-xm5 (MIT), docs/hardening.md,
    // against their own daemon; the same hole was here.
    for (what, plant) in [
        ("a regular file", &(|p: &std::path::Path| std::fs::write(p, "not ours\n").expect("planted"))
            as &dyn Fn(&std::path::Path)),
        ("a symlink", &|p: &std::path::Path| {
            std::os::unix::fs::symlink("/dev/null", p).expect("planted");
        }),
        ("a directory", &|p: &std::path::Path| std::fs::create_dir(p).expect("planted")),
    ] {
        let base = work(&format!("notasocket-{}", what.replace(' ', "-")));
        let socket = base.join("daemon.sock");
        plant(&socket);

        // `Bound` is deliberately not Debug -- it holds a live listener and a
        // lock -- so the Err is matched out rather than unwrapped.
        let Err(err) = serve::bind(&socket) else {
            panic!("{what} at the socket path was accepted");
        };
        assert!(
            matches!(&err, serve::BindError::Io(e) if e.kind() == std::io::ErrorKind::AlreadyExists),
            "{what}: {err:?}"
        );
        assert!(
            std::fs::symlink_metadata(&socket).is_ok(),
            "{what} at the socket path was deleted"
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}

#[test]
fn an_over_long_command_is_refused_rather_than_buffered() {
    let base = work("toolong");
    let socket = base.join("daemon.sock");
    let bound = serve::bind(&socket).expect("it binds");
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let recorder = {
        let seen = Arc::clone(&seen);
        move |_: ClientId, line: &str| {
            seen.lock().expect("not poisoned").push(line.to_string());
            "ok".to_string()
        }
    };
    let limits = Limits { line: 64, ..quick() };
    let (path, stop) = start(bound, recorder, limits);

    // No newline, so a server that read until one would read forever.
    let mut stream = UnixStream::connect(&path).expect("connected");
    stream.set_read_timeout(Some(Duration::from_secs(3))).expect("deadline");
    stream.write_all(&vec![b'x'; 4096]).expect("written");
    let mut reply = String::new();
    let _ = stream.read_to_string(&mut reply);
    assert_eq!(reply.trim(), "error command too long");

    // And it never reached the dispatcher, which is the point: the limit is
    // enforced by the socket, not by the command parser downstream.
    assert!(seen.lock().expect("not poisoned").is_empty(), "an over-long line was dispatched");

    // AND WITH A NEWLINE, which is a different path through the reader: the
    // loop stops at the newline before it ever checks the length, so a line
    // over the limit that arrives complete in one read is caught only by the
    // check after the loop. Both checks are load-bearing and neither is the
    // other's spare.
    assert_eq!(ask(&path, &"y".repeat(200)).expect("a reply"), "error command too long");
    assert!(seen.lock().expect("not poisoned").is_empty(), "an over-long line was dispatched");

    // A line at exactly the limit is not over it.
    let edge = "x".repeat(64);
    assert_eq!(ask(&path, &edge).expect("a reply"), "ok");
    assert_eq!(seen.lock().expect("not poisoned").len(), 1);
    stop();
}

#[test]
fn more_clients_than_the_cap_are_told_so_rather_than_dropped() {
    // A closed connection with no reply is indistinguishable from a crashed
    // daemon, and `qs-sony` would report it as one. A busy daemon says it is
    // busy.
    let base = work("cap");
    let socket = base.join("daemon.sock");
    let bound = serve::bind(&socket).expect("it binds");

    // A gate rather than a channel: every dispatch waits on the same flag, so
    // releasing it releases the clients still holding slots AND lets the ones
    // that come afterwards through. A channel with one token per holder would
    // block the very connection this test uses to prove the slots came back.
    let open = Arc::new(AtomicBool::new(false));
    let entered = Arc::new(AtomicU64::new(0));
    let blocker = {
        let entered = Arc::clone(&entered);
        let open = Arc::clone(&open);
        move |_: ClientId, _: &str| {
            entered.fetch_add(1, Ordering::SeqCst);
            let deadline = Instant::now() + Duration::from_secs(5);
            while !open.load(Ordering::SeqCst) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            "ok".to_string()
        }
    };
    let limits = Limits { clients: 2, ..quick() };
    let (path, stop) = start(bound, blocker, limits);

    let mut held = Vec::new();
    for _ in 0..2 {
        let mut s = UnixStream::connect(&path).expect("connected");
        s.write_all(b"status\n").expect("written");
        held.push(s);
    }
    // Both are inside the dispatcher before the third connects, so the third
    // meets a full daemon rather than a race.
    let deadline = Instant::now() + Duration::from_secs(5);
    while entered.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(entered.load(Ordering::SeqCst), 2, "the two holders never reached the dispatcher");

    assert_eq!(ask(&path, "status").expect("a reply"), "error this daemon has too many clients");

    // Released, the slots come back and the daemon serves again.
    open.store(true, Ordering::SeqCst);
    for mut s in held {
        let mut reply = String::new();
        let _ = s.read_to_string(&mut reply);
        assert_eq!(reply.trim(), "ok");
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut served = false;
    while Instant::now() < deadline {
        if matches!(ask(&path, "status").as_deref(), Ok("ok")) {
            served = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(served, "a slot freed by a finished client was never reused");
    stop();
}

#[test]
fn a_client_that_connects_and_says_nothing_does_not_keep_its_slot() {
    // The cheapest denial there is: connect, send nothing, hold a thread. With
    // a cap of one, a daemon that waited forever would answer nobody again.
    let base = work("silent");
    let socket = base.join("daemon.sock");
    let bound = serve::bind(&socket).expect("it binds");
    let limits = Limits { clients: 1, idle: Duration::from_millis(200), ..quick() };
    let (path, stop) = start(bound, |_: ClientId, _: &str| "ok".to_string(), limits);

    let silent = UnixStream::connect(&path).expect("connected");
    // Held open and never written to.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut served = false;
    while Instant::now() < deadline {
        if matches!(ask(&path, "status").as_deref(), Ok("ok")) {
            served = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(served, "a client that sent nothing held the only slot until the test gave up");
    drop(silent);
    stop();
}

#[test]
fn each_client_is_told_apart_from_the_others() {
    // The engine routes replies by this id, so two clients sharing one would
    // get each other's answers.
    let base = work("ids");
    let socket = base.join("daemon.sock");
    let bound = serve::bind(&socket).expect("it binds");
    let (path, stop) = start(bound, |id: ClientId, _: &str| format!("ok {}", id.0), quick());

    let mut ids = Vec::new();
    for _ in 0..5 {
        ids.push(ask(&path, "status").expect("a reply"));
    }
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), ids.len(), "two clients shared an id: {ids:?}");
    stop();
}

#[test]
fn a_stopped_server_leaves_no_socket_behind() {
    let base = work("cleanup");
    let socket = base.join("daemon.sock");
    let bound = serve::bind(&socket).expect("it binds");
    let (path, stop) = start(bound, |_: ClientId, _: &str| "ok".to_string(), quick());
    assert!(path.exists());
    stop();
    assert!(!path.exists(), "a clean shutdown left the socket for the next daemon to clear");
}

#[test]
fn a_command_with_no_newline_is_cut_off_rather_than_buffered() {
    // The other half of the length limit, and the half that is about MEMORY
    // rather than about which lines are legal. A server that checked the
    // length only after reading to end of input would accept whatever a client
    // cared to send first -- gigabytes of it, in this process's heap -- and
    // then refuse the line. The refusal would look identical from outside,
    // which is why this is asserted on what the client can PUSH rather than on
    // what it is told.
    let base = work("unbounded");
    let socket = base.join("daemon.sock");
    let bound = serve::bind(&socket).expect("it binds");
    let limits = Limits { line: 64, ..quick() };
    let (path, stop) = start(bound, |_: ClientId, _: &str| "ok".to_string(), limits);

    let mut writer = UnixStream::connect(&path).expect("connected");
    writer.set_write_timeout(Some(Duration::from_millis(500))).expect("deadline");
    let mut reader = writer.try_clone().expect("a second handle");
    reader.set_read_timeout(Some(Duration::from_secs(3))).expect("deadline");

    // Written from another thread, because the server answers while this is
    // still blocked -- and a client that could not read until it had finished
    // writing would deadlock rather than fail.
    let pushed = std::thread::spawn(move || {
        let chunk = vec![b'x'; 64 * 1024];
        let mut written = 0usize;
        // Well past any socket buffer, and past anything a bounded reader
        // would accept.
        while written < 16 * 1024 * 1024 {
            match writer.write(&chunk) {
                Ok(0) => break,
                Ok(n) => written += n,
                Err(_) => break,
            }
        }
        written
    });

    let mut reply = String::new();
    let _ = reader.read_to_string(&mut reply);
    assert_eq!(reply.trim(), "error command too long");

    let written = pushed.join().expect("the writer finished");
    // The socket's own buffers plus the closing drain, and nothing like the 16
    // MiB on offer. A server that read to end of input would have taken it all.
    assert!(
        written < 4 * 1024 * 1024,
        "the server buffered {written} bytes of a command it was going to refuse"
    );
    stop();
}
