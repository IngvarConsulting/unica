//! Connection secrets of the current call, removed from its result (#2124).
//!
//! The key-based redactor hides a value only after its key, so a runner that
//! repeats a password bare would pass it on to results, Tasks and receipts.
//! Unica reads the project configuration it gives the runner, so it knows the
//! secret values of the call and hides them in the result before it is kept.
//! The runner's own output is left intact: Unica parses and checks it, and a
//! short password must not change a path, a version or a nonce there.
use super::v13_workspace_bootstrap::read_yaml_config;
use serde_yaml::Value;
use std::path::Path;
use unica_execution_contract::invocation::DomainResult;

const REDACTED: &str = "<redacted>";
/// Keys whose value is a secret, compared case-insensitively as the whole
/// key or its ending: `password`, `agent_password`, but not `passwordFile`.
const SECRET_KEYS: &[&str] = &["password", "pwd", "token", "secret"];

fn is_secret_key(key: &str) -> bool {
    SECRET_KEYS.iter().any(|secret| key.ends_with(secret))
}

/// The secret values of the project configuration in `root`: the two layers
/// the runner is given. A layer that cannot be read contributes nothing; the
/// runner refuses such a layer itself.
pub(super) fn known_secrets(root: &Path) -> Vec<String> {
    let mut secrets = Vec::new();
    for name in ["v8project.yaml", "v8project.local.yaml"] {
        if let Ok(Some(layer)) = read_yaml_config(root, name) {
            collect(&layer, &mut secrets);
        }
    }
    secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    secrets.dedup();
    secrets
}

fn collect(value: &Value, secrets: &mut Vec<String>) {
    match value {
        Value::Mapping(mapping) => {
            for (key, value) in mapping {
                let key = key.as_str().unwrap_or_default().to_ascii_lowercase();
                if key == "connection" {
                    if let Some(connection) = value.as_str() {
                        connection_secrets(connection, secrets);
                    }
                } else if is_secret_key(&key) {
                    // A password of digits is a YAML number, still a secret.
                    match value {
                        Value::String(text) => push(secrets, text),
                        Value::Number(number) => push(secrets, &number.to_string()),
                        _ => collect(value, secrets),
                    }
                } else {
                    collect(value, secrets);
                }
            }
        }
        Value::Sequence(items) => items.iter().for_each(|item| collect(item, secrets)),
        Value::Tagged(tagged) => collect(&tagged.value, secrets),
        _ => {}
    }
}

/// The password of a 1C connection string: `Pwd=` of the key form
/// `Srvr=s;Ref=ib;Usr=u;Pwd=p;` and `/P` of the command-line form
/// `/F base /N user /P pass`. A quoted value may hold `;` and doubles its
/// quotes, `Pwd="a;""b"`: both the value and its written form are secrets.
fn connection_secrets(connection: &str, secrets: &mut Vec<String>) {
    let mut words = connection.split_whitespace();
    while let Some(word) = words.next() {
        let value = match word.get(..2) {
            Some(flag) if flag.eq_ignore_ascii_case("/p") && word.len() == 2 => words.next(),
            Some(flag) if flag.eq_ignore_ascii_case("/p") => word.get(2..),
            _ => None,
        };
        if let Some(value) = value {
            push(secrets, value.trim_matches('"'));
        }
    }
    let mut rest = connection;
    while !rest.is_empty() {
        let Some((key, after)) = rest.split_once('=') else {
            return;
        };
        let key = key
            .trim()
            .trim_start_matches(';')
            .trim()
            .to_ascii_lowercase();
        let (value, written, next) = match after.trim_start().strip_prefix('"') {
            Some(quoted) => {
                let mut value = String::new();
                let mut chars = quoted.char_indices().peekable();
                let mut end = quoted.len();
                while let Some((index, ch)) = chars.next() {
                    if ch == '"' {
                        if chars.peek().is_some_and(|(_, next)| *next == '"') {
                            value.push('"');
                            chars.next();
                        } else {
                            end = index;
                            break;
                        }
                    } else {
                        value.push(ch);
                    }
                }
                let written = quoted[..end].to_string();
                let next = quoted
                    .get(end + 1..)
                    .and_then(|tail| tail.split_once(';'))
                    .map_or("", |(_, next)| next);
                (value, Some(written), next)
            }
            None => match after.split_once(';') {
                Some((value, next)) => (value.trim().to_string(), None, next),
                None => (after.trim().to_string(), None, ""),
            },
        };
        if key == "pwd" || key == "password" {
            push(secrets, &value);
            if let Some(written) = written {
                push(secrets, &written);
            }
        }
        rest = next;
    }
}

fn push(secrets: &mut Vec<String>, value: &str) {
    if !value.is_empty() {
        secrets.push(value.to_string());
    }
}

/// `text` with every whole occurrence of a secret replaced: an occurrence
/// glued to a letter or digit, like `1` inside `8.3.27.2074` or a UUID, is
/// another word and stays.
fn replace_all(text: &str, secrets: &[String]) -> String {
    let mut text = text.to_string();
    for secret in secrets {
        let mut output = String::with_capacity(text.len());
        let mut rest = text.as_str();
        let mut previous: Option<char> = None;
        while let Some(start) = rest.find(secret.as_str()) {
            let before = rest[..start].chars().next_back().or(previous);
            let after = rest[start + secret.len()..].chars().next();
            let glued = |edge: Option<char>, neighbour: Option<char>| {
                edge.is_some_and(char::is_alphanumeric)
                    && neighbour.is_some_and(char::is_alphanumeric)
            };
            let whole =
                !glued(secret.chars().next(), before) && !glued(secret.chars().next_back(), after);
            let end = if whole {
                start + secret.len()
            } else {
                start + secret.chars().next().map_or(1, char::len_utf8)
            };
            output.push_str(&rest[..start]);
            if whole {
                output.push_str(REDACTED);
            } else {
                output.push_str(&rest[start..end]);
            }
            previous = rest[..end].chars().next_back();
            rest = &rest[end..];
        }
        output.push_str(rest);
        text = output;
    }
    text
}

