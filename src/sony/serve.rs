//! The client socket: one command per connection, answered and closed.
//!
//! `qs-sony` is the only client that ships, and it does exactly one thing per
//! connection -- connect, write a line, read a reply, exit. That is the whole
//! protocol, and it is worth stating what it buys: there is no session, no
//! subscription and no ordering between clients, so a client that dies mid
//! command costs one connection and nothing else. The state document, not this
//! socket, is how a reader watches.
//!
//! WHAT THIS MODULE DOES NOT KNOW
//!
//! Anything about headsets. It takes a [`Dispatch`], hands it a line, and
//! writes back whatever it returns. The daemon's implementation forwards into
//! the session engine and waits for the reply; a test's answers directly. That
//! is what makes the socket's own behaviour -- the limits, the permissions,
//! the peer check, the timeouts -- testable without a Bluetooth adapter.
//!
//! WHY A LOCK FILE AND NOT JUST A BIND
//!
//! A Unix socket left behind by a daemon that was killed is indistinguishable
//! from one a running daemon is listening on: the path exists either way, and
//! `bind` fails with EADDRINUSE either way. The usual fix -- unlink it and bind
//! again -- is how two daemons end up running, each having decided the other's
//! socket was debris, with the second one's `bind` silently stealing every
//! client. So the socket is not the mutex. `daemon.lock` is, held with `flock`
//! for the life of the process, and only the holder is allowed to conclude
//! that a socket is stale.

use super::session::ClientId;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Answering one command line.
///
/// BLOCKING, AND EXPECTED TO RETURN PROMPTLY. The server does not put its own
/// deadline around this call, because it could not abandon it without a second
/// thread per connection and a leaked one for every slow answer. The daemon's
/// implementation bounds itself instead -- it waits on the engine with a
/// timeout and answers `error` when that expires -- which is where the
/// knowledge to pick a bound actually is.
pub trait Dispatch: Send + Sync + 'static {
    fn dispatch(&self, id: ClientId, line: &str) -> String;
}

impl<F> Dispatch for F
where
    F: Fn(ClientId, &str) -> String + Send + Sync + 'static,
{
    fn dispatch(&self, id: ClientId, line: &str) -> String {
        self(id, line)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Connections served at once.
    ///
    /// A thread apiece, and every one of them is a client waiting on a headset
    /// that answers in tens of milliseconds. The cap is what stops a loop in a
    /// shell script -- or a panel that re-reads on every frame -- from turning
    /// into a thread per iteration.
    pub clients: usize,
    /// The longest command line accepted, in bytes.
    ///
    /// `qs-sony` refuses to send more than this, so a line over it is either a
    /// different client or a probe. Bytes rather than characters, because that
    /// is what a socket read is measured in and what a limit has to bound.
    pub line: usize,
    /// How long a connection may hold a thread without finishing its command.
    ///
    /// A client that connects and says nothing is the cheapest way to hold a
    /// slot, so a slot is not held indefinitely for one.
    pub idle: Duration,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            clients: 16,
            // The same 4096 `qs-sony` bounds itself to.
            line: 4096,
            idle: Duration::from_secs(5),
        }
    }
}

/// A bound socket, and the lock that says this process may own it.
pub struct Bound {
    listener: UnixListener,
    path: PathBuf,
    /// Held, not used. Dropping it releases the flock, which is exactly what
    /// should happen when the daemon exits -- including by a signal, since the
    /// kernel closes the descriptor either way.
    _lock: std::fs::File,
    stop: Arc<AtomicBool>,
    /// The socket's directory, as the descriptor `ensure_private_dir`
    /// verified, its name in it, and the inode the bind made.
    ///
    /// KEPT FOR THE DAEMON'S LIFE, and that is the whole of plan 23's item 8.
    /// The bind went through this descriptor so a directory swapped after the
    /// check could not redirect it -- and then the descriptor was dropped, and
    /// the two operations that came later, the stopper's wake-up connection
    /// and the shutdown unlink, went back to the PATH. Whatever stood at the
    /// path by then is what they reached: the unlink could remove a socket
    /// this process never made, and the wake-up could connect to something
    /// else and leave `serve` blocked in `accept` for ever.
    here: Arc<Here>,
}

/// The socket as this process made it: where, under what name, which inode.
struct Here {
    dir: std::fs::File,
    name: String,
    ino: libc::ino_t,
}

