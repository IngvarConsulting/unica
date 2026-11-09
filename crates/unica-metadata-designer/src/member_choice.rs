// module: private
//! Strict field settings: edit existing property spans, never infer a default.
use roxmltree::{Document, Node};
use std::ops::Range;
use unica_metadata::{
    MemberChoiceChanges, MemberChoiceProperty, MetaDiagnostic, MetaDiagnosticCode,
};
use unica_source_contract::{
    proposal::{ApplyPlanError, ApplyPlanErrorKind, ApplyProposal},
    SourceSetKind,
};
const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
mod form_profile;

pub type StandardChoiceFormCheck = fn(&[u8], &str, Option<&[u8]>) -> Result<(), String>;

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
    let name = (crate::dialect::writable().member_choice_name)(property);
    let node = child(properties, name)?;
    if properties.attributes().len() != 0
        || node.attributes().len() != 0
        || node.children().any(|child| !child.is_text())
        || !property.accepts(node.text().unwrap_or_default())
    {
        return Err(format!("{name} is not a supported scalar"));
    }
    Ok(node)
}

/// Resolve exact registered form identities through the proposal's retained reads.
fn validate_form(
    staged: &mut ApplyProposal<'_>,
    value: &str,
    source_kind: SourceSetKind,
    reference_table: &str,
    check_form: StandardChoiceFormCheck,
) -> Result<(), String> {
    use std::path::Path;
    use unica_address::source_target::MetadataAddress;
    use unica_source_designer::layout::metadata_descriptor_relative;
    if value.is_empty() {
        return Ok(());
    }
    if source_kind != SourceSetKind::Configuration {
        return Err("standard choice form profile supports Configuration source sets only".into());
    }
    let parts = value.split('.').collect::<Vec<_>>();
    let (kind, owner_name, form_name) = match parts.as_slice() {
        ["CommonForm", name] => ("CommonForm", *name, *name),
        [kind, name, "Form", form] => (*kind, *name, *form),
        _ => return Err("expected CommonForm.Name or Kind.Name.Form.Name".into()),
    };
    let address = MetadataAddress::parse_logical(value).map_err(|e| e.to_string())?;
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
    let table_address =
        MetadataAddress::parse_logical(reference_table).map_err(|e| e.to_string())?;
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
        let owner = MetadataAddress::parse_logical(&format!("{kind}.{owner_name}"))
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
    let module_path = payload.with_extension("").join("Module.bsl");
    let module = staged.read(&module_path).map_err(|e| e.to_string())?;
    check_form(&payload_bytes, reference_table, module.as_deref())
}

/// Retain exact member identity across later edits and inspect the final proposal.
#[derive(Default)]
pub struct MemberChoicePlan {
    checks: std::collections::BTreeMap<(std::path::PathBuf, uuid::Uuid), ChoiceCheck>,
}

struct ChoiceCheck {
    original_type: Option<String>,
    explicit: bool,
    op_index: usize,
}

fn form_error(message: impl std::fmt::Display, op_index: usize) -> ApplyPlanError {
    ApplyPlanError::new(
        ApplyPlanErrorKind::BadValue,
        format!("choiceForm: {message}"),
    )
    .at_path(format!("ops[{op_index}].args.values.choiceForm"))
}

