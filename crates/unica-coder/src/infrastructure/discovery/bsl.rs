use crate::application::discovery::ports::{BslSearchPort, DefinitionPort, RuntimeFlowPort};
use crate::domain::discovery::{
    normalize_discovery_identity, AnalyzedFile, ArtifactId, ArtifactKind, BslFact, DefinitionFact,
    DiscoveryQuery, EvidenceLocation, FactBatch, PortableRelativePath, ProviderDiagnostic,
    ProviderOutcome, RuntimeFlowFact, SourceFile, SourceInventory,
};
use crate::infrastructure::discovery::metadata::{
    build_batch, contributors_for_records, inventory_is_bounded, validate_platform_identifier,
};
use crate::infrastructure::metadata_kinds::metadata_kind_by_directory;
use crate::infrastructure::platform::contained_file::{
    read_contained_regular_file_cancellable, ContainedFileError, VerifiedFile,
};
use crate::infrastructure::workspace_index::{
    find_indexed_definitions, BslIndexStatus, IndexQueryError, IndexedMethodHit, IndexedMethodKind,
};
use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

pub(crate) struct InventoryBslSearchProvider;

impl BslSearchPort for InventoryBslSearchProvider {
    fn search(
        &self,
        query: &DiscoveryQuery<'_>,
        files: &SourceInventory,
    ) -> ProviderOutcome<FactBatch<BslFact>> {
        if let Some(outcome) = crate::infrastructure::discovery::cancellation_outcome(query) {
            return outcome;
        }
        match collect_lexical_facts(query, files) {
            Ok(BslCollection::Complete(batch)) => ProviderOutcome::Complete(batch),
            Ok(BslCollection::Bounded { batch, diagnostic }) => ProviderOutcome::Bounded {
                data: batch,
                diagnostic,
            },
            Err(diagnostic)
                if crate::infrastructure::discovery::is_cancellation_diagnostic(&diagnostic) =>
            {
                ProviderOutcome::Failed(diagnostic)
            }
            Err(diagnostic) => ProviderOutcome::ContractViolation(diagnostic),
        }
    }
}

enum BslCollection<T> {
    Complete(FactBatch<T>),
    Bounded {
        batch: FactBatch<T>,
        diagnostic: ProviderDiagnostic,
    },
}

fn collect_lexical_facts(
    query: &DiscoveryQuery<'_>,
    inventory: &SourceInventory,
) -> Result<BslCollection<BslFact>, ProviderDiagnostic> {
    crate::infrastructure::discovery::check_cancellation(query)?;
    let terms = query_terms(query)?;
    let mut analyzed_files = Vec::new();
    let mut records = BTreeSet::new();
    let max_evidence = usize::from(query.limits().max_evidence);
    for file in inventory
        .files
        .iter()
        .filter(|file| is_bsl_path(&file.relative_path))
    {
        crate::infrastructure::discovery::check_cancellation(query)?;
        let facts = lexical_facts_for_file(file, &terms, query, max_evidence);
        crate::infrastructure::discovery::check_cancellation(query)?;
        match facts {
            Ok(facts) => {
                analyzed_files.push(file.analyzed_file());
                for fact in facts {
                    insert_bounded_fact(&mut records, fact, max_evidence);
                }
            }
            Err(BslParseError::Cancelled) => {
                return Err(crate::infrastructure::discovery::cancellation_diagnostic())
            }
            Err(BslParseError::Bounded(BslParseBound::LineBytes { .. })) => {
                let batch = build_lexical_batch(records, analyzed_files, max_evidence)?;
                return Ok(BslCollection::Bounded {
                    batch,
                    diagnostic: ProviderDiagnostic::material(
                        "bsl_source_line_bound",
                        format!(
                            "BSL source {} exceeded the per-line byte limit",
                            file.relative_path.as_str()
                        ),
                    ),
                });
            }
            Err(BslParseError::Bounded(BslParseBound::Signature { .. })) => {
                let batch = build_lexical_batch(records, analyzed_files, max_evidence)?;
                return Ok(BslCollection::Bounded {
                    batch,
                    diagnostic: ProviderDiagnostic::material(
                        "bsl_source_signature_bound",
                        format!(
                            "BSL source {} exceeded the method-signature parse limit",
                            file.relative_path.as_str()
                        ),
                    ),
                });
            }
            Err(BslParseError::Malformed(message)) => {
                return Err(ProviderDiagnostic::material(
                    "bsl_malformed",
                    format!(
                        "BSL source {} is malformed: {message}",
                        file.relative_path.as_str()
                    ),
                ))
            }
        }
    }
    let evidence_bounded = records.len() > max_evidence;
    let batch = build_lexical_batch(records, analyzed_files, max_evidence)?;
    if evidence_bounded {
        Ok(BslCollection::Bounded {
            batch,
            diagnostic: ProviderDiagnostic::material(
                "bsl_evidence_bound",
                "BSL lexical facts stopped at the maxEvidence limit",
            ),
        })
    } else if inventory_is_bounded(inventory) {
        Ok(BslCollection::Bounded {
            batch,
            diagnostic: ProviderDiagnostic::material(
                "bsl_inventory_bounded",
                "BSL lexical scope is incomplete because source inventory was truncated",
            ),
        })
    } else {
        Ok(BslCollection::Complete(batch))
    }
}

fn insert_bounded_fact(records: &mut BTreeSet<BslFact>, fact: BslFact, max_evidence: usize) {
    records.insert(fact);
    let retained = max_evidence.saturating_add(1);
    if records.len() > retained {
        let _discarded = records.pop_last();
    }
}

fn build_lexical_batch(
    records: BTreeSet<BslFact>,
    analyzed_files: Vec<AnalyzedFile>,
    max_evidence: usize,
) -> Result<FactBatch<BslFact>, ProviderDiagnostic> {
    let records = records.into_iter().take(max_evidence).collect::<Vec<_>>();
    let contributors = contributors_for_records(&records, &analyzed_files);
    build_batch(records, analyzed_files, contributors)
}

fn query_terms(query: &DiscoveryQuery<'_>) -> Result<Vec<String>, ProviderDiagnostic> {
    let mut terms = BTreeSet::new();
    for term in std::iter::once(query.task())
        .chain(query.search_terms().iter().map(String::as_str))
        .chain(
            query
                .concepts()
                .iter()
                .map(|concept| concept.value.as_str()),
        )
    {
        crate::infrastructure::discovery::check_cancellation(query)?;
        let normalized = normalize_discovery_identity(term);
        if !normalized.is_empty() {
            terms.insert(normalized);
        }
    }
    Ok(terms.into_iter().collect())
}

fn raw_query_terms(query: &DiscoveryQuery<'_>) -> Result<Vec<String>, ProviderDiagnostic> {
    query_terms(query)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BslMethodKind {
    Procedure,
    Function,
}

const MAX_BSL_SIGNATURE_LINES: usize = 64;
const MAX_BSL_SIGNATURE_BYTES: usize = 64 * 1024;
const MAX_BSL_LINE_BYTES: usize = 64 * 1024;

#[derive(Debug)]
struct ParsedBslMethod {
    kind: BslMethodKind,
    name: String,
    exported: bool,
    declaration_line: u32,
    end_line: u32,
}

#[derive(Debug)]
struct ParsedBslSource {
    methods: Vec<ParsedBslMethod>,
    method_ranges: Vec<ParsedBslMethodRange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ParsedBslMethodRange {
    start_line: u32,
    end_line: u32,
    method_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BslParseBound {
    LineBytes { limit: usize },
    Signature { max_lines: usize, max_bytes: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BslParseError {
    Cancelled,
    Bounded(BslParseBound),
    Malformed(String),
}

impl std::fmt::Display for BslParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("discovery cancelled"),
            Self::Bounded(BslParseBound::LineBytes { limit }) => {
                write!(
                    formatter,
                    "BSL line exceeds the supported bound of {limit} bytes"
                )
            }
            Self::Bounded(BslParseBound::Signature {
                max_lines,
                max_bytes,
            }) => write!(
                formatter,
                "method signature exceeds {max_lines} lines or {max_bytes} bytes"
            ),
            Self::Malformed(message) => formatter.write_str(message),
        }
    }
}

impl From<String> for BslParseError {
    fn from(message: String) -> Self {
        if message == "discovery cancelled" {
            Self::Cancelled
        } else {
            Self::Malformed(message)
        }
    }
}

enum BslParseState {
    OutsideMethod,
    Signature(SignatureState),
    Body(usize),
}

struct SignatureState {
    method_index: usize,
    depth: usize,
    lines: usize,
    bytes: usize,
    closed: bool,
}

struct DeclarationStart {
    kind: BslMethodKind,
    name: String,
    open_parenthesis: usize,
}

struct ScannedBslLine {
    code: String,
    structural: String,
}

fn lexical_facts_for_file(
    file: &SourceFile,
    terms: &[String],
    query: &DiscoveryQuery<'_>,
    max_evidence: usize,
) -> Result<BTreeSet<BslFact>, BslParseError> {
    let text = std::str::from_utf8(&file.bytes)
        .map_err(|error| BslParseError::Malformed(format!("input is not UTF-8: {error}")))?;
    let text = strip_source_bom(text);
    let module = module_artifact_for_path(&file.relative_path)?;
    let parsed = parse_bsl_source_cancellable(text, query)?;
    let mut method_artifacts = Vec::new();
    for method in &parsed.methods {
        if query.is_cancelled() {
            return Err(BslParseError::Cancelled);
        }
        method_artifacts.push(method_artifact(&module, &method.name)?);
    }
    let mut facts = BTreeSet::new();
    let mut offset = 0_usize;
    let mut zero_based_line = 0_usize;
    while let Some(line) = next_bsl_line(text, &mut offset, &mut || query.is_cancelled())? {
        if query.is_cancelled() {
            return Err(BslParseError::Cancelled);
        }
        let line_number = zero_based_line
            .checked_add(1)
            .and_then(|line| u32::try_from(line).ok())
            .ok_or_else(|| "BSL line number exceeds u32".to_string())?;
        let normalized_line = normalize_bsl_line_cancellable(line, &mut || query.is_cancelled())?;
        let artifact = match method_index_for_line(&parsed.method_ranges, line_number) {
            Some(method_index) => {
                let method = method_artifacts
                    .get(method_index)
                    .ok_or_else(|| "BSL method line ownership is invalid".to_string())?;
                (method.clone(), ArtifactKind::Method)
            }
            None => (module.clone(), ArtifactKind::Module),
        };
        for term in terms {
            if query.is_cancelled() {
                return Err(BslParseError::Cancelled);
            }
            if !contains_cancellable(&normalized_line, term, &mut || query.is_cancelled())? {
                continue;
            }
            insert_bounded_fact(
                &mut facts,
                BslFact {
                    artifact: artifact.0.clone(),
                    artifact_kind: artifact.1,
                    matched_text: term.clone(),
                    location: EvidenceLocation {
                        relative_path: file.relative_path.clone(),
                        line: Some(line_number),
                        column: matching_column(line, term, &mut || query.is_cancelled())?,
                        xml_path: None,
                    },
                },
                max_evidence,
            );
        }
        zero_based_line = zero_based_line
            .checked_add(1)
            .ok_or_else(|| "BSL line index overflowed".to_string())?;
    }
    Ok(facts)
}

fn normalize_bsl_line_cancellable(
    line: &str,
    is_cancelled: &mut dyn FnMut() -> bool,
) -> Result<String, String> {
    let mut normalized = String::new();
    for character in line.chars() {
        if is_cancelled() {
            return Err("discovery cancelled".to_string());
        }
        normalized.extend(character.to_lowercase());
    }
    Ok(normalized.trim().to_string())
}

fn contains_cancellable(
    value: &str,
    needle: &str,
    is_cancelled: &mut dyn FnMut() -> bool,
) -> Result<bool, String> {
    Ok(cancellable_match_offset(value, needle, is_cancelled)?.is_some())
}

fn matching_column(
    line: &str,
    normalized_term: &str,
    is_cancelled: &mut dyn FnMut() -> bool,
) -> Result<Option<u32>, String> {
    let mut normalized_line = String::new();
    let mut original_columns = Vec::new();
    for (zero_based_column, character) in line.chars().enumerate() {
        if is_cancelled() {
            return Err("discovery cancelled".to_string());
        }
        let column = zero_based_column
            .checked_add(1)
            .and_then(|column| u32::try_from(column).ok())
            .ok_or_else(|| "BSL column exceeds u32".to_string())?;
        for lowercase in character.to_lowercase() {
            original_columns.push((normalized_line.len(), column));
            normalized_line.push(lowercase);
        }
    }
    let Some(byte_offset) =
        cancellable_match_offset(&normalized_line, normalized_term, is_cancelled)?
    else {
        return Ok(None);
    };
    Ok(original_columns
        .binary_search_by_key(&byte_offset, |(offset, _column)| *offset)
        .ok()
        .and_then(|index| original_columns.get(index).map(|(_offset, column)| *column)))
}

fn cancellable_match_offset(
    value: &str,
    needle: &str,
    is_cancelled: &mut dyn FnMut() -> bool,
) -> Result<Option<usize>, String> {
    const SEARCH_CHUNK_BYTES: usize = 64 * 1_024;
    if needle.is_empty() {
        return Ok(Some(0));
    }
    let mut start = 0_usize;
    while start < value.len() {
        if is_cancelled() {
            return Err("discovery cancelled".to_string());
        }
        let mut primary_end = start.saturating_add(SEARCH_CHUNK_BYTES).min(value.len());
        while primary_end > start && !value.is_char_boundary(primary_end) {
            primary_end -= 1;
        }
        let mut search_end = primary_end
            .saturating_add(needle.len().saturating_sub(1))
            .min(value.len());
        while search_end < value.len() && !value.is_char_boundary(search_end) {
            search_end += 1;
        }
        if let Some(relative) = value[start..search_end].find(needle) {
            return Ok(Some(start + relative));
        }
        start = primary_end;
    }
    Ok(None)
}

fn strip_source_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}')
        .map_or(text, std::convert::identity)
}

