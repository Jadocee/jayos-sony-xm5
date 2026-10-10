//! Argument parsing, to the shape the Python scripts already had.
//!
//! WHY NOT CLAP
//!
//! These programs replace scripts whose interface is already fixed by 140 call
//! sites in QML, bash and Lua. The job is not to design a command line; it is
//! to accept exactly the one that is already being sent. clap would impose its
//! own conventions -- `--help` output, error text, abbreviation matching,
//! `--` handling -- on interfaces that were never designed to have them, and
//! its errors would go to stderr in a format the QML callers do not read.
//!
//! It also has a cost these programs specifically exist to avoid: the whole
//! reason for the Rust rewrite is that a helper invoked on every keystroke
//! should not spend milliseconds getting started.
//!
//! So: a small parser that does what `sys.argv` did.
//!
//! AN UNKNOWN FLAG, AND WHO GETS TOLD
//!
//! `has()` answered out of a list of everything that looked like `--name`, and
//! nothing ever asked whether a name in that list was one the program knows.
//! So `qs-calendar-sync --chek` fetched the calendar instead of running the
//! check, and `qs-aur audit --dry-run` -- a flag that does not exist -- audited
//! for real. The typo was invisible in both directions: no message, and the
//! program did the thing the flag was passed to prevent.
//!
//! [`Args::unknown_flags`] makes it answerable. What a helper then DOES with
//! the answer is not one decision, because the callers are not one kind:
//!
//!   A shell script, a keybind or a person reads the exit status and stderr.
//!   Refusing is both actionable and safe -- the caller's `|| fallback` fires,
//!   and the person is told what to type instead. [`Args::reject_unknown`].
//!
//!   A QML `Process` reads stdout and NOTHING else -- not the exit status, not
//!   stderr (see `emit`). For those, the shape of stdout is the only channel,
//!   and turning a typo into a failure envelope replaces a widget showing
//!   slightly the wrong thing with a widget showing nothing at all, while the
//!   explanation goes to a log. So they say it on stderr, where Quickshell
//!   records it, and still answer. [`Args::note_unknown`].
//!
//! WHAT STATUS A REFUSAL IS: 2, EVERYWHERE
//!
//! Refusing had two statuses for one policy. The six helpers below reached it
//! through `emit::unknown_flag`, which returned `ExitCode::FAILURE` -- 1 --
//! while the helpers that hand-roll a parser answered 2 for the identical
//! mistake: `qs-controllers --alll`, `qs-steam-downloads --alll`,
//! `qs-focus-stats --appp` and `qs-localsend port-free 53317 --quiet` all
//! exited 2 where `qs-calendar-sync --chek` exited 1. Same refusal, same
//! reason, two answers, decided by which parser the helper happened to use.
//!
//! It is 2. A refused flag is not a failed run: nothing was opened, nothing was
//! fetched, no answer was computed, so there is no result to report -- which is
//! what 2 means everywhere else in this tree (`qs-check`'s report header spells
//! the same 0/1/2 contract out for the checkers). 1 is already spoken for in
//! these programs and means something else in each: `emit::failure` returns it
//! for "I ran, and the answer is that it failed", and `qs-localsend port-free`
//! returns it for "the port is taken" -- a refusal answering 1 there answers a
//! question nobody asked. 2 is also what argparse returned for a usage error,
//! which is what the shell callers of the hand-rolled helpers were written
//! against, so settling on it changed no call site.
//!
//! The number lives in `emit::REFUSED`, and `emit::unknown_flag` is the only
//! thing that reads it, so the hand-rolled parsers agree by reporting through
//! that function rather than by each remembering a digit.
//!
//! WHICH HELPER GETS WHICH, AND WHY
//!
//! The dividing line is not "is it called from QML" -- it is what the flag was
//! passed to PREVENT, and whether any call site in this tree passes a flag from
//! a `Process` block at all.
//!
//! Refusing (`reject_unknown`). Every flag here restrains the program, so
//! ignoring one makes it do the larger thing the user was trying to avoid --
//! and NO `Process` block passes any of them, so the refusal cannot reach a
//! widget:
//!
//!   qs-aur              --force skips PKGBUILD vetting and --remove deletes
//!                       packages; `audit --dry-run` is the reported case.
//!                       updates.sh reads the exit status (`|| exit 1`).
//!   qs-calendar-sync    --check diagnoses instead of fetching.
//!                       CalendarPopup.qml runs it with no flags at all.
//!   qs-theme-palette    --print prints the palette instead of WRITING it, and
//!                       Theme.qml watches that file. Two shell callers only.
//!   qs-keymapp-layers   --stdin reads the envelope on stdin instead of opening
//!                       Keymapp's database. moonlander.sh only.
//!   qs-osk-controller   --probe and --devices are the diagnostics docs/OSK.md and
//!                       doctor tell people to run; ControllerService.qml
//!                       starts it bare, as a daemon.
//!   qs-wallpaper-search ddg_search.sh is the only caller, and its unknown-verb
//!                       arm already prints an envelope on this program's
//!                       otherwise tab-separated stdout for exactly this reason.
//!   qs-localsend        `send --pin` and `--checksum` are what authenticates a
//!                       transfer and what proves it arrived intact, and
//!                       `daemon --interval` is a broadcast rate. It reaches
//!                       this policy through its own parser rather than
//!                       through `reject_unknown` -- see the hand-rolled list
//!                       below -- and no Process passes it a flag:
//!                       IslandService.qml and autostart.lua both start
//!                       `daemon` bare, and localsend-setup.sh only asks
//!                       `port-free`.
//!
//! Saying so and carrying on (`note_unknown`). A `Process` block passes these
//! as literals, and each only narrows or widens a read-only answer, so ignoring
//! one gives a different amount of the right thing rather than a wrong kind of
//! thing -- while refusing would give the widget an envelope its binding cannot
//! render:
//!
//!   qs-arch-news        HomePage.qml runs `list --unread`.
//!   qs-steam-library    QuickLaunch.qml runs `recent --limit 3`.
//!   qs-connections      CanaryPage.qml appends `--all` when its toggle is on.
//!
//! Not opted in, for two different reasons.
//!
//! The helpers that read no flags at all: every one is polled by a widget, none
//! has a flag to mistype, and declaring an empty set for each would add thirty
//! call sites whose only content is a promise to keep a list empty.
//!
//! The one helper that reads a flag and deliberately ignores an unknown one:
//! `qs-lock-osd`. `--probe` turns a daemon into a diagnostic, so a mistyped
//! `--prob` starts the daemon that lock-osd.sh wanted anyway -- and refusing
//! would answer a typo in a login script by leaving the Caps Lock OSD switched
//! off for the session. It says so at its `has("probe")` call; it is named here
//! so this table is the whole list rather than most of it.
//!
//! And the five that hand-roll a parser because `Args` cannot express what
//! their callers send -- `qs-controllers`, `qs-steam-downloads`,
//! `qs-focus-stats`, `qs-localsend` and `qs-obsidian-search`. Each says beside
//! its parser why it is not `Args`, and the first four reject what they do not
//! recognise.
//!
//! `qs-localsend` was the one this table missed. Hand-rolling is what put it
//! out of reach of `reject_unknown`, and being out of reach is what kept it off
//! the list -- so `send`'s catch-all went on pushing an unrecognised `--name`
//! onto the POSITIONAL list, where it became the peer or a filename. It now
//! asks the same question through its own `check_options`, which also covers
//! the case `Args` cannot even represent: a value handed to a switch,
//! `--checksum=yes`, whose NAME is known and which was therefore waved through
//! into the file list.
//!
//! `qs-obsidian-search` does NOT, and it is named here rather than left to
//! read as one of a rejecting four, because this table is where someone looks
//! to find out whether a typo there is caught. It is not: everything after the
//! program name is the query a person typed after `/o` in the launcher, so a
//! leading `--` has to survive into the search text. `--vaultt` is therefore
//! searched FOR, and answers with an empty result list and exit 0 -- the same
//! answer as a vault holding nothing that matches. That is the failure this
//! header opens with, invisible in both directions, and it is accepted here
//! anyway: refusing would drop a real query that begins with `--`, and a
//! launcher that silently discards what was typed is the worse of the two. Its
//! own header points back here rather than restating the argument.
//!
//! `qs-controllers` and `qs-steam-downloads` are worth naming for the opposite
//! reason: they refuse on a flag a `Process` DOES pass -- the opposite of the
//! choice made above, and deliberately so.
//! Their flags carry the poll interval of a daemon a widget starts, so ignoring
//! one leaves it running at a rate nobody chose, forever, on a machine whose
//! battery notices. Ignoring `--unread` only shows more news.
//!
//! The declared list is the risk this carries: a flag a caller really does pass
//! that is missing from it turns a working call site into a refusal. That is
//! why the list sits beside the `has()` calls that read it, and why no refusing
//! helper is one a `Process` block passes a flag to -- the three that are, note
//! and carry on.

