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
//!
//! The first page also counts the whole corpus: once the page is full the
//! traversal goes on to the end and keeps only counters, one per module
//! with matches, so the first answer carries the exact volume of the
//! search. A first page therefore never ends by time; a long one becomes a
//! Task like any slow answer.

use super::super::super::v13_read_modes::SearchMatcher;
use super::{
    read_search_line_bounded, ActorReadSourceCapability, CANONICAL_SEARCH_MAX_DEPTH,
    CANONICAL_SEARCH_MAX_LINE_BYTES, CANONICAL_SEARCH_MAX_UNCOVERED_DETAILS,
};
use crate::application::v13::view::ViewReadAuthority;
use crate::infrastructure::event_sources::module_address_for_relative;
use crate::infrastructure::v13_large_configuration::{RegistrationCache, RegistrationIndex};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use unica_adapter_lexical::lines::{
    holds_foreign_work_tree, inside_work_tree, stream_candidate_lines, CandidateCorpus,
    CandidateEnd, CandidatePattern,
};
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_execution_contract::view_error::ViewError;
use unica_navigation::result_store::SearchResume;
use unica_process::runner::system_process_runner;
use unica_process::StreamControl;
use unica_refusal::RefusalCode;
use unica_source_contract::read::{SourceDirectory, SourceEntry};
use unica_source_contract::SourceSetKind;
use unica_work_control::cancellation::CancellationToken;

/// Invocation-local placement evidence. This value is not a read capability;
/// the actor's retained source and its reader still own every observation.
#[derive(Debug)]
pub(in crate::infrastructure::daemon) struct ExternalSearchBinding {
    pub binding: unica_source_designer::external_artifact::ExternalArtifactBinding,
    pub stamp: unica_source_contract::read::FileStamp,
}

impl Clone for ExternalSearchBinding {
    fn clone(&self) -> Self {
        Self {
            binding: self.binding.clone(),
            stamp: self.stamp,
        }
    }
}

impl ExternalSearchBinding {
    pub(in crate::infrastructure::daemon) fn validate(
        &self,
        source: &ActorReadSourceCapability,
    ) -> Result<(), ViewError> {
        let stamp = source
            .retained_root()
            .open_relative_regular_nofollow(self.binding.descriptor())
            .and_then(|file| file.stamp())
            .map_err(|error| ViewError::new(RefusalCode::ConcurrentChange, error.to_string()))?;
        if stamp != self.stamp {
            return Err(ViewError::new(
                RefusalCode::ConcurrentChange,
                "external owner descriptor changed during search",
            ));
        }
        Ok(())
    }
}

/// Proves that the owner of a module address derived from a hit path is
/// registered in this source set, so a hit names only a module `view` admits.
/// Configuration and extension owners are checked against the source set's own
/// `Configuration.xml` registrations, read once per revision through the
/// retained root and shared with `view` by the registration cache; each owner
/// is decided once per scan. An extension object that is neither its own nor
/// borrowed is not registered there and stays unproven.
pub(in crate::infrastructure::daemon) struct SearchHitOwners<'a> {
    root: Arc<SourceDirectory>,
    source_kind: SourceSetKind,
    // The issuer supplies the retained root/kind/cache for one traversal.
    // Collector supplies that same source borrow to each external proof.
    external: std::collections::BTreeMap<std::path::PathBuf, ExternalSearchBinding>,
    // Logical-address admission must also establish unique external names.
    // One shared inventory per source scan (including a fallback scan), never
    // one inventory per physical hit. Errors cannot publish a valid prefix.
    external_inventory: Option<
        Result<unica_source_designer::external_artifact::ExternalArtifactInventory, ViewError>,
    >,
    cache: &'a RegistrationCache,
    index: Option<Option<Arc<RegistrationIndex>>>,
    owners: std::collections::BTreeMap<(&'static str, String), bool>,
}

impl<'a> SearchHitOwners<'a> {
    pub(super) fn new(
        root: Arc<SourceDirectory>,
        source_kind: SourceSetKind,
        cache: &'a RegistrationCache,
    ) -> Self {
        Self {
            root,
            source_kind,
            cache,
            index: None,
            owners: std::collections::BTreeMap::new(),
            external: std::collections::BTreeMap::new(),
            external_inventory: None,
        }
    }