impl MemberChoicePlan {
    pub(crate) fn retain(
        &mut self,
        source: &str,
        relative: &std::path::Path,
        owner: unica_metadata::MetadataKind,
        edit: &unica_metadata::MetaEditOperation,
        op_index: usize,
    ) -> Result<(), ApplyPlanError> {
        let unica_metadata::MetaEditOperation::Update {
            collection,
            scope,
            elements,
        } = edit
        else {
            return Ok(());
        };
        if !unica_metadata::member_choice_site(owner, *collection, scope.is_some()) {
            return Ok(());
        }
        let tag = match collection {
            unica_metadata::MetaCollection::Attributes => "Attribute",
            unica_metadata::MetaCollection::Dimensions => "Dimension",
            unica_metadata::MetaCollection::Resources => "Resource",
            _ => return Ok(()),
        };
        for element in elements {
            let explicit = element
                .choice
                .0
                .iter()
                .any(|(property, _)| *property == MemberChoiceProperty::Form);
            if !explicit && element.r#type.is_none() {
                continue;
            }
            let (range, _) = crate::edit::find_typed_element_identity(
                source,
                tag,
                scope.as_ref().map(|scope| scope.tabular_section.as_str()),
                &element.name,
            )
            .map_err(|error| form_error(error.message, op_index))?;
            let doc = Document::parse(source).map_err(|error| form_error(error, op_index))?;
            let member = doc
                .descendants()
                .find(|member| member.has_tag_name((MD, tag)) && member.range() == range)
                .ok_or_else(|| form_error("field is unavailable", op_index))?;
            let properties =
                child(member, "Properties").map_err(|error| form_error(error, op_index))?;
            let form_name =
                (crate::dialect::writable().member_choice_name)(MemberChoiceProperty::Form);
            if !explicit
                && !properties.children().any(|node| {
                    node.is_element()
                        && node.tag_name().name() == form_name
                        && node.text().is_some_and(|text| !text.is_empty())
                })
            {
                continue;
            }
            let id = member_uuid(member)
                .ok_or_else(|| form_error("field UUID is unavailable", op_index))?;
            let check = self
                .checks
                .entry((relative.to_owned(), id))
                .or_insert_with(|| ChoiceCheck {
                    original_type: type_image(properties).map(str::to_owned),
                    explicit: false,
                    op_index,
                });
            if explicit {
                check.explicit = true;
                check.op_index = op_index;
            }
        }
        Ok(())
    }

    pub(crate) fn validate_final(
        &self,
        staged: &mut ApplyProposal<'_>,
        source_kind: SourceSetKind,
        check_form: StandardChoiceFormCheck,
    ) -> Result<(), ApplyPlanError> {
        for ((relative, id), check) in &self.checks {
            let fail = |message: String| form_error(message, check.op_index);
            let bytes = staged
                .read(relative)
                .map_err(|error| {
                    ApplyPlanError::staging(
                        error,
                        format!("ops[{}].args.values.choiceForm", check.op_index),
                    )
                })?
                .ok_or_else(|| fail("field owner is unavailable".into()))?;
            let (_, doc) = unica_xml_format::read::parse_metadata_image(&bytes).map_err(fail)?;
            let mut members = doc.descendants().filter(|member| {
                member.tag_name().namespace() == Some(MD)
                    && matches!(
                        member.tag_name().name(),
                        "Attribute" | "Dimension" | "Resource"
                    )
                    && member_uuid(*member) == Some(*id)
            });
            let Some(member) = members.next() else {
                continue;
            };
            if members.next().is_some() {
                return Err(fail("field UUID is duplicated".into()));
            }
            let properties = child(member, "Properties").map_err(fail)?;
            if !check.explicit && type_image(properties) == check.original_type.as_deref() {
                continue;
            }
            let form = observed(properties, MemberChoiceProperty::Form).map_err(fail)?;
            let value = form.text().unwrap_or_default();
            if !value.is_empty() {
                let reference_table = form_profile::reference_table(properties).map_err(fail)?;
                validate_form(staged, value, source_kind, &reference_table, check_form)
                    .map_err(fail)?;
            }
        }
        Ok(())
    }
}

fn member_uuid(member: Node<'_, '_>) -> Option<uuid::Uuid> {
    uuid::Uuid::parse_str(member.attribute("uuid")?).ok()
}

fn type_image<'input>(properties: Node<'_, 'input>) -> Option<&'input str> {
    let ty = child(properties, "Type").ok()?;
    Some(&ty.document().input_text()[ty.range()])
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
    let Some((first, _)) = changes.0.first() else {
        return Ok(edits);
    };
    let doc = Document::parse(xml).map_err(|error| bad(*first, error.to_string()))?;
    let member = doc
        .descendants()
        .find(|node| node.is_element() && node.range() == selected)
        .ok_or_else(|| bad(*first, "field is unavailable"))?;
    let properties = child(member, "Properties").map_err(|message| bad(*first, message))?;
    for (property, value) in &changes.0 {
        if !property.accepts(value) {
            return Err(bad(*property, "invalid choice setting"));
        }
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