use super::emit;
use std::env;
use std::ffi::OsString;
use std::process::ExitCode;

/// The arguments a helper was invoked with, split into a subcommand, flags and
/// positionals.
///
/// Flags are `--name` or `--name=value`. A bare `--name value` is NOT treated
/// as taking a value, because the Python scripts did not: `--unread` was a
/// flag and `--limit 3` was two arguments the script read positionally. Guessing
/// differently here would silently swallow a positional.
#[derive(Debug, Clone, Default)]
pub struct Args {
    /// The first non-flag argument, which every multi-command script used as
    /// its verb.
    pub command: Option<String>,
    positionals: Vec<String>,
    flags: Vec<(String, Option<String>)>,
    /// The program name, for usage messages.
    pub program: String,
}

impl Args {
    /// Parse the process arguments.
    pub fn from_env() -> Self {
        let mut it = env::args_os();
        let program = it
            .next()
            .map(|p| {
                std::path::Path::new(&p)
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_else(|| p.to_string_lossy().into_owned())
            })
            .unwrap_or_default();
        Self::parse(program, it)
    }

    /// Parse an explicit argument list. Used by the tests, and by any helper
    /// that dispatches to a subcommand implementation.
    pub fn parse(program: String, args: impl IntoIterator<Item = OsString>) -> Self {
        let mut out = Args { program, ..Default::default() };

        for raw in args {
            // Lossy rather than rejecting: a filename that is not UTF-8 is
            // rare but real, and refusing to run is a worse answer than
            // handling the path with a replacement character in a message.
            let arg = raw.to_string_lossy().into_owned();

            if let Some(rest) = arg.strip_prefix("--") {
                if rest.is_empty() {
                    continue;
                }
                match rest.split_once('=') {
                    Some((k, v)) => out.flags.push((k.to_string(), Some(v.to_string()))),
                    None => out.flags.push((rest.to_string(), None)),
                }
                continue;
            }

            if out.command.is_none() {
                out.command = Some(arg);
            } else {
                out.positionals.push(arg);
            }
        }

        out
    }

