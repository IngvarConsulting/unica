use crate::application::{AdapterOutcome, SupportGuardRequirement};
use crate::domain::source_target::{
    MetadataAddress, SourceTarget, SourceTargetError, SourceTargetErrorCode, TargetKind,
    PLATFORM_XML_8_3_27_FORMAT_2_20,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::common::guard_resolved_platform_xml_target_dependencies;
use crate::infrastructure::native_operations::compile_transaction::CompileTransaction;
use crate::infrastructure::path_policy::WorkspacePathPolicy;
use crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point;
use crate::infrastructure::platform_xml_source_targets::{
    platform_xml_resource_evidence, resolve_platform_xml_target, ClosedPlatformXmlTarget,
    PlatformXmlResourceEvidence, TargetKindPolicy,
};
use crate::infrastructure::source_roots::normalize_path_identity;
use crate::infrastructure::support_guard::{
    evaluate_resolved_support_guard, ResolvedSupportGuardCheck,
};
use roxmltree::{Document, Node};
use serde_json::{json, Map, Value};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

mod model;
mod validation;

use model::{PackageModel, XDTO_NS};
use validation::{validate, ValidationDiff};

const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

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
    let planned = (|| -> Result<MutationPlan, PlanningFailure> {
        let package = resolve_package(args, context)?;
        let before = fs::read(&package.path)
            .map_err(|error| format!("package_resource_missing: {error}"))?;
        let text = decode(&before)?;
        let before_model = PackageModel::parse(&text).map_err(model_error)?;
        let descriptor_namespace = descriptor_namespace(&package.descriptor_path)?;
        if before_model.target_namespace() != Some(descriptor_namespace.as_str()) {
            return Err(PlanningFailure::plain(
                "namespace_mismatch: descriptor Namespace must equal package targetNamespace",
            ));
        }
        let baseline_findings = validate(&before_model);
        let operation = required(args, "operation")?;
        let after_text = mutation(&text, args, operation)?;
        let after = encode_like(&before, &after_text);
        let post = decode(&after)?;
        let after_model = PackageModel::parse(&post).map_err(model_error)?;
        let validation = ValidationDiff::between(&baseline_findings, validate(&after_model));
        let no_op = before == after;
        let mut findings = serde_json::to_value(&validation.findings)
            .expect("typed XDTO findings must serialize")
            .as_array()
            .cloned()
            .expect("typed XDTO findings serialize as an array");
        if no_op {
            findings.push(json!({
                "code": "duplicate_or_already_applied",
                "severity": "info",
                "state": "pre_existing",
                "message": "the requested XDTO mutation is already applied",
                "location": {"key": "$operation", "span": {"start": 0, "end": 0}}
            }));
        }
        let data = json!({"sourceSet": package.source_set, "metadataPath": package.metadata_path, "operation": operation, "noOp": no_op, "findings": findings});
        if validation.blocks() {
            let codes = validation
                .findings
                .iter()
                .filter(|finding| finding.state == validation::FindingState::Introduced)
                .map(|finding| finding.code.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(PlanningFailure {
                error: format!("xdto_validation_failed: introduced findings: {codes}"),
                data: Some(data),
            });
        }
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
        Err(failure) => {
            return XdtoExecution {
                outcome: AdapterOutcome {
                    ok: false,
                    summary: "unica.xdto.edit rejected XDTO mutation".to_string(),
                    changes: Vec::new(),
                    warnings: Vec::new(),
                    errors: vec![failure.error.clone()],
                    artifacts: Vec::new(),
                    stdout: None,
                    stderr: Some(format!("{}\n", failure.error)),
                    command: None,
                },
                data: failure.data,
            }
        }
    };
    if !preview && !no_op {
        let publish_result = (|| -> Result<(), String> {
            let mut transaction = CompileTransaction::new();
            transaction.replace_bytes(&package.path, &before, after.clone())?;
            let descriptor = guard_resolved_platform_xml_target_dependencies(
                &mut transaction,
                &package.handle,
                context,
            )?;
            if descriptor != package.descriptor_path {
                return Err("target_not_found: resolved XDTO descriptor changed".to_string());
            }
            let resource = resolve_xdto_resource(&package.handle, context)?;
            if resource != package.path {
                return Err("containment_denied: resolved XDTO resource changed".to_string());
            }
            guard_resolved_support(&resource, context)?;
            transaction.commit()?;
            Ok(())
        })();
        if let Err(error) = publish_result {
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
    descriptor_path: PathBuf,
    handle: ClosedPlatformXmlTarget,
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

struct PlanningFailure {
    error: String,
    data: Option<Value>,
}

impl PlanningFailure {
    fn plain(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            data: None,
        }
    }
}

impl From<String> for PlanningFailure {
    fn from(error: String) -> Self {
        Self::plain(error)
    }
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
    let target = SourceTarget {
        source_set: source_set.to_string(),
        metadata_path: Some(address),
    };
    let resolution = resolve_platform_xml_target(context, &target, TargetKindPolicy::Any)
        .map_err(map_xdto_target_error)?;
    if resolution.resolved.target_kind != TargetKind::MetadataObject {
        return Err("not_an_xdto_package: metadataPath must identify an XDTO package".to_string());
    }
    let evidence = platform_xml_resource_evidence(context, &resolution.handle)
        .map_err(map_xdto_target_error)?;
    let path = prove_xdto_resource(&evidence, context)?;
    Ok(Package {
        path,
        descriptor_path: evidence.target_path,
        handle: resolution.handle,
        source_set: resolution.resolved.source_set,
        metadata_path: resolution
            .resolved
            .metadata_path
            .expect("an XDTO object resolution carries its address")
            .as_str()
            .to_string(),
    })
}

fn resolve_xdto_resource(
    handle: &ClosedPlatformXmlTarget,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    let evidence =
        platform_xml_resource_evidence(context, handle).map_err(map_xdto_target_error)?;
    prove_xdto_resource(&evidence, context)
}

fn prove_xdto_resource(
    evidence: &PlatformXmlResourceEvidence,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    let descriptor_stem = evidence
        .target_path
        .file_stem()
        .filter(|stem| !stem.is_empty())
        .ok_or_else(|| "target_not_found: XDTO descriptor has no file stem".to_string())?;
    let descriptor_parent = evidence
        .target_path
        .parent()
        .ok_or_else(|| "containment_denied: XDTO descriptor has no parent".to_string())?;
    let resource = WorkspacePathPolicy::new(context)
        .resolve_write(
            descriptor_parent
                .join(descriptor_stem)
                .join("Ext")
                .join("Package.bin"),
        )
        .map_err(|_| "containment_denied: cannot resolve XDTO package resource".to_string())?;
    ensure_no_link_components(&evidence.source_root, &resource)?;
    let metadata = match fs::symlink_metadata(&resource) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(
                "package_resource_missing: logical XDTO package has no Package.bin resource"
                    .to_string(),
            )
        }
        Err(_) => {
            return Err("containment_denied: cannot inspect XDTO package resource".to_string())
        }
    };
    if metadata_is_link_or_reparse_point(&metadata) {
        return Err("containment_denied: XDTO package resource must not be a link".to_string());
    }
    if !metadata.is_file() {
        return Err("package_resource_missing: Package.bin is not a regular file".to_string());
    }
    let source_root = normalize_path_identity(&evidence.source_root)
        .map_err(|_| "containment_denied: cannot resolve XDTO sourceSet".to_string())?;
    let resource_identity = normalize_path_identity(&resource)
        .map_err(|_| "containment_denied: cannot resolve XDTO package resource".to_string())?;
    if !resource_identity.starts_with(&source_root) {
        return Err("containment_denied: XDTO package resource escapes sourceSet".to_string());
    }
    Ok(resource)
}

fn ensure_no_link_components(source_root: &Path, path: &Path) -> Result<(), String> {
    let relative = path
        .strip_prefix(source_root)
        .map_err(|_| "containment_denied: XDTO package resource escapes sourceSet".to_string())?;
    let mut current = source_root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => break,
            Err(_) => {
                return Err("containment_denied: cannot inspect XDTO package path".to_string())
            }
        };
        if metadata_is_link_or_reparse_point(&metadata) {
            return Err("containment_denied: XDTO package path contains a link".to_string());
        }
    }
    Ok(())
}

