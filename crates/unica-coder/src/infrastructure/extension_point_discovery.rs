use crate::domain::project_sources::{
    discover_project_source_map, ProjectSourceSet, SourceFormat, SourceSetKind,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::bounded_file::{read_bounded_bytes, BoundedReadError};
use crate::infrastructure::internal_adapters::{
    find_definitions, search_rlm_index, BslAnalyzerMcpAdapter,
};
use crate::infrastructure::metadata_kinds::{MetadataKind, METADATA_KINDS};
use crate::infrastructure::native_operations::common::{
    parse_support_state_text, support_status_for_uuid_with_state, SupportState,
};
use crate::infrastructure::native_operations::form::validate_form_snapshot;
use crate::infrastructure::workspace_index::read_bsl_index_status;
use crate::infrastructure::AdapterOutcome;
use roxmltree::{Document, Node};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const SCHEMA_VERSION: u32 = 1;
const DEFAULT_LIMIT: usize = 20;
const MAX_KEYWORDS: usize = 24;
const MAX_SEARCH_QUERIES: usize = 6;
const MAX_DEFINITION_QUERIES: usize = 4;
const MAX_EVIDENCE: usize = 160;
const MAX_METADATA_DESCRIPTORS: usize = 25_000;
const MAX_METADATA_DIRECTORY_ENTRIES: usize = 100_000;
const MAX_METADATA_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_METADATA_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SUPPORT_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_CODE_FILES: usize = 2_000;
const MAX_CODE_MANIFEST_FILES: usize = 50_000;
const MAX_CODE_DIRECTORY_ENTRIES: usize = 150_000;
const MAX_CODE_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CODE_TOTAL_BYTES: u64 = 32 * 1024 * 1024;
const MAX_SCAN_DIRECTORIES: usize = 60_000;
const MAX_CANDIDATES: usize = 500;
const MAX_WARNINGS: usize = 64;
const MAX_MISSING_CHECKS: usize = 64;
const MAX_CODE_IDENTIFIERS: usize = 512;
const MAX_IDENTIFIERS_PER_FILE: usize = 16;

pub(crate) struct ExtensionPointDiscoveryAdapter<'a> {
    code_evidence: &'a dyn CodeEvidenceProvider,
}

impl ExtensionPointDiscoveryAdapter<'static> {
    pub(crate) fn new() -> Self {
        Self {
            code_evidence: &SYSTEM_CODE_EVIDENCE_PROVIDER,
        }
    }
}

impl<'a> ExtensionPointDiscoveryAdapter<'a> {
    #[cfg(test)]
    fn with_code_evidence(code_evidence: &'a dyn CodeEvidenceProvider) -> Self {
        Self { code_evidence }
    }

    pub(crate) fn invoke(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
    ) -> Result<AdapterOutcome, String> {
        let task = args
            .get("task")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{tool_name} requires `task` argument"))?;
        let objects = string_array(args, "objects");
        let proposed_extension_points = string_array(args, "proposedExtensionPoints");
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(DEFAULT_LIMIT);

        let mut discovery = Discovery::new(
            context,
            task,
            objects,
            proposed_extension_points,
            limit,
            self.code_evidence,
        );
        let report = discovery.run(args.get("sourceDir").and_then(Value::as_str))?;
        let candidate_count = report.candidate_extension_points.len();
        let is_partial = report.status == DiscoveryStatus::Partial;
        let has_evidence = !report.evidence.is_empty();
        let stdout = serde_json::to_string(&report)
            .map_err(|error| format!("failed to serialize discovery result: {error}"))?;
        let outer_warnings = report
            .warnings
            .iter()
            .map(|warning| format!("[{}] {}", warning.code, warning.message))
            .chain(report.missing_checks.iter().map(|missing| {
                format!(
                    "[missing check: {} / {}] {}",
                    missing.check, missing.status, missing.reason
                )
            }))
            .collect::<Vec<_>>();
        let summary = if is_partial {
            format!(
                "{tool_name} returned {candidate_count} extension point candidates with partial evidence"
            )
        } else {
            format!(
                "{tool_name} returned {candidate_count} evidence-backed extension point candidates"
            )
        };

        Ok(AdapterOutcome {
            ok: has_evidence || dry_run,
            summary: if dry_run {
                format!("{tool_name} dry run: {summary}")
            } else {
                summary
            },
            changes: Vec::new(),
            warnings: outer_warnings,
            errors: if has_evidence || dry_run {
                Vec::new()
            } else {
                vec!["discovery could not inspect any supported source provider".to_string()]
            },
            artifacts: Vec::new(),
            stdout: Some(stdout),
            stderr: None,
            command: None,
        })
    }
}

impl Default for ExtensionPointDiscoveryAdapter<'static> {
    fn default() -> Self {
        Self::new()
    }
}

fn string_array(args: &Map<String, Value>, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

trait CodeEvidenceProvider {
    fn index_state(&self, context: &WorkspaceContext, source_root: &Path) -> DiscoveryIndexState;

    fn search_rlm(
        &self,
        query: &str,
        db_path: &Path,
        source_root: &Path,
        limit: usize,
    ) -> Result<Vec<ProviderHit>, String>;

    fn find_definition(
        &self,
        name: &str,
        db_path: &Path,
        source_root: &Path,
        limit: usize,
    ) -> Result<Vec<ProviderHit>, String>;

    fn check_graph(
        &self,
        query: &str,
        source_dir: &Path,
        context: &WorkspaceContext,
        limit: usize,
    ) -> Result<ProviderCheck, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderHit {
    target: String,
    path: PathBuf,
    line: usize,
}

#[derive(Debug, Clone, Copy)]
struct ProviderCheck {
    matched: bool,
    degraded: bool,
}

struct SystemCodeEvidenceProvider;

static SYSTEM_CODE_EVIDENCE_PROVIDER: SystemCodeEvidenceProvider = SystemCodeEvidenceProvider;

impl CodeEvidenceProvider for SystemCodeEvidenceProvider {
    fn index_state(&self, context: &WorkspaceContext, source_root: &Path) -> DiscoveryIndexState {
        let Some(status) = read_bsl_index_status(context) else {
            return DiscoveryIndexState::Missing;
        };
        let Some(indexed_source_root) = status.source_root.as_deref() else {
            return DiscoveryIndexState::Unavailable(
                "index status does not identify its source root".to_string(),
            );
        };
        let indexed_source_root = status_path(context, indexed_source_root);
        let same_source = fs::canonicalize(indexed_source_root)
            .ok()
            .zip(fs::canonicalize(source_root).ok())
            .is_some_and(|(indexed, selected)| indexed == selected);
        if !same_source {
            return DiscoveryIndexState::Unavailable(
                "index source does not match the selected sourceDir".to_string(),
            );
        }
        match status.status.to_ascii_lowercase().as_str() {
            "ready" => match status.db_path {
                Some(db_path) => match safe_index_db_path(context, &db_path) {
                    Some(db_path) => DiscoveryIndexState::Ready { db_path },
                    None => DiscoveryIndexState::Unavailable(
                        "index database is not a regular file inside the workspace cache"
                            .to_string(),
                    ),
                },
                _ => DiscoveryIndexState::Unavailable(
                    "index status is ready but its database is unavailable".to_string(),
                ),
            },
            "stale" => DiscoveryIndexState::Stale,
            "building" => DiscoveryIndexState::Building,
            "failed" => {
                DiscoveryIndexState::Failed("RLM/BSL index reported failed status".to_string())
            }
            other => DiscoveryIndexState::Unavailable(format!(
                "RLM/BSL index reported unsupported status `{}`",
                bounded_status(other)
            )),
        }
    }

    fn search_rlm(
        &self,
        query: &str,
        db_path: &Path,
        source_root: &Path,
        limit: usize,
    ) -> Result<Vec<ProviderHit>, String> {
        let mut args = Map::new();
        args.insert("query".to_string(), json!(query));
        args.insert("limit".to_string(), json!(limit.min(20)));
        let output = search_rlm_index(db_path, &args)?.unwrap_or_default();
        Ok(parse_index_hits(&output, query, source_root, limit))
    }

    fn find_definition(
        &self,
        name: &str,
        db_path: &Path,
        source_root: &Path,
        limit: usize,
    ) -> Result<Vec<ProviderHit>, String> {
        let mut args = Map::new();
        args.insert("name".to_string(), json!(name));
        args.insert("limit".to_string(), json!(limit.min(20)));
        let output = find_definitions(db_path, &args)?;
        Ok(parse_index_hits(&output, name, source_root, limit))
    }

    fn check_graph(
        &self,
        query: &str,
        source_dir: &Path,
        context: &WorkspaceContext,
        limit: usize,
    ) -> Result<ProviderCheck, String> {
        let mut args = Map::new();
        args.insert("mode".to_string(), json!("resolve"));
        args.insert("query".to_string(), json!(query));
        args.insert("sourceDir".to_string(), json!(source_dir));
        args.insert("limit".to_string(), json!(limit.min(20)));
        let outcome =
            BslAnalyzerMcpAdapter::new().invoke("unica.code.graph", &args, context, false)?;
        Ok(ProviderCheck {
            matched: outcome.stdout.as_deref().is_some_and(code_graph_has_match),
            degraded: !outcome.warnings.is_empty(),
        })
    }
}

fn status_path(context: &WorkspaceContext, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        context.workspace_root.join(path)
    }
}

fn safe_index_db_path(context: &WorkspaceContext, raw: &str) -> Option<PathBuf> {
    let path = status_path(context, raw);
    let file_type = fs::symlink_metadata(&path).ok()?.file_type();
    if file_type.is_symlink() || !file_type.is_file() {
        return None;
    }
    let cache_root = fs::canonicalize(&context.cache_root).ok()?;
    let db_path = fs::canonicalize(path).ok()?;
    (db_path.starts_with(cache_root) && db_path.is_file()).then_some(db_path)
}

fn parse_index_hits(
    output: &str,
    target: &str,
    source_root: &Path,
    limit: usize,
) -> Vec<ProviderHit> {
    output
        .lines()
        .filter_map(|line| parse_index_hit(line, target, source_root))
        .take(limit.min(20))
        .collect()
}

fn parse_index_hit(line: &str, target: &str, source_root: &Path) -> Option<ProviderHit> {
    let mut fields = line.trim().strip_prefix("- ")?.split_whitespace();
    let location = fields.next()?;
    let _method_type = fields.next()?;
    let indexed_target = fields
        .next()
        .and_then(|signature| signature.split('(').next())
        .filter(|name| !name.is_empty())
        .unwrap_or(target);
    let (raw_path, raw_line) = location.rsplit_once(':')?;
    let line = raw_line.parse::<usize>().ok().filter(|line| *line > 0)?;
    let path = PathBuf::from(raw_path);
    let path = if path.is_absolute() {
        path
    } else {
        source_root.join(path)
    };
    let path = contained_file(source_root, &path)?;
    Some(ProviderHit {
        target: bounded_text(indexed_target, 128),
        path,
        line,
    })
}

fn code_graph_has_match(stdout: &str) -> bool {
    let Some(json_start) = stdout.find('{') else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&stdout[json_start..]) else {
        return false;
    };
    ["nodes", "edges", "results", "matches"].iter().any(|key| {
        value
            .get(*key)
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty())
    }) || value.get("node").is_some_and(|node| !node.is_null())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DiscoveryIndexState {
    Ready { db_path: PathBuf },
    Missing,
    Stale,
    Building,
    Failed(String),
    Unavailable(String),
}

impl DiscoveryIndexState {
    fn status(&self) -> &'static str {
        match self {
            Self::Ready { .. } => "ready",
            Self::Missing => "missing",
            Self::Stale => "stale",
            Self::Building => "building",
            Self::Failed(_) => "failed",
            Self::Unavailable(_) => "unavailable",
        }
    }

    fn reason_code(&self) -> &'static str {
        match self {
            Self::Ready { .. } => "ready",
            Self::Missing => "index_missing",
            Self::Stale => "index_stale",
            Self::Building => "index_building",
            Self::Failed(_) => "index_failed",
            Self::Unavailable(_) => "backend_unavailable",
        }
    }

    fn detail(&self) -> Option<&str> {
        match self {
            Self::Failed(detail) | Self::Unavailable(detail) => Some(detail),
            Self::Ready { .. } | Self::Missing | Self::Stale | Self::Building => None,
        }
    }
}

