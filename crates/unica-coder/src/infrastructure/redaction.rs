pub(crate) fn redactor(text: &str) -> String {
    let mut stream = StreamRedactor::new();
    let mut output = stream.push(text);
    output.push_str(&stream.finish());
    output
}

#[derive(Debug, Default)]
pub(crate) struct StreamRedactor {
    pending: String,
    redacting_secret_value: bool,
}

impl StreamRedactor {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push(&mut self, chunk: &str) -> String {
        self.pending.push_str(chunk);

        let chars = self.pending.chars().collect::<Vec<_>>();
        let mut output = String::with_capacity(chars.len());
        let mut index = 0;

        while index < chars.len() {
            if self.redacting_secret_value {
                if secret_value_delimiter(chars[index]) {
                    output.push(chars[index]);
                    self.redacting_secret_value = false;
                }
                index += 1;
                continue;
            }

            if !secret_key_char(chars[index]) {
                output.push(chars[index]);
                index += 1;
                continue;
            }

            let key_start = index;
            index += 1;
            while index < chars.len() && secret_key_char(chars[index]) {
                index += 1;
            }
            let key_end = index;

            if key_end == chars.len() {
                self.pending = chars[key_start..].iter().collect();
                return output;
            }

            let mut separator = index;
            while separator < chars.len() && chars[separator].is_whitespace() {
                separator += 1;
            }
            if separator == chars.len() {
                if is_secret_key(&chars[key_start..key_end].iter().collect::<String>()) {
                    self.pending = chars[key_start..].iter().collect();
                    return output;
                }
                output.extend(chars[key_start..separator].iter());
                index = separator;
                continue;
            }

            if matches!(chars[separator], '=' | ':')
                && is_secret_key(&chars[key_start..key_end].iter().collect::<String>())
            {
                let mut value_start = separator + 1;
                while value_start < chars.len() && chars[value_start].is_whitespace() {
                    value_start += 1;
                }
                output.extend(chars[key_start..value_start].iter());
                output.push_str("<redacted>");
                self.redacting_secret_value = true;
                index = value_start;
                continue;
            }

            output.extend(chars[key_start..key_end].iter());
        }

        self.pending.clear();
        output
    }

    pub(crate) fn finish(&mut self) -> String {
        self.redacting_secret_value = false;
        std::mem::take(&mut self.pending)
    }
}

pub(crate) fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key == "connection"
        || key == "pwd"
        || key.contains("password")
        || key.contains("token")
        || key.contains("secret")
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
    fn redactor_preserves_normal_text() {
        assert_eq!(
            redactor("starting build\nfinished successfully\n"),
            "starting build\nfinished successfully\n"
        );
    }
}
