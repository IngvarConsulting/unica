use roxmltree::{Document, Node};

use crate::domain::metadata::{
    DateFractions, MetadataKind, MetadataType, MetadataTypeVariant, NumberSign, StringLengthMode,
};
use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};

use super::template_catalog::{
    metadata_generated_types_8_3_27, minimal_auxiliary_files, minimal_metadata_xml,
    minimal_metadata_xml_for_tests, split_meta_camel_case, MinimalTemplateContext,
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
fn typed_minimal_external_data_source_matches_platform_8_3_27_shape() {
    let (xml, _) = minimal_metadata_xml_for_tests(MetadataKind::ExternalDataSource, "Remote")
        .expect("typed ExternalDataSource template");
    let document = Document::parse(&xml).unwrap();
    let source = document
        .descendants()
        .find(|node| node.tag_name().name() == "ExternalDataSource")
        .expect("ExternalDataSource descriptor");
    let properties = meta_info_child(source, "Properties").unwrap();

    assert_eq!(
        direct_child_names(properties),
        ["Name", "Synonym", "Comment", "DataLockControlMode"],
        "{xml}"
    );
    assert_eq!(
        meta_info_child_text(properties, "DataLockControlMode").as_deref(),
        Some("Automatic")
    );
    assert!(meta_info_child(source, "ChildObjects").is_some(), "{xml}");
    assert_eq!(
        metadata_generated_types_8_3_27("ExternalDataSource"),
        Some(
            &[
                ("ExternalDataSourceManager", "Manager"),
                ("ExternalDataSourceTablesManager", "TablesManager"),
                ("ExternalDataSourceCubesManager", "CubesManager"),
            ][..]
        )
    );
}

#[test]
fn minimal_templates_emit_child_objects_only_where_the_kind_declares_them() {
    // 8.3.27 refuses to import a descriptor carrying `ChildObjects` for a kind
    // that has no child collection: `document format error: unexpected read
    // property. Current property: ChildObjects, expected property: <Kind>`.
    // The childless set is measured against a real 8.3.27 dump — across 12k
    // descriptors every kind is all-or-nothing, never mixed — and confirmed by
    // the exact platform gate, where every other kind imports with the element.
    const CHILDLESS: &[MetadataKind] = &[
        MetadataKind::CommonModule,
        MetadataKind::Constant,
        MetadataKind::DefinedType,
        MetadataKind::EventSubscription,
        MetadataKind::ScheduledJob,
    ];

    for kind in MetadataKind::ALL {
        let (xml, _) = minimal_metadata_xml_for_tests(*kind, "Evidence").unwrap();
        let document = Document::parse(&xml).unwrap();
        let object = document
            .root_element()
            .children()
            .find(Node::is_element)
            .unwrap();

        let emitted = meta_info_child(object, "ChildObjects").is_some();
        assert_eq!(
            emitted,
            !CHILDLESS.contains(kind),
            "{} emitted ChildObjects={emitted}: {xml}",
            kind.as_str()
        );
    }
}

#[test]
fn business_process_template_names_its_mandatory_task() {
    // 8.3.27 refuses to import a business process without a task:
    // `БизнесПроцесс.<Имя> - Business process task not selected`.
    let context = MinimalTemplateContext {
        chart_of_accounts: None,
        chart_of_calculation_types: None,
        task: Some("Task.CorpusTask".to_string()),
        registered_documents: vec![],
        method_name: None,
        event_source: None,
        event_handler: None,
        dependencies: Vec::new(),
    };
    let (xml, _) =
        minimal_metadata_xml(MetadataKind::BusinessProcess, "Evidence", "2.20", &context).unwrap();
    assert!(
        xml.contains("<Task>Task.CorpusTask</Task>"),
        "business process template must reference the registered task: {xml}"
    );
}

#[test]
fn document_journal_template_registers_its_mandatory_document() {
    // 8.3.27 refuses to import a journal with no registered documents:
    // `ЖурналДокументов.<Имя> - No recorded documents specified for log`.
    let context = MinimalTemplateContext {
        chart_of_accounts: None,
        chart_of_calculation_types: None,
        task: None,
        registered_documents: vec!["Document.CorpusDocument".to_string()],
        method_name: None,
        event_source: None,
        event_handler: None,
        dependencies: Vec::new(),
    };
    let (xml, _) =
        minimal_metadata_xml(MetadataKind::DocumentJournal, "Evidence", "2.20", &context).unwrap();
    assert!(
        xml.contains(
            "<RegisteredDocuments>\n\t\t\t\t<xr:Item xsi:type=\"xr:MDObjectRef\">Document.CorpusDocument</xr:Item>\n\t\t\t</RegisteredDocuments>"
        ),
        "journal template must register the document: {xml}"
    );
    assert!(!xml.contains("<RegisteredDocuments/>"), "{xml}");
}

#[test]
fn minimal_templates_never_invent_content() {
    // Выдуманный ресурс — молчаливое решение за вызывающего, которое почти
    // всегда переделывают, а в выгрузке остаётся мусором (ADR-0030).
    for kind in [
        MetadataKind::AccountingRegister,
        MetadataKind::CalculationRegister,
    ] {
        let (xml, _) = minimal_metadata_xml_for_tests(kind, "Evidence").unwrap();
        assert!(
            !xml.contains("<Resource"),
            "{} invented a resource: {xml}",
            kind.as_str()
        );
    }
}

#[test]
fn generated_synonym_separates_digits_from_the_words_around_them() {
    for (name, synonym) in [
        ("СуммаЗакупокЗа30Дней", "Сумма закупок за 30 дней"),
        ("SumOfPurchasesFor30Days", "Sum of purchases for 30 days"),
        ("Строка1", "Строка 1"),
        ("Версия20250101", "Версия 20250101"),
        ("ДатаНачала", "Дата начала"),
        ("Товар", "Товар"),
        ("", ""),
    ] {
        assert_eq!(split_meta_camel_case(name), synonym, "{name}");
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
