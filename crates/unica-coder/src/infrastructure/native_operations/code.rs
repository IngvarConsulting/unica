use crate::application::AdapterOutcome;
use crate::domain::project_sources::{SourceFormat, SourceSetKind};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::path_policy::WorkspacePathPolicy;
use crate::infrastructure::project_sources::discover_project_source_map;
use crate::infrastructure::source_roots::resolve_source_root;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

use super::single_file_publisher::{publish, PublishMode, PublishRequest};

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
        let before = fs::read(&target.path)
            .map_err(|error| format!("failed to read {}: {error}", target.path.display()))?;
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
        let postimage = std::str::from_utf8(&after)
            .map_err(|_| "patched Module.bsl must remain UTF-8".to_string())?;
        methods(postimage).map_err(|error| format!("validate patched Module.bsl: {error}"))?;
        let (start_line, start_column) = line_column(postimage, offset);
        let (end_line, end_column) = line_column(postimage, offset + insertion.len());
        let relative = target
            .path
            .strip_prefix(&context.workspace_root)
            .unwrap_or(&target.path)
            .display()
            .to_string();
        let details = json!({
            "path": relative,
            "sourceSet": target.source_set,
            "preHash": hash(&before),
            "postHash": hash(&after),
            "noOp": no_op,
            "changedRanges": if no_op { Vec::<Value>::new() } else { vec![json!({
                "startByte": offset,
                "endByte": offset + insertion.len(),
                "startLine": start_line,
                "startColumn": start_column,
                "endLine": end_line,
                "endColumn": end_column,
            })] },
            "diff": unified_diff(&relative, &before, offset, &insertion, no_op),
            "affectedTarget": {
                "path": relative,
                "sourceSet": target.source_set,
                "moduleRole": target.module_role,
                "rawHash": hash(&after),
            },
        });
        if !dry_run && !no_op {
            publish(PublishRequest {
                target: &target.path,
                replacement: &after,
                mode: PublishMode::ReplaceExisting {
                    expected_preimage: &before,
                },
            })
            .map_err(|error| format!("publish BSL module: {error}"))?;
        }
        let stdout = serde_json::to_string_pretty(&details)
            .map_err(|error| format!("serialize code patch details: {error}"))?;
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
                .then(|| format!("{}: inserted BSL content", target.path.display()))
                .into_iter()
                .collect(),
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: vec![target.path.display().to_string()],
            stdout: Some(stdout),
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

struct ResolvedTarget {
    path: PathBuf,
    source_set: String,
    module_role: String,
}

fn resolve_target(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<ResolvedTarget, String> {
    let target = WorkspacePathPolicy::new(context).resolve_write(string_arg(args, "path")?)?;
    if !target
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with("Module.bsl"))
        || !target.is_file()
    {
        return Err("unica.code.patch v1 accepts only an existing *Module.bsl".to_string());
    }
    let source_root = resolve_source_root(context, args.get("sourceDir").and_then(Value::as_str))?;
    let source_name = source_root
        .source_set
        .as_deref()
        .ok_or_else(|| "sourceDir must select a configured Configuration source set".to_string())?;
    let source_map = discover_project_source_map(&context.workspace_root)?;
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
    if !target.starts_with(&source_root.path) {
        return Err("Module.bsl is outside the selected Configuration source set".to_string());
    }
    let module_role = target
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Module.bsl filename is not valid UTF-8".to_string())?
        .to_string();
    Ok(ResolvedTarget {
        path: target,
        source_set: source_name.to_string(),
        module_role,
    })
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
    let methods = methods(text)?;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MethodKind {
    Procedure,
    Function,
}

#[derive(Debug)]
struct Method<'a> {
    name: &'a str,
    start: usize,
    end: usize,
}

fn methods(text: &str) -> Result<Vec<Method<'_>>, String> {
    let mut result = Vec::new();
    let mut open: Option<(&str, usize, MethodKind)> = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start().trim_start_matches('\u{feff}');
        let declaration = trimmed
            .strip_prefix("Процедура ")
            .map(|rest| (rest, MethodKind::Procedure))
            .or_else(|| {
                trimmed
                    .strip_prefix("Функция ")
                    .map(|rest| (rest, MethodKind::Function))
            })
            .or_else(|| {
                trimmed
                    .strip_prefix("Procedure ")
                    .map(|rest| (rest, MethodKind::Procedure))
            })
            .or_else(|| {
                trimmed
                    .strip_prefix("Function ")
                    .map(|rest| (rest, MethodKind::Function))
            });
        if let Some((rest, kind)) = declaration {
            if open.is_none() {
                let name = rest
                    .split(|ch: char| ch == '(' || ch.is_whitespace())
                    .next()
                    .unwrap_or("");
                if !name.is_empty() {
                    open = Some((name, offset, kind));
                }
            }
        } else if let Some(closing_kind) = closing_kind(trimmed) {
            if let Some((name, start, opening_kind)) = open.take() {
                if closing_kind != opening_kind {
                    return Err(
                        "BSL method closing token does not match its declaration".to_string()
                    );
                }
                result.push(Method {
                    name,
                    start,
                    end: offset + line.len(),
                });
            }
        }
        offset += line.len();
    }
    if open.is_some() {
        return Err("BSL method declaration has no closing token".to_string());
    }
    Ok(result)
}

