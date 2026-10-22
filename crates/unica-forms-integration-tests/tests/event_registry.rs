//! Existing real immutable catalogue and form event-context oracles.
mod tests {

    use unica_address::address::NodeKind;
    use unica_bsl::event_catalog::module_event_catalog_8_3_27;
    use unica_bsl::projection::MethodParameter;
    use unica_forms::events::test_support::{NO_EVENTS, OBJECT_RECORD_FORM_EVENTS};
    use unica_forms::events::*;
    use unica_forms_designer::event_context::context_from_root;
    use unica_metadata::event_catalog::*;

    use roxmltree::Document;

    const FORM_PREFIX: &str = r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"
        xmlns:v8="http://v8.1c.ru/8.1/data/core">"#;

    fn context(xml: &str) -> FormEventContext {
        let document = Document::parse(xml).unwrap();
        context_from_root(document.root_element())
    }

    fn regular_context(main_attribute: MainAttributeKind) -> FormEventContext {
        FormEventContext {
            definition: FormDefinitionKind::Regular,
            direct_part_writable: true,
            main_attribute,
            main_attribute_type: None,
            main_attribute_provenance: MainAttributeProvenance::Missing,
            main_attribute_name: None,
            metadata_owner: None,
        }
    }

    fn extension_context(main_attribute: MainAttributeKind) -> FormEventContext {
        FormEventContext {
            definition: FormDefinitionKind::Extension,
            direct_part_writable: true,
            main_attribute,
            main_attribute_type: None,
            main_attribute_provenance: MainAttributeProvenance::InheritedBaseFormUnavailable,
            main_attribute_name: None,
            metadata_owner: None,
        }
    }