    /// The subcommand, or a default when none was given.
    pub fn command_or(&self, default: &'static str) -> &str {
        self.command.as_deref().unwrap_or(default)
    }

    /// Whether `--name` was passed, with or without a value.
    pub fn has(&self, name: &str) -> bool {
        self.flags.iter().any(|(k, _)| k == name)
    }

    /// The value of `--name=value`, if it was given that way.
    pub fn value(&self, name: &str) -> Option<&str> {
        self.flags.iter().find(|(k, _)| k == name).and_then(|(_, v)| v.as_deref())
    }

    /// The value of `--name=value` or, failing that, `--name value`.
    ///
    /// Only for flags whose callers genuinely use the separated form --
    /// `--limit 3` is one. Using it for a flag that does not take a value
    /// eats the next positional.
    pub fn value_or_next(&self, name: &str) -> Option<&str> {
        if let Some(v) = self.value(name) {
            return Some(v);
        }
        if !self.has(name) {
            return None;
        }
        self.positionals.first().map(String::as_str)
    }

    /// A numeric flag value, or the default when absent or unparseable.
    ///
    /// Unparseable falls back rather than failing: these are called from QML
    /// string interpolation, where an empty binding produces `--limit=` and
    /// refusing would take a widget down over a value it has a sane default
    /// for.
    pub fn number(&self, name: &str, default: i64) -> i64 {
        self.value_or_next(name).and_then(|v| v.trim().parse().ok()).unwrap_or(default)
    }

    /// The positional at `index`, counting after the subcommand.
    pub fn positional(&self, index: usize) -> Option<&str> {
        self.positionals.get(index).map(String::as_str)
    }

    /// A positional parsed as a number, with a default.
    pub fn positional_number(&self, index: usize, default: i64) -> i64 {
        self.positional(index).and_then(|v| v.trim().parse().ok()).unwrap_or(default)
    }

    /// Every positional after the subcommand.
    pub fn rest(&self) -> &[String] {
        &self.positionals
    }

    /// The `--name` flags that were given and are not in `known`, in the order
    /// they were given.
    ///
    /// A pure function over what was parsed, so both policies below and the
    /// tests ask the same question. `known` is spelled without the dashes,
    /// matching [`has`](Self::has) and [`value`](Self::value) -- the dashes are
    /// added back when a name is printed.
    ///
    /// Duplicates are kept. `--chek --chek` was typed twice and naming it once
    /// would leave half the command line unexplained.
    ///
    /// A bare `--` is not a flag and never reaches here; `parse` drops it.
    pub fn unknown_flags(&self, known: &[&str]) -> Vec<String> {
        self.flags
            .iter()
            .map(|(name, _)| name)
            .filter(|name| !known.contains(&name.as_str()))
            .cloned()
            .collect()
    }