fn redact_json(value: &mut serde_json::Value, secrets: &[String]) {
    match value {
        serde_json::Value::String(text) => *text = replace_all(text, secrets),
        serde_json::Value::Array(items) => {
            items.iter_mut().for_each(|item| redact_json(item, secrets))
        }
        serde_json::Value::Object(fields) => fields
            .values_mut()
            .for_each(|field| redact_json(field, secrets)),
        _ => {}
    }
}

/// The secrets of the project that holds `workspace`, found the way Unica
/// finds the workspace root: the nearest ancestor with a project
/// configuration or a `.git` file.
pub(super) fn project_secrets(workspace: &Path) -> Vec<String> {
    workspace
        .ancestors()
        .find(|root| root.join("v8project.yaml").is_file() || root.join(".git").is_file())
        .map(known_secrets)
        .unwrap_or_default()
}

/// `result` with every known secret hidden in its text: the summary, data,
/// warnings and diagnostics, where a runner's prose lands. The continuation,
/// revision, cursor, changed addresses and artifacts are built by Unica from
/// the request and its own paths and are kept as they are.
pub(super) fn redact_result(mut result: DomainResult, secrets: &[String]) -> DomainResult {
    if secrets.is_empty() {
        return result;
    }
    result.summary = replace_all(&result.summary, secrets);
    if let Some(data) = result.data.as_mut() {
        redact_json(data, secrets);
    }
    for value in result
        .warnings
        .iter_mut()
        .chain(result.diagnostics.iter_mut())
    {
        redact_json(value, secrets);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_secrets_come_from_secret_keys_and_the_connection_password() {
        let layer: Value = serde_yaml::from_str(
            "infobase:\n  connection: 'Srvr=s;Ref=ib;Usr=u;Pwd=conn-pass;'\n  user: operator\n  password: layer-pass\ninfobases:\n  origin:\n    password: origin-pass\ntools:\n  agent:\n    token: tool-token\n",
        )
        .unwrap();
        let mut secrets = Vec::new();
        collect(&layer, &mut secrets);
        secrets.sort();
        assert_eq!(
            secrets,
            ["conn-pass", "layer-pass", "origin-pass", "tool-token"]
        );
    }

    #[test]
    fn a_number_password_and_a_quoted_connection_password_are_secrets() {
        let layer: Value = serde_yaml::from_str(
            "infobase:\n  password: 123\n  connection: 'Srvr=\"s\";Ref=\"ib\";Pwd=\"a;\"\"b\";'\n",
        )
        .unwrap();
        let mut secrets = Vec::new();
        collect(&layer, &mut secrets);
        secrets.sort();
        assert_eq!(secrets, ["123", "a;\"\"b", "a;\"b"]);
        let mut secrets = Vec::new();
        connection_secrets("/F \"C:\\base\" /N admin /P qwerty", &mut secrets);
        connection_secrets("/F base /Pglued", &mut secrets);
        assert_eq!(secrets, ["qwerty", "glued"]);
        let layer: Value =
            serde_yaml::from_str("tools:\n  passwordFile: keys/agent\n  useSecret: false\n")
                .unwrap();
        let mut secrets = Vec::new();
        collect(&layer, &mut secrets);
        assert_eq!(secrets.len(), 0, "a path or a flag is not a secret");
    }

    #[test]
    fn a_short_password_hides_only_its_whole_word() {
        let secrets = vec!["1".to_string(), "admin".to_string()];
        assert_eq!(
            replace_all("platform 8.3.27.2074 uuid a1b2 login 1 as admin", &secrets),
            "platform 8.3.27.2074 uuid a1b2 login <redacted> as <redacted>"
        );
        assert_eq!(
            replace_all("/Users/admin/base Pwd=1;", &secrets),
            "/Users/<redacted>/base Pwd=<redacted>;"
        );
        assert_eq!(replace_all("administrator", &secrets), "administrator");
    }

    #[test]
    fn a_known_secret_is_hidden_in_the_result_text_but_not_in_its_continuation() {
        let secrets = vec!["s3cr3t".to_string()];
        let mut result = DomainResult::success("login s3cr3t failed");
        result.data = Some(serde_json::json!({"count": 1, "records": [{"version": "Pwd=s3cr3t"}]}));
        result
            .warnings
            .push(serde_json::json!({"message": "s3cr3t"}));
        result
            .diagnostics
            .push(serde_json::json!({"message": "bare s3cr3t"}));
        result
            .next
            .push(serde_json::json!({"args": {"cwd": "/work/s3cr3t"}}));
        let result = redact_result(result, &secrets);
        assert_eq!(result.summary, "login <redacted> failed");
        assert_eq!(result.data.as_ref().unwrap()["count"], 1);
        assert_eq!(
            result.data.as_ref().unwrap()["records"][0]["version"],
            "Pwd=<redacted>"
        );
        assert_eq!(result.warnings[0]["message"], "<redacted>");
        assert_eq!(result.diagnostics[0]["message"], "bare <redacted>");
        assert_eq!(result.next[0]["args"]["cwd"], "/work/s3cr3t");
    }
}
