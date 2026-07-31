use crate::application::AdapterOutcome;
use crate::domain::project_sources::SourceFormat;
use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::single_file_publisher::{
    publish, PublishMode, PublishRequest,
};
use crate::infrastructure::source_roots::resolve_named_source_set;
use roxmltree::{Document, Node};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::PathBuf;

const XDTO_NS: &str = "http://v8.1c.ru/8.1/xdto";

pub(crate) struct XdtoExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<Value>,
}

pub(crate) fn invoke_read(
    operation: &str,
    _tool: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Result<AdapterOutcome, String>> {
    (operation == "xdto-info").then(|| info(args, context).map(|execution| execution.outcome))
}

pub(crate) fn info_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<XdtoExecution, String> {
    info(args, context)
}

pub(crate) fn resolve_xdto_guard_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    resolve_package(args, context).map(|package| package.path)
}

pub(crate) fn preview_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> XdtoExecution {
    edit(args, context, true)
}
pub(crate) fn apply_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> XdtoExecution {
    edit(args, context, false)
}

fn info(args: &Map<String, Value>, context: &WorkspaceContext) -> Result<XdtoExecution, String> {
    let package = resolve_package(args, context)?;
    let raw =
        fs::read(&package.path).map_err(|error| format!("package_resource_missing: {error}"))?;
    let text = decode(&raw)?;
    let doc = parse(&text)?;
    let root = doc.root_element();
    let types = root.children().filter(|node| {
        node.is_element() && matches!(node.tag_name().name(), "valueType" | "objectType")
    });
    let requested = args
        .get("typeName")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty());
    let listed = types
        .filter(|node| requested.is_none_or(|name| node.attribute("name") == Some(name)))
        .map(type_json)
        .collect::<Vec<_>>();
    let data = json!({
        "sourceSet": package.source_set,
        "metadataPath": package.metadata_path,
        "location": {"addressability":"addressed"},
        "targetNamespace": root.attribute("targetNamespace"),
        "imports": root.children().filter(|node| node.has_tag_name((XDTO_NS, "import"))).filter_map(|node| node.attribute("namespace")).collect::<Vec<_>>(),
        "types": listed,
    });
    Ok(XdtoExecution {
        outcome: AdapterOutcome {
            ok: true,
            summary: "unica.xdto.info inspected XDTO package".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: vec![format!(
                "{} + {}",
                package.source_set, package.metadata_path
            )],
            stdout: None,
            stderr: None,
            command: None,
        },
        data: Some(data),
    })
}

fn edit(args: &Map<String, Value>, context: &WorkspaceContext, preview: bool) -> XdtoExecution {
    let planned = (|| -> Result<MutationPlan, String> {
        let package = resolve_package(args, context)?;
        let before = fs::read(&package.path)
            .map_err(|error| format!("package_resource_missing: {error}"))?;
        let text = decode(&before)?;
        let operation = required(args, "operation")?;
        let after_text = mutation(&text, args, operation)?;
        let after = encode_like(&before, &after_text);
        let post = decode(&after)?;
        parse(&post)?;
        let no_op = before == after;
        let data = json!({"sourceSet": package.source_set, "metadataPath": package.metadata_path, "operation": operation, "noOp": no_op, "findings": if no_op { vec![json!({"code":"duplicate_or_already_applied"})] } else { Vec::new() }});
        Ok(MutationPlan {
            package,
            before,
            after,
            data,
            no_op,
        })
    })();
    let MutationPlan {
        package,
        before,
        after,
        data,
        no_op,
    } = match planned {
        Ok(plan) => plan,
        Err(error) => {
            return XdtoExecution {
                outcome: AdapterOutcome {
                    ok: false,
                    summary: "unica.xdto.edit rejected XDTO mutation".to_string(),
                    changes: Vec::new(),
                    warnings: Vec::new(),
                    errors: vec![error.clone()],
                    artifacts: Vec::new(),
                    stdout: None,
                    stderr: Some(format!("{error}\n")),
                    command: None,
                },
                data: None,
            }
        }
    };
    if !preview && !no_op {
        if let Err(error) = publish(PublishRequest {
            target: &package.path,
            replacement: &after,
            mode: PublishMode::ReplaceExisting {
                expected_preimage: &before,
            },
        }) {
            return XdtoExecution {
                outcome: AdapterOutcome {
                    ok: false,
                    summary: "unica.xdto.edit could not publish XDTO mutation".to_string(),
                    changes: Vec::new(),
                    warnings: Vec::new(),
                    errors: vec![error.to_string()],
                    artifacts: Vec::new(),
                    stdout: None,
                    stderr: None,
                    command: None,
                },
                data: Some(data),
            };
        }
    }
    XdtoExecution {
        outcome: AdapterOutcome {
            ok: true,
            summary: if no_op {
                "unica.xdto.edit is already applied".to_string()
            } else if preview {
                "dry run: unica.xdto.edit planned XDTO mutation".to_string()
            } else {
                "unica.xdto.edit applied XDTO mutation".to_string()
            },
            changes: (!preview && !no_op)
                .then(|| {
                    format!(
                        "{} + {}: XDTO package updated",
                        package.source_set, package.metadata_path
                    )
                })
                .into_iter()
                .collect(),
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: vec![format!(
                "{} + {}",
                package.source_set, package.metadata_path
            )],
            stdout: None,
            stderr: None,
            command: None,
        },
        data: Some(data),
    }
}

