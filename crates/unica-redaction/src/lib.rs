//! Shared secret redaction for complete text and bounded stream chunks.

pub fn redactor(text: &str) -> String {
    let mut stream = StreamRedactor::new();
    let mut output = stream.push(text);
    output.push_str(&stream.finish());
    output
}

const EXACT_SECRET_KEYS: &[&str] = &["connection", "pwd"];
const SUBSTRING_SECRET_KEYS: &[&str] = &["password", "token", "secret"];
const MAX_RETAINED_KEY_BYTES: usize = "connection".len();

#[derive(Debug)]
pub struct StreamRedactor {
    state: StreamRedactorState,
}

#[derive(Debug)]
enum StreamRedactorState {
    Text,
    Candidate {
        pending: String,
        exact_key: ExactKeyStatus,
    },
    ConfirmedSecretKey,
    AfterSecretKey {
        kind: SecretValueKind,
    },
    RedactingSecretValue {
        marker: RedactionMarker,
        kind: SecretValueKind,
    },
}

#[derive(Debug)]
enum ExactKeyStatus {
    Possible,
    Discarded,
}

#[derive(Debug)]
enum RedactionMarker {
    Pending,
    Written,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SecretValueKind {
    Connection,
    Other,
}

impl Default for StreamRedactor {
    fn default() -> Self {
        Self {
            state: StreamRedactorState::Text,
        }
    }
}

impl StreamRedactor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, chunk: &str) -> String {
        let mut output = String::with_capacity(chunk.len());

        for ch in chunk.chars() {
            let state = std::mem::replace(&mut self.state, StreamRedactorState::Text);
            self.state = state.push(ch, &mut output);
        }
        output
    }

    pub fn finish(&mut self) -> String {
        let state = std::mem::replace(&mut self.state, StreamRedactorState::Text);
        state.finish()
    }

    #[cfg(test)]
    fn retained_len(&self) -> usize {
        match &self.state {
            StreamRedactorState::Candidate { pending, .. } => pending.len(),
            StreamRedactorState::Text
            | StreamRedactorState::ConfirmedSecretKey
            | StreamRedactorState::AfterSecretKey { .. }
            | StreamRedactorState::RedactingSecretValue { .. } => 0,
        }
    }
}

impl StreamRedactorState {
    fn push(self, ch: char, output: &mut String) -> Self {
        match self {
            Self::Text => {
                if secret_key_char(ch) {
                    Self::advance_candidate(ch.to_string(), ExactKeyStatus::Possible, output)
                } else {
                    output.push(ch);
                    Self::Text
                }
            }
            Self::Candidate {
                mut pending,
                exact_key,
            } => {
                if secret_key_char(ch) {
                    pending.push(ch);
                    Self::advance_candidate(pending, exact_key, output)
                } else {
                    let is_secret = matches!(exact_key, ExactKeyStatus::Possible)
                        && is_exact_secret_key(&pending);
                    output.push_str(&pending);
                    if is_secret {
                        let kind = if is_connection_key(&pending) {
                            SecretValueKind::Connection
                        } else {
                            SecretValueKind::Other
                        };
                        Self::after_secret_key(ch, kind, output)
                    } else {
                        output.push(ch);
                        Self::Text
                    }
                }
            }
            Self::ConfirmedSecretKey => {
                if secret_key_char(ch) {
                    output.push(ch);
                    Self::ConfirmedSecretKey
                } else {
                    Self::after_secret_key(ch, SecretValueKind::Other, output)
                }
            }
            Self::AfterSecretKey { kind } => Self::after_secret_key_value(ch, kind, output),
            Self::RedactingSecretValue { marker, kind } => {
                Self::redact_value(ch, marker, kind, output)
            }
        }
    }

    fn advance_candidate(
        mut pending: String,
        exact_key: ExactKeyStatus,
        output: &mut String,
    ) -> Self {
        if contains_substring_secret_key(&pending) {
            output.push_str(&pending);
            return Self::ConfirmedSecretKey;
        }

        let exact_key = exact_key.advance(&pending);
        if matches!(exact_key, ExactKeyStatus::Possible) {
            return Self::Candidate { pending, exact_key };
        }

        retain_secret_key_suffix(&mut pending, output);
        Self::Candidate { pending, exact_key }
    }

    fn after_secret_key(ch: char, kind: SecretValueKind, output: &mut String) -> Self {
        if matches!(ch, '=' | ':') {
            output.push(ch);
            Self::RedactingSecretValue {
                marker: RedactionMarker::Pending,
                kind,
            }
        } else if ch.is_whitespace() {
            output.push(ch);
            Self::AfterSecretKey { kind }
        } else if secret_key_char(ch) {
            Self::advance_candidate(ch.to_string(), ExactKeyStatus::Possible, output)
        } else {
            output.push(ch);
            Self::Text
        }
    }

