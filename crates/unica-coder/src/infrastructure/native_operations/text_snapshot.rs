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
        let (bom, content_start) = if let Some(without_bom) = raw.strip_prefix(UTF8_BOM) {
            if without_bom.starts_with(UTF8_BOM) {
                return Err(SnapshotError::DuplicateUtf8Bom);
            }
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

    pub(crate) fn text(&self) -> &str {
        &self.decoded_text[self.content_start..]
    }

    pub(crate) fn decoded_text(&self) -> &str {
        &self.decoded_text
    }

    pub(crate) fn bom(&self) -> Utf8Bom {
        self.bom
    }

    pub(crate) fn line_endings(&self) -> LineEndingProfile {
        self.line_endings
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineEndingProfile {
    None,
    Uniform(LineEnding),
    Mixed { lf: usize, crlf: usize, cr: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SnapshotError {
    InvalidUtf8,
    DuplicateUtf8Bom,
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUtf8 => formatter.write_str("source is not valid UTF-8"),
            Self::DuplicateUtf8Bom => {
                formatter.write_str("source contains more than one UTF-8 BOM")
            }
        }
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
    use super::{LineEnding, LineEndingProfile, SnapshotError, SourceTextSnapshot, Utf8Bom};

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
    fn snapshot_rejects_duplicate_bom() {
        let raw = b"\xef\xbb\xbf\xef\xbb\xbfProcedure Run()\nEndProcedure\n";

        assert_eq!(
            SourceTextSnapshot::from_bytes(raw),
            Err(SnapshotError::DuplicateUtf8Bom)
        );
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
