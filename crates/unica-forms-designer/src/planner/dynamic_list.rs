use std::path::Path;

use serde_json::Value;
use unica_address::source_target::MetadataAddress;
use unica_source_contract::proposal::input::bad;
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyProposal, ApplySourceAuthority,
};
use unica_source_contract::SourceSetKind;
use unica_source_designer::owner::{
    metadata_artifact_property, parse_platform_xml_document, prove_already_read_metadata_owner,
    prove_already_read_source_set_owner, MD_CLASSES_NS,
};

/// Validate the just-rendered attribute definition against the same staged
/// source set. Guarded reads include existing and preceding in-batch edits.
pub(super) fn prove_bindings(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    definition: &Value,
    op_index: usize,
) -> Result<(), ApplyPlanError> {
    let mut items = definition
        .get("attributes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, item)| item.get("settings").map(|settings| (index, settings)))
        .peekable();
    let Some((first, _)) = items.peek() else {
        return Ok(());
    };
    let source_kind = authority.source_kind();
    if !matches!(
        source_kind,
        SourceSetKind::Configuration | SourceSetKind::Extension
    ) {
        return Err(bad(
            op_index,
            &format!("items[{first}].settings.mainTable"),
            "DynamicList mainTable must be registered in this configuration source set",
        ));
    }

    let read = |staged: &mut ApplyProposal<'_>, relative: &Path, field: &str| {
        staged
            .read_guarded_bounded(relative, usize::MAX)
            .map_err(|error| {
                ApplyPlanError::staging(error, format!("ops[{op_index}].args.{field}"))
            })?
            .ok_or_else(|| {
                bad(
                    op_index,
                    field,
                    "DynamicList main table descriptor was not found",
                )
            })
    };
    let root_path = Path::new("Configuration.xml");
    let first_field = format!("items[{first}].settings.mainTable");
    let image = read(staged, root_path, &first_field)?;
    let root =
        prove_already_read_source_set_owner(root_path, &image, source_kind).map_err(|error| {
            ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error.message)
                .at_path(format!("ops[{op_index}].args.{first_field}"))
        })?;

    for (index, settings) in items {
        let table = settings["mainTable"]
            .as_str()
            .expect("attribute writer proved mainTable");
        let (kind, name) = table
            .split_once('.')
            .expect("attribute writer proved a main table");
        let field_path = format!("items[{index}].settings.mainTable");
        if !root.registers(kind, name) {
            return Err(bad(op_index, &field_path, format!(
                "DynamicList main table `{table}` is not registered with this exact name in the source set"
            )));
        }
        let address = MetadataAddress::parse_logical(table)
            .map_err(|error| bad(op_index, &field_path, error.to_string()))?;
        let relative =
            unica_source_designer::layout::metadata_descriptor_relative(&address, source_kind)
                .map_err(|message| bad(op_index, &field_path, message))?;
        let bytes = read(staged, &relative, &field_path)?;
        let descriptor = prove_already_read_metadata_owner(&relative, &bytes).map_err(|error| {
            ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error.message)
                .at_path(format!("ops[{op_index}].args.{field_path}"))
        })?;
        if descriptor.artifact_kind() != kind || descriptor.artifact_name() != Some(name) {
            return Err(bad(
                op_index,
                &field_path,
                format!("the DynamicList main table descriptor does not declare `{table}`"),
            ));
        }

        if let Some(filter) = settings.get("filter").and_then(Value::as_array) {
            let (_, document) = parse_platform_xml_document(&relative, &bytes)
                .expect("descriptor proof parsed this image");
            let artifact = document
                .root_element()
                .first_element_child()
                .expect("descriptor proof confirmed one artifact");
            for (slot, comparison) in filter.iter().enumerate() {
                let field = comparison["field"]
                    .as_str()
                    .expect("attribute writer proved a field");
                let direct = ["Attribute", "Dimension", "Resource"]
                    .iter()
                    .any(|member| descriptor.registers(member, field));
                if !direct && !proven_standard_field(artifact, kind, field) {
                    return Err(bad(op_index, &format!("items[{index}].settings.filter[{slot}].field"),
                        format!("DynamicList filter field `{field}` is not a supported direct field of `{table}`")));
                }
            }
        }
    }
    Ok(())
}

fn proven_standard_field(artifact: roxmltree::Node<'_, '_>, kind: &str, field: &str) -> bool {
    const READABLE: &str = "http://v8.1c.ru/8.3/xcf/readable";

    let (standard, required_length) = match (kind, field) {
        ("Catalog" | "Document", "Ref" | "Ссылка") => ("Ref", None),
        ("Catalog" | "Document", "DeletionMark" | "ПометкаУдаления") => {
            ("DeletionMark", None)
        }
        ("Catalog", "Code" | "Код") => ("Code", Some("CodeLength")),
        ("Catalog", "Description" | "Наименование") => {
            ("Description", Some("DescriptionLength"))
        }
        _ => return false,
    };
    if required_length.is_some_and(|property| {
        !metadata_artifact_property(artifact, property)
            .and_then(|length| length.parse::<u32>().ok())
            .is_some_and(|length| length > 0)
    }) {
        return false;
    }
    artifact
        .children()
        .find(|node| node.has_tag_name((MD_CLASSES_NS, "Properties")))
        .and_then(|props| {
            props
                .children()
                .find(|node| node.has_tag_name((MD_CLASSES_NS, "StandardAttributes")))
        })
        .is_some_and(|list| {
            list.children().any(|node| {
                node.has_tag_name((READABLE, "StandardAttribute"))
                    && node.attribute("name") == Some(standard)
            })
        })
}
