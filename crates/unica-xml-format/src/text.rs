use std::fmt;

const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTextSnapshot {
    raw: Vec<u8>,
    decoded_text: String,
    content_start: usize,
    bom: Utf8Bom,
    line_endings: LineEndingProfile,
    terminal_line_ending: Option<LineEnding>,
}

impl SourceTextSnapshot {
    pub fn from_bytes(raw: &[u8]) -> Result<Self, SnapshotError> {
        let (bom, content_start) = if raw.starts_with(UTF8_BOM) {
            (Utf8Bom::Present, UTF8_BOM.len())
        } else {
            (Utf8Bom::Absent, 0)
        };
        let decoded_text = std::str::from_utf8(raw)
            .map_err(|_| SnapshotError::InvalidUtf8)?
            .to_owned();
        let text = decoded_text
            .get(content_start..)
            .ok_or(SnapshotError::InvalidUtf8)?;
        let (line_endings, terminal_line_ending) = classify_line_endings(text);

        Ok(Self {
            raw: raw.to_vec(),
            decoded_text,
            content_start,
            bom,
            line_endings,
            terminal_line_ending,
        })
    }

    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// The decoded text without a leading byte order mark, so a caller can ask
    /// whether the module holds content rather than merely bytes.
    pub fn text(&self) -> &str {
        &self.decoded_text[self.content_start..]
    }

    pub fn decoded_text(&self) -> &str {
        &self.decoded_text
    }

    pub fn bom(&self) -> Utf8Bom {
        self.bom
    }

    pub fn line_endings(&self) -> LineEndingProfile {
        self.line_endings
    }

