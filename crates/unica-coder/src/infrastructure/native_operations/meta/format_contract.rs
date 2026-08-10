use roxmltree::{Document, Node};

use crate::domain::metadata::MetadataKind;

use super::validation::{meta_validate_property_values, meta_validate_valid_types};
use super::xml_model::{meta_info_child, meta_info_child_text};

fn format_contract_object_node<'a>(document: &'a Document<'a>) -> Result<Node<'a, 'a>, String> {
    document
        .root_element()
        .children()
        .find(|node| {
            node.is_element() && meta_validate_valid_types().contains(&node.tag_name().name())
        })
        .ok_or_else(|| "metadata object node was not found".to_string())
}

/// Whether platform 8.3.27 declares a root `ChildObjects` container.
///
/// The exact platform corpus proves a total split: a supported kind either
/// always carries this container or rejects it even when the container is
/// empty.
pub(crate) const fn meta_8_3_27_kind_declares_child_objects(kind: MetadataKind) -> bool {
    !matches!(
        kind,
        MetadataKind::CommonModule
            | MetadataKind::Constant
            | MetadataKind::DefinedType
            | MetadataKind::EventSubscription
            | MetadataKind::ScheduledJob
    )
}

pub(crate) fn meta_8_3_27_boolean_properties(object_type: &str) -> &'static [&'static str] {
    match object_type {
        "AccountingFlag" | "AddressingAttribute" | "Attribute" | "ExtDimensionAccountingFlag" => &[
            "PasswordMode",
            "MarkNegatives",
            "MultiLine",
            "ExtendedEdit",
            "FillFromFillingValue",
        ],
        "AccountingRegister" => &[
            "UseStandardCommands",
            "IncludeHelpInContents",
            "Correspondence",
            "EnableTotalsSplitting",
        ],
        "AccumulationRegister" => &[
            "UseStandardCommands",
            "IncludeHelpInContents",
            "EnableTotalsSplitting",
        ],
        "BusinessProcess" => &[
            "UseStandardCommands",
            "CheckUnique",
            "Autonumbering",
            "CreateTaskInPrivilegedMode",
            "IncludeHelpInContents",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "CalculationRegister" => &[
            "UseStandardCommands",
            "ActionPeriod",
            "BasePeriod",
            "IncludeHelpInContents",
        ],
        "Catalog" => &[
            "Hierarchical",
            "LimitLevelCount",
            "FoldersOnTop",
            "UseStandardCommands",
            "CheckUnique",
            "Autonumbering",
            "QuickChoice",
            "IncludeHelpInContents",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "ChartOfAccounts" => &[
            "UseStandardCommands",
            "IncludeHelpInContents",
            "CheckUnique",
            "QuickChoice",
            "AutoOrderByCode",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "ChartOfCalculationTypes" => &[
            "UseStandardCommands",
            "QuickChoice",
            "ActionPeriodUse",
            "IncludeHelpInContents",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "ChartOfCharacteristicTypes" => &[
            "UseStandardCommands",
            "IncludeHelpInContents",
            "Hierarchical",
            "FoldersOnTop",
            "CheckUnique",
            "Autonumbering",
            "QuickChoice",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "Command" => &["ModifiesData"],
        "CommonModule" => &[
            "Global",
            "ClientManagedApplication",
            "Server",
            "ExternalConnection",
            "ClientOrdinaryApplication",
            "Client",
            "ServerCall",
            "Privileged",
        ],
        "Constant" => &[
            "UseStandardCommands",
            "PasswordMode",
            "MarkNegatives",
            "MultiLine",
            "ExtendedEdit",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "DataProcessor" | "DocumentJournal" | "Report" => {
            &["UseStandardCommands", "IncludeHelpInContents"]
        }
        "Dimension" => &[
            "PasswordMode",
            "MarkNegatives",
            "MultiLine",
            "ExtendedEdit",
            "DenyIncompleteValues",
            "BaseDimension",
            "UseInTotals",
            "FillFromFillingValue",
            "Master",
            "MainFilter",
            "Balance",
        ],
        "Document" => &[
            "UseStandardCommands",
            "CheckUnique",
            "Autonumbering",
            "PostInPrivilegedMode",
            "UnpostInPrivilegedMode",
            "IncludeHelpInContents",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "Enum" => &["UseStandardCommands", "QuickChoice"],
        "ExchangePlan" => &[
            "UseStandardCommands",
            "QuickChoice",
            "DistributedInfoBase",
            "IncludeConfigurationExtensions",
            "IncludeHelpInContents",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "InformationRegister" => &[
            "UseStandardCommands",
            "MainFilterOnPeriod",
            "IncludeHelpInContents",
            "EnableTotalsSliceFirst",
            "EnableTotalsSliceLast",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "Operation" => &["Nillable", "Transactioned"],
        "Parameter" => &["Nillable"],
        "Resource" => &[
            "PasswordMode",
            "MarkNegatives",
            "MultiLine",
            "ExtendedEdit",
            "Balance",
            "FillFromFillingValue",
        ],
        "ScheduledJob" => &["Use", "Predefined"],
        "Task" => &[
            "UseStandardCommands",
            "CheckUnique",
            "Autonumbering",
            "IncludeHelpInContents",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        _ => &[],
    }
}

pub(crate) fn validate_meta_8_3_27_boolean_property_value(
    context: &str,
    object_type: &str,
    property_name: &str,
    value: &str,
) -> Result<(), String> {
    if !meta_8_3_27_boolean_properties(object_type).contains(&property_name) {
        return Ok(());
    }
    if matches!(value, "true" | "false") {
        Ok(())
    } else {
        Err(format!(
            "{context} property {object_type}.{property_name} value '{value}' is not a canonical xs:boolean for the fixed 8.3.27 contract; expected true or false"
        ))
    }
}

pub(crate) fn validate_metadata_8_3_27_boolean_contract(
    xml_text: &str,
    context: &str,
) -> Result<(), String> {
    let document = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("XML parse error: {error}"))?;
    let root_object = format_contract_object_node(&document)?;
    if !meta_validate_valid_types().contains(&root_object.tag_name().name()) {
        return Ok(());
    }

    for object in root_object
        .descendants()
        .filter(roxmltree::Node::is_element)
    {
        let object_type = object.tag_name().name();
        let boolean_properties = meta_8_3_27_boolean_properties(object_type);
        if boolean_properties.is_empty() {
            continue;
        }
        let Some(properties) = meta_info_child(object, "Properties") else {
            continue;
        };
        for property in properties.children().filter(roxmltree::Node::is_element) {
            let property_name = property.tag_name().name();
            if boolean_properties.contains(&property_name) {
                validate_meta_8_3_27_boolean_property_value(
                    context,
                    object_type,
                    property_name,
                    property.text().unwrap_or(""),
                )?;
            }
        }
    }

    Ok(())
}

pub(crate) fn validate_metadata_8_3_27_enum_contract(
    xml_text: &str,
    context: &str,
) -> Result<(), String> {
    let document = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("XML parse error: {error}"))?;
    let root_object = format_contract_object_node(&document)?;
    if !meta_validate_valid_types().contains(&root_object.tag_name().name()) {
        return Ok(());
    }

    for object in root_object
        .descendants()
        .filter(roxmltree::Node::is_element)
    {
        let Some(properties) = meta_info_child(object, "Properties") else {
            continue;
        };
        for (property_name, allowed) in meta_validate_property_values() {
            let Some(value) =
                meta_info_child_text(properties, property_name).filter(|value| !value.is_empty())
            else {
                continue;
            };
            if !allowed.contains(&value.as_str()) {
                return Err(format!(
                    "{context} property {}.{property_name} value '{value}' is not valid for the fixed 8.3.27 contract; expected one of: {}",
                    object.tag_name().name(),
                    allowed.join(", ")
                ));
            }
        }
    }

    Ok(())
}
