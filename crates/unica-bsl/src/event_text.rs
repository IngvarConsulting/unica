//! BSL handler emission from required typed method facts.
use crate::projection::MethodKind;
use unica_bsl_parser::parse as parse_bsl_syntax;
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind};
use unica_source_contract::ScriptVariant;
use unica_xml_format::text::{
    event_line_start, event_local_line_ending, reject_event_lone_cr, resolve_observed_line_ending,
    SourceTextSnapshot, Utf8Bom,
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequiredEventDirective {
    None,
    Client,
    Server,
    /// Reading accepts `&НаСервере` and `&НаСервереБезКонтекста`; a new
    /// handler is emitted without form context.
    ServerOrServerNoContext,
}

impl RequiredEventDirective {
    pub const fn canonical(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Client => Some("&НаКлиенте"),
            Self::Server => Some("&НаСервере"),
            Self::ServerOrServerNoContext => Some("&НаСервереБезКонтекста"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModuleInsertionPlan {
    ExistingRegion { offset: usize },
    AppendRegion { offset: usize },
}
impl ModuleInsertionPlan {
    const fn offset(self) -> usize {
        match self {
            Self::ExistingRegion { offset } | Self::AppendRegion { offset } => offset,
        }
    }
}
pub fn emit_event_module(
    current: Option<&[u8]>,
    handler: &str,
    signature: &str,
    method_kind: MethodKind,
    directive: RequiredEventDirective,
    region: &str,
) -> Result<Vec<u8>, ApplyPlanError> {
    emit_event_module_with_variant(
        current,
        handler,
        signature,
        method_kind,
        directive,
        region,
        ScriptVariant::Russian,
    )
}

/// The region is supplied by the event catalog; only its generated name and
/// markers follow the source-set script profile. Existing text stays intact.
pub fn emit_event_module_with_variant(
    current: Option<&[u8]>,
    handler: &str,
    signature: &str,
    method_kind: MethodKind,
    directive: RequiredEventDirective,
    region: &str,
    script_variant: ScriptVariant,
) -> Result<Vec<u8>, ApplyPlanError> {
    let (region, region_start, region_end) = match script_variant {
        ScriptVariant::Russian => (region, "#Область", "#КонецОбласти"),
        ScriptVariant::English => (
            match region {
                "ОбработчикиСобытий" => "EventHandlers",
                "ОбработчикиСобытийФормы" => "FormEventHandlers",
                "ОбработчикиСобытийЭлементовФормы" => {
                    "FormHeaderItemsEventHandlers"
                }
                "ОбработчикиКомандФормы" => "FormCommandsEventHandlers",
                other => other,
            },
            "#Region",
            "#EndRegion",
        ),
    };
    let owned_default;
    let bytes = match current {
        Some(bytes) => bytes,
        None => {
            owned_default = Vec::new();
            &owned_default
        }
    };
    let snapshot =
        SourceTextSnapshot::from_bytes(bytes).map_err(|error| invalid_source(error.to_string()))?;
    reject_event_lone_cr(&snapshot).map_err(invalid_source)?;
    let text = snapshot.text();
    let parsed = parse_bsl_syntax(text);
    if !parsed.errors().is_empty() {
        return Err(invalid_source(format!(
            "BSL parser reported {} diagnostic(s)",
            parsed.errors().len()
        )));
    }
    let handler_folded = handler.to_lowercase();
    let matching_methods = parsed
        .methods()
        .into_iter()
        .filter_map(|method| method.name)
        .filter(|name| name.to_lowercase() == handler_folded)
        .count();
    if matching_methods != 0 {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            format!(
                "event handler `{}` collides with an existing method",
                handler
            ),
        ));
    }

    let markers = parsed.region_markers();
    let canonical = region.to_lowercase();
    let mut stack = Vec::<(Option<String>, bool)>::new();
    let mut canonical_closes = Vec::new();
    for marker in markers {
        if marker.is_start {
            let name = marker.name;
            let is_canonical = name
                .as_deref()
                .is_some_and(|name| name.to_lowercase() == canonical);
            if is_canonical && !stack.is_empty() {
                return Err(invalid_source("canonical handler region is nested"));
            }
            stack.push((name, is_canonical));
        } else if marker.is_end {
            let Some((_, is_canonical)) = stack.pop() else {
                return Err(invalid_source("BSL region close marker has no opener"));
            };
            if is_canonical {
                canonical_closes.push(marker.range.start);
            }
        }
    }
    if !stack.is_empty() {
        return Err(invalid_source("BSL region is unclosed"));
    }
    if canonical_closes.len() > 1 {
        return Err(invalid_source("canonical handler region is duplicated"));
    }

    let insertion_plan = canonical_closes.first().copied().map_or(
        ModuleInsertionPlan::AppendRegion { offset: text.len() },
        |marker_offset| ModuleInsertionPlan::ExistingRegion {
            offset: event_line_start(text, marker_offset),
        },
    );
    let eol = event_local_line_ending(text, insertion_plan.offset()).map_err(invalid_source)?;
    let eol = resolve_observed_line_ending(&snapshot, eol)
        .map_err(|error| invalid_source(error.to_string()))?
        .as_str();
    let method = render_method(signature, method_kind, directive, eol);
    let mut body = text.to_string();
    match insertion_plan {
        ModuleInsertionPlan::ExistingRegion { offset } => {
            body.insert_str(offset, &format!("{method}{eol}"));
        }
        ModuleInsertionPlan::AppendRegion { .. } => {
            if !body.is_empty() {
                if !(body.ends_with('\n') || body.ends_with('\r')) {
                    body.push_str(eol);
                }
                body.push_str(eol);
            }
            body.push_str(&format!(
                "{region_start} {region}{eol}{eol}{method}{eol}{region_end}{eol}"
            ));
        }
    }
    let reparsed = parse_bsl_syntax(&body);
    if !reparsed.errors().is_empty() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::Postcondition,
            "emitted BSL failed parser postcondition",
        ));
    }
    let mut output = Vec::with_capacity(body.len() + 3);
    if current.is_none() || snapshot.bom() == Utf8Bom::Present {
        output.extend_from_slice(b"\xef\xbb\xbf");
    }
    output.extend_from_slice(body.as_bytes());
    Ok(output)
}
fn render_method(
    signature: &str,
    method_kind: MethodKind,
    directive: RequiredEventDirective,
    eol: &str,
) -> String {
    let directive = directive
        .canonical()
        .map(|directive| format!("{directive}{eol}"))
        .unwrap_or_default();
    let end = match method_kind {
        MethodKind::Procedure => "КонецПроцедуры",
        MethodKind::Function => "КонецФункции",
    };
    format!("{directive}{}{eol}{eol}{end}{eol}", signature)
}

