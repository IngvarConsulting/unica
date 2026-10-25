//! The text search without a provider role: one traversal over the selected
//! BSL sources, read by `git grep` where every selected source set lies in a
//! Git work tree and by the actor's own walk elsewhere.
//!
//! Both engines feed the same line consumer, so a match, its column, its
//! address and its place in the traversal do not depend on the engine. Git
//! only selects candidate lines; the consumer matches every line itself.
//! The traversal order is the full source-relative path compared byte-wise,
//! source sets in admission order.
//!
//! Git answers the first page only: it reads the corpus fastest from its
//! start. Every continuation is the walk, which resumes at the file of the
//! last delivered match without reading the corpus before it again, and
//! proves that file's identity and its delivered matches. One cursor model
//! serves both engines because they agree on order and matches. A walk
//! page also ends at a time share of the caller's response budget, so a
//! long walk continues by pages.

use super::super::super::v13_read_modes::SearchMatcher;
use super::{
    read_search_line_bounded, ActorReadSourceCapability, SearchHitOwners,
    CANONICAL_SEARCH_MAX_DEPTH, CANONICAL_SEARCH_MAX_LINE_BYTES,
    CANONICAL_SEARCH_MAX_UNCOVERED_DETAILS,
};
use crate::infrastructure::event_sources::module_address_for_relative;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use unica_adapter_lexical::lines::{
    holds_foreign_work_tree, inside_work_tree, stream_candidate_lines, CandidateCorpus,
    CandidateEnd, CandidatePattern,
};
use unica_execution_contract::view_error::ViewError;
use unica_navigation::result_store::SearchResume;
use unica_process::runner::system_process_runner;
use unica_process::StreamControl;
use unica_refusal::RefusalCode;
use unica_source_contract::read::{SourceDirectory, SourceEntry};
use unica_work_control::cancellation::CancellationToken;

/// The share of the caller's remaining response budget one walk page may
/// spend before it ends and hands the rest to its cursor.
pub(in crate::infrastructure::daemon) const WALK_PAGE_RESPONSE_BUDGET_DIVISOR: u32 = 2;

/// The question a text search answers, independent of the engine.
pub(in crate::infrastructure::daemon) struct TextQuery {
    pub(in crate::infrastructure::daemon) matcher: SearchMatcher,
    /// `literal` or `regex`.
    pub(in crate::infrastructure::daemon) mode: &'static str,
    /// What `git grep` may preselect lines with; `None` leaves the question
    /// to the walk.
    pub(in crate::infrastructure::daemon) candidates: Option<CandidatePattern>,
    /// Whether the bytes of a line that is not UTF-8 may hold a match: only
    /// such a line leaves its file uncovered, the line Git would select.
    candidate_bytes: CandidateBytes,
}

impl TextQuery {
    /// Builds the matcher and the Git preselection of a query. A pattern
    /// that cannot be compiled within the size budget is a caller error.
    pub(in crate::infrastructure::daemon) fn new(
        query: &str,
        regex: bool,
        ignore_case: bool,
    ) -> Result<Self, String> {
        let compile = |pattern: &str| {
            regex::RegexBuilder::new(pattern)
                .case_insensitive(ignore_case)
                .size_limit(1 << 20)
                .dfa_size_limit(1 << 20)
                .build()
                .map_err(|error| format!("search regex is not a valid pattern: {error}"))
        };
        let escaped = regex::escape(query);
        let candidate_bytes = if !regex && !ignore_case {
            CandidateBytes::Literal(query.as_bytes().to_vec())
        } else {
            let pattern = if regex { query } else { escaped.as_str() };
            CandidateBytes::Pattern(
                regex::bytes::RegexBuilder::new(pattern)
                    .case_insensitive(ignore_case)
                    .size_limit(1 << 20)
                    .dfa_size_limit(1 << 20)
                    .build()
                    .map_err(|error| format!("search regex is not a valid pattern: {error}"))?,
            )
        };
        if regex {
            return Ok(Self {
                matcher: SearchMatcher::Regex(compile(query)?),
                mode: "regex",
                candidates: None,
                candidate_bytes,
            });
        }
        if !ignore_case {
            return Ok(Self {
                matcher: SearchMatcher::Literal(query.to_string()),
                mode: "literal",
                candidates: Some(CandidatePattern::Fixed(query.to_string())),
                candidate_bytes,
            });
        }
        Ok(Self {
            candidate_bytes,
            matcher: SearchMatcher::Regex(compile(&escaped)?),
            mode: "literal",
            candidates: case_folded_candidates(query).map(CandidatePattern::Extended),
        })
    }
}