fn parse_bsl_source_cancellable(
    text: &str,
    query: &DiscoveryQuery<'_>,
) -> Result<ParsedBslSource, BslParseError> {
    parse_bsl_source_observing(text, || query.is_cancelled())
}

fn parse_bsl_source_observing(
    text: &str,
    mut is_cancelled: impl FnMut() -> bool,
) -> Result<ParsedBslSource, BslParseError> {
    let text = strip_source_bom(text);
    let mut methods = Vec::new();
    let mut state = BslParseState::OutsideMethod;
    let mut in_string = false;
    let mut offset = 0_usize;
    let mut zero_based_line = 0_usize;

    while let Some(line) = next_bsl_line(text, &mut offset, &mut is_cancelled)? {
        if is_cancelled() {
            return Err(BslParseError::Cancelled);
        }
        let line_number = zero_based_line
            .checked_add(1)
            .and_then(|line| u32::try_from(line).ok())
            .ok_or_else(|| "BSL line number exceeds u32".to_string())?;
        let scanned = scan_bsl_line(line, &mut in_string, &mut is_cancelled)?;
        if is_cancelled() {
            return Err(BslParseError::Cancelled);
        }
        state = match state {
            BslParseState::OutsideMethod => {
                if parse_method_end(&scanned.code, &mut is_cancelled)?.is_some() {
                    return Err(BslParseError::Malformed(format!(
                        "method terminator without declaration at line {line_number}"
                    )));
                }
                if is_cancelled() {
                    return Err(BslParseError::Cancelled);
                }
                match parse_method_declaration(&scanned.code, &mut is_cancelled)? {
                    Some(declaration) => {
                        let method_index = methods.len();
                        methods.push(ParsedBslMethod {
                            kind: declaration.kind,
                            name: declaration.name,
                            exported: false,
                            declaration_line: line_number,
                            end_line: 0,
                        });
                        let mut signature = SignatureState {
                            method_index,
                            depth: 0,
                            lines: 0,
                            bytes: 0,
                            closed: false,
                        };
                        extend_signature_bounds(&mut signature, line)?;
                        let exported = consume_signature_line(
                            &scanned,
                            declaration.open_parenthesis,
                            &mut signature.depth,
                            &mut is_cancelled,
                        )?;
                        match exported {
                            Some(exported) => {
                                signature.closed = true;
                                if exported {
                                    set_method_exported(&mut methods, method_index)?;
                                    BslParseState::Body(method_index)
                                } else {
                                    BslParseState::Signature(signature)
                                }
                            }
                            None => BslParseState::Signature(signature),
                        }
                    }
                    None => BslParseState::OutsideMethod,
                }
            }
            BslParseState::Signature(mut signature) => {
                if signature.closed {
                    let trimmed = scanned.code.trim();
                    if trimmed.is_empty() {
                        extend_signature_bounds(&mut signature, line)?;
                        BslParseState::Signature(signature)
                    } else if signature_line_is_export(trimmed, &mut is_cancelled)? {
                        extend_signature_bounds(&mut signature, line)?;
                        set_method_exported(&mut methods, signature.method_index)?;
                        BslParseState::Body(signature.method_index)
                    } else {
                        process_body_line(
                            signature.method_index,
                            &scanned.code,
                            line_number,
                            &mut methods,
                            &mut is_cancelled,
                        )?
                    }
                } else {
                    extend_signature_bounds(&mut signature, line)?;
                    match consume_signature_line(
                        &scanned,
                        0,
                        &mut signature.depth,
                        &mut is_cancelled,
                    )? {
                        Some(exported) => {
                            signature.closed = true;
                            if exported {
                                set_method_exported(&mut methods, signature.method_index)?;
                                BslParseState::Body(signature.method_index)
                            } else {
                                BslParseState::Signature(signature)
                            }
                        }
                        None => BslParseState::Signature(signature),
                    }
                }
            }
            BslParseState::Body(method_index) => process_body_line(
                method_index,
                &scanned.code,
                line_number,
                &mut methods,
                &mut is_cancelled,
            )?,
        };
        zero_based_line = zero_based_line
            .checked_add(1)
            .ok_or_else(|| "BSL line index overflowed".to_string())?;
    }

    if in_string {
        return Err(BslParseError::Malformed(
            "BSL source has an unterminated string literal".to_string(),
        ));
    }
    match state {
        BslParseState::OutsideMethod => Ok(parsed_bsl_source(methods)),
        BslParseState::Signature(signature) if !signature.closed => Err(BslParseError::Malformed(
            "method signature has no balanced closing parenthesis".to_string(),
        )),
        BslParseState::Signature(_) | BslParseState::Body(_) => Err(BslParseError::Malformed(
            "method declaration has no terminator".to_string(),
        )),
    }
}

fn next_bsl_line<'a>(
    text: &'a str,
    offset: &mut usize,
    is_cancelled: &mut dyn FnMut() -> bool,
) -> Result<Option<&'a str>, BslParseError> {
    if *offset >= text.len() {
        return Ok(None);
    }
    let start = *offset;
    for index in start..text.len() {
        if index % 1_024 == 0 && is_cancelled() {
            return Err(BslParseError::Cancelled);
        }
        if text.as_bytes()[index] == b'\n' {
            *offset = index + 1;
            let end = index
                .checked_sub(usize::from(
                    index > start && text.as_bytes()[index - 1] == b'\r',
                ))
                .ok_or_else(|| "BSL line boundary underflowed".to_string())?;
            let line = &text[start..end];
            validate_bsl_line_bound(line)?;
            return Ok(Some(line));
        }
        let bytes_from_start = index
            .checked_sub(start)
            .ok_or_else(|| BslParseError::Malformed("BSL line boundary underflowed".to_string()))?;
        if bytes_from_start >= MAX_BSL_LINE_BYTES
            && !(text.as_bytes()[index] == b'\r' && text.as_bytes().get(index + 1) == Some(&b'\n'))
        {
            return Err(BslParseError::Bounded(BslParseBound::LineBytes {
                limit: MAX_BSL_LINE_BYTES,
            }));
        }
    }
    *offset = text.len();
    let line = &text[start..];
    validate_bsl_line_bound(line)?;
    Ok(Some(line))
}

fn validate_bsl_line_bound(line: &str) -> Result<(), BslParseError> {
    if line.len() > MAX_BSL_LINE_BYTES {
        return Err(BslParseError::Bounded(BslParseBound::LineBytes {
            limit: MAX_BSL_LINE_BYTES,
        }));
    }
    Ok(())
}

fn parsed_bsl_source(methods: Vec<ParsedBslMethod>) -> ParsedBslSource {
    let method_ranges = methods
        .iter()
        .enumerate()
        .map(|(method_index, method)| ParsedBslMethodRange {
            start_line: method.declaration_line,
            end_line: method.end_line,
            method_index,
        })
        .collect();
    ParsedBslSource {
        methods,
        method_ranges,
    }
}

fn method_index_for_line(ranges: &[ParsedBslMethodRange], line: u32) -> Option<usize> {
    let candidate = ranges
        .partition_point(|range| range.start_line <= line)
        .checked_sub(1)?;
    let range = ranges.get(candidate)?;
    (line <= range.end_line).then_some(range.method_index)
}

fn scan_bsl_line(
    line: &str,
    in_string: &mut bool,
    is_cancelled: &mut dyn FnMut() -> bool,
) -> Result<ScannedBslLine, String> {
    let mut code = String::new();
    let mut structural = String::new();
    let mut characters = line.chars().peekable();
    while let Some(character) = characters.next() {
        if is_cancelled() {
            return Err("discovery cancelled".to_string());
        }
        if character == '"' {
            code.push(character);
            structural.push(' ');
            if *in_string && characters.peek() == Some(&'"') {
                code.push('"');
                structural.push(' ');
                let _next = characters.next();
            } else {
                *in_string = !*in_string;
            }
            continue;
        }
        if !*in_string && character == '/' && characters.peek() == Some(&'/') {
            break;
        }
        code.push(character);
        if *in_string && matches!(character, '(' | ')') {
            structural.push(' ');
        } else {
            structural.push(character);
        }
    }
    Ok(ScannedBslLine { code, structural })
}