fn invalid_source(message: impl Into<String>) -> ApplyPlanError {
    ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message)
}

#[cfg(test)]
mod script_variant_tests {
    use super::*;

    #[test]
    fn english_form_handler_uses_the_existing_english_region() {
        let text = b"#Region FormEventHandlers\r\n\r\n#EndRegion\r\n";
        let output = emit_event_module_with_variant(
            Some(text),
            "Opened",
            "Procedure Opened()",
            MethodKind::Procedure,
            RequiredEventDirective::Client,
            "ОбработчикиСобытийФормы",
            ScriptVariant::English,
        )
        .unwrap();
        let output = std::str::from_utf8(&output).unwrap();
        assert_eq!(output.matches("#Region FormEventHandlers").count(), 1);
        assert!(output.find("Opened()").unwrap() < output.find("#EndRegion").unwrap());
        assert!(!output.contains("#Область"));
        assert!(!output.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn english_form_handler_creates_an_english_region_without_rewriting_existing_text() {
        let text = "// comment\n";
        let output = emit_event_module_with_variant(
            Some(text.as_bytes()),
            "Clicked",
            "Procedure Clicked()",
            MethodKind::Procedure,
            RequiredEventDirective::Client,
            "ОбработчикиКомандФормы",
            ScriptVariant::English,
        )
        .unwrap();
        let output = std::str::from_utf8(&output).unwrap();
        assert!(output.starts_with(text));
        assert!(output.contains("#Region FormCommandsEventHandlers\n"));
        assert!(output.contains("#EndRegion\n"));
    }
}