fn closing_kind(line: &str) -> Option<MethodKind> {
    if line.starts_with("КонецПроцедуры") || line.starts_with("EndProcedure") {
        Some(MethodKind::Procedure)
    } else if line.starts_with("КонецФункции") || line.starts_with("EndFunction") {
        Some(MethodKind::Function)
    } else {
        None
    }
}

fn line_end(text: &str, from: usize) -> usize {
    text[from..]
        .find('\n')
        .map(|offset| from + offset + 1)
        .unwrap_or(text.len())
}

fn line_column(text: &str, offset: usize) -> (usize, usize) {
    let prefix = &text[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix, |(_, current)| current)
        .chars()
        .count()
        + 1;
    (line, column)
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

fn unified_diff(path: &str, before: &[u8], offset: usize, insertion: &[u8], no_op: bool) -> String {
    if no_op {
        return String::new();
    }
    let line = before[..offset]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
        + 1;
    let added = String::from_utf8_lossy(insertion)
        .lines()
        .map(|line| format!("+{line}\n"))
        .collect::<String>();
    let added_lines = insertion.iter().filter(|byte| **byte == b'\n').count()
        + usize::from(!insertion.ends_with(b"\n"));
    format!("--- a/{path}\n+++ b/{path}\n@@ -{line},0 +{line},{added_lines} @@\n{added}")
}

#[cfg(test)]
mod tests {
    use super::{
        insertion_is_present, line_column, locate_insertion, methods, normalized_content,
        patch_inner, unified_diff,
    };
    use crate::domain::workspace::WorkspaceContext;
    use serde_json::{json, Map, Value};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

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

    #[test]
    fn method_selector_accepts_bom_prefixed_english_bsl() {
        let module = "\u{feff}Procedure Run()\n    Message(\"ok\");\nEndProcedure\n";
        let args = arguments(json!({"method": "Run"}), "after");

        assert!(locate_insertion(module, &args).is_ok());
    }

    #[test]
    fn diff_uses_a_valid_insertion_hunk() {
        let diff = unified_diff(
            "CommonModules/X/Ext/Module.bsl",
            b"one\ntwo\n",
            4,
            b"insert\n",
            false,
        );

        assert!(diff.contains("@@ -2,0 +2,1 @@"));
        assert!(diff.contains("+insert\n"));
    }

    #[test]
    fn index_rejects_an_unclosed_method() {
        assert!(methods("Procedure Run()\n").is_err());
    }

    #[test]
    fn line_column_reports_utf8_character_columns() {
        assert_eq!(line_column("Процедура Run()\n", 0), (1, 1));
        assert_eq!(
            line_column("Процедура Run()\n", "Процедура ".len()),
            (1, 11)
        );
        assert_eq!(line_column("A\nBC", 3), (2, 2));
    }

    #[test]
    fn applied_patch_reports_typed_target_and_repeated_apply_is_noop() {
        let context = temp_context("applied-patch");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(&module, "\u{feff}Procedure Run()\r\nEndProcedure\r\n").unwrap();
        let args = patch_args(
            "src/CommonModules/Sample/Ext/Module.bsl",
            "Run",
            "Message(\"ok\");",
        );

        let applied = patch_inner(&args, &context, false);
        assert!(applied.ok, "{:?}", applied.errors);
        let details: Value = serde_json::from_str(applied.stdout.as_deref().unwrap()).unwrap();
        assert_eq!(details["sourceSet"], "main");
        assert_eq!(details["affectedTarget"]["moduleRole"], "Module");
        assert_eq!(details["changedRanges"][0]["startLine"], 3);
        assert!(details["diff"].as_str().unwrap().starts_with("--- a/"));

        let repeated = patch_inner(&args, &context, false);
        assert!(repeated.ok, "{:?}", repeated.errors);
        assert!(repeated.changes.is_empty());
        let details: Value = serde_json::from_str(repeated.stdout.as_deref().unwrap()).unwrap();
        assert_eq!(details["preHash"], details["postHash"]);
        assert!(details["changedRanges"].as_array().unwrap().is_empty());
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    fn arguments(selector: Value, position: &str) -> Map<String, Value> {
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("insert"));
        args.insert("selector".to_string(), selector);
        args.insert("position".to_string(), json!(position));
        args
    }

    fn patch_args(path: &str, method: &str, content: &str) -> Map<String, Value> {
        let mut args = arguments(json!({"method": method}), "after");
        args.insert("path".to_string(), json!(path));
        args.insert("content".to_string(), json!(content));
        args.insert("sourceDir".to_string(), json!("src"));
        args
    }

    fn temp_context(name: &str) -> WorkspaceContext {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("unica-code-patch-{name}-{nonce}"));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::write(root.join("src/Configuration.xml"), "<MetaDataObject/>").unwrap();
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }
}
