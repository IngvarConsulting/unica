use serde_json::Map;
use std::path::{Path, PathBuf};
use unica_code_intelligence::model::*;
use unica_code_intelligence::provider_sections::*;
use unica_code_intelligence::source_locations::{SearchLocationProjector, SearchSourceBindings};
use unica_process::runner::{
    system_process_runner, ProcessCommand, ProcessRunner, ProcessStreamOutput,
};
use unica_process::StreamControl;
use unica_work_control::cancellation::{cancelled_error, CancellationToken, CANCELLED_PREFIX};
use unica_work_control::provider_deadline::ProviderDeadline;
const SEARCH_CAPABILITIES: &[ProviderCapability] = &[ProviderCapability::Search];

pub struct GitGrepProvider<'a> {
    bindings: SearchSourceBindings,
    runner: &'a (dyn ProcessRunner + Send + Sync),
}

impl GitGrepProvider<'static> {
    pub fn new(bindings: SearchSourceBindings) -> Self {
        Self {
            runner: system_process_runner(),
            bindings,
        }
    }
}

impl<'a> GitGrepProvider<'a> {
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_runner(
        runner: &'a (dyn ProcessRunner + Send + Sync),
        bindings: SearchSourceBindings,
    ) -> Self {
        Self { runner, bindings }
    }