    /// Reuse the point locator's owner/child-registration proof. No full name
    /// directory or second XML-name resolver is built by text search.
    fn external_module(
        &mut self,
        source: &ActorReadSourceCapability,
        relative: &std::path::Path,
        admitted: Option<&unica_source_designer::external_artifact::ExternalArtifactBinding>,
        cancellation: &CancellationToken,
    ) -> Result<Option<ExternalSearchBinding>, ViewError> {
        use crate::infrastructure::v13_find::{LayoutFindSource, WorkspaceFindDirectoryBuilder};
        use unica_source_designer::external_artifact::ExternalArtifactBinding;
        // Check the helper's physical context before any cached observation.
        // The existing Collector/factory/publication chain owns actor identity.
        if self.root.identity() != source.retained_root().identity()
            || self.source_kind != source.source_kind()
        {
            return Err(ViewError::new(
                RefusalCode::ConcurrentChange,
                "search owner proof belongs to a different source context",
            ));
        }
        if admitted.is_some_and(|binding| binding.logical_path(relative).is_none()) {
            return Ok(None);
        }
        let layout = [LayoutFindSource::new(
            source.source_set_name(),
            self.source_kind,
            &self.root,
        )];
        let entry = WorkspaceFindDirectoryBuilder::default()
            .locate_path(
                &layout,
                &relative.to_string_lossy(),
                source.deadline(),
                cancellation,
            )
            .map_err(|error| match error.detail() {
                Some(detail) => ViewError::detailed(detail, error.to_string()),
                None => ViewError::new(error.code(), error.to_string()),
            })?;
        let Some(entry) = entry.filter(|entry| entry.kind() == "Module") else {
            return Ok(None);
        };
        let at = QualifiedAddress::parse(entry.at())
            .map_err(|error| ViewError::new(RefusalCode::InvalidSource, error.to_string()))?;
        let Some(name) = at.segments().first().and_then(|part| part.name()) else {
            return Ok(None);
        };
        let Some(descriptor) = entry.owner_descriptor() else {
            return Ok(None);
        };
        let binding =
            ExternalArtifactBinding::new(self.source_kind, name, std::path::Path::new(descriptor))
                .map_err(|error| ViewError::new(RefusalCode::InvalidSource, error))?;
        if admitted.is_some_and(|admitted| *admitted != binding) {
            return Err(ViewError::new(
                RefusalCode::ConcurrentChange,
                "external search placement changed after admission",
            ));
        }
        if admitted.is_none() {
            let mut checkpoint = || {
                if cancellation.is_cancelled() {
                    Err(ViewError::new(
                        RefusalCode::Cancelled,
                        "external owner admission was cancelled",
                    ))
                } else if source.deadline().is_elapsed() {
                    Err(ViewError::new(
                        RefusalCode::DeadlineExceeded,
                        "external owner admission deadline elapsed",
                    ))
                } else {
                    Ok(())
                }
            };
            checkpoint()?;
            if self.external_inventory.is_none() {
                self.external_inventory = Some(
                    crate::infrastructure::external_artifact_inventory::read_inventory(
                        &self.root,
                        self.source_kind,
                        &mut checkpoint,
                    ),
                );
            }
            checkpoint()?;
            let inventory = self
                .external_inventory
                .as_ref()
                .expect("external inventory observed")
                .as_ref()
                .map_err(Clone::clone)?;
            if inventory.get(name) != Some(&binding) {
                return Err(ViewError::new(
                    RefusalCode::ConcurrentChange,
                    "external logical owner differs from its physical placement",
                ));
            }
        }
        if let Some(proved) = self.external.get(binding.descriptor()) {
            if proved.binding != binding {
                return Err(ViewError::new(
                    RefusalCode::ConcurrentChange,
                    "external owner identity changed during search",
                ));
            }
            proved.validate(source)?;
            return Ok(Some(proved.clone()));
        }
        let stamp = self
            .root
            .open_relative_regular_nofollow(binding.descriptor())
            .and_then(|file| file.stamp())
            .map_err(|error| ViewError::new(RefusalCode::InvalidSource, error.to_string()))?;
        let authority = source
            .logical_view_read_authority(cancellation)
            .map_err(|error| ViewError::new(RefusalCode::InvalidSource, error))?
            .with_external_point_binding(binding.clone());
        let owner = at.prefix(1);
        let snapshot = authority.snapshot(&owner)?;
        authority.validate_search_scope_owner(&owner, &snapshot)?;
        let proved = ExternalSearchBinding { binding, stamp };
        proved.validate(source)?;
        self.external
            .insert(proved.binding.descriptor().to_path_buf(), proved.clone());
        Ok(Some(proved))
    }

