use roxmltree::{Document, Node};

use crate::domain::metadata::{
    DateFractions, MetadataKind, MetadataType, MetadataTypeVariant, NumberSign, StringLengthMode,
};
use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};

use super::template_catalog::{
    metadata_generated_types_8_3_27, minimal_auxiliary_files, minimal_metadata_xml_for_tests,
};
use super::xml_model::{emit_meta_typed_value_type, meta_info_child, meta_info_child_text};

fn direct_child_names<'a>(node: Node<'a, '_>) -> Vec<&'a str> {
    node.children()
        .filter(Node::is_element)
        .map(|child| child.tag_name().name())
        .collect()
}

#[test]
fn typed_minimal_catalog_emits_non_hierarchical_platform_shape() {
    let (xml, _) = minimal_metadata_xml_for_tests(MetadataKind::ChartOfAccounts, "Accounts")
        .expect("typed ChartOfAccounts template");
    let document = Document::parse(&xml).unwrap();
    let properties = document
        .descendants()
        .find(|node| node.tag_name().name() == "ChartOfAccounts")
        .and_then(|node| meta_info_child(node, "Properties"))
        .unwrap();

    assert_eq!(
        direct_child_names(properties),
        [
            "Name",
            "Synonym",
            "Comment",
            "UseStandardCommands",
            "IncludeHelpInContents",
            "BasedOn",
            "ExtDimensionTypes",
            "MaxExtDimensionCount",
            "CodeMask",
            "CodeLength",
            "DescriptionLength",
            "CodeSeries",
            "CheckUnique",
            "DefaultPresentation",
            "StandardAttributes",
            "Characteristics",
            "StandardTabularSections",
            "PredefinedDataUpdate",
            "EditType",
            "QuickChoice",
            "ChoiceMode",
            "InputByString",
            "SearchStringModeOnInputByString",
            "FullTextSearchOnInputByString",
            "ChoiceDataGetModeOnInputByString",
            "CreateOnInput",
            "ChoiceHistoryOnInput",
            "DefaultObjectForm",
            "DefaultListForm",
            "DefaultChoiceForm",
            "AuxiliaryObjectForm",
            "AuxiliaryListForm",
            "AuxiliaryChoiceForm",
            "AutoOrderByCode",
            "OrderLength",
            "DataLockFields",
            "DataLockControlMode",
            "FullTextSearch",
            "DataHistory",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
            "ObjectPresentation",
            "ExtendedObjectPresentation",
            "ListPresentation",
            "ExtendedListPresentation",
            "Explanation",
        ],
        "{xml}"
    );
    assert!(
        meta_info_child(properties, "Hierarchical").is_none(),
        "{xml}"
    );
    assert_eq!(
        meta_info_child_text(properties, "MaxExtDimensionCount").as_deref(),
        Some("0")
    );
}

#[test]
fn typed_minimal_templates_cover_all_public_add_kinds() {
    for kind in MetadataKind::ALL {
        let (xml, _) = minimal_metadata_xml_for_tests(*kind, "Evidence").unwrap();
        let document = Document::parse(&xml).unwrap();
        assert_eq!(
            document.root_element().tag_name().name(),
            "MetaDataObject",
            "{}",
            kind.as_str()
        );
        assert!(
            metadata_generated_types_8_3_27(kind.as_str()).is_some(),
            "{}",
            kind.as_str()
        );
    }
}

#[test]
fn typed_value_type_groups_platform_tags_before_qualifiers() {
    let value_type = MetadataType {
        variants: vec![
            MetadataTypeVariant::DefinedType {
                metadata_path: MetadataAddress::parse(
                    PLATFORM_XML_8_3_27_FORMAT_2_20,
                    "DefinedType.Money",
                )
                .unwrap(),
            },
            MetadataTypeVariant::String {
                length: 100,
                allowed_length: StringLengthMode::Variable,
            },
            MetadataTypeVariant::Number {
                digits: 15,
                fraction: 2,
                sign: NumberSign::Any,
            },
            MetadataTypeVariant::Date {
                fractions: DateFractions::DateTime,
            },
        ],
    };
    let mut lines = Vec::new();
    emit_meta_typed_value_type(&mut lines, "", &value_type);
    let xml = lines.join("");

    let concrete = xml.find("<v8:Type>xs:string</v8:Type>").unwrap();
    let type_set = xml
        .find("<v8:TypeSet>cfg:DefinedType.Money</v8:TypeSet>")
        .unwrap();
    let number = xml.find("<v8:NumberQualifiers>").unwrap();
    let string = xml.find("<v8:StringQualifiers>").unwrap();
    let date = xml.find("<v8:DateQualifiers>").unwrap();
    assert!(concrete < type_set && type_set < number && number < string && string < date);
}

#[test]
fn typed_auxiliary_xml_matches_platform_fixtures() {
    let exchange = minimal_auxiliary_files(MetadataKind::ExchangePlan, "2.20");
    assert_eq!(exchange.len(), 1);
    assert_eq!(
        exchange[0].2.replace("\r\n", "\n"),
        include_str!("../../../../../../tests/fixtures/platform_8_3_27/exchange_plan/Content.xml")
            .replace("\r\n", "\n")
    );

    let process = minimal_auxiliary_files(MetadataKind::BusinessProcess, "2.20");
    assert_eq!(process.len(), 1);
    assert!(process[0].2.contains("<GraphicalSchema "));
    assert!(process[0].2.contains("<Items/>"));
}
