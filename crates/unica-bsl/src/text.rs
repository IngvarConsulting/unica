//! Pure BSL byte planning, repeat proof and patch facts.
use crate::input::*;
use diffy::{apply, DiffOptions, Patch};
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use unica_address::address::QualifiedAddress;
use unica_address::source_target::TargetKind;
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind};
use unica_xml_format::text::{
    resolve_observed_line_ending, LineEnding, LineEndingProfile, SourceTextSnapshot,
};
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodePatchData {
    pub source_set: String,
    pub metadata_path: String,
    pub target_kind: TargetKind,
    pub pre_hash: String,
    pub post_hash: String,
    pub no_op: bool,
    pub changed_ranges: Vec<ChangedRange>,
    pub diff: String,
    pub affected_target: AffectedTarget,
    pub validation: SourceValidation,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedRange {
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AffectedTarget {
    pub source_set: String,
    pub metadata_path: String,
    pub target_kind: TargetKind,
    pub owner: String,
    pub module_role: String,
    pub raw_hash: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceValidation {
    pub kind: ValidationKind,
    pub status: ValidationStatus,
    pub validated_post_hash: String,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ValidationKind {
    #[serde(rename = "bsl-analyzer-parser")]
    BslAnalyzerParser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ValidationStatus {
    Passed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationDiagnostic {
    pub code: &'static str,
    pub message: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Before,
    After,
}

impl Position {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "before" => Ok(Self::Before),
            "after" => Ok(Self::After),
            _ => Err("position must be before or after".to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selector {
    Method(String),
    Anchor(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectorResolutionCause {
    ZeroMatches,
    MultipleMatches,
    InvalidSource,
}

#[derive(Debug)]
pub struct SelectorResolutionError {
    pub cause: SelectorResolutionCause,
    pub legacy_message: String,
}

impl SelectorResolutionError {
    pub fn zero_matches(selector: &Selector) -> Self {
        Self::match_count(selector, 0, SelectorResolutionCause::ZeroMatches)
    }

    pub fn multiple_matches(selector: &Selector, count: usize) -> Self {
        Self::match_count(selector, count, SelectorResolutionCause::MultipleMatches)
    }

    pub fn match_count(selector: &Selector, count: usize, cause: SelectorResolutionCause) -> Self {
        let name = match selector {
            Selector::Method(_) => "method",
            Selector::Anchor(_) => "anchor",
        };
        Self {
            cause,
            legacy_message: format!(
                "{name} selector must match exactly once; matched {count} times"
            ),
        }
    }

    pub fn invalid_source(message: impl Into<String>) -> Self {
        Self {
            cause: SelectorResolutionCause::InvalidSource,
            legacy_message: message.into(),
        }
    }

    pub const fn cause(&self) -> SelectorResolutionCause {
        self.cause
    }
}

impl std::fmt::Display for SelectorResolutionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.legacy_message.fmt(formatter)
    }
}

impl Selector {
    /// `insert` names a place only when it has one to name. An absent selector
    /// is not an error: it means the end of the module, which is the one place
    /// every module has, including a module that holds no method yet.
    pub fn parse_optional(args: &Map<String, Value>) -> Result<Option<Self>, String> {
        if !args.contains_key("selector") {
            return Ok(None);
        }
        Self::parse(args).map(Some)
    }

    pub fn parse(args: &Map<String, Value>) -> Result<Self, String> {
        let selector = args
            .get("selector")
            .and_then(Value::as_object)
            .ok_or_else(|| "selector must be an object".to_string())?;
        if selector.len() != 1 {
            return Err("selector must contain exactly one of method or anchor".to_string());
        }
        match (
            selector.get("method").and_then(Value::as_str),
            selector.get("anchor").and_then(Value::as_str),
        ) {
            (Some(name), None) if !name.is_empty() => Ok(Self::Method(name.to_string())),
            (None, Some(anchor)) if !anchor.is_empty() => {
                Ok(Self::Anchor(canonicalize_eol(anchor)))
            }
            _ => Err("selector must contain exactly one non-empty method or anchor".to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeadingSeparator {
    None,
    LocalEol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchOperation {
    Insert,
    Replace,
}

pub struct StagedCodePostimage {
    pub after: Vec<u8>,
    pub no_op: bool,
}

pub fn plan_code_postimage(
    before: &[u8],
    operation: &CodePlanOperation,
    op_index: usize,
) -> Result<StagedCodePostimage, ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let text_path = format!("ops[{op_index}].args.text");
    let snapshot = SourceTextSnapshot::from_bytes(before).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "original BSL module is not a supported text snapshot",
        )
        .at_path(&at_path)
    })?;
    let indexed = analyze_module(snapshot.decoded_text()).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "original BSL module cannot be analyzed",
        )
        .at_path(&at_path)
    })?;
    if !indexed.diagnostics.is_empty() {
        return Err(syntax_refusal(
            ApplyPlanErrorKind::InvalidSource,
            "original BSL module does not parse",
            snapshot.decoded_text(),
            &indexed.diagnostics,
            None,
        )
        .at_path(&at_path));
    }
    // Адрес и охват замены разбирают текст замены; он один на весь план.
    let replace_target = match operation {
        CodePlanOperation::Replace(args) => {
            Some(replacement_target(&args.at, &args.text, &text_path)?)
        }
        CodePlanOperation::Insert(_) => None,
    };
    // The bytes the patch writes and their line ending: the sent text lies
    // inside them, with its line endings normalized.
    let written: (std::ops::Range<usize>, LineEnding);
    let (selector, content, site, no_op, after) = match operation {
        CodePlanOperation::Insert(args) => {
            if args.text.is_empty() {
                return Err(bad_code_arg(&text_path, "text must be a non-empty string"));
            }
            let selector = args.selector.as_ref().map(legacy_selector);
            // Метод, названный адресом, ищется так же, но отказ называет `at`.
            let addressed_method = matches!(
                code_projection(&args.at),
                Some((_, CodeProjection::Method(_)))
            );
            let site = match (&selector, args.position) {
                (Some(selector), Some(position)) => locate_selector_typed(
                    &snapshot,
                    legacy_position(position),
                    selector,
                    &indexed.methods,
                )
                .map_err(|error| {
                    if addressed_method {
                        addressed_selector_error(error, &at_path)
                    } else {
                        code_selector_error(error, op_index, selector)
                    }
                })?,
                (Some(_), None) => {
                    return Err(bad_code_arg(
                        format!("ops[{op_index}].args.position"),
                        "position is required when selector is present",
                    ))
                }
                (None, Some(_)) => {
                    return Err(bad_code_arg(
                        format!("ops[{op_index}].args.position"),
                        "position is unavailable without selector",
                    ))
                }
                (None, None) => locate_module_tail(&snapshot).map_err(|_| {
                    ApplyPlanError::new(
                        ApplyPlanErrorKind::InvalidSource,
                        "module tail cannot be resolved from the observed source",
                    )
                    .at_path(&at_path)
                })?,
            };
            let (insertion, no_op) =
                prepare_insertion(&snapshot, site, &args.text, &indexed.methods);
            let mut after = snapshot.raw().to_vec();
            if !no_op {
                after.splice(site.offset..site.offset, insertion.iter().copied());
            }
            written = (site.offset..site.offset + insertion.len(), site.eol);
            (
                selector,
                args.text.as_str(),
                PatchSite::Insertion(site),
                no_op,
                after,
            )
        }
        CodePlanOperation::Replace(args) => {
            if args.text.is_empty() {
                return Err(bad_code_arg(&text_path, "text must be a non-empty string"));
            }
            let selector = args.anchor.clone().map(Selector::Anchor);
            let target = replace_target
                .as_ref()
                .expect("a replacement resolves its target above");
            let site = match &selector {
                Some(selector) => {
                    locate_replacement_typed(&snapshot, selector, &indexed.methods)
                        .map_err(|error| code_selector_error(error, op_index, selector))?
                }
                None => {
                    locate_addressed_replacement(&snapshot, target, &indexed.methods, &at_path)?
                }
            };
            let replacement = site.replacement(&args.text);
            let no_op =
                replacement_is_present(snapshot.raw(), site, &replacement, |found, methods| {
                    relocate_replaced_method(found, methods, target)
                });
            let mut after = snapshot.raw().to_vec();
            if !no_op {
                after.splice(site.start..site.end, replacement.iter().copied());
            }
            written = (site.start..site.start + replacement.len(), site.eol);
            (
                selector,
                args.text.as_str(),
                PatchSite::Replacement(site),
                no_op,
                after,
            )
        }
    };
    let postimage = std::str::from_utf8(&after).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::Postcondition,
            "patched BSL module is not UTF-8",
        )
        .at_path(&text_path)
    })?;
    let post = analyze_module(postimage).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::Postcondition,
            "patched BSL module cannot be analyzed",
        )
        .at_path(&text_path)
    })?;
    if !post.diagnostics.is_empty() {
        // The source parsed before the patch: the sent text broke it.
        let (range, eol) = written;
        let sent = SentText::locate(postimage, range, content, eol);
        return Err(syntax_refusal(
            ApplyPlanErrorKind::BadValue,
            "text makes the BSL module unparsable",
            postimage,
            &post.diagnostics,
            sent,
        )
        .at_path(&text_path));
    }
    if let CodePlanOperation::Replace(args) = operation {
        if replace_target
            .as_ref()
            .is_some_and(|target| replaces_whole_method(&target.at))
        {
            reject_redeclared_methods(&args.text, &indexed.methods, &post.methods).map_err(
                |message| {
                    ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(&text_path)
                },
            )?;
        }
    }
    let repeat = match operation {
        CodePlanOperation::Replace(args) if args.anchor.is_none() => {
            let repeat_snapshot = SourceTextSnapshot::from_bytes(&after)
                .map_err(|_| bad_code_arg(&text_path, "invalid patched snapshot"))?;
            let target = replace_target
                .as_ref()
                .expect("a replacement resolves its target above");
            match locate_addressed_replacement(&repeat_snapshot, target, &post.methods, &at_path) {
                Err(error) if error.kind() == ApplyPlanErrorKind::NotFound => Ok(()),
                Err(_) => Err("patched BSL address became ambiguous or invalid".to_string()),
                Ok(repeated) => {
                    let bytes = repeated.replacement(content);
                    if replacement_is_present(
                        repeat_snapshot.raw(),
                        repeated,
                        &bytes,
                        |found, methods| relocate_replaced_method(found, methods, target),
                    ) {
                        Ok(())
                    } else {
                        Err("addressed replacement would change bytes again".to_string())
                    }
                }
            }
        }
        _ => prove_repeat_is_noop_parts(postimage, selector.as_ref(), content, site, &post.methods),
    };
    repeat.map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::Postcondition,
            "patched BSL module does not prove repeat-noop behavior",
        )
        .at_path(&text_path)
    })?;
    Ok(StagedCodePostimage { after, no_op })
}