    fn run_search(
        &self,
        request: &SearchRequest,
        context: &CodeIntelligenceContext,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> ProviderSearchSection {
        let mut checkpoint = || {
            if cancellation.is_cancelled() {
                Err(cancelled_error(
                    "git-grep search stopped before process start",
                ))
            } else if deadline.is_elapsed() {
                Err("git-grep provider deadline exceeded".to_string())
            } else {
                Ok(())
            }
        };
        if let Err(error) = checkpoint() {
            return failed_section(ProviderId::GitGrep, error);
        }
        let all_absent = match git_grep_scope_is_absent(context, &mut checkpoint) {
            Ok(absent) => absent,
            Err(error) => return failed_section(ProviderId::GitGrep, error),
        };
        if let Err(error) = checkpoint() {
            return failed_section(ProviderId::GitGrep, error);
        }
        if all_absent {
            return empty_section(ProviderId::GitGrep, SearchRanking::None);
        }
        let scope = context.search_scope.as_ref();
        let source_pathspecs = git_grep_bsl_pathspecs(scope);
        if source_pathspecs.is_empty() {
            return empty_section(ProviderId::GitGrep, SearchRanking::None);
        }
        let timeout = deadline.remaining();
        let mut args = vec![
            "-c".to_string(),
            "core.quotepath=false".to_string(),
            "grep".to_string(),
            "--no-color".to_string(),
            "--untracked".to_string(),
            "--no-exclude-standard".to_string(),
            "--null".to_string(),
            "-n".to_string(),
            "-F".to_string(),
            "-e".to_string(),
            request.query.clone(),
            "--".to_string(),
        ];
        args.extend(source_pathspecs);
        args.push(generated_corpus_exclusion());
        if let Some(scope) = scope {
            for subtree in &scope.excluded_subtrees {
                args.push(format!(
                    ":(exclude,literal){}",
                    subtree.to_string_lossy().replace('\\', "/")
                ));
            }
        }
        let command = ProcessCommand {
            program: PathBuf::from("git"),
            args,
            cwd: context.source_root.path.clone(),
            env: Vec::new(),
            env_remove: Vec::new(),
            capture_limits: None,
            timeout,
            cancellation: cancellation.clone(),
        };
        let mut hits = Vec::new();
        let mut diagnostics = Vec::new();
        let mut fatal = None;
        let mut locations =
            SearchLocationProjector::with_resolver(context, cancellation, self.bindings.resolver);
        let mut consume =
            |line_number: usize, bytes: &[u8]| match parse_git_grep_record(bytes, &mut locations) {
                Ok(hit) => {
                    hits.push(hit);
                    if hits.len() >= request.limit {
                        StreamControl::Stop
                    } else {
                        StreamControl::Continue
                    }
                }
                Err(error) if error.starts_with(CANCELLED_PREFIX) => {
                    fatal = Some(error);
                    StreamControl::Stop
                }
                Err(error) => {
                    diagnostics.push(format!(
                        "ignored malformed git-grep record #{line_number}: {error}"
                    ));
                    StreamControl::Continue
                }
            };
        let output = match self
            .runner
            .run_streaming(&command, 1024 * 1024, &mut consume)
        {
            Ok(output) => output,
            Err(error) => {
                return unavailable_section(
                    ProviderId::GitGrep,
                    format!("failed to start git grep: {error}"),
                );
            }
        };
        if let Some((line, error)) = &output.line_error {
            fatal = Some(format!("git-grep record #{line}: {error}"));
        }
        git_grep_stream_section(output, hits, diagnostics, fatal)
    }
}

/// Git tracking does not define the admitted BSL corpus. Restrict each
/// positive pathspec itself: separate BSL and scope patterns would form a
/// union and could expose ignored files outside the selected module.
pub fn git_grep_bsl_pathspecs(
    scope: Option<&unica_code_intelligence::model::CodeSearchScope>,
) -> Vec<String> {
    use unica_code_intelligence::model::RelativeSearchFilter;

    const BSL_GLOB: &str = "**/*.[bB][sS][lL]";
    let Some(scope) = scope.filter(|scope| !scope.filters.is_empty()) else {
        return vec![format!(":(glob){BSL_GLOB}")];
    };
    scope
        .filters
        .iter()
        .filter_map(|filter| match filter {
            RelativeSearchFilter::Exact(path) => path
                .extension()
                .and_then(std::ffi::OsStr::to_str)
                .filter(|extension| extension.eq_ignore_ascii_case("bsl"))
                .map(|_| format!(":(literal){}", path.to_string_lossy().replace('\\', "/"))),
            RelativeSearchFilter::Subtree(path) => {
                let prefix = path.to_string_lossy().replace('\\', "/");
                Some(format!(
                    ":(glob){}/{BSL_GLOB}",
                    git_glob_literal_prefix(&prefix)
                ))
            }
        })
        .collect()
}

/// Quotes a literal path for a `:(glob)` pathspec with one-character classes.
/// A backslash escape is not portable: Git for Windows reads `\` in a
/// pathspec as a directory separator, so `Ignored\[1\]` names the absent
/// directory `Ignored/` and the selected module reads as empty.
pub fn git_glob_literal_prefix(prefix: &str) -> String {
    let mut quoted = String::with_capacity(prefix.len());
    for character in prefix.chars() {
        if matches!(character, '*' | '?' | '[' | ']') {
            quoted.push('[');
            quoted.push(character);
            quoted.push(']');
        } else {
            quoted.push(character);
        }
    }
    quoted
}

/// Git warns about an absent pathspec parent even when it finds no matches.
/// Prove absence through retained directories instead of interpreting that warning.
pub fn git_grep_scope_is_absent(
    context: &CodeIntelligenceContext,
    checkpoint: &mut impl FnMut() -> Result<(), String>,
) -> Result<bool, String> {
    use std::path::Component;
    use unica_code_intelligence::model::RelativeSearchFilter;
    use unica_workspace::read::{
        open_absolute_read_directory_nofollow, open_any_read_child_nofollow,
        open_read_child_for_secure_tree_use, OpenedChildKind,
    };

    let Some(scope) = context
        .search_scope
        .as_ref()
        .filter(|scope| !scope.filters.is_empty())
    else {
        return Ok(false);
    };
    if scope.source_root != context.source_root.path {
        return Err("git-grep scope root does not match the selected source root".to_string());
    }
    for filter in &scope.filters {
        let (RelativeSearchFilter::Exact(path) | RelativeSearchFilter::Subtree(path)) = filter;
        let git_path = path.to_string_lossy().replace('\\', "/");
        if path.as_os_str().is_empty()
            || Path::new(&git_path) != path
            || unica_workspace::paths::is_foreign_absolute_path(&git_path)
            || path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err("git-grep scope filter is not a relative source path".to_string());
        }
    }
    let io_error =
        |error: std::io::Error| format!("git-grep scope could not be inspected safely: {error}");
    let root =
        open_absolute_read_directory_nofollow(&context.source_root.path).map_err(io_error)?;
    let mut all_absent = true;
    for filter in &scope.filters {
        let (RelativeSearchFilter::Exact(path) | RelativeSearchFilter::Subtree(path)) = filter;
        let mut parent = root.try_clone().map_err(io_error)?;
        let mut parts = path.components().peekable();
        while let Some(Component::Normal(name)) = parts.next() {
            checkpoint()?;
            let (child, kind) = match open_any_read_child_nofollow(&parent, name) {
                Ok(child) => child,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(error) => return Err(io_error(error)),
            };
            if parts.peek().is_some() {
                if kind != OpenedChildKind::Directory {
                    return Err("git-grep scope parent is not a regular directory".to_string());
                }
                // Windows classification handles need directory rights; this shared
                // primitive also verifies the reopened handle against its anchor.
                parent = open_read_child_for_secure_tree_use(&parent, name, child, kind)
                    .map_err(io_error)?;
            } else {
                let expected = match filter {
                    RelativeSearchFilter::Exact(_) => OpenedChildKind::RegularFile,
                    RelativeSearchFilter::Subtree(_) => OpenedChildKind::Directory,
                };
                if kind != expected {
                    return Err("git-grep scope target has an unexpected file type".to_string());
                }
                all_absent = false;
            }
        }
    }
    if all_absent {
        checkpoint()?;
        let current_root =
            open_absolute_read_directory_nofollow(&context.source_root.path).map_err(io_error)?;
        if root.identity().map_err(io_error)? != current_root.identity().map_err(io_error)? {
            return Err("git-grep source root changed during scope inspection".to_string());
        }
    }
    Ok(all_absent)
}