fn parse_method_declaration(
    code: &str,
    is_cancelled: &mut dyn FnMut() -> bool,
) -> Result<Option<DeclarationStart>, String> {
    let trimmed = code.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let mut tokens = trimmed.split_whitespace();
    let Some(mut keyword) = tokens.next() else {
        return Ok(None);
    };
    let first_normalized = normalize_bsl_line_cancellable(keyword, is_cancelled)?;
    if matches!(first_normalized.as_str(), "async" | "асинх") {
        let Some(next) = tokens.next() else {
            return Err("async modifier has no method declaration".to_string());
        };
        keyword = next;
    }
    let method_kind = match normalize_bsl_line_cancellable(keyword, is_cancelled)?.as_str() {
        "procedure" | "процедура" => BslMethodKind::Procedure,
        "function" | "функция" => BslMethodKind::Function,
        _ => return Ok(None),
    };
    let keyword_offset = trimmed
        .find(keyword)
        .ok_or_else(|| "method keyword offset is invalid".to_string())?;
    let rest_offset = keyword_offset
        .checked_add(keyword.len())
        .ok_or_else(|| "method keyword offset overflowed".to_string())?;
    let rest_with_leading = trimmed
        .get(rest_offset..)
        .ok_or_else(|| "method declaration boundary is invalid".to_string())?;
    let rest = rest_with_leading.trim_start();
    let leading_name_whitespace = rest_with_leading
        .len()
        .checked_sub(rest.len())
        .ok_or_else(|| "method name whitespace is invalid".to_string())?;
    let name_end = match rest.find(|character: char| character == '(' || character.is_whitespace())
    {
        Some(name_end) => name_end,
        None => rest.len(),
    };
    let name = rest
        .get(..name_end)
        .ok_or_else(|| "method name boundary is invalid".to_string())?;
    if name.is_empty() {
        return Err("method name must not be empty".to_string());
    }
    validate_platform_identifier(name)
        .map_err(|message| format!("method name {name:?} is invalid: {message}"))?;
    let after_name = rest
        .get(name_end..)
        .ok_or_else(|| "method signature boundary is invalid".to_string())?
        .trim_start();
    if !after_name.starts_with('(') {
        return Err(format!("method {name:?} has no parameter list"));
    }
    let after_name_offset = rest_offset
        .checked_add(name_end)
        .ok_or_else(|| "method signature offset overflowed".to_string())?;
    let whitespace_bytes = rest
        .get(name_end..)
        .ok_or_else(|| "method signature boundary is invalid".to_string())?
        .len()
        .checked_sub(after_name.len())
        .ok_or_else(|| "method signature whitespace is invalid".to_string())?;
    let trimmed_offset = code
        .len()
        .checked_sub(code.trim_start().len())
        .ok_or_else(|| "method declaration trim offset is invalid".to_string())?;
    let open_parenthesis = trimmed_offset
        .checked_add(after_name_offset)
        .and_then(|offset| offset.checked_add(leading_name_whitespace))
        .and_then(|offset| offset.checked_add(whitespace_bytes))
        .ok_or_else(|| "method signature offset overflowed".to_string())?;
    Ok(Some(DeclarationStart {
        kind: method_kind,
        name: name.to_string(),
        open_parenthesis,
    }))
}

fn consume_signature_line(
    scanned: &ScannedBslLine,
    start_offset: usize,
    depth: &mut usize,
    is_cancelled: &mut dyn FnMut() -> bool,
) -> Result<Option<bool>, String> {
    let structural = scanned
        .structural
        .get(start_offset..)
        .ok_or_else(|| "method signature start is outside the source line".to_string())?;
    for (relative_offset, character) in structural.char_indices() {
        if is_cancelled() {
            return Err("discovery cancelled".to_string());
        }
        match character {
            '(' => {
                *depth = depth
                    .checked_add(1)
                    .ok_or_else(|| "method signature nesting overflowed".to_string())?;
            }
            ')' => {
                *depth = depth.checked_sub(1).ok_or_else(|| {
                    "method signature has an unmatched closing parenthesis".to_string()
                })?;
                if *depth == 0 {
                    let tail_offset = start_offset
                        .checked_add(relative_offset)
                        .and_then(|offset| offset.checked_add(character.len_utf8()))
                        .ok_or_else(|| "method signature tail offset overflowed".to_string())?;
                    let tail = scanned.code.get(tail_offset..).ok_or_else(|| {
                        "method signature tail is outside the source line".to_string()
                    })?;
                    return parse_signature_tail(tail, is_cancelled).map(Some);
                }
            }
            _ => {}
        }
    }
    Ok(None)
}

fn parse_signature_tail(
    tail: &str,
    is_cancelled: &mut dyn FnMut() -> bool,
) -> Result<bool, String> {
    let trimmed = tail.trim();
    if trimmed.is_empty() {
        return Ok(false);
    }
    if signature_line_is_export(trimmed, is_cancelled)? {
        return Ok(true);
    }
    Err(format!(
        "unexpected content after method signature: {trimmed:?}"
    ))
}

fn signature_line_is_export(
    line: &str,
    is_cancelled: &mut dyn FnMut() -> bool,
) -> Result<bool, String> {
    let token = line.trim().trim_end_matches(';').trim();
    Ok(matches!(
        normalize_bsl_line_cancellable(token, is_cancelled)?.as_str(),
        "export" | "экспорт"
    ))
}

fn extend_signature_bounds(state: &mut SignatureState, line: &str) -> Result<(), BslParseError> {
    state.lines = state.lines.checked_add(1).ok_or_else(|| {
        BslParseError::Malformed("method signature line count overflowed".to_string())
    })?;
    let line_bytes = line.len().checked_add(1).ok_or_else(|| {
        BslParseError::Malformed("method signature byte count overflowed".to_string())
    })?;
    state.bytes = state.bytes.checked_add(line_bytes).ok_or_else(|| {
        BslParseError::Malformed("method signature byte count overflowed".to_string())
    })?;
    if state.lines > MAX_BSL_SIGNATURE_LINES || state.bytes > MAX_BSL_SIGNATURE_BYTES {
        return Err(BslParseError::Bounded(BslParseBound::Signature {
            max_lines: MAX_BSL_SIGNATURE_LINES,
            max_bytes: MAX_BSL_SIGNATURE_BYTES,
        }));
    }
    Ok(())
}

fn set_method_exported(methods: &mut [ParsedBslMethod], method_index: usize) -> Result<(), String> {
    let method = methods
        .get_mut(method_index)
        .ok_or_else(|| "BSL method index is invalid".to_string())?;
    method.exported = true;
    Ok(())
}

fn process_body_line(
    method_index: usize,
    code: &str,
    line_number: u32,
    methods: &mut [ParsedBslMethod],
    is_cancelled: &mut dyn FnMut() -> bool,
) -> Result<BslParseState, String> {
    if let Some(declaration) = parse_method_declaration(code, is_cancelled)? {
        return Err(format!(
            "nested method declaration {:?} at line {line_number}",
            declaration.name
        ));
    }
    if is_cancelled() {
        return Err("discovery cancelled".to_string());
    }
    let Some(end_kind) = parse_method_end(code, is_cancelled)? else {
        return Ok(BslParseState::Body(method_index));
    };
    let method = methods
        .get_mut(method_index)
        .ok_or_else(|| "BSL method index is invalid".to_string())?;
    if method.kind != end_kind {
        return Err(format!(
            "mismatched method terminator at line {line_number}"
        ));
    }
    method.end_line = line_number;
    Ok(BslParseState::OutsideMethod)
}

fn parse_method_end(
    code: &str,
    is_cancelled: &mut dyn FnMut() -> bool,
) -> Result<Option<BslMethodKind>, String> {
    let keyword = code.trim().trim_end_matches(';').trim();
    Ok(
        match normalize_bsl_line_cancellable(keyword, is_cancelled)?.as_str() {
            "endprocedure" | "конецпроцедуры" => Some(BslMethodKind::Procedure),
            "endfunction" | "конецфункции" => Some(BslMethodKind::Function),
            _ => None,
        },
    )
}

fn is_bsl_path(path: &PortableRelativePath) -> bool {
    path.as_str()
        .rsplit_once('.')
        .is_some_and(|(_stem, extension)| extension.eq_ignore_ascii_case("bsl"))
}

fn module_artifact_for_path(path: &PortableRelativePath) -> Result<ArtifactId, String> {
    let components = path.as_str().split('/').collect::<Vec<_>>();
    let artifact = if components.len() == 5
        && components[0].eq_ignore_ascii_case("CommonForms")
        && components[2].eq_ignore_ascii_case("Ext")
        && components[3].eq_ignore_ascii_case("Form")
        && components[4].eq_ignore_ascii_case("Module.bsl")
    {
        validate_module_component(components[1], "common form name")?;
        let kind = metadata_kind_by_directory(components[0])
            .ok_or_else(|| "common form module uses an unknown metadata directory".to_string())?;
        format!("{}.{}.Module.FormModule", kind.tag, components[1])
    } else if components.len() == 7
        && components
            .get(2)
            .is_some_and(|part| part.eq_ignore_ascii_case("Forms"))
        && components
            .get(4)
            .is_some_and(|part| part.eq_ignore_ascii_case("Ext"))
        && components
            .get(5)
            .is_some_and(|part| part.eq_ignore_ascii_case("Form"))
        && components
            .last()
            .is_some_and(|part| part.eq_ignore_ascii_case("Module.bsl"))
    {
        let kind = metadata_kind_by_directory(components[0])
            .ok_or_else(|| "form module uses an unknown metadata directory".to_string())?;
        validate_module_component(components[1], "metadata object name")?;
        validate_module_component(components[3], "form name")?;
        format!(
            "{}.{}.Form.{}.Module.FormModule",
            kind.tag, components[1], components[3]
        )
    } else if components.len() == 6
        && components[2].eq_ignore_ascii_case("Commands")
        && components[4].eq_ignore_ascii_case("Ext")
        && components[5].eq_ignore_ascii_case("CommandModule.bsl")
    {
        let kind = metadata_kind_by_directory(components[0])
            .ok_or_else(|| "command module uses an unknown metadata directory".to_string())?;
        validate_module_component(components[1], "metadata object name")?;
        validate_module_component(components[3], "command name")?;
        format!(
            "{}.{}.Command.{}.Module.CommandModule",
            kind.tag, components[1], components[3]
        )
    } else if components.len() == 4
        && components[2].eq_ignore_ascii_case("Ext")
        && components[3]
            .rsplit_once('.')
            .is_some_and(|(_stem, extension)| extension.eq_ignore_ascii_case("bsl"))
    {
        let kind = metadata_kind_by_directory(components[0])
            .ok_or_else(|| "module uses an unknown metadata directory".to_string())?;
        validate_module_component(components[1], "metadata object name")?;
        let module_name = components[3]
            .rsplit_once('.')
            .map(|(stem, _extension)| stem)
            .ok_or_else(|| "module filename has no extension".to_string())?;
        validate_module_component(module_name, "module name")?;
        if kind.tag == "CommonModule" && components[3].eq_ignore_ascii_case("Module.bsl") {
            format!("{}.{}.Module.Module", kind.tag, components[1])
        } else {
            format!("{}.{}.Module.{module_name}", kind.tag, components[1])
        }
    } else if components.len() == 2 && components[0].eq_ignore_ascii_case("Ext") {
        let module_name = components[1]
            .rsplit_once('.')
            .map(|(stem, _extension)| stem)
            .ok_or_else(|| "configuration module filename has no extension".to_string())?;
        validate_module_component(module_name, "configuration module name")?;
        format!("Configuration.Root.Module.{module_name}")
    } else {
        return Err("BSL path does not identify a supported canonical module".to_string());
    };
    ArtifactId::parse(&artifact)
        .map_err(|error| format!("module artifact {artifact:?} is invalid: {error}"))
}

fn validate_module_component(value: &str, role: &str) -> Result<(), String> {
    validate_platform_identifier(value)
        .map_err(|message| format!("{role} {value:?} is invalid: {message}"))
}

fn method_artifact(module: &ArtifactId, name: &str) -> Result<ArtifactId, String> {
    ArtifactId::parse(&format!("{}.Method.{name}", module.as_str()))
        .map_err(|error| format!("method artifact {name:?} is invalid: {error}"))
}

pub(crate) struct ExistingIndexDefinitionProvider<'a> {
    selected_root: &'a Path,
    inventory: &'a SourceInventory,
    status: Option<&'a BslIndexStatus>,
}

impl<'a> ExistingIndexDefinitionProvider<'a> {
    pub(crate) fn new(
        selected_root: &'a Path,
        inventory: &'a SourceInventory,
        status: Option<&'a BslIndexStatus>,
    ) -> Self {
        Self {
            selected_root,
            inventory,
            status,
        }
    }

    fn collect(
        &self,
        query: &DiscoveryQuery<'_>,
    ) -> Result<BslCollection<DefinitionFact>, DefinitionCollectionError> {
        check_definition_cancellation(query)?;
        // Ready status proves operational availability, not snapshot identity. Keep
        // structural validation diagnostics, but never publish the unbound batch.
        let _unbound_batch = self.collect_index_facts(query)?;
        Err(DefinitionCollectionError::Unavailable(
            ProviderDiagnostic::material(
                "bsl_definition_freshness_unverified",
                "ready RLM status does not bind the index to the captured source snapshot",
            ),
        ))
    }

