use super::*;
fn fixture_uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}
#[test]
fn cfe_borrow_report_emits_required_empty_child_objects() {
    let xml = cfe_borrow_object_xml(
        "Report",
        "PriceList",
        "11111111-1111-1111-1111-111111111111",
        None,
        "2.20",
        &CfeBorrowIdentity::default(),
        fixture_uuid_v4,
    )
    .unwrap();

    let generated = Document::parse(&xml).unwrap();
    let report = generated
        .descendants()
        .find(|node| node.has_tag_name((CFE_PATCH_MD_NAMESPACE, "Report")))
        .unwrap();
    assert!(
        unica_xml_format::read::child_element(report, "ChildObjects").is_some(),
        "8.3.27 requires ChildObjects even for a report without forms or templates: {xml}"
    );
}

#[test]
fn cfe_borrow_emits_child_objects_for_every_proven_platform_kind() {
    for type_name in [
        "Subsystem",
        "FilterCriterion",
        "Sequence",
        "SettingsStorage",
        "DocumentJournal",
        "HTTPService",
        "WebService",
        "IntegrationService",
    ] {
        let xml = cfe_borrow_object_xml(
            type_name,
            "Evidence",
            "11111111-1111-1111-1111-111111111111",
            None,
            "2.20",
            &CfeBorrowIdentity::default(),
            fixture_uuid_v4,
        )
        .unwrap();

        let generated = Document::parse(&xml).unwrap();
        let object = generated
            .root_element()
            .children()
            .find(roxmltree::Node::is_element)
            .unwrap();
        assert!(
            object
                .children()
                .any(|node| { node.has_tag_name((CFE_PATCH_MD_NAMESPACE, "ChildObjects")) }),
            "{type_name} must follow the proven 8.3.27 ChildObjects contract: {xml}"
        );
    }
}

