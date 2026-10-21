use roxmltree::Document;
use std::collections::HashMap;
use unica_metadata::support_state::{SupportReadError, SupportReadErrorCode};

#[derive(Debug, Clone)]
pub struct SupportState {
    pub global_editing_enabled: bool,
    pub vendor_count: usize,
    pub removed: bool,
    pub counts: [usize; 3],
    pub object_rules: HashMap<String, u8>,
    pub vendors: Vec<SupportVendor>,
}

impl SupportState {
    pub fn object_rule(&self, object_uuid: &str) -> Option<u8> {
        self.object_rules
            .get(&object_uuid.to_ascii_lowercase())
            .copied()
    }

    pub fn global_editing_enabled(&self) -> bool {
        self.global_editing_enabled
    }

    pub fn removed(&self) -> bool {
        self.removed
    }

    pub fn counts(&self) -> [usize; 3] {
        self.counts
    }
}

#[derive(Debug, Clone)]
pub struct SupportVendor {
    pub version: String,
    pub vendor: String,
    pub name: String,
}

/// Strict marker parsing retains the result and bounded input windows, never the
/// complete marker. The compatibility mutation guard intentionally has its own
/// historical acceptance rules (lossy UTF-8 and short-marker removal).
#[derive(Default)]
pub struct SupportStateStreamParser {
    nonempty: bool,
    utf8_tail: Vec<u8>,
    rule_tail: Vec<u8>,
    header: SupportHeaderStream,
    pub counts: [usize; 3],
    pub object_rules: HashMap<String, u8>,
    quote_open: bool,
    quote_pending: bool,
    quoted_value: String,
    vendor_fields: Vec<String>,
    pub vendors: Vec<SupportVendor>,
}

impl SupportStateStreamParser {
    pub fn feed(&mut self, chunk: &[u8]) -> Result<(), SupportReadError> {
        self.nonempty |= !chunk.is_empty();
        let mut input = std::mem::take(&mut self.utf8_tail);
        input.extend_from_slice(chunk);
        let valid = match std::str::from_utf8(&input) {
            Ok(text) => text,
            Err(error) if error.error_len().is_none() => {
                self.utf8_tail
                    .extend_from_slice(&input[error.valid_up_to()..]);
                // valid_up_to is an established UTF-8 boundary, not a guessed character split.
                std::str::from_utf8(&input[..error.valid_up_to()]).expect("validated UTF-8 prefix")
            }
            Err(_) => return Err(Self::invalid_utf8()),
        };
        for ch in valid.chars() {
            self.header.push(ch);
            if self.header.removed() {
                continue;
            }
            if self.quote_pending {
                self.quote_pending = false;
                if ch == '"' {
                    self.quoted_value.push('"');
                    continue;
                }
                self.finish_quoted_value();
            }
            if ch == '"' {
                if self.quote_open {
                    self.quote_pending = true;
                } else {
                    self.quote_open = true;
                }
            } else if self.quote_open {
                self.quoted_value.push(ch);
            }
        }
        if !self.header.removed() {
            self.scan_rules(valid.as_bytes());
        }
        Ok(())
    }

    fn scan_rules(&mut self, bytes: &[u8]) {
        let mut input = std::mem::take(&mut self.rule_tail);
        input.extend_from_slice(bytes);
        let mut offset = 0;
        while offset + 40 <= input.len() {
            let candidate = &input[offset..offset + 40];
            if matches!(candidate[0], b'0'..=b'2')
                && &candidate[1..4] == b",0,"
                && candidate[4..].iter().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        *byte == b'-'
                    } else {
                        byte.is_ascii_hexdigit()
                    }
                })
            {
                let flag = candidate[0] - b'0';
                self.counts[flag as usize] += 1;
                let uuid = std::str::from_utf8(&candidate[4..])
                    .expect("UUID consists of ASCII bytes")
                    .to_ascii_lowercase();
                self.object_rules
                    .entry(uuid)
                    .and_modify(|value| *value = (*value).min(flag))
                    .or_insert(flag);
                // Legacy matching consumes the complete match, including its last UUID byte.
                offset += 40;
            } else {
                offset += 1;
            }
        }
        self.rule_tail.extend_from_slice(&input[offset..]);
    }

    fn finish_quoted_value(&mut self) {
        self.quote_open = false;
        self.vendor_fields
            .push(std::mem::take(&mut self.quoted_value));
        if self.vendor_fields.len() == 3 {
            let mut fields = self.vendor_fields.drain(..);
            self.vendors.push(SupportVendor {
                version: fields.next().expect("three vendor fields"),
                vendor: fields.next().expect("three vendor fields"),
                name: fields.next().expect("three vendor fields"),
            });
        }
    }

    pub fn finish(mut self) -> Result<SupportState, SupportReadError> {
        if !self.utf8_tail.is_empty() {
            return Err(Self::invalid_utf8());
        }
        let (global_flag, vendor_count) = if self.nonempty {
            self.header.finish().ok_or_else(|| {
                SupportReadError::new(
                    SupportReadErrorCode::StateInvalid,
                    "support-state marker has an unsupported header",
                )
            })?
        } else {
            (0, 0)
        };
        if self.quote_open {
            self.finish_quoted_value();
        }
        let removed = vendor_count == 0;
        Ok(SupportState {
            global_editing_enabled: removed || global_flag == 0,
            vendor_count,
            removed,
            counts: if removed { [0; 3] } else { self.counts },
            object_rules: if removed {
                HashMap::new()
            } else {
                self.object_rules
            },
            vendors: if removed { Vec::new() } else { self.vendors },
        })
    }

    fn invalid_utf8() -> SupportReadError {
        SupportReadError::new(
            SupportReadErrorCode::StateInvalid,
            "support-state marker is not valid UTF-8",
        )
    }
}