    fn collect_index_facts(
        &self,
        query: &DiscoveryQuery<'_>,
    ) -> Result<FactBatch<DefinitionFact>, DefinitionCollectionError> {
        check_definition_cancellation(query)?;
        let db_path = self.validated_db_path()?;
        check_definition_cancellation(query)?;
        let inventory = self.inventory_files()?;
        let inventory_bounded = inventory_is_bounded(self.inventory);
        let max_evidence = usize::from(query.limits().max_evidence);
        let query_limit = max_evidence.max(1);
        let mut hits = Vec::new();
        for term in raw_query_terms(query).map_err(DefinitionCollectionError::Failed)? {
            check_definition_cancellation(query)?;
            let page = find_indexed_definitions(&db_path, &term, query_limit)
                .map_err(classify_index_query_error)?;
            check_definition_cancellation(query)?;
            let page_bounded = page.has_more;
            hits.extend(page.hits);
            if page_bounded || hits.len() > max_evidence {
                break;
            }
        }

        let mut validated_files: BTreeMap<PortableRelativePath, AnalyzedFile> = BTreeMap::new();
        let mut records = Vec::new();
        for hit in hits {
            check_definition_cancellation(query)?;
            let relative_path = PortableRelativePath::parse(&hit.module_path).map_err(|error| {
                DefinitionCollectionError::ContractViolation(ProviderDiagnostic::material(
                    "bsl_index_path_invalid",
                    format!("indexed module path is not canonical and portable: {error}"),
                ))
            })?;
            if !inventory.contains_key(&relative_path) && inventory_bounded {
                continue;
            }
            let analyzed = match validated_files.get(&relative_path) {
                Some(analyzed) => analyzed.clone(),
                None => {
                    let analyzed = self.validate_indexed_file(&relative_path, &inventory, query)?;
                    validated_files.insert(relative_path.clone(), analyzed.clone());
                    analyzed
                }
            };
            let owner = module_artifact_for_path(&relative_path).map_err(|message| {
                DefinitionCollectionError::ContractViolation(ProviderDiagnostic::material(
                    "bsl_index_module_identity",
                    message,
                ))
            })?;
            let source = inventory.get(&relative_path).ok_or_else(|| {
                DefinitionCollectionError::Unavailable(ProviderDiagnostic::material(
                    "bsl_index_stale",
                    "validated indexed module disappeared from the inventory map",
                ))
            })?;
            validate_indexed_method_source(&hit, source, query)?;
            check_definition_cancellation(query)?;
            validate_hit_identity(&hit, &owner)?;
            let definition = method_artifact(&owner, &hit.name).map_err(|message| {
                DefinitionCollectionError::ContractViolation(ProviderDiagnostic::material(
                    "bsl_index_method_identity",
                    message,
                ))
            })?;
            records.push(DefinitionFact {
                owner,
                definition,
                name: hit.name,
                location: EvidenceLocation {
                    relative_path: analyzed.relative_path,
                    line: Some(hit.line),
                    column: None,
                    xml_path: None,
                },
            });
        }
        records.sort();
        reject_duplicate_definition_records(&records)
            .map_err(DefinitionCollectionError::ContractViolation)?;
        if records.len() > max_evidence {
            records.truncate(max_evidence);
        }
        let analyzed_files = validated_files.into_values().collect::<Vec<_>>();
        let contributors = contributors_for_records(&records, &analyzed_files);
        build_batch(records, analyzed_files, contributors)
            .map_err(DefinitionCollectionError::ContractViolation)
    }

    fn validated_db_path(&self) -> Result<PathBuf, DefinitionCollectionError> {
        let selected_root = std::fs::canonicalize(self.selected_root).map_err(|error| {
            DefinitionCollectionError::ContractViolation(ProviderDiagnostic::material(
                "bsl_selected_root_invalid",
                format!("selected source root is not canonical: {error}"),
            ))
        })?;
        if selected_root != self.selected_root {
            return Err(DefinitionCollectionError::ContractViolation(
                ProviderDiagnostic::material(
                    "bsl_selected_root_invalid",
                    "selected source root must already be canonical",
                ),
            ));
        }
        let Some(status) = self.status else {
            return Err(DefinitionCollectionError::Unavailable(
                ProviderDiagnostic::material(
                    "bsl_index_missing",
                    "no existing RLM index status is available",
                ),
            ));
        };
        match status.status.as_str() {
            "ready" => {}
            "stale" | "building" => {
                return Err(DefinitionCollectionError::Unavailable(
                    ProviderDiagnostic::material(
                        "bsl_index_stale",
                        "the existing RLM index is not fresh",
                    ),
                ))
            }
            "failed" | "unavailable" => {
                return Err(DefinitionCollectionError::Unavailable(
                    ProviderDiagnostic::material(
                        "bsl_index_unavailable",
                        "the existing RLM index is unavailable",
                    ),
                ))
            }
            value => {
                return Err(DefinitionCollectionError::ContractViolation(
                    ProviderDiagnostic::material(
                        "bsl_index_status_invalid",
                        format!("RLM index status {value:?} is not recognized"),
                    ),
                ))
            }
        }
        let stored_root = status.source_root.as_deref().ok_or_else(|| {
            DefinitionCollectionError::ContractViolation(ProviderDiagnostic::material(
                "bsl_index_status_invalid",
                "ready RLM index status has no source root",
            ))
        })?;
        let stored_root_path = Path::new(stored_root);
        if !stored_root_path.is_absolute() {
            return Err(DefinitionCollectionError::ContractViolation(
                ProviderDiagnostic::material(
                    "bsl_index_status_invalid",
                    "ready RLM index source root must be absolute",
                ),
            ));
        }
        let stored_root = std::fs::canonicalize(stored_root_path).map_err(|error| {
            DefinitionCollectionError::Unavailable(ProviderDiagnostic::material(
                "bsl_index_stale",
                format!("indexed source root is no longer available: {error}"),
            ))
        })?;
        if stored_root != selected_root {
            return Err(DefinitionCollectionError::Unavailable(
                ProviderDiagnostic::material(
                    "bsl_index_stale",
                    "the existing RLM index belongs to a different source root",
                ),
            ));
        }
        let db_path = status.db_path.as_deref().ok_or_else(|| {
            DefinitionCollectionError::ContractViolation(ProviderDiagnostic::material(
                "bsl_index_status_invalid",
                "ready RLM index status has no database path",
            ))
        })?;
        let db_path = PathBuf::from(db_path);
        if !db_path.is_absolute() {
            return Err(DefinitionCollectionError::ContractViolation(
                ProviderDiagnostic::material(
                    "bsl_index_status_invalid",
                    "ready RLM index database path must be absolute",
                ),
            ));
        }
        let metadata = std::fs::symlink_metadata(&db_path).map_err(|error| {
            DefinitionCollectionError::Unavailable(ProviderDiagnostic::material(
                "bsl_index_missing",
                format!("RLM index database is unavailable: {error}"),
            ))
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(DefinitionCollectionError::ContractViolation(
                ProviderDiagnostic::material(
                    "bsl_index_database_invalid",
                    "RLM index database must be a regular non-link file",
                ),
            ));
        }
        Ok(db_path)
    }

    fn inventory_files(
        &self,
    ) -> Result<BTreeMap<PortableRelativePath, &SourceFile>, DefinitionCollectionError> {
        let mut files = BTreeMap::new();
        for file in self
            .inventory
            .files
            .iter()
            .filter(|file| is_bsl_path(&file.relative_path))
        {
            if files.insert(file.relative_path.clone(), file).is_some() {
                return Err(DefinitionCollectionError::ContractViolation(
                    ProviderDiagnostic::material(
                        "bsl_inventory_path_conflict",
                        "source inventory contains duplicate canonical BSL paths",
                    ),
                ));
            }
        }
        Ok(files)
    }

    fn validate_indexed_file(
        &self,
        relative_path: &PortableRelativePath,
        inventory: &BTreeMap<PortableRelativePath, &SourceFile>,
        query: &DiscoveryQuery<'_>,
    ) -> Result<AnalyzedFile, DefinitionCollectionError> {
        check_definition_cancellation(query)?;
        let result = self.validate_indexed_file_with_reader(
            relative_path,
            inventory,
            |root, path, max_bytes| {
                read_contained_regular_file_cancellable(root, path, max_bytes, || {
                    query.is_cancelled()
                })
            },
        );
        check_definition_cancellation(query)?;
        result
    }

    fn validate_indexed_file_with_reader(
        &self,
        relative_path: &PortableRelativePath,
        inventory: &BTreeMap<PortableRelativePath, &SourceFile>,
        reader: impl FnOnce(&Path, &Path, u64) -> Result<VerifiedFile, ContainedFileError>,
    ) -> Result<AnalyzedFile, DefinitionCollectionError> {
        let Some(expected) = inventory.get(relative_path) else {
            return Err(DefinitionCollectionError::Unavailable(
                ProviderDiagnostic::material(
                    "bsl_index_stale",
                    format!(
                        "indexed module {} is absent from the verified source inventory",
                        relative_path.as_str()
                    ),
                ),
            ));
        };
        let max_bytes = u64::try_from(expected.bytes.len()).map_err(|_error| {
            DefinitionCollectionError::ContractViolation(ProviderDiagnostic::material(
                "bsl_inventory_byte_count_overflow",
                "inventory byte count is not representable as u64",
            ))
        })?;
        let full_path = self.selected_root.join(relative_path.as_str());
        let verified = reader(self.selected_root, &full_path, max_bytes)
            .map_err(classify_indexed_file_error)?;
        if verified.relative_path != expected.relative_path
            || verified.bytes_read != max_bytes
            || verified.raw_sha256 != expected.raw_hash
            || verified.bytes.as_slice() != expected.bytes.as_ref()
        {
            return Err(DefinitionCollectionError::Unavailable(
                ProviderDiagnostic::material(
                    "bsl_index_stale",
                    format!(
                        "indexed module {} changed after inventory capture",
                        relative_path.as_str()
                    ),
                ),
            ));
        }
        Ok(expected.analyzed_file())
    }
}

fn reject_duplicate_definition_records(
    records: &[DefinitionFact],
) -> Result<(), ProviderDiagnostic> {
    let mut unique = BTreeSet::new();
    for record in records {
        if !unique.insert(record) {
            return Err(ProviderDiagnostic::material(
                "bsl_definition_duplicate",
                "validated index rows produced duplicate definition evidence",
            ));
        }
    }
    Ok(())
}

impl DefinitionPort for ExistingIndexDefinitionProvider<'_> {
    fn definitions(
        &self,
        query: &DiscoveryQuery<'_>,
    ) -> ProviderOutcome<FactBatch<DefinitionFact>> {
        if let Some(outcome) = crate::infrastructure::discovery::cancellation_outcome(query) {
            return outcome;
        }
        match self.collect(query) {
            Ok(BslCollection::Complete(batch)) => ProviderOutcome::Complete(batch),
            Ok(BslCollection::Bounded { batch, diagnostic }) => ProviderOutcome::Bounded {
                data: batch,
                diagnostic,
            },
            Err(DefinitionCollectionError::Unavailable(diagnostic)) => {
                ProviderOutcome::Unavailable(diagnostic)
            }
            Err(DefinitionCollectionError::Failed(diagnostic)) => {
                ProviderOutcome::Failed(diagnostic)
            }
            Err(DefinitionCollectionError::ContractViolation(diagnostic)) => {
                ProviderOutcome::ContractViolation(diagnostic)
            }
        }
    }
}