/// The question over the raw bytes of a line that is not UTF-8.
enum CandidateBytes {
    Literal(Vec<u8>),
    Pattern(regex::bytes::Regex),
}

impl CandidateBytes {
    fn may_match(&self, raw: &[u8]) -> bool {
        match self {
            Self::Literal(needle) => raw.windows(needle.len()).any(|part| part == needle),
            Self::Pattern(pattern) => pattern.is_match(raw),
        }
    }
}

/// A byte-wise POSIX extended expression that selects every line the
/// case-insensitive literal matcher accepts: each character becomes the
/// alternation of its simple case-folding class, the same classes the
/// matcher folds by.
fn case_folded_candidates(query: &str) -> Option<String> {
    use regex_syntax::hir::{ClassUnicode, ClassUnicodeRange};
    let mut pattern = String::new();
    for character in query.chars() {
        let mut class = ClassUnicode::new([ClassUnicodeRange::new(character, character)]);
        class.try_case_fold_simple().ok()?;
        let variants = class
            .iter()
            .flat_map(|range| range.start()..=range.end())
            .collect::<Vec<_>>();
        if variants.len() > 1 {
            pattern.push('(');
        }
        for (index, variant) in variants.iter().enumerate() {
            if index > 0 {
                pattern.push('|');
            }
            if r"\^$.|?*+()[]{}".contains(*variant) {
                pattern.push('\\');
            }
            pattern.push(*variant);
        }
        if variants.len() > 1 {
            pattern.push(')');
        }
    }
    Some(pattern)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::infrastructure::daemon) enum TextEngine {
    Git,
    Walk,
}

/// Where the traversal continues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::infrastructure::daemon) enum TextContinuation {
    Start,
    /// Re-enter a file of the traversal.
    Resume(SearchResume),
}

/// The resume key of a file: its source set and source-relative path.
fn resume_key(source_set: &str, path: &str) -> Vec<u8> {
    let mut key = source_set.as_bytes().to_vec();
    key.push(0);
    key.extend_from_slice(path.as_bytes());
    key
}

impl TextContinuation {
    /// The source set and the source-relative path a walk re-enters.
    fn resume_target(&self) -> Option<(&[u8], &[u8])> {
        let Self::Resume(resume) = self else {
            return None;
        };
        let split = resume.file.iter().position(|byte| *byte == 0)?;
        Some((&resume.file[..split], &resume.file[split + 1..]))
    }
}

/// One source set of the traversal.
pub(in crate::infrastructure::daemon) struct TextSource<'a> {
    pub(in crate::infrastructure::daemon) source: &'a ActorReadSourceCapability,
    /// The logical scope every hit of this set reports.
    pub(in crate::infrastructure::daemon) scope: String,
    /// Retained-relative subtree to read; `None` reads the whole root.
    pub(in crate::infrastructure::daemon) prefix: Option<String>,
    /// Retained-relative roots of nested source sets, read by their owner.
    pub(in crate::infrastructure::daemon) excluded: Vec<String>,
}

pub(in crate::infrastructure::daemon) struct TextPageRequest<'a> {
    pub(in crate::infrastructure::daemon) query: &'a TextQuery,
    pub(in crate::infrastructure::daemon) sources: Vec<TextSource<'a>>,
    /// Keep only the matches of this module.
    pub(in crate::infrastructure::daemon) module: Option<String>,
    /// Matches to collect: the page and one lookahead.
    pub(in crate::infrastructure::daemon) wanted: usize,
    pub(in crate::infrastructure::daemon) engine: TextEngine,
    pub(in crate::infrastructure::daemon) continuation: TextContinuation,
    /// When a walk page ends and leaves the rest to its cursor.
    pub(in crate::infrastructure::daemon) walk_budget: Option<Duration>,
}

/// A collected match and the continuation right after it.
pub(in crate::infrastructure::daemon) struct TextHit {
    pub(in crate::infrastructure::daemon) value: Value,
    pub(in crate::infrastructure::daemon) source_set: String,
    pub(in crate::infrastructure::daemon) next: (TextContinuation, String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::infrastructure::daemon) enum TextScanEnd {
    /// The traversal reached the end of the selected sources.
    Complete,
    /// The lookahead match was found.
    Limit,
    /// The walk spent its page budget at a file boundary.
    Time,
}

pub(in crate::infrastructure::daemon) struct TextScan {
    pub(in crate::infrastructure::daemon) hits: Vec<TextHit>,
    pub(in crate::infrastructure::daemon) end: TextScanEnd,
    /// The digest of what the incoming continuation passed over; `None`
    /// when the traversal never reached its position.
    pub(in crate::infrastructure::daemon) passed: Option<String>,
    /// The continuation of a page that ended by time.
    pub(in crate::infrastructure::daemon) time_next: Option<(TextContinuation, String)>,
    pub(in crate::infrastructure::daemon) uncovered: usize,
    pub(in crate::infrastructure::daemon) uncovered_lower_bound: bool,
    pub(in crate::infrastructure::daemon) details: Vec<Value>,
}

/// Why the traversal could not answer.
pub(in crate::infrastructure::daemon) enum TextSearchError {
    Interrupted(ViewError),
    Engine(String),
}

impl TextSearchError {
    fn io(error: std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::Interrupted => Self::Interrupted(ViewError::new(
                RefusalCode::Cancelled,
                "text search was cancelled",
            )),
            std::io::ErrorKind::TimedOut => Self::Interrupted(ViewError::new(
                RefusalCode::DeadlineExceeded,
                "text search operation deadline elapsed",
            )),
            _ => Self::Engine(error.to_string()),
        }
    }
}