#[derive(Default)]
struct SupportHeaderStream {
    // 0: leading BOMs, 1: leading whitespace, 2..=4: header tokens, 5: complete.
    phase: u8,
    invalid: bool,
    token: SupportHeaderNumber,
    global_flag: u8,
    pub vendor_count: usize,
}

impl SupportHeaderStream {
    fn push(&mut self, ch: char) {
        if self.invalid || self.phase == 5 {
            return;
        }
        if self.phase == 0 {
            if ch == '\u{feff}' {
                return;
            }
            self.phase = 1;
        }
        if self.phase == 1 {
            if ch.is_whitespace() {
                return;
            }
            if ch == '{' {
                self.phase = 2;
            } else {
                self.invalid = true;
            }
        } else if ch == ',' {
            self.finish_token();
        } else {
            self.token.push(ch);
        }
    }

    fn finish_token(&mut self) {
        let token = std::mem::take(&mut self.token);
        if token.invalid || token.digits == 0 {
            self.invalid = true;
            return;
        }
        match self.phase {
            2 if token.value == 6 && token.digits == 1 && !token.plus => {}
            3 => match u8::try_from(token.value) {
                Ok(value) => self.global_flag = value,
                Err(_) => self.invalid = true,
            },
            4 => self.vendor_count = token.value,
            _ => self.invalid = true,
        }
        self.phase += 1;
    }

    fn removed(&self) -> bool {
        self.phase == 5 && !self.invalid && self.vendor_count == 0
    }

    fn finish(&mut self) -> Option<(u8, usize)> {
        if self.phase == 4 {
            self.finish_token();
        }
        (!self.invalid && self.phase == 5).then_some((self.global_flag, self.vendor_count))
    }
}

#[derive(Default)]
struct SupportHeaderNumber {
    value: usize,
    digits: u8,
    plus: bool,
    ended: bool,
    invalid: bool,
}

impl SupportHeaderNumber {
    fn push(&mut self, ch: char) {
        if ch.is_whitespace() {
            self.ended |= self.digits > 0 || self.plus;
        } else if ch == '+' && self.digits == 0 && !self.plus && !self.ended {
            self.plus = true;
        } else if ch.is_ascii_digit() && !self.ended {
            self.digits = self.digits.saturating_add(1);
            if let Some(value) = self
                .value
                .checked_mul(10)
                .and_then(|value| value.checked_add(ch as usize - '0' as usize))
            {
                self.value = value;
            } else {
                self.invalid = true;
            }
        } else {
            self.invalid = true;
        }
    }
}

/// Parses already-read support marker bytes with the current V12 mutating
/// guard semantics. Identity and no-follow proof remain the caller's job.
pub fn parse_support_state_compat_bytes(data: Option<&[u8]>) -> Option<SupportState> {
    let data = data?;
    if data.len() <= 32 {
        return Some(SupportState {
            global_editing_enabled: true,
            vendor_count: 0,
            removed: true,
            counts: [0, 0, 0],
            object_rules: HashMap::new(),
            vendors: Vec::new(),
        });
    }
    let text = String::from_utf8_lossy(data.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(data));
    let (global_flag, vendor_count) = parse_support_header(&text)?;
    if vendor_count == 0 {
        return Some(SupportState {
            global_editing_enabled: true,
            vendor_count,
            removed: true,
            counts: [0, 0, 0],
            object_rules: HashMap::new(),
            vendors: Vec::new(),
        });
    }
    let (counts, object_rules) = parse_support_object_rules(&text);
    Some(SupportState {
        global_editing_enabled: global_flag == 0,
        vendor_count,
        removed: false,
        counts,
        object_rules,
        vendors: parse_support_vendors(&text),
    })
}

