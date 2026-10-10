//! `sony::atomic_write` and `sony::ensure_private_dir`, against a real
//! filesystem.
//!
//! These publish the state document: one JSON object per line, 0600 in a 0700
//! directory, replaced in place while anything on the machine may be reading
//! it. The properties below are the ones a reader depends on and that a
//! plausible-looking implementation does not give you.
//!
//! The symlink case is the reason this file exists. The temporary's name is
//! PREDICTABLE -- it carries the pid -- and an `OpenOptions::create(true)`
//! follows a symlink, so a create-and-truncate open writes the document
//! through whatever a pre-existing `status.tmp.<pid>` points at. The directory
//! is 0700, which makes it a hardening measure rather than a hole; it is still
//! a class of bug that costs nothing to close and cannot be closed later by
//! anyone who has not thought about it.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

fn work(name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("qs-sony-publish-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("a working directory");
    base
}

fn mode_of(path: &Path) -> u32 {
    fs::metadata(path).expect("it exists").permissions().mode() & 0o777
}

#[test]
fn the_document_lands_with_the_modes_a_reader_depends_on() {
    let base = work("modes");
    let dir = base.join("state").join("sony-headphones");
    let file = dir.join("status.json");

    sony_xm5::sony::atomic_write(&file, "{\"schema_version\":1}\n").expect("it publishes");

    assert_eq!(fs::read_to_string(&file).expect("readable"), "{\"schema_version\":1}\n");
    assert_eq!(mode_of(&file), 0o600, "the document must not be group- or world-readable");
    assert_eq!(mode_of(&dir), 0o700, "nor may the directory be enterable");
    // The parent it had to create on the way is private too: a 0700 leaf
    // inside a 0755 parent still lets anyone list the directory's name, which
    // is not the leak but is the thing people assume was handled.
    assert_eq!(mode_of(&base.join("state")), 0o700);
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn a_symlink_at_the_temporary_is_removed_rather_than_written_through() {
    // THE ONE THIS FILE IS FOR. Plant a symlink where the temporary will go,
    // pointing at a file that must not change, and publish.
    let base = work("symlink");
    let dir = base.join("state");
    fs::create_dir_all(&dir).expect("the directory");
    let file = dir.join("status.json");
    let target = base.join("do-not-touch");
    fs::write(&target, "original\n").expect("the target");

    let tmp = dir.join(format!("status.tmp.{}", std::process::id()));
    std::os::unix::fs::symlink(&target, &tmp).expect("the symlink");

    sony_xm5::sony::atomic_write(&file, "new\n").expect("it publishes anyway");

    assert_eq!(
        fs::read_to_string(&target).expect("still readable"),
        "original\n",
        "the document was written through a symlink at the temporary"
    );
    assert_eq!(fs::read_to_string(&file).expect("readable"), "new\n");
    assert!(!tmp.exists(), "the temporary was left behind");
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn debris_from_a_crash_does_not_block_the_next_write() {
    // `panic = "abort"` in release means a crash between the open and the
    // rename leaves the temporary behind. With `create_new` and no recovery,
    // the next run with the same pid would fail forever.
    let base = work("debris");
    let dir = base.join("state");
    fs::create_dir_all(&dir).expect("the directory");
    let file = dir.join("status.json");
    let tmp = dir.join(format!("status.tmp.{}", std::process::id()));
    fs::write(&tmp, "half a document").expect("the debris");

    sony_xm5::sony::atomic_write(&file, "whole\n").expect("it publishes over the debris");
    assert_eq!(fs::read_to_string(&file).expect("readable"), "whole\n");
    assert!(!tmp.exists());
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn publishing_twice_replaces_rather_than_appends() {
    let base = work("replace");
    let file = base.join("state").join("status.json");
    sony_xm5::sony::atomic_write(&file, "first\n").expect("one");
    sony_xm5::sony::atomic_write(&file, "second\n").expect("two");
    assert_eq!(fs::read_to_string(&file).expect("readable"), "second\n");
    // And the mode survives the replacement -- a rename over an existing file
    // takes the SOURCE's mode, so this is asserting the temporary's 0600 and
    // not the original's.
    assert_eq!(mode_of(&file), 0o600);
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn a_directory_that_already_exists_wide_open_is_closed() {
    // The case a create-only path leaves alone, and the reason
    // ensure_private_dir chmods unconditionally.
    let base = work("preexisting");
    let dir = base.join("state");
    fs::create_dir_all(&dir).expect("the directory");
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).expect("wide open");
    assert_eq!(mode_of(&dir), 0o755);

    sony_xm5::sony::atomic_write(&dir.join("status.json"), "x\n").expect("it publishes");
    assert_eq!(mode_of(&dir), 0o700, "an existing directory was left readable");
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn a_symlinked_directory_is_refused_rather_than_chmodded() {
    // THE EXPERIMENT, KEPT. Before the fix this passed silently and left
    // `target` at 0700: `DirBuilder::recursive(true)` treats EEXIST as success
    // after a `stat` that resolves the link, and `fs::set_permissions` then
    // chmods what the link points at. The daemon would have published
    // status.json inside it and bound daemon.sock and daemon.lock there too.
    //
    // Who can plant such a link: whoever can write the parent. Under
    // $XDG_RUNTIME_DIR that is only this user. Under the documented fallback
    // /tmp/sony-headphones-<uid> it is any local user, because /tmp is
    // world-writable and its sticky bit stops them deleting our directory, not
    // creating the name first.
    //
    // Reported by delarosa1312/omarchy-sony-xm5 (MIT), docs/hardening.md,
    // "The runtime directory", against their own daemon. Measured here.
    let base = work("symlinked-dir");
    let target = base.join("target");
    let link = base.join("link");
    fs::create_dir_all(&target).expect("the target");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).expect("wide open");
    std::os::unix::fs::symlink(&target, &link).expect("the link");

    let err = sony_xm5::sony::ensure_private_dir(&link).expect_err("a symlinked directory was accepted");
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied, "{err}");
    assert!(err.to_string().contains("symbolic link"), "{err}");
    assert_eq!(mode_of(&target), 0o755, "the symlink's target was chmodded through the link");

    // And the same through atomic_write, which is how the daemon reaches it:
    // nothing may be published inside the link's target.
    let err = sony_xm5::sony::atomic_write(&link.join("status.json"), "x\n")
        .expect_err("a document was published through a symlinked directory");
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied, "{err}");
    assert!(!target.join("status.json").exists(), "the document landed in the link's target");
    assert_eq!(mode_of(&target), 0o755);
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn a_path_with_no_parent_is_refused_rather_than_panicking() {
    let err = sony_xm5::sony::atomic_write(Path::new("/"), "x").expect_err("refused");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn an_unwritable_directory_reports_the_error_and_leaves_nothing() {
    // Running as root defeats mode-based denial, so this asserts the honest
    // property in both cases: either it failed, or it succeeded and left no
    // temporary. What it must never do is fail AND leave debris.
    let base = work("readonly");
    let dir = base.join("state");
    fs::create_dir_all(&dir).expect("the directory");
    let file = dir.join("status.json");
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).expect("read-only");

    let result = sony_xm5::sony::atomic_write(&file, "x\n");
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).expect("restore");
    let leftovers: Vec<String> = fs::read_dir(&dir)
        .expect("listable")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".tmp."))
        .collect();
    assert!(leftovers.is_empty(), "a failed write left {leftovers:?} behind");
    if result.is_ok() {
        assert_eq!(fs::read_to_string(&file).expect("readable"), "x\n");
    }
    let _ = fs::remove_dir_all(&base);
}
