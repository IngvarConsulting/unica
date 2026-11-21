//! Базовые факты корня у видов без собственного профиля чтения: синоним
//! и выбранные свойства платформы (#1568), настройки общей команды (#1616).
//!
//! Вид без записи в профиле не получает здесь ничего. Каждое свойство
//! читается независимо: порча одного не скрывает остальные и не становится
//! отсутствием, а называется в `unreadable` своим полем.

use crate::info_projection::{
    direct_meta_info_property, direct_text_content, localized_property_as_nstr,
    meta_info_property_value_is_valid, node_is_empty_placeholder,
    observed_type_container_is_strict_but_unmodelled, parse_observed_metadata_type_container_node,
};
use roxmltree::{Document, Node};
use serde_json::{json, Value};
use unica_metadata::{MetaInfoPropertyValueKind, MetaObservedPropertyValue};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fact {
    /// Многоязычная строка, публикуется в форме НСтр.
    Localized,
    Boolean,
    UnsignedInteger,
    /// Непустое слово без вложенных элементов.
    Text,
    ExactText,
    /// Одно из значений перечисления платформы.
    Enum(&'static [&'static str]),
    /// Описание типа в форме, которую читает и `Type` константы; контейнер
    /// называется именем свойства (`Type`, `CommandParameterType`).
    Type,
    /// Картинка: ссылка `xr:Ref` и признак `xr:LoadTransparent`.
    Picture,
}

/// ButtonRepresentation платформы.
const REPRESENTATION: Fact = Fact::Enum(&["Auto", "Text", "Picture", "PictureAndText"]);
const PARAMETER_USE_MODE: Fact = Fact::Enum(&["Single", "Multiple"]);

/// Закрытый профиль: вид, затем его свойства в порядке XML.
const PROFILE: &[(&str, &[(&str, Fact)])] = &[
    (
        "CommonPicture",
        &[
            ("Synonym", Fact::Localized),
            ("AvailabilityForChoice", Fact::Boolean),
        ],
    ),
    (
        "SessionParameter",
        &[("Synonym", Fact::Localized), ("Type", Fact::Type)],
    ),
    (
        "FilterCriterion",
        &[("Synonym", Fact::Localized), ("Type", Fact::Type)],
    ),
    (
        "CommonAttribute",
        &[("Synonym", Fact::Localized), ("Type", Fact::Type)],
    ),
    ("SettingsStorage", &[("Synonym", Fact::Localized)]),
    ("FunctionalOption", &[("Synonym", Fact::Localized)]),
    (
        "FunctionalOptionsParameter",
        &[("Synonym", Fact::Localized)],
    ),
    (
        "Bot",
        &[("Synonym", Fact::Localized), ("Predefined", Fact::Boolean)],
    ),
    (
        "CommonCommand",
        &[
            ("Synonym", Fact::Localized),
            ("Group", Fact::Text),
            ("Representation", REPRESENTATION),
            ("ToolTip", Fact::Localized),
            ("Picture", Fact::Picture),
            ("CommandParameterType", Fact::Type),
            ("ParameterUseMode", PARAMETER_USE_MODE),
            ("ModifiesData", Fact::Boolean),
        ],
    ),
    (
        "DocumentNumerator",
        &[
            ("Synonym", Fact::Localized),
            ("NumberLength", Fact::UnsignedInteger),
        ],
    ),
    ("IntegrationService", &[("Synonym", Fact::Localized)]),
    ("WSReference", &[("Synonym", Fact::Localized)]),
    ("ExternalDataSource", &[("Synonym", Fact::Localized)]),
    (
        "Table",
        &[
            ("Synonym", Fact::Localized),
            ("TableType", Fact::Enum(&["Table", "Expression"])),
            ("NameInDataSource", Fact::ExactText),
            ("ExpressionInDataSource", Fact::ExactText),
            ("ReadOnly", Fact::Boolean),
        ],
    ),
    (
        "Cube",
        &[
            ("Synonym", Fact::Localized),
            ("NameInDataSource", Fact::ExactText),
        ],
    ),
    (
        "DimensionTable",
        &[
            ("Synonym", Fact::Localized),
            ("NameInDataSource", Fact::ExactText),
        ],
    ),
];

/// Виды, у которых профиль читает базовые факты корня.
pub fn identity_root_fact_kinds() -> impl Iterator<Item = &'static str> {
    PROFILE.iter().map(|(kind, _)| *kind)
}