enum DefinitionCollectionError {
    Unavailable(ProviderDiagnostic),
    Failed(ProviderDiagnostic),
    ContractViolation(ProviderDiagnostic),
}

fn check_definition_cancellation(
    query: &DiscoveryQuery<'_>,
) -> Result<(), DefinitionCollectionError> {
    crate::infrastructure::discovery::check_cancellation(query)
        .map_err(DefinitionCollectionError::Failed)
}

fn classify_index_query_error(error: IndexQueryError) -> DefinitionCollectionError {
    match error {
        IndexQueryError::Unavailable(message) => DefinitionCollectionError::Unavailable(
            ProviderDiagnostic::material("bsl_index_missing", message),
        ),
        IndexQueryError::InvalidLimit(message) => DefinitionCollectionError::ContractViolation(
            ProviderDiagnostic::material("bsl_index_query_limit_invalid", message),
        ),
        IndexQueryError::MalformedSchema(message) => DefinitionCollectionError::ContractViolation(
            ProviderDiagnostic::material("bsl_index_schema_invalid", message),
        ),
        IndexQueryError::MalformedRow(message) => DefinitionCollectionError::ContractViolation(
            ProviderDiagnostic::material("bsl_index_row_invalid", message),
        ),
        IndexQueryError::Failed(message) => DefinitionCollectionError::Failed(
            ProviderDiagnostic::material("bsl_index_query_failed", message),
        ),
    }
}

fn classify_indexed_file_error(error: ContainedFileError) -> DefinitionCollectionError {
    match error {
        ContainedFileError::Cancelled => DefinitionCollectionError::Failed(
            crate::infrastructure::discovery::cancellation_diagnostic(),
        ),
        ContainedFileError::UnsupportedHost => {
            DefinitionCollectionError::Unavailable(ProviderDiagnostic::material(
                "bsl_index_validation_unavailable",
                "verified indexed-file validation is unavailable on this host",
            ))
        }
        ContainedFileError::SizeLimitExceeded { limit: _ } => {
            DefinitionCollectionError::Unavailable(ProviderDiagnostic::material(
                "bsl_index_stale",
                "indexed module grew after inventory capture",
            ))
        }
        ContainedFileError::IdentityChanged => {
            DefinitionCollectionError::Unavailable(ProviderDiagnostic::material(
                "bsl_index_stale",
                "indexed module identity changed after inventory capture",
            ))
        }
        ContainedFileError::Io { operation, source } if source.kind() == ErrorKind::NotFound => {
            DefinitionCollectionError::Unavailable(ProviderDiagnostic::material(
                "bsl_index_stale",
                format!("indexed module disappeared during {operation}: {source}"),
            ))
        }
        ContainedFileError::Io { operation, source } => {
            DefinitionCollectionError::Failed(ProviderDiagnostic::material(
                "bsl_index_validation_failed",
                format!("indexed module validation failed during {operation}: {source}"),
            ))
        }
        error @ (ContainedFileError::RootNotCanonical
        | ContainedFileError::RootNotDirectory
        | ContainedFileError::PathOutsideRoot
        | ContainedFileError::FinalPathOutsideRoot
        | ContainedFileError::FinalPathMismatch
        | ContainedFileError::AmbiguousHostPath
        | ContainedFileError::InvalidRelativePath(_)
        | ContainedFileError::SymlinkOrReparsePoint
        | ContainedFileError::NotRegularFile
        | ContainedFileError::LengthOverflow) => {
            DefinitionCollectionError::ContractViolation(ProviderDiagnostic::material(
                "bsl_index_file_contract",
                format!("indexed module failed contained validation: {error}"),
            ))
        }
    }
}

fn validate_hit_identity(
    hit: &IndexedMethodHit,
    owner: &ArtifactId,
) -> Result<(), DefinitionCollectionError> {
    validate_platform_identifier(&hit.name).map_err(|message| {
        DefinitionCollectionError::ContractViolation(ProviderDiagnostic::material(
            "bsl_index_method_identity",
            format!("indexed method name {:?} is invalid: {message}", hit.name),
        ))
    })?;
    let path = PortableRelativePath::parse(&hit.module_path).map_err(|error| {
        DefinitionCollectionError::ContractViolation(ProviderDiagnostic::material(
            "bsl_index_path_invalid",
            format!("indexed module path is not canonical and portable: {error}"),
        ))
    })?;
    let components = path.as_str().split('/').collect::<Vec<_>>();
    let expected_category = owner.as_str().split('.').next().ok_or_else(|| {
        DefinitionCollectionError::ContractViolation(ProviderDiagnostic::material(
            "bsl_index_module_identity",
            "canonical module owner has no category segment",
        ))
    })?;
    if let Some(category) = hit.category.as_deref() {
        if normalize_discovery_identity(category) != normalize_discovery_identity(expected_category)
        {
            return Err(DefinitionCollectionError::ContractViolation(
                ProviderDiagnostic::material(
                    "bsl_index_module_identity",
                    "indexed category conflicts with its canonical module path",
                ),
            ));
        }
    }
    if let (Some(object_name), Some(expected_object)) =
        (hit.object_name.as_deref(), components.get(1))
    {
        if normalize_discovery_identity(object_name)
            != normalize_discovery_identity(expected_object)
        {
            return Err(DefinitionCollectionError::ContractViolation(
                ProviderDiagnostic::material(
                    "bsl_index_module_identity",
                    "indexed object name conflicts with its canonical module path",
                ),
            ));
        }
    }
    if let Some(module_type) = hit.module_type.as_deref() {
        let expected_module_type = if components
            .first()
            .is_some_and(|component| component.eq_ignore_ascii_case("CommonForms"))
            || components
                .iter()
                .any(|component| component.eq_ignore_ascii_case("Forms"))
        {
            Some("FormModule")
        } else {
            components
                .last()
                .and_then(|file_name| file_name.rsplit_once('.').map(|(stem, _extension)| stem))
        };
        if expected_module_type.is_some_and(|expected| {
            normalize_discovery_identity(module_type) != normalize_discovery_identity(expected)
        }) {
            return Err(DefinitionCollectionError::ContractViolation(
                ProviderDiagnostic::material(
                    "bsl_index_module_identity",
                    "indexed module type conflicts with its canonical module path",
                ),
            ));
        }
    }
    Ok(())
}

fn validate_indexed_method_source(
    hit: &IndexedMethodHit,
    source: &SourceFile,
    query: &DiscoveryQuery<'_>,
) -> Result<(), DefinitionCollectionError> {
    check_definition_cancellation(query)?;
    let text = std::str::from_utf8(&source.bytes).map_err(|error| {
        DefinitionCollectionError::ContractViolation(ProviderDiagnostic::material(
            "bsl_malformed",
            format!(
                "indexed BSL source {} is not UTF-8: {error}",
                source.relative_path.as_str()
            ),
        ))
    })?;
    let parsed = parse_bsl_source_cancellable(text, query).map_err(|error| match error {
        BslParseError::Cancelled => DefinitionCollectionError::Failed(
            crate::infrastructure::discovery::cancellation_diagnostic(),
        ),
        BslParseError::Bounded(_) => {
            DefinitionCollectionError::Unavailable(ProviderDiagnostic::material(
                "bsl_index_source_bound",
                format!(
                    "indexed BSL source {} exceeded a parser resource limit",
                    source.relative_path.as_str()
                ),
            ))
        }
        BslParseError::Malformed(message) => {
            DefinitionCollectionError::ContractViolation(ProviderDiagnostic::material(
                "bsl_malformed",
                format!(
                    "indexed BSL source {} is malformed: {message}",
                    source.relative_path.as_str()
                ),
            ))
        }
    })?;
    let Some(method) = parsed
        .methods
        .iter()
        .find(|method| method.declaration_line == hit.line)
    else {
        return Err(stale_method_location(hit));
    };
    let indexed_kind = match hit.method_kind {
        IndexedMethodKind::Procedure => BslMethodKind::Procedure,
        IndexedMethodKind::Function => BslMethodKind::Function,
    };
    if method.kind != indexed_kind
        || normalize_discovery_identity(&method.name) != normalize_discovery_identity(&hit.name)
        || method.exported != hit.exported
        || method.end_line != hit.end_line
    {
        return Err(stale_method_location(hit));
    }
    Ok(())
}

fn stale_method_location(hit: &IndexedMethodHit) -> DefinitionCollectionError {
    DefinitionCollectionError::Unavailable(ProviderDiagnostic::material(
        "bsl_index_stale",
        format!(
            "indexed method {:?} no longer matches {}:{}-{}",
            hit.name,
            hit.module_path.display(),
            hit.line,
            hit.end_line
        ),
    ))
}

pub(crate) struct UnavailableRuntimeFlowProvider;

