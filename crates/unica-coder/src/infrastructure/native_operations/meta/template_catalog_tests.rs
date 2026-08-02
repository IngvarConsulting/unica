#![allow(dead_code, unused_imports)]

use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::fs;

use super::info::resolve_meta_info_path;
use super::legacy_dsl::{meta_compile_extra_ext_files, meta_compile_object_xml};
use super::template_catalog::{
    emit_meta_standard_attribute, emit_meta_standard_attributes, meta_compile_catalog_xml,
};
use super::xml_model::{
    emit_meta_type_content, emit_meta_type_contents, meta_info_child, meta_info_child_text,
    meta_info_children, meta_info_inner_text, validate_meta_type_union,
};

#[test]
fn meta_info_directory_fallback_selects_first_xml_by_file_name() {
    let root = std::env::temp_dir().join(format!(
        "unica-meta-info-sorted-fallback-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    fs::create_dir_all(&root).unwrap();
    let later = root.join("z-later.xml");
    let first = root.join("a-first.xml");
    fs::write(&later, "<later/>").unwrap();
    fs::write(&first, "<first/>").unwrap();

    assert_eq!(resolve_meta_info_path(root.clone()).unwrap(), first);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn exchange_plan_content_matches_8_3_27() {
    let files = meta_compile_extra_ext_files("ExchangePlan", "2.20");
    let (name, content) = &files[0];
    let expected =
        include_str!("../../../../../../tests/fixtures/platform_8_3_27/exchange_plan/Content.xml");

    assert_eq!(*name, "Content.xml");
    assert_eq!(
        content.replace("\r\n", "\n"),
        expected.replace("\r\n", "\n")
    );
}

#[test]
fn standard_attributes_emit_platform_type_reduction_modes_in_order() {
    let (xml, _) = meta_compile_catalog_xml(&Map::new(), "CorpusCatalog", "2.20").unwrap();
    let document = Document::parse(&xml).unwrap();
    let standard_attributes = document
        .descendants()
        .find(|node| node.tag_name().name() == "StandardAttributes")
        .unwrap();
    let expected = [
        ("PredefinedDataName", "TransformValues"),
        ("Predefined", "TransformValues"),
        ("Ref", "TransformValues"),
        ("DeletionMark", "TransformValues"),
        ("IsFolder", "TransformValues"),
        ("Owner", "Deny"),
        ("Parent", "TransformValues"),
        ("Description", "TransformValues"),
        ("Code", "TransformValues"),
    ];
    let attributes = standard_attributes
        .children()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    assert_eq!(attributes.len(), expected.len());

    for (attribute, (expected_name, expected_mode)) in attributes.iter().zip(expected) {
        assert_eq!(attribute.attribute("name"), Some(expected_name));
        let children = attribute
            .children()
            .filter(|node| node.is_element())
            .collect::<Vec<_>>();
        let child_names = children
            .iter()
            .map(|node| node.tag_name().name())
            .collect::<Vec<_>>();
        let create_on_input = child_names
            .iter()
            .position(|name| *name == "CreateOnInput")
            .unwrap();
        let type_reduction_mode = child_names
            .iter()
            .position(|name| *name == "TypeReductionMode")
            .unwrap();
        let max_value = child_names
            .iter()
            .position(|name| *name == "MaxValue")
            .unwrap();
        assert_eq!(type_reduction_mode, create_on_input + 1, "{expected_name}");
        assert_eq!(max_value, type_reduction_mode + 1, "{expected_name}");
        assert_eq!(children[type_reduction_mode].text(), Some(expected_mode));
    }

    for object_type in [
        "Document",
        "Enum",
        "InformationRegister",
        "AccumulationRegister",
        "AccountingRegister",
        "CalculationRegister",
        "ChartOfAccounts",
        "ChartOfCharacteristicTypes",
        "ChartOfCalculationTypes",
        "BusinessProcess",
        "Task",
        "ExchangePlan",
        "DocumentJournal",
        "TabularSection",
    ] {
        let mut lines = Vec::new();
        emit_meta_standard_attributes(&mut lines, "\t", object_type);
        let xml = format!(
                "<Properties xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">{}</Properties>",
                lines.join("")
            );
        let document = Document::parse(&xml).unwrap();
        let attributes = document
            .descendants()
            .filter(|node| node.tag_name().name() == "StandardAttribute")
            .collect::<Vec<_>>();
        assert!(!attributes.is_empty(), "{object_type}");
        for attribute in attributes {
            let children = attribute
                .children()
                .filter(|node| node.is_element())
                .collect::<Vec<_>>();
            let child_names = children
                .iter()
                .map(|node| node.tag_name().name())
                .collect::<Vec<_>>();
            let create_on_input = child_names
                .iter()
                .position(|name| *name == "CreateOnInput")
                .unwrap();
            let type_reduction_mode = child_names
                .iter()
                .position(|name| *name == "TypeReductionMode")
                .unwrap();
            let max_value = child_names
                .iter()
                .position(|name| *name == "MaxValue")
                .unwrap();
            assert_eq!(
                type_reduction_mode,
                create_on_input + 1,
                "{object_type}.{}",
                attribute.attribute("name").unwrap_or("")
            );
            assert_eq!(max_value, type_reduction_mode + 1, "{object_type}");
            assert_eq!(
                children[type_reduction_mode].text(),
                Some("TransformValues"),
                "{object_type}.{}",
                attribute.attribute("name").unwrap_or("")
            );
        }
    }

    let mut ext_dimension_lines = Vec::new();
    emit_meta_standard_attribute(
        &mut ext_dimension_lines,
        "",
        "ChartOfAccounts.ExtDimensionTypes",
        "ExtDimensionType",
    );
    assert!(
        ext_dimension_lines
            .iter()
            .any(|line| line
                .contains("<xr:TypeReductionMode>TransformValues</xr:TypeReductionMode>")),
        "{}",
        ext_dimension_lines.join("\n")
    );
}

#[test]
fn document_tabular_section_emits_platform_line_number_length() {
    let definition = json!({
        "tabularSections": {
            "Lines": ["Quantity:Number(10,2)"]
        }
    });
    let (xml, _) = meta_compile_object_xml(
        definition.as_object().unwrap(),
        "Document",
        "CorpusDocument",
        "2.20",
    )
    .unwrap();
    let document = Document::parse(&xml).unwrap();
    let properties = document
        .descendants()
        .find(|node| node.tag_name().name() == "TabularSection" && node.attribute("uuid").is_some())
        .and_then(|section| {
            section
                .children()
                .find(|node| node.is_element() && node.tag_name().name() == "Properties")
        })
        .unwrap();
    let children = properties
        .children()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    let standard_attributes = children
        .iter()
        .position(|node| node.tag_name().name() == "StandardAttributes")
        .unwrap();
    let line_number_length = children
        .iter()
        .position(|node| node.tag_name().name() == "LineNumberLength")
        .unwrap();

    assert_eq!(line_number_length, standard_attributes + 1, "{xml}");
    assert_eq!(children[line_number_length].text(), Some("9"), "{xml}");
}

#[test]
fn business_process_flowchart_matches_platform_8_3_27_schema() {
    let files = meta_compile_extra_ext_files("BusinessProcess", "2.20");
    let (name, content) = files
        .iter()
        .find(|(name, _)| *name == "Flowchart.xml")
        .unwrap();
    let document = Document::parse(content.trim_start_matches('\u{feff}')).unwrap();
    let root = document.root_element();

    assert_eq!(*name, "Flowchart.xml");
    assert_eq!(root.tag_name().name(), "GraphicalSchema");
    assert_eq!(
        root.tag_name().namespace(),
        Some("http://v8.1c.ru/8.3/xcf/scheme")
    );
    assert_eq!(root.attribute("version"), Some("2.20"));
    assert_eq!(
        test_meta_direct_child_names(root),
        [
            "BackColor",
            "GridEnabled",
            "DrawGridMode",
            "GridHorizontalStep",
            "GridVerticalStep",
            "PrintParameters",
            "Items",
        ]
    );
    let print_parameters = meta_info_child(root, "PrintParameters").unwrap();
    assert_eq!(
        test_meta_direct_child_names(print_parameters),
        [
            "TopMargin",
            "LeftMargin",
            "BottomMargin",
            "RightMargin",
            "BlackAndWhite",
            "FitPageMode",
        ]
    );
}

#[test]
fn chart_characteristic_generated_type_uses_platform_prefix() {
    let xml = test_compile_meta_xml(
        "ChartOfCharacteristicTypes",
        "CorpusCharacteristics",
        json!({}),
    );
    let document = Document::parse(&xml).unwrap();
    let characteristic = document
        .descendants()
        .find(|node| {
            node.is_element()
                && node.tag_name().name() == "GeneratedType"
                && node.attribute("category") == Some("Characteristic")
        })
        .unwrap();

    assert_eq!(
        characteristic.attribute("name"),
        Some("Characteristic.CorpusCharacteristics")
    );
}

#[test]
fn multi_value_types_follow_platform_type_description_order() {
    for (object_type, object_name) in [
        ("ChartOfCharacteristicTypes", "CorpusCharacteristics"),
        ("DefinedType", "CorpusDefinedType"),
    ] {
        let xml = test_compile_meta_xml(
            object_type,
            object_name,
            json!({"valueTypes": ["String(100)", "Number(15,2)"]}),
        );
        let document = Document::parse(&xml).unwrap();
        let properties = test_meta_root_properties(&document);
        let type_node = meta_info_child(properties, "Type").unwrap();

        assert_eq!(
            test_meta_direct_child_names(type_node),
            ["Type", "Type", "NumberQualifiers", "StringQualifiers"],
            "{object_type}: {xml}"
        );
    }
}

#[test]
fn object_specific_standard_attributes_match_platform_order() {
    for (object_type, expected) in [
        (
            "BusinessProcess",
            vec![
                "Started",
                "HeadTask",
                "Completed",
                "Ref",
                "DeletionMark",
                "Date",
                "Number",
            ],
        ),
        (
            "Task",
            vec![
                "Executed",
                "Description",
                "RoutePoint",
                "BusinessProcess",
                "Ref",
                "DeletionMark",
                "Date",
                "Number",
            ],
        ),
        (
            "ChartOfCharacteristicTypes",
            vec![
                "PredefinedDataName",
                "ValueType",
                "Description",
                "Code",
                "IsFolder",
                "Parent",
                "Predefined",
                "DeletionMark",
                "Ref",
            ],
        ),
        (
            "ExchangePlan",
            vec![
                "ExchangeDate",
                "ThisNode",
                "ReceivedNo",
                "SentNo",
                "Ref",
                "DeletionMark",
                "Description",
                "Code",
            ],
        ),
    ] {
        let mut lines = Vec::new();
        emit_meta_standard_attributes(&mut lines, "", object_type);
        let xml = format!(
                "<Properties xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">{}</Properties>",
                lines.join("")
            );
        let document = Document::parse(&xml).unwrap();
        let attributes = document
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "StandardAttribute")
            .map(|node| node.attribute("name").unwrap_or_default())
            .collect::<Vec<_>>();

        assert_eq!(attributes, expected, "{object_type}");
    }
}

#[test]
fn business_process_properties_match_platform_order_and_defaults() {
    let xml = test_compile_meta_xml(
        "BusinessProcess",
        "CorpusBusinessProcess",
        json!({"task": "Task.CorpusTask"}),
    );
    let document = Document::parse(&xml).unwrap();
    let properties = test_meta_root_properties(&document);

    assert_eq!(
        test_meta_direct_child_names(properties),
        [
            "Name",
            "Synonym",
            "Comment",
            "UseStandardCommands",
            "EditType",
            "InputByString",
            "CreateOnInput",
            "SearchStringModeOnInputByString",
            "ChoiceDataGetModeOnInputByString",
            "FullTextSearchOnInputByString",
            "DefaultObjectForm",
            "DefaultListForm",
            "DefaultChoiceForm",
            "AuxiliaryObjectForm",
            "AuxiliaryListForm",
            "AuxiliaryChoiceForm",
            "ChoiceHistoryOnInput",
            "NumberType",
            "NumberLength",
            "NumberAllowedLength",
            "CheckUnique",
            "StandardAttributes",
            "Characteristics",
            "Autonumbering",
            "BasedOn",
            "NumberPeriodicity",
            "Task",
            "CreateTaskInPrivilegedMode",
            "DataLockFields",
            "DataLockControlMode",
            "IncludeHelpInContents",
            "FullTextSearch",
            "ObjectPresentation",
            "ExtendedObjectPresentation",
            "ListPresentation",
            "ExtendedListPresentation",
            "Explanation",
            "DataHistory",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ]
    );
    assert_eq!(
        meta_info_child_text(properties, "NumberPeriodicity").as_deref(),
        Some("Nonperiodical")
    );
    assert_eq!(
        meta_info_child_text(properties, "CreateTaskInPrivilegedMode").as_deref(),
        Some("true")
    );
}

#[test]
fn task_properties_match_platform_order_and_defaults() {
    let xml = test_compile_meta_xml("Task", "CorpusTask", json!({}));
    let document = Document::parse(&xml).unwrap();
    let properties = test_meta_root_properties(&document);

    assert_eq!(
        test_meta_direct_child_names(properties),
        [
            "Name",
            "Synonym",
            "Comment",
            "UseStandardCommands",
            "NumberType",
            "NumberLength",
            "NumberAllowedLength",
            "CheckUnique",
            "Autonumbering",
            "TaskNumberAutoPrefix",
            "DescriptionLength",
            "Addressing",
            "MainAddressingAttribute",
            "CurrentPerformer",
            "BasedOn",
            "StandardAttributes",
            "Characteristics",
            "DefaultPresentation",
            "EditType",
            "InputByString",
            "SearchStringModeOnInputByString",
            "FullTextSearchOnInputByString",
            "ChoiceDataGetModeOnInputByString",
            "CreateOnInput",
            "DefaultObjectForm",
            "DefaultListForm",
            "DefaultChoiceForm",
            "AuxiliaryObjectForm",
            "AuxiliaryListForm",
            "AuxiliaryChoiceForm",
            "ChoiceHistoryOnInput",
            "IncludeHelpInContents",
            "DataLockFields",
            "DataLockControlMode",
            "FullTextSearch",
            "ObjectPresentation",
            "ExtendedObjectPresentation",
            "ListPresentation",
            "ExtendedListPresentation",
            "Explanation",
            "DataHistory",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ]
    );
}

#[test]
fn chart_characteristic_properties_match_platform_order_and_defaults() {
    let xml = test_compile_meta_xml(
        "ChartOfCharacteristicTypes",
        "CorpusCharacteristics",
        json!({"valueTypes": ["String(50)", "Number(15,2)"]}),
    );
    let document = Document::parse(&xml).unwrap();
    let properties = test_meta_root_properties(&document);

    assert_eq!(
        test_meta_direct_child_names(properties),
        [
            "Name",
            "Synonym",
            "Comment",
            "UseStandardCommands",
            "IncludeHelpInContents",
            "CharacteristicExtValues",
            "Type",
            "Hierarchical",
            "FoldersOnTop",
            "CodeLength",
            "CodeAllowedLength",
            "DescriptionLength",
            "CodeSeries",
            "CheckUnique",
            "Autonumbering",
            "DefaultPresentation",
            "StandardAttributes",
            "Characteristics",
            "PredefinedDataUpdate",
            "EditType",
            "QuickChoice",
            "ChoiceMode",
            "InputByString",
            "CreateOnInput",
            "SearchStringModeOnInputByString",
            "ChoiceDataGetModeOnInputByString",
            "FullTextSearchOnInputByString",
            "ChoiceHistoryOnInput",
            "DefaultObjectForm",
            "DefaultFolderForm",
            "DefaultListForm",
            "DefaultChoiceForm",
            "DefaultFolderChoiceForm",
            "AuxiliaryObjectForm",
            "AuxiliaryFolderForm",
            "AuxiliaryListForm",
            "AuxiliaryChoiceForm",
            "AuxiliaryFolderChoiceForm",
            "BasedOn",
            "DataLockFields",
            "DataLockControlMode",
            "FullTextSearch",
            "ObjectPresentation",
            "ExtendedObjectPresentation",
            "ListPresentation",
            "ExtendedListPresentation",
            "Explanation",
            "DataHistory",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ]
    );
    assert_eq!(
        meta_info_child_text(properties, "CodeSeries").as_deref(),
        Some("WholeCharacteristicKind")
    );
}

#[test]
fn accumulation_register_standard_attributes_match_platform_order() {
    let xml = test_compile_meta_xml(
        "AccumulationRegister",
        "CorpusAccumulationRegister",
        json!({
            "registerType": "Balances",
            "dimensions": ["Warehouse:String(50)|index"],
            "resources": ["Quantity:Number(15,3)"]
        }),
    );
    let document = Document::parse(&xml).unwrap();
    let properties = test_meta_root_properties(&document);

    assert_eq!(
        test_meta_standard_attribute_names(properties),
        ["RecordType", "Active", "LineNumber", "Recorder", "Period"],
        "{xml}"
    );
}

#[test]
fn accounting_register_matches_platform_order_and_defaults() {
    let xml = test_compile_meta_xml(
        "AccountingRegister",
        "CorpusAccountingRegister",
        json!({
            "chartOfAccounts": "ChartOfAccounts.CorpusAccounts",
            "dimensions": ["Department:String(50)"],
            "resources": ["Amount:Number(15,2)"]
        }),
    );
    let document = Document::parse(&xml).unwrap();
    let properties = test_meta_root_properties(&document);

    assert_eq!(
        test_meta_direct_child_names(properties),
        [
            "Name",
            "Synonym",
            "Comment",
            "UseStandardCommands",
            "IncludeHelpInContents",
            "ChartOfAccounts",
            "Correspondence",
            "PeriodAdjustmentLength",
            "DefaultListForm",
            "AuxiliaryListForm",
            "StandardAttributes",
            "DataLockControlMode",
            "EnableTotalsSplitting",
            "FullTextSearch",
            "ListPresentation",
            "ExtendedListPresentation",
            "Explanation",
        ],
        "{xml}"
    );
    assert_eq!(
        test_meta_standard_attribute_names(properties),
        [
            "Account",
            "RecordType",
            "Active",
            "LineNumber",
            "Recorder",
            "Period"
        ],
        "{xml}"
    );
    assert_eq!(
        meta_info_child_text(properties, "EnableTotalsSplitting").as_deref(),
        Some("false"),
        "{xml}"
    );

    let child_objects = test_meta_root_child_objects(&document);
    assert_eq!(
        test_meta_direct_child_names(child_objects),
        ["Dimension", "Resource"],
        "{xml}"
    );
    let dimension = test_meta_named_object(&document, "Dimension", "Department");
    assert_eq!(
        test_meta_direct_child_names(meta_info_child(dimension, "Properties").unwrap()),
        [
            "Name",
            "Synonym",
            "Comment",
            "Type",
            "PasswordMode",
            "Format",
            "EditFormat",
            "ToolTip",
            "MarkNegatives",
            "Mask",
            "MultiLine",
            "ExtendedEdit",
            "MinValue",
            "MaxValue",
            "FillChecking",
            "ChoiceFoldersAndItems",
            "ChoiceParameterLinks",
            "ChoiceParameters",
            "QuickChoice",
            "CreateOnInput",
            "ChoiceForm",
            "LinkByType",
            "ChoiceHistoryOnInput",
            "Balance",
            "AccountingFlag",
            "DenyIncompleteValues",
            "Indexing",
            "FullTextSearch",
        ],
        "{xml}"
    );
    let resource = test_meta_named_object(&document, "Resource", "Amount");
    assert_eq!(
        test_meta_direct_child_names(meta_info_child(resource, "Properties").unwrap()),
        [
            "Name",
            "Synonym",
            "Comment",
            "Type",
            "PasswordMode",
            "Format",
            "EditFormat",
            "ToolTip",
            "MarkNegatives",
            "Mask",
            "MultiLine",
            "ExtendedEdit",
            "MinValue",
            "MaxValue",
            "FillChecking",
            "ChoiceFoldersAndItems",
            "ChoiceParameterLinks",
            "ChoiceParameters",
            "QuickChoice",
            "CreateOnInput",
            "ChoiceForm",
            "LinkByType",
            "ChoiceHistoryOnInput",
            "Balance",
            "AccountingFlag",
            "ExtDimensionAccountingFlag",
            "FullTextSearch",
        ],
        "{xml}"
    );
}

#[test]
fn calculation_register_matches_platform_order_and_defaults() {
    let xml = test_compile_meta_xml(
        "CalculationRegister",
        "CorpusCalculationRegister",
        json!({
            "chartOfCalculationTypes": "ChartOfCalculationTypes.CorpusCalculationTypes",
            "periodicity": "Month",
            "dimensions": ["Employee:String(50)"],
            "resources": ["Result:Number(15,2)"]
        }),
    );
    let document = Document::parse(&xml).unwrap();
    let properties = test_meta_root_properties(&document);

    assert_eq!(
        test_meta_direct_child_names(properties),
        [
            "Name",
            "Synonym",
            "Comment",
            "UseStandardCommands",
            "DefaultListForm",
            "AuxiliaryListForm",
            "Periodicity",
            "ActionPeriod",
            "BasePeriod",
            "Schedule",
            "ScheduleValue",
            "ScheduleDate",
            "ChartOfCalculationTypes",
            "IncludeHelpInContents",
            "StandardAttributes",
            "DataLockControlMode",
            "FullTextSearch",
            "ListPresentation",
            "ExtendedListPresentation",
            "Explanation",
        ],
        "{xml}"
    );
    assert_eq!(
        test_meta_standard_attribute_names(properties),
        [
            "RegistrationPeriod",
            "ReversingEntry",
            "Active",
            "EndOfBasePeriod",
            "BegOfBasePeriod",
            "EndOfActionPeriod",
            "BegOfActionPeriod",
            "ActionPeriod",
            "CalculationType",
            "LineNumber",
            "Recorder",
        ],
        "{xml}"
    );
    let dimension = test_meta_named_object(&document, "Dimension", "Employee");
    let dimension_properties = meta_info_child(dimension, "Properties").unwrap();
    assert_eq!(
        test_meta_direct_child_names(dimension_properties),
        [
            "Name",
            "Synonym",
            "Comment",
            "Type",
            "PasswordMode",
            "Format",
            "EditFormat",
            "ToolTip",
            "MarkNegatives",
            "Mask",
            "MultiLine",
            "ExtendedEdit",
            "MinValue",
            "MaxValue",
            "FillChecking",
            "ChoiceFoldersAndItems",
            "ChoiceParameterLinks",
            "ChoiceParameters",
            "QuickChoice",
            "CreateOnInput",
            "ChoiceForm",
            "LinkByType",
            "ChoiceHistoryOnInput",
            "DenyIncompleteValues",
            "BaseDimension",
            "ScheduleLink",
            "Indexing",
            "FullTextSearch",
        ],
        "{xml}"
    );
}

#[test]
fn chart_of_accounts_matches_platform_order_and_defaults() {
    let xml = test_compile_meta_xml(
        "ChartOfAccounts",
        "CorpusAccounts",
        json!({"accountingFlags": ["Tax"]}),
    );
    let document = Document::parse(&xml).unwrap();
    let properties = test_meta_root_properties(&document);

    assert_eq!(
        test_meta_direct_child_names(properties),
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
    assert_eq!(
        meta_info_child_text(properties, "MaxExtDimensionCount").as_deref(),
        Some("0"),
        "{xml}"
    );
    assert_eq!(
        meta_info_child_text(properties, "CodeSeries").as_deref(),
        Some("WholeChartOfAccounts"),
        "{xml}"
    );
    assert_eq!(
        test_meta_standard_attribute_names(properties),
        [
            "PredefinedDataName",
            "Order",
            "OffBalance",
            "Type",
            "Description",
            "Code",
            "Parent",
            "Predefined",
            "DeletionMark",
            "Ref",
        ],
        "{xml}"
    );
    let section = meta_info_child(properties, "StandardTabularSections")
        .unwrap()
        .children()
        .find(roxmltree::Node::is_element)
        .unwrap();
    assert_eq!(
        test_meta_direct_child_names(section),
        [
            "Synonym",
            "Comment",
            "ToolTip",
            "FillChecking",
            "StandardAttributes"
        ],
        "{xml}"
    );
    assert_eq!(
        section
            .descendants()
            .find(|node| node.tag_name().name() == "content")
            .and_then(|node| node.text()),
        Some("Extra dimension types"),
        "{xml}"
    );
    let flag = test_meta_named_object(&document, "AccountingFlag", "Tax");
    assert_eq!(
        test_meta_direct_child_names(meta_info_child(flag, "Properties").unwrap()),
        [
            "Name",
            "Synonym",
            "Comment",
            "Type",
            "PasswordMode",
            "Format",
            "EditFormat",
            "ToolTip",
            "MarkNegatives",
            "Mask",
            "MultiLine",
            "ExtendedEdit",
            "MinValue",
            "MaxValue",
            "FillFromFillingValue",
            "FillValue",
            "FillChecking",
            "ChoiceFoldersAndItems",
            "ChoiceParameterLinks",
            "ChoiceParameters",
            "QuickChoice",
            "CreateOnInput",
            "ChoiceForm",
            "LinkByType",
            "ChoiceHistoryOnInput",
            "DataHistory",
        ],
        "{xml}"
    );
}

#[test]
fn chart_of_calculation_types_matches_platform_order_and_defaults() {
    let xml = test_compile_meta_xml(
        "ChartOfCalculationTypes",
        "CorpusCalculationTypes",
        json!({}),
    );
    let document = Document::parse(&xml).unwrap();
    let properties = test_meta_root_properties(&document);

    assert_eq!(
        test_meta_direct_child_names(properties),
        [
            "Name",
            "Synonym",
            "Comment",
            "UseStandardCommands",
            "CodeLength",
            "DescriptionLength",
            "CodeType",
            "CodeAllowedLength",
            "DefaultPresentation",
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
            "BasedOn",
            "DependenceOnCalculationTypes",
            "BaseCalculationTypes",
            "ActionPeriodUse",
            "StandardAttributes",
            "Characteristics",
            "PredefinedDataUpdate",
            "IncludeHelpInContents",
            "DataLockFields",
            "DataLockControlMode",
            "FullTextSearch",
            "ObjectPresentation",
            "ExtendedObjectPresentation",
            "ListPresentation",
            "ExtendedListPresentation",
            "Explanation",
            "DataHistory",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "{xml}"
    );
    assert_eq!(
        test_meta_standard_attribute_names(properties),
        [
            "PredefinedDataName",
            "Predefined",
            "Ref",
            "DeletionMark",
            "ActionPeriodIsBasic",
            "Description",
            "Code",
        ],
        "{xml}"
    );
}

#[test]
fn exchange_plan_properties_match_platform_order_and_defaults() {
    let xml = test_compile_meta_xml(
        "ExchangePlan",
        "CorpusExchangePlan",
        json!({
            "distributedInfoBase": true,
            "includeConfigurationExtensions": true
        }),
    );
    let document = Document::parse(&xml).unwrap();
    let properties = test_meta_root_properties(&document);

    assert_eq!(
        test_meta_direct_child_names(properties),
        [
            "Name",
            "Synonym",
            "Comment",
            "UseStandardCommands",
            "CodeLength",
            "CodeAllowedLength",
            "DescriptionLength",
            "DefaultPresentation",
            "EditType",
            "QuickChoice",
            "ChoiceMode",
            "InputByString",
            "SearchStringModeOnInputByString",
            "FullTextSearchOnInputByString",
            "ChoiceDataGetModeOnInputByString",
            "DefaultObjectForm",
            "DefaultListForm",
            "DefaultChoiceForm",
            "AuxiliaryObjectForm",
            "AuxiliaryListForm",
            "AuxiliaryChoiceForm",
            "StandardAttributes",
            "Characteristics",
            "BasedOn",
            "DistributedInfoBase",
            "IncludeConfigurationExtensions",
            "CreateOnInput",
            "ChoiceHistoryOnInput",
            "IncludeHelpInContents",
            "DataLockFields",
            "DataLockControlMode",
            "FullTextSearch",
            "ObjectPresentation",
            "ExtendedObjectPresentation",
            "ListPresentation",
            "ExtendedListPresentation",
            "Explanation",
            "DataHistory",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ]
    );
}

#[test]
fn constant_data_lock_mode_precedes_data_history() {
    let xml = test_compile_meta_xml(
        "Constant",
        "CorpusConstant",
        json!({"valueType": "Boolean"}),
    );
    let document = Document::parse(&xml).unwrap();
    let properties = test_meta_root_properties(&document);
    let names = test_meta_direct_child_names(properties);
    let lock_mode = names
        .iter()
        .position(|name| name == "DataLockControlMode")
        .unwrap();
    let data_history = names.iter().position(|name| name == "DataHistory").unwrap();

    assert_eq!(lock_mode + 1, data_history, "{xml}");
}

#[test]
fn unbounded_tabular_sections_omit_line_number_length() {
    for object_type in ["DataProcessor", "Report"] {
        let xml = test_compile_meta_xml(
            object_type,
            &format!("Corpus{object_type}"),
            json!({"tabularSections": {"Rows": ["Value:String(100)"]}}),
        );
        let document = Document::parse(&xml).unwrap();
        let section = test_meta_named_object(&document, "TabularSection", "Rows");
        let properties = meta_info_child(section, "Properties").unwrap();

        assert!(
            meta_info_child(properties, "LineNumberLength").is_none(),
            "{object_type}: {xml}"
        );
    }
}

#[test]
fn document_journal_includes_help_before_standard_attributes() {
    let xml = test_compile_meta_xml(
        "DocumentJournal",
        "CorpusDocumentJournal",
        json!({"registeredDocuments": ["Document.CorpusDocument"]}),
    );
    let document = Document::parse(&xml).unwrap();
    let properties = test_meta_root_properties(&document);
    let names = test_meta_direct_child_names(properties);
    let registered = names
        .iter()
        .position(|name| name == "RegisteredDocuments")
        .unwrap();
    let include_help = names
        .iter()
        .position(|name| name == "IncludeHelpInContents")
        .unwrap();
    let standard = names
        .iter()
        .position(|name| name == "StandardAttributes")
        .unwrap();

    assert_eq!(include_help, registered + 1, "{xml}");
    assert_eq!(standard, include_help + 1, "{xml}");
}

#[test]
fn http_service_child_properties_include_comments() {
    let xml = test_compile_meta_xml(
        "HTTPService",
        "CorpusHTTPService",
        json!({
            "urlTemplates": {
                "Items": {"template": "/items/{id}", "methods": {"Get": "GET"}}
            }
        }),
    );
    let document = Document::parse(&xml).unwrap();
    let template = test_meta_named_object(&document, "URLTemplate", "Items");
    let method = test_meta_named_object(&document, "Method", "Get");

    assert_eq!(
        test_meta_direct_child_names(meta_info_child(template, "Properties").unwrap()),
        ["Name", "Synonym", "Comment", "Template"]
    );
    assert_eq!(
        test_meta_direct_child_names(meta_info_child(method, "Properties").unwrap()),
        ["Name", "Synonym", "Comment", "HTTPMethod", "Handler"]
    );
}

#[test]
fn information_register_dimension_emits_type_reduction_mode() {
    let xml = test_compile_meta_xml(
        "InformationRegister",
        "CorpusInformationRegister",
        json!({"dimensions": ["Item:String(50)|master,index"]}),
    );
    let document = Document::parse(&xml).unwrap();
    let dimension = test_meta_named_object(&document, "Dimension", "Item");
    let properties = meta_info_child(dimension, "Properties").unwrap();
    let children = properties
        .children()
        .filter(roxmltree::Node::is_element)
        .collect::<Vec<_>>();
    let last = children.last().unwrap();

    assert_eq!(last.tag_name().name(), "TypeReductionMode", "{xml}");
    assert_eq!(last.text(), Some("TransformValues"), "{xml}");
}

#[test]
fn web_service_emits_platform_defaults_and_parameter_comment() {
    let xml = test_compile_meta_xml(
        "WebService",
        "CorpusWebService",
        json!({
            "namespace": "urn:corpus",
            "operations": {
                "Ping": {"returnType": "xs:string", "parameters": {"Text": "xs:string"}}
            }
        }),
    );
    let document = Document::parse(&xml).unwrap();
    let properties = test_meta_root_properties(&document);
    let operation = test_meta_named_object(&document, "Operation", "Ping");
    let parameter = test_meta_named_object(&document, "Parameter", "Text");

    assert_eq!(
        meta_info_child_text(properties, "DescriptorFileName").as_deref(),
        Some("ws1.1cws")
    );
    assert_eq!(
        meta_info_child_text(
            meta_info_child(operation, "Properties").unwrap(),
            "DataLockControlMode"
        )
        .as_deref(),
        Some("Managed")
    );
    assert_eq!(
        test_meta_direct_child_names(meta_info_child(parameter, "Properties").unwrap()),
        [
            "Name",
            "Synonym",
            "Comment",
            "XDTOValueType",
            "Nillable",
            "TransferDirection",
        ]
    );
}

#[test]
fn value_type_unions_reject_duplicate_wire_types() {
    for (object_type, value_types) in [
        (
            "DefinedType",
            json!(["String(50)", "String(20)", "Number(15,2)"]),
        ),
        ("ChartOfCharacteristicTypes", json!(["Date", "DateTime"])),
    ] {
        let definition = json!({"valueTypes": value_types});
        let error = meta_compile_object_xml(
            definition.as_object().unwrap(),
            object_type,
            "CorpusType",
            "2.20",
        )
        .unwrap_err();

        assert!(error.contains("duplicate platform type"), "{error}");
    }
}

#[test]
fn nested_value_type_unions_reject_duplicate_wire_types() {
    for definition in [
        json!({"attributes": ["Value: String(50) + String(20)"]}),
        json!({"resources": ["Value: Date + DateTime"]}),
        json!({
            "tabularSections": {
                "Lines": ["Value: Number(15,2) + Number(10,0)"]
            }
        }),
    ] {
        let error = meta_compile_object_xml(
            definition.as_object().unwrap(),
            if definition.get("resources").is_some() {
                "InformationRegister"
            } else {
                "Catalog"
            },
            "CorpusObject",
            "2.20",
        )
        .unwrap_err();

        assert!(error.contains("duplicate platform type"), "{error}");
    }
}

#[test]
fn value_type_parameters_must_follow_the_8_3_27_contract() {
    for value_type in [
        "String(foo)",
        "String(10,20)",
        "String(-1)",
        "String(1025)",
        "String(1.5)",
        "Number(x,2)",
        "Number(15,y)",
        "Number(15,2,wrong)",
        "Number(15,2,nonneg,extra)",
        "Number(-1,0)",
        "Number(39,0)",
        "Number(10,11)",
        "Number(10,-1)",
        "Number(10,1.5)",
        "Garbage",
        "xs:string",
        "v8:UUID",
        "v8:ValueStorage",
        "CatalogRef.Bad Name",
        "CatalogRef.Bad:Name",
        "CatalogRef.Bad.Name",
    ] {
        let definition = json!({"attributes": [format!("Value: {value_type}")]});
        let error = meta_compile_object_xml(
            definition.as_object().unwrap(),
            "Catalog",
            "CorpusCatalog",
            "2.20",
        )
        .unwrap_err();

        assert!(error.contains(value_type), "{value_type}: {error}");
        assert!(error.contains("8.3.27"), "{value_type}: {error}");
    }
}

#[test]
fn value_type_parameter_boundaries_match_8_3_27() {
    for value_type in [
        "String(0)",
        "String(1024)",
        "Number(0,0)",
        "Number(38,0)",
        "Number(38,38)",
        "Number(38,38,nonneg)",
    ] {
        let definition = json!({"attributes": [format!("Value: {value_type}")]});
        meta_compile_object_xml(
            definition.as_object().unwrap(),
            "Catalog",
            "CorpusCatalog",
            "2.20",
        )
        .unwrap_or_else(|error| panic!("{value_type}: {error}"));
    }
}

#[test]
fn configuration_type_names_accept_unicode_xml_ncnames() {
    validate_meta_type_union(["CatalogRef.Контрагенты_1"]).unwrap();
    let mut lines = Vec::new();
    emit_meta_type_content(&mut lines, "", "CatalogRef.Контрагенты_1");
    assert_eq!(lines, ["<v8:Type>cfg:CatalogRef.Контрагенты_1</v8:Type>"]);
}

#[test]
fn value_type_tags_follow_the_8_3_27_canonical_order() {
    let mut lines = Vec::new();

    emit_meta_type_contents(
        &mut lines,
        "",
        [
            "Number(15,2)",
            "DateTime",
            "String(50)",
            "Boolean",
            "DefinedType.CorpusDefinedType",
            "CatalogRef.CorpusCatalog",
        ],
    );

    assert_eq!(
        &lines[..6],
        [
            "<v8:Type>cfg:CatalogRef.CorpusCatalog</v8:Type>",
            "<v8:Type>xs:boolean</v8:Type>",
            "<v8:Type>xs:string</v8:Type>",
            "<v8:Type>xs:dateTime</v8:Type>",
            "<v8:Type>xs:decimal</v8:Type>",
            "<v8:TypeSet>cfg:DefinedType.CorpusDefinedType</v8:TypeSet>",
        ]
    );
    assert!(lines[6].starts_with("<v8:NumberQualifiers>"), "{lines:?}");
    assert!(lines[11].starts_with("<v8:StringQualifiers>"), "{lines:?}");
    assert!(lines[15].starts_with("<v8:DateQualifiers>"), "{lines:?}");
}

#[test]
fn event_subscription_sources_form_one_8_3_27_type_description() {
    let xml = test_compile_meta_xml(
        "EventSubscription",
        "CorpusSubscription",
        json!({
            "source": ["String(10)", "DocumentObject.CorpusDocument"],
            "event": "BeforeWrite",
            "handler": "CorpusModule.Handle"
        }),
    );
    let document = Document::parse(&xml).unwrap();
    let source = meta_info_child(test_meta_root_properties(&document), "Source").unwrap();

    assert_eq!(
        test_meta_direct_child_names(source),
        ["Type", "Type", "StringQualifiers"],
        "{xml}"
    );
}

#[test]
fn event_subscription_source_string_is_unbounded_in_8_3_27() {
    let xml = test_compile_meta_xml(
        "EventSubscription",
        "CorpusSubscription",
        json!({
            "source": [
                "DocumentObject.CorpusDocument",
                "String(37)",
                "CatalogObject.CorpusCatalog"
            ],
            "event": "BeforeWrite",
            "handler": "CorpusModule.Handle"
        }),
    );
    let document = Document::parse(&xml).unwrap();
    let source = meta_info_child(test_meta_root_properties(&document), "Source").unwrap();
    let mut source_types = meta_info_children(source, "Type")
        .into_iter()
        .map(meta_info_inner_text)
        .collect::<Vec<_>>();
    source_types.sort();

    assert_eq!(
        source_types,
        [
            "cfg:CatalogObject.CorpusCatalog",
            "cfg:DocumentObject.CorpusDocument",
            "xs:string",
        ],
        "{xml}"
    );
    let qualifiers = meta_info_child(source, "StringQualifiers").unwrap();
    assert_eq!(
        meta_info_child_text(qualifiers, "Length").as_deref(),
        Some("0"),
        "{xml}"
    );
    assert_eq!(
        meta_info_child_text(qualifiers, "AllowedLength").as_deref(),
        Some("Variable"),
        "{xml}"
    );
}

#[test]
fn event_subscription_sources_validate_as_one_union() {
    for (sources, expected) in [
        (vec!["String(10)", "String(20)"], "duplicate platform type"),
        (
            vec!["ValueStorage", "DocumentObject.CorpusDocument"],
            "only platform type",
        ),
    ] {
        let definition = json!({
            "source": sources,
            "event": "BeforeWrite",
            "handler": "CorpusModule.Handle"
        });
        let error = meta_compile_object_xml(
            definition.as_object().unwrap(),
            "EventSubscription",
            "CorpusSubscription",
            "2.20",
        )
        .unwrap_err();

        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn configuration_type_order_is_not_faked_without_workspace_type_ids() {
    let mut lines = Vec::new();

    emit_meta_type_contents(
        &mut lines,
        "",
        [
            "DocumentRef.CorpusDocument",
            "CatalogRef.CorpusCatalog",
            "Boolean",
        ],
    );

    assert_eq!(
        &lines[..3],
        [
            "<v8:Type>cfg:DocumentRef.CorpusDocument</v8:Type>",
            "<v8:Type>cfg:CatalogRef.CorpusCatalog</v8:Type>",
            "<v8:Type>xs:boolean</v8:Type>",
        ]
    );
}

#[test]
fn value_storage_uses_the_8_3_27_core_type() {
    for alias in ["ValueStorage", "valuestorage", "ХранилищеЗначения"] {
        validate_meta_type_union([alias]).unwrap();
        let mut lines = Vec::new();
        emit_meta_type_content(&mut lines, "", alias);
        assert_eq!(lines, ["<v8:Type>v8:ValueStorage</v8:Type>"]);
    }

    let error = validate_meta_type_union(["Boolean", "ValueStorage"]).unwrap_err();
    assert!(error.contains("only platform type"), "{error}");
}

#[test]
fn defined_type_references_use_type_set_after_concrete_types() {
    let mut lines = Vec::new();

    emit_meta_type_contents(
        &mut lines,
        "",
        ["DefinedType.CorpusDefinedType", "String(50)"],
    );

    assert_eq!(lines[0], "<v8:Type>xs:string</v8:Type>");
    assert_eq!(
        lines[1],
        "<v8:TypeSet>cfg:DefinedType.CorpusDefinedType</v8:TypeSet>"
    );
    assert!(lines[2].starts_with("<v8:StringQualifiers>"), "{lines:?}");
}

#[test]
fn chart_of_accounts_rejects_positive_extra_dimension_count_without_type() {
    for ext_dimension_types in [None, Some(""), Some("   ")] {
        let mut definition = json!({"maxExtDimensionCount": 3});
        if let Some(value) = ext_dimension_types {
            definition["extDimensionTypes"] = json!(value);
        }
        let error = meta_compile_object_xml(
            definition.as_object().unwrap(),
            "ChartOfAccounts",
            "CorpusAccounts",
            "2.20",
        )
        .unwrap_err();

        assert!(error.contains("extDimensionTypes"), "{error}");
        assert!(error.contains("maxExtDimensionCount"), "{error}");
    }
}

#[test]
fn meta_compile_rejects_invalid_object_and_child_names_before_emission() {
    let cases = [
        ("metadata object", "Catalog", "../EscapedName", json!({})),
        (
            "attributes",
            "Catalog",
            "ValidCatalog",
            json!({"attributes": [{"name": "Bad Name", "type": "String"}]}),
        ),
        (
            "tabularSections",
            "Document",
            "ValidDocument",
            json!({"tabularSections": [{"name": "Bad/Section", "attributes": []}]}),
        ),
        (
            "enum value",
            "Enum",
            "ValidEnum",
            json!({"values": ["Bad Value"]}),
        ),
        (
            "URL template",
            "HTTPService",
            "ValidService",
            json!({"urlTemplates": {"Bad/Template": "/probe"}}),
        ),
        (
            "operation parameter",
            "WebService",
            "ValidWebService",
            json!({
                "operations": {
                    "ValidOperation": {"parameters": {"Bad Parameter": "xs:string"}}
                }
            }),
        ),
    ];

    for (context, object_type, object_name, definition) in cases {
        let error = meta_compile_object_xml(
            definition.as_object().unwrap(),
            object_type,
            object_name,
            "2.20",
        )
        .unwrap_err();

        assert!(error.contains(context), "{context}: {error}");
        assert!(error.contains("valid 1C identifier"), "{context}: {error}");
    }
}

#[test]
fn meta_compile_rejects_invalid_8_3_27_enum_before_emission() {
    let definition = json!({"hierarchyType": "Bogus"});

    let error = meta_compile_object_xml(
        definition.as_object().unwrap(),
        "Catalog",
        "ValidCatalog",
        "2.20",
    )
    .unwrap_err();

    assert!(error.contains("HierarchyType"), "{error}");
    assert!(error.contains("Bogus"), "{error}");
    assert!(error.contains("8.3.27"), "{error}");
}

fn test_compile_meta_xml(object_type: &str, object_name: &str, definition: Value) -> String {
    meta_compile_object_xml(
        definition.as_object().unwrap(),
        object_type,
        object_name,
        "2.20",
    )
    .unwrap()
    .0
}

fn test_meta_root_properties<'a, 'input>(
    document: &'a Document<'input>,
) -> roxmltree::Node<'a, 'input> {
    let object = document
        .root_element()
        .children()
        .find(roxmltree::Node::is_element)
        .unwrap();
    meta_info_child(object, "Properties").unwrap()
}

fn test_meta_direct_child_names(node: roxmltree::Node<'_, '_>) -> Vec<String> {
    node.children()
        .filter(roxmltree::Node::is_element)
        .map(|child| child.tag_name().name().to_string())
        .collect()
}

fn test_meta_standard_attribute_names(properties: roxmltree::Node<'_, '_>) -> Vec<String> {
    meta_info_child(properties, "StandardAttributes")
        .unwrap()
        .children()
        .filter(roxmltree::Node::is_element)
        .map(|child| child.attribute("name").unwrap_or_default().to_string())
        .collect()
}

fn test_meta_root_child_objects<'a, 'input>(
    document: &'a Document<'input>,
) -> roxmltree::Node<'a, 'input> {
    let object = document
        .root_element()
        .children()
        .find(roxmltree::Node::is_element)
        .unwrap();
    meta_info_child(object, "ChildObjects").unwrap()
}

fn test_meta_named_object<'a, 'input>(
    document: &'a Document<'input>,
    object_type: &str,
    name: &str,
) -> roxmltree::Node<'a, 'input> {
    document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == object_type)
        .find(|node| {
            meta_info_child(*node, "Properties")
                .and_then(|properties| meta_info_child_text(properties, "Name"))
                .as_deref()
                == Some(name)
        })
        .unwrap_or_else(|| panic!("{object_type} {name} not found"))
}