    /// Refuse to run when a flag was not recognised.
    ///
    /// `Some(status)` is this program's whole answer -- return it. `None` means
    /// every flag was recognised and there is nothing to do.
    ///
    /// The status is `emit::REFUSED`, which is 2 and not 1: see the module
    /// header. The hand-rolled parsers reach the same number by reporting
    /// through the same `emit::unknown_flag`.
    ///
    /// For a helper whose callers read the exit status: a shell script, a
    /// keybind, doctor, a person at a terminal. See the decision table in the
    /// module header for which helpers those are, and why the ones a widget
    /// polls do not use this.
    #[must_use]
    pub fn reject_unknown(&self, known: &[&str]) -> Option<ExitCode> {
        let unknown = self.unknown_flags(known);
        if unknown.is_empty() {
            return None;
        }
        Some(emit::unknown_flag(&self.program, &unknown, known))
    }

    /// Say a flag was not recognised, on stderr, and carry on.
    ///
    /// For a helper a QML `Process` polls with a flag written into the call
    /// site. Stdout keeps its shape, so the widget still renders; the typo is
    /// named in Quickshell's log, which is where someone debugging that widget
    /// is already looking.
    pub fn note_unknown(&self, known: &[&str]) {
        let unknown = self.unknown_flags(known);
        if !unknown.is_empty() {
            emit::note_unknown_flag(&self.program, &unknown, known);
        }
    }
}

/// Every argument after the program name, decoded the way [`Args`] decodes them.
///
/// For the handful of helpers that want the raw argv rather than the
/// command/flag/positional split -- a fixed positional list, or a verb plus an
/// unparsed tail handed to a subcommand's own reader.
///
/// It exists because `std::env::args()` PANICS on an argument that is not
/// UTF-8, and four helpers were calling it: `qs-check`, `qs-theme-merge`,
/// `qs-hypr-events` and `qs-localsend`. Under the release profile's
/// `panic = "abort"` that is a SIGABRT and a stripped backtrace on stderr,
/// with nothing on stdout -- so `qs-check all ~/.config/quickshell` over a
/// tree holding one file with a latin-1 name aborted the whole check run, and
/// `qs-localsend send` on a file copied off a FAT volume aborted instead of
/// reporting the file. `qs-localsend` had it both ways in one function: it
/// built an `Args` (which is lossy and survives) and then took argv again with
/// `env::args()` on the next line.
///
/// Lossy for the same reason [`Args::parse`] is: a name that is not UTF-8 is
/// rare but real, and a message with a replacement character in it is a better
/// answer than refusing to run -- and far better than no answer at all.
///
/// A helper whose arguments are PATHS should take `env::args_os()` itself and
/// keep them as `OsString`, because a lossy path no longer opens the file it
/// names. `qs-theme-merge` does.
pub fn argv_lossy() -> Vec<String> {
    lossy(env::args_os().skip(1))
}