/// Parses already-admitted support-state bytes with the same semantics as the
/// path wrapper. The caller owns file identity/no-follow checks.
pub fn parse_support_state_strict_bytes(
    data: Option<&[u8]>,
) -> Result<Option<SupportState>, SupportReadError> {
    let Some(data) = data else {
        return Ok(None);
    };
    let mut parser = SupportStateStreamParser::default();
    // The slice adapter shares the streaming parser, including whole-tail validation.
    for chunk in data.chunks(64 * 1024) {
        parser.feed(chunk)?;
    }
    parser.finish().map(Some)
}

pub fn parse_support_header(text: &str) -> Option<(u8, usize)> {
    let mut parts = text
        .trim_start_matches('\u{feff}')
        .trim_start()
        .strip_prefix('{')?
        .split(',')
        .map(str::trim);
    if parts.next()? != "6" {
        return None;
    }
    let global_flag = parts.next()?.parse::<u8>().ok()?;
    let vendor_count = parts.next()?.parse::<usize>().ok()?;
    Some((global_flag, vendor_count))
}

pub fn parse_support_object_rules(text: &str) -> ([usize; 3], HashMap<String, u8>) {
    let mut counts = [0usize; 3];
    let mut object_rules = HashMap::<String, u8>::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i + 40 <= bytes.len() {
        let flag = bytes[i];
        if matches!(flag, b'0'..=b'2') && bytes.get(i + 1..i + 4) == Some(b",0,") {
            let uuid_start = i + 4;
            let uuid_end = uuid_start + 36;
            if uuid_end <= bytes.len() {
                let uuid = &text[uuid_start..uuid_end];
                if is_uuid_text(uuid) {
                    let flag_value = flag - b'0';
                    counts[flag_value as usize] += 1;
                    let entry = object_rules
                        .entry(uuid.to_ascii_lowercase())
                        .or_insert(flag_value);
                    if flag_value < *entry {
                        *entry = flag_value;
                    }
                    i = uuid_end;
                    continue;
                }
            }
        }
        i += 1;
    }
    (counts, object_rules)
}

pub fn parse_support_vendors(text: &str) -> Vec<SupportVendor> {
    let quoted = parse_quoted_support_strings(text);
    quoted
        .chunks(3)
        .filter_map(|chunk| {
            if chunk.len() == 3 {
                Some(SupportVendor {
                    version: chunk[0].clone(),
                    vendor: chunk[1].clone(),
                    name: chunk[2].clone(),
                })
            } else {
                None
            }
        })
        .collect()
}

pub fn parse_quoted_support_strings(text: &str) -> Vec<String> {
    let mut result = Vec::<String>::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '"' {
            continue;
        }
        let mut value = String::new();
        while let Some(next) = chars.next() {
            if next == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    value.push('"');
                    continue;
                }
                break;
            }
            value.push(next);
        }
        result.push(value);
    }
    result
}

pub fn is_uuid_text(value: &str) -> bool {
    value.len() == 36
        && value.chars().enumerate().all(|(index, ch)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                ch == '-'
            } else {
                ch.is_ascii_hexdigit()
            }
        })
}

pub fn support_root_uuid_from_bytes(raw: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(raw).ok()?;
    let doc = Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
    let root = doc.root_element();
    if let Some(uuid) = root.attribute("uuid") {
        return Some(uuid.to_ascii_lowercase());
    }
    root.children()
        .find(|node| node.is_element() && node.attribute("uuid").is_some())
        .and_then(|node| node.attribute("uuid"))
        .map(str::to_ascii_lowercase)
}

#[cfg(test)]
mod support_stream_tests {
    use super::*;