struct Discovery<'a> {
    context: &'a WorkspaceContext,
    seeds: Vec<String>,
    proposed: Vec<String>,
    limit: usize,
    code_evidence: &'a dyn CodeEvidenceProvider,
    terms: Vec<Term>,
    keywords: Vec<String>,
    candidates: BTreeMap<String, CandidateBuilder>,
    warnings: Vec<DiscoveryWarning>,
    evidence: BTreeSet<DiscoveryEvidence>,
    missing_checks: Vec<MissingCheck>,
    code_identifiers: BTreeSet<String>,
    validated_code_identities: BTreeSet<String>,
    support_state: Option<SupportState>,
    metadata_bytes: u64,
    code_bytes: u64,
    scanned_metadata_entries: usize,
    scanned_code_entries: usize,
    scanned_code_files: usize,
    scanned_directories: usize,
}

impl<'a> Discovery<'a> {
    fn new(
        context: &'a WorkspaceContext,
        task: &'a str,
        seeds: Vec<String>,
        proposed: Vec<String>,
        limit: usize,
        code_evidence: &'a dyn CodeEvidenceProvider,
    ) -> Self {
        let mut priority_keywords = Vec::new();
        enrich_domain_keywords(task, &mut priority_keywords);
        for object in seeds.iter().chain(proposed.iter()) {
            if let Some(leaf) = object.split('.').next_back() {
                push_keyword(&mut priority_keywords, leaf);
            }
        }
        let mut keywords = priority_keywords.clone();
        for keyword in task_keywords(task) {
            push_keyword(&mut keywords, &keyword);
        }
        let terms = terms_from_keywords(&keywords, &priority_keywords);
        Self {
            context,
            seeds,
            proposed,
            limit,
            code_evidence,
            terms,
            keywords,
            candidates: BTreeMap::new(),
            warnings: Vec::new(),
            evidence: BTreeSet::new(),
            missing_checks: Vec::new(),
            code_identifiers: BTreeSet::new(),
            validated_code_identities: BTreeSet::new(),
            support_state: None,
            metadata_bytes: 0,
            code_bytes: 0,
            scanned_metadata_entries: 0,
            scanned_code_entries: 0,
            scanned_code_files: 0,
            scanned_directories: 0,
        }
    }