#[test]
fn cfe_borrow_has_complete_8_3_27_generated_type_profiles() {
    let profiles: &[(&str, &[(&str, &str)])] = &[
        (
            "Catalog",
            &[
                ("CatalogObject", "Object"),
                ("CatalogRef", "Ref"),
                ("CatalogSelection", "Selection"),
                ("CatalogList", "List"),
                ("CatalogManager", "Manager"),
            ],
        ),
        (
            "Document",
            &[
                ("DocumentObject", "Object"),
                ("DocumentRef", "Ref"),
                ("DocumentSelection", "Selection"),
                ("DocumentList", "List"),
                ("DocumentManager", "Manager"),
            ],
        ),
        (
            "Enum",
            &[
                ("EnumRef", "Ref"),
                ("EnumManager", "Manager"),
                ("EnumList", "List"),
            ],
        ),
        (
            "Constant",
            &[
                ("ConstantManager", "Manager"),
                ("ConstantValueManager", "ValueManager"),
                ("ConstantValueKey", "ValueKey"),
            ],
        ),
        (
            "InformationRegister",
            &[
                ("InformationRegisterRecord", "Record"),
                ("InformationRegisterManager", "Manager"),
                ("InformationRegisterSelection", "Selection"),
                ("InformationRegisterList", "List"),
                ("InformationRegisterRecordSet", "RecordSet"),
                ("InformationRegisterRecordKey", "RecordKey"),
                ("InformationRegisterRecordManager", "RecordManager"),
            ],
        ),
        (
            "AccumulationRegister",
            &[
                ("AccumulationRegisterRecord", "Record"),
                ("AccumulationRegisterManager", "Manager"),
                ("AccumulationRegisterSelection", "Selection"),
                ("AccumulationRegisterList", "List"),
                ("AccumulationRegisterRecordSet", "RecordSet"),
                ("AccumulationRegisterRecordKey", "RecordKey"),
            ],
        ),
        (
            "AccountingRegister",
            &[
                ("AccountingRegisterRecord", "Record"),
                ("AccountingRegisterExtDimensions", "ExtDimensions"),
                ("AccountingRegisterRecordSet", "RecordSet"),
                ("AccountingRegisterRecordKey", "RecordKey"),
                ("AccountingRegisterSelection", "Selection"),
                ("AccountingRegisterList", "List"),
                ("AccountingRegisterManager", "Manager"),
            ],
        ),
        (
            "CalculationRegister",
            &[
                ("CalculationRegisterRecord", "Record"),
                ("CalculationRegisterManager", "Manager"),
                ("CalculationRegisterSelection", "Selection"),
                ("CalculationRegisterList", "List"),
                ("CalculationRegisterRecordSet", "RecordSet"),
                ("CalculationRegisterRecordKey", "RecordKey"),
                ("RecalculationsManager", "Recalcs"),
            ],
        ),
        (
            "ChartOfAccounts",
            &[
                ("ChartOfAccountsObject", "Object"),
                ("ChartOfAccountsRef", "Ref"),
                ("ChartOfAccountsSelection", "Selection"),
                ("ChartOfAccountsList", "List"),
                ("ChartOfAccountsManager", "Manager"),
                ("ChartOfAccountsExtDimensionTypes", "ExtDimensionTypes"),
                (
                    "ChartOfAccountsExtDimensionTypesRow",
                    "ExtDimensionTypesRow",
                ),
            ],
        ),
        (
            "ChartOfCharacteristicTypes",
            &[
                ("ChartOfCharacteristicTypesObject", "Object"),
                ("ChartOfCharacteristicTypesRef", "Ref"),
                ("ChartOfCharacteristicTypesSelection", "Selection"),
                ("ChartOfCharacteristicTypesList", "List"),
                ("Characteristic", "Characteristic"),
                ("ChartOfCharacteristicTypesManager", "Manager"),
            ],
        ),
        (
            "ChartOfCalculationTypes",
            &[
                ("ChartOfCalculationTypesObject", "Object"),
                ("ChartOfCalculationTypesRef", "Ref"),
                ("ChartOfCalculationTypesSelection", "Selection"),
                ("ChartOfCalculationTypesList", "List"),
                ("ChartOfCalculationTypesManager", "Manager"),
                ("DisplacingCalculationTypes", "DisplacingCalculationTypes"),
                (
                    "DisplacingCalculationTypesRow",
                    "DisplacingCalculationTypesRow",
                ),
                ("BaseCalculationTypes", "BaseCalculationTypes"),
                ("BaseCalculationTypesRow", "BaseCalculationTypesRow"),
                ("LeadingCalculationTypes", "LeadingCalculationTypes"),
                ("LeadingCalculationTypesRow", "LeadingCalculationTypesRow"),
            ],
        ),
        (
            "BusinessProcess",
            &[
                ("BusinessProcessObject", "Object"),
                ("BusinessProcessRef", "Ref"),
                ("BusinessProcessSelection", "Selection"),
                ("BusinessProcessList", "List"),
                ("BusinessProcessManager", "Manager"),
                ("BusinessProcessRoutePointRef", "RoutePointRef"),
            ],
        ),
        (
            "Task",
            &[
                ("TaskObject", "Object"),
                ("TaskRef", "Ref"),
                ("TaskSelection", "Selection"),
                ("TaskList", "List"),
                ("TaskManager", "Manager"),
            ],
        ),
        (
            "ExchangePlan",
            &[
                ("ExchangePlanObject", "Object"),
                ("ExchangePlanRef", "Ref"),
                ("ExchangePlanSelection", "Selection"),
                ("ExchangePlanList", "List"),
                ("ExchangePlanManager", "Manager"),
            ],
        ),
        (
            "DocumentJournal",
            &[
                ("DocumentJournalSelection", "Selection"),
                ("DocumentJournalList", "List"),
                ("DocumentJournalManager", "Manager"),
            ],
        ),
        (
            "Report",
            &[("ReportObject", "Object"), ("ReportManager", "Manager")],
        ),
        (
            "DataProcessor",
            &[
                ("DataProcessorObject", "Object"),
                ("DataProcessorManager", "Manager"),
            ],
        ),
        ("DefinedType", &[("DefinedType", "DefinedType")]),
    ];

    for (object_type, expected) in profiles {
        let shared = crate::template_catalog::metadata_generated_types_8_3_27(object_type).unwrap();
        let borrowed = cfe_borrow_generated_types(object_type).unwrap();
        assert_eq!(borrowed, *expected, "{object_type}");
        assert!(
            std::ptr::eq(borrowed, shared),
            "{object_type} must use the shared 8.3.27 GeneratedType profile"
        );

        let borrowed_xml = cfe_borrow_internal_info_xml(
            object_type,
            "SharedContract",
            "\t\t",
            &CfeBorrowIdentity::default(),
            fixture_uuid_v4,
        )
        .unwrap();
        let mut meta_lines = Vec::new();
        let mut next = || "11111111-1111-1111-1111-111111111111".to_string();
        crate::template_catalog::emit_meta_internal_info(
            &mut meta_lines,
            "\t\t",
            object_type,
            "SharedContract",
            &mut next,
        );
        let meta_xml = meta_lines.join("\n");
        for (prefix, category) in *expected {
            let generated = format!("name=\"{prefix}.SharedContract\" category=\"{category}\"");
            assert!(
                borrowed_xml.contains(&generated),
                "cfe.borrow lost {object_type} {generated}: {borrowed_xml}"
            );
            assert!(
                meta_xml.contains(&generated),
                "meta.compile lost {object_type} {generated}: {meta_xml}"
            );
        }
        assert_eq!(
            borrowed_xml.matches("<xr:GeneratedType ").count(),
            expected.len(),
            "cfe.borrow emitted extra GeneratedType entries for {object_type}"
        );
        assert_eq!(
            meta_xml.matches("<xr:GeneratedType ").count(),
            expected.len(),
            "meta.compile emitted extra GeneratedType entries for {object_type}"
        );
    }
    assert_eq!(
        crate::template_catalog::metadata_generated_types_8_3_27("CommonModule"),
        Some(&[][..])
    );
    assert_eq!(cfe_borrow_generated_types("CommonModule"), Some(&[][..]));

    let exchange = cfe_borrow_internal_info_xml(
        "ExchangePlan",
        "CorpusExchangePlan",
        "\t\t",
        &CfeBorrowIdentity::default(),
        fixture_uuid_v4,
    )
    .unwrap();
    assert!(
        exchange.contains("<xr:ThisNode>"),
        "ExchangePlan requires its own internal node id: {exchange}"
    );
    let mut exchange_meta = Vec::new();
    let mut next = || "11111111-1111-1111-1111-111111111111".to_string();
    crate::template_catalog::emit_meta_internal_info(
        &mut exchange_meta,
        "\t\t",
        "ExchangePlan",
        "CorpusExchangePlan",
        &mut next,
    );
    assert_eq!(exchange.matches("<xr:ThisNode>").count(), 1);
    assert_eq!(exchange_meta.join("\n").matches("<xr:ThisNode>").count(), 1);
}