fn observe(
    properties: Node<'_, '_>,
    kind: &str,
    name: &str,
    fact: Fact,
) -> Result<Option<Value>, String> {
    if fact == Fact::Localized {
        return localized_property_as_nstr(properties, kind, name)
            .map(|value| value.map(|value| json!(MetaObservedPropertyValue::Text { value })));
    }
    let Some(node) =
        direct_meta_info_property(properties, kind, name).map_err(|error| error.message)?
    else {
        return Ok(None);
    };
    let malformed = || format!("{kind} contains a malformed {name}");
    match fact {
        Fact::Localized => unreachable!("handled above"),
        Fact::Boolean => {
            if !meta_info_property_value_is_valid(node, MetaInfoPropertyValueKind::Boolean) {
                return Err(malformed());
            }
            Ok(Some(json!(MetaObservedPropertyValue::Boolean {
                value: direct_text_content(node) == "true",
            })))
        }
        Fact::UnsignedInteger => {
            if !meta_info_property_value_is_valid(node, MetaInfoPropertyValueKind::UnsignedInteger)
            {
                return Err(malformed());
            }
            let value = direct_text_content(node)
                .parse::<u32>()
                .map_err(|_| malformed())?;
            Ok(Some(json!(MetaObservedPropertyValue::UnsignedInteger {
                value
            })))
        }
        Fact::Text | Fact::ExactText => {
            let value = direct_text_content(node);
            if !meta_info_property_value_is_valid(node, MetaInfoPropertyValueKind::String)
                || (fact == Fact::Text && value.trim() != value)
            {
                return Err(malformed());
            }
            // Пустое значение свойства не создаёт.
            Ok((!value.is_empty()).then(|| json!(MetaObservedPropertyValue::Text { value })))
        }
        Fact::Enum(values) => {
            let value = direct_text_content(node);
            if !meta_info_property_value_is_valid(node, MetaInfoPropertyValueKind::String)
                || !values.contains(&value.as_str())
            {
                return Err(malformed());
            }
            Ok(Some(json!(MetaObservedPropertyValue::Text { value })))
        }
        Fact::Type => observe_type(node, kind, name),
        Fact::Picture => crate::info_projection::parse_picture_node(node)
            .map_err(|_| malformed())?
            .map(|picture| serde_json::to_value(picture).map_err(|error| error.to_string()))
            .transpose(),
    }
}

fn observe_type(container: Node<'_, '_>, kind: &str, name: &str) -> Result<Option<Value>, String> {
    if node_is_empty_placeholder(container) {
        return Ok(None);
    }
    match parse_observed_metadata_type_container_node(container) {
        Ok(observed) => serde_json::to_value(observed)
            .map(Some)
            .map_err(|error| error.to_string()),
        // Корректный, но не моделируемый тип платформы не сводится к модели
        // записи и не выдаётся за отсутствие: ответ называет предел.
        Err(diagnostic) if observed_type_container_is_strict_but_unmodelled(container) => {
            Err(format!(
                "{kind} type composition is incomplete: {}",
                diagnostic.message
            ))
        }
        Err(diagnostic) => Err(format!(
            "{kind} contains a malformed {name}: {}",
            diagnostic.message
        )),
    }
}