/// Chooses Git when the query has a Git preselection and every selected
/// source root lies in a Git work tree that holds no work tree of its own:
/// `git grep` reads no submodule and no embedded repository, the walk reads
/// both.
pub(in crate::infrastructure::daemon) fn choose_engine(
    query: &TextQuery,
    sources: &[TextSource<'_>],
    cancellation: &CancellationToken,
) -> TextEngine {
    let all_in_git = || {
        sources.iter().all(|source| {
            let retained = source.source.retained_root();
            let root = retained.path();
            inside_work_tree(system_process_runner(), root, cancellation)
                && !holds_foreign_work_tree(
                    system_process_runner(),
                    root,
                    &candidate_corpus(source),
                    CANONICAL_SEARCH_MAX_LINE_BYTES,
                    cancellation,
                )
        })
    };
    if query.candidates.is_some() && all_in_git() {
        TextEngine::Git
    } else {
        TextEngine::Walk
    }
}

pub(in crate::infrastructure::daemon) fn scan_text_page<'a>(
    request: &'a TextPageRequest<'a>,
    cancellation: &'a CancellationToken,
) -> Result<TextScan, TextSearchError> {
    let scan = scan_with(request, request.engine, cancellation)?;
    // A Git record whose file is unknown leaves coverage the walk can name
    // file by file and a cursor can carry; such a page is read again.
    if request.engine == TextEngine::Git && scan.uncovered_lower_bound {
        return scan_with(request, TextEngine::Walk, cancellation);
    }
    Ok(scan)
}

fn scan_with<'a>(
    request: &'a TextPageRequest<'a>,
    engine: TextEngine,
    cancellation: &'a CancellationToken,
) -> Result<TextScan, TextSearchError> {
    let mut collector = Collector::new(request, engine, cancellation);
    for source in &request.sources {
        // A resumed walk starts in the source set of the file it re-enters.
        let seek = match request.continuation.resume_target() {
            Some((set, path)) if collector.passed_digest.is_none() => {
                if set != source.source.source_set_name().as_bytes() {
                    continue;
                }
                Some(path.to_vec())
            }
            _ => None,
        };
        collector.enter_source(source);
        let flow = match engine {
            TextEngine::Git => git_source(request, source, &mut collector, cancellation)?,
            TextEngine::Walk => walk_source(source, seek, &mut collector, cancellation)
                .map_err(TextSearchError::io)?,
        };
        collector.leave_file();
        if let Some(error) = collector.fatal.take() {
            return Err(error);
        }
        if flow == Flow::Stop {
            break;
        }
    }
    Ok(collector.finish())
}

