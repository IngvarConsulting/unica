use roxmltree::{Document, Node};

use unica_metadata::event::FormCallType;
use unica_source_designer::owner::{
    EventOwnerDescriptorExpectation, EventOwnerDescriptorProof, EventOwnerEvidencePlan,
};

use super::*;
use unica_bsl::projection::{EventProjection, EventState, MethodKind};
use unica_metadata::event::BindingFact;
use unica_workspace::apply::ApplyStagedState;

use unica_bindings::events::{EventImplementationOwner, EventImplementationShape};
use unica_bsl::event_text::RequiredEventDirective;

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use unica_address::address::QualifiedAddress;
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_forms::event_targets::{PropertyEventOwner, PropertyEventOwnerKind};
use unica_source_contract::SourceSetKind;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;

fn shape(
    owner: EventImplementationOwner,
    handler: &str,
    signature: &str,
) -> EventImplementationShape {
    EventImplementationShape {
        projection: EventProjection {
            at: "main:CommonForm.Main.Event.OnOpen".to_string(),
            event_id: "OnOpen".to_string(),
            state: EventState::Available,
            signature: signature.to_string(),
            contexts: vec!["thinClient".to_string()],
            binding: BindingFact::Property,
            handler: handler.to_string(),
            handler_en: String::new(),
            implementation_at: None,
            call_type: None,
            handlers: Vec::new(),
            can: Vec::new(),
        },
        owner,
        method_kind: MethodKind::Procedure,
        handler: handler.to_string(),
        implementation_at: format!("main:CommonForm.Main.Module.Form.Method.{handler}"),
        signature: signature.to_string(),
        directive: RequiredEventDirective::Client,
        region: match owner {
            EventImplementationOwner::Form => "ОбработчикиСобытийФормы",
            _ => "ОбработчикиСобытий",
        },
    }
}

fn form_source() -> PropertyEventSource {
    let form_at = QualifiedAddress::parse("main:CommonForm.Main").unwrap();
    PropertyEventSource {
        event_at: QualifiedAddress::parse("main:CommonForm.Main.Event.OnOpen").unwrap(),
        form_at: form_at.clone(),
        form_target: MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "CommonForm.Main")
            .unwrap(),
        form_xml_relative: PathBuf::from("CommonForms/Main/Ext/Form.xml"),
        module_relative: PathBuf::from("CommonForms/Main/Ext/Form/Module.bsl"),
        module_at: QualifiedAddress::parse("main:CommonForm.Main.Module.Form").unwrap(),
        owner_chain: vec![PropertyEventOwner {
            kind: PropertyEventOwnerKind::Form,
            at: form_at,
        }],
        descriptor_requirements: vec![PathBuf::from("CommonForms/Main.xml")],
        owner_evidence: EventOwnerEvidencePlan {
            source_set_kind: SourceSetKind::Configuration,
            descriptors: Vec::new(),
        },
    }
}

fn temp_root(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "unica-event-b1b-{name}-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn staged(root: &std::path::Path) -> ApplyStagedState {
    unica_workspace::test_support::staged_state(
        &std::fs::canonicalize(root).unwrap(),
        ProviderDeadline::from_budget(Duration::from_secs(5)),
        CancellationToken::new(),
    )
    .unwrap()
}

fn common_form_owner_plan() -> EventOwnerEvidencePlan {
    EventOwnerEvidencePlan {
        source_set_kind: SourceSetKind::Configuration,
        descriptors: vec![
            EventOwnerDescriptorExpectation {
                relative: PathBuf::from("Configuration.xml"),
                proof: EventOwnerDescriptorProof::SourceSet,
                expected_kind: "Configuration".to_string(),
                expected_name: None,
                registered_by: None,
            },
            EventOwnerDescriptorExpectation {
                relative: PathBuf::from("CommonForms/Main.xml"),
                proof: EventOwnerDescriptorProof::Metadata,
                expected_kind: "CommonForm".to_string(),
                expected_name: Some("Main".to_string()),
                registered_by: Some(0),
            },
        ],
    }
}