/// The sent text inside the patched module: which module line holds each
/// of its lines, and the column its first line starts at.
struct SentText<'a> {
    module: &'a str,
    /// Module line (one-based) to text line (zero-based).
    lines: Vec<(usize, usize)>,
    /// Column (one-based) of the text's first character on its first line.
    first_column: usize,
    /// Characters on the text's last line, for an error that runs past it.
    last_line_chars: usize,
}

impl<'a> SentText<'a> {
    /// Finds the sent text in the bytes the patch wrote. A replacement
    /// writes it whole; an insertion may add blank lines before, after and
    /// between its methods, so its lines are matched in order.
    fn locate(
        module: &'a str,
        written: std::ops::Range<usize>,
        content: &str,
        eol: LineEnding,
    ) -> Option<Self> {
        let text = canonicalize_eol(content);
        let text_lines = text
            .strip_suffix('\n')
            .unwrap_or(&text)
            .split('\n')
            .collect::<Vec<_>>();
        let last_line_chars = text_lines.last()?.chars().count();
        let block = module.get(written.clone())?;
        let normalized = normalized_replacement(content, eol, false);
        if let Some(found) = block.find(std::str::from_utf8(&normalized).ok()?) {
            let start = written.start + found;
            let (first_line, first_column) = line_column(module, start).ok()?;
            return Some(Self {
                module,
                lines: (0..text_lines.len())
                    .map(|line| (first_line + line, line))
                    .collect(),
                first_column,
                last_line_chars,
            });
        }
        let (block_line, block_column) = line_column(module, written.start).ok()?;
        if block_column != 1 {
            return None;
        }
        let mut lines = Vec::with_capacity(text_lines.len());
        let mut next = 0;
        for (index, line) in block.split('\n').enumerate() {
            let line = line.strip_suffix('\r').unwrap_or(line);
            match text_lines.get(next) {
                Some(expected) if line == *expected => {
                    lines.push((block_line + index, next));
                    next += 1;
                }
                // A blank line the insertion added around the text.
                _ if line.trim().is_empty() => {}
                _ => return None,
            }
        }
        (next == text_lines.len()).then_some(Self {
            module,
            lines,
            first_column: 1,
            last_line_chars,
        })
    }

    /// The zero-based line and column of module byte `offset` within the
    /// sent text, if it lies there.
    fn position(&self, offset: usize) -> Option<(usize, usize)> {
        let (line, column) = line_column(self.module, offset).ok()?;
        let text_line = self
            .lines
            .iter()
            .find(|(module_line, _)| *module_line == line)
            .map(|(_, text_line)| *text_line)?;
        let start = if text_line == 0 { self.first_column } else { 1 };
        Some((text_line, column.checked_sub(start)?))
    }

    /// The end of the text, for an error that starts in it and ends past it.
    fn end(&self) -> (usize, usize) {
        let last = self.lines.len() - 1;
        (last, self.last_line_chars)
    }
}

/// A refusal for a module that does not parse. Its message names the first
/// error; every error follows as a diagnostic of its own, with its place in
/// the module and, for one that starts inside the sent text, its place in
/// that text (INV.SOURCE.CODE-TEXT-SYNTAX-ERRORS).
fn syntax_refusal(
    kind: ApplyPlanErrorKind,
    cause: &str,
    module: &str,
    errors: &[ValidationDiagnostic],
    sent: Option<SentText<'_>>,
) -> ApplyPlanError {
    // The parser counts a byte order mark as a character of the first line;
    // the editor does not show it.
    let bom = module.starts_with('\u{feff}');
    let column = |line: usize, column: usize| {
        if bom && line == 1 {
            column.saturating_sub(1).max(1)
        } else {
            column
        }
    };
    let first = &errors[0];
    let in_text = sent
        .as_ref()
        .and_then(|sent| sent.position(first.start_byte))
        .map(|(line, _)| format!(" (line {} of text)", line + 1))
        .unwrap_or_default();
    let total = match errors.len() {
        1 => String::new(),
        count => format!("; {count} syntax errors in all"),
    };
    let message = format!(
        "{cause}: {} at line {}, column {}{in_text}{total}",
        first.message,
        first.start_line,
        column(first.start_line, first.start_column)
    );
    let diagnostics = errors
        .iter()
        .map(|error| {
            let mut diagnostic = serde_json::json!({
                "code": error.code,
                "severity": "error",
                "message": error.message,
                "focus": {
                    "kind": "sourceRange",
                    "range": {
                        "startLine": error.start_line - 1,
                        "startColumn": column(error.start_line, error.start_column) - 1,
                        "endLine": error.end_line - 1,
                        "endColumn": column(error.end_line, error.end_column) - 1,
                    },
                },
            });
            if let Some(sent) = &sent {
                if let Some(from) = sent.position(error.start_byte) {
                    let to = sent.position(error.end_byte).unwrap_or_else(|| sent.end());
                    diagnostic["textRange"] = serde_json::json!({
                        "startLine": from.0,
                        "startColumn": from.1,
                        "endLine": to.0,
                        "endColumn": to.1,
                    });
                }
            }
            diagnostic
        })
        .collect();
    ApplyPlanError::new(kind, message).with_diagnostics(diagnostics)
}

/// Адрес замены и её охват.
pub struct ReplacementTarget {
    pub at: QualifiedAddress,
    /// Замена начинается с документирующего комментария над методом.
    pub with_doc_comment: bool,
}

/// `…Method.<Имя>.Body` с текстом, который сам объявляет метод, — это текст,
/// показанный `view` по тому же адресу: документирующий комментарий,
/// директива, объявление, тело и окончание. Он заменяет метод вместе с
/// комментарием над ним. Текст без объявления по-прежнему заменяет только
/// строки между объявлением и окончанием: тело метода в BSL не может
/// объявить другой метод, поэтому формы не смешиваются. Текст с объявлением,
/// который не разбирается, отклоняется с указанием на сам текст: иначе он
/// ушёл бы в тело метода и отказ назвал бы испорченный модуль.
pub fn replacement_target(
    at: &QualifiedAddress,
    text: &str,
    text_path: &str,
) -> Result<ReplacementTarget, ApplyPlanError> {
    let unchanged = || ReplacementTarget {
        at: at.clone(),
        with_doc_comment: false,
    };
    if !matches!(
        code_projection(at),
        Some((_, CodeProjection::MethodBody(_)))
    ) {
        return Ok(unchanged());
    }
    // Условие препроцессора вокруг метода в его текст не входит; обёртка
    // в тексте замены удвоила бы ту, что уже стоит в модуле.
    if method_head(text).is_some_and(|head| head.after_preprocessor) {
        return Err(bad_code_arg(
            text_path,
            "Method Body text does not include the preprocessor around the method; pass the method text without #If/#Region lines, or change the wrapper with code.insert or code.replace on the module address with selector.anchor",
        ));
    }
    if !declares_method(text) {
        return Ok(unchanged());
    }
    // A method text with syntax errors still replaces the whole method: the
    // patched module then names each error by its place.
    let parsed = analyze_module(text)
        .is_ok_and(|analysis| !analysis.diagnostics.is_empty() || !analysis.methods.is_empty());
    if !parsed {
        return Err(bad_code_arg(
            text_path,
            "text declares a method but is not valid BSL; pass the method text as view returned it, with the edit applied",
        ));
    }
    let method_at = at
        .to_string()
        .strip_suffix(".Body")
        .and_then(|method| QualifiedAddress::parse(method).ok());
    Ok(method_at.map_or_else(unchanged, |at| ReplacementTarget {
        at,
        with_doc_comment: true,
    }))
}

/// Объявление метода в тексте: первая значащая строка после комментариев,
/// аннотаций, пустых строк и инструкций препроцессора.
pub struct MethodHead {
    /// Перед объявлением стоят инструкции препроцессора.
    pub after_preprocessor: bool,
}

