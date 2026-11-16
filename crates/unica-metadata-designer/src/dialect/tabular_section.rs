//! Existing scalar tabular-section properties in the admitted XML dialect.
use crate::edit::typed_diagnostic;
use std::ops::Range;
use unica_metadata::{MetaDiagnostic, MetaDiagnosticCode, MetaElementUpdate, TABULAR_SECTION_USES};

pub(crate) fn supports_root(xml: &str, root: roxmltree::Node<'_, '_>) -> bool {
    let format = unica_source_designer::roots::root_version_literal(xml, root);
    super::exact(format.as_deref())
        .and_then(|dialect| {
            unica_source_contract::SourceProfile::platform_xml_for_export_format(dialect.version)
        })
        .is_some_and(unica_source_contract::format_identity::supports_tabular_section_properties)
}

type ScalarEdits = (Range<usize>, Vec<(Range<usize>, String)>);

/// Stage byte ranges before mutating the descriptor, preserving namespace
/// declarations and all properties that were not requested.
pub(crate) fn edits(
    xml: &str,
    section_range: Range<usize>,
    update: &MetaElementUpdate,
) -> Result<ScalarEdits, MetaDiagnostic> {
    let refused = || {
        typed_diagnostic(
            MetaDiagnosticCode::InvalidArguments,
            "tabular section must have one metadata Properties element",
            None,
        )
    };
    let doc = roxmltree::Document::parse(xml).map_err(|_| refused())?;
    let section = doc
        .descendants()
        .find(|node| node.is_element() && node.range() == section_range)
        .ok_or_else(refused)?;
    let mut replacements = Vec::new();
    for (tag, field, desired, allowed) in [
        (
            "Use",
            "Use",
            update.tabular_section_use.as_deref(),
            TABULAR_SECTION_USES,
        ),
        (
            "FillChecking",
            "required",
            update
                .required
                .map(|v| if v { "ShowError" } else { "DontCheck" }),
            &["DontCheck", "ShowError"][..],
        ),
    ] {
        if let Some(desired) = desired {
            let (range, _) = value(xml, section, tag, field, allowed)?;
            if !allowed.contains(&desired) {
                return Err(typed_diagnostic(
                    MetaDiagnosticCode::InvalidArguments,
                    "unsupported tabular section property value",
                    Some(field),
                ));
            }
            replacements.push((range, desired.to_owned()));
        }
    }
    let properties = section
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "Properties")
        .ok_or_else(refused)?;
    Ok((properties.range(), replacements))
}

pub(crate) fn value(
    xml: &str,
    section: roxmltree::Node<'_, '_>,
    property: &str,
    field: &str,
    allowed: &[&str],
) -> Result<(std::ops::Range<usize>, String), MetaDiagnostic> {
    let refused = || {
        typed_diagnostic(MetaDiagnosticCode::InvalidArguments,
        format!("tabular section {property} must be a single existing property with a supported value"), Some(field))
    };
    if !supports_root(xml, section.document().root_element())
        || section.tag_name().namespace() != Some(crate::info_projection::MD_CLASSES_NAMESPACE)
    {
        return Err(refused());
    }
    let properties = section
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Properties")
        .collect::<Vec<_>>();
    let [properties] = properties.as_slice() else {
        return Err(refused());
    };
    if properties.tag_name().namespace() != Some(crate::info_projection::MD_CLASSES_NAMESPACE)
        || properties.attributes().len() != 0
    {
        return Err(refused());
    }
    let nodes = properties
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == property)
        .collect::<Vec<_>>();
    let [node] = nodes.as_slice() else {
        return Err(refused());
    };
    let children = node.children().collect::<Vec<_>>();
    let [text] = children.as_slice() else {
        return Err(refused());
    };
    if node.tag_name().namespace() != Some(crate::info_projection::MD_CLASSES_NAMESPACE)
        || node.attributes().len() != 0
        || !text.is_text()
        || !allowed.contains(&text.text().unwrap_or_default().trim())
    {
        return Err(refused());
    }
    let raw = &xml[text.range()];
    let start = text.range().start + raw.len() - raw.trim_start().len();
    let end = text.range().end - (raw.len() - raw.trim_end().len());
    Ok((
        start..end,
        text.text().unwrap_or_default().trim().to_owned(),
    ))
}
