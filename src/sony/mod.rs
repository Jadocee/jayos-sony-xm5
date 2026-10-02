//! The Sony headphone daemon: its state document, its command language and
//! where both of them live.
//!
//! WHY THESE PATHS AND NOT THE TREE'S OWN
//!
//! `$XDG_STATE_HOME/sony-headphones/status.json` and
//! `$XDG_RUNTIME_DIR/sony-headphones/daemon.sock` are the C++ daemon's paths,
//! kept deliberately rather than moved under jayos's `qs_core::paths`. Three things
//! depend on it: jayos's `doctor.d/50-hardware.sh` keys off that exact state path,
//! `qs-sony` carries a schema-version guard whose whole purpose is that the two
//! implementations are interchangeable, and anyone who already installed the
//! C++ daemon keeps working. Moving the paths would break interop for precisely
//! the people who cared enough to install the thing this replaces.

pub mod command;
pub mod link;
pub mod replay;
pub mod sdp;
pub mod serve;
pub mod session;
pub mod state;

use std::ffi::CString;
use std::fs;
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;

/// The version this daemon reports in `daemon_version`.
///
/// Its own, not the C++ project's: a reader that gates on this is asking which
/// implementation it is talking to, and answering "0.2.3" would be a lie about
/// provenance. The field a reader should gate on for SHAPE is
/// `schema_version`, which is the same 1 in both.
pub const DAEMON_VERSION: &str = concat!("qs-", env!("CARGO_PKG_VERSION"));

/// Sony models the daemon supports, matched against a device's name.
///
/// Matched on the model prefix rather than a full list: Sony's naming is
/// systematic (WH- over-ear, WF- true wireless, WI- neckband, LinkBuds, and
/// the ULT WEAR and INZONE lines, which carry no model code), and a list of
/// exact model numbers would go stale with every release. The seven are the
/// C++ daemon's own list (sony-headphones-linux mdr_backend.cpp, `prefixes`).
///
/// The point of matching at all is to offer the right thing: a Sony headset
/// gets an offer to install the daemon that can control it, and everything
/// else gets no offer rather than a button that installs software for hardware
/// it cannot talk to.
///
/// In the library, and public, because it is part of the contract: jayos's
/// Devices page makes the same offer on a machine where this package is not
/// installed yet -- so there is no `qs-sony` to ask -- and it does so from a
/// copy of this list. `contract/sony-prefixes.txt` is generated from it and a
/// jayos suite holds the page's copy to that file.
pub const SONY_PREFIXES: [&str; 7] = ["wh-", "wf-", "wi-", "linkbuds", "mdr-", "ult wear", "inzone"];

/// `$HOME`, or `/` when the environment has none -- which happens under a
/// systemd unit with an empty environment, and is worth not panicking over.
///
/// Public because `qs-sony` needs the same answer and is a separate crate: a
/// `pub(crate)` here cannot reach a binary. It was `qs_core::paths::home` in
/// jayos, which is not a dependency of this repository; the two bodies were
/// already identical, so one of them went.
#[doc(hidden)]
pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// `$XDG_STATE_HOME/sony-headphones`, or `~/.local/state/sony-headphones`.
///
/// An empty variable is treated as unset, which is what the reference does and
/// what the XDG spec asks for -- `XDG_STATE_HOME=` is not a request to write to
/// `/sony-headphones`.
pub fn state_dir() -> PathBuf {
    match std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()) {
        Some(v) => PathBuf::from(v).join("sony-headphones"),
        None => home().join(".local/state/sony-headphones"),
    }
}

pub fn state_file() -> PathBuf {
    state_dir().join("status.json")
}

/// `$XDG_RUNTIME_DIR/sony-headphones`, or `/tmp/sony-headphones-<uid>`.
///
/// The fallback carries the uid because `/tmp` is shared: without it, the first
/// user to start the daemon owns the path and every other user on the machine
/// is refused, which reads as the daemon being broken.
pub fn runtime_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        Some(v) => PathBuf::from(v).join("sony-headphones"),
        // SAFETY: getuid cannot fail and has no preconditions.
        None => PathBuf::from(format!("/tmp/sony-headphones-{}", unsafe { libc::getuid() })),
    }
}