    fn run(&mut self, explicit_source_dir: Option<&str>) -> Result<DiscoveryReport, String> {
        let source = resolve_source(self.context, explicit_source_dir)?;
        let configuration = source.absolute_path.join("Configuration.xml");
        if let Some(configuration) = contained_file(&source.absolute_path, &configuration) {
            self.add_evidence(
                "project_map",
                &source.source_set,
                Some(workspace_relative(
                    &self.context.workspace_root,
                    &configuration,
                )),
                None,
                format!(
                    "Selected `{}` source-set with sourceFormat={}.",
                    source.source_set,
                    source_format_name(source.source_format)
                ),
            );
        }
        if source.source_format != SourceFormat::PlatformXml {
            self.add_missing(MissingCheck::new(
                "metadata/form discovery",
                source_format_name(source.source_format),
                "unsupported_source_format",
                "Discovery schema v1 supports platform XML metadata; code checks may still be available.",
            ));
        } else {
            self.load_support_state(&source.absolute_path);
            self.scan_platform_metadata(&source.absolute_path);
            self.scan_workspace_code(&source.absolute_path);
        }

        let index_state = self
            .code_evidence
            .index_state(self.context, &source.absolute_path);
        self.run_index_checks(&source.absolute_path, index_state);
        self.add_architecture_warnings();

        if self.candidates.is_empty() && !self.evidence.is_empty() {
            self.add_warning(DiscoveryWarning::new(
                "no_candidates",
                "Проверки завершены, но подтверждённые candidate extension points не найдены.",
            ));
        }

        let mut candidates = self
            .candidates
            .values()
            .cloned()
            .map(CandidateBuilder::finish)
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| normalize(&left.object).cmp(&normalize(&right.object)))
        });
        if candidates.len() > self.limit {
            candidates.truncate(self.limit);
            self.add_missing(MissingCheck::new(
                "candidate result set",
                "truncated",
                "result_truncated",
                format!("Result was limited to {} candidates.", self.limit),
            ));
        }

        self.keywords.truncate(MAX_KEYWORDS);
        self.warnings.sort_by(|left, right| {
            left.code
                .cmp(&right.code)
                .then_with(|| left.message.cmp(&right.message))
        });
        self.warnings
            .dedup_by(|left, right| left.code == right.code && left.message == right.message);
        self.missing_checks.sort_by(|left, right| {
            left.check
                .cmp(&right.check)
                .then_with(|| left.status.cmp(&right.status))
        });
        self.missing_checks.dedup_by(|left, right| {
            left.check == right.check && left.status == right.status && left.reason == right.reason
        });

        Ok(DiscoveryReport {
            schema_version: SCHEMA_VERSION,
            status: if self.missing_checks.is_empty() {
                DiscoveryStatus::Complete
            } else {
                DiscoveryStatus::Partial
            },
            source: DiscoverySource {
                source_dir: workspace_relative(&self.context.workspace_root, &source.absolute_path),
                source_set: source.source_set,
                source_format: source_format_name(source.source_format).to_string(),
            },
            keywords: self.keywords.clone(),
            candidate_extension_points: candidates,
            warnings: self.warnings.clone(),
            evidence: self.evidence.iter().cloned().collect(),
            missing_checks: self.missing_checks.clone(),
        })
    }

    fn scan_platform_metadata(&mut self, source_root: &Path) {
        let mut descriptors = Vec::<(MetadataKind, PathBuf)>::new();
        'kinds: for kind in METADATA_KINDS {
            let raw_directory = source_root.join(kind.directory);
            if !raw_directory.exists() {
                continue;
            }
            let Some(directory) = contained_directory(source_root, &raw_directory) else {
                self.add_missing(MissingCheck::new(
                    "metadata directory",
                    "rejected",
                    "backend_unavailable",
                    format!(
                        "Refusing metadata directory `{}` that resolves outside sourceDir.",
                        kind.directory
                    ),
                ));
                continue;
            };
            let directory_entries = match fs::read_dir(&directory) {
                Ok(entries) => entries,
                Err(_) => {
                    self.add_missing(MissingCheck::new(
                        "metadata descriptor scan",
                        "failed",
                        "backend_unavailable",
                        format!(
                            "Could not enumerate `{}` metadata descriptors.",
                            kind.directory
                        ),
                    ));
                    continue;
                }
            };
            for entry in directory_entries {
                if self.scanned_metadata_entries >= MAX_METADATA_DIRECTORY_ENTRIES {
                    self.add_missing(MissingCheck::new(
                        "metadata descriptor scan",
                        "truncated",
                        "result_truncated",
                        format!(
                            "Metadata directory enumeration is limited to {MAX_METADATA_DIRECTORY_ENTRIES} entries."
                        ),
                    ));
                    break 'kinds;
                }
                self.scanned_metadata_entries += 1;
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(_) => {
                        self.add_missing(MissingCheck::new(
                            "metadata descriptor scan",
                            "degraded",
                            "backend_unavailable",
                            "At least one metadata directory entry could not be read.",
                        ));
                        continue;
                    }
                };
                let file_type = match entry.file_type() {
                    Ok(file_type) => file_type,
                    Err(_) => {
                        self.add_missing(MissingCheck::new(
                            "metadata descriptor scan",
                            "degraded",
                            "backend_unavailable",
                            "At least one metadata entry type could not be inspected.",
                        ));
                        continue;
                    }
                };
                let path = entry.path();
                if file_type.is_symlink() {
                    if path.extension().and_then(|extension| extension.to_str()) == Some("xml") {
                        self.add_missing(MissingCheck::new(
                            "metadata descriptor scan",
                            "rejected",
                            "backend_unavailable",
                            "Refusing a symlinked metadata descriptor.",
                        ));
                    }
                    continue;
                }
                if file_type.is_file()
                    && path.extension().and_then(|extension| extension.to_str()) == Some("xml")
                {
                    descriptors.push((*kind, path));
                    if descriptors.len() >= MAX_METADATA_DESCRIPTORS {
                        self.add_missing(MissingCheck::new(
                            "metadata descriptor scan",
                            "truncated",
                            "result_truncated",
                            format!(
                                "Metadata scan is limited to {MAX_METADATA_DESCRIPTORS} descriptors."
                            ),
                        ));
                        break 'kinds;
                    }
                }
            }
        }
        descriptors.sort_by(|left, right| {
            self.descriptor_priority(right.0, &right.1)
                .cmp(&self.descriptor_priority(left.0, &left.1))
                .then_with(|| left.1.cmp(&right.1))
        });

        for (kind, path) in descriptors {
            self.inspect_descriptor(source_root, kind, &path);
            if self.metadata_bytes >= MAX_METADATA_TOTAL_BYTES {
                break;
            }
        }
    }

    fn descriptor_priority(&self, kind: MetadataKind, path: &Path) -> u16 {
        let name = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let object = format!("{}.{}", kind.tag, name);
        if self.is_seed_object(&object) {
            100
        } else {
            self.relevance_bonus(name)
        }
    }

    fn inspect_descriptor(&mut self, source_root: &Path, kind: MetadataKind, path: &Path) {
        let Some(safe_path) = contained_file(source_root, path) else {
            self.add_missing(MissingCheck::new(
                "metadata descriptor",
                "rejected",
                "backend_unavailable",
                "Refusing a metadata descriptor that resolves outside sourceDir.",
            ));
            return;
        };
        let path = safe_path.as_path();
        let Some(text) = self.read_metadata_utf8(source_root, path, "metadata descriptor") else {
            return;
        };
        let doc = match Document::parse(text.trim_start_matches('\u{feff}')) {
            Ok(doc) => doc,
            Err(error) => {
                self.add_missing(MissingCheck::new(
                    "metadata descriptor",
                    "failed",
                    "backend_unavailable",
                    format!(
                        "{}: {error}",
                        workspace_relative(&self.context.workspace_root, path)
                    ),
                ));
                return;
            }
        };
        let Some(metadata_node) = doc
            .descendants()
            .find(|node| node.is_element() && node.tag_name().name() == kind.tag)
        else {
            return;
        };
        let Some(name) = property_name(metadata_node) else {
            return;
        };
        let object = format!("{}.{}", kind.tag, name);
        self.validated_code_identities.insert(object.clone());
        let relative_path = workspace_relative(&self.context.workspace_root, path);
        let child_keyword_match =
            element_child(metadata_node, "ChildObjects").is_some_and(|children| {
                children.children().filter(Node::is_element).any(|child| {
                    if child.tag_name().name() == "Form" {
                        child
                            .text()
                            .map(str::trim)
                            .is_some_and(|name| self.matches_terms(name))
                    } else {
                        child.descendants().filter(Node::is_element).any(|node| {
                            property_name(node)
                                .as_deref()
                                .is_some_and(|name| self.matches_terms(name))
                        })
                    }
                })
            });
        let is_context = self.is_seed_object(&object);
        let relevance_quality = self.term_match_quality(&name);
        let relevance_bonus = self.relevance_bonus(&name);
        let is_keyword_match = relevance_bonus > 0;
        let is_relevant = is_context || is_keyword_match || child_keyword_match;
        let base_score = 25
            + if is_context { 25 } else { 0 }
            + relevance_bonus
            + if relevance_quality == 6 { 25 } else { 0 }
            + if child_keyword_match { 10 } else { 0 };
        if is_relevant {
            self.add_evidence(
                "platform_xml",
                &object,
                Some(relative_path.clone()),
                None,
                "Metadata object identity and structure were parsed from platform XML.",
            );
            let mut reason_codes = vec!["metadata_structure"];
            if is_context {
                reason_codes.push("context_object");
            }
            if is_keyword_match || child_keyword_match {
                reason_codes.push("keyword_match");
            }
            self.add_candidate(
                &object,
                "metadata_object",
                base_score,
                &reason_codes,
                format!(
                    "Метаданные подтверждают объект `{object}` как часть исследуемого механизма."
                ),
                format!("platform_xml: {relative_path}"),
            );
            if is_keyword_match {
                push_keyword(&mut self.keywords, &name);
            }

            if self.support_state.as_ref().is_some_and(|state| {
                let support =
                    support_status_for_uuid_with_state(metadata_node.attribute("uuid"), state);
                support.contains("на замке") || support.contains("read-only")
            }) {
                self.add_warning(
                    DiscoveryWarning::new(
                        "vendor_supported_object",
                        "Найдены объекты на поддержке/замке; прямая правка рискованна, проверьте вариант расширения CFE.",
                    )
                    .with_object(object.clone())
                    .with_evidence(relative_path.clone()),
                );
            }
        }

        if let Some(child_objects) = element_child(metadata_node, "ChildObjects") {
            for child in child_objects.children().filter(Node::is_element) {
                match child.tag_name().name() {
                    "TabularSection" => {
                        self.inspect_tabular_section(&object, &relative_path, child)
                    }
                    "Attribute" => self.inspect_attribute(&object, &relative_path, child),
                    "Form" => {
                        if let Some(form_name) =
                            child.text().map(str::trim).filter(|v| !v.is_empty())
                        {
                            self.validated_code_identities
                                .insert(format!("{object}.Form.{form_name}"));
                            self.inspect_form(source_root, kind, &name, &object, form_name, path);
                        }
                    }
                    "Command" => {
                        if let Some(command_name) =
                            child.text().map(str::trim).filter(|v| !v.is_empty())
                        {
                            self.validated_code_identities
                                .insert(format!("{object}.Command.{command_name}"));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn inspect_tabular_section(&mut self, parent: &str, descriptor_path: &str, node: Node<'_, '_>) {
        let Some(name) = property_name(node) else {
            return;
        };
        let child_keyword_match = element_child(node, "ChildObjects").is_some_and(|children| {
            children
                .children()
                .filter(|child| child.is_element() && child.tag_name().name() == "Attribute")
                .filter_map(property_name)
                .any(|attribute| self.matches_terms(&attribute))
        });
        if !self.matches_terms(&name) && !self.is_seed_object(parent) && !child_keyword_match {
            return;
        }
        let object = format!("{parent}.TabularSection.{name}");
        let match_score = self.relevance_bonus(&name);
        let context_score = if self.is_seed_object(parent) { 25 } else { 0 };
        let parent_score = self.relevance_bonus(parent);
        let evidence = format!("platform_xml: {descriptor_path}#TabularSection.{name}");
        self.add_evidence(
            "platform_xml",
            &object,
            Some(descriptor_path.to_string()),
            None,
            format!("Platform XML declares tabular section `{name}`."),
        );
        let mut reason_codes = vec!["metadata_structure"];
        if context_score > 0 {
            reason_codes.push("context_object");
        }
        if match_score > 0 || child_keyword_match {
            reason_codes.push("keyword_match");
        }
        self.add_candidate(
            &object,
            "tabular_section",
            25 + match_score + context_score + parent_score,
            &reason_codes,
            format!("Объект `{parent}` содержит отдельную табличную часть `{name}`."),
            evidence,
        );
        if self.matches_terms(&name) {
            push_keyword(&mut self.keywords, &name);
        }

        if let Some(children) = element_child(node, "ChildObjects") {
            for attribute in children
                .children()
                .filter(|child| child.is_element() && child.tag_name().name() == "Attribute")
            {
                let Some(attribute_name) = property_name(attribute) else {
                    continue;
                };
                if !self.matches_terms(&attribute_name) {
                    continue;
                }
                let attribute_object = format!("{object}.Attribute.{attribute_name}");
                self.add_candidate(
                    &attribute_object,
                    "attribute",
                    55,
                    &["metadata_structure", "keyword_match", "context_object"],
                    format!(
                        "Табличная часть `{name}` содержит связанный реквизит `{attribute_name}`."
                    ),
                    format!(
                        "platform_xml: {descriptor_path}#TabularSection.{name}.Attribute.{attribute_name}"
                    ),
                );
                push_keyword(&mut self.keywords, &attribute_name);
            }
        }
    }

    fn inspect_attribute(&mut self, parent: &str, descriptor_path: &str, node: Node<'_, '_>) {
        let Some(name) = property_name(node) else {
            return;
        };
        if !self.matches_terms(&name) {
            return;
        }
        let object = format!("{parent}.Attribute.{name}");
        self.add_candidate(
            &object,
            "attribute",
            35 + self.relevance_bonus(&name) + if self.is_seed_object(parent) { 25 } else { 0 },
            &["metadata_structure", "keyword_match"],
            format!("Метаданные объекта `{parent}` содержат связанный реквизит `{name}`."),
            format!("platform_xml: {descriptor_path}#Attribute.{name}"),
        );
        push_keyword(&mut self.keywords, &name);
    }

    fn inspect_form(
        &mut self,
        source_root: &Path,
        kind: MetadataKind,
        parent_name: &str,
        parent: &str,
        form_name: &str,
        descriptor_path: &Path,
    ) {
        let object = format!("{parent}.Form.{form_name}");
        if !self.matches_terms(form_name) && !self.is_seed_object(&object) {
            return;
        }
        let forms_root = source_root
            .join(kind.directory)
            .join(parent_name)
            .join("Forms");
        let managed_form = forms_root.join(form_name).join("Ext").join("Form.xml");
        let confirmed_path = contained_file(source_root, &managed_form).and_then(|path| {
            let text = self.read_metadata_utf8(source_root, &path, "managed form")?;
            valid_managed_form_snapshot(self.context, &path, &text).then_some(path)
        });
        if confirmed_path.is_none() && managed_form.exists() {
            self.add_missing(MissingCheck::new(
                "unica.form.validate",
                "failed",
                "backend_unavailable",
                format!("Managed form `{object}` did not pass native form validation."),
            ));
        }
        let confirmed = confirmed_path.is_some();
        let path = confirmed_path.unwrap_or_else(|| descriptor_path.to_path_buf());
        let relative_path = workspace_relative(&self.context.workspace_root, &path);
        self.add_evidence(
            if confirmed {
                "managed_form_xml"
            } else {
                "platform_xml"
            },
            &object,
            Some(relative_path.clone()),
            None,
            if confirmed {
                "Registered form passed the native platform XML validator."
            } else {
                "Form is registered in the parent metadata descriptor."
            },
        );
        let match_bonus = self
            .relevance_bonus(form_name)
            .max(self.relevance_bonus(parent_name));
        self.add_candidate(
            &object,
            "form",
            35 + match_bonus + if confirmed { 10 } else { 0 },
            if confirmed {
                &["metadata_structure", "keyword_match", "form_confirmation"]
            } else {
                &["metadata_structure", "keyword_match"]
            },
            format!(
                "Форма `{form_name}` зарегистрирована в `{parent}` и участвует в связанном пользовательском сценарии."
            ),
            format!(
                "{}: {relative_path}",
                if confirmed {
                    "unica.form.validate"
                } else {
                    "platform_xml"
                }
            ),
        );
        push_keyword(&mut self.keywords, form_name);
    }

    fn scan_workspace_code(&mut self, source_root: &Path) {
        let mut pending = vec![source_root.to_path_buf()];
        let mut code_paths = Vec::new();
        'enumeration: while let Some(directory) = pending.pop() {
            if self.scanned_directories >= MAX_SCAN_DIRECTORIES {
                self.add_missing(MissingCheck::new(
                    "filesystem lexical scan",
                    "truncated",
                    "result_truncated",
                    format!("Directory scan is limited to {MAX_SCAN_DIRECTORIES} directories."),
                ));
                break;
            }
            self.scanned_directories += 1;
            let directory_entries = match fs::read_dir(&directory) {
                Ok(entries) => entries,
                Err(_) => {
                    self.add_missing(MissingCheck::new(
                        "filesystem lexical scan",
                        "degraded",
                        "backend_unavailable",
                        "At least one source directory could not be enumerated.",
                    ));
                    continue;
                }
            };
            let mut entries = Vec::new();
            for entry in directory_entries {
                if self.scanned_code_entries >= MAX_CODE_DIRECTORY_ENTRIES {
                    self.add_missing(MissingCheck::new(
                        "filesystem lexical scan",
                        "truncated",
                        "result_truncated",
                        format!(
                            "Source directory enumeration is limited to {MAX_CODE_DIRECTORY_ENTRIES} entries."
                        ),
                    ));
                    break 'enumeration;
                }
                self.scanned_code_entries += 1;
                match entry {
                    Ok(entry) => entries.push(entry),
                    Err(_) => self.add_missing(MissingCheck::new(
                        "filesystem lexical scan",
                        "degraded",
                        "backend_unavailable",
                        "At least one source directory entry could not be read.",
                    )),
                }
            }
            entries.sort_by_key(|entry| entry.path());
            for entry in entries.into_iter().rev() {
                let file_type = match entry.file_type() {
                    Ok(file_type) => file_type,
                    Err(_) => {
                        self.add_missing(MissingCheck::new(
                            "filesystem lexical scan",
                            "degraded",
                            "backend_unavailable",
                            "At least one source entry type could not be inspected.",
                        ));
                        continue;
                    }
                };
                if file_type.is_symlink() {
                    self.add_missing(MissingCheck::new(
                        "filesystem lexical scan",
                        "rejected",
                        "backend_unavailable",
                        "Refusing a symlink while scanning BSL source.",
                    ));
                    continue;
                }
                if file_type.is_dir() {
                    if let Some(path) = contained_directory(source_root, &entry.path()) {
                        pending.push(path);
                    } else {
                        self.add_missing(MissingCheck::new(
                            "filesystem lexical scan",
                            "rejected",
                            "backend_unavailable",
                            "Refusing a source directory that resolves outside sourceDir.",
                        ));
                    }
                    continue;
                }
                if !file_type.is_file()
                    || entry.path().extension().and_then(|value| value.to_str()) != Some("bsl")
                {
                    continue;
                }
                if code_paths.len() >= MAX_CODE_MANIFEST_FILES {
                    self.add_missing(MissingCheck::new(
                        "filesystem lexical scan",
                        "truncated",
                        "result_truncated",
                        format!("BSL manifest is limited to {MAX_CODE_MANIFEST_FILES} files."),
                    ));
                    break 'enumeration;
                }
                code_paths.push(entry.path());
            }
        }

        code_paths.sort_by(|left, right| {
            self.relevance_bonus(&right.to_string_lossy())
                .cmp(&self.relevance_bonus(&left.to_string_lossy()))
                .then_with(|| left.cmp(right))
        });
        for raw_path in code_paths {
            if self.scanned_code_files >= MAX_CODE_FILES {
                self.add_missing(MissingCheck::new(
                    "filesystem lexical scan",
                    "truncated",
                    "result_truncated",
                    format!("Code scan is limited to {MAX_CODE_FILES} files."),
                ));
                break;
            }
            self.scanned_code_files += 1;
            let Some(path) = contained_file(source_root, &raw_path) else {
                self.add_missing(MissingCheck::new(
                    "filesystem lexical scan",
                    "rejected",
                    "backend_unavailable",
                    "Refusing a BSL source file that resolves outside sourceDir.",
                ));
                continue;
            };
            let Some(text) = self.read_code_utf8(source_root, &path) else {
                if self.code_bytes >= MAX_CODE_TOTAL_BYTES {
                    break;
                }
                continue;
            };
            self.inspect_code_file(source_root, &path, &text);
        }
    }

    fn inspect_code_file(&mut self, source_root: &Path, path: &Path, text: &str) {
        let Some(module_object) = self.validated_scope_for_code_path(source_root, path) else {
            if code_scope_for_path(source_root, path).is_some() {
                self.add_missing(MissingCheck::new(
                    "BSL metadata identity",
                    "rejected",
                    "unverified_identity",
                    "Ignored BSL source whose object or nested form/command is not registered by a parsed metadata descriptor.",
                ));
            }
            return;
        };
        let mut matched_identifiers = BTreeMap::<String, usize>::new();
        'lines: for (line_index, line) in text.lines().enumerate() {
            let line = bsl_without_comment(line);
            for identifier in identifiers(line) {
                if identifier.chars().count() > 128 || !self.matches_terms(identifier) {
                    continue;
                }
                if !matched_identifiers.contains_key(identifier)
                    && matched_identifiers.len() >= MAX_IDENTIFIERS_PER_FILE
                {
                    self.add_missing(MissingCheck::new(
                        "BSL identifiers per file",
                        "truncated",
                        "result_truncated",
                        format!(
                            "Task-related identifiers are limited to {MAX_IDENTIFIERS_PER_FILE} per BSL file."
                        ),
                    ));
                    break 'lines;
                }
                matched_identifiers
                    .entry(identifier.to_string())
                    .or_insert(line_index + 1);
            }
        }
        if matched_identifiers.is_empty() {
            return;
        }
        let relative_path = workspace_relative(&self.context.workspace_root, path);
        let first_line = matched_identifiers.values().copied().min();
        let finding = matched_identifiers
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        self.add_evidence(
            "bsl_lexical_scan",
            &module_object,
            Some(relative_path.clone()),
            first_line,
            format!("Task-related lexical terms occur in BSL source: {finding}."),
        );
        self.add_candidate(
            &module_object,
            "module",
            45,
            &["source_path_identity", "code_reference"],
            format!("Модуль `{module_object}` содержит связанные с задачей лексические термины: {finding}."),
            format!(
                "bsl_lexical_scan: {relative_path}:{}",
                first_line.unwrap_or(1)
            ),
        );
        for identifier in matched_identifiers.into_keys() {
            if self.code_identifiers.len() >= MAX_CODE_IDENTIFIERS {
                self.add_missing(MissingCheck::new(
                    "BSL identifier collection",
                    "truncated",
                    "result_truncated",
                    format!(
                        "Code identifier collection is limited to {MAX_CODE_IDENTIFIERS} items."
                    ),
                ));
                break;
            }
            self.code_identifiers.insert(identifier.clone());
            push_keyword(&mut self.keywords, &identifier);
        }
    }

    fn run_index_checks(&mut self, source_root: &Path, index_state: DiscoveryIndexState) {
        let db_path = match index_state {
            DiscoveryIndexState::Ready { db_path } => db_path,
            other => {
                let detail = other
                    .detail()
                    .unwrap_or("RLM/BSL index did not report ready state.");
                self.add_missing(MissingCheck::new(
                    "bsl_index",
                    other.status(),
                    other.reason_code(),
                    detail,
                ));
                self.add_missing(MissingCheck::new(
                    "unica.code.definition",
                    "not_run",
                    other.reason_code(),
                    "Definition checks require a ready RLM/BSL index.",
                ));
                self.add_missing(MissingCheck::new(
                    "unica.code.graph",
                    "not_run",
                    other.reason_code(),
                    "Candidate graph checks were not run because indexed code symbols were not resolved.",
                ));
                return;
            }
        };

        let mut queries = self
            .keywords
            .iter()
            .filter(|keyword| keyword.chars().count() >= 4)
            .cloned()
            .collect::<Vec<_>>();
        let code_identifiers = self
            .code_identifiers
            .iter()
            .map(|identifier| normalize(identifier))
            .collect::<BTreeSet<_>>();
        queries.sort_by(|left, right| {
            let left_normalized = normalize(left);
            let right_normalized = normalize(right);
            let left_priority = usize::from(!code_identifiers.contains(&left_normalized));
            let right_priority = usize::from(!code_identifiers.contains(&right_normalized));
            left_priority
                .cmp(&right_priority)
                .then_with(|| right_normalized.len().cmp(&left_normalized.len()))
                .then_with(|| left_normalized.cmp(&right_normalized))
        });
        queries.dedup_by(|left, right| normalize(left) == normalize(right));
        if queries.len() > MAX_SEARCH_QUERIES {
            self.add_missing(MissingCheck::new(
                "unica.code.search query set",
                "truncated",
                "result_truncated",
                format!("Code search is limited to {MAX_SEARCH_QUERIES} task queries."),
            ));
        }
        queries.truncate(MAX_SEARCH_QUERIES);

        if queries.is_empty() {
            self.add_missing(MissingCheck::new(
                "unica.code.search",
                "not_run",
                "no_search_terms",
                "RLM method search had no bounded task terms to query.",
            ));
        }
        for query in queries {
            match self
                .code_evidence
                .search_rlm(&query, &db_path, source_root, self.limit)
            {
                Ok(hits) => {
                    for hit in hits {
                        self.record_provider_hit(
                            source_root,
                            &hit,
                            "rlm_method_index",
                            "rlm_method_hit",
                            55,
                            15,
                        );
                    }
                }
                Err(error) => self.add_missing(MissingCheck::new(
                    "unica.code.search",
                    "failed",
                    "backend_unavailable",
                    bounded_detail(&error),
                )),
            }
        }

        let mut definition_names = self.code_identifiers.iter().cloned().collect::<Vec<_>>();
        definition_names.sort_by(|left, right| {
            normalize(right)
                .len()
                .cmp(&normalize(left).len())
                .then_with(|| normalize(left).cmp(&normalize(right)))
        });
        if definition_names.len() > MAX_DEFINITION_QUERIES {
            self.add_missing(MissingCheck::new(
                "unica.code.definition query set",
                "truncated",
                "result_truncated",
                format!("Definition lookup is limited to {MAX_DEFINITION_QUERIES} symbol queries."),
            ));
        }
        definition_names.truncate(MAX_DEFINITION_QUERIES);
        if definition_names.is_empty() {
            self.add_missing(MissingCheck::new(
                "unica.code.definition",
                "not_run",
                "no_symbol_candidates",
                "No BSL symbol candidates were collected for definition lookup.",
            ));
            self.add_missing(MissingCheck::new(
                "unica.code.graph",
                "not_run",
                "no_symbol_candidates",
                "Typed graph resolution requires a symbol confirmed by definition lookup.",
            ));
            return;
        }
        let mut graph_anchor = None;
        for name in definition_names {
            match self
                .code_evidence
                .find_definition(&name, &db_path, source_root, self.limit)
            {
                Ok(hits) => {
                    for hit in hits {
                        if self.record_provider_hit(
                            source_root,
                            &hit,
                            "rlm_definition",
                            "definition",
                            65,
                            15,
                        ) {
                            graph_anchor.get_or_insert(hit);
                        }
                    }
                    if graph_anchor.is_some() {
                        break;
                    }
                }
                Err(error) => self.add_missing(MissingCheck::new(
                    "unica.code.definition",
                    "failed",
                    "backend_unavailable",
                    bounded_detail(&error),
                )),
            }
        }

        if let Some(anchor) = graph_anchor {
            match self.code_evidence.check_graph(
                &anchor.target,
                source_root,
                self.context,
                self.limit,
            ) {
                Ok(check) => {
                    if check.matched {
                        let relative_path =
                            workspace_relative(&self.context.workspace_root, &anchor.path);
                        self.add_evidence(
                            "bsl_graph",
                            &anchor.target,
                            Some(relative_path.clone()),
                            Some(anchor.line),
                            format!("Typed code graph resolved `{}`.", anchor.target),
                        );
                        if let Some(scope) =
                            self.validated_scope_for_code_path(source_root, &anchor.path)
                        {
                            self.boost_candidates_for_scope(
                                &scope,
                                15,
                                "graph_relation",
                                &format!("unica.code.graph: {relative_path}:{}", anchor.line),
                            );
                        }
                    }
                    if check.degraded {
                        self.add_missing(MissingCheck::new(
                            "unica.code.graph",
                            "degraded",
                            "backend_unavailable",
                            "Code graph reported a non-ready or partial backend state.",
                        ));
                    }
                }
                Err(_) => self.add_missing(MissingCheck::new(
                    "unica.code.graph",
                    "failed",
                    "backend_unavailable",
                    "Typed code graph check did not complete.",
                )),
            }
        } else {
            self.add_missing(MissingCheck::new(
                "unica.code.graph",
                "not_run",
                "symbol_not_resolved",
                "Definition lookup completed but did not resolve a graph anchor.",
            ));
        }
    }

    fn record_provider_hit(
        &mut self,
        source_root: &Path,
        hit: &ProviderHit,
        source: &'static str,
        reason_code: &'static str,
        candidate_score: u16,
        related_bonus: u16,
    ) -> bool {
        let Some(scope) = self.validated_scope_for_code_path(source_root, &hit.path) else {
            if code_scope_for_path(source_root, &hit.path).is_some() {
                self.add_missing(MissingCheck::new(
                    "indexed BSL metadata identity",
                    "rejected",
                    "unverified_identity",
                    "Ignored an indexed BSL hit whose object or nested form/command is not registered by a parsed metadata descriptor.",
                ));
            }
            return false;
        };
        let relative_path = workspace_relative(&self.context.workspace_root, &hit.path);
        self.add_evidence(
            source,
            &scope,
            Some(relative_path.clone()),
            Some(hit.line),
            format!(
                "Indexed BSL symbol `{}` is anchored in `{scope}`.",
                hit.target
            ),
        );
        let evidence = format!("{source}: {relative_path}:{}", hit.line);
        self.add_candidate(
            &scope,
            "module",
            candidate_score,
            &["source_path_identity", reason_code],
            format!(
                "Индекс BSL подтверждает символ `{}` в модуле `{scope}`.",
                hit.target
            ),
            evidence.clone(),
        );
        if self.code_identifiers.len() < MAX_CODE_IDENTIFIERS {
            self.code_identifiers.insert(hit.target.clone());
            push_keyword(&mut self.keywords, &hit.target);
        } else {
            self.add_missing(MissingCheck::new(
                "BSL identifier collection",
                "truncated",
                "result_truncated",
                format!("Code identifier collection is limited to {MAX_CODE_IDENTIFIERS} items."),
            ));
        }
        self.boost_candidates_for_scope(&scope, related_bonus, reason_code, &evidence);
        true
    }

    fn boost_candidates_for_scope(
        &mut self,
        scope: &str,
        score: u16,
        reason_code: &str,
        evidence: &str,
    ) {
        for candidate in self.candidates.values_mut() {
            if (scope == candidate.object
                || scope.starts_with(&(candidate.object.clone() + "."))
                || candidate.object.starts_with(&(scope.to_string() + ".")))
                && candidate.reason_codes.insert(reason_code.to_string())
            {
                candidate.score = (candidate.score + score).min(100);
                candidate.evidence.insert(evidence.to_string());
            }
        }
    }

    fn add_architecture_warnings(&mut self) {
        let mut dedicated_sections = self
            .candidates
            .keys()
            .filter(|object| {
                object.contains(".TabularSection.")
                    && object
                        .split('.')
                        .next_back()
                        .is_some_and(|name| normalize(name) == "серии")
            })
            .cloned()
            .collect::<Vec<_>>();
        if dedicated_sections.is_empty() {
            return;
        }
        if dedicated_sections.len() > 32 {
            dedicated_sections.truncate(32);
            self.add_missing(MissingCheck::new(
                "architecture warning evidence",
                "truncated",
                "result_truncated",
                "Architecture warning evidence is limited to 32 objects.",
            ));
        }
        let checks_only_goods_series = !self.proposed.is_empty()
            && self.proposed.iter().all(|object| {
                let normalized = normalize(object);
                normalized.contains("товары") && normalized.contains("серия")
            });
        let code = if checks_only_goods_series {
            "proposed_point_may_be_incomplete"
        } else {
            "separate_tabular_section"
        };
        let message = if checks_only_goods_series {
            "Предложена проверка только по `Товары.Серия`, но найдена отдельная табличная часть `Серии`; такая точка может не покрыть типовой сценарий."
        } else {
            "Найдена отдельная табличная часть `Серии`; проверка только по `Товары.Серия` может не покрыть типовой сценарий."
        };
        let mut warning = DiscoveryWarning::new(code, message);
        for object in dedicated_sections {
            warning.objects.push(bounded_text(&object, 512));
            if let Some(candidate) = self.candidates.get(&object) {
                let remaining = 32usize.saturating_sub(warning.evidence.len());
                warning.evidence.extend(
                    candidate
                        .evidence
                        .iter()
                        .take(remaining)
                        .map(|evidence| bounded_text(evidence, 1024)),
                );
            }
        }
        self.add_warning(warning);
    }

    fn load_support_state(&mut self, source_root: &Path) {
        let raw_path = source_root.join("Ext").join("ParentConfigurations.bin");
        if !raw_path.exists() {
            return;
        }
        let file_type = match fs::symlink_metadata(&raw_path) {
            Ok(metadata) => metadata.file_type(),
            Err(_) => {
                self.add_missing(MissingCheck::new(
                    "support state",
                    "failed",
                    "backend_unavailable",
                    "Support-state metadata could not be inspected.",
                ));
                return;
            }
        };
        if file_type.is_symlink() || !file_type.is_file() {
            self.add_missing(MissingCheck::new(
                "support state",
                "rejected",
                "backend_unavailable",
                "Refusing a non-regular or symlinked support-state file.",
            ));
            return;
        }
        let Some(path) = contained_file(source_root, &raw_path) else {
            self.add_missing(MissingCheck::new(
                "support state",
                "rejected",
                "backend_unavailable",
                "Refusing a support-state file that resolves outside sourceDir.",
            ));
            return;
        };
        let contents = match read_bounded_bytes(&path, MAX_SUPPORT_FILE_BYTES, Some(source_root)) {
            Ok(contents) => contents,
            Err(BoundedReadError::TooLarge) => {
                self.add_missing(MissingCheck::new(
                    "support state",
                    "truncated",
                    "result_truncated",
                    "Support-state file exceeds its dedicated bounded read budget.",
                ));
                return;
            }
            Err(BoundedReadError::NotRegular) => {
                self.add_missing(MissingCheck::new(
                    "support state",
                    "rejected",
                    "backend_unavailable",
                    "Refusing a non-regular support-state file.",
                ));
                return;
            }
            Err(BoundedReadError::InvalidUtf8) => {
                self.add_missing(MissingCheck::new(
                    "support state",
                    "failed",
                    "backend_unavailable",
                    "Support-state byte reader reported an unexpected decoding failure.",
                ));
                return;
            }
            Err(BoundedReadError::Open | BoundedReadError::Metadata | BoundedReadError::Read) => {
                self.add_missing(MissingCheck::new(
                    "support state",
                    "failed",
                    "backend_unavailable",
                    "Support-state file could not be read.",
                ));
                return;
            }
        };
        let text = String::from_utf8_lossy(&contents.bytes);
        self.support_state = parse_support_state_text(&text, contents.bytes.len());
        if self.support_state.is_none() {
            self.add_missing(MissingCheck::new(
                "support state",
                "failed",
                "backend_unavailable",
                "Support-state file could not be parsed.",
            ));
        }
    }

    fn read_metadata_utf8(
        &mut self,
        source_root: &Path,
        path: &Path,
        check: &str,
    ) -> Option<String> {
        let remaining = MAX_METADATA_TOTAL_BYTES.saturating_sub(self.metadata_bytes);
        let max_bytes = MAX_METADATA_FILE_BYTES.min(remaining);
        match read_bounded_utf8(path, max_bytes, Some(source_root)) {
            Ok(contents) => {
                self.metadata_bytes += contents.bytes_read;
                Some(contents.text)
            }
            Err(BoundedReadError::TooLarge) if remaining < MAX_METADATA_FILE_BYTES => {
                self.metadata_bytes = MAX_METADATA_TOTAL_BYTES;
                self.add_missing(MissingCheck::new(
                    "metadata scan",
                    "truncated",
                    "result_truncated",
                    format!(
                        "Metadata scan is limited to {} MiB in total.",
                        MAX_METADATA_TOTAL_BYTES / 1024 / 1024
                    ),
                ));
                None
            }
            Err(BoundedReadError::TooLarge) => {
                self.add_missing(MissingCheck::new(
                    check,
                    "truncated",
                    "result_truncated",
                    format!(
                        "Skipped oversized metadata file `{}`.",
                        workspace_relative(&self.context.workspace_root, path)
                    ),
                ));
                None
            }
            Err(BoundedReadError::NotRegular) => {
                self.add_missing(MissingCheck::new(
                    check,
                    "rejected",
                    "backend_unavailable",
                    "Refusing a non-regular metadata file.",
                ));
                None
            }
            Err(BoundedReadError::InvalidUtf8) => {
                self.add_missing(MissingCheck::new(
                    check,
                    "failed",
                    "backend_unavailable",
                    format!(
                        "Metadata file `{}` is not valid UTF-8.",
                        workspace_relative(&self.context.workspace_root, path)
                    ),
                ));
                None
            }
            Err(BoundedReadError::Open | BoundedReadError::Metadata | BoundedReadError::Read) => {
                self.add_missing(MissingCheck::new(
                    check,
                    "failed",
                    "backend_unavailable",
                    format!(
                        "Could not read `{}`.",
                        workspace_relative(&self.context.workspace_root, path)
                    ),
                ));
                None
            }
        }
    }

    fn read_code_utf8(&mut self, source_root: &Path, path: &Path) -> Option<String> {
        let remaining = MAX_CODE_TOTAL_BYTES.saturating_sub(self.code_bytes);
        let max_bytes = MAX_CODE_FILE_BYTES.min(remaining);
        match read_bounded_utf8(path, max_bytes, Some(source_root)) {
            Ok(contents) => {
                self.code_bytes += contents.bytes_read;
                Some(contents.text)
            }
            Err(BoundedReadError::TooLarge) if remaining < MAX_CODE_FILE_BYTES => {
                self.code_bytes = MAX_CODE_TOTAL_BYTES;
                self.add_missing(MissingCheck::new(
                    "filesystem lexical scan",
                    "truncated",
                    "result_truncated",
                    format!(
                        "BSL scan is limited to {} MiB in total.",
                        MAX_CODE_TOTAL_BYTES / 1024 / 1024
                    ),
                ));
                None
            }
            Err(BoundedReadError::TooLarge) => {
                self.add_missing(MissingCheck::new(
                    "filesystem lexical scan",
                    "truncated",
                    "result_truncated",
                    "Skipped an oversized BSL source file.",
                ));
                None
            }
            Err(BoundedReadError::NotRegular) => {
                self.add_missing(MissingCheck::new(
                    "filesystem lexical scan",
                    "rejected",
                    "backend_unavailable",
                    "Refusing a non-regular BSL source file.",
                ));
                None
            }
            Err(BoundedReadError::InvalidUtf8) => {
                self.add_missing(MissingCheck::new(
                    "filesystem lexical scan",
                    "degraded",
                    "backend_unavailable",
                    "At least one BSL source file could not be read as UTF-8.",
                ));
                None
            }
            Err(BoundedReadError::Open | BoundedReadError::Metadata | BoundedReadError::Read) => {
                self.add_missing(MissingCheck::new(
                    "filesystem lexical scan",
                    "degraded",
                    "backend_unavailable",
                    "At least one BSL source file could not be read.",
                ));
                None
            }
        }
    }

    fn add_warning(&mut self, warning: DiscoveryWarning) {
        if let Some(index) = self.warnings.iter().position(|existing| {
            existing.code == warning.code && existing.message == warning.message
        }) {
            let existing = &mut self.warnings[index];
            let mut truncated = false;
            for object in warning.objects {
                if existing.objects.contains(&object) {
                    continue;
                }
                if existing.objects.len() >= 32 {
                    truncated = true;
                    break;
                }
                existing.objects.push(object);
            }
            for evidence in warning.evidence {
                if existing.evidence.contains(&evidence) {
                    continue;
                }
                if existing.evidence.len() >= 32 {
                    truncated = true;
                    break;
                }
                existing.evidence.push(evidence);
            }
            if truncated {
                self.add_missing(MissingCheck::new(
                    "warning evidence set",
                    "truncated",
                    "result_truncated",
                    "Aggregated warning objects/evidence are limited to 32 items.",
                ));
            }
            return;
        }
        if self.warnings.len() >= MAX_WARNINGS {
            self.add_missing(MissingCheck::new(
                "warning result set",
                "truncated",
                "result_truncated",
                format!("Warning output is limited to {MAX_WARNINGS} items."),
            ));
            return;
        }
        self.warnings.push(warning);
    }

    fn add_missing(&mut self, missing: MissingCheck) {
        if self.missing_checks.iter().any(|existing| {
            existing.check == missing.check
                && existing.status == missing.status
                && existing.reason == missing.reason
                && existing.detail == missing.detail
        }) {
            return;
        }
        if self.missing_checks.len() < MAX_MISSING_CHECKS.saturating_sub(1) {
            self.missing_checks.push(missing);
        } else if !self
            .missing_checks
            .iter()
            .any(|existing| existing.check == "missing check ledger")
        {
            self.missing_checks.push(MissingCheck::new(
                "missing check ledger",
                "truncated",
                "result_truncated",
                format!("Missing-check output is limited to {MAX_MISSING_CHECKS} items."),
            ));
        }
    }

    fn add_candidate(
        &mut self,
        object: &str,
        kind: &'static str,
        score: u16,
        reason_codes: &[&str],
        reason: String,
        evidence: String,
    ) {
        if object.chars().count() > 512 {
            self.add_missing(MissingCheck::new(
                "candidate identity",
                "truncated",
                "result_truncated",
                "Skipped an oversized candidate identity.",
            ));
            return;
        }
        if !self.candidates.contains_key(object) && self.candidates.len() >= MAX_CANDIDATES {
            self.add_missing(MissingCheck::new(
                "candidate collection",
                "truncated",
                "result_truncated",
                format!("Candidate collection is limited to {MAX_CANDIDATES} items."),
            ));
            return;
        }
        let candidate = self
            .candidates
            .entry(object.to_string())
            .or_insert_with(|| CandidateBuilder::new(object, kind));
        candidate.score = candidate.score.max(score.min(100));
        candidate.reason = bounded_detail(&reason);
        candidate.evidence.insert(bounded_text(&evidence, 1024));
        for code in reason_codes {
            candidate.reason_codes.insert((*code).to_string());
        }
    }

    fn add_evidence(
        &mut self,
        source: &'static str,
        target: &str,
        path: Option<String>,
        line: Option<usize>,
        finding: impl Into<String>,
    ) {
        let evidence = DiscoveryEvidence {
            source: bounded_text(source, 64),
            target: bounded_text(target, 512),
            path: path.map(|path| bounded_text(&path, 1024)),
            line,
            finding: bounded_text(&finding.into(), 500),
        };
        if self.evidence.contains(&evidence) {
            return;
        }
        if self.evidence.len() >= MAX_EVIDENCE {
            self.add_missing(MissingCheck::new(
                "evidence result set",
                "truncated",
                "result_truncated",
                format!("Evidence output is limited to {MAX_EVIDENCE} items."),
            ));
            return;
        }
        self.evidence.insert(evidence);
    }

    fn is_seed_object(&self, object: &str) -> bool {
        let normalized = normalize(object);
        self.seeds.iter().any(|seed| {
            let seed = normalize(seed);
            seed == normalized
                || seed.starts_with(&(normalized.clone() + "."))
                || normalized.starts_with(&(seed + "."))
        })
    }

    fn term_match_quality(&self, value: &str) -> u8 {
        let normalized = normalize(value);
        self.terms
            .iter()
            .map(|term| {
                let long_priority = term.priority && term.normalized.chars().count() >= 10;
                if normalized == term.normalized && long_priority {
                    6
                } else if normalized.contains(&term.normalized) && long_priority {
                    5
                } else if normalized.contains(&term.normalized) {
                    if term.priority {
                        4
                    } else {
                        3
                    }
                } else if term.allow_stem && normalized.contains(&term.stem) {
                    if term.priority {
                        2
                    } else {
                        1
                    }
                } else {
                    0
                }
            })
            .max()
            .unwrap_or(0)
    }

    fn matches_terms(&self, value: &str) -> bool {
        self.term_match_quality(value) > 0
    }

    fn relevance_bonus(&self, value: &str) -> u16 {
        match self.term_match_quality(value) {
            6 => 60,
            5 => 45,
            4 => 30,
            3 => 20,
            2 => 10,
            1 => 5,
            _ => 0,
        }
    }

    fn validated_scope_for_code_path(&self, source_root: &Path, path: &Path) -> Option<String> {
        let scope = code_scope_for_path(source_root, path)?;
        self.validated_code_identities
            .contains(&scope.identity)
            .then_some(scope.scope)
    }
}

#[derive(Clone)]
struct CandidateBuilder {
    object: String,
    kind: &'static str,
    score: u16,
    reason_codes: BTreeSet<String>,
    reason: String,
    evidence: BTreeSet<String>,
}

impl CandidateBuilder {
    fn new(object: &str, kind: &'static str) -> Self {
        Self {
            object: object.to_string(),
            kind,
            score: 0,
            reason_codes: BTreeSet::new(),
            reason: String::new(),
            evidence: BTreeSet::new(),
        }
    }

    fn finish(self) -> CandidateExtensionPoint {
        CandidateExtensionPoint {
            object: self.object,
            kind: self.kind.to_string(),
            score: self.score,
            confidence: match self.score {
                70..=100 => "high",
                40..=69 => "medium",
                _ => "low",
            }
            .to_string(),
            reason_codes: self.reason_codes.into_iter().collect(),
            reason: self.reason,
            evidence: self.evidence.into_iter().take(8).collect(),
        }
    }
}

#[derive(Debug, Clone)]
struct Term {
    normalized: String,
    stem: String,
    priority: bool,
    allow_stem: bool,
}

fn terms_from_keywords(keywords: &[String], priority_keywords: &[String]) -> Vec<Term> {
    let priorities = priority_keywords
        .iter()
        .map(|keyword| normalize(keyword))
        .collect::<BTreeSet<_>>();
    let mut terms = BTreeMap::<String, Term>::new();
    for keyword in keywords {
        let normalized = normalize(keyword);
        if normalized.chars().count() < 4 {
            continue;
        }
        let priority = priorities.contains(&normalized);
        let character_count = normalized.chars().count();
        let stem_length = if priority && normalized.starts_with("сери") {
            4
        } else if character_count >= 8 {
            character_count.saturating_sub(2)
        } else {
            character_count
        };
        let stem = normalized.chars().take(stem_length).collect::<String>();
        terms.entry(normalized.clone()).or_insert(Term {
            normalized,
            stem,
            priority,
            allow_stem: priority || character_count >= 8,
        });
    }
    terms.into_values().collect()
}

fn task_keywords(task: &str) -> Vec<String> {
    const STOPWORDS: &[&str] = &[
        "для",
        "при",
        "если",
        "как",
        "или",
        "это",
        "через",
        "только",
        "нужно",
        "надо",
        "должен",
        "должна",
        "должно",
        "товаров",
        "товара",
        "документ",
        "документа",
        "контроль",
        "контролировать",
        "меньше",
        "больше",
        "указанный",
        "указанного",
        "процент",
    ];
    let stopwords = STOPWORDS.iter().copied().collect::<BTreeSet<_>>();
    let mut keywords = Vec::new();
    for token in identifiers(task) {
        let normalized = normalize(token);
        if normalized.chars().count() < 4
            || normalized
                .chars()
                .all(|character| character.is_ascii_digit())
            || stopwords.contains(normalized.as_str())
        {
            continue;
        }
        push_keyword(&mut keywords, token);
    }
    keywords
}

fn enrich_domain_keywords(task: &str, keywords: &mut Vec<String>) {
    let normalized = normalize_with_spaces(task);
    if normalized.contains("срок годности") {
        push_keyword(keywords, "ГоденДо");
        push_keyword(keywords, "СрокГодности");
    }
    if normalized.contains("дата производства") {
        push_keyword(keywords, "ДатаПроизводства");
    }
    if normalized.contains("сери") {
        push_keyword(keywords, "Серия");
        push_keyword(keywords, "Серии");
    }
    if normalized.contains("поступлен") {
        push_keyword(keywords, "ПриобретениеТоваровУслуг");
    }
    if normalized.contains("поступлен") && normalized.contains("сери") {
        push_keyword(keywords, "ПодборСерийВДокументы");
        push_keyword(keywords, "ПодборСерий");
    }
}

fn push_keyword(keywords: &mut Vec<String>, value: &str) {
    if keywords.len() >= MAX_KEYWORDS || value.trim().is_empty() {
        return;
    }
    let value = bounded_text(value.trim(), 128);
    let normalized = normalize(&value);
    if keywords
        .iter()
        .any(|existing| normalize(existing) == normalized)
    {
        return;
    }
    keywords.push(value);
}

fn identifiers(text: &str) -> impl Iterator<Item = &str> {
    text.split(|character: char| !(character.is_alphanumeric() || character == '_'))
        .filter(|token| !token.is_empty())
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric() || *character == '.')
        .flat_map(char::to_lowercase)
        .map(|character| if character == 'ё' { 'е' } else { character })
        .collect()
}

fn normalize_with_spaces(value: &str) -> String {
    identifiers(value)
        .map(normalize)
        .collect::<Vec<_>>()
        .join(" ")
}

fn element_child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == name)
}