    #[test]
    fn every_marker_split_preserves_counts_duplicate_minimum_and_quoted_vendors() {
        let uuid = "ABCDEF00-1234-5678-90AB-123456789AB0";
        let other = "00000000-0000-0000-0000-000000000001";
        let text = format!("\u{feff}\u{feff}\u{2003}{{6, +000, +1,2,0,{uuid},0,0,{uuid},1,0,{other},\"версия\" \"По\"\"ставщик\" \"имя🦀\"}}");
        for split in 0..=text.len() {
            let mut parser = SupportStateStreamParser::default();
            parser.feed(&text.as_bytes()[..split]).unwrap();
            assert!(parser.utf8_tail.len() <= 3);
            assert!(parser.rule_tail.len() <= 39);
            parser.feed(&text.as_bytes()[split..]).unwrap();
            let state = parser.finish().unwrap();
            assert_eq!(state.counts(), [1, 1, 1], "split {split}");
            assert_eq!(state.object_rule(uuid), Some(0));
            assert_eq!(state.object_rule(other), Some(1));
            assert!(state.global_editing_enabled());
            assert_eq!(state.vendor_count, 1);
            assert_eq!(state.vendors.len(), 1);
            assert_eq!(state.vendors[0].version, "версия");
            assert_eq!(state.vendors[0].vendor, "По\"ставщик");
            assert_eq!(state.vendors[0].name, "имя🦀");
        }
    }

    #[test]
    fn stream_header_preserves_whole_token_numeric_and_empty_marker_semantics() {
        assert!(parse_support_state_strict_bytes(None).unwrap().is_none());
        assert!(parse_support_state_strict_bytes(Some(b""))
            .unwrap()
            .unwrap()
            .removed());
        for text in [
            "\u{feff}",
            " ",
            "{6,0,1}",
            "{06,0,1,",
            "{+6,0,1,",
            "{6,256,1,",
            "{6,-0,1,",
            "{6,0,+ 1,",
            "{6,0,1 2,",
            "{6,0,18446744073709551616,",
            " \u{feff}{6,0,1,",
        ] {
            let error = parse_support_state_strict_bytes(Some(text.as_bytes())).unwrap_err();
            assert_eq!(error.code, SupportReadErrorCode::StateInvalid, "{text:?}");
        }
        for (text, editing, removed) in [
            ("{6,255,1", false, false),
            ("{6,0,0,ignored tail", true, true),
            (
                "{\u{2003}6\u{2003},\u{2003}+000\u{2003},+0001\u{2003},",
                true,
                false,
            ),
        ] {
            let state = parse_support_state_strict_bytes(Some(text.as_bytes()))
                .unwrap()
                .unwrap();
            assert_eq!(state.global_editing_enabled(), editing);
            assert_eq!(state.removed(), removed);
        }
        let leading_zeroes = format!("{{6,{},1,", "0".repeat(70_000));
        assert!(
            parse_support_state_strict_bytes(Some(leading_zeroes.as_bytes()))
                .unwrap()
                .unwrap()
                .global_editing_enabled()
        );
    }

    #[test]
    fn stream_validates_utf8_in_the_entire_tail_even_after_removed_header() {
        for header in [b"{6,0,0,".as_slice(), b"{6,0,1,", b"{9,0,1,"] {
            for tail in [
                b"\xff".as_slice(),
                b"\xf0\x9f\xa6",
                b"\xed\xa0\x80",
                b"\xc0\xaf",
            ] {
                let mut input = header.to_vec();
                input.resize(64 * 1024 - 1, b' ');
                input.extend_from_slice(tail);
                let error = parse_support_state_strict_bytes(Some(&input)).unwrap_err();
                assert_eq!(error.code, SupportReadErrorCode::StateInvalid);
                assert!(error.message.contains("UTF-8"));
            }
        }
    }

    #[test]
    fn rule_match_consumes_its_last_uuid_character_and_unclosed_quotes_keep_legacy_meaning() {
        let first = "00000000-0000-0000-0000-000000000000";
        let second = "11111111-1111-1111-1111-111111111111";
        // The last zero of the first UUID cannot start an overlapping second match.
        let text = format!("{{6,0,1,2,0,{first},0,{second},\"v\"\"endor\" \"name\" \"unfinished");
        for size in [1, 2, 3, 7, 39, 40, 41, 64 * 1024] {
            let mut parser = SupportStateStreamParser::default();
            for chunk in text.as_bytes().chunks(size) {
                parser.feed(chunk).unwrap();
            }
            let state = parser.finish().unwrap();
            assert_eq!(state.counts(), [0, 0, 1]);
            assert_eq!(state.object_rule(second), None);
            assert_eq!(state.vendors.len(), 1);
            assert_eq!(state.vendors[0].version, "v\"endor");
            assert_eq!(state.vendors[0].vendor, "name");
            assert_eq!(state.vendors[0].name, "unfinished");
        }
    }
}
