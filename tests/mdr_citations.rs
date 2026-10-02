//! Every fixture this port's prose cites must exist, and must say what is
//! claimed beside it.
//!
//! WHY THIS TEST EXISTS
//!
//! Twice now, a comment naming a captured frame named the wrong one. Once the
//! fixture did not exist at all (`000149`, in a corpus that stops at `000138`);
//! once it existed but held a different message (`000109`, cited for
//! Speak-to-Chat, which is actually the head-gesture frame). Both times the
//! BYTES quoted beside the name were right, so every other test passed and no
//! decode was wrong.
//!
//! That is precisely why it matters. This module's whole method is "here is the
//! claim, here is the frame you can check it against". A citation nobody can
//! follow is the method failing quietly -- and the reader who follows it lands
//! on a different message and concludes the port is confused about the
//! protocol.
//!
//! So: scan the prose for fixture counters, and for each one, insist the
//! fixture is there and that any bytes quoted with it are really in it.

use sony_xm5::mdr::frame;
use std::fs;
use std::path::PathBuf;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every file whose prose is allowed to cite a fixture.
const PROSE: &[&str] = &[
    "src/mdr/payload.rs",
    "src/mdr/frame.rs",
    "src/mdr/mod.rs",
    "src/sony/state.rs",
    "src/sony/command.rs",
    "src/sony/mod.rs",
    "tests/mdr_payload.rs",
    "tests/mdr_frame.rs",
    "tests/mdr_support_functions.rs",
    "tests/mdr_encode.rs",
    "src/mdr/encode.rs",
    "tests/sony_contract.rs",
    "tests/fixtures/mdr/PROVENANCE.md",
    "docs/OUTSTANDING.md",
    "docs/EVIDENCE.md",
    "docs/CAPTURE.md",
];

/// The payload of the fixture whose counter is `counter`, if there is one.
/// What a citation may quote: the decoded payload, or the frame on the wire.
///
/// BOTH, because this file's prose quotes both. A message is usually cited by
/// its payload (`67 17 01 00 01 00 14`) but a frame-level claim -- the two ACK
/// byte strings, an escape, a length header -- is cited whole. Checking only
/// the payload would silently pass every frame-level quote, which is the
/// "nothing looked at is not the same as nothing wrong" failure this suite
/// exists to avoid.
struct Fixture {
    payload: Vec<u8>,
    wire: Vec<u8>,
}

impl Fixture {
    /// Whether `run` is a contiguous slice of either form.
    fn contains(&self, run: &[u8]) -> bool {
        self.payload.windows(run.len()).any(|w| w == run)
            || self.wire.windows(run.len()).any(|w| w == run)
    }
}

fn fixture_payload(counter: &str) -> Option<Fixture> {
    let dir = crate_dir().join("tests/fixtures/mdr");
    let needle = format!("-{counter}-");
    let name = fs::read_dir(&dir)
        .expect("the fixture directory is there")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.ends_with(".bin") && n.contains(&needle))?;
    let wire = fs::read(dir.join(name)).ok()?;
    let payload = frame::unpack(&wire).ok().map(|f| f.payload)?;
    Some(Fixture { payload, wire })
}

/// Six-digit runs that are a fixture counter rather than a hex byte or a year.
///
/// The corpus names them zero-padded to six digits, and nothing else in this
/// prose is a bare six-digit decimal run -- checked by the fact that this test
/// finds citations at all and none of them are false.
fn counters(text: &str) -> Vec<String> {
    let bytes: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let run: String = bytes[start..i].iter().collect();
            let before = start.checked_sub(1).and_then(|p| bytes.get(p)).copied();
            // Six digits, starting 000, and not glued to a word character --
            // which excludes hex literals and struct suffixes.
            if run.len() == 6 && run.starts_with("000") && before != Some('x') {
                out.push(run);
            }
        } else {
            i += 1;
        }
    }
    out.sort();
    out.dedup();
    out
}