/// The generated cache lives inside the source root but is not source: the
/// source walks skip it by name, and the analyzer databases it holds run to
/// gigabytes on a vendor-class configuration. `git grep` emits in path order
/// and `.build` sorts ahead of every metadata directory, so a corpus that
/// keeps it makes the provider read Unica's own index of the tree it is
/// searching before it can emit the first source record — long enough on a
/// 48k-file configuration to spend the whole budget and return nothing. The
/// `glob` magic makes `**/` match at any depth, including the top level, which
/// is the same rule the walks apply to a directory of that name anywhere.
pub fn generated_corpus_exclusion() -> String {
    format!(
        ":(exclude,glob)**/{}/**",
        unica_workspace::paths::GENERATED_DIR_NAME
    )
}

impl CodeIntelligenceProvider for GitGrepProvider<'_> {
    fn identity(&self) -> unica_code_intelligence::model::ProviderIdentity {
        ProviderId::GitGrep.identity()
    }

    fn capabilities(&self) -> &[ProviderCapability] {
        SEARCH_CAPABILITIES
    }

    fn search(
        &self,
        request: &SearchRequest,
        context: &CodeIntelligenceContext,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> ProviderSearchSection {
        self.run_search(request, context, deadline, cancellation)
    }
}

pub fn git_grep_stream_section(
    output: ProcessStreamOutput,
    hits: Vec<ProviderSearchHit>,
    mut diagnostics: Vec<String>,
    fatal: Option<String>,
) -> ProviderSearchSection {
    if output.cancelled {
        return failed_section(
            ProviderId::GitGrep,
            cancelled_error("git-grep search process stopped"),
        );
    }
    if let Some(fatal) = fatal {
        return failed_section(ProviderId::GitGrep, fatal);
    }
    if output.stopped_by_consumer {
        if !diagnostics.is_empty() {
            diagnostics.push("git-grep result limit was also reached".to_string());
            return ProviderSearchSection::partial(
                ProviderId::GitGrep.identity(),
                SearchRanking::None,
                SearchOrdering::ProviderTraversal,
                hits,
                diagnostics,
            )
            .expect("partial git-grep section is valid");
        }
        return ProviderSearchSection::limit_reached(
            ProviderId::GitGrep.identity(),
            SearchRanking::None,
            SearchOrdering::ProviderTraversal,
            hits,
            diagnostics,
        )
        .expect("bounded git-grep section is valid");
    }
    if output.timed_out {
        diagnostics.push("git-grep provider deadline exceeded".to_string());
        return ProviderSearchSection::timed_out(
            ProviderId::GitGrep.identity(),
            SearchRanking::None,
            SearchOrdering::ProviderTraversal,
            hits,
            diagnostics,
        )
        .expect("timed-out git-grep section is valid");
    }
    let no_matches = !output.status_success
        && hits.is_empty()
        && output.stderr.trim().is_empty()
        && process_exit_code_is(&output.status, 1);
    if no_matches {
        return empty_section(ProviderId::GitGrep, SearchRanking::None);
    }
    if !output.status_success
        && output.stderr.contains("not a git repository")
        && process_exit_code_is(&output.status, 128)
    {
        return unavailable_section(ProviderId::GitGrep, output.stderr.trim().to_string());
    }
    if !output.status_success {
        let detail = if output.stderr.trim().is_empty() {
            format!("git grep exited with status {}", output.status)
        } else {
            output.stderr.trim().to_string()
        };
        return failed_section(ProviderId::GitGrep, detail);
    }

    let status = if hits.is_empty() {
        if diagnostics.is_empty() {
            ProviderSectionStatus::Empty
        } else {
            diagnostics.insert(
                0,
                "git-grep returned non-empty output without any valid results".to_string(),
            );
            ProviderSectionStatus::Failed
        }
    } else if !diagnostics.is_empty() {
        ProviderSectionStatus::Partial
    } else {
        ProviderSectionStatus::Ok
    };
    match status {
        ProviderSectionStatus::Ok | ProviderSectionStatus::Empty => {
            ProviderSearchSection::complete(
                ProviderId::GitGrep.identity(),
                SearchRanking::None,
                SearchOrdering::ProviderTraversal,
                hits,
                diagnostics,
            )
            .unwrap_or_else(|error| {
                ProviderSearchSection::failed(ProviderId::GitGrep.identity(), error)
            })
        }
        ProviderSectionStatus::Partial => ProviderSearchSection::partial(
            ProviderId::GitGrep.identity(),
            SearchRanking::None,
            SearchOrdering::ProviderTraversal,
            hits,
            diagnostics,
        )
        .expect("partial git-grep section is valid"),
        _ => ProviderSearchSection::failed(ProviderId::GitGrep.identity(), diagnostics.join("; ")),
    }
}