    /// `Err` only for cancellation or an elapsed deadline; an unreadable
    /// registration source leaves the owner unproven instead.
    fn proves(
        &mut self,
        at: &QualifiedAddress,
        checkpoint: &dyn Fn() -> Result<(), ViewError>,
    ) -> Result<bool, ViewError> {
        let interrupted = |error: &ViewError| {
            matches!(
                error.code(),
                RefusalCode::Cancelled | RefusalCode::DeadlineExceeded
            )
        };
        let Some(owner) = at.segments().first() else {
            return Ok(false);
        };
        let configuration = matches!(
            self.source_kind,
            SourceSetKind::Configuration | SourceSetKind::Extension
        );
        // Application modules belong to the admitted Configuration itself.
        if owner.kind() == NodeKind::Module {
            return Ok(configuration);
        }
        let Some(name) = owner.name() else {
            return Ok(false);
        };
        let key = (owner.kind().as_str(), name.to_owned());
        if let Some(proved) = self.owners.get(&key) {
            return Ok(*proved);
        }
        checkpoint()?;
        let proved = if configuration {
            if self.index.is_none() {
                let built = self.cache.registrations(&self.root, checkpoint);
                self.index = Some(match built {
                    Ok(index) => Some(index),
                    Err(error) if interrupted(&error) => return Err(error),
                    Err(_) => None,
                });
            }
            match self.index.as_ref().and_then(Option::as_ref) {
                Some(index) => match index.contains(owner.kind().as_str(), name, checkpoint) {
                    Ok(contains) => contains,
                    Err(error) if interrupted(&error) => return Err(error),
                    Err(_) => false,
                },
                None => false,
            }
        } else {
            // External paths require the shared typed binding and point proof;
            // a file named after the logical owner is not authority.
            false
        };
        self.owners.insert(key, proved);
        Ok(proved)
    }
}

/// The share of the caller's remaining response budget one walk page may
/// spend before it ends and hands the rest to its cursor.
pub(in crate::infrastructure::daemon) const WALK_PAGE_RESPONSE_BUDGET_DIVISOR: u32 = 2;

/// How many modules with the most matches the volume of a first answer
/// names; the rest is one line of counts.
pub(in crate::infrastructure::daemon) const MAX_TEXT_TOTALS_LEADING_MODULES: usize = 20;

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
    pub(in crate::infrastructure::daemon) external_binding:
        Option<unica_source_designer::external_artifact::ExternalArtifactBinding>,
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
    /// When a walk page ends and leaves the rest to its cursor. A page that
    /// counts the whole corpus does not end by time.
    pub(in crate::infrastructure::daemon) walk_budget: Option<Duration>,
    /// Read the corpus to its end and count it, past the page.
    pub(in crate::infrastructure::daemon) count_totals: bool,
    /// Cancels the search once the page is full and only counting remains.
    #[cfg(test)]
    pub(in crate::infrastructure::daemon) cancel_past_page: bool,
}