fn write_catalog_form_fixture(root: &std::path::Path) -> Vec<u8> {
    const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
    std::fs::create_dir_all(root.join("Catalogs/Products/Forms/Main/Ext")).unwrap();
    std::fs::write(
            root.join("Configuration.xml"),
            format!(
                "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Configuration><Properties><Name>Fixture</Name></Properties><ChildObjects><Catalog>Products</Catalog></ChildObjects></Configuration></MetaDataObject>"
            ),
        )
        .unwrap();
    std::fs::write(
            root.join("Catalogs/Products.xml"),
            format!(
                "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Catalog><Properties><Name>Products</Name></Properties><ChildObjects><Form>Main</Form></ChildObjects></Catalog></MetaDataObject>"
            ),
        )
        .unwrap();
    std::fs::write(
            root.join("Catalogs/Products/Forms/Main.xml"),
            format!(
                "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Form><Properties><Name>Main</Name></Properties></Form></MetaDataObject>"
            ),
        )
        .unwrap();
    let form = concat!(
        "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
        "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\r\n",
        "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\r\n",
        "\t<ChildItems/>\r\n",
        "\t<Commands/>\r\n",
        "</Form>\r\n"
    )
    .as_bytes()
    .to_vec();
    std::fs::write(
        root.join("Catalogs/Products/Forms/Main/Ext/Form.xml"),
        &form,
    )
    .unwrap();
    form
}

fn write_extension_catalog_form_fixture(root: &std::path::Path, direct_part: bool) -> Vec<u8> {
    const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
    std::fs::create_dir_all(root.join("Catalogs/Products/Forms/Main/Ext")).unwrap();
    std::fs::write(
            root.join("Configuration.xml"),
            format!(
                "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Configuration><Properties><Name>FixtureExtension</Name><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose></Properties><ChildObjects><Catalog>Products</Catalog></ChildObjects></Configuration></MetaDataObject>"
            ),
        )
        .unwrap();
    std::fs::write(
            root.join("Catalogs/Products.xml"),
            format!(
                "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Catalog><Properties><Name>Products</Name></Properties><ChildObjects><Form>Main</Form></ChildObjects></Catalog></MetaDataObject>"
            ),
        )
        .unwrap();
    std::fs::write(
            root.join("Catalogs/Products/Forms/Main.xml"),
            format!(
                "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Form><Properties><Name>Main</Name></Properties></Form></MetaDataObject>"
            ),
        )
        .unwrap();
    let direct = if direct_part {
        "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\r\n\t<ChildItems/>\r\n\t<Commands/>\r\n"
    } else {
        ""
    };
    let form = format!(
            "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\r\n{direct}\t<BaseForm version=\"2.20\"><AutoCommandBar name=\"BaseBar\" id=\"-1\"/></BaseForm>\r\n</Form>\r\n"
        )
        .into_bytes();
    std::fs::write(
        root.join("Catalogs/Products/Forms/Main/Ext/Form.xml"),
        &form,
    )
    .unwrap();
    form
}

fn write_external_fixture(root: &std::path::Path, kind: &str, name: &str) {
    const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
    std::fs::write(
            root.join(format!("{name}.xml")),
            format!(
                "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><{kind}><Properties><Name>{name}</Name></Properties><ChildObjects><Command>Run</Command></ChildObjects></{kind}></MetaDataObject>"
            ),
        )
        .unwrap();
    std::fs::create_dir_all(root.join(format!("{name}/Commands"))).unwrap();
    std::fs::write(
            root.join(format!("{name}/Commands/Run.xml")),
            format!(
                "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Command><Properties><Name>Run</Name></Properties></Command></MetaDataObject>"
            ),
        )
        .unwrap();
}

