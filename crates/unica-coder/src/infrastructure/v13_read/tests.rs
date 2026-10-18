use super::{
    project_known_suffix, project_typed_payload, resolve_platform_xml_target,
    LogicalViewReadAuthority, MetadataAddress, SourceTarget, TargetKindPolicy,
    PLATFORM_XML_8_3_27_FORMAT_2_20,
};
use crate::application::result_store::ViewCursorStore;
use crate::application::v13::find::{FindRequest, FindResult};
use crate::application::v13::view::{ViewError, ViewReadAuthority, ViewRequest, ViewService};
use crate::domain::address::QualifiedAddress;
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::platform_profile::PlatformProfile;
use crate::domain::project_sources::SourceSetKind;
use crate::domain::refusal::RefusalCode;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::logical_tree::route_logical_address;
use crate::infrastructure::platform::filesystem::{
    supports_retained_root_replacement_test, RetainedDirectoryCapability,
};

use crate::infrastructure::v13_read_port::{
    review_clear_revision_identity_hooks, review_set_revision_identity_hooks, ProviderReadAuthority,
};
use serde::Deserialize;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[test]
fn borrowed_common_module_keeps_missing_privileged_unknown() {
    let borrowed = fixture_text("platform_8_3_27/cfe_borrow/extension-common-module.xml");
    let props = super::common_module_properties(borrowed.as_bytes(), SourceSetKind::Extension)
        .expect("platform borrowed CommonModule omits Privileged");
    let serialized = serde_json::to_value(props).unwrap();
    assert_eq!(serialized["privileged"], serde_json::Value::Null);

    let ordinary = fixture_text("platform_8_3_27/cfe_borrow/parent-common-module.xml");
    let missing = ordinary.replace("<Privileged>false</Privileged>", "");
    assert_eq!(
        super::common_module_properties(missing.as_bytes(), SourceSetKind::Configuration)
            .unwrap_err()
            .code(),
        RefusalCode::ProviderUnavailable,
    );
    assert_eq!(
        super::common_module_properties(borrowed.as_bytes(), SourceSetKind::Configuration)
            .unwrap_err()
            .code(),
        RefusalCode::ProviderUnavailable,
    );
    let own_extension = borrowed.replace(
        "<ExtendedConfigurationObject>ac847dc9-e222-45cf-af4a-6fa863c919a8</ExtendedConfigurationObject>",
        "",
    );
    assert!(
        super::common_module_properties(own_extension.as_bytes(), SourceSetKind::Extension)
            .is_err(),
        "an extension's own CommonModule still requires Privileged",
    );
    for (raw, expected) in [("true", true), ("false", false)] {
        let explicit = borrowed.replace(
            "<ReturnValuesReuse>",
            &format!("<Privileged>{raw}</Privileged><ReturnValuesReuse>"),
        );
        let props =
            super::common_module_properties(explicit.as_bytes(), SourceSetKind::Extension).unwrap();
        assert_eq!(props.privileged, Some(expected));
    }
    for raw in ["", "unknown"] {
        let invalid = borrowed.replace(
            "<ReturnValuesReuse>",
            &format!("<Privileged>{raw}</Privileged><ReturnValuesReuse>"),
        );
        assert!(
            super::common_module_properties(invalid.as_bytes(), SourceSetKind::Extension).is_err(),
            "explicit invalid Privileged must not become null",
        );
    }
}

#[test]
fn borrowed_common_module_view_serializes_unknown_privileged_without_losing_contexts() {
    let fixture = RealReaderFixture::new();
    let configuration = fs::read_to_string(fixture.source.join("Configuration.xml")).unwrap();
    write(
        &fixture.source.join("Configuration.xml"),
        &configuration.replace(
            "</ChildObjects>",
            "<CommonModule>CorpusModule</CommonModule></ChildObjects>",
        ),
    );
    write(
        &fixture.source.join("CommonModules/CorpusModule.xml"),
        &fixture_text("platform_8_3_27/cfe_borrow/extension-common-module.xml"),
    );
    write(
        &fixture
            .source
            .join("CommonModules/CorpusModule/Ext/Module.bsl"),
        "Процедура Проверка() Экспорт\nКонецПроцедуры\n",
    );
    let view = fixture
        .extension_view_service()
        .view(ViewRequest::new("main:CommonModule.CorpusModule").unwrap());
    assert!(view.ok, "view must succeed");
    let data = view.data.unwrap();
    assert_eq!(
        data["props"]["commonModule"]["privileged"],
        serde_json::Value::Null
    );
    assert!(data["props"]["commonModule"]["server"].is_boolean());
    assert!(data["branches"].is_array());
}

const COMMON_MODULE_PROPERTY_KEYS: [&str; 8] = [
    "global",
    "clientManagedApplication",
    "server",
    "externalConnection",
    "clientOrdinaryApplication",
    "serverCall",
    "privileged",
    "returnValuesReuse",
];

/// #1304: платформа 8.3.27 выгружает заимствованный общий модуль без любого
/// из восьми свойств. Отсутствие читается как неизвестное, не как значение
/// по умолчанию; у обычного модуля и у собственного модуля расширения оно
/// остаётся ошибкой источника, записанное некорректное значение — тоже.
#[test]
fn borrowed_common_module_reads_platform_absent_properties_as_unknown() {
    let inherited =
        fixture_text("platform_8_3_27/cfe_borrow/extension-common-module-inherited.xml");
    let props = super::common_module_properties(inherited.as_bytes(), SourceSetKind::Extension)
        .expect("platform borrowed CommonModule without properties");
    let serialized = serde_json::to_value(&props).unwrap();
    for key in COMMON_MODULE_PROPERTY_KEYS {
        assert_eq!(serialized[key], serde_json::Value::Null, "{key}");
    }
    assert!(!props.execution_environment_known());

    let environment =
        fixture_text("platform_8_3_27/cfe_borrow/extension-common-module-environment-only.xml");
    let props = super::common_module_properties(environment.as_bytes(), SourceSetKind::Extension)
        .expect("platform borrowed CommonModule without ReturnValuesReuse");
    assert_eq!(props.global, Some(false));
    assert_eq!(props.client_managed_application, Some(false));
    assert_eq!(props.server, Some(true));
    assert_eq!(props.external_connection, Some(true));
    assert_eq!(props.client_ordinary_application, Some(false));
    assert_eq!(props.server_call, Some(true));
    assert_eq!(props.privileged, None);
    assert_eq!(props.return_values_reuse, None);
    assert!(props.execution_environment_known());

    let parent = fixture_text("platform_8_3_27/cfe_borrow/parent-common-module-cache.xml");
    let props =
        super::common_module_properties(parent.as_bytes(), SourceSetKind::Configuration).unwrap();
    assert_eq!(props.return_values_reuse.as_deref(), Some("DuringRequest"));
    assert_eq!(props.privileged, Some(true));
    for (tag, value) in [
        ("ReturnValuesReuse", "DuringRequest"),
        ("Global", "false"),
        ("Server", "true"),
    ] {
        let missing = parent.replace(&format!("<{tag}>{value}</{tag}>"), "");
        assert_ne!(missing, parent, "{tag}");
        let error =
            super::common_module_properties(missing.as_bytes(), SourceSetKind::Configuration)
                .unwrap_err();
        assert_eq!(error.code(), RefusalCode::ProviderUnavailable, "{tag}");
        assert!(error.to_string().contains(tag), "{tag}: {error}");
    }
    assert!(
        super::common_module_properties(inherited.as_bytes(), SourceSetKind::Configuration)
            .is_err(),
        "outside an extension the borrowing marker proves nothing",
    );
    let own_extension = inherited.replace(
        "<ExtendedConfigurationObject>3f6c2a10-5b7e-4c1d-9e2f-0a1b2c3d4e51</ExtendedConfigurationObject>",
        "",
    );
    assert_ne!(own_extension, inherited);
    assert!(
        super::common_module_properties(own_extension.as_bytes(), SourceSetKind::Extension)
            .is_err(),
        "an extension's own CommonModule still requires every property",
    );
    for (tag, original, invalid) in [
        (
            "ReturnValuesReuse",
            "<ServerCall>true</ServerCall>",
            "<ServerCall>true</ServerCall><ReturnValuesReuse></ReturnValuesReuse>",
        ),
        ("Global", "<Global>false</Global>", "<Global>maybe</Global>"),
        ("Server", "<Server>true</Server>", "<Server></Server>"),
    ] {
        let invalid = environment.replace(original, invalid);
        assert_ne!(invalid, environment, "{tag}");
        assert!(
            super::common_module_properties(invalid.as_bytes(), SourceSetKind::Extension).is_err(),
            "explicit invalid {tag} must not become null",
        );
    }
}

/// #1304: заимствованный модуль без свойств читается, его методы доступны.
/// Контексты без флагов среды неизвестны и сериализуются `null`, фильтр по
/// контексту отказывает типизированно; при записанных флагах контексты
/// вычисляются как прежде, а неизвестные свойства остаются `null`.
#[test]
fn borrowed_common_module_view_keeps_absent_properties_and_contexts_unknown() {
    let fixture = RealReaderFixture::new();
    let configuration = fs::read_to_string(fixture.source.join("Configuration.xml")).unwrap();
    write(
        &fixture.source.join("Configuration.xml"),
        &configuration.replace(
            "</ChildObjects>",
            "<CommonModule>КешЗначений</CommonModule><CommonModule>СерверныеФункции</CommonModule></ChildObjects>",
        ),
    );
    for (name, descriptor) in [
        ("КешЗначений", "extension-common-module-inherited.xml"),
        (
            "СерверныеФункции",
            "extension-common-module-environment-only.xml",
        ),
    ] {
        write(
            &fixture.source.join(format!("CommonModules/{name}.xml")),
            &fixture_text(&format!("platform_8_3_27/cfe_borrow/{descriptor}")),
        );
        write(
            &fixture
                .source
                .join(format!("CommonModules/{name}/Ext/Module.bsl")),
            "Функция ПР_Удвоенное() Экспорт\n\tВозврат 2;\nКонецФункции\n",
        );
    }
    let service = fixture.extension_view_service();

    let view = service.view(ViewRequest::new("main:CommonModule.КешЗначений").unwrap());
    assert!(view.ok, "view must succeed");
    let data = view.data.unwrap();
    for key in COMMON_MODULE_PROPERTY_KEYS {
        assert_eq!(
            data["props"]["commonModule"][key],
            serde_json::Value::Null,
            "{key}"
        );
    }
    let method = service
        .view(ViewRequest::new("main:CommonModule.КешЗначений.Method.ПР_Удвоенное").unwrap());
    assert!(method.ok, "method must succeed");
    let method = method.data.unwrap();
    assert_eq!(method["kind"], "Method");
    assert_eq!(method["props"]["export"], true);
    assert_eq!(
        method["props"]["compile"]["contexts"],
        serde_json::Value::Null,
        "contexts of a borrowed module without environment flags are unknown"
    );
    let body = service
        .view(ViewRequest::new("main:CommonModule.КешЗначений.Method.ПР_Удвоенное.Body").unwrap());
    assert!(body.ok, "body must succeed");
    let methods = service.view(ViewRequest::new("main:CommonModule.КешЗначений.Method").unwrap());
    assert!(methods.ok, "methods of a borrowed module stay readable");
    assert_eq!(
        methods.data.unwrap()["items"][0]["props"]["compile"]["contexts"],
        serde_json::Value::Null,
        "the collection keeps the unknown contexts of its methods"
    );

    let view = service.view(ViewRequest::new("main:CommonModule.СерверныеФункции").unwrap());
    assert!(view.ok, "view must succeed");
    let common = &view.data.as_ref().unwrap()["props"]["commonModule"];
    assert_eq!(common["server"], true);
    assert_eq!(common["externalConnection"], true);
    assert_eq!(common["returnValuesReuse"], serde_json::Value::Null);
    assert_eq!(common["privileged"], serde_json::Value::Null);
    let method = service
        .view(ViewRequest::new("main:CommonModule.СерверныеФункции.Method.ПР_Удвоенное").unwrap());
    assert!(method.ok, "method must succeed");
    assert_eq!(
        method.data.unwrap()["props"]["compile"]["contexts"],
        json!([
            "server",
            "mobileAppServer",
            "mobileStandaloneServer",
            "externalConnection"
        ])
    );
}

/// #1304: контексты известны только при всех четырёх флагах среды.
/// Без любого одного из них контексты метода — `null`, а остальные
/// записанные флаги читаются как есть.
#[test]
fn borrowed_common_module_without_any_single_environment_flag_has_unknown_contexts() {
    let environment =
        fixture_text("platform_8_3_27/cfe_borrow/extension-common-module-environment-only.xml");
    for (flag, element) in [
        (
            "clientManagedApplication",
            "<ClientManagedApplication>false</ClientManagedApplication>",
        ),
        (
            "clientOrdinaryApplication",
            "<ClientOrdinaryApplication>false</ClientOrdinaryApplication>",
        ),
        ("server", "<Server>true</Server>"),
        (
            "externalConnection",
            "<ExternalConnection>true</ExternalConnection>",
        ),
    ] {
        let descriptor = environment.replace(element, "");
        assert_ne!(descriptor, environment, "{flag}");
        let props =
            super::common_module_properties(descriptor.as_bytes(), SourceSetKind::Extension)
                .unwrap();
        assert!(!props.execution_environment_known(), "{flag}");

        let fixture = RealReaderFixture::new();
        let configuration = fs::read_to_string(fixture.source.join("Configuration.xml")).unwrap();
        write(
            &fixture.source.join("Configuration.xml"),
            &configuration.replace(
                "</ChildObjects>",
                "<CommonModule>СерверныеФункции</CommonModule></ChildObjects>",
            ),
        );
        write(
            &fixture.source.join("CommonModules/СерверныеФункции.xml"),
            &descriptor,
        );
        write(
            &fixture
                .source
                .join("CommonModules/СерверныеФункции/Ext/Module.bsl"),
            "Функция ПР_Разность(А, Б) Экспорт\n\tВозврат А - Б;\nКонецФункции\n",
        );
        let service = fixture.extension_view_service();
        let module = service.view(ViewRequest::new("main:CommonModule.СерверныеФункции").unwrap());
        assert!(module.ok, "{flag}: module view must succeed");
        let common = &module.data.as_ref().unwrap()["props"]["commonModule"];
        assert_eq!(common[flag], serde_json::Value::Null, "{flag}");
        assert_eq!(common["serverCall"], true, "{flag}");
        let method = service.view(
            ViewRequest::new("main:CommonModule.СерверныеФункции.Method.ПР_Разность").unwrap(),
        );
        assert!(method.ok, "{flag}: method view must succeed");
        assert_eq!(
            method.data.unwrap()["props"]["compile"]["contexts"],
            serde_json::Value::Null,
            "{flag}"
        );
    }
}

#[test]
fn borrowed_common_module_missing_reuse_preserves_methods_and_source() {
    // Synthetic omission reproduces #1304; it is not a platform export.
    let fixture = RealReaderFixture::new();
    let configuration = fs::read_to_string(fixture.source.join("Configuration.xml")).unwrap();
    write(
        &fixture.source.join("Configuration.xml"),
        &configuration.replace(
            "</ChildObjects>",
            "<CommonModule>CorpusModule</CommonModule></ChildObjects>",
        ),
    );
    let descriptor = fixture_text("platform_8_3_27/cfe_borrow/extension-common-module.xml")
        .replace("<ReturnValuesReuse>DontUse</ReturnValuesReuse>", "");
    let path = fixture.source.join("CommonModules/CorpusModule.xml");
    write(&path, &descriptor);
    let module = fixture
        .source
        .join("CommonModules/CorpusModule/Ext/Module.bsl");
    let source = "Процедура Проверка() Экспорт\nКонецПроцедуры\n";
    write(&module, source);
    let service = fixture.extension_view_service();
    let view = service.view(ViewRequest::new("main:CommonModule.CorpusModule").unwrap());
    assert!(view.ok, "view must succeed");
    let data = view.data.unwrap();
    assert_eq!(
        data["props"]["commonModule"]["returnValuesReuse"],
        serde_json::Value::Null
    );
    for at in [
        "main:CommonModule.CorpusModule.Method",
        "main:CommonModule.CorpusModule.Method.Проверка",
        "main:CommonModule.CorpusModule.Body",
    ] {
        let view = service.view(ViewRequest::new(at).unwrap());
        assert!(view.ok, "{at}: view must succeed");
    }
    assert_eq!(fs::read_to_string(path).unwrap(), descriptor);
    assert_eq!(fs::read_to_string(module).unwrap(), source);
}

#[test]
fn common_module_present_damage_is_not_missing() {
    // Synthetic corruptions exercise reader boundaries, not platform validity.
    let borrowed = fixture_text("platform_8_3_27/cfe_borrow/extension-common-module.xml");
    for value in [
        "<Server/>",
        "<Server>maybe</Server>",
        "<Server>false</Server><Server>true</Server>",
        "<Server xmlns=\"urn:foreign\">false</Server>",
        "<Server><Nested>false</Nested></Server>",
        "<Server extra=\"false\">false</Server>",
    ] {
        let invalid = borrowed.replace("<Server>false</Server>", value);
        assert!(
            super::common_module_properties(invalid.as_bytes(), SourceSetKind::Extension).is_err(),
            "{value}"
        );
    }
    for value in [
        "<ReturnValuesReuse/>",
        "<ReturnValuesReuse><Nested>DontUse</Nested></ReturnValuesReuse>",
    ] {
        let invalid = borrowed.replace("<ReturnValuesReuse>DontUse</ReturnValuesReuse>", value);
        assert!(
            super::common_module_properties(invalid.as_bytes(), SourceSetKind::Extension).is_err(),
            "{value}"
        );
    }
    // The identity is proved on the exact structure: one CommonModule object
    // with one direct Properties container of the metadata namespace.
    let structural = [
        borrowed
            .replacen("<CommonModule ", "<Catalog ", 1)
            .replacen("</CommonModule>", "</Catalog>", 1),
        borrowed.replacen("<Properties>", "<Properties xmlns=\"urn:foreign\">", 1),
        borrowed.replacen("</Properties>", "</Properties><Properties/>", 1),
    ];
    for invalid in structural {
        assert_ne!(invalid, borrowed);
        assert!(
            super::common_module_properties(invalid.as_bytes(), SourceSetKind::Extension).is_err(),
            "a damaged descriptor structure must not open the borrowed absence policy"
        );
    }
}

fn configuration_payload(
    reader: &ProviderReadAuthority,
) -> Result<serde_json::Value, crate::application::v13::view::ViewError> {
    reader
        .configuration_payload_with_checkpoint(&mut || Ok(()))
        .map(|payload| payload.as_ref().clone())
}

#[test]
fn typed_projection_keeps_summary_compact_and_content_address_selected() {
    let summary =
        QualifiedAddress::parse("main:Report.Продажи.Template.Схема.DataSet.Продажи").unwrap();
    let route = route_logical_address(&summary, PlatformProfile::v8_3_27()).unwrap();
    let payload = json!({
        "support": {"state": "Editable"},
        "dataSets": [{"name": "Продажи", "kind": "Query", "query": "SELECT\n  *"}],
        "sourceSet": "main",
        "metadataPath": "Report.Продажи.Template.Схема"
    });
    let view =
        serde_json::to_value(project_typed_payload(&route, payload.clone()).unwrap()).unwrap();
    assert!(!view.to_string().contains("SELECT"));
    assert!(view.get("sourceSet").is_none());
    assert!(view["branches"].as_array().unwrap().iter().any(|branch| {
        branch["at"] == "main:Report.Продажи.Template.Схема.DataSet.Продажи.Query"
    }));

    let query = QualifiedAddress::parse("main:Report.Продажи.Template.Схема.DataSet.Продажи.Query")
        .unwrap();
    let data = project_known_suffix(
        crate::infrastructure::logical_tree::LogicalReader::Dcs,
        &query,
        &payload,
        &query.segments()[2..],
    )
    .unwrap();
    let view = serde_json::to_value(data).unwrap();
    assert_eq!(view["items"][0], json!({"line": 1, "text": "SELECT"}));
    assert!(view["items"][0].get("at").is_none());
}

#[test]
fn typed_projection_never_leaks_provider_or_physical_slots() {
    let at = QualifiedAddress::parse("main:Subsystem.Продажи").unwrap();
    let route = route_logical_address(&at, PlatformProfile::v8_3_27()).unwrap();
    let view = serde_json::to_value(
        project_typed_payload(
            &route,
            json!({
                "name": "Продажи",
                "location": {"path": "/secret/Subsystems/Продажи.xml"},
                "fileExists": true,
                "provider": "raw",
                "support": {
                    "state": "locked",
                    "directEditSafe": false,
                    "location": "/secret/ParentConfigurations.bin",
                    "provider": {"raw": "x".repeat(4096)}
                },
                "content": ["Catalog.Товары"],
                "commandInterface": {"visibility": []}
            }),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        view.get("props")
            .and_then(serde_json::Value::as_object)
            .into_iter()
            .flat_map(|props| props.values())
            .all(|value| !value.is_object() && !value.is_array()),
        "subsystem props must contain only scalar values: {view}"
    );
    assert_eq!(view["props"]["support"], "locked");
    assert_eq!(view["props"]["directEditSafe"], false);
    let text = view.to_string();
    for forbidden in ["/secret", "fileExists", "provider", "sourceState", "layout"] {
        assert!(!text.contains(forbidden), "leaked {forbidden}: {text}");
    }
}

#[test]
fn typed_projection_rejects_unknown_provider_payload_instead_of_dumping_it() {
    let at = QualifiedAddress::parse("main:Configuration").unwrap();
    let route = route_logical_address(&at, PlatformProfile::v8_3_27()).unwrap();

    let error = project_typed_payload(
        &route,
        json!({"name": "Main", "mysteryProviderPayload": {"raw": "secret"}}),
    )
    .unwrap_err();

    assert_eq!(error.code().as_str(), "provider_unavailable");
}

#[test]
fn actor_owned_reader_never_follows_a_source_set_remap_after_admission() {
    let fixture = RealReaderFixture::new();
    let cancellation = CancellationToken::new();
    let source_root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
        source_root,
        PlatformProfile::v8_3_27(),
    );
    let service = ViewService::new(authority, ViewCursorStore::default());
    let admitted = service.view(ViewRequest::new("main:Configuration").unwrap());
    assert!(admitted.ok, "{:?}", admitted.diagnostics);
    assert_eq!(
        admitted.data.as_ref().unwrap()["props"]["name"],
        "CorpusConfiguration"
    );

    let replacement = fixture.root.path().join("replacement");
    fs::create_dir_all(&replacement).unwrap();
    fs::write(
        replacement.join("Configuration.xml"),
        fixture_text("xdto/enterprise-data-minimal/Configuration.xml").replace(
            "<Name>CorpusConfiguration</Name>",
            "<Name>ReplacementConfiguration</Name>",
        ),
    )
    .unwrap();
    fs::write(
        fixture.root.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: replacement\n",
    )
    .unwrap();

    let after_remap = service.view(ViewRequest::new("main:Configuration").unwrap());
    assert!(after_remap.ok, "{:?}", after_remap.diagnostics);
    assert_eq!(after_remap.rev, admitted.rev);
    assert_eq!(
        after_remap.data.as_ref().unwrap()["props"]["name"],
        "CorpusConfiguration",
        "a retained reader must never combine replacement bytes with the admitted source revision",
    );
}

#[test]
fn actor_owned_configuration_support_and_home_page_sidecars_are_retained() {
    let fixture = RealReaderFixture::new();
    fs::create_dir_all(fixture.source.join("Ext")).unwrap();
    fs::copy(
        fixture_path("platform_8_3_27/support-edit-bin-only/src/Ext/ParentConfigurations.bin"),
        fixture.source.join("Ext/ParentConfigurations.bin"),
    )
    .unwrap();
    fs::copy(
        fixture_path("unica_mcp_script_parity/cf-info/Ext/HomePageWorkArea.xml"),
        fixture.source.join("Ext/HomePageWorkArea.xml"),
    )
    .unwrap();
    let cancellation = CancellationToken::new();
    let source_root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
        source_root,
        PlatformProfile::v8_3_27(),
    );
    let route = route_logical_address(
        &QualifiedAddress::parse("main:Configuration").unwrap(),
        PlatformProfile::v8_3_27(),
    )
    .unwrap();
    let admitted = authority.typed_payload(&route).unwrap();
    assert_eq!(admitted["support"]["state"], "supported");
    assert_eq!(admitted["homePage"]["template"], "TwoColumns");

    let replacement = fixture.root.path().join("replacement");
    fs::create_dir_all(&replacement).unwrap();
    fs::write(
        replacement.join("Configuration.xml"),
        fixture_text("xdto/enterprise-data-minimal/Configuration.xml"),
    )
    .unwrap();
    fs::write(
        fixture.root.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: replacement\n",
    )
    .unwrap();

    let after_remap = authority.typed_payload(&route).unwrap();
    assert_eq!(after_remap["support"], admitted["support"]);
    assert_eq!(after_remap["homePage"], admitted["homePage"]);
}

#[test]
fn retained_home_page_distinguishes_missing_from_malformed_and_wrong_root() {
    let fixture = RealReaderFixture::new();
    fs::create_dir_all(fixture.source.join("Ext")).unwrap();
    let root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let reader = ProviderReadAuthority::new(
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
        root,
    );
    let sidecar = fixture.source.join("Ext/HomePageWorkArea.xml");

    let missing = configuration_payload(&reader).unwrap();
    assert!(missing["homePage"].is_null());

    fs::write(&sidecar, "<broken>").unwrap();
    let malformed = configuration_payload(&reader).unwrap_err();
    assert_eq!(malformed.code().as_str(), "provider_unavailable");

    fs::write(
        &sidecar,
        r#"<NotHomePage xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"/>"#,
    )
    .unwrap();
    let wrong_root = configuration_payload(&reader).unwrap_err();
    assert_eq!(wrong_root.code().as_str(), "provider_unavailable");

    fs::copy(
        fixture_path("unica_mcp_script_parity/cf-info/Ext/HomePageWorkArea.xml"),
        &sidecar,
    )
    .unwrap();
    let valid = configuration_payload(&reader).unwrap();
    assert_eq!(valid["homePage"]["template"], "TwoColumns");
}

#[test]
fn actor_supplied_extension_kind_preserves_extension_support_semantics() {
    let fixture = RealReaderFixture::new();
    let root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let reader = ProviderReadAuthority::new(
        "extension",
        "actor-fixture-extension",
        SourceSetKind::Extension,
        root,
    );

    let payload = configuration_payload(&reader).unwrap();
    assert_eq!(payload["support"]["state"], "extension");
}

#[test]
fn actor_owned_typed_form_reader_never_follows_a_source_set_remap() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    let admitted = service
        .view(ViewRequest::new("main:Report.ParityReport.Form.MainForm.Item.Goods").unwrap());
    assert!(admitted.ok, "{:?}", admitted.diagnostics);

    let replacement = fixture.root.path().join("replacement");
    fs::create_dir_all(replacement.join("Reports/ParityReport/Forms/MainForm/Ext")).unwrap();
    fs::write(
        replacement.join("Configuration.xml"),
        replace_child_objects(
            &fixture_text("xdto/enterprise-data-minimal/Configuration.xml"),
            "<Report>ParityReport</Report>",
        ),
    )
    .unwrap();
    fs::copy(
        fixture.source.join("Reports/ParityReport.xml"),
        replacement.join("Reports/ParityReport.xml"),
    )
    .unwrap();
    fs::copy(
        fixture
            .source
            .join("Reports/ParityReport/Forms/MainForm.xml"),
        replacement.join("Reports/ParityReport/Forms/MainForm.xml"),
    )
    .unwrap();
    fs::write(
        replacement.join("Reports/ParityReport/Forms/MainForm/Ext/Form.xml"),
        r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><ChildItems><InputField name="Replacement"/></ChildItems></Form>"#,
    )
    .unwrap();
    fs::write(
        fixture.root.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: replacement\n",
    )
    .unwrap();

    let replacement_only = service
        .view(ViewRequest::new("main:Report.ParityReport.Form.MainForm.Item.Replacement").unwrap());
    assert!(!replacement_only.ok, "{:?}", replacement_only.data);
    let retained = service
        .view(ViewRequest::new("main:Report.ParityReport.Form.MainForm.Item.Goods").unwrap());
    assert!(retained.ok, "{:?}", retained.diagnostics);
    assert_eq!(retained.rev, admitted.rev);
}

#[test]
fn actor_owned_module_reader_never_follows_a_source_set_remap() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    let address = "main:CommonModule.РеактивныйСервер.Method";
    let admitted = service.view(ViewRequest::new(address).unwrap());
    assert!(admitted.ok, "{:?}", admitted.diagnostics);
    assert!(admitted.data.as_ref().unwrap()["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["at"] == "main:CommonModule.РеактивныйСервер.Method.InternalService"));

    let replacement = fixture.root.path().join("replacement");
    fs::create_dir_all(replacement.join("CommonModules/РеактивныйСервер/Ext")).unwrap();
    fs::write(
        replacement.join("Configuration.xml"),
        replace_child_objects(
            &fixture_text("xdto/enterprise-data-minimal/Configuration.xml"),
            "<CommonModule>РеактивныйСервер</CommonModule>",
        ),
    )
    .unwrap();
    fs::copy(
        fixture.source.join("CommonModules/РеактивныйСервер.xml"),
        replacement.join("CommonModules/РеактивныйСервер.xml"),
    )
    .unwrap();
    fs::write(
        replacement.join("CommonModules/РеактивныйСервер/Ext/Module.bsl"),
        "Процедура ReplacementOnly() Экспорт\nКонецПроцедуры\n",
    )
    .unwrap();
    fs::write(
        fixture.root.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: replacement\n",
    )
    .unwrap();

    let retained = service.view(ViewRequest::new(address).unwrap());
    assert!(retained.ok, "{:?}", retained.diagnostics);
    assert_eq!(retained.rev, admitted.rev);
    let items = retained.data.as_ref().unwrap()["items"].as_array().unwrap();
    assert!(items
        .iter()
        .any(|item| item["at"] == "main:CommonModule.РеактивныйСервер.Method.InternalService"));
    assert!(!items
        .iter()
        .any(|item| item["at"] == "main:CommonModule.РеактивныйСервер.Method.ReplacementOnly"));
}

#[test]
fn every_typed_reader_remains_on_the_admitted_root_after_source_set_remap() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    let addresses = [
        "main:Catalog.Items.Attribute.Code",
        "main:Role.SalesReader.Right.Catalog_Products.RLS.View",
        "main:Subsystem.Sales.Interface",
        "main:Report.ParityReport.Template.MainSchema.DataSet",
        "main:Report.ParityReport.Template.Print.Area",
        "main:XDTOPackage.EnterpriseData_1_17_3.Type",
    ];
    for address in addresses {
        let result = service.view(ViewRequest::new(address).unwrap());
        assert!(result.ok, "admission {address}: {:?}", result.diagnostics);
    }

    let replacement = fixture.root.path().join("replacement");
    fs::create_dir_all(&replacement).unwrap();
    fs::write(
        replacement.join("Configuration.xml"),
        fixture_text("xdto/enterprise-data-minimal/Configuration.xml"),
    )
    .unwrap();
    fs::write(
        fixture.root.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: replacement\n",
    )
    .unwrap();

    for address in addresses {
        let result = service.view(ViewRequest::new(address).unwrap());
        assert!(
            result.ok,
            "retained read {address}: {} {:?}",
            result.summary, result.diagnostics
        );
    }
}

/// The root projection carries what the retired `unica.cf.info` reported:
/// vendor facts, the support state, the closed root properties and the
/// object total, every declared key present even when its value is null.
#[test]
fn configuration_root_carries_the_retired_cf_info_facts() {
    let fixture = RealReaderFixture::new();
    let cancellation = CancellationToken::new();
    let source_root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
        source_root,
        PlatformProfile::v8_3_27(),
    );
    let service = ViewService::new(authority, ViewCursorStore::default());

    let root = service.view(ViewRequest::new("main:Configuration").unwrap());
    let props = root.data.as_ref().unwrap()["props"].as_object().unwrap();
    for key in [
        "format",
        "name",
        "synonym",
        "version",
        "vendor",
        "extensionPurpose",
        "totalObjects",
        "support",
        "properties",
        "homePage",
    ] {
        assert!(
            props.contains_key(key),
            "root props miss `{key}`: {props:?}"
        );
    }
    assert_eq!(props["format"], "2.20");
    assert!(props["totalObjects"].as_u64().unwrap() > 0);
    assert!(props["support"]["state"].is_string());
    let properties = props["properties"].as_object().unwrap();
    for key in [
        "compatibilityMode",
        "defaultRunMode",
        "scriptVariant",
        "defaultLanguage",
        "dataLockControlMode",
        "modalityUseMode",
        "interfaceCompatibilityMode",
        "extensionCompatibilityMode",
        "objectAutonumerationMode",
        "synchronousCallUseMode",
        "databaseTablespacesUseMode",
        "mainWindowMode",
        "comment",
        "namePrefix",
        "updateCatalogAddress",
    ] {
        assert!(properties.contains_key(key), "root properties miss `{key}`");
    }
}

#[test]
fn configuration_root_branch_counts_match_every_reachable_collection() {
    let fixture = RealReaderFixture::new();
    let cancellation = CancellationToken::new();
    let source_root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
        source_root,
        PlatformProfile::v8_3_27(),
    );
    let service = ViewService::new(authority, ViewCursorStore::default());

    let root = service.view(ViewRequest::new("main:Configuration").unwrap());
    let branches = root.data.as_ref().unwrap()["branches"].as_array().unwrap();
    assert!(!branches.is_empty());
    for branch in branches {
        let at = branch["at"].as_str().unwrap();
        let collection = service.view(ViewRequest::new(at).unwrap());
        assert!(
            collection.ok,
            "{at}: {} {:?}",
            collection.summary, collection.diagnostics
        );
        let items = collection.data.as_ref().unwrap()["items"]
            .as_array()
            .unwrap();
        assert_eq!(
            branch["count"].as_u64().unwrap(),
            items.len() as u64,
            "{at}"
        );
        assert!(
            items.iter().all(|item| item.get("at").is_some()),
            "addressable branch rows must keep canonical addresses: {at}"
        );
    }
}

#[test]
fn module_capability_parents_expose_canonical_module_collections() {
    let fixture = RealReaderFixture::new();
    fixture.install_accepted_profile_sources();
    let service = fixture.view_service();
    let cases = [
        (
            "main:Configuration",
            "main:Module",
            4,
            vec![
                "main:Module.ManagedApplication",
                "main:Module.OrdinaryApplication",
                "main:Module.Session",
                "main:Module.ExternalConnection",
            ],
        ),
        (
            "main:Document.Заказ",
            "main:Document.Заказ.Module",
            2,
            vec![
                "main:Document.Заказ.Module.Object",
                "main:Document.Заказ.Module.Manager",
            ],
        ),
        (
            "main:Document.Заказ.Form.ФормаДокумента",
            "main:Document.Заказ.Form.ФормаДокумента.Module",
            1,
            vec!["main:Document.Заказ.Form.ФормаДокумента.Module.Form"],
        ),
    ];

    for (parent, branch, count, expected) in cases {
        let parent_result = service.view(ViewRequest::new(parent).unwrap());
        assert!(parent_result.ok, "{parent}: {parent_result:?}");
        let parent_data = parent_result.data.unwrap();
        let parent_branches = parent_data["branches"].as_array().unwrap();
        assert_eq!(
            parent_branches
                .iter()
                .map(|item| item["at"].as_str().unwrap())
                .collect::<std::collections::HashSet<_>>()
                .len(),
            parent_branches.len(),
            "{parent} exposed duplicate branch addresses: {parent_data}",
        );
        assert!(
            parent_branches.iter().any(|item| {
                item["at"] == branch && item["count"].as_u64() == Some(count as u64)
            }),
            "{parent} did not expose {branch}: {parent_data}"
        );

        let branch_result = service.view(ViewRequest::new(branch).unwrap());
        assert!(branch_result.ok, "{branch}: {branch_result:?}");
        let branch_data = branch_result.data.unwrap();
        let items = branch_data["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["at"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(items, expected, "{branch}");
        assert_eq!(items.len(), count, "{branch}");
    }
}

#[test]
fn configuration_runtime_modules_are_read_from_the_shared_ext_layout() {
    let fixture = RealReaderFixture::new();
    let cases = [
        (
            "ManagedApplication",
            "ManagedApplicationModule",
            "ManagedFromExt",
        ),
        (
            "OrdinaryApplication",
            "OrdinaryApplicationModule",
            "OrdinaryFromExt",
        ),
        ("Session", "SessionModule", "SessionFromExt"),
        (
            "ExternalConnection",
            "ExternalConnectionModule",
            "ExternalFromExt",
        ),
    ];
    for (_, file, method) in cases {
        write(
            &fixture.source.join(format!("Ext/{file}.bsl")),
            &format!("Procedure {method}()\nEndProcedure\n"),
        );
        write(
            &fixture.source.join(format!("{file}.bsl")),
            "Procedure WrongRootLayout()\nEndProcedure\n",
        );
    }
    let service = fixture.view_service();

    for (role, _, method) in cases {
        let at = format!("main:Module.{role}.Method.{method}");
        let result = service.view(ViewRequest::new(&at).unwrap());
        assert!(
            result.ok,
            "{at}: {} {:?}",
            result.summary, result.diagnostics
        );
        assert_eq!(result.data.as_ref().unwrap()["at"], at);
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AcceptedAddressProfile {
    valid_addresses: Vec<AcceptedAddressCase>,
}

#[derive(Debug, Deserialize)]
struct AcceptedAddressCase {
    case: String,
    input: String,
}

#[test]
fn every_accepted_profile_address_has_a_real_non_skipping_view() {
    let fixture = RealReaderFixture::new();
    fixture.install_accepted_profile_sources();
    let service = fixture.view_service();
    let profile: AcceptedAddressProfile = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/v013/address-profile-8.3.27.json"
    ))
    .unwrap();

    for case in profile.valid_addresses {
        let expected_at = QualifiedAddress::parse(&case.input).unwrap().to_string();
        let result = service.view(ViewRequest::new(&case.input).unwrap());
        assert!(
            result.ok,
            "{} ({expected_at}): {} {:?}",
            case.case, result.summary, result.diagnostics
        );
        assert_eq!(
            result.data.as_ref().unwrap()["at"],
            expected_at,
            "{}",
            case.case
        );
    }
}

#[test]
fn capability_bound_configuration_reader_preserves_complete_cf_info_semantics() {
    let fixture = RealReaderFixture::new();
    let cancellation = CancellationToken::new();
    let root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let reader = ProviderReadAuthority::new(
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
        root,
    );
    reader
        .exact_revision(
            crate::domain::code_intelligence::ProviderDeadline::from_budget(
                std::time::Duration::from_secs(7),
            ),
            &cancellation,
        )
        .unwrap();

    let payload = configuration_payload(&reader).unwrap();
    assert_eq!(
        payload["properties"]["defaultRunMode"],
        "ManagedApplication"
    );
    assert_eq!(payload["support"]["state"], "notSupported");
    assert_eq!(payload["totalObjects"], 17);
    assert_eq!(
        payload["childObjects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["count"].as_u64().unwrap())
            .sum::<u64>(),
        17,
    );
    assert_eq!(payload["registeredObjects"].as_array().unwrap().len(), 17);
}

#[test]
fn metadata_kind_branch_lists_registered_objects_with_canonical_addresses() {
    let fixture = RealReaderFixture::new();
    let cancellation = CancellationToken::new();
    let source_root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
        source_root,
        PlatformProfile::v8_3_27(),
    );
    let service = ViewService::new(authority, ViewCursorStore::default());

    let branch = service.view(ViewRequest::new("main:Catalog").unwrap());
    assert!(branch.ok, "{:?}", refusal_codes(&branch));
    assert_eq!(
        branch.data.as_ref().unwrap()["items"],
        json!([
            {
                "at": "main:Catalog.Items",
                "kind": "Catalog",
                "title": "Items"
            },
            {
                "at": "main:Catalog.Владельцы",
                "kind": "Catalog",
                "title": "Владельцы"
            }
        ]),
    );
}

#[test]
fn metadata_node_props_carry_the_observed_object_properties() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    let result = service.view(ViewRequest::new("main:Catalog.Items").unwrap());

    assert!(result.ok, "{:?}", refusal_codes(&result));
    let props = &result.data.as_ref().unwrap()["props"];
    // Свойства объекта отвечают на узле объекта: раньше проекция искала шесть
    // скаляров в `details`, где их нет ни у одного вида, и узел молчал.
    assert_eq!(props["Hierarchical"], json!(true));
    assert_eq!(props["CodeLength"], json!(11));
    assert_eq!(props["kind"], json!("Catalog"));
}

/// Три роли дампа `PrintWebDAV`: заимствован с перекрытием, заимствован без
/// перекрытия, собственный объект расширения.
///
/// Признак заимствования — `ExtendedConfigurationObject`, а не `Adopted`:
/// корень `Configuration` самого расширения тоже `Adopted`, и читать его как
/// заимствованный объект — дефект. `xr:PropertyState` несёт **список**
/// перекрытых свойств, а не флаг: заимствованный объект без перекрытий его не
/// несёт вовсе.
#[test]
fn borrowing_props_name_the_three_roles_of_an_extension_source_set() {
    let fixture = RealReaderFixture::new();
    fixture.borrow_catalog("Catalogs/Владельцы.xml", Some("Synonym"));
    fixture.borrow_catalog("Documents/Order.xml", None);
    let service = fixture.extension_view_service();

    let overridden = service.view(ViewRequest::new("main:Catalog.Владельцы").unwrap());
    assert!(overridden.ok, "{:?}", refusal_codes(&overridden));
    let props = &overridden.data.as_ref().unwrap()["props"];
    assert_eq!(props["belonging"], json!("borrowed"));
    assert_eq!(
        props["parentId"],
        json!("11111111-1111-4111-8111-111111111111")
    );
    assert_eq!(props["overrides"], json!("Synonym"));
    assert!(
        props.get("extends").is_none(),
        "UUID is not a readable parent address"
    );
    assert_eq!(props["parentStatus"], json!("unavailable"));

    let plain = service.view(ViewRequest::new("main:Document.Order").unwrap());
    assert!(plain.ok, "{:?}", refusal_codes(&plain));
    let props = &plain.data.as_ref().unwrap()["props"];
    assert_eq!(props["belonging"], json!("borrowed"));
    assert_eq!(
        props["parentId"],
        json!("11111111-1111-4111-8111-111111111111")
    );
    assert!(
        props.get("overrides").is_none(),
        "заимствование без перекрытий ключа `overrides` не несёт: пустой список — это его отсутствие"
    );

    let own = service.view(ViewRequest::new("main:Catalog.Items").unwrap());
    assert!(own.ok, "{:?}", refusal_codes(&own));
    let props = &own.data.as_ref().unwrap()["props"];
    assert_eq!(props["belonging"], json!("own"));
    assert!(props.get("extends").is_none());
    assert!(props.get("overrides").is_none());
}

/// Заимствование не стоит второго чтения дескриптора.
///
/// Свойства объекта и его заимствование разбираются из одних и тех же байтов.
/// Второе чтение того же файла стоило бы лишнего доступа и, что хуже, могло бы
/// застать файл изменившимся между чтениями — тогда два ответа об одном
/// объекте описывали бы разные состояния источника.
#[test]
fn borrowing_costs_no_second_descriptor_read() {
    let fixture = RealReaderFixture::new();
    fixture.borrow_catalog("Catalogs/Владельцы.xml", Some("Synonym"));
    let authority = fixture.extension_read_authority();

    assert_reader_reaches(&authority, &["main:Catalog.Владельцы"]);

    assert_eq!(
        authority.metadata_descriptor_read_count("Catalog.Владельцы"),
        2,
        "заимствование читается тем же дескриптором, что и свойства объекта: \
         доказательство владельца плюс типизированная проекция, и ни чтением больше",
    );
}

/// В наборе вида `configuration` заимствования не бывает, и `belonging: own`
/// у каждого объекта было бы шумом.
#[test]
fn a_configuration_source_set_carries_no_borrowing_props() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    let result = service.view(ViewRequest::new("main:Catalog.Items").unwrap());

    assert!(result.ok, "{:?}", refusal_codes(&result));
    let props = &result.data.as_ref().unwrap()["props"];
    for key in ["belonging", "extends", "overrides"] {
        assert!(props.get(key).is_none(), "{key} в конфигурации не отвечает");
    }
}

#[test]
fn metadata_attribute_fill_value_is_a_bounded_observed_fact() {
    let fixture = RealReaderFixture::new();
    let descriptor = fixture.source.join("Catalogs/Items.xml");
    let original = fs::read_to_string(&descriptor).unwrap();
    let document = roxmltree::Document::parse(&original).unwrap();
    let range = document
        .root_element()
        .first_element_child()
        .unwrap()
        .children()
        .find(|node| node.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "ChildObjects")))
        .unwrap()
        .range();
    let attributes = r#"<Attribute xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><Properties><Name>Empty</Name><Type><v8:Type>cfg:CatalogRef.Items</v8:Type></Type><FillValue xsi:type="xr:DesignTimeRef">Catalog.Items.EmptyRef</FillValue></Properties></Attribute><Attribute xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><Properties><Name>Flag</Name><Type><v8:Type>cfg:EnumRef.Flags</v8:Type></Type><FillValue xsi:type="xr:DesignTimeRef">Enum.Flags.EnumValue.Yes</FillValue></Properties></Attribute>"#;
    write(
        &descriptor,
        &format!(
            "{}<ChildObjects>{attributes}</ChildObjects>{}",
            &original[..range.start],
            &original[range.end..]
        ),
    );
    let service = fixture.view_service();
    for (name, owner, item) in [
        ("Empty", "Catalog.Items", "EmptyRef"),
        ("Flag", "Enum.Flags", "EnumValue.Yes"),
    ] {
        let result = service
            .view(ViewRequest::new(&format!("main:Catalog.Items.Attribute.{name}")).unwrap());
        assert!(result.ok, "{} {:?}", result.summary, result.diagnostics);
        let props = &result.data.as_ref().unwrap()["props"];
        assert!(props.get("incomplete").is_none(), "{props}");
        let observed: serde_json::Value =
            serde_json::from_str(props["fillValue"].as_str().unwrap()).unwrap();
        assert_eq!(
            observed,
            json!({"kind": "reference", "metadataPath": owner, "item": item})
        );
    }
}

#[test]
fn metadata_node_props_lay_out_the_per_kind_facts_by_role() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    // Обработчик регламентного задания: два поля, две роли.
    let job = service.view(ViewRequest::new("main:ScheduledJob.MonthClose").unwrap());
    assert!(job.ok, "{:?}", refusal_codes(&job));
    let props = &job.data.as_ref().unwrap()["props"];
    assert_eq!(props["handlerModule"], json!("CommonModule.MonthClose"));
    assert_eq!(props["handlerMethod"], json!("RunScheduled"));

    // Расписание регистра расчёта: три поля, три роли.
    let register = service.view(ViewRequest::new("main:CalculationRegister.Payroll").unwrap());
    assert!(register.ok, "{:?}", refusal_codes(&register));
    let props = &register.data.as_ref().unwrap()["props"];
    assert_eq!(
        props["scheduleRegister"],
        json!("InformationRegister.WorkSchedules")
    );
    assert_eq!(
        props["scheduleValueField"],
        json!("InformationRegister.WorkSchedules.Resource.DayValue")
    );
    assert_eq!(
        props["scheduleDateField"],
        json!("InformationRegister.WorkSchedules.Dimension.Date")
    );

    // Тип константы: одно составное значение, компактной строкой — тем же
    // механизмом, каким отвечает тип реквизита.
    let constant = service.view(ViewRequest::new("main:Constant.MainCurrency").unwrap());
    assert!(constant.ok, "{:?}", refusal_codes(&constant));
    let rendered = constant.data.as_ref().unwrap()["props"]["type"]
        .as_str()
        .expect("constant type renders compactly");
    assert!(rendered.contains("string"), "{rendered}");
}

/// Сервис в том виде, в каком его выгружает Конфигуратор 8.3.27: у шаблона
/// и метода есть `Synonym` и `<Comment/>` (#634).
fn install_configurator_http_service(fixture: &RealReaderFixture, edit: impl Fn(&str) -> String) {
    let xml = fixture_text("platform_8_3_27/meta_info/edge/http-service-configurator.xml")
        .replace("<Name>ПроверкаСвязи</Name>", "<Name>ExternalAPI</Name>");
    write(
        &fixture.source.join("HTTPServices/ExternalAPI.xml"),
        &edit(&xml),
    );
}

#[test]
fn configurator_http_service_view_lists_its_templates_and_methods() {
    let fixture = RealReaderFixture::new();
    install_configurator_http_service(&fixture, str::to_string);
    let service = fixture.view_service();

    let http = service.view(ViewRequest::new("main:HTTPService.ExternalAPI").unwrap());
    assert!(http.ok, "{:?}", refusal_codes(&http));
    let node = http.data.as_ref().unwrap();
    assert!(
        node["branches"]
            .as_array()
            .unwrap()
            .contains(&json!({"at": "main:HTTPService.ExternalAPI.URLTemplate", "count": 2})),
        "{node:#}"
    );
    assert!(node.get("limits").is_none(), "{node:#}");

    let templates =
        service.view(ViewRequest::new("main:HTTPService.ExternalAPI.URLTemplate").unwrap());
    assert!(templates.ok, "{:?}", refusal_codes(&templates));
    let names = templates.data.as_ref().unwrap()["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["at"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "main:HTTPService.ExternalAPI.URLTemplate.Пинг",
            "main:HTTPService.ExternalAPI.URLTemplate.ПринятьДанные",
        ]
    );

    let method = service.view(
        ViewRequest::new("main:HTTPService.ExternalAPI.URLTemplate.ПринятьДанные.Method.POST")
            .unwrap(),
    );
    assert!(method.ok, "{:?}", refusal_codes(&method));
    let method = method.data.as_ref().unwrap();
    assert_eq!(method["props"]["httpMethod"], json!("POST"));
    assert_eq!(method["props"]["handler"], json!("ПринятьДанныеPOST"));
}

/// Веб-сервис в том виде, в каком его выгружает Конфигуратор 8.3.27:
/// у операции есть `Synonym`, `<Comment/>` и `DataLockControlMode`,
/// у параметра — `Synonym` и `<Comment/>`.
#[test]
fn configurator_web_service_view_opens_its_operations_and_parameters() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture.source.join("WebServices/Exchange.xml"),
        &fixture_text("platform_8_3_27/meta_info/edge/web-service-configurator.xml")
            .replace("<Name>ОбменДанными</Name>", "<Name>Exchange</Name>"),
    );
    let service = fixture.view_service();

    let web = service.view(ViewRequest::new("main:WebService.Exchange").unwrap());
    assert!(web.ok, "{:?}", refusal_codes(&web));
    let node = web.data.as_ref().unwrap();
    assert!(
        node["branches"]
            .as_array()
            .unwrap()
            .contains(&json!({"at": "main:WebService.Exchange.Operation", "count": 2})),
        "{node:#}"
    );
    assert!(node.get("limits").is_none(), "{node:#}");

    let operation =
        service.view(ViewRequest::new("main:WebService.Exchange.Operation.ПринятьПакет").unwrap());
    assert!(operation.ok, "{:?}", refusal_codes(&operation));
    let operation = operation.data.as_ref().unwrap();
    assert_eq!(operation["props"]["procedure"], json!("ПринятьПакет"));
    assert_eq!(
        operation["branches"],
        json!([{"at": "main:WebService.Exchange.Operation.ПринятьПакет.Parameter", "count": 3}]),
    );

    let parameter = service.view(
        ViewRequest::new("main:WebService.Exchange.Operation.ПринятьПакет.Parameter.Ошибка")
            .unwrap(),
    );
    assert!(parameter.ok, "{:?}", refusal_codes(&parameter));
    assert_eq!(
        parameter.data.as_ref().unwrap()["props"]["direction"],
        json!("out")
    );
}

/// Непрочитанный шаблон не выдаётся за пустой сервис: узел называет поле
/// в `limits`, а ветвь шаблонов отказывает с причиной вместо «нет такой».
#[test]
fn an_unreadable_http_template_is_named_rather_than_reported_absent() {
    let fixture = RealReaderFixture::new();
    install_configurator_http_service(&fixture, |xml| {
        xml.replacen("<Name>Пинг</Name>", "<Name>bad name</Name>", 1)
    });
    let service = fixture.view_service();

    let http = service.view(ViewRequest::new("main:HTTPService.ExternalAPI").unwrap());
    assert!(http.ok, "{:?}", refusal_codes(&http));
    let node = http.data.as_ref().unwrap();
    let limits = node["limits"]
        .as_array()
        .expect("the unread field is named");
    assert!(
        limits.iter().any(|limit| limit
            .as_str()
            .is_some_and(|limit| limit.starts_with("details.urlTemplates[0].name"))),
        "{node:#}"
    );

    let templates =
        service.view(ViewRequest::new("main:HTTPService.ExternalAPI.URLTemplate").unwrap());
    assert!(!templates.ok, "{:#?}", templates.data);
    assert_eq!(refusal_codes(&templates), ["provider_unavailable"]);
    let message = templates.diagnostics[0]["message"].as_str().unwrap();
    assert!(
        message.contains("details.urlTemplates[0].name"),
        "{message}"
    );
}

#[test]
fn declared_service_kinds_finally_get_their_subject() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    // Виды `URLTemplate` и `Operation` были объявлены в грамматике и не
    // подключены ни к чему; здесь у них появляется предмет.
    let http = service.view(ViewRequest::new("main:HTTPService.ExternalAPI").unwrap());
    assert!(http.ok, "{:?}", refusal_codes(&http));
    let branches = http.data.as_ref().unwrap()["branches"].as_array().unwrap();
    assert!(
        branches.contains(&json!({
            "at": "main:HTTPService.ExternalAPI.URLTemplate",
            "count": 1
        })),
        "{branches:#?}"
    );

    let template =
        service.view(ViewRequest::new("main:HTTPService.ExternalAPI.URLTemplate.Metrics").unwrap());
    assert!(template.ok, "{:?}", refusal_codes(&template));
    let template = template.data.as_ref().unwrap();
    assert_eq!(template["props"]["template"], json!("/v1/kpi/"));
    // Последовательность методов — ветвь, а не строка в `props`: коллекция
    // обязана быть адресуемой.
    assert_eq!(
        template["branches"],
        json!([{
            "at": "main:HTTPService.ExternalAPI.URLTemplate.Metrics.Method",
            "count": 1
        }]),
    );

    let method = service.view(
        ViewRequest::new("main:HTTPService.ExternalAPI.URLTemplate.Metrics.Method.Get").unwrap(),
    );
    assert!(method.ok, "{:?}", refusal_codes(&method));
    let method = method.data.as_ref().unwrap();
    assert_eq!(method["props"]["httpMethod"], json!("GET"));
    assert_eq!(method["props"]["handler"], json!("MetricsGet"));

    let operation =
        service.view(ViewRequest::new("main:WebService.Exchange.Operation.Send").unwrap());
    assert!(operation.ok, "{:?}", refusal_codes(&operation));
    let operation = operation.data.as_ref().unwrap();
    assert_eq!(operation["props"]["procedure"], json!("SendData"));
    assert_eq!(operation["props"]["transactioned"], json!(true));
    assert!(
        operation["props"]["returnType"]
            .as_str()
            .is_some_and(|rendered| rendered.contains("boolean")),
        "{operation:#}"
    );
    assert_eq!(
        operation["branches"],
        json!([{"at": "main:WebService.Exchange.Operation.Send.Parameter", "count": 1}]),
    );

    let parameter = service.view(
        ViewRequest::new("main:WebService.Exchange.Operation.Send.Parameter.Payload").unwrap(),
    );
    assert!(parameter.ok, "{:?}", refusal_codes(&parameter));
    assert_eq!(
        parameter.data.as_ref().unwrap()["props"]["direction"],
        json!("in")
    );
}

#[test]
fn every_reference_of_an_object_answers_in_one_relation_branch() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    let document = service.view(ViewRequest::new("main:Document.Order").unwrap());
    assert!(document.ok, "{:?}", refusal_codes(&document));
    let branches = document.data.as_ref().unwrap()["branches"]
        .as_array()
        .unwrap();
    // Платформа держит движение и основание в разных местах, но вопрос один:
    // на что объект показывает. Одна ветвь, имя связи — поле элемента.
    assert!(
        branches.contains(&json!({"at": "main:Document.Order.Relation", "count": 2})),
        "{branches:#?}"
    );

    let relations = service.view(ViewRequest::new("main:Document.Order.Relation").unwrap());
    assert!(relations.ok, "{:?}", refusal_codes(&relations));
    assert_eq!(
        relations.data.as_ref().unwrap()["items"],
        json!([
            {
                "relation": "registerRecord",
                "at": "main:CalculationRegister.Payroll",
                "kind": "CalculationRegister"
            },
            {
                "relation": "basedOn",
                "at": "main:Catalog.Items",
                "kind": "Catalog"
            }
        ]),
    );

    // Правило обещает всякую ссылку, поэтому проверяются все её источники, а
    // не те два, что лежали ближе. `relations` и пофактовая часть вида
    // приходят из разных мест платформы и должны сойтись в одной ветви.
    let sources = |at: &str| -> Vec<(String, String)> {
        let node = service.view(ViewRequest::new(at).unwrap());
        assert!(node.ok, "{at}: {:?}", refusal_codes(&node));
        node.data.as_ref().unwrap()["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| {
                (
                    item["relation"].as_str().unwrap().to_string(),
                    item["at"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    };
    assert_eq!(
        sources("main:Catalog.Items.Relation"),
        [
            ("owner".to_string(), "main:Catalog.Владельцы".to_string()),
            (
                "inputByString".to_string(),
                "main:Catalog.Items.Attribute.Code".to_string()
            ),
            (
                "dataLockField".to_string(),
                "main:Catalog.Items.Attribute.Code".to_string()
            ),
        ]
    );
    assert_eq!(
        sources("main:DocumentJournal.Журнал.Relation"),
        [(
            "registeredDocument".to_string(),
            "main:Document.Order".to_string()
        )]
    );
    assert_eq!(
        sources("main:ChartOfCalculationTypes.ВидыРасчета.Relation"),
        [(
            "baseCalculationType".to_string(),
            "main:ChartOfCalculationTypes.ВидыРасчета".to_string()
        )]
    );
    assert_eq!(
        sources("main:EventSubscription.ПриЗаписи.Relation"),
        [("source".to_string(), "main:Catalog.Items".to_string())]
    );
    // Пакет XDTO бывает ссылкой и бывает пространством имён; в ветвь попадает
    // только адресуемый вариант, и это названная граница, а не потеря.
    assert_eq!(
        sources("main:WebService.Exchange.Relation"),
        [(
            "xdtoPackage".to_string(),
            "main:XDTOPackage.Exchange".to_string()
        )]
    );

    // Элемент ветви показывает наружу, поэтому спускаться в него нельзя:
    // цель читается по своему адресу.
    let inward = service.view(ViewRequest::new("main:Document.Order.Relation.Items").unwrap());
    assert!(!inward.ok, "{:?}", refusal_codes(&inward));
    assert_eq!(refusal_codes(&inward), ["not_found"]);
}

#[test]
fn root_declarations_get_branches_and_only_the_named_one_is_addressable() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    // Стандартная табличная часть носит имя, поэтому адресуется, а её
    // собственная последовательность становится вложенной ветвью.
    let section = service.view(
        ViewRequest::new("main:ChartOfAccounts.Accounts.StandardTabularSection.ExtDimensionTypes")
            .unwrap(),
    );
    assert!(section.ok, "{:?}", refusal_codes(&section));
    let section = section.data.as_ref().unwrap();
    assert_eq!(section["kind"], "StandardTabularSection");
    assert_eq!(
        section["branches"],
        json!([{
            "at": "main:ChartOfAccounts.Accounts.StandardTabularSection.ExtDimensionTypes.StandardAttribute",
            "count": 1
        }]),
    );

    let attribute = service.view(
        ViewRequest::new(
            "main:ChartOfAccounts.Accounts.StandardTabularSection.ExtDimensionTypes.StandardAttribute.ExtDimensionType",
        )
        .unwrap(),
    );
    assert!(attribute.ok, "{:?}", refusal_codes(&attribute));
    assert_eq!(
        attribute.data.as_ref().unwrap()["title"],
        "ExtDimensionType"
    );

    // Характеристика имени не носит: ветвь адресуема, а её элементы —
    // строки данных, и адреса у них нет.
    let characteristics =
        service.view(ViewRequest::new("main:Catalog.Items.Characteristic").unwrap());
    assert!(characteristics.ok, "{:?}", refusal_codes(&characteristics));
    let items = characteristics.data.as_ref().unwrap()["items"]
        .as_array()
        .unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0].get("at").is_none(), "{items:#?}");
    assert_eq!(items[0]["values"]["source"], "Catalog.Items");

    let named = service.view(ViewRequest::new("main:Catalog.Items.Characteristic.Any").unwrap());
    assert!(!named.ok, "{:?}", refusal_codes(&named));
    assert_eq!(refusal_codes(&named), ["not_found"]);
}

#[test]
fn predefined_items_beyond_first_page_answer_by_address() {
    let fixture = predefined_page_fixture();
    let service = fixture.view_service();
    let owner = service.view(ViewRequest::new("main:Catalog.Items").unwrap());
    assert!(owner.ok, "{owner:?}");
    assert!(owner.data.as_ref().unwrap()["branches"]
        .as_array()
        .unwrap()
        .contains(&json!({"at": "main:Catalog.Items.PredefinedItem", "count": 72})));
    for index in [1, 16, 17, 19, 21, 50, 51, 53, 54, 55, 72] {
        let result = service.view(
            ViewRequest::new(&format!("main:Catalog.Items.PredefinedItem.Item{index:03}")).unwrap(),
        );
        assert!(result.ok, "item {index}: {result:?}");
        let data = result.data.unwrap();
        assert_eq!(data["title"], format!("Item{index:03}"));
        assert_eq!(
            data["props"]["id"],
            format!("00000000-0000-4000-8000-{index:012}")
        );
        assert_eq!(data["props"]["code"], format!("{index:03}"));
        assert_eq!(data["props"]["description"], format!("Description {index}"));
        assert_eq!(data["props"]["isFolder"], predefined_page_is_folder(index));
        assert_eq!(
            data["props"]["parentId"],
            predefined_page_parent(index).map_or(json!(null), |parent| json!(format!(
                "00000000-0000-4000-8000-{parent:012}"
            )))
        );
    }
    let missing =
        service.view(ViewRequest::new("main:Catalog.Items.PredefinedItem.Absent").unwrap());
    assert_eq!(refusal_codes(&missing), ["not_found"]);
}

#[test]
fn predefined_items_pages_preserve_complete_document_order() {
    let fixture = predefined_page_fixture();
    let service = fixture.view_service();
    for limit in [1, 17, 50] {
        let mut cursor = None;
        let mut names = Vec::new();
        loop {
            let mut request = ViewRequest::new("main:Catalog.Items.PredefinedItem")
                .unwrap()
                .with_limit(limit)
                .unwrap();
            if let Some(previous) = cursor.take() {
                request = request.with_cursor(previous);
            }
            let page = service.view(request);
            assert!(page.ok, "{page:?}");
            let data = page.data.unwrap();
            let items = data["items"].as_array().unwrap();
            assert!(!items.is_empty() && items.len() <= limit);
            names.extend(
                items
                    .iter()
                    .map(|item| item["at"].as_str().unwrap().to_owned()),
            );
            cursor = page.cursor;
            if cursor.is_none() {
                break;
            }
            assert!(names.len() < 72, "continuation must make progress");
        }
        let expected = (1..=72)
            .map(|index| format!("main:Catalog.Items.PredefinedItem.Item{index:03}"))
            .collect::<Vec<_>>();
        assert_eq!(
            names, expected,
            "limit {limit}: no missing or duplicate items"
        );
    }
}

/// Родитель элемента страничной фикстуры. Группы стоят в середине списка и
/// вложены в два уровня, а границы страниц 17 и 50 проходят внутри их
/// поддеревьев: обход в ширину переставил бы детей в хвост, и порядок
/// `Item001..Item072` перестал бы совпадать с документным.
fn predefined_page_parent(index: usize) -> Option<usize> {
    match index {
        17 | 18 | 21 => Some(16),
        19 | 20 => Some(18),
        51 | 52 | 54 => Some(50),
        53 => Some(52),
        _ => None,
    }
}

fn predefined_page_is_folder(index: usize) -> bool {
    matches!(index, 16 | 18 | 50 | 52)
}

fn push_predefined_page_item(items: &mut String, index: usize) {
    items.push_str(&format!(
        "<Item id=\"00000000-0000-4000-8000-{index:012}\"><Name>Item{index:03}</Name><Code>{index:03}</Code><Description>Description {index}</Description><IsFolder>{}</IsFolder>",
        predefined_page_is_folder(index),
    ));
    let children = (index + 1..=72)
        .filter(|child| predefined_page_parent(*child) == Some(index))
        .collect::<Vec<_>>();
    if !children.is_empty() {
        items.push_str("<ChildItems>");
        for child in children {
            push_predefined_page_item(items, child);
        }
        items.push_str("</ChildItems>");
    }
    items.push_str("</Item>");
}

fn predefined_page_fixture() -> RealReaderFixture {
    let fixture = RealReaderFixture::new();
    let mut items = String::new();
    for index in (1..=72).filter(|index| predefined_page_parent(*index).is_none()) {
        push_predefined_page_item(&mut items, index);
    }
    write(&fixture.source.join("Catalogs/Items/Ext/Predefined.xml"), &format!(
        "<PredefinedData xmlns=\"http://v8.1c.ru/8.3/xcf/predef\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"CatalogPredefinedItems\" version=\"2.20\">{items}</PredefinedData>"
    ));
    fixture
}

#[test]
fn predefined_items_empty_inapplicable_and_unreadable_are_distinct() {
    let fixture = RealReaderFixture::new();
    let path = fixture.source.join("Catalogs/Items/Ext/Predefined.xml");
    fs::remove_file(&path).unwrap();
    let service = fixture.view_service();
    let owner = service.view(ViewRequest::new("main:Catalog.Items").unwrap());
    assert!(owner.ok, "{owner:?}");
    let authority = fixture.read_authority();
    let target = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Items").unwrap();
    let empty_data = authority
        .read
        .predefined_items(
            &target,
            crate::domain::metadata::MetadataKind::Catalog,
            Some("String"),
            &mut || Ok(()),
        )
        .unwrap()
        .unwrap();
    assert_eq!(empty_data.total, 0);
    assert!(!empty_data.truncated);
    let empty = service.view(ViewRequest::new("main:Catalog.Items.PredefinedItem").unwrap());
    assert!(empty.ok, "{empty:?}");
    assert_eq!(empty.data.unwrap()["items"], json!([]));
    assert!(empty.cursor.is_none());
    let absent =
        service.view(ViewRequest::new("main:Catalog.Items.PredefinedItem.Absent").unwrap());
    assert_eq!(refusal_codes(&absent), ["not_found"]);
    let inapplicable =
        service.view(ViewRequest::new("main:Document.Order.PredefinedItem").unwrap());
    // Неприменимому виду отказано в самой коллекции, а не в элементе:
    // пустой список он не получает.
    assert!(!inapplicable.ok, "{inapplicable:?}");
    assert_eq!(refusal_codes(&inapplicable), ["not_found"]);
    assert_eq!(
        inapplicable.diagnostics[0]["message"],
        "metadata has no PredefinedItem collection"
    );
    assert!(inapplicable.cursor.is_none());

    write(&path, "<PredefinedData><broken>");
    let unreadable = fixture
        .view_service()
        .view(ViewRequest::new("main:Catalog.Items.PredefinedItem").unwrap());
    assert!(!unreadable.ok, "{unreadable:?}");
    assert_eq!(refusal_codes(&unreadable), ["provider_unavailable"]);
    assert_eq!(unreadable.diagnostics[0]["detailCode"], "source_unreadable");
    assert!(unreadable.cursor.is_none());
}

#[test]
fn predefined_items_cursor_replays_after_source_removal() {
    let fixture = predefined_page_fixture();
    let service = fixture.view_service();
    let request = || {
        ViewRequest::new("main:Catalog.Items.PredefinedItem")
            .unwrap()
            .with_limit(50)
            .unwrap()
    };
    let first = service.view(request());
    assert!(first.ok, "{first:?}");
    let cursor = first.cursor.expect("72 elements need a continuation");
    let before = service.view(request().with_cursor(cursor.clone()));
    assert!(before.ok, "{before:?}");
    fs::remove_file(fixture.source.join("Catalogs/Items/Ext/Predefined.xml")).unwrap();
    let after = service.view(request().with_cursor(cursor.clone()));
    assert!(after.ok, "{after:?}");
    assert_eq!(before.data, after.data);
    assert_eq!(before.cursor, after.cursor);
    let items = after.data.unwrap()["items"].as_array().unwrap().clone();
    assert_eq!(items.len(), 22);
    assert_eq!(items[0]["at"], "main:Catalog.Items.PredefinedItem.Item051");
    assert_eq!(items[21]["at"], "main:Catalog.Items.PredefinedItem.Item072");
    let tampered = service.view(request().with_cursor(format!("{cursor}x")));
    assert_eq!(refusal_codes(&tampered), ["invalid_cursor"]);
    let fresh = fixture.view_service().view(request());
    assert!(fresh.ok, "{fresh:?}");
    assert_eq!(fresh.data.unwrap()["items"], json!([]));
}

#[test]
fn predefined_items_refuse_file_and_snapshot_budget_without_cursor() {
    let fixture = predefined_page_fixture();
    let service = ViewService::new(
        fixture.read_authority(),
        ViewCursorStore::new(crate::application::result_store::DEFAULT_TTL, 128, 512),
    );
    let result = service.view(ViewRequest::new("main:Catalog.Items.PredefinedItem").unwrap());
    assert_eq!(refusal_codes(&result), ["result_too_large"]);
    assert!(result.cursor.is_none());

    write(
        &fixture.source.join("Catalogs/Items/Ext/Predefined.xml"),
        &"x".repeat(8 * 1024 * 1024 + 1),
    );
    let result = fixture
        .view_service()
        .view(ViewRequest::new("main:Catalog.Items.PredefinedItem").unwrap());
    assert!(!result.ok, "{result:?}");
    assert!(
        serde_json::to_string(&result.diagnostics)
            .unwrap()
            .contains("8388608-byte read limit"),
        "{result:?}"
    );
    assert!(result.cursor.is_none());
}

#[test]
fn predefined_items_checkpoint_preserves_cancellation_and_deadline() {
    use crate::domain::metadata::MetadataKind;
    let fixture = predefined_page_fixture();
    let authority = fixture.read_authority();
    let target = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Items").unwrap();
    let mut completed_checkpoints = 0;
    let data = authority
        .read
        .predefined_items(&target, MetadataKind::Catalog, Some("String"), &mut || {
            completed_checkpoints += 1;
            Ok(())
        })
        .unwrap()
        .unwrap();
    assert_eq!(data.total, 72);
    assert!(completed_checkpoints > 2 * data.total);
    for code in [RefusalCode::Cancelled, RefusalCode::DeadlineExceeded] {
        for stop_at in [2, 16, completed_checkpoints - 10] {
            let mut checkpoints = 0;
            let error = authority
                .read
                .predefined_items(&target, MetadataKind::Catalog, Some("String"), &mut || {
                    checkpoints += 1;
                    if checkpoints == stop_at {
                        Err(ViewError::new(code, "interrupted predefined read"))
                    } else {
                        Ok(())
                    }
                })
                .unwrap_err();
            assert_eq!(error.code(), code, "checkpoint {stop_at}");
            assert_eq!(checkpoints, stop_at);
        }
    }
}

#[test]
fn predefined_items_other_owner_fields_survive_addressed_reads() {
    use crate::domain::metadata::{MetadataType, MetadataTypeVariant};
    let fixture = RealReaderFixture::new();
    let config_path = fixture.source.join("Configuration.xml");
    let config = fs::read_to_string(&config_path).unwrap();
    write(&config_path, &config.replace("</ChildObjects>",
        "<ChartOfCharacteristicTypes>Characteristics</ChartOfCharacteristicTypes><ChartOfCalculationTypes>Calculations</ChartOfCalculationTypes></ChildObjects>"));
    for (kind, directory, name) in [
        (
            "ChartOfCharacteristicTypes",
            "ChartsOfCharacteristicTypes",
            "Characteristics",
        ),
        (
            "ChartOfCalculationTypes",
            "ChartsOfCalculationTypes",
            "Calculations",
        ),
    ] {
        write_metadata_owner(&fixture.source, directory, kind, name, "");
        if kind == "ChartOfCalculationTypes" {
            let path = fixture.source.join(format!("{directory}/{name}.xml"));
            let xml = fs::read_to_string(&path).unwrap();
            write(
                &path,
                &xml.replace("</Properties>", "<CodeType>String</CodeType></Properties>"),
            );
        }
    }
    for (directory, name, fixture_file) in [
        ("ChartsOfAccounts", "Accounts", "ChartOfAccounts.xml"),
        (
            "ChartsOfCharacteristicTypes",
            "Characteristics",
            "ChartOfCharacteristicTypes.xml",
        ),
        (
            "ChartsOfCalculationTypes",
            "Calculations",
            "ChartOfCalculationTypes.xml",
        ),
    ] {
        write(
            &fixture
                .source
                .join(format!("{directory}/{name}/Ext/Predefined.xml")),
            &fixture_text(&format!("platform_8_3_27/predefined/{fixture_file}")),
        );
    }
    let service = fixture.view_service();
    let account = service.view(
        ViewRequest::new("main:ChartOfAccounts.Accounts.PredefinedItem.ОСвОрганизации").unwrap(),
    );
    assert!(account.ok, "{account:?}");
    let props = &account.data.as_ref().unwrap()["props"];
    assert_eq!(props["accountType"], "Active");
    assert_eq!(props["offBalance"], false);
    assert_eq!(props["order"], " 01.01");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(props["accountingFlags"].as_str().unwrap())
            .unwrap(),
        json!({
            "ChartOfAccounts.Хозрасчетный.AccountingFlag.Валютный": false,
            "ChartOfAccounts.Хозрасчетный.AccountingFlag.Количественный": false,
            "ChartOfAccounts.Хозрасчетный.AccountingFlag.УчетПоПодразделениям": true,
            "ChartOfAccounts.Хозрасчетный.AccountingFlag.НалоговыйУчет": true,
        })
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(props["extDimensionTypes"].as_str().unwrap())
            .unwrap(),
        json!([{
            "name": "ChartOfCharacteristicTypes.ВидыСубконтоХозрасчетные.ОсновныеСредства",
            "turnover": false,
            "accountingFlags": {
                "ChartOfAccounts.Хозрасчетный.ExtDimensionAccountingFlag.Суммовой": true,
                "ChartOfAccounts.Хозрасчетный.ExtDimensionAccountingFlag.Валютный": true,
                "ChartOfAccounts.Хозрасчетный.ExtDimensionAccountingFlag.Количественный": true,
            },
        }])
    );
    let characteristic = service.view(ViewRequest::new("main:ChartOfCharacteristicTypes.Characteristics.PredefinedItem.УдалитьБухгалтерскийУчет").unwrap());
    assert!(characteristic.ok, "{characteristic:?}");
    assert_eq!(
        characteristic.data.as_ref().unwrap()["props"]["isFolder"],
        false
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(
            characteristic.data.as_ref().unwrap()["props"]["type"]
                .as_str()
                .unwrap(),
        )
        .unwrap(),
        json!(MetadataType::new(vec![MetadataTypeVariant::Reference {
            metadata_path: MetadataAddress::parse(
                PLATFORM_XML_8_3_27_FORMAT_2_20,
                "ChartOfCharacteristicTypes.РазделыДатЗапретаИзменения"
            )
            .unwrap(),
        }])
        .unwrap())
    );
    let calculation = service.view(
        ViewRequest::new("main:ChartOfCalculationTypes.Calculations.PredefinedItem.ОкладПоДням")
            .unwrap(),
    );
    assert!(calculation.ok, "{calculation:?}");
    assert_eq!(
        calculation.data.as_ref().unwrap()["props"]["actionPeriodIsBase"],
        false
    );
}

#[test]
fn predefined_items_answer_by_address_with_the_count_from_the_reader() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    let node = service.view(ViewRequest::new("main:Catalog.Items").unwrap());
    assert!(node.ok, "{:?}", refusal_codes(&node));
    let branches = node.data.as_ref().unwrap()["branches"].as_array().unwrap();
    // Счёт пришёл из ответа читателя, а не из длины страницы.
    assert!(
        branches.contains(&json!({"at": "main:Catalog.Items.PredefinedItem", "count": 1})),
        "{branches:#?}"
    );

    let page = service.view(ViewRequest::new("main:Catalog.Items.PredefinedItem").unwrap());
    assert!(page.ok, "{:?}", refusal_codes(&page));
    let items = page.data.as_ref().unwrap()["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["at"], "main:Catalog.Items.PredefinedItem.Основной");
    assert_eq!(items[0]["kind"], "PredefinedItem");

    let item =
        service.view(ViewRequest::new("main:Catalog.Items.PredefinedItem.Основной").unwrap());
    assert!(item.ok, "{:?}", refusal_codes(&item));
    let item = item.data.as_ref().unwrap();
    // Заголовок называет элемент его именем; представление — отдельный факт.
    assert_eq!(item["title"], "Основной");
    assert_eq!(item["props"]["description"], "Основной элемент");
    assert_eq!(item["props"]["id"], "c7d2e6fc-3824-4b56-b4be-ae6be4944c0e");

    // Вид без этой коллекции ветви не получает: неприменимое не отвечает
    // пустотой, его просто нет.
    let document = service.view(ViewRequest::new("main:Document.Order").unwrap());
    assert!(document.ok, "{:?}", refusal_codes(&document));
    let branch_addresses: Vec<&str> = document.data.as_ref().unwrap()["branches"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|branch| branch["at"].as_str())
        .collect();
    assert!(
        !branch_addresses
            .iter()
            .any(|at| at.ends_with(".PredefinedItem")),
        "{branch_addresses:?}"
    );
}

#[test]
fn metadata_tabular_section_attribute_consumes_the_complete_suffix() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    let result = service.view(
        ViewRequest::new("main:Catalog.Items.TabularSection.Lines.Attribute.Quantity").unwrap(),
    );

    assert!(result.ok, "{} {:?}", result.summary, result.diagnostics);
    let data = result.data.as_ref().unwrap();
    assert_eq!(
        data["at"],
        "main:Catalog.Items.TabularSection.Lines.Attribute.Quantity"
    );
    assert_eq!(data["kind"], "Attribute");
    assert_eq!(data["title"], "Quantity");
}

#[test]
fn role_right_rls_consumes_the_complete_suffix() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    let result =
        service.view(ViewRequest::new("main:Role.SalesReader.Right.Products.RLS.View").unwrap());

    assert!(result.ok, "{} {:?}", result.summary, result.diagnostics);
    let data = result.data.as_ref().unwrap();
    assert_eq!(
        data["at"],
        "main:Role.SalesReader.Right.Catalog_Products.RLS.View"
    );
    assert_eq!(data["kind"], "RLS");
    assert_eq!(data["title"], "View");
}

#[test]
fn role_merges_access_by_canonical_object_and_keeps_rls_under_that_right() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    let root = service.view(ViewRequest::new("main:Role.SalesReader").unwrap());
    assert!(root.ok, "{root:?}");
    let root_data = root.data.unwrap();
    assert_eq!(
        root_data["branches"],
        json!([{"at": "main:Role.SalesReader.Right", "count": 2}]),
    );

    let rights = service.view(ViewRequest::new("main:Role.SalesReader.Right").unwrap());
    assert!(rights.ok, "{rights:?}");
    let items = rights.data.unwrap()["items"].as_array().unwrap().clone();
    assert_eq!(items.len(), 2);
    assert_eq!(
        items
            .iter()
            .map(|item| item["at"].as_str().unwrap())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        2,
    );

    let alias = service.view(ViewRequest::new("main:Role.SalesReader.Right.Products").unwrap());
    assert!(alias.ok, "{alias:?}");
    let alias_data = alias.data.unwrap();
    assert_eq!(
        alias_data["at"],
        "main:Role.SalesReader.Right.Catalog_Products"
    );
    assert_eq!(alias_data["props"]["allowedCount"], 2);
    assert_eq!(alias_data["props"]["deniedCount"], 1);
    assert_eq!(
        alias_data["branches"],
        json!([{
            "at": "main:Role.SalesReader.Right.Catalog_Products.RLS",
            "count": 1,
        }]),
    );

    let rls_branch =
        service.view(ViewRequest::new("main:Role.SalesReader.Right.Catalog_Products.RLS").unwrap());
    assert!(rls_branch.ok, "{rls_branch:?}");
    let rls_items = rls_branch.data.unwrap()["items"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(rls_items.len(), 1);
    assert_eq!(
        rls_items
            .iter()
            .map(|item| item["at"].as_str().unwrap())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        1,
    );

    let rls = service
        .view(ViewRequest::new("main:Role.SalesReader.Right.Catalog_Products.RLS.View").unwrap());
    assert!(rls.ok, "{rls:?}");
    assert_eq!(
        rls.data.unwrap()["at"],
        "main:Role.SalesReader.Right.Catalog_Products.RLS.View"
    );
}

#[test]
fn form_table_column_event_consumes_arbitrary_depth_and_preserves_owner_address() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    let event_at = "main:Report.ParityReport.Form.MainForm.Item.Goods.Item.Quantity.Event.OnChange";

    let event = service.view(ViewRequest::new(event_at).unwrap());
    assert!(event.ok, "{} {:?}", event.summary, event.diagnostics);
    let event_data = event.data.as_ref().unwrap();
    assert_eq!(event_data["at"], event_at);
    assert_eq!(event_data["kind"], "Event");

    let method = service.view(
        ViewRequest::new(
            "main:Report.ParityReport.Form.MainForm.Module.Form.Method.QuantityOnChange",
        )
        .unwrap(),
    );
    assert!(method.ok, "{} {:?}", method.summary, method.diagnostics);
    let method_data = method.data.as_ref().unwrap();
    assert!(
        method_data["props"]["handles"]
            .as_array()
            .is_some_and(|handles| handles.iter().any(|handle| {
                handle["owner"] == "column"
                    && handle["at"]
                        == "main:Report.ParityReport.Form.MainForm.Item.Goods.Item.Quantity"
                    && handle["event"] == "OnChange"
            })),
        "{method_data}"
    );
}

#[test]
fn pure_event_source_resolver_returns_relative_platform_and_nested_form_identities() {
    use crate::infrastructure::event_projection::resolve_property_event_source;
    use crate::infrastructure::logical_event_source::{
        resolve_event_source, LogicalEventSource, PropertyEventOwnerKind,
    };

    let platform = resolve_event_source(
        "main",
        SourceSetKind::Configuration,
        PlatformProfile::v8_3_27(),
        &QualifiedAddress::parse("main:Document.Заказ.Module.Object.Event.BeforeWrite").unwrap(),
    )
    .unwrap();
    let LogicalEventSource::Platform(platform) = platform else {
        panic!("platform event must resolve to a platform module")
    };
    assert_eq!(
        platform.module_at.to_string(),
        "main:Document.Заказ.Module.Object"
    );
    assert_eq!(
        platform.module_target.as_str(),
        "Document.Заказ.ObjectModule"
    );
    assert_eq!(
        platform.module_relative,
        PathBuf::from("Documents/Заказ/Ext/ObjectModule.bsl")
    );
    assert_eq!(
        platform.descriptor_requirements,
        vec![PathBuf::from("Documents/Заказ.xml")]
    );

    let fixture = RealReaderFixture::new();
    let property_at = QualifiedAddress::parse(
        "main:Report.ParityReport.Form.MainForm.Item.Goods.Item.Quantity.Event.OnChange",
    )
    .unwrap();
    let form_xml = fs::read_to_string(
        fixture
            .source
            .join("Reports/ParityReport/Forms/MainForm/Ext/Form.xml"),
    )
    .unwrap();
    let property = resolve_property_event_source(
        "main",
        SourceSetKind::Configuration,
        PlatformProfile::v8_3_27(),
        &property_at,
        &form_xml,
    )
    .unwrap();
    assert_eq!(
        property.form_xml_relative,
        PathBuf::from("Reports/ParityReport/Forms/MainForm/Ext/Form.xml")
    );
    assert_eq!(
        property.module_relative,
        PathBuf::from("Reports/ParityReport/Forms/MainForm/Ext/Form/Module.bsl")
    );
    assert_eq!(
        property
            .owner_chain
            .iter()
            .map(|owner| owner.kind)
            .collect::<Vec<_>>(),
        vec![
            PropertyEventOwnerKind::Form,
            PropertyEventOwnerKind::Table,
            PropertyEventOwnerKind::Column,
        ]
    );
    assert_eq!(
        property.descriptor_requirements,
        vec![
            PathBuf::from("Reports/ParityReport.xml"),
            PathBuf::from("Reports/ParityReport/Forms/MainForm.xml"),
        ]
    );

    let external = resolve_event_source(
        "epf",
        SourceSetKind::ExternalProcessor,
        PlatformProfile::v8_3_27(),
        &QualifiedAddress::parse(
            "epf:ExternalDataProcessor.Import.Module.Object.Event.BeforeWrite",
        )
        .unwrap(),
    )
    .unwrap();
    let LogicalEventSource::Platform(external) = external else {
        panic!("external object event must resolve to its object module")
    };
    assert_eq!(
        external.module_relative,
        PathBuf::from("Import/Ext/ObjectModule.bsl")
    );
    assert_eq!(
        external.descriptor_requirements,
        vec![PathBuf::from("Import.xml")]
    );
}

#[test]
fn pure_event_source_resolver_requires_form_evidence_for_every_item_depth() {
    use crate::infrastructure::logical_event_source::resolve_event_source;

    for raw in [
        "main:Report.ParityReport.Form.MainForm.Item.Field.Event.OnChange",
        "main:Report.ParityReport.Form.MainForm.Item.Rows.Item.Column.Event.OnChange",
        "main:Report.ParityReport.Form.MainForm.Item.Tabs.Item.Page.Item.Field.Event.OnChange",
    ] {
        let error = resolve_event_source(
            "main",
            SourceSetKind::Configuration,
            PlatformProfile::v8_3_27(),
            &QualifiedAddress::parse(raw).unwrap(),
        )
        .unwrap_err();
        assert_eq!(
            error.code().as_str(),
            "provider_unavailable",
            "{raw}: {error}"
        );
    }
}

#[test]
fn form_evidence_resolver_returns_complete_typed_owner_chains_at_arbitrary_depth() {
    use crate::infrastructure::event_projection::{
        project_property_event, resolve_property_event_source,
    };
    use crate::infrastructure::logical_event_source::PropertyEventOwnerKind;
    use crate::infrastructure::v13_read::event_node_value;

    let fixture = RealReaderFixture::new();
    fixture.install_main_form_sources(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <ChildItems>
    <InputField name="Field"><Events><Event name="OnChange">FieldChanged</Event></Events></InputField>
    <Table name="Rows"><Events><Event name="Selection">RowsSelected</Event></Events><ChildItems>
      <InputField name="Column"><Events><Event name="OnChange">ColumnChanged</Event></Events></InputField>
    </ChildItems></Table>
    <UsualGroup name="Group"><ChildItems>
      <InputField name="GroupedField"><Events><Event name="OnChange">GroupedFieldChanged</Event></Events></InputField>
    </ChildItems></UsualGroup>
    <Pages name="Tabs"><ChildItems><Page name="FirstPage"><ChildItems>
      <InputField name="PageField"><Events><Event name="OnChange">PageFieldChanged</Event></Events></InputField>
    </ChildItems></Page></ChildItems></Pages>
  </ChildItems>
</Form>"#,
        "",
    );
    let form_xml = fs::read_to_string(
        fixture
            .source
            .join("Reports/ParityReport/Forms/MainForm/Ext/Form.xml"),
    )
    .unwrap();
    let cases = [
        (
            "main:Report.ParityReport.Form.MainForm.Item.Field.Event.OnChange",
            vec![
                PropertyEventOwnerKind::Form,
                PropertyEventOwnerKind::Element,
            ],
            vec![
                "main:Report.ParityReport.Form.MainForm",
                "main:Report.ParityReport.Form.MainForm.Item.Field",
            ],
        ),
        (
            "main:Report.ParityReport.Form.MainForm.Item.Rows.Event.Selection",
            vec![
                PropertyEventOwnerKind::Form,
                PropertyEventOwnerKind::Table,
            ],
            vec![
                "main:Report.ParityReport.Form.MainForm",
                "main:Report.ParityReport.Form.MainForm.Item.Rows",
            ],
        ),
        (
            "main:Report.ParityReport.Form.MainForm.Item.Rows.Item.Column.Event.OnChange",
            vec![
                PropertyEventOwnerKind::Form,
                PropertyEventOwnerKind::Table,
                PropertyEventOwnerKind::Column,
            ],
            vec![
                "main:Report.ParityReport.Form.MainForm",
                "main:Report.ParityReport.Form.MainForm.Item.Rows",
                "main:Report.ParityReport.Form.MainForm.Item.Rows.Item.Column",
            ],
        ),
        (
            "main:Report.ParityReport.Form.MainForm.Item.Group.Item.GroupedField.Event.OnChange",
            vec![
                PropertyEventOwnerKind::Form,
                PropertyEventOwnerKind::Element,
                PropertyEventOwnerKind::Element,
            ],
            vec![
                "main:Report.ParityReport.Form.MainForm",
                "main:Report.ParityReport.Form.MainForm.Item.Group",
                "main:Report.ParityReport.Form.MainForm.Item.Group.Item.GroupedField",
            ],
        ),
        (
            "main:Report.ParityReport.Form.MainForm.Item.Tabs.Item.FirstPage.Item.PageField.Event.OnChange",
            vec![
                PropertyEventOwnerKind::Form,
                PropertyEventOwnerKind::Element,
                PropertyEventOwnerKind::Element,
                PropertyEventOwnerKind::Element,
            ],
            vec![
                "main:Report.ParityReport.Form.MainForm",
                "main:Report.ParityReport.Form.MainForm.Item.Tabs",
                "main:Report.ParityReport.Form.MainForm.Item.Tabs.Item.FirstPage",
                "main:Report.ParityReport.Form.MainForm.Item.Tabs.Item.FirstPage.Item.PageField",
            ],
        ),
    ];
    let service = fixture.view_service();
    for (raw, expected_kinds, expected_addresses) in cases {
        let at = QualifiedAddress::parse(raw).unwrap();
        let source = resolve_property_event_source(
            "main",
            SourceSetKind::Configuration,
            PlatformProfile::v8_3_27(),
            &at,
            &form_xml,
        )
        .unwrap();
        assert_eq!(
            source
                .owner_chain
                .iter()
                .map(|owner| owner.kind)
                .collect::<Vec<_>>(),
            expected_kinds,
            "{raw}"
        );
        assert_eq!(
            source
                .owner_chain
                .iter()
                .map(|owner| owner.at.to_string())
                .collect::<Vec<_>>(),
            expected_addresses,
            "{raw}"
        );
        let pure = project_property_event(&source, &form_xml, Some("")).unwrap();
        let physical = service.view(ViewRequest::new(raw).unwrap());
        assert!(
            physical.ok,
            "{raw}: {} {:?}",
            physical.summary, physical.diagnostics
        );
        assert_eq!(physical.data.as_ref().unwrap()["props"]["state"], "missing");
        assert_eq!(physical.data.unwrap(), event_node_value(&pure));
    }
}

#[test]
fn pure_event_source_resolver_fails_closed_for_unproved_event_layouts() {
    use crate::infrastructure::logical_event_source::resolve_event_source;

    for (kind, raw) in [
        (
            SourceSetKind::Extension,
            "main:Document.Заказ.Module.Object.Event.BeforeWrite",
        ),
        (
            SourceSetKind::Configuration,
            "main:WebSocketClient.Телефония.Module.WebSocketClient.Event.OnMessage",
        ),
        (
            SourceSetKind::Configuration,
            "main:CommonModule.ЗаказыСервер.Event.BeforeStart",
        ),
        (
            SourceSetKind::Configuration,
            "main:EventSubscription.Изменение.Event.Change",
        ),
        (
            SourceSetKind::Configuration,
            "main:Report.ParityReport.Form.MainForm.Item.Rows.Event.Selection",
        ),
    ] {
        let error = resolve_event_source(
            "main",
            kind,
            PlatformProfile::v8_3_27(),
            &QualifiedAddress::parse(raw).unwrap(),
        )
        .unwrap_err();
        assert_eq!(
            error.code().as_str(),
            "provider_unavailable",
            "{raw}: {error}"
        );
    }
}

#[test]
fn pure_event_projectors_match_physical_view_service_event_nodes() {
    use crate::infrastructure::event_projection::{
        project_platform_event, project_property_event, resolve_property_event_source,
    };
    use crate::infrastructure::logical_event_source::{resolve_event_source, LogicalEventSource};
    use crate::infrastructure::v13_read::event_node_value;

    let profile = PlatformProfile::v8_3_27();
    let platform_at =
        QualifiedAddress::parse("main:Document.Заказ.Module.Object.Event.BeforeWrite").unwrap();
    for source in [
        None,
        Some("Процедура ПередЗаписью(Отказ, РежимЗаписи, РежимПроведения)\nКонецПроцедуры"),
        Some("Функция ПередЗаписью(Отказ, РежимЗаписи, РежимПроведения)\nКонецФункции"),
    ] {
        let fixture = RealReaderFixture::new();
        fixture.install_accepted_profile_sources();
        if let Some(source) = source {
            write(
                &fixture.source.join("Documents/Заказ/Ext/ObjectModule.bsl"),
                source,
            );
        }
        let LogicalEventSource::Platform(resolved) =
            resolve_event_source("main", SourceSetKind::Configuration, profile, &platform_at)
                .unwrap()
        else {
            panic!("platform event must resolve to a module source")
        };
        let capability = profile.module_prefix_capability(&platform_at).unwrap();
        let pure = project_platform_event(&resolved, capability, source).unwrap();
        let physical = fixture
            .view_service()
            .view(ViewRequest::new(&platform_at.to_string()).unwrap());
        assert!(
            physical.ok,
            "{} {:?}",
            physical.summary, physical.diagnostics
        );
        assert_eq!(physical.data.unwrap(), event_node_value(&pure));
    }

    let fixture = RealReaderFixture::new();
    let property_at = QualifiedAddress::parse(
        "main:Report.ParityReport.Form.MainForm.Item.Goods.Item.Quantity.Event.OnChange",
    )
    .unwrap();
    let form_xml = fs::read_to_string(
        fixture
            .source
            .join("Reports/ParityReport/Forms/MainForm/Ext/Form.xml"),
    )
    .unwrap();
    let resolved = resolve_property_event_source(
        "main",
        SourceSetKind::Configuration,
        profile,
        &property_at,
        &form_xml,
    )
    .unwrap();
    let module_bsl = fs::read_to_string(fixture.source.join(&resolved.module_relative)).unwrap();
    let pure = project_property_event(&resolved, &form_xml, Some(&module_bsl)).unwrap();
    let physical = fixture
        .view_service()
        .view(ViewRequest::new(&property_at.to_string()).unwrap());
    assert!(
        physical.ok,
        "{} {:?}",
        physical.summary, physical.diagnostics
    );
    assert_eq!(physical.data.unwrap(), event_node_value(&pure));
}

#[test]
fn pure_form_projector_matches_every_physical_owner_and_all_event_states() {
    use crate::infrastructure::event_projection::{
        project_property_event, resolve_property_event_source,
    };
    use crate::infrastructure::v13_read::event_node_value;

    let fixture = RealReaderFixture::new();
    fixture.install_main_form_sources(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <ChildItems>
    <InputField name="Field"/>
    <Table name="Rows"><DataPath>Rows</DataPath>
      <Events><Event name="Selection">MissingSelection</Event></Events>
      <ChildItems><InputField name="Column"><Events><Event name="OnChange">WrongColumn</Event></Events></InputField></ChildItems>
    </Table>
  </ChildItems>
  <Events><Event name="OnOpen">FormOpen</Event></Events>
  <Commands><Command name="Run" id="1"><Action>RunAction</Action></Command></Commands>
</Form>"#,
        concat!(
            "&AtClient\nProcedure FormOpen(Cancel)\nEndProcedure\n\n",
            "Procedure WrongColumn()\nEndProcedure\n\n",
            "&AtClient\nProcedure RunAction(Command)\nEndProcedure\n",
        ),
    );
    let form_path = fixture
        .source
        .join("Reports/ParityReport/Forms/MainForm/Ext/Form.xml");
    let module_path = fixture
        .source
        .join("Reports/ParityReport/Forms/MainForm/Ext/Form/Module.bsl");
    let form_xml = fs::read_to_string(form_path).unwrap();
    let module_bsl = fs::read_to_string(module_path).unwrap();
    let service = fixture.view_service();
    for (event_at, state) in [
        (
            "main:Report.ParityReport.Form.MainForm.Event.OnOpen",
            "implemented",
        ),
        (
            "main:Report.ParityReport.Form.MainForm.Item.Field.Event.OnChange",
            "available",
        ),
        (
            "main:Report.ParityReport.Form.MainForm.Item.Rows.Event.Selection",
            "missing",
        ),
        (
            "main:Report.ParityReport.Form.MainForm.Item.Rows.Item.Column.Event.OnChange",
            "invalid",
        ),
        (
            "main:Report.ParityReport.Form.MainForm.Command.Run.Event.Execute",
            "implemented",
        ),
    ] {
        let at = QualifiedAddress::parse(event_at).unwrap();
        let source = resolve_property_event_source(
            "main",
            SourceSetKind::Configuration,
            PlatformProfile::v8_3_27(),
            &at,
            &form_xml,
        )
        .unwrap();
        let pure = project_property_event(&source, &form_xml, Some(&module_bsl)).unwrap();
        let physical = service.view(ViewRequest::new(event_at).unwrap());
        assert!(
            physical.ok,
            "{} {:?}",
            physical.summary, physical.diagnostics
        );
        assert_eq!(physical.data.as_ref().unwrap()["props"]["state"], state);
        assert_eq!(physical.data.unwrap(), event_node_value(&pure));
    }
}

#[test]
fn production_form_command_execute_has_one_semantic_identity_across_view_and_find() {
    let fixture = RealReaderFixture::new();
    fixture.install_main_form_sources(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <Commands>
    <Command name="Zero" id="1"/>
    <Command name="Missing" id="2"><Action>MissingAction</Action></Command>
    <Command name="Implemented" id="3"><Action>ImplementedAction</Action></Command>
    <Command name="Duplicate" id="4"><Action>FirstAction</Action><Action>SecondAction</Action></Command>
  </Commands>
</Form>"#,
        "&НаКлиенте\nПроцедура ImplementedAction(Команда)\nКонецПроцедуры\n",
    );
    let service = fixture.view_service();
    let form_at = "main:Report.ParityReport.Form.MainForm";
    let expected_contexts = json!([
        "thinClient",
        "webClient",
        "thickClientManaged",
        "mobileClient",
        "mobileAppClient"
    ]);
    for (command, state, handler, has_can) in [
        ("Zero", "available", Some("ОбработкаКоманды"), true),
        ("Missing", "missing", Some("MissingAction"), true),
        (
            "Implemented",
            "implemented",
            Some("ImplementedAction"),
            false,
        ),
        ("Duplicate", "invalid", None, false),
    ] {
        let command_at = format!("{form_at}.Command.{command}");
        let event_at = format!("{command_at}.Event.Execute");
        let collection = service.view(ViewRequest::new(&format!("{command_at}.Event")).unwrap());
        assert!(
            collection.ok,
            "{command}: {} {:?}",
            collection.summary, collection.diagnostics
        );
        let items = collection.data.as_ref().unwrap()["items"]
            .as_array()
            .unwrap();
        assert_eq!(items.len(), 1, "{command}: {items:?}");
        assert_eq!(items[0]["at"], event_at);
        assert_eq!(items[0]["props"]["eventId"], "Execute");
        assert_eq!(items[0]["props"]["state"], state);
        assert_eq!(items[0]["props"]["contexts"], expected_contexts);
        if let Some(handler) = handler {
            assert_eq!(items[0]["props"]["handler"], handler);
            let expected_signature = format!("Процедура {handler}(Команда)");
            assert_eq!(items[0]["props"]["signature"], expected_signature);
        }
        assert_eq!(
            items[0].get("can").is_some(),
            has_can,
            "{command}: {}",
            items[0]
        );
        if has_can {
            assert_eq!(items[0]["can"].as_array().unwrap().len(), 1);
            assert_eq!(items[0]["can"][0]["op"], "event.implement");
            assert_eq!(items[0]["can"][0]["args"], json!({"at": event_at}));
        }
        if state == "missing" {
            assert!(items[0]["props"]["implementationAt"].is_null());
        }
        if state == "implemented" {
            assert_eq!(
                items[0]["props"]["implementationAt"],
                format!("{form_at}.Module.Form.Method.ImplementedAction")
            );
        }

        let direct = service.view(ViewRequest::new(&event_at).unwrap());
        assert!(
            direct.ok,
            "{command}: {} {:?}",
            direct.summary, direct.diagnostics
        );
        assert_eq!(direct.data.as_ref().unwrap(), &items[0]);
    }

    let authority = fixture.operation_read_authority();
    let index = ReaderReach::new(vec![("main", &authority)]);
    for command in ["Zero", "Missing", "Implemented", "Duplicate"] {
        let event_at = format!("{form_at}.Command.{command}.Event.Execute");
        let found = index.find(
            FindRequest::new(&event_at)
                .unwrap()
                .with_limit(100)
                .unwrap(),
        );
        assert!(!found.is_nearest(), "{event_at}: {found:?}");
        assert_eq!(
            found
                .candidates()
                .iter()
                .filter(|candidate| candidate.at() == event_at)
                .count(),
            1,
            "{event_at}: {found:?}"
        );
    }
}

#[test]
fn production_form_command_execute_requires_an_exact_client_directive() {
    let fixture = RealReaderFixture::new();
    let module_bsl = concat!(
        "&НаКлиенте\n",
        "Процедура RussianClientAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "&AtClient\n",
        "Процедура EnglishClientAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "&наклиенте\n",
        "Процедура RussianLowerAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "&нАкЛиЕнТе\n",
        "Процедура RussianMixedAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "&atclient\n",
        "Процедура EnglishLowerAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "&aTcLiEnT\n",
        "Процедура EnglishMixedAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "  &AtClient  \r\n",
        "Процедура WhitespaceCrLfAction(Команда)\r\n",
        "КонецПроцедуры\r\n\r\n",
        "Процедура MissingDirectiveAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "&наклиентенасервере\n",
        "Процедура RussianClientServerLowerAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "&aTcLiEnTaTsErVeR\n",
        "Процедура EnglishClientServerMixedAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "#Если Клиент Тогда\n",
        "Процедура GuardOnlyAction(Команда)\n",
        "КонецПроцедуры\n",
        "#КонецЕсли\n",
    );
    fixture.install_main_form_sources(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <Commands>
    <Command name="RussianClient" id="1"><Action>RussianClientAction</Action></Command>
    <Command name="EnglishClient" id="2"><Action>EnglishClientAction</Action></Command>
    <Command name="RussianLower" id="3"><Action>RussianLowerAction</Action></Command>
    <Command name="RussianMixed" id="4"><Action>RussianMixedAction</Action></Command>
    <Command name="EnglishLower" id="5"><Action>EnglishLowerAction</Action></Command>
    <Command name="EnglishMixed" id="6"><Action>EnglishMixedAction</Action></Command>
    <Command name="WhitespaceCrLf" id="7"><Action>WhitespaceCrLfAction</Action></Command>
    <Command name="MissingDirective" id="8"><Action>MissingDirectiveAction</Action></Command>
    <Command name="RussianClientServerLower" id="9"><Action>RussianClientServerLowerAction</Action></Command>
    <Command name="EnglishClientServerMixed" id="10"><Action>EnglishClientServerMixedAction</Action></Command>
    <Command name="GuardOnly" id="11"><Action>GuardOnlyAction</Action></Command>
  </Commands>
</Form>"#,
        module_bsl,
    );
    let service = fixture.view_service();
    let form_at = "main:Report.ParityReport.Form.MainForm";
    for (command, handler, state) in [
        ("RussianClient", "RussianClientAction", "implemented"),
        ("EnglishClient", "EnglishClientAction", "implemented"),
        ("RussianLower", "RussianLowerAction", "implemented"),
        ("RussianMixed", "RussianMixedAction", "implemented"),
        ("EnglishLower", "EnglishLowerAction", "implemented"),
        ("EnglishMixed", "EnglishMixedAction", "implemented"),
        ("WhitespaceCrLf", "WhitespaceCrLfAction", "implemented"),
        ("MissingDirective", "MissingDirectiveAction", "invalid"),
        (
            "RussianClientServerLower",
            "RussianClientServerLowerAction",
            "invalid",
        ),
        (
            "EnglishClientServerMixed",
            "EnglishClientServerMixedAction",
            "invalid",
        ),
        ("GuardOnly", "GuardOnlyAction", "invalid"),
    ] {
        let command_at = format!("{form_at}.Command.{command}");
        let event_at = format!("{command_at}.Event.Execute");
        let collection = service.view(ViewRequest::new(&format!("{command_at}.Event")).unwrap());
        assert!(
            collection.ok,
            "{command}: {} {:?}",
            collection.summary, collection.diagnostics
        );
        let items = collection.data.as_ref().unwrap()["items"]
            .as_array()
            .unwrap();
        assert_eq!(items.len(), 1, "{command}: {items:?}");
        assert_eq!(items[0]["at"], event_at);
        assert_eq!(items[0]["props"]["eventId"], "Execute");
        assert_eq!(items[0]["props"]["state"], state, "{command}: {}", items[0]);
        assert_eq!(
            items[0]["props"]["implementationAt"],
            format!("{form_at}.Module.Form.Method.{handler}")
        );
        assert!(items[0].get("can").is_none(), "{command}: {}", items[0]);

        let direct = service.view(ViewRequest::new(&event_at).unwrap());
        assert!(
            direct.ok,
            "{command}: {} {:?}",
            direct.summary, direct.diagnostics
        );
        assert_eq!(direct.data.as_ref().unwrap(), &items[0]);
    }

    let authority = fixture.operation_read_authority();
    let index = ReaderReach::new(vec![("main", &authority)]);
    for command in [
        "RussianClient",
        "EnglishClient",
        "RussianLower",
        "RussianMixed",
        "EnglishLower",
        "EnglishMixed",
        "WhitespaceCrLf",
        "MissingDirective",
        "RussianClientServerLower",
        "EnglishClientServerMixed",
        "GuardOnly",
    ] {
        let event_at = format!("{form_at}.Command.{command}.Event.Execute");
        let found = index.find(
            FindRequest::new(&event_at)
                .unwrap()
                .with_limit(100)
                .unwrap(),
        );
        assert!(!found.is_nearest(), "{event_at}: {found:?}");
        assert_eq!(
            found
                .candidates()
                .iter()
                .filter(|candidate| candidate.at() == event_at)
                .count(),
            1,
            "{event_at}: {found:?}"
        );
    }
}

#[test]
fn production_borrowed_command_defaults_after_and_base_form_only_has_no_can() {
    let borrowed = RealReaderFixture::new();
    borrowed.install_main_form_sources(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <Commands><Command name="Borrowed" id="1"/></Commands>
  <BaseForm version="2.20"/>
</Form>"#,
        "",
    );
    let event_at = "main:Report.ParityReport.Form.MainForm.Command.Borrowed.Event.Execute";
    let event = borrowed
        .view_service()
        .view(ViewRequest::new(event_at).unwrap());
    assert!(event.ok, "{} {:?}", event.summary, event.diagnostics);
    let event = event.data.as_ref().unwrap();
    assert_eq!(event["props"]["state"], "available");
    assert_eq!(event["can"][0]["op"], "event.implement");
    assert_eq!(
        event["can"][0]["args"],
        json!({"at": event_at, "callType": "After"})
    );

    let base_only = RealReaderFixture::new();
    base_only.install_main_form_sources(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <BaseForm version="2.20"/>
</Form>"#,
        "",
    );
    let event = base_only
        .view_service()
        .view(ViewRequest::new("main:Report.ParityReport.Form.MainForm.Event.OnOpen").unwrap());
    assert!(event.ok, "{} {:?}", event.summary, event.diagnostics);
    let event = event.data.as_ref().unwrap();
    assert_eq!(event["props"]["state"], "available");
    assert!(event.get("can").is_none(), "{event}");
}

#[test]
fn production_form_empty_handlers_are_invalid_for_every_property_owner() {
    let regular = RealReaderFixture::new();
    regular.install_main_form_sources(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <ChildItems>
    <InputField name="Field"><Events><Event name="OnChange">   </Event></Events></InputField>
    <Table name="Rows">
      <DataPath>Rows</DataPath>
      <Events><Event name="Selection"></Event></Events>
      <ChildItems>
        <InputField name="Column"><Events><Event name="OnChange">
        </Event></Events></InputField>
      </ChildItems>
    </Table>
  </ChildItems>
  <Events><Event name="OnOpen"> </Event></Events>
  <Commands><Command name="Regular" id="1"><Action>   </Action></Command></Commands>
</Form>"#,
        "",
    );
    let service = regular.view_service();
    for event_at in [
        "main:Report.ParityReport.Form.MainForm.Event.OnOpen",
        "main:Report.ParityReport.Form.MainForm.Item.Field.Event.OnChange",
        "main:Report.ParityReport.Form.MainForm.Item.Rows.Event.Selection",
        "main:Report.ParityReport.Form.MainForm.Item.Rows.Item.Column.Event.OnChange",
        "main:Report.ParityReport.Form.MainForm.Command.Regular.Event.Execute",
    ] {
        let event = service.view(ViewRequest::new(event_at).unwrap());
        assert!(
            event.ok,
            "{event_at}: {} {:?}",
            event.summary, event.diagnostics
        );
        let event = event.data.as_ref().unwrap();
        assert_eq!(event["props"]["state"], "invalid", "{event_at}: {event}");
        assert!(event.get("can").is_none(), "{event_at}: {event}");
        assert!(
            event["props"]
                .as_object()
                .unwrap()
                .contains_key("implementationAt"),
            "{event_at}: {event}"
        );
    }

    let borrowed = RealReaderFixture::new();
    borrowed.install_main_form_sources(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <Commands><Command name="Borrowed" id="1"><Action callType="After">   </Action></Command></Commands>
  <BaseForm version="2.20"/>
</Form>"#,
        "",
    );
    let event_at = "main:Report.ParityReport.Form.MainForm.Command.Borrowed.Event.Execute";
    let event = borrowed
        .view_service()
        .view(ViewRequest::new(event_at).unwrap());
    assert!(event.ok, "{} {:?}", event.summary, event.diagnostics);
    let event = event.data.as_ref().unwrap();
    assert_eq!(event["props"]["state"], "invalid", "{event}");
    assert!(event.get("can").is_none(), "{event}");
    assert!(event["props"]
        .as_object()
        .unwrap()
        .contains_key("implementationAt"));
}

#[test]
fn dcs_dataset_field_and_parameter_consume_the_complete_suffix() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    for (at, kind, title) in [
        (
            "main:Report.ParityReport.Template.MainSchema.DataSet.MainData.Field.Code",
            "Field",
            "Code",
        ),
        (
            "main:Report.ParityReport.Template.MainSchema.DataSet.MainData.Parameter.Period",
            "Parameter",
            "Period",
        ),
    ] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(
            result.ok,
            "{at}: {} {:?}",
            result.summary, result.diagnostics
        );
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["at"], at);
        assert_eq!(data["kind"], kind);
        assert_eq!(data["title"], title);
    }
}

/// Имя с точкой — один сегмент адреса: читатель публикует его с `\.`, и по
/// опубликованному адресу узел читается. Составной `dataPath` поля схемы,
/// группа `CommandGroup.X` интерфейса, право на вложенный объект роли и
/// область с точкой в имени прежде получали адрес, который грамматика
/// не разбирала, или не получали его вовсе.
#[test]
fn names_with_a_dot_are_published_escaped_and_read_back() {
    let fixture = RealReaderFixture::new();
    let schema = fixture
        .source
        .join("Reports/ParityReport/Templates/MainSchema/Ext/Template.xml");
    let text = fs::read_to_string(&schema).unwrap();
    write(
        &schema,
        &text.replacen(
            "<field xsi:type=\"DataSetFieldField\">",
            "<field xsi:type=\"DataSetFieldField\"><dataPath>Owner.Code</dataPath><field>Owner.Code</field></field>\n\t\t<field xsi:type=\"DataSetFieldField\">",
            1,
        ),
    );
    let rights = fixture.source.join("Roles/SalesReader/Ext/Rights.xml");
    let text = fs::read_to_string(&rights).unwrap();
    write(
        &rights,
        &text.replacen(
            "    <object>",
            "    <object>\n        <name>Catalog.Products.Command.Print</name>\n        <right>\n            <name>View</name>\n            <value>true</value>\n        </right>\n    </object>\n    <object>",
            1,
        ),
    );
    let print = fixture
        .source
        .join("Reports/ParityReport/Templates/Print/Ext/Template.xml");
    let text = fs::read_to_string(&print).unwrap();
    write(
        &print,
        &text.replace("<name>Header</name>", "<name>Header.Total</name>"),
    );
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Ext/CommandInterface.xml"),
        r#"<?xml version="1.0" encoding="utf-8"?>
<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20">
	<CommandsOrder>
		<Command name="Catalog.Items.StandardCommand.OpenList">
			<CommandGroup>CommandGroup.Pricing</CommandGroup>
		</Command>
	</CommandsOrder>
	<GroupsOrder>
		<Group>NavigationPanelImportant</Group>
		<Group>CommandGroup.Pricing</Group>
	</GroupsOrder>
</CommandInterface>
"#,
    );
    let service = fixture.view_service();
    // Член коллекции по опубликованному адресу: имя и вид совпадают.
    let member = |branch: &str, title: &str| -> String {
        let listed = service.view(ViewRequest::new(branch).unwrap());
        assert!(listed.ok, "{branch}: {:?}", refusal_codes(&listed));
        let items = listed.data.as_ref().unwrap()["items"]
            .as_array()
            .unwrap()
            .clone();
        let at = items
            .iter()
            .find(|item| item["title"] == title)
            .and_then(|item| item["at"].as_str())
            .unwrap_or_else(|| panic!("{branch} lists `{title}` with an address"))
            .to_string();
        let read = service.view(ViewRequest::new(&at).unwrap());
        assert!(read.ok, "{at}: {:?}", refusal_codes(&read));
        assert_eq!(read.data.as_ref().unwrap()["at"], at.as_str());
        at
    };
    assert_eq!(
        member(
            "main:Report.ParityReport.Template.MainSchema.DataSet.MainData.Field",
            "Owner.Code"
        ),
        r"main:Report.ParityReport.Template.MainSchema.DataSet.MainData.Field.Owner\.Code"
    );
    assert_eq!(
        member("main:Role.SalesReader.Right", "Products.Command.Print"),
        r"main:Role.SalesReader.Right.Catalog_Products\.Command\.Print"
    );
    assert_eq!(
        member(
            "main:Report.ParityReport.Template.Print.Area",
            "Header.Total"
        ),
        r"main:Report.ParityReport.Template.Print.Area.Header\.Total"
    );
    assert_eq!(
        member(
            "main:Subsystem.Sales.Interface.Group",
            "CommandGroup.Pricing"
        ),
        r"main:Subsystem.Sales.Interface.Group.CommandGroup\.Pricing"
    );
    let group = service.view(
        ViewRequest::new(r"main:Subsystem.Sales.Interface.Group.CommandGroup\.Pricing").unwrap(),
    );
    assert_eq!(
        group.data.as_ref().unwrap()["items"],
        json!([{"order": 1, "command": "Catalog.Items.StandardCommand.OpenList"}])
    );
}

/// Свойство XDTO любой вложенности читается по цепочке `Property`: узел
/// свойства со вложенным анонимным типом объявляет ветвь его свойств, и
/// каждый опубликованный адрес читается.
#[test]
fn xdto_properties_are_read_at_any_depth() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    let owner = "main:XDTOPackage.EnterpriseData_1_17_3.Type.ЛюбаяСсылка.Property.СсылкаНаОбъект";
    let node = service.view(ViewRequest::new(owner).unwrap());
    assert!(node.ok, "{:?}", refusal_codes(&node));
    assert_eq!(
        node.data.as_ref().unwrap()["branches"],
        json!([{"at": format!("{owner}.Property"), "count": 1}])
    );
    let nested = service.view(ViewRequest::new(&format!("{owner}.Property")).unwrap());
    assert!(nested.ok, "{:?}", refusal_codes(&nested));
    let leaf = format!("{owner}.Property.Документ_ЗаказКлиента");
    assert_eq!(
        nested.data.as_ref().unwrap()["items"][0]["at"],
        leaf.as_str()
    );
    let read = service.view(ViewRequest::new(&leaf).unwrap());
    assert!(read.ok, "{:?}", refusal_codes(&read));
    assert_eq!(read.data.as_ref().unwrap()["kind"], "Property");
    assert!(read
        .data
        .as_ref()
        .unwrap()
        .get("branches")
        .is_none_or(|value| value.as_array().is_some_and(Vec::is_empty)));
    // У свойства без вложенного типа детей нет.
    let missing = service.view(ViewRequest::new(&format!("{leaf}.Property.Нет")).unwrap());
    assert_eq!(refusal_codes(&missing), vec!["not_found"]);
}

#[test]
fn mxl_area_parameter_consumes_the_complete_suffix() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    let at = "main:Report.ParityReport.Template.Print.Area.Header.Parameter.Title";

    let result = service.view(ViewRequest::new(at).unwrap());

    assert!(result.ok, "{} {:?}", result.summary, result.diagnostics);
    let data = result.data.as_ref().unwrap();
    assert_eq!(data["at"], at);
    assert_eq!(data["kind"], "Parameter");
    assert_eq!(data["title"], "Title");
}

/// Коды отказов из ответа. Сообщение провала несёт только их: разбор в панике
/// затягивает в текст всю нагрузку вместе с идентификаторами фикстуры, и
/// сканер справедливо не отличает такой текст от записи в журнал.
fn refusal_codes(result: &crate::domain::invocation::DomainResult) -> Vec<&str> {
    result
        .diagnostics
        .iter()
        .filter_map(|diagnostic| diagnostic.get("code").and_then(serde_json::Value::as_str))
        .collect()
}

/// Видимость команды в интерфейсе — это не одно значение. Замер на боевой
/// конфигурации: 99 блоков видимости из 1050 несут переопределения по ролям,
/// то есть каждая одиннадцатая команда. Отдать `visible` за всю правду
/// значило бы читать девять процентов команд неверно.
#[test]
fn command_visibility_says_when_roles_override_it() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Ext/CommandInterface.xml"),
        r#"<?xml version="1.0" encoding="utf-8"?>
<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20">
	<CommandsVisibility>
		<Command name="Catalog.Items.Command.Refresh">
			<Visibility>
				<xr:Common>true</xr:Common>
				<xr:Value name="Role.Кладовщик">false</xr:Value>
				<xr:Value name="Role.ПолныеПрава">true</xr:Value>
			</Visibility>
		</Command>
		<Command name="CommonCommand.ОткрытьНастройки">
			<Visibility>
				<xr:Common>false</xr:Common>
			</Visibility>
		</Command>
	</CommandsVisibility>
</CommandInterface>
"#,
    );
    let service = fixture.view_service();

    let commands =
        service.view(ViewRequest::new("main:Subsystem.Sales.Interface.Command").unwrap());

    assert!(commands.ok, "{:?}", refusal_codes(&commands));
    let items = commands.data.as_ref().unwrap()["items"].as_array().unwrap();
    let props = |title: &str| {
        items
            .iter()
            .find(|item| item["title"] == title)
            .unwrap_or_else(|| panic!("{title} among {items:?}"))["props"]
            .clone()
    };
    let overridden = props("Catalog.Items.Command.Refresh");
    assert_eq!(overridden["visible"], true);
    assert_eq!(overridden["roleOverrides"], 2);
    // Ноль говорит «переопределений нет», а не «не смотрели»: отсутствие поля
    // читатель не отличил бы от несчитанного.
    let plain = props("CommonCommand.ОткрытьНастройки");
    assert_eq!(plain["visible"], false);
    assert_eq!(plain["roleOverrides"], 0);
}

/// Порядок команд, групп и дочерних подсистем — три из пяти секций
/// командного интерфейса. Две из них наружу не выходили вовсе, а писать то,
/// чего инструмент не показывает, нельзя: предпросмотр правки нечем
/// подтвердить, а забор ревизии по прочитанному над невидимым фактом
/// несостоятелен.
#[test]
fn the_command_interface_shows_every_order_it_holds() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Ext/CommandInterface.xml"),
        r#"<?xml version="1.0" encoding="utf-8"?>
<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20">
	<CommandsOrder>
		<Command name="Catalog.Items.StandardCommand.Zeta">
			<CommandGroup>NavigationPanelImportant</CommandGroup>
		</Command>
		<Command name="Catalog.Items.StandardCommand.OpenList">
			<CommandGroup>NavigationPanelImportant</CommandGroup>
		</Command>
	</CommandsOrder>
	<SubsystemsOrder>
		<Subsystem>Subsystem.Zeta</Subsystem>
		<Subsystem>Subsystem.Sales</Subsystem>
	</SubsystemsOrder>
	<GroupsOrder>
		<Group>NavigationPanelSeeAlso</Group>
		<Group>NavigationPanelImportant</Group>
	</GroupsOrder>
</CommandInterface>
"#,
    );
    let service = fixture.view_service();
    let at = "main:Subsystem.Sales.Interface";

    let node = service.view(ViewRequest::new(at).unwrap());
    assert!(node.ok, "{:?}", refusal_codes(&node));
    let branches = node.data.as_ref().unwrap()["branches"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let branch = |kind: &str| {
        branches
            .iter()
            .find(|branch| branch["at"] == format!("{at}.{kind}"))
            .cloned()
    };
    assert_eq!(branch("Group").unwrap()["count"], 2, "{branches:?}");
    assert_eq!(branch("Subsystem").unwrap()["count"], 2, "{branches:?}");

    // Группы идут в объявленном порядке, и пустая группа остаётся видимой:
    // порядок объявляется отдельно от наполнения.
    let groups = service.view(ViewRequest::new(&format!("{at}.Group")).unwrap());
    assert!(groups.ok, "{:?}", refusal_codes(&groups));
    let items = groups.data.as_ref().unwrap()["items"].as_array().unwrap();
    assert_eq!(items[0]["title"], "NavigationPanelSeeAlso");
    assert_eq!(items[0]["commands"], 0);
    assert_eq!(items[1]["title"], "NavigationPanelImportant");
    assert_eq!(items[1]["commands"], 2);

    // Внутри группы — её команды в порядке `CommandsOrder`.
    let one =
        service.view(ViewRequest::new(&format!("{at}.Group.NavigationPanelImportant")).unwrap());
    assert!(one.ok, "{:?}", refusal_codes(&one));
    let ordered = one.data.as_ref().unwrap()["items"].as_array().unwrap();
    assert_eq!(
        ordered,
        &vec![
            json!({"order": 1, "command": "Catalog.Items.StandardCommand.Zeta"}),
            json!({"order": 2, "command": "Catalog.Items.StandardCommand.OpenList"}),
        ]
    );

    // Ссылка платформы дополняется до адреса, по которому можно спуститься.
    let children = service.view(ViewRequest::new(&format!("{at}.Subsystem")).unwrap());
    assert!(children.ok, "{:?}", refusal_codes(&children));
    assert_eq!(
        children.data.as_ref().unwrap()["items"],
        json!([
            {"order": 1, "at": "main:Subsystem.Zeta"},
            {"order": 2, "at": "main:Subsystem.Sales"},
        ])
    );

    // Группы, которой в порядке нет, не существует и для чтения.
    let missing = service.view(ViewRequest::new(&format!("{at}.Group.Нет")).unwrap());
    assert!(!missing.ok);
    assert_eq!(missing.diagnostics[0]["code"], "not_found");
}

fn subsystem_descriptor(name: &str, content: &[&str], children: &[&str]) -> String {
    let content = content
        .iter()
        .map(|item| format!(r#"<xr:Item xsi:type="xr:MDObjectRef">{item}</xr:Item>"#))
        .collect::<String>();
    let children = children
        .iter()
        .map(|child| format!("<Subsystem>{child}</Subsystem>"))
        .collect::<String>();
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
  <Subsystem><Properties><Name>{name}</Name><Synonym/><IncludeInCommandInterface>true</IncludeInCommandInterface><UseOneCommand>false</UseOneCommand><Content>{content}</Content></Properties><ChildObjects>{children}</ChildObjects></Subsystem>
</MetaDataObject>"#
    )
}

fn install_supported_subsystem_descriptors(fixture: &RealReaderFixture) {
    for (relative, name, children, uuid) in [
        (
            "Subsystems/Sales.xml",
            "Sales",
            &["Orders"][..],
            "11111111-1111-4111-8111-111111111111",
        ),
        (
            "Subsystems/Sales/Subsystems/Orders.xml",
            "Orders",
            &[][..],
            "22222222-2222-4222-8222-222222222222",
        ),
    ] {
        write(
            &fixture.source.join(relative),
            &subsystem_descriptor(name, &[], children).replacen(
                "<Subsystem>",
                &format!("<Subsystem uuid=\"{uuid}\">"),
                1,
            ),
        );
    }
}

#[test]
fn canonical_subsystem_view_preserves_support_for_own_and_nested_descriptors() {
    let parent_uuid = "11111111-1111-4111-8111-111111111111";
    let child_uuid = "22222222-2222-4222-8222-222222222222";
    let support =
        |state, direct_edit_safe| json!({"support": state, "directEditSafe": direct_edit_safe});
    for (case, marker, parent, child) in [
        (
            "different object rules",
            Some(format!("{{6,0,1,0,0,{parent_uuid},1,0,{child_uuid}}}")),
            support("locked", Some(false)),
            support("editableWithSupport", Some(true)),
        ),
        (
            "removed object",
            Some(format!("{{6,0,1,2,0,{parent_uuid},0,0,{child_uuid}}}")),
            support("removedFromSupport", Some(true)),
            support("locked", Some(false)),
        ),
        (
            "configuration read only",
            Some(format!("{{6,1,1,0,0,{parent_uuid},1,0,{child_uuid}}}")),
            support("configurationReadOnly", Some(false)),
            support("configurationReadOnly", Some(false)),
        ),
        (
            "configuration removed from support",
            Some(String::new()),
            support("removedFromSupport", Some(true)),
            support("removedFromSupport", Some(true)),
        ),
        (
            "marker absent",
            None,
            support("notSupported", None),
            support("notSupported", None),
        ),
        (
            "UUID absent from marker",
            Some("{6,0,1,0,0,33333333-3333-4333-8333-333333333333}".to_string()),
            support("notSupported", None),
            support("notSupported", None),
        ),
    ] {
        let fixture = RealReaderFixture::new();
        install_supported_subsystem_descriptors(&fixture);
        if let Some(marker) = marker {
            write(
                &fixture.source.join("Ext/ParentConfigurations.bin"),
                &marker,
            );
        }
        let service = fixture.view_service();
        for (at, expected) in [
            ("main:Subsystem.Sales", &parent),
            ("main:Subsystem.Sales.Subsystem.Orders", &child),
            ("main:Subsystem.Sales", &parent),
        ] {
            let result = service.view(ViewRequest::new(at).unwrap());
            assert!(result.ok, "{case} {at}: view must succeed");
            let props = &result.data.as_ref().unwrap()["props"];
            assert_eq!(props["support"], expected["support"], "{case} {at}");
            assert_eq!(
                props["directEditSafe"], expected["directEditSafe"],
                "{case} {at}"
            );
            assert!(!serde_json::to_string(&result)
                .unwrap()
                .contains(fixture.source.to_str().unwrap()));
        }
    }
}

#[test]
fn canonical_view_distinguishes_absent_support_from_unreadable_and_invalid_markers() {
    for marker_kind in ["absent", "invalid", "directory"] {
        let fixture = RealReaderFixture::new();
        install_supported_subsystem_descriptors(&fixture);
        let marker = fixture.source.join("Ext/ParentConfigurations.bin");
        match marker_kind {
            "invalid" => write(&marker, "{9,0,1}, invalid support header"),
            "directory" => fs::create_dir_all(&marker).unwrap(),
            _ => {}
        }
        let service = fixture.view_service();
        for at in [
            "main:Catalog.Items",
            "main:Subsystem.Sales",
            "main:Subsystem.Sales.Subsystem.Orders",
        ] {
            let result = service.view(ViewRequest::new(at).unwrap());
            if marker_kind == "absent" {
                assert!(
                    result.ok,
                    "{at}: view must succeed without a support marker"
                );
                let expected = if at == "main:Catalog.Items" {
                    json!("supported")
                } else {
                    json!("notSupported")
                };
                assert_eq!(
                    result.data.as_ref().unwrap()["props"]["support"],
                    expected,
                    "{at}"
                );
                let props = &result.data.as_ref().unwrap()["props"];
                if at == "main:Catalog.Items" {
                    assert!(props.get("directEditSafe").is_none());
                } else {
                    assert_eq!(props["directEditSafe"], serde_json::Value::Null);
                    assert!(props.as_object().unwrap().contains_key("directEditSafe"));
                }
            } else {
                assert!(!result.ok, "{marker_kind} {at}: view must refuse");
                assert!(
                    result.data.is_none(),
                    "{marker_kind} {at}: data must be absent"
                );
                assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
                assert_eq!(result.diagnostics[0]["detailCode"], "source_unreadable");
                assert_eq!(result.diagnostics[0]["outcome"], "fixSource");
            }
            assert!(!serde_json::to_string(&result)
                .unwrap()
                .contains(fixture.source.to_str().unwrap()));
        }
    }
}

fn view_items(result: &crate::domain::invocation::DomainResult) -> Vec<serde_json::Value> {
    assert!(result.ok, "{:?}", refusal_codes(result));
    result.data.as_ref().unwrap()["items"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn view_branch_count(result: &crate::domain::invocation::DomainResult, at: &str) -> Option<u64> {
    assert!(result.ok, "{:?}", refusal_codes(result));
    result.data.as_ref().unwrap()["branches"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|branch| branch["at"] == at)
        .and_then(|branch| branch["count"].as_u64())
}

/// Состав подсистемы и её дочерние подсистемы — то, ради чего подсистему
/// читают. Состав — ссылки на чужие объекты, поэтому он живёт в ветви
/// `Relation`; дочерняя подсистема — потомок с собственным адресом, и
/// читается по нему на любой глубине регистрации.
#[test]
fn a_subsystem_shows_its_content_and_reads_registered_children_at_depth() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture.source.join("Subsystems/Sales.xml"),
        &subsystem_descriptor("Sales", &["Catalog.Items", "Document.Order"], &["Orders"]),
    );
    // Ссылка по идентификатору — так платформа пишет указатель на
    // исчезнувший объект. Она не роняет чтение и не прячется: её называет
    // слот `limits`, а счёт ветви равен тому, что покажет страница.
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Subsystems/Orders.xml"),
        &subsystem_descriptor(
            "Orders",
            &["Document.Order", "0:78b4152e-0000-0000-0000-000000000000"],
            &["Returns"],
        ),
    );
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Subsystems/Orders/Subsystems/Returns.xml"),
        &subsystem_descriptor("Returns", &["Catalog.Items"], &[]),
    );
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Subsystems/Orders/Ext/CommandInterface.xml"),
        r#"<?xml version="1.0" encoding="utf-8"?>
<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20">
	<SubsystemsOrder>
		<Subsystem>Subsystem.Sales.Subsystem.Orders.Subsystem.Returns</Subsystem>
	</SubsystemsOrder>
</CommandInterface>
"#,
    );
    // Файл на диске, который родитель не регистрирует, подсистемой не является.
    write(
        &fixture.source.join("Subsystems/Sales/Subsystems/Ghost.xml"),
        &subsystem_descriptor("Ghost", &[], &[]),
    );
    let service = fixture.view_service();

    let root = service.view(ViewRequest::new("main:Subsystem.Sales").unwrap());
    assert_eq!(
        view_branch_count(&root, "main:Subsystem.Sales.Relation"),
        Some(2)
    );
    assert_eq!(
        view_branch_count(&root, "main:Subsystem.Sales.Subsystem"),
        Some(1)
    );
    assert_eq!(
        view_items(&service.view(ViewRequest::new("main:Subsystem.Sales.Relation").unwrap())),
        vec![
            json!({"relation": "content", "at": "main:Catalog.Items", "kind": "Catalog"}),
            json!({"relation": "content", "at": "main:Document.Order", "kind": "Document"}),
        ]
    );
    let children =
        view_items(&service.view(ViewRequest::new("main:Subsystem.Sales.Subsystem").unwrap()));
    assert_eq!(children.len(), 1);
    assert_eq!(children[0]["at"], "main:Subsystem.Sales.Subsystem.Orders");
    assert_eq!(children[0]["kind"], "Subsystem");

    let nested_at = "main:Subsystem.Sales.Subsystem.Orders";
    let nested = service.view(ViewRequest::new(nested_at).unwrap());
    assert!(nested.ok, "{:?}", refusal_codes(&nested));
    let data = nested.data.as_ref().unwrap();
    assert_eq!(data["at"], nested_at);
    assert_eq!(data["kind"], "Subsystem");
    assert_eq!(data["title"], "Orders");
    assert_eq!(
        view_branch_count(&nested, &format!("{nested_at}.Relation")),
        Some(1)
    );
    assert_eq!(
        view_branch_count(&nested, &format!("{nested_at}.Subsystem")),
        Some(1)
    );
    assert_eq!(
        view_branch_count(&nested, &format!("{nested_at}.Interface")),
        Some(1)
    );
    let content = service.view(ViewRequest::new(&format!("{nested_at}.Relation")).unwrap());
    assert_eq!(
        view_items(&content),
        vec![json!({"relation": "content", "at": "main:Document.Order", "kind": "Document"})]
    );
    let limits = content.data.as_ref().unwrap()["limits"].to_string();
    assert!(
        limits.contains("0:78b4152e-0000-0000-0000-000000000000"),
        "content limits must name the dangling reference"
    );

    // Интерфейс вложенной подсистемы читается из её собственного файла, а
    // не из файла корня.
    let order = view_items(
        &service.view(ViewRequest::new(&format!("{nested_at}.Interface.Subsystem")).unwrap()),
    );
    assert_eq!(
        order,
        vec![json!({"order": 1, "at": "main:Subsystem.Sales.Subsystem.Orders.Subsystem.Returns"})]
    );
    let deepest = service.view(ViewRequest::new(order[0]["at"].as_str().unwrap()).unwrap());
    assert!(deepest.ok, "{:?}", refusal_codes(&deepest));
    assert_eq!(deepest.data.as_ref().unwrap()["title"], "Returns");

    // Родитель, прочитанный после потомка, не получает его нагрузку из кэша.
    let again = service.view(ViewRequest::new("main:Subsystem.Sales").unwrap());
    assert_eq!(
        again.data.as_ref().unwrap()["title"],
        root.data.as_ref().unwrap()["title"]
    );
    assert_eq!(
        view_branch_count(&again, "main:Subsystem.Sales.Relation"),
        Some(2)
    );

    // Страница состава режется общим курсором.
    let first = service.view(
        ViewRequest::new("main:Subsystem.Sales.Relation")
            .unwrap()
            .with_limit(1)
            .unwrap(),
    );
    assert_eq!(view_items(&first).len(), 1);
    assert!(
        first.cursor.is_some(),
        "a one-item page must leave a cursor"
    );

    for missing in [
        "main:Subsystem.Sales.Subsystem.Ghost",
        "main:Subsystem.Sales.Subsystem.Нет",
        "main:Subsystem.Sales.Relation.Catalog",
    ] {
        let result = service.view(ViewRequest::new(missing).unwrap());
        assert!(!result.ok, "{missing} must be refused");
        assert_eq!(result.diagnostics[0]["code"], "not_found", "{missing}");
    }
}

/// Спуск идёт по регистрации, и каждый дескриптор цепочки должен называть
/// ту подсистему, по имени которой к нему пришли. Файл с чужим именем на её
/// месте не становится ни ею, ни дорогой к её детям.
#[test]
fn a_nested_subsystem_read_rejects_a_descriptor_that_names_another_subsystem() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture.source.join("Subsystems/Sales.xml"),
        &subsystem_descriptor("Sales", &[], &["Orders"]),
    );
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Subsystems/Orders.xml"),
        &subsystem_descriptor("Impostor", &[], &["Returns"]),
    );
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Subsystems/Orders/Subsystems/Returns.xml"),
        &subsystem_descriptor("Returns", &[], &[]),
    );
    let service = fixture.view_service();

    for at in [
        "main:Subsystem.Sales.Subsystem.Orders",
        "main:Subsystem.Sales.Subsystem.Orders.Subsystem.Returns",
    ] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(!result.ok, "{at} must be refused");
        assert_ne!(result.diagnostics[0]["code"], "not_found", "{at}");
    }
}

#[test]
fn template_area_cell_content_is_a_branch_read_only_when_its_address_is_asked() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    let area = "main:Report.ParityReport.Template.Print.Area.Header";

    let node = service.view(ViewRequest::new(area).unwrap());
    assert!(node.ok, "{:?}", refusal_codes(&node));
    let data = node.data.as_ref().unwrap();
    // Структурное чтение области знает длину содержимого, но не несёт текста.
    assert_eq!(data["props"]["contentCount"], 2);
    assert_eq!(data.get("content"), None);
    let branches = data["branches"].as_array().unwrap();
    assert!(
        branches
            .iter()
            .any(|branch| branch["at"] == format!("{area}.Body") && branch["count"] == 2),
        "{branches:?}"
    );

    let body = service.view(ViewRequest::new(&format!("{area}.Body")).unwrap());
    assert!(body.ok, "{:?}", refusal_codes(&body));
    let items = body.data.as_ref().unwrap()["items"].as_array().unwrap();
    assert_eq!(
        items,
        &vec![
            json!({"index": 1, "template": false, "text": "Sales report"}),
            json!({"index": 2, "template": true, "text": "Period: [Since]"}),
        ],
        "порядок ячеек — это и есть смысл печатной формы"
    );
}

#[test]
fn a_structural_template_read_never_serves_a_cached_payload_to_a_content_read() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    let area = "main:Report.ParityReport.Template.Print.Area.Header";

    // Один вызов сначала берёт структуру, потом текст: если признак
    // содержимого не входит в ключ кэша, второе чтение получит первый разбор.
    let structural = service.view(ViewRequest::new(area).unwrap());
    assert!(structural.ok, "{:?}", refusal_codes(&structural));
    let body = service.view(ViewRequest::new(&format!("{area}.Body")).unwrap());

    assert!(body.ok, "{:?}", refusal_codes(&body));
    assert_eq!(
        body.data.as_ref().unwrap()["items"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
}

/// `Body` отдаёт исходные строки с их номерами: среза по контексту у `view`
/// нет (#1374), и клиентский код формы остаётся в ответе.
#[test]
fn module_body_returns_the_source_text_byte_for_byte() {
    let fixture = RealReaderFixture::new();
    let module = fixture
        .source
        .join("Reports/ParityReport/Forms/MainForm/Ext/Form/Module.bsl");
    let at = "main:Report.ParityReport.Form.MainForm.Module.Form.Body";
    let original = fs::read_to_string(&module).unwrap();
    // Начальные U+FEFF снимаются, CRLF, табуляция, кавычки и отсутствие
    // разделителя в конце файла сохраняются как есть.
    let crlf = "\u{feff}// Комментарий \"формы\".\r\nПроцедура А()\r\n\tВозврат;\r\nКонецПроцедуры";
    for source in [original.as_str(), crlf] {
        fs::write(&module, source).unwrap();
        let result = fixture.view_service().view(ViewRequest::new(at).unwrap());
        assert!(result.ok, "module Body must be read");
        assert_eq!(
            result.source_text.as_deref(),
            Some(source.trim_start_matches('\u{feff}'))
        );
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["at"], at);
        assert_eq!(data["kind"], "Body");
        assert!(data.get("items").is_none());
        assert!(result.cursor.is_none());
    }
}

#[test]
fn view_never_returns_module_text_above_the_largest_host_budget() {
    let fixture = RealReaderFixture::new();
    let module = fixture
        .source
        .join("Reports/ParityReport/Forms/MainForm/Ext/Form/Module.bsl");
    let at = "main:Report.ParityReport.Form.MainForm.Module.Form.Body";
    // 72 КиБ — наибольший бюджет известного хоста; число записано здесь
    // явно, чтобы смена предела в коде была видна этой проверке.
    let mut source = vec![b'a'; 73_728];
    fs::write(&module, &source).unwrap();
    let whole = fixture.view_service().view(ViewRequest::new(at).unwrap());
    assert_eq!(whole.source_text.as_ref().map(String::len), Some(73_728));
    source.push(b'a');
    fs::write(&module, &source).unwrap();
    let refused = fixture.view_service().view(ViewRequest::new(at).unwrap());
    assert_eq!(refused.diagnostics[0]["code"], "result_too_large");
    assert!(refused.source_text.is_none());
}

#[test]
fn module_body_beyond_the_limit_is_still_checked_as_utf8_in_full() {
    let fixture = RealReaderFixture::new();
    let module = fixture
        .source
        .join("Reports/ParityReport/Forms/MainForm/Ext/Form/Module.bsl");
    let at = "main:Report.ParityReport.Form.MainForm.Module.Form.Body";
    let mut source = vec![b'a'; crate::application::v13::source_text::MAX_SOURCE_TEXT_BYTES + 1];
    let valid = fixture.view_service();
    fs::write(&module, &source).unwrap();
    let oversized = valid.view(ViewRequest::new(at).unwrap());
    assert_eq!(oversized.diagnostics[0]["code"], "result_too_large");
    // Неверный байт за пределом не теряется: отказ называет испорченный
    // исходник, а не размер.
    source.push(0xff);
    fs::write(&module, &source).unwrap();
    let broken = fixture.view_service().view(ViewRequest::new(at).unwrap());
    assert!(!broken.ok);
    assert_ne!(
        broken.diagnostics[0]["code"], "result_too_large",
        "a broken byte beyond the limit must not be reported as size"
    );
    assert!(broken.source_text.is_none());
}

#[test]
fn ordinary_xml_read_over_eight_mib_keeps_complete_configuration_facts() {
    let fixture = RealReaderFixture::new();
    let path = fixture.source.join("Configuration.xml");
    let baseline = fixture
        .view_service()
        .view(ViewRequest::new("main:Configuration").unwrap());
    assert!(baseline.ok, "baseline Configuration view failed");
    let xml = fs::read_to_string(&path).unwrap();
    let inflated = xml.replace(
        "</MetaDataObject>",
        &format!("<!--{}--></MetaDataObject>", "x".repeat(8 * 1024 * 1024)),
    );
    fs::write(path, inflated).unwrap();

    let result = fixture
        .view_service()
        .view(ViewRequest::new("main:Configuration").unwrap());
    assert!(result.ok, "large Configuration view failed");
    assert!(
        result.data.as_ref().unwrap()["props"] == baseline.data.as_ref().unwrap()["props"],
        "Configuration root properties changed"
    );
    assert!(
        result.data.as_ref().unwrap()["branches"] == baseline.data.as_ref().unwrap()["branches"],
        "Configuration root branches changed"
    );
}

#[test]
fn large_configuration_root_keeps_legacy_duplicate_and_namespace_semantics() {
    let fixture = RealReaderFixture::new();
    let path = fixture.source.join("Configuration.xml");
    let xml = fs::read_to_string(&path).unwrap();
    let xml = xml.replacen(
        "</ChildObjects>",
        "<CommonModule>РеактивныйСервер</CommonModule><alien:Catalog xmlns:alien=\"urn:other\">Ghost</alien:Catalog></ChildObjects>",
        1,
    );
    fs::write(&path, &xml).unwrap();

    let reader = fixture.read_authority();
    let legacy_payload = reader
        .read
        .configuration_payload_with_checkpoint(&mut || Ok(()))
        .unwrap();
    let route = route_logical_address(
        &QualifiedAddress::parse("main:Configuration").unwrap(),
        PlatformProfile::v8_3_27(),
    )
    .unwrap();
    let legacy = project_typed_payload(&route, legacy_payload.as_ref().clone()).unwrap();
    let module_branch = super::module_branch_for_parent(route.at(), PlatformProfile::v8_3_27())
        .expect("Configuration has a profile module branch");
    let expected = serde_json::to_value(legacy.with_branch(module_branch)).unwrap();

    fs::write(
        &path,
        xml.replace(
            "</MetaDataObject>",
            &format!("<!--{}--></MetaDataObject>", "x".repeat(8 * 1024 * 1024)),
        ),
    )
    .unwrap();
    let actual = fixture
        .view_service()
        .view(ViewRequest::new("main:Configuration").unwrap());
    assert!(actual.ok, "large Configuration view failed");
    assert!(
        actual.data.as_ref().unwrap() == &expected,
        "streaming Configuration root differs from the bounded reader"
    );
}

#[test]
fn large_configuration_root_rejects_malformed_tail() {
    let fixture = RealReaderFixture::new();
    let path = fixture.source.join("Configuration.xml");
    let xml = fs::read_to_string(&path).unwrap();
    fs::write(
        path,
        xml.replace(
            "</MetaDataObject>",
            &format!(
                "<!--{}--><Broken></MetaDataObject>",
                "x".repeat(8 * 1024 * 1024)
            ),
        ),
    )
    .unwrap();
    let result = fixture
        .view_service()
        .view(ViewRequest::new("main:Configuration").unwrap());
    assert!(!result.ok, "malformed XML tail escaped");
    assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
    assert_eq!(result.diagnostics[0]["detailCode"], "source_unreadable");
}

#[test]
fn every_reader_rejects_an_extra_unconsumed_address_tail() {
    let cases = [
        (
            "main:XDTOPackage.EnterpriseData_1_17_3.Type.Order.Property.Id.Event.Change",
            json!({
                "targetNamespace": "urn:test",
                "types": [{"name": "Order", "properties": [{"name": "Id"}]}],
                "globalProperties": [],
                "counts": {},
                "imports": [],
                "findings": []
            }),
        ),
        (
            "main:Subsystem.Sales.Interface.Command.Open.Event.Change",
            json!({
                "name": "Sales",
                "content": [],
                "children": [],
                "commandInterface": {
                    "visibility": [{"command": "Open", "visible": true}],
                    "placement": []
                }
            }),
        ),
    ];
    for (at, payload) in cases {
        let address = QualifiedAddress::parse(at).unwrap();
        let route = route_logical_address(&address, PlatformProfile::v8_3_27()).unwrap();
        let error = project_typed_payload(&route, payload).unwrap_err();
        assert_eq!(error.code().as_str(), "not_found", "{at}");
    }

    let fixture = RealReaderFixture::new();
    let module = fixture.view_service().view(
        ViewRequest::new(
            "main:Report.ParityReport.Form.MainForm.Module.Form.Method.OnOpen.Body.source.Event.Change",
        )
        .unwrap(),
    );
    assert!(!module.ok, "{:?}", module.data);
    assert_eq!(module.diagnostics[0]["code"], "not_found");
}

#[test]
fn form_projection_uses_a_positive_nested_scalar_allowlist() {
    let at = QualifiedAddress::parse("main:CommonForm.Test.Item.Secret").unwrap();
    let route = route_logical_address(&at, PlatformProfile::v8_3_27()).unwrap();
    let view = project_typed_payload(
        &route,
        json!({
            "name": "Test",
            "elements": [{
                "name": "Secret",
                "tag": "[Input]",
                "visible": true,
                "enabled": true,
                "readOnly": false,
                "mysteryProviderScalar": "must-not-leak",
                "events": [],
                "children": []
            }],
            "attributes": [],
            "parameters": [],
            "commands": [],
            "events": []
        }),
    )
    .unwrap();

    let serialized = serde_json::to_string(&view).unwrap();
    assert!(
        !serialized.contains("mysteryProviderScalar"),
        "{serialized}"
    );
    assert!(!serialized.contains("must-not-leak"), "{serialized}");
}

#[test]
fn role_right_projection_never_serializes_an_unbounded_rights_array_into_props() {
    let at = QualifiedAddress::parse("main:Role.Test.Right.Products").unwrap();
    let route = route_logical_address(&at, PlatformProfile::v8_3_27()).unwrap();
    let rights = (0..500)
        .map(|index| json!({"name": format!("Right{index}"), "restricted": false}))
        .collect::<Vec<_>>();
    let view = project_typed_payload(
        &route,
        json!({
            "name": "Test",
            "allowed": [{"kind": "Catalog", "objects": [{"name": "Products", "rights": rights}]}],
            "denied": [],
            "restrictedObjects": [],
            "templates": [],
            "totals": {"allowed": 500, "denied": 0}
        }),
    )
    .unwrap();
    let serialized = serde_json::to_value(view).unwrap();

    assert!(serialized["props"].get("rights").is_none(), "{serialized}");
    assert!(serialized["props"]
        .as_object()
        .unwrap()
        .values()
        .all(|value| value.as_str().is_none_or(|text| text.len() <= 2_048)));
}

#[test]
fn xdto_property_view_preserves_type_identity_and_explicit_and_effective_bounds() {
    let fixture = RealReaderFixture::new();
    let package = fixture
        .source
        .join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin");
    let original = fs::read_to_string(&package).unwrap();
    let xml = original.replace(
        "</package>",
        r#"<objectType name="ClusterAdministrationInfo">
  <property name="ClusterPort" type="xs:decimal"/>
  <property name="WorkServerUserName" type="xs:string" lowerBound="0" nillable="true"/>
  <property name="Ports" type="xs:decimal" lowerBound="1" upperBound="-1"/>
</objectType></package>"#,
    );
    fs::write(&package, &xml).unwrap();
    let service = fixture.view_service();
    for (name, local, lower, upper, lower_explicit, upper_explicit, unbounded) in [
        ("ClusterPort", "decimal", "1", "1", false, false, false),
        ("WorkServerUserName", "string", "0", "1", true, false, false),
        ("Ports", "decimal", "1", "-1", true, true, true),
    ] {
        let at = format!(
            "main:XDTOPackage.EnterpriseData_1_17_3.Type.ClusterAdministrationInfo.Property.{name}"
        );
        let result = service.view(ViewRequest::new(&at).unwrap());
        assert!(result.ok, "{} {:?}", result.summary, result.diagnostics);
        let data = result.data.unwrap();
        let props = &data["props"];
        let type_info: serde_json::Value = serde_json::from_str(
            props["type"]
                .as_str()
                .unwrap_or_else(|| panic!("missing type: {data}")),
        )
        .unwrap();
        assert_eq!(type_info["namespace"], "http://www.w3.org/2001/XMLSchema");
        assert_eq!(type_info["local"], local);
        assert_eq!(type_info["raw"], format!("xs:{local}"));
        let bounds: serde_json::Value = serde_json::from_str(
            props["bounds"]
                .as_str()
                .unwrap_or_else(|| panic!("missing bounds: {data}")),
        )
        .unwrap();
        assert_eq!(
            bounds,
            json!({
                "lower": lower, "upper": upper,
                "lowerExplicit": lower_explicit, "upperExplicit": upper_explicit,
                "unbounded": unbounded
            })
        );
        assert!(props.get("location").is_none());
    }
    assert_eq!(fs::read_to_string(package).unwrap(), xml);
}

#[test]
fn xdto_property_view_names_facts_that_exceed_the_compact_limit() {
    let fixture = RealReaderFixture::new();
    let package = fixture
        .source
        .join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin");
    let original = fs::read_to_string(&package).unwrap();
    let xml = original.replace(
        "upperBound=\"3\"",
        &format!("upperBound=\"{}\"", "9".repeat(2100)),
    );
    fs::write(&package, &xml).unwrap();
    let result = fixture.view_service().view(ViewRequest::new(
        "main:XDTOPackage.EnterpriseData_1_17_3.Type.Документ_ЗаказКлиента.Property.Идентификаторы"
    ).unwrap());
    assert!(result.ok, "{} {:?}", result.summary, result.diagnostics);
    let data = result.data.unwrap();
    assert!(data["props"]["type"].as_str().is_some(), "{data}");
    assert!(data["props"].get("bounds").is_none(), "{data}");
    assert!(
        data["limits"].as_array().unwrap().iter().any(|limit| limit
            .as_str()
            .is_some_and(|text| text.contains("bounds") && text.contains("2048"))),
        "{data}"
    );
    assert_eq!(fs::read_to_string(package).unwrap(), xml);
}

#[test]
fn real_typed_readers_cover_every_task14_profile_without_skipping() {
    let fixture = RealReaderFixture::new();
    let cancellation = CancellationToken::new();
    let source_root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let metadata_target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Items").unwrap();
    let metadata_resolution = resolve_platform_xml_target(
        &fixture.context,
        &SourceTarget {
            source_set: "main".to_string(),
            metadata_path: Some(metadata_target),
        },
        TargetKindPolicy::Any,
    );
    assert!(metadata_resolution.is_ok(), "{metadata_resolution:?}");
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
        source_root,
        PlatformProfile::v8_3_27(),
    );
    let service = ViewService::new(authority, ViewCursorStore::default());
    let cases = [
        ("configuration", "main:Configuration", "Configuration"),
        ("metadata", "main:Catalog.Items", "Catalog"),
        ("form", "main:Report.ParityReport.Form.MainForm", "Form"),
        (
            "role-rls",
            "main:Role.SalesReader.Right.Catalog_Products.RLS",
            "RLS",
        ),
        ("subsystem", "main:Subsystem.Sales", "Subsystem"),
        ("interface", "main:Subsystem.Sales.Interface", "Interface"),
        (
            "dcs",
            "main:Report.ParityReport.Template.MainSchema.DataSet",
            "DataSet",
        ),
        (
            "mxl",
            "main:Report.ParityReport.Template.Print.Area",
            "Area",
        ),
        (
            "xdto",
            "main:XDTOPackage.EnterpriseData_1_17_3.Type",
            "Type",
        ),
        (
            "module",
            "main:CommonModule.РеактивныйСервер.Method",
            "Method",
        ),
        (
            "form-module-binding",
            "main:Report.ParityReport.Form.MainForm.Module.Form.Method.OnOpen",
            "Method",
        ),
    ];
    for (case, at, kind) in cases {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(
            result.ok,
            "{case}: {} {:?}",
            result.summary, result.diagnostics
        );
        let data = result
            .data
            .as_ref()
            .unwrap_or_else(|| panic!("{case}: missing data"));
        assert_eq!(data["kind"], kind, "{case}: {data}");
        assert!(!data
            .to_string()
            .contains(fixture.root.path().to_string_lossy().as_ref()));
        if case == "form-module-binding" {
            assert!(
                data["props"]["handles"]
                    .as_array()
                    .is_some_and(|handles| !handles.is_empty()),
                "{data}"
            );
        }
    }
}

#[test]
fn websocket_client_source_view_is_an_explicit_provider_gap() {
    let fixture = RealReaderFixture::new();
    let configuration_path = fixture.source.join("Configuration.xml");
    let configuration = fs::read_to_string(&configuration_path).unwrap();
    fs::write(
        &configuration_path,
        configuration.replacen(
            "</ChildObjects>",
            "<WebSocketClient>Телефония</WebSocketClient></ChildObjects>",
            1,
        ),
    )
    .unwrap();
    let cancellation = CancellationToken::new();
    let source_root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
        source_root,
        PlatformProfile::v8_3_27(),
    );

    let result = ViewService::new(authority, ViewCursorStore::default())
        .view(ViewRequest::new("main:WebSocketClient.Телефония.Module.WebSocketClient").unwrap());

    assert!(!result.ok);
    assert!(result.data.is_none());
    assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
}

#[test]
fn extension_platform_event_does_not_advertise_unproved_interception() {
    let fixture = RealReaderFixture::new();
    fixture.install_accepted_profile_sources();
    let cancellation = CancellationToken::new();
    let source_root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        "main",
        "actor-fixture-extension",
        SourceSetKind::Extension,
        source_root,
        PlatformProfile::v8_3_27(),
    );

    let result = ViewService::new(authority, ViewCursorStore::default())
        .view(ViewRequest::new("main:Document.Заказ.Module.Object.Event.BeforeWrite").unwrap());

    assert!(result.ok, "{} {:?}", result.summary, result.diagnostics);
    let data = result.data.as_ref().unwrap();
    assert_eq!(data["props"]["state"], "available");
    assert!(data.get("can").is_none(), "{data}");
}

#[test]
fn extension_root_platform_modules_are_owned_by_the_extension_root() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture.source.join("Ext/ManagedApplicationModule.bsl"),
        "Procedure ПередНачаломРаботыСистемы(Отказ)\nEndProcedure\n\nProcedure РасширениеПриСтарте()\nEndProcedure\n",
    );
    let cancellation = CancellationToken::new();
    let source_root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        "main",
        "actor-fixture-extension-root",
        SourceSetKind::Extension,
        source_root,
        PlatformProfile::v8_3_27(),
    );

    let module = ViewService::new(authority, ViewCursorStore::default())
        .view(ViewRequest::new("main:Module.ManagedApplication").unwrap());
    assert!(module.ok, "{module:?}");

    let source_root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        "main",
        "actor-fixture-extension-root-find",
        SourceSetKind::Extension,
        source_root,
        PlatformProfile::v8_3_27(),
    );
    let index = ReaderReach::new(vec![("main", &authority)]);
    for expected in [
        "main:Module.ManagedApplication",
        "main:Module.ManagedApplication.Method.РасширениеПриСтарте",
    ] {
        let found = index.find(FindRequest::new(expected).unwrap());
        assert!(
            !found.is_nearest()
                && found
                    .candidates()
                    .iter()
                    .any(|candidate| candidate.at() == expected),
            "extension root module identity is missing from find: {expected}: {found:?}"
        );
    }
}

#[test]
fn find_walks_every_real_addressable_reader_family_without_parallel_xml_semantics() {
    let fixture = RealReaderFixture::new();
    fixture.install_accepted_profile_sources();
    let authority = fixture.operation_read_authority();
    let index = ReaderReach::new(vec![("main", &authority)]);
    let expected_addresses = [
        "main:Configuration",
        "main:Catalog",
        "main:Catalog.Items",
        "main:Catalog.Items.TabularSection.Lines.Attribute.Quantity",
        "main:Report.ParityReport.Form.MainForm",
        "main:Report.ParityReport.Template.MainSchema",
        "main:Report.ParityReport.Form.MainForm.Item.Goods.Item.Quantity.Event.OnChange",
        "main:Report.ParityReport.Form.MainForm.Event.OnOpen",
        "main:Role.SalesReader.Right.Catalog_Products.RLS.View",
        "main:Subsystem.Sales.Interface",
        "main:Report.ParityReport.Template.MainSchema.DataSet.MainData.Field.Code",
        "main:Report.ParityReport.Template.MainSchema.DataSet.MainData.Parameter.Period",
        "main:Report.ParityReport.Template.Print.Area.Header.Parameter.Title",
        "main:XDTOPackage.EnterpriseData_1_17_3.Type.Документ_ЗаказКлиента.Property.Идентификаторы",
        "main:CommonModule.РеактивныйСервер.Method.InternalService",
        "main:CommonModule.РеактивныйСервер.Region.ПрограммныйИнтерфейс",
        "main:Report.ParityReport.Form.MainForm.Module.Form.Method.OnOpen",
    ];
    for expected in expected_addresses {
        let found = index.find(FindRequest::new(expected).unwrap());
        assert!(
            !found.is_nearest()
                && found
                    .candidates()
                    .iter()
                    .any(|candidate| candidate.at() == expected),
            "missing real logical identity {expected}: {found:?}",
        );
    }
}

#[test]
fn read_authority_rejects_named_root_replacement_before_node_read() {
    if !supports_retained_root_replacement_test() {
        return;
    }
    let fixture = RealReaderFixture::new();
    let root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let deadline = crate::domain::code_intelligence::ProviderDeadline::no_deadline();
    let authority = LogicalViewReadAuthority::with_read_authority(
        &fixture.cancellation,
        ProviderReadAuthority::new(
            "main",
            "actor-fixture-main",
            SourceSetKind::Configuration,
            root,
        ),
        PlatformProfile::v8_3_27(),
        deadline,
    );
    let saved = fixture.root.path().join("retained-a");
    let replacement = fixture.root.path().join("replacement-b");
    fs::create_dir_all(&replacement).unwrap();
    fs::write(
        replacement.join("Configuration.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Replacement</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
    )
    .unwrap();
    fs::rename(&fixture.source, &saved).unwrap();
    fs::rename(&replacement, &fixture.source).unwrap();

    let result = ViewService::new(authority, ViewCursorStore::default())
        .view(ViewRequest::new("main:Configuration").unwrap());

    assert!(!result.ok, "a replaced capability name escaped: {result:?}");
    assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
}

#[test]
pub(crate) fn typed_readers_never_publish_their_export_path_in_view_props() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    for at in [
        "main:Report.ParityReport.Form.MainForm",
        "main:Report.ParityReport.Form.MainForm.Item.Goods",
        "main:Role.SalesReader.Right.Catalog_Products.RLS.View",
        "main:Subsystem.Sales.Interface",
        "main:Report.ParityReport.Template.MainSchema.DataSet.MainData.Field.Code",
        "main:XDTOPackage.EnterpriseData_1_17_3.Type.Документ_ЗаказКлиента.Property.Идентификаторы",
    ] {
        let result = service.view(ViewRequest::new(at).unwrap());
        // Assertion messages name the address and the diagnostic code only:
        // a projected payload can carry descriptor identity.
        let codes = result
            .diagnostics
            .iter()
            .filter_map(|diagnostic| diagnostic["code"].as_str())
            .collect::<Vec<_>>();
        assert!(result.ok, "{at}: {codes:?}");
        let mut props = result.data.as_ref().unwrap()["props"].clone();
        if at.starts_with("main:XDTOPackage.") {
            let mut type_info: serde_json::Value =
                serde_json::from_str(props["type"].as_str().unwrap()).unwrap();
            // A namespace URI is a type identity, not an exported file path.
            assert_eq!(type_info["namespace"], "http://v8.1c.ru/8.1/data/core");
            type_info.as_object_mut().unwrap().remove("namespace");
            props["type"] = json!(serde_json::to_string(&type_info).unwrap());
        }
        let props = props.to_string();
        assert!(
            !props.contains(".xml") && !props.contains(".bsl") && !props.contains('/'),
            "{at} published a physical path in view props"
        );
    }
}

#[test]
fn missing_owner_module_branch_is_not_invented_but_registered_owner_without_bsl_is_kept() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    let missing = service.view(ViewRequest::new("main:Document.Missing.Module").unwrap());
    assert!(
        !missing.ok,
        "missing owner produced a module branch: {missing:?}"
    );
    assert_eq!(missing.diagnostics[0]["code"], "not_found");

    let registered = service.view(ViewRequest::new("main:Catalog.Items.Module").unwrap());
    assert!(registered.ok, "{:?}", registered.diagnostics);
    let items = registered.data.as_ref().unwrap()["items"]
        .as_array()
        .unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["at"], "main:Catalog.Items.Module.Object");
    assert_eq!(items[1]["at"], "main:Catalog.Items.Module.Manager");
}

#[test]
fn real_external_sources_are_traversable_without_configuration_xml_and_hide_root_runtime_modules() {
    let fixture = RealExternalReaderFixture::new();
    for (source, kind, owner) in [
        (
            "artifact_processor",
            SourceSetKind::ExternalProcessor,
            "ExternalDataProcessor.Import",
        ),
        (
            "artifact_report",
            SourceSetKind::ExternalReport,
            "ExternalReport.Sales",
        ),
    ] {
        let authority = fixture.read_authority(source, kind);
        let service = ViewService::new(authority, ViewCursorStore::default());
        let address = format!("{source}:Configuration");
        let root = service.view(ViewRequest::new(&address).unwrap());
        assert!(root.ok, "{source} root failed: {:?}", root.diagnostics);
        let root = root.data.unwrap();
        let owner_branch = root["branches"]
            .as_array()
            .unwrap()
            .iter()
            .find(|branch| branch["at"] == format!("{source}:{}", owner.split('.').next().unwrap()))
            .unwrap();
        assert_eq!(owner_branch["count"], 2);
        assert!(
            root["branches"]
                .as_array()
                .unwrap()
                .iter()
                .all(|branch| branch["at"] != format!("{source}:Module")),
            "external source root exposed configuration runtime modules: {root:?}",
        );
    }

    let epf = fixture.read_authority("artifact_processor", SourceSetKind::ExternalProcessor);
    let erf = fixture.read_authority("artifact_report", SourceSetKind::ExternalReport);
    let index = ReaderReach::new(vec![
        ("artifact_processor", &epf),
        ("artifact_report", &erf),
    ]);
    for expected in [
        "artifact_processor:ExternalDataProcessor.Import",
        "artifact_processor:ExternalDataProcessor.Import.Form.Main",
        "artifact_processor:ExternalDataProcessor.Import.Command.Run",
        "artifact_processor:ExternalDataProcessor.Import.Module.Object",
        "artifact_report:ExternalReport.Sales",
        "artifact_report:ExternalReport.Sales.Form.Main",
        "artifact_report:ExternalReport.Sales.Command.Run",
        "artifact_report:ExternalReport.Sales.Module.Object",
    ] {
        let found = index.find(FindRequest::new(expected).unwrap().with_limit(64).unwrap());
        assert!(
            !found.is_nearest()
                && found
                    .candidates()
                    .iter()
                    .any(|candidate| candidate.at() == expected),
            "missing real external identity {expected}: {found:?}",
        );
    }

    // The export-path half of this contract is proven against the find
    // directory in `v13_find`, where a candidate carries its real reason;
    // through the reader probe every candidate reads "reader".
}

#[test]
fn external_inventory_skips_runtime_sidecar_and_fails_closed_on_malformed_or_ambiguous_owner() {
    let malformed_fixture = RealExternalReaderFixture::new();
    fs::write(malformed_fixture.processor.join("Broken.xml"), "<broken>").unwrap();
    let malformed = ViewService::new(
        malformed_fixture.read_authority("artifact_processor", SourceSetKind::ExternalProcessor),
        ViewCursorStore::default(),
    )
    .view(ViewRequest::new("artifact_processor:Configuration").unwrap());
    assert!(!malformed.ok);
    assert_eq!(malformed.diagnostics[0]["code"], "provider_unavailable");

    let ambiguous_fixture = RealExternalReaderFixture::new();
    fs::copy(
        ambiguous_fixture.processor.join("Import.xml"),
        ambiguous_fixture.processor.join("Alias.xml"),
    )
    .unwrap();
    let ambiguous = ViewService::new(
        ambiguous_fixture.read_authority("artifact_processor", SourceSetKind::ExternalProcessor),
        ViewCursorStore::default(),
    )
    .view(ViewRequest::new("artifact_processor:Configuration").unwrap());
    assert!(!ambiguous.ok);
    assert_eq!(ambiguous.diagnostics[0]["code"], "provider_unavailable");
    assert!(ambiguous.diagnostics[0]["message"]
        .as_str()
        .unwrap()
        .contains("ambiguous"));
}

#[test]
fn retained_external_inventory_is_cancellable() {
    let cancelled_fixture = RealExternalReaderFixture::new();
    let root = Arc::new(RetainedDirectoryCapability::open(&cancelled_fixture.processor).unwrap());
    let reader = ProviderReadAuthority::new(
        "artifact_processor",
        "actor-external-cancel",
        SourceSetKind::ExternalProcessor,
        root,
    );
    let cancellation = CancellationToken::new();
    let mut checkpoints = 0_usize;
    let error = reader
        .configuration_payload_with_checkpoint(&mut || {
            checkpoints += 1;
            if checkpoints == 8 {
                cancellation.cancel();
            }
            if cancellation.is_cancelled() {
                Err(crate::application::v13::view::ViewError::new(
                    RefusalCode::Cancelled,
                    "external inventory cancelled",
                ))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(error.code().as_str(), "cancelled");
}

fn large_external_inventory_fixture() -> RealExternalReaderFixture {
    let fixture = RealExternalReaderFixture::new();
    add_large_external_descriptors(&fixture, 5);
    fixture
}

fn add_large_external_descriptors(fixture: &RealExternalReaderFixture, count: usize) {
    for index in 0..count {
        let name = format!("Large{index}");
        write_external_artifact(&fixture.processor, "ExternalDataProcessor", &name);
        let path = fixture.processor.join(format!("{name}.xml"));
        let descriptor = fs::read_to_string(&path).unwrap();
        let padding = format!("<!--{}-->", "x".repeat(7 * 1024 * 1024));
        fs::write(
            &path,
            descriptor.replace("</MetaDataObject>", &format!("{padding}</MetaDataObject>")),
        )
        .unwrap();
    }
}

fn external_inventory_reader(fixture: &RealExternalReaderFixture) -> ProviderReadAuthority {
    let root = Arc::new(RetainedDirectoryCapability::open(&fixture.processor).unwrap());
    ProviderReadAuthority::new(
        "artifact_processor",
        "actor-external-complete",
        SourceSetKind::ExternalProcessor,
        root,
    )
}

fn assert_complete_large_external_inventory(payload: &serde_json::Value) {
    let expected = [
        "Import",
        "Large0",
        "Large1",
        "Large2",
        "Large3",
        "Large4",
        "Импорт",
    ]
    .map(|name| json!({"kind": "ExternalDataProcessor", "name": name}));
    assert_eq!(payload["registeredObjects"], json!(expected));
}

#[test]
fn retained_external_inventory_crosses_former_aggregate_bytes_without_losing_owners() {
    let fixture = large_external_inventory_fixture();
    let reader = external_inventory_reader(&fixture);
    let payload =
        configuration_payload(&reader).expect("all sequential descriptors remain readable");
    assert_complete_large_external_inventory(&payload);
}

#[test]
fn retained_external_inventory_reads_malformed_tail_after_former_aggregate_bytes() {
    let fixture = large_external_inventory_fixture();
    // Retained names are sorted. The five Large*.xml files precede this entry
    // and contain more than 32 MiB in total.
    fs::write(fixture.processor.join("ZZBroken.xml"), "<broken>").unwrap();
    let reader = external_inventory_reader(&fixture);
    let error = configuration_payload(&reader).unwrap_err();
    assert_eq!(error.code().as_str(), "provider_unavailable");
    assert!(error.to_string().contains("ZZBroken.xml"), "{error}");
    assert!(!error.to_string().contains("read limit"), "{error}");
}

#[test]
fn retained_external_inventory_cancels_inside_a_large_descriptor_without_publishing_prefix() {
    // This stays below the former aggregate limit, so its RED cannot be
    // masked by an unrelated size refusal.
    let fixture = RealExternalReaderFixture::new();
    add_large_external_descriptors(&fixture, 1);
    assert!(fs::read_dir(&fixture.processor).unwrap().count() < 32);
    let reader = external_inventory_reader(&fixture);
    let mut checkpoints = 0;
    let error = reader
        .configuration_payload_with_checkpoint(&mut || {
            checkpoints += 1;
            // Enumeration and the preceding small descriptors use fewer calls;
            // the first 7-MiB descriptor needs more than 100 actual read chunks.
            if checkpoints == 64 {
                Err(ViewError::new(
                    RefusalCode::Cancelled,
                    "cancel inside retained XML read",
                ))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(error.code(), RefusalCode::Cancelled, "{error}");
    assert_eq!(checkpoints, 64);
    let payload =
        configuration_payload(&reader).expect("cancel did not publish a partial inventory");
    assert_eq!(
        payload["registeredObjects"],
        json!([
            {"kind": "ExternalDataProcessor", "name": "Import"},
            {"kind": "ExternalDataProcessor", "name": "Large0"},
            {"kind": "ExternalDataProcessor", "name": "Импорт"}
        ])
    );
}

#[test]
pub(crate) fn production_authorities_reach_all_profile_module_capabilities_from_real_parent_inventories(
) {
    let main_fixture = RealReaderFixture::new();
    main_fixture.install_module_matrix_sources();
    let external_fixture = RealExternalReaderFixture::new();
    let main_navigation = main_fixture.view_service();
    for address in [
        "main:WebSocketClient",
        "main:WebSocketClient.Телефония",
        "main:WebSocketClient.Телефония.Module",
    ] {
        let viewed = main_navigation.view(ViewRequest::new(address).unwrap());
        assert!(viewed.ok, "{address}: {:?}", viewed.diagnostics);
    }
    let main = main_fixture.operation_read_authority();
    let epf = external_fixture.operation_read_authority("epf", SourceSetKind::ExternalProcessor);
    let erf = external_fixture.operation_read_authority("erf", SourceSetKind::ExternalReport);
    let expected = serde_json::from_str::<ModuleCapabilityFixture>(include_str!(
        "../../../../../tests/fixtures/v013/address-profile-8.3.27.json"
    ))
    .unwrap()
    .module_capabilities
    .into_iter()
    .filter(|case| case.exists)
    .map(|case| case.at)
    .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(expected.len(), 25);

    let index = ReaderReach::new(vec![("main", &main), ("epf", &epf), ("erf", &erf)]);
    for expected_at in expected {
        let found = index.find(
            FindRequest::new(&expected_at)
                .unwrap()
                .with_limit(100)
                .unwrap(),
        );
        assert!(
            !found.is_nearest()
                && found
                    .candidates()
                    .iter()
                    .any(|candidate| candidate.at() == expected_at),
            "missing production module identity {expected_at}: {found:?}",
        );
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModuleCapabilityFixture {
    module_capabilities: Vec<ModuleCapabilityCase>,
}

#[derive(Deserialize)]
struct ModuleCapabilityCase {
    at: String,
    exists: bool,
}

#[test]
pub(crate) fn one_read_parses_each_module_source_once_per_actor_revision() {
    let fixture = RealReaderFixture::new();
    let authority = fixture.read_authority();
    assert_reader_reaches(
        &authority,
        &[
            "main:CommonModule.РеактивныйСервер.Method",
            "main:Catalog.Items",
            "main:Catalog.Items.TabularSection.Lines",
            "main:Catalog.Items.TabularSection.Lines.Attribute.Quantity",
            "main:Configuration",
        ],
    );
    assert_eq!(
        authority.module_source_read_count("CommonModule.РеактивныйСервер.Module"),
        1,
        "one actor-owned revision must parse each module source once",
    );
    assert_eq!(
        authority.configuration_payload_read_count(),
        1,
        "one actor-owned revision must parse its source-set inventory once",
    );

    let module = fixture
        .source
        .join("CommonModules/РеактивныйСервер/Ext/Module.bsl");
    let mut changed = fs::read_to_string(&module).unwrap();
    changed.push_str("\nProcedure AfterRevisionChange()\nEndProcedure\n");
    fs::write(&module, changed).unwrap();
    assert_reader_reaches(
        &authority,
        &[
            "main:CommonModule.РеактивныйСервер.Method",
            "main:Catalog.Items",
            "main:Catalog.Items.TabularSection.Lines",
            "main:Catalog.Items.TabularSection.Lines.Attribute.Quantity",
            "main:Configuration",
        ],
    );
    assert_eq!(
        authority.module_source_read_count("CommonModule.РеактивныйСервер.Module"),
        1,
        "one read keeps its own projection; it is not a snapshot of later edits",
    );
    assert_eq!(authority.configuration_payload_read_count(), 1);
    let changed_methods = ViewService::new(fixture.read_authority(), ViewCursorStore::default())
        .view(ViewRequest::new("main:CommonModule.РеактивныйСервер.Method").unwrap());
    assert!(
        changed_methods.ok,
        "a new read of the changed module must succeed"
    );
    assert!(
        serde_json::to_string(&changed_methods.data)
            .unwrap()
            .contains("AfterRevisionChange"),
        "a new read must parse the changed module source"
    );

    let second_authority = fixture.read_authority();
    assert_reader_reaches(
        &second_authority,
        &[
            "main:CommonModule.РеактивныйСервер.Method",
            "main:Catalog.Items",
            "main:Catalog.Items.TabularSection.Lines",
            "main:Catalog.Items.TabularSection.Lines.Attribute.Quantity",
            "main:Configuration",
        ],
    );
    assert_eq!(
        second_authority.module_source_read_count("CommonModule.РеактивныйСервер.Module"),
        1,
        "a second actor must own an independent operation-local projection",
    );
    assert_eq!(second_authority.configuration_payload_read_count(), 1);
}

#[test]
pub(crate) fn one_read_parses_each_metadata_descriptor_once_per_actor_revision() {
    let fixture = RealReaderFixture::new();
    let authority = fixture.read_authority();
    assert_reader_reaches(
        &authority,
        &[
            "main:Catalog.Items",
            "main:Catalog.Items.TabularSection.Lines",
            "main:Catalog.Items.TabularSection.Lines.Attribute.Quantity",
        ],
    );
    // Owner proof reads the descriptor once and the typed projection once
    // more; every logical address projected from that owner shares the parse.
    assert_eq!(
        authority.metadata_descriptor_read_count("Catalog.Items"),
        2,
        "one actor-owned revision must parse each metadata descriptor once per authority",
    );
}

#[test]
pub(crate) fn typed_reads_parse_the_support_marker_once_per_actor_revision() {
    use crate::application::v13::view::ViewReadAuthority;
    let fixture = RealReaderFixture::new();
    fs::create_dir_all(fixture.source.join("Ext")).unwrap();
    fs::copy(
        fixture_path("platform_8_3_27/support-edit-bin-only/src/Ext/ParentConfigurations.bin"),
        fixture.source.join("Ext/ParentConfigurations.bin"),
    )
    .unwrap();
    let authority = fixture.read_authority();
    // Three identities under two owners: every typed metadata read asks for
    // the owner's support state, the marker itself is parsed once.
    for at in [
        "main:Catalog.Items",
        "main:Catalog.Items.TabularSection.Lines",
        "main:Report.ParityReport",
    ] {
        let at = QualifiedAddress::parse(at).unwrap();
        let admitted = authority.snapshot(&at).unwrap();
        authority
            .read_exact(&at, &admitted)
            .unwrap_or_else(|error| panic!("{at}: {error:?}"));
    }
    assert_eq!(
        authority.support_state_read_count(),
        1,
        "one actor-owned revision must parse Ext/ParentConfigurations.bin once, not once per owner",
    );
}

/// A support marker replaced after an object read of the same operation
/// must not reach the configuration root, and the retained root must never
/// pair the new marker's stamp with the state read before it.
#[test]
fn replaced_support_marker_never_reaches_a_retained_root_with_the_old_state() {
    use crate::infrastructure::v13_large_configuration::RegistrationCache;

    let fixture = RealReaderFixture::new();
    let marker = fixture.source.join("Ext/ParentConfigurations.bin");
    fs::create_dir_all(marker.parent().unwrap()).unwrap();
    fs::copy(
        fixture_path("platform_8_3_27/support-edit-bin-only/src/Ext/ParentConfigurations.bin"),
        &marker,
    )
    .unwrap();
    let reader = |artifacts: Arc<RegistrationCache>| {
        ProviderReadAuthority::new(
            "main",
            "actor-fixture-main",
            SourceSetKind::Configuration,
            Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap()),
        )
        .with_artifacts(artifacts)
    };
    let fresh = || reader(Arc::new(RegistrationCache::default()));
    let supported = configuration_payload(&fresh()).unwrap()["support"].clone();

    let retained = Arc::new(RegistrationCache::with_settle_window_for_test(
        std::time::Duration::ZERO,
    ));
    let operation = reader(Arc::clone(&retained));
    let target = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Items").unwrap();
    operation.object_support(&target, &mut || Ok(())).unwrap();
    fs::remove_file(&marker).unwrap();

    let expected = configuration_payload(&fresh()).unwrap()["support"].clone();
    assert_ne!(
        expected, supported,
        "the fixture must change the support answer"
    );
    assert_eq!(
        configuration_payload(&operation).unwrap()["support"],
        expected,
        "the root combined a removed marker with the state read before it"
    );
    assert_eq!(
        configuration_payload(&reader(retained)).unwrap()["support"],
        expected,
        "the retained root kept the support state of a replaced marker"
    );
}

#[test]
fn large_support_marker_reads_late_rules_once_for_multiple_owners() {
    use crate::domain::support_state::ObjectSupportState;
    use crate::infrastructure::native_operations::common::support_root_uuid_from_bytes;

    let fixture = RealReaderFixture::new();
    let catalog_uuid =
        support_root_uuid_from_bytes(&fs::read(fixture.source.join("Catalogs/Items.xml")).unwrap())
            .unwrap();
    let report_uuid = support_root_uuid_from_bytes(
        &fs::read(fixture.source.join("Reports/ParityReport.xml")).unwrap(),
    )
    .unwrap();
    assert_ne!(catalog_uuid, report_uuid);
    let mut marker = format!("{{6,0,1,0,0,{catalog_uuid},").into_bytes();
    // The final rule straddles a 64 KiB window after the former 8 MiB ceiling.
    marker.resize(8 * 1024 * 1024 + 64 * 1024 - 13, b' ');
    marker.extend_from_slice(format!("1,0,{report_uuid},}}").as_bytes());
    fs::create_dir_all(fixture.source.join("Ext")).unwrap();
    fs::write(fixture.source.join("Ext/ParentConfigurations.bin"), marker).unwrap();
    let authority = fixture.read_authority();
    for (path, expected) in [
        ("Catalog.Items", ObjectSupportState::Locked),
        (
            "Report.ParityReport",
            ObjectSupportState::EditableWithSupport,
        ),
        ("Catalog.Items", ObjectSupportState::Locked),
    ] {
        let target = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, path).unwrap();
        assert_eq!(
            authority
                .read
                .object_support(&target, &mut || Ok(()))
                .unwrap()
                .state,
            expected
        );
    }
    let configuration = configuration_payload(&authority.read).unwrap();
    assert_eq!(
        configuration["support"]["objects"],
        json!({"locked": 1, "editable": 1, "removed": 0})
    );
    assert_eq!(authority.support_state_read_count(), 1);
}

#[test]
fn support_marker_stream_preserves_terminal_errors_and_never_memoizes_interruption() {
    use crate::application::v13::view::ViewError;
    let fixture = RealReaderFixture::new();
    let marker_path = fixture.source.join("Ext/ParentConfigurations.bin");
    fs::create_dir_all(marker_path.parent().unwrap()).unwrap();
    let mut marker = b"{6,0,1,".to_vec();
    marker.resize(64 * 1024 + 1, b' ');
    fs::write(&marker_path, marker).unwrap();
    let target = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Items").unwrap();
    for code in [RefusalCode::Cancelled, RefusalCode::DeadlineExceeded] {
        // Before opening, between chunks, after EOF, and immediately before memo publication.
        for stop_at in [1, 3, 5, 6] {
            let authority = fixture.read_authority();
            let mut calls = 0;
            let error = authority
                .read
                .object_support(&target, &mut || {
                    calls += 1;
                    if calls == stop_at {
                        Err(ViewError::new(code, "original operation stopped"))
                    } else {
                        Ok(())
                    }
                })
                .unwrap_err();
            assert_eq!(error.code(), code, "checkpoint {stop_at}");
            assert_eq!(calls, stop_at);
            let reads_before_retry = authority.support_state_read_count();
            authority
                .read
                .object_support(&target, &mut || Ok(()))
                .unwrap();
            assert_eq!(
                authority.support_state_read_count(),
                reads_before_retry + 1,
                "interrupted state was not published"
            );
            let mut calls = 0;
            let error = authority
                .read
                .object_support(&target, &mut || {
                    calls += 1;
                    if calls == 2 {
                        Err(ViewError::new(code, "cached read stopped"))
                    } else {
                        Ok(())
                    }
                })
                .unwrap_err();
            assert_eq!(error.code(), code);
            assert_eq!(calls, 2, "cached return keeps the caller checkpoint");
            assert_eq!(authority.support_state_read_count(), reads_before_retry + 1);
        }
    }
    fs::remove_file(marker_path).unwrap();
    for code in [RefusalCode::Cancelled, RefusalCode::DeadlineExceeded] {
        let authority = fixture.read_authority();
        let mut calls = 0;
        let error = authority
            .read
            .object_support(&target, &mut || {
                calls += 1;
                if calls == 2 {
                    Err(ViewError::new(code, "absent marker read stopped"))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        assert_eq!(error.code(), code);
        authority
            .read
            .object_support(&target, &mut || Ok(()))
            .unwrap();
        assert_eq!(
            authority.support_state_read_count(),
            0,
            "an absent marker has nothing to parse"
        );
    }
}

#[test]
fn ambiguous_short_role_alias_is_rejected_and_canonical_aliases_work() {
    let payload = json!({
        "name": "SalesReader",
        "allowed": [
            {"kind": "Catalog", "objects": [{"name": "Orders", "rights": []}]},
            {"kind": "Document", "objects": [{"name": "Orders", "rights": []}]}
        ],
        "denied": []
    });
    let short = QualifiedAddress::parse("main:Role.SalesReader.Right.Orders").unwrap();
    let route = route_logical_address(&short, PlatformProfile::v8_3_27()).unwrap();
    let error = project_typed_payload(&route, payload.clone()).unwrap_err();
    assert_eq!(error.code().as_str(), "bad_value");
    assert!(error.to_string().contains("Catalog_Orders"));
    assert!(error.to_string().contains("Document_Orders"));

    for canonical in ["Catalog_Orders", "Document_Orders"] {
        let at =
            QualifiedAddress::parse(&format!("main:Role.SalesReader.Right.{canonical}")).unwrap();
        let route = route_logical_address(&at, PlatformProfile::v8_3_27()).unwrap();
        let projected = project_typed_payload(&route, payload.clone()).unwrap();
        assert_eq!(
            projected.at(),
            format!("main:Role.SalesReader.Right.{canonical}")
        );
    }
}

struct RealExternalReaderFixture {
    _root: tempfile::TempDir,
    context: WorkspaceContext,
    processor: PathBuf,
    report: PathBuf,
    cancellation: CancellationToken,
}

impl RealExternalReaderFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("cache");
        let processor = root.path().join("processor");
        let report = root.path().join("report");
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(&processor).unwrap();
        fs::create_dir_all(&report).unwrap();
        write_external_artifact(&processor, "ExternalDataProcessor", "Import");
        write_external_artifact(&processor, "ExternalDataProcessor", "Импорт");
        write_external_artifact(&report, "ExternalReport", "Sales");
        write_external_artifact(&report, "ExternalReport", "Продажи");
        fs::copy(
            fixture_path("platform_8_3_27/staged_dump_roots/ConfigDumpInfo.xml"),
            processor.join("ConfigDumpInfo.xml"),
        )
        .unwrap();
        fs::copy(
            fixture_path("platform_8_3_27/staged_dump_roots/ConfigDumpInfo.xml"),
            report.join("ConfigDumpInfo.xml"),
        )
        .unwrap();
        fs::write(
            root.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: artifact_processor\n    type: EXTERNAL_PROCESSOR\n    path: processor\n  - name: artifact_report\n    type: EXTERNAL_REPORT\n    path: report\n",
        )
        .unwrap();
        let canonical_root = fs::canonicalize(root.path()).unwrap();
        let processor = fs::canonicalize(processor).unwrap();
        let report = fs::canonicalize(report).unwrap();
        let context = WorkspaceContext {
            cwd: canonical_root.clone(),
            workspace_root: canonical_root,
            cache_root: cache,
            workspace_epoch: 1,
        };
        Self {
            _root: root,
            context,
            processor,
            report,
            cancellation: CancellationToken::new(),
        }
    }

    fn read_authority(
        &self,
        source_set: &str,
        kind: SourceSetKind,
    ) -> LogicalViewReadAuthority<'_> {
        let source = if kind == SourceSetKind::ExternalProcessor {
            &self.processor
        } else {
            &self.report
        };
        let root = Arc::new(RetainedDirectoryCapability::open(source).unwrap());
        LogicalViewReadAuthority::new(
            &self.cancellation,
            source_set,
            format!("actor-{source_set}"),
            kind,
            root,
            PlatformProfile::v8_3_27(),
        )
    }

    fn operation_read_authority(
        &self,
        source_set: &str,
        kind: SourceSetKind,
    ) -> LogicalViewReadAuthority<'_> {
        let source = if kind == SourceSetKind::ExternalProcessor {
            &self.processor
        } else {
            &self.report
        };
        let root = Arc::new(RetainedDirectoryCapability::open(source).unwrap());
        let deadline = ProviderDeadline::no_deadline();
        LogicalViewReadAuthority::with_read_authority(
            &self.cancellation,
            ProviderReadAuthority::new(source_set, format!("actor-{source_set}"), kind, root),
            PlatformProfile::v8_3_27(),
            deadline,
        )
    }
}

fn write_external_artifact(source: &Path, kind: &str, name: &str) {
    let (form_name, command_name) = match name {
        "Импорт" => ("Основная", "Выполнить"),
        "Продажи" => ("Основная", "Сформировать"),
        _ => ("Main", "Run"),
    };
    write(
        &source.join(format!("{name}.xml")),
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
  <{kind} uuid="10000000-0000-4000-8000-000000000010">
    <Properties><Name>{name}</Name></Properties>
    <ChildObjects><Form>{form_name}</Form><Command uuid="10000000-0000-4000-8000-000000000012"><Properties><Name>{command_name}</Name></Properties></Command></ChildObjects>
  </{kind}>
</MetaDataObject>"#,
        ),
    );
    write(
        &source.join(format!("{name}/Forms/{form_name}.xml")),
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form uuid="10000000-0000-4000-8000-000000000011"><Properties><Name>{form_name}</Name><FormType>Managed</FormType></Properties></Form></MetaDataObject>"#
        ),
    );
    write(
        &source.join(format!("{name}/Forms/{form_name}/Ext/Form.xml")),
        r#"<?xml version="1.0" encoding="UTF-8"?><Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><ChildItems/></Form>"#,
    );
    write(
        &source.join(format!("{name}/Commands/{command_name}/Ext/CommandModule.bsl")),
        "&AtClient\nProcedure CommandProcessing(CommandParameter, CommandExecuteParameters)\nEndProcedure\n",
    );
    write(
        &source.join(format!("{name}/Ext/ObjectModule.bsl")),
        "Procedure Execute()\nEndProcedure\n",
    );
}

/// Proves the typed reader reaches an address. `find` no longer traverses the
/// logical tree, so reader coverage is asserted against the reader itself.
fn assert_reader_reaches(authority: &LogicalViewReadAuthority<'_>, addresses: &[&str]) {
    for at in addresses {
        let address = QualifiedAddress::parse(at).unwrap_or_else(|error| panic!("{at}: {error}"));
        let admitted = authority
            .snapshot(&address)
            .unwrap_or_else(|error| panic!("{at}: {error:?}"));
        authority
            .read_exact(&address, &admitted)
            .unwrap_or_else(|error| panic!("{at}: {error:?}"));
    }
}

/// `find` is a layout directory now, so reader-coverage tests probe the typed
/// reader directly. The shim keeps the assertion shape those tests already use.
struct ReaderReach<'a> {
    authorities: Vec<(&'a str, &'a LogicalViewReadAuthority<'a>)>,
}

impl<'a> ReaderReach<'a> {
    fn new(authorities: Vec<(&'a str, &'a LogicalViewReadAuthority<'a>)>) -> Self {
        Self { authorities }
    }

    fn find(&self, request: FindRequest) -> FindResult {
        let at = request.query();
        let reached = QualifiedAddress::parse(at).is_ok_and(|address| {
            self.authorities
                .iter()
                .filter(|(name, _)| *name == address.source_set())
                .any(|(_, authority)| {
                    authority.snapshot(&address).is_ok_and(|admitted| {
                        match authority.read_exact(&address, &admitted) {
                            Ok(_) => true,
                            // WebSocketClient is the one profile identity whose
                            // source layout 8.3.27 leaves unspecified: it
                            // answers a typed `provider_unavailable` while the
                            // reader still knows the address. No other provider
                            // failure counts as coverage.
                            Err(error) => {
                                error.code().as_str() == "provider_unavailable"
                                    && address.segments().first().is_some_and(|segment| {
                                        segment.kind()
                                            == crate::domain::address::NodeKind::WebSocketClient
                                    })
                            }
                        }
                    })
                })
        });
        FindResult::reader_probe(at, reached)
    }
}

fn write_metadata_owner(
    source: &Path,
    directory: &str,
    kind: &str,
    name: &str,
    child_objects: &str,
) {
    write(
        &source.join(format!("{directory}/{name}.xml")),
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
  <{kind} uuid="10000000-0000-4000-8000-000000000020">
    <Properties><Name>{name}</Name></Properties>
    <ChildObjects>{child_objects}</ChildObjects>
  </{kind}>
</MetaDataObject>"#,
        ),
    );
}

struct RealReaderFixture {
    root: tempfile::TempDir,
    context: WorkspaceContext,
    source: PathBuf,
    cancellation: CancellationToken,
}

impl RealReaderFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("src");
        let cache = root.path().join("cache");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&cache).unwrap();
        fs::write(
                root.path().join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
        let config = fixture_text("xdto/enterprise-data-minimal/Configuration.xml");
        fs::write(
                source.join("Configuration.xml"),
                replace_child_objects(
                    &config,
                    "\n\t\t\t<Constant>MainCurrency</Constant>\n\t\t\t<Catalog>Items</Catalog>\n\t\t\t<Document>Order</Document>\n\t\t\t<ChartOfAccounts>Accounts</ChartOfAccounts>\n\t\t\t<Catalog>Владельцы</Catalog>\n\t\t\t<DocumentJournal>Журнал</DocumentJournal>\n\t\t\t<ChartOfCalculationTypes>ВидыРасчета</ChartOfCalculationTypes>\n\t\t\t<EventSubscription>ПриЗаписи</EventSubscription>\n\t\t\t<CalculationRegister>Payroll</CalculationRegister>\n\t\t\t<Report>ParityReport</Report>\n\t\t\t<Role>SalesReader</Role>\n\t\t\t<Subsystem>Sales</Subsystem>\n\t\t\t<XDTOPackage>EnterpriseData_1_17_3</XDTOPackage>\n\t\t\t<CommonModule>РеактивныйСервер</CommonModule>\n\t\t\t<ScheduledJob>MonthClose</ScheduledJob>\n\t\t\t<HTTPService>ExternalAPI</HTTPService>\n\t\t\t<WebService>Exchange</WebService>\n\t\t",
                ),
            )
            .unwrap();
        let catalog = with_root_version(&fixture_text(
            "platform_8_3_27/meta_info/edge/catalog-child-kinds.xml",
        ))
        .replace(
            "<TabularSection><Properties><Name>Lines</Name></Properties><ChildObjects/></TabularSection>",
            "<TabularSection><Properties><Name>Lines</Name></Properties><ChildObjects><Attribute><Properties><Name>Quantity</Name><Type><v8:Type>xs:decimal</v8:Type></Type></Properties></Attribute></ChildObjects></TabularSection>",
        )
        // Свойства корня каталога: без них узел метаданных нечем проверить.
        .replace(
            "<Properties><Name>Items</Name></Properties>",
            concat!(
                "<Properties xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\"",
                " xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"",
                " xmlns:xs=\"http://www.w3.org/2001/XMLSchema\">",
                "<Name>Items</Name><Hierarchical>true</Hierarchical>",
                "<CodeType>String</CodeType><CodeLength>11</CodeLength>",
                "<Owners><xr:Item xsi:type=\"xr:MDObjectRef\">Catalog.Владельцы</xr:Item></Owners>",
                "<InputByString><xr:Field>Catalog.Items.Attribute.Code</xr:Field></InputByString>",
                "<DataLockFields><xr:Field>Catalog.Items.Attribute.Code</xr:Field></DataLockFields>",
                // Характеристика названа парой источников, а не именем.
                "<Characteristics>",
                "<xr:Characteristic>",
                "<xr:CharacteristicTypes from=\"Catalog.Items.TabularSection.Lines.Attribute.Quantity\">",
                "<xr:KeyField>Catalog.Items.Attribute.Code</xr:KeyField>",
                "<xr:TypesFilterField>Catalog.Items.Attribute.Code</xr:TypesFilterField>",
                "<xr:TypesFilterValue xsi:type=\"xs:string\">Catalog_Items</xr:TypesFilterValue>",
                "<xr:DataPathField>-1</xr:DataPathField>",
                "<xr:MultipleValuesUseField>-1</xr:MultipleValuesUseField>",
                "</xr:CharacteristicTypes>",
                "<xr:CharacteristicValues from=\"Catalog.Items\">",
                "<xr:ObjectField>Catalog.Items.Attribute.Code</xr:ObjectField>",
                "<xr:TypeField>Catalog.Items.Attribute.Code</xr:TypeField>",
                "<xr:ValueField>Catalog.Items.Attribute.Code</xr:ValueField>",
                "<xr:MultipleValuesKeyField>-1</xr:MultipleValuesKeyField>",
                "<xr:MultipleValuesOrderField>-1</xr:MultipleValuesOrderField>",
                "</xr:CharacteristicValues>",
                "</xr:Characteristic></Characteristics></Properties>",
            ),
        );
        write(&source.join("Catalogs/Items.xml"), &catalog);
        // Остальные источники ссылок: владелец, журнал, базовые виды расчёта и
        // источник события подписки. Правило обещает всякую ссылку, и проверить
        // его можно только всеми её источниками.
        for (relative, body) in [
            (
                "Catalogs/Владельцы.xml",
                r#"<Catalog uuid="99999999-9999-4999-8999-999999999991"><Properties><Name>Владельцы</Name></Properties><ChildObjects/></Catalog>"#,
            ),
            (
                "DocumentJournals/Журнал.xml",
                r#"<DocumentJournal uuid="99999999-9999-4999-8999-999999999992"><Properties><Name>Журнал</Name><RegisteredDocuments><xr:Item xsi:type="xr:MDObjectRef">Document.Order</xr:Item></RegisteredDocuments></Properties><ChildObjects/></DocumentJournal>"#,
            ),
            (
                "ChartsOfCalculationTypes/ВидыРасчета.xml",
                r#"<ChartOfCalculationTypes uuid="99999999-9999-4999-8999-999999999993"><Properties><Name>ВидыРасчета</Name><BaseCalculationTypes><xr:Item xsi:type="xr:MDObjectRef">ChartOfCalculationTypes.ВидыРасчета</xr:Item></BaseCalculationTypes></Properties><ChildObjects/></ChartOfCalculationTypes>"#,
            ),
            (
                "EventSubscriptions/ПриЗаписи.xml",
                r#"<EventSubscription uuid="99999999-9999-4999-8999-999999999994"><Properties><Name>ПриЗаписи</Name><Source><v8:Type>cfg:CatalogObject.Items</v8:Type></Source><Event>BeforeWrite</Event><Handler>CommonModule.РеактивныйСервер.ПриЗаписи</Handler></Properties></EventSubscription>"#,
            ),
        ] {
            write(
                &source.join(relative),
                &format!(
                    concat!(
                        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
                        "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\"",
                        " xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"",
                        " xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\"",
                        " xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\"",
                        " xmlns:xs=\"http://www.w3.org/2001/XMLSchema\"",
                        " xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"",
                        " version=\"2.20\">{}</MetaDataObject>"
                    ),
                    body
                ),
            );
        }
        // План счетов со стандартной табличной частью: у неё есть имя, и она
        // адресуется, а внутри — своя последовательность.
        write(
            &source.join("ChartsOfAccounts/Accounts.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20">
  <ChartOfAccounts uuid="77777777-7777-4777-8777-777777777778">
    <Properties>
      <Name>Accounts</Name>
      <StandardTabularSections>
        <xr:StandardTabularSection name="ExtDimensionTypes">
          <xr:Synonym/><xr:Comment/><xr:ToolTip/>
          <xr:FillChecking>DontCheck</xr:FillChecking>
          <xr:StandardAttributes>
            <xr:StandardAttribute name="ExtDimensionType"/>
          </xr:StandardAttributes>
        </xr:StandardTabularSection>
      </StandardTabularSections>
    </Properties>
    <ChildObjects/>
  </ChartOfAccounts>
</MetaDataObject>"#,
        );
        // Предопределённые элементы справочника: писатель у них есть, и теперь
        // есть читатель.
        write(
            &source.join("Catalogs/Items/Ext/Predefined.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20">
  <Item id="c7d2e6fc-3824-4b56-b4be-ae6be4944c0e">
    <Name>Основной</Name>
    <Code/>
    <Description>Основной элемент</Description>
    <IsFolder>false</IsFolder>
  </Item>
</PredefinedData>"#,
        );
        // Документ со ссылками наружу: движение по регистру и основание.
        write(
            &source.join("Documents/Order.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
  <Document uuid="88888888-8888-4888-8888-888888888888">
    <Properties>
      <Name>Order</Name>
      <RegisterRecords>
        <xr:Item xsi:type="xr:MDObjectRef">CalculationRegister.Payroll</xr:Item>
      </RegisterRecords>
      <BasedOn>
        <xr:Item xsi:type="xr:MDObjectRef">Catalog.Items</xr:Item>
      </BasedOn>
    </Properties>
    <ChildObjects/>
  </Document>
</MetaDataObject>"#,
        );
        // Три вида с собственными данными: обработчик, расписание и тип.
        for (relative, fixture) in [
            ("Constants/MainCurrency.xml", "constant-type.xml"),
            (
                "CalculationRegisters/Payroll.xml",
                "calculation-register.xml",
            ),
            ("ScheduledJobs/MonthClose.xml", "scheduled-job.xml"),
            ("HTTPServices/ExternalAPI.xml", "http-service.xml"),
            ("WebServices/Exchange.xml", "web-service.xml"),
        ] {
            write(
                &source.join(relative),
                &with_root_version(&fixture_text(&format!(
                    "platform_8_3_27/meta_info/edge/{fixture}"
                ))),
            );
        }
        write(
            &source.join("Catalogs/Items/Forms/ItemForm.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form uuid="10000000-0000-4000-8000-000000000031"><Properties><Name>ItemForm</Name><FormType>Managed</FormType></Properties></Form></MetaDataObject>"#,
        );
        write(
            &source.join("Catalogs/Items/Forms/ItemForm/Ext/Form.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><ChildItems/></Form>"#,
        );
        write(
            &source.join("Catalogs/Items/Templates/Print.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template uuid="10000000-0000-4000-8000-000000000032"><Properties><Name>Print</Name><TemplateType>SpreadsheetDocument</TemplateType></Properties></Template></MetaDataObject>"#,
        );
        copy_fixture(
            "platform_8_3_27/mxl/Template.xml",
            &source.join("Catalogs/Items/Templates/Print/Ext/Template.xml"),
        );
        write(
            &source.join("Catalogs/Items/Commands/Refresh/Ext/CommandModule.bsl"),
            "&AtClient\nProcedure CommandProcessing(CommandParameter, CommandExecuteParameters)\nEndProcedure\n",
        );
        let report = fixture_text("unica_mcp_script_parity/form-remove/ParityReport.xml");
        fs::create_dir_all(source.join("Reports")).unwrap();
        fs::write(
                source.join("Reports/ParityReport.xml"),
                replace_child_objects(
                    &report,
                    "\n\t\t\t<Form>MainForm</Form>\n\t\t\t<Template>MainSchema</Template>\n\t\t\t<Template>Print</Template>\n\t\t",
                ),
            )
            .unwrap();
        copy_fixture(
            "unica_mcp_script_parity/form-remove/ParityReport/Forms/MainForm.xml",
            &source.join("Reports/ParityReport/Forms/MainForm.xml"),
        );
        let form = r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <ChildItems>
    <Table name="Goods">
      <ChildItems>
        <InputField name="Quantity">
          <Events><Event name="OnChange">QuantityOnChange</Event></Events>
        </InputField>
      </ChildItems>
    </Table>
  </ChildItems>
  <Events><Event name="OnOpen">OnOpen</Event></Events>
</Form>"#;
        write(
            &source.join("Reports/ParityReport/Forms/MainForm/Ext/Form.xml"),
            form,
        );
        write(
            &source.join("Reports/ParityReport/Forms/MainForm/Ext/Form/Module.bsl"),
            "&AtClient\nProcedure OnOpen()\nEndProcedure\n\n&AtClient\nProcedure QuantityOnChange()\nEndProcedure\n",
        );
        write(
            &source.join("Roles/SalesReader.xml"),
            &with_root_version(&fixture_text(
                "unica_mcp_script_parity/role-info/SalesReader.xml",
            )),
        );
        write(
            &source.join("Roles/SalesReader/Ext/Rights.xml"),
            &fixture_text("unica_mcp_script_parity/role-info/SalesReader/Ext/Rights.xml")
                .replace("version=\"2.17\"", "version=\"2.20\""),
        );
        let subsystem = r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
  <Subsystem><Properties><Name>Sales</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Продажи</v8:content></v8:item></Synonym><IncludeInCommandInterface>true</IncludeInCommandInterface><UseOneCommand>false</UseOneCommand><Content><xr:Item xsi:type="xr:MDObjectRef">Catalog.Items</xr:Item></Content></Properties><ChildObjects/></Subsystem>
</MetaDataObject>"#;
        write(&source.join("Subsystems/Sales.xml"), subsystem);
        copy_fixture(
            "unica_mcp_script_parity/meta-remove/Subsystems/Sales/Ext/CommandInterface.xml",
            &source.join("Subsystems/Sales/Ext/CommandInterface.xml"),
        );
        copy_fixture(
            "unica_mcp_script_parity/template-remove/ParityReport/Templates/MainSchema.xml",
            &source.join("Reports/ParityReport/Templates/MainSchema.xml"),
        );
        let dcs = fixture_text("unica_mcp_script_parity/dcs-validate/BadPrefix.xml")
            .replace(
                "xmlns:bad=\"http://example.com\">bad:CatalogRef.X",
                ">xs:string",
            )
            .replace(
                "\n</DataCompositionSchema>",
                "\n\t<parameter><name>Period</name><valueType><v8:Type>xs:dateTime</v8:Type></valueType></parameter>\n</DataCompositionSchema>",
            );
        write(
            &source.join("Reports/ParityReport/Templates/MainSchema/Ext/Template.xml"),
            &dcs,
        );
        let print_descriptor = fixture_text(
            "unica_mcp_script_parity/template-remove/ParityReport/Templates/MainSchema.xml",
        )
        .replace("MainSchema", "Print")
        .replace("DataCompositionSchema", "SpreadsheetDocument");
        write(
            &source.join("Reports/ParityReport/Templates/Print.xml"),
            &print_descriptor,
        );
        let mxl = fixture_text("platform_8_3_27/mxl/Template.xml")
            .replace(
                "\n\t<templateMode>true</templateMode>",
                "\n\t<namedItem xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"NamedItemCells\"><name>Header</name><area><type>Rows</type><beginRow>0</beginRow><endRow>0</endRow><beginColumn>-1</beginColumn><endColumn>-1</endColumn></area></namedItem>\n\t<templateMode>true</templateMode>",
            )
            .replace(
                "\n\t\t\t<empty>true</empty>",
                "\n\t\t\t<c><c><f>0</f><parameter>Title</parameter></c></c><c><c><f>1</f><tl><content>Sales report</content></tl></c></c><c><c><f>2</f><tl><content>Period: [Since]</content></tl></c></c>",
            );
        write(
            &source.join("Reports/ParityReport/Templates/Print/Ext/Template.xml"),
            &mxl,
        );
        copy_fixture(
            "xdto/enterprise-data-minimal/XDTOPackages/EnterpriseData_1_17_3.xml",
            &source.join("XDTOPackages/EnterpriseData_1_17_3.xml"),
        );
        copy_fixture(
            "xdto/enterprise-data-minimal/XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin",
            &source.join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin"),
        );
        copy_fixture(
            "platform_8_3_27/support-edit-bin-only/src/CommonModules/РеактивныйСервер.xml",
            &source.join("CommonModules/РеактивныйСервер.xml"),
        );
        copy_fixture(
                "platform_8_3_27/support-edit-bin-only/src/CommonModules/РеактивныйСервер/Ext/Module.bsl",
                &source.join("CommonModules/РеактивныйСервер/Ext/Module.bsl"),
            );
        let common_module_path = source.join("CommonModules/РеактивныйСервер/Ext/Module.bsl");
        let mut common_module = fs::read_to_string(&common_module_path).unwrap();
        common_module.push_str("\nПроцедура InternalService()\nКонецПроцедуры\n");
        fs::write(common_module_path, common_module).unwrap();
        let canonical_root = fs::canonicalize(root.path()).unwrap();
        let source = fs::canonicalize(&source).unwrap();
        let context = WorkspaceContext {
            cwd: canonical_root.clone(),
            workspace_root: canonical_root,
            cache_root: cache,
            workspace_epoch: 1,
        };
        Self {
            root,
            context,
            source,
            cancellation: CancellationToken::new(),
        }
    }

    fn read_authority(&self) -> LogicalViewReadAuthority<'_> {
        let source_root = Arc::new(RetainedDirectoryCapability::open(&self.source).unwrap());
        LogicalViewReadAuthority::new(
            &self.cancellation,
            "main",
            "actor-fixture-main",
            SourceSetKind::Configuration,
            source_root,
            PlatformProfile::v8_3_27(),
        )
    }

    fn operation_read_authority(&self) -> LogicalViewReadAuthority<'_> {
        let root = Arc::new(RetainedDirectoryCapability::open(&self.source).unwrap());
        let deadline = ProviderDeadline::no_deadline();
        LogicalViewReadAuthority::with_read_authority(
            &self.cancellation,
            ProviderReadAuthority::new(
                "main",
                "actor-fixture-main",
                SourceSetKind::Configuration,
                root,
            ),
            PlatformProfile::v8_3_27(),
            deadline,
        )
    }

    fn view_service(&self) -> ViewService<LogicalViewReadAuthority<'_>> {
        ViewService::new(self.read_authority(), ViewCursorStore::default())
    }

    /// Тот же набор, объявленный расширением: заимствование живёт только здесь.
    fn extension_read_authority(&self) -> LogicalViewReadAuthority<'_> {
        let source_root = Arc::new(RetainedDirectoryCapability::open(&self.source).unwrap());
        LogicalViewReadAuthority::new(
            &self.cancellation,
            "main",
            "actor-fixture-extension-borrowing",
            SourceSetKind::Extension,
            source_root,
            PlatformProfile::v8_3_27(),
        )
    }

    fn extension_view_service(&self) -> ViewService<LogicalViewReadAuthority<'_>> {
        ViewService::new(self.extension_read_authority(), ViewCursorStore::default())
    }

    /// Платформенная форма заимствования: `Adopted` в свойствах,
    /// `ExtendedConfigurationObject` с UUID родителя после имени и, когда
    /// свойство перекрыто, запись `xr:PropertyState` в `InternalInfo`.
    fn borrow_catalog(&self, relative: &str, overridden: Option<&str>) {
        let path = self.source.join(relative);
        let text = fs::read_to_string(&path).unwrap();
        let properties = text.find("<Properties>").unwrap();
        let after_open = properties + "<Properties>".len();
        let mut patched = format!(
            "{}<ObjectBelonging>Adopted</ObjectBelonging>{}",
            &text[..after_open],
            &text[after_open..]
        );
        let name_end = patched.find("</Name>").unwrap() + "</Name>".len();
        patched = format!(
            "{}<ExtendedConfigurationObject>11111111-1111-4111-8111-111111111111</ExtendedConfigurationObject>{}",
            &patched[..name_end],
            &patched[name_end..]
        );
        if let Some(property) = overridden {
            let properties = patched.find("<Properties>").unwrap();
            patched = format!(
                "{}<InternalInfo><xr:PropertyState><xr:Property>{property}</xr:Property><xr:State>Extended</xr:State></xr:PropertyState></InternalInfo>{}",
                &patched[..properties],
                &patched[properties..]
            );
        }
        write(&path, &patched);
    }

    fn install_main_form_sources(&self, form_xml: &str, module_bsl: &str) {
        write(
            &self
                .source
                .join("Reports/ParityReport/Forms/MainForm/Ext/Form.xml"),
            form_xml,
        );
        write(
            &self
                .source
                .join("Reports/ParityReport/Forms/MainForm/Ext/Form/Module.bsl"),
            module_bsl,
        );
    }

    fn install_module_matrix_sources(&self) {
        let configuration_path = self.source.join("Configuration.xml");
        let configuration = fs::read_to_string(&configuration_path).unwrap();
        let registrations = [
            ("Document", "Заказ"),
            ("Document", "ЕщеНеВыгружен"),
            ("InformationRegister", "Цены"),
            ("Constant", "ОсновнаяВалюта"),
            ("CommonModule", "ЗаказыСервер"),
            ("CommonForm", "Подбор"),
            ("CommonCommand", "ОткрытьНастройки"),
            ("HTTPService", "API"),
            ("WebService", "Обмен"),
            ("IntegrationService", "Шина"),
            ("Bot", "Помощник"),
            ("WebSocketClient", "Телефония"),
        ]
        .into_iter()
        .map(|(kind, name)| format!("\n\t\t\t<{kind}>{name}</{kind}>"))
        .collect::<String>();
        fs::write(
            &configuration_path,
            configuration.replacen(
                "</ChildObjects>",
                &format!("{registrations}\n\t\t</ChildObjects>"),
                1,
            ),
        )
        .unwrap();

        write_metadata_owner(
            &self.source,
            "Documents",
            "Document",
            "Заказ",
            "<Form>ФормаДокумента</Form><Command uuid=\"10000000-0000-4000-8000-000000000022\"><Properties><Name>ПровестиИЗакрыть</Name></Properties></Command>",
        );
        write_metadata_owner(&self.source, "Documents", "Document", "ЕщеНеВыгружен", "");
        write_metadata_owner(
            &self.source,
            "InformationRegisters",
            "InformationRegister",
            "Цены",
            "",
        );
        write_metadata_owner(&self.source, "Constants", "Constant", "ОсновнаяВалюта", "");
        let common = fixture_text(
            "platform_8_3_27/support-edit-bin-only/src/CommonModules/РеактивныйСервер.xml",
        )
        .replace("РеактивныйСервер", "ЗаказыСервер");
        write(&self.source.join("CommonModules/ЗаказыСервер.xml"), &common);
        write_metadata_owner(&self.source, "CommonForms", "CommonForm", "Подбор", "");
        write(
            &self.source.join("CommonForms/Подбор/Ext/Form.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><ChildItems/></Form>"#,
        );
        write_metadata_owner(
            &self.source,
            "CommonCommands",
            "CommonCommand",
            "ОткрытьНастройки",
            "",
        );
        for (directory, kind, name) in [
            ("HTTPServices", "HTTPService", "API"),
            ("WebServices", "WebService", "Обмен"),
            ("IntegrationServices", "IntegrationService", "Шина"),
            ("Bots", "Bot", "Помощник"),
        ] {
            write_metadata_owner(&self.source, directory, kind, name, "");
        }
        write(
            &self.source.join("Documents/Заказ/Forms/ФормаДокумента.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form uuid="10000000-0000-4000-8000-000000000021"><Properties><Name>ФормаДокумента</Name><FormType>Managed</FormType></Properties></Form></MetaDataObject>"#,
        );
        write(
            &self
                .source
                .join("Documents/Заказ/Forms/ФормаДокумента/Ext/Form.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><ChildItems/></Form>"#,
        );
        write(
            &self
                .source
                .join("Documents/Заказ/Commands/ПровестиИЗакрыть/Ext/CommandModule.bsl"),
            "&AtClient\nProcedure CommandProcessing(CommandParameter, CommandExecuteParameters)\nEndProcedure\n",
        );
    }

    fn install_accepted_profile_sources(&self) {
        let configuration_path = self.source.join("Configuration.xml");
        let configuration = fs::read_to_string(&configuration_path).unwrap();
        fs::write(
            configuration_path,
            configuration.replace(
                "</ChildObjects>",
                "\n<Document>Заказ</Document>\n<Role>Кладовщик</Role>\n<Report>Продажи</Report>\n</ChildObjects>",
            ),
        )
        .unwrap();

        write(
            &self.source.join("Documents/Заказ.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema" version="2.20">
  <Document uuid="10000000-0000-4000-8000-000000000001">
    <Properties><Name>Заказ</Name></Properties>
    <ChildObjects>
      <Attribute><Properties><Name>Контрагент</Name><Type><v8:Type>xs:string</v8:Type></Type></Properties></Attribute>
      <TabularSection><Properties><Name>Товары</Name></Properties><ChildObjects>
        <Attribute><Properties><Name>Количество</Name><Type><v8:Type>xs:decimal</v8:Type></Type></Properties></Attribute>
      </ChildObjects></TabularSection>
      <Form>ФормаДокумента</Form>
    </ChildObjects>
  </Document>
</MetaDataObject>"#,
        );
        write(
            &self.source.join("Documents/Заказ/Forms/ФормаДокумента.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form uuid="10000000-0000-4000-8000-000000000002"><Properties><Name>ФормаДокумента</Name><FormType>Managed</FormType></Properties></Form></MetaDataObject>"#,
        );
        write(
            &self
                .source
                .join("Documents/Заказ/Forms/ФормаДокумента/Ext/Form.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema" version="2.20">
  <ChildItems>
    <InputField name="Склад" id="1"/>
    <Table name="Товары" id="2"><ChildItems><InputField name="Количество" id="3"><Events><Event name="Change">КоличествоИзменение</Event></Events></InputField></ChildItems></Table>
  </ChildItems>
  <Attributes><Attribute name="Объект" id="1"><Type><v8:Type>xs:string</v8:Type></Type></Attribute></Attributes>
  <Parameters><Parameter name="Режим"><Type><v8:Type>xs:string</v8:Type></Type></Parameter></Parameters>
  <Commands><Command name="Пересчитать" id="1"/></Commands>
</Form>"#,
        );

        write(
            &self.source.join("Roles/Кладовщик.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Role uuid="10000000-0000-4000-8000-000000000003"><Properties><Name>Кладовщик</Name></Properties></Role></MetaDataObject>"#,
        );
        write(
            &self.source.join("Roles/Кладовщик/Ext/Rights.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Rights xmlns="http://v8.1c.ru/8.2/roles" version="2.20"><object><name>Catalog.Товары</name><right><name>Read</name><value>true</value><restrictionByCondition><condition>Товары.Владелец = &amp;ТекущийПользователь</condition></restrictionByCondition></right></object></Rights>"#,
        );

        let report = fixture_text("unica_mcp_script_parity/form-remove/ParityReport.xml")
            .replace("ParityReport", "Продажи");
        write(
            &self.source.join("Reports/Продажи.xml"),
            &replace_child_objects(
                &report,
                "<Template>ОсновнаяСхема</Template><Template>Печать</Template>",
            ),
        );
        let schema_descriptor = fixture_text(
            "unica_mcp_script_parity/template-remove/ParityReport/Templates/MainSchema.xml",
        )
        .replace("MainSchema", "ОсновнаяСхема");
        write(
            &self
                .source
                .join("Reports/Продажи/Templates/ОсновнаяСхема.xml"),
            &schema_descriptor,
        );
        let dcs = fixture_text("unica_mcp_script_parity/dcs-validate/BadPrefix.xml")
            .replace("MainData", "Продажи")
            .replace("Code", "Сумма")
            .replace(
                "xmlns:bad=\"http://example.com\">bad:CatalogRef.X",
                ">xs:decimal",
            )
            .replace(
                "\n</DataCompositionSchema>",
                "\n<parameter><name>Период</name><valueType><v8:Type>xs:dateTime</v8:Type></valueType></parameter>\n</DataCompositionSchema>",
            );
        write(
            &self
                .source
                .join("Reports/Продажи/Templates/ОсновнаяСхема/Ext/Template.xml"),
            &dcs,
        );
        let print_descriptor = schema_descriptor
            .replace("ОсновнаяСхема", "Печать")
            .replace("DataCompositionSchema", "SpreadsheetDocument");
        write(
            &self.source.join("Reports/Продажи/Templates/Печать.xml"),
            &print_descriptor,
        );
        let mxl = fixture_text("platform_8_3_27/mxl/Template.xml")
            .replace(
                "\n\t<templateMode>true</templateMode>",
                "\n\t<namedItem xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"NamedItemCells\"><name>Шапка</name><area><type>Rows</type><beginRow>0</beginRow><endRow>0</endRow><beginColumn>-1</beginColumn><endColumn>-1</endColumn></area></namedItem>\n\t<templateMode>true</templateMode>",
            )
            .replace(
                "\n\t\t\t<empty>true</empty>",
                "\n\t\t\t<c><c><f>0</f><parameter>Заголовок</parameter></c></c>",
            );
        write(
            &self
                .source
                .join("Reports/Продажи/Templates/Печать/Ext/Template.xml"),
            &mxl,
        );
    }
}

fn fixture_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(relative)
}

fn fixture_text(relative: &str) -> String {
    fs::read_to_string(fixture_path(relative)).unwrap()
}

fn copy_fixture(relative: &str, destination: &Path) {
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::copy(fixture_path(relative), destination).unwrap();
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn replace_child_objects(document: &str, body: &str) -> String {
    let start = document.find("<ChildObjects>").unwrap();
    let end = document.find("</ChildObjects>").unwrap() + "</ChildObjects>".len();
    format!(
        "{}<ChildObjects>{body}</ChildObjects>{}",
        &document[..start],
        &document[end..]
    )
}

fn with_root_version(document: &str) -> String {
    let start = document.find("<MetaDataObject").unwrap();
    let end = document[start..].find('>').unwrap() + start;
    if document[start..end].contains("version=") {
        return document.to_string();
    }
    format!("{} version=\"2.20\"{}", &document[..end], &document[end..])
}

#[test]
fn cursor_retry_uses_saved_role_results_without_recanonicalizing() {
    let fixture = RealReaderFixture::new();
    let rights_path = fixture.source.join("Roles/SalesReader/Ext/Rights.xml");
    let rights = fs::read_to_string(&rights_path).unwrap().replacen(
        "</Rights>",
        "<object><name>Document.Orders</name><right><name>Read</name><value>true</value></right></object></Rights>",
        1,
    );
    fs::write(&rights_path, &rights).unwrap();
    let service = fixture.view_service();
    let first = service.view(
        ViewRequest::new("main:Role.SalesReader.Right")
            .unwrap()
            .with_limit(1)
            .unwrap(),
    );
    assert!(first.ok, "{:?}", first.diagnostics);
    let cursor = first.cursor.expect("two role objects require a cursor");
    fs::write(&rights_path, b"changed source is no longer valid XML").unwrap();
    let replay = service.view(
        ViewRequest::new("main:Role.SalesReader.Right")
            .unwrap()
            .with_limit(1)
            .unwrap()
            .with_cursor(cursor),
    );

    assert!(replay.ok, "saved cursor must remain usable: {replay:?}");
}

#[test]
pub(crate) fn configuration_level_rights_are_readable_role_objects() {
    let fixture = RealReaderFixture::new();
    let rights_path = fixture.source.join("Roles/SalesReader/Ext/Rights.xml");
    let rights = fs::read_to_string(&rights_path).unwrap().replacen(
        "</Rights>",
        "<object><name>Configuration.CorpusConfiguration</name><right><name>Administration</name><value>true</value></right><right><name>ThinClient</name><value>true</value></right><right><name>FutureConfigurationReadCapability</name><value>true</value></right><right><name>FutureConfigurationWriteCapability</name><value>false</value></right></object></Rights>",
        1,
    );
    fs::write(&rights_path, &rights).unwrap();
    let service = fixture.view_service();

    let role = service.view(ViewRequest::new("main:Role.SalesReader").unwrap());
    assert!(role.ok, "{:?}", role.diagnostics);
    let right = service.view(
        ViewRequest::new("main:Role.SalesReader.Right.Configuration_CorpusConfiguration").unwrap(),
    );
    assert!(right.ok, "{:?}", right.diagnostics);
    let data = right.data.as_ref().unwrap();
    assert_eq!(data["kind"], "Right");
    assert_eq!(data["props"]["objectKind"], "Configuration");
    assert_eq!(data["props"]["allowedCount"], 3);
    assert_eq!(data["props"]["deniedCount"], 1);
    assert_eq!(fs::read_to_string(&rights_path).unwrap(), rights);

    let authority = fixture.read_authority();
    let index = ReaderReach::new(vec![("main", &authority)]);
    let expected = "main:Role.SalesReader.Right.Configuration_CorpusConfiguration";
    let found = index.find(FindRequest::new(expected).unwrap());
    assert!(
        !found.is_nearest()
            && found
                .candidates()
                .iter()
                .any(|candidate| candidate.at() == expected),
        "configuration right is missing from find: {found:?}"
    );
}

#[test]
pub(crate) fn object_commands_are_registered_inline_without_descriptor_files() {
    let fixture = RealReaderFixture::new();
    let descriptor = fixture.source.join("Catalogs/Items.xml");
    let owner = fs::read_to_string(&descriptor).unwrap();
    // The owner's own `ChildObjects` closes last; nested tabular sections
    // close theirs earlier.
    let close = owner.rfind("</ChildObjects>").unwrap();
    let owner = format!(
        "{}{}{}",
        &owner[..close],
        r#"<Command uuid="10000000-0000-4000-8000-000000000079"><Properties><Name>Inline</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Инлайн</v8:content></v8:item></Synonym></Properties></Command>"#,
        &owner[close..]
    );
    fs::write(&descriptor, owner).unwrap();
    write(
        &fixture.source.join("Catalogs/Items/Commands/Inline/Ext/CommandModule.bsl"),
        "&AtClient\nProcedure CommandProcessing(CommandParameter, CommandExecuteParameters)\nEndProcedure\n",
    );
    let service = fixture.view_service();

    let owner = service.view(ViewRequest::new("main:Catalog.Items").unwrap());
    assert!(owner.ok, "{:?}", owner.diagnostics);
    // The shared fixture registers `Refresh` by text and the test adds an
    // inline definition; neither has a `Commands/<Name>.xml` on disk.
    for (at, title) in [
        ("main:Catalog.Items.Command.Refresh", "Refresh"),
        ("main:Catalog.Items.Command.Inline", "Инлайн"),
    ] {
        let command = service.view(ViewRequest::new(at).unwrap());
        assert!(command.ok, "{at}: {:?}", command.diagnostics);
        let data = command.data.as_ref().unwrap();
        assert_eq!(data["kind"], "Command", "{at}");
        assert_eq!(data["title"], title, "{at}");
    }

    let authority = fixture.read_authority();
    let index = ReaderReach::new(vec![("main", &authority)]);
    for expected in [
        "main:Catalog.Items.Command.Refresh",
        "main:Catalog.Items.Command.Inline",
    ] {
        let found = index.find(FindRequest::new(expected).unwrap());
        assert!(
            !found.is_nearest()
                && found
                    .candidates()
                    .iter()
                    .any(|candidate| candidate.at() == expected),
            "inline command identity is missing from find: {expected}: {found:?}"
        );
    }
}

#[test]
pub(crate) fn template_bodies_are_read_only_when_the_template_node_is_addressed() {
    let fixture = RealReaderFixture::new();
    let body = fixture
        .source
        .join("Reports/ParityReport/Templates/Print/Ext/Template.xml");
    let readable = fs::read(&body).unwrap();
    // An unreadable spreadsheet body must not take the owner or the template
    // collection down with it: only the template node opens the body.
    fs::write(
        &body,
        "<document xmlns=\"http://v8.1c.ru/8.2/data/spreadsheet\"><unclosed>",
    )
    .unwrap();
    let service = fixture.view_service();
    for at in [
        "main:Report.ParityReport",
        "main:Report.ParityReport.Template",
    ] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(result.ok, "{at}: {:?}", result.diagnostics);
    }
    let node = service.view(ViewRequest::new("main:Report.ParityReport.Template.Print").unwrap());
    assert!(
        !node.ok,
        "an unreadable template body escaped through its own node"
    );
    assert_eq!(node.diagnostics[0]["code"], "provider_unavailable");

    // A readable body still publishes its interior on the template node.
    fs::write(&body, readable).unwrap();
    let service = fixture.view_service();
    let node = service.view(ViewRequest::new("main:Report.ParityReport.Template.Print").unwrap());
    assert!(node.ok, "{:?}", node.diagnostics);
    let branches = node.data.as_ref().unwrap()["branches"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        branches
            .iter()
            .any(|branch| branch["at"] == "main:Report.ParityReport.Template.Print.Area"),
        "{branches:?}"
    );
}

#[test]
pub(crate) fn add_in_templates_stop_addressing_at_the_template_without_reading_the_payload() {
    let fixture = RealReaderFixture::new();
    let descriptor = fixture.source.join("Catalogs/Items.xml");
    let owner = fs::read_to_string(&descriptor).unwrap();
    let close = owner.rfind("</ChildObjects>").unwrap();
    let owner = format!(
        "{}<Template>Driver</Template>{}",
        &owner[..close],
        &owner[close..]
    );
    fs::write(&descriptor, owner).unwrap();
    write(
        &fixture.source.join("Catalogs/Items/Templates/Driver.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template uuid="10000000-0000-4000-8000-000000000081"><Properties><Name>Driver</Name><TemplateType>AddIn</TemplateType></Properties></Template></MetaDataObject>"#,
    );
    // The payload is an opaque archive; make it something no XML reader could parse.
    fs::create_dir_all(fixture.source.join("Catalogs/Items/Templates/Driver/Ext")).unwrap();
    fs::write(
        fixture
            .source
            .join("Catalogs/Items/Templates/Driver/Ext/Template.bin"),
        [0x50, 0x4b, 0x03, 0x04, 0xff, 0x00, 0x80],
    )
    .unwrap();
    let service = fixture.view_service();
    let node = service.view(ViewRequest::new("main:Catalog.Items.Template.Driver").unwrap());
    assert!(node.ok, "{:?}", node.diagnostics);
    let data = node.data.as_ref().unwrap();
    assert_eq!(data["kind"], "Template");
    assert!(
        data.get("branches").is_none(),
        "an add-in template has no addressable interior: {data}"
    );

    let authority = fixture.read_authority();
    let index = ReaderReach::new(vec![("main", &authority)]);
    let expected = "main:Catalog.Items.Template.Driver";
    let found = index.find(FindRequest::new(expected).unwrap());
    assert!(
        !found.is_nearest()
            && found
                .candidates()
                .iter()
                .any(|candidate| candidate.at() == expected),
        "add-in template identity is missing from find: {found:?}"
    );
}

#[test]
fn review_rejects_orphan_nested_module_owners_not_registered_by_parent() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture.source.join("Catalogs/Items/Forms/Orphan.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form uuid="10000000-0000-4000-8000-000000000077"><Properties><Name>Orphan</Name><FormType>Managed</FormType></Properties></Form></MetaDataObject>"#,
    );
    write(
        &fixture.source.join("Catalogs/Items/Commands/Orphan/Ext/CommandModule.bsl"),
        "&AtClient\nProcedure CommandProcessing(CommandParameter, CommandExecuteParameters)\nEndProcedure\n",
    );
    let service = fixture.view_service();

    for address in [
        "main:Catalog.Items.Form.Orphan.Module",
        "main:Catalog.Items.Command.Orphan.Module",
    ] {
        let result = service.view(ViewRequest::new(address).unwrap());
        assert!(
            !result.ok,
            "orphan owner was invented for {address}: {result:?}"
        );
        assert_eq!(result.diagnostics[0]["code"], "not_found");
    }
}

#[test]
fn review_rejects_direct_typed_owner_absent_from_configuration_inventory() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture.source.join("Roles/Orphan.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Role uuid="10000000-0000-4000-8000-000000000079"><Properties><Name>Orphan</Name></Properties></Role></MetaDataObject>"#,
    );
    write(
        &fixture.source.join("Roles/Orphan/Ext/Rights.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><Rights xmlns="http://v8.1c.ru/8.2/roles" version="2.20"/>"#,
    );
    let service = fixture.view_service();

    let result = service.view(ViewRequest::new("main:Role.Orphan").unwrap());
    assert!(
        !result.ok,
        "unregistered role entered the logical tree: {result:?}"
    );
    assert_eq!(result.diagnostics[0]["code"], "not_found");
}

#[test]
fn registered_physical_child_with_wrong_descriptor_fails_direct_and_parent_navigation() {
    let fixture = RealReaderFixture::new();
    let owner_path = fixture.source.join("Reports/ParityReport.xml");
    let owner = fs::read_to_string(&owner_path).unwrap().replacen(
        "</ChildObjects>",
        "<Form>Wrong</Form></ChildObjects>",
        1,
    );
    fs::write(owner_path, owner).unwrap();
    write(
        &fixture.source.join("Reports/ParityReport/Forms/Wrong.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Command><Properties><Name>Wrong</Name></Properties></Command></MetaDataObject>"#,
    );
    let service = fixture.view_service();

    for at in [
        "main:Report.ParityReport.Form.Wrong",
        "main:Report.ParityReport",
    ] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(!result.ok, "wrong child descriptor escaped through {at}");
        assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
    }
}

#[test]
fn registered_top_level_owner_without_descriptor_fails_kind_branch_and_direct_view() {
    let fixture = RealReaderFixture::new();
    let configuration = fixture.source.join("Configuration.xml");
    let owner = fs::read_to_string(&configuration).unwrap().replacen(
        "</ChildObjects>",
        "<Role>Missing</Role></ChildObjects>",
        1,
    );
    fs::write(configuration, owner).unwrap();
    let service = fixture.view_service();

    let root = service.view(ViewRequest::new("main:Configuration").unwrap());
    assert!(root.ok, "the root proves only Configuration.xml");
    for at in ["main:Role", "main:Role.Missing"] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(
            !result.ok,
            "missing top-level descriptor escaped through {at}"
        );
        assert_eq!(
            result.diagnostics[0]["code"], "provider_unavailable",
            "{at}"
        );
    }
}

#[test]
fn orphan_and_missing_physical_children_fail_closed_across_reader_families() {
    let orphan = RealReaderFixture::new();
    for (directory, kind, name) in [
        ("Forms", "Form", "OrphanForm"),
        ("Templates", "Template", "OrphanDcs"),
        ("Templates", "Template", "OrphanMxl"),
        ("Commands", "Command", "OrphanCommand"),
    ] {
        write(
            &orphan
                .source
                .join(format!("Reports/ParityReport/{directory}/{name}.xml")),
            &format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{kind}><Properties><Name>{name}</Name><TemplateType>DataCompositionSchema</TemplateType></Properties></{kind}></MetaDataObject>"#
            ),
        );
    }
    let service = orphan.view_service();
    for at in [
        "main:Report.ParityReport.Form.OrphanForm",
        "main:Report.ParityReport.Template.OrphanDcs.DataSet",
        "main:Report.ParityReport.Template.OrphanMxl.Area",
        "main:Report.ParityReport.Command.OrphanCommand",
    ] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(!result.ok, "orphan physical child escaped through {at}");
        assert_eq!(result.diagnostics[0]["code"], "not_found", "{at}");
    }

    let missing = RealReaderFixture::new();
    let owner_path = missing.source.join("Reports/ParityReport.xml");
    let owner = fs::read_to_string(&owner_path).unwrap().replacen(
        "</ChildObjects>",
        "<Form>MissingForm</Form><Template>MissingDcs</Template><Template>MissingMxl</Template></ChildObjects>",
        1,
    );
    fs::write(owner_path, owner).unwrap();
    let service = missing.view_service();
    for at in [
        "main:Report.ParityReport.Form.MissingForm",
        "main:Report.ParityReport.Template.MissingDcs.DataSet",
        "main:Report.ParityReport.Template.MissingMxl.Area",
    ] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(
            !result.ok,
            "missing registered descriptor escaped through {at}"
        );
        assert_eq!(
            result.diagnostics[0]["code"], "provider_unavailable",
            "{at}"
        );
    }
}

#[test]
fn unregistered_top_level_descriptors_cannot_enter_any_typed_reader_family() {
    let fixture = RealReaderFixture::new();
    write_metadata_owner(&fixture.source, "Catalogs", "Catalog", "Orphan", "");
    write_metadata_owner(&fixture.source, "Subsystems", "Subsystem", "Orphan", "");
    write_metadata_owner(&fixture.source, "XDTOPackages", "XDTOPackage", "Orphan", "");
    write_metadata_owner(&fixture.source, "CommonForms", "CommonForm", "Orphan", "");
    write(
        &fixture.source.join("CommonForms/Orphan/Ext/Form.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><ChildItems/></Form>"#,
    );
    let service = fixture.view_service();
    for at in [
        "main:Catalog.Orphan",
        "main:Catalog.Orphan.Module",
        "main:Subsystem.Orphan",
        "main:Subsystem.Orphan.Interface",
        "main:XDTOPackage.Orphan",
        "main:CommonForm.Orphan",
    ] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(
            !result.ok,
            "unregistered top-level owner escaped through {at}"
        );
        assert_eq!(result.diagnostics[0]["code"], "not_found", "{at}");
    }
}

#[test]
fn external_parent_childobjects_are_the_only_nested_owner_authority() {
    let fixture = RealExternalReaderFixture::new();
    write(
        &fixture.processor.join("Импорт/Forms/Orphan.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form><Properties><Name>Orphan</Name></Properties></Form></MetaDataObject>"#,
    );
    let authority = fixture.read_authority("artifact_processor", SourceSetKind::ExternalProcessor);
    let service = ViewService::new(authority, ViewCursorStore::default());
    let orphan = service.view(
        ViewRequest::new("artifact_processor:ExternalDataProcessor.Импорт.Form.Orphan").unwrap(),
    );
    assert!(!orphan.ok, "orphan external form became addressable");
    assert_eq!(orphan.diagnostics[0]["code"], "not_found");

    let descriptor = fixture.processor.join("Импорт.xml");
    let owner = fs::read_to_string(&descriptor).unwrap().replacen(
        "</ChildObjects>",
        "<Form>Missing</Form></ChildObjects>",
        1,
    );
    fs::write(descriptor, owner).unwrap();
    let authority = fixture.read_authority("artifact_processor", SourceSetKind::ExternalProcessor);
    let service = ViewService::new(authority, ViewCursorStore::default());
    let parent =
        service.view(ViewRequest::new("artifact_processor:ExternalDataProcessor.Импорт").unwrap());
    assert!(
        !parent.ok,
        "external parent published a missing registered child"
    );
    assert_eq!(parent.diagnostics[0]["code"], "provider_unavailable");
}

#[test]
fn review_revision_authority_cannot_be_swapped_after_named_identity_validation() {
    let fixture = RealReaderFixture::new();
    let replacement = fixture.root.path().join("replacement");
    fs::create_dir_all(&replacement).unwrap();
    fs::write(
        replacement.join("Configuration.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>ReplacementConfiguration</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
    )
    .unwrap();
    let saved = fixture.root.path().join("retained-a");
    let source_before = fixture.source.clone();
    let replacement_before = replacement.clone();
    let saved_before = saved.clone();
    let source_after = fixture.source.clone();
    let replacement_after = replacement;
    let saved_after = saved;
    review_set_revision_identity_hooks(
        move || {
            if saved_before.exists() {
                fs::rename(&source_before, &replacement_before).unwrap();
                fs::rename(&saved_before, &source_before).unwrap();
            }
        },
        move || {
            fs::rename(&source_after, &saved_after).unwrap();
            fs::rename(&replacement_after, &source_after).unwrap();
        },
    );

    let service = fixture.view_service();
    let result = service.view(ViewRequest::new("main:Configuration").unwrap());
    review_clear_revision_identity_hooks();

    assert!(
        !result.ok,
        "revision came from replacement B while retained bytes came from A: {result:?}"
    );
}

#[test]
fn review_role_canonical_encoding_cannot_collapse_distinct_kind_name_pairs() {
    let fixture = RealReaderFixture::new();
    fs::write(
        fixture.source.join("Roles/SalesReader/Ext/Rights.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Rights xmlns="http://v8.1c.ru/8.2/roles" version="2.20">
  <object><name>Catalog.Orders_X</name><right><name>Read</name><value>true</value></right></object>
  <object><name>Catalog_Orders.X</name><right><name>Read</name><value>true</value></right></object>
</Rights>"#,
    )
    .unwrap();
    let service = fixture.view_service();

    let result = service.view(ViewRequest::new("main:Role.SalesReader.Right").unwrap());
    assert!(!result.ok, "invalid platform kinds must fail closed");
    assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
    assert!(result.diagnostics[0]["message"]
        .as_str()
        .is_some_and(|message| message.contains("Catalog_Orders")));
}

#[test]
fn review_role_rejects_non_platform_metadata_node_kinds() {
    let fixture = RealReaderFixture::new();
    fs::write(
        fixture.source.join("Roles/SalesReader/Ext/Rights.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Rights xmlns="http://v8.1c.ru/8.2/roles" version="2.20">
  <object><name>WebSocketClient.Telephony</name><right><name>Read</name><value>true</value></right></object>
</Rights>"#,
    )
    .unwrap();
    let service = fixture.view_service();

    let result = service.view(ViewRequest::new("main:Role.SalesReader.Right").unwrap());
    assert!(
        !result.ok,
        "non-platform role object kind escaped: {result:?}"
    );
    assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
}

#[test]
fn review_production_read_port_has_no_nocancel_inventory_entrypoint() {
    let source = include_str!("../v13_read_port.rs");
    assert!(
        !source.contains("configuration_payload_with_checkpoint(&mut || Ok(()))"),
        "production-compiled read port retains a no-op cancellation bypass"
    );
}

#[test]
fn diagnostic_module_scope_preserves_exact_common_module_and_catalog_roles() {
    let fixture = RealReaderFixture::new();
    for (role, text) in [("Object", "// object\n"), ("Manager", "// manager\n")] {
        write(
            &fixture
                .source
                .join(format!("Catalogs/Items/Ext/{role}Module.bsl")),
            text,
        );
    }
    let authority = fixture.read_authority();
    for (at, relative) in [
        (
            "main:CommonModule.РеактивныйСервер",
            "CommonModules/РеактивныйСервер/Ext/Module.bsl",
        ),
        (
            "main:CommonModule.РеактивныйСервер.Body",
            "CommonModules/РеактивныйСервер/Ext/Module.bsl",
        ),
        (
            "main:Catalog.Items.Module.Object",
            "Catalogs/Items/Ext/ObjectModule.bsl",
        ),
        (
            "main:Catalog.Items.Module.Manager",
            "Catalogs/Items/Ext/ManagerModule.bsl",
        ),
    ] {
        let expected = fixture.source.join(relative);
        let before = fs::read(&expected).unwrap();
        let scope = authority
            .diagnostic_mapping(&QualifiedAddress::parse(at).unwrap(), &fixture.context)
            .map(|mapping| mapping.module_scope())
            .unwrap();
        assert_eq!(scope.source_set, "main", "{at}");
        assert_eq!(scope.source_root, fixture.source, "{at}");
        assert_eq!(scope.module_path, expected, "{at}");
        assert_eq!(fs::read(expected).unwrap(), before, "{at}");
    }
}

#[test]
fn diagnostic_module_scope_refuses_foreign_source_object_and_unregistered_module() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture.source.join("CommonModules/Orphan/Ext/Module.bsl"),
        "// not registered\n",
    );
    let authority = fixture.read_authority();
    for at in [
        "other:CommonModule.РеактивныйСервер",
        "main:Catalog.Items",
        "main:CommonModule.Orphan",
        "main:Catalog.Missing.Module.Manager",
    ] {
        assert!(
            authority
                .diagnostic_mapping(&QualifiedAddress::parse(at).unwrap(), &fixture.context)
                .map(|mapping| mapping.module_scope())
                .is_err(),
            "invented module scope for {at}"
        );
    }
}

#[test]
fn diagnostic_module_scope_keeps_admitted_source_after_project_remap() {
    let fixture = RealReaderFixture::new();
    let authority = fixture.read_authority();
    let at = QualifiedAddress::parse("main:CommonModule.РеактивныйСервер.Body").unwrap();
    let admitted = authority
        .diagnostic_mapping(&at, &fixture.context)
        .map(|mapping| mapping.module_scope())
        .unwrap();
    let replacement = fixture.root.path().join("replacement");
    write(
        &replacement.join("CommonModules/РеактивныйСервер/Ext/Module.bsl"),
        "// replacement only\n",
    );
    fs::write(fixture.root.path().join("v8project.yaml"), "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: replacement\n").unwrap();
    let retained = authority
        .diagnostic_mapping(&at, &fixture.context)
        .map(|mapping| mapping.module_scope())
        .unwrap();
    assert_eq!(retained.source_root, admitted.source_root);
    assert_eq!(retained.module_path, admitted.module_path);
    assert_ne!(retained.source_root, replacement);
}

#[test]
fn diagnostic_module_scope_refuses_replaced_retained_root() {
    if !supports_retained_root_replacement_test() {
        return;
    }
    let fixture = RealReaderFixture::new();
    let authority = fixture.operation_read_authority();
    let at = QualifiedAddress::parse("main:CommonModule.РеактивныйСервер").unwrap();
    assert!(authority
        .diagnostic_mapping(&at, &fixture.context)
        .map(|mapping| mapping.module_scope())
        .is_ok());
    fs::rename(&fixture.source, fixture.root.path().join("saved-source")).unwrap();
    fs::create_dir(&fixture.source).unwrap();
    assert!(authority
        .diagnostic_mapping(&at, &fixture.context)
        .map(|mapping| mapping.module_scope())
        .is_err());
}

#[test]
fn diagnostic_module_scope_refuses_module_replaced_by_foreign_symlink() {
    use crate::infrastructure::platform::testing::{
        create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };
    let fixture = RealReaderFixture::new();
    let authority = fixture.read_authority();
    let at = QualifiedAddress::parse("main:CommonModule.РеактивныйСервер").unwrap();
    let module = authority
        .diagnostic_mapping(&at, &fixture.context)
        .map(|mapping| mapping.module_scope())
        .unwrap()
        .module_path;
    let foreign = fixture.root.path().join("foreign.bsl");
    fs::write(&foreign, "// foreign module\n").unwrap();
    fs::remove_file(&module).unwrap();
    let outcome = create_file_link_fixture_for_test(&foreign, &module)
        .expect("unexpected file-link creation error must fail the fixture test");
    if outcome != FileLinkFixtureOutcome::Created {
        eprintln!("[SKIPPED FIXTURE] file-link fixture unavailable: {outcome:?}");
        return;
    }
    assert!(authority
        .diagnostic_mapping(&at, &fixture.context)
        .map(|mapping| mapping.module_scope())
        .is_err());
    assert_eq!(fs::read_to_string(foreign).unwrap(), "// foreign module\n");
}

mod canonical_module_diagnostics {
    use super::*;
    use crate::application::diagnostics::{DiagnosticCoordinator, DiagnosticMapping};
    use crate::domain::diagnostics::*;
    use crate::domain::source_location::SourceLocation;
    use crate::domain::source_target::TargetKind;
    use crate::infrastructure::diagnostics::CanonicalModuleDiagnosticMapping;
    use std::time::Duration;

    fn request(mapping: &CanonicalModuleDiagnosticMapping) -> DiagnosticRequest {
        DiagnosticRequest {
            action: DiagnosticAction::Analyze,
            source_set: mapping.target().source_set.clone(),
            metadata_path: mapping.target().metadata_path.clone(),
            filter: DiagnosticFilter::default(),
            range: None,
            limit: 200,
            timeout: Some(Duration::from_secs(5)),
        }
    }

    fn finding(path: &Path, code: &str) -> DiagnosticObservation {
        DiagnosticObservation::Diagnostic {
            provider: BSL_ANALYZER_PROVIDER,
            location: DiagnosticObservationLocation::Resource {
                handle: path.to_string_lossy().into_owned(),
            },
            focus: DiagnosticObservationFocus::Target,
            code: code.into(),
            severity: DiagnosticSeverity::Error,
            message: "fixture finding".into(),
            tags: Vec::new(),
        }
    }

    fn assert_mapping(
        authority: &LogicalViewReadAuthority<'_>,
        workspace: &WorkspaceContext,
        at: &str,
        expected: &str,
        physical: &Path,
    ) {
        let mapping = authority
            .diagnostic_mapping(&QualifiedAddress::parse(at).unwrap(), workspace)
            .unwrap_or_else(|error| panic!("{at}: {error:?}"));
        let target = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, expected).unwrap();
        assert_eq!(mapping.target().metadata_path.as_ref(), Some(&target));
        assert_eq!(mapping.target().target_kind, TargetKind::Module);
        let cancellation = CancellationToken::new();
        let context = mapping
            .resolve_context(&request(&mapping), workspace, &cancellation)
            .unwrap();
        let item = mapping
            .map_observation(finding(physical, "Requested"), &context, &cancellation)
            .unwrap();
        assert!(
            matches!(item, DiagnosticItem::Diagnostic {
            location: SourceLocation::Addressed { metadata_path: Some(path), target_kind: TargetKind::Module, .. }, ..
        } if path == target),
            "{at}: provider resource must map to proved module"
        );
    }

    #[test]
    fn external_object_form_and_body_use_proven_canonical_modules() {
        let fixture = RealExternalReaderFixture::new();
        for (set, kind, class, name, root) in [
            (
                "artifact_processor",
                SourceSetKind::ExternalProcessor,
                "ExternalDataProcessor",
                "Import",
                &fixture.processor,
            ),
            (
                "artifact_report",
                SourceSetKind::ExternalReport,
                "ExternalReport",
                "Sales",
                &fixture.report,
            ),
        ] {
            let form = root.join(format!("{name}/Forms/Main/Ext/Form/Module.bsl"));
            write(&form, "Procedure OnOpen()\nEndProcedure\n");
            let authority = fixture.operation_read_authority(set, kind);
            for tail in ["", ".Body"] {
                assert_mapping(
                    &authority,
                    &fixture.context,
                    &format!("{set}:{class}.{name}.Module.Object{tail}"),
                    &format!("{class}.{name}.ObjectModule"),
                    &root.join(format!("{name}/Ext/ObjectModule.bsl")),
                );
                assert_mapping(
                    &authority,
                    &fixture.context,
                    &format!("{set}:{class}.{name}.Form.Main.Module.Form{tail}"),
                    &format!("{class}.{name}.Form.Main.FormModule"),
                    &form,
                );
            }
        }
    }

    #[test]
    fn configuration_common_object_and_form_modules_keep_exact_diagnostic_targets() {
        let fixture = RealReaderFixture::new();
        write(
            &fixture.source.join("Catalogs/Items/Ext/ObjectModule.bsl"),
            "Procedure Run()\nEndProcedure\n",
        );
        write(
            &fixture.source.join("Catalogs/Items/Ext/ManagerModule.bsl"),
            "Procedure Manage()\nEndProcedure\n",
        );
        for authority in [
            fixture.operation_read_authority(),
            fixture.extension_read_authority(),
        ] {
            for (at, expected, relative) in [
                (
                    "CommonModule.РеактивныйСервер",
                    "CommonModule.РеактивныйСервер.Module",
                    "CommonModules/РеактивныйСервер/Ext/Module.bsl",
                ),
                (
                    "Catalog.Items.Module.Object",
                    "Catalog.Items.ObjectModule",
                    "Catalogs/Items/Ext/ObjectModule.bsl",
                ),
                (
                    "Catalog.Items.Module.Manager",
                    "Catalog.Items.ManagerModule",
                    "Catalogs/Items/Ext/ManagerModule.bsl",
                ),
                (
                    "Report.ParityReport.Form.MainForm.Module.Form",
                    "Report.ParityReport.Form.MainForm.FormModule",
                    "Reports/ParityReport/Forms/MainForm/Ext/Form/Module.bsl",
                ),
            ] {
                for tail in ["", ".Body"] {
                    assert_mapping(
                        &authority,
                        &fixture.context,
                        &format!("main:{at}{tail}"),
                        expected,
                        &fixture.source.join(relative),
                    );
                }
            }
        }
    }

    struct AnalysisFixture(DiagnosticProviderOutcome);

    impl DiagnosticProvider for AnalysisFixture {
        fn descriptor(&self) -> &'static DiagnosticProviderDescriptor {
            static DESCRIPTOR: DiagnosticProviderDescriptor = DiagnosticProviderDescriptor {
                id: BSL_ANALYZER_PROVIDER,
                actions: &[DiagnosticAction::Analyze],
                findings_target_kinds: &[TargetKind::Module],
                emits_focus_kinds: &[DiagnosticFocusKind::Target],
            };
            &DESCRIPTOR
        }
        fn execute(
            &self,
            request: &DiagnosticProviderRequest,
            context: &DiagnosticContext,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> DiagnosticProviderOutcome {
            assert_eq!(request.action, DiagnosticAction::Analyze);
            assert_eq!(context.target.target_kind, TargetKind::Module);
            self.0.clone()
        }
    }

    #[test]
    fn analyze_filters_neighbors_after_mapping_but_keeps_outside_and_incomplete_failures() {
        let fixture = RealExternalReaderFixture::new();
        let authority = fixture
            .operation_read_authority("artifact_processor", SourceSetKind::ExternalProcessor);
        let mapping = authority
            .diagnostic_mapping(
                &QualifiedAddress::parse(
                    "artifact_processor:ExternalDataProcessor.Import.Module.Object",
                )
                .unwrap(),
                &fixture.context,
            )
            .unwrap();
        let selected = fixture.processor.join("Import/Ext/ObjectModule.bsl");
        let neighbor = fixture.processor.join("Импорт/Ext/ObjectModule.bsl");
        for scenario in [
            "neighbor",
            "outside",
            "incomplete",
            "neighbor_only",
            "incomplete_empty",
        ] {
            let mut observations = vec![
                finding(&selected, "Selected"),
                finding(&neighbor, "Neighbor"),
            ];
            if scenario == "neighbor_only" {
                observations = vec![finding(&neighbor, "Neighbor")];
            } else if scenario == "incomplete_empty" {
                observations.clear();
            }
            let complete = !matches!(scenario, "incomplete" | "incomplete_empty");
            if scenario == "outside" {
                observations.push(finding(
                    &fixture.report.join("Sales/Ext/ObjectModule.bsl"),
                    "Outside",
                ));
            }
            let outcome = DiagnosticProviderOutcome {
                status: DiagnosticProviderStatus::Completed,
                complete,
                version: None,
                observations,
                rules: Vec::new(),
                readiness: None,
                error: None,
                suppressions: Vec::new(),
            };
            let registry =
                DiagnosticProviderRegistry::new(vec![Arc::new(AnalysisFixture(outcome))]).unwrap();
            let result = DiagnosticCoordinator::new(registry, &mapping)
                .execute(&request(&mapping), &fixture.context, &fixture.cancellation)
                .unwrap();
            if scenario == "outside" {
                assert!(!result.ok);
                assert!(!result.complete);
                assert_eq!(result.providers[0].status, DiagnosticProviderStatus::Failed);
                assert!(
                    result.items.is_empty(),
                    "outside resource must poison the whole provider before filtering"
                );
            } else {
                let has_selected = !matches!(scenario, "neighbor_only" | "incomplete_empty");
                assert_eq!(
                    result.items.len(),
                    usize::from(has_selected),
                    "{scenario}: only selected module findings may remain"
                );
                if has_selected {
                    assert!(
                        matches!(&result.items[0], DiagnosticItem::Diagnostic { code, .. } if code == "Selected")
                    );
                }
                assert_eq!(result.complete, complete);
                assert_eq!(
                    result.state,
                    if !complete {
                        DiagnosticResultState::Partial
                    } else {
                        DiagnosticResultState::Completed
                    }
                );
            }
        }
    }

    #[test]
    fn diagnostic_mapping_refuses_an_unregistered_form_even_with_a_module_file() {
        let fixture = RealExternalReaderFixture::new();
        write(
            &fixture.processor.join("Import/Forms/Orphan.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form><Properties><Name>Orphan</Name></Properties></Form></MetaDataObject>"#,
        );
        write(
            &fixture
                .processor
                .join("Import/Forms/Orphan/Ext/Form/Module.bsl"),
            "Procedure Run()\nEndProcedure\n",
        );
        let authority = fixture
            .operation_read_authority("artifact_processor", SourceSetKind::ExternalProcessor);
        let error = authority
            .diagnostic_mapping(
                &QualifiedAddress::parse(
                    "artifact_processor:ExternalDataProcessor.Import.Form.Orphan.Module.Form",
                )
                .unwrap(),
                &fixture.context,
            )
            .err()
            .expect("orphan form must not acquire diagnostic authority");
        assert_eq!(error.code(), RefusalCode::NotFound);
    }
    #[test]
    fn diagnostic_mapping_refuses_replacement_of_its_retained_source_root() {
        if !supports_retained_root_replacement_test() {
            return;
        }
        let fixture = RealReaderFixture::new();
        let authority = fixture.operation_read_authority();
        let saved = fixture.root.path().join("retained-a");
        fs::rename(&fixture.source, &saved).unwrap();
        fs::create_dir_all(&fixture.source).unwrap();
        fs::copy(
            saved.join("Configuration.xml"),
            fixture.source.join("Configuration.xml"),
        )
        .unwrap();
        let error = authority
            .diagnostic_mapping(
                &QualifiedAddress::parse("main:CommonModule.РеактивныйСервер").unwrap(),
                &fixture.context,
            )
            .err()
            .expect("changed named root must not acquire diagnostic authority");
        assert_eq!(error.code(), RefusalCode::ProviderUnavailable);
    }
}

#[test]
fn dcs_validation_input_keeps_the_authority_cancellation_and_deadline() {
    let fixture = RealReaderFixture::new();
    let address = QualifiedAddress::parse("main:Report.ParityReport.Template.MainSchema").unwrap();
    let mut authority = fixture.read_authority();
    authority.deadline = ProviderDeadline::from_budget(std::time::Duration::ZERO);
    assert_eq!(
        authority
            .dcs_validation_input(&address)
            .err()
            .unwrap()
            .code(),
        RefusalCode::DeadlineExceeded
    );
    let authority = fixture.read_authority();
    fixture.cancellation.cancel();
    assert_eq!(
        authority
            .dcs_validation_input(&address)
            .err()
            .unwrap()
            .code(),
        RefusalCode::Cancelled
    );
}

#[test]
fn canonical_dcs_validation_keeps_captured_format_evidence_after_replacement() {
    let fixture = RealReaderFixture::new();
    let address = QualifiedAddress::parse("main:Report.ParityReport.Template.MainSchema").unwrap();
    let authority = fixture.read_authority();
    let input = authority.dcs_validation_input(&address).unwrap();
    let wrapper = fixture
        .source
        .join("Reports/ParityReport/Templates/MainSchema.xml");
    let original = fs::read_to_string(&wrapper).unwrap();
    fs::write(&wrapper, original.replace("2.20", "2.21")).unwrap();
    fs::write(&input.artifact, "not XML: a later writer owns these bytes").unwrap();
    let checked = crate::application::v13::check::normalize_native_outcome(
        &address,
        "Template",
        crate::application::v13::check::CheckValidator::Dcs,
        crate::infrastructure::native_operations::v13_analysis::validate_dcs_input(input),
    )
    .unwrap();
    assert!(
        checked.diagnostics().iter().all(|diagnostic| !matches!(
            diagnostic.code(),
            "formatVersionInvalid" | "platformVersionUnsupported"
        )),
        "format diagnostics must describe the captured input: {:?}",
        checked.diagnostics()
    );
}

#[test]
fn canonical_dcs_validation_keeps_captured_owner_format_warnings() {
    for relative in ["Configuration.xml"] {
        for (version, expected) in [
            ("2.21", "platformVersionUnsupported"),
            ("2.19", "formatMigrationAvailable"),
            ("2.&#50;0", "formatVersionInvalid"),
        ] {
            let fixture = RealReaderFixture::new();
            let owner = fixture.source.join(relative);
            let original = fs::read_to_string(&owner).unwrap();
            fs::write(&owner, original.replace("2.20", version)).unwrap();
            let address =
                QualifiedAddress::parse("main:Report.ParityReport.Template.MainSchema").unwrap();
            let captured = fixture.read_authority().dcs_validation_input(&address);
            let input = captured.unwrap();
            fs::write(&owner, original).unwrap();
            let checked = crate::application::v13::check::normalize_native_outcome(
                &address,
                "Template",
                crate::application::v13::check::CheckValidator::Dcs,
                crate::infrastructure::native_operations::v13_analysis::validate_dcs_input(input),
            )
            .unwrap();
            assert_eq!(
                checked.diagnostics()[0].code(),
                expected,
                "{relative}: {version}"
            );
        }
    }

    let fixture = RealExternalReaderFixture::new();
    let template_owner = fixture_text(
        "unica_mcp_script_parity/template-remove/ParityReport/Templates/MainSchema.xml",
    );
    let dcs = fixture_text("unica_mcp_script_parity/dcs-validate/BadPrefix.xml").replace(
        "xmlns:bad=\"http://example.com\">bad:CatalogRef.X",
        ">xs:string",
    );
    for (source_set, kind, object_kind, name, source) in [
        (
            "artifact_report",
            SourceSetKind::ExternalReport,
            "ExternalReport",
            "Sales",
            &fixture.report,
        ),
        (
            "artifact_processor",
            SourceSetKind::ExternalProcessor,
            "ExternalDataProcessor",
            "Import",
            &fixture.processor,
        ),
    ] {
        let owner = source.join(format!("{name}.xml"));
        let original = fs::read_to_string(&owner).unwrap().replace(
            "</ChildObjects>",
            "<Template>MainSchema</Template></ChildObjects>",
        );
        write(
            &source.join(format!("{name}/Templates/MainSchema.xml")),
            &template_owner,
        );
        write(
            &source.join(format!("{name}/Templates/MainSchema/Ext/Template.xml")),
            &dcs,
        );
        // Only the external root has the older format; its Template stays 2.20.
        fs::write(
            &owner,
            original.replace("version=\"2.20\"", "version=\"2.19\""),
        )
        .unwrap();
        let address = QualifiedAddress::parse(&format!(
            "{source_set}:{object_kind}.{name}.Template.MainSchema"
        ))
        .unwrap();
        let input = fixture
            .read_authority(source_set, kind)
            .dcs_validation_input(&address)
            .unwrap();
        fs::write(&owner, &original).unwrap();
        let checked = crate::application::v13::check::normalize_native_outcome(
            &address,
            "Template",
            crate::application::v13::check::CheckValidator::Dcs,
            crate::infrastructure::native_operations::v13_analysis::validate_dcs_input(input),
        )
        .unwrap();
        assert_eq!(
            checked.diagnostics()[0].code(),
            "formatMigrationAvailable",
            "{source_set}: restored owner bytes must not replace captured format evidence"
        );
        assert_eq!(fs::read_to_string(&owner).unwrap(), original);
    }
}

#[test]
fn canonical_dcs_validation_does_not_follow_links_after_input_capture() {
    use crate::infrastructure::platform::testing::{
        create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };

    let fixture = RealReaderFixture::new();
    let address = QualifiedAddress::parse("main:Report.ParityReport.Template.MainSchema").unwrap();
    let input = fixture
        .read_authority()
        .dcs_validation_input(&address)
        .unwrap();
    let outside = fixture.root.path().join("foreign.xml");
    fs::write(&outside, "not XML: foreign input must never participate").unwrap();
    for path in [
        input.artifact.clone(),
        fixture.source.join("Configuration.xml"),
        fixture
            .source
            .join("Reports/ParityReport/Templates/MainSchema.xml"),
    ] {
        fs::remove_file(&path).unwrap();
        let outcome = create_file_link_fixture_for_test(&outside, &path)
            .expect("unexpected file-link creation error must fail the fixture test");
        if outcome != FileLinkFixtureOutcome::Created {
            eprintln!("[SKIPPED FIXTURE] file-link fixture unavailable: {outcome:?}");
            return;
        }
    }
    let checked = crate::application::v13::check::normalize_native_outcome(
        &address,
        "Template",
        crate::application::v13::check::CheckValidator::Dcs,
        crate::infrastructure::native_operations::v13_analysis::validate_dcs_input(input),
    )
    .unwrap();
    assert!(checked
        .diagnostics()
        .iter()
        .all(|diagnostic| diagnostic.code() != "formatVersionInvalid"));
    assert_eq!(
        fs::read_to_string(outside).unwrap(),
        "not XML: foreign input must never participate"
    );
}

#[test]
fn canonical_dcs_validation_refuses_linked_format_owners_before_capture() {
    use crate::infrastructure::platform::testing::{
        create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };

    for relative in [
        "Configuration.xml",
        "Reports/ParityReport/Templates/MainSchema.xml",
    ] {
        let fixture = RealReaderFixture::new();
        let owner = fixture.source.join(relative);
        let outside = fixture.root.path().join("foreign.xml");
        fs::rename(&owner, &outside).unwrap();
        let outcome = create_file_link_fixture_for_test(&outside, &owner)
            .expect("unexpected file-link creation error must fail the fixture test");
        if outcome != FileLinkFixtureOutcome::Created {
            eprintln!("[SKIPPED FIXTURE] file-link fixture unavailable: {outcome:?}");
            return;
        }
        let address =
            QualifiedAddress::parse("main:Report.ParityReport.Template.MainSchema").unwrap();
        assert!(
            fixture
                .read_authority()
                .dcs_validation_input(&address)
                .is_err(),
            "{relative}"
        );
    }
}

#[test]
fn canonical_dcs_validation_keeps_large_configuration_owner_format_warning() {
    let fixture = RealReaderFixture::new();
    let path = fixture.source.join("Configuration.xml");
    let xml = fs::read_to_string(&path).unwrap().replace("2.20", "2.21");
    fs::write(
        &path,
        xml.replace(
            "</MetaDataObject>",
            &format!("<!--{}--></MetaDataObject>", "x".repeat(8 * 1024 * 1024)),
        ),
    )
    .unwrap();
    let address = QualifiedAddress::parse("main:Report.ParityReport.Template.MainSchema").unwrap();
    let input = fixture
        .read_authority()
        .dcs_validation_input(&address)
        .unwrap();
    let checked = crate::application::v13::check::normalize_native_outcome(
        &address,
        "Template",
        crate::application::v13::check::CheckValidator::Dcs,
        crate::infrastructure::native_operations::v13_analysis::validate_dcs_input(input),
    )
    .unwrap();
    assert_eq!(
        checked.diagnostics()[0].code(),
        "platformVersionUnsupported"
    );
}

#[test]
fn canonical_dcs_validation_keeps_exact_versionless_dcs_root_guard() {
    let fixture = RealReaderFixture::new();
    let artifact = fixture
        .source
        .join("Reports/ParityReport/Templates/MainSchema/Ext/Template.xml");
    fs::write(&artifact, r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema" version="2.20"/>"#).unwrap();
    let address = QualifiedAddress::parse("main:Report.ParityReport.Template.MainSchema").unwrap();
    let input = fixture
        .read_authority()
        .dcs_validation_input(&address)
        .unwrap();
    let checked = crate::application::v13::check::normalize_native_outcome(
        &address,
        "Template",
        crate::application::v13::check::CheckValidator::Dcs,
        crate::infrastructure::native_operations::v13_analysis::validate_dcs_input(input),
    )
    .unwrap();
    assert!(
        checked
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == "formatVersionInvalid"),
        "versionless DCS publication policy must survive the retained-input bridge"
    );
}

#[test]
fn canonical_dcs_validation_reports_readable_template_owner_versions() {
    for (version, expected) in [
        (Some("2.19"), "formatMigrationAvailable"),
        (Some("2.21"), "platformVersionUnsupported"),
        (None, "formatMigrationAvailable"),
    ] {
        let fixture = RealReaderFixture::new();
        let owner = fixture
            .source
            .join("Reports/ParityReport/Templates/MainSchema.xml");
        let original = fs::read_to_string(&owner).unwrap();
        let xml = original.replace(
            "version=\"2.20\"",
            &version
                .map(|v| format!("version=\"{v}\""))
                .unwrap_or_default(),
        );
        fs::write(&owner, &xml).unwrap();
        let address =
            QualifiedAddress::parse("main:Report.ParityReport.Template.MainSchema").unwrap();
        let input = fixture
            .read_authority()
            .dcs_validation_input(&address)
            .unwrap();
        // Format evidence belongs to the captured input, not a later reread.
        fs::write(&owner, &original).unwrap();
        let checked = crate::application::v13::check::normalize_native_outcome(
            &address,
            "Template",
            crate::application::v13::check::CheckValidator::Dcs,
            crate::infrastructure::native_operations::v13_analysis::validate_dcs_input(input),
        )
        .unwrap();
        assert_eq!(checked.diagnostics()[0].code(), expected);
        assert_eq!(fs::read_to_string(&owner).unwrap(), original);
    }
}

#[test]
fn canonical_dcs_format_owner_read_stops_at_typed_midstream_checkpoint() {
    for code in [RefusalCode::Cancelled, RefusalCode::DeadlineExceeded] {
        let fixture = RealReaderFixture::new();
        let owner = fixture.source.join("Configuration.xml");
        let original = fs::read_to_string(&owner).unwrap();
        fs::write(
            &owner,
            original.replace(
                "</MetaDataObject>",
                &format!("<!--{}--></MetaDataObject>", "x".repeat(256 * 1024)),
            ),
        )
        .unwrap();
        let authority = fixture.read_authority();
        let target = crate::domain::source_target::MetadataAddress::parse(
            crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            "Report.ParityReport.Template.MainSchema",
        )
        .unwrap();
        let mut checkpoints = 0;
        let error = authority
            .read
            .dcs_validation_input(&target, &mut || {
                checkpoints += 1;
                if checkpoints == 2 {
                    Err(ViewError::new(
                        code,
                        "interrupted after the first owner chunk",
                    ))
                } else {
                    Ok(())
                }
            })
            .err()
            .unwrap();
        assert_eq!(error.code(), code);
        assert_eq!(checkpoints, 2, "read must stop at the rejecting checkpoint");
        assert!(
            authority
                .read
                .dcs_validation_input(&target, &mut || Ok(()))
                .is_ok(),
            "partial capture must not be published or cached"
        );
    }
}

fn external_leaf_descriptor(kind: &str, name: &str, comment_bytes: usize) -> String {
    format!(
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{kind} uuid="10000000-0000-4000-8000-000000000010"><Properties><Name>{name}</Name></Properties><ChildObjects/></{kind}><!--{}--></MetaDataObject>"#,
        "x".repeat(comment_bytes)
    )
}

fn external_leaf_fixture(kind: SourceSetKind) -> (tempfile::TempDir, PathBuf) {
    let workspace = tempfile::tempdir().unwrap();
    let source = workspace.path().join("external");
    fs::create_dir(&source).unwrap();
    let source_type = if kind == SourceSetKind::ExternalProcessor {
        "EXTERNAL_DATA_PROCESSORS"
    } else {
        "EXTERNAL_REPORTS"
    };
    fs::write(workspace.path().join("v8project.yaml"), format!("format: DESIGNER\nsource-set:\n  - name: artifact\n    type: {source_type}\n    path: external\n")).unwrap();
    let source = fs::canonicalize(source).unwrap();
    (workspace, source)
}

fn external_leaf_read(source: &Path, kind: SourceSetKind) -> ProviderReadAuthority {
    ProviderReadAuthority::new_local_snapshot(
        "artifact",
        "external-leaf-identity",
        kind,
        Arc::new(RetainedDirectoryCapability::open(source).unwrap()),
        "external-leaf-local-read".into(),
    )
}

#[test]
fn external_inventory_accepts_a_single_owner_descriptor_beyond_eight_mib() {
    for (kind, xml_kind) in [
        (SourceSetKind::ExternalProcessor, "ExternalDataProcessor"),
        (SourceSetKind::ExternalReport, "ExternalReport"),
    ] {
        let (_workspace, source) = external_leaf_fixture(kind);
        let original = external_leaf_descriptor(xml_kind, "Large", 9 * 1024 * 1024);
        fs::write(source.join("Large.xml"), &original).unwrap();
        let payload = configuration_payload(&external_leaf_read(&source, kind))
            .expect("valid external owner must not fail the old per-file cap");
        assert_eq!(
            payload["registeredObjects"],
            json!([{"kind":xml_kind,"name":"Large"}])
        );
        assert_eq!(payload["format"], "PlatformXml");
        assert_eq!(payload["name"], "artifact");
        assert_eq!(
            fs::read_to_string(source.join("Large.xml")).unwrap(),
            original
        );
    }
}

#[test]
fn named_external_owner_view_has_no_hidden_eight_mib_descriptor_fallback() {
    for (kind, xml_kind) in [
        (SourceSetKind::ExternalProcessor, "ExternalDataProcessor"),
        (SourceSetKind::ExternalReport, "ExternalReport"),
    ] {
        let (_workspace, source) = external_leaf_fixture(kind);
        fs::write(
            source.join("Large.xml"),
            external_leaf_descriptor(xml_kind, "Large", 9 * 1024 * 1024),
        )
        .unwrap();
        let cancellation = CancellationToken::new();
        let authority = LogicalViewReadAuthority::with_read_authority(
            &cancellation,
            external_leaf_read(&source, kind),
            PlatformProfile::v8_3_27(),
            ProviderDeadline::no_deadline(),
        );
        let at = format!("artifact:{xml_kind}.Large");
        let result = ViewService::new(authority, ViewCursorStore::default())
            .view(ViewRequest::new(&at).unwrap());
        assert!(result.ok, "large external named view refused: {result:?}");
        let node = result.data.unwrap();
        assert_eq!(node["at"], at);
        assert_eq!(node["kind"], xml_kind);
        assert_eq!(node["title"], "Large");
        assert_eq!(node["props"]["kind"], xml_kind);
    }
}

#[test]
fn external_inventory_retains_all_257_immediate_owners_and_the_last_named_view() {
    for (kind, xml_kind) in [
        (SourceSetKind::ExternalProcessor, "ExternalDataProcessor"),
        (SourceSetKind::ExternalReport, "ExternalReport"),
    ] {
        let (_workspace, source) = external_leaf_fixture(kind);
        let mut expected = Vec::new();
        for index in 0..257 {
            let name = format!("Owner{index:03}");
            fs::write(
                source.join(format!("{name}.xml")),
                external_leaf_descriptor(xml_kind, &name, 0),
            )
            .unwrap();
            expected.push(json!({"kind":xml_kind,"name":name}));
        }
        let payload = configuration_payload(&external_leaf_read(&source, kind))
            .expect("all immediate owner names must be inspected without a count quota");
        assert_eq!(
            payload["registeredObjects"],
            serde_json::Value::Array(expected)
        );
        let cancellation = CancellationToken::new();
        let authority = LogicalViewReadAuthority::with_read_authority(
            &cancellation,
            external_leaf_read(&source, kind),
            PlatformProfile::v8_3_27(),
            ProviderDeadline::no_deadline(),
        );
        let at = format!("artifact:{xml_kind}.Owner256");
        let viewed = ViewService::new(authority, ViewCursorStore::default())
            .view(ViewRequest::new(&at).unwrap());
        assert!(viewed.ok, "last real owner was lost: {viewed:?}");
        assert_eq!(viewed.data.as_ref().unwrap()["at"], at);
        assert_eq!(viewed.data.as_ref().unwrap()["title"], "Owner256");
    }
}

#[test]
fn external_inventory_does_not_limit_noise_before_a_valid_owner() {
    let (_workspace, source) = external_leaf_fixture(SourceSetKind::ExternalProcessor);
    for index in 0..257 {
        fs::write(source.join(format!("noise{index:03}.txt")), "unrelated").unwrap();
    }
    fs::write(
        source.join("Useful.xml"),
        external_leaf_descriptor("ExternalDataProcessor", "Useful", 0),
    )
    .unwrap();
    let payload = configuration_payload(&external_leaf_read(
        &source,
        SourceSetKind::ExternalProcessor,
    ))
    .unwrap();
    assert_eq!(
        payload["registeredObjects"],
        json!([{"kind":"ExternalDataProcessor","name":"Useful"}])
    );
}

#[test]
fn external_inventory_rejects_malformed_and_non_utf8_late_tails_without_partial_rows() {
    for invalid_utf8 in [false, true] {
        let (_workspace, source) = external_leaf_fixture(SourceSetKind::ExternalProcessor);
        fs::write(
            source.join("AGood.xml"),
            external_leaf_descriptor("ExternalDataProcessor", "AGood", 0),
        )
        .unwrap();
        let mut bytes =
            external_leaf_descriptor("ExternalDataProcessor", "ZLarge", 9 * 1024 * 1024)
                .into_bytes();
        if invalid_utf8 {
            bytes.push(0xff);
        } else {
            bytes.extend_from_slice(b"<unfinished>");
        }
        fs::write(source.join("ZLarge.xml"), &bytes).unwrap();
        let error = configuration_payload(&external_leaf_read(
            &source,
            SourceSetKind::ExternalProcessor,
        ))
        .unwrap_err();
        assert_eq!(error.code(), RefusalCode::ProviderUnavailable);
        assert!(
            !error.to_string().contains("read limit"),
            "must reach actual full parser beyond former cap: {error}"
        );
        let cancellation = CancellationToken::new();
        let authority = LogicalViewReadAuthority::with_read_authority(
            &cancellation,
            external_leaf_read(&source, SourceSetKind::ExternalProcessor),
            PlatformProfile::v8_3_27(),
            ProviderDeadline::no_deadline(),
        );
        let result = ViewService::new(authority, ViewCursorStore::default())
            .view(ViewRequest::new("artifact:Configuration").unwrap());
        assert!(!result.ok);
        assert!(
            result.data.is_none(),
            "malformed late descriptor cannot publish earlier partial rows"
        );
        assert_eq!(fs::read(source.join("ZLarge.xml")).unwrap(), bytes);
    }
}

#[test]
fn external_inventory_preserves_explicit_cancellation_during_a_large_owner_read() {
    let (_workspace, source) = external_leaf_fixture(SourceSetKind::ExternalProcessor);
    fs::write(
        source.join("Large.xml"),
        external_leaf_descriptor("ExternalDataProcessor", "Large", 9 * 1024 * 1024),
    )
    .unwrap();
    let cancellation = CancellationToken::new();
    let mut checks = 0;
    let error = external_leaf_read(&source, SourceSetKind::ExternalProcessor)
        .configuration_payload_with_checkpoint(&mut || {
            checks += 1;
            if checks == 16 {
                cancellation.cancel();
            }
            if cancellation.is_cancelled() {
                Err(ViewError::new(
                    RefusalCode::Cancelled,
                    "manual external cancellation",
                ))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(checks, 16);
    assert_eq!(error.code(), RefusalCode::Cancelled);
}

#[test]
fn named_external_view_rejects_replaced_retained_root_without_reading_ambient_owner() {
    if !supports_retained_root_replacement_test() {
        return;
    }
    let (_workspace, source) = external_leaf_fixture(SourceSetKind::ExternalProcessor);
    fs::write(
        source.join("Original.xml"),
        external_leaf_descriptor("ExternalDataProcessor", "Original", 0),
    )
    .unwrap();
    let read = external_leaf_read(&source, SourceSetKind::ExternalProcessor);
    let saved = source.with_file_name("retained-external");
    fs::rename(&source, &saved).unwrap();
    fs::create_dir(&source).unwrap();
    let foreign = external_leaf_descriptor("ExternalDataProcessor", "Foreign", 0);
    fs::write(source.join("Foreign.xml"), &foreign).unwrap();
    let cancellation = CancellationToken::new();
    let authority = LogicalViewReadAuthority::with_read_authority(
        &cancellation,
        read,
        PlatformProfile::v8_3_27(),
        ProviderDeadline::no_deadline(),
    );
    let result = ViewService::new(authority, ViewCursorStore::default())
        .view(ViewRequest::new("artifact:Configuration").unwrap());
    assert!(!result.ok);
    assert!(result.data.is_none());
    assert_eq!(
        fs::read_to_string(source.join("Foreign.xml")).unwrap(),
        foreign
    );
    assert!(saved.join("Original.xml").is_file());
}

#[test]
fn external_inventory_leaf_preserves_actor_admission_membership_finality() {
    let (workspace, source) = external_leaf_fixture(SourceSetKind::ExternalProcessor);
    fs::write(
        source.join("Useful.xml"),
        external_leaf_descriptor("ExternalDataProcessor", "Useful", 0),
    )
    .unwrap();
    for index in 0..257 {
        fs::write(source.join(format!("noise{index:03}.txt")), "unrelated").unwrap();
    }
    let admission =
        crate::infrastructure::source_selection_evidence::discover_project_source_admission(
            workspace.path(),
            &mut || Ok(()),
        )
        .unwrap();
    assert_eq!(admission.map().source_sets.len(), 1);
    assert_eq!(
        admission.map().source_sets[0].kind,
        SourceSetKind::ExternalProcessor
    );
    let before_format = admission.map().source_sets[0].source_format;
    let before_evidence = admission.map().source_sets[0].format_evidence.clone();
    let evidence = admission.into_evidence();
    evidence
        .validate(ProviderDeadline::no_deadline(), &CancellationToken::new())
        .unwrap();
    fs::write(
        source.join("noise-after-admission.txt"),
        "late unrelated member",
    )
    .unwrap();
    let current = crate::infrastructure::project_sources::discover_project_source_map_controlled(
        workspace.path(),
        &mut || Ok(()),
    )
    .unwrap();
    assert_eq!(current.source_sets[0].source_format, before_format);
    assert_eq!(current.source_sets[0].format_evidence, before_evidence);
    assert_eq!(
        evidence
            .validate(ProviderDeadline::no_deadline(), &CancellationToken::new())
            .unwrap_err()
            .kind(),
        crate::infrastructure::source_selection_evidence::SourceSelectionEvidenceErrorKind::Changed
    );
}

#[test]
fn named_external_descriptor_and_owner_evidence_preserve_chunk_cancellation() {
    for evidence_read in [false, true] {
        let (_workspace, source) = external_leaf_fixture(SourceSetKind::ExternalProcessor);
        fs::write(
            source.join("Large.xml"),
            external_leaf_descriptor("ExternalDataProcessor", "Large", 9 * 1024 * 1024),
        )
        .unwrap();
        let reader = external_leaf_read(&source, SourceSetKind::ExternalProcessor);
        assert_eq!(
            configuration_payload(&reader).unwrap()["registeredObjects"],
            json!([{"kind":"ExternalDataProcessor","name":"Large"}])
        );
        let target = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            "ExternalDataProcessor.Large",
        )
        .unwrap();
        let cancellation = CancellationToken::new();
        let mut checks = 0;
        let mut checkpoint = || {
            checks += 1;
            if checks == 16 {
                cancellation.cancel();
            }
            if cancellation.is_cancelled() {
                Err(ViewError::new(
                    RefusalCode::Cancelled,
                    "manual named owner cancellation",
                ))
            } else {
                Ok(())
            }
        };
        let error = if evidence_read {
            reader
                .metadata_owner_evidence(&target, &mut checkpoint)
                .expect_err("cancelled named proof cannot succeed")
        } else {
            reader
                .external_metadata_payload(&target, &mut checkpoint)
                .expect_err("cancelled named payload cannot succeed")
        };
        assert_eq!(checks, 16);
        assert_eq!(error.code(), RefusalCode::Cancelled);
    }
}

#[test]
fn named_external_descriptor_distinguishes_missing_from_unreadable_source() {
    for evidence_read in [false, true] {
        let (_workspace, source) = external_leaf_fixture(SourceSetKind::ExternalProcessor);
        let reader = external_leaf_read(&source, SourceSetKind::ExternalProcessor);
        let target = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            "ExternalDataProcessor.Missing",
        )
        .unwrap();
        let missing = if evidence_read {
            reader
                .metadata_owner_evidence(&target, &mut || Ok(()))
                .unwrap_err()
        } else {
            reader
                .external_metadata_payload(&target, &mut || Ok(()))
                .unwrap_err()
        };
        assert_eq!(missing.code(), RefusalCode::NotFound);
        assert_eq!(missing.detail(), None);
        assert_eq!(
            missing.to_string(),
            "metadata descriptor `Missing.xml` is absent"
        );
        assert_eq!(reader.metadata_descriptor_read_count(target.as_str()), 1);

        fs::create_dir(source.join("Missing.xml")).unwrap();
        let unreadable = if evidence_read {
            reader
                .metadata_owner_evidence(&target, &mut || Ok(()))
                .unwrap_err()
        } else {
            reader
                .external_metadata_payload(&target, &mut || Ok(()))
                .unwrap_err()
        };
        assert_eq!(
            unreadable.detail(),
            Some(crate::domain::refusal::RefusalDetail::SourceUnreadable)
        );
        assert_ne!(unreadable.code(), RefusalCode::NotFound);
        assert_eq!(reader.metadata_descriptor_read_count(target.as_str()), 2);
        assert!(source.join("Missing.xml").is_dir());
    }
}

#[test]
fn named_external_missing_descriptor_preserves_explicit_cancellation() {
    for evidence_read in [false, true] {
        let (_workspace, source) = external_leaf_fixture(SourceSetKind::ExternalProcessor);
        let reader = external_leaf_read(&source, SourceSetKind::ExternalProcessor);
        let target = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            "ExternalDataProcessor.Missing",
        )
        .unwrap();
        let expected = ViewError::new(RefusalCode::Cancelled, "manual missing owner cancellation");
        let mut checks = 0;
        let mut checkpoint = || {
            checks += 1;
            Err(expected.clone())
        };
        let error = if evidence_read {
            reader
                .metadata_owner_evidence(&target, &mut checkpoint)
                .unwrap_err()
        } else {
            reader
                .external_metadata_payload(&target, &mut checkpoint)
                .unwrap_err()
        };
        assert_eq!(error, expected);
        assert_eq!(checks, 1);
        assert!(!source.join("Missing.xml").exists());
    }
}

#[test]
fn external_leaf_preserves_an_explicit_finite_relative_read_limit() {
    let (_workspace, source) = external_leaf_fixture(SourceSetKind::ExternalProcessor);
    let bytes = external_leaf_descriptor("ExternalDataProcessor", "Large", 1024).into_bytes();
    fs::write(source.join("Large.xml"), &bytes).unwrap();
    let reader = external_leaf_read(&source, SourceSetKind::ExternalProcessor);
    let error = reader
        .read_relative(Path::new("Large.xml"), 128)
        .unwrap_err();
    assert_eq!(
        error.detail(),
        Some(crate::domain::refusal::RefusalDetail::SourceUnreadable)
    );
    assert!(error.to_string().contains("128-byte read limit"));
    assert_eq!(
        reader
            .read_relative(Path::new("Large.xml"), bytes.len())
            .unwrap(),
        bytes
    );
    assert_eq!(
        configuration_payload(&reader).unwrap()["registeredObjects"],
        json!([{"kind":"ExternalDataProcessor","name":"Large"}])
    );
}

#[test]
fn mxl_check_consumes_captured_body_without_reopening_its_display_path() {
    let fixture = RealReaderFixture::new();
    fs::copy(fixture_path("acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/F06Template/Ext/Template.xml"), fixture.source.join("Reports/ParityReport/Templates/Print/Ext/Template.xml")).unwrap();
    let at = QualifiedAddress::parse("main:Report.ParityReport.Template.Print").unwrap();
    let authority = fixture.read_authority();
    let input = authority.mxl_validation_input(&at).unwrap();
    fs::remove_file(&input.artifact).unwrap();
    let result = crate::infrastructure::native_operations::mxl::validate_mxl_input(
        &serde_json::Map::new(),
        &input.text,
        input.artifact,
    );
    assert!(result.ok, "{:?}", result.errors);
    assert!(authority.mxl_validation_input(&at).is_err());
    let schema = QualifiedAddress::parse("main:Report.ParityReport.Template.MainSchema").unwrap();
    assert!(authority.mxl_validation_input(&schema).is_err());
}

#[test]
fn retained_mxl_check_supports_configuration_extension_epf_and_erf_owners() {
    let body = fixture_text(
        "acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/F06Template/Ext/Template.xml",
    );
    let wrapper =
        fixture_text("acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/F06Template.xml")
            .replace("F06Template", "Print");
    let fixture = RealReaderFixture::new();
    write(
        &fixture
            .source
            .join("Reports/ParityReport/Templates/Print.xml"),
        &wrapper,
    );
    write(
        &fixture
            .source
            .join("Reports/ParityReport/Templates/Print/Ext/Template.xml"),
        &body,
    );
    for authority in [fixture.read_authority(), fixture.extension_read_authority()] {
        let at = QualifiedAddress::parse("main:Report.ParityReport.Template.Print").unwrap();
        let input = authority.mxl_validation_input(&at).unwrap();
        assert!(matches!(
            input.format_guard,
            crate::application::ports::FormatGuardCheck::Allow
        ));
        assert!(
            crate::infrastructure::native_operations::mxl::validate_mxl_input(
                &serde_json::Map::new(),
                &input.text,
                input.artifact
            )
            .ok
        );
    }
    let fixture = RealExternalReaderFixture::new();
    for (root, kind, source_set, owner) in [
        (
            &fixture.processor,
            SourceSetKind::ExternalProcessor,
            "epf",
            "ExternalDataProcessor.Import",
        ),
        (
            &fixture.report,
            SourceSetKind::ExternalReport,
            "erf",
            "ExternalReport.Sales",
        ),
    ] {
        let name = owner.split('.').nth(1).unwrap();
        let descriptor = root.join(format!("{name}.xml"));
        let xml = fs::read_to_string(&descriptor).unwrap().replace(
            "</ChildObjects>",
            "<Template>Print</Template></ChildObjects>",
        );
        write(&descriptor, &xml);
        write(&root.join(format!("{name}/Templates/Print.xml")), &wrapper);
        write(
            &root.join(format!("{name}/Templates/Print/Ext/Template.xml")),
            &body,
        );
        let authority = fixture.read_authority(source_set, kind);
        let at = QualifiedAddress::parse(&format!("{source_set}:{owner}.Template.Print")).unwrap();
        let input = authority.mxl_validation_input(&at).unwrap();
        assert!(matches!(
            input.format_guard,
            crate::application::ports::FormatGuardCheck::Allow
        ));
        assert!(
            crate::infrastructure::native_operations::mxl::validate_mxl_input(
                &serde_json::Map::new(),
                &input.text,
                input.artifact
            )
            .ok
        );
        let orphan =
            QualifiedAddress::parse(&format!("{source_set}:{owner}.Template.Orphan")).unwrap();
        assert!(authority.mxl_validation_input(&orphan).is_err());
    }
}

#[test]
fn retained_mxl_check_keeps_unbounded_body_read_and_format_warnings() {
    let fixture = RealReaderFixture::new();
    let body = fixture_text(
        "acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/F06Template/Ext/Template.xml",
    );
    let wrapper =
        fixture_text("acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/F06Template.xml")
            .replace("F06Template", "Print")
            .replace("version=\"2.20\"", "version=\"2.19\"");
    write(
        &fixture
            .source
            .join("Reports/ParityReport/Templates/Print.xml"),
        &wrapper,
    );
    let padded = body.replacen(
        "</document>",
        &format!("<!--{}--></document>", "x".repeat(8 * 1024 * 1024)),
        1,
    );
    write(
        &fixture
            .source
            .join("Reports/ParityReport/Templates/Print/Ext/Template.xml"),
        &padded,
    );
    let authority = fixture.read_authority();
    let at = QualifiedAddress::parse("main:Report.ParityReport.Template.Print").unwrap();
    let input = authority.mxl_validation_input(&at).unwrap();
    assert_eq!(input.text, padded);
    assert!(matches!(
        input.format_guard,
        crate::application::ports::FormatGuardCheck::Warn { .. }
    ));
    assert!(
        crate::infrastructure::native_operations::mxl::validate_mxl_input(
            &serde_json::Map::new(),
            &input.text,
            input.artifact
        )
        .ok
    );
    fixture.cancellation.cancel();
    assert!(authority.mxl_validation_input(&at).is_err());
}

/// Описание из выгрузки платформы, сверенное с `manifest.json` опыта: тест
/// читает ровно ту форму, которую выгрузила платформа.
fn platform_borrow_fixture(name: &str) -> String {
    use sha2::{Digest, Sha256};
    let manifest: serde_json::Value =
        serde_json::from_str(&fixture_text("platform_8_3_27/cfe_borrow/manifest.json")).unwrap();
    let text = fixture_text(&format!("platform_8_3_27/cfe_borrow/{name}"));
    let digest = Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(
        manifest["files"][name]["sha256"].as_str(),
        Some(digest.as_str()),
        "{name} differs from the platform export recorded in the manifest"
    );
    text
}

/// Дерево из выгрузки опыта #1381: роль, общая форма, обработка с формой и
/// двумя макетами и пакет XDTO заимствованы без своих файлов содержимого.
fn install_inherited_objects(fixture: &RealReaderFixture) {
    let configuration = fs::read_to_string(fixture.source.join("Configuration.xml")).unwrap();
    write(
        &fixture.source.join("Configuration.xml"),
        &configuration.replace(
            "</ChildObjects>",
            "<CommonForm>ОбщаяФорма</CommonForm><DataProcessor>Обработка</DataProcessor><XDTOPackage>ПакетОбмена</XDTOPackage></ChildObjects>",
        ),
    );
    fs::remove_dir_all(fixture.source.join("Roles/SalesReader")).unwrap();
    for (relative, name) in [
        ("Roles/SalesReader.xml", "extension-role-inherited.xml"),
        (
            "CommonForms/ОбщаяФорма.xml",
            "extension-common-form-inherited.xml",
        ),
        (
            "DataProcessors/Обработка.xml",
            "extension-data-processor.xml",
        ),
        (
            "DataProcessors/Обработка/Forms/Форма.xml",
            "extension-object-form-inherited.xml",
        ),
        (
            "DataProcessors/Обработка/Templates/Макет.xml",
            "extension-template-spreadsheet-inherited.xml",
        ),
        (
            "DataProcessors/Обработка/Templates/СхемаКомпоновки.xml",
            "extension-template-schema-inherited.xml",
        ),
        (
            "XDTOPackages/ПакетОбмена.xml",
            "extension-xdto-package-inherited.xml",
        ),
    ] {
        write(
            &fixture.source.join(relative),
            &platform_borrow_fixture(name),
        );
    }
}

/// #1381: платформа 8.3.27 выгружает заимствованные роль, формы, макеты и
/// пакет XDTO без `Rights.xml`, `Form.xml`, `Template.xml` и `Package.bin`.
/// Такой узел читается по описанию: заимствование, `content: "inherited"`,
/// ни счёта, ни ветвей содержимого. Адрес внутри содержимого отказывает
/// `not_found`, а не `source_unreadable`.
#[test]
fn borrowed_objects_without_companion_files_read_their_content_as_inherited() {
    let fixture = RealReaderFixture::new();
    install_inherited_objects(&fixture);
    let service = fixture.extension_view_service();
    for (at, kind) in [
        ("main:Role.SalesReader", "Role"),
        ("main:CommonForm.ОбщаяФорма", "Form"),
        ("main:DataProcessor.Обработка.Form.Форма", "Form"),
        ("main:XDTOPackage.ПакетОбмена", "XDTOPackage"),
    ] {
        let view = service.view(ViewRequest::new(at).unwrap());
        assert!(view.ok, "{at}: an inherited node must be readable");
        let data = view.data.unwrap();
        assert_eq!(data["kind"], kind, "{at}");
        assert_eq!(data["props"]["content"], "inherited", "{at}");
        assert_eq!(data["props"]["belonging"], "borrowed", "{at}");
        assert!(
            data["props"]["parentId"].is_string(),
            "{at}: the parent stays addressable"
        );
        if kind == "Form" {
            assert_eq!(
                data["branches"],
                json!([{"at":format!("{at}.Metadata"),"count":1}]),
                "{at}: only the local declaration is reachable, no inherited content or Module"
            );
        } else {
            assert!(
                data.get("branches").is_none(),
                "{at}: no inherited content branches"
            );
        }
    }
    assert_eq!(
        service
            .view(ViewRequest::new("main:Role.SalesReader").unwrap())
            .data
            .unwrap()["props"]["overrides"],
        "Rights"
    );
    for at in [
        "main:DataProcessor.Обработка.Template.Макет",
        "main:DataProcessor.Обработка.Template.СхемаКомпоновки",
    ] {
        let view = service.view(ViewRequest::new(at).unwrap());
        assert!(view.ok, "{at}: an inherited template must be readable");
        let data = view.data.unwrap();
        // The extension descriptor may carry the platform's default
        // TemplateType instead of the parent's: the format stays unknown.
        assert_eq!(data["props"]["format"], serde_json::Value::Null, "{at}");
        assert_eq!(data["props"]["content"], "inherited", "{at}");
        assert_eq!(data["props"]["belonging"], "borrowed", "{at}");
        assert!(data["props"]["parentId"].is_string(), "{at}");
        assert!(data.get("branches").is_none(), "{at}");
    }
    for (at, resource, owner) in [
        (
            "main:Role.SalesReader.Right",
            "Rights.xml",
            "main:Role.SalesReader",
        ),
        (
            "main:CommonForm.ОбщаяФорма.Item",
            "Form.xml",
            "main:CommonForm.ОбщаяФорма",
        ),
        (
            "main:CommonForm.ОбщаяФорма.Module",
            "Form.xml",
            "main:CommonForm.ОбщаяФорма",
        ),
        (
            "main:CommonForm.ОбщаяФорма.Module.Form",
            "Form.xml",
            "main:CommonForm.ОбщаяФорма",
        ),
        (
            "main:DataProcessor.Обработка.Form.Форма.Attribute",
            "Form.xml",
            "main:DataProcessor.Обработка",
        ),
        (
            "main:DataProcessor.Обработка.Form.Форма.Module",
            "Form.xml",
            "main:DataProcessor.Обработка",
        ),
        (
            "main:DataProcessor.Обработка.Template.СхемаКомпоновки.DataSet",
            "Template.xml",
            "main:DataProcessor.Обработка",
        ),
        (
            "main:DataProcessor.Обработка.Template.Макет.Area",
            "Template.xml",
            "main:DataProcessor.Обработка",
        ),
        (
            "main:XDTOPackage.ПакетОбмена.Type",
            "Package.bin",
            "main:XDTOPackage.ПакетОбмена",
        ),
    ] {
        let view = service.view(ViewRequest::new(at).unwrap());
        assert!(!view.ok, "{at}: inherited content has no interior here");
        assert_eq!(view.diagnostics[0]["code"], "not_found", "{at}");
        let way = format!("props.extends of `{owner}`");
        assert!(
            view.diagnostics[0]["message"]
                .as_str()
                .is_some_and(|message| message.contains(resource) && message.contains(&way)),
            "{at}: the refusal names the absent file and the extension object naming the parent"
        );
    }
    let owner = service.view(ViewRequest::new("main:DataProcessor.Обработка").unwrap());
    assert!(owner.ok, "the borrowed owner of inherited children reads");

    // Source location of a method reads the module projection directly, not
    // through the node reader: the form without `Form.xml` refuses it the same way.
    let authority = fixture.extension_read_authority();
    let method =
        QualifiedAddress::parse("main:CommonForm.ОбщаяФорма.Module.Form.Method.ПР_А").unwrap();
    let snapshot = authority.snapshot(&method).unwrap();
    let error = authority
        .source_lines(&method, &snapshot)
        .expect_err("a form module the platform does not keep has no lines");
    assert_eq!(error.code(), RefusalCode::NotFound);
    assert!(error.to_string().contains("Form.xml"));
}

/// #1381: послабление касается только заимствованного объекта расширения.
/// В конфигурации, у собственного объекта расширения и у заимствованного
/// объекта со своим файлом отсутствие файла остаётся ошибкой источника, а
/// присутствующий файл читается как прежде.
#[test]
fn missing_companion_file_stays_a_source_error_outside_borrowing() {
    let fixture = RealReaderFixture::new();
    install_inherited_objects(&fixture);
    let configuration = fixture.view_service();
    for at in [
        "main:Role.SalesReader",
        "main:CommonForm.ОбщаяФорма",
        "main:XDTOPackage.ПакетОбмена",
        "main:DataProcessor.Обработка.Template.Макет",
    ] {
        let view = configuration.view(ViewRequest::new(at).unwrap());
        assert!(!view.ok, "{at}: a configuration object needs its file");
        assert_eq!(
            view.diagnostics[0]["detailCode"], "source_unreadable",
            "{at}"
        );
    }

    for (descriptor, at) in [
        ("Roles/SalesReader.xml", "main:Role.SalesReader"),
        ("CommonForms/ОбщаяФорма.xml", "main:CommonForm.ОбщаяФорма"),
        (
            "XDTOPackages/ПакетОбмена.xml",
            "main:XDTOPackage.ПакетОбмена",
        ),
        (
            "DataProcessors/Обработка/Templates/Макет.xml",
            "main:DataProcessor.Обработка.Template.Макет",
        ),
    ] {
        let own = RealReaderFixture::new();
        install_inherited_objects(&own);
        let path = own.source.join(descriptor);
        let text = fs::read_to_string(&path).unwrap();
        let start = text.find("<ExtendedConfigurationObject>").unwrap();
        let end = text.find("</ExtendedConfigurationObject>").unwrap()
            + "</ExtendedConfigurationObject>".len();
        write(&path, &format!("{}{}", &text[..start], &text[end..]));
        let view = own
            .extension_view_service()
            .view(ViewRequest::new(at).unwrap());
        assert!(!view.ok, "{at}: an own extension object needs its file");
        assert_eq!(
            view.diagnostics[0]["detailCode"], "source_unreadable",
            "{at}"
        );
    }

    // Заимствованный ребёнок собственного владельца ничего не наследует:
    // у владельца нет родителя.
    let own_owner = RealReaderFixture::new();
    install_inherited_objects(&own_owner);
    let path = own_owner.source.join("DataProcessors/Обработка.xml");
    let text = fs::read_to_string(&path).unwrap();
    let start = text.find("<ExtendedConfigurationObject>").unwrap();
    let end = text.find("</ExtendedConfigurationObject>").unwrap()
        + "</ExtendedConfigurationObject>".len();
    write(&path, &format!("{}{}", &text[..start], &text[end..]));
    for at in [
        "main:DataProcessor.Обработка.Form.Форма",
        "main:DataProcessor.Обработка.Template.Макет",
    ] {
        let view = own_owner
            .extension_view_service()
            .view(ViewRequest::new(at).unwrap());
        assert!(!view.ok, "{at}: a child of an own owner needs its file");
        assert_eq!(
            view.diagnostics[0]["detailCode"], "source_unreadable",
            "{at}"
        );
    }

    // Маркер без UUID родителя заимствования не доказывает.
    let marked = RealReaderFixture::new();
    install_inherited_objects(&marked);
    let path = marked.source.join("Roles/SalesReader.xml");
    let text = fs::read_to_string(&path).unwrap();
    let start =
        text.find("<ExtendedConfigurationObject>").unwrap() + "<ExtendedConfigurationObject>".len();
    let end = text.find("</ExtendedConfigurationObject>").unwrap();
    write(&path, &format!("{}parent{}", &text[..start], &text[end..]));
    let view = marked
        .extension_view_service()
        .view(ViewRequest::new("main:Role.SalesReader").unwrap());
    assert!(
        !view.ok,
        "a marker without a parent UUID proves no borrowing"
    );
    assert_eq!(view.diagnostics[0]["detailCode"], "source_unreadable");

    copy_fixture(
        "acceptance/workspace-borrowed/src/cf/Roles/SalesReader/Ext/Rights.xml",
        &fixture.source.join("Roles/SalesReader/Ext/Rights.xml"),
    );
    let view = fixture
        .extension_view_service()
        .view(ViewRequest::new("main:Role.SalesReader").unwrap());
    assert!(view.ok, "a borrowed role with its own rights reads them");
    let data = view.data.unwrap();
    assert!(data["props"].get("content").is_none());
    assert_eq!(data["props"]["totalallowed"], 0);
}

#[test]
fn platform_borrowed_common_module_preserves_unknown_contexts_and_independent_facts() {
    let fixture = RealReaderFixture::new();
    let configuration = fs::read_to_string(fixture.source.join("Configuration.xml")).unwrap();
    write(
        &fixture.source.join("Configuration.xml"),
        &configuration.replace(
            "</ChildObjects>",
            "<CommonModule>CorpusModule</CommonModule></ChildObjects>",
        ),
    );
    let descriptor =
        fixture_text("platform_8_3_27/incomplete_borrowed/extension-common-module.xml");
    let path = fixture.source.join("CommonModules/CorpusModule.xml");
    write(&path, &descriptor);
    let module = fixture
        .source
        .join("CommonModules/CorpusModule/Ext/Module.bsl");
    let source = "#Если Сервер Тогда\nПроцедура Проверка() Экспорт\nКонецПроцедуры\n#КонецЕсли\n";
    write(&module, source);
    let service = fixture.extension_view_service();
    let view = service.view(ViewRequest::new("main:CommonModule.CorpusModule").unwrap());
    assert!(view.ok, "view must succeed");
    let data = view.data.unwrap();
    let props = data["props"]["commonModule"].as_object().unwrap();
    assert_eq!(props.len(), 8);
    assert!(props.values().all(serde_json::Value::is_null));
    for at in [
        "main:CommonModule.CorpusModule.Method",
        "main:CommonModule.CorpusModule.Method.Проверка",
        "main:CommonModule.CorpusModule.Method.Проверка.Body",
        "main:CommonModule.CorpusModule.Body",
    ] {
        let view = service.view(ViewRequest::new(at).unwrap());
        assert!(view.ok, "{at}: view must succeed");
    }
    let method = service
        .view(ViewRequest::new("main:CommonModule.CorpusModule.Method.Проверка").unwrap())
        .data
        .unwrap();
    assert_eq!(
        method["props"]["compile"]["contexts"],
        serde_json::Value::Null
    );
    assert_eq!(method["props"]["compile"]["guard"], "Server");
    assert_eq!(method["props"]["export"], true);
    let compilation = service
        .view(ViewRequest::new("main:CommonModule.CorpusModule.Compilation").unwrap())
        .data
        .unwrap();
    assert_eq!(compilation["items"][0]["contexts"], serde_json::Value::Null);
    assert_eq!(fs::read_to_string(&path).unwrap(), descriptor);
    assert_eq!(fs::read_to_string(&module).unwrap(), source);

    // A synthetic unknown descriptive property cannot prevent BSL exploration.
    write(
        &path,
        &descriptor.replace(
            "</Properties>",
            "<FutureDescription>Observed</FutureDescription></Properties>",
        ),
    );
    assert!(
        fixture
            .extension_view_service()
            .view(ViewRequest::new("main:CommonModule.CorpusModule.Method.Проверка").unwrap())
            .ok
    );

    // Synthetic damage must not turn into permission to omit module properties.
    for (name, valid) in [
        (
            "ExtendedConfigurationObject",
            "ac847dc9-e222-45cf-af4a-6fa863c919a8",
        ),
        ("ObjectBelonging", "Adopted"),
    ] {
        let original = format!("<{name}>{valid}</{name}>");
        assert!(descriptor.contains(&original));
        for replacement in [
            format!("{original}<{name}>broken</{name}>"),
            format!("<{name}>broken</{name}>{original}"),
            format!("<foreign:{name} xmlns:foreign=\"urn:foreign\">{valid}</foreign:{name}>"),
            format!("<{name} unexpected=\"true\">{valid}</{name}>"),
            format!("<{name}>{valid}<Nested/></{name}>"),
            format!("<{name}>broken</{name}>"),
        ] {
            let damaged = descriptor.replace(&original, &replacement);
            write(&path, &damaged);
            let service = fixture.extension_view_service();
            for at in [
                "main:CommonModule.CorpusModule",
                "main:CommonModule.CorpusModule.Method",
                "main:CommonModule.CorpusModule.Method.Проверка",
            ] {
                let result = service.view(ViewRequest::new(at).unwrap());
                assert!(
                    !result.ok,
                    "{replacement}: {at}: damaged identity must be refused"
                );
                assert_eq!(result.diagnostics[0]["detailCode"], "source_unreadable");
                assert!(result.diagnostics[0]["message"]
                    .as_str()
                    .unwrap()
                    .contains(name));
            }
            assert_eq!(fs::read_to_string(&path).unwrap(), damaged);
            assert_eq!(fs::read_to_string(&module).unwrap(), source);
        }
    }
}

#[test]
fn common_module_absence_policy_keeps_ordinary_properties_required() {
    let ordinary = fixture_text("platform_8_3_27/incomplete_borrowed/parent-common-module.xml");
    let borrowed = fixture_text("platform_8_3_27/incomplete_borrowed/extension-common-module.xml");
    for name in [
        "Global",
        "ClientManagedApplication",
        "Server",
        "ExternalConnection",
        "ClientOrdinaryApplication",
        "ServerCall",
        "Privileged",
        "ReturnValuesReuse",
    ] {
        let doc = roxmltree::Document::parse(ordinary.trim_start_matches('\u{feff}')).unwrap();
        let node = doc
            .descendants()
            .find(|node| node.has_tag_name(name))
            .unwrap();
        let omitted = ordinary.replace(&doc.input_text()[node.range()], "");
        assert!(
            super::common_module_properties(omitted.as_bytes(), SourceSetKind::Configuration)
                .is_err(),
            "{name}"
        );
        let raw = if name == "ReturnValuesReuse" {
            "DuringSession"
        } else {
            "false"
        };
        let explicit = borrowed.replace(
            "</Properties>",
            &format!("<{name}>{raw}</{name}></Properties>"),
        );
        let observed = serde_json::to_value(
            super::common_module_properties(explicit.as_bytes(), SourceSetKind::Extension).unwrap(),
        )
        .unwrap();
        assert_eq!(
            observed
                .as_object()
                .unwrap()
                .values()
                .filter(|value| !value.is_null())
                .count(),
            1
        );
        let invalid = borrowed.replace(
            "</Properties>",
            &format!("<{name}>broken</{name}></Properties>"),
        );
        assert!(
            super::common_module_properties(invalid.as_bytes(), SourceSetKind::Extension).is_err(),
            "{name}"
        );
    }
}

#[test]
fn missing_or_damaged_code_type_limits_only_dependent_predefined_read() {
    // Synthetic combinations exercise the reader; platform acceptance of
    // borrowed predefined data without CodeType has not been established.
    for replacement in [
        "",
        "<CodeType/>",
        "<CodeType>broken</CodeType>",
        "<CodeType>String</CodeType><CodeType>Number</CodeType>",
    ] {
        let fixture = RealReaderFixture::new();
        let path = fixture.source.join("Catalogs/Items.xml");
        let original = fs::read_to_string(&path).unwrap();
        assert!(original.contains("<CodeType>String</CodeType>"));
        let descriptor = original.replace("<CodeType>String</CodeType>", replacement);
        write(&path, &descriptor);
        let service = fixture.view_service();
        let result = service.view(ViewRequest::new("main:Catalog.Items").unwrap());
        assert!(result.ok, "the read must succeed");
        let data = result.data.unwrap();
        assert_eq!(data["props"]["CodeLength"], 11);
        assert!(data["limits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|limit| limit.as_str().unwrap().contains("predefinedItems")));
        assert_eq!(
            data["limits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|limit| limit.as_str().unwrap().contains("properties.CodeType")),
            !replacement.is_empty()
        );
        let dependent =
            service.view(ViewRequest::new("main:Catalog.Items.PredefinedItem").unwrap());
        assert!(!dependent.ok, "the dependent read must be refused");
        assert_eq!(fs::read_to_string(path).unwrap(), descriptor);
    }
}

#[test]
fn missing_relation_preserves_known_targets_and_reports_incompleteness() {
    // Synthetic relation variants isolate reader behavior; the raw platform
    // borrowed catalog separately proves that these containers may be absent.
    for (owners, expected_unknown) in [
        ("", true),
        ("<Owners/>", false),
        ("<Owners><Broken/></Owners>", true),
        ("<Owners xmlns=\"urn:foreign\"/>", true),
    ] {
        let fixture = RealReaderFixture::new();
        let path = fixture.source.join("Catalogs/Items.xml");
        let xml = fs::read_to_string(&path).unwrap();
        let document = roxmltree::Document::parse(&xml).unwrap();
        let owner_range = document
            .descendants()
            .find(|node| node.has_tag_name("Owners"))
            .unwrap()
            .range();
        let mut xml = xml.clone();
        xml.replace_range(owner_range, "");
        let xml = xml.replace("</Properties>", &format!(r#"{owners}<BasedOn xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><xr:Item xsi:type="xr:MDObjectRef">Document.Order</xr:Item></BasedOn></Properties>"#));
        write(&path, &xml);
        let service = fixture.view_service();
        for at in ["main:Catalog.Items", "main:Catalog.Items.Relation"] {
            let result = service.view(ViewRequest::new(at).unwrap());
            assert!(result.ok, "the read must succeed");
            let data = result.data.unwrap();
            let has_limit = data["limits"].as_array().is_some_and(|limits| {
                limits
                    .iter()
                    .any(|limit| limit.as_str().unwrap().contains("relations.owners"))
            });
            assert_eq!(
                has_limit, expected_unknown,
                "relation limits must follow the relation state"
            );
            if at.ends_with(".Relation") {
                assert!(
                    data["items"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|item| item["relation"] == "basedOn"
                            && item["at"] == "main:Document.Order"),
                    "relation limits must follow the relation state"
                );
            }
        }
        assert_eq!(fs::read_to_string(path).unwrap(), xml);
    }
}

#[test]
fn specialized_relation_limits_distinguish_absence_empty_and_damage() {
    // Synthetic edits exercise each reader route. Absence of BaseCalculationTypes
    // also occurs in the unmodified platform export used by acceptance S389.
    for (directory, kind, name, property, field) in [
        (
            "DocumentJournals",
            "DocumentJournal",
            "Журнал",
            "RegisteredDocuments",
            "registeredDocuments",
        ),
        (
            "ChartsOfCalculationTypes",
            "ChartOfCalculationTypes",
            "ВидыРасчета",
            "BaseCalculationTypes",
            "baseCalculationTypes",
        ),
        (
            "WebServices",
            "WebService",
            "Exchange",
            "XDTOPackages",
            "xdtoPackages",
        ),
    ] {
        for variant in ["missing", "empty", "broken", "duplicate", "foreign"] {
            let fixture = RealReaderFixture::new();
            let path = fixture.source.join(format!("{directory}/{name}.xml"));
            let xml = fs::read_to_string(&path).unwrap();
            let doc = roxmltree::Document::parse(&xml).unwrap();
            let range = doc
                .descendants()
                .find(|node| node.has_tag_name(property))
                .unwrap()
                .range();
            let replacement = match variant {
                "missing" => String::new(),
                "empty" => format!("<{property}/>"),
                "broken" => format!("<{property}><Broken/></{property}>"),
                "duplicate" => format!("<{property}/><{property}/>"),
                "foreign" => format!("<{property} xmlns=\"urn:foreign\"/>"),
                _ => unreachable!(),
            };
            let mut xml = xml.clone();
            xml.replace_range(range, &replacement);
            write(&path, &xml);
            let service = fixture.view_service();
            for suffix in ["", ".Relation"] {
                let result =
                    service.view(ViewRequest::new(&format!("main:{kind}.{name}{suffix}")).unwrap());
                assert!(result.ok, "{variant}: the read must succeed");
                let data = result.data.unwrap();
                let relevant = data["limits"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(serde_json::Value::as_str)
                    .filter(|limit| {
                        limit.contains(&format!("details.{field}"))
                            || limit.contains(&format!("properties.{property}"))
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    !relevant.is_empty(),
                    variant != "empty",
                    "{variant}: limits must follow the property state"
                );
                assert_eq!(
                    relevant.iter().any(|limit| limit.contains(" is absent;")),
                    variant == "missing",
                    "{variant}: limits must follow the property state"
                );
            }
            assert_eq!(fs::read_to_string(path).unwrap(), xml);
        }
    }
}

/// #1381 (вторая вычитка): неизвестное сообщается пределом узла. Регистр без
/// `WriteMode` не утверждает список регистраторов, модуль без флагов среды —
/// контексты; ребёнок заимствованного владельца называет, где разрешается
/// его родитель.
#[test]
fn unknown_facts_of_borrowed_objects_are_named_as_node_limits() {
    let fixture = RealReaderFixture::new();
    install_inherited_objects(&fixture);
    let configuration = fs::read_to_string(fixture.source.join("Configuration.xml")).unwrap();
    write(
        &fixture.source.join("Configuration.xml"),
        &configuration.replace(
            "</ChildObjects>",
            "<InformationRegister>CorpusRegister</InformationRegister><CommonModule>CorpusModule</CommonModule></ChildObjects>",
        ),
    );
    let register =
        fixture_text("platform_8_3_27/incomplete_borrowed/extension-information-register.xml");
    let register_path = fixture
        .source
        .join("InformationRegisters/CorpusRegister.xml");
    write(&register_path, &register);
    write(
        &fixture.source.join("CommonModules/CorpusModule.xml"),
        &fixture_text("platform_8_3_27/incomplete_borrowed/extension-common-module.xml"),
    );
    write(
        &fixture
            .source
            .join("CommonModules/CorpusModule/Ext/Module.bsl"),
        "Процедура Проверка() Экспорт\nКонецПроцедуры\n",
    );
    let limits = |at: &str| -> Vec<String> {
        let view = fixture
            .extension_view_service()
            .view(ViewRequest::new(at).unwrap());
        assert!(view.ok, "{at}: view must succeed");
        view.data.unwrap()["limits"]
            .as_array()
            .map(|limits| {
                limits
                    .iter()
                    .filter_map(|limit| limit.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let registrars = |limits: &[String]| {
        limits
            .iter()
            .any(|limit| limit.contains("relations.registrars") && limit.contains("WriteMode"))
    };
    assert!(registrars(&limits(
        "main:InformationRegister.CorpusRegister"
    )));
    assert!(registrars(&limits(
        "main:InformationRegister.CorpusRegister.Relation"
    )));
    write(
        &register_path,
        &register.replace(
            "</Properties>",
            "<WriteMode>Independent</WriteMode></Properties>",
        ),
    );
    assert!(!registrars(&limits(
        "main:InformationRegister.CorpusRegister"
    )));

    let contexts = |limits: &[String]| {
        limits
            .iter()
            .any(|limit| limit == super::UNKNOWN_CONTEXTS_LIMIT)
    };
    assert!(contexts(&limits("main:CommonModule.CorpusModule")));
    assert!(contexts(&limits("main:CommonModule.CorpusModule.Method")));
    write(
        &fixture.source.join("CommonModules/CorpusModule.xml"),
        &fixture_text("platform_8_3_27/cfe_borrow/extension-common-module-environment-only.xml")
            .replace("СерверныеФункции", "CorpusModule"),
    );
    assert!(!contexts(&limits("main:CommonModule.CorpusModule")));

    let service = fixture.extension_view_service();
    for at in [
        "main:DataProcessor.Обработка.Form.Форма",
        "main:DataProcessor.Обработка.Template.Макет",
    ] {
        let data = service.view(ViewRequest::new(at).unwrap()).data.unwrap();
        assert!(
            data["props"]["parentReason"].as_str().is_some_and(
                |reason| reason.contains("props.extends of `main:DataProcessor.Обработка`")
            ),
            "{at}: the child names where its parent is resolved"
        );
    }
    let role = service
        .view(ViewRequest::new("main:Role.SalesReader").unwrap())
        .data
        .unwrap();
    assert!(role["props"].get("parentReason").is_none());
}

fn nested_check_fixture() -> RealReaderFixture {
    let fixture = RealReaderFixture::new();
    write(
        &fixture.source.join("Subsystems/Sales.xml"),
        &fixture_text("acceptance/workspace-nested-check/src/Subsystems/Parent.xml")
            .replace("Parent", "Sales")
            .replace(
                "<Subsystem>Child</Subsystem>",
                "<Subsystem>Orders</Subsystem>",
            ),
    );
    write(&fixture.source.join("Subsystems/Sales/Subsystems/Orders.xml"),
        &fixture_text("acceptance/workspace-nested-check/src/Subsystems/Parent/Subsystems/Child/Subsystems/Grandchild.xml")
            .replace("Grandchild", "Orders"));
    fixture
}

#[test]
fn canonical_subsystem_check_validates_the_child_and_rejects_wrong_identity() {
    let fixture = nested_check_fixture();
    let address = QualifiedAddress::parse("main:Subsystem.Sales.Subsystem.Orders").unwrap();
    let path = fixture
        .source
        .join("Subsystems/Sales/Subsystems/Orders.xml");
    let original = fs::read_to_string(&path).unwrap();
    let checked = fixture
        .read_authority()
        .validate_subsystem(&address)
        .unwrap();
    let checked = crate::application::v13::check::normalize_native_outcome(
        &address,
        "Subsystem",
        crate::application::v13::check::CheckValidator::Subsystem,
        checked,
    )
    .unwrap();
    assert!(checked.ok(), "{:?}", checked.diagnostics());
    fs::write(&path, original.replace("Orders", "Other")).unwrap();
    assert_eq!(
        fixture
            .read_authority()
            .validate_subsystem(&address)
            .unwrap_err()
            .code(),
        RefusalCode::ProviderUnavailable
    );
    fixture.cancellation.cancel();
    assert_eq!(
        fixture
            .read_authority()
            .validate_subsystem(&address)
            .unwrap_err()
            .code(),
        RefusalCode::Cancelled
    );
}

#[test]
fn canonical_subsystem_check_refuses_linked_dependencies() {
    use crate::infrastructure::platform::testing::{
        create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };
    for dependency in [
        "Orders.xml",
        "Orders/Ext/CommandInterface.xml",
        "Orders/Subsystems/Next.xml",
    ] {
        let fixture = nested_check_fixture();
        let child = fixture
            .source
            .join("Subsystems/Sales/Subsystems/Orders.xml");
        if dependency.ends_with("Next.xml") {
            let text = fs::read_to_string(&child).unwrap().replace(
                "<ChildObjects>",
                "<ChildObjects><Subsystem>Next</Subsystem>",
            );
            fs::write(&child, text).unwrap();
        }
        let path = fixture
            .source
            .join("Subsystems/Sales/Subsystems")
            .join(dependency);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }
        let foreign = fixture.source.join("foreign.xml");
        fs::write(&foreign, "<CommandInterface version=\"2.20\"/>").unwrap();
        let outcome = create_file_link_fixture_for_test(&foreign, &path).unwrap();
        if outcome != FileLinkFixtureOutcome::Created {
            eprintln!("[SKIPPED FIXTURE] file-link fixture unavailable: {outcome:?}");
            return;
        }
        let address = QualifiedAddress::parse("main:Subsystem.Sales.Subsystem.Orders").unwrap();
        assert_eq!(
            fixture
                .read_authority()
                .validate_subsystem(&address)
                .unwrap_err()
                .code(),
            RefusalCode::ProviderUnavailable,
            "{dependency}"
        );
    }
}

#[test]
fn canonical_subsystem_check_refuses_replaced_root() {
    if !supports_retained_root_replacement_test() {
        return;
    }
    let fixture = nested_check_fixture();
    let authority = fixture.read_authority();
    let displaced = fixture.source.with_extension("retained");
    fs::rename(&fixture.source, &displaced).unwrap();
    fs::create_dir_all(&fixture.source).unwrap();
    let address = QualifiedAddress::parse("main:Subsystem.Sales.Subsystem.Orders").unwrap();
    assert_eq!(
        authority.validate_subsystem(&address).unwrap_err().code(),
        RefusalCode::ProviderUnavailable
    );
}

#[test]
fn canonical_subsystem_check_preserves_bom_format_warning() {
    for version in ["2.20", "2.21"] {
        let fixture = nested_check_fixture();
        let path = fixture
            .source
            .join("Subsystems/Sales/Subsystems/Orders.xml");
        let text = fs::read_to_string(&path).unwrap().replace("2.20", version);
        fs::write(&path, format!("\u{feff}{text}")).unwrap();
        let address = QualifiedAddress::parse("main:Subsystem.Sales.Subsystem.Orders").unwrap();
        let native = fixture
            .read_authority()
            .validate_subsystem(&address)
            .unwrap();
        let checked = crate::application::v13::check::normalize_native_outcome(
            &address,
            "Subsystem",
            crate::application::v13::check::CheckValidator::Subsystem,
            native,
        )
        .unwrap();
        assert!(checked.ok());
        assert!(!checked
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == "formatVersionInvalid"));
        assert_eq!(
            checked
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == "platformVersionUnsupported"),
            version == "2.21"
        );
    }
}

fn install_form_metadata_test_sources(fixture: &RealReaderFixture) {
    let config = fs::read_to_string(fixture.source.join("Configuration.xml")).unwrap();
    write(&fixture.source.join("Configuration.xml"), &config.replace("</ChildObjects>", "<CommonForm>ОбщаяФорма</CommonForm><DataProcessor>Обработка</DataProcessor></ChildObjects>"));
    write_metadata_owner(
        &fixture.source,
        "DataProcessors",
        "DataProcessor",
        "Обработка",
        "<Form>Форма</Form>",
    );
    for (descriptor, artifact, name, content) in [
        (
            "CommonForms/ОбщаяФорма.xml",
            "CommonForm",
            "ОбщаяФорма",
            "CommonForms/ОбщаяФорма/Ext/Form.xml",
        ),
        (
            "DataProcessors/Обработка/Forms/Форма.xml",
            "Form",
            "Форма",
            "DataProcessors/Обработка/Forms/Форма/Ext/Form.xml",
        ),
    ] {
        write(
            &fixture.source.join(descriptor),
            &format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{artifact} uuid="e9455333-3333-4333-8333-333333333333"><Properties><Name>{name}</Name><FormType>Managed</FormType></Properties></{artifact}></MetaDataObject>"#
            ),
        );
        write(
            &fixture.source.join(content),
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><ChildItems/></Form>"#,
        );
    }
}

#[test]
fn form_metadata_has_its_own_descriptor_properties_and_resource_identity() {
    let fixture = RealReaderFixture::new();
    install_form_metadata_test_sources(&fixture);
    for (root, descriptor, artifact, name, uuid, content) in [
        (
            "main:CommonForm.ОбщаяФорма",
            "CommonForms/ОбщаяФорма.xml",
            "CommonForm",
            "ОбщаяФорма",
            "e9455111-1111-4111-8111-111111111111",
            "CommonForms/ОбщаяФорма/Ext/Form.xml",
        ),
        (
            "main:DataProcessor.Обработка.Form.Форма",
            "DataProcessors/Обработка/Forms/Форма.xml",
            "Form",
            "Форма",
            "e9455222-2222-4222-8222-222222222222",
            "DataProcessors/Обработка/Forms/Форма/Ext/Form.xml",
        ),
    ] {
        write(
            &fixture.source.join(descriptor),
            &format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" version="2.20"><{artifact} uuid="{uuid}"><Properties><Name>{name}</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Описание формы</v8:content></v8:item><v8:item><v8:lang>en</v8:lang><v8:content>Form declaration</v8:content></v8:item></Synonym><ExtendedPresentation><v8:item><v8:lang>ru</v8:lang><v8:content>Расширенное представление</v8:content></v8:item><v8:item><v8:lang>en</v8:lang><v8:content>Extended presentation</v8:content></v8:item></ExtendedPresentation><Explanation><v8:item><v8:lang>ru</v8:lang><v8:content>Пояснение</v8:content></v8:item><v8:item><v8:lang>en</v8:lang><v8:content>Explanation</v8:content></v8:item></Explanation><FutureProperty><Nested>preserved source</Nested></FutureProperty><Comment>Only descriptor properties</Comment><FormType>Managed</FormType><UseStandardCommands>false</UseStandardCommands><UsePurposes><v8:Value xsi:type="app:ApplicationUsePurpose">PlatformApplication</v8:Value><v8:Value xsi:type="app:ApplicationUsePurpose">MobilePlatformApplication</v8:Value></UsePurposes></Properties></{artifact}></MetaDataObject>"#
            ),
        );
        let service = fixture.view_service();
        let metadata_at = format!("{root}.Metadata");
        let view = service.view(ViewRequest::new(&metadata_at).unwrap());
        assert!(view.ok, "{metadata_at}: {view:?}");
        let data = view.data.unwrap();
        assert_eq!(data["kind"], "Metadata");
        assert_eq!(data["props"]["uuid"], uuid);
        assert_eq!(data["props"]["Name"], name);
        assert_eq!(data["props"]["Comment"], "Only descriptor properties");
        assert_eq!(data["props"]["FormType"], "Managed");
        assert_eq!(data["props"]["UseStandardCommands"], false);
        assert_eq!(
            data["props"]["UsePurposes"],
            json!(["PlatformApplication", "MobilePlatformApplication"])
        );
        assert_eq!(
            data["props"]["Synonym"],
            json!({"kind":"localizedString","values":[{"language":"ru","content":"Описание формы"},{"language":"en","content":"Form declaration"}]})
        );
        for (key, ru, en) in [
            (
                "ExtendedPresentation",
                "Расширенное представление",
                "Extended presentation",
            ),
            ("Explanation", "Пояснение", "Explanation"),
        ] {
            assert_eq!(
                data["props"][key],
                json!({"kind":"localizedString","values":[{"language":"ru","content":ru},{"language":"en","content":en}]})
            );
        }
        assert!(data["limits"].as_array().unwrap().iter().any(|limit| limit
            .as_str()
            .is_some_and(|text| text.contains("FutureProperty"))));
        assert!(data["props"].get("FutureProperty").is_none());
        let root_view = service.view(ViewRequest::new(root).unwrap());
        assert!(root_view.ok);
        let root_data = root_view.data.unwrap();
        assert_eq!(root_data["kind"], "Form");
        assert!(root_data["props"].get("Comment").is_none());
        assert!(root_data["branches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|branch| branch["at"] == metadata_at));
        let authority = fixture.read_authority();
        assert_eq!(
            authority
                .existing_export_path(&QualifiedAddress::parse(root).unwrap(), None)
                .unwrap(),
            content
        );
        assert_eq!(
            authority
                .existing_export_path(&QualifiedAddress::parse(&metadata_at).unwrap(), None)
                .unwrap(),
            descriptor
        );
    }
}

#[test]
fn inherited_form_metadata_is_local_without_claiming_a_local_form_payload() {
    let fixture = RealReaderFixture::new();
    install_inherited_objects(&fixture);
    let service = fixture.extension_view_service();
    for (root, descriptor) in [
        ("main:CommonForm.ОбщаяФорма", "CommonForms/ОбщаяФорма.xml"),
        (
            "main:DataProcessor.Обработка.Form.Форма",
            "DataProcessors/Обработка/Forms/Форма.xml",
        ),
    ] {
        let metadata_at = format!("{root}.Metadata");
        let metadata = service.view(ViewRequest::new(&metadata_at).unwrap());
        assert!(metadata.ok, "{metadata_at}: {metadata:?}");
        let data = metadata.data.unwrap();
        assert_eq!(data["kind"], "Metadata");
        assert!(data["props"]["uuid"].is_string());
        assert!(data["props"]["Name"].is_string());
        assert!(data["props"].get("content").is_none());
        let form = service.view(ViewRequest::new(root).unwrap());
        assert!(form.ok);
        assert_eq!(form.data.unwrap()["props"]["content"], "inherited");
        let authority = fixture.extension_read_authority();
        assert_eq!(
            authority
                .existing_export_path(&QualifiedAddress::parse(&metadata_at).unwrap(), None)
                .unwrap(),
            descriptor
        );
        assert!(authority
            .existing_export_path(&QualifiedAddress::parse(root).unwrap(), None)
            .is_err());
    }
}

#[test]
fn form_metadata_rejects_orphans_and_does_not_follow_descriptor_links() {
    let fixture = RealReaderFixture::new();
    install_form_metadata_test_sources(&fixture);
    copy_fixture(
        "acceptance/workspace-resolve-resources/src/CommonForms/Orphan.xml",
        &fixture.source.join("CommonForms/Orphan.xml"),
    );
    let service = fixture.view_service();
    let orphan = service.view(ViewRequest::new("main:CommonForm.Orphan.Metadata").unwrap());
    assert!(!orphan.ok);
    assert_eq!(orphan.diagnostics[0]["code"], "not_found");
    for raw in [
        "main:CommonForm.ОбщаяФорма.Metadata.Item.X",
        "main:DataProcessor.Обработка.Metadata",
        "main:DataProcessor.Обработка.Form.Metadata",
    ] {
        let refused = ViewRequest::new(raw).map(|request| service.view(request));
        assert!(refused.is_err() || !refused.unwrap().ok, "{raw}");
    }
    {
        use crate::infrastructure::platform::testing::{
            create_file_link_fixture_for_test, FileLinkFixtureOutcome,
        };
        let descriptor = fixture.source.join("CommonForms/ОбщаяФорма.xml");
        let outside = fixture.root.path().join("outside-form.xml");
        fs::rename(&descriptor, &outside).unwrap();
        let outcome = create_file_link_fixture_for_test(&outside, &descriptor).unwrap();
        if outcome != FileLinkFixtureOutcome::Created {
            eprintln!("[SKIPPED FIXTURE] file link unavailable: {outcome:?}");
            return;
        }
        let fresh = fixture.view_service();
        let refused = fresh.view(ViewRequest::new("main:CommonForm.ОбщаяФорма.Metadata").unwrap());
        assert!(
            !refused.ok,
            "metadata must stay under the retained no-follow root"
        );
    }
}

#[test]
fn external_form_metadata_uses_the_local_descriptor_without_a_companion() {
    let fixture = RealExternalReaderFixture::new();
    for (source, kind, root, descriptor) in [
        (
            "artifact_processor",
            SourceSetKind::ExternalProcessor,
            "ExternalDataProcessor.Import.Form.Main",
            fixture.processor.join("Import/Forms/Main.xml"),
        ),
        (
            "artifact_report",
            SourceSetKind::ExternalReport,
            "ExternalReport.Sales.Form.Main",
            fixture.report.join("Sales/Forms/Main.xml"),
        ),
    ] {
        let form_xml = descriptor.with_extension("").join("Ext/Form.xml");
        fs::remove_file(form_xml).unwrap();
        let service = ViewService::new(
            fixture.read_authority(source, kind),
            ViewCursorStore::default(),
        );
        let metadata_at = format!("{source}:{root}.Metadata");
        let view = service.view(ViewRequest::new(&metadata_at).unwrap());
        assert!(view.ok, "{metadata_at}: {view:?}");
        let data = view.data.unwrap();
        assert_eq!(data["kind"], "Metadata");
        assert_eq!(data["props"]["Name"], "Main");
        assert!(data["props"].get("Comment").is_none());
        assert!(data["props"].get("UseStandardCommands").is_none());
        assert_eq!(
            data["props"]["uuid"],
            "10000000-0000-4000-8000-000000000011"
        );
        assert!(
            !service
                .view(ViewRequest::new(&format!("{source}:{root}")).unwrap())
                .ok
        );
    }
}