/// The exact volume of a whole traversal. Nothing is kept per match: a
/// counter per kind and per module with matches.
#[derive(Debug, Default)]
pub(in crate::infrastructure::daemon) struct TextTotals {
    matches: u64,
    files: u64,
    /// Matches and files per kind of the module's owner, in first-seen order.
    kinds: Vec<(&'static str, u64, u64)>,
    /// Matches and files without a proven module.
    unaddressed: (u64, u64),
    /// Matches per module, in traversal order.
    modules: Vec<(String, u64)>,
    module_index: std::collections::HashMap<String, usize>,
    /// Files whose lines could not all be read.
    uncovered: u64,
}

impl TextTotals {
    fn add(&mut self, module: &Result<ModuleOf, &'static str>, matches: u64, new_file: bool) {
        self.matches += matches;
        self.files += u64::from(new_file);
        let Ok(module) = module else {
            self.unaddressed.0 += matches;
            self.unaddressed.1 += u64::from(new_file);
            return;
        };
        match self
            .kinds
            .iter_mut()
            .find(|(kind, _, _)| *kind == module.kind)
        {
            Some(kind) => {
                kind.1 += matches;
                kind.2 += u64::from(new_file);
            }
            None => self.kinds.push((module.kind, matches, u64::from(new_file))),
        }
        match self.module_index.get(&module.at) {
            Some(index) => self.modules[*index].1 += matches,
            None => {
                self.module_index
                    .insert(module.at.clone(), self.modules.len());
                self.modules.push((module.at.clone(), matches));
            }
        }
    }