/// The part of a source root Git reads for one source set.
fn candidate_corpus(source: &TextSource<'_>) -> CandidateCorpus {
    CandidateCorpus {
        subtree: source.prefix.clone(),
        excluded: source.excluded.clone(),
        excluded_name_prefixes: vec![
            unica_workspace::compile_transaction::PUBLICATION_ARTIFACT_PREFIX.to_string(),
        ],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Continue,
    Stop,
}

fn git_source(
    request: &TextPageRequest<'_>,
    source: &TextSource<'_>,
    collector: &mut Collector<'_>,
    cancellation: &CancellationToken,
) -> Result<Flow, TextSearchError> {
    let pattern = request
        .query
        .candidates
        .as_ref()
        .ok_or_else(|| TextSearchError::Engine("the query has no Git preselection".into()))?;
    // An absent subtree has no candidates, and Git would only warn about it.
    if retained_subtree(source)
        .map_err(TextSearchError::io)?
        .is_none()
    {
        return Ok(Flow::Continue);
    }
    let root = source.source.retained_root();
    let mut current: Option<Vec<u8>> = None;
    let mut skip_file = false;
    let mut stopped = false;
    let (end, stream) = stream_candidate_lines(
        system_process_runner(),
        root.path(),
        pattern,
        &candidate_corpus(source),
        CANONICAL_SEARCH_MAX_LINE_BYTES,
        cancellation,
        &mut |record| {
            if current.as_deref() != Some(record.path) {
                current = Some(record.path.to_vec());
                let relative = match std::str::from_utf8(record.path) {
                    Ok(path) => PathBuf::from(path),
                    Err(_) => {
                        collector.unaddressable_record("invalid_path");
                        skip_file = true;
                        return StreamControl::Continue;
                    }
                };
                match collector.file(&relative) {
                    FileStep::Read => skip_file = false,
                    FileStep::Skip => skip_file = true,
                    FileStep::Stop => {
                        stopped = true;
                        return StreamControl::Stop;
                    }
                }
            }
            if skip_file {
                return StreamControl::Continue;
            }
            match collector.line(record.line, record.text) {
                LineStep::Continue => StreamControl::Continue,
                LineStep::SkipFile => {
                    skip_file = true;
                    StreamControl::Continue
                }
                LineStep::Stop => {
                    stopped = true;
                    StreamControl::Stop
                }
            }
        },
    );
    for _ in 0..stream.overlong_lines {
        collector.unaddressable_record("line_too_long");
    }
    if stream.malformed_records > 0 {
        return Err(TextSearchError::Engine(format!(
            "git grep returned {} malformed record(s)",
            stream.malformed_records
        )));
    }
    match end {
        CandidateEnd::Completed => Ok(Flow::Continue),
        CandidateEnd::Stopped if stopped => Ok(Flow::Stop),
        CandidateEnd::Stopped => Err(TextSearchError::Engine(
            "git grep stopped without a reason".into(),
        )),
        CandidateEnd::Cancelled => Err(TextSearchError::Interrupted(ViewError::new(
            RefusalCode::Cancelled,
            "text search was cancelled",
        ))),
        CandidateEnd::Unavailable(message) | CandidateEnd::Failed(message) => {
            Err(TextSearchError::Engine(message))
        }
    }
}

/// Passes the rest of a line the bounded reader refused.
fn skip_line(
    reader: &mut impl std::io::BufRead,
    mut checkpoint: impl FnMut() -> std::io::Result<()>,
) -> std::io::Result<()> {
    loop {
        checkpoint()?;
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(());
        }
        if let Some(end) = available.iter().position(|byte| *byte == b'\n') {
            reader.consume(end + 1);
            return Ok(());
        }
        let count = available.len();
        reader.consume(count);
    }
}

/// The retained directory a source's prefix names, or `None` when the
/// admitted logical scope has no files there.
fn retained_subtree(
    source: &TextSource<'_>,
) -> std::io::Result<Option<(PathBuf, SourceDirectory)>> {
    let root = source.source.retained_root();
    let mut relative = PathBuf::new();
    let mut directory = root.as_ref().clone();
    let Some(prefix) = &source.prefix else {
        return Ok(Some((relative, directory)));
    };
    for component in prefix.split('/') {
        if component.is_empty() || matches!(component, "." | "..") {
            return Err(std::io::Error::other(
                "canonical search scope prefix is invalid",
            ));
        }
        relative.push(component);
        match directory.retain_immediate_child_nofollow(std::ffi::OsStr::new(component)) {
            Ok(SourceEntry::Directory(next)) => directory = next,
            Ok(SourceEntry::ReparsePoint) => {
                return Err(std::io::Error::other(
                    "canonical search scope crosses a link/reparse point",
                ))
            }
            Ok(SourceEntry::RegularFile(_) | SourceEntry::Unsupported) => return Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(std::io::Error::new(
                    error.kind(),
                    format!("canonical search could not open the scope subtree: {error}"),
                ))
            }
        }
    }
    Ok(Some((relative, directory)))
}