pub fn socket_path() -> PathBuf {
    runtime_dir().join("daemon.sock")
}

/// Create `path` as a private directory, and make it one if it already exists.
///
/// The mode is set at CREATION as well as afterwards. `create_dir_all` applies
/// the umask, so a create-then-chmod leaves a window in which the directory
/// exists group- and world-readable; anything watching for it can open a
/// handle in that window and keep it across the chmod. The second call is not
/// redundant with the first: it is what fixes a directory that already existed
/// with the wrong mode.
///
/// A SYMLINK HERE IS REFUSED, NOT FOLLOWED, and this was a real hole.
///
/// The two lines above used to be the whole function, and both of them follow
/// a symlink. `DirBuilder::recursive(true)` treats EEXIST as success after a
/// `stat` that resolves the link, and `fs::set_permissions` chmods whatever
/// the link points at. MEASURED, not reasoned about: a 0755 directory reached
/// through a symlink came back 0700, and the daemon would then have published
/// `status.json` inside it and bound `daemon.sock` and `daemon.lock` there
/// too. `a_symlinked_directory_is_refused_rather_than_chmodded` is that
/// experiment, kept.
///
/// Who could plant the link: whoever can write the parent. For
/// `$XDG_RUNTIME_DIR/sony-headphones` that is only this user, so it is
/// hardening. For the documented fallback `/tmp/sony-headphones-<uid>` it is
/// ANY local user -- `/tmp` is world-writable, and its sticky bit stops them
/// deleting our directory, not creating the name before we do. `runtime_dir`
/// above already reasons about `/tmp` being shared and considered only the
/// collision, which is how this got here.
///
/// So: open the final component with `O_DIRECTORY|O_NOFOLLOW`, check it is
/// ours, and `fchmod` the DESCRIPTOR rather than the path, so the mode lands
/// on the object that was checked. The descriptor is RETURNED so callers can
/// do every later create, replace and unlink relative to it — `openat`,
/// `renameat`, `unlinkat`, `fstatat` — and the check is atomic with every
/// operation that follows, not just with the chmod.
///
/// NOT OUR FINDING. `delarosa1312/omarchy-sony-xm5` (MIT, Copyright (c) 2026
/// Delarosa1312), `docs/hardening.md`, "The runtime directory" -- reported
/// there against their own daemon, prompted by a marketplace security review.
/// The hole it names is in this function too, and the experiment above is how
/// we established that rather than taking their word for it.
pub fn ensure_private_dir(path: &std::path::Path) -> std::io::Result<fs::File> {
    fs::DirBuilder::new().recursive(true).mode(0o700).create(path)?;

    let dir = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(dir) => dir,
        // BOTH ERRNOS, and the reason is a Linux quirk worth writing down.
        // `open(2)` says O_NOFOLLOW answers ELOOP for a symlink -- but with
        // O_DIRECTORY set as well, Linux answers ENOTDIR instead, which is
        // MEASURED rather than read: `a_symlinked_directory_is_refused_rather
        // _than_chmodded` asserted ELOOP and got ENOTDIR.
        //
        // Said in words, because "Too many levels of symbolic links" about a
        // path containing one link reads as a bug in the daemon rather than as
        // a refusal.
        //
        // The `lstat` tells a symlink from anything else, and in the ordinary
        // run only the symlink arm is reachable: a REGULAR file wearing this
        // name is refused above with EEXIST, because `DirBuilder::recursive`
        // swallows EEXIST only for a directory. The other arm is for the path
        // having become something else between that call and this one -- the
        // window this function narrows and does not close.
        Err(e) if matches!(e.raw_os_error(), Some(libc::ELOOP) | Some(libc::ENOTDIR)) => {
            let what = match fs::symlink_metadata(path) {
                Ok(m) if m.file_type().is_symlink() => {
                    "a symbolic link; this daemon will not write through one"
                }
                _ => "not a directory",
            };
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("{} is {what}", path.display()),
            ));
        }
        Err(e) => return Err(e),
    };

    // SAFETY: getuid cannot fail and has no preconditions.
    let ours = unsafe { libc::getuid() };
    let owner = dir.metadata()?.uid();
    if owner != ours {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("{} belongs to uid {owner}, not {ours}", path.display()),
        ));
    }

    dir.set_permissions(fs::Permissions::from_mode(0o700))?;
    Ok(dir)
}