impl Here {
    /// The socket by descriptor, for a connect: `/proc/self/fd/<N>/<name>`,
    /// which the kernel resolves through the open directory, not the path.
    fn by_descriptor(&self) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}/{}", self.dir.as_raw_fd(), self.name))
    }

    /// Unlink the socket if, and only if, it is still the one the bind made:
    /// a socket, at that name, in that directory, with that inode. Anything
    /// else at the name is somebody else's and is left alone.
    fn remove_if_ours(&self) {
        let Ok(c_name) = super::c_name(&self.name) else { return };
        let dir_fd = self.dir.as_raw_fd();
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: dir_fd is a directory descriptor this struct owns; c_name
        // outlives the call; stat is a live, writable struct.
        let rc =
            unsafe { libc::fstatat(dir_fd, c_name.as_ptr(), &mut stat, libc::AT_SYMLINK_NOFOLLOW) };
        if rc == 0 && (stat.st_mode & libc::S_IFMT) == libc::S_IFSOCK && stat.st_ino == self.ino {
            // SAFETY: as above.
            let _ = unsafe { libc::unlinkat(dir_fd, c_name.as_ptr(), 0) };
        }
    }
}

/// Why the socket could not be taken.
#[derive(Debug)]
pub enum BindError {
    /// Another daemon holds the lock. Not an error to work around: the socket
    /// it is listening on is serving clients correctly.
    AlreadyRunning,
    Io(std::io::Error),
}

impl std::fmt::Display for BindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BindError::AlreadyRunning => write!(f, "another sony daemon is already running"),
            BindError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<std::io::Error> for BindError {
    fn from(e: std::io::Error) -> BindError {
        BindError::Io(e)
    }
}

/// Take the lock, clear a stale socket, and bind.
///
/// The order matters and is the whole point of the lock: nothing unlinks a
/// socket it has not first proved nobody is listening on, and holding the lock
/// is that proof.
///
/// Every file operation after `ensure_private_dir` is done relative to the
/// directory descriptor it returns — `openat`, `fstatat`, `unlinkat`,
/// `fchmodat` — so a swap of the directory between the check and the
/// operations cannot redirect them.  The socket is bound through
/// `/proc/self/fd/<N>/<socket_name>`, which the kernel resolves through the
/// descriptor, not the pathname.
pub fn bind(socket: &Path) -> Result<Bound, BindError> {
    let Some(dir_path) = socket.parent() else {
        return Err(BindError::Io(std::io::Error::new(ErrorKind::InvalidInput, "no parent")));
    };
    let Some(socket_name) = socket.file_name().and_then(|n| n.to_str()) else {
        return Err(BindError::Io(std::io::Error::new(ErrorKind::InvalidInput, "no filename")));
    };
    let dir = super::ensure_private_dir(dir_path)?;
    let dir_fd = dir.as_raw_fd();

    let c_lock = super::c_name("daemon.lock")?;
    // SAFETY: dir_fd is a valid directory descriptor; c_lock outlives the call.
    let lock_fd = unsafe {
        libc::openat(
            dir_fd,
            c_lock.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_CLOEXEC,
            0o600,
        )
    };
    if lock_fd < 0 {
        return Err(BindError::Io(std::io::Error::last_os_error()));
    }
    let lock = unsafe { std::fs::File::from_raw_fd(lock_fd) };

    // LOCK_NB: a daemon that waited would sit in the background forever behind
    // a running one, which is worse than saying so.
    // SAFETY: flock's only precondition is a valid descriptor.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let e = std::io::Error::last_os_error();
        return match e.kind() {
            ErrorKind::WouldBlock => Err(BindError::AlreadyRunning),
            _ => Err(BindError::Io(e)),
        };
    }

    // We hold the lock, so any SOCKET here is debris from a process that no
    // longer exists.  AND IT REMOVES ONLY A SOCKET — see the long comment in
    // the git history for why.
    //
    // `fstatat` with `AT_SYMLINK_NOFOLLOW`, so a symlink is judged as a
    // symlink.  `unlinkat` removes only what was checked.
    //
    // NOT OUR FINDING, and the same source as `ensure_private_dir`:
    // `delarosa1312/omarchy-sony-xm5` (MIT, Copyright (c) 2026 Delarosa1312),
    // `docs/hardening.md`, "The runtime directory".
    let c_socket = super::c_name(socket_name)?;
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: dir_fd valid, c_socket outlives the call, stat is live.
    let rc = unsafe {
        libc::fstatat(dir_fd, c_socket.as_ptr(), &mut stat, libc::AT_SYMLINK_NOFOLLOW)
    };
    if rc == 0 {
        if (stat.st_mode & libc::S_IFMT) == libc::S_IFSOCK {
            // SAFETY: dir_fd valid, c_socket outlives the call.
            let _ = unsafe { libc::unlinkat(dir_fd, c_socket.as_ptr(), 0) };
        } else {
            return Err(BindError::Io(std::io::Error::new(
                ErrorKind::AlreadyExists,
                format!("{} exists and is not a socket", socket.display()),
            )));
        }
    } else {
        let e = std::io::Error::last_os_error();
        if e.kind() != ErrorKind::NotFound {
            return Err(BindError::Io(e));
        }
    }

    // Bind through `/proc/self/fd/<N>/<name>` so the kernel resolves the
    // directory through the descriptor, not the pathname.
    let proc_socket = format!("/proc/self/fd/{dir_fd}/{socket_name}");
    let listener = UnixListener::bind(&proc_socket)?;

    // AFTER the bind, because `bind` applies the umask and there is no
    // create-with-mode for a socket.
    // SAFETY: dir_fd valid, c_socket outlives the call.
    if unsafe { libc::fchmodat(dir_fd, c_socket.as_ptr(), 0o600, 0) } != 0 {
        return Err(BindError::Io(std::io::Error::last_os_error()));
    }

    // The inode the bind made, so shutdown can tell this socket from one
    // somebody put at the same name since.
    let mut made: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: dir_fd valid, c_socket outlives the call, made is live.
    if unsafe { libc::fstatat(dir_fd, c_socket.as_ptr(), &mut made, libc::AT_SYMLINK_NOFOLLOW) } != 0 {
        return Err(BindError::Io(std::io::Error::last_os_error()));
    }

    Ok(Bound {
        listener,
        path: socket.to_path_buf(),
        _lock: lock,
        stop: Arc::new(AtomicBool::new(false)),
        here: Arc::new(Here { dir, name: socket_name.to_string(), ino: made.st_ino }),
    })
}