impl RuntimeFlowPort for UnavailableRuntimeFlowProvider {
    fn runtime_flow(
        &self,
        _query: &DiscoveryQuery<'_>,
    ) -> ProviderOutcome<FactBatch<RuntimeFlowFact>> {
        ProviderOutcome::Unavailable(ProviderDiagnostic::material(
            "runtime_flow_unavailable",
            "no validated typed runtime-flow graph is available without starting workspace services",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ExistingIndexDefinitionProvider, InventoryBslSearchProvider, UnavailableRuntimeFlowProvider,
    };
    use crate::application::discovery::ports::{BslSearchPort, DefinitionPort, RuntimeFlowPort};
    use crate::domain::discovery::{
        ArtifactId, ArtifactKind, ContentHash, DefinitionFact, DiscoveryQuery,
        DiscoveryQueryLimits, EvidenceLocation, PortableRelativePath, ProviderCoverage,
        ProviderOutcome, SourceFile, SourceInventory,
    };
    use crate::infrastructure::workspace_index::BslIndexStatus;
    use rusqlite::Connection;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    const MODULE_PATH: &str = "CommonModules/Серии/Ext/Module.bsl";
    const BSL: &[u8] = b"// module\n\
\xD0\x9F\xD1\x80\xD0\xBE\xD1\x86\xD0\xB5\xD0\xB4\xD1\x83\xD1\x80\xD0\xB0 \
\xD0\xA0\xD0\xB0\xD1\x81\xD1\x81\xD1\x87\xD0\xB8\xD1\x82\xD0\xB0\xD1\x82\xD1\x8C\
\xD0\xA1\xD0\xB5\xD1\x80\xD0\xB8\xD1\x8E() \
\xD0\xAD\xD0\xBA\xD1\x81\xD0\xBF\xD0\xBE\xD1\x80\xD1\x82\n\
    // body\n\
\xD0\x9A\xD0\xBE\xD0\xBD\xD0\xB5\xD1\x86\xD0\x9F\xD1\x80\xD0\xBE\xD1\x86\xD0\xB5\xD0\xB4\xD1\x83\xD1\x80\xD1\x8B\n\
\xD0\xA4\xD1\x83\xD0\xBD\xD0\xBA\xD1\x86\xD0\xB8\xD1\x8F \
\xD0\x9F\xD0\xBE\xD0\xBB\xD1\x83\xD1\x87\xD0\xB8\xD1\x82\xD1\x8C\
\xD0\xA1\xD0\xB5\xD1\x80\xD0\xB8\xD1\x8E(\xD0\x9A\xD0\xBE\xD0\xB4)\n\
    \xD0\x92\xD0\xBE\xD0\xB7\xD0\xB2\xD1\x80\xD0\xB0\xD1\x82 \xD0\x9A\xD0\xBE\xD0\xB4;\n\
\xD0\x9A\xD0\xBE\xD0\xBD\xD0\xB5\xD1\x86\xD0\xA4\xD1\x83\xD0\xBD\xD0\xBA\xD1\x86\xD0\xB8\xD0\xB8\n";

    #[test]
    fn cancelled_query_stops_lexical_and_index_providers_before_records() {
        let cancellation = crate::domain::cancellation::CancellationToken::new();
        cancellation.cancel();
        let query = query("серии", &[], 10).with_cancellation(&cancellation);
        let inventory = SourceInventory::empty();

        let lexical = InventoryBslSearchProvider.search(&query, &inventory);
        let definitions =
            ExistingIndexDefinitionProvider::new(Path::new("/must/not/be/read"), &inventory, None)
                .definitions(&query);

        let ProviderOutcome::Failed(lexical_diagnostic) = lexical else {
            panic!("cancelled lexical provider must return failed");
        };
        let ProviderOutcome::Failed(definition_diagnostic) = definitions else {
            panic!("cancelled index provider must return failed");
        };
        assert_eq!(lexical_diagnostic.code, "discovery_cancelled");
        assert_eq!(definition_diagnostic.code, "discovery_cancelled");
    }

    #[test]
    fn indexed_file_revalidation_cancels_between_verified_read_chunks() {
        let fixture = Fixture::new("definition-cancelled-during-reread");
        let bytes = vec![
            b'x';
            crate::infrastructure::platform::contained_file::VERIFIED_READ_CHUNK_BYTES
                * 3
        ];
        let source = fixture.write_source(MODULE_PATH, &bytes);
        let inventory = inventory(vec![source]);
        let provider = ExistingIndexDefinitionProvider::new(&fixture.source_root, &inventory, None);
        let Ok(inventory_files) = provider.inventory_files() else {
            panic!("expected inventory map");
        };
        let relative_path = PortableRelativePath::parse_str(MODULE_PATH).unwrap();
        let cancellation = crate::domain::cancellation::CancellationToken::new();
        let mut chunks = 0_u8;

        let error = provider
            .validate_indexed_file_with_reader(
                &relative_path,
                &inventory_files,
                |root, path, max_bytes| {
                    crate::infrastructure::platform::contained_file::read_contained_regular_file_with_chunk_observer_cancellable(
                        root,
                        path,
                        max_bytes,
                        || cancellation.is_cancelled(),
                        |_| {
                            chunks += 1;
                            cancellation.cancel();
                        },
                    )
                },
            )
            .expect_err("indexed-file reread must stop between chunks");

        let super::DefinitionCollectionError::Failed(diagnostic) = error else {
            panic!("cancellation must be a failed provider outcome");
        };
        assert_eq!(diagnostic.code, "discovery_cancelled");
        assert_eq!(chunks, 1);
    }

    #[test]
    fn indexed_file_identity_change_is_staleness_not_a_contract_violation() {
        let error = super::classify_indexed_file_error(
            crate::infrastructure::platform::contained_file::ContainedFileError::IdentityChanged,
        );

        let super::DefinitionCollectionError::Unavailable(diagnostic) = error else {
            panic!("post-inventory replacement must be classified as stale");
        };
        assert_eq!(diagnostic.code, "bsl_index_stale");
    }

    #[test]
    fn parser_observes_cancellation_while_scanning_one_long_line() {
        let source = "content ".repeat(64);
        let mut checks = 0_u8;

        let result = super::parse_bsl_source_observing(&source, || {
            checks += 1;
            checks == 4
        });
        let Err(error) = result else {
            panic!("long-line scan must stop at the cancellation boundary");
        };

        assert_eq!(error, super::BslParseError::Cancelled);
        assert_eq!(checks, 4);
    }

    #[test]
    fn parser_observes_cancellation_at_the_post_scan_handoff() {
        let source = "content ".repeat(64);
        let cancel_at = source.chars().count() + 3;
        let mut checks = 0_usize;

        let result = super::parse_bsl_source_observing(&source, || {
            checks += 1;
            checks == cancel_at
        });
        let Err(error) = result else {
            panic!("post-scan parsing must observe cancellation before token work");
        };

        assert_eq!(error, super::BslParseError::Cancelled);
        assert_eq!(checks, cancel_at);
    }

    #[test]
    fn parser_rejects_a_line_larger_than_the_bounded_parse_unit() {
        let source = "x".repeat(super::MAX_BSL_LINE_BYTES + 1);

        let result = super::parse_bsl_source_observing(&source, || false);
        let Err(error) = result else {
            panic!("an oversized BSL line must be rejected");
        };

        assert_eq!(
            error,
            super::BslParseError::Bounded(super::BslParseBound::LineBytes {
                limit: super::MAX_BSL_LINE_BYTES,
            })
        );
    }

    #[test]
    fn oversized_line_stops_at_the_first_byte_beyond_the_parse_unit() {
        let source = "x".repeat(8 * super::MAX_BSL_LINE_BYTES);
        let mut cancellation_polls = 0_usize;

        let error = super::parse_bsl_source_observing(&source, || {
            cancellation_polls += 1;
            false
        })
        .expect_err("oversized line must stop without scanning the full tail");

        assert!(matches!(
            error,
            super::BslParseError::Bounded(super::BslParseBound::LineBytes { .. })
        ));
        assert!(cancellation_polls < 100, "polls: {cancellation_polls}");
    }

    #[test]
    fn crlf_line_shape_keeps_exact_limit_and_rejects_limit_plus_one() {
        let exact = format!("{}\r\n", "x".repeat(super::MAX_BSL_LINE_BYTES));
        let oversized = format!("{}\r\n", "x".repeat(super::MAX_BSL_LINE_BYTES + 1));

        assert!(super::parse_bsl_source_observing(&exact, || false).is_ok());
        assert!(matches!(
            super::parse_bsl_source_observing(&oversized, || false),
            Err(super::BslParseError::Bounded(
                super::BslParseBound::LineBytes { .. }
            ))
        ));
    }

    #[test]
    fn method_ownership_is_stored_as_sparse_ranges() {
        let source = format!(
            "// header\n{}Процедура Первая()\nКонецПроцедуры\n{}Функция Вторая()\nКонецФункции\n",
            "// gap\n".repeat(10_000),
            "\n".repeat(10_000),
        );

        let parsed = super::parse_bsl_source_observing(&source, || false).expect("valid source");

        assert_eq!(parsed.methods.len(), 2);
        assert_eq!(parsed.method_ranges.len(), 2);
        assert_eq!(
            super::method_index_for_line(&parsed.method_ranges, 10_002),
            Some(0)
        );
        assert_eq!(
            super::method_index_for_line(&parsed.method_ranges, 20_004),
            Some(1)
        );
        assert_eq!(super::method_index_for_line(&parsed.method_ranges, 2), None);
    }

    #[test]
    fn line_bound_is_a_bounded_lexical_prefix_with_prior_file_coverage() {
        let prior = source_file("CommonModules/A/Ext/Module.bsl", b"// needle\n");
        let oversized = source_file(
            "CommonModules/Z/Ext/Module.bsl",
            "x".repeat(super::MAX_BSL_LINE_BYTES + 1).as_bytes(),
        );

        let outcome = InventoryBslSearchProvider.search(
            &query("needle", &[], 10),
            &inventory(vec![prior.clone(), oversized]),
        );

        let ProviderOutcome::Bounded { data, diagnostic } = outcome else {
            panic!("line resource exhaustion must be bounded");
        };
        assert_eq!(diagnostic.code, "bsl_source_line_bound");
        assert_eq!(data.analyzed_files, vec![prior.analyzed_file()]);
        assert_eq!(data.records.len(), 1);
    }

    #[test]
    fn lexical_matches_keep_the_same_sorted_n_plus_one_prefix_for_any_file_order() {
        let first = source_file(
            "CommonModules/A/Ext/Module.bsl",
            b"// needle one\n// needle two\n",
        );
        let second = source_file(
            "CommonModules/Z/Ext/Module.bsl",
            b"// needle three\n// needle four\n",
        );
        let forward = InventoryBslSearchProvider.search(
            &query("needle", &[], 2),
            &inventory(vec![first.clone(), second.clone()]),
        );
        let reverse = InventoryBslSearchProvider
            .search(&query("needle", &[], 2), &inventory(vec![second, first]));

        let ProviderOutcome::Bounded { data: forward, .. } = forward else {
            panic!("expected bounded forward result");
        };
        let ProviderOutcome::Bounded { data: reverse, .. } = reverse else {
            panic!("expected bounded reverse result");
        };
        assert_eq!(forward.records, reverse.records);
        assert_eq!(forward.records.len(), 2);
    }

    #[test]
    fn lexical_scan_extracts_cyrillic_procedure_and_function_matches_at_exact_lines() {
        let inventory = inventory(vec![source_file(MODULE_PATH, BSL)]);
        let search_terms = vec!["получитьсерию".to_string()];
        let query = query("рассчитатьсерию", &search_terms, 10);

        let outcome = InventoryBslSearchProvider.search(&query, &inventory);

        let ProviderOutcome::Complete(batch) = outcome else {
            panic!("expected complete lexical evidence");
        };
        assert_eq!(batch.records.len(), 2);
        assert!(batch
            .records
            .iter()
            .all(|record| record.artifact_kind == ArtifactKind::Method));
        let mut lines = batch
            .records
            .iter()
            .map(|record| record.location.line)
            .collect::<Vec<_>>();
        lines.sort();
        assert_eq!(lines, vec![Some(2), Some(5)]);
        assert_eq!(
            batch
                .records
                .iter()
                .find(|record| record.location.line == Some(2))
                .map(|record| &record.artifact),
            Some(&artifact(
                "CommonModule.Серии.Module.Module.Method.РассчитатьСерию"
            ))
        );
        assert_eq!(batch.contributors, batch.analyzed_files);
    }

    #[test]
    fn canonical_module_identity_covers_form_and_command_modules() {
        assert_eq!(
            super::module_artifact_for_path(&path(
                "Catalogs/Товары/Forms/ФормаЭлемента/Ext/Form/Module.bsl"
            )),
            Ok(artifact(
                "Catalog.Товары.Form.ФормаЭлемента.Module.FormModule"
            ))
        );
        assert_eq!(
            super::module_artifact_for_path(&path(
                "Documents/Заказ/Commands/Заполнить/Ext/CommandModule.bsl"
            )),
            Ok(artifact(
                "Document.Заказ.Command.Заполнить.Module.CommandModule"
            ))
        );
        assert_eq!(
            super::module_artifact_for_path(&path("CommonForms/ВыборСерии/Ext/Form/Module.bsl")),
            Ok(artifact("CommonForm.ВыборСерии.Module.FormModule"))
        );
        assert_eq!(
            super::module_artifact_for_path(&path("CommonModules/Серии/Ext/Module.bsl")),
            Ok(artifact("CommonModule.Серии.Module.Module"))
        );
    }

    #[test]
    fn canonical_module_identity_rejects_hostile_variable_components() {
        for raw_path in [
            "CommonModules/Bad Name/Ext/Module.bsl",
            "CommonModules/Bad.Name/Ext/Module.bsl",
            "CommonModules/Bad\u{0085}Name/Ext/Module.bsl",
            "Catalogs/Товары/Forms/Bad Name/Ext/Form/Module.bsl",
            "Documents/Заказ/Commands/Bad.Name/Ext/CommandModule.bsl",
            "Catalogs/Товары/Ext/Bad Name.bsl",
        ] {
            let outcome = InventoryBslSearchProvider.search(
                &query("Несуществующий", &[], 10),
                &inventory(vec![source_file(raw_path, b"// valid module\n")]),
            );
            assert!(
                matches!(outcome, ProviderOutcome::ContractViolation(_)),
                "hostile path component must fail: {raw_path:?}"
            );
        }
    }

    #[test]
    fn lexical_no_match_is_complete_but_invalid_utf8_and_malformed_bsl_are_violations() {
        let no_match = InventoryBslSearchProvider.search(
            &query("НесуществующийМетод", &[], 10),
            &inventory(vec![source_file(MODULE_PATH, BSL)]),
        );
        let ProviderOutcome::Complete(no_match) = no_match else {
            panic!("expected complete no-match evidence");
        };
        assert!(no_match.records.is_empty());
        assert_eq!(no_match.analyzed_files.len(), 1);
        assert!(no_match.contributors.is_empty());

        for bytes in [
            vec![0xff, 0xfe],
            "Процедура ()\nКонецПроцедуры\n".as_bytes().to_vec(),
        ] {
            let outcome = InventoryBslSearchProvider.search(
                &query("Процедура", &[], 10),
                &inventory(vec![source_file(MODULE_PATH, &bytes)]),
            );
            assert!(matches!(outcome, ProviderOutcome::ContractViolation(_)));
        }
    }

    #[test]
    fn lexical_parser_handles_bom_multiline_export_and_url_default_strings() {
        let source = "\u{feff}Функция ПолучитьСерию(\n\
    Адрес = \"http://example.test/a//b\",\n\
    Текст = \"Он сказал \"\"Да // именно\"\"\"\n\
)\n\
Экспорт\n\
    Возврат Адрес;\n\
КонецФункции\n";
        let terms = vec!["http://example.test/a//b".to_string()];

        let outcome = InventoryBslSearchProvider.search(
            &query("ПолучитьСерию", &terms, 10),
            &inventory(vec![source_file(MODULE_PATH, source.as_bytes())]),
        );

        let ProviderOutcome::Complete(batch) = outcome else {
            panic!("expected valid multiline BSL signature");
        };
        assert!(batch.records.iter().all(|fact| {
            fact.artifact == artifact("CommonModule.Серии.Module.Module.Method.ПолучитьСерию")
                && fact.artifact_kind == ArtifactKind::Method
        }));
        let url = batch
            .records
            .iter()
            .find(|fact| fact.matched_text.contains("http://"))
            .expect("URL default evidence");
        assert_eq!(url.location.line, Some(2));
        assert_eq!(url.location.column, Some(10));
    }

    #[test]
    fn definition_revalidation_uses_multiline_signature_and_later_export() {
        let fixture = Fixture::new("definition-multiline-signature");
        let source_bytes = "\u{feff}Функция ПолучитьСерию(\n\
    Адрес = \"http://example.test/a//b\",\n\
    Текст = \"Он сказал \"\"Да // именно\"\"\"\n\
)\n\
Экспорт\n\
    Возврат Адрес;\n\
КонецФункции\n"
            .as_bytes();
        let source = fixture.write_source(MODULE_PATH, source_bytes);
        let inventory = inventory(vec![source]);
        let db_path = fixture.root.join("index.db");
        create_index(&db_path, MODULE_PATH, 1, 7);
        Connection::open(&db_path)
            .unwrap()
            .execute("UPDATE methods SET is_export = 1", ())
            .unwrap();
        let status = ready_status(&fixture.source_root, &db_path);

        let provider =
            ExistingIndexDefinitionProvider::new(&fixture.source_root, &inventory, Some(&status));
        let Ok(batch) = provider.collect_index_facts(&query("ПолучитьСерию", &[], 10))
        else {
            panic!("expected validated multiline indexed definition");
        };
        assert_eq!(batch.records.len(), 1);
        assert_eq!(
            batch.records[0].owner,
            artifact("CommonModule.Серии.Module.Module")
        );
        assert_eq!(batch.records[0].location.line, Some(1));
    }

    #[test]
    fn definition_revalidation_treats_parser_resource_bounds_as_unavailable() {
        let fixture = Fixture::new("definition-source-bound");
        let source_bytes = "x".repeat(super::MAX_BSL_LINE_BYTES + 1).into_bytes();
        let source = fixture.write_source(MODULE_PATH, &source_bytes);
        let inventory = inventory(vec![source]);
        let db_path = fixture.root.join("index.db");
        create_index(&db_path, MODULE_PATH, 1, 1);
        let status = ready_status(&fixture.source_root, &db_path);
        let provider =
            ExistingIndexDefinitionProvider::new(&fixture.source_root, &inventory, Some(&status));

        let outcome = provider.collect_index_facts(&query("ПолучитьСерию", &[], 10));

        let Err(super::DefinitionCollectionError::Unavailable(diagnostic)) = outcome else {
            panic!("parser bounds must not become definition contract violations");
        };
        assert_eq!(diagnostic.code, "bsl_index_source_bound");
    }

    #[test]
    fn common_form_lexical_and_definition_method_identities_agree() {
        let fixture = Fixture::new("common-form-method-identity");
        let module_path = "CommonForms/ВыборСерии/Ext/Form/Module.bsl";
        let source_bytes = "Функция ПолучитьСерию(Код)\nКонецФункции\n".as_bytes();
        let source = fixture.write_source(module_path, source_bytes);
        let inventory = inventory(vec![source]);
        let db_path = fixture.root.join("index.db");
        create_index(&db_path, module_path, 1, 2);
        Connection::open(&db_path)
            .unwrap()
            .execute(
                "UPDATE modules
                 SET category = 'CommonForm', object_name = 'ВыборСерии',
                     module_type = 'FormModule'",
                (),
            )
            .unwrap();
        let status = ready_status(&fixture.source_root, &db_path);

        let lexical =
            InventoryBslSearchProvider.search(&query("ПолучитьСерию", &[], 10), &inventory);
        let provider =
            ExistingIndexDefinitionProvider::new(&fixture.source_root, &inventory, Some(&status));
        let definitions = provider.collect_index_facts(&query("ПолучитьСерию", &[], 10));

        let ProviderOutcome::Complete(lexical) = lexical else {
            panic!("expected common-form lexical evidence");
        };
        let Ok(definitions) = definitions else {
            panic!("expected common-form definition evidence");
        };
        assert_eq!(lexical.records.len(), 1);
        assert_eq!(definitions.records.len(), 1);
        assert_eq!(
            lexical.records[0].artifact,
            definitions.records[0].definition
        );
        assert_eq!(
            definitions.records[0].owner,
            artifact("CommonForm.ВыборСерии.Module.FormModule")
        );
    }

    #[test]
    fn lexical_parser_rejects_unbalanced_or_unbounded_signatures() {
        let unbalanced = "Функция ПолучитьСерию(Адрес = \"http://example.test\"\n\
КонецФункции\n";
        let too_many_lines = format!(
            "Функция ПолучитьСерию(\n{}\n)\nКонецФункции\n",
            "Параметр,\n".repeat(65)
        );
        let oversized = format!(
            "Функция ПолучитьСерию(Текст = \"{}\")\nКонецФункции\n",
            "x".repeat(65 * 1024)
        );

        let malformed = InventoryBslSearchProvider.search(
            &query("ПолучитьСерию", &[], 10),
            &inventory(vec![source_file(MODULE_PATH, unbalanced.as_bytes())]),
        );
        assert!(matches!(malformed, ProviderOutcome::ContractViolation(_)));

        for source in [too_many_lines, oversized] {
            let bounded = InventoryBslSearchProvider.search(
                &query("ПолучитьСерию", &[], 10),
                &inventory(vec![source_file(MODULE_PATH, source.as_bytes())]),
            );
            assert!(matches!(bounded, ProviderOutcome::Bounded { .. }));
        }
    }

    #[test]
    fn lexical_evidence_and_inventory_truncation_are_bounded() {
        let evidence = InventoryBslSearchProvider.search(
            &query("сер", &["получить".to_string()], 1),
            &inventory(vec![source_file(MODULE_PATH, BSL)]),
        );
        let ProviderOutcome::Bounded { data, diagnostic } = evidence else {
            panic!("expected evidence bound");
        };
        assert_eq!(data.records.len(), 1);
        assert_eq!(diagnostic.code, "bsl_evidence_bound");

        let file = source_file(MODULE_PATH, BSL);
        let bounded_inventory = SourceInventory {
            files: vec![file.clone()],
            coverage: ProviderCoverage::new(2, 1, file.bytes.len() as u64, 1),
        };
        let outcome = InventoryBslSearchProvider
            .search(&query("НесуществующийМетод", &[], 10), &bounded_inventory);
        assert!(matches!(outcome, ProviderOutcome::Bounded { .. }));
    }

    #[test]
    fn existing_index_definition_facts_are_structural_and_source_validated() {
        let fixture = Fixture::new("definition-ready");
        let source = fixture.write_source(MODULE_PATH, BSL);
        let inventory = inventory(vec![source]);
        let db_path = fixture.root.join("index.db");
        create_index(&db_path, MODULE_PATH, 5, 7);
        let status = ready_status(&fixture.source_root, &db_path);
        let provider =
            ExistingIndexDefinitionProvider::new(&fixture.source_root, &inventory, Some(&status));
        let search_terms = vec!["ПолучитьСерию".to_string()];

        let outcome = provider.collect_index_facts(&query("Найти серию", &search_terms, 10));

        let Ok(batch) = outcome else {
            panic!("expected complete definition evidence");
        };
        assert_eq!(batch.records.len(), 1);
        assert_eq!(
            batch.records[0].owner,
            artifact("CommonModule.Серии.Module.Module")
        );
        assert_eq!(
            batch.records[0].definition,
            artifact("CommonModule.Серии.Module.Module.Method.ПолучитьСерию")
        );
        assert_eq!(batch.records[0].location.line, Some(5));
        assert_eq!(batch.contributors, batch.analyzed_files);
    }

    #[test]
    fn definition_query_preserves_distinct_cyrillic_term_spelling() {
        let fixture = Fixture::new("definition-cyrillic-case");
        let source = fixture.write_source(MODULE_PATH, BSL);
        let inventory = inventory(vec![source]);
        let db_path = fixture.root.join("index.db");
        create_index(&db_path, MODULE_PATH, 5, 7);
        let status = ready_status(&fixture.source_root, &db_path);
        let provider =
            ExistingIndexDefinitionProvider::new(&fixture.source_root, &inventory, Some(&status));
        let search_terms = vec!["ПолучитьСерию".to_string()];

        let outcome = provider.collect_index_facts(&query("получитьсерию", &search_terms, 10));

        let Ok(batch) = outcome else {
            panic!("expected exact Cyrillic search term to remain queryable");
        };
        assert_eq!(batch.records.len(), 1);
    }

    #[test]
    fn definition_query_terms_deduplicate_by_accepted_identity() {
        let search_terms = vec!["ПолучитьСерию".to_string(), "  получитьсерию  ".to_string()];
        let query = query("ПОЛУЧИТЬСЕРИЮ", &search_terms, 10);

        let terms = super::raw_query_terms(&query).expect("query terms");

        assert_eq!(terms, vec!["получитьсерию"]);
    }

    #[test]
    fn duplicate_validated_definition_records_are_contract_violations() {
        let fact = DefinitionFact {
            owner: artifact("CommonModule.Серии.Module.Module"),
            definition: artifact("CommonModule.Серии.Module.Module.Method.ПолучитьСерию"),
            name: "ПолучитьСерию".to_string(),
            location: EvidenceLocation {
                relative_path: path(MODULE_PATH),
                line: Some(5),
                column: None,
                xml_path: None,
            },
        };

        let result = super::reject_duplicate_definition_records(&[fact.clone(), fact]);

        assert!(result.is_err());
    }

    #[test]
    fn missing_or_stale_existing_index_is_unavailable_without_starting_work() {
        let fixture = Fixture::new("definition-unavailable");
        let inventory = inventory(vec![fixture.write_source(MODULE_PATH, BSL)]);
        let missing = ExistingIndexDefinitionProvider::new(&fixture.source_root, &inventory, None)
            .definitions(&query("Найти", &[], 10));
        assert!(matches!(missing, ProviderOutcome::Unavailable(_)));

        let stale = BslIndexStatus {
            status: "stale".to_string(),
            source_root: Some(fixture.source_root.display().to_string()),
            db_path: Some(fixture.root.join("index.db").display().to_string()),
            message: None,
            updated_at: 0,
            last_run: None,
        };
        let stale =
            ExistingIndexDefinitionProvider::new(&fixture.source_root, &inventory, Some(&stale))
                .definitions(&query("Найти", &[], 10));
        assert!(matches!(stale, ProviderOutcome::Unavailable(_)));
    }

    #[test]
    fn ready_index_hits_are_unavailable_without_snapshot_generation_proof() {
        let fixture = Fixture::new("definition-freshness-hit");
        let inventory = inventory(vec![fixture.write_source(MODULE_PATH, BSL)]);
        let db_path = fixture.root.join("index.db");
        create_index(&db_path, MODULE_PATH, 5, 7);
        let status = ready_status(&fixture.source_root, &db_path);

        let outcome =
            ExistingIndexDefinitionProvider::new(&fixture.source_root, &inventory, Some(&status))
                .definitions(&query("ПолучитьСерию", &[], 10));

        let ProviderOutcome::Unavailable(diagnostic) = outcome else {
            panic!("unbound ready index evidence must be unavailable");
        };
        assert_eq!(diagnostic.code, "bsl_definition_freshness_unverified");
        assert_eq!(
            diagnostic.materiality,
            crate::domain::discovery::MissingCheckMateriality::Material
        );
    }

    #[test]
    fn ready_index_empty_result_is_not_negative_complete_without_snapshot_proof() {
        let fixture = Fixture::new("definition-freshness-empty");
        let inventory = inventory(vec![fixture.write_source(MODULE_PATH, BSL)]);
        let db_path = fixture.root.join("index.db");
        create_empty_index(&db_path);
        let status = ready_status(&fixture.source_root, &db_path);

        let outcome =
            ExistingIndexDefinitionProvider::new(&fixture.source_root, &inventory, Some(&status))
                .definitions(&query("НесуществующийМетод", &[], 10));

        let ProviderOutcome::Unavailable(diagnostic) = outcome else {
            panic!("empty unbound ready index result must be unavailable");
        };
        assert_eq!(diagnostic.code, "bsl_definition_freshness_unverified");
    }

    #[test]
    fn ready_index_evidence_bound_is_unavailable_without_snapshot_proof() {
        let fixture = Fixture::new("definition-resource-bound");
        let inventory = inventory(vec![fixture.write_source(MODULE_PATH, BSL)]);
        let db_path = fixture.root.join("index.db");
        create_index(&db_path, MODULE_PATH, 5, 7);
        let status = ready_status(&fixture.source_root, &db_path);

        let outcome =
            ExistingIndexDefinitionProvider::new(&fixture.source_root, &inventory, Some(&status))
                .definitions(&query("ПолучитьСерию", &[], 0));

        let ProviderOutcome::Unavailable(diagnostic) = outcome else {
            panic!("an evidence bound must not override unverified freshness");
        };
        assert_eq!(diagnostic.code, "bsl_definition_freshness_unverified");
    }

    #[test]
    fn out_of_root_or_malformed_index_rows_are_contract_violations() {
        let fixture = Fixture::new("definition-invalid");
        let inventory = inventory(vec![fixture.write_source(MODULE_PATH, BSL)]);

        let escaped_db = fixture.root.join("escaped.db");
        create_index(&escaped_db, "../outside.bsl", 5, 7);
        let escaped_status = ready_status(&fixture.source_root, &escaped_db);
        let escaped = ExistingIndexDefinitionProvider::new(
            &fixture.source_root,
            &inventory,
            Some(&escaped_status),
        )
        .definitions(&query("ПолучитьСерию", &[], 10));
        assert!(matches!(escaped, ProviderOutcome::ContractViolation(_)));

        let malformed_db = fixture.root.join("malformed.db");
        Connection::open(&malformed_db).unwrap();
        let malformed_status = ready_status(&fixture.source_root, &malformed_db);
        let malformed = ExistingIndexDefinitionProvider::new(
            &fixture.source_root,
            &inventory,
            Some(&malformed_status),
        )
        .definitions(&query("ПолучитьСерию", &[], 10));
        assert!(matches!(malformed, ProviderOutcome::ContractViolation(_)));

        let identity_db = fixture.root.join("identity.db");
        create_index(&identity_db, MODULE_PATH, 5, 7);
        Connection::open(&identity_db)
            .unwrap()
            .execute("UPDATE modules SET category = 'Document'", ())
            .unwrap();
        let identity_status = ready_status(&fixture.source_root, &identity_db);
        let identity = ExistingIndexDefinitionProvider::new(
            &fixture.source_root,
            &inventory,
            Some(&identity_status),
        )
        .definitions(&query("ПолучитьСерию", &[], 10));
        assert!(matches!(identity, ProviderOutcome::ContractViolation(_)));
    }

    #[test]
    fn stale_index_path_or_changed_source_bytes_are_unavailable_not_negative_complete() {
        let fixture = Fixture::new("definition-stale-row");
        let source = fixture.write_source(MODULE_PATH, BSL);
        let inventory = inventory(vec![source]);

        let missing_db = fixture.root.join("missing-row.db");
        create_index(&missing_db, "CommonModules/Нет/Ext/Module.bsl", 1, 2);
        let missing_status = ready_status(&fixture.source_root, &missing_db);
        let missing = ExistingIndexDefinitionProvider::new(
            &fixture.source_root,
            &inventory,
            Some(&missing_status),
        )
        .definitions(&query("ПолучитьСерию", &[], 10));
        assert!(matches!(missing, ProviderOutcome::Unavailable(_)));

        let changed_db = fixture.root.join("changed.db");
        create_index(&changed_db, MODULE_PATH, 5, 7);
        fs::write(fixture.source_root.join(MODULE_PATH), b"changed").unwrap();
        let changed_status = ready_status(&fixture.source_root, &changed_db);
        let changed = ExistingIndexDefinitionProvider::new(
            &fixture.source_root,
            &inventory,
            Some(&changed_status),
        )
        .definitions(&query("ПолучитьСерию", &[], 10));
        assert!(matches!(changed, ProviderOutcome::Unavailable(_)));
    }

    #[test]
    fn stale_index_line_range_is_unavailable_not_definition_evidence() {
        let fixture = Fixture::new("definition-stale-lines");
        let inventory = inventory(vec![fixture.write_source(MODULE_PATH, BSL)]);
        let db_path = fixture.root.join("stale-lines.db");
        create_index(&db_path, MODULE_PATH, 4, 7);
        let status = ready_status(&fixture.source_root, &db_path);

        let outcome =
            ExistingIndexDefinitionProvider::new(&fixture.source_root, &inventory, Some(&status))
                .definitions(&query("ПолучитьСерию", &[], 10));

        assert!(matches!(outcome, ProviderOutcome::Unavailable(_)));
    }

    #[test]
    fn ready_index_inventory_bound_is_unavailable_without_snapshot_proof() {
        let fixture = Fixture::new("definition-bounded-inventory");
        let source = fixture.write_source(MODULE_PATH, BSL);
        let inventory = SourceInventory {
            files: vec![source.clone()],
            coverage: ProviderCoverage::new(2, 1, source.bytes.len() as u64, 1),
        };
        let db_path = fixture.root.join("empty.db");
        create_empty_index(&db_path);
        let status = ready_status(&fixture.source_root, &db_path);

        let outcome =
            ExistingIndexDefinitionProvider::new(&fixture.source_root, &inventory, Some(&status))
                .definitions(&query("НесуществующийМетод", &[], 10));

        let ProviderOutcome::Unavailable(diagnostic) = outcome else {
            panic!("an inventory bound must not override unverified freshness");
        };
        assert_eq!(diagnostic.code, "bsl_definition_freshness_unverified");
    }

    #[test]
    fn runtime_flow_gap_is_explicit_and_material() {
        let outcome = UnavailableRuntimeFlowProvider.runtime_flow(&query("Найти", &[], 10));

        let ProviderOutcome::Unavailable(diagnostic) = outcome else {
            panic!("expected unavailable runtime flow");
        };
        assert_eq!(diagnostic.code, "runtime_flow_unavailable");
        assert_eq!(
            diagnostic.materiality,
            crate::domain::discovery::MissingCheckMateriality::Material
        );
    }

    fn query<'a>(
        task: &'a str,
        search_terms: &'a [String],
        max_evidence: u16,
    ) -> DiscoveryQuery<'a> {
        DiscoveryQuery::new(
            task,
            &[],
            search_terms,
            &[],
            DiscoveryQueryLimits {
                max_files: 100,
                max_bytes: 1_000_000,
                max_evidence,
                max_candidates: 10,
                max_graph_depth: 3,
            },
        )
    }

