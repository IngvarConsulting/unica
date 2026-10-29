mod catalog_attribute_properties;
mod catalog_attribute_tooltip;
mod catalog_tabular_properties;
mod dcs_parameter_expression;
mod defined_type_reference;
mod form;
mod form_conditional_appearance;
mod form_support;
mod job_schedule;
mod metadata;
mod metadata_declaration_diagnostics;
mod numerator_read_profile;
mod session_parameter_comment;
mod support;
mod xdto_missing_target;

use support::*;

use super::{
    project_known_suffix, project_typed_payload, resolve_platform_xml_target,
    LogicalViewReadAuthority, MetadataAddress, SourceTarget, TargetKindPolicy,
    PLATFORM_XML_8_3_27_FORMAT_2_20,
};
use crate::application::v13::find::FindRequest;
use crate::infrastructure::logical_tree::route_logical_address;
use unica_address::address::QualifiedAddress;
use unica_bsl::profile::PlatformProfile;
use unica_navigation::result_store::ViewCursorStore;
use unica_refusal::RefusalCode;
use {
    crate::application::v13::view::ViewReadAuthority, crate::application::v13::view::ViewRequest,
    crate::application::v13::view::ViewService, unica_execution_contract::view_error::ViewError,
};

use unica_source_contract::workspace::WorkspaceContext;
use unica_source_contract::SourceSetKind;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::test_support::supports_retained_root_replacement_test;

use crate::infrastructure::v13_read_port::{
    review_clear_revision_identity_hooks, review_set_revision_identity_hooks, ProviderReadAuthority,
};
use serde::Deserialize;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[test]
fn styleitem_font_border_values_reach_flat_public_props() {
    let fixture = RealReaderFixture::new();
    let configuration = fixture.source.join("Configuration.xml");
    let mut xml = fs::read_to_string(&configuration).unwrap();
    for (name, expected) in [
        (
            "StyleFont",
            json!({"Type":"Font", "fontRef":"style:NormalTextFont", "fontKind":"StyleItem", "fontBold":true, "fontItalic":false, "fontUnderline":false, "fontStrikeout":false}),
        ),
        (
            "AbsoluteFont",
            json!({"Type":"Font", "fontFaceName":"Arial", "fontHeight":16, "fontScale":100, "fontKind":"Absolute", "fontBold":true, "fontItalic":false, "fontUnderline":false, "fontStrikeout":false}),
        ),
        (
            "SingleBorder",
            json!({"Type":"Border", "borderWidth":1, "borderStyle":"Single"}),
        ),
    ] {
        xml = xml.replace(
            "</ChildObjects>",
            &format!("<StyleItem>{name}</StyleItem></ChildObjects>"),
        );
        write(
            &fixture.source.join(format!("StyleItems/{name}.xml")),
            &fixture_text(&format!("styleitem_values/{name}.xml")),
        );
        write(&configuration, &xml);
        let result = fixture
            .view_service()
            .view(ViewRequest::new(&format!("main:StyleItem.{name}")).unwrap());
        assert!(result.ok, "the call failed");
        let props = &result.data.as_ref().unwrap()["props"];
        for (key, value) in expected.as_object().unwrap() {
            assert_eq!(&props[key], value, "{name}.{key}");
        }
        assert_eq!(props["kind"], "StyleItem");
        assert!(props
            .as_object()
            .unwrap()
            .values()
            .all(|value| !value.is_object() && !value.is_array()));
    }
}

#[test]
fn styleitem_preserves_absent_false_and_damaged_overrides_independently() {
    for (edit, expected_flag, limited) in [
        ("italic=\"false\"", Some(false), false),
        ("italic=\"true\"", Some(true), false),
        ("", None, false),
        ("italic=\"broken\"", None, true),
        ("italic=\"\"", None, true),
    ] {
        let fixture = RealReaderFixture::new();
        let configuration = fixture.source.join("Configuration.xml");
        let xml = fs::read_to_string(&configuration).unwrap();
        write(
            &configuration,
            &xml.replace(
                "</ChildObjects>",
                "<StyleItem>AbsoluteFont</StyleItem></ChildObjects>",
            ),
        );
        let source = fixture_text("styleitem_values/AbsoluteFont.xml")
            .replace("italic=\"false\"", edit)
            .replace("scale=\"100\"", "scale=\"100\" vendor=\"private\"");
        write(&fixture.source.join("StyleItems/AbsoluteFont.xml"), &source);
        let result = fixture
            .view_service()
            .view(ViewRequest::new("main:StyleItem.AbsoluteFont").unwrap());
        assert!(result.ok, "the call failed");
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["props"]["fontHeight"], 16);
        assert_eq!(data["props"]["Type"], "Font");
        assert!(data["props"]["Synonym"].is_string());
        assert_eq!(
            data["props"].get("fontItalic"),
            expected_flag.map(serde_json::Value::Bool).as_ref()
        );
        assert!(data["props"].get("vendor").is_none());
        let limits = data.get("limits").and_then(serde_json::Value::as_array);
        assert_eq!(
            limits.is_some_and(|v| v
                .iter()
                .any(|v| v.as_str().is_some_and(|s| s.contains("fontItalic")))),
            limited,
            "{data}"
        );
    }
}

#[test]
fn styleitem_value_type_is_namespace_aware_and_damage_preserves_identity() {
    for (from, to, visible) in [
        ("v8ui", "aliased", true),
        ("http://v8.1c.ru/8.1/data/ui", "urn:vendor", false),
        ("xsi:type=\"v8ui:Font\"", "xsi:type=\"v8ui:Border\"", false),
        (
            "<Type>Font</Type>",
            "<Type>Font</Type><Type>Border</Type>",
            false,
        ),
    ] {
        let fixture = RealReaderFixture::new();
        let configuration = fixture.source.join("Configuration.xml");
        let xml = fs::read_to_string(&configuration).unwrap();
        write(
            &configuration,
            &xml.replace(
                "</ChildObjects>",
                "<StyleItem>AbsoluteFont</StyleItem></ChildObjects>",
            ),
        );
        write(
            &fixture.source.join("StyleItems/AbsoluteFont.xml"),
            &fixture_text("styleitem_values/AbsoluteFont.xml").replace(from, to),
        );
        let result = fixture
            .view_service()
            .view(ViewRequest::new("main:StyleItem.AbsoluteFont").unwrap());
        assert!(result.ok, "the call failed");
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["props"].get("fontHeight").is_some(), visible, "{data}");
        assert_eq!(data["props"]["kind"], "StyleItem");
        assert!(data["props"]["Synonym"].is_string());
        if !visible {
            assert!(!data["limits"].as_array().unwrap().is_empty());
        }
    }
}

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
) -> Result<serde_json::Value, unica_execution_contract::view_error::ViewError> {
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
    for raw in [
        "main:Configuration",
        "main:Catalog.Items",
        "main:CommonForm.Test",
    ] {
        let at = QualifiedAddress::parse(raw).unwrap();
        let route = route_logical_address(&at, PlatformProfile::v8_3_27()).unwrap();

        let error = project_typed_payload(
            &route,
            json!({"name": "Main", "mysteryProviderPayload": {"raw": "secret"}}),
        )
        .unwrap_err();

        assert_eq!(error.code().as_str(), "provider_unavailable", "{raw}");
    }
}