#[test]
fn cfe_borrow_generated_type_profile_is_total_for_the_metadata_registry() {
    let missing = cf_validate_child_object_types()
        .iter()
        .copied()
        .filter(|object_type| cfe_borrow_generated_types(object_type).is_none())
        .collect::<Vec<_>>();

    assert!(
        missing.is_empty(),
        "cfe.borrow must fail closed instead of using an implicit empty InternalInfo profile: {missing:?}"
    );
    assert_eq!(cfe_borrow_generated_types("UnknownMetadataType"), None);
    let error = cfe_borrow_internal_info_xml(
        "UnknownMetadataType",
        "Unknown",
        "\t\t",
        &CfeBorrowIdentity::default(),
        fixture_uuid_v4,
    )
    .unwrap_err();
    assert!(error.contains("no proven cfe.borrow InternalInfo profile"));
    assert_eq!(
        cfe_borrow_internal_info_xml(
            "CommonModule",
            "KnownEmpty",
            "\t\t",
            &CfeBorrowIdentity::default(),
            fixture_uuid_v4
        )
        .unwrap(),
        "\t\t<InternalInfo/>"
    );
}

#[test]
fn cfe_borrow_generated_type_profiles_cover_8_3_27_dynamic_families() {
    let profiles: &[(&str, &[(&str, &str)])] = &[
        (
            "FilterCriterion",
            &[
                ("FilterCriterionManager", "Manager"),
                ("FilterCriterionList", "List"),
            ],
        ),
        ("SettingsStorage", &[("SettingsStorageManager", "Manager")]),
        ("Sequence", &[("SequenceRecordSet", "RecordSet")]),
        (
            "IntegrationService",
            &[("IntegrationServiceManager", "Manager")],
        ),
    ];

    for (object_type, expected) in profiles {
        assert_eq!(cfe_borrow_generated_types(object_type), Some(*expected));
        let borrowed_xml = cfe_borrow_internal_info_xml(
            object_type,
            "Evidence",
            "\t\t",
            &CfeBorrowIdentity::default(),
            fixture_uuid_v4,
        )
        .unwrap();
        let mut meta_lines = Vec::new();
        let mut next = || "11111111-1111-1111-1111-111111111111".to_string();
        crate::template_catalog::emit_meta_internal_info(
            &mut meta_lines,
            "\t\t",
            object_type,
            "Evidence",
            &mut next,
        );
        let meta_xml = meta_lines.join("\n");
        for (prefix, category) in *expected {
            let fragment = format!("name=\"{prefix}.Evidence\" category=\"{category}\"");
            assert!(borrowed_xml.contains(&fragment), "{borrowed_xml}");
            assert!(meta_xml.contains(&fragment), "{meta_xml}");
        }
    }
}