    fn after_secret_key_value(ch: char, kind: SecretValueKind, output: &mut String) -> Self {
        if ch.is_whitespace() {
            output.push(ch);
            Self::AfterSecretKey { kind }
        } else if secret_value_delimiter(ch, kind) {
            output.push(ch);
            Self::Text
        } else {
            output.push_str("<redacted>");
            Self::RedactingSecretValue {
                marker: RedactionMarker::Written,
                kind,
            }
        }
    }

    fn redact_value(
        ch: char,
        marker: RedactionMarker,
        kind: SecretValueKind,
        output: &mut String,
    ) -> Self {
        match marker {
            RedactionMarker::Pending if ch.is_whitespace() => {
                output.push(ch);
                Self::RedactingSecretValue {
                    marker: RedactionMarker::Pending,
                    kind,
                }
            }
            RedactionMarker::Pending => {
                output.push_str("<redacted>");
                if secret_value_delimiter(ch, kind) {
                    output.push(ch);
                    Self::Text
                } else {
                    Self::RedactingSecretValue {
                        marker: RedactionMarker::Written,
                        kind,
                    }
                }
            }
            RedactionMarker::Written if secret_value_delimiter(ch, kind) => {
                output.push(ch);
                Self::Text
            }
            RedactionMarker::Written => Self::RedactingSecretValue {
                marker: RedactionMarker::Written,
                kind,
            },
        }
    }

    fn finish(self) -> String {
        match self {
            Self::Candidate { pending, .. } => pending,
            Self::RedactingSecretValue {
                marker: RedactionMarker::Pending,
                ..
            } => "<redacted>".to_string(),
            Self::Text
            | Self::ConfirmedSecretKey
            | Self::AfterSecretKey { .. }
            | Self::RedactingSecretValue {
                marker: RedactionMarker::Written,
                ..
            } => String::new(),
        }
    }
}

impl ExactKeyStatus {
    fn advance(self, key: &str) -> Self {
        match self {
            Self::Possible if is_possible_exact_secret_key(key) => Self::Possible,
            Self::Possible | Self::Discarded => Self::Discarded,
        }
    }
}

pub fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    EXACT_SECRET_KEYS.contains(&key.as_str())
        || SUBSTRING_SECRET_KEYS
            .iter()
            .any(|secret_key| key.contains(secret_key))
}

#[cfg(any(test, feature = "test-support"))]
pub fn production_secret_key_matrix() -> Vec<String> {
    EXACT_SECRET_KEYS
        .iter()
        .map(|key| (*key).to_string())
        .chain(SUBSTRING_SECRET_KEYS.iter().map(|key| (*key).to_string()))
        .collect()
}

fn exact_key_name(key: &str) -> &str {
    key.strip_prefix("--")
        .or_else(|| key.strip_prefix('-'))
        .unwrap_or(key)
}

fn is_possible_exact_secret_key(key: &str) -> bool {
    let key = exact_key_name(key);
    EXACT_SECRET_KEYS.iter().any(|secret_key| {
        secret_key
            .get(..key.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(key))
    })
}

fn is_exact_secret_key(key: &str) -> bool {
    let key = exact_key_name(key);
    EXACT_SECRET_KEYS
        .iter()
        .any(|secret_key| secret_key.eq_ignore_ascii_case(key))
}

fn is_connection_key(key: &str) -> bool {
    exact_key_name(key).eq_ignore_ascii_case("connection")
}

fn contains_substring_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    SUBSTRING_SECRET_KEYS
        .iter()
        .any(|secret_key| key.contains(secret_key))
}

fn retain_secret_key_suffix(pending: &mut String, output: &mut String) {
    let mut retained_len = 0;
    for length in (1..=MAX_RETAINED_KEY_BYTES).rev() {
        let Some(suffix_start) = pending.len().checked_sub(length) else {
            continue;
        };
        let Some(suffix) = pending.get(suffix_start..) else {
            continue;
        };
        if SUBSTRING_SECRET_KEYS.iter().any(|secret_key| {
            secret_key
                .get(..length)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(suffix))
        }) {
            retained_len = length;
            break;
        }
    }

    if retained_len == 0 {
        output.push_str(pending);
        pending.clear();
        return;
    }

    let Some(suffix_start) = pending.len().checked_sub(retained_len) else {
        return;
    };
    let Some(prefix) = pending.get(..suffix_start) else {
        return;
    };
    let Some(suffix) = pending.get(suffix_start..) else {
        return;
    };
    output.push_str(prefix);
    *pending = suffix.to_string();
}