impl Bound {
    /// Ask a running [`Bound::serve`] to return.
    ///
    /// Setting the flag is not enough on its own: the loop is blocked in
    /// `accept`, and nothing wakes it but a connection. So the stopper makes
    /// one and lets it close, which is also why `serve` re-checks the flag
    /// immediately after accepting rather than only at the top.
    pub fn stopper(&self) -> Stopper {
        Stopper { stop: Arc::clone(&self.stop), here: Arc::clone(&self.here) }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Serve until stopped. One thread per connection, capped.
    pub fn serve<D: Dispatch>(self, dispatch: Arc<D>, limits: Limits) {
        let live = Arc::new(AtomicUsize::new(0));
        let next = AtomicU64::new(1);
        // Our own uid, read once. A daemon cannot change uid mid-run, and
        // reading it per connection would only add a syscall to the hot path.
        // SAFETY: geteuid cannot fail and has no preconditions.
        let ours = unsafe { libc::geteuid() };

        for incoming in self.listener.incoming() {
            if self.stop.load(Ordering::SeqCst) {
                break;
            }
            let Ok(stream) = incoming else { continue };
            if self.stop.load(Ordering::SeqCst) {
                break;
            }

            // The peer check, before a thread is spent on it. The directory is
            // 0700 and the socket 0600, so this should be unreachable -- which
            // is the reason to check rather than the reason not to: if it ever
            // is reached, the permissions are not what this module believes.
            match peer_uid(&stream) {
                Ok(uid) if uid == ours => {}
                _ => continue,
            }

            // Over the cap the client is TOLD, not dropped. A closed connection
            // with no reply reads as a broken daemon; this reads as a busy one.
            //
            // On the accept thread, with its own short grace rather than the
            // idle limit: this path exists because the daemon is already busy,
            // and a refusal that could hold the acceptor for seconds would
            // make a flood of over-cap clients into a way to stop accepting
            // the under-cap ones.
            if live.load(Ordering::SeqCst) >= limits.clients {
                let mut stream = stream;
                farewell(&mut stream, "error this daemon has too many clients", BUSY_GRACE);
                continue;
            }

            live.fetch_add(1, Ordering::SeqCst);
            let dispatch = Arc::clone(&dispatch);
            let live_now = Arc::clone(&live);
            let id = ClientId(next.fetch_add(1, Ordering::SeqCst));
            let spawned = std::thread::Builder::new()
                .name("qs-sonyd-client".to_string())
                .spawn(move || {
                    converse(stream, id, dispatch.as_ref(), limits);
                    live_now.fetch_sub(1, Ordering::SeqCst);
                });
            if spawned.is_err() {
                // Out of threads. The count must come back down or the daemon
                // answers "too many clients" forever after.
                live.fetch_sub(1, Ordering::SeqCst);
            }
        }
        // The socket outlives the process only as debris, and the next daemon
        // to hold the lock removes it. Removing it here too means the common
        // case leaves nothing behind -- through the directory descriptor and
        // only if the inode is still ours. See `Bound::here`.
        self.here.remove_if_ours();
    }
}

pub struct Stopper {
    stop: Arc<AtomicBool>,
    here: Arc<Here>,
}

impl Stopper {
    /// Ask the server to return, and wake it.
    ///
    /// The wake-up connects through the directory descriptor, so it reaches
    /// this server's socket even if the path now names something else.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = UnixStream::connect(self.here.by_descriptor());
    }
}