fn property_name(node: Node<'_, '_>) -> Option<String> {
    element_child(node, "Properties")
        .and_then(|properties| element_child(properties, "Name"))
        .and_then(|name| name.text())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
}

fn valid_managed_form_snapshot(context: &WorkspaceContext, path: &Path, text: &str) -> bool {
    validate_form_snapshot(path, text, context).ok
}

fn module_suffix_from_path(path: &str) -> String {
    let path = Path::new(path);
    let file_stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Module");
    let parent = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if file_stem == "Module" && !parent.is_empty() && parent != "Ext" {
        format!("{parent}Module")
    } else {
        file_stem.to_string()
    }
}

struct CodeScope {
    scope: String,
    identity: String,
}

fn code_scope_for_path(source_root: &Path, path: &Path) -> Option<CodeScope> {
    if path.extension().and_then(|extension| extension.to_str()) != Some("bsl") {
        return None;
    }
    let source_root = fs::canonicalize(source_root).ok()?;
    let path = fs::canonicalize(path).ok()?;
    let relative = path.strip_prefix(&source_root).ok()?;
    let parts = relative
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>();
    let directory = *parts.first()?;
    let kind = METADATA_KINDS
        .iter()
        .find(|kind| kind.directory == directory)?;
    let object_name = *parts.get(1)?;
    let base_identity = format!("{}.{}", kind.tag, object_name);
    let mut identity = base_identity.clone();

    if let Some(index) = parts.iter().position(|part| *part == "Forms") {
        if let Some(form_name) = parts.get(index + 1) {
            identity = format!("{base_identity}.Form.{form_name}");
        }
    } else if let Some(index) = parts.iter().position(|part| *part == "Commands") {
        if let Some(command_name) = parts.get(index + 1) {
            identity = format!("{base_identity}.Command.{command_name}");
        }
    }

    let module = module_suffix_from_path(&relative.display().to_string());
    let scope = if kind.tag == "CommonModule" && module == "Module" {
        identity.clone()
    } else {
        format!("{identity}.{module}")
    };
    Some(CodeScope { scope, identity })
}