struct Package {
    path: PathBuf,
    source_set: String,
    metadata_path: String,
}

struct MutationPlan {
    package: Package,
    before: Vec<u8>,
    after: Vec<u8>,
    data: Value,
    no_op: bool,
}

fn resolve_package(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<Package, String> {
    let source_set = required(args, "sourceSet")?;
    let raw_address = required(args, "metadataPath")?;
    let address = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw_address)
        .map_err(|error| format!("target_not_found: {error}"))?;
    let parts = address.as_str().split('.').collect::<Vec<_>>();
    if parts.len() != 2 || parts[0] != "XDTOPackage" {
        return Err("not_an_xdto_package: metadataPath must be XDTOPackage.<name>".to_string());
    }
    if parts[1].is_empty() || parts[1].contains(['/', '\\']) || parts[1] == "." || parts[1] == ".."
    {
        return Err("containment_denied: XDTO package name is not a path segment".to_string());
    }
    let selected = resolve_named_source_set(context, source_set)
        .map_err(|_| format!("source_set_unknown: `{source_set}`"))?;
    if selected.source_set.source_format != SourceFormat::PlatformXml {
        return Err("not_an_xdto_package: sourceSet is not Platform XML".to_string());
    }
    let path = selected
        .path
        .join("XDTOPackages")
        .join(parts[1])
        .join("Ext")
        .join("Package.bin");
    if !path.is_file() {
        return Err(
            "package_resource_missing: logical XDTO package has no Package.bin resource"
                .to_string(),
        );
    }
    let identity = crate::infrastructure::source_roots::normalize_path_identity(&path)
        .map_err(|_| "containment_denied: cannot resolve XDTO package resource".to_string())?;
    if !identity.starts_with(&selected.path) {
        return Err("containment_denied: XDTO package resource escapes sourceSet".to_string());
    }
    Ok(Package {
        path,
        source_set: selected.source_set.name,
        metadata_path: address.as_str().to_string(),
    })
}