#[test]
fn cfe_borrow_parse_rejects_unsafe_object_and_form_names_before_path_use() {
    for value in [
        "Catalog.../Outside",
        "Catalog.Items.Form.../Outside",
        "Catalog.Bad:Name",
        "Catalog.Items.Form.Bad:Name",
        "Catalog.",
        "Catalog.Items.Form.",
    ] {
        let Err(error) = cfe_borrow_parse_object_spec(value) else {
            panic!("unsafe metadata name must be rejected: {value}");
        };
        assert!(error.contains("XML NCName"), "{value}: {error}");
        assert!(error.contains("path component"), "{value}: {error}");
    }

    let parsed = cfe_borrow_parse_object_spec("Catalog.Товары.Form.ФормаЭлемента")
        .expect("Unicode metadata names must remain supported");
    assert_eq!(parsed.object_name, "Товары");
    assert_eq!(parsed.form_name.as_deref(), Some("ФормаЭлемента"));
}

#[test]
fn cfe_borrow_form_metadata_marks_the_form_property_as_extended() {
    let generated = cfe_borrow_form_metadata_xml(
        "MainForm",
        "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
        "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
        "2.20",
    );
    let document = Document::parse(&generated).unwrap();
    let internal_info = document
        .descendants()
        .find(|node| node.has_tag_name("InternalInfo"))
        .expect("borrowed form metadata must contain InternalInfo");
    let property_states = internal_info
        .children()
        .filter(|node| node.has_tag_name(("http://v8.1c.ru/8.3/xcf/readable", "PropertyState")))
        .collect::<Vec<_>>();

    assert_eq!(property_states.len(), 1, "{generated}");
    let property_state = property_states[0];
    assert_eq!(
        property_state
            .children()
            .find(|node| { node.has_tag_name(("http://v8.1c.ru/8.3/xcf/readable", "Property")) })
            .and_then(|node| node.text()),
        Some("Form"),
        "{generated}"
    );
    assert_eq!(
        property_state
            .children()
            .find(|node| { node.has_tag_name(("http://v8.1c.ru/8.3/xcf/readable", "State")) })
            .and_then(|node| node.text()),
        Some("Extended"),
        "{generated}"
    );
}

#[test]
fn seeded_borrow_uuid_is_custom_and_separates_seed_role_and_name() {
    let identity = CfeBorrowIdentity::for_plan([1; 32]);
    let value = identity.new_uuid("wrapper", "Demo", fixture_uuid_v4);
    let parsed = uuid::Uuid::parse_str(&value).unwrap();
    assert_eq!(parsed.get_version_num(), 8);
    assert_eq!(value, identity.new_uuid("wrapper", "Demo", fixture_uuid_v4));
    assert_ne!(value, identity.new_uuid("node", "Demo", fixture_uuid_v4));
    assert_ne!(
        value,
        identity.new_uuid("wrapper", "Other", fixture_uuid_v4)
    );
    assert_ne!(
        value,
        CfeBorrowIdentity::for_plan([2; 32]).new_uuid("wrapper", "Demo", fixture_uuid_v4)
    );
}