    /// The public volume: exact counts, the kinds by their matches, the
    /// `leading` modules with the most matches and one line for the rest.
    pub(in crate::infrastructure::daemon) fn to_value(&self, leading: usize) -> Value {
        let by_count = |left: &(u64, usize), right: &(u64, usize)| {
            right.0.cmp(&left.0).then(left.1.cmp(&right.1))
        };
        let mut kinds = self
            .kinds
            .iter()
            .enumerate()
            .map(|(index, kind)| ((kind.1, index), kind))
            .collect::<Vec<_>>();
        kinds.sort_by(|left, right| by_count(&left.0, &right.0));
        let mut ranked = self
            .modules
            .iter()
            .enumerate()
            .map(|(index, module)| (module.1, index))
            .collect::<Vec<_>>();
        if ranked.len() > leading && leading > 0 {
            ranked.select_nth_unstable_by(leading - 1, by_count);
        }
        ranked.truncate(leading);
        ranked.sort_by(by_count);
        let mut value = Map::new();
        value.insert("matches".into(), self.matches.into());
        value.insert("files".into(), self.files.into());
        value.insert(
            "kinds".into(),
            Value::Object(
                kinds
                    .into_iter()
                    .map(|(_, (kind, matches, files))| {
                        (
                            (*kind).to_owned(),
                            json!({"matches": matches, "files": files}),
                        )
                    })
                    .collect(),
            ),
        );
        if self.unaddressed.0 > 0 {
            value.insert(
                "unaddressed".into(),
                json!({"matches": self.unaddressed.0, "files": self.unaddressed.1}),
            );
        }
        let shown = ranked.iter().map(|(matches, _)| matches).sum::<u64>();
        value.insert(
            "modules".into(),
            Value::Array(
                ranked
                    .iter()
                    .map(|(matches, index)| json!({"at": self.modules[*index].0, "matches": matches}))
                    .collect(),
            ),
        );
        if self.modules.len() > ranked.len() {
            value.insert(
                "rest".into(),
                json!({
                    "modules": self.modules.len() - ranked.len(),
                    "matches": self.matches - self.unaddressed.0 - shown,
                }),
            );
        }
        if self.uncovered > 0 {
            value.insert("uncovered".into(), self.uncovered.into());
        }
        Value::Object(value)
    }
}

/// The proven module of a file and the kind of its owner.
#[derive(Debug, Clone)]
struct ModuleOf {
    external_binding: Option<ExternalSearchBinding>,
    at: String,
    kind: &'static str,
}

/// A collected match and the continuation right after it.
pub(in crate::infrastructure::daemon) struct TextHit {
    pub(in crate::infrastructure::daemon) external_binding: Option<ExternalSearchBinding>,
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
    /// The volume of the whole corpus, when the request counts it.
    pub(in crate::infrastructure::daemon) totals: Option<TextTotals>,
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
    module: Option<Result<ModuleOf, &'static str>>,
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
    external_binding: Option<unica_source_designer::external_artifact::ExternalArtifactBinding>,
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
    /// The page is full; the traversal only counts.
    page_full: bool,
    totals: Option<TextTotals>,
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
            external_binding: None,
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
            page_full: false,
            totals: request.count_totals.then(TextTotals::default),
        }
    }

    fn enter_source(&mut self, source: &'a TextSource<'a>) {
        self.leave_file();
        self.source_set = source.source.source_set_name().to_owned();
        self.scope = source.scope.clone();
        self.source = Some(source.source);
        self.owners = Some(source.source.search_hit_owners());
        self.external_binding = source.external_binding.clone();
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
        let physical_identity = digest(&[self.source_set.as_bytes(), path.as_bytes()]);
        let source = self.source.expect("source entered");
        let external_module = if matches!(
            source.source_kind(),
            unica_source_contract::SourceSetKind::ExternalProcessor
                | unica_source_contract::SourceSetKind::ExternalReport
        ) {
            match self.external_module_of(relative) {
                Ok(module) => Some(module),
                Err(error) => {
                    self.fatal = Some(error);
                    return FileStep::Stop;
                }
            }
        } else {
            None
        };
        // Continuations retain their physical file. The proof of its logical
        // owner must also survive, even when the module bytes are unchanged.
        let identity = match external_module
            .as_ref()
            .and_then(|module| module.as_ref().ok())
            .and_then(|module| module.external_binding.as_ref())
        {
            Some(proof) => digest(&[
                &physical_identity,
                proof.binding.kind().as_bytes(),
                proof.binding.name().as_bytes(),
                proof.binding.descriptor().as_os_str().as_encoded_bytes(),
                format!("{:?}", proof.stamp).as_bytes(),
            ]),
            None => physical_identity,
        };
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
        if self.engine == TextEngine::Walk && self.files_read > 0 && self.totals.is_none() {
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
            file_id: format!("bsl-{}", &hex(&physical_identity)[..16]),
            identity,
            relative: relative.to_path_buf(),
            module: external_module,
            matches: 0,
            chain: [0; 32],
            passing,
            uncovered: false,
        };
        if let Some(module) = &self.request.module {
            match self.module_of(&mut current) {
                Ok(Ok(at)) if at.at == *module => {}
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
    ) -> Result<Result<ModuleOf, &'static str>, TextSearchError> {
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
                        Ok(ModuleOf {
                            external_binding: None,
                            at: at.to_string(),
                            // The configuration's own modules belong to it.
                            kind: match at.segments()[0].kind() {
                                unica_address::address::NodeKind::Module => "Configuration",
                                kind => kind.as_str(),
                            },
                        })
                    } else {
                        Err("owner_unproven")
                    }
                }
            },
        };
        current.module = Some(mapped.clone());
        Ok(mapped)
    }

    fn external_module_of(
        &mut self,
        relative: &Path,
    ) -> Result<Result<ModuleOf, &'static str>, TextSearchError> {
        let source = self.source.expect("source entered");
        let proved = self
            .owners
            .as_mut()
            .expect("source entered")
            .external_module(
                source,
                relative,
                self.external_binding.as_ref(),
                self.cancellation,
            );
        let proof = match proved {
            Ok(Some(proof)) => proof,
            Ok(None) => return Ok(Err("outside_module_layout")),
            Err(error)
                if matches!(
                    error.code(),
                    RefusalCode::Cancelled
                        | RefusalCode::DeadlineExceeded
                        | RefusalCode::ConcurrentChange
                ) =>
            {
                return Err(TextSearchError::Interrupted(error))
            }
            Err(_) => return Ok(Err("owner_unproven")),
        };
        let Some(logical) = proof.binding.logical_path(relative) else {
            return Ok(Err("outside_module_layout"));
        };
        let Some(profile) =
            crate::domain::source_capabilities::bsl_profile_for_source(source.source_profile())
        else {
            return Ok(Err("unsupported_source_profile"));
        };
        let Some(at) = module_address_for_relative(
            source.source_set_name(),
            source.source_kind(),
            profile,
            &logical,
        ) else {
            return Ok(Err("outside_module_layout"));
        };
        Ok(Ok(ModuleOf {
            kind: at.segments()[0].kind().as_str(),
            at: at.to_string(),
            external_binding: Some(proof),
        }))
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
        if let Some(totals) = self.totals.as_mut() {
            totals.uncovered += 1;
        }
        // Past the page the file belongs to the next page's coverage.
        if self.page_full {
            return;
        }
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
        // The walk reads such a page again and names the file.
        self.uncovered_lower_bound = true;
        if self.page_full {
            return;
        }
        self.uncovered += 1;
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
        if let Some(totals) = self.totals.as_mut() {
            totals.add(&module, starts.len() as u64, current.matches == 0);
        }
        if self.page_full {
            current.matches += starts.len() as u64;
            self.current = Some(current);
            return LineStep::Continue;
        }
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
                Ok(module) => {
                    item.insert("at".to_owned(), module.at.clone().into());
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
                external_binding: module
                    .as_ref()
                    .ok()
                    .and_then(|module| module.external_binding.clone()),
                value,
                source_set: self.source_set.clone(),
                next,
            });
            if self.hits.len() >= self.request.wanted {
                self.end = TextScanEnd::Limit;
                if self.totals.is_some() {
                    // The rest of the corpus is only counted.
                    self.page_full = true;
                    #[cfg(test)]
                    if self.request.cancel_past_page {
                        self.cancellation.cancel();
                    }
                } else {
                    step = LineStep::Stop;
                }
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
            totals: self.totals,
        }
    }
}