fn mutation(text: &str, args: &Map<String, Value>, operation: &str) -> Result<String, String> {
    let doc = parse(text)?;
    let root = doc.root_element();
    match operation {
        "add-value-type" => {
            let name = required(args, "name")?; let base = required(args, "base")?;
            if named_type(root, name).is_some() { return Ok(text.to_string()); }
            insert_before_close(text, root, &format!("<valueType name=\"{}\" base=\"{}\"/>", esc(name), esc(base)))
        }
        "add-object-type" => {
            let name = required(args, "name")?;
            if named_type(root, name).is_some() { return Ok(text.to_string()); }
            insert_before_close(
                text,
                root,
                &format!("<objectType name=\"{}\"></objectType>", esc(name)),
            )
        }
        "add-property" => {
            let type_name = required(args, "typeName")?;
            let property = args.get("property").and_then(Value::as_object).ok_or_else(|| "property must be an object".to_string())?;
            let name = object_string(property, "name")?; let kind = object_string(property, "type")?;
            let target = property_target(root, type_name, args.get("propertyPath").and_then(Value::as_str))?;
            if target.children().any(|node| node.has_tag_name((XDTO_NS, "property")) && node.attribute("name") == Some(name)) { return Ok(text.to_string()); }
            let lower = property.get("minOccurs").or_else(|| property.get("lowerBound")).and_then(Value::as_u64).map(|value| format!(" lowerBound=\"{value}\"")).unwrap_or_default();
            insert_before_close(text, target, &format!("<property name=\"{}\" type=\"{}\"{lower}/>", esc(name), esc(kind)))
        }
        "remove-type" => remove_named(text, named_type(root, required(args, "name")?).ok_or_else(|| "target_not_found: type does not exist".to_string())?),
        "remove-property" => {
            let target = property_target(root, required(args, "typeName")?, args.get("propertyPath").and_then(Value::as_str))?;
            let name = required(args, "name")?;
            remove_named(text, target.children().find(|node| node.has_tag_name((XDTO_NS, "property")) && node.attribute("name") == Some(name)).ok_or_else(|| "target_not_found: property does not exist".to_string())?)
        }
        _ => Err("unsupported_node: supported operations are add-value-type, add-object-type, add-property, remove-type, remove-property".to_string()),
    }
}

fn property_target<'a>(
    root: Node<'a, 'a>,
    type_name: &str,
    property_path: Option<&str>,
) -> Result<Node<'a, 'a>, String> {
    let mut node = named_type(root, type_name)
        .ok_or_else(|| "target_not_found: type does not exist".to_string())?;
    for segment in property_path
        .unwrap_or("")
        .split('.')
        .filter(|part| !part.is_empty())
    {
        let property = node
            .children()
            .find(|child| {
                child.has_tag_name((XDTO_NS, "property"))
                    && child.attribute("name") == Some(segment)
            })
            .ok_or_else(|| {
                format!("target_not_found: property path segment `{segment}` does not exist")
            })?;
        node = property
            .children()
            .find(|child| child.has_tag_name((XDTO_NS, "typeDef")))
            .ok_or_else(|| {
                format!("unsupported_node: property path segment `{segment}` has no nested typeDef")
            })?;
    }
    Ok(node)
}
fn named_type<'a>(root: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
    root.children().find(|node| {
        node.is_element()
            && matches!(node.tag_name().name(), "valueType" | "objectType")
            && node.attribute("name") == Some(name)
    })
}
fn remove_named(text: &str, node: Node<'_, '_>) -> Result<String, String> {
    let range = node.range();
    let line_start = text[..range.start]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let leading_is_whitespace = text[line_start..range.start]
        .chars()
        .all(|character| matches!(character, ' ' | '\t' | '\r'));
    let start = if leading_is_whitespace {
        line_start
    } else {
        range.start
    };
    let end = text[range.end..]
        .strip_prefix("\r\n")
        .map(|_| range.end + 2)
        .or_else(|| text[range.end..].strip_prefix('\n').map(|_| range.end + 1))
        .unwrap_or(range.end);
    Ok(format!("{}{}", &text[..start], &text[end..]))
}
fn insert_before_close(text: &str, node: Node<'_, '_>, fragment: &str) -> Result<String, String> {
    let range = node.range();
    let close = text[range.start..range.end]
        .rfind("</")
        .map(|index| range.start + index)
        .ok_or_else(|| {
            "unsupported_node: insertion target must have an explicit closing tag".to_string()
        })?;
    let parent_indent = line_indent(text, close);
    let close_line_start = text[..close]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let prefix_end = if text[close_line_start..close] == *parent_indent {
        close_line_start
    } else {
        close
    };
    let child_indent = format!("{parent_indent}\t");
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    Ok(format!(
        "{}{}{}{}{}{}{}",
        &text[..prefix_end],
        eol,
        child_indent,
        fragment,
        eol,
        parent_indent,
        &text[close..]
    ))
}
fn line_indent(text: &str, offset: usize) -> &str {
    let start = text[..offset]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let line = &text[start..offset];
    let width = line
        .char_indices()
        .take_while(|(_, character)| matches!(character, ' ' | '\t' | '\r'))
        .last()
        .map(|(index, character)| index + character.len_utf8())
        .unwrap_or(0);
    &line[..width]
}
fn decode(raw: &[u8]) -> Result<String, String> {
    std::str::from_utf8(raw)
        .map(|value| value.trim_start_matches('\u{feff}').to_string())
        .map_err(|_| "unsupported_node: Package.bin is not UTF-8".to_string())
}
fn encode_like(before: &[u8], text: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    if before.starts_with(&[0xef, 0xbb, 0xbf]) {
        bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
    }
    bytes.extend_from_slice(text.as_bytes());
    bytes
}
fn parse(text: &str) -> Result<Document<'_>, String> {
    let doc = Document::parse(text)
        .map_err(|error| format!("unsupported_node: invalid XDTO XML: {error}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "package" || root.tag_name().namespace() != Some(XDTO_NS) {
        return Err("not_an_xdto_package: Package.bin root is not an XDTO package".to_string());
    }
    Ok(doc)
}
fn required<'a>(args: &'a Map<String, Value>, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{name} must be a non-empty string"))
}
fn object_string<'a>(object: &'a Map<String, Value>, name: &str) -> Result<&'a str, String> {
    object
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("property.{name} must be a non-empty string"))
}
fn esc(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}
fn type_json(node: Node<'_, '_>) -> Value {
    json!({"kind": node.tag_name().name(), "name": node.attribute("name"), "base": node.attribute("base"), "properties": node.children().filter(|child| child.has_tag_name((XDTO_NS, "property"))).map(|child| json!({"name": child.attribute("name"), "type": child.attribute("type"), "lowerBound": child.attribute("lowerBound")})).collect::<Vec<_>>()})
}

