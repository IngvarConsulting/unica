//! Strict field settings: edit existing property spans, never infer a default.
use roxmltree::{Document, Node};
use std::ops::Range;
use unica_metadata::{
    MemberChoiceChanges, MemberChoiceProperty, MetaDiagnostic, MetaDiagnosticCode,
};
const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
mod form_profile;

fn bad(property: MemberChoiceProperty, message: impl Into<String>) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::InvalidArguments, message)
        .with_field(property.argument())
}

pub(crate) fn child<'a, 'i>(parent: Node<'a, 'i>, name: &str) -> Result<Node<'a, 'i>, String> {
    let nodes = parent
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == name)
        .collect::<Vec<_>>();
    match nodes.as_slice() {
        [node] if node.has_tag_name((MD, name)) => Ok(*node),
        _ => Err(format!("expected one metadata {name}")),
    }
}

fn observed<'a, 'i>(
    properties: Node<'a, 'i>,
    property: MemberChoiceProperty,
) -> Result<Node<'a, 'i>, String> {
    let node = child(properties, property.xml_name())?;
    if properties.attributes().len() != 0
        || node.attributes().len() != 0
        || node.children().any(|child| !child.is_text())
        || !property.accepts(node.text().unwrap_or_default())
    {
        return Err(format!("{} is not a supported scalar", property.xml_name()));
    }
    Ok(node)
}

/// Resolve exact registered form identities through the proposal's retained reads.
fn validate_form(
    staged: &mut unica_source_contract::proposal::ApplyProposal<'_>,
    value: &str,
    source_kind: unica_source_contract::SourceSetKind,
    reference_table: &str,
) -> Result<(), String> {
    use std::path::Path;
    use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
    use unica_source_designer::layout::metadata_descriptor_relative;
    if value.is_empty() {
        return Ok(());
    }
    if source_kind != unica_source_contract::SourceSetKind::Configuration {
        return Err("standard choice form profile supports Configuration source sets only".into());
    }
    let parts = value.split('.').collect::<Vec<_>>();
    let (kind, owner_name, form_name) = match parts.as_slice() {
        ["CommonForm", name] => ("CommonForm", *name, *name),
        [kind, name, "Form", form] => (*kind, *name, *form),
        _ => return Err("expected CommonForm.Name or Kind.Name.Form.Name".into()),
    };
    let address = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, value)
        .map_err(|e| e.to_string())?;
    let mut read = |path: &Path| {
        staged
            .read(path)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("missing {}", path.display()))
    };
    let config = read(Path::new("Configuration.xml"))?;
    let registrations = crate::validation_context::inspect_metadata_registration_image(&config)?;
    let (table_kind, table_name) = reference_table
        .split_once('.')
        .ok_or("expected a reference metadata table")?;
    if !registrations
        .registrations
        .iter()
        .any(|(k, n)| k == table_kind && unica_metadata::metadata_name_eq(n, table_name))
    {
        return Err("field reference table is not registered in Configuration".into());
    }
    let table_address = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, reference_table)
        .map_err(|e| e.to_string())?;
    let table_bytes = read(&metadata_descriptor_relative(&table_address, source_kind)?)?;
    let table_identity = crate::validation_context::inspect_metadata_image_identity(&table_bytes)
        .map_err(|e| e.to_string())?;
    if table_identity.object_type != table_kind
        || !unica_metadata::metadata_name_eq(&table_identity.object_name, table_name)
    {
        return Err("field reference table identity mismatch".into());
    }
    if !registrations
        .registrations
        .iter()
        .any(|(k, n)| k == kind && n == owner_name)
    {
        return Err("form owner is not registered in Configuration".into());
    }
    if kind != "CommonForm" {
        let owner = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!("{kind}.{owner_name}"),
        )
        .map_err(|e| e.to_string())?;
        let owner_bytes = read(&metadata_descriptor_relative(&owner, source_kind)?)?;
        let identity = crate::validation_context::inspect_metadata_image_identity(&owner_bytes)
            .map_err(|e| e.to_string())?;
        if identity.object_type != kind || identity.object_name != owner_name {
            return Err("form owner identity mismatch".into());
        }
        let (_, doc) = unica_xml_format::read::parse_metadata_image(&owner_bytes)?;
        if !child(child(doc.root_element(), kind)?, "ChildObjects")?
            .children()
            .any(|node| {
                node.has_tag_name((MD, "Form"))
                    && (node.text() == Some(form_name)
                        || child(node, "Properties")
                            .and_then(|properties| child(properties, "Name"))
                            .is_ok_and(|name| {
                                name.attributes().len() == 0
                                    && name.children().all(|child| child.is_text())
                                    && name.text() == Some(form_name)
                            }))
            })
        {
            return Err("form is not registered by its owner".into());
        }
    }
    let path = metadata_descriptor_relative(&address, source_kind)?;
    let form_bytes = read(&path)?;
    let identity = crate::validation_context::inspect_form_metadata_image_identity(&form_bytes)
        .map_err(|e| e.to_string())?;
    if identity.object_type
        != if kind == "CommonForm" {
            "CommonForm"
        } else {
            "Form"
        }
        || identity.object_name != form_name
    {
        return Err("form identity mismatch".into());
    }
    let (_, descriptor) = unica_xml_format::read::parse_metadata_image(&form_bytes)?;
    let properties = child(
        child(descriptor.root_element(), &identity.object_type)?,
        "Properties",
    )?;
    if child(properties, "FormType")?.text() != Some("Managed") {
        return Err("expected a managed form descriptor".into());
    }
    let payload = path.with_extension("").join("Ext/Form.xml");
    let payload_bytes = read(&payload)?;
    let payload_text = std::str::from_utf8(&payload_bytes)
        .map_err(|e| e.to_string())?
        .trim_start_matches('\u{feff}');
    let doc = Document::parse(payload_text).map_err(|e| e.to_string())?;
    if !doc
        .root_element()
        .has_tag_name(("http://v8.1c.ru/8.3/xcf/logform", "Form"))
    {
        return Err("expected managed form payload".into());
    }
    let module_path = payload.with_extension("").join("Module.bsl");
    let module = staged.read(&module_path).map_err(|e| e.to_string())?;
    form_profile::validate(payload_text, reference_table, module.as_deref())
}

