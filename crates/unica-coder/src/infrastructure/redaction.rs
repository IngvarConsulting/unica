pub(crate) fn redactor(text: &str) -> String {
    let mut stream = StreamRedactor::new();
    let mut output = stream.push(text);
    output.push_str(&stream.finish());
    output
}

const EXACT_SECRET_KEYS: &[&str] = &["connection", "pwd"];
const SUBSTRING_SECRET_KEYS: &[&str] = &["password", "token", "secret"];
const MAX_RETAINED_KEY_BYTES: usize = "connection".len();

#[derive(Debug)]
pub(crate) struct StreamRedactor {
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
    AfterSecretKey,
    RedactingSecretValue {
        marker: RedactionMarker,
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

impl Default for StreamRedactor {
    fn default() -> Self {
        Self {
            state: StreamRedactorState::Text,
        }
    }
}

impl StreamRedactor {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push(&mut self, chunk: &str) -> String {
        let mut output = String::with_capacity(chunk.len());

        for ch in chunk.chars() {
            let state = std::mem::replace(&mut self.state, StreamRedactorState::Text);
            self.state = state.push(ch, &mut output);
        }
        output
    }

    pub(crate) fn finish(&mut self) -> String {
        let state = std::mem::replace(&mut self.state, StreamRedactorState::Text);
        state.finish()
    }

    #[cfg(test)]
    fn retained_len(&self) -> usize {
        match &self.state {
            StreamRedactorState::Candidate { pending, .. } => pending.len(),
            StreamRedactorState::Text
            | StreamRedactorState::ConfirmedSecretKey
            | StreamRedactorState::AfterSecretKey
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
                        Self::after_secret_key(ch, output)
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
                    Self::after_secret_key(ch, output)
                }
            }
            Self::AfterSecretKey => Self::after_secret_key_value(ch, output),
            Self::RedactingSecretValue { marker } => Self::redact_value(ch, marker, output),
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

    fn after_secret_key(ch: char, output: &mut String) -> Self {
        if matches!(ch, '=' | ':') {
            output.push(ch);
            Self::RedactingSecretValue {
                marker: RedactionMarker::Pending,
            }
        } else if ch.is_whitespace() {
            output.push(ch);
            Self::AfterSecretKey
        } else if secret_key_char(ch) {
            Self::advance_candidate(ch.to_string(), ExactKeyStatus::Possible, output)
        } else {
            output.push(ch);
            Self::Text
        }
    }

    fn after_secret_key_value(ch: char, output: &mut String) -> Self {
        if ch.is_whitespace() {
            output.push(ch);
            Self::AfterSecretKey
        } else if secret_value_delimiter(ch) {
            output.push(ch);
            Self::Text
        } else {
            output.push_str("<redacted>");
            Self::RedactingSecretValue {
                marker: RedactionMarker::Written,
            }
        }
    }

    fn redact_value(ch: char, marker: RedactionMarker, output: &mut String) -> Self {
        match marker {
            RedactionMarker::Pending if ch.is_whitespace() => {
                output.push(ch);
                Self::RedactingSecretValue {
                    marker: RedactionMarker::Pending,
                }
            }
            RedactionMarker::Pending => {
                output.push_str("<redacted>");
                if secret_value_delimiter(ch) {
                    output.push(ch);
                    Self::Text
                } else {
                    Self::RedactingSecretValue {
                        marker: RedactionMarker::Written,
                    }
                }
            }
            RedactionMarker::Written if secret_value_delimiter(ch) => {
                output.push(ch);
                Self::Text
            }
            RedactionMarker::Written => Self::RedactingSecretValue {
                marker: RedactionMarker::Written,
            },
        }
    }

    fn finish(self) -> String {
        match self {
            Self::Candidate { pending, .. } => pending,
            Self::RedactingSecretValue {
                marker: RedactionMarker::Pending,
            } => "<redacted>".to_string(),
            Self::Text
            | Self::ConfirmedSecretKey
            | Self::AfterSecretKey
            | Self::RedactingSecretValue {
                marker: RedactionMarker::Written,
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

pub(crate) fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    EXACT_SECRET_KEYS.contains(&key.as_str())
        || SUBSTRING_SECRET_KEYS
            .iter()
            .any(|secret_key| key.contains(secret_key))
}

fn is_possible_exact_secret_key(key: &str) -> bool {
    EXACT_SECRET_KEYS.iter().any(|secret_key| {
        secret_key
            .get(..key.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(key))
    })
}

fn is_exact_secret_key(key: &str) -> bool {
    EXACT_SECRET_KEYS
        .iter()
        .any(|secret_key| secret_key.eq_ignore_ascii_case(key))
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

fn secret_value_delimiter(ch: char) -> bool {
    matches!(ch, ';' | '&' | ',' | '\n' | '\r' | '}')
}

#[cfg(test)]
mod tests {
    use super::{redactor, StreamRedactor};

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
