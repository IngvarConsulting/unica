use crate::application::AdapterOutcome;
use crate::domain::project_sources::{SourceFormat, SourceSetKind};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::path_policy::WorkspacePathPolicy;
use crate::infrastructure::project_sources::discover_project_source_map;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn invoke_mutation(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<AdapterOutcome> {
    (operation == "code-patch").then(|| patch_inner(args, context, false))
}

pub(crate) fn preview(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    patch_inner(args, context, true)
}

fn patch_inner(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    dry_run: bool,
) -> AdapterOutcome {
    let result: Result<AdapterOutcome, String> = (|| {
        let target = resolve_target(args, context)?;
        let before = fs::read(&target)
            .map_err(|error| format!("failed to read {}: {error}", target.display()))?;
        let text =
            std::str::from_utf8(&before).map_err(|_| "Module.bsl must be UTF-8".to_string())?;
        let offset = locate_insertion(text, args)?;
        let insertion = normalized_content(string_arg(args, "content")?, local_eol(text));
        let no_op = insertion_is_present(
            text.as_bytes(),
            offset,
            &insertion,
            string_arg(args, "position")?,
        );
        let mut after = before.clone();
        if !no_op {
            after.splice(offset..offset, insertion.iter().copied());
        }
        let relative = target
            .strip_prefix(&context.workspace_root)
            .unwrap_or(&target)
            .display()
            .to_string();
        let details = json!({
            "path": relative,
            "preHash": hash(&before),
            "postHash": hash(&after),
            "noOp": no_op,
            "changedRanges": if no_op { Vec::<Value>::new() } else { vec![json!({"startByte": offset, "endByte": offset + insertion.len()})] },
            "diff": unified_diff(&relative, &insertion, no_op),
        });
        if !dry_run && !no_op {
            atomic_replace(&target, &after)?;
        }
        Ok(AdapterOutcome {
            ok: true,
            summary: if no_op {
                "unica.code.patch is already applied".to_string()
            } else if dry_run {
                "dry run: unica.code.patch planned one insertion".to_string()
            } else {
                "unica.code.patch applied one insertion".to_string()
            },
            changes: (!no_op)
                .then(|| format!("{}: inserted BSL content", target.display()))
                .into_iter()
                .collect(),
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: vec![target.display().to_string()],
            stdout: Some(serde_json::to_string_pretty(&details).expect("details serialize")),
            stderr: None,
            command: None,
        })
    })();
    result.unwrap_or_else(|error| AdapterOutcome {
        ok: false,
        summary: "unica.code.patch failed".to_string(),
        changes: Vec::new(),
        warnings: Vec::new(),
        errors: vec![error.clone()],
        artifacts: Vec::new(),
        stdout: None,
        stderr: Some(format!("{error}\n")),
        command: None,
    })
}

fn resolve_target(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    let target = WorkspacePathPolicy::new(context).resolve_write(string_arg(args, "path")?)?;
    if target.file_name().and_then(|name| name.to_str()) != Some("Module.bsl") || !target.is_file()
    {
        return Err("unica.code.patch v1 accepts only an existing Module.bsl".to_string());
    }
    let source_map = discover_project_source_map(&context.workspace_root)?;
    let source_name = source_map
        .effective_source_set
        .as_deref()
        .ok_or_else(|| "no unambiguous Configuration source set is available".to_string())?;
    let source_root = source_map
        .effective_source_root
        .as_deref()
        .ok_or_else(|| "no unambiguous Configuration source set is available".to_string())?;
    let source_set = source_map
        .source_sets
        .iter()
        .find(|set| set.name == source_name)
        .ok_or_else(|| "effective source set is unavailable".to_string())?;
    if source_set.kind != SourceSetKind::Configuration
        || source_set.source_format != SourceFormat::PlatformXml
    {
        return Err(
            "unica.code.patch v1 requires a platform XML Configuration source set".to_string(),
        );
    }
    if !target.starts_with(Path::new(source_root)) {
        return Err("Module.bsl is outside the selected Configuration source set".to_string());
    }
    Ok(target)
}

fn locate_insertion(text: &str, args: &Map<String, Value>) -> Result<usize, String> {
    if string_arg(args, "operation")? != "insert" {
        return Err("unica.code.patch v1 supports only operation=insert".to_string());
    }
    let position = string_arg(args, "position")?;
    let selector = args
        .get("selector")
        .and_then(Value::as_object)
        .ok_or_else(|| "selector must be an object".to_string())?;
    if selector.len() != 1 {
        return Err("selector must contain exactly one of method or anchor".to_string());
    }
    let methods = methods(text);
    let (start, end) = if let Some(name) = selector.get("method").and_then(Value::as_str) {
        let found = methods
            .iter()
            .filter(|method| method.name == name)
            .collect::<Vec<_>>();
        if found.len() != 1 {
            return Err(format!(
                "method selector must match exactly once; matched {} times",
                found.len()
            ));
        }
        (found[0].start, found[0].end)
    } else if let Some(anchor) = selector.get("anchor").and_then(Value::as_str) {
        let found = text.match_indices(anchor).collect::<Vec<_>>();
        if found.len() != 1 {
            return Err(format!(
                "anchor selector must match exactly once; matched {} times",
                found.len()
            ));
        }
        let start = found[0].0;
        let end = start + anchor.len();
        if !methods
            .iter()
            .any(|method| start >= method.start && end <= method.end)
        {
            return Err("anchor selector must be inside exactly one BSL method".to_string());
        }
        (start, end)
    } else {
        return Err("selector must contain exactly one of method or anchor".to_string());
    };
    match position {
        "before" => Ok(start),
        "after" => Ok(if selector.contains_key("method") {
            end
        } else {
            line_end(text, end)
        }),
        _ => Err("position must be before or after".to_string()),
    }
}

#[derive(Debug)]
struct Method<'a> {
    name: &'a str,
    start: usize,
    end: usize,
}