fn secret_key_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-')
}

fn secret_value_delimiter(ch: char, kind: SecretValueKind) -> bool {
    match kind {
        // A 1C connection string contains semicolon-delimited fields. Releasing
        // them as independent text would expose values such as Ref and Usr.
        SecretValueKind::Connection => matches!(ch, '\n' | '\r'),
        SecretValueKind::Other => matches!(ch, ';' | '&' | ',' | '\n' | '\r' | '}'),
    }
}

pub fn bounded_tail(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut start = text.len().saturating_sub(max_bytes);
    while start < text.len() && !text.is_char_boundary(start) {
        start = start.saturating_add(1);
    }
    text[start..].to_string()
}

pub fn bounded_char_tail(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }
    text.chars()
        .skip(char_count.saturating_sub(max_chars))
        .collect()
}

pub fn bounded_redacted_tail(text: &str, max_bytes: usize) -> String {
    bounded_tail(&redactor(text), max_bytes)
}

#[cfg(test)]
mod tests {
    use super::{is_secret_key, production_secret_key_matrix, redactor, StreamRedactor};

    #[test]
    fn stream_redactor_redacts_secret_key_split_across_chunks() {
        let mut redactor = StreamRedactor::new();

        assert_eq!(redactor.push("starting; P"), "starting; ");
        assert_eq!(
            redactor.push("wd=super-secret\nfinished\n"),
            "Pwd=<redacted>\nfinished\n"
        );
    }

    #[test]
    fn stream_redactor_covers_production_secret_keys_at_every_chunk_boundary() {
        for key in production_secret_key_matrix() {
            assert!(
                is_secret_key(&key),
                "fixture key must come from production policy: {key}"
            );
            let input = format!("prefix; {key}=value-for-{key}\nsuffix");
            for split in 0..=input.len() {
                let mut stream = StreamRedactor::new();
                let mut output = stream.push(&input[..split]);
                output.push_str(&stream.push(&input[split..]));
                output.push_str(&stream.finish());
                assert!(
                    !output.contains(&format!("value-for-{key}")),
                    "key={key}, split={split}: {output}"
                );
                assert!(
                    output.contains("<redacted>"),
                    "key={key}, split={split}: {output}"
                );
            }
        }
    }

    #[test]
    fn stream_redactor_hides_complete_connection_strings_at_every_chunk_boundary() {
        for (input, expected) in [
            (
                "prefix\n--connection Srvr=server;Ref=private-ib;Usr=service;Pwd=private-password\nPwd=separate-secret\nsuffix\n",
                "prefix\n--connection <redacted>\nPwd=<redacted>\nsuffix\n",
            ),
            (
                "connection=File=/private/path;Ref=private-ib&Pwd=private-password,next=private-data\nfinished\n",
                "connection=<redacted>\nfinished\n",
            ),
        ] {
            for split in 0..=input.len() {
                let mut stream = StreamRedactor::new();
                let mut output = stream.push(&input[..split]);
                output.push_str(&stream.push(&input[split..]));
                output.push_str(&stream.finish());
                assert_eq!(output, expected, "split={split}");
            }
        }

        // Separators still terminate ordinary key values on the same line.
        assert_eq!(
            redactor("token=private-token; stage=done\n"),
            "token=<redacted>; stage=done\n"
        );
    }

    #[test]
    fn stream_redactor_bounds_state_for_delimiter_free_chunks() {
        let mut redactor = StreamRedactor::new();

        for _ in 0..1_024 {
            assert_eq!(redactor.push("x"), "x");
            assert!(redactor.retained_len() <= 10);
        }

        assert_eq!(redactor.push("token=super-secret\n"), "token=<redacted>\n");
    }

    #[test]
    fn redactor_preserves_normal_text() {
        assert_eq!(
            redactor("starting build\nfinished successfully\n"),
            "starting build\nfinished successfully\n"
        );
    }

    #[test]
    fn redactor_hides_whitespace_delimited_runtime_secrets() {
        let output = redactor(
            "v8-runner --password runtime-secret --connection Srvr=server;Pwd=connection-secret\n",
        );

        assert!(!output.contains("runtime-secret"), "{output}");
        assert!(!output.contains("connection-secret"), "{output}");
        assert!(output.contains("--password <redacted>"), "{output}");
        assert!(output.contains("Pwd=<redacted>"), "{output}");
    }
}