pub fn method_head(text: &str) -> Option<MethodHead> {
    const KEYWORDS: &[&str] = &[
        "процедура",
        "функция",
        "procedure",
        "function",
        "асинх",
        "async",
    ];
    let mut after_preprocessor = false;
    let line = text.lines().map(str::trim).find(|line| {
        if line.starts_with('#') {
            after_preprocessor = true;
            return false;
        }
        !line.is_empty() && !line.starts_with("//") && !line.starts_with('&')
    })?;
    let lower = line.to_lowercase();
    KEYWORDS
        .iter()
        .any(|keyword| {
            lower
                .strip_prefix(keyword)
                .is_some_and(|rest| rest.starts_with(|c: char| c.is_whitespace()))
        })
        .then_some(MethodHead { after_preprocessor })
}

/// Текст начинается объявлением метода (без препроцессора перед ним).
pub fn declares_method(text: &str) -> bool {
    method_head(text).is_some_and(|head| !head.after_preprocessor)
}

/// Начало документирующего комментария: подряд идущие строки `//`
/// непосредственно над строкой `line_start`; пустая строка его обрывает.
pub fn doc_comment_start(text: &str, line_start: usize) -> usize {
    let mut start = line_start;
    while start > 0 {
        let previous = safe_line_start(text, start - 1);
        if previous >= start || !text[previous..start].trim_start().starts_with("//") {
            break;
        }
        start = previous;
    }
    start
}

/// Участок метода, рядом с которым встаёт новый код: документирующий
/// комментарий, аннотации и директивы, сам метод и условие препроцессора,
/// которое обрамляет только этот метод. Условие, где есть и другой код,
/// не охватывается: вставка встаёт внутрь него, рядом с методом, иначе
/// новый код вышел бы из своего контекста компиляции или изменил бы чужой.
pub fn method_unit(text: &str, method: &Method) -> (usize, usize) {
    let mut start = doc_comment_start(text, safe_line_start(text, method.start));
    let mut end = line_end(text, method.end);
    loop {
        if start == 0 || end >= text.len() {
            break;
        }
        let above = safe_line_start(text, start - 1);
        let below = line_end(text, end);
        if above >= start
            || !preprocessor_line(&text[above..start], &["#если", "#if"])
            || !preprocessor_line(&text[end..below], &["#конецесли", "#endif"])
        {
            break;
        }
        start = doc_comment_start(text, above);
        end = below;
    }
    (start, end)
}

pub fn preprocessor_line(line: &str, keywords: &[&str]) -> bool {
    let line = line.trim().to_lowercase();
    keywords.iter().any(|keyword| {
        line.strip_prefix(keyword)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(|c: char| !c.is_alphanumeric()))
    })
}

pub fn locate_addressed_replacement(
    snapshot: &SourceTextSnapshot,
    target: &ReplacementTarget,
    methods: &[Method],
    path: &str,
) -> Result<ReplacementSite, ApplyPlanError> {
    let at = &target.at;
    let (_, projection) = code_projection(at)
        .ok_or_else(|| bad_code_arg(path, "at does not select a writable BSL node"))?;
    reject_lone_cr_line_endings(snapshot).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "unsupported BSL line endings",
        )
        .at_path(path)
    })?;
    let text = snapshot.decoded_text();
    let (start, end, body, leading_eol) = match projection {
        CodeProjection::ModuleBody => {
            (text.len() - snapshot.text().len(), text.len(), false, false)
        }
        CodeProjection::Method(name) => {
            let method = addressed_method(methods, &name, path)?;
            let line_start = safe_line_start(text, method.start);
            let line_end = line_end(text, method.end);
            let start = if text[line_start..method.start].trim().is_empty() {
                if target.with_doc_comment {
                    doc_comment_start(text, line_start)
                } else {
                    line_start
                }
            } else {
                method.start
            };
            // A full Method.Body roundtrip includes an inline closing comment,
            // but executable code on that line belongs outside this method.
            let closing_tail = text[method.end..line_end].trim();
            let end = if closing_tail.is_empty()
                || (target.with_doc_comment && closing_tail.starts_with("//"))
            {
                line_end
            } else {
                method.end
            };
            (start, end, false, false)
        }
        CodeProjection::MethodBody(name) => {
            let method = addressed_method(methods, &name, path)?;
            let (declaration, closing) = method.body_bounds.ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidSource,
                    "method body has no parser-proven boundaries",
                )
                .at_path(path)
            })?;
            let header_end = line_end(text, declaration);
            let closing_line = safe_line_start(text, closing);
            let body_end = if text[closing_line..closing].trim().is_empty() {
                closing_line
            } else {
                closing
            };
            let header_tail = text[declaration..header_end.min(closing)].trim();
            if header_end <= closing_line
                && (header_tail.is_empty() || header_tail.starts_with("//"))
            {
                (header_end, body_end, true, false)
            } else {
                (declaration, body_end, true, true)
            }
        }
    };
    let eol = resolve_observed_line_ending(
        snapshot,
        local_line_ending_at(text, start, Position::Before),
    )
    .map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "BSL line ending cannot be resolved",
        )
        .at_path(path)
    })?;
    let trailing_eol = body
        || text
            .get(start..end)
            .is_some_and(|span| span.ends_with('\n'));
    Ok(ReplacementSite {
        start,
        end,
        eol,
        trailing_eol,
        leading_eol,
    })
}

pub fn addressed_method<'a>(
    methods: &'a [Method],
    name: &str,
    path: &str,
) -> Result<&'a Method, ApplyPlanError> {
    let folded_name = name.to_lowercase();
    let selector = Selector::Method(name.to_owned());
    let mut found = methods
        .iter()
        .filter(|method| method.name.to_lowercase() == folded_name);
    let method = found.next().ok_or_else(|| {
        addressed_selector_error(SelectorResolutionError::zero_matches(&selector), path)
    })?;
    if found.next().is_some() {
        return Err(addressed_selector_error(
            SelectorResolutionError::multiple_matches(&selector, 2),
            path,
        ));
    }
    Ok(method)
}

pub fn addressed_selector_error(error: SelectorResolutionError, path: &str) -> ApplyPlanError {
    let kind = match error.cause() {
        SelectorResolutionCause::ZeroMatches => ApplyPlanErrorKind::NotFound,
        SelectorResolutionCause::MultipleMatches => ApplyPlanErrorKind::InvalidState,
        SelectorResolutionCause::InvalidSource => ApplyPlanErrorKind::InvalidSource,
    };
    ApplyPlanError::new(kind, "BSL address cannot identify one stable writable node").at_path(path)
}

pub fn legacy_selector(selector: &CodeSelector) -> Selector {
    match selector {
        CodeSelector::Method(value) => Selector::Method(value.clone()),
        CodeSelector::Anchor(value) => Selector::Anchor(value.clone()),
    }
}

pub const fn legacy_position(position: CodePosition) -> Position {
    match position {
        CodePosition::Before => Position::Before,
        CodePosition::After => Position::After,
    }
}

pub fn code_selector_error(
    error: SelectorResolutionError,
    op_index: usize,
    selector: &Selector,
) -> ApplyPlanError {
    let kind = match error.cause() {
        SelectorResolutionCause::ZeroMatches => ApplyPlanErrorKind::NotFound,
        SelectorResolutionCause::MultipleMatches => ApplyPlanErrorKind::InvalidState,
        SelectorResolutionCause::InvalidSource => ApplyPlanErrorKind::InvalidSource,
    };
    let field = match selector {
        Selector::Method(_) => "method",
        Selector::Anchor(_) => "anchor",
    };
    ApplyPlanError::new(kind, "code selector cannot identify one stable BSL site")
        .at_path(format!("ops[{op_index}].args.selector.{field}"))
}

impl PatchOperation {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "insert" => Ok(Self::Insert),
            "replace" => Ok(Self::Replace),
            _ => Err("unica.code.patch supports operation `insert` or `replace`".to_string()),
        }
    }
}

/// The span a replacement overwrites, so an edit costs the selected method or
/// anchor rather than a rewrite of the whole module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplacementSite {
    pub start: usize,
    pub end: usize,
    pub eol: LineEnding,
    pub trailing_eol: bool,
    pub leading_eol: bool,
}