    fn form_event_catalog_8_3_27(owner: FormEventOwnerKind) -> Vec<&'static PlatformEventSpec> {
        unica_forms::events::form_event_catalog_8_3_27(
            &regular_context(MainAttributeKind::Other),
            owner,
            None,
        )
    }

    fn projection_context(
        definition: FormDefinitionKind,
        main_attribute: MainAttributeKind,
        main_attribute_type: Option<&str>,
        metadata_owner: NodeKind,
    ) -> FormEventContext {
        FormEventContext {
            definition,
            direct_part_writable: true,
            main_attribute,
            main_attribute_type: main_attribute_type.map(str::to_string),
            main_attribute_provenance: MainAttributeProvenance::DirectForm,
            main_attribute_name: Some("Object".to_string()),
            metadata_owner: Some(metadata_owner),
        }
    }

    #[test]
    fn discovers_root_persistent_object_main_attribute() {
        let xml = format!(
            r#"{FORM_PREFIX}
                <Attributes>
                    <Attribute name="Object" id="1">
                        <Type><v8:Type>cfg:BusinessProcessObject.Task</v8:Type></Type>
                        <MainAttribute>true</MainAttribute>
                    </Attribute>
                </Attributes>
            </Form>"#
        );

        let actual = context(&xml);

        assert_eq!(actual.definition, FormDefinitionKind::Regular);
        assert_eq!(actual.main_attribute, MainAttributeKind::PersistentObject);
        assert_eq!(
            actual.main_attribute_provenance,
            MainAttributeProvenance::DirectForm
        );
        assert_eq!(
            actual.main_attribute_type.as_deref(),
            Some("cfg:BusinessProcessObject.Task")
        );
    }

    #[test]
    fn root_main_attribute_has_priority_over_base_form() {
        let xml = format!(
            r#"{FORM_PREFIX}
                <Attributes>
                    <Attribute name="List" id="1">
                        <Type><v8:Type>cfg:DynamicList</v8:Type></Type>
                        <MainAttribute>true</MainAttribute>
                    </Attribute>
                </Attributes>
                <BaseForm>
                    <Attributes>
                        <Attribute name="Object" id="1">
                            <Type><v8:Type>cfg:CatalogObject.Products</v8:Type></Type>
                            <MainAttribute>true</MainAttribute>
                        </Attribute>
                    </Attributes>
                </BaseForm>
            </Form>"#
        );

        let actual = context(&xml);

        assert_eq!(actual.definition, FormDefinitionKind::Extension);
        assert_eq!(actual.main_attribute, MainAttributeKind::DynamicList);
        assert_eq!(
            actual.main_attribute_provenance,
            MainAttributeProvenance::DirectForm
        );
        assert_eq!(
            actual.main_attribute_type.as_deref(),
            Some("cfg:DynamicList")
        );
    }

    #[test]
    fn falls_back_to_direct_base_form_main_attribute() {
        let xml = format!(
            r#"{FORM_PREFIX}
                <BaseForm>
                    <Attributes>
                        <Attribute name="Record" id="1">
                            <Type>
                                <v8:Type>cfg:InformationRegisterRecordManager.Prices</v8:Type>
                            </Type>
                            <MainAttribute>true</MainAttribute>
                        </Attribute>
                    </Attributes>
                </BaseForm>
            </Form>"#
        );

        let actual = context(&xml);

        assert_eq!(actual.definition, FormDefinitionKind::Extension);
        assert_eq!(actual.main_attribute, MainAttributeKind::PersistentRecord);
        assert_eq!(
            actual.main_attribute_provenance,
            MainAttributeProvenance::DirectBaseForm
        );
    }

    #[test]
    fn malformed_direct_main_attribute_overrides_valid_base_form_context() {
        let xml = format!(
            r#"{FORM_PREFIX}
                <Attributes>
                    <Attribute name="Object" id="1">
                        <Type/>
                        <MainAttribute>true</MainAttribute>
                    </Attribute>
                </Attributes>
                <BaseForm>
                    <Attributes>
                        <Attribute name="BaseObject" id="1">
                            <Type><v8:Type>cfg:CatalogObject.Products</v8:Type></Type>
                            <MainAttribute>true</MainAttribute>
                        </Attribute>
                    </Attributes>
                </BaseForm>
            </Form>"#
        );

        let actual = context(&xml);

        assert_eq!(actual.definition, FormDefinitionKind::Extension);
        assert_eq!(actual.main_attribute, MainAttributeKind::Unknown);
        assert_eq!(actual.main_attribute_type, None);
        assert_eq!(
            actual.main_attribute_provenance,
            MainAttributeProvenance::DirectForm
        );
    }

    #[test]
    fn distinguishes_unavailable_borrowed_context_from_malformed_base_context() {
        let unavailable = context(&format!(r#"{FORM_PREFIX}<BaseForm/></Form>"#));
        assert_eq!(unavailable.main_attribute, MainAttributeKind::Unknown);
        assert_eq!(
            unavailable.main_attribute_provenance,
            MainAttributeProvenance::InheritedBaseFormUnavailable
        );

        let malformed = context(&format!(
            r#"{FORM_PREFIX}
                <BaseForm>
                    <Attributes>
                        <Attribute name="BaseObject" id="1">
                            <Type/>
                            <MainAttribute>true</MainAttribute>
                        </Attribute>
                    </Attributes>
                </BaseForm>
            </Form>"#
        ));
        assert_eq!(malformed.main_attribute, MainAttributeKind::Unknown);
        assert_eq!(
            malformed.main_attribute_provenance,
            MainAttributeProvenance::DirectBaseForm
        );
    }

    #[test]
    fn multiple_v8_types_are_not_classified_as_a_persistent_main_type() {
        let xml = format!(
            r#"{FORM_PREFIX}
                <Attributes>
                    <Attribute name="Object" id="1">
                        <Type>
                            <v8:Type>cfg:CatalogObject.Products</v8:Type>
                            <v8:Type>xs:string</v8:Type>
                        </Type>
                        <MainAttribute>true</MainAttribute>
                    </Attribute>
                </Attributes>
            </Form>"#
        );

        let actual = context(&xml);

        assert_eq!(actual.main_attribute, MainAttributeKind::Other);
        assert_eq!(
            actual.main_attribute_type.as_deref(),
            Some("cfg:CatalogObject.Products|xs:string")
        );
        assert_eq!(
            actual.main_attribute_provenance,
            MainAttributeProvenance::DirectForm
        );

        let mixed_empty = context(&format!(
            r#"{FORM_PREFIX}
                <Attributes>
                    <Attribute name="Object" id="1">
                        <Type>
                            <v8:Type>cfg:CatalogObject.Products</v8:Type>
                            <v8:Type/>
                        </Type>
                        <MainAttribute>true</MainAttribute>
                    </Attribute>
                </Attributes>
            </Form>"#
        ));
        assert_eq!(mixed_empty.main_attribute, MainAttributeKind::Unknown);
        assert_eq!(mixed_empty.main_attribute_type, None);
        assert_eq!(
            mixed_empty.main_attribute_provenance,
            MainAttributeProvenance::DirectForm
        );
    }

    #[test]
    fn ignores_wrong_namespace_and_nested_main_attribute_traps() {
        let xml = format!(
            r#"{FORM_PREFIX}
                <Attributes xmlns="urn:not-logform">
                    <Attribute>
                        <Type><v8:Type>cfg:CatalogObject.Trap</v8:Type></Type>
                        <MainAttribute>true</MainAttribute>
                    </Attribute>
                </Attributes>
                <ChildItems>
                    <InputField name="Trap" id="1">
                        <Attributes>
                            <Attribute>
                                <Type><v8:Type>cfg:CatalogObject.NestedTrap</v8:Type></Type>
                                <MainAttribute>true</MainAttribute>
                            </Attribute>
                        </Attributes>
                    </InputField>
                </ChildItems>
            </Form>"#
        );

        let actual = context(&xml);

        assert_eq!(actual.main_attribute, MainAttributeKind::Unknown);
        assert_eq!(actual.main_attribute_type, None);
        assert_eq!(
            actual.main_attribute_provenance,
            MainAttributeProvenance::Missing
        );
    }

    #[test]
    fn classifies_known_main_attribute_families() {
        for persistent_object in [
            "cfg:CatalogObject.Goods",
            "cfg:DocumentObject.Order",
            "cfg:BusinessProcessObject.Approval",
            "cfg:TaskObject.Review",
            "cfg:ExchangePlanObject.Sync",
            "cfg:ChartOfAccountsObject.Main",
            "cfg:ChartOfCharacteristicTypesObject.Properties",
            "cfg:ChartOfCalculationTypesObject.Payroll",
        ] {
            assert_eq!(
                MainAttributeKind::from_type_name(persistent_object),
                MainAttributeKind::PersistentObject,
                "{persistent_object} must support persistent object form events"
            );
        }
        assert_eq!(
            MainAttributeKind::from_type_name("cfg:ConstantsSet"),
            MainAttributeKind::PersistentObject
        );
        for persistent_record in [
            "cfg:InformationRegisterRecordManager.Prices",
            "cfg:InformationRegisterRecordSet.Prices",
            "cfg:AccumulationRegisterRecordSet.Stock",
            "cfg:AccountingRegisterRecordSet.Accounting",
            "cfg:CalculationRegisterRecordSet.Payroll",
        ] {
            assert_eq!(
                MainAttributeKind::from_type_name(persistent_record),
                MainAttributeKind::PersistentRecord,
                "{persistent_record} must support persistent record form events"
            );
        }
        assert_eq!(
            MainAttributeKind::from_type_name("cfg:DynamicList"),
            MainAttributeKind::DynamicList
        );
        assert_eq!(
            MainAttributeKind::from_type_name("cfg:DataProcessorObject.Import"),
            MainAttributeKind::Other
        );
        for malformed in [
            "cfg:ConstantsSet.ApplicationSettings",
            "cfg:CatalogObject",
            "cfg:CatalogObject.Goods|string",
            "cfg:CatalogObject.Goods+string",
            "cfg:CatalogObject.Goods.Extra",
            "cfg:CatalogObject.Goods Name",
            "cfg:CatalogObject.Goods,Other",
            "cfg:CatalogObject.Goods/Other",
            "cfg:CatalogObject.Goods#x",
            "cfg:CatalogObject.123",
        ] {
            assert_eq!(
                MainAttributeKind::from_type_name(malformed),
                MainAttributeKind::Other,
                "{malformed} must not enter the persistent event whitelist"
            );
        }
        for unsupported in [
            "cfg:ReportObject.Sales",
            "cfg:DataProcessorObject.Import",
            "cfg:ExternalReportObject.Sales",
            "cfg:ExternalDataProcessorObject.Import",
        ] {
            assert_eq!(
                MainAttributeKind::from_type_name(unsupported),
                MainAttributeKind::Other,
                "{unsupported} must not enter the persistent event whitelist"
            );
        }
        assert_eq!(
            MainAttributeKind::from_type_name("  "),
            MainAttributeKind::Unknown
        );
    }

    #[test]
    fn on_read_accepts_persistent_object_and_record_contexts() {
        let binding = FormEventBinding::new("OnReadAtServer", "ObjectOnReadAtServer");

        assert!(validate_event(
            &regular_context(MainAttributeKind::PersistentObject),
            FormEventTarget::Form,
            &binding,
        )
        .is_ok());
        assert!(validate_event(
            &regular_context(MainAttributeKind::PersistentRecord),
            FormEventTarget::Form,
            &binding,
        )
        .is_ok());
    }

    #[test]
    fn on_read_rejects_known_nonpersistent_context() {
        let mut context = regular_context(MainAttributeKind::DynamicList);
        context.main_attribute_type = Some("cfg:DynamicList".to_string());
        let error = validate_event(
            &context,
            FormEventTarget::Form,
            &FormEventBinding::new("OnReadAtServer", "ListOnReadAtServer"),
        )
        .unwrap_err();

        assert_eq!(error.code, FormEventDiagnosticCode::EventNotAllowed);
        assert!(error.detail.contains("cfg:DynamicList"));
    }

    #[test]
    fn on_read_reports_unknown_context_separately() {
        let error = validate_event(
            &regular_context(MainAttributeKind::Unknown),
            FormEventTarget::Form,
            &FormEventBinding::new("OnReadAtServer", "ObjectOnReadAtServer"),
        )
        .unwrap_err();

        assert_eq!(error.code, FormEventDiagnosticCode::ContextUnknown);
    }

    #[test]
    fn all_object_record_events_are_context_gated() {
        for event in OBJECT_RECORD_FORM_EVENTS {
            assert!(validate_event(
                &regular_context(MainAttributeKind::PersistentObject),
                FormEventTarget::Form,
                &FormEventBinding::new(event, "ObjectEventHandler"),
            )
            .is_ok());

            let unknown_error = validate_event(
                &regular_context(MainAttributeKind::Unknown),
                FormEventTarget::Form,
                &FormEventBinding::new(event, "ObjectEventHandler"),
            )
            .unwrap_err();
            assert_eq!(
                unknown_error.code,
                FormEventDiagnosticCode::ContextUnknown,
                "{event} must report unknown context"
            );

            let unsupported_error = validate_event(
                &regular_context(MainAttributeKind::Other),
                FormEventTarget::Form,
                &FormEventBinding::new(event, "ObjectEventHandler"),
            )
            .unwrap_err();
            assert_eq!(
                unsupported_error.code,
                FormEventDiagnosticCode::EventNotAllowed,
                "{event} must reject a known unsupported context"
            );
        }
    }

    #[test]
    fn generic_write_processing_events_are_not_main_context_gated() {
        for event in ["NewWriteProcessing", "FillCheckProcessingAtServer"] {
            for context in [
                regular_context(MainAttributeKind::DynamicList),
                regular_context(MainAttributeKind::Other),
                regular_context(MainAttributeKind::Unknown),
            ] {
                assert!(
                    validate_event(
                        &context,
                        FormEventTarget::Form,
                        &FormEventBinding::new(event, "GenericWriteHandler"),
                    )
                    .is_ok(),
                    "{event} must be valid without a persistent object/record main attribute"
                );
            }
        }
    }

    #[test]
    fn validates_root_event_union() {
        let context = regular_context(MainAttributeKind::Other);

        assert!(validate_event(
            &context,
            FormEventTarget::Form,
            &FormEventBinding::new("OnCreateAtServer", "FormOnCreateAtServer"),
        )
        .is_ok());
        assert!(validate_event(
            &context,
            FormEventTarget::Form,
            &FormEventBinding::new(
                "OnMainServerAvailabilityChange",
                "FormOnMainServerAvailabilityChange",
            ),
        )
        .is_ok());
        assert!(validate_event(
            &context,
            FormEventTarget::Form,
            &FormEventBinding::new("URLListGetProcessing", "FormURLListGetProcessing"),
        )
        .is_ok());
        let error = validate_event(
            &context,
            FormEventTarget::Form,
            &FormEventBinding::new("Opening", "FormOpening"),
        )
        .unwrap_err();
        assert_eq!(error.code, FormEventDiagnosticCode::EventNotAllowed);
    }

    #[test]
    fn opening_is_allowed_only_for_input_field() {
        let context = regular_context(MainAttributeKind::Other);
        let binding = FormEventBinding::new("Opening", "FieldOpening");

        assert!(validate_event(
            &context,
            FormEventTarget::Element(FormElementKind::InputField),
            &binding,
        )
        .is_ok());
        let error = validate_event(
            &context,
            FormEventTarget::Element(FormElementKind::LabelField),
            &binding,
        )
        .unwrap_err();
        assert_eq!(error.code, FormEventDiagnosticCode::EventNotAllowed);
    }

    #[test]
    fn current_page_change_is_allowed_only_for_pages() {
        let context = regular_context(MainAttributeKind::Other);
        let binding = FormEventBinding::new("OnCurrentPageChange", "PagesOnChange");

        assert!(validate_event(
            &context,
            FormEventTarget::Element(FormElementKind::Pages),
            &binding,
        )
        .is_ok());
        let error = validate_event(
            &context,
            FormEventTarget::Element(FormElementKind::Page),
            &binding,
        )
        .unwrap_err();
        assert_eq!(error.code, FormEventDiagnosticCode::EventNotAllowed);
    }

    #[test]
    fn maps_xml_tags_and_dsl_keys_to_the_same_matrix() {
        let cases = [
            ("InputField", "input", FormElementKind::InputField),
            ("CheckBoxField", "check", FormElementKind::CheckBoxField),
            ("TrackBarField", "trackBar", FormElementKind::TrackBarField),
            ("Table", "table", FormElementKind::Table),
            ("Pages", "pages", FormElementKind::Pages),
            (
                "PictureDecoration",
                "picture",
                FormElementKind::PictureDecoration,
            ),
            (
                "ExtendedTooltip",
                "extendedTooltip",
                FormElementKind::ExtendedTooltip,
            ),
            (
                "FormattedDocumentField",
                "formattedDoc",
                FormElementKind::FormattedDocumentField,
            ),
            (
                "SpreadSheetDocumentField",
                "spreadsheet",
                FormElementKind::SpreadsheetDocumentField,
            ),
            ("UsualGroup", "group", FormElementKind::Group),
        ];

        for (xml_tag, dsl_key, expected) in cases {
            assert_eq!(FormElementKind::from_xml_tag(xml_tag), Some(expected));
            assert_eq!(FormElementKind::from_dsl_key(dsl_key), Some(expected));
        }
        assert_eq!(FormElementKind::from_xml_tag("Popup"), None);
        assert_eq!(FormElementKind::from_xml_tag("UnknownElement"), None);
        assert_eq!(FormElementKind::from_dsl_key("unknown"), None);
    }

    #[test]
    fn uses_platform_audited_element_event_matrix() {
        assert_eq!(FormElementKind::Button.allowed_events(), NO_EVENTS);
        assert_eq!(
            FormElementKind::LabelField.allowed_events(),
            &["URLProcessing", "Click", "OnChange"]
        );
        assert_eq!(FormElementKind::PictureField.allowed_events(), &["Click"]);
        assert_eq!(
            FormElementKind::CalendarField.allowed_events(),
            &["Selection", "OnChange", "OnPeriodOutput"]
        );
        assert_eq!(
            FormElementKind::HtmlDocumentField.allowed_events(),
            &["OnClick", "DocumentComplete"]
        );
        assert_eq!(
            FormElementKind::SpreadsheetDocumentField.allowed_events(),
            &[
                "DetailProcessing",
                "Selection",
                "OnActivate",
                "AdditionalDetailProcessing",
                "OnChange",
                "Drag",
                "URLProcessing",
                "BeforePrint",
                "BeforeWrite",
                "DragCheck",
                "OnChangeAreaContent",
            ]
        );

        assert!(!FormElementKind::InputField
            .allowed_events()
            .contains(&"Click"));
        assert!(!FormElementKind::Table.allowed_events().contains(&"Drop"));

        let button_error = validate_event(
            &regular_context(MainAttributeKind::Other),
            FormEventTarget::Element(FormElementKind::Button),
            &FormEventBinding::new("Click", "ButtonClick"),
        )
        .unwrap_err();
        assert_eq!(button_error.code, FormEventDiagnosticCode::EventNotAllowed);
    }

    #[test]
    fn form_compile_skill_documents_registered_events_for_each_documented_element() {
        const SKILL: &str = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/unica/skills/form-compile/SKILL.md"
        ));
        const START: &str = "<!-- form-event-registry:start -->";
        const END: &str = "<!-- form-event-registry:end -->";

        let section = SKILL
            .split_once(START)
            .and_then(|(_, tail)| tail.split_once(END).map(|(section, _)| section))
            .expect("form-compile event table must be delimited for parity checks");
        let cases = [
            ("input", FormElementKind::InputField),
            ("check", FormElementKind::CheckBoxField),
            ("labelField", FormElementKind::LabelField),
            ("table", FormElementKind::Table),
            ("pages", FormElementKind::Pages),
            ("page", FormElementKind::Page),
            ("button", FormElementKind::Button),
            ("cmdBar", FormElementKind::CommandBar),
            ("autoCmdBar", FormElementKind::CommandBar),
            ("group", FormElementKind::Group),
        ];

        let documented_keys = section
            .lines()
            .filter_map(|line| {
                line.strip_prefix("| `")
                    .and_then(|line| line.split_once("` | "))
                    .map(|(key, _)| key)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            documented_keys,
            cases.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
            "the documented event table must cover exactly the documented form DSL elements"
        );

        for (key, kind) in cases {
            let expected = if kind.allowed_events().is_empty() {
                "—".to_string()
            } else {
                kind.allowed_events()
                    .iter()
                    .map(|event| format!("`{event}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let prefix = format!("| `{key}` | ");
            let documented = section
                .lines()
                .find_map(|line| line.strip_prefix(&prefix))
                .and_then(|value| value.strip_suffix(" |"))
                .expect("documented form element event row must be present");

            assert_eq!(documented, expected, "event list for `{key}`");
        }
    }

    #[test]
    fn rejects_empty_handler() {
        let error = validate_event(
            &regular_context(MainAttributeKind::Other),
            FormEventTarget::Element(FormElementKind::LabelDecoration),
            &FormEventBinding::new("Click", "  \n"),
        )
        .unwrap_err();

        assert_eq!(error.code, FormEventDiagnosticCode::EmptyHandler);
    }

    #[test]
    fn rejects_call_type_in_regular_form() {
        let error = validate_event(
            &regular_context(MainAttributeKind::Other),
            FormEventTarget::Form,
            &FormEventBinding::new("OnOpen", "FormOnOpen").with_call_type("After"),
        )
        .unwrap_err();

        assert_eq!(error.code, FormEventDiagnosticCode::CallTypeNotAllowed);
    }

    #[test]
    fn accepts_all_call_types_in_extension_form() {
        let context = extension_context(MainAttributeKind::Other);

        for call_type in ["Before", "After", "Override"] {
            assert!(validate_event(
                &context,
                FormEventTarget::Form,
                &FormEventBinding::new("OnOpen", "FormOnOpen").with_call_type(call_type),
            )
            .is_ok());
        }
    }

    #[test]
    fn extension_form_binding_requires_an_explicit_call_type() {
        let error = validate_event_call_type(
            &extension_context(MainAttributeKind::Other),
            FormEventTarget::Form,
            &FormEventBinding::new("OnOpen", "FormOnOpen"),
        )
        .unwrap_err();

        assert_eq!(error.code.as_str(), "FORM_EVENT_CALL_TYPE_REQUIRED");
    }

    #[test]
    fn rejects_invalid_call_type_in_extension_form() {
        for invalid in ["Instead", "after", ""] {
            let error = validate_event(
                &extension_context(MainAttributeKind::Other),
                FormEventTarget::Form,
                &FormEventBinding::new("OnOpen", "FormOnOpen").with_call_type(invalid),
            )
            .unwrap_err();

            assert_eq!(
                error.code,
                FormEventDiagnosticCode::InvalidCallType,
                "'{invalid}' must be rejected"
            );
        }

        let error = validate_event(
            &extension_context(MainAttributeKind::Unknown),
            FormEventTarget::Form,
            &FormEventBinding::new("OnReadAtServer", "FormOnReadAtServer").with_call_type("after"),
        )
        .unwrap_err();
        assert_eq!(error.code, FormEventDiagnosticCode::InvalidCallType);
    }

    #[test]
    fn diagnostic_codes_and_display_are_stable() {
        let codes = [
            (
                FormEventDiagnosticCode::EventNotAllowed,
                "FORM_EVENT_NOT_ALLOWED",
            ),
            (
                FormEventDiagnosticCode::ContextUnknown,
                "FORM_EVENT_CONTEXT_UNKNOWN",
            ),
            (
                FormEventDiagnosticCode::EmptyHandler,
                "FORM_EVENT_EMPTY_HANDLER",
            ),
            (FormEventDiagnosticCode::Duplicate, "FORM_EVENT_DUPLICATE"),
            (
                FormEventDiagnosticCode::BindingConflict,
                "FORM_EVENT_BINDING_CONFLICT",
            ),
            (
                FormEventDiagnosticCode::TargetNotFound,
                "FORM_EVENT_TARGET_NOT_FOUND",
            ),
            (
                FormEventDiagnosticCode::InvalidCallType,
                "FORM_EVENT_INVALID_CALL_TYPE",
            ),
            (
                FormEventDiagnosticCode::CallTypeRequired,
                "FORM_EVENT_CALL_TYPE_REQUIRED",
            ),
            (
                FormEventDiagnosticCode::CallTypeNotAllowed,
                "FORM_EVENT_CALL_TYPE_NOT_ALLOWED",
            ),
        ];

        for (code, expected) in codes {
            assert_eq!(code.as_str(), expected);
        }

        let diagnostic =
            FormEventDiagnostic::new(FormEventDiagnosticCode::EventNotAllowed, "form", "Opening")
                .with_detail("event is not present in the target event matrix");
        assert_eq!(
            diagnostic.to_string(),
            "[FORM_EVENT_NOT_ALLOWED] event 'Opening' on form: event is not present in the target event matrix"
        );
    }

    #[test]
    fn module_event_applicability_covers_every_approved_role_family() {
        use unica_address::address::QualifiedAddress;
        use unica_bsl::profile::PlatformProfile;

        let cases: [(&str, &[&str]); 16] = [
            (
                "main:Document.Заказ.Module.Object",
                &[
                    "BeforeDelete",
                    "BeforeWrite",
                    "FillCheckProcessing",
                    "Filling",
                    "GenerateFromDataHistoryVersionProcessing",
                    "OnCopy",
                    "OnSetNewNumber",
                    "OnWrite",
                    "Posting",
                    "UndoPosting",
                ],
            ),
            (
                "main:Document.Заказ.Module.Manager",
                &[
                    "AfterWriteDataHistoryVersionsProcessing",
                    "ChoiceDataGetProcessing",
                    "FormGetProcessing",
                    "PresentationFieldsGetProcessing",
                    "PresentationGetProcessing",
                ],
            ),
            (
                "main:InformationRegister.Цены.Module.RecordSet",
                &[
                    "BeforeWrite",
                    "FillCheckProcessing",
                    "Filling",
                    "GenerateFromDataHistoryVersionProcessing",
                    "OnWrite",
                ],
            ),
            (
                "main:Constant.ОсновнаяВалюта.Module.ValueManager",
                &[
                    "BeforeWrite",
                    "FillCheckProcessing",
                    "GenerateFromDataHistoryVersionProcessing",
                    "OnWrite",
                ],
            ),
            ("main:CommonModule.ЗаказыСервер", &[]),
            ("main:Document.Заказ.Form.ФормаДокумента.Module.Form", &[]),
            (
                "main:Document.Заказ.Command.ПровестиИЗакрыть.Module.Command",
                &["CommandProcessing"],
            ),
            (
                "main:Module.ManagedApplication",
                &[
                    "AddInDetachmentOnErrorProcessing",
                    "AfterExchangeDataWithMainServer",
                    "BeforeExit",
                    "BeforeStart",
                    "CollaborationSystemExternalUserInvitationProcessing",
                    "CollaborationSystemMessageTemplateChoiceProcessing",
                    "CollaborationSystemUsersAutoComplete",
                    "CollaborationSystemUsersChoiceFormGetProcessing",
                    "ErrorDisplayProcessing",
                    "ExternEventProcessing",
                    "IncomingShareRequestCommandGenerateProcessing",
                    "NavigationByURLProcessing",
                    "OnChangeDisplaySettings",
                    "OnClientApplicationResume",
                    "OnClientApplicationSuspend",
                    "OnCollaborationSystemMessageActionChoice",
                    "OnCollaborationSystemMessageButtonPanelButtonClick",
                    "OnExit",
                    "OnGlobalSearch",
                    "OnGlobalSearchResultActionChoice",
                    "OnGlobalSearchResultChoice",
                    "OnMainServerAvailabilityChange",
                    "OnPasteFromClipboard",
                    "OnStart",
                ],
            ),
            (
                "main:Module.OrdinaryApplication",
                &[
                    "BeforeExit",
                    "BeforeStart",
                    "ExternEventProcessing",
                    "OnChangeDisplaySettings",
                    "OnExit",
                    "OnStart",
                ],
            ),
            ("main:Module.Session", &["SessionParametersSetting"]),
            ("main:Module.ExternalConnection", &["OnExit", "OnStart"]),
            ("main:HTTPService.API.Module.HTTPService", &[]),
            ("main:WebService.Обмен.Module.WebService", &[]),
            (
                "main:IntegrationService.Шина.Module.IntegrationService",
                &[],
            ),
            (
                "main:Bot.Помощник.Module.Bot",
                &[
                    "CollaborationSystemMessageButtonPanelButtonClickProcessing",
                    "CollaborationSystemMessageProcessing",
                    "OnAddToCollaborationSystemConversation",
                    "OnCreateCollaborationSystemConversation",
                    "OnDeleteFromCollaborationSystemConversation",
                ],
            ),
            (
                "main:WebSocketClient.Телефония.Module.WebSocketClient",
                &[
                    "BeforeConnect",
                    "OnCloseConnection",
                    "OnError",
                    "OnMessage",
                    "OnOpenConnection",
                ],
            ),
        ];
        let profile = PlatformProfile::v8_3_27();
        for (raw, expected) in cases {
            let at = QualifiedAddress::parse(raw).unwrap();
            let capability = profile.module_capability(&at).unwrap();
            let actual = module_event_catalog_8_3_27(capability)
                .iter()
                .map(|event| event.event_id.as_str())
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "{raw}");
        }
    }

    #[test]
    fn module_catalog_covers_the_task12_direct_owner_role_matrix_exactly() {
        use unica_address::address::NodeKind;
        use unica_address::module_routes::ModuleRole;
        use unica_bsl::profile::PlatformProfile;

        let direct_roles = [
            ModuleRole::Object,
            ModuleRole::Manager,
            ModuleRole::RecordSet,
            ModuleRole::ValueManager,
        ];
        let profile = PlatformProfile::v8_3_27();
        let expected = NodeKind::metadata_kinds()
            .iter()
            .flat_map(|owner| direct_roles.iter().map(move |role| (*owner, *role)))
            .filter(|(owner, role)| profile.supports_direct_module_role(*owner, *role))
            .map(|(owner, role)| (owner.as_str(), role.as_str()))
            .collect::<std::collections::BTreeSet<_>>();
        let actual = platform_event_catalog_fixture()
            .module_catalogs
            .iter()
            .filter(|catalog| {
                direct_roles
                    .iter()
                    .any(|role| role.as_str() == catalog.module_role)
            })
            .map(|catalog| (catalog.owner_kind.as_str(), catalog.module_role.as_str()))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn form_event_applicability_preserves_every_logical_owner_family() {
        let owners = std::iter::once(FormEventOwnerKind::Form)
            .chain(std::iter::once(FormEventOwnerKind::Table))
            .chain(std::iter::once(FormEventOwnerKind::Command))
            .chain(
                FormElementKind::ALL
                    .into_iter()
                    .filter(|kind| *kind != FormElementKind::Table)
                    .map(FormEventOwnerKind::Element),
            )
            .chain(
                FormElementKind::ALL
                    .into_iter()
                    .filter(|kind| *kind != FormElementKind::Table)
                    .map(FormEventOwnerKind::Column),
            )
            .collect::<Vec<_>>();
        let mut observed_keys = std::collections::BTreeSet::new();
        for owner in owners {
            let key = unica_forms::events::test_support::form_event_owner_catalog_key(owner);
            let expected = platform_event_catalog_fixture()
                .form_catalogs
                .iter()
                .find(|catalog| catalog.owner_kinds.contains(&key))
                .unwrap_or_else(|| panic!("missing checked catalog for {key}"));
            assert_eq!(
                form_event_catalog_8_3_27(owner)
                    .into_iter()
                    .map(|event| event.event_id.as_str())
                    .collect::<Vec<_>>(),
                expected
                    .events
                    .iter()
                    .map(|event| event.event_id.as_str())
                    .collect::<Vec<_>>(),
                "{key}"
            );
            observed_keys.insert(key);
        }
        let expected_keys = platform_event_catalog_fixture()
            .form_catalogs
            .iter()
            .flat_map(|catalog| catalog.owner_kinds.iter().cloned())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(observed_keys, expected_keys);
    }

    #[test]
    fn form_applicability_variants_have_exact_closed_event_additions_and_counts() {
        let base_context = projection_context(
            FormDefinitionKind::Regular,
            MainAttributeKind::Other,
            None,
            NodeKind::CommonForm,
        );
        let base = unica_forms::events::form_event_catalog_8_3_27(
            &base_context,
            FormEventOwnerKind::Form,
            None,
        )
        .into_iter()
        .map(|event| event.event_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(base.len(), 26);

        let cases = [
            (
                "catalog-object",
                projection_context(
                    FormDefinitionKind::Regular,
                    MainAttributeKind::PersistentObject,
                    Some("cfg:CatalogObject.Items"),
                    NodeKind::Catalog,
                ),
                &[
                    "AfterWrite",
                    "AfterWriteAtServer",
                    "BeforeWrite",
                    "BeforeWriteAtServer",
                    "OnReadAtServer",
                    "OnWriteAtServer",
                    "ValueChoice",
                ][..],
            ),
            (
                "document-object",
                projection_context(
                    FormDefinitionKind::Regular,
                    MainAttributeKind::PersistentObject,
                    Some("cfg:DocumentObject.Order"),
                    NodeKind::Document,
                ),
                &[
                    "AfterWrite",
                    "AfterWriteAtServer",
                    "BeforeWrite",
                    "BeforeWriteAtServer",
                    "OnReadAtServer",
                    "OnWriteAtServer",
                    "ValueChoice",
                ][..],
            ),
            (
                "business-process-object",
                projection_context(
                    FormDefinitionKind::Regular,
                    MainAttributeKind::PersistentObject,
                    Some("cfg:BusinessProcessObject.Route"),
                    NodeKind::BusinessProcess,
                ),
                &[
                    "AfterWrite",
                    "AfterWriteAtServer",
                    "BeforeStart",
                    "BeforeWrite",
                    "BeforeWriteAtServer",
                    "OnReadAtServer",
                    "OnWriteAtServer",
                    "ValueChoice",
                ][..],
            ),
            (
                "task-object",
                projection_context(
                    FormDefinitionKind::Regular,
                    MainAttributeKind::PersistentObject,
                    Some("cfg:TaskObject.Task"),
                    NodeKind::Task,
                ),
                &[
                    "AfterWrite",
                    "AfterWriteAtServer",
                    "BeforeExecute",
                    "BeforeWrite",
                    "BeforeWriteAtServer",
                    "OnReadAtServer",
                    "OnWriteAtServer",
                    "ValueChoice",
                ][..],
            ),
            (
                "characteristic-object",
                projection_context(
                    FormDefinitionKind::Regular,
                    MainAttributeKind::PersistentObject,
                    Some("cfg:ChartOfCharacteristicTypesObject.Kinds"),
                    NodeKind::ChartOfCharacteristicTypes,
                ),
                &[
                    "AfterWrite",
                    "AfterWriteAtServer",
                    "BeforeWrite",
                    "BeforeWriteAtServer",
                    "OnReadAtServer",
                    "OnWriteAtServer",
                    "ValueChoice",
                ][..],
            ),
            (
                "constant-set",
                projection_context(
                    FormDefinitionKind::Regular,
                    MainAttributeKind::PersistentObject,
                    Some("cfg:ConstantsSet"),
                    NodeKind::Constant,
                ),
                &[
                    "AfterWrite",
                    "AfterWriteAtServer",
                    "BeforeWrite",
                    "BeforeWriteAtServer",
                    "OnReadAtServer",
                    "OnWriteAtServer",
                ][..],
            ),
            (
                "information-register-record",
                projection_context(
                    FormDefinitionKind::Regular,
                    MainAttributeKind::PersistentRecord,
                    Some("cfg:InformationRegisterRecordManager.Prices"),
                    NodeKind::InformationRegister,
                ),
                &[
                    "AfterWrite",
                    "AfterWriteAtServer",
                    "BeforeWrite",
                    "BeforeWriteAtServer",
                    "OnReadAtServer",
                    "OnWriteAtServer",
                ][..],
            ),
            (
                "record-set",
                projection_context(
                    FormDefinitionKind::Regular,
                    MainAttributeKind::PersistentRecord,
                    Some("cfg:AccumulationRegisterRecordSet.Stock"),
                    NodeKind::AccumulationRegister,
                ),
                &[
                    "AfterWrite",
                    "AfterWriteAtServer",
                    "BeforeWrite",
                    "BeforeWriteAtServer",
                    "OnReadAtServer",
                    "OnWriteAtServer",
                ][..],
            ),
            (
                "report",
                projection_context(
                    FormDefinitionKind::Regular,
                    MainAttributeKind::Other,
                    Some("cfg:ReportObject.Sales"),
                    NodeKind::Report,
                ),
                &[
                    "BeforeLoadUserSettingsAtServer",
                    "BeforeLoadVariantAtServer",
                    "OnLoadUserSettingsAtServer",
                    "OnLoadVariantAtServer",
                    "OnSaveUserSettingsAtServer",
                    "OnSaveVariantAtServer",
                    "OnUpdateUserSettingSetAtServer",
                ][..],
            ),
        ];
        for (case, context, expected_additions) in cases {
            let actual = unica_forms::events::form_event_catalog_8_3_27(
                &context,
                FormEventOwnerKind::Form,
                None,
            )
            .into_iter()
            .map(|event| event.event_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
            let additions = actual.difference(&base).copied().collect::<Vec<_>>();
            assert_eq!(additions, expected_additions, "{case}");
            assert_eq!(
                actual.len(),
                base.len() + expected_additions.len(),
                "{case}"
            );
        }

        let extension_document = projection_context(
            FormDefinitionKind::Extension,
            MainAttributeKind::PersistentObject,
            Some("cfg:DocumentObject.Order"),
            NodeKind::Document,
        );
        let extension_ids = unica_forms::events::form_event_catalog_8_3_27(
            &extension_document,
            FormEventOwnerKind::Form,
            None,
        )
        .into_iter()
        .map(|event| event.event_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(extension_ids.len(), 33);

        let mut dynamic = projection_context(
            FormDefinitionKind::Regular,
            MainAttributeKind::DynamicList,
            Some("cfg:DynamicList"),
            NodeKind::Catalog,
        );
        dynamic.main_attribute_name = Some("List".to_string());
        let ordinary_table = unica_forms::events::form_event_catalog_8_3_27(
            &dynamic,
            FormEventOwnerKind::Table,
            Some("OtherRows"),
        );
        let dynamic_table = unica_forms::events::form_event_catalog_8_3_27(
            &dynamic,
            FormEventOwnerKind::Table,
            Some("List"),
        );
        let inherited_dynamic_table = unica_forms::events::form_event_catalog_8_3_27(
            &dynamic,
            FormEventOwnerKind::Table,
            Some("~List"),
        );
        assert_eq!(ordinary_table.len(), 23);
        assert_eq!(dynamic_table.len(), 30);
        assert_eq!(inherited_dynamic_table.len(), 30);
        let ordinary_ids = ordinary_table
            .into_iter()
            .map(|event| event.event_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            dynamic_table
                .into_iter()
                .map(|event| event.event_id.as_str())
                .filter(|event| !ordinary_ids.contains(event))
                .collect::<Vec<_>>(),
            [
                "BeforeLoadUserSettingsAtServer",
                "OnGetDataAtServer",
                "OnLoadUserSettingsAtServer",
                "OnSaveUserSettingsAtServer",
                "OnUpdateUserSettingSetAtServer",
                "URLGetProcessing",
                "URLListGetProcessing",
            ]
        );
    }

    #[test]
    fn event_catalog_entries_have_exact_bilingual_shape_context_and_provenance() {
        let fixture = platform_event_catalog_fixture();
        for events in fixture
            .module_catalogs
            .iter()
            .map(|catalog| catalog.events.as_slice())
            .chain(
                fixture
                    .form_catalogs
                    .iter()
                    .map(|catalog| catalog.events.as_slice()),
            )
        {
            for event in events {
                let expected_head = if event.method_kind == "function" {
                    ("Функция ", "Function ")
                } else {
                    ("Процедура ", "Procedure ")
                };
                assert!(event.signature_ru.starts_with(expected_head.0));
                assert!(event.signature_en.starts_with(expected_head.1));
                assert!(event.signature_ru.contains(&event.handler_ru));
                assert!(event.signature_en.contains(&event.handler_en));
                assert!(!event.contexts.is_empty());
                assert!(event
                    .source_page_id
                    .starts_with("platform-syntax-help:syntax-context:"));
            }
        }
    }

    #[test]
    fn every_catalog_has_unique_semantic_event_ids_not_generic_storage_names() {
        let fixture = platform_event_catalog_fixture();
        for (catalog, events) in fixture
            .module_catalogs
            .iter()
            .map(|catalog| {
                (
                    format!("{} × {}", catalog.owner_kind, catalog.module_role),
                    catalog.events.as_slice(),
                )
            })
            .chain(fixture.form_catalogs.iter().map(|catalog| {
                (
                    format!("{:?}", catalog.owner_kinds),
                    catalog.events.as_slice(),
                )
            }))
        {
            let ids = events
                .iter()
                .map(|event| event.event_id.as_str())
                .collect::<Vec<_>>();
            let unique = ids
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(unique.len(), ids.len(), "{catalog}: {ids:?}");
            assert!(
                !ids.contains(&"event"),
                "{catalog}: storage filename is not a semantic event id"
            );
        }
    }

    #[test]
    fn form_catalog_execution_contexts_distinguish_client_and_server_callbacks() {
        let form = form_event_catalog_8_3_27(FormEventOwnerKind::Form);
        let on_open = form
            .iter()
            .find(|event| event.event_id == "OnOpen")
            .unwrap();
        assert_eq!(
            on_open.contexts,
            [
                "thinClient",
                "webClient",
                "thickClientManaged",
                "mobileClient",
                "mobileAppClient",
            ]
        );
        let create_at_server = form
            .iter()
            .find(|event| event.event_id == "OnCreateAtServer")
            .unwrap();
        assert_eq!(create_at_server.contexts, ["server"]);
        let field =
            form_event_catalog_8_3_27(FormEventOwnerKind::Element(FormElementKind::InputField));
        assert_eq!(
            field
                .iter()
                .find(|event| event.event_id == "OnChange")
                .unwrap()
                .contexts,
            [
                "thinClient",
                "webClient",
                "thickClientManaged",
                "mobileClient",
                "mobileAppClient",
            ]
        );
    }

    #[test]
    fn checked_event_catalog_is_a_closed_immutable_8_3_27_set() {
        use sha2::{Digest, Sha256};

        let bytes =
            include_bytes!("../../unica-metadata/src/platform-event-catalog-8.3.27.2074.json");
        let digest = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            digest,
            "4f3b22f55276e7146f77db717e56684f950df11cadccbce2df66da39aa33bf74"
        );
        let fixture = platform_event_catalog_fixture();
        assert_eq!(fixture.profile, "8.3.27");
        assert_eq!(fixture.source.installation_version, "8.3.27.2074");
        assert_eq!(fixture.module_catalogs.len(), 48);
        assert_eq!(fixture.form_catalogs.len(), 31);
        assert_eq!(
            fixture
                .module_catalogs
                .iter()
                .map(|catalog| catalog.events.len())
                .sum::<usize>(),
            205
        );
        assert_eq!(
            fixture
                .form_catalogs
                .iter()
                .map(|catalog| catalog.events.len())
                .sum::<usize>(),
            199
        );
        assert_eq!(fixture.excluded_structural_pages.len(), 4);
        assert_eq!(fixture.excluded_external_data_source_pages.len(), 13);
        assert_eq!(fixture.excluded_generic_template_pages.len(), 5);
        assert_eq!(fixture.excluded_out_of_profile_pages.len(), 269);
        assert_eq!(fixture.source.english_container, "shcntx_root.hbk");
        assert_eq!(
            fixture.source.english_sha256,
            "0113af559ef2001dedbbd9e6bbfcf8bc02e3241ca715f95c8a34c4f24c962421"
        );
    }

    #[test]
    fn checked_event_fixture_is_non_skipping_closed_partition_evidence() {
        let fixture = platform_event_catalog_fixture();
        let mut memberships = std::collections::BTreeMap::<&str, Vec<&str>>::new();
        let mut selected_records = std::collections::BTreeMap::<&str, usize>::new();
        for event in fixture
            .module_catalogs
            .iter()
            .flat_map(|catalog| catalog.events.iter())
            .chain(
                fixture
                    .form_catalogs
                    .iter()
                    .flat_map(|catalog| catalog.events.iter()),
            )
        {
            *selected_records
                .entry(event.source_page_id.as_str())
                .or_default() += 1;
            memberships
                .entry(event.source_page_id.as_str())
                .or_default()
                .push("selected");
        }
        for (category, pages) in [
            ("structural", fixture.excluded_structural_pages.as_slice()),
            (
                "externalDataSourceDeferred",
                fixture.excluded_external_data_source_pages.as_slice(),
            ),
            (
                "genericTemplate",
                fixture.excluded_generic_template_pages.as_slice(),
            ),
            (
                "reviewedOutOfProfile",
                fixture.excluded_out_of_profile_pages.as_slice(),
            ),
        ] {
            for page in pages {
                memberships
                    .entry(page.page_id.as_str())
                    .or_default()
                    .push(category);
            }
        }
        let duplicate_categories = memberships
            .iter()
            .filter(|(_, categories)| {
                categories
                    .iter()
                    .copied()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != 1
            })
            .collect::<Vec<_>>();
        assert!(
            duplicate_categories.is_empty(),
            "page IDs must belong to exactly one partition category: {duplicate_categories:#?}"
        );
        assert_eq!(memberships.len(), fixture.source.event_markup_page_count);
        assert_eq!(fixture.source.event_markup_page_count, 693);
        assert_eq!(fixture.source.signature_event_leaf_count, 689);
        assert_eq!(fixture.excluded_structural_pages.len(), 4);
        assert_eq!(fixture.excluded_external_data_source_pages.len(), 13);
        assert_eq!(fixture.excluded_generic_template_pages.len(), 5);
        assert_eq!(fixture.excluded_out_of_profile_pages.len(), 269);
        assert_eq!(
            fixture.source.event_page_ids_sha256,
            "19ad908ce8af0ba191dc642545d70ec163d402f129c34cd9f3c2d68318a09a31"
        );
        assert_eq!(selected_records.len(), 402, "unique selected vendor pages");
        assert_eq!(
            selected_records.values().sum::<usize>(),
            404,
            "selected projection records"
        );
        let repeated = selected_records
            .into_iter()
            .filter(|(_, count)| *count > 1)
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            repeated,
            std::collections::BTreeMap::from([
                (
                    "platform-syntax-help:syntax-context:objects/catalog1649/Command module/events/CommandProcessing563.html",
                    2,
                ),
                (
                    "platform-syntax-help:syntax-context:objects/catalog1649/catalog1676/FormField/events/OnChange326.html",
                    2,
                ),
            ])
        );
        let approved_reasons = [
            "out-of-profile/ordinary-form: ordinary-form runtime owners are outside the approved managed-form projection",
            "out-of-profile/managed-form-element: the element kind is absent from the approved v0.13 form owner taxonomy",
            "out-of-profile/external-data-source-module: ExternalDataSource has no approved v0.13 address/profile owner",
            "out-of-profile/non-module-owner: the platform event owner is not an approved module or managed-form owner",
            "out-of-profile/settings-composer: no approved source-derived form applicability fact identifies this owner",
        ];
        assert!(fixture
            .excluded_out_of_profile_pages
            .iter()
            .all(|page| approved_reasons.contains(&page.reason.as_str())));
        let reason_counts = fixture.excluded_out_of_profile_pages.iter().fold(
            std::collections::BTreeMap::<&str, usize>::new(),
            |mut counts, page| {
                *counts.entry(page.reason.as_str()).or_default() += 1;
                counts
            },
        );
        assert_eq!(
            reason_counts,
            std::collections::BTreeMap::from([
                (approved_reasons[0], 189),
                (approved_reasons[1], 40),
                (approved_reasons[2], 19),
                (approved_reasons[3], 20),
                (approved_reasons[4], 1),
            ])
        );
    }

    /// A catalog item form: object main attribute owned by a catalog.
    fn item_form() -> FormEventContext {
        FormEventContext {
            definition: FormDefinitionKind::Regular,
            direct_part_writable: true,
            main_attribute: MainAttributeKind::PersistentObject,
            main_attribute_type: Some("cfg:CatalogObject.Валюты".to_string()),
            main_attribute_provenance: MainAttributeProvenance::DirectForm,
            main_attribute_name: Some("Объект".to_string()),
            metadata_owner: Some(NodeKind::Catalog),
        }
    }

    fn list_form() -> FormEventContext {
        FormEventContext {
            main_attribute: MainAttributeKind::DynamicList,
            main_attribute_type: Some("cfg:DynamicList".to_string()),
            main_attribute_name: Some("Список".to_string()),
            ..item_form()
        }
    }

    fn events_of(owner: FormEventOwnerKind) -> Vec<FormEventParameterForms> {
        form_owner_event_parameters_8_3_27(&item_form(), owner, Some("Объект.Строки"))
    }

    fn forms_of(owner: FormEventOwnerKind, event: &str) -> Vec<Vec<String>> {
        events_of(owner)
            .into_iter()
            .find(|candidate| candidate.event_id == event)
            .map(|candidate| candidate.forms)
            .unwrap_or_default()
    }

    fn parameters(names: &[&str]) -> Vec<MethodParameter> {
        names
            .iter()
            .map(|name| MethodParameter {
                name: (*name).to_string(),
                optional: false,
            })
            .collect()
    }

    fn check(
        owner: FormEventOwnerKind,
        event: &str,
        handler: &[MethodParameter],
    ) -> Option<HandlerSignatureFinding> {
        foreign_handler_signature_8_3_27(
            &events_of(owner),
            event,
            &handler
                .iter()
                .map(|parameter| (parameter.name.as_str(), parameter.optional))
                .collect::<Vec<_>>(),
        )
    }

    const INPUT: FormEventOwnerKind = FormEventOwnerKind::Element(FormElementKind::InputField);

    #[test]
    fn item_parameter_follows_the_help_and_the_dump_evidence() {
        assert_eq!(
            forms_of(INPUT, "ChoiceProcessing"),
            [[
                "Элемент",
                "ВыбранноеЗначение",
                "ДополнительныеДанные",
                "ВыборДобавлением",
                "СтандартнаяОбработка"
            ]]
        );
        assert_eq!(
            forms_of(FormEventOwnerKind::Form, "ChoiceProcessing"),
            [["ВыбранноеЗначение", "ИсточникВыбора"]]
        );
        let table = FormEventOwnerKind::Table;
        assert_eq!(
            forms_of(table, "NewWriteProcessing"),
            [["НовыйОбъект", "Источник", "СтандартнаяОбработка"]]
        );
        // Dynamic-list table events exist only for a table bound to the list.
        let dynamic = form_owner_event_parameters_8_3_27(&list_form(), table, Some("Список"));
        let dynamic_forms = |event: &str| {
            dynamic
                .iter()
                .find(|candidate| candidate.event_id == event)
                .unwrap()
                .clone()
        };
        assert_eq!(
            dynamic_forms("URLGetProcessing").forms[0],
            [
                "Элемент",
                "Ключ",
                "Представление",
                "НавигационнаяСсылка",
                "СтандартнаяОбработка"
            ]
        );
        assert_eq!(
            dynamic_forms("OnUpdateUserSettingSetAtServer").forms,
            [["Элемент", "СтандартнаяОбработка"]]
        );
        let on_get_data = dynamic_forms("OnGetDataAtServer");
        assert_eq!(on_get_data.forms, [["ИмяЭлемента", "Настройки", "Строки"]]);
        assert_eq!(on_get_data.handler_ru, "ПриПолученииДанныхНаСервере");
        assert!(forms_of(table, "OnGetDataAtServer").is_empty());
        assert_eq!(
            forms_of(
                FormEventOwnerKind::Element(FormElementKind::LabelDecoration),
                "Click"
            ),
            [vec!["Элемент"], vec!["Элемент", "СтандартнаяОбработка"]]
        );
        assert!(events_of(FormEventOwnerKind::Element(
            FormElementKind::ExtendedTooltip
        ))
        .is_empty());
    }

    #[test]
    fn handler_signature_puts_the_item_first_only_where_the_platform_passes_it() {
        let signature = |context: &FormEventContext,
                         owner: FormEventOwnerKind,
                         data_path: Option<&str>,
                         event: &str| {
            unica_forms::events::form_event_catalog_8_3_27(context, owner, data_path)
                .into_iter()
                .find(|spec| spec.event_id == event)
                .map(|spec| form_event_handler_signature_8_3_27(owner, spec))
                .unwrap_or_default()
        };
        let item = |owner, event| signature(&item_form(), owner, Some("Объект.Строки"), event);
        assert_eq!(item(INPUT, "OnChange"), "Процедура ПриИзменении(Элемент)");
        assert_eq!(
            item(
                FormEventOwnerKind::Column(FormElementKind::InputField),
                "EditTextChange"
            ),
            "Процедура ИзменениеТекстаРедактирования(Элемент, Текст, СтандартнаяОбработка)"
        );
        assert_eq!(
            item(FormEventOwnerKind::Table, "BeforeAddRow"),
            "Процедура ПередНачаломДобавления(Элемент, Отказ, Копирование, Родитель, ЭтоГруппа, Параметр)"
        );
        assert_eq!(
            item(FormEventOwnerKind::Table, "NewWriteProcessing"),
            "Процедура ОбработкаЗаписиНового(НовыйОбъект, Источник, СтандартнаяОбработка)"
        );
        assert_eq!(
            item(FormEventOwnerKind::Form, "OnOpen"),
            "Процедура ПриОткрытии(Отказ)"
        );
        assert_eq!(
            item(FormEventOwnerKind::Command, "Execute"),
            "Процедура ОбработкаКоманды(Команда)"
        );
        let list = |event| {
            signature(
                &list_form(),
                FormEventOwnerKind::Table,
                Some("Список"),
                event,
            )
        };
        assert_eq!(
            list("OnUpdateUserSettingSetAtServer"),
            "Процедура ПриОбновленииСоставаПользовательскихНастроекНаСервере(Элемент, СтандартнаяОбработка)"
        );
        assert_eq!(
            list("OnGetDataAtServer"),
            "Процедура ПриПолученииДанныхНаСервере(ИмяЭлемента, Настройки, Строки)"
        );
        // The check compares a handler with the same list.
        assert_eq!(
            forms_of(INPUT, "OnChange")[0],
            form_event_handler_parameters_8_3_27(
                INPUT,
                unica_forms::events::form_event_catalog_8_3_27(&item_form(), INPUT, None)
                    .into_iter()
                    .find(|spec| spec.event_id == "OnChange")
                    .unwrap()
            )
            .unwrap()
        );
    }

    #[test]
    fn form_kind_selects_one_variant_of_each_event() {
        // The object form of a catalog writes; a form without a persistent
        // main attribute has no write events at all.
        assert_eq!(
            forms_of(FormEventOwnerKind::Form, "BeforeWrite"),
            [["Отказ", "ПараметрыЗаписи"]]
        );
        let plain = FormEventContext {
            main_attribute: MainAttributeKind::Other,
            main_attribute_type: Some("cfg:DataProcessorObject.Обработка".to_string()),
            metadata_owner: Some(NodeKind::DataProcessor),
            ..item_form()
        };
        let events = form_owner_event_parameters_8_3_27(&plain, FormEventOwnerKind::Form, None);
        assert!(events.iter().all(|event| event.event_id != "BeforeWrite"));
        let task = FormEventContext {
            main_attribute_type: Some("cfg:TaskObject.Задача".to_string()),
            metadata_owner: Some(NodeKind::Task),
            ..item_form()
        };
        let activation = form_owner_event_parameters_8_3_27(&task, FormEventOwnerKind::Form, None)
            .into_iter()
            .find(|event| event.event_id == "ActivationProcessing")
            .unwrap();
        assert_eq!(activation.forms, [Vec::<String>::new()]);
    }

    #[test]
    fn foreign_signature_needs_a_strictly_better_unambiguous_match_and_another_count() {
        let start_choice = [
            "Элемент",
            "ДанныеВыбора",
            "ВыборДобавлением",
            "СтандартнаяОбработка",
        ];
        assert_eq!(
            check(INPUT, "AutoComplete", &parameters(&start_choice)),
            Some(HandlerSignatureFinding::ForeignEvent {
                bound_ru: "АвтоПодбор".to_string(),
                expected: forms_of(INPUT, "AutoComplete")[0].clone(),
                other_ru: "НачалоВыбора".to_string(),
                other: forms_of(INPUT, "StartChoice")[0].clone(),
            })
        );
        // Legacy shorter forms of the bound event.
        for names in [
            &["Элемент", "ВыбранноеЗначение", "СтандартнаяОбработка"][..],
            &["Элемент", "Результат", "СтандартнаяОбработка"][..],
            &[][..],
        ] {
            assert_eq!(
                check(INPUT, "ChoiceProcessing", &parameters(names)),
                None,
                "{names:?}"
            );
        }
        // One shared name is weak evidence.
        assert_eq!(
            check(INPUT, "OnChange", &parameters(&["ДанныеВыбора"])),
            None
        );
        let mut single = parameters(&["Ожидание", "Прочее"]);
        single[1].optional = true;
        assert_eq!(check(INPUT, "OnChange", &single), None);
        // Equal scores without one count-matching leader are a tie.
        let mut tied = parameters(&["Элемент", "СтандартнаяОбработка", "Прочее"]);
        tied[1].optional = true;
        tied[2].optional = true;
        assert_eq!(check(INPUT, "OnChange", &tied), None);
        // An equally good other event does not outweigh the bound one.
        assert_eq!(
            check(
                INPUT,
                "ChoiceProcessing",
                &parameters(&["Элемент", "ВыборДобавлением", "СтандартнаяОбработка"])
            ),
            None
        );
        // The same count as the bound event is a rename, not another event.
        assert_eq!(
            check(
                FormEventOwnerKind::Table,
                "ValueChoice",
                &parameters(&["Элемент", "ВыбранноеЗначение", "СтандартнаяОбработка"])
            ),
            None
        );
        assert_eq!(
            check(INPUT, "NoSuchEvent", &parameters(&start_choice)),
            None
        );
    }

    #[test]
    fn a_tie_is_broken_by_the_only_leader_with_the_handler_parameter_count() {
        // `Элемент`, `Текст` and `СтандартнаяОбработка` score 3 with
        // EditTextChange, AutoComplete and TextEditEnd; only EditTextChange
        // has three parameters, so it wins the tie.
        let handler = parameters(&["Элемент", "Текст", "СтандартнаяОбработка"]);
        assert_eq!(
            check(INPUT, "OnChange", &handler),
            Some(HandlerSignatureFinding::ForeignEvent {
                bound_ru: "ПриИзменении".to_string(),
                expected: vec!["Элемент".to_string()],
                other_ru: "ИзменениеТекстаРедактирования".to_string(),
                other: forms_of(INPUT, "EditTextChange")[0].clone(),
            })
        );
    }

    #[test]
    fn extra_required_parameters_are_counted_against_every_known_list() {
        assert_eq!(
            check(
                FormEventOwnerKind::Form,
                "URLProcessing",
                &parameters(&["Элемент", "Ссылка", "СтандартнаяОбработка"]),
            ),
            Some(HandlerSignatureFinding::ExtraRequired {
                bound_ru: "ОбработкаНавигационнойСсылки".to_string(),
                required: 3,
                expected: forms_of(FormEventOwnerKind::Form, "URLProcessing")[0].clone(),
            })
        );
        let mut optional = parameters(&["Ссылка", "СтандартнаяОбработка", "Режим"]);
        optional[2].optional = true;
        assert_eq!(
            check(FormEventOwnerKind::Form, "URLProcessing", &optional),
            None
        );
        // The undocumented two-parameter click of a label decoration is known.
        assert_eq!(
            check(
                FormEventOwnerKind::Element(FormElementKind::LabelDecoration),
                "Click",
                &parameters(&["Элемент", "СтандартнаяОбработка"])
            ),
            None
        );
        assert!(check(
            FormEventOwnerKind::Table,
            "NewWriteProcessing",
            &parameters(&["Элемент", "НовыйОбъект", "Источник", "СтандартнаяОбработка"])
        )
        .is_some());
    }
}
