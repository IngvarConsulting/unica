//! An immutable, anonymous on-disk copy of a BSL Body. The cursor retains a
//! byte position, not the source text or a vector of every line.

use crate::application::invocation_store::MAX_CANONICAL_RESULT_BYTES;
use crate::application::v13::view::ViewError;
use crate::domain::refusal::RefusalDetail;
use serde_json::{json, Value};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::sync::Mutex;

// A complete line is kept when it can safely fit in one canonical response.
// Longer lines are reversible UTF-8 fragments, independent of total file size.
const WHOLE_LINE_BYTES: u64 = (MAX_CANONICAL_RESULT_BYTES - 4096) as u64;
const FRAGMENT_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub(crate) struct BodySnapshot {
    file: Mutex<File>,
    len: u64,
    bom_prefix_len: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BodyPosition {
    pub(crate) byte: u64,
    pub(crate) line_start: u64,
    pub(crate) line: u64,
    pub(crate) text_end: Option<u64>,
    pub(crate) next_line: Option<u64>,
}

impl Default for BodyPosition {
    fn default() -> Self {
        Self {
            byte: 0,
            line_start: 0,
            line: 1,
            text_end: None,
            next_line: None,
        }
    }
}

impl BodySnapshot {
    pub(crate) fn new(mut file: File, len: u64) -> Self {
        let mut bom_prefix_len = 0_u64;
        if file.seek(SeekFrom::Start(0)).is_ok() {
            let mut reader = BufReader::new(&mut file);
            while len.saturating_sub(bom_prefix_len) >= 3 {
                let mut head = [0_u8; 3];
                if reader.read_exact(&mut head).is_err() || head != [0xef, 0xbb, 0xbf] {
                    break;
                }
                bom_prefix_len += 3;
            }
        }
        let len = if bom_prefix_len == len { 0 } else { len };
        Self {
            file: Mutex::new(file),
            len,
            bom_prefix_len,
        }
    }

    pub(crate) fn has_more(&self, position: BodyPosition) -> bool {
        position.byte < self.len
    }

    pub(crate) fn next_item(
        &self,
        mut position: BodyPosition,
        mut checkpoint: impl FnMut() -> Result<(), ViewError>,
    ) -> Result<Option<(Value, BodyPosition)>, ViewError> {
        checkpoint()?;
        if position.byte >= self.len {
            return Ok(None);
        }
        let mut file = self.file.lock().map_err(|_| {
            ViewError::detailed(
                RefusalDetail::CachePoisoned,
                "Body snapshot lock is poisoned",
            )
        })?;
        if position.text_end.is_none() {
            let start = position.byte;
            file.seek(SeekFrom::Start(start)).map_err(read_error)?;
            let mut buffer = [0_u8; 64 * 1024];
            let mut end = start;
            let mut next_line = self.len;
            loop {
                checkpoint()?;
                let count = file.read(&mut buffer).map_err(read_error)?;
                if count == 0 {
                    break;
                }
                if let Some(index) = buffer[..count].iter().position(|byte| *byte == b'\n') {
                    end += index as u64;
                    next_line = end + 1;
                    break;
                }
                end += count as u64;
            }
            if next_line == end + 1 && end > start {
                file.seek(SeekFrom::Start(end - 1)).map_err(read_error)?;
                let mut tail = [0_u8; 1];
                file.read_exact(&mut tail).map_err(read_error)?;
                if tail[0] == b'\r' {
                    end -= 1;
                }
            }
            // Match module_source(): every leading U+FEFF is omitted, not
            // only the first UTF-8 BOM triplet.
            if start == 0 {
                position.byte = self.bom_prefix_len;
                position.line_start = self.bom_prefix_len;
            }
            position.text_end = Some(end);
            position.next_line = Some(next_line);
        }
        let end = position.text_end.expect("line end was established");
        let next_line = position.next_line.expect("line successor was established");
        let remaining = end.saturating_sub(position.byte);
        let mut fragment = end.saturating_sub(position.line_start) > WHOLE_LINE_BYTES;
        let requested = if fragment {
            remaining.min(FRAGMENT_BYTES as u64)
        } else {
            remaining
        };
        let mut bytes = vec![0_u8; requested as usize];
        file.seek(SeekFrom::Start(position.byte))
            .map_err(read_error)?;
        for chunk in bytes.chunks_mut(64 * 1024) {
            checkpoint()?;
            file.read_exact(chunk).map_err(read_error)?;
        }
        if !fragment
            && serde_json::to_vec(std::str::from_utf8(&bytes).map_err(|_| {
                ViewError::detailed(RefusalDetail::SourceUnreadable, "BSL module is not UTF-8")
            })?)
            .map_or(true, |json| json.len() > WHOLE_LINE_BYTES as usize)
        {
            fragment = true;
            bytes.truncate(FRAGMENT_BYTES.min(bytes.len()));
        }
        // For fragmented lines, avoid splitting a UTF-8 scalar.
        while fragment && std::str::from_utf8(&bytes).is_err() {
            if bytes.is_empty() {
                return Err(read_error(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid BSL UTF-8",
                )));
            }
            bytes.pop();
        }
        let text = String::from_utf8(bytes).map_err(|_| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, "BSL module is not UTF-8")
        })?;
        let consumed = text.len() as u64;
        let result = if fragment {
            json!({"line": position.line, "text": text, "byteOffset": position.byte.saturating_sub(position.line_start), "endOfLine": position.byte + consumed == end})
        } else {
            json!({"line": position.line, "text": text})
        };
        if position.byte + consumed >= end {
            position.byte = next_line;
            position.line_start = next_line;
            position.line = position.line.saturating_add(1);
            position.text_end = None;
            position.next_line = None;
        } else {
            position.byte += consumed;
        }
        Ok(Some((result, position)))
    }
}