fn map_xdto_target_error(error: SourceTargetError) -> String {
    let code = match error.code {
        SourceTargetErrorCode::SourceSetRequired | SourceTargetErrorCode::SourceSetNotFound => {
            "source_set_unknown"
        }
        SourceTargetErrorCode::SourceRootNotAddressable
            if error.message.contains("must be a Platform XML") =>
        {
            "not_an_xdto_package"
        }
        SourceTargetErrorCode::SourceRootNotAddressable => "source_set_unknown",
        SourceTargetErrorCode::TargetKindMismatch => "not_an_xdto_package",
        SourceTargetErrorCode::ContainmentDenied => "containment_denied",
        SourceTargetErrorCode::MetadataAddressInvalid
        | SourceTargetErrorCode::MetadataAddressNotFound
        | SourceTargetErrorCode::AddressProfileUnsupported => "target_not_found",
    };
    format!("{code}: {}", error.message)
}

fn guard_resolved_support(target: &Path, context: &WorkspaceContext) -> Result<(), String> {
    match evaluate_resolved_support_guard(target, SupportGuardRequirement::Editable, context) {
        ResolvedSupportGuardCheck::Allow | ResolvedSupportGuardCheck::Warn(_) => Ok(()),
        ResolvedSupportGuardCheck::Block(violation) => {
            Err(format!("support_locked: {}", violation.reason))
        }
    }
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
fn model_error(error: model::ModelError) -> String {
    format!(
        "{}: {} at byte {}",
        error.code, error.message, error.span.start
    )
}
fn descriptor_namespace(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path)
        .map_err(|_| "target_not_found: cannot read proven XDTO descriptor".to_string())?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "not_an_xdto_package: XDTO descriptor is not UTF-8".to_string())?;
    let document = Document::parse(text)
        .map_err(|_| "not_an_xdto_package: XDTO descriptor is not valid XML".to_string())?;
    let root = document.root_element();
    if !root.has_tag_name((MD_CLASSES_NS, "MetaDataObject")) {
        return Err(
            "not_an_xdto_package: descriptor root must be an MDClasses MetaDataObject".to_string(),
        );
    }
    let package = root
        .children()
        .find(|child| child.has_tag_name((MD_CLASSES_NS, "XDTOPackage")))
        .ok_or_else(|| "not_an_xdto_package: descriptor must contain an XDTOPackage".to_string())?;
    let properties = package
        .children()
        .find(|child| child.has_tag_name((MD_CLASSES_NS, "Properties")))
        .ok_or_else(|| "namespace_mismatch: XDTO descriptor has no Properties".to_string())?;
    properties
        .children()
        .find(|child| child.has_tag_name((MD_CLASSES_NS, "Namespace")))
        .and_then(|namespace| namespace.text())
        .filter(|namespace| !namespace.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "namespace_mismatch: XDTO descriptor has no Namespace".to_string())
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
    use super::{apply_with_data, decode, encode_like, mutation, preview_with_data};
    use crate::application::SupportGuardRequirement;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::native_operations::common::support_guard_violation;
    use crate::infrastructure::native_operations::single_file_publisher::with_before_commit_hook;
    use crate::infrastructure::platform::testing::{
        create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };
    use serde_json::{json, Map, Value};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    const PACKAGE: &str = r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:tns="urn:test" targetNamespace="urn:test">
	<objectType name="ЛюбаяСсылка"><property name="СсылкаНаОбъект"><typeDef xsi:type="ObjectType"></typeDef></property></objectType>
	<objectType name="СоставнойЛюбойОбъект"/>