pub(crate) fn validate_references(
    staged: &mut unica_source_contract::proposal::ApplyProposal<'_>,
    bytes: &[u8],
    source_kind: unica_source_contract::SourceSetKind,
    explicit: Option<&unica_metadata::MetaEditOperation>,
    previous: Option<&[u8]>,
) -> Result<(), unica_source_contract::proposal::ApplyPlanError> {
    use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind};
    let fail = |message: String| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!("choiceForm: {message}"),
        )
    };
    let (_, doc) = unica_xml_format::read::parse_metadata_image(bytes).map_err(fail)?;
    let previous_doc =
        previous.and_then(|bytes| unica_xml_format::read::parse_metadata_image(bytes).ok());
    let selected = |member: Node<'_, '_>, original: bool| {
        let Some(unica_metadata::MetaEditOperation::Update {
            collection,
            scope,
            elements,
        }) = explicit
        else {
            return false;
        };
        let tag = match collection {
            unica_metadata::MetaCollection::Attributes => "Attribute",
            unica_metadata::MetaCollection::Dimensions => "Dimension",
            unica_metadata::MetaCollection::Resources => "Resource",
            _ => return false,
        };
        if !member.has_tag_name((MD, tag)) {
            return false;
        }
        let section = member
            .parent_element()
            .and_then(|parent| parent.parent_element())
            .filter(|parent| parent.has_tag_name((MD, "TabularSection")))
            .and_then(|parent| child(parent, "Properties").ok())
            .and_then(|p| child(p, "Name").ok())
            .and_then(|n| n.text());
        if !match (section, scope.as_ref()) {
            (None, None) => true,
            (Some(actual), Some(scope)) => {
                unica_metadata::metadata_name_eq(actual, &scope.tabular_section)
            }
            _ => false,
        } {
            return false;
        }
        let name = child(member, "Properties")
            .and_then(|p| child(p, "Name"))
            .ok()
            .and_then(|n| n.text());
        elements.iter().any(|element| {
            element
                .choice
                .0
                .iter()
                .any(|(property, _)| *property == MemberChoiceProperty::Form)
                && name.is_some_and(|name| {
                    unica_metadata::metadata_name_eq(
                        if original {
                            &element.name
                        } else {
                            element.new_name.as_deref().unwrap_or(&element.name)
                        },
                        name,
                    )
                })
        })
    };
    let forced_ids = previous_doc
        .as_ref()
        .into_iter()
        .flat_map(|(_, doc)| doc.descendants())
        .filter(|member| selected(*member, true))
        .filter_map(|member| member.attribute("uuid"))
        .collect::<Vec<_>>();
    let previous_forms = previous
        .and_then(|bytes| unica_xml_format::read::parse_metadata_image(bytes).ok())
        .map(|(_, doc)| {
            doc.descendants()
                .filter(|n| n.has_tag_name((MD, "ChoiceForm")))
                .map(|n| {
                    (
                        n.parent()
                            .and_then(|p| p.parent())
                            .and_then(|m| m.attribute("uuid"))
                            .map(str::to_owned),
                        n.text().unwrap_or_default().to_owned(),
                        n.parent()
                            .and_then(|p| child(p, "Type").ok())
                            .map(|ty| doc.input_text()[ty.range()].to_owned()),
                    )
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for member in doc.descendants().filter(|n| {
        n.tag_name().namespace() == Some(MD)
            && matches!(n.tag_name().name(), "Attribute" | "Dimension" | "Resource")
    }) {
        let properties = child(member, "Properties").map_err(fail)?;
        let Some(form) = properties
            .children()
            .find(|n| n.has_tag_name((MD, "ChoiceForm")))
        else {
            continue;
        };
        let value = form.text().unwrap_or_default();
        let force = selected(member, false)
            || member
                .attribute("uuid")
                .is_some_and(|id| forced_ids.contains(&id));
        if explicit.is_some() && !force {
            continue;
        }
        if value.is_empty()
            || (!force
                && previous_forms.iter().any(|(id, old, old_type)| {
                    id.as_deref() == member.attribute("uuid")
                        && old == value
                        && child(properties, "Type").ok().is_some_and(|ty| {
                            old_type.as_deref() == Some(&doc.input_text()[ty.range()])
                        })
                }))
        {
            continue;
        }
        observed(properties, MemberChoiceProperty::Form).map_err(fail)?;
        let reference_table = form_profile::reference_table(properties).map_err(fail)?;
        validate_form(staged, value, source_kind, &reference_table).map_err(fail)?;
    }
    Ok(())
}

#[derive(Default)]
pub(crate) struct ChoiceEdits {
    pub properties_range: Option<Range<usize>>,
    pub replacements: Vec<(Range<usize>, String)>,
}

pub(crate) fn edits(
    xml: &str,
    selected: Range<usize>,
    changes: &MemberChoiceChanges,
) -> Result<ChoiceEdits, MetaDiagnostic> {
    let mut edits = ChoiceEdits::default();
    for (property, value) in &changes.0 {
        if !property.accepts(value) {
            return Err(bad(*property, "invalid choice setting"));
        }
        let doc = Document::parse(xml).map_err(|error| bad(*property, error.to_string()))?;
        let member = doc
            .descendants()
            .find(|node| node.is_element() && node.range() == selected)
            .ok_or_else(|| bad(*property, "field is unavailable"))?;
        let properties = child(member, "Properties").map_err(|message| bad(*property, message))?;
        edits.properties_range.get_or_insert(properties.range());
        let node = observed(properties, *property).map_err(|message| bad(*property, message))?;
        if node.text().unwrap_or_default() != value {
            let raw = &xml[node.range()];
            let (opening, tag) = crate::edit::scalar_element_shell(raw);
            let replacement = if value.is_empty() {
                format!("{opening}/>")
            } else {
                format!(
                    "{opening}>{}</{tag}>",
                    unica_xml_format::text::escape_xml(value)
                )
            };
            edits.replacements.push((node.range(), replacement));
        }
    }
    Ok(edits)
}

/// Optional malformed settings remain local diagnostics instead of removing a field.
pub(crate) fn observe(
    properties: Node<'_, '_>,
) -> (Vec<unica_metadata::MetaObservedProperty>, Vec<String>) {
    let mut values = Vec::new();
    let mut limits = Vec::new();
    for property in MemberChoiceProperty::ALL {
        if !properties
            .children()
            .any(|n| n.is_element() && n.tag_name().name() == property.xml_name())
        {
            continue;
        }
        match observed(properties, *property) {
            Ok(node) => values.push(unica_metadata::MetaObservedProperty {
                name: property.xml_name().into(),
                value: unica_metadata::MetaObservedPropertyValue::Text {
                    value: node.text().unwrap_or_default().into(),
                },
            }),
            Err(message) => limits.push(message),
        }
    }
    (values, limits)
}