fn c_name(name: &str) -> std::io::Result<CString> {
    CString::new(name.as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "null in filename"))
}

/// Publish `data` at `path`, atomically and readable only by this user.
///
/// Write, fsync, rename, fsync the directory.
///
/// WHAT EACH STEP IS FOR, because three of the four look like ceremony and are
/// not:
///
///   * `create_new` on the temporary, NOT create-and-truncate. The temporary's
///     name is predictable -- it carries the pid -- and `create` follows a
///     symlink, so a create-and-truncate open writes the status document
///     through whatever a pre-existing `status.tmp.<pid>` points at. The
///     directory is 0700, so that is a hardening measure rather than a hole,
///     but `create_new` closes the class outright and costs nothing.
///   * Removing a stale temporary and retrying ONCE. `panic = "abort"` in
///     release means a crash between open and rename leaves the temporary
///     behind, and the next run with the same pid would then fail forever.
///     Retried once, not looped: a second failure is a real one.
///   * `sync_all` on the file. Without it the rename can be durable while the
///     bytes are not, so a machine that loses power mid-write comes back to a
///     status file that exists and is empty -- which every reader here treats
///     as "the daemon has nothing to say" rather than as corruption.
///   * `sync_all` on the DIRECTORY, after the rename. Without it the bytes can
///     be durable while the rename is not, and the machine comes back to the
///     previous document plus a stray temporary. That is the less bad of the
///     two failures, which is presumably why it is the one usually left out;
///     it is still not what the sentence above promises.
///
/// What is still NOT guaranteed: that a reader sees either the old document or
/// the new one and never a partial. `rename` gives that within a filesystem,
/// and `path` and its temporary are always in the same directory, so it holds
/// here -- but it is a property of the call, not of this function, and a
/// caller that moves the temporary elsewhere loses it.
pub fn atomic_write(path: &std::path::Path, data: &str) -> std::io::Result<()> {
    atomic_write_bytes(path, data.as_bytes())
}

/// [`atomic_write`] for bytes that are not text: a captured frame, which
/// `capture::Recorder` writes the same way and for the same reason -- a
/// fixture half-written when the daemon was killed would be a frame nobody
/// sent.
pub fn atomic_write_bytes(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    let Some(parent) = path.parent() else {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent"));
    };
    let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no filename"));
    };
    let dir = ensure_private_dir(parent)?;
    let dir_fd = dir.as_raw_fd();

    // SAFETY: getpid cannot fail and has no preconditions.
    let stem = std::path::Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(file_name);
    let tmp_name = format!("{stem}.tmp.{}", unsafe { libc::getpid() });
    let c_tmp = c_name(&tmp_name)?;
    let c_final = c_name(file_name)?;

    // SAFETY for every openat/renameat/unlinkat below: dir_fd is a valid
    // directory descriptor held open by `dir`; the CStrings outlive each call.
    let create = || -> std::io::Result<fs::File> {
        let fd = unsafe {
            libc::openat(
                dir_fd,
                c_tmp.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(unsafe { fs::File::from_raw_fd(fd) })
    };

    let write = || -> std::io::Result<()> {
        let mut f = match create() {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = unsafe { libc::unlinkat(dir_fd, c_tmp.as_ptr(), 0) };
                create()?
            }
            Err(e) => return Err(e),
        };
        f.write_all(data)?;
        f.sync_all()
    };

    let publish = || -> std::io::Result<()> {
        write()?;
        if unsafe { libc::renameat(dir_fd, c_tmp.as_ptr(), dir_fd, c_final.as_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let _ = dir.sync_all();
        Ok(())
    };

    match publish() {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = unsafe { libc::unlinkat(dir_fd, c_tmp.as_ptr(), 0) };
            Err(e)
        }
    }
}