    #[allow(dead_code)]
    pub fn terminal_line_ending(&self) -> Option<LineEnding> {
        self.terminal_line_ending
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Utf8Bom {
    Present,
    Absent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
    Cr,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
            Self::Cr => "\r",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEndingProfile {
    None,
    Uniform(LineEnding),
    Mixed { lf: usize, crlf: usize, cr: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    InvalidUtf8,
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUtf8 => formatter.write_str("source is not valid UTF-8"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum EolPolicy {
    Preserve,
    Lf,
    CrLf,
    Repository,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EolPolicyError {
    AmbiguousPreservePolicy,
    MissingPreserveContext,
    RepositoryPolicyUnresolved,
}

impl fmt::Display for EolPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AmbiguousPreservePolicy => formatter.write_str(
                "preserve EOL policy is ambiguous for mixed line endings without local context",
            ),
            Self::MissingPreserveContext => {
                formatter.write_str("preserve EOL policy requires local or uniform source context")
            }
            Self::RepositoryPolicyUnresolved => {
                formatter.write_str("repository EOL policy is unresolved")
            }
        }
    }
}

/// Писательский контракт INV-SOURCE-OBSERVED-EOL в одной точке: источник без
/// переводов строк обслуживается явной политикой `Lf`, любой наблюдённый
/// профиль — политикой `Preserve` (для смешанного без локального контекста это
/// отказ, а не выбор за автора файла).
pub fn resolve_observed_line_ending(
    snapshot: &SourceTextSnapshot,
    local: Option<LineEnding>,
) -> Result<LineEnding, EolPolicyError> {
    let policy = match snapshot.line_endings() {
        LineEndingProfile::None => EolPolicy::Lf,
        LineEndingProfile::Uniform(_) | LineEndingProfile::Mixed { .. } => EolPolicy::Preserve,
    };
    resolve_line_ending(policy, snapshot, local)
}

pub fn resolve_line_ending(
    policy: EolPolicy,
    snapshot: &SourceTextSnapshot,
    local: Option<LineEnding>,
) -> Result<LineEnding, EolPolicyError> {
    match policy {
        EolPolicy::Lf => Ok(LineEnding::Lf),
        EolPolicy::CrLf => Ok(LineEnding::CrLf),
        EolPolicy::Repository => Err(EolPolicyError::RepositoryPolicyUnresolved),
        EolPolicy::Preserve => match (local, snapshot.line_endings()) {
            (Some(line_ending), _) => Ok(line_ending),
            (None, LineEndingProfile::Uniform(line_ending)) => Ok(line_ending),
            (None, LineEndingProfile::Mixed { .. }) => Err(EolPolicyError::AmbiguousPreservePolicy),
            (None, LineEndingProfile::None) => Err(EolPolicyError::MissingPreserveContext),
        },
    }
}

fn classify_line_endings(text: &str) -> (LineEndingProfile, Option<LineEnding>) {
    let bytes = text.as_bytes();
    let mut lf = 0;
    let mut crlf = 0;
    let mut cr = 0;
    let mut offset = 0;

    while offset < bytes.len() {
        match bytes[offset] {
            b'\r' if bytes.get(offset + 1) == Some(&b'\n') => {
                crlf += 1;
                offset += 2;
            }
            b'\r' => {
                cr += 1;
                offset += 1;
            }
            b'\n' => {
                lf += 1;
                offset += 1;
            }
            _ => offset += 1,
        }
    }

    let profile = match (lf, crlf, cr) {
        (0, 0, 0) => LineEndingProfile::None,
        (lf, 0, 0) if lf > 0 => LineEndingProfile::Uniform(LineEnding::Lf),
        (0, crlf, 0) if crlf > 0 => LineEndingProfile::Uniform(LineEnding::CrLf),
        (0, 0, cr) if cr > 0 => LineEndingProfile::Uniform(LineEnding::Cr),
        (lf, crlf, cr) => LineEndingProfile::Mixed { lf, crlf, cr },
    };
    let terminal = if bytes.ends_with(b"\r\n") {
        Some(LineEnding::CrLf)
    } else if bytes.ends_with(b"\n") {
        Some(LineEnding::Lf)
    } else if bytes.ends_with(b"\r") {
        Some(LineEnding::Cr)
    } else {
        None
    };

    (profile, terminal)
}

#[cfg(test)]
mod tests {
    use super::{
        resolve_line_ending, resolve_observed_line_ending, EolPolicy, EolPolicyError, LineEnding,
        LineEndingProfile, SnapshotError, SourceTextSnapshot, Utf8Bom,
    };

    #[test]
    fn snapshot_preserves_raw_bytes_and_excludes_one_bom_from_text() {
        let raw = b"\xef\xbb\xbfProcedure Run()\r\nEndProcedure\r\n";

        let snapshot = SourceTextSnapshot::from_bytes(raw).unwrap();

        assert_eq!(snapshot.raw(), raw);
        assert_eq!(snapshot.text(), "Procedure Run()\r\nEndProcedure\r\n");
        assert_eq!(
            snapshot.decoded_text(),
            "\u{feff}Procedure Run()\r\nEndProcedure\r\n"
        );
        assert_eq!(snapshot.bom(), Utf8Bom::Present);
    }

    #[test]
    fn snapshot_without_bom_reports_bom_absent() {
        let snapshot = SourceTextSnapshot::from_bytes(b"Procedure Run()\n").unwrap();

        assert_eq!(snapshot.bom(), Utf8Bom::Absent);
        assert_eq!(snapshot.text(), snapshot.decoded_text());
    }

    #[test]
    fn snapshot_treats_only_the_first_bom_as_preamble() {
        let raw = b"\xef\xbb\xbf\xef\xbb\xbfProcedure Run()\nEndProcedure\n";
        let snapshot = SourceTextSnapshot::from_bytes(raw).unwrap();

        assert_eq!(snapshot.raw(), raw);
        assert_eq!(
            snapshot.decoded_text(),
            "\u{feff}\u{feff}Procedure Run()\nEndProcedure\n"
        );
        assert_eq!(snapshot.text(), "\u{feff}Procedure Run()\nEndProcedure\n");
        assert_eq!(snapshot.bom(), Utf8Bom::Present);
    }

    #[test]
    fn snapshot_rejects_invalid_utf8() {
        assert_eq!(
            SourceTextSnapshot::from_bytes(&[0xff, 0xfe]),
            Err(SnapshotError::InvalidUtf8)
        );
    }

    #[test]
    fn snapshot_classifies_no_line_endings() {
        assert_line_endings("", LineEndingProfile::None, None);
    }

    #[test]
    fn snapshot_classifies_uniform_lf_and_terminal_newline() {
        assert_line_endings(
            "A\nB\n",
            LineEndingProfile::Uniform(LineEnding::Lf),
            Some(LineEnding::Lf),
        );
    }

    #[test]
    fn snapshot_classifies_uniform_crlf_and_terminal_newline() {
        assert_line_endings(
            "A\r\nB\r\n",
            LineEndingProfile::Uniform(LineEnding::CrLf),
            Some(LineEnding::CrLf),
        );
    }

    #[test]
    fn snapshot_classifies_uniform_cr_and_terminal_newline() {
        assert_line_endings(
            "A\rB\r",
            LineEndingProfile::Uniform(LineEnding::Cr),
            Some(LineEnding::Cr),
        );
    }

    #[test]
    fn snapshot_classifies_mixed_endings_with_exact_counts() {
        assert_line_endings(
            "A\r\nB\nC\r",
            LineEndingProfile::Mixed {
                lf: 1,
                crlf: 1,
                cr: 1,
            },
            Some(LineEnding::Cr),
        );
    }

    #[test]
    fn snapshot_reports_missing_terminal_newline() {
        assert_line_endings("A\nB", LineEndingProfile::Uniform(LineEnding::Lf), None);
    }

    #[test]
    fn explicit_policies_ignore_source_profile() {
        let snapshot = SourceTextSnapshot::from_bytes(b"A\r\nB\n").unwrap();

        assert_eq!(
            resolve_line_ending(EolPolicy::Lf, &snapshot, None),
            Ok(LineEnding::Lf)
        );
        assert_eq!(
            resolve_line_ending(EolPolicy::CrLf, &snapshot, None),
            Ok(LineEnding::CrLf)
        );
    }

    #[test]
    fn preserve_prefers_local_context_for_mixed_source() {
        let snapshot = SourceTextSnapshot::from_bytes(b"A\r\nB\n").unwrap();

        assert_eq!(
            resolve_line_ending(EolPolicy::Preserve, &snapshot, Some(LineEnding::CrLf)),
            Ok(LineEnding::CrLf)
        );
    }

    #[test]
    fn preserve_uses_uniform_source_without_local_context() {
        let snapshot = SourceTextSnapshot::from_bytes(b"A\nB\n").unwrap();

        assert_eq!(
            resolve_line_ending(EolPolicy::Preserve, &snapshot, None),
            Ok(LineEnding::Lf)
        );
    }

    #[test]
    fn preserve_rejects_ambiguous_or_missing_context() {
        let mixed = SourceTextSnapshot::from_bytes(b"A\r\nB\n").unwrap();
        assert_eq!(
            resolve_line_ending(EolPolicy::Preserve, &mixed, None),
            Err(EolPolicyError::AmbiguousPreservePolicy)
        );
        let empty = SourceTextSnapshot::from_bytes(b"A").unwrap();
        assert_eq!(
            resolve_line_ending(EolPolicy::Preserve, &empty, None),
            Err(EolPolicyError::MissingPreserveContext)
        );
    }

    #[test]
    fn observed_resolution_serves_no_eol_source_with_explicit_lf() {
        let snapshot = SourceTextSnapshot::from_bytes(b"A").unwrap();

        assert_eq!(
            resolve_observed_line_ending(&snapshot, None),
            Ok(LineEnding::Lf)
        );
    }

    #[test]
    fn observed_resolution_preserves_uniform_profile_and_prefers_local() {
        let uniform = SourceTextSnapshot::from_bytes(b"A\r\nB\r\n").unwrap();
        assert_eq!(
            resolve_observed_line_ending(&uniform, None),
            Ok(LineEnding::CrLf)
        );

        let mixed = SourceTextSnapshot::from_bytes(b"A\r\nB\n").unwrap();
        assert_eq!(
            resolve_observed_line_ending(&mixed, Some(LineEnding::Lf)),
            Ok(LineEnding::Lf)
        );
    }

    #[test]
    fn observed_resolution_rejects_mixed_profile_without_local_context() {
        let mixed = SourceTextSnapshot::from_bytes(b"A\r\nB\n").unwrap();

        assert_eq!(
            resolve_observed_line_ending(&mixed, None),
            Err(EolPolicyError::AmbiguousPreservePolicy)
        );
    }

    #[test]
    fn repository_policy_is_fail_closed() {
        let snapshot = SourceTextSnapshot::from_bytes(b"A\n").unwrap();

        assert_eq!(
            resolve_line_ending(EolPolicy::Repository, &snapshot, None),
            Err(EolPolicyError::RepositoryPolicyUnresolved)
        );
    }

    fn assert_line_endings(
        text: &str,
        expected_profile: LineEndingProfile,
        expected_terminal: Option<LineEnding>,
    ) {
        let snapshot = SourceTextSnapshot::from_bytes(text.as_bytes()).unwrap();

        assert_eq!(snapshot.line_endings(), expected_profile);
        assert_eq!(snapshot.terminal_line_ending(), expected_terminal);
    }
}

pub fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn escape_xml_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn split_utf8_bom_prefix(bytes: &[u8]) -> (&[u8], &[u8]) {
    let mut offset = 0usize;
    while bytes[offset..].starts_with(UTF8_BOM) {
        offset += UTF8_BOM.len();
    }
    bytes.split_at(offset)
}
pub fn preserve_inserted_line_endings(source: &str, updated: &str) -> String {
    let line_ending = source_line_ending(source);
    if line_ending == "\n" {
        return updated.to_string();
    }

    let (prefix, _source_end, updated_end) = string_diff_bounds(source, updated);
    let changed = &updated[prefix..updated_end];
    let changed = replace_bare_lf(changed, line_ending);
    format!(
        "{}{}{}",
        &updated[..prefix],
        changed,
        &updated[updated_end..]
    )
}
fn source_line_ending(text: &str) -> &'static str {
    let bytes = text.as_bytes();
    if let Some(index) = bytes.iter().position(|byte| *byte == b'\n') {
        if index > 0 && bytes[index - 1] == b'\r' {
            "\r\n"
        } else {
            "\n"
        }
    } else if bytes.contains(&b'\r') {
        "\r"
    } else {
        "\n"
    }
}
fn replace_bare_lf(text: &str, line_ending: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut previous_was_cr = false;
    for character in text.chars() {
        if character == '\n' && !previous_was_cr {
            output.push_str(line_ending);
        } else {
            output.push(character);
        }
        previous_was_cr = character == '\r';
    }
    output
}
fn string_diff_bounds(before: &str, after: &str) -> (usize, usize, usize) {
    let mut prefix = common_prefix_len(before.as_bytes(), after.as_bytes());
    while prefix > 0 && (!before.is_char_boundary(prefix) || !after.is_char_boundary(prefix)) {
        prefix -= 1;
    }
    let max_suffix = before.len().min(after.len()).saturating_sub(prefix);
    let mut suffix = common_suffix_len(&before.as_bytes()[prefix..], &after.as_bytes()[prefix..])
        .min(max_suffix);
    while suffix > 0
        && (!before.is_char_boundary(before.len() - suffix)
            || !after.is_char_boundary(after.len() - suffix))
    {
        suffix -= 1;
    }
    (prefix, before.len() - suffix, after.len() - suffix)
}
pub fn common_prefix_len(left: &[u8], right: &[u8]) -> usize {
    left.iter()
        .zip(right)
        .take_while(|(left, right)| left == right)
        .count()
}
pub fn common_suffix_len(left: &[u8], right: &[u8]) -> usize {
    left.iter()
        .rev()
        .zip(right.iter().rev())
        .take_while(|(left, right)| left == right)
        .count()
}

pub fn unescape_xml(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&gt;", ">")
        .replace("&lt;", "<")
        .replace("&amp;", "&")
}

#[derive(Clone)]
pub struct Utf8TextSnapshot {
    pub raw: Vec<u8>,
    pub text: String,
}
pub fn utf8_bom_bytes(content: &str) -> Vec<u8> {
    let content = content.trim_start_matches('\u{feff}');
    let mut bytes = Vec::with_capacity(content.len() + 3);
    bytes.extend_from_slice(b"\xef\xbb\xbf");
    bytes.extend_from_slice(content.as_bytes());
    bytes
}
pub fn preserve_source_final_newline(mut output: String, source_text: &str) -> String {
    let source_final_newline = if source_text.ends_with("\r\n") {
        Some("\r\n")
    } else if source_text.ends_with('\n') {
        Some("\n")
    } else if source_text.ends_with('\r') {
        Some("\r")
    } else {
        None
    };

    match source_final_newline {
        Some(line_ending) if !output.ends_with('\n') && !output.ends_with('\r') => {
            output.push_str(line_ending);
        }
        None if output.ends_with("\r\n") => {
            output.truncate(output.len() - 2);
        }
        None if output.ends_with('\n') || output.ends_with('\r') => {
            output.pop();
        }
        _ => {}
    }
    output
}
pub fn lxml_tree_serialized_text(text: &str) -> String {
    let mut output = text.to_string();
    output = output.replace(" />", "/>");
    output = output.replace("\r\n", "\n");
    output = output.replace('\r', "&#13;");
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output
}
pub fn lxml_tree_serialized_text_like_source(text: &str, source_text: &str) -> String {
    let output = lxml_tree_serialized_text(text);
    if source_text.contains("\r\n") {
        output.replace('\n', "\r\n")
    } else {
        output
    }
}
pub fn lxml_tree_serialized_text_like_source_preserving_final_newline(
    text: &str,
    source_text: &str,
) -> String {
    preserve_source_final_newline(
        lxml_tree_serialized_text_like_source(text, source_text),
        source_text,
    )
}
pub fn lxml_parser_normalized_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}
pub fn ensure_trailing_lf(text: &str) -> String {
    if text.ends_with('\n') {
        text.to_string()
    } else {
        format!("{text}\n")
    }
}

pub fn find_element_bounds(
    xml_text: &str,
    local_name: &str,
    start: usize,
) -> Option<(usize, usize, usize, usize, String, String)> {
    for tag in [local_name.to_string(), format!("md:{local_name}")] {
        let open_needle = format!("<{tag}");
        let Some(open_rel) = xml_text[start..].find(&open_needle) else {
            continue;
        };
        let open_start = start + open_rel;
        let Some(open_end_rel) = xml_text[open_start..].find('>') else {
            continue;
        };
        let content_start = open_start + open_end_rel + 1;
        let close_tag = format!("</{tag}>");
        let Some(close_rel) = xml_text[content_start..].find(&close_tag) else {
            continue;
        };
        let close_start = content_start + close_rel;
        let close_end = close_start + close_tag.len();
        let open_tag = xml_text[open_start..content_start].to_string();
        return Some((
            open_start,
            content_start,
            close_start,
            close_end,
            open_tag,
            close_tag,
        ));
    }
    None
}

pub fn cfe_borrow_ensure_namespace_declaration(
    mut open_tag: String,
    prefix: &str,
    namespace: &str,
) -> String {
    let attribute_name = format!("xmlns:{prefix}");
    match cfe_borrow_xml_attribute_value_range(&open_tag, &attribute_name) {
        Ok(Some((value_start, value_end))) => {
            if &open_tag[value_start..value_end] != namespace {
                open_tag.replace_range(value_start..value_end, namespace);
            }
            return open_tag;
        }
        Err(_) => return open_tag,
        Ok(None) => {}
    }
    let insert_at = open_tag
        .rfind("/>")
        .or_else(|| open_tag.rfind('>'))
        .unwrap_or(open_tag.len());
    open_tag.insert_str(insert_at, &format!(" xmlns:{prefix}=\"{namespace}\""));
    open_tag
}
/// Malformed syntax of the requested attribute assignment or quoted value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttributeSyntaxError;

pub fn cfe_borrow_xml_attribute_value_range(
    open_tag: &str,
    attribute_name: &str,
) -> Result<Option<(usize, usize)>, AttributeSyntaxError> {
    let bytes = open_tag.as_bytes();
    let mut index = usize::from(bytes.first() == Some(&b'<'));
    while index < bytes.len() && !cfe_borrow_is_xml_space(bytes[index]) && bytes[index] != b'>' {
        index += 1;
    }

    while index < bytes.len() {
        while index < bytes.len() && cfe_borrow_is_xml_space(bytes[index]) {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] == b'>' || bytes[index] == b'/' {
            return Ok(None);
        }

        let name_start = index;
        while index < bytes.len()
            && !cfe_borrow_is_xml_space(bytes[index])
            && !matches!(bytes[index], b'=' | b'>' | b'/')
        {
            index += 1;
        }
        let is_target = &open_tag[name_start..index] == attribute_name;
        while index < bytes.len() && cfe_borrow_is_xml_space(bytes[index]) {
            index += 1;
        }
        if bytes.get(index) != Some(&b'=') {
            if is_target {
                return Err(AttributeSyntaxError);
            }
            continue;
        }
        index += 1;
        while index < bytes.len() && cfe_borrow_is_xml_space(bytes[index]) {
            index += 1;
        }
        let Some(quote @ (b'\'' | b'"')) = bytes.get(index).copied() else {
            if is_target {
                return Err(AttributeSyntaxError);
            }
            while index < bytes.len()
                && !cfe_borrow_is_xml_space(bytes[index])
                && bytes[index] != b'>'
            {
                index += 1;
            }
            continue;
        };
        index += 1;
        let value_start = index;
        while index < bytes.len() && bytes[index] != quote {
            index += 1;
        }
        if index >= bytes.len() {
            return if is_target {
                Err(AttributeSyntaxError)
            } else {
                Ok(None)
            };
        }
        if is_target {
            return Ok(Some((value_start, index)));
        }
        index += 1;
    }
    Ok(None)
}
pub fn cfe_borrow_is_xml_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}
#[derive(Clone, Debug)]
pub struct CfeBorrowTag {
    pub local_name: String,
    pub end: usize,
    pub self_closing: bool,
}
pub fn cfe_borrow_start_tag_at(text: &str, open: usize) -> Option<CfeBorrowTag> {
    let rest = text.get(open..)?;
    if !rest.starts_with('<')
        || rest.starts_with("</")
        || rest.starts_with("<?")
        || rest.starts_with("<!")
    {
        return None;
    }
    let after = open + 1;
    let mut name_end = after;
    for (idx, ch) in text[after..].char_indices() {
        if ch.is_whitespace() || ch == '>' || ch == '/' {
            break;
        }
        name_end = after + idx + ch.len_utf8();
    }
    if name_end == after {
        return None;
    }
    let raw_name = &text[after..name_end];
    let gt = text[open..].find('>')? + open;
    let open_tag = &text[open..=gt];
    Some(CfeBorrowTag {
        local_name: cfe_borrow_tag_local_name(raw_name).to_string(),
        end: gt + 1,
        self_closing: open_tag.trim_end().ends_with("/>"),
    })
}
pub fn cfe_borrow_tag_local_name(raw_name: &str) -> &str {
    raw_name
        .rsplit_once(':')
        .map(|(_, name)| name)
        .unwrap_or(raw_name)
}
pub fn cfe_borrow_find_start_tag(text: &str, local_name: &str, offset: usize) -> Option<usize> {
    let mut cursor = offset;
    while let Some(rel_open) = text[cursor..].find('<') {
        let open = cursor + rel_open;
        if let Some(tag) = cfe_borrow_start_tag_at(text, open) {
            if tag.local_name == local_name {
                return Some(open);
            }
            cursor = tag.end;
        } else {
            cursor = open + 1;
        }
    }
    None
}
pub fn form_edit_line_indent(line: &str) -> String {
    line.chars().take_while(|ch| *ch == '\t').collect()
}
pub fn form_edit_line_indent_at(xml_text: &str, pos: usize) -> String {
    let line_start = xml_text[..pos].rfind('\n').map(|idx| idx + 1).unwrap_or(0);
    form_edit_line_indent(&xml_text[line_start..pos])
}

pub fn reject_event_lone_cr(snapshot: &SourceTextSnapshot) -> Result<(), String> {
    if matches!(
        snapshot.line_endings(),
        LineEndingProfile::Uniform(LineEnding::Cr) | LineEndingProfile::Mixed { cr: 1.., .. }
    ) {
        Err("source contains a lone CR line ending".to_string())
    } else {
        Ok(())
    }
}

pub fn event_local_line_ending(text: &str, offset: usize) -> Result<Option<LineEnding>, String> {
    let bytes = text.as_bytes();
    let previous = bytes[..offset]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map(|newline| {
            if newline > 0 && bytes[newline - 1] == b'\r' {
                LineEnding::CrLf
            } else {
                LineEnding::Lf
            }
        });
    let next = bytes[offset..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|relative| offset + relative)
        .map(|newline| {
            if newline > 0 && bytes[newline - 1] == b'\r' {
                LineEnding::CrLf
            } else {
                LineEnding::Lf
            }
        });
    match (previous, next) {
        (Some(left), Some(right)) if left != right => {
            Err("local insertion EOL is ambiguous".to_string())
        }
        (Some(value), _) | (_, Some(value)) => Ok(Some(value)),
        (None, None) => Ok(None),
    }
}

pub fn event_line_start(text: &str, offset: usize) -> usize {
    text[..offset].rfind('\n').map_or(0, |newline| newline + 1)
}