</package>"#;

    static TEMP_NONCE: AtomicU64 = AtomicU64::new(0);

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
                json!({"name":"Документ_Новый", "type":"tns:Документ_Новый", "minOccurs":0}),
            ),
        ]);
        let once = mutation(PACKAGE, &args, "add-property").unwrap();
        assert_eq!(
            once,
            r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:tns="urn:test" targetNamespace="urn:test">
	<objectType name="ЛюбаяСсылка"><property name="СсылкаНаОбъект"><typeDef xsi:type="ObjectType">
		<property name="Документ_Новый" type="tns:Документ_Новый" lowerBound="0"/>
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

    fn xdto_guard_fixture(
        name: &str,
    ) -> (
        WorkspaceContext,
        Map<String, Value>,
        std::path::PathBuf,
        std::path::PathBuf,
    ) {
        let root = std::env::temp_dir().join(format!(
            "unica-xdto-guard-{name}-{}-{}",
            std::process::id(),
            TEMP_NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        let descriptor = root.join("src/XDTOPackages/Sample.xml");
        let package = root.join("src/XDTOPackages/Sample/Ext/Package.bin");
        fs::create_dir_all(package.parent().unwrap()).unwrap();
        fs::create_dir_all(root.join("src/Ext")).unwrap();
        fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::write(
            root.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"><Properties><Name>Main</Name></Properties></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            &descriptor,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><XDTOPackage uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"><Properties><Name>Sample</Name><Namespace>urn:test</Namespace></Properties></XDTOPackage></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(&package, PACKAGE).unwrap();
        let context = WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        };
        let args = args(&[
            ("sourceSet", json!("main")),
            ("metadataPath", json!("XDTOPackage.Sample")),
            ("operation", json!("add-object-type")),
            ("name", json!("Added")),
        ]);
        (context, args, package, descriptor)
    }

    fn finding_codes(execution: &super::XdtoExecution, state: &str) -> Vec<String> {
        execution
            .data
            .as_ref()
            .and_then(|data| data.get("findings"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|finding| finding.get("state").and_then(Value::as_str) == Some(state))
            .filter_map(|finding| finding.get("code").and_then(Value::as_str))
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn xdto_validation_reports_unrelated_baseline_findings_without_blocking() {
        let (context, mut args, package, _) = xdto_guard_fixture("validation-baseline");
        fs::write(
            &package,
            r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:tns="urn:test" xmlns:ext="urn:external" targetNamespace="urn:test">
	<property name="Global" type="xs:string"/>
	<valueType name="Duplicate" base="xs:string"><enumeration value="x"/></valueType>
	<import namespace="urn:other"/>
	<objectType name="Duplicate">
		<property name="bad:name" type="ext:Remote" lowerBound="2" upperBound="1"/>
		<property name="bad:name" type="xs:string"><typeDef xsi:type="ObjectType"/></property>
		<property name="Nested"><typeDef xsi:type="ValueType"/></property>
		<property name="Undeclared" type="ghost:Remote"/>
		<property name="MissingLocal" type="tns:Missing"/>
		<property name="Both" ref="tns:Global"/>
		<property ref="tns:MissingGlobal"/>
	</objectType>
</package>"#,
        )
        .unwrap();
        args.insert("operation".to_string(), json!("add-object-type"));
        args.insert("name".to_string(), json!("Safe"));

        let execution = preview_with_data(&args, &context);

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        let codes = finding_codes(&execution, "pre_existing");
        for expected in [
            "invalid_group_order",
            "unsupported_node",
            "duplicate_type",
            "duplicate_property",
            "invalid_ncname",
            "missing_import",
            "undeclared_prefix",
            "unknown_type_reference",
            "unknown_property_reference",
            "invalid_property_identity",
            "type_definition_conflict",
            "invalid_bounds",
        ] {
            assert!(
                codes.iter().any(|code| code == expected),
                "{expected}: {codes:?}"
            );
        }
        let rendered = serde_json::to_string(&execution.data).unwrap();
        assert!(!rendered.contains(package.to_string_lossy().as_ref()));
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_validation_accepts_the_untouched_enterprise_data_fixture() {
        let bytes = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/xdto/enterprise-data-minimal/XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin"
        ));
        let text = decode(bytes).unwrap();
        let model = super::model::PackageModel::parse(&text).unwrap();

        let findings = super::validation::validate(&model);

        assert!(findings.is_empty(), "{findings:#?}");
    }

    #[test]
    fn xdto_validation_diff_uses_code_and_logical_key_not_message_or_span() {
        use super::validation::{
            Finding, FindingLocation, FindingSeverity, FindingState, SourceSpanDto, ValidationDiff,
        };
        let baseline = vec![Finding {
            code: "unknown_type_reference".to_string(),
            severity: FindingSeverity::Error,
            message: "old wording".to_string(),
            location: FindingLocation {
                key: "$package/objectType:Consumer/property:Value/@type".to_string(),
                span: SourceSpanDto { start: 10, end: 18 },
            },
        }];
        let candidate = vec![Finding {
            code: "unknown_type_reference".to_string(),
            severity: FindingSeverity::Error,
            message: "new wording".to_string(),
            location: FindingLocation {
                key: "$package/objectType:Consumer/property:Value/@type".to_string(),
                span: SourceSpanDto {
                    start: 110,
                    end: 118,
                },
            },
        }];

        let diff = ValidationDiff::between(&baseline, candidate);

        assert_eq!(diff.findings.len(), 1);
        assert_eq!(diff.findings[0].state, FindingState::PreExisting);
        assert!(!diff.blocks());
    }

    #[test]
    fn xdto_validation_diff_blocks_an_additional_finding_with_the_same_semantic_key() {
        use super::validation::{
            Finding, FindingLocation, FindingSeverity, FindingState, SourceSpanDto, ValidationDiff,
        };
        let finding = |start| Finding {
            code: "duplicate_property".to_string(),
            severity: FindingSeverity::Error,
            message: "duplicate".to_string(),
            location: FindingLocation {
                key: "$package/objectType:Consumer/property:Value/@unique".to_string(),
                span: SourceSpanDto {
                    start,
                    end: start + 1,
                },
            },
        };

        let diff = ValidationDiff::between(&[finding(10)], vec![finding(20), finding(30)]);

        assert_eq!(diff.findings[0].state, FindingState::PreExisting);
        assert_eq!(diff.findings[1].state, FindingState::Introduced);
        assert!(diff.blocks());
    }

    #[test]
    fn xdto_validation_requires_exact_root_and_target_namespace() {
        let wrong_root = super::model::PackageModel::parse(
            r#"<package xmlns="urn:not-xdto" targetNamespace="urn:test"/>"#,
        )
        .unwrap_err();
        assert_eq!(wrong_root.code, "not_an_xdto_package");

        let missing_namespace =
            super::model::PackageModel::parse(r#"<package xmlns="http://v8.1c.ru/8.1/xdto"/>"#)
                .unwrap();
        let findings = super::validation::validate(&missing_namespace);
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].code, "target_namespace_required");
        assert_eq!(findings[0].location.key, "$package/@targetNamespace");
    }

    #[test]
    fn xdto_validation_uses_xml_ncname_character_ranges() {
        let model = super::model::PackageModel::parse(
            r#"<package xmlns="http://v8.1c.ru/8.1/xdto" targetNamespace="urn:test">
	<objectType name="_ok"/>
	<objectType name="Имя"/>
	<objectType name="A·B"/>
	<objectType name="Á"/>
	<objectType name="1bad"/>
	<objectType name="bad:name"/>
</package>"#,
        )
        .unwrap();

        let invalid_names = super::validation::validate(&model)
            .into_iter()
            .filter(|finding| finding.code == "invalid_ncname")
            .map(|finding| finding.location.key)
            .collect::<Vec<_>>();

        assert_eq!(invalid_names.len(), 2, "{invalid_names:#?}");
        assert!(invalid_names.iter().any(|key| key.contains("1bad")));
        assert!(invalid_names.iter().any(|key| key.contains("bad:name")));
    }

    #[test]
    fn xdto_validation_rejects_bare_self_qname_until_the_writer_emits_a_prefix() {
        let (context, mut args, _, _) = xdto_guard_fixture("validation-bare-qname");
        args.insert("operation".to_string(), json!("add-property"));
        args.insert("typeName".to_string(), json!("ЛюбаяСсылка"));
        args.insert(
            "property".to_string(),
            json!({"name":"BareSelf", "type":"Missing"}),
        );

        let execution = preview_with_data(&args, &context);

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        assert_eq!(
            finding_codes(&execution, "introduced"),
            vec!["invalid_qname"]
        );
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_validation_blocks_candidate_unknown_type_and_invalid_ncname() {
        let (context, mut args, package, _) = xdto_guard_fixture("validation-candidate");
        args.insert("operation".to_string(), json!("add-property"));
        args.insert("typeName".to_string(), json!("ЛюбаяСсылка"));
        args.insert(
            "property".to_string(),
            json!({"name":"bad:name", "type":"tns:Missing"}),
        );

        let execution = preview_with_data(&args, &context);

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        let codes = finding_codes(&execution, "introduced");
        assert!(
            codes.iter().any(|code| code == "invalid_ncname"),
            "{codes:?}"
        );
        assert!(
            codes.iter().any(|code| code == "unknown_type_reference"),
            "{codes:?}"
        );
        fs::remove_dir_all(context.workspace_root).unwrap();
        assert!(!package.exists());
    }

    #[test]
    fn xdto_validation_blocks_removing_a_referenced_local_type() {
        let (context, mut args, package, _) = xdto_guard_fixture("validation-remove-type");
        fs::write(
            &package,
            r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:tns="urn:test" targetNamespace="urn:test">
	<valueType name="Used" base="xs:string"/>
	<objectType name="Consumer"><property name="Value" type="tns:Used"/></objectType>
</package>"#,
        )
        .unwrap();
        args.insert("operation".to_string(), json!("remove-type"));
        args.insert("name".to_string(), json!("Used"));

        let execution = preview_with_data(&args, &context);

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        assert_eq!(
            finding_codes(&execution, "introduced"),
            vec!["unknown_type_reference"]
        );
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_validation_rejects_descriptor_namespace_mismatch_without_path_leak() {
        let (context, args, package, descriptor) = xdto_guard_fixture("validation-descriptor");
        fs::write(
            &descriptor,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><XDTOPackage uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"><Properties><Name>Sample</Name><Namespace>urn:other</Namespace></Properties></XDTOPackage></MetaDataObject>"#,
        )
        .unwrap();

        let execution = preview_with_data(&args, &context);

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        let rendered = format!("{:?}", execution.outcome.errors);
        assert!(rendered.contains("namespace_mismatch"), "{rendered}");
        assert!(!rendered.contains(package.to_string_lossy().as_ref()));
        assert!(!rendered.contains(descriptor.to_string_lossy().as_ref()));
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_guard_rejects_descriptor_identity_drift_before_commit() {
        let (context, args, package, descriptor) = xdto_guard_fixture("descriptor-drift");
        let before = fs::read(&package).unwrap();
        let descriptor_for_hook = descriptor.clone();

        let execution = with_before_commit_hook(
            move |_| fs::write(&descriptor_for_hook, "<concurrent/>").unwrap(),
            || apply_with_data(&args, &context),
        );

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        assert_eq!(fs::read(&package).unwrap(), before);
        assert_eq!(fs::read_to_string(&descriptor).unwrap(), "<concurrent/>");
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_guard_rejects_source_identity_drift_before_commit() {
        let (context, args, package, _) = xdto_guard_fixture("source-drift");
        let before = fs::read(&package).unwrap();
        let project = context.workspace_root.join("v8project.yaml");
        let project_for_hook = project.clone();
        let concurrent = "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: moved\n";

        let execution = with_before_commit_hook(
            move |_| fs::write(&project_for_hook, concurrent).unwrap(),
            || apply_with_data(&args, &context),
        );

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        assert_eq!(fs::read(&package).unwrap(), before);
        assert_eq!(fs::read_to_string(&project).unwrap(), concurrent);
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_guard_rejects_support_state_drift_before_commit() {
        let (context, args, package, _) = xdto_guard_fixture("support-drift");
        let before = fs::read(&package).unwrap();
        let support = context
            .workspace_root
            .join("src/Ext/ParentConfigurations.bin");
        let support_for_hook = support.clone();
        let concurrent_support = concat!(
            "\u{feff}{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
            "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
            "\"VendorConf\",3,1,0,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,",
            "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,0,0,",
            "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,",
            "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,2,0,",
            "cccccccc-cccc-cccc-cccc-cccccccccccc,",
            "cccccccc-cccc-cccc-cccc-cccccccccccc}"
        )
        .as_bytes()
        .to_vec();
        let concurrent_support_for_hook = concurrent_support.clone();
        assert!(
            support_guard_violation(&package, SupportGuardRequirement::Editable).is_none(),
            "the initial absent support state must allow editing"
        );

        let execution = with_before_commit_hook(
            move |_| fs::write(&support_for_hook, &concurrent_support_for_hook).unwrap(),
            || apply_with_data(&args, &context),
        );

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        let error = execution.outcome.errors.join("\n");
        assert!(error.contains("absence guard"), "{error}");
        assert!(error.contains("ParentConfigurations.bin"), "{error}");
        assert_eq!(fs::read(&package).unwrap(), before);
        assert_eq!(fs::read(&support).unwrap(), concurrent_support);
        let violation = support_guard_violation(&package, SupportGuardRequirement::Editable)
            .expect("concurrent support state must change the XDTO package verdict to locked");
        assert_eq!(violation.code, "locked");
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_guard_rejects_resource_preimage_drift_before_commit() {
        let (context, args, package, _) = xdto_guard_fixture("preimage-drift");
        let concurrent = b"concurrent package bytes".to_vec();
        let concurrent_for_hook = concurrent.clone();

        let execution = with_before_commit_hook(
            move |target| fs::write(target, &concurrent_for_hook).unwrap(),
            || apply_with_data(&args, &context),
        );

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        assert_eq!(fs::read(&package).unwrap(), concurrent);
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_guard_rejects_resource_outside_selected_source_set() {
        let (context, args, package, _) = xdto_guard_fixture("outside-source-set");
        let outside = context.workspace_root.join("outside/Package.bin");
        fs::create_dir_all(outside.parent().unwrap()).unwrap();
        fs::write(&outside, PACKAGE).unwrap();
        fs::remove_file(&package).unwrap();
        let outcome = create_file_link_fixture_for_test(&outside, &package)
            .expect("unexpected file-link creation error must fail the fixture test");
        if outcome != FileLinkFixtureOutcome::Created {
            fs::remove_dir_all(context.workspace_root).unwrap();
            return;
        }

        let execution = apply_with_data(&args, &context);

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        assert!(
            execution
                .outcome
                .errors
                .join("\n")
                .contains("containment_denied"),
            "{:?}",
            execution.outcome.errors
        );
        assert_eq!(fs::read(&outside).unwrap(), PACKAGE.as_bytes());
        fs::remove_dir_all(context.workspace_root).unwrap();
    }
}