/// The decoding half of [`argv_lossy`], split out so a test can hand it bytes
/// that no shell will produce on demand.
fn lossy(args: impl IntoIterator<Item = OsString>) -> Vec<String> {
    args.into_iter().map(|a| a.to_string_lossy().into_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Args {
        Args::parse("test".into(), args.iter().map(|a| OsString::from(*a)))
    }

    #[test]
    fn splits_command_flags_and_positionals() {
        let a = parse(&["list", "--unread", "--limit=5", "extra"]);
        assert_eq!(a.command.as_deref(), Some("list"));
        assert!(a.has("unread"));
        assert_eq!(a.value("limit"), Some("5"));
        assert_eq!(a.positional(0), Some("extra"));
        assert_eq!(a.number("limit", 1), 5);
    }

    #[test]
    fn separated_flag_values_are_opt_in() {
        // `recent --limit 3` is how QuickLaunch.qml calls steam-library.
        let a = parse(&["recent", "--limit", "3"]);
        assert_eq!(a.number("limit", 99), 3);
        // value() alone does not see it, which is what stops a valueless flag
        // from eating a positional.
        assert_eq!(a.value("limit"), None);
    }

    #[test]
    fn missing_and_unparseable_numbers_fall_back() {
        assert_eq!(parse(&["x"]).number("limit", 7), 7);
        assert_eq!(parse(&["x", "--limit="]).number("limit", 7), 7);
        assert_eq!(parse(&["x", "--limit=abc"]).number("limit", 7), 7);
        // Whitespace, because QML string interpolation produces it.
        assert_eq!(parse(&["x", "--limit= 4 "]).number("limit", 7), 4);
    }

    #[test]
    fn an_argument_that_is_not_utf8_is_decoded_rather_than_fatal() {
        // `env::args()` panics here, and under `panic = "abort"` that is a
        // SIGABRT with a stripped backtrace and nothing on stdout. Both readers
        // have to survive it: `Args`, which every helper uses, and `lossy`,
        // which the four that want raw argv use.
        use std::os::unix::ffi::OsStringExt;
        let bad = OsString::from_vec(b"\xff\xfe-bad".to_vec());

        let raw = lossy([OsString::from("send"), bad.clone()]);
        assert_eq!(raw.len(), 2);
        assert_eq!(raw[0], "send");
        assert!(raw[1].ends_with("-bad"), "the readable tail is kept: {:?}", raw[1]);

        let a = Args::parse("test".into(), [OsString::from("send"), bad]);
        assert_eq!(a.command.as_deref(), Some("send"));
        assert!(a.positional(0).is_some_and(|p| p.ends_with("-bad")));
    }

    #[test]
    fn a_bare_double_dash_is_ignored_not_treated_as_a_flag() {
        let a = parse(&["run", "--", "thing"]);
        assert_eq!(a.command.as_deref(), Some("run"));
        assert_eq!(a.positional(0), Some("thing"));
    }

    #[test]
    fn a_mistyped_flag_is_named_rather_than_ignored() {
        // The reported failure: `qs-calendar-sync --chek` did an ordinary
        // fetch, because `has("check")` was false and nothing asked why.
        let a = parse(&["--chek"]);
        assert!(!a.has("check"));
        assert_eq!(a.unknown_flags(&["check", "refresh"]), vec!["chek".to_string()]);
    }

    #[test]
    fn a_flag_the_program_knows_is_not_unknown_in_either_form() {
        // `--limit=5` and `--limit 3` are both the limit flag; only the name
        // half is compared, or the separated form would report itself.
        let a = parse(&["recent", "--limit=5", "--unread"]);
        assert!(a.unknown_flags(&["limit", "unread"]).is_empty());
        let b = parse(&["recent", "--limit", "3"]);
        assert!(b.unknown_flags(&["limit"]).is_empty());
        assert_eq!(b.positional(0), Some("3"), "the value is still a positional");
    }

    #[test]
    fn every_offender_is_named_not_just_the_first() {
        // A command line with two typos in it is one someone will retype from
        // this message, and a message naming half of them earns a second run.
        let a = parse(&["audit", "--dry-run", "--forse"]);
        assert_eq!(
            a.unknown_flags(&["force"]),
            vec!["dry-run".to_string(), "forse".to_string()]
        );
    }

    #[test]
    fn the_same_typo_twice_is_reported_twice() {
        let a = parse(&["--chek", "--chek"]);
        assert_eq!(a.unknown_flags(&["check"]), vec!["chek".to_string(), "chek".to_string()]);
    }

    #[test]
    fn a_program_that_takes_no_flags_finds_every_flag_unknown() {
        assert_eq!(parse(&["list", "--json"]).unknown_flags(&[]), vec!["json".to_string()]);
        assert!(parse(&["list"]).unknown_flags(&[]).is_empty());
    }

    #[test]
    fn a_bare_double_dash_is_not_an_unknown_flag() {
        // `parse` drops it, so nothing downstream can report the empty name as
        // an option -- `qs-localsend send -- ./file` must keep working.
        assert!(parse(&["send", "--", "./file"]).unknown_flags(&["to"]).is_empty());
    }

    #[test]
    fn a_positional_that_looks_like_a_flag_value_is_not_a_flag() {
        // The one that would make this unusable if it were wrong: a negative
        // number or a path is a positional, not a `--name`, so it can never be
        // reported as an unrecognised option.
        let a = parse(&["read", "-3", "/tmp/x"]);
        assert!(a.unknown_flags(&["unread"]).is_empty());
    }

    #[test]
    fn recognised_flags_produce_no_refusal() {
        // reject_unknown returning None is the "carry on" path, and it must be
        // the path for every call site in the tree today.
        assert!(parse(&["check", "--refresh"]).reject_unknown(&["check", "refresh"]).is_none());
        assert!(parse(&["--chek"]).reject_unknown(&["check"]).is_some());
    }
}