    fn inventory(files: Vec<SourceFile>) -> SourceInventory {
        let count = u32::try_from(files.len()).unwrap();
        let bytes = files.iter().map(|file| file.bytes.len() as u64).sum();
        SourceInventory {
            files,
            coverage: ProviderCoverage::new(count, count, bytes, count),
        }
    }

    fn source_file(path: &str, bytes: &[u8]) -> SourceFile {
        SourceFile {
            relative_path: PortableRelativePath::parse_str(path).unwrap(),
            bytes: bytes.to_vec().into(),
            raw_hash: ContentHash::sha256(bytes),
        }
    }

    fn artifact(value: &str) -> ArtifactId {
        ArtifactId::parse(value).unwrap()
    }

    fn path(value: &str) -> PortableRelativePath {
        PortableRelativePath::parse(Path::new(value)).unwrap()
    }

    struct Fixture {
        root: PathBuf,
        source_root: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!("unica-bsl-{name}-{nanos}"));
            let source_root = root.join("src");
            fs::create_dir_all(&source_root).unwrap();
            let source_root = fs::canonicalize(source_root).unwrap();
            Self { root, source_root }
        }

        fn write_source(&self, path: &str, bytes: &[u8]) -> SourceFile {
            let full_path = self.source_root.join(path);
            fs::create_dir_all(full_path.parent().unwrap()).unwrap();
            fs::write(full_path, bytes).unwrap();
            source_file(path, bytes)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn ready_status(source_root: &Path, db_path: &Path) -> BslIndexStatus {
        BslIndexStatus {
            status: "ready".to_string(),
            source_root: Some(source_root.display().to_string()),
            db_path: Some(db_path.display().to_string()),
            message: None,
            updated_at: 0,
            last_run: None,
        }
    }

    fn create_index(db_path: &Path, module_path: &str, line: i64, end_line: i64) {
        let connection = Connection::open(db_path).unwrap();
        create_index_schema(&connection);
        connection
            .execute(
                "INSERT INTO modules (id, rel_path, category, object_name, module_type)
                 VALUES (1, ?1, 'CommonModule', 'Серии', 'Module')",
                (module_path,),
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO methods
                 (id, module_id, name, type, is_export, line, end_line, params)
                 VALUES (1, 1, 'ПолучитьСерию', 'Function', 0, ?1, ?2, 'Код')",
                (line, end_line),
            )
            .unwrap();
    }

    fn create_empty_index(db_path: &Path) {
        let connection = Connection::open(db_path).unwrap();
        create_index_schema(&connection);
    }

    fn create_index_schema(connection: &Connection) {
        connection
            .execute_batch(
                "CREATE TABLE modules (
                    id INTEGER PRIMARY KEY,
                    rel_path TEXT NOT NULL,
                    category TEXT,
                    object_name TEXT,
                    module_type TEXT
                );
                CREATE TABLE methods (
                    id INTEGER PRIMARY KEY,
                    module_id INTEGER NOT NULL,
                    name TEXT NOT NULL,
                    type TEXT NOT NULL,
                    is_export INTEGER NOT NULL,
                    line INTEGER NOT NULL,
                    end_line INTEGER NOT NULL,
                    params TEXT
                );",
            )
            .unwrap();
    }
}