fn read_error(error: std::io::Error) -> ViewError {
    ViewError::detailed(
        RefusalDetail::BackendBroken,
        format!("Body snapshot read failed: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::{BodyPosition, BodySnapshot};
    use crate::application::v13::view::ViewError;
    use crate::domain::refusal::RefusalCode;
    use std::io::{Seek, SeekFrom, Write};

    #[test]
    fn fragments_reassemble_utf8_line_and_preserve_crlf_boundaries() {
        let mut file = tempfile::tempfile().unwrap();
        let mut source = Vec::new();
        source.extend_from_slice(b"\xef\xbb\xbf");
        source.extend_from_slice("Я".repeat(5 * 1024 * 1024).as_bytes());
        source.extend_from_slice(b"\r\nend\n");
        file.write_all(&source).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        let snapshot = BodySnapshot::new(file, source.len() as u64);
        let mut position = BodyPosition::default();
        let mut reconstructed = String::new();
        let mut offsets = Vec::new();
        let mut line_two = None;
        while let Some((item, next)) = snapshot.next_item(position, || Ok(())).unwrap() {
            if item["line"] == 1 {
                offsets.push(item["byteOffset"].as_u64().unwrap());
                reconstructed.push_str(item["text"].as_str().unwrap());
                if item["endOfLine"] == true {
                    assert_eq!(next.line, 2);
                }
            } else {
                line_two = Some(item);
            }
            position = next;
        }
        assert_eq!(reconstructed, "Я".repeat(5 * 1024 * 1024));
        assert!(offsets.len() > 1);
        assert!(offsets.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(line_two.unwrap()["text"], "end");
    }

    #[test]
    fn repeated_leading_bom_matches_module_source_and_bom_only_is_empty() {
        let mut file = tempfile::tempfile().unwrap();
        let source = b"\xef\xbb\xbf\xef\xbb\xbf\xef\xbb\xbfProcedure A()\nEndProcedure";
        file.write_all(source).unwrap();
        let snapshot = BodySnapshot::new(file, source.len() as u64);
        let (first, next) = snapshot
            .next_item(BodyPosition::default(), || Ok(()))
            .unwrap()
            .unwrap();
        assert_eq!(first["text"], "Procedure A()");
        assert_eq!(first["line"], 1);
        let (second, end) = snapshot.next_item(next, || Ok(())).unwrap().unwrap();
        assert_eq!(second["text"], "EndProcedure");
        assert!(!snapshot.has_more(end));

        let mut only_bom = tempfile::tempfile().unwrap();
        only_bom.write_all(b"\xef\xbb\xbf\xef\xbb\xbf").unwrap();
        let empty = BodySnapshot::new(only_bom, 6);
        assert!(!empty.has_more(BodyPosition::default()));
        assert!(empty
            .next_item(BodyPosition::default(), || Ok(()))
            .unwrap()
            .is_none());
    }

    #[test]
    fn long_line_scan_honors_cancellation_before_returning_a_partial_item() {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(&vec![b'a'; 9 * 1024 * 1024]).unwrap();
        let snapshot = BodySnapshot::new(file, 9 * 1024 * 1024);
        let mut checkpoints = 0;
        let error = snapshot
            .next_item(BodyPosition::default(), || {
                checkpoints += 1;
                if checkpoints == 3 {
                    Err(ViewError::new(RefusalCode::Cancelled, "cancelled"))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        assert_eq!(error.code(), RefusalCode::Cancelled);
    }
}
