//! Port of candidate lines for the literal text search.
//!
//! An engine selects candidate lines of one source root; it decides no
//! match, column or address. The caller re-matches every line it receives
//! with its own matcher, so a pattern must select a superset of the lines
//! that matcher accepts. Records arrive in the engine's path order: the full
//! relative path compared byte-wise, which a directory walk reproduces by
//! sorting a directory's entries as `name` for a file and `name/` for a
//! directory. The product composition chooses the engine.

use std::ops::ControlFlow;
use std::path::Path;
use unica_work_control::cancellation::CancellationToken;

/// How the engine selects candidate lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidatePattern {
    /// One literal byte string (`-F`).
    Fixed(String),
    /// A POSIX extended expression matched byte-wise (`-E`, `LC_ALL=C`).
    Extended(String),
}

/// Which part of the source root the engine reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateCorpus {
    /// Source-root-relative directory to read, `/`-separated; `None` reads
    /// the whole root.
    pub subtree: Option<String>,
    /// Source-root-relative directories that belong to another source set.
    pub excluded: Vec<String>,
    /// Entries whose name starts with one of these are not source, at any
    /// depth.
    pub excluded_name_prefixes: Vec<String>,
}

/// One candidate line: the source-root-relative path as the engine prints it, the
/// one-based line number and the line bytes without the line feed.
pub struct CandidateLine<'a> {
    pub path: &'a [u8],
    pub line: usize,
    pub text: &'a [u8],
}

/// How the stream ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateEnd {
    /// The engine read the whole corpus.
    Completed,
    /// The consumer stopped the stream.
    Stopped,
    /// Cancellation stopped the process.
    Cancelled,
    /// The engine cannot read the root or cannot start: nothing was read
    /// and another engine must answer.
    Unavailable(String),
    /// The engine failed after it started.
    Failed(String),
}

/// What the consumer learns besides the lines themselves.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CandidateStream {
    /// Output lines longer than the line bound: the engine selected them, their
    /// path is not known.
    pub overlong_lines: usize,
    /// Output records without the `path NUL line NUL text` shape.
    pub malformed_records: usize,
}

/// An engine of candidate lines, chosen by the product composition.
pub trait CandidateLines: Send + Sync {
    /// Whether the engine can read `corpus` of `root` as a whole: `root` lies
    /// inside a work tree the engine reads and holds no work tree of its own
    /// that the engine would skip. Any failure answers `false`.
    fn reads(
        &self,
        root: &Path,
        corpus: &CandidateCorpus,
        max_line_bytes: usize,
        cancellation: &CancellationToken,
    ) -> bool;

    /// Stream the candidate lines of `corpus` until the corpus ends or
    /// `on_line` breaks.
    fn stream(
        &self,
        root: &Path,
        pattern: &CandidatePattern,
        corpus: &CandidateCorpus,
        max_line_bytes: usize,
        cancellation: &CancellationToken,
        on_line: &mut dyn FnMut(CandidateLine<'_>) -> ControlFlow<()>,
    ) -> (CandidateEnd, CandidateStream);
}