/// One connection: read a line, answer it, close.
fn converse<D: Dispatch + ?Sized>(mut stream: UnixStream, id: ClientId, dispatch: &D, limits: Limits) {
    let _ = stream.set_read_timeout(Some(limits.idle));
    let _ = stream.set_write_timeout(Some(limits.idle));

    let line = match read_line(&mut stream, limits.line) {
        Ok(Some(line)) => line,
        // No command: a probe, a client that gave up, or a connection that
        // timed out. Nothing to answer and nothing to log.
        Ok(None) => return,
        Err(TooLong) => {
            farewell(&mut stream, "error command too long", limits.idle);
            return;
        }
    };

    let reply = dispatch.dispatch(id, &line);
    farewell(&mut stream, &reply, limits.idle);
}

/// The grace a REFUSED connection gets, as against a served one.
///
/// Short on purpose: see the refusal path in `serve`.
const BUSY_GRACE: Duration = Duration::from_millis(100);

/// Write the reply and close in a way the client can actually read.
///
/// THE DRAIN IS NOT POLITENESS. Closing a socket with unread bytes still in
/// its receive queue makes the kernel send RST rather than FIN, and an RST
/// discards whatever was already written in the other direction. So a daemon
/// that answers a client and closes without reading the rest of what the
/// client sent destroys its own reply: `qs-sony`, which writes a line and then
/// reads, gets ECONNRESET and reports the daemon as unreachable. It is a race,
/// which is worse than a bug that always happens -- it passes in a test and
/// fails on a loaded machine.
///
/// Every exit from a connection goes through here for that reason, including
/// the two that are refusals: a refusal the client cannot read is the same as
/// no answer at all.
///
/// Bounded twice, by bytes and by time, because the point is to clear what a
/// finished client left rather than to keep reading whatever it sends.
fn farewell(stream: &mut UnixStream, reply: &str, grace: Duration) {
    let _ = stream.set_write_timeout(Some(grace));
    let _ = stream.write_all(reply.as_bytes());
    let _ = stream.write_all(b"\n");
    let _ = stream.flush();
    // Says "nothing more is coming", so a client still writing stops rather
    // than filling the queue this is about to drain.
    let _ = stream.shutdown(std::net::Shutdown::Write);

    let _ = stream.set_read_timeout(Some(grace));
    let mut scratch = [0u8; 1024];
    let mut drained = 0usize;
    while drained < DRAIN_LIMIT {
        match stream.read(&mut scratch) {
            Ok(0) => break,
            Ok(n) => drained += n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}

/// How much a closing connection will read and throw away.
///
/// One command line's worth several times over. A client with more than this
/// still queued is not one this protocol has anything to say to, and at that
/// point an RST is the honest outcome.
const DRAIN_LIMIT: usize = 64 * 1024;

struct TooLong;

/// Read up to and including the first newline, or to end of input.
///
/// BOUNDED BY BYTES READ, not by the length of what is kept. A client sending
/// megabytes with no newline must be cut off at the limit rather than buffered
/// until it stops, so the check is on the accumulated buffer and happens on
/// every chunk.
///
/// End of input without a newline is accepted as a complete line, because
/// `qs-sony` writes its line and then reads -- it does not shut down its
/// writing half -- and a client that closes instead is saying the same thing.
fn read_line(stream: &mut UnixStream, limit: usize) -> Result<Option<String>, TooLong> {
    let mut buf: Vec<u8> = Vec::with_capacity(128);
    let mut chunk = [0u8; 512];
    loop {
        let n = match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            // A timeout surfaces as one of these two depending on how the
            // deadline was set; both mean the client stopped talking.
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => break,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        buf.extend_from_slice(&chunk[..n]);
        if let Some(at) = buf.iter().position(|b| *b == b'\n') {
            buf.truncate(at);
            break;
        }
        if buf.len() > limit {
            return Err(TooLong);
        }
    }
    if buf.len() > limit {
        return Err(TooLong);
    }
    // Lossy rather than a refusal: the command language is ASCII, so invalid
    // UTF-8 becomes a word the parser does not have and is refused there, with
    // a message about the command rather than about encoding.
    let line = String::from_utf8_lossy(&buf).trim().to_string();
    if line.is_empty() {
        return Ok(None);
    }
    Ok(Some(line))
}

/// The uid on the other end of `stream`, from the kernel rather than the peer.
///
/// SO_PEERCRED is filled in by the kernel at connect time from the connecting
/// process's real credentials. A client cannot claim someone else's, which is
/// what makes this worth asking at all.
fn peer_uid(stream: &UnixStream) -> std::io::Result<libc::uid_t> {
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `cred` is a live, correctly sized ucred and `len` describes it;
    // the descriptor is owned by `stream` for the duration of the call.
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut cred).cast::<libc::c_void>(),
            &raw mut len,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(cred.uid)
}