/// A run of two or more `NN` hex tokens on one line: the bytes a citation
/// quotes. `|` and `--` split fields for a reader and are stepped over.
fn quoted_bytes(line: &str) -> Vec<Vec<u8>> {
    let mut runs: Vec<Vec<u8>> = Vec::new();
    let mut current: Vec<u8> = Vec::new();
    for token in line.split_whitespace() {
        let token = token.trim_matches(|c| c == '`' || c == ',' || c == ':');
        if token == "|" {
            continue;
        }
        let is_byte = token.len() == 2 && token.chars().all(|c| c.is_ascii_hexdigit());
        // A lone `00` inside prose is not a byte run; two in a row is.
        match (is_byte, u8::from_str_radix(token, 16)) {
            (true, Ok(byte)) => current.push(byte),
            _ => {
                if current.len() >= 2 {
                    runs.push(std::mem::take(&mut current));
                } else {
                    current.clear();
                }
            }
        }
    }
    if current.len() >= 2 {
        runs.push(current);
    }
    runs
}

#[test]
fn every_fixture_the_prose_cites_is_one_that_exists() {
    let mut checked = 0;
    for relative in PROSE {
        let path = crate_dir().join(relative);
        let text = fs::read_to_string(&path).unwrap_or_else(|_| panic!("{relative} is there"));
        for counter in counters(&text) {
            assert!(
                fixture_payload(&counter).is_some(),
                "{relative} cites fixture {counter}, which is not in the corpus"
            );
            checked += 1;
        }
    }
    // The citations are the point of the prose. If this reaches zero, either
    // they were all deleted or the scanner stopped seeing them, and both are
    // worth knowing about.
    // 42 today. The floor is well below that so ordinary edits do not trip it,
    // and well above zero so a scanner that stopped seeing citations does.
    assert!(checked >= 25, "only {checked} fixture citations found; the scanner has gone blind");
}

#[test]
fn a_citation_that_quotes_bytes_quotes_that_fixtures_bytes() {
    // The stronger half, and the one that catches a citation pointing at a
    // real frame that says something else. A quoted run must be a contiguous
    // slice of the payload it is cited beside -- prefix, middle or whole.
    let mut checked = 0;
    for relative in PROSE {
        let path = crate_dir().join(relative);
        let text = fs::read_to_string(&path).unwrap_or_else(|_| panic!("{relative} is there"));
        let lines: Vec<&str> = text.lines().collect();
        for (n, line) in lines.iter().enumerate() {
            let cited = counters(line);
            let Some(counter) = cited.first() else { continue };
            let fixture =
                fixture_payload(counter).unwrap_or_else(|| panic!("{relative}:{} cites the missing {counter}", n + 1));
            // WHERE THE BYTES ARE, relative to the citation. Three shapes
            // occur in this project's prose and all three must be reached
            // without reaching past them:
            //
            //   table row        the fixture and its bytes on ONE line
            //   wrapped comment  the fixture, then the bytes on the NEXT line
            //   fenced block     the fixture, a blank, ```, then the bytes
            //
            // The citation line's own bytes always win -- a table row that
            // looked ahead would be charged with the next row's evidence.
            //
            // The lookahead is deliberately narrow. An earlier version skipped
            // blank lines freely, which is right for markdown and wrong for
            // Rust, where a blank line separates unrelated statements: it
            // walked out of one assertion in `mdr_encode.rs` and into the next
            // one's bytes. So a blank is stepped over ONLY when a code fence
            // follows it, which is the fenced shape and nothing else.
            //
            // The fenced case matters because the battery correction -- the
            // most important byte quote in PROVENANCE.md -- sits three lines
            // below its citation, and a mutation to it went unnoticed until
            // this was fixed.
            let mut runs = quoted_bytes(line);
            if runs.is_empty() {
                let at = |k: usize| lines.get(n + k).map(|l| l.trim()).unwrap_or("");
                let fenced = |k: usize| at(k).starts_with("```");
                let next = if fenced(1) {
                    Some(n + 2)
                } else if at(1).is_empty() && fenced(2) {
                    Some(n + 3)
                } else if !at(1).is_empty() {
                    Some(n + 1)
                } else {
                    None
                };
                if let Some(next) = next.and_then(|i| lines.get(i)) {
                    runs = quoted_bytes(next);
                }
            }
            for run in runs {
                assert!(
                    fixture.contains(&run),
                    "{relative}:{} cites fixture {counter} beside bytes {run:02x?}, \
                     which are in neither its payload {:02x?} nor its frame {:02x?}",
                    n + 1,
                    fixture.payload,
                    fixture.wire
                );
                checked += 1;
            }
        }
    }
    // 17 today, across all three shapes: table rows, wrapped comments and
    // fenced blocks. Before the fenced case was reached it was 11, and the
    // battery correction was among the six nobody was checking.
    assert!(checked >= 12, "only {checked} byte-quoting citations checked");
}