impl ReplacementSite {
    pub fn replacement(self, content: &str) -> Vec<u8> {
        let mut bytes = normalized_replacement(content, self.eol, self.trailing_eol);
        if self.leading_eol {
            bytes.splice(0..0, self.eol.as_str().bytes());
        }
        bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchSite {
    Insertion(InsertionSite),
    Replacement(ReplacementSite),
}

impl PatchSite {
    /// First byte the patch writes, in postimage coordinates.
    pub fn changed_start(self) -> usize {
        match self {
            Self::Insertion(site) => site.offset,
            Self::Replacement(site) => site.start,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsertionSite {
    pub offset: usize,
    /// Конец головы метода, перед которым вставка `before`: строки
    /// комментария, вставленные над методом, становятся его документирующим
    /// комментарием, и голова растёт вверх. Повтор ищет вставку на любой
    /// границе строк от `offset` до этой точки.
    pub head_end: usize,
    pub position: Position,
    pub eol: LineEnding,
    pub leading_separator: LeadingSeparator,
}

pub fn patch_data(
    plan: &CodeTextPatchPlan,
    postimage: &str,
    post_hash: String,
    validation: SourceValidation,
    identity: CodePatchIdentity<'_>,
) -> Result<CodePatchData, String> {
    let changed_ranges = if plan.no_op {
        Vec::new()
    } else {
        let start = plan.site.changed_start();
        let end = start + plan.insertion.len();
        let (start_line, start_column) = line_column(postimage, start)?;
        let (end_line, end_column) = line_column(postimage, end)?;
        vec![ChangedRange {
            start_byte: start,
            end_byte: end,
            start_line,
            start_column,
            end_line,
            end_column,
        }]
    };
    let diff = if plan.no_op {
        String::new()
    } else {
        let preimage = std::str::from_utf8(&plan.before)
            .map_err(|_| "original BSL module must be UTF-8".to_string())?;
        unified_diff(identity.metadata_path, preimage, postimage)?
    };
    Ok(CodePatchData {
        source_set: identity.source_set.to_string(),
        metadata_path: identity.metadata_path.to_string(),
        target_kind: identity.target_kind,
        pre_hash: hash(&plan.before),
        post_hash: post_hash.clone(),
        no_op: plan.no_op,
        changed_ranges,
        diff,
        affected_target: AffectedTarget {
            source_set: identity.source_set.to_string(),
            metadata_path: identity.metadata_path.to_string(),
            target_kind: identity.target_kind,
            owner: identity.owner.to_string(),
            module_role: identity.module_role.to_string(),
            raw_hash: post_hash,
        },
        validation,
    })
}

#[cfg(test)]
pub fn locate_insertion(text: &str, args: &Map<String, Value>) -> Result<InsertionSite, String> {
    if string_arg(args, "operation")? != "insert" {
        return Err("unica.code.patch v1 supports only operation=insert".to_string());
    }
    let position = Position::parse(string_arg(args, "position")?)?;
    let selector = Selector::parse(args)?;
    let snapshot = SourceTextSnapshot::from_bytes(text.as_bytes())
        .map_err(|error| format!("BSL module snapshot: {error}"))?;
    let indexed = analyze_module(text)?;
    reject_parse_diagnostics(&indexed.diagnostics, "validate original BSL module")?;
    locate_selector(&snapshot, position, &selector, &indexed.methods)
}

pub fn locate_selector_typed(
    snapshot: &SourceTextSnapshot,
    position: Position,
    selector: &Selector,
    methods: &[Method],
) -> Result<InsertionSite, SelectorResolutionError> {
    reject_lone_cr_line_endings(snapshot).map_err(SelectorResolutionError::invalid_source)?;
    let text = snapshot.decoded_text();
    let mut head_end = None;
    let offset = match selector {
        Selector::Method(name) => {
            let folded_name = name.to_lowercase();
            let found = methods
                .iter()
                .filter(|method| method.name.to_lowercase() == folded_name)
                .collect::<Vec<_>>();
            let method = match found.as_slice() {
                [method] => *method,
                [] => return Err(SelectorResolutionError::zero_matches(selector)),
                _ => {
                    return Err(SelectorResolutionError::multiple_matches(
                        selector,
                        found.len(),
                    ))
                }
            };
            let (start, end) = method_unit(text, method);
            match position {
                Position::Before => {
                    head_end = Some(safe_line_start(text, method.start));
                    start
                }
                Position::After => end,
            }
        }
        Selector::Anchor(anchor) => {
            let found = anchor_occurrences(text, anchor, methods);
            let selected = match found.as_slice() {
                [selected] => *selected,
                [] => return Err(SelectorResolutionError::zero_matches(selector)),
                _ => {
                    return Err(SelectorResolutionError::multiple_matches(
                        selector,
                        found.len(),
                    ))
                }
            };
            match position {
                Position::Before => safe_line_start(text, selected.start),
                Position::After => anchor_line_end(text, selected.end),
            }
        }
    };
    let local = local_line_ending_at(text, offset, position);
    let eol = resolve_observed_line_ending(snapshot, local).map_err(|error| {
        SelectorResolutionError::invalid_source(format!("resolve code.patch EOL: {error}"))
    })?;
    let leading_separator = if position == Position::After
        && offset == text.len()
        && !text.is_empty()
        && !text.as_bytes().ends_with(b"\n")
    {
        LeadingSeparator::LocalEol
    } else {
        LeadingSeparator::None
    };
    Ok(InsertionSite {
        offset,
        head_end: head_end.unwrap_or(offset),
        position,
        eol,
        leading_separator,
    })
}

pub fn locate_selector(
    snapshot: &SourceTextSnapshot,
    position: Position,
    selector: &Selector,
    methods: &[Method],
) -> Result<InsertionSite, String> {
    locate_selector_typed(snapshot, position, selector, methods).map_err(|error| error.to_string())
}

/// Resolves the one place every module has: its end.
///
/// The site is `Before` the end-of-text offset rather than `After` it, because
/// that is what makes the repeat provable. On the next identical call the offset
/// has moved to the new end, and `insertion_is_present` then asks whether the
/// text already ends with the same bytes — which is exactly the question. An
/// `After` site would look at the empty tail past the end and never match.
///
/// A module holding no method yet is not a special case here: its text is empty,
/// so the offset is zero and no separator is owed.
pub fn locate_module_tail(snapshot: &SourceTextSnapshot) -> Result<InsertionSite, String> {
    reject_lone_cr_line_endings(snapshot)?;
    let text = snapshot.decoded_text();
    let offset = text.len();
    let local = local_line_ending_at(text, offset, Position::Before);
    let eol = resolve_observed_line_ending(snapshot, local)
        .map_err(|error| format!("resolve code.patch EOL: {error}"))?;
    // The separator question is about content, not bytes: a module holding only
    // a byte order mark has nothing to be separated from, so it owes no blank
    // line. `text()` is the same span as `decoded_text()` without that mark.
    let content = snapshot.text();
    let leading_separator = if content.is_empty() || content.as_bytes().ends_with(b"\n") {
        LeadingSeparator::None
    } else {
        LeadingSeparator::LocalEol
    };
    Ok(InsertionSite {
        offset,
        head_end: offset,
        position: Position::Before,
        eol,
        leading_separator,
    })
}

/// Resolves the span a replacement overwrites. A method selector takes the whole
/// method including the line it starts and ends on; an anchor selector takes the
/// exact occurrence, so an inline edit stays inline.
pub fn locate_replacement_typed(
    snapshot: &SourceTextSnapshot,
    selector: &Selector,
    methods: &[Method],
) -> Result<ReplacementSite, SelectorResolutionError> {
    reject_lone_cr_line_endings(snapshot).map_err(SelectorResolutionError::invalid_source)?;
    let text = snapshot.decoded_text();
    let (start, end) = match selector {
        Selector::Method(name) => {
            let folded_name = name.to_lowercase();
            let found = methods
                .iter()
                .filter(|method| method.name.to_lowercase() == folded_name)
                .collect::<Vec<_>>();
            let method = match found.as_slice() {
                [method] => *method,
                [] => return Err(SelectorResolutionError::zero_matches(selector)),
                _ => {
                    return Err(SelectorResolutionError::multiple_matches(
                        selector,
                        found.len(),
                    ))
                }
            };
            (
                safe_line_start(text, method.start),
                line_end(text, method.end),
            )
        }
        Selector::Anchor(anchor) => {
            let found = anchor_occurrences(text, anchor, methods);
            let selected = match found.as_slice() {
                [selected] => *selected,
                [] => return Err(SelectorResolutionError::zero_matches(selector)),
                _ => {
                    return Err(SelectorResolutionError::multiple_matches(
                        selector,
                        found.len(),
                    ))
                }
            };
            (selected.start, selected.end)
        }
    };
    let local = local_line_ending_at(text, start, Position::Before);
    let eol = resolve_observed_line_ending(snapshot, local).map_err(|error| {
        SelectorResolutionError::invalid_source(format!("resolve code.patch EOL: {error}"))
    })?;
    // The replacement keeps a trailing newline only where the span it overwrites
    // had one, so replacing an inline anchor does not break the line.
    let trailing_eol = text
        .get(..end)
        .is_some_and(|head| head.ends_with('\n') || head.ends_with('\r'));
    Ok(ReplacementSite {
        start,
        end,
        eol,
        trailing_eol,
        leading_eol: false,
    })
}

pub fn locate_replacement(
    snapshot: &SourceTextSnapshot,
    selector: &Selector,
    methods: &[Method],
) -> Result<ReplacementSite, String> {
    locate_replacement_typed(snapshot, selector, methods).map_err(|error| error.to_string())
}

pub fn normalized_replacement(content: &str, eol: LineEnding, trailing_eol: bool) -> Vec<u8> {
    let eol = eol.as_str();
    let mut bytes = canonicalize_eol(content).replace('\n', eol).into_bytes();
    if trailing_eol && !bytes.ends_with(eol.as_bytes()) {
        bytes.extend_from_slice(eol.as_bytes());
    }
    bytes
}

pub fn reject_lone_cr_line_endings(snapshot: &SourceTextSnapshot) -> Result<(), String> {
    match snapshot.line_endings() {
        LineEndingProfile::Uniform(LineEnding::Cr) | LineEndingProfile::Mixed { cr: 1.., .. } => {
            Err(
                "unica.code.patch v1 does not support source containing lone CR line endings"
                    .to_string(),
            )
        }
        LineEndingProfile::None
        | LineEndingProfile::Uniform(LineEnding::Lf | LineEnding::CrLf)
        | LineEndingProfile::Mixed { cr: 0, .. } => Ok(()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnchorOccurrence {
    pub start: usize,
    pub end: usize,
}

pub fn anchor_occurrences(text: &str, anchor: &str, methods: &[Method]) -> Vec<AnchorOccurrence> {
    canonical_occurrences(text, anchor)
        .into_iter()
        .filter(|occurrence| {
            methods
                .iter()
                .filter(|method| {
                    occurrence.start >= method.start && occurrence.end <= line_end(text, method.end)
                })
                .count()
                == 1
        })
        .collect()
}

pub fn canonical_occurrences(text: &str, needle: &str) -> Vec<AnchorOccurrence> {
    if needle.is_empty() {
        return Vec::new();
    }
    text.char_indices()
        .filter_map(|(start, _)| {
            if text.as_bytes().get(start) == Some(&b'\n')
                && start > 0
                && text.as_bytes().get(start - 1) == Some(&b'\r')
            {
                return None;
            }
            canonical_match_end(text, start, needle).map(|end| AnchorOccurrence { start, end })
        })
        .collect()
}

pub fn canonical_match_end(text: &str, start: usize, needle: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut offset = start;
    for expected in needle.chars() {
        if expected == '\n' {
            match bytes.get(offset) {
                Some(b'\r') if bytes.get(offset + 1) == Some(&b'\n') => offset += 2,
                Some(b'\r' | b'\n') => offset += 1,
                _ => return None,
            }
            continue;
        }
        let actual = text.get(offset..)?.chars().next()?;
        if actual != expected {
            return None;
        }
        offset += actual.len_utf8();
    }
    Some(offset)
}

pub fn prove_repeat_is_noop(
    postimage: &str,
    plan: &CodeTextPatchPlan,
    methods: &[Method],
) -> Result<(), String> {
    prove_repeat_is_noop_parts(
        postimage,
        plan.selector.as_ref(),
        &plan.content,
        plan.site,
        methods,
    )
}

pub fn prove_repeat_is_noop_parts(
    postimage: &str,
    selector: Option<&Selector>,
    content: &str,
    site: PatchSite,
    methods: &[Method],
) -> Result<(), String> {
    let snapshot = SourceTextSnapshot::from_bytes(postimage.as_bytes())
        .map_err(|error| format!("patched BSL module snapshot: {error}"))?;
    let stale =
        |error: String| format!("patch cannot be applied idempotently on the next call: {error}");
    let repeated_is_noop = match site {
        PatchSite::Insertion(site) => {
            let repeat_site = match selector {
                Some(selector) => {
                    locate_selector(&snapshot, site.position, selector, methods).map_err(stale)?
                }
                // The end of the module is still the end of the module, so the
                // repeat resolves; whether it writes is decided just below.
                None => locate_module_tail(&snapshot).map_err(stale)?,
            };
            prepare_insertion(&snapshot, repeat_site, content, methods).1
        }
        PatchSite::Replacement(_) => {
            let selector = selector.expect("replacement has a selector");
            match locate_replacement(&snapshot, selector, methods) {
                // The edit consumed its own selector — an anchor rewritten to new
                // text, a method renamed. A repeated identical call then resolves
                // nothing and fails closed without writing, which is exactly the
                // double application the guard exists to prevent.
                Err(_) => true,
                Ok(repeat_site) => {
                    let repeat_replacement = repeat_site.replacement(content);
                    replacement_is_present(
                        snapshot.raw(),
                        repeat_site,
                        &repeat_replacement,
                        |found, methods| relocate_selected_method(found, methods, selector),
                    )
                }
            }
        }
    };
    if repeated_is_noop {
        Ok(())
    } else {
        Err(
            "patch cannot be applied idempotently on the next call: repeated planning would change bytes"
                .to_string(),
        )
    }
}

/// Whether a replacement already stands where its target is found today.
///
/// A whole-method replacement may extend past the method it addresses: the
/// "extract method" refactor sends the method together with the helpers
/// carved out of it. After the first write the addressed method spans only
/// its own part of that text, so comparing the method's span alone would
/// plan the helpers a second time. The comparison therefore anchors at the
/// method's place inside the replacement and asks whether the whole
/// replacement already surrounds the method found in the module. Targets
/// without a method of their own (anchors, bodies, the module body) keep the
/// exact span comparison.
pub fn replacement_is_present(
    raw: &[u8],
    site: ReplacementSite,
    replacement: &[u8],
    relocate: impl Fn(&SourceTextSnapshot, &[Method]) -> Option<ReplacementSite>,
) -> bool {
    if raw.get(site.start..site.end) == Some(replacement) {
        return true;
    }
    let Some(lead) = replacement_lead(replacement, relocate) else {
        return false;
    };
    site.start
        .checked_sub(lead)
        .and_then(|from| raw.get(from..from.checked_add(replacement.len())?))
        == Some(replacement)
}

/// Offset at which the addressed method starts inside its own replacement,
/// located by the same rule that finds it in the module. The method must sit
/// on whole lines of the replacement, so the text around it is whole lines
/// too; an inline tail after `КонецПроцедуры` would otherwise read as a fixed
/// point while each repeat appended it again. The equality with the module
/// then puts the same line breaks there.
pub fn replacement_lead(
    replacement: &[u8],
    relocate: impl Fn(&SourceTextSnapshot, &[Method]) -> Option<ReplacementSite>,
) -> Option<usize> {
    let snapshot = SourceTextSnapshot::from_bytes(replacement).ok()?;
    let analysis = analyze_module(snapshot.decoded_text()).ok()?;
    if !analysis.diagnostics.is_empty() {
        return None;
    }
    let site = relocate(&snapshot, &analysis.methods)?;
    let raw = snapshot.raw();
    let starts_line = site.start == 0 || raw.get(..site.start)?.ends_with(b"\n");
    let ends_line = raw.get(..site.end)?.ends_with(b"\n");
    (starts_line && ends_line).then_some(site.start)
}

/// Finds the whole method a staged replacement addresses by its
/// `…Method.<Имя>` address, in any module text.
pub fn relocate_replaced_method(
    snapshot: &SourceTextSnapshot,
    methods: &[Method],
    target: &ReplacementTarget,
) -> Option<ReplacementSite> {
    match code_projection(&target.at) {
        Some((_, CodeProjection::Method(_))) => {
            locate_addressed_replacement(snapshot, target, methods, "").ok()
        }
        Some((_, CodeProjection::ModuleBody | CodeProjection::MethodBody(_))) | None => None,
    }
}

/// Whether a staged replacement targets a whole method, the only target that
/// can carry methods besides itself.
pub fn replaces_whole_method(at: &QualifiedAddress) -> bool {
    matches!(code_projection(at), Some((_, CodeProjection::Method(_))))
}

pub fn relocate_selected_method(
    snapshot: &SourceTextSnapshot,
    methods: &[Method],
    selector: &Selector,
) -> Option<ReplacementSite> {
    match selector {
        Selector::Method(_) => locate_replacement_typed(snapshot, selector, methods).ok(),
        Selector::Anchor(_) => None,
    }
}

/// A whole-method replacement may declare methods besides the one it
/// addresses, but never one the rest of the module already defines: the
/// result would carry two methods with one name, which no repeat can undo.
///
/// Same-named methods in `#Если … #Иначе` branches are legitimate and the
/// parser does not tell them apart from duplicates, so twins already in the
/// module or wholly inside the replacement stay allowed; only a name whose
/// count grows while the rest of the module also declares it is refused.
pub fn reject_redeclared_methods(
    replacement: &str,
    pre: &[Method],
    post: &[Method],
) -> Result<(), String> {
    let Ok(declared) = analyze_module(replacement) else {
        return Ok(());
    };
    let count = |methods: &[Method], folded: &str| {
        methods
            .iter()
            .filter(|candidate| candidate.name.to_lowercase() == folded)
            .count()
    };
    for method in &declared.methods {
        let folded = method.name.to_lowercase();
        let in_text = count(&declared.methods, &folded);
        let after = count(post, &folded);
        if after > in_text && after > count(pre, &folded) {
            return Err(format!(
                "replacement declares method `{}` that the module already defines elsewhere",
                method.name
            ));
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct Method {
    pub name: String,
    pub start: usize,
    pub end: usize,
    pub body_bounds: Option<(usize, usize)>,
}

pub struct ModuleAnalysis {
    pub methods: Vec<Method>,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

pub fn analyze_module(text: &str) -> Result<ModuleAnalysis, String> {
    if text.len() > u32::MAX as usize {
        return Err("BSL module is too large for the analyzer parser".to_string());
    }
    let parsed = unica_bsl_parser::parse(text);
    let diagnostics = parsed
        .errors()
        .iter()
        .map(|error| validation_diagnostic(text, error))
        .collect::<Result<Vec<_>, _>>()?;
    let mut methods = parsed
        .methods()
        .into_iter()
        .filter_map(|method| {
            method.name.map(|name| Method {
                name,
                start: method.range.start,
                end: method.range.end,
                body_bounds: method
                    .direct_declaration_end
                    .zip(method.direct_closing_start),
            })
        })
        .collect::<Vec<_>>();
    methods.sort_by_key(|method| method.start);
    Ok(ModuleAnalysis {
        methods,
        diagnostics,
    })
}

pub fn validation_diagnostic(
    text: &str,
    error: &unica_bsl_parser::ParseDiagnostic,
) -> Result<ValidationDiagnostic, String> {
    let start_byte = error.range.start;
    let end_byte = error.range.end;
    let (start_line, start_column) = line_column(text, start_byte)?;
    let (end_line, end_column) = line_column(text, end_byte)?;
    Ok(ValidationDiagnostic {
        code: "bsl-parse-error",
        message: error.message.clone(),
        start_byte,
        end_byte,
        start_line,
        start_column,
        end_line,
        end_column,
    })
}

pub fn reject_parse_diagnostics(
    diagnostics: &[ValidationDiagnostic],
    context: &str,
) -> Result<(), String> {
    if diagnostics.is_empty() {
        return Ok(());
    }
    let details = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .take(5)
        .collect::<Vec<_>>()
        .join("; ");
    Err(format!("{context}: {details}"))
}

pub fn safe_line_start(text: &str, from: usize) -> usize {
    let start = text
        .as_bytes()
        .get(..from)
        .and_then(|prefix| prefix.iter().rposition(|byte| *byte == b'\n'))
        .map_or(0, |position| position + 1);
    if start == 0 && text.as_bytes().starts_with(b"\xef\xbb\xbf") {
        3
    } else {
        start
    }
}

pub fn line_end(text: &str, from: usize) -> usize {
    text.as_bytes()
        .get(from..)
        .and_then(|suffix| suffix.iter().position(|byte| *byte == b'\n'))
        .map_or(text.len(), |position| from + position + 1)
}

pub fn anchor_line_end(text: &str, anchor_end: usize) -> usize {
    if anchor_end > 0 && text.as_bytes().get(anchor_end - 1) == Some(&b'\n') {
        anchor_end
    } else {
        line_end(text, anchor_end)
    }
}

pub fn line_column(text: &str, offset: usize) -> Result<(usize, usize), String> {
    let prefix = text
        .get(..offset)
        .ok_or_else(|| format!("byte offset {offset} is not a UTF-8 boundary in BSL module"))?;
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix, |(_, current)| current)
        .chars()
        .count()
        + 1;
    Ok((line, column))
}

pub fn local_line_ending_at(text: &str, offset: usize, position: Position) -> Option<LineEnding> {
    let bytes = text.as_bytes();
    let before = bytes
        .get(..offset)
        .and_then(|prefix| prefix.iter().rposition(|byte| *byte == b'\n'))
        .map(|newline| line_ending_at_newline(bytes, newline));
    let after = bytes
        .get(offset..)
        .and_then(|suffix| suffix.iter().position(|byte| *byte == b'\n'))
        .map(|newline| line_ending_at_newline(bytes, offset + newline));
    match position {
        Position::Before => after.or(before),
        Position::After => before.or(after),
    }
}

pub fn line_ending_at_newline(bytes: &[u8], newline: usize) -> LineEnding {
    if newline > 0 && bytes.get(newline - 1) == Some(&b'\r') {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    }
}

pub fn canonicalize_eol(content: &str) -> String {
    content.replace("\r\n", "\n").replace('\r', "\n")
}

/// Разделители принадлежат месту вставки, а не тексту операции: при повторе
/// у того же метода уже может стоять добавленная нами пустая строка.
fn prepare_insertion(
    snapshot: &SourceTextSnapshot,
    site: InsertionSite,
    content: &str,
    methods: &[Method],
) -> (Vec<u8>, bool) {
    let mut insertion = normalized_content(content, site.eol, site.leading_separator);
    if method_head(content).is_none() {
        let present = insertion_is_present(snapshot.raw(), site, &insertion);
        return (insertion, present);
    }
    insertion = normalized_content(content, site.eol, LeadingSeparator::None);
    let text = snapshot.decoded_text();
    let eol = site.eol.as_str().as_bytes();
    let leading_breaks = |bytes: &[u8]| {
        bytes
            .iter()
            .take_while(|b| b.is_ascii_whitespace())
            .filter(|&&b| b == b'\n')
            .count()
    };
    let trailing_breaks = |bytes: &[u8]| {
        bytes
            .iter()
            .rev()
            .take_while(|b| b.is_ascii_whitespace())
            .filter(|&&b| b == b'\n')
            .count()
    };
    if methods.iter().any(|method| method.start >= site.offset) {
        // Замыкающий комментарий вставки становится частью головы соседа.
        // Разделяем методы перед этим комментарием, не отрывая его от соседа.
        let boundary = doc_comment_start(std::str::from_utf8(&insertion).unwrap(), insertion.len());
        let following = if boundary == insertion.len() {
            &text.as_bytes()[site.offset..]
        } else {
            &insertion[boundary..]
        };
        let missing = 2usize
            .saturating_sub(trailing_breaks(&insertion[..boundary]) + leading_breaks(following));
        insertion.splice(boundary..boundary, eol.repeat(missing));
    }
    // Метод, уже стоящий вплотную к соседу, — тоже повтор: модуль мог быть
    // написан вручную или прежней вставкой без разделителей.
    let bare = normalized_content(content, site.eol, LeadingSeparator::None);
    let present = method_insertion_is_present(text, site, &bare)
        || method_insertion_is_present(text, site, &insertion);
    if methods.iter().any(|method| method.end <= site.offset) {
        let missing = 2usize.saturating_sub(
            trailing_breaks(&snapshot.raw()[..site.offset]) + leading_breaks(&insertion),
        );
        insertion.splice(0..0, eol.repeat(missing));
    } else if site.leading_separator == LeadingSeparator::LocalEol {
        insertion.splice(0..0, eol.iter().copied());
    }
    (insertion, present)
}

fn method_insertion_is_present(text: &str, mut site: InsertionSite, plain: &[u8]) -> bool {
    loop {
        if insertion_is_present(text.as_bytes(), site, plain) {
            return true;
        }
        match site.position {
            Position::Before if site.offset > 0 => {
                let previous = safe_line_start(text, site.offset - 1);
                if previous >= site.offset || !text[previous..site.offset].trim().is_empty() {
                    return false;
                }
                site.offset = previous;
                site.head_end = previous;
            }
            Position::After if site.offset < text.len() => {
                let next = line_end(text, site.offset);
                if !text[site.offset..next].trim().is_empty() {
                    return false;
                }
                site.offset = next;
            }
            _ => return false,
        }
    }
}

pub fn normalized_content(
    content: &str,
    eol: LineEnding,
    leading_separator: LeadingSeparator,
) -> Vec<u8> {
    let eol = eol.as_str();
    let normalized = canonicalize_eol(content).replace('\n', eol);
    let mut bytes = Vec::new();
    if leading_separator == LeadingSeparator::LocalEol {
        bytes.extend_from_slice(eol.as_bytes());
    }
    bytes.extend_from_slice(normalized.as_bytes());
    if !bytes.ends_with(eol.as_bytes()) {
        bytes.extend_from_slice(eol.as_bytes());
    }
    bytes
}

pub fn insertion_is_present(text: &[u8], site: InsertionSite, insertion: &[u8]) -> bool {
    match site.position {
        Position::Before => {
            (site.offset..=site.head_end.max(site.offset))
                .filter(|&boundary| {
                    boundary == site.offset || text.get(boundary - 1) == Some(&b'\n')
                })
                .any(|boundary| {
                    text.get(..boundary)
                        .is_some_and(|head| head.ends_with(insertion))
                })
                || unterminated_tail_is(text, site, insertion)
        }
        Position::After => text
            .get(site.offset..)
            .is_some_and(|tail| tail.starts_with(insertion)),
    }
}

/// Вставка всегда кончается переводом строки, а модуль может кончаться без
/// него. Повтор вставки в такой конец — тот же текст без последнего перевода
/// строки, с начала строки (#1911, INV.SOURCE.IDEMPOTENT-PRECHECK).
fn unterminated_tail_is(text: &[u8], site: InsertionSite, insertion: &[u8]) -> bool {
    if site.offset != text.len() || text.ends_with(b"\n") {
        return false;
    }
    let Some(body) = insertion.strip_suffix(b"\n") else {
        return false;
    };
    let body = body.strip_suffix(b"\r").unwrap_or(body);
    // Разделитель в начале вставки не нужен, когда она — весь модуль.
    let bare = body
        .strip_prefix(b"\r\n")
        .or_else(|| body.strip_prefix(b"\n"))
        .unwrap_or(body);
    [body, bare].into_iter().any(|candidate| {
        let Some(head) = text
            .len()
            .checked_sub(candidate.len())
            .map(|end| &text[..end])
        else {
            return false;
        };
        // Вставка начинается с начала строки: своим разделителем, после
        // перевода строки текста или в начале модуля (после BOM).
        !candidate.is_empty()
            && text.ends_with(candidate)
            && (candidate.starts_with(b"\n")
                || candidate.starts_with(b"\r\n")
                || head.ends_with(b"\n")
                || head.chunks(3).all(|chunk| chunk == b"\xEF\xBB\xBF"))
    })
}

pub fn string_arg<'a>(args: &'a Map<String, Value>, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("unica.code.patch requires non-empty `{name}`"))
}

pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn unified_diff(path: &str, before: &str, after: &str) -> Result<String, String> {
    let mut options = DiffOptions::new();
    options
        .set_original_filename(format!("a/{path}"))
        .set_modified_filename(format!("b/{path}"));
    let patch = options.create_patch(before, after);
    let rendered = patch.to_string();
    let reparsed = Patch::from_str(&rendered)
        .map_err(|error| format!("generated unified diff cannot be parsed: {error}"))?;
    let rebuilt = apply(before, &reparsed)
        .map_err(|error| format!("generated unified diff cannot be applied: {error}"))?;
    if rebuilt.as_bytes() != after.as_bytes() {
        return Err("generated unified diff does not reproduce the exact postimage".to_string());
    }
    Ok(rendered)
}

pub struct CodeTextPatchPlan {
    pub before: Vec<u8>,
    pub after: Vec<u8>,
    pub selector: Option<Selector>,
    pub content: String,
    pub insertion: Vec<u8>,
    pub site: PatchSite,
    pub no_op: bool,
}

pub struct CodePatchIdentity<'a> {
    pub source_set: &'a str,
    pub metadata_path: &'a str,
    pub target_kind: TargetKind,
    pub owner: &'a str,
    pub module_role: &'a str,
}

pub struct CodePatchFailure {
    pub message: String,
    pub data: Option<Box<CodePatchData>>,
}
impl CodePatchFailure {
    pub fn new(message: String, data: Option<CodePatchData>) -> Self {
        Self {
            message,
            data: data.map(Box::new),
        }
    }
}

pub fn plan_legacy_patch(
    args: &Map<String, Value>,
    before: Vec<u8>,
) -> Result<CodeTextPatchPlan, String> {
    let snapshot = SourceTextSnapshot::from_bytes(&before)
        .map_err(|error| format!("BSL module snapshot: {error}"))?;
    let text = snapshot.decoded_text();
    let operation = PatchOperation::parse(string_arg(args, "operation")?)?;
    let indexed = analyze_module(text)?;
    reject_parse_diagnostics(&indexed.diagnostics, "validate original BSL module")?;
    let content = string_arg(args, "content")?.to_string();
    let (selector, site, insertion, no_op, after) = match operation {
        PatchOperation::Insert => {
            let selector = Selector::parse_optional(args)?;
            let site = match &selector {
                Some(selector) => {
                    let position = Position::parse(string_arg(args, "position")?)?;
                    locate_selector(&snapshot, position, selector, &indexed.methods)?
                }
                // Without a selector there is nothing to place content relative
                // to, so `position` is refused rather than silently ignored.
                None => {
                    if args.contains_key("position") {
                        return Err(
                            "unica.code.patch does not accept `position` without a `selector`; content goes to the end of the module"
                                .to_string(),
                        );
                    }
                    locate_module_tail(&snapshot)?
                }
            };
            let (insertion, no_op) = prepare_insertion(&snapshot, site, &content, &indexed.methods);
            let mut after = snapshot.raw().to_vec();
            if !no_op {
                after.splice(site.offset..site.offset, insertion.iter().copied());
            }
            (
                selector,
                PatchSite::Insertion(site),
                insertion,
                no_op,
                after,
            )
        }
        PatchOperation::Replace => {
            let selector = Selector::parse(args)?;
            let site = locate_replacement(&snapshot, &selector, &indexed.methods)?;
            let replacement = site.replacement(&content);
            let no_op =
                replacement_is_present(snapshot.raw(), site, &replacement, |found, methods| {
                    relocate_selected_method(found, methods, &selector)
                });
            let mut after = snapshot.raw().to_vec();
            if !no_op {
                after.splice(site.start..site.end, replacement.iter().copied());
            }
            (
                Some(selector),
                PatchSite::Replacement(site),
                replacement,
                no_op,
                after,
            )
        }
    };
    Ok(CodeTextPatchPlan {
        before,
        after,
        selector,
        content,
        insertion,
        site,
        no_op,
    })
}
pub fn validate_legacy_patch(
    plan: &CodeTextPatchPlan,
    identity: CodePatchIdentity<'_>,
) -> Result<CodePatchData, CodePatchFailure> {
    let postimage = match std::str::from_utf8(&plan.after) {
        Ok(postimage) => postimage,
        Err(_) => {
            return Err(CodePatchFailure::new(
                "patched BSL module must remain UTF-8".to_string(),
                None,
            ))
        }
    };
    let post_hash = hash(&plan.after);
    let analysis = match analyze_module(postimage) {
        Ok(analysis) => analysis,
        Err(error) => return Err(CodePatchFailure::new(error, None)),
    };
    let validation_status = if analysis.diagnostics.is_empty() {
        ValidationStatus::Passed
    } else {
        ValidationStatus::Failed
    };
    let validation = SourceValidation {
        kind: ValidationKind::BslAnalyzerParser,
        status: validation_status,
        validated_post_hash: post_hash.clone(),
        diagnostics: analysis.diagnostics,
    };
    let data = match patch_data(plan, postimage, post_hash, validation, identity) {
        Ok(data) => data,
        Err(error) => return Err(CodePatchFailure::new(error, None)),
    };
    if data.validation.status == ValidationStatus::Failed {
        let details = data
            .validation
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .take(5)
            .collect::<Vec<_>>()
            .join("; ");
        return Err(CodePatchFailure::new(
            format!("validate patched BSL module: {details}"),
            Some(data),
        ));
    }
    if matches!(plan.site, PatchSite::Replacement(_))
        && matches!(plan.selector, Some(Selector::Method(_)))
    {
        let pre = std::str::from_utf8(&plan.before)
            .map_err(|_| "original BSL module is not UTF-8".to_string())
            .and_then(analyze_module);
        let checked = pre.and_then(|pre| {
            reject_redeclared_methods(&plan.content, &pre.methods, &analysis.methods)
        });
        if let Err(error) = checked {
            return Err(CodePatchFailure::new(error, Some(data)));
        }
    }
    if let Err(error) = prove_repeat_is_noop(postimage, plan, &analysis.methods) {
        return Err(CodePatchFailure::new(error, Some(data)));
    }
    Ok(data)
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use serde_json::{json, Map, Value};
    pub fn arguments(selector: Value, position: &str) -> Map<String, Value> {
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("insert"));
        args.insert("selector".to_string(), selector);
        args.insert("position".to_string(), json!(position));
        args
    }
}
#[cfg(test)]
mod tests {
    use super::test_support::arguments;
    use super::*;
    use diffy::{apply, Patch};
    use serde_json::json;
    use unica_xml_format::text::{resolve_line_ending, EolPolicy};
    const MODULE: &str = "Процедура Первая()\n    Сообщить(\"один\");\nКонецПроцедуры\n\nФункция Вторая()\n    Возврат Истина;\nКонецФункции\n";
    #[test]
    fn method_selector_places_after_the_complete_method() {
        let args = arguments(json!({"method": "Первая"}), "after");
        let site = locate_insertion(MODULE, &args).unwrap();
        assert!(MODULE[site.offset..].starts_with("\nФункция Вторая()"));
    }

    #[test]
    fn method_selector_is_case_insensitive_for_bsl_identifiers() {
        let args = arguments(json!({"method": "пЕРВАЯ"}), "after");

        let site = locate_insertion(MODULE, &args).unwrap();

        assert!(MODULE[site.offset..].starts_with("\nФункция Вторая()"));
    }

    #[test]
    fn method_before_keeps_bom_first_and_includes_its_annotation() {
        let module = "\u{feff}&НаКлиенте\nProcedure Run()\nEndProcedure\n";
        let args = arguments(json!({"method": "Run"}), "before");
        let site = locate_insertion(module, &args).unwrap();

        assert_eq!(site.offset, "\u{feff}".len());
        assert!(module[site.offset..].starts_with("&НаКлиенте"));
    }

    #[test]
    fn anchor_must_be_unique_and_inside_one_method() {
        let args = arguments(json!({"anchor": "Сообщить(\"один\");"}), "before");
        assert!(locate_insertion(MODULE, &args).is_ok());

        let args = arguments(json!({"anchor": "КонецПроцедуры"}), "before");
        assert!(locate_insertion(MODULE, &args).is_ok());

        let args = arguments(json!({"anchor": "отсутствует"}), "before");
        assert!(locate_insertion(MODULE, &args).is_err());

        let args = arguments(json!({"anchor": "\n\n"}), "before");
        assert!(locate_insertion(MODULE, &args).is_err());
    }

    #[test]
    fn anchor_before_uses_the_start_of_the_anchored_line() {
        let module = "Procedure Run()\n    Message(\"ok\");\nEndProcedure\n";
        let args = arguments(json!({"anchor": "Message(\"ok\");"}), "before");

        let site = locate_insertion(module, &args).unwrap();

        assert_eq!(site.offset, module.find("    Message").unwrap());
    }

    #[test]
    fn multiline_anchor_ending_with_eol_does_not_skip_the_following_line() {
        let module = "Procedure Run()\n    First();\n    Second();\n    Third();\nEndProcedure\n";
        let anchor = "    First();\n    Second();\n";
        let args = arguments(json!({"anchor": anchor}), "after");

        let site = locate_insertion(module, &args).unwrap();

        assert_eq!(site.offset, module.find("    Third();").unwrap());
        assert_eq!(site.eol, LineEnding::Lf);
    }

    #[test]
    fn multiline_anchor_matches_mixed_source_eol_and_uses_real_byte_range() {
        let module =
            "Procedure Run()\r\n    First();\n    Second();\r\n    Third();\nEndProcedure\n";
        let anchor = "    First();\n    Second();\n";
        let args = arguments(json!({"anchor": anchor}), "after");

        let site = locate_insertion(module, &args).unwrap();

        assert_eq!(site.offset, module.find("    Third();").unwrap());
        assert_eq!(site.eol, LineEnding::CrLf);
    }

    #[test]
    fn overlapping_anchor_occurrences_are_counted() {
        let module = "Procedure Run()\n    aaaa = 1;\nEndProcedure\n";
        let args = arguments(json!({"anchor": "aaa"}), "before");

        let error = locate_insertion(module, &args).unwrap_err();

        assert!(error.contains("matched 2 times"), "{error}");
    }

    #[test]
    fn anchor_cardinality_ignores_non_method_decoys() {
        let module = "// Target();\nProcedure Run()\n    Target();\nEndProcedure\n";
        let args = arguments(json!({"anchor": "Target();"}), "before");

        let site = locate_insertion(module, &args).unwrap();

        assert_eq!(site.offset, module.find("    Target();").unwrap());
    }

    #[test]
    fn mixed_eol_uses_the_target_method_line_ending() {
        let module = "Procedure First()\r\nEndProcedure\r\nProcedure Second()\nEndProcedure\n";
        let args = arguments(json!({"method": "Second"}), "after");
        let site = locate_insertion(module, &args).unwrap();

        assert_eq!(site.eol, LineEnding::Lf);
    }

    #[test]
    fn code_patch_rejects_lone_cr_instead_of_inventing_or_gaining_an_eol_policy() {
        let args = arguments(json!({"method": "First"}), "after");

        for module in [
            "Procedure First()\rEndProcedure\r",
            "Procedure First()\rEndProcedure\rProcedure Second()\nEndProcedure\n",
        ] {
            let error = locate_insertion(module, &args).unwrap_err();

            assert!(error.contains("lone CR line endings"), "{error}");
        }
    }

    #[test]
    fn local_eol_observation_is_resolved_by_shared_preserve_policy() {
        let snapshot = SourceTextSnapshot::from_bytes(
            b"Procedure First()\r\nEndProcedure\r\nProcedure Second()\nEndProcedure\n",
        )
        .unwrap();
        let offset = snapshot.decoded_text().len();
        let local = local_line_ending_at(snapshot.decoded_text(), offset, Position::After);

        assert_eq!(
            resolve_line_ending(EolPolicy::Preserve, &snapshot, local),
            Ok(LineEnding::Lf)
        );
    }

    #[test]
    fn inserted_content_uses_local_line_ending_once() {
        assert_eq!(
            normalized_content("A\r\nB", LineEnding::Lf, LeadingSeparator::None),
            b"A\nB\n"
        );
        assert_eq!(
            normalized_content("A\nB", LineEnding::CrLf, LeadingSeparator::None),
            b"A\r\nB\r\n"
        );
        assert_eq!(
            normalized_content("A", LineEnding::Lf, LeadingSeparator::LocalEol),
            b"\nA\n"
        );
    }

    #[test]
    fn repeated_before_or_after_insertion_is_a_noop() {
        let before = b"// marker\nProcedure First()";
        let before_site = super::InsertionSite {
            offset: 10,
            head_end: 10,
            position: Position::Before,
            eol: LineEnding::Lf,
            leading_separator: LeadingSeparator::None,
        };
        assert!(insertion_is_present(before, before_site, b"// marker\n"));

        let after = b"Procedure First()\n// marker\n";
        let after_site = super::InsertionSite {
            offset: after.len() - b"// marker\n".len(),
            head_end: after.len() - b"// marker\n".len(),
            position: Position::After,
            eol: LineEnding::Lf,
            leading_separator: LeadingSeparator::None,
        };
        assert!(insertion_is_present(after, after_site, b"// marker\n"));
    }

    #[test]
    fn analyzer_index_accepts_bom_english_case_and_common_bsl_regions() {
        let module = "\u{feff}&MyAnnotation\nprocedure Run()\n#Region R\nif true then\nMessage(\"ok\");\n#EndRegion\nendif;\nendprocedure\n";
        let analysis = analyze_module(module).unwrap();

        assert!(analysis.diagnostics.is_empty());
        assert_eq!(analysis.methods.len(), 1);
        assert_eq!(analysis.methods[0].name, "Run");
        assert_eq!(analysis.methods[0].start, "\u{feff}".len());
    }

    #[test]
    fn analyzer_index_ignores_declaration_words_in_comments_and_strings() {
        let module =
        "// Procedure Fake()\nValue = \"Function AlsoFake()\";\nProcedure Run()\nEndProcedure\n";
        let analysis = analyze_module(module).unwrap();

        assert!(analysis.diagnostics.is_empty());
        assert_eq!(analysis.methods.len(), 1);
        assert_eq!(analysis.methods[0].name, "Run");
    }

    #[test]
    fn analyzer_rejects_invalid_control_flow_and_unclosed_methods() {
        let invalid_if =
            analyze_module("Procedure Run()\nIf True Then\nMessage(\"bad\");\nEndProcedure\n")
                .unwrap();
        assert!(!invalid_if.diagnostics.is_empty());

        let unclosed = analyze_module("Procedure Run()\n").unwrap();
        assert!(!unclosed.diagnostics.is_empty());
    }

    #[test]
    fn line_column_reports_utf8_character_columns() {
        assert_eq!(line_column("Процедура Run()\n", 0).unwrap(), (1, 1));
        assert_eq!(
            line_column("Процедура Run()\n", "Процедура ".len()).unwrap(),
            (1, 11)
        );
        assert_eq!(line_column("A\nBC", 3).unwrap(), (2, 2));
        assert!(line_column("Я", 1).is_err());
    }

    #[test]
    fn unified_diff_round_trips_crlf_and_missing_terminal_eol() {
        let before = "Procedure Run()\r\nEndProcedure";
        let after = "Procedure Run()\r\nEndProcedure\r\nProcedure Added()\r\nEndProcedure\r\n";
        let diff = unified_diff("src/CommonModules/X/Ext/Module.bsl", before, after).unwrap();
        let patch = Patch::from_str(&diff).unwrap();
        let rebuilt = apply(before, &patch).unwrap();

        assert_eq!(rebuilt.as_bytes(), after.as_bytes());
        assert!(diff.contains("\\ No newline at end of file"));
        assert!(diff.starts_with("--- a/src/CommonModules/X/Ext/Module.bsl\n"));
    }

    #[test]
    fn addressed_method_body_replaces_statements_on_the_closing_line() {
        for before in [
            "Procedure Base()\n    Value = 1; EndProcedure\n",
            "Procedure Base()\n    Value = 1;\n    Value = 3; EndProcedure\n",
        ] {
            let operation = CodePlanOperation::Replace(CodeReplaceArgs {
                at: unica_address::address::QualifiedAddress::parse(
                    "main:CommonModule.Sample.Method.Base.Body",
                )
                .unwrap(),
                text: "    Value = 2;".to_owned(),
                anchor: None,
            });
            let post = plan_code_postimage(before.as_bytes(), &operation, 0).unwrap();
            assert_eq!(
                post.after,
                b"Procedure Base()\n    Value = 2;\nEndProcedure\n"
            );
            let repeated = plan_code_postimage(&post.after, &operation, 0).unwrap();
            assert!(repeated.no_op);
            assert_eq!(repeated.after, post.after);
        }
    }

    #[test]
    fn v12_selector_adapter_preserves_exact_zero_and_multiple_messages() {
        let cases = [
            (
                "zero-method",
                "Procedure Base()\nEndProcedure\n",
                super::Selector::Method("Missing".to_string()),
                "method selector must match exactly once; matched 0 times",
            ),
            (
                "multiple-method",
                "Procedure Base()\nEndProcedure\nProcedure Base()\nEndProcedure\n",
                super::Selector::Method("Base".to_string()),
                "method selector must match exactly once; matched 2 times",
            ),
            (
                "zero-anchor",
                "Procedure Base()\nEndProcedure\n",
                super::Selector::Anchor("// marker".to_string()),
                "anchor selector must match exactly once; matched 0 times",
            ),
            (
                "multiple-anchor",
                "Procedure Base()\n    // marker\n    // marker\nEndProcedure\n",
                super::Selector::Anchor("// marker".to_string()),
                "anchor selector must match exactly once; matched 2 times",
            ),
        ];
        for (name, source, selector, expected) in cases {
            let snapshot = SourceTextSnapshot::from_bytes(source.as_bytes()).unwrap();
            let indexed = analyze_module(source).unwrap();
            assert!(indexed.diagnostics.is_empty(), "{name}");
            assert_eq!(
                super::locate_selector(&snapshot, Position::After, &selector, &indexed.methods,)
                    .unwrap_err(),
                expected,
                "insert {name}"
            );
            assert_eq!(
                super::locate_replacement(&snapshot, &selector, &indexed.methods).unwrap_err(),
                expected,
                "replace {name}"
            );
        }
    }

    #[test]
    fn diff_and_post_hash_are_derived_from_the_same_postimage() {
        let before = "Procedure Run()\nEndProcedure\n";
        let after = "Procedure Run()\nEndProcedure\nProcedure Added()\nEndProcedure\n";
        let diff = unified_diff("src/CommonModules/X/Ext/Module.bsl", before, after).unwrap();
        let rebuilt = apply(before, &Patch::from_str(&diff).unwrap()).unwrap();

        assert_eq!(hash(rebuilt.as_bytes()), hash(after.as_bytes()));
    }
}
