//! The one place where LSP positions meet Unica text coordinates.
//!
//! LSP counts lines from zero and characters in UTF-16 code units; Unica
//! counts lines from one (`Body#L205`) and addresses text by UTF-8 bytes.
//! A line ends at `\n`, `\r\n` or a lone `\r`, as LSP defines it.

use std::cmp::Ordering;

/// An LSP position: zero-based line, UTF-16 code units into that line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Utf16Position {
    pub(crate) line: u32,
    pub(crate) character: u32,
}

/// A Unica text point: one-based line and the UTF-8 byte column in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UnicaPoint {
    pub(crate) line: usize,
    pub(crate) byte_column: usize,
}

/// Why a position does not name a point of the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PositionError {
    /// The line is past the end of the text.
    LineOutOfRange,
    /// The position falls between the two units of a surrogate pair.
    InsideSurrogatePair,
}

/// Line starts of one text, built once per text version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LineIndex {
    /// Byte offset where each line starts, and the text length at the end.
    starts: Vec<usize>,
}

impl LineIndex {
    pub(crate) fn new(text: &str) -> Self {
        let bytes = text.as_bytes();
        let mut starts = vec![0];
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'\n' => starts.push(index + 1),
                b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                    index += 1;
                    starts.push(index + 1);
                }
                b'\r' => starts.push(index + 1),
                _ => {}
            }
            index += 1;
        }
        Self { starts }
    }

    pub(crate) fn line_count(&self) -> usize {
        self.starts.len()
    }

    /// The byte range of `line` without its terminator.
    fn line_bytes(&self, text: &str, line: usize) -> Option<(usize, usize)> {
        let start = *self.starts.get(line)?;
        let mut end = self.starts.get(line + 1).copied().unwrap_or(text.len());
        let bytes = text.as_bytes();
        if end > start && bytes[end - 1] == b'\n' {
            end -= 1;
        }
        if end > start && bytes[end - 1] == b'\r' {
            end -= 1;
        }
        Some((start, end))
    }

    /// The byte offset of an LSP position. A character past the end of its
    /// line names the line end, as LSP requires; a line past the text does
    /// not exist.
    pub(crate) fn offset(
        &self,
        text: &str,
        position: Utf16Position,
    ) -> Result<usize, PositionError> {
        let line = usize::try_from(position.line).map_err(|_| PositionError::LineOutOfRange)?;
        let (start, end) = self
            .line_bytes(text, line)
            .ok_or(PositionError::LineOutOfRange)?;
        let wanted = position.character as usize;
        let mut units = 0;
        for (offset, character) in text[start..end].char_indices() {
            match units.cmp(&wanted) {
                Ordering::Equal => return Ok(start + offset),
                Ordering::Greater => return Err(PositionError::InsideSurrogatePair),
                Ordering::Less => units += character.len_utf16(),
            }
        }
        match units.cmp(&wanted) {
            Ordering::Greater => Err(PositionError::InsideSurrogatePair),
            _ => Ok(end),
        }
    }

    /// The LSP position of a byte offset, which must lie on a character
    /// boundary of the text. An offset inside a line terminator names the
    /// end of that line.
    pub(crate) fn position(&self, text: &str, offset: usize) -> Utf16Position {
        let offset = offset.min(text.len());
        let line = match self.starts.binary_search(&offset) {
            Ok(line) => line,
            Err(next) => next - 1,
        };
        let (start, end) = self
            .line_bytes(text, line)
            .expect("an offset within the text has a line");
        let column = &text[start..offset.min(end)];
        Utf16Position {
            line: line as u32,
            character: column.encode_utf16().count() as u32,
        }
    }

    /// The Unica point of an LSP position.
    pub(crate) fn unica_point(
        &self,
        text: &str,
        position: Utf16Position,
    ) -> Result<UnicaPoint, PositionError> {
        let offset = self.offset(text, position)?;
        let line = position.line as usize;
        Ok(UnicaPoint {
            line: line + 1,
            byte_column: offset - self.starts[line],
        })
    }

    /// The LSP position of a Unica point; `None` when the line does not
    /// exist or the column is not a character boundary of it.
    pub(crate) fn lsp_position(&self, text: &str, point: UnicaPoint) -> Option<Utf16Position> {
        let line = point.line.checked_sub(1)?;
        let (start, end) = self.line_bytes(text, line)?;
        let offset = start.checked_add(point.byte_column)?;
        (offset <= end && text.is_char_boundary(offset)).then(|| self.position(text, offset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(line: u32, character: u32) -> Utf16Position {
        Utf16Position { line, character }
    }

    #[test]
    fn cyrillic_and_surrogate_pairs_round_trip_through_utf16() {
        // «Процедура» is two bytes and one UTF-16 unit per letter; the emoji
        // is four bytes and a surrogate pair of two units.
        let text = "Процедура А()\r\n\t// 😀 Б\nКонецПроцедуры";
        let index = LineIndex::new(text);
        assert_eq!(index.line_count(), 3);

        let before_a = index.offset(text, at(0, 10)).unwrap();
        assert_eq!(&text[before_a..before_a + 2], "А");
        assert_eq!(index.position(text, before_a), at(0, 10));

        // Line 1: tab, "//", space, emoji (2 units), space, "Б".
        let emoji = index.offset(text, at(1, 4)).unwrap();
        assert_eq!(&text[emoji..emoji + 4], "😀");
        assert_eq!(
            index.offset(text, at(1, 5)),
            Err(PositionError::InsideSurrogatePair)
        );
        let after_emoji = index.offset(text, at(1, 6)).unwrap();
        assert_eq!(after_emoji, emoji + 4);
        assert_eq!(index.position(text, after_emoji), at(1, 6));
        let b = index.offset(text, at(1, 7)).unwrap();
        assert_eq!(&text[b..b + 2], "Б");

        assert_eq!(
            index.unica_point(text, at(1, 7)).unwrap(),
            UnicaPoint {
                line: 2,
                byte_column: b - (text.find('\t').unwrap())
            }
        );
        assert_eq!(
            index.lsp_position(
                text,
                UnicaPoint {
                    line: 2,
                    byte_column: b - text.find('\t').unwrap()
                }
            ),
            Some(at(1, 7))
        );
        // A column inside the emoji is not a character boundary.
        assert_eq!(
            index.lsp_position(
                text,
                UnicaPoint {
                    line: 2,
                    byte_column: emoji - text.find('\t').unwrap() + 1
                }
            ),
            None
        );
    }

    #[test]
    fn line_ends_follow_lsp_and_a_long_character_clamps_to_the_line() {
        let text = "a\rb\r\nc\nd";
        let index = LineIndex::new(text);
        assert_eq!(index.line_count(), 4);
        for (line, letter) in ["a", "b", "c", "d"].iter().enumerate() {
            let offset = index.offset(text, at(line as u32, 0)).unwrap();
            assert_eq!(&text[offset..offset + 1], *letter);
        }
        // Past the end of a line: the line end, before its terminator.
        assert_eq!(index.offset(text, at(1, 99)), Ok(3));
        assert_eq!(
            index.offset(text, at(4, 0)),
            Err(PositionError::LineOutOfRange)
        );
        // The end of the text and an offset inside "\r\n".
        assert_eq!(index.position(text, text.len()), at(3, 1));
        assert_eq!(index.position(text, 4), at(1, 1));
    }

    #[test]
    fn an_empty_text_has_one_empty_line() {
        let index = LineIndex::new("");
        assert_eq!(index.line_count(), 1);
        assert_eq!(index.offset("", at(0, 0)), Ok(0));
        assert_eq!(index.position("", 0), at(0, 0));
        let trailing = LineIndex::new("x\n");
        assert_eq!(trailing.line_count(), 2);
        assert_eq!(trailing.offset("x\n", at(1, 0)), Ok(2));
    }
}
