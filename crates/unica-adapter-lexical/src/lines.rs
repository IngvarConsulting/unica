//! Candidate BSL lines of one source root through `git grep`.
//!
//! The engine only selects lines; it decides no match, column or address.
//! The caller re-matches every line it receives with its own matcher, so a
//! pattern here must select a superset of the lines that matcher accepts.
//! Records arrive in Git's path order: the full relative path compared
//! byte-wise, which a directory walk reproduces by sorting a directory's
//! entries as `name` for a file and `name/` for a directory.

use crate::provider::{generated_corpus_exclusion, git_glob_literal_prefix, process_exit_code_is};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use unica_process::runner::{ProcessCommand, ProcessRunner, ProcessStreamFailure};
use unica_process::StreamControl;
use unica_work_control::cancellation::CancellationToken;

/// How `git grep` selects candidate lines.
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

/// One candidate line: the source-root-relative path as Git prints it, the
/// one-based line number and the line bytes without the line feed.
pub struct CandidateLine<'a> {
    pub path: &'a [u8],
    pub line: usize,
    pub text: &'a [u8],
}

/// How the stream ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateEnd {
    /// Git read the whole corpus.
    Completed,
    /// The consumer stopped the stream.
    Stopped,
    /// Cancellation stopped the process.
    Cancelled,
    /// The root is not inside a Git work tree, or Git cannot be started:
    /// nothing was read and another engine must answer.
    Unavailable(String),
    /// Git failed after it started.
    Failed(String),
}

/// What the consumer learns besides the lines themselves.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CandidateStream {
    /// Output lines longer than the line bound: Git selected them, their
    /// path is not known.
    pub overlong_lines: usize,
    /// Output records without the `path NUL line NUL text` shape.
    pub malformed_records: usize,
}

/// Whether `root` lies inside a Git work tree that a candidate stream can
/// read. A missing Git or any failure answers `false`.
pub fn inside_work_tree(
    runner: &(dyn ProcessRunner + Send + Sync),
    root: &Path,
    cancellation: &CancellationToken,
) -> bool {
    let command = ProcessCommand {
        program: PathBuf::from("git"),
        args: vec!["rev-parse".to_string(), "--is-inside-work-tree".to_string()],
        cwd: root.to_path_buf(),
        env: Vec::new(),
        env_remove: Vec::new(),
        capture_limits: Some((64, 4096)),
        timeout: None,
        cancellation: cancellation.clone(),
    };
    runner
        .run(&command)
        .is_ok_and(|output| output.status_success && output.stdout.trim() == "true")
}

/// Streams candidate BSL lines of `root` to `on_line`.
///
/// Tracked, untracked and ignored files are read alike, binary-looking
/// files as text, the generated `.build` cache never. The process runs
/// without its own deadline; only `cancellation` stops it early.
pub fn stream_candidate_lines(
    runner: &(dyn ProcessRunner + Send + Sync),
    root: &Path,
    pattern: &CandidatePattern,
    corpus: &CandidateCorpus,
    max_line_bytes: usize,
    cancellation: &CancellationToken,
    on_line: &mut dyn FnMut(CandidateLine<'_>) -> StreamControl,
) -> (CandidateEnd, CandidateStream) {
    let mut args = [
        "-c",
        "core.quotepath=false",
        // A user's grep defaults must not change the record shape or the
        // paths: one column-free record per line, relative to the root.
        "-c",
        "grep.column=false",
        "-c",
        "grep.fullName=false",
        "grep",
        "--no-color",
        "--untracked",
        "--no-exclude-standard",
        "--null",
        "--text",
        "-n",
    ]
    .map(str::to_string)
    .to_vec();
    match pattern {
        CandidatePattern::Fixed(text) => args.extend(["-F".into(), "-e".into(), text.clone()]),
        CandidatePattern::Extended(text) => args.extend(["-E".into(), "-e".into(), text.clone()]),
    }
    args.push("--".into());
    const BSL_GLOB: &str = "**/*.[bB][sS][lL]";
    args.push(match &corpus.subtree {
        None => format!(":(glob){BSL_GLOB}"),
        Some(prefix) => format!(":(glob){}/{BSL_GLOB}", git_glob_literal_prefix(prefix)),
    });
    args.push(generated_corpus_exclusion());
    for excluded in &corpus.excluded {
        args.push(format!(":(exclude,literal){excluded}"));
    }
    for prefix in &corpus.excluded_name_prefixes {
        let prefix = git_glob_literal_prefix(prefix);
        args.push(format!(":(exclude,glob)**/{prefix}*"));
        args.push(format!(":(exclude,glob)**/{prefix}*/**"));
    }
    let command = ProcessCommand {
        program: PathBuf::from("git"),
        args,
        cwd: root.to_path_buf(),
        // Byte-wise matching and bracket ranges, whatever the host locale.
        env: vec![(OsString::from("LC_ALL"), OsString::from("C"))],
        env_remove: Vec::new(),
        capture_limits: None,
        timeout: None,
        cancellation: cancellation.clone(),
    };
    let mut stream = CandidateStream::default();
    let mut consume = |_: usize, record: &[u8]| {
        let mut parts = record.splitn(3, |byte| *byte == 0);
        let (Some(path), Some(line), Some(text)) = (parts.next(), parts.next(), parts.next())
        else {
            stream.malformed_records += 1;
            return StreamControl::Continue;
        };
        let Some(line) = std::str::from_utf8(line)
            .ok()
            .and_then(|line| line.parse::<usize>().ok())
            .filter(|line| *line > 0 && !path.is_empty())
        else {
            stream.malformed_records += 1;
            return StreamControl::Continue;
        };
        on_line(CandidateLine { path, line, text })
    };
    let output = match runner.run_streaming_staged(&command, max_line_bytes, &mut consume) {
        Ok(output) => output,
        Err(ProcessStreamFailure::Spawn(message)) => {
            return (
                CandidateEnd::Unavailable(format!("git could not be started: {message}")),
                stream,
            )
        }
        Err(ProcessStreamFailure::Run(message)) => {
            return (
                CandidateEnd::Failed(format!("git grep failed: {message}")),
                stream,
            )
        }
    };
    if output.line_error.is_some() {
        stream.overlong_lines += 1;
    }
    let end = if output.cancelled {
        CandidateEnd::Cancelled
    } else if output.stopped_by_consumer {
        CandidateEnd::Stopped
    } else if output.stderr.trim().is_empty()
        && (output.status_success || process_exit_code_is(&output.status, 1))
    {
        CandidateEnd::Completed
    } else if output.stderr.contains("not a git repository")
        && process_exit_code_is(&output.status, 128)
    {
        CandidateEnd::Unavailable("the source root is not inside a Git work tree".to_string())
    } else if output.stderr.trim().is_empty() {
        CandidateEnd::Failed(format!("git grep exited with status {}", output.status))
    } else {
        // Git reports a directory it could not read on stderr and still
        // exits successfully: that corpus was not read, which is no answer.
        CandidateEnd::Failed(format!("git grep: {}", output.stderr.trim()))
    };
    (end, stream)
}