#[test]
fn cfe_patch_method_8_3_27_grammar_matrix_has_exactly_51_roles() {
    let object_types = [
        "Catalog",
        "Document",
        "ExchangePlan",
        "ChartOfAccounts",
        "ChartOfCharacteristicTypes",
        "ChartOfCalculationTypes",
        "BusinessProcess",
        "Task",
        "Report",
        "DataProcessor",
    ];
    let manager_types = [
        "Catalog",
        "Document",
        "ExchangePlan",
        "ChartOfAccounts",
        "ChartOfCharacteristicTypes",
        "ChartOfCalculationTypes",
        "BusinessProcess",
        "Task",
        "Report",
        "DataProcessor",
        "Enum",
        "InformationRegister",
        "AccumulationRegister",
        "AccountingRegister",
        "CalculationRegister",
        "Constant",
        "DocumentJournal",
        "FilterCriterion",
    ];
    let record_set_types = [
        "InformationRegister",
        "AccumulationRegister",
        "AccountingRegister",
        "CalculationRegister",
    ];
    let form_types = [
        "Catalog",
        "Document",
        "Enum",
        "Report",
        "DataProcessor",
        "ExchangePlan",
        "ChartOfAccounts",
        "ChartOfCharacteristicTypes",
        "ChartOfCalculationTypes",
        "BusinessProcess",
        "Task",
        "InformationRegister",
        "AccumulationRegister",
        "AccountingRegister",
        "CalculationRegister",
        "DocumentJournal",
        "FilterCriterion",
    ];
    let extension = Path::new("/extension");
    assert!(
        cfe_patch_module_target(extension, "CommonModule.Items").is_ok(),
        "CommonModule role"
    );
    for type_name in object_types {
        assert!(
            cfe_patch_module_target(extension, &format!("{type_name}.Items.ObjectModule")).is_ok(),
            "{type_name}.ObjectModule"
        );
    }
    for type_name in manager_types {
        assert!(
            cfe_patch_module_target(extension, &format!("{type_name}.Items.ManagerModule")).is_ok(),
            "{type_name}.ManagerModule"
        );
    }
    for type_name in record_set_types {
        assert!(
            cfe_patch_module_target(extension, &format!("{type_name}.Items.RecordSetModule"))
                .is_ok(),
            "{type_name}.RecordSetModule"
        );
    }
    assert!(
        cfe_patch_module_target(extension, "Constant.Items.ValueManagerModule").is_ok(),
        "Constant.ValueManagerModule"
    );
    for type_name in form_types {
        assert!(
            cfe_patch_module_target(extension, &format!("{type_name}.Items.Form.Main")).is_ok(),
            "{type_name}.Form"
        );
    }

    let mut accepted = 1usize;
    for type_name in cf_validate_child_object_types() {
        for role in [
            "ObjectModule",
            "ManagerModule",
            "RecordSetModule",
            "ValueManagerModule",
        ] {
            accepted += cfe_patch_module_target(extension, &format!("{type_name}.Items.{role}"))
                .is_ok() as usize;
        }
        accepted += cfe_patch_module_target(extension, &format!("{type_name}.Items.Form.Main"))
            .is_ok() as usize;
    }
    assert_eq!(accepted, 51, "8.3.27 cfe.patch_method grammar matrix");
}