#[cfg(test)]
mod proof_context_tests {
    use super::*;
    use unica_source_contract::{SourceFormat, SourceProfile};
    use unica_work_control::provider_deadline::ProviderDeadline;
    use unica_workspace::actor::WorkspaceSourceSetInput;

    fn external_source() -> (tempfile::TempDir, ActorReadSourceCapability) {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().join("src");
        std::fs::create_dir_all(root.join("report-source/Ext")).unwrap();
        std::fs::write(
            root.join("report-source.xml"),
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/acceptance/workspace-external-alias/erf/report-source.xml"
            )),
        )
        .unwrap();
        std::fs::write(
            root.join("report-source/Ext/ObjectModule.bsl"),
            "Функция Probe() Экспорт\nВозврат 1;\nКонецФункции\n",
        )
        .unwrap();
        let context = crate::infrastructure::workspace::discover_workspace(Some(
            workspace.path().to_path_buf(),
        ))
        .unwrap();
        let actor = crate::infrastructure::source_workspace::actor_registry()
            .get_or_create(
                &context,
                [WorkspaceSourceSetInput::new(
                    "main",
                    &root,
                    SourceSetKind::ExternalReport,
                    SourceFormat::PlatformXml,
                    SourceProfile::platform_xml_8_3_27_format_2_20(),
                )],
                "canonical-v0.13",
            )
            .unwrap();
        let binding = actor.bind_provider_root("main", &root).unwrap();
        let identity = format!(
            "{}:{}",
            actor.safe_identity_hash().unwrap().as_str(),
            binding.source_set_name()
        );
        let source = super::super::actor_read_source_capability_for_test(
            binding,
            identity,
            ProviderDeadline::no_deadline(),
        );
        (workspace, source)
    }

    #[test]
    fn external_owner_proof_rejects_changed_root_and_kind_before_cached_placement() {
        let (_workspace, source) = external_source();
        let (_foreign_workspace, foreign) = external_source();
        let relative = Path::new("report-source/Ext/ObjectModule.bsl");
        let cancellation = CancellationToken::new();
        let mut owners = source.search_hit_owners();
        for _ in 0..2 {
            assert!(owners
                .external_module(&source, relative, None, &cancellation)
                .unwrap()
                .is_some());
        }
        // A helper reused with another root must not publish its earlier
        // cached placement, even when both descriptors have the same name.
        let original_root = Arc::clone(&owners.root);
        owners.root = foreign.retained_root();
        let error = owners
            .external_module(&source, relative, None, &cancellation)
            .unwrap_err();
        assert_eq!(error.code(), RefusalCode::ConcurrentChange);
        owners.root = original_root;
        owners.source_kind = SourceSetKind::ExternalProcessor;
        let error = owners
            .external_module(&source, relative, None, &cancellation)
            .unwrap_err();
        assert_eq!(error.code(), RefusalCode::ConcurrentChange);
        owners.source_kind = SourceSetKind::ExternalReport;
        assert!(owners
            .external_module(&source, relative, None, &cancellation)
            .unwrap()
            .is_some());
    }
}