/// The walk in Git's order: a directory's entries sorted as `name` for a
/// file and `name/` for a directory. A directory waits until no smaller
/// key remains in its parent, so it is entered exactly where Git would
/// print its first path.
///
/// A resumed walk seeks its file: an entry whose whole subtree sorts
/// before that path is passed by name, without being opened or listed.
fn walk_source(
    source: &TextSource<'_>,
    mut seek: Option<Vec<u8>>,
    collector: &mut Collector<'_>,
    cancellation: &CancellationToken,
) -> std::io::Result<Flow> {
    use super::ordered_search_names::OrderedNames;
    use std::collections::BTreeMap;
    use std::ffi::OsString;

    struct Frame {
        relative: PathBuf,
        directory: SourceDirectory,
        names: Option<OrderedNames>,
        pending: BTreeMap<Vec<u8>, (PathBuf, SourceDirectory)>,
        peeked: Option<OsString>,
    }
    let vanished = |error: &std::io::Error| error.kind() == std::io::ErrorKind::NotFound;
    let context = |phase: String, error: std::io::Error| {
        std::io::Error::new(error.kind(), format!("canonical search {phase}: {error}"))
    };
    let entry = |relative: &Path| relative.to_string_lossy().replace('\\', "/");
    let checkpoint = || {
        if cancellation.is_cancelled() {
            Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "canonical search was cancelled",
            ))
        } else if source.source.deadline.is_elapsed() {
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "canonical search operation deadline elapsed",
            ))
        } else {
            Ok(())
        }
    };
    let Some((start_relative, start)) = retained_subtree(source)? else {
        return Ok(Flow::Continue);
    };
    let key = |name: &OsString, directory: bool| {
        let mut key = name.to_string_lossy().into_owned().into_bytes();
        if directory {
            key.push(b'/');
        }
        key
    };
    let mut stack = vec![Frame {
        relative: start_relative,
        directory: start,
        names: None,
        pending: BTreeMap::new(),
        peeked: None,
    }];
    while let Some(frame) = stack.last_mut() {
        checkpoint()?;
        if frame.names.is_none() {
            match frame.directory.validate_named_identity() {
                Ok(()) => {}
                Err(error) if vanished(&error) => {
                    stack.pop();
                    continue;
                }
                Err(error) => {
                    return Err(context(
                        format!("lost retained directory `{}`", entry(&frame.relative)),
                        error,
                    ))
                }
            }
            frame.names = Some(OrderedNames::collect(&frame.directory, checkpoint).map_err(
                |error| {
                    context(
                        format!(
                            "could not list source directory `{}`",
                            entry(&frame.relative)
                        ),
                        error,
                    )
                },
            )?);
        }
        let name = match frame.peeked.take() {
            Some(name) => Some(name),
            None => frame
                .names
                .as_mut()
                .expect("enumerated search directory")
                .next(false)
                .map_err(|error| context("could not read sorted source names".into(), error))?,
        };
        let enter_pending = match &name {
            None => !frame.pending.is_empty(),
            Some(name) => frame
                .pending
                .first_key_value()
                .is_some_and(|(pending, _)| pending.as_slice() < key(name, false).as_slice()),
        };
        if enter_pending {
            frame.peeked = name;
            let (_, (relative, directory)) =
                frame.pending.pop_first().expect("a pending directory");
            if stack.len() > CANONICAL_SEARCH_MAX_DEPTH {
                return Err(std::io::Error::other(
                    "canonical search exceeded its retained depth limit",
                ));
            }
            stack.push(Frame {
                relative,
                directory,
                names: None,
                pending: BTreeMap::new(),
                peeked: None,
            });
            continue;
        }
        let Some(name) = name else {
            stack.pop();
            continue;
        };
        // A staged copy or a preimage left beside a module right after
        // apply is not source, and on Windows it may refuse to open.
        if name
            .to_string_lossy()
            .starts_with(unica_workspace::compile_transaction::PUBLICATION_ARTIFACT_PREFIX)
        {
            continue;
        }
        let relative = frame.relative.join(&name);
        // Where the entry sorts against the sought path. A subtree wholly
        // before it is passed unopened; otherwise the entry's kind decides:
        // a directory after it waits in order like any other, a file after
        // it means the sought file is gone.
        let sought = seek.as_deref().map(|target| {
            let file_key = entry(&relative).into_bytes();
            let mut directory_key = file_key.clone();
            directory_key.push(b'/');
            (file_key, directory_key, target.to_vec())
        });
        if let Some((_, directory_key, target)) = &sought {
            if directory_key.as_slice() < target.as_slice() && !target.starts_with(directory_key) {
                continue;
            }
        }
        let child = match frame.directory.retain_immediate_child_nofollow(&name) {
            Ok(child) => child,
            Err(error) if vanished(&error) => continue,
            Err(error) => {
                return Err(context(
                    format!("could not open source entry `{}`", entry(&relative)),
                    error,
                ))
            }
        };
        if let Some((file_key, _, target)) = &sought {
            if !matches!(child, SourceEntry::Directory(_)) {
                match file_key.as_slice().cmp(target.as_slice()) {
                    std::cmp::Ordering::Less => continue,
                    std::cmp::Ordering::Equal | std::cmp::Ordering::Greater => seek = None,
                }
            }
        }
        match child {
            SourceEntry::Directory(directory) => {
                let skipped = name.to_string_lossy() == unica_workspace::paths::GENERATED_DIR_NAME
                    || source.excluded.contains(&entry(&relative));
                if !skipped {
                    let directory_key = key(&name, true);
                    frame.pending.insert(directory_key, (relative, directory));
                }
            }
            SourceEntry::RegularFile(file)
                if relative
                    .extension()
                    .and_then(std::ffi::OsStr::to_str)
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("bsl")) =>
            {
                match collector.file(&relative) {
                    FileStep::Read => {}
                    FileStep::Skip => continue,
                    FileStep::Stop => return Ok(Flow::Stop),
                }
                let opened = match file.open_named_identity_for_read() {
                    Ok(opened) => opened,
                    Err(error) if vanished(&error) => continue,
                    Err(error) => {
                        return Err(context(
                            format!("lost retained source file `{}`", entry(&relative)),
                            error,
                        ))
                    }
                };
                let mut reader = std::io::BufReader::new(opened);
                let mut raw = Vec::new();
                let mut line = 0_usize;
                loop {
                    match read_search_line_bounded(
                        &mut reader,
                        &mut raw,
                        CANONICAL_SEARCH_MAX_LINE_BYTES,
                        checkpoint,
                    ) {
                        Ok(true) => {}
                        Ok(false) => break,
                        Err(error) if error.kind() == std::io::ErrorKind::FileTooLarge => {
                            // The unread line stays uncovered; the lines
                            // after it are read like any other.
                            line += 1;
                            collector.uncovered_line("line_too_long");
                            skip_line(&mut reader, checkpoint).map_err(|error| {
                                context(
                                    format!("could not read source file `{}`", entry(&relative)),
                                    error,
                                )
                            })?;
                            continue;
                        }
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::Interrupted | std::io::ErrorKind::TimedOut
                            ) =>
                        {
                            return Err(error)
                        }
                        Err(error) => {
                            return Err(context(
                                format!("could not read source file `{}`", entry(&relative)),
                                error,
                            ))
                        }
                    }
                    line += 1;
                    let text = raw.strip_suffix(b"\n").unwrap_or(&raw);
                    match collector.line(line, text) {
                        LineStep::Continue => {}
                        LineStep::SkipFile => break,
                        LineStep::Stop => return Ok(Flow::Stop),
                    }
                }
            }
            SourceEntry::RegularFile(_) | SourceEntry::ReparsePoint | SourceEntry::Unsupported => {}
        }
    }
    Ok(Flow::Continue)
}