/// Pure image-generation evidence; role absence is not a publication no-op.
/// rule: INV.SOURCE.WRITABLE-PROFILE
#[test]
fn extension_init_images_round_trip_with_base_language_and_optional_default_role() {
    let args = Map::from_iter([("vendor".to_string(), serde_json::json!("Synthetic & Co"))]);
    let base_language_uuid = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    for no_role in [false, true] {
        let images = cfe_init_images(CfeInitImageInput {
            args: &args,
            name: "SyntheticExtension",
            synonym: "Synthetic <extension>",
            name_prefix: "Synthetic_",
            purpose: "Customization",
            compatibility: "Version8_3_27",
            interface_mode: "TaxiEnableVersion8_2",
            base_lang_uuid: base_language_uuid,
            role_name: "SyntheticAccess",
            no_role,
        });
        let info = parse_cf_info_xml(
            &images.configuration,
            unica_metadata::support_state::ConfigurationSupportData {
                state: unica_metadata::support_state::ConfigurationSupportState::Extension,
                editing_enabled: None,
                objects: None,
            },
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(info.data.name, "SyntheticExtension");
        assert_eq!(info.data.synonym.as_deref(), Some("Synthetic <extension>"));
        assert_eq!(info.data.vendor.as_deref(), Some("Synthetic & Co"));
        assert_eq!(
            info.data.extension_purpose.as_deref(),
            Some("Customization")
        );
        assert_eq!(
            info.data.properties.extension_compatibility_mode.as_deref(),
            Some("Version8_3_27")
        );
        assert_eq!(
            info.data.properties.name_prefix.as_deref(),
            Some("Synthetic_")
        );
        assert_eq!(info.data.format, "2.20");
        let configuration = Document::parse(&images.configuration).unwrap();
        let language = Document::parse(&images.language).unwrap();
        for document in [&configuration, &language] {
            let root = document.root_element();
            assert!(root.has_tag_name((CFE_BORROW_MD_NAMESPACE, "MetaDataObject")));
            assert_eq!(root.attribute("version"), Some("2.20"));
            assert!(matches!(
                classify_root_version(root.attribute("version")).unwrap(),
                FormatCompatibility::Supported { .. }
            ));
        }
        let cfg = configuration
            .root_element()
            .children()
            .find(|node| node.has_tag_name((CFE_BORROW_MD_NAMESPACE, "Configuration")))
            .unwrap();
        let lang = language
            .root_element()
            .children()
            .find(|node| node.has_tag_name((CFE_BORROW_MD_NAMESPACE, "Language")))
            .unwrap();
        let properties = lang
            .children()
            .find(|node| node.has_tag_name((CFE_BORROW_MD_NAMESPACE, "Properties")))
            .unwrap();
        let language_name = cf_prop_text(properties, "Name");
        assert_eq!(language_name, "Русский");
        assert_eq!(cf_prop_text(properties, "LanguageCode"), "ru");
        assert_eq!(
            cf_prop_text(properties, "ExtendedConfigurationObject"),
            base_language_uuid
        );
        assert_eq!(
            info.data.properties.default_language,
            Some(format!("Language.{language_name}"))
        );
        assert!(info
            .registered_objects
            .iter()
            .any(|object| object.kind == "Language" && object.name == language_name));
        let cfg_uuid = uuid::Uuid::parse_str(cfg.attribute("uuid").unwrap()).unwrap();
        let lang_uuid = uuid::Uuid::parse_str(lang.attribute("uuid").unwrap()).unwrap();
        assert_ne!(cfg_uuid, uuid::Uuid::nil());
        assert_ne!(lang_uuid, uuid::Uuid::nil());
        assert_ne!(cfg_uuid, lang_uuid);
        assert_ne!(lang_uuid.to_string(), base_language_uuid);
        let properties = cfg
            .children()
            .find(|node| node.has_tag_name((CFE_BORROW_MD_NAMESPACE, "Properties")))
            .unwrap();
        let default_roles = properties
            .children()
            .find(|node| node.has_tag_name((CFE_BORROW_MD_NAMESPACE, "DefaultRoles")))
            .unwrap();
        if no_role {
            assert!(images.role.is_none());
            assert!(info
                .registered_objects
                .iter()
                .all(|object| object.kind != "Role"));
            assert_eq!(
                default_roles
                    .children()
                    .filter(|node| node.is_element())
                    .count(),
                0
            );
            assert_eq!(info.data.total_objects, 1);
        } else {
            let role_image = images.role.as_deref().unwrap();
            let role = Document::parse(role_image).unwrap();
            assert!(role
                .root_element()
                .has_tag_name((CFE_BORROW_MD_NAMESPACE, "MetaDataObject")));
            assert_eq!(role.root_element().attribute("version"), Some("2.20"));
            let role_node = role
                .root_element()
                .children()
                .find(|node| node.has_tag_name((CFE_BORROW_MD_NAMESPACE, "Role")))
                .unwrap();
            let role_props = role_node
                .children()
                .find(|node| node.has_tag_name((CFE_BORROW_MD_NAMESPACE, "Properties")))
                .unwrap();
            let role_name = cf_prop_text(role_props, "Name");
            assert_eq!(role_name, "SyntheticAccess");
            let role_uuid = uuid::Uuid::parse_str(role_node.attribute("uuid").unwrap()).unwrap();
            assert_ne!(role_uuid, uuid::Uuid::nil());
            assert_ne!(role_uuid, cfg_uuid);
            assert_ne!(role_uuid, lang_uuid);
            assert!(info
                .registered_objects
                .iter()
                .any(|object| object.kind == "Role" && object.name == role_name));
            let refs = default_roles
                .children()
                .filter(|node| node.is_element())
                .collect::<Vec<_>>();
            assert_eq!(refs.len(), 1);
            assert!(refs[0].has_tag_name((CFE_PATCH_XR_NAMESPACE, "Item")));
            assert_eq!(refs[0].text(), Some(format!("Role.{role_name}").as_str()));
            assert_eq!(info.data.total_objects, 2);
        }
    }
}