fn bsl_without_comment(line: &str) -> &str {
    let mut in_string = false;
    let mut chars = line.char_indices().peekable();
    while let Some((index, character)) = chars.next() {
        if character == '"' {
            if in_string && chars.peek().is_some_and(|(_, next)| *next == '"') {
                chars.next();
            } else {
                in_string = !in_string;
            }
            continue;
        }
        if !in_string && character == '/' && chars.peek().is_some_and(|(_, next)| *next == '/') {
            return &line[..index];
        }
    }
    line
}

struct ResolvedSource {
    absolute_path: PathBuf,
    source_set: String,
    source_format: SourceFormat,
}

fn resolve_source(
    context: &WorkspaceContext,
    explicit_source_dir: Option<&str>,
) -> Result<ResolvedSource, String> {
    let workspace = fs::canonicalize(&context.workspace_root).map_err(|error| {
        format!(
            "failed to resolve workspace root {}: {error}",
            context.workspace_root.display()
        )
    })?;
    let source_map = discover_project_source_map(&context.workspace_root)?;
    if let Some(raw) = explicit_source_dir {
        let candidate = PathBuf::from(raw);
        let candidate = if candidate.is_absolute() {
            candidate
        } else {
            context.cwd.join(candidate)
        };
        let candidate = fs::canonicalize(&candidate)
            .map_err(|error| format!("failed to resolve sourceDir `{raw}`: {error}"))?;
        if !candidate.starts_with(&workspace) {
            return Err(
                "unica.project.discover sourceDir must stay inside the workspace".to_string(),
            );
        }
        let source_set = source_map.source_sets.iter().find(|source_set| {
            fs::canonicalize(context.workspace_root.join(&source_set.path))
                .ok()
                .is_some_and(|path| path == candidate)
        });
        let source_format = source_set
            .map(|source_set| source_set.source_format)
            .unwrap_or_else(|| detect_explicit_format(&candidate));
        return Ok(ResolvedSource {
            absolute_path: candidate,
            source_set: source_set
                .map(|source_set| source_set.name.clone())
                .unwrap_or_else(|| "explicit".to_string()),
            source_format,
        });
    }

    let candidates = source_map
        .source_sets
        .iter()
        .filter(|source_set| source_set.kind == SourceSetKind::Configuration)
        .collect::<Vec<_>>();
    let source_set =
        match candidates.as_slice() {
            [source_set] => *source_set,
            [] => {
                return Err(
                    "unica.project.discover could not find a configuration source-set".to_string(),
                )
            }
            _ => return Err(
                "unica.project.discover found multiple configuration source-sets; pass sourceDir"
                    .to_string(),
            ),
        };
    resolve_configured_source(context, &workspace, source_set)
}