#[test]
fn actor_owned_reader_never_follows_a_source_set_remap_after_admission() {
    let fixture = RealReaderFixture::new();
    let cancellation = CancellationToken::new();
    let source_root =
        Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
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
    let source_root =
        Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
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
    let root = Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
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
    let root = Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
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
    let source_root =
        Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
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
    let source_root =
        Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
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
    let root = Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
    let reader = ProviderReadAuthority::new(
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
        root,
    );
    reader
        .exact_revision(
            unica_work_control::provider_deadline::ProviderDeadline::from_budget(
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

fn view_items(
    result: &unica_execution_contract::invocation::DomainResult,
) -> Vec<serde_json::Value> {
    assert!(result.ok, "{:?}", refusal_codes(result));
    result.data.as_ref().unwrap()["items"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn view_branch_count(
    result: &unica_execution_contract::invocation::DomainResult,
    at: &str,
) -> Option<u64> {
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
    let module_branch = super::module_branch_for_parent(
        route.at(),
        PlatformProfile::v8_3_27(),
        SourceSetKind::Configuration,
    )
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
fn root_kind_collection_follows_child_objects_order_in_both_readers() {
    let fixture = RealReaderFixture::new();
    let path = fixture.source.join("Configuration.xml");
    let xml = fs::read_to_string(&path).unwrap();
    // `Владельцы` sorts after `Items` by code points but stands first here.
    let xml = xml.replacen("<Catalog>Items</Catalog>", "", 1).replacen(
        "<Catalog>Владельцы</Catalog>",
        "<Catalog>Владельцы</Catalog><Catalog>Items</Catalog>",
        1,
    );
    fs::write(&path, &xml).unwrap();
    let names = |fixture: &RealReaderFixture| {
        let result = fixture
            .view_service()
            .view(ViewRequest::new("main:Catalog").unwrap());
        assert!(result.ok, "Catalog collection failed");
        result.data.unwrap()["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["title"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    let bounded = names(&fixture);
    assert_eq!(bounded, ["Владельцы", "Items"]);
    fs::write(
        &path,
        xml.replace(
            "</MetaDataObject>",
            &format!("<!--{}--></MetaDataObject>", "x".repeat(8 * 1024 * 1024)),
        ),
    )
    .unwrap();
    assert_eq!(
        names(&fixture),
        bounded,
        "streaming collection order differs"
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
fn xdto_anonymous_property_view_returns_recursive_addresses_and_bounds() {
    let fixture = RealReaderFixture::new();
    let package = fixture
        .source
        .join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin");
    let original = fs::read_to_string(&package).unwrap();
    let xml = original.replace(
        "</package>",
        r#"<objectType name="NestedExample">
        <property name="Rows" lowerBound="0" upperBound="-1">
            <typeDef xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="ObjectType">
                <property name="Basis" type="xs:string" lowerBound="0" upperBound="1"/>
                <property name="Nested"><typeDef xsi:type="ObjectType">
                    <property name="Recipient" type="xs:decimal" lowerBound="0"/>
                    <property name="Dotted.Name" type="xs:string"/>
                </typeDef></property>
            </typeDef>
        </property>
    </objectType></package>"#,
    );
    fs::write(&package, &xml).unwrap();
    let service = fixture.view_service();
    let root = "main:XDTOPackage.EnterpriseData_1_17_3.Type.NestedExample.Property.Rows";
    let read = |at: &str| {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(result.ok, "{at}: the call failed");
        result.data.unwrap()
    };
    let rows = read(root);
    assert_eq!(rows["props"]["anonymousType"], "ObjectType");
    let bounds = rows["props"]["bounds"].clone();
    assert_eq!(bounds["lower"], "0");
    assert_eq!(bounds["upper"], "-1");
    let children = read(rows["branches"][0]["at"].as_str().unwrap());
    assert_eq!(children["items"].as_array().unwrap().len(), 2);
    let nested = read(children["items"][1]["at"].as_str().unwrap());
    assert_eq!(nested["props"]["anonymousType"], "ObjectType");
    let grandchildren = read(nested["branches"][0]["at"].as_str().unwrap());
    let recipient = read(grandchildren["items"][0]["at"].as_str().unwrap());
    let type_info = recipient["props"]["type"].clone();
    assert_eq!(type_info["local"], "decimal");
    let dotted_at = format!("{root}.Property.Nested.Property.Dotted\\.Name");
    assert_eq!(grandchildren["items"][1]["at"], dotted_at);
    let dotted = read(&dotted_at);
    assert_eq!(dotted["props"], grandchildren["items"][1]["props"]);
    let type_info = dotted["props"]["type"].clone();
    assert_eq!(type_info["local"], "string");
    assert!(dotted
        .get("limits")
        .is_none_or(|limits| limits.as_array().unwrap().is_empty()));
    assert_eq!(fs::read_to_string(&package).unwrap(), xml);
}

#[test]
fn xdto_object_view_follows_local_base_to_inherited_property_without_flattening() {
    let fixture = RealReaderFixture::new();
    let package = fixture
        .source
        .join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin");
    let original = fs::read_to_string(&package).unwrap();
    let xml = original.replace(
        "</package>",
        r#"<objectType name="Основа"><property name="Код" type="xs:string"/></objectType>
<objectType xmlns:tns="http://v8.1c.ru/edi/edi_stnd/EnterpriseData/1.17.3" name="Наследник" base="tns:Основа"/>
<objectType xmlns:tns="http://v8.1c.ru/edi/edi_stnd/EnterpriseData/1.17.3" name="НетОсновы" base="tns:НеСуществует"/>
</package>"#,
    );
    fs::write(&package, &xml).unwrap();
    let service = fixture.view_service();
    let result = service
        .view(ViewRequest::new("main:XDTOPackage.EnterpriseData_1_17_3.Type.Наследник").unwrap());
    assert!(result.ok, "the call failed");
    let data = result.data.unwrap();
    let base = data["props"]["baseAt"]
        .as_str()
        .expect("local base must be navigable");
    assert_eq!(base, "main:XDTOPackage.EnterpriseData_1_17_3.Type.Основа");
    let qname = data["props"]["base"].clone();
    assert_eq!(qname["local"], "Основа");
    let inherited = service.view(ViewRequest::new(&format!("{base}.Property.Код")).unwrap());
    assert!(inherited.ok, "the call failed");
    let props = inherited.data.unwrap();
    let type_info = props["props"]["type"].clone();
    assert_eq!(type_info["local"], "string");
    let missing = service
        .view(ViewRequest::new("main:XDTOPackage.EnterpriseData_1_17_3.Type.НетОсновы").unwrap());
    assert!(missing.ok, "the call failed");
    assert!(missing.data.unwrap()["props"].get("baseAt").is_none());
    assert_eq!(fs::read_to_string(package).unwrap(), xml);
}

#[test]
fn xdto_object_view_follows_dotted_local_base_address() {
    let fixture = RealReaderFixture::new();
    let package = fixture
        .source
        .join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin");
    let original = fs::read_to_string(&package).unwrap();
    let xml = original.replace(
        "</package>",
        r#"<objectType name="Base.WithDot"><property name="Код" type="xs:string"/></objectType>
<objectType xmlns:tns="http://v8.1c.ru/edi/edi_stnd/EnterpriseData/1.17.3" name="Наследник" base="tns:Base.WithDot"/>
<objectType xmlns:tns="http://v8.1c.ru/edi/edi_stnd/EnterpriseData/1.17.3" name="НетОсновы" base="tns:НеСуществует"/>
</package>"#,
    );
    fs::write(&package, &xml).unwrap();
    let service = fixture.view_service();
    let result = service
        .view(ViewRequest::new("main:XDTOPackage.EnterpriseData_1_17_3.Type.Наследник").unwrap());
    assert!(result.ok, "the call failed");
    let data = result.data.unwrap();
    let base = data["props"]["baseAt"]
        .as_str()
        .expect("local base must be navigable");
    assert_eq!(
        base,
        "main:XDTOPackage.EnterpriseData_1_17_3.Type.Base\\.WithDot"
    );
    let qname = data["props"]["base"].clone();
    assert_eq!(qname["local"], "Base.WithDot");
    let inherited = service.view(ViewRequest::new(&format!("{base}.Property.Код")).unwrap());
    assert!(inherited.ok, "the call failed");
    let props = inherited.data.unwrap();
    let type_info = props["props"]["type"].clone();
    assert_eq!(type_info["local"], "string");
    let missing = service
        .view(ViewRequest::new("main:XDTOPackage.EnterpriseData_1_17_3.Type.НетОсновы").unwrap());
    assert!(missing.ok, "the call failed");
    assert!(missing.data.unwrap()["props"].get("baseAt").is_none());
    assert_eq!(fs::read_to_string(package).unwrap(), xml);
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
        assert!(result.ok, "the call failed");
        let data = result.data.unwrap();
        let props = &data["props"];
        let type_info = props["type"].clone();
        assert!(
            type_info.is_object(),
            "property type arrives as nested JSON"
        );
        assert_eq!(type_info["namespace"], "http://www.w3.org/2001/XMLSchema");
        assert_eq!(type_info["local"], local);
        assert_eq!(type_info["raw"], format!("xs:{local}"));
        let bounds = props["bounds"].clone();
        assert!(bounds.is_object(), "property bounds arrive as nested JSON");
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
    assert!(result.ok, "the call failed");
    let data = result.data.unwrap();
    assert!(data["props"]["type"].is_object());
    assert!(data["props"].get("bounds").is_none());
    assert!(
        data["limits"].as_array().unwrap().iter().any(|limit| limit
            .as_str()
            .is_some_and(|text| text.contains("bounds") && text.contains("2048"))),
        "unexpected answer"
    );
    assert_eq!(fs::read_to_string(package).unwrap(), xml);
}

#[test]
fn real_typed_readers_cover_every_task14_profile_without_skipping() {
    let fixture = RealReaderFixture::new();
    let cancellation = CancellationToken::new();
    let source_root =
        Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
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
    let source_root =
        Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
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
    let source_root =
        Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
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
    let source_root =
        Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
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

    let source_root =
        Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
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
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::RenameUnderOpenHandle,
            "the platform keeps an open handle from being renamed or replaced",
        );
        return;
    }
    let fixture = RealReaderFixture::new();
    let root = Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
    let deadline = unica_work_control::provider_deadline::ProviderDeadline::no_deadline();
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
            let type_info = props["type"]
                .as_object_mut()
                .expect("XDTO type arrives as nested JSON");
            // A namespace URI is a type identity, not an exported file path.
            assert_eq!(type_info["namespace"], "http://v8.1c.ru/8.1/data/core");
            type_info.remove("namespace");
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
    let root = Arc::new(
        unica_workspace::read::open_source_directory(&cancelled_fixture.processor).unwrap(),
    );
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
                Err(unica_execution_contract::view_error::ViewError::new(
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
    let root = Arc::new(unica_workspace::read::open_source_directory(&fixture.processor).unwrap());
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
            Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap()),
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
    use unica_metadata::support_state::ObjectSupportState;
    use unica_metadata_designer::support::support_root_uuid_from_bytes;

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
    use unica_execution_contract::view_error::ViewError;
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
            .map(|mapping| {
                mapping
                    .module_scope()
                    .expect("a module mapping has its scope")
            })
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
                .map(|mapping| mapping
                    .module_scope()
                    .expect("a module mapping has its scope"))
                .is_err(),
            "invented module scope for {at}"
        );
    }
}

/// #1966: объект и корень проверяются как цель целиком — без модульной
/// области и без выдуманного объекта; общий модуль как объект — тоже объект.
#[test]
fn diagnostic_node_mapping_proves_object_common_module_and_root_without_module_scope() {
    use unica_address::source_target::TargetKind;
    let fixture = RealReaderFixture::new();
    let authority = fixture.read_authority();
    for (at, metadata_path, kind) in [
        (
            "main:Catalog.Items",
            Some("Catalog.Items"),
            TargetKind::MetadataObject,
        ),
        (
            "main:CommonModule.РеактивныйСервер",
            Some("CommonModule.РеактивныйСервер"),
            TargetKind::MetadataObject,
        ),
        ("main:Configuration", None, TargetKind::SourceRoot),
    ] {
        let mapping = authority
            .diagnostic_node_mapping(&QualifiedAddress::parse(at).unwrap(), &fixture.context)
            .unwrap_or_else(|error| panic!("{at}: {error:?}"));
        assert_eq!(
            mapping
                .target()
                .metadata_path
                .as_ref()
                .map(MetadataAddress::as_str),
            metadata_path,
            "{at}"
        );
        assert_eq!(mapping.target().target_kind, kind, "{at}");
        assert!(mapping.module_scope().is_none(), "{at}");
    }
    for at in [
        "main:Catalog.Missing",
        "other:Catalog.Items",
        "main:CommonModule.РеактивныйСервер.Body",
    ] {
        assert!(
            authority
                .diagnostic_node_mapping(&QualifiedAddress::parse(at).unwrap(), &fixture.context)
                .is_err(),
            "invented node target for {at}"
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
        .map(|mapping| {
            mapping
                .module_scope()
                .expect("a module mapping has its scope")
        })
        .unwrap();
    let replacement = fixture.root.path().join("replacement");
    write(
        &replacement.join("CommonModules/РеактивныйСервер/Ext/Module.bsl"),
        "// replacement only\n",
    );
    fs::write(fixture.root.path().join("v8project.yaml"), "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: replacement\n").unwrap();
    let retained = authority
        .diagnostic_mapping(&at, &fixture.context)
        .map(|mapping| {
            mapping
                .module_scope()
                .expect("a module mapping has its scope")
        })
        .unwrap();
    assert_eq!(retained.source_root, admitted.source_root);
    assert_eq!(retained.module_path, admitted.module_path);
    assert_ne!(retained.source_root, replacement);
}

#[test]
fn diagnostic_module_scope_refuses_replaced_retained_root() {
    if !supports_retained_root_replacement_test() {
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::RenameUnderOpenHandle,
            "the platform keeps an open handle from being renamed or replaced",
        );
        return;
    }
    let fixture = RealReaderFixture::new();
    let authority = fixture.operation_read_authority();
    let at = QualifiedAddress::parse("main:CommonModule.РеактивныйСервер").unwrap();
    assert!(authority
        .diagnostic_mapping(&at, &fixture.context)
        .map(|mapping| mapping
            .module_scope()
            .expect("a module mapping has its scope"))
        .is_ok());
    fs::rename(&fixture.source, fixture.root.path().join("saved-source")).unwrap();
    fs::create_dir(&fixture.source).unwrap();
    assert!(authority
        .diagnostic_mapping(&at, &fixture.context)
        .map(|mapping| mapping
            .module_scope()
            .expect("a module mapping has its scope"))
        .is_err());
}

#[test]
fn diagnostic_module_scope_refuses_module_replaced_by_foreign_symlink() {
    use unica_workspace::test_support::{
        create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };
    let fixture = RealReaderFixture::new();
    let authority = fixture.read_authority();
    let at = QualifiedAddress::parse("main:CommonModule.РеактивныйСервер").unwrap();
    let module = authority
        .diagnostic_mapping(&at, &fixture.context)
        .map(|mapping| {
            mapping
                .module_scope()
                .expect("a module mapping has its scope")
        })
        .unwrap()
        .module_path;
    let foreign = fixture.root.path().join("foreign.bsl");
    fs::write(&foreign, "// foreign module\n").unwrap();
    fs::remove_file(&module).unwrap();
    let outcome = create_file_link_fixture_for_test(&foreign, &module)
        .expect("unexpected file-link creation error must fail the fixture test");
    if outcome != FileLinkFixtureOutcome::Created {
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::Symlink,
            "creating a link fixture needs the symlink privilege",
        );
        return;
    }
    assert!(authority
        .diagnostic_mapping(&at, &fixture.context)
        .map(|mapping| mapping
            .module_scope()
            .expect("a module mapping has its scope"))
        .is_err());
    assert_eq!(fs::read_to_string(foreign).unwrap(), "// foreign module\n");
}

mod canonical_module_diagnostics {
    use super::*;
    use crate::infrastructure::diagnostics::CanonicalDiagnosticMapping;
    use std::time::Duration;
    use unica_address::source_target::TargetKind;
    use unica_diagnostics::coordinator::DiagnosticCoordinator;
    use unica_diagnostics::coordinator::DiagnosticMapping;
    use unica_diagnostics::model::*;
    use unica_source_contract::location::SourceLocation;

    fn request(mapping: &CanonicalDiagnosticMapping) -> DiagnosticRequest {
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
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::RenameUnderOpenHandle,
                "the platform keeps an open handle from being renamed or replaced",
            );
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
fn form_validation_input_keeps_the_authority_cancellation_and_deadline() {
    let fixture = RealReaderFixture::new();
    let address = QualifiedAddress::parse("main:Report.ParityReport.Form.MainForm").unwrap();
    let mut authority = fixture.read_authority();
    authority.deadline = ProviderDeadline::from_budget(std::time::Duration::ZERO);
    assert_eq!(
        authority
            .form_validation_input(&address)
            .err()
            .unwrap()
            .code(),
        RefusalCode::DeadlineExceeded
    );
    let authority = fixture.read_authority();
    fixture.cancellation.cancel();
    assert_eq!(
        authority
            .form_validation_input(&address)
            .err()
            .unwrap()
            .code(),
        RefusalCode::Cancelled
    );
}

#[test]
fn form_validation_input_does_not_read_a_module_without_checkable_bindings() {
    let fixture = RealExternalReaderFixture::new();
    let address =
        QualifiedAddress::parse("artifact_processor:ExternalDataProcessor.Import.Form.Main")
            .unwrap();
    let body = fixture.processor.join("Import/Forms/Main/Ext/Form.xml");
    let module = fixture
        .processor
        .join("Import/Forms/Main/Ext/Form/Module.bsl");
    if module.exists() {
        fs::remove_file(&module).unwrap();
    }
    // Каталог вместо файла доказывает отсутствие обращения к ненужному модулю.
    fs::create_dir_all(&module).unwrap();
    for text in [
        r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><AutoCommandBar name="Панель" id="-1"/><ChildItems/></Form>"#,
        r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><Events><Event name="UnknownEvent">Обработчик</Event></Events></Form>"#,
        r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><Events><Event name="OnOpen" callType="Before">ПриОткрытии</Event></Events></Form>"#,
        "<Form>",
    ] {
        write(&body, text);
        let input = fixture
            .read_authority("artifact_processor", SourceSetKind::ExternalProcessor)
            .form_validation_input(&address)
            .unwrap();
        assert!(input.module.is_none());
    }
}

#[test]
fn form_validation_input_refuses_linked_form_and_needed_module() {
    use unica_workspace::test_support::{
        create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };
    for linked in ["Form.xml", "Form/Module.bsl"] {
        let fixture = RealExternalReaderFixture::new();
        let address =
            QualifiedAddress::parse("artifact_processor:ExternalDataProcessor.Import.Form.Main")
                .unwrap();
        let ext = fixture.processor.join("Import/Forms/Main/Ext");
        write(
            &ext.join("Form.xml"),
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><Events><Event name="OnOpen">ПриОткрытии</Event></Events></Form>"#,
        );
        write(
            &ext.join("Form/Module.bsl"),
            "&НаКлиенте\nПроцедура ПриОткрытии(Отказ)\nКонецПроцедуры\n",
        );
        let path = ext.join(linked);
        let foreign = fixture.context.cwd.join("foreign");
        fs::rename(&path, &foreign).unwrap();
        let before = fs::read(&foreign).unwrap();
        let outcome = create_file_link_fixture_for_test(&foreign, &path)
            .expect("unexpected file-link creation error must fail the fixture test");
        if outcome != FileLinkFixtureOutcome::Created {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a link fixture needs the symlink privilege",
            );
            return;
        }
        assert!(
            fixture
                .read_authority("artifact_processor", SourceSetKind::ExternalProcessor)
                .form_validation_input(&address)
                .is_err(),
            "{linked}"
        );
        assert_eq!(fs::read(&foreign).unwrap(), before);
    }
}

#[test]
fn canonical_form_validation_reports_captured_malformed_xml() {
    let fixture = RealExternalReaderFixture::new();
    let address =
        QualifiedAddress::parse("artifact_processor:ExternalDataProcessor.Import.Form.Main")
            .unwrap();
    write(
        &fixture.processor.join("Import/Forms/Main/Ext/Form.xml"),
        "<Form>",
    );
    let input = fixture
        .read_authority("artifact_processor", SourceSetKind::ExternalProcessor)
        .form_validation_input(&address)
        .unwrap();
    let checked = crate::application::v13::check::normalize_native_outcome(
        &address,
        "Form",
        crate::application::v13::check::CheckValidator::Form,
        crate::infrastructure::native_operations::v13_analysis::validate_form_input(input),
    )
    .unwrap();
    assert!(!checked.ok());
    assert!(checked
        .diagnostics()
        .iter()
        .any(|diagnostic| diagnostic.severity() == "error"
            && diagnostic.message().contains("XML parse error")));
}

#[test]
fn retained_form_check_ignores_source_set_remapping() {
    let fixture = RealExternalReaderFixture::new();
    let address =
        QualifiedAddress::parse("artifact_processor:ExternalDataProcessor.Import.Form.Main")
            .unwrap();
    let body = fixture.processor.join("Import/Forms/Main/Ext/Form.xml");
    let original = fs::read_to_string(&body).unwrap();
    let authority = fixture.read_authority("artifact_processor", SourceSetKind::ExternalProcessor);
    let admitted = authority.form_validation_input(&address).unwrap();
    assert_eq!(admitted.text, original);

    // Новый корень содержит отличающуюся форму; право чтения остаётся прежним.
    let replacement = fixture.context.cwd.join("replacement");
    write(
        &replacement.join("Import.xml"),
        &fs::read_to_string(fixture.processor.join("Import.xml")).unwrap(),
    );
    write(
        &replacement.join("Import/Forms/Main.xml"),
        &fs::read_to_string(fixture.processor.join("Import/Forms/Main.xml")).unwrap(),
    );
    write(
        &replacement.join("Import/Forms/Main/Ext/Form.xml"),
        "<Form/>",
    );
    fs::write(
        fixture.context.cwd.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: artifact_processor\n    type: EXTERNAL_DATA_PROCESSORS\n    path: replacement\n",
    )
    .unwrap();
    let retained = authority.form_validation_input(&address).unwrap();
    assert_eq!(retained.text, original);
    assert_eq!(retained.artifact, admitted.artifact);
    assert!(!retained.config_context);
}

#[test]
fn canonical_form_validation_keeps_captured_xml_module_and_owner() {
    let fixture = RealExternalReaderFixture::new();
    for (source_set, kind, root, owner) in [
        (
            "artifact_processor",
            SourceSetKind::ExternalProcessor,
            &fixture.processor,
            "ExternalDataProcessor.Import",
        ),
        (
            "artifact_report",
            SourceSetKind::ExternalReport,
            &fixture.report,
            "ExternalReport.Sales",
        ),
    ] {
        let name = owner.split('.').nth(1).unwrap();
        let body = root.join(format!("{name}/Forms/Main/Ext/Form.xml"));
        let module = root.join(format!("{name}/Forms/Main/Ext/Form/Module.bsl"));
        write(
            &body,
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><AutoCommandBar name="КоманднаяПанель" id="-1"/><Events><Event name="OnOpen">ПриОткрытии</Event></Events><ChildItems/></Form>"#,
        );
        // Предел выдачи текста не ограничивает вход проверки: обработчик
        // после этого предела всё равно должен участвовать в диагностике.
        let padding =
            "// Заполнение\n".repeat(crate::application::v13::source_text::MAX_SOURCE_TEXT_BYTES);
        write(&module, &format!("{padding}&НаКлиенте\nПроцедура ПриОткрытии(Отказ, Лишний, ЕщёОдин)\nКонецПроцедуры\n"));
        let address = QualifiedAddress::parse(&format!("{source_set}:{owner}.Form.Main")).unwrap();
        let input = fixture
            .read_authority(source_set, kind)
            .form_validation_input(&address)
            .unwrap();

        // Валидатор обязан использовать захваченные байты даже после замены всех входов.
        write(&body, "это уже не XML");
        write(
            &module,
            "&НаКлиенте\nПроцедура ПриОткрытии(Отказ)\nКонецПроцедуры\n",
        );
        write(&root.join(format!("{name}.xml")), "описание заменено");
        write(
            &root.join(format!("{name}/Forms/Main.xml")),
            "описание формы заменено",
        );
        let checked = crate::application::v13::check::normalize_native_outcome(
            &address,
            "Form",
            crate::application::v13::check::CheckValidator::Form,
            crate::infrastructure::native_operations::v13_analysis::validate_form_input(input),
        )
        .unwrap();
        assert!(checked.ok());
        assert!(checked
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.severity() == "warning"
                && diagnostic.message().contains("ПриОткрытии")));
        assert_eq!(fs::read_to_string(&body).unwrap(), "это уже не XML");
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
    use unica_workspace::test_support::{
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
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a link fixture needs the symlink privilege",
            );
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
    use unica_workspace::test_support::{
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
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a link fixture needs the symlink privilege",
            );
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
        let target = unica_address::source_target::MetadataAddress::parse(
            unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
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
        Arc::new(unica_workspace::read::open_source_directory(source).unwrap()),
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
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::RenameUnderOpenHandle,
            "the platform keeps an open handle from being renamed or replaced",
        );
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
    let admission = crate::infrastructure::source_workspace::discover_project_source_admission(
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
        unica_workspace::selection_evidence::SourceSelectionEvidenceErrorKind::Changed
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
            Some(unica_refusal::RefusalDetail::SourceUnreadable)
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
fn external_leaf_reads_a_relative_file_whole_whatever_its_size() {
    let (_workspace, source) = external_leaf_fixture(SourceSetKind::ExternalProcessor);
    let bytes =
        external_leaf_descriptor("ExternalDataProcessor", "Large", 9 * 1024 * 1024).into_bytes();
    fs::write(source.join("Large.xml"), &bytes).unwrap();
    let reader = external_leaf_read(&source, SourceSetKind::ExternalProcessor);
    assert!(
        reader.read_relative_whole(Path::new("Large.xml")).unwrap() == bytes,
        "large relative file differs from its source bytes"
    );
    assert_eq!(
        configuration_payload(&reader).unwrap()["registeredObjects"],
        json!([{"kind":"ExternalDataProcessor","name":"Large"}])
    );
}

/// #1119: a correct descriptor larger than the former 8 MiB read limit is
/// read through the public view instead of refused by its size.
#[test]
fn a_descriptor_larger_than_eight_mebibytes_is_read_through_public_view() {
    let fixture = RealReaderFixture::new();
    let relative = fixture
        .source
        .read_dir()
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.ends_with("Catalogs"))
        .expect("the real fixture has catalogs");
    let descriptor = fs::read_dir(&relative)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|extension| extension == "xml"))
        .unwrap();
    let name = descriptor
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let original = fs::read_to_string(&descriptor).unwrap();
    let padding = format!("<!--{}-->", "x".repeat(9 * 1024 * 1024));
    let at = original.find("<MetaDataObject").unwrap();
    let mut large = original.clone();
    large.insert_str(at, &padding);
    fs::write(&descriptor, &large).unwrap();
    assert!(large.len() > 8 * 1024 * 1024);
    let result = fixture
        .view_service()
        .view(ViewRequest::new(&format!("main:Catalog.{name}")).unwrap());
    assert!(result.ok);
    assert_eq!(result.data.unwrap()["at"], format!("main:Catalog.{name}"));
}

#[test]
fn mxl_check_consumes_captured_body_without_reopening_its_display_path() {
    let fixture = RealReaderFixture::new();
    fs::copy(fixture_path("acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/F06Template/Ext/Template.xml"), fixture.source.join("Reports/ParityReport/Templates/Print/Ext/Template.xml")).unwrap();
    let at = QualifiedAddress::parse("main:Report.ParityReport.Template.Print").unwrap();
    let authority = fixture.read_authority();
    let input = authority.mxl_validation_input(&at).unwrap();
    fs::remove_file(&input.artifact).unwrap();
    let result = crate::infrastructure::native_operations::mxl_composition::validate_mxl_input(
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
            crate::infrastructure::native_operations::mxl_composition::validate_mxl_input(
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
            crate::infrastructure::native_operations::mxl_composition::validate_mxl_input(
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
        crate::infrastructure::native_operations::mxl_composition::validate_mxl_input(
            &serde_json::Map::new(),
            &input.text,
            input.artifact
        )
        .ok
    );
    fixture.cancellation.cancel();
    assert!(authority.mxl_validation_input(&at).is_err());
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
    assert!(checked.ok(), "the call failed");
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
    use unica_workspace::test_support::{
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
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a link fixture needs the symlink privilege",
            );
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
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::RenameUnderOpenHandle,
            "the platform keeps an open handle from being renamed or replaced",
        );
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

#[test]
fn tabular_sections_expose_source_standard_attributes_without_losing_applied_attributes() {
    let fixture = RealReaderFixture::new();
    copy_fixture(
        "acceptance/workspace-tabular-standard/src/Catalogs/Items.xml",
        &fixture.source.join("Catalogs/Items.xml"),
    );
    let before = fs::read(fixture.source.join("Catalogs/Items.xml")).unwrap();
    let service = fixture.view_service();
    let at = "main:Catalog.Items.TabularSection.Rows";
    let section = service.view(ViewRequest::new(at).unwrap());
    assert_eq!(
        view_branch_count(&section, &format!("{at}.StandardAttribute")),
        Some(1)
    );
    let applied = view_items(&service.view(ViewRequest::new(&format!("{at}.Attribute")).unwrap()));
    let xml = std::str::from_utf8(&before).unwrap();
    let doc = roxmltree::Document::parse(xml).unwrap();
    let rows = doc
        .descendants()
        .find(|n| n.has_tag_name("TabularSection"))
        .unwrap();
    let names = rows
        .descendants()
        .filter(|n| n.has_tag_name("Attribute"))
        .map(|n| {
            n.descendants()
                .find(|c| c.has_tag_name("Name"))
                .unwrap()
                .text()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(names.len(), 10);
    assert_eq!(
        applied
            .iter()
            .map(|v| v["title"].as_str().unwrap())
            .collect::<Vec<_>>(),
        names
    );
    let standard =
        view_items(&service.view(ViewRequest::new(&format!("{at}.StandardAttribute")).unwrap()));
    assert_eq!(standard.len(), 1);
    let detail_at = standard[0]["at"].as_str().unwrap();
    assert_eq!(detail_at, format!("{at}.StandardAttribute.LineNumber"));
    let detail = service.view(ViewRequest::new(detail_at).unwrap());
    assert!(detail.ok, "the call failed");
    let data = detail.data.unwrap();
    assert_eq!(data["kind"], "StandardAttribute");
    // Пустые наблюдения (`empty`, `nil`, многоязычная строка без переводов)
    // свойства не создают; остальные приходят без обёртки наблюдения (#2031).
    assert_eq!(data["props"].as_object().unwrap().len(), 16);
    for (key, expected) in [
        ("LinkByType", None),
        ("FillChecking", Some(json!("DontCheck"))),
        ("MultiLine", Some(json!(false))),
        ("FillFromFillingValue", Some(json!(false))),
        ("CreateOnInput", Some(json!("Auto"))),
        ("TypeReductionMode", Some(json!("TransformValues"))),
        ("MaxValue", None),
        ("ToolTip", None),
        ("ExtendedEdit", Some(json!(false))),
        ("Format", None),
        ("ChoiceForm", Some(json!(""))),
        ("QuickChoice", Some(json!("Auto"))),
        ("ChoiceHistoryOnInput", Some(json!("Auto"))),
        ("EditFormat", None),
        ("PasswordMode", Some(json!(false))),
        ("DataHistory", Some(json!("Use"))),
        ("MarkNegatives", Some(json!(false))),
        ("MinValue", None),
        (
            "Synonym",
            Some(json!([{"language":"ru","content":"Номер строки"}])),
        ),
        ("Comment", Some(json!("Line-number note"))),
        ("FullTextSearch", Some(json!("Use"))),
        ("ChoiceParameterLinks", None),
        ("FillValue", None),
        ("Mask", Some(json!(""))),
        ("ChoiceParameters", None),
    ] {
        match expected {
            Some(expected) => assert_eq!(data["props"][key], expected, "{key}"),
            None => {
                assert!(data["props"].get(key).is_none(), "{key} must stay absent");
                assert!(
                    !data["limits"].to_string().contains(key),
                    "an empty {key} is not a limit"
                );
            }
        }
        assert_eq!(data["props"].get(key), standard[0]["props"].get(key));
    }
    assert!(data["props"].get("type").is_none());
    let missing =
        service.view(ViewRequest::new(&format!("{at}.StandardAttribute.Missing")).unwrap());
    assert_eq!(missing.diagnostics[0]["code"], "not_found");
    assert_eq!(
        fs::read(fixture.source.join("Catalogs/Items.xml")).unwrap(),
        before
    );
}
#[test]
fn tabular_standard_attribute_damage_is_local_and_not_an_empty_observation() {
    for variant in [
        "missing",
        "empty",
        "no-properties",
        "bad-entry",
        "duplicate-container",
        "foreign-container",
        "container-attributes",
        "foreign-entry",
        "duplicate-property",
        "bare-localized-text",
        "invalid-name",
        "whitespace-name",
        "duplicate-name",
    ] {
        let fixture = RealReaderFixture::new();
        let path = fixture.source.join("Catalogs/Items.xml");
        let original = fixture_text("acceptance/workspace-tabular-standard/src/Catalogs/Items.xml");
        let doc = roxmltree::Document::parse(&original).unwrap();
        let container = doc
            .descendants()
            .find(|n| n.has_tag_name("StandardAttributes"))
            .unwrap();
        let observed = &original[container.range()];
        let replacement = match variant {
            "missing" => String::new(),
            "empty" => "<StandardAttributes/>".into(),
            "no-properties" => "<StandardAttributes><xr:StandardAttribute name=\"LineNumber\"/></StandardAttributes>".into(),
            "bad-entry" => "<StandardAttributes><xr:StandardAttribute/></StandardAttributes>".into(),
            "duplicate-container" => format!("{observed}{observed}"),
            "foreign-container" => observed.replace("<StandardAttributes>", "<StandardAttributes xmlns=\"urn:foreign\">"),
            "container-attributes" => observed.replace("<StandardAttributes>", "<StandardAttributes unexpected=\"yes\">"),
            "foreign-entry" => observed.replace("<xr:StandardAttribute ", "<xr:StandardAttribute xmlns:xr=\"urn:foreign\" "),
            "bare-localized-text" => observed.replace("<xr:ToolTip/>", "<xr:ToolTip>broken</xr:ToolTip>"),
            "duplicate-name" => "<StandardAttributes><xr:StandardAttribute name=\"LineNumber\"><xr:Comment>first</xr:Comment></xr:StandardAttribute><xr:StandardAttribute name=\"LineNumber\"><xr:Comment>second</xr:Comment></xr:StandardAttribute></StandardAttributes>".into(),
            "invalid-name" => observed.replace("name=\"LineNumber\"", "name=\"bad name\""),
            "whitespace-name" => observed.replace("name=\"LineNumber\"", "name=\"   \""),
            "duplicate-property" => observed.replace("<xr:ToolTip/>", "<xr:ToolTip/><xr:ToolTip/>"),
            _ => unreachable!(),
        };
        let damaged = !matches!(variant, "missing" | "empty" | "no-properties");
        let mut xml = original.clone();
        xml.replace_range(container.range(), &replacement);
        write(&path, &xml);
        let before = fs::read(&path).unwrap();
        let service = fixture.view_service();
        let sections = view_items(
            &service.view(ViewRequest::new("main:Catalog.Items.TabularSection").unwrap()),
        );
        for name in ["Rows", "Healthy"] {
            let at = format!("main:Catalog.Items.TabularSection.{name}");
            let section = service.view(ViewRequest::new(&at).unwrap());
            assert!(section.ok, "{variant} {name}: the call failed");
            let data = section.data.as_ref().unwrap();
            let is_damaged = damaged && name == "Rows";
            assert_eq!(
                data["limits"].to_string().contains("standardAttributes"),
                is_damaged,
                "{variant} {name}: {data}"
            );
            let listed = sections.iter().find(|v| v["title"] == name).unwrap();
            assert_eq!(
                listed.get("limits"),
                data.get("limits"),
                "{variant}: collection and detail disagree"
            );
            assert_eq!(
                view_branch_count(&section, &format!("{at}.Attribute")),
                Some(10)
            );
            assert_eq!(
                view_items(&service.view(ViewRequest::new(&format!("{at}.Attribute")).unwrap()))
                    .len(),
                10
            );
            let collection =
                service.view(ViewRequest::new(&format!("{at}.StandardAttribute")).unwrap());
            let detail = service
                .view(ViewRequest::new(&format!("{at}.StandardAttribute.LineNumber")).unwrap());
            if name == "Healthy" || variant == "no-properties" {
                let items = view_items(&collection);
                assert_eq!(items.len(), 1);
                assert!(detail.ok, "{variant}: the call failed");
                assert_eq!(detail.data.as_ref().unwrap()["kind"], "StandardAttribute");
                if variant == "no-properties" && name == "Rows" {
                    assert!(
                        detail.data.as_ref().unwrap().get("props").is_none(),
                        "missing properties must not be invented"
                    );
                }
            } else if variant == "empty" {
                assert!(view_items(&collection).is_empty());
                assert_eq!(detail.diagnostics[0]["code"], "not_found");
            } else if variant == "missing" {
                assert_eq!(collection.diagnostics[0]["code"], "not_found");
                assert_eq!(detail.diagnostics[0]["code"], "not_found");
            } else {
                for refused in [&collection, &detail] {
                    assert!(
                        !refused.ok,
                        "{variant}: the call was expected to be refused"
                    );
                    assert_eq!(refused.diagnostics[0]["detailCode"], "source_unreadable");
                    assert!(refused.diagnostics[0]["message"]
                        .as_str()
                        .unwrap()
                        .contains("collections.tabularSections[0].standardAttributes"));
                }
            }
        }
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}

#[test]
fn tabular_standard_attributes_preserve_profiled_member_facts_and_independent_limits() {
    for (damaged_standard, damaged_member) in
        [(false, false), (true, false), (false, true), (true, true)]
    {
        let fixture = RealReaderFixture::new();
        let original = fixture_text("acceptance/workspace-tabular-standard/src/Catalogs/Items.xml");
        let use_value = if damaged_member { "broken" } else { "ForItem" };
        let mut xml = original.replacen(
            "<Name>Rows</Name>",
            &format!(
                "<Name>Rows</Name><LineNumberLength>5</LineNumberLength><Use>{use_value}</Use>"
            ),
            1,
        );
        if damaged_standard {
            xml = xml.replacen("<xr:ToolTip/>", "<xr:ToolTip>broken</xr:ToolTip>", 1);
        }
        write(&fixture.source.join("Catalogs/Items.xml"), &xml);
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let service = fixture.view_service();
        let at = "main:Catalog.Items.TabularSection.Rows";
        let section = service.view(ViewRequest::new(at).unwrap());
        assert!(section.ok, "the call failed");
        let data = section.data.as_ref().unwrap();
        assert_eq!(data["props"]["LineNumberLength"], 5);
        if damaged_member {
            assert!(data["props"].get("Use").is_none());
        } else {
            assert_eq!(data["props"]["Use"], "ForItem");
        }
        let limits = data["limits"].to_string();
        assert_eq!(limits.contains(".properties.Use"), damaged_member);
        assert_eq!(limits.contains(".standardAttributes"), damaged_standard);
        let sections = view_items(
            &service.view(ViewRequest::new("main:Catalog.Items.TabularSection").unwrap()),
        );
        let listed = sections.iter().find(|item| item["at"] == at).unwrap();
        assert_eq!(listed["props"], data["props"]);
        assert_eq!(listed.get("limits"), data.get("limits"));
        assert_eq!(
            view_items(&service.view(ViewRequest::new(&format!("{at}.Attribute")).unwrap())).len(),
            10
        );
        let standard =
            service.view(ViewRequest::new(&format!("{at}.StandardAttribute.LineNumber")).unwrap());
        if damaged_standard {
            assert!(!standard.ok, "the call was expected to be refused");
            assert_eq!(standard.diagnostics[0]["detailCode"], "source_unreadable");
        } else {
            assert!(standard.ok, "the call failed");
            assert_eq!(
                standard.data.as_ref().unwrap()["props"]
                    .as_object()
                    .unwrap()
                    .len(),
                16
            );
        }
        let healthy_at = "main:Catalog.Items.TabularSection.Healthy";
        let healthy = service.view(ViewRequest::new(healthy_at).unwrap());
        assert!(healthy.ok, "the call failed");
        let healthy_limits = healthy.data.as_ref().unwrap()["limits"].to_string();
        assert!(!healthy_limits.contains(".properties.Use"));
        assert!(!healthy_limits.contains(".standardAttributes"));
        assert!(
            service
                .view(
                    ViewRequest::new(&format!("{healthy_at}.StandardAttribute.LineNumber"))
                        .unwrap()
                )
                .ok
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
}

fn install_xdto_check_fixture(fixture: &RealReaderFixture) {
    let path = fixture.source.join("Configuration.xml");
    let config=fs::read_to_string(&path).unwrap().replace("</ChildObjects>","<XDTOPackage>Control</XDTOPackage><XDTOPackage>Unsupported</XDTOPackage><XDTOPackage>Paged</XDTOPackage><XDTOPackage>Mixed</XDTOPackage></ChildObjects>");
    write(&path, &config);
    for name in ["Control", "Unsupported", "Paged", "Mixed"] {
        for suffix in [".xml", "/Ext/Package.bin"] {
            let relative = format!("XDTOPackages/{name}{suffix}");
            write(
                &fixture.source.join(&relative),
                &fixture_text(&format!("acceptance/workspace-xdto-check/src/{relative}")),
            );
        }
    }
}

#[test]
fn xdto_check_preserves_complete_native_findings_and_distinguishes_profile_limits() {
    let fixture = RealReaderFixture::new();
    install_xdto_check_fixture(&fixture);
    for (name, count, error_count) in [
        ("Control", 0, 0),
        ("Unsupported", 7, 0),
        ("Paged", 55, 0),
        ("Mixed", 56, 1),
    ] {
        let at = QualifiedAddress::parse(&format!("main:XDTOPackage.{name}")).unwrap();
        let path = fixture
            .source
            .join(format!("XDTOPackages/{name}/Ext/Package.bin"));
        let before = fs::read(&path).unwrap();
        let authority = fixture.read_authority();
        let target = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!("XDTOPackage.{name}"),
        )
        .unwrap();
        let native = authority.read.xdto_payload(&target, None).unwrap();
        let (_, outcome) = authority.check_xdto(&at).unwrap();
        let checked = crate::application::v13::check::normalize_native_outcome(
            &at,
            "XDTOPackage",
            crate::application::v13::check::CheckValidator::Xdto,
            outcome.unwrap(),
        )
        .unwrap();
        let findings = native["findings"].as_array().unwrap();
        assert_eq!(findings.len(), count, "{name}");
        assert_eq!(checked.diagnostics().len(), count, "{name}");
        assert_eq!(checked.ok(), error_count == 0, "{name}");
        assert_eq!(
            checked
                .diagnostics()
                .iter()
                .filter(|d| d.severity() == "error")
                .count(),
            error_count,
            "{name}"
        );
        for (finding, diagnostic) in findings.iter().zip(checked.diagnostics()) {
            assert_eq!(
                finding["severity"], "error",
                "native/writer classification must remain unchanged"
            );
            assert_eq!(diagnostic.code(), finding["code"].as_str().unwrap());
            let offset = finding["location"]["bodyByteRange"]["start"]
                .as_u64()
                .unwrap();
            assert!(
                diagnostic
                    .message()
                    .contains(&format!("byte offset {offset} (0-based, BOM excluded)")),
                "{diagnostic:?}"
            );
            assert!(
                diagnostic
                    .message()
                    .contains(finding["message"].as_str().unwrap()),
                "{diagnostic:?}"
            );
            if diagnostic.code() == "unsupported_node" {
                assert_eq!(diagnostic.severity(), "warning");
                assert!(diagnostic.message().contains("Unica profile limitation"));
            }
        }
        assert_eq!(fs::read(path).unwrap(), before);
    }
}

#[test]
fn xdto_check_rejects_unreadable_sources_and_keeps_inherited_packages_opaque() {
    let fixture = RealReaderFixture::new();
    install_xdto_check_fixture(&fixture);
    let at = QualifiedAddress::parse("main:XDTOPackage.Control").unwrap();
    let path = fixture.source.join("XDTOPackages/Control/Ext/Package.bin");
    let body = fs::read(&path).unwrap();
    for invalid in [
        b"not XML".as_slice(),
        b"\xff",
        b"\xef\xbb\xbf\xef\xbb\xbf<package/>",
    ] {
        fs::write(&path, invalid).unwrap();
        let result = fixture.read_authority().check_xdto(&at);
        assert!(
            result.is_err(),
            "parse failures are not unsupported-node warnings"
        );
    }
    fs::write(&path, &body).unwrap();
    let descriptor = fixture.source.join("XDTOPackages/Control.xml");
    let original = fs::read_to_string(&descriptor).unwrap();
    for invalid in [
        original.replace("urn:sample", "urn:other"),
        original.replace("<Name>Control</Name>", "<Name>Other</Name>"),
        original.replace("http://v8.1c.ru/8.3/MDClasses", "urn:foreign"),
    ] {
        fs::write(&descriptor, invalid).unwrap();
        assert!(fixture.read_authority().check_xdto(&at).is_err());
    }
    fs::write(&descriptor, &original).unwrap();
    let config = fixture.source.join("Configuration.xml");
    let original_config = fs::read_to_string(&config).unwrap();
    fs::write(
        &config,
        original_config.replace("<XDTOPackage>Control</XDTOPackage>", ""),
    )
    .unwrap();
    assert_eq!(
        fixture.read_authority().check_xdto(&at).unwrap_err().code(),
        RefusalCode::NotFound
    );
    fs::write(&config, original_config).unwrap();
    fs::write(&descriptor,original.replace("</Properties>","<ExtendedConfigurationObject>99999999-9999-4999-8999-999999999999</ExtendedConfigurationObject></Properties>")).unwrap();
    fs::remove_file(&path).unwrap();
    assert!(fixture
        .extension_read_authority()
        .check_xdto(&at)
        .unwrap()
        .1
        .is_none());
    fixture.cancellation.cancel();
    assert_eq!(
        fixture.read_authority().check_xdto(&at).unwrap_err().code(),
        RefusalCode::Cancelled
    );
}

#[test]
fn xdto_check_refuses_linked_descriptor_and_package_resources() {
    use crate::composition::testing::{create_file_link_fixture_for_test, FileLinkFixtureOutcome};
    for relative in [
        "XDTOPackages/Control.xml",
        "XDTOPackages/Control/Ext/Package.bin",
    ] {
        let fixture = RealReaderFixture::new();
        install_xdto_check_fixture(&fixture);
        let path = fixture.source.join(relative);
        let bytes = fs::read(&path).unwrap();
        let foreign = fixture.root.path().join("foreign.xml");
        fs::write(&foreign, bytes).unwrap();
        fs::remove_file(&path).unwrap();
        let outcome = create_file_link_fixture_for_test(&foreign, &path).unwrap();
        if outcome != FileLinkFixtureOutcome::Created {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a file link needs the symlink privilege",
            );
            return;
        }
        let at = QualifiedAddress::parse("main:XDTOPackage.Control").unwrap();
        assert_eq!(
            fixture.read_authority().check_xdto(&at).unwrap_err().code(),
            RefusalCode::ProviderUnavailable,
            "{relative}"
        );
    }
}

#[test]
fn subsystem_text_properties_preserve_presence_and_damage_at_every_depth() {
    for (fragment, comment, explanation, damaged) in [
        ("<Comment/><Explanation/>", Some(""), Some(""), false),
        ("<Comment></Comment><Explanation></Explanation>", Some(""), Some(""), false),
        ("<Comment>  </Comment><Explanation>  </Explanation>", Some("  "), Some(""), false),
        ("<Comment>а<![CDATA[б]]>в</Comment><Explanation><v8:item><v8:lang>en</v8:lang><v8:content>English</v8:content></v8:item><v8:item><v8:lang>ru</v8:lang><v8:content>Пояснение</v8:content></v8:item></Explanation>", Some("абв"), Some("Пояснение"), false),
        ("", None, None, false),
        ("<Comment><Bad/></Comment><Explanation>garbage</Explanation>", None, None, true),
        ("<Comment attr=\"x\"/><Explanation attr=\"x\"/>", None, None, true),
        ("<Comment/><Comment/><Explanation/><Explanation/>", None, None, true),
        ("<Comment xmlns=\"urn:foreign\"/><Explanation xmlns=\"urn:foreign\"/>", None, None, true),
        ("<Comment><Bad/></Comment><Explanation><v8:item><v8:lang>ru</v8:lang></v8:item></Explanation>", None, None, true),
        ("<Comment/><Explanation><item xmlns=\"urn:foreign\"><lang>ru</lang><content>x</content></item></Explanation>", Some(""), None, true),
    ] {
        let fixture = RealReaderFixture::new();
        let paths = ["Subsystems/Sales.xml", "Subsystems/Sales/Subsystems/Orders.xml"];
        for (path, name, child) in [(paths[0], "Sales", "Orders"), (paths[1], "Orders", "Returns")] {
            let xml = subsystem_descriptor(name, &["Catalog.Items"], &[child])
                .replace("<Synonym/>", &format!("<Synonym/>{fragment}"));
            write(&fixture.source.join(path), &xml);
            let ci = fixture.source.join(path.trim_end_matches(".xml")).join("Ext/CommandInterface.xml");
            write(&ci, r#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"/>"#);
        }
        let service = fixture.view_service();
        for (path, at) in paths.into_iter().zip(["main:Subsystem.Sales", "main:Subsystem.Sales.Subsystem.Orders"]) {
            let before = fs::read(fixture.source.join(path)).unwrap();
            let result = service.view(ViewRequest::new(at).unwrap());
            assert!(result.ok, "{fragment}: the call failed");
            let data = result.data.as_ref().unwrap();
            assert_eq!(data["props"]["Comment"], json!(comment), "{at}: {fragment}");
            assert_eq!(data["props"]["Explanation"], json!(explanation), "{at}: {fragment}");
            let limits = data["limits"].to_string();
            assert_eq!(limits.contains("properties."), damaged, "{at}: {fragment}: unexpected answer");
            for branch in ["Relation", "Subsystem", "Interface"] {
                assert_eq!(view_branch_count(&result, &format!("{at}.{branch}")), Some(1), "{at}: {fragment}");
                assert!(service.view(ViewRequest::new(&format!("{at}.{branch}")).unwrap()).ok);
            }
            assert_eq!(fs::read(fixture.source.join(path)).unwrap(), before);
        }
    }
}

fn install_common_template_fixture(fixture: &RealReaderFixture) {
    let config_path = fixture.source.join("Configuration.xml");
    let config = fs::read_to_string(&config_path).unwrap().replace("</ChildObjects>", "<CommonTemplate>Print</CommonTemplate><CommonTemplate>Schema</CommonTemplate><CommonTemplate>Html</CommonTemplate></ChildObjects>");
    write(&config_path, &config);
    for name in ["Print", "Schema", "Html"] {
        for suffix in [".xml", "/Ext/Template.xml"] {
            let relative = format!("CommonTemplates/{name}{suffix}");
            write(
                &fixture.source.join(&relative),
                &fixture_text(&format!(
                    "acceptance/workspace-common-template/src/{relative}"
                )),
            );
        }
    }
}

#[test]
fn common_template_empty_synonyms_have_no_value_like_other_metadata() {
    for synonym in [
        "",
        "<Synonym/>",
        "<Synonym>   </Synonym>",
        "<Synonym><v8:item><v8:lang>ru</v8:lang><v8:content/></v8:item></Synonym>",
    ] {
        let fixture = RealReaderFixture::new();
        install_common_template_fixture(&fixture);
        let descriptor = fixture.source.join("CommonTemplates/Print.xml");
        let original = fs::read_to_string(&descriptor).unwrap();
        let start = original.find("<Synonym>").unwrap();
        let end = original.find("</Synonym>").unwrap() + "</Synonym>".len();
        let mut modified = original;
        modified.replace_range(start..end, synonym);
        write(&descriptor, &modified);
        let result = fixture
            .view_service()
            .view(ViewRequest::new("main:CommonTemplate.Print").unwrap());
        assert!(result.ok, "{synonym:?}: the call failed");
        let data = result.data.unwrap();
        assert!(
            data["props"]["Synonym"].is_null(),
            "an empty synonym publishes no Synonym value"
        );
        assert_eq!(data["props"]["format"], "SpreadsheetDocument");
    }
}

#[test]
fn common_template_damaged_synonym_keeps_independent_template_facts() {
    for synonym in [
        "<Synonym><Bad/></Synonym>",
        "<Synonym>broken</Synonym>",
        "<Synonym xmlns=\"urn:foreign\"/>",
        "<Synonym/><Synonym/>",
        "<Synonym unexpected=\"yes\"/>",
    ] {
        let fixture = RealReaderFixture::new();
        install_common_template_fixture(&fixture);
        let descriptor = fixture.source.join("CommonTemplates/Print.xml");
        let mut xml = fs::read_to_string(&descriptor).unwrap();
        let start = xml.find("<Synonym>").unwrap();
        let end = xml.find("</Synonym>").unwrap() + "</Synonym>".len();
        xml.replace_range(start..end, synonym);
        write(&descriptor, &xml);
        let service = fixture.view_service();
        let result = service.view(ViewRequest::new("main:CommonTemplate.Print").unwrap());
        assert!(result.ok, "{synonym}: the call failed");
        let data = result.data.unwrap();
        assert_eq!(data["title"], "Print");
        assert!(
            data["props"]["Synonym"].is_null(),
            "a damaged synonym publishes no Synonym value"
        );
        assert_eq!(data["props"]["format"], "SpreadsheetDocument");
        assert!(
            data["limits"].to_string().contains("properties.Synonym"),
            "{synonym}: unexpected answer"
        );
        assert!(
            service
                .view(ViewRequest::new("main:CommonTemplate.Print.Area.A.Body").unwrap())
                .ok
        );
        assert_eq!(fs::read_to_string(&descriptor).unwrap(), xml);
    }
}

#[test]
fn common_template_exposes_mxl_areas_body_and_parameters_through_public_view() {
    let fixture = RealReaderFixture::new();
    install_common_template_fixture(&fixture);
    let service = fixture.view_service();
    let at = "main:CommonTemplate.Print";
    let template = service.view(ViewRequest::new(at).unwrap());
    assert!(template.ok, "the call failed");
    let data = template.data.unwrap();
    assert_eq!(data["props"]["format"], "SpreadsheetDocument");
    assert_eq!(data["props"]["Synonym"], "Общий Print");
    assert!(data["branches"]
        .as_array()
        .unwrap()
        .contains(&json!({"at":format!("{at}.Area"),"count":2})));
    let area = service.view(ViewRequest::new(&format!("{at}.Area.A")).unwrap());
    assert!(area.ok, "the call failed");
    let body = service.view(ViewRequest::new(&format!("{at}.Area.A.Body")).unwrap());
    assert!(body.ok, "the call failed");
    assert_eq!(body.data.as_ref().unwrap()["items"][0]["text"], "Changed");
    let parameter =
        service.view(ViewRequest::new(&format!("{at}.Area.A.Parameter.Probe")).unwrap());
    assert!(parameter.ok, "the call failed");
}

#[test]
fn common_templates_share_mxl_and_dcs_readers_and_validation_inputs() {
    let fixture = RealReaderFixture::new();
    install_common_template_fixture(&fixture);
    for (common, nested) in [("Print", "Print"), ("Schema", "MainSchema")] {
        let bytes = fs::read(
            fixture
                .source
                .join(format!("CommonTemplates/{common}/Ext/Template.xml")),
        )
        .unwrap();
        fs::write(
            fixture.source.join(format!(
                "Reports/ParityReport/Templates/{nested}/Ext/Template.xml"
            )),
            bytes,
        )
        .unwrap();
    }
    let service = fixture.view_service();
    for (common, nested, suffix) in [
        ("Print", "Print", ".Area"),
        ("Print", "Print", ".Area.A"),
        ("Print", "Print", ".Area.A.Body"),
        ("Print", "Print", ".Area.A.Parameter"),
        ("Print", "Print", ".Area.A.Parameter.Probe"),
        ("Schema", "MainSchema", ".DataSet.MainData"),
        ("Schema", "MainSchema", ".DataSet.MainData.Query"),
        ("Schema", "MainSchema", ".Parameter"),
        ("Schema", "MainSchema", ".Parameter.Period"),
        ("Schema", "MainSchema", ".Item"),
        ("Schema", "MainSchema", ".Setting.Основной.Item"),
    ] {
        let a = service
            .view(ViewRequest::new(&format!("main:CommonTemplate.{common}{suffix}")).unwrap());
        let b = service.view(
            ViewRequest::new(&format!(
                "main:Report.ParityReport.Template.{nested}{suffix}"
            ))
            .unwrap(),
        );
        assert!(a.ok && b.ok, "{suffix}: the call failed");
        let normalize = |data: &Option<serde_json::Value>| {
            serde_json::to_string(data)
                .unwrap()
                .replace(&format!("main:CommonTemplate.{common}"), "OWNER")
                .replace(
                    &format!("main:Report.ParityReport.Template.{nested}"),
                    "OWNER",
                )
        };
        assert_eq!(normalize(&a.data), normalize(&b.data), "{suffix}");
    }
    let authority = fixture.read_authority();
    let at = QualifiedAddress::parse("main:CommonTemplate.Print").unwrap();
    let input = authority.mxl_validation_input(&at).unwrap();
    assert!(
        crate::infrastructure::native_operations::mxl_composition::validate_mxl_input(
            &serde_json::Map::new(),
            &input.text,
            input.artifact
        )
        .ok
    );
    let schema = QualifiedAddress::parse("main:CommonTemplate.Schema").unwrap();
    assert!(authority.dcs_validation_input(&schema).is_ok());
    assert!(authority.dcs_validation_input(&at).is_err());
    assert!(authority.mxl_validation_input(&schema).is_err());
}

#[test]
fn common_template_descriptor_and_registration_boundaries_fail_closed() {
    let fixture = RealReaderFixture::new();
    install_common_template_fixture(&fixture);
    let descriptor = fixture.source.join("CommonTemplates/Print.xml");
    let original = fs::read_to_string(&descriptor).unwrap();
    for variant in [
        "root-qname",
        "artifact-qname",
        "wrong-name",
        "missing-type",
        "unknown-type",
        "duplicate-type",
        "nested-type",
        "missing-properties",
    ] {
        let text = match variant {
            "root-qname" => original
                .replace(
                    "<MetaDataObject ",
                    "<MetaDataObject xmlns:x=\"urn:foreign\" ",
                )
                .replace("<MetaDataObject ", "<x:MetaDataObject ")
                .replace("</MetaDataObject>", "</x:MetaDataObject>"),
            "artifact-qname" => {
                original.replace("<CommonTemplate ", "<CommonTemplate xmlns=\"urn:foreign\" ")
            }
            "wrong-name" => original.replace("<Name>Print</Name>", "<Name>Other</Name>"),
            "missing-type" => {
                original.replace("<TemplateType>SpreadsheetDocument</TemplateType>", "")
            }
            "unknown-type" => original.replace("SpreadsheetDocument", "FutureFormat"),
            "duplicate-type" => original.replace(
                "</TemplateType>",
                "</TemplateType><TemplateType>SpreadsheetDocument</TemplateType>",
            ),
            "nested-type" => {
                original.replace("SpreadsheetDocument", "<Value>SpreadsheetDocument</Value>")
            }
            "missing-properties" => original
                .replace("<Properties>", "<Other>")
                .replace("</Properties>", "</Other>"),
            _ => unreachable!(),
        };
        fs::write(&descriptor, text).unwrap();
        let view = fixture
            .view_service()
            .view(ViewRequest::new("main:CommonTemplate.Print").unwrap());
        assert!(!view.ok, "{variant}: the call was expected to be refused");
        assert_eq!(
            view.diagnostics[0]["code"], "provider_unavailable",
            "{variant}"
        );
    }
    fs::write(&descriptor, &original).unwrap();
    let config_path = fixture.source.join("Configuration.xml");
    let config = fs::read_to_string(&config_path).unwrap();
    fs::write(
        &config_path,
        config.replace("<CommonTemplate>Print</CommonTemplate>", ""),
    )
    .unwrap();
    for at in [
        "main:CommonTemplate.Print",
        "main:CommonTemplate.Print.Area.A",
        "main:CommonTemplate.Print.Area.A.Body",
    ] {
        let view = fixture.view_service().view(ViewRequest::new(at).unwrap());
        assert!(!view.ok, "{at}: the call was expected to be refused");
        assert_eq!(view.diagnostics[0]["code"], "not_found");
    }
    fs::write(&config_path, config).unwrap();
    fs::remove_file(&descriptor).unwrap();
    for at in ["main:CommonTemplate.Print", "main:CommonTemplate"] {
        let view = fixture.view_service().view(ViewRequest::new(at).unwrap());
        assert!(!view.ok, "{at}: the call was expected to be refused");
        assert_eq!(view.diagnostics[0]["code"], "provider_unavailable");
    }
}

#[test]
fn common_opaque_and_inherited_templates_do_not_open_a_body() {
    let fixture = RealReaderFixture::new();
    install_common_template_fixture(&fixture);
    let html = fixture
        .view_service()
        .view(ViewRequest::new("main:CommonTemplate.Html").unwrap());
    assert!(html.ok, "the call failed");
    let data = html.data.unwrap();
    assert_eq!(data["props"]["format"], "HTMLDocument");
    assert_eq!(data["props"]["Synonym"], "Общий Html");
    assert!(data.get("branches").is_none());
    for suffix in [".Area", ".DataSet", ".Parameter", ".Item", ".Area.A.Body"] {
        let view = fixture
            .view_service()
            .view(ViewRequest::new(&format!("main:CommonTemplate.Html{suffix}")).unwrap());
        assert!(!view.ok, "{suffix}: the call was expected to be refused");
        assert_eq!(view.diagnostics[0]["code"], "not_found");
    }
    let descriptor = fixture.source.join("CommonTemplates/Print.xml");
    let original = fs::read_to_string(&descriptor).unwrap();
    let payload = fixture
        .source
        .join("CommonTemplates/Print/Ext/Template.xml");
    let original_payload = fs::read(&payload).unwrap();
    for format in [
        "SpreadsheetDocument",
        "HTMLDocument",
        "DataCompositionAppearanceTemplate",
    ] {
        fs::write(&descriptor, original.replace("SpreadsheetDocument", format).replace("</Properties>", "<ExtendedConfigurationObject>99999999-9999-4999-8999-999999999999</ExtendedConfigurationObject></Properties>")).unwrap();
        fs::write(&payload, &original_payload).unwrap();
        let local = fixture
            .extension_view_service()
            .view(ViewRequest::new("main:CommonTemplate.Print").unwrap());
        assert!(local.ok, "{format}: the call failed");
        let data = local.data.unwrap();
        assert_eq!(data["props"]["format"], format);
        if format != "SpreadsheetDocument" {
            assert!(data.get("branches").is_none(), "opaque {format}");
        }
        fs::remove_file(&payload).unwrap();
        let view = fixture
            .extension_view_service()
            .view(ViewRequest::new("main:CommonTemplate.Print").unwrap());
        assert!(view.ok, "{format}: the call failed");
        let data = view.data.unwrap();
        assert_eq!(
            data["props"]["content"], "inherited",
            "{format}: unexpected answer"
        );
        assert!(
            data["props"]["format"].is_null(),
            "{format}: unexpected answer"
        );
        assert!(data.get("branches").is_none());
    }
}

/// #1924: заимствованный план обмена без своего `Ext/Content.xml` не знает
/// состава в расширении — его несёт план родителя. Чтение не выдаёт пустой
/// список за известный состав.
#[test]
fn a_borrowed_exchange_plan_without_its_own_content_does_not_read_as_empty() {
    let fixture = RealReaderFixture::new();
    let configuration = fs::read_to_string(fixture.source.join("Configuration.xml")).unwrap();
    write(
        &fixture.source.join("Configuration.xml"),
        &configuration.replace(
            "</ChildObjects>",
            "<ExchangePlan>Узлы</ExchangePlan><ExchangePlan>Собственный</ExchangePlan></ChildObjects>",
        ),
    );
    // Собственный план расширения без `Ext/Content.xml` — доказанно пустой состав.
    write(
        &fixture.source.join("ExchangePlans/Собственный.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<ExchangePlan uuid="2f0e0000-0000-4000-8000-000000000302">
		<Properties>
			<Name>Собственный</Name>
			<Comment/>
		</Properties>
		<ChildObjects/>
	</ExchangePlan>
</MetaDataObject>"#,
    );
    write(
        &fixture.source.join("ExchangePlans/Узлы.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20">
	<ExchangePlan uuid="2f0e0000-0000-4000-8000-000000000301">
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>Узлы</Name>
			<Comment/>
			<ExtendedConfigurationObject>1f0e0000-0000-4000-8000-000000000301</ExtendedConfigurationObject>
		</Properties>
		<ChildObjects/>
	</ExchangePlan>
</MetaDataObject>"#,
    );
    let service = fixture.extension_view_service();
    let content_limit = "relations.content is absent; the relation list may be incomplete";
    let limits = |at: &str| {
        let view = service.view(ViewRequest::new(at).unwrap());
        assert!(view.ok, "{at}");
        view.data.unwrap()["limits"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    };
    for at in ["main:ExchangePlan.Узлы", "main:ExchangePlan.Узлы.Relation"] {
        assert!(limits(at).contains(&json!(content_limit)), "{at}");
    }
    for at in [
        "main:ExchangePlan.Собственный",
        "main:ExchangePlan.Собственный.Relation",
    ] {
        assert!(!limits(at).contains(&json!(content_limit)), "{at}");
    }
}

#[test]
fn xdto_restricted_value_view_returns_named_and_anonymous_facets() {
    let fixture = RealReaderFixture::new();
    let package = fixture
        .source
        .join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin");
    let original = fs::read_to_string(&package).unwrap();
    let document = roxmltree::Document::parse(original.trim_start_matches('\u{feff}')).unwrap();
    let namespace = document
        .root_element()
        .attribute("targetNamespace")
        .unwrap();
    let source = include_str!("../../../../../tests/fixtures/xdto/restricted-values/Package.bin")
        .replace("urn:unica:anonymous", namespace);
    fs::write(&package, source.as_bytes()).unwrap();
    let service = fixture.view_service();
    for (suffix, facets, anonymous, base) in [
        ("Type.КодПримера", json!({"maxLength":10}), false, "string"),
        (
            "Type.СуммаПримера",
            json!({"totalDigits":5,"fractionDigits":2,"minInclusive":"0","maxInclusive":"999.99"}),
            false,
            "decimal",
        ),
        (
            "Type.Пример.Property.АнонимныйКод",
            json!({"maxLength":10}),
            true,
            "string",
        ),
    ] {
        let result = service.view(
            ViewRequest::new(&format!("main:XDTOPackage.EnterpriseData_1_17_3.{suffix}")).unwrap(),
        );
        assert!(result.ok, "the call failed");
        let data = result.data.unwrap();
        let props = &data["props"];
        assert_eq!(props["facets"], facets);
        let actual_base = &props["base"];
        assert!(actual_base.is_object(), "base arrives as nested JSON");
        assert_eq!(actual_base["local"], base);
        assert_eq!(actual_base["namespace"], "http://www.w3.org/2001/XMLSchema");
        if anonymous {
            assert_eq!(props["anonymousType"], "ValueType");
        }
    }
    assert_eq!(fs::read(&package).unwrap(), source.as_bytes());
}

#[test]
fn xdto_restricted_value_view_reports_oversized_facets() {
    let fixture = RealReaderFixture::new();
    let package = fixture
        .source
        .join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin");
    let original = fs::read_to_string(&package).unwrap();
    let document = roxmltree::Document::parse(original.trim_start_matches('\u{feff}')).unwrap();
    let namespace = document
        .root_element()
        .attribute("targetNamespace")
        .unwrap();
    let source = include_str!("../../../../../tests/fixtures/xdto/restricted-values/Package.bin")
        .replace("urn:unica:anonymous", namespace)
        .replace("999.99", &"9".repeat(3000))
        .replace(
            "base=\"xs:string\" maxLength=\"10\"",
            &format!("base=\"xs:decimal\" maxInclusive=\"{}\"", "9".repeat(3000)),
        );
    fs::write(&package, source.as_bytes()).unwrap();
    let service = fixture.view_service();
    for suffix in ["Type.СуммаПримера", "Type.Пример.Property.АнонимныйКод"]
    {
        let result = service.view(
            ViewRequest::new(&format!("main:XDTOPackage.EnterpriseData_1_17_3.{suffix}")).unwrap(),
        );
        assert!(result.ok, "the call failed");
        let data = result.data.unwrap();
        assert!(data["props"].get("facets").is_none());
        assert!(data["limits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().is_some_and(|s| s.contains("facets"))));
    }
    assert_eq!(fs::read(&package).unwrap(), source.as_bytes());
}

#[test]
fn an_open_buffer_is_read_in_place_of_its_module_file_and_the_file_is_untouched() {
    use unica_workspace::documents::{DocumentOverlay, DocumentText};

    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().canonicalize().unwrap();
    let module = Path::new("CommonModules/Общий/Ext/Module.bsl");
    fs::create_dir_all(workspace.as_path().join(module).parent().unwrap()).unwrap();
    fs::write(workspace.as_path().join(module), "\u{feff}// диск\n").unwrap();
    let target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "CommonModule.Общий.Module")
            .unwrap();
    let absent =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "CommonModule.Новый.Module")
            .unwrap();
    let overlay = DocumentOverlay::default();
    for relative in [module, Path::new("CommonModules/Новый/Ext/Module.bsl")] {
        overlay
            .put(
                "editor",
                "main",
                relative,
                DocumentText {
                    version: 3,
                    text: Arc::from("\u{feff}// буфер\n// вторая\n"),
                },
            )
            .unwrap();
    }
    let reader = |documents| {
        let root =
            Arc::new(unica_workspace::read::open_source_directory(workspace.as_path()).unwrap());
        let reader =
            ProviderReadAuthority::new("main", "actor-main", SourceSetKind::Configuration, root);
        match documents {
            Some(documents) => reader.with_documents(documents),
            None => reader,
        }
    };

    let editor = reader(Some(overlay.session_documents("editor", "main").unwrap()));
    assert_eq!(
        editor.module_source(&target).unwrap().as_deref(),
        Some("// буфер\n// вторая\n")
    );
    assert!(editor.module_source_exists(&absent).unwrap());
    assert_eq!(
        editor.module_source(&absent).unwrap().as_deref(),
        Some("// буфер\n// вторая\n")
    );

    // Every read of the module in this session sees the same buffer.
    let lines = editor.module_line_count(&target, || Ok(())).unwrap();
    assert_eq!(
        lines,
        reader(None)
            .module_line_count(&target, || Ok(()))
            .unwrap()
            .map(|n| n + 1)
    );

    // Another session and an agent call read the disk.
    for other in [
        Some(overlay.session_documents("agent", "main").unwrap()),
        None,
    ] {
        let other = reader(other);
        assert_eq!(
            other.module_source(&target).unwrap().as_deref(),
            Some("// диск\n")
        );
        assert!(!other.module_source_exists(&absent).unwrap());
    }
    assert_eq!(
        fs::read_to_string(workspace.as_path().join(module)).unwrap(),
        "\u{feff}// диск\n"
    );
    assert!(!workspace
        .as_path()
        .join("CommonModules/Новый")
        .join("Ext/Module.bsl")
        .exists());
}
