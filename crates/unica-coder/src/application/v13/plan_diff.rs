//! Bounded, lossless windows of immutable prepared source images.
use crate::application::source_navigation::{authenticate_cursor, page_bounds_from_offset};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::ops::Range;

const CHUNK_BYTES: usize = 2048;
const PAGE_BYTES: usize = 64 * 1024;

pub(crate) struct SourceImage<'a> {
    pub(crate) path_encoding: &'static str,
    pub(crate) path: String,
    pub(crate) before: Option<&'a [u8]>,
    pub(crate) after: Option<&'a [u8]>,
}

struct Window {
    image: usize,
    after: bool,
    range: Range<usize>,
    utf8: bool,
}

pub(crate) struct PlanDiffLayout {
    windows: Vec<Window>,
    positions: usize,
}

impl PlanDiffLayout {
    pub(crate) fn new(
        images: &[SourceImage<'_>],
        checkpoint: impl Fn() -> Result<(), String>,
    ) -> Result<Self, String> {
        let mut windows = Vec::new();
        let mut positions = 0usize;
        let mut order: Vec<_> = (0..images.len()).collect();
        order.sort_by_key(|&index| (images[index].path_encoding, &images[index].path));
        for image in order {
            checkpoint()?;
            let source = &images[image];
            let before = source.before.unwrap_or_default();
            let after = source.after.unwrap_or_default();
            let common = before.len().min(after.len());
            let mut prefix = 0;
            while prefix < common && before[prefix] == after[prefix] {
                if prefix % 65536 == 0 {
                    checkpoint()?;
                }
                prefix += 1;
            }
            if source.before.is_some() == source.after.is_some()
                && prefix == before.len()
                && prefix == after.len()
            {
                continue;
            }
            let mut suffix = 0;
            while suffix < common - prefix
                && before[before.len() - suffix - 1] == after[after.len() - suffix - 1]
            {
                if suffix % 65536 == 0 {
                    checkpoint()?;
                }
                suffix += 1;
            }
            for (is_after, bytes) in [(false, before), (true, after)] {
                checkpoint()?;
                let text = std::str::from_utf8(bytes).ok();
                let mut range = prefix..bytes.len() - suffix;
                if let Some(text) = text {
                    // Byte equality may stop inside a multi-byte character.
                    while !text.is_char_boundary(range.start) {
                        range.start -= 1;
                    }
                    while !text.is_char_boundary(range.end) {
                        range.end += 1;
                    }
                    let mut lines = 0;
                    while range.start > 0 && lines < 3 {
                        range.start -= 1;
                        lines += usize::from(bytes[range.start] == b'\n');
                        if range.start % 65536 == 0 {
                            checkpoint()?;
                        }
                    }
                    let mut lines = 0;
                    while range.end < bytes.len() && lines < 3 {
                        lines += usize::from(bytes[range.end] == b'\n');
                        range.end += 1;
                        if range.end % 65536 == 0 {
                            checkpoint()?;
                        }
                    }
                }
                positions = positions
                    .checked_add(range.len().max(1))
                    .ok_or("saved plan source ranges exceed the supported address space")?;
                windows.push(Window {
                    image,
                    after: is_after,
                    range,
                    utf8: text.is_some(),
                });
            }
        }
        Ok(Self { windows, positions })
    }

    pub(crate) fn page(
        &self,
        images: &[SourceImage<'_>],
        key: &str,
        limit: usize,
        cursor: Option<&str>,
        checkpoint: impl Fn() -> Result<(), String>,
    ) -> Result<Value, String> {
        let offset = authenticate_cursor(cursor, key)?;
        if offset > self.positions {
            return Err("saved plan cursor is outside its ranges".into());
        }
        let mut position = 0;
        let mut consumed = 0;
        let mut size = 0;
        let mut chunks = Vec::new();
        let mut stopped_by = "complete";
        'windows: for window in &self.windows {
            checkpoint()?;
            let span = window.range.len().max(1);
            if position + span <= offset {
                position += span;
                continue;
            }
            let source = &images[window.image];
            let bytes = if window.after {
                source.after
            } else {
                source.before
            };
            let mut start = window.range.start + offset.saturating_sub(position);
            loop {
                checkpoint()?;
                let mut end = start.saturating_add(CHUNK_BYTES).min(window.range.end);
                let image = bytes.unwrap_or_default();
                let (encoding, content) = if window.utf8 {
                    let text = std::str::from_utf8(&image[start..end]);
                    if let Err(error) = text {
                        end = start + error.valid_up_to();
                    }
                    (
                        "utf8",
                        std::str::from_utf8(&image[start..end])
                            .expect("aligned UTF-8 range")
                            .to_owned(),
                    )
                } else {
                    ("base64", STANDARD.encode(&image[start..end]))
                };
                let chunk = json!({
                    "pathEncoding": source.path_encoding, "path": source.path, "side": if window.after { "after" } else { "before" },
                    "present": bytes.is_some(), "totalBytes": image.len(),
                    "rangeStart": window.range.start, "rangeEnd": window.range.end,
                    "startByte": start, "endByte": end, "encoding": encoding, "content": content
                });
                let encoded = serde_json::to_vec(&chunk)
                    .expect("JSON chunk is serializable")
                    .len();
                if encoded > PAGE_BYTES {
                    return Err("saved plan path exceeds the page byte limit".into());
                }
                if chunks.len() == limit {
                    stopped_by = "limit";
                    break 'windows;
                }
                if !chunks.is_empty() && size + encoded > PAGE_BYTES {
                    stopped_by = "bytes";
                    break 'windows;
                }
                size += encoded;
                consumed += (end - start).max(1);
                chunks.push(chunk);
                if end == window.range.end {
                    break;
                }
                start = end;
            }
            position += span;
        }
        let (_, _, next) = page_bounds_from_offset(offset, key, consumed, self.positions)?;
        Ok(
            json!({"chunks": chunks, "truncated": next.is_some(), "cursor": next, "stoppedBy": stopped_by}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(images: &[SourceImage<'_>], limit: usize) -> Vec<Value> {
        let layout = PlanDiffLayout::new(images, || Ok(())).unwrap();
        let mut cursor = None;
        let mut chunks = Vec::new();
        loop {
            let page = layout
                .page(images, "plan-a", limit, cursor.as_deref(), || Ok(()))
                .unwrap();
            assert_eq!(
                page,
                layout
                    .page(images, "plan-a", limit, cursor.as_deref(), || Ok(()))
                    .unwrap()
            );
            assert!(page["chunks"]
                .as_array()
                .unwrap()
                .iter()
                .all(|chunk| chunk["pathEncoding"] == "utf8"));
            chunks.extend(page["chunks"].as_array().unwrap().clone());
            cursor = page["cursor"].as_str().map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        chunks
    }

    #[test]
    fn saved_source_pages_distinguish_equal_text_with_different_path_encodings() {
        for (native_encoding, path) in [("base64-unix-bytes", "gA=="), ("base64-utf16le", "ANg=")] {
            let images = [
                SourceImage {
                    path_encoding: "utf8",
                    path: path.into(),
                    before: Some(b"unicode-name"),
                    after: None,
                },
                SourceImage {
                    path_encoding: native_encoding,
                    path: path.into(),
                    before: Some(b"native-name"),
                    after: None,
                },
            ];
            let layout = PlanDiffLayout::new(&images, || Ok(())).unwrap();
            let page = layout
                .page(&images, "encoded-name-lookalikes", 4, None, || Ok(()))
                .unwrap();
            assert_eq!(
                page,
                layout
                    .page(&images, "encoded-name-lookalikes", 4, None, || Ok(()))
                    .unwrap()
            );
            assert_eq!(page["stoppedBy"], "complete");
            let chunks = page["chunks"].as_array().unwrap();
            assert_eq!(chunks.len(), 4);
            assert!(chunks.iter().all(|chunk| chunk["path"] == path));
            assert_eq!(chunks[0]["pathEncoding"], native_encoding);
            assert_eq!(chunks[0]["side"], "before");
            assert_eq!(chunks[0]["content"], "native-name");
            assert_eq!(chunks[1]["pathEncoding"], native_encoding);
            assert_eq!(chunks[1]["side"], "after");
            assert_eq!(chunks[2]["pathEncoding"], "utf8");
            assert_eq!(chunks[2]["side"], "before");
            assert_eq!(chunks[2]["content"], "unicode-name");
            assert_eq!(chunks[3]["pathEncoding"], "utf8");
            assert_eq!(chunks[3]["side"], "after");
        }
    }

    #[test]
    fn saved_source_pages_report_count_bytes_and_completion() {
        let body = vec![0; 100000];
        let images = [SourceImage {
            path_encoding: "utf8",
            path: "Module.bsl".into(),
            before: None,
            after: Some(&body),
        }];
        let layout = PlanDiffLayout::new(&images, || Ok(())).unwrap();
        let limited = layout.page(&images, "limit1", 1, None, || Ok(())).unwrap();
        assert_eq!(limited["stoppedBy"], "limit");
        assert!(limited["cursor"].is_string());
        let bytes = layout
            .page(&images, "limit50", 50, None, || Ok(()))
            .unwrap();
        assert_eq!(bytes["stoppedBy"], "bytes");
        assert!(bytes["cursor"].is_string());
        let mut cursor = bytes["cursor"].as_str().map(str::to_owned);
        while let Some(current) = cursor {
            let page = layout
                .page(&images, "limit50", 50, Some(&current), || Ok(()))
                .unwrap();
            assert_eq!(
                page,
                layout
                    .page(&images, "limit50", 50, Some(&current), || Ok(()))
                    .unwrap()
            );
            cursor = page["cursor"].as_str().map(str::to_owned);
            if cursor.is_none() {
                assert_eq!(page["stoppedBy"], "complete");
            }
        }
        let small = [SourceImage {
            path_encoding: "utf8",
            path: "small".into(),
            before: None,
            after: Some(b"a"),
        }];
        let exact = PlanDiffLayout::new(&small, || Ok(()))
            .unwrap()
            .page(&small, "two", 2, None, || Ok(()))
            .unwrap();
        assert_eq!(exact["stoppedBy"], "complete");
        assert!(exact["cursor"].is_null());
        let empty = PlanDiffLayout::new(&[], || Ok(()))
            .unwrap()
            .page(&[], "empty", 2, None, || Ok(()))
            .unwrap();
        assert_eq!(empty["stoppedBy"], "complete");
        assert!(empty["cursor"].is_null());
    }

    #[test]
    fn saved_source_pages_preserve_bom_crlf_indentation_and_no_final_newline() {
        let before = "\u{feff}Процедура Тест()\r\n\tПервый();\r\nКонецПроцедуры";
        let after = "\u{feff}Процедура Тест()\r\n\tПервый();\r\n\tВторой();\r\nКонецПроцедуры";
        let images = [SourceImage {
            path_encoding: "utf8",
            path: "Module.bsl".into(),
            before: Some(before.as_bytes()),
            after: Some(after.as_bytes()),
        }];
        let chunks = collect(&images, 1);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0]["content"], before);
        assert_eq!(chunks[1]["content"], after);
        assert_eq!(chunks[1]["endByte"], after.len());
    }

    #[test]
    fn saved_source_pages_are_lossless_across_unicode_binary_and_empty_sides() {
        let text = format!("\u{feff}{}", "я\t\r\n\0".repeat(10000));
        let binary: Vec<_> = (0..=255).cycle().take(18000).collect();
        let images = [
            SourceImage {
                path_encoding: "utf8",
                path: "a".into(),
                before: None,
                after: Some(text.as_bytes()),
            },
            SourceImage {
                path_encoding: "utf8",
                path: "b".into(),
                before: Some(&binary),
                after: None,
            },
            SourceImage {
                path_encoding: "utf8",
                path: "c".into(),
                before: None,
                after: Some(b""),
            },
        ];
        let chunks = collect(&images, 3);
        for (path, side, expected) in [
            ("a", "after", text.as_bytes()),
            ("b", "before", binary.as_slice()),
        ] {
            let mut actual = Vec::new();
            for chunk in chunks
                .iter()
                .filter(|c| c["path"] == path && c["side"] == side)
            {
                assert_eq!(chunk["startByte"], actual.len());
                let content = chunk["content"].as_str().unwrap();
                actual.extend(if chunk["encoding"] == "base64" {
                    STANDARD.decode(content).unwrap()
                } else {
                    content.as_bytes().to_vec()
                });
                assert_eq!(chunk["endByte"], actual.len());
            }
            assert_eq!(actual, expected);
        }
        let empty: Vec<_> = chunks.iter().filter(|c| c["path"] == "c").collect();
        assert_eq!(empty.len(), 2);
        assert_eq!(empty[0]["present"], false);
        assert_eq!(empty[1]["present"], true);
    }

    #[test]
    fn saved_source_page_bound_counts_escaped_bytes_and_rejects_foreign_cursor() {
        let body = vec![0; 100000];
        let images = [SourceImage {
            path_encoding: "utf8",
            path: "Module.bsl".into(),
            before: None,
            after: Some(&body),
        }];
        let layout = PlanDiffLayout::new(&images, || Ok(())).unwrap();
        let page = layout
            .page(&images, "plan-a-limit100", 100, None, || Ok(()))
            .unwrap();
        assert!(serde_json::to_vec(&page).unwrap().len() < PAGE_BYTES + 1024);
        let cursor = page["cursor"].as_str().unwrap();
        assert!(layout
            .page(&images, "plan-b-limit100", 100, Some(cursor), || Ok(()))
            .is_err());
        assert!(layout
            .page(&images, "plan-a-limit1", 1, Some(cursor), || Ok(()))
            .is_err());
        assert!(layout
            .page(&images, "plan-a-limit100", 100, Some(cursor), || Err(
                "cancelled".into()
            ))
            .is_err());
    }
}