#[cfg(test)]
mod tests {
    use super::{decode, encode_like, mutation};
    use serde_json::{json, Map, Value};

    const PACKAGE: &str = r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" targetNamespace="urn:test">
	<objectType name="ЛюбаяСсылка"><property name="СсылкаНаОбъект"><typeDef xsi:type="ObjectType"></typeDef></property></objectType>
	<objectType name="СоставнойЛюбойОбъект"/>
</package>"#;

    fn args(entries: &[(&str, Value)]) -> Map<String, Value> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    #[test]
    fn add_property_descends_through_property_path_and_is_idempotent() {
        let args = args(&[
            ("typeName", json!("ЛюбаяСсылка")),
            ("propertyPath", json!("СсылкаНаОбъект")),
            (
                "property",
                json!({"name":"Документ_Новый", "type":"Документ_Новый", "minOccurs":0}),
            ),
        ]);
        let once = mutation(PACKAGE, &args, "add-property").unwrap();
        assert_eq!(
            once,
            r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" targetNamespace="urn:test">
	<objectType name="ЛюбаяСсылка"><property name="СсылкаНаОбъект"><typeDef xsi:type="ObjectType">
		<property name="Документ_Новый" type="Документ_Новый" lowerBound="0"/>
	</typeDef></property></objectType>
	<objectType name="СоставнойЛюбойОбъект"/>
</package>"#
        );
        assert_eq!(mutation(&once, &args, "add-property").unwrap(), once);
    }

    #[test]
    fn byte_encoding_keeps_bom_and_crlf() {
        let before = format!("\u{feff}{}", PACKAGE.replace('\n', "\r\n")).into_bytes();
        let text = decode(&before).unwrap();
        let after = mutation(
            &text,
            &args(&[("name", json!("Новый")), ("base", json!("xs:string"))]),
            "add-value-type",
        )
        .unwrap();
        let bytes = encode_like(&before, &after);
        assert!(bytes.starts_with(&[0xef, 0xbb, 0xbf]));
        assert!(decode(&bytes).unwrap().contains("\r\n"));
    }
}