/// Добавляет в `payload` факты профиля вида `kind`. Вид вне профиля
/// оставляет `payload` как есть.
pub fn enrich_identity_root_facts(
    bytes: &[u8],
    kind: &str,
    payload: &mut Value,
) -> Result<(), String> {
    let Some((_, facts)) = PROFILE.iter().find(|(profiled, _)| *profiled == kind) else {
        return Ok(());
    };
    let text = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
    let document =
        Document::parse(text.trim_start_matches('\u{feff}')).map_err(|error| error.to_string())?;
    let properties = crate::descriptor_comment::descriptor_properties(&document, kind)?;
    let mut observed = Vec::new();
    let mut unreadable = Vec::new();
    for &(name, fact) in *facts {
        match observe(properties, kind, name, fact) {
            Ok(Some(value)) => observed.push(json!({"key": name, "value": value})),
            Ok(None) => {}
            Err(message) => unreadable.push(json!({
                "field": format!("properties.{name}"),
                "message": message,
            })),
        }
    }
    for (key, values) in [("properties", observed), ("unreadable", unreadable)] {
        if values.is_empty() {
            continue;
        }
        if let Some(existing) = payload[key].as_array_mut() {
            existing.extend(values);
        } else {
            payload[key] = json!(values);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

    fn descriptor(kind: &str, properties: &str) -> Vec<u8> {
        format!(
            "<MetaDataObject xmlns=\"{MD}\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" \
             xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" \
             xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" \
             xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\">\
             <{kind}><Properties><Name>Probe</Name>{properties}</Properties></{kind}>\
             </MetaDataObject>"
        )
        .into_bytes()
    }

    fn read(kind: &str, properties: &str) -> Value {
        let mut payload = json!({"properties": [{"key": "Comment", "value": "kept"}]});
        enrich_identity_root_facts(&descriptor(kind, properties), kind, &mut payload).unwrap();
        payload
    }

    fn property<'a>(payload: &'a Value, key: &str) -> Option<&'a Value> {
        payload["properties"]
            .as_array()?
            .iter()
            .find(|property| property["key"] == key)
            .map(|property| &property["value"])
    }

    fn unreadable(payload: &Value) -> Vec<String> {
        payload["unreadable"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|entry| entry["field"].as_str().unwrap().to_string())
            .collect()
    }

    const SYNONYM: &str =
        "<Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Проба</v8:content></v8:item></Synonym>";

    /// rule: INV.SOURCE.METADATA-NODE-PROPERTIES
    #[test]
    fn every_profiled_kind_reads_its_synonym_as_nstr() {
        for kind in identity_root_fact_kinds() {
            let payload = read(kind, SYNONYM);
            assert_eq!(
                property(&payload, "Synonym"),
                Some(&json!({"kind": "text", "value": "ru = 'Проба'"})),
                "{kind}"
            );
            assert_eq!(
                property(&payload, "Comment"),
                Some(&json!("kept")),
                "{kind}"
            );
            assert!(unreadable(&payload).is_empty(), "{kind}");
        }
    }

    #[test]
    fn scalar_facts_keep_their_platform_type() {
        let picture = read(
            "CommonPicture",
            "<AvailabilityForChoice>false</AvailabilityForChoice>",
        );
        assert_eq!(
            property(&picture, "AvailabilityForChoice"),
            Some(&json!({"kind": "boolean", "value": false}))
        );
        let bot = read("Bot", "<Predefined>false</Predefined>");
        assert_eq!(
            property(&bot, "Predefined"),
            Some(&json!({"kind": "boolean", "value": false}))
        );
        let numerator = read("DocumentNumerator", "<NumberLength>11</NumberLength>");
        assert_eq!(
            property(&numerator, "NumberLength"),
            Some(&json!({"kind": "unsignedInteger", "value": 11}))
        );
        let command = read("CommonCommand", "<Group>NavigationPanelOrdinary</Group>");
        assert_eq!(
            property(&command, "Group"),
            Some(&json!({"kind": "text", "value": "NavigationPanelOrdinary"}))
        );
    }

    /// rule: INV.SOURCE.METADATA-NODE-PROPERTIES
    #[test]
    fn type_keeps_every_variant_and_qualifier() {
        let payload = read(
            "SessionParameter",
            "<Type><v8:Type>xs:boolean</v8:Type><v8:Type>xs:string</v8:Type>\
             <v8:StringQualifiers><v8:Length>1</v8:Length>\
             <v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers></Type>",
        );
        assert_eq!(
            property(&payload, "Type"),
            Some(&json!({"variants": [
                {"kind": "boolean"},
                {"kind": "string", "length": 1, "allowedLength": "variable"},
            ], "mutationCapability": "editable"}))
        );
        let references = read(
            "FilterCriterion",
            "<Type><v8:Type>cfg:CatalogRef.Partners</v8:Type>\
             <v8:Type>cfg:CatalogRef.Contacts</v8:Type></Type>",
        );
        assert_eq!(
            property(&references, "Type").unwrap()["variants"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    /// rule: INV.SOURCE.OBSERVED-TYPE-CAPABILITY
    #[test]
    fn an_unmodelled_platform_type_is_a_limit_not_an_absence() {
        let payload = read(
            "SessionParameter",
            "<Type><v8:Type>v8:FixedStructure</v8:Type></Type>",
        );
        assert_eq!(property(&payload, "Type"), None);
        assert_eq!(unreadable(&payload), ["properties.Type"]);
        assert!(payload["unreadable"][0]["message"]
            .as_str()
            .unwrap()
            .contains("incomplete"));
    }

    #[test]
    fn absent_or_empty_values_create_no_property() {
        for (kind, properties) in [
            ("CommonPicture", ""),
            ("SessionParameter", "<Synonym/><Type/>"),
            ("CommonCommand", "<Group/>"),
            ("Bot", ""),
        ] {
            let payload = read(kind, properties);
            assert_eq!(
                payload["properties"].as_array().unwrap().len(),
                1,
                "{kind}: {payload}"
            );
            assert!(unreadable(&payload).is_empty(), "{kind}: {payload}");
        }
    }

    #[test]
    fn a_damaged_present_value_is_unreadable_and_spares_the_rest() {
        for (kind, damaged, field) in [
            ("CommonPicture", "<AvailabilityForChoice>maybe</AvailabilityForChoice>", "AvailabilityForChoice"),
            ("Bot", "<Predefined><x/></Predefined>", "Predefined"),
            ("DocumentNumerator", "<NumberLength>-1</NumberLength>", "NumberLength"),
            ("CommonCommand", "<Group attr=\"x\">A</Group>", "Group"),
            ("CommonCommand", "<Group>A</Group><Group>B</Group>", "Group"),
            ("SessionParameter", "<Type><v8:Type>xs:string</v8:Type><v8:StringQualifiers><v8:Length>x</v8:Length></v8:StringQualifiers></Type>", "Type"),
            ("FilterCriterion", "<Synonym><v8:item><v8:lang>ru</v8:lang></v8:item></Synonym>", "Synonym"),
        ] {
            let payload = read(kind, &format!("{SYNONYM}{damaged}").replacen(
                SYNONYM,
                if field == "Synonym" { "" } else { SYNONYM },
                1,
            ));
            assert_eq!(unreadable(&payload), [format!("properties.{field}")], "{kind}: {damaged}");
            assert_eq!(property(&payload, field), None, "{kind}: {damaged}");
            assert_eq!(property(&payload, "Comment"), Some(&json!("kept")));
            if field != "Synonym" {
                assert!(property(&payload, "Synonym").is_some(), "{kind}: {damaged}");
            }
        }
    }

    #[test]
    fn a_kind_outside_the_profile_is_left_as_is() {
        let mut payload = json!({"properties": []});
        enrich_identity_root_facts(&descriptor("Language", SYNONYM), "Language", &mut payload)
            .unwrap();
        assert_eq!(payload, json!({"properties": []}));
    }

    const EXPORT_COMMAND: &str = "<Representation>Auto</Representation>\
        <ToolTip><v8:item><v8:lang>ru</v8:lang><v8:content>Выгрузить</v8:content></v8:item></ToolTip>\
        <Picture><xr:Ref>CommonPicture.Export</xr:Ref><xr:LoadTransparent>true</xr:LoadTransparent></Picture>\
        <CommandParameterType><v8:TypeSet>cfg:DefinedType.Organization</v8:TypeSet></CommandParameterType>\
        <ParameterUseMode>Multiple</ParameterUseMode><ModifiesData>true</ModifiesData>";

    /// rule: INV.SOURCE.METADATA-NODE-PROPERTIES
    #[test]
    fn common_command_settings_keep_their_platform_roles() {
        let payload = read("CommonCommand", EXPORT_COMMAND);
        assert!(unreadable(&payload).is_empty(), "{payload}");
        assert_eq!(
            property(&payload, "Representation"),
            Some(&json!({"kind": "text", "value": "Auto"}))
        );
        assert_eq!(
            property(&payload, "ToolTip"),
            Some(&json!({"kind": "text", "value": "ru = 'Выгрузить'"}))
        );
        assert_eq!(
            property(&payload, "Picture"),
            Some(&json!({"Ref": "CommonPicture.Export", "LoadTransparent": true}))
        );
        assert_eq!(
            property(&payload, "CommandParameterType"),
            Some(&json!({"variants": [
                {"kind": "definedType", "metadataPath": "DefinedType.Organization"},
            ], "mutationCapability": "editable"}))
        );
        assert_eq!(
            property(&payload, "ParameterUseMode"),
            Some(&json!({"kind": "text", "value": "Multiple"}))
        );
        assert_eq!(
            property(&payload, "ModifiesData"),
            Some(&json!({"kind": "boolean", "value": true}))
        );
        let single = read(
            "CommonCommand",
            "<ParameterUseMode>Single</ParameterUseMode><ModifiesData>false</ModifiesData>",
        );
        assert_eq!(
            property(&single, "ParameterUseMode"),
            Some(&json!({"kind": "text", "value": "Single"}))
        );
        assert_eq!(
            property(&single, "ModifiesData"),
            Some(&json!({"kind": "boolean", "value": false}))
        );
    }

    #[test]
    fn empty_command_settings_create_no_property() {
        let payload = read(
            "CommonCommand",
            "<ToolTip/><Picture/><CommandParameterType/>",
        );
        assert_eq!(
            payload["properties"].as_array().unwrap().len(),
            1,
            "{payload}"
        );
        assert!(unreadable(&payload).is_empty(), "{payload}");
        let reference_only = read(
            "CommonCommand",
            "<Picture><xr:Ref>StdPicture.Print</xr:Ref></Picture>",
        );
        assert_eq!(
            property(&reference_only, "Picture"),
            Some(&json!({"Ref": "StdPicture.Print"}))
        );
    }

    #[test]
    fn damaged_command_settings_are_unreadable_and_spare_the_rest() {
        for (kind, damaged, field) in [
            ("CommonCommand", "<Representation>Sometimes</Representation>", "Representation"),
            ("CommonCommand", "<Representation/>", "Representation"),
            ("CommonCommand", "<ParameterUseMode>Many</ParameterUseMode>", "ParameterUseMode"),
            ("CommonCommand", "<ModifiesData>bad</ModifiesData>", "ModifiesData"),
            ("CommonCommand", "<Picture><xr:Ref>CommonPicture.A</xr:Ref><xr:LoadTransparent>yes</xr:LoadTransparent></Picture>", "Picture"),
            ("CommonCommand", "<Picture><xr:LoadTransparent>true</xr:LoadTransparent></Picture>", "Picture"),
            ("CommonCommand", "<Picture><xr:Ref>StdPicture.A</xr:Ref><xr:Ref>StdPicture.B</xr:Ref></Picture>", "Picture"),
            ("CommonCommand", "<Picture>StdPicture.A</Picture>", "Picture"),
            ("CommonCommand", "<CommandParameterType><v8:Type>xs:string</v8:Type><v8:StringQualifiers><v8:Length>bad</v8:Length><v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers></CommandParameterType>", "CommandParameterType"),
        ] {
            let payload = read(kind, &format!("{SYNONYM}{damaged}"));
            assert_eq!(unreadable(&payload), [format!("properties.{field}")], "{kind}: {damaged}");
            assert_eq!(property(&payload, field), None, "{kind}: {damaged}");
            assert!(property(&payload, "Synonym").is_some(), "{kind}: {damaged}");
        }
        let mixed = read(
            "CommonCommand",
            "<Representation>Text</Representation><ModifiesData>bad</ModifiesData>",
        );
        assert_eq!(unreadable(&mixed), ["properties.ModifiesData"]);
        assert_eq!(
            property(&mixed, "Representation"),
            Some(&json!({"kind": "text", "value": "Text"}))
        );
    }

    /// rule: INV.SOURCE.OBSERVED-TYPE-CAPABILITY
    #[test]
    fn command_parameter_type_keeps_read_only_and_unmodelled_types() {
        let read_only = read(
            "CommonCommand",
            "<CommandParameterType><v8:TypeId>16160000-0000-4000-8000-000000000002</v8:TypeId></CommandParameterType>",
        );
        assert!(unreadable(&read_only).is_empty(), "{read_only}");
        let observed = property(&read_only, "CommandParameterType").unwrap();
        assert_eq!(observed["mutationCapability"], "readOnly");
        assert_eq!(observed["variants"][0]["kind"], "typeId");

        let future = read(
            "CommonCommand",
            "<CommandParameterType><v8:Type>xs:futureType</v8:Type></CommandParameterType>\
             <ModifiesData>false</ModifiesData>",
        );
        assert_eq!(unreadable(&future), ["properties.CommandParameterType"]);
        assert!(future["unreadable"][0]["message"]
            .as_str()
            .unwrap()
            .contains("incomplete"));
        assert_eq!(
            property(&future, "ModifiesData"),
            Some(&json!({"kind": "boolean", "value": false}))
        );
    }
}