#[test]
fn owner_proof_distinguishes_unregistered_from_registered_inconsistent_descriptor() {
    const HEADER: &str =
        "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">";
    let root = temp_root("owner-proof");
    std::fs::create_dir_all(root.join("CommonForms")).unwrap();
    std::fs::write(
        root.join("Configuration.xml"),
        format!("{HEADER}<Configuration><ChildObjects/></Configuration></MetaDataObject>"),
    )
    .unwrap();
    std::fs::write(
            root.join("CommonForms/Main.xml"),
            format!("{HEADER}<CommonForm><Properties><Name>Main</Name></Properties></CommonForm></MetaDataObject>"),
        )
        .unwrap();
    let error =
        prove_owner_plan(&mut staged(&root).proposal(), &common_form_owner_plan()).unwrap_err();
    assert_eq!(error.kind(), ApplyPlanErrorKind::NotFound);

    std::fs::write(
            root.join("Configuration.xml"),
            format!("{HEADER}<Configuration><ChildObjects><CommonForm>Main</CommonForm></ChildObjects></Configuration></MetaDataObject>"),
        )
        .unwrap();
    std::fs::write(root.join("CommonForms/Main.xml"), b"not xml").unwrap();
    let error =
        prove_owner_plan(&mut staged(&root).proposal(), &common_form_owner_plan()).unwrap_err();
    assert_eq!(error.kind(), ApplyPlanErrorKind::ProviderUnavailable);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn staged_property_planner_composes_two_events_and_poisoned_batch_publishes_nothing() {
    let root = temp_root("staged-planner");
    let original_form = write_catalog_form_fixture(&root);
    let operations = ["OnOpen", "OnClose"].map(|event| EventImplementArgs {
        at: QualifiedAddress::parse(&format!("main:Catalog.Products.Form.Main.Event.{event}"))
            .unwrap(),
        call_type: None,
    });
    let (planned, effects) = plan_event_implement_batch_owned_for_test(
        staged(&root),
        "main",
        SourceSetKind::Configuration,
        PlatformProfile::v8_3_27(),
        &operations,
    )
    .unwrap();
    let changes = planned.planned_changes();
    assert_eq!(changes.len(), 2);
    let form_after = changes
        .iter()
        .find(|change| change.relative_path.ends_with("Form.xml"))
        .and_then(|change| match &change.current {
            unica_source_contract::proposal::StagedFileState::Bytes(bytes) => Some(bytes),
            _ => None,
        })
        .unwrap();
    let form_after = String::from_utf8(form_after.clone()).unwrap();
    assert!(form_after.contains("name=\"OnOpen\""));
    assert!(form_after.contains("name=\"OnClose\""));
    assert_eq!(effects.events().len(), 2, "Form+Module effects deduplicate");
    assert_eq!(effects.events()[0].kind, DomainEventKind::FormChanged);
    assert_eq!(effects.events()[1].kind, DomainEventKind::ModuleChanged);
    assert_eq!(
        std::fs::read(root.join("Catalogs/Products/Forms/Main/Ext/Form.xml")).unwrap(),
        original_form,
        "planning never publishes source bytes"
    );
    assert!(!root
        .join("Catalogs/Products/Forms/Main/Ext/Form/Module.bsl")
        .exists());

    let duplicate = [operations[0].clone(), operations[0].clone()];
    let error = plan_event_implement_batch_owned_for_test(
        staged(&root),
        "main",
        SourceSetKind::Configuration,
        PlatformProfile::v8_3_27(),
        &duplicate,
    )
    .unwrap_err();
    assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
    assert_eq!(
        std::fs::read(root.join("Catalogs/Products/Forms/Main/Ext/Form.xml")).unwrap(),
        original_form
    );
    assert!(!root
        .join("Catalogs/Products/Forms/Main/Ext/Form/Module.bsl")
        .exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn staged_planner_extends_compact_existing_events_inside_root_and_arbitrary_nested_owners() {
    struct Case {
        name: &'static str,
        form: &'static str,
        target: &'static str,
        owner_marker: &'static str,
        owner_close: &'static str,
        owner_name: Option<&'static str>,
        existing_event: &'static str,
        target_event: &'static str,
        handler: &'static str,
    }

    let cases = [
            Case {
                name: "compact-root-events",
                form: concat!(
                    "\u{feff}<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\n",
                    "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\n",
                    "\t<Events><Event name=\"OnClose\">ExistingClose</Event></Events>\n",
                    "\t<ChildItems/>\n",
                    "\t<Commands/>\n",
                    "</Form>"
                ),
                target: "main:Catalog.Products.Form.Main.Event.OnOpen",
                owner_marker: "<Form ",
                owner_close: "</Form>",
                owner_name: None,
                existing_event: "OnClose",
                target_event: "OnOpen",
                handler: "ПриОткрытии",
            },
            Case {
                name: "compact-arbitrary-nested-events",
                form: concat!(
                    "\u{feff}<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\n",
                    "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\n",
                    "\t<ChildItems>\n",
                    "\t\t<UsualGroup name=\"Outer\" id=\"1\"><ChildItems><Table name=\"Goods\" id=\"2\"><Events><Event name=\"Selection\">ExistingSelection</Event></Events><ChildItems/></Table></ChildItems></UsualGroup>\n",
                    "\t</ChildItems>\n",
                    "\t<Commands/>\n",
                    "</Form>"
                ),
                target: "main:Catalog.Products.Form.Main.Item.Outer.Item.Goods.Event.BeforeAddRow",
                owner_marker: "<Table name=\"Goods\"",
                owner_close: "</Table>",
                owner_name: Some("Goods"),
                existing_event: "Selection",
                target_event: "BeforeAddRow",
                handler: "GoodsПередНачаломДобавления",
            },
        ];

    for case in cases {
        let root = temp_root(case.name);
        write_catalog_form_fixture(&root);
        let form_path = root.join("Catalogs/Products/Forms/Main/Ext/Form.xml");
        std::fs::write(&form_path, case.form.as_bytes()).unwrap();
        let (planned, _) = plan_event_implement_batch_owned_for_test(
            staged(&root),
            "main",
            SourceSetKind::Configuration,
            PlatformProfile::v8_3_27(),
            &[EventImplementArgs {
                at: QualifiedAddress::parse(case.target).unwrap(),
                call_type: None,
            }],
        )
        .expect("compact direct Events must remain the exact projected owner");
        let form_after = planned
            .planned_changes()
            .iter()
            .find(|change| change.relative_path.ends_with("Form.xml"))
            .and_then(|change| match &change.current {
                unica_source_contract::proposal::StagedFileState::Bytes(bytes) => {
                    Some(String::from_utf8(bytes.clone()).unwrap())
                }
                _ => None,
            })
            .unwrap();

        let document = Document::parse(form_after.trim_start_matches('\u{feff}')).unwrap();
        let owner = match case.owner_name {
            None => document.root_element(),
            Some(name) => document
                .descendants()
                .find(|node| node.is_element() && node.attribute("name") == Some(name))
                .unwrap(),
        };
        let events = owner
            .children()
            .filter(Node::is_element)
            .filter(|node| node.tag_name().name() == "Events")
            .collect::<Vec<_>>();
        assert_eq!(events.len(), 1, "{}", case.name);
        let direct_names = events[0]
            .children()
            .filter(Node::is_element)
            .filter(|node| node.tag_name().name() == "Event")
            .filter_map(|node| node.attribute("name"))
            .collect::<Vec<_>>();
        assert_eq!(
            direct_names,
            vec![case.existing_event, case.target_event],
            "{}",
            case.name
        );
        assert!(form_after.contains(case.handler), "{}", case.name);

        let before_start = case.form.find(case.owner_marker).unwrap();
        let after_start = form_after.find(case.owner_marker).unwrap();
        assert_eq!(
            &case.form[..before_start],
            &form_after[..after_start],
            "bytes before the exact owner changed in {}",
            case.name
        );
        let before_end = case.form[before_start..]
            .find(case.owner_close)
            .map(|relative| before_start + relative + case.owner_close.len())
            .unwrap();
        let after_end = form_after[after_start..]
            .find(case.owner_close)
            .map(|relative| after_start + relative + case.owner_close.len())
            .unwrap();
        assert_eq!(
            &case.form[before_end..],
            &form_after[after_end..],
            "bytes after the exact owner changed in {}",
            case.name
        );
        assert_eq!(std::fs::read_to_string(&form_path).unwrap(), case.form);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn staged_regular_owner_matrix_composes_exact_form_and_module_postimages() {
    let root = temp_root("regular-owner-matrix");
    write_catalog_form_fixture(&root);
    let form_path = root.join("Catalogs/Products/Forms/Main/Ext/Form.xml");
    let form = concat!(
            "\u{feff}<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\n",
            "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\n",
            "\t<ChildItems>\n",
            "\t\t<InputField name=\"Field\" id=\"1\"/>\n",
            "\t\t<Table name=\"Goods\" id=\"2\"><ChildItems><InputField name=\"Quantity\" id=\"3\"/></ChildItems></Table>\n",
            "\t</ChildItems>\n",
            "\t<Commands><Command name=\"Заполнить\"><Title>keep</Title></Command></Commands>\n",
            "</Form>"
        )
        .as_bytes()
        .to_vec();
    std::fs::write(&form_path, &form).unwrap();
    let targets = [
        "main:Catalog.Products.Form.Main.Event.OnOpen",
        "main:Catalog.Products.Form.Main.Item.Field.Event.OnChange",
        "main:Catalog.Products.Form.Main.Item.Goods.Event.BeforeAddRow",
        "main:Catalog.Products.Form.Main.Item.Goods.Item.Quantity.Event.OnChange",
        "main:Catalog.Products.Form.Main.Command.Заполнить.Event.Execute",
    ];
    let operations = targets.map(|target| EventImplementArgs {
        at: QualifiedAddress::parse(target).unwrap(),
        call_type: None,
    });
    let (planned, effects) = plan_event_implement_batch_owned_for_test(
        staged(&root),
        "main",
        SourceSetKind::Configuration,
        PlatformProfile::v8_3_27(),
        &operations,
    )
    .unwrap();
    assert_eq!(planned.planned_changes().len(), 2);
    let form_after = planned
        .planned_changes()
        .iter()
        .find(|change| change.relative_path.ends_with("Form.xml"))
        .and_then(|change| match &change.current {
            unica_source_contract::proposal::StagedFileState::Bytes(bytes) => {
                Some(String::from_utf8(bytes.clone()).unwrap())
            }
            _ => None,
        })
        .unwrap();
    for binding in [
        "<Event name=\"OnOpen\">ПриОткрытии</Event>",
        "<Event name=\"OnChange\">FieldПриИзменении</Event>",
        "<Event name=\"BeforeAddRow\">GoodsПередНачаломДобавления</Event>",
        "<Event name=\"OnChange\">QuantityПриИзменении</Event>",
        "<Action>ЗаполнитьОбработкаКоманды</Action>",
    ] {
        assert!(form_after.contains(binding), "missing binding {binding}");
    }
    assert!(form_after.starts_with('\u{feff}'));
    assert!(!form_after.ends_with('\n') && !form_after.ends_with('\r'));
    let module_after = planned
        .planned_changes()
        .iter()
        .find(|change| change.relative_path.ends_with("Module.bsl"))
        .and_then(|change| match &change.current {
            unica_source_contract::proposal::StagedFileState::Bytes(bytes) => {
                Some(String::from_utf8(bytes.clone()).unwrap())
            }
            _ => None,
        })
        .unwrap();
    assert!(module_after.contains("Процедура ЗаполнитьОбработкаКоманды(Команда)"));
    assert!(!module_after.contains("CommandProcessing"));
    assert_eq!(effects.events().len(), 2);
    assert_eq!(std::fs::read(&form_path).unwrap(), form);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn borrowed_available_accepts_all_call_types_and_base_form_only_stays_byte_exact() {
    for call_type in [
        FormCallType::Before,
        FormCallType::After,
        FormCallType::Override,
    ] {
        let root = temp_root(call_type.as_str());
        let original = write_extension_catalog_form_fixture(&root, true);
        let (planned, _) = plan_event_implement_batch_owned_for_test(
            staged(&root),
            "main",
            SourceSetKind::Extension,
            PlatformProfile::v8_3_27(),
            &[EventImplementArgs {
                at: QualifiedAddress::parse("main:Catalog.Products.Form.Main.Event.OnOpen")
                    .unwrap(),
                call_type: Some(call_type),
            }],
        )
        .unwrap();
        let form_after = planned
            .planned_changes()
            .iter()
            .find(|change| change.relative_path.ends_with("Form.xml"))
            .and_then(|change| match &change.current {
                unica_source_contract::proposal::StagedFileState::Bytes(bytes) => {
                    Some(String::from_utf8(bytes.clone()).unwrap())
                }
                _ => None,
            })
            .unwrap();
        assert!(form_after.starts_with('\u{feff}'));
        assert!(form_after.ends_with("\r\n"));
        assert!(form_after.contains(&format!(
            "<Event name=\"OnOpen\" callType=\"{}\">ПриОткрытии</Event>",
            call_type.as_str()
        )));
        assert_eq!(
            std::fs::read(root.join("Catalogs/Products/Forms/Main/Ext/Form.xml")).unwrap(),
            original
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    let root = temp_root("base-form-only");
    let original = write_extension_catalog_form_fixture(&root, false);
    let error = plan_event_implement_batch_owned_for_test(
        staged(&root),
        "main",
        SourceSetKind::Extension,
        PlatformProfile::v8_3_27(),
        &[EventImplementArgs {
            at: QualifiedAddress::parse("main:Catalog.Products.Form.Main.Event.OnOpen").unwrap(),
            call_type: Some(FormCallType::After),
        }],
    )
    .unwrap_err();
    assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
    assert_eq!(
        std::fs::read(root.join("Catalogs/Products/Forms/Main/Ext/Form.xml")).unwrap(),
        original
    );
    assert!(!root
        .join("Catalogs/Products/Forms/Main/Ext/Form/Module.bsl")
        .exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn staged_missing_property_keeps_form_byte_exact_and_platform_creates_only_module() {
    let root = temp_root("missing-and-platform");
    write_catalog_form_fixture(&root);
    let form_path = root.join("Catalogs/Products/Forms/Main/Ext/Form.xml");
    let missing_form = concat!(
        "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
        "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\r\n",
        "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\r\n",
        "\t<Events><Event name=\"OnOpen\">ExistingOpen</Event></Events>\r\n",
        "\t<ChildItems/>\r\n",
        "\t<Commands/>\r\n",
        "</Form>\r\n"
    )
    .as_bytes()
    .to_vec();
    std::fs::write(&form_path, &missing_form).unwrap();
    let missing = [EventImplementArgs {
        at: QualifiedAddress::parse("main:Catalog.Products.Form.Main.Event.OnOpen").unwrap(),
        call_type: None,
    }];
    let (planned, effects) = plan_event_implement_batch_owned_for_test(
        staged(&root),
        "main",
        SourceSetKind::Configuration,
        PlatformProfile::v8_3_27(),
        &missing,
    )
    .unwrap();
    let changes = planned.planned_changes();
    assert_eq!(changes.len(), 1);
    assert!(changes[0].relative_path.ends_with("Module.bsl"));
    assert_eq!(effects.events().len(), 1);
    assert_eq!(effects.events()[0].kind, DomainEventKind::ModuleChanged);
    assert_eq!(std::fs::read(&form_path).unwrap(), missing_form);

    let clean_root = temp_root("platform-module");
    write_catalog_form_fixture(&clean_root);
    let platform = [EventImplementArgs {
        at: QualifiedAddress::parse("main:Catalog.Products.Module.Object.Event.BeforeWrite")
            .unwrap(),
        call_type: None,
    }];
    let (planned, effects) = plan_event_implement_batch_owned_for_test(
        staged(&clean_root),
        "main",
        SourceSetKind::Configuration,
        PlatformProfile::v8_3_27(),
        &platform,
    )
    .unwrap();
    let changes = planned.planned_changes();
    assert_eq!(changes.len(), 1);
    let bytes = match &changes[0].current {
        unica_source_contract::proposal::StagedFileState::Bytes(bytes) => bytes,
        _ => panic!("Platform module must be created"),
    };
    assert!(bytes.starts_with(b"\xef\xbb\xbf"));
    assert!(String::from_utf8_lossy(bytes).contains("Процедура ПередЗаписью"));
    assert_eq!(effects.events().len(), 1);
    assert_eq!(effects.events()[0].kind, DomainEventKind::ModuleChanged);
    assert!(!clean_root
        .join("Catalogs/Products/Ext/ObjectModule.bsl")
        .exists());
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(clean_root).unwrap();
}

#[test]
fn unsupported_frozen_catalog_event_is_provider_unavailable_not_malformed_source() {
    let root = temp_root("unsupported-platform-event");
    write_catalog_form_fixture(&root);
    let error = plan_event_implement_batch_owned_for_test(
        staged(&root),
        "main",
        SourceSetKind::Configuration,
        PlatformProfile::v8_3_27(),
        &[EventImplementArgs {
            at: QualifiedAddress::parse("main:Catalog.Products.Module.Object.Event.DoesNotExist")
                .unwrap(),
            call_type: None,
        }],
    )
    .unwrap_err();
    assert_eq!(error.kind(), ApplyPlanErrorKind::ProviderUnavailable);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn external_epf_and_erf_platform_owners_plan_only_proved_module_postimages() {
    let epf = temp_root("external-epf");
    write_external_fixture(&epf, "ExternalDataProcessor", "Import");
    let (planned, effects) = plan_event_implement_batch_owned_for_test(
            staged(&epf),
            "epf",
            SourceSetKind::ExternalProcessor,
            PlatformProfile::v8_3_27(),
            &[
                EventImplementArgs {
                    at: QualifiedAddress::parse(
                        "epf:ExternalDataProcessor.Import.Module.Object.Event.FillCheckProcessing",
                    )
                    .unwrap(),
                    call_type: None,
                },
                EventImplementArgs {
                    at: QualifiedAddress::parse(
                        "epf:ExternalDataProcessor.Import.Command.Run.Module.Command.Event.CommandProcessing",
                    )
                    .unwrap(),
                    call_type: None,
                },
            ],
        )
        .unwrap();
    assert_eq!(planned.planned_changes().len(), 2);
    assert_eq!(effects.events().len(), 2);
    assert!(!epf.join("Import/Ext/ObjectModule.bsl").exists());
    assert!(!epf
        .join("Import/Commands/Run/Ext/CommandModule.bsl")
        .exists());
    std::fs::remove_dir_all(epf).unwrap();

    let erf = temp_root("external-erf");
    write_external_fixture(&erf, "ExternalReport", "Sales");
    let (planned, effects) = plan_event_implement_batch_owned_for_test(
        staged(&erf),
        "erf",
        SourceSetKind::ExternalReport,
        PlatformProfile::v8_3_27(),
        &[EventImplementArgs {
            at: QualifiedAddress::parse(
                "erf:ExternalReport.Sales.Module.Object.Event.OnComposeResult",
            )
            .unwrap(),
            call_type: None,
        }],
    )
    .unwrap();
    assert_eq!(planned.planned_changes().len(), 1);
    assert_eq!(effects.events().len(), 1);
    assert!(!erf.join("Sales/Ext/ObjectModule.bsl").exists());
    std::fs::remove_dir_all(erf).unwrap();
}