fn methods(text: &str) -> Vec<Method<'_>> {
    let mut result = Vec::new();
    let mut open: Option<(&str, usize)> = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed
            .strip_prefix("Процедура ")
            .or_else(|| trimmed.strip_prefix("Функция "))
        {
            if open.is_none() {
                let name = rest
                    .split(|ch: char| ch == '(' || ch.is_whitespace())
                    .next()
                    .unwrap_or("");
                if !name.is_empty() {
                    open = Some((name, offset));
                }
            }
        } else if (trimmed.starts_with("КонецПроцедуры") || trimmed.starts_with("КонецФункции"))
            && open.is_some()
        {
            let (name, start) = open.take().expect("checked");
            result.push(Method {
                name,
                start,
                end: offset + line.len(),
            });
        }
        offset += line.len();
    }
    result
}

fn line_end(text: &str, from: usize) -> usize {
    text[from..]
        .find('\n')
        .map(|offset| from + offset + 1)
        .unwrap_or(text.len())
}

fn local_eol(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn normalized_content(content: &str, eol: &str) -> Vec<u8> {
    let normalized = content
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', eol);
    let mut bytes = normalized.into_bytes();
    if !bytes.ends_with(eol.as_bytes()) {
        bytes.extend_from_slice(eol.as_bytes());
    }
    bytes
}

fn insertion_is_present(text: &[u8], offset: usize, insertion: &[u8], position: &str) -> bool {
    match position {
        "before" => text
            .get(..offset)
            .is_some_and(|head| head.ends_with(insertion)),
        "after" => text
            .get(offset..)
            .is_some_and(|tail| tail.starts_with(insertion)),
        _ => false,
    }
}

fn string_arg<'a>(args: &'a Map<String, Value>, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("unica.code.patch requires non-empty `{name}`"))
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn unified_diff(path: &str, insertion: &[u8], no_op: bool) -> String {
    if no_op {
        return String::new();
    }
    format!(
        "--- a/{path}\n+++ b/{path}\n@@ insertion @@\n+{}",
        String::from_utf8_lossy(insertion)
    )
}

fn atomic_replace(target: &Path, bytes: &[u8]) -> Result<(), String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let staged = target.with_file_name(format!(
        ".{}.unica-stage-{}-{nonce}",
        target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Module.bsl"),
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)
        .map_err(|error| format!("create staging file: {error}"))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("write staging file: {error}"))?;
    fs::rename(&staged, target).map_err(|error| {
        let _ = fs::remove_file(&staged);
        format!("replace BSL module: {error}")
    })
}

#[cfg(test)]
mod tests {
    use super::{insertion_is_present, locate_insertion, normalized_content};
    use serde_json::{json, Map, Value};

    const MODULE: &str = "Процедура Первая()\n    Сообщить(\"один\");\nКонецПроцедуры\n\nФункция Вторая()\n    Возврат Истина;\nКонецФункции\n";

    #[test]
    fn method_selector_places_after_the_complete_method() {
        let args = arguments(json!({"method": "Первая"}), "after");
        let offset = locate_insertion(MODULE, &args).unwrap();
        assert!(MODULE[offset..].starts_with("\nФункция Вторая()"));
    }

    #[test]
    fn anchor_must_be_unique_and_inside_a_method() {
        let args = arguments(json!({"anchor": "Сообщить(\"один\");"}), "before");
        assert!(locate_insertion(MODULE, &args).is_ok());

        let args = arguments(json!({"anchor": "КонецПроцедуры"}), "before");
        assert!(locate_insertion(MODULE, &args).is_ok());

        let args = arguments(json!({"anchor": "отсутствует"}), "before");
        assert!(locate_insertion(MODULE, &args).is_err());
    }

    #[test]
    fn inserted_content_uses_local_line_ending_once() {
        assert_eq!(normalized_content("A\r\nB", "\n"), b"A\nB\n");
        assert_eq!(normalized_content("A\nB", "\r\n"), b"A\r\nB\r\n");
    }

    #[test]
    fn repeated_before_or_after_insertion_is_a_noop() {
        let before = b"// marker\nProcedure First()";
        assert!(insertion_is_present(before, 10, b"// marker\n", "before"));
        let after = b"Procedure First()\n// marker\n";
        let offset = after.len() - b"\n// marker\n".len();
        assert!(insertion_is_present(
            after,
            offset,
            b"\n// marker\n",
            "after"
        ));
    }

    fn arguments(selector: Value, position: &str) -> Map<String, Value> {
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("insert"));
        args.insert("selector".to_string(), selector);
        args.insert("position".to_string(), json!(position));
        args
    }
}