fn resolve_configured_source(
    context: &WorkspaceContext,
    workspace: &Path,
    source_set: &ProjectSourceSet,
) -> Result<ResolvedSource, String> {
    let candidate =
        fs::canonicalize(context.workspace_root.join(&source_set.path)).map_err(|error| {
            format!(
                "failed to resolve source-set `{}` at {}: {error}",
                source_set.name, source_set.path
            )
        })?;
    if !candidate.starts_with(workspace) {
        return Err(format!(
            "source-set `{}` resolves outside the workspace",
            source_set.name
        ));
    }
    Ok(ResolvedSource {
        absolute_path: candidate,
        source_set: source_set.name.clone(),
        source_format: source_set.source_format,
    })
}

fn detect_explicit_format(path: &Path) -> SourceFormat {
    if path.join("Configuration.xml").is_file() {
        SourceFormat::PlatformXml
    } else if path.join("Configuration/Configuration.mdo").is_file()
        || path.join("src/Configuration/Configuration.mdo").is_file()
    {
        SourceFormat::Edt
    } else {
        SourceFormat::Unknown
    }
}

fn source_format_name(format: SourceFormat) -> &'static str {
    match format {
        SourceFormat::PlatformXml => "platform_xml",
        SourceFormat::Edt => "edt",
        SourceFormat::Unknown => "unknown",
        SourceFormat::Invalid => "invalid",
    }
}

fn contained_file(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let root = fs::canonicalize(root).ok()?;
    let candidate = fs::canonicalize(candidate).ok()?;
    (candidate.starts_with(root) && candidate.is_file()).then_some(candidate)
}

fn contained_directory(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let root = fs::canonicalize(root).ok()?;
    let candidate = fs::canonicalize(candidate).ok()?;
    (candidate.starts_with(root) && candidate.is_dir()).then_some(candidate)
}

struct BoundedUtf8 {
    text: String,
    bytes_read: u64,
}

fn read_bounded_utf8(
    path: &Path,
    max_bytes: u64,
    containment_root: Option<&Path>,
) -> Result<BoundedUtf8, BoundedReadError> {
    let contents = read_bounded_bytes(path, max_bytes, containment_root)?;
    let text = String::from_utf8(contents.bytes).map_err(|_| BoundedReadError::InvalidUtf8)?;
    Ok(BoundedUtf8 {
        text,
        bytes_read: contents.bytes_read,
    })
}

fn workspace_relative(root: &Path, path: &Path) -> String {
    let canonical_root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let canonical_path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let value = canonical_path
        .strip_prefix(&canonical_root)
        .unwrap_or(&canonical_path)
        .display()
        .to_string();
    value.replace('\\', "/")
}

fn bounded_detail(detail: &str) -> String {
    bounded_text(detail, 500)
}

