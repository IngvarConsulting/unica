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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeReplaceArgs {
    pub at: unica_address::address::QualifiedAddress,
    pub text: String,
    /// Текстовый якорь внутри модуля. Метод целиком задаётся адресом
    /// `…Method.M`, а не селектором (#1353).
    pub anchor: Option<String>,
    /// Ревизия текста модуля, от которой начиналась правка (#2225).
    pub if_rev: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodePlanOperation {
    Insert(CodeInsertArgs),
    Replace(CodeReplaceArgs),
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
        _ => {
            return Err(bad_code_arg(
                &base,
                "code planner accepts only the closed code.insert/code.replace catalog",
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
    let text_path = format!("{base}.text");
    let text = non_empty_code_string(object.get("text"), &text_path, "text")?.to_string();
    let addressed = code_projection(&at);
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
            let selector = match object
                .get("selector")
                .map(|value| parse_code_selector(value, &format!("{base}.selector")))
                .transpose()?
            {
                Some(CodeSelector::Method(name)) => {
                    return Err(bad_code_arg(
                        format!("{base}.selector.method"),
                        format!(
                            "code.insert addresses a method: `{at}.Method.{name}` with `position`"
                        ),
                    ))
                }
                selector => selector.or(method),
            };
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
            let anchor = match object
                .get("selector")
                .map(|value| parse_code_selector(value, &selector_path))
                .transpose()?
            {
                Some(CodeSelector::Method(name)) => {
                    return Err(bad_code_arg(
                        format!("{selector_path}.method"),
                        format!("code.replace addresses a whole method: `{at}.Method.{name}`"),
                    ))
                }
                Some(CodeSelector::Anchor(anchor)) => Some(anchor),
                None => None,
            };
            if anchor.is_none() && addressed.is_none() {
                return Err(bad_code_arg(&selector_path, "selector.anchor is required for a module address; address its Body or a Method instead"));
            }
            let if_rev_path = format!("{base}.ifRev");
            let if_rev = object
                .get("ifRev")
                .map(|value| match value.as_str() {
                    Some(revision) if !revision.trim().is_empty() => Ok(revision.to_owned()),
                    _ => Err(bad_code_arg(
                        &if_rev_path,
                        "ifRev must be a module text revision",
                    )),
                })
                .transpose()?;
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

pub fn parse_code_selector(value: &Value, path: &str) -> Result<CodeSelector, ApplyPlanError> {
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
    let value = value
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| bad_code_arg(&field_path, "selector value must be a non-empty string"))?;
    match name.as_str() {
        "method" => Ok(CodeSelector::Method(value.to_string())),
        "anchor" => Ok(CodeSelector::Anchor(canonicalize_eol(value))),
        _ => Err(bad_code_arg(
            field_path,
            "selector accepts only anchor; address a method by `…Method.<Name>`",
        )),
    }
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
    }
}
