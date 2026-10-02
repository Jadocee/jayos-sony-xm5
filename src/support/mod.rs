//! Argument parsing, output and subprocesses, for `qs-sony` and `qs-sonyd`.
//!
//! WHY A COPY AND NOT A DEPENDENCY
//!
//! These three files are jayos's `rust/crates/qs-core/src/{cli,emit,proc}.rs`
//! at jayos commit 16acd81, copied whole with their tests. In jayos they are
//! shared by forty-odd helpers, and the binaries here were two of them. The
//! alternative was a git dependency on jayos's workspace, which would make this
//! repository's build depend on the repository it was moved out of -- the
//! coupling the move exists to remove -- and would drag in the rest of
//! `qs-core` (`hypr`, `notify`, `store`, `pw_route` ...) for three modules.
//!
//! Whole files rather than the functions the binaries call: `output_within` is
//! one public function over six private ones, and the tests that pin its
//! deadline and reaping behaviour exercise all of them. A trimmed copy would
//! have kept the code and lost the evidence. The two edits are the import in
//! `cli` and one doc link in `emit`, both from `crate::` to `super::`.
//!
//! A fix to one copy is a fix to one copy. If `qs-core` changes one of these,
//! the change has to be carried here by hand, and the jayos commit above is
//! where to diff from.

pub mod cli;
pub mod emit;
pub mod proc;
