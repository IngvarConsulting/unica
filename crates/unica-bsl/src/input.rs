//! Typed BSL operation arguments over already admitted source-set identity.
use crate::text::canonicalize_eol;
use serde_json::Value;
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodePosition {
    Before,
    After,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeSelector {
    Method(String),
    Anchor(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeInsertArgs {
    pub at: unica_address::address::QualifiedAddress,
    pub text: String,
    pub selector: Option<CodeSelector>,
    pub position: Option<CodePosition>,
}

pub(crate) const MODULE_TEXT_REVISION_HEX_LEN: usize = 64;
pub(crate) const MODULE_TEXT_REVISION_PATTERN: &str = r"^[0-9a-f]{64}$";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeReplaceArgs {
    pub at: unica_address::address::QualifiedAddress,
    pub text: String,
    /// Текстовый якорь внутри модуля. Метод целиком задаётся адресом
    /// `…Method.M`, а не селектором (#1353).
    pub anchor: Option<String>,
    pub if_rev: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeDeleteArgs {
    pub at: QualifiedAddress,
    pub if_rev: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodePlanOperation {
    Insert(CodeInsertArgs),
    Replace(CodeReplaceArgs),
    Delete(CodeDeleteArgs),
}

impl CodePlanOperation {
    pub fn if_rev(&self) -> Option<&str> {
        match self {
            Self::Insert(_) => None,
            Self::Replace(args) => args.if_rev.as_deref(),
            Self::Delete(args) => args.if_rev.as_deref(),
        }
    }
}

pub fn parse_code_plan_operation(
    operation: &str,
    value: &Value,
    op_index: usize,
    admitted_source_set: &str,
) -> Result<CodePlanOperation, ApplyPlanError> {
    let base = format!("ops[{op_index}].args");
    let object = value
        .as_object()
        .ok_or_else(|| bad_code_arg(&base, "code operation args must be an object"))?;
    let allowed = match operation {
        "code.insert" => &["at", "text", "selector", "position"][..],
        "code.replace" => &["at", "text", "selector", "ifRev"][..],
        "code.delete" => &["at", "ifRev"][..],
        _ => {
            return Err(bad_code_arg(
                &base,
                "code planner accepts only code.insert, code.replace or code.delete",
            ))
        }
    };
    if let Some(unknown) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(bad_code_arg(
            format!("{base}.{unknown}"),
            "unknown staged code argument",
        ));
    }
    let at_path = format!("{base}.at");
    let at_raw = non_empty_code_string(object.get("at"), &at_path, "at")?;
    let at = unica_address::address::QualifiedAddress::parse(at_raw)
        .map_err(|_| bad_code_arg(&at_path, "at must be a qualified logical address"))?;
    if at.line_range().is_some() {
        return Err(bad_code_arg(
            &at_path,
            "Body line ranges are read-only; address the whole Method or module Body",
        ));
    }
    if at.source_set() != admitted_source_set {
        return Err(bad_code_arg(
            &at_path,
            "at belongs to another actor-admitted source set",
        ));
    }
    let addressed = code_projection(&at);
    if operation == "code.delete" {
        if !matches!(&addressed, Some((module, CodeProjection::Method(_)))
            if crate::applicability::module_terminal(module.segments()))
        {
            return Err(bad_code_arg(
                &at_path,
                "code.delete requires a named Method address",
            ));
        }
        let if_rev = parse_if_rev(object.get("ifRev"), &base)?;
        return Ok(CodePlanOperation::Delete(CodeDeleteArgs { at, if_rev }));
    }
    let text_path = format!("{base}.text");
    let text = non_empty_code_string(object.get("text"), &text_path, "text")?.to_string();
    if addressed.is_some() && object.contains_key("selector") {
        return Err(bad_code_arg(
            format!("{base}.selector"),
            "an addressed BSL node cannot also have a selector",
        ));
    }

    match operation {
        "code.insert" => {
            // Метод задаётся адресом `…Method.M`, место — `position`
            // относительно него (#1354).
            let method = match addressed.as_ref().map(|(_, site)| site) {
                None | Some(CodeProjection::ModuleBody) => None,
                Some(CodeProjection::Method(name)) => Some(CodeSelector::Method(name.clone())),
                Some(CodeProjection::MethodBody(_)) => {
                    return Err(bad_code_arg(
                        &at_path,
                        "code.insert requires a module, its Body or a Method",
                    ))
                }
            };
            let addressed_method = method.is_some();
            let selector = object
                .get("selector")
                .map(|value| parse_code_anchor(value, &format!("{base}.selector")))
                .transpose()?
                .map(CodeSelector::Anchor)
                .or(method);
            let position_path = format!("{base}.position");
            let position = match (selector.is_some(), object.get("position")) {
                (true, Some(value)) => Some(parse_code_position(value, &position_path)?),
                (true, None) if addressed_method => {
                    return Err(bad_code_arg(
                        position_path,
                        "code.insert at a method needs `position`: before or after it",
                    ))
                }
                (true, None) => {
                    return Err(bad_code_arg(
                        position_path,
                        "position is required when selector is present",
                    ))
                }
                (false, Some(_)) => {
                    return Err(bad_code_arg(
                        position_path,
                        "position is unavailable without selector",
                    ))
                }
                (false, None) => None,
            };
            Ok(CodePlanOperation::Insert(CodeInsertArgs {
                at,
                text,
                selector,
                position,
            }))
        }
        "code.replace" => {
            let selector_path = format!("{base}.selector");
            let anchor = object
                .get("selector")
                .map(|value| parse_code_anchor(value, &selector_path))
                .transpose()?;
            if anchor.is_none() && addressed.is_none() {
                return Err(bad_code_arg(&selector_path, "selector.anchor is required for a module address; address its Body or a Method instead"));
            }
            let if_rev = parse_if_rev(object.get("ifRev"), &base)?;
            Ok(CodePlanOperation::Replace(CodeReplaceArgs {
                at,
                text,
                anchor,
                if_rev,
            }))
        }
        _ => unreachable!("closed operation checked above"),
    }
}

fn parse_if_rev(value: Option<&Value>, base: &str) -> Result<Option<String>, ApplyPlanError> {
    value
        .map(|value| {
            value
                .as_str()
                .filter(|value| {
                    value.len() == MODULE_TEXT_REVISION_HEX_LEN
                        && value
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                })
                .map(str::to_owned)
                .ok_or_else(|| {
                    bad_code_arg(
                        format!("{base}.ifRev"),
                        "ifRev must be a module text revision returned by a read",
                    )
                })
        })
        .transpose()
}

#[derive(Debug)]
pub enum CodeProjection {
    ModuleBody,
    Method(String),
    MethodBody(String),
}

pub fn code_projection(at: &QualifiedAddress) -> Option<(QualifiedAddress, CodeProjection)> {
    let segments = at.segments();
    let last = segments.last()?;
    let (length, site) = match last.kind() {
        NodeKind::Body if last.name().is_none() => {
            match segments.get(segments.len().checked_sub(2)?) {
                Some(method) if method.kind() == NodeKind::Method => (
                    segments.len().checked_sub(2)?,
                    CodeProjection::MethodBody(method.name()?.to_owned()),
                ),
                _ => (segments.len().checked_sub(1)?, CodeProjection::ModuleBody),
            }
        }
        NodeKind::Method => (
            segments.len().checked_sub(1)?,
            CodeProjection::Method(last.name()?.to_owned()),
        ),
        _ => return None,
    };
    let mut parts = Vec::new();
    for segment in &segments[..length] {
        parts.push(segment.kind().as_str());
        if let Some(name) = segment.name() {
            parts.push(name);
        }
    }
    let module =
        QualifiedAddress::parse(&format!("{}:{}", at.source_set(), parts.join("."))).ok()?;
    Some((module, site))
}

pub fn non_empty_code_string<'a>(
    value: Option<&'a Value>,
    path: &str,
    name: &str,
) -> Result<&'a str, ApplyPlanError> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| bad_code_arg(path, format!("{name} must be a non-empty string")))
}

/// Якорь `selector.anchor`: единственный ключ селектора, который публикует
/// контракт; метод адресуется `…Method.<Имя>`.
pub fn parse_code_anchor(value: &Value, path: &str) -> Result<String, ApplyPlanError> {
    let object = value
        .as_object()
        .ok_or_else(|| bad_code_arg(path, "selector must be an object"))?;
    if object.len() != 1 {
        return Err(bad_code_arg(
            path,
            "selector must contain exactly one anchor; address a method by `…Method.<Name>`",
        ));
    }
    let (name, value) = object.iter().next().expect("one selector field");
    let field_path = format!("{path}.{name}");
    if name != "anchor" {
        return Err(bad_code_arg(
            field_path,
            "unknown argument; selector accepts only anchor",
        ));
    }
    value
        .as_str()
        .filter(|value| !value.is_empty())
        .map(canonicalize_eol)
        .ok_or_else(|| bad_code_arg(&field_path, "selector value must be a non-empty string"))
}

pub fn parse_code_position(value: &Value, path: &str) -> Result<CodePosition, ApplyPlanError> {
    match value.as_str() {
        Some("before") => Ok(CodePosition::Before),
        Some("after") => Ok(CodePosition::After),
        _ => Err(bad_code_arg(path, "position must be before or after")),
    }
}

pub fn bad_code_arg(path: impl Into<String>, message: impl Into<String>) -> ApplyPlanError {
    ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(path)
}

pub fn code_operation_at(
    operation: &CodePlanOperation,
) -> &unica_address::address::QualifiedAddress {
    match operation {
        CodePlanOperation::Insert(args) => &args.at,
        CodePlanOperation::Replace(args) => &args.at,
        CodePlanOperation::Delete(args) => &args.at,
    }
}