fn bounded_text(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn bounded_status(status: &str) -> String {
    status
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
        .take(32)
        .collect()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiscoveryReport {
    schema_version: u32,
    status: DiscoveryStatus,
    source: DiscoverySource,
    keywords: Vec<String>,
    candidate_extension_points: Vec<CandidateExtensionPoint>,
    warnings: Vec<DiscoveryWarning>,
    evidence: Vec<DiscoveryEvidence>,
    missing_checks: Vec<MissingCheck>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DiscoveryStatus {
    Complete,
    Partial,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiscoverySource {
    source_dir: String,
    source_set: String,
    source_format: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CandidateExtensionPoint {
    object: String,
    kind: String,
    score: u16,
    confidence: String,
    reason_codes: Vec<String>,
    reason: String,
    evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiscoveryWarning {
    code: String,
    message: String,
    objects: Vec<String>,
    evidence: Vec<String>,
}

impl DiscoveryWarning {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        let code = code.into();
        let message = message.into();
        Self {
            code: bounded_text(&code, 64),
            message: bounded_text(&message, 500),
            objects: Vec::new(),
            evidence: Vec::new(),
        }
    }

    fn with_object(mut self, object: String) -> Self {
        if self.objects.len() < 32 {
            self.objects.push(bounded_text(&object, 512));
        }
        self
    }

    fn with_evidence(mut self, evidence: String) -> Self {
        if self.evidence.len() < 32 {
            self.evidence.push(bounded_text(&evidence, 1024));
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiscoveryEvidence {
    source: String,
    target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<usize>,
    finding: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MissingCheck {
    check: String,
    status: String,
    reason: String,
    detail: String,
}

impl MissingCheck {
    fn new(
        check: impl Into<String>,
        status: impl Into<String>,
        reason: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        let check = check.into();
        let status = status.into();
        let reason = reason.into();
        let detail = detail.into();
        Self {
            check: bounded_text(&check, 128),
            status: bounded_text(&status, 64),
            reason: bounded_text(&reason, 64),
            detail: bounded_text(&detail, 500),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::UnicaApplication;
    use sha2::{Digest, Sha256};
    use std::cell::RefCell;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn project_discover_dry_run_accepts_an_empty_explicit_source() {
        let workspace = temp_root("unica-discovery-dry-run");
        fs::create_dir_all(&workspace).unwrap();
        let context = WorkspaceContext::discover(workspace).unwrap();
        let mut args = Map::new();
        args.insert("task".to_string(), json!("Проверить точки расширения"));
        args.insert("sourceDir".to_string(), json!("."));

        let outcome = ExtensionPointDiscoveryAdapter::new()
            .invoke("unica.project.discover", &args, &context, true)
            .unwrap();

        assert!(outcome.ok);
        assert!(outcome.errors.is_empty());
        assert!(outcome.summary.contains("dry run"));
    }

    #[test]
    fn project_discover_finds_ut115_series_flow_when_bsl_index_is_stale() {
        let (_root, workspace) = fixture_workspace("unica-discovery-stale");
        write_index_status(&workspace, "stale");
        let before = source_snapshot(&workspace.join("src"));
        let result = call_discovery(
            &workspace,
            Some(vec![
                "Document.ПриобретениеТоваровУслуг.TabularSection.Товары.Серия",
            ]),
        );
        let after = source_snapshot(&workspace.join("src"));

        assert!(result.ok, "{:?}", result.errors);
        assert!(result.errors.is_empty());
        assert_eq!(
            result.stdout, None,
            "structured data must not be hidden in stdout"
        );
        assert_eq!(result.cache.mode, "read");
        assert!(result.cache.events.is_empty());
        assert!(result.cache.stale.contains(&"bsl_index".to_string()));
        assert_eq!(before, after, "read-only discovery changed its source-set");

        let data = result.data.expect("discovery has structured data");
        assert_eq!(data["schemaVersion"], 1);
        assert_eq!(data["status"], "partial");
        assert!(data.get("task").is_none(), "raw task must not be echoed");
        assert_eq!(data["source"]["sourceDir"], "src");
        assert_eq!(data["source"]["sourceFormat"], "platform_xml");

        let expected = [
            "Document.ПриобретениеТоваровУслуг.TabularSection.Серии",
            "DataProcessor.ПодборСерийВДокументы",
            "DataProcessor.ПодборСерийВДокументы.Form.РегистрацияИПодборСерийПоОднойСтрокеТоваров",
        ];
        let candidates = data["candidateExtensionPoints"].as_array().unwrap();
        for expected_object in expected {
            let matching = candidates
                .iter()
                .filter(|candidate| candidate["object"] == expected_object)
                .collect::<Vec<_>>();
            assert_eq!(
                matching.len(),
                1,
                "missing or duplicate {expected_object}: {candidates:#?}"
            );
            assert!(!matching[0]["reason"].as_str().unwrap().is_empty());
            assert!(!matching[0]["evidence"].as_array().unwrap().is_empty());
        }

        let warnings = data["warnings"].as_array().unwrap();
        let warning_text = warnings
            .iter()
            .filter_map(|warning| warning["message"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(warning_text.contains("Товары.Серия"), "{warning_text}");
        assert!(
            warning_text.contains("табличная часть `Серии`"),
            "{warning_text}"
        );
        assert!(
            warning_text.contains("на поддержке/замке") && warning_text.contains("CFE"),
            "{warning_text}"
        );

        let missing = data["missingChecks"].as_array().unwrap();
        assert!(missing
            .iter()
            .any(|check| { check["check"] == "bsl_index" && check["status"] == "stale" }));
        assert!(missing
            .iter()
            .any(|check| check["check"] == "unica.code.definition"));
        assert!(missing
            .iter()
            .any(|check| check["check"] == "unica.code.graph"));
        assert!(!missing.iter().any(|check| {
            check["check"]
                .as_str()
                .is_some_and(|name| name.contains("metadata") || name.contains("form"))
        }));

        let evidence = data["evidence"].as_array().unwrap();
        for item in evidence {
            if let Some(path) = item["path"].as_str() {
                assert!(
                    !Path::new(path).is_absolute(),
                    "absolute evidence path: {path}"
                );
                assert!(
                    path.starts_with("src/"),
                    "out-of-source evidence path: {path}"
                );
                assert!(
                    workspace.join(path).is_file(),
                    "missing evidence path: {path}"
                );
            }
        }
        assert!(evidence.iter().any(|item| {
            item["path"] == "src/Documents/ПриобретениеТоваровУслуг.xml"
        }));
        assert!(evidence.iter().any(|item| {
            item["path"] == "src/DataProcessors/ПодборСерийВДокументы.xml"
        }));
        assert!(evidence.iter().any(|item| {
            item["source"] == "bsl_lexical_scan"
                && item["line"].as_u64().is_some_and(|line| line > 0)
        }));
        assert!(evidence.iter().any(|item| {
            item["path"].as_str().is_some_and(|path| {
                path.ends_with("РегистрацияИПодборСерийПоОднойСтрокеТоваров/Ext/Form.xml")
            })
        }));
    }

    #[test]
    fn project_discover_finds_required_flow_with_task_only() {
        let (_root, workspace) = fixture_workspace("unica-discovery-task-only");
        write_index_status(&workspace, "stale");
        let mut args = discovery_args(&workspace, None);
        args.remove("objects");

        let result = UnicaApplication::new()
            .call_tool("unica.project.discover", &args)
            .unwrap();
        let data = result.data.unwrap();
        let candidates = data["candidateExtensionPoints"].as_array().unwrap();

        for expected in [
            "Document.ПриобретениеТоваровУслуг.TabularSection.Серии",
            "DataProcessor.ПодборСерийВДокументы",
            "DataProcessor.ПодборСерийВДокументы.Form.РегистрацияИПодборСерийПоОднойСтрокеТоваров",
        ] {
            assert!(
                candidates
                    .iter()
                    .any(|candidate| candidate["object"] == expected),
                "missing {expected}: {candidates:#?}"
            );
        }
    }

    #[test]
    fn project_discover_prioritizes_task_targets_before_total_metadata_budget() {
        let (_root, workspace) = fixture_workspace("unica-discovery-priority-budget");
        write_index_status(&workspace, "stale");
        let catalogs = workspace.join("src/Catalogs");
        fs::create_dir_all(&catalogs).unwrap();
        for index in 0..8 {
            let file = fs::File::create(catalogs.join(format!("0000Decoy{index}.xml"))).unwrap();
            file.set_len(MAX_METADATA_FILE_BYTES).unwrap();
        }
        let mut args = discovery_args(&workspace, None);
        args.remove("objects");

        let result = UnicaApplication::new()
            .call_tool("unica.project.discover", &args)
            .unwrap();
        let data = result.data.unwrap();
        let candidates = data["candidateExtensionPoints"].as_array().unwrap();

        for expected in [
            "Document.ПриобретениеТоваровУслуг.TabularSection.Серии",
            "DataProcessor.ПодборСерийВДокументы",
            "DataProcessor.ПодборСерийВДокументы.Form.РегистрацияИПодборСерийПоОднойСтрокеТоваров",
        ] {
            assert!(
                candidates
                    .iter()
                    .any(|candidate| candidate["object"] == expected),
                "priority target was starved by decoys: {expected}: {candidates:#?}"
            );
        }
        assert!(data["missingChecks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| {
                check["check"] == "metadata scan" && check["reason"] == "result_truncated"
            }));
    }

    #[test]
    fn project_discover_uses_typed_code_search_when_index_is_ready() {
        let (_root, workspace) = fixture_workspace("unica-discovery-ready");
        let context = WorkspaceContext::discover(workspace.clone()).unwrap();
        let provider = FakeCodeEvidenceProvider::ready();
        let adapter = ExtensionPointDiscoveryAdapter::with_code_evidence(&provider);
        let args = discovery_args(&workspace, None);

        let outcome = adapter
            .invoke("unica.project.discover", &args, &context, false)
            .unwrap();
        let data: Value = serde_json::from_str(outcome.stdout.as_deref().unwrap()).unwrap();

        assert!(outcome.ok);
        let queries = provider.queries.borrow();
        assert!(!queries.is_empty());
        assert!(queries
            .iter()
            .any(|query| query.contains("ПодборСерийВДокументы")));
        assert!(queries
            .iter()
            .any(|query| query.contains("ПараметрыУказанияСерий")));
        assert!(!provider.definitions.borrow().is_empty());
        assert!(!provider.graphs.borrow().is_empty());
        assert!(data["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| { item["source"] == "rlm_method_index" }));
        assert!(data["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| { item["source"] == "rlm_definition" }));
        assert!(data["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| { item["source"] == "bsl_graph" }));
        assert!(!data["missingChecks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| { item["check"] == "bsl_index" }));
    }

    #[test]
    fn project_discover_does_not_promote_decoy_names_without_metadata_identity() {
        let (_root, workspace) = fixture_workspace("unica-discovery-decoy");
        write_index_status(&workspace, "stale");
        fs::remove_file(workspace.join("src/DataProcessors/ПодборСерийВДокументы.xml")).unwrap();

        let result = call_discovery(&workspace, None);
        let data = result.data.unwrap();
        let candidates = data["candidateExtensionPoints"].as_array().unwrap();

        assert!(result.ok);
        assert!(!candidates.iter().any(|candidate| {
            candidate["object"]
                .as_str()
                .is_some_and(|object| object.starts_with("DataProcessor.ПодборСерийВДокументы"))
        }));
        assert!(!data["evidence"].as_array().unwrap().iter().any(|item| {
            item["path"]
                .as_str()
                .is_some_and(|path| path.starts_with("decoy/"))
        }));
        assert!(data["missingChecks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| {
                item["reason"] == "unverified_identity" && item["check"] == "BSL metadata identity"
            }));
    }

    #[test]
    fn project_discover_does_not_claim_form_confirmation_for_malformed_xml() {
        let (_root, workspace) = fixture_workspace("unica-discovery-malformed-form");
        write_index_status(&workspace, "stale");
        fs::write(
            workspace.join(
                "src/DataProcessors/ПодборСерийВДокументы/Forms/РегистрацияИПодборСерийПоОднойСтрокеТоваров/Ext/Form.xml",
            ),
            "<Form>",
        )
        .unwrap();

        let result = call_discovery(&workspace, None);
        let data = result.data.unwrap();
        let form = data["candidateExtensionPoints"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| {
                candidate["object"]
                    == "DataProcessor.ПодборСерийВДокументы.Form.РегистрацияИПодборСерийПоОднойСтрокеТоваров"
            })
            .unwrap();

        assert!(!form["reasonCodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == "form_confirmation"));
        assert!(data["missingChecks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| check["check"] == "unica.form.validate"));
    }

    #[test]
    fn managed_form_validation_uses_the_bounded_snapshot() {
        let (_root, workspace) = fixture_workspace("unica-discovery-form-snapshot");
        let context = WorkspaceContext::discover(workspace.clone()).unwrap();
        let form_path = workspace.join(
            "src/DataProcessors/ПодборСерийВДокументы/Forms/РегистрацияИПодборСерийПоОднойСтрокеТоваров/Ext/Form.xml",
        );
        let snapshot = fs::read_to_string(&form_path).unwrap();
        fs::write(&form_path, "<Form>").unwrap();

        assert!(valid_managed_form_snapshot(&context, &form_path, &snapshot));
    }

    #[test]
    fn project_discover_rejects_source_dir_escape() {
        let (_root, workspace) = fixture_workspace("unica-discovery-escape");
        let mut args = discovery_args(&workspace, None);
        args.insert("sourceDir".to_string(), json!(".."));

        let error = UnicaApplication::new()
            .call_tool("unica.project.discover", &args)
            .unwrap_err();

        assert!(error.contains("inside the workspace"), "{error}");
    }

    #[test]
    fn project_discover_marks_oversized_metadata_as_partial() {
        let (_root, workspace) = fixture_workspace("unica-discovery-budget");
        write_index_status(&workspace, "stale");
        let catalogs = workspace.join("src/Catalogs");
        fs::create_dir_all(&catalogs).unwrap();
        let oversized = catalogs.join("СерииНоменклатуры.xml");
        let file = fs::File::create(oversized).unwrap();
        file.set_len(MAX_METADATA_FILE_BYTES + 1).unwrap();

        let result = call_discovery(&workspace, None);
        let data = result.data.unwrap();

        assert_eq!(data["status"], "partial");
        assert!(data["missingChecks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| check["reason"] == "result_truncated"));
    }

    #[cfg(unix)]
    #[test]
    fn project_discover_rejects_symlinked_metadata_evidence() {
        use std::os::unix::fs::symlink;

        let (root, workspace) = fixture_workspace("unica-discovery-metadata-symlink");
        write_index_status(&workspace, "stale");
        let outside = root.join("СерииСекрет.xml");
        fs::write(
            &outside,
            "<MetaDataObject><Catalog><Properties><Name>СерииСекрет</Name></Properties></Catalog></MetaDataObject>",
        )
        .unwrap();
        let catalogs = workspace.join("src/Catalogs");
        fs::create_dir_all(&catalogs).unwrap();
        symlink(&outside, catalogs.join("СерииСекрет.xml")).unwrap();

        let result = call_discovery(&workspace, None);
        let data = result.data.unwrap();

        assert!(data["missingChecks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| check["status"] == "rejected"));
        assert!(!data["evidence"].as_array().unwrap().iter().any(|item| {
            item["path"]
                .as_str()
                .is_some_and(|path| path.contains("СерииСекрет"))
        }));
    }

    #[test]
    fn graph_match_requires_non_empty_typed_results() {
        assert!(!code_graph_has_match(
            "=== bsl-analyzer-graph ===\n{\"action\":\"resolve\",\"nodes\":[],\"edges\":[]}"
        ));
        assert!(code_graph_has_match(
            "=== bsl-analyzer-graph ===\n{\"action\":\"resolve\",\"nodes\":[{\"id\":\"method:x\"}]}"
        ));
        assert!(!code_graph_has_match(
            "=== bsl-analyzer-graph ===\nnot-json"
        ));
    }

    #[test]
    fn bounded_reader_rejects_content_above_limit() {
        let root = temp_root("unica-discovery-bounded-read");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("source.xml");
        fs::write(&path, "1234").unwrap();

        assert_eq!(
            read_bounded_utf8(&path, 3, None).err(),
            Some(BoundedReadError::TooLarge)
        );
    }

    #[test]
    fn bounded_reader_rejects_opened_file_outside_containment_root() {
        let root = temp_root("unica-discovery-bounded-containment");
        let contained = root.join("contained");
        fs::create_dir_all(&contained).unwrap();
        let outside = root.join("outside.xml");
        fs::write(&outside, "<root/>").unwrap();

        assert_eq!(
            read_bounded_utf8(&outside, 1024, Some(&contained)).err(),
            Some(BoundedReadError::NotRegular)
        );
    }

    #[cfg(unix)]
    #[test]
    fn bounded_reader_rejects_symlink() {
        use std::os::unix::fs::symlink;

        let root = temp_root("unica-discovery-bounded-symlink");
        fs::create_dir_all(&root).unwrap();
        let target = root.join("target.xml");
        let link = root.join("link.xml");
        fs::write(&target, "<root/>").unwrap();
        symlink(&target, &link).unwrap();

        assert_eq!(
            read_bounded_utf8(&link, 1024, None).err(),
            Some(BoundedReadError::NotRegular)
        );
    }

    #[test]
    fn vendor_warning_aggregation_preserves_architecture_warning_capacity() {
        let (_root, workspace) = fixture_workspace("unica-discovery-warning-capacity");
        let context = WorkspaceContext::discover(workspace).unwrap();
        let provider = FakeCodeEvidenceProvider::ready();
        let mut discovery = Discovery::new(
            &context,
            "Контроль серий",
            Vec::new(),
            Vec::new(),
            DEFAULT_LIMIT,
            &provider,
        );
        for index in 0..(MAX_WARNINGS + 8) {
            discovery.add_warning(
                DiscoveryWarning::new(
                    "vendor_supported_object",
                    "Найдены объекты на поддержке/замке; прямая правка рискованна, проверьте вариант расширения CFE.",
                )
                .with_object(format!("Catalog.Серия{index}")),
            );
        }
        discovery.add_warning(DiscoveryWarning::new(
            "separate_tabular_section",
            "Найдена отдельная табличная часть `Серии`.",
        ));

        assert_eq!(
            discovery
                .warnings
                .iter()
                .filter(|warning| warning.code == "vendor_supported_object")
                .count(),
            1
        );
        assert!(discovery
            .warnings
            .iter()
            .any(|warning| warning.code == "separate_tabular_section"));
        assert!(discovery
            .missing_checks
            .iter()
            .any(|check| check.check == "warning evidence set"));
    }

    #[test]
    fn indexed_hit_keeps_actual_method_and_contained_line_anchor() {
        let (_root, workspace) = fixture_workspace("unica-discovery-index-hit");
        let source_root = workspace.join("src");
        let hit = parse_index_hit(
            "- DataProcessors/ПодборСерийВДокументы/Ext/ManagerModule.bsl:1 Function ПараметрыУказанияСерий() export",
            "Серии",
            &source_root,
        )
        .unwrap();

        assert_eq!(hit.target, "ПараметрыУказанияСерий");
        assert_eq!(hit.line, 1);
        assert!(hit.path.starts_with(fs::canonicalize(source_root).unwrap()));
    }

    #[test]
    fn ready_index_must_match_selected_source_and_stay_in_cache() {
        let (_root, workspace) = fixture_workspace("unica-discovery-index-trust");
        let other_source = workspace.join("other-src");
        fs::create_dir_all(&other_source).unwrap();
        let context = WorkspaceContext::discover(workspace.clone()).unwrap();
        let db_path = context.cache_root.join("rlm-tools-bsl/index.db");
        fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        fs::write(&db_path, "").unwrap();
        write_ready_index_status(&workspace, &other_source, &db_path);

        let wrong_source =
            SYSTEM_CODE_EVIDENCE_PROVIDER.index_state(&context, &workspace.join("src"));
        assert!(matches!(wrong_source, DiscoveryIndexState::Unavailable(_)));

        let outside_db = workspace.parent().unwrap().join("outside-index.db");
        fs::write(&outside_db, "").unwrap();
        write_ready_index_status(&workspace, &workspace.join("src"), &outside_db);
        let outside = SYSTEM_CODE_EVIDENCE_PROVIDER.index_state(&context, &workspace.join("src"));
        assert!(matches!(outside, DiscoveryIndexState::Unavailable(_)));
    }

    #[cfg(unix)]
    #[test]
    fn ready_index_rejects_symlinked_database() {
        use std::os::unix::fs::symlink;

        let (root, workspace) = fixture_workspace("unica-discovery-index-symlink");
        let context = WorkspaceContext::discover(workspace.clone()).unwrap();
        let outside_db = root.join("outside-index.db");
        fs::write(&outside_db, "").unwrap();
        let db_path = context.cache_root.join("rlm-tools-bsl/index.db");
        fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        symlink(&outside_db, &db_path).unwrap();
        write_ready_index_status(&workspace, &workspace.join("src"), &db_path);

        let state = SYSTEM_CODE_EVIDENCE_PROVIDER.index_state(&context, &workspace.join("src"));
        assert!(matches!(state, DiscoveryIndexState::Unavailable(_)));
    }

    struct FakeCodeEvidenceProvider {
        queries: RefCell<Vec<String>>,
        definitions: RefCell<Vec<String>>,
        graphs: RefCell<Vec<String>>,
    }

    impl FakeCodeEvidenceProvider {
        fn ready() -> Self {
            Self {
                queries: RefCell::new(Vec::new()),
                definitions: RefCell::new(Vec::new()),
                graphs: RefCell::new(Vec::new()),
            }
        }
    }

    impl CodeEvidenceProvider for FakeCodeEvidenceProvider {
        fn index_state(
            &self,
            _context: &WorkspaceContext,
            _source_root: &Path,
        ) -> DiscoveryIndexState {
            DiscoveryIndexState::Ready {
                db_path: PathBuf::from("fake-index.sqlite"),
            }
        }

        fn search_rlm(
            &self,
            query: &str,
            _db_path: &Path,
            source_root: &Path,
            _limit: usize,
        ) -> Result<Vec<ProviderHit>, String> {
            self.queries.borrow_mut().push(query.to_string());
            Ok(vec![ProviderHit {
                target: query.to_string(),
                path: source_root
                    .join("DataProcessors/ПодборСерийВДокументы/Ext/ManagerModule.bsl"),
                line: 1,
            }])
        }

        fn find_definition(
            &self,
            name: &str,
            _db_path: &Path,
            source_root: &Path,
            _limit: usize,
        ) -> Result<Vec<ProviderHit>, String> {
            self.definitions.borrow_mut().push(name.to_string());
            Ok(vec![ProviderHit {
                target: name.to_string(),
                path: source_root
                    .join("DataProcessors/ПодборСерийВДокументы/Ext/ManagerModule.bsl"),
                line: 1,
            }])
        }

        fn check_graph(
            &self,
            query: &str,
            _source_dir: &Path,
            _context: &WorkspaceContext,
            _limit: usize,
        ) -> Result<ProviderCheck, String> {
            self.graphs.borrow_mut().push(query.to_string());
            Ok(ProviderCheck {
                matched: true,
                degraded: false,
            })
        }
    }

    fn call_discovery(
        workspace: &Path,
        proposed_extension_points: Option<Vec<&str>>,
    ) -> crate::application::OperationResult {
        UnicaApplication::new()
            .call_tool(
                "unica.project.discover",
                &discovery_args(workspace, proposed_extension_points),
            )
            .unwrap()
    }

    fn discovery_args(
        workspace: &Path,
        proposed_extension_points: Option<Vec<&str>>,
    ) -> Map<String, Value> {
        let mut args = Map::new();
        args.insert("cwd".to_string(), json!(workspace));
        args.insert(
            "task".to_string(),
            json!("При поступлении проверять остаточный срок годности серий по соглашению"),
        );
        args.insert(
            "objects".to_string(),
            json!(["Document.ПриобретениеТоваровУслуг"]),
        );
        if let Some(points) = proposed_extension_points {
            args.insert("proposedExtensionPoints".to_string(), json!(points));
        }
        args
    }

    fn fixture_workspace(prefix: &str) -> (PathBuf, PathBuf) {
        let root = temp_root(prefix);
        let workspace = root.join("workspace");
        copy_tree(&fixture_root(), &workspace);
        (root, workspace)
    }

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/extension-point-discovery/ut115")
    }

    fn copy_tree(source: &Path, target: &Path) {
        fs::create_dir_all(target).unwrap();
        let mut entries = fs::read_dir(source)
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let destination = target.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &destination);
            } else {
                fs::copy(entry.path(), destination).unwrap();
            }
        }
    }

    fn write_index_status(workspace: &Path, status: &str) {
        let cache = workspace.join(".build/unica/caches");
        fs::create_dir_all(&cache).unwrap();
        fs::write(
            cache.join("bsl_index_status.json"),
            serde_json::to_string(&json!({
                "status": status,
                "source_root": workspace.join("src"),
                "db_path": null,
                "message": "fixture index state",
                "updated_at": 0
            }))
            .unwrap(),
        )
        .unwrap();
    }

    fn write_ready_index_status(workspace: &Path, source_root: &Path, db_path: &Path) {
        let cache = workspace.join(".build/unica/caches");
        fs::create_dir_all(&cache).unwrap();
        fs::write(
            cache.join("bsl_index_status.json"),
            serde_json::to_string(&json!({
                "status": "ready",
                "source_root": source_root,
                "db_path": db_path,
                "message": null,
                "updated_at": 0
            }))
            .unwrap(),
        )
        .unwrap();
    }

    fn source_snapshot(root: &Path) -> BTreeMap<String, String> {
        let mut snapshot = BTreeMap::new();
        snapshot_tree(root, root, &mut snapshot);
        snapshot
    }

    fn snapshot_tree(root: &Path, current: &Path, snapshot: &mut BTreeMap<String, String>) {
        let mut entries = fs::read_dir(current)
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            if entry.file_type().unwrap().is_dir() {
                snapshot_tree(root, &entry.path(), snapshot);
            } else {
                let bytes = fs::read(entry.path()).unwrap();
                let digest = Sha256::digest(bytes);
                snapshot.insert(
                    workspace_relative(root, &entry.path()),
                    format!("{digest:x}"),
                );
            }
        }
    }

    fn temp_root(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()))
    }
}
