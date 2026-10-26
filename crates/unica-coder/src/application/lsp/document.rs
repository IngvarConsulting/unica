//! One open editor buffer: its text and the version the client gave it.
//!
//! Changes arrive as LSP `didChange` content changes, whole or by range;
//! ranges are UTF-16 positions resolved through [`LineIndex`]. A change set
//! applies atomically: an invalid range leaves the buffer as it was.

use super::position::{LineIndex, PositionError, Utf16Position};

/// One LSP content change: the whole text, or a range and its replacement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ContentChange {
    Full(String),
    Range {
        start: Utf16Position,
        end: Utf16Position,
        text: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DocumentChangeError {
    /// The client's version does not move forward.
    StaleVersion { current: i32, offered: i32 },
    /// A range does not name a span of the text.
    Range(PositionError),
    /// The range ends before it starts.
    ReversedRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpenDocument {
    version: i32,
    text: String,
    lines: LineIndex,
}

impl OpenDocument {
    pub(crate) fn new(version: i32, text: String) -> Self {
        let lines = LineIndex::new(&text);
        Self {
            version,
            text,
            lines,
        }
    }

    pub(crate) fn version(&self) -> i32 {
        self.version
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn lines(&self) -> &LineIndex {
        &self.lines
    }

    /// Apply the changes of one `didChange` in order and take `version`.
    pub(crate) fn change(
        &mut self,
        version: i32,
        changes: &[ContentChange],
    ) -> Result<(), DocumentChangeError> {
        if version <= self.version {
            return Err(DocumentChangeError::StaleVersion {
                current: self.version,
                offered: version,
            });
        }
        let mut text = self.text.clone();
        let mut lines = self.lines.clone();
        for change in changes {
            match change {
                ContentChange::Full(whole) => text.clone_from(whole),
                ContentChange::Range {
                    start,
                    end,
                    text: replacement,
                } => {
                    let from = lines
                        .offset(&text, *start)
                        .map_err(DocumentChangeError::Range)?;
                    let to = lines
                        .offset(&text, *end)
                        .map_err(DocumentChangeError::Range)?;
                    if to < from {
                        return Err(DocumentChangeError::ReversedRange);
                    }
                    text.replace_range(from..to, replacement);
                }
            }
            lines = LineIndex::new(&text);
        }
        self.version = version;
        self.text = text;
        self.lines = lines;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(line: u32, character: u32) -> Utf16Position {
        Utf16Position { line, character }
    }

    #[test]
    fn incremental_changes_apply_in_order_by_utf16_ranges() {
        let mut document = OpenDocument::new(1, "Сообщить(\"😀 мир\");\n".to_owned());
        document
            .change(
                2,
                &[
                    // Replace «мир» after the surrogate pair: «Сообщить("» is 10
                    // units, the emoji 2, the space 1.
                    ContentChange::Range {
                        start: at(0, 13),
                        end: at(0, 16),
                        text: "Unica".to_owned(),
                    },
                    // Then insert a second line at the start of the empty last
                    // line, against the text the first change produced.
                    ContentChange::Range {
                        start: at(1, 0),
                        end: at(1, 0),
                        text: "Возврат;".to_owned(),
                    },
                ],
            )
            .unwrap();
        assert_eq!(document.text(), "Сообщить(\"😀 Unica\");\nВозврат;");
        assert_eq!(document.version(), 2);
        assert_eq!(document.lines().line_count(), 2);
    }

    #[test]
    fn a_full_change_replaces_the_text_and_a_bad_change_keeps_the_buffer() {
        let mut document = OpenDocument::new(3, "a\nb".to_owned());
        assert_eq!(
            document.change(3, &[ContentChange::Full("x".to_owned())]),
            Err(DocumentChangeError::StaleVersion {
                current: 3,
                offered: 3
            })
        );
        let broken = document.change(
            4,
            &[
                ContentChange::Full("x😀".to_owned()),
                ContentChange::Range {
                    start: at(0, 2),
                    end: at(0, 3),
                    text: String::new(),
                },
            ],
        );
        assert_eq!(
            broken,
            Err(DocumentChangeError::Range(
                PositionError::InsideSurrogatePair
            ))
        );
        assert_eq!((document.version(), document.text()), (3, "a\nb"));
        assert_eq!(
            document.change(
                4,
                &[ContentChange::Range {
                    start: at(1, 1),
                    end: at(0, 0),
                    text: String::new(),
                }]
            ),
            Err(DocumentChangeError::ReversedRange)
        );
        document
            .change(5, &[ContentChange::Full("целиком".to_owned())])
            .unwrap();
        assert_eq!((document.version(), document.text()), (5, "целиком"));
    }
}