enum FileStep {
    Read,
    Skip,
    Stop,
}

enum LineStep {
    Continue,
    SkipFile,
    Stop,
}

/// The traversal file being read.
struct CurrentFile {
    key: Vec<u8>,
    identity: [u8; 32],
    file_id: String,
    relative: PathBuf,
    module: Option<Result<String, &'static str>>,
    matches: u64,
    chain: [u8; 32],
    /// Matches of a resumed file still to pass over.
    passing: u64,
    /// Whether the file is already counted uncovered.
    uncovered: bool,
}

/// The one line consumer both engines feed.
struct Collector<'a> {
    request: &'a TextPageRequest<'a>,
    engine: TextEngine,
    cancellation: &'a CancellationToken,
    started: Instant,
    files_read: usize,
    // The current source set.
    source_set: String,
    scope: String,
    source: Option<&'a ActorReadSourceCapability>,
    owners: Option<SearchHitOwners<'a>>,
    // The traversal.
    current: Option<CurrentFile>,
    passed_digest: Option<String>,
    hits: Vec<TextHit>,
    end: TextScanEnd,
    time_next: Option<(TextContinuation, String)>,
    uncovered: usize,
    uncovered_lower_bound: bool,
    details: Vec<Value>,
    fatal: Option<TextSearchError>,
}

fn digest(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    hasher.finalize().into()
}

fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl<'a> Collector<'a> {
    fn new(
        request: &'a TextPageRequest<'a>,
        engine: TextEngine,
        cancellation: &'a CancellationToken,
    ) -> Self {
        Self {
            request,
            engine,
            cancellation,
            started: Instant::now(),
            files_read: 0,
            source_set: String::new(),
            scope: String::new(),
            source: None,
            owners: None,
            current: None,
            passed_digest: match &request.continuation {
                TextContinuation::Start => Some(String::new()),
                _ => None,
            },
            hits: Vec::new(),
            end: TextScanEnd::Complete,
            time_next: None,
            uncovered: match &request.continuation {
                TextContinuation::Resume(resume) => {
                    usize::try_from(resume.uncovered).unwrap_or(usize::MAX)
                }
                _ => 0,
            },
            uncovered_lower_bound: false,
            details: Vec::new(),
            fatal: None,
        }
    }

    fn enter_source(&mut self, source: &'a TextSource<'a>) {
        self.leave_file();
        self.source_set = source.source.source_set_name().to_owned();
        self.scope = source.scope.clone();
        self.source = Some(source.source);
        self.owners = Some(source.source.search_hit_owners());
    }

    fn leave_file(&mut self) {
        self.current = None;
    }

    /// The digest of the passed part a walk cursor names: the identity of
    /// the file it re-enters and the matches of that file before it.
    fn walk_digest(identity: &[u8; 32], chain: &[u8; 32]) -> String {
        format!(
            "walk-v1:{}",
            hex(&digest(&[b"unica-text-walk-v1", identity, chain]))
        )
    }

    fn file(&mut self, relative: &Path) -> FileStep {
        self.leave_file();
        let path = relative.to_string_lossy().replace('\\', "/");
        // Git cannot see these by name, the walk never opens them.
        if path.split('/').any(|part| {
            part.starts_with(unica_workspace::compile_transaction::PUBLICATION_ARTIFACT_PREFIX)
                || part == unica_workspace::paths::GENERATED_DIR_NAME
        }) {
            return FileStep::Skip;
        }
        let identity = digest(&[self.source_set.as_bytes(), path.as_bytes()]);
        let key = resume_key(&self.source_set, &path);
        let mut passing = 0;
        if let TextContinuation::Resume(resume) = &self.request.continuation {
            if self.passed_digest.is_none() {
                // The walk skips everything before the re-entered file; any
                // other file here means that file is gone or ended before
                // its delivered matches: the cursor names another traversal.
                if key != resume.file || self.files_read > 0 {
                    return FileStep::Stop;
                }
                passing = resume.hits;
                if passing == 0 {
                    self.passed_digest = Some(Self::walk_digest(&identity, &[0; 32]));
                }
            }
        }
        if self.engine == TextEngine::Walk && self.files_read > 0 {
            if let Some(budget) = self.request.walk_budget {
                if self.started.elapsed() >= budget {
                    self.end = TextScanEnd::Time;
                    self.time_next = Some((
                        TextContinuation::Resume(SearchResume {
                            file: key,
                            hits: 0,
                            uncovered: self.uncovered as u64,
                        }),
                        Self::walk_digest(&identity, &[0; 32]),
                    ));
                    return FileStep::Stop;
                }
            }
        }
        self.files_read += 1;
        let mut current = CurrentFile {
            key,
            file_id: format!("bsl-{}", &hex(&identity)[..16]),
            identity,
            relative: relative.to_path_buf(),
            module: None,
            matches: 0,
            chain: [0; 32],
            passing,
            uncovered: false,
        };
        if let Some(module) = &self.request.module {
            match self.module_of(&mut current) {
                Ok(Ok(at)) if at == *module => {}
                Ok(_) => return FileStep::Skip,
                Err(error) => {
                    self.fatal = Some(error);
                    return FileStep::Stop;
                }
            }
        }
        self.current = Some(current);
        FileStep::Read
    }

    /// The proven module of the current file, or why it has none. Mapped
    /// once per file, from the path in hand: no descriptor or module read
    /// beyond the owner's registration.
    fn module_of(
        &mut self,
        current: &mut CurrentFile,
    ) -> Result<Result<String, &'static str>, TextSearchError> {
        if let Some(module) = &current.module {
            return Ok(module.clone());
        }
        let source = self.source.expect("a source set is entered");
        let owners = self.owners.as_mut().expect("a source set is entered");
        let mapped = match crate::domain::source_capabilities::bsl_profile_for_source(
            source.source_profile(),
        ) {
            None => Err("unsupported_source_profile"),
            Some(profile) => match module_address_for_relative(
                source.source_set_name(),
                source.source_kind(),
                profile,
                &current.relative,
            ) {
                None => Err("outside_module_layout"),
                Some(at) => {
                    let cancellation = self.cancellation;
                    let checkpoint = || {
                        if cancellation.is_cancelled() {
                            Err(ViewError::new(
                                RefusalCode::Cancelled,
                                "text search was cancelled",
                            ))
                        } else {
                            Ok(())
                        }
                    };
                    if owners
                        .proves(&at, &checkpoint)
                        .map_err(TextSearchError::Interrupted)?
                    {
                        Ok(at.to_string())
                    } else {
                        Err("owner_unproven")
                    }
                }
            },
        };
        current.module = Some(mapped.clone());
        Ok(mapped)
    }

    /// Leaves the current file uncovered once and stops reading it.
    fn uncovered(&mut self, reason: &'static str) {
        self.uncovered_line(reason);
        self.current = None;
    }

    /// Leaves the current file uncovered once; its other lines stay read.
    fn uncovered_line(&mut self, reason: &'static str) {
        let Some(current) = self.current.as_mut() else {
            return;
        };
        if current.uncovered {
            return;
        }
        current.uncovered = true;
        let file_id = current.file_id.clone();
        self.uncovered += 1;
        if self.details.len() < CANONICAL_SEARCH_MAX_UNCOVERED_DETAILS {
            let mut detail = Map::new();
            detail.insert("sourceSet".to_owned(), self.source_set.clone().into());
            detail.insert("fileId".to_owned(), file_id.into());
            detail.insert("reason".to_owned(), reason.into());
            self.details.push(Value::Object(detail));
        }
    }

    /// A Git record whose file is unknown: it stays visible as uncovered,
    /// and the count becomes a lower bound.
    fn unaddressable_record(&mut self, reason: &'static str) {
        self.uncovered += 1;
        self.uncovered_lower_bound = true;
        if self.details.len() < CANONICAL_SEARCH_MAX_UNCOVERED_DETAILS {
            let mut detail = Map::new();
            detail.insert("sourceSet".to_owned(), self.source_set.clone().into());
            detail.insert("reason".to_owned(), reason.into());
            self.details.push(Value::Object(detail));
        }
    }

    fn line(&mut self, number: usize, raw: &[u8]) -> LineStep {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        // `view` reads the module without its byte-order mark; the column
        // and snippet must match it.
        let raw = if number == 1 {
            raw.strip_prefix("\u{feff}".as_bytes()).unwrap_or(raw)
        } else {
            raw
        };
        let Ok(text) = std::str::from_utf8(raw) else {
            // A line that cannot hold a match is no gap in the answer.
            if !self.request.query.candidate_bytes.may_match(raw) {
                return LineStep::Continue;
            }
            self.uncovered("invalid_utf8");
            return LineStep::SkipFile;
        };
        let starts = self
            .request
            .query
            .matcher
            .match_starts(text)
            .collect::<Vec<_>>();
        if starts.is_empty() {
            return LineStep::Continue;
        }
        let Some(mut current) = self.current.take() else {
            return LineStep::SkipFile;
        };
        let module = match self.module_of(&mut current) {
            Ok(module) => module,
            Err(error) => {
                self.fatal = Some(error);
                return LineStep::Stop;
            }
        };
        let snippet = text.chars().take(512).collect::<String>();
        let mut previous = 0;
        let mut column = 1;
        let mut step = LineStep::Continue;
        for start in starts {
            column += text[previous..start].chars().count();
            previous = start;
            let mut item = Map::new();
            item.insert("scope".to_owned(), self.scope.clone().into());
            match &module {
                Ok(at) => {
                    item.insert("at".to_owned(), at.clone().into());
                }
                Err(reason) => {
                    // No proven module owns this file: name it by its
                    // identifier, never by path.
                    item.insert("fileId".to_owned(), current.file_id.clone().into());
                    item.insert("unaddressed".to_owned(), (*reason).into());
                }
            }
            item.insert("line".to_owned(), number.into());
            item.insert("column".to_owned(), column.into());
            item.insert("snippet".to_owned(), snippet.clone().into());
            let value = Value::Object(item);
            let hit_digest = digest(&[serde_json::to_vec(&value)
                .expect("a text hit serializes")
                .as_slice()]);
            current.matches += 1;
            current.chain = digest(&[&current.chain, &hit_digest]);
            if current.passing > 0 {
                current.passing -= 1;
                if current.passing == 0 {
                    self.passed_digest = Some(Self::walk_digest(&current.identity, &current.chain));
                }
                continue;
            }
            if self.passed_digest.is_none() {
                // A resumed walk reached matches before the position it
                // re-enters: the cursor never names such a traversal.
                continue;
            }
            // Either engine names the same place: the walk resumes there.
            let next = (
                TextContinuation::Resume(SearchResume {
                    file: current.key.clone(),
                    hits: current.matches,
                    // The count before the current file: the re-entered
                    // file counts itself again.
                    uncovered: (self.uncovered - usize::from(current.uncovered)) as u64,
                }),
                Self::walk_digest(&current.identity, &current.chain),
            );
            self.hits.push(TextHit {
                value,
                source_set: self.source_set.clone(),
                next,
            });
            if self.hits.len() >= self.request.wanted {
                self.end = TextScanEnd::Limit;
                step = LineStep::Stop;
                break;
            }
        }
        self.current = Some(current);
        step
    }

    fn finish(self) -> TextScan {
        TextScan {
            hits: self.hits,
            end: self.end,
            passed: self.passed_digest,
            time_next: self.time_next,
            uncovered: self.uncovered,
            uncovered_lower_bound: self.uncovered_lower_bound,
            details: self.details,
        }
    }
}