pub fn parse_git_grep_record(
    record: &[u8],
    locations: &mut SearchLocationProjector<'_>,
) -> Result<ProviderSearchHit, String> {
    let mut parts = record.splitn(3, |byte| *byte == 0);
    let path = std::str::from_utf8(parts.next().ok_or("path is missing")?)
        .map_err(|_| "path is not UTF-8")?
        .trim();
    let line_number = std::str::from_utf8(parts.next().ok_or("line is missing")?)
        .map_err(|_| "line is not UTF-8")?
        .parse::<usize>()
        .map_err(|_| "line is not a positive integer")?;
    let snippet = std::str::from_utf8(parts.next().ok_or("snippet is missing")?)
        .map_err(|_| "snippet is not UTF-8")?
        .trim_end();
    if path.is_empty() {
        return Err("path is empty".to_string());
    }
    let relative = PathBuf::from(path.replace('\\', "/"));
    Ok(ProviderSearchHit {
        rank: None,
        provider_score: None,
        location: locations.project(&relative)?,
        line: line_number,
        end_line: None,
        symbol: None,
        kind: None,
        snippet: snippet.to_string(),
        attributes: Map::new(),
    })
}

pub fn process_exit_code_is(status: &str, code: i32) -> bool {
    let status = status.trim();
    status == code.to_string() || status.ends_with(&format!(": {code}"))
}
