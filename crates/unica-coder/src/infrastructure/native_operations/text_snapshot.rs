use std::fmt;

const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceTextSnapshot {
    raw: Vec<u8>,
    decoded_text: String,
    content_start: usize,
    bom: Utf8Bom,
    line_endings: LineEndingProfile,
    terminal_line_ending: Option<LineEnding>,
}

impl SourceTextSnapshot {
    pub(crate) fn from_bytes(raw: &[u8]) -> Result<Self, SnapshotError> {
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

    pub(crate) fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// The decoded text without a leading byte order mark, so a caller can ask
    /// whether the module holds content rather than merely bytes.
    pub(crate) fn text(&self) -> &str {
        &self.decoded_text[self.content_start..]
    }

    pub(crate) fn decoded_text(&self) -> &str {
        &self.decoded_text
    }

    // Part of the shared writer contract; code.patch is intentionally the first
    // consumer and does not need every observation yet.
    #[allow(dead_code)]
    pub(crate) fn bom(&self) -> Utf8Bom {
        self.bom
    }

    pub(crate) fn line_endings(&self) -> LineEndingProfile {
        self.line_endings
    }

    #[allow(dead_code)]
    pub(crate) fn terminal_line_ending(&self) -> Option<LineEnding> {
        self.terminal_line_ending
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Utf8Bom {
    Present,
    Absent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineEnding {
    Lf,
    CrLf,
    Cr,
}

impl LineEnding {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
            Self::Cr => "\r",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineEndingProfile {
    None,
    Uniform(LineEnding),
    Mixed { lf: usize, crlf: usize, cr: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SnapshotError {
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
pub(crate) enum EolPolicy {
    Preserve,
    Lf,
    CrLf,
    Repository,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EolPolicyError {
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

pub(crate) fn resolve_line_ending(
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
        resolve_line_ending, EolPolicy, EolPolicyError, LineEnding, LineEndingProfile,
        SnapshotError, SourceTextSnapshot, Utf8Bom,
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
