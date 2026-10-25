//! Свойства корня управляемой формы 8.3.27: порядок в `Form.xml`, значения,
//! умолчания и формы, к которым свойство относится (#1984).
//!
//! Порядок и умолчания сверены с выгрузкой платформы 8.3.27.2074: свойство
//! со значением по умолчанию платформа не выгружает, свойство чужого вида
//! формы (проведение у формы справочника) отбрасывает при загрузке.
//! `Title` — мультиязычный текст и идёт первым, до этой таблицы.

/// Значение свойства в `Form.xml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootValue {
    Flag,
    /// Целое `0..=4294967295`.
    Size,
    Choice(&'static [&'static str]),
    /// Имя реквизита, элемента или объекта формы.
    Reference,
}

/// Формы, у которых свойство есть: по типу основного реквизита.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootScope {
    Any,
    Document,
    Catalog,
    Report,
}

impl RootScope {
    /// Тип основного реквизита формы относится к этому виду форм: объект
    /// по имени (`DocumentObject.Заказ`) или любой объект вида
    /// (`ReportObject` у общей формы отчёта).
    pub fn admits(self, main_attribute_type: Option<&str>) -> bool {
        let kinds: &[&str] = match self {
            Self::Any => return true,
            Self::Document => &["DocumentObject"],
            Self::Catalog => &["CatalogObject", "ChartOfCharacteristicTypesObject"],
            Self::Report => &["ReportObject", "ExternalReportObject"],
        };
        main_attribute_type.is_some_and(|value| {
            let kind = value.split_once('.').map_or(value, |(kind, _)| kind);
            kinds.contains(&kind)
        })
    }

    pub const fn describe(self) -> &'static str {
        match self {
            Self::Any => "any form",
            Self::Document => "a document form (main attribute DocumentObject)",
            Self::Catalog => {
                "a catalog or characteristic types form (main attribute CatalogObject or ChartOfCharacteristicTypesObject)"
            }
            Self::Report => "a report form (main attribute ReportObject or ExternalReportObject)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootProperty {
    pub name: &'static str,
    pub value: RootValue,
    /// Значение, которое платформа не выгружает.
    pub default: Option<&'static str>,
    pub scope: RootScope,
}

const fn property(
    name: &'static str,
    value: RootValue,
    default: Option<&'static str>,
    scope: RootScope,
) -> RootProperty {
    RootProperty {
        name,
        value,
        default,
        scope,
    }
}

use RootScope::{Any, Catalog, Document, Report};
use RootValue::{Choice, Flag, Reference, Size};

const AUTO_SPACING: &[&str] = &["Auto", "None", "Half", "Single", "OneAndHalf", "Double"];

/// Скалярные свойства корня в порядке схемы 8.3.27.
pub const ROOT_PROPERTIES: &[RootProperty] = &[
    property("Width", Size, Some("0"), Any),
    property("Height", Size, Some("0"), Any),
    property(
        "WindowOpeningMode",
        Choice(&["Independent", "LockOwnerWindow", "LockWholeInterface"]),
        Some("Independent"),
        Any,
    ),
    property(
        "EnterKeyBehavior",
        Choice(&["ControlNavigation", "DefaultButton"]),
        Some("ControlNavigation"),
        Any,
    ),
    property(
        "AutoSaveDataInSettings",
        Choice(&["DontUse", "Use"]),
        Some("DontUse"),
        Any,
    ),
    property(
        "SaveDataInSettings",
        Choice(&["DontUse", "UseList"]),
        Some("DontUse"),
        Any,
    ),
    property("SaveWindowSettings", Flag, Some("true"), Any),
    property("SettingsStorage", Reference, None, Any),
    property("AutoTitle", Flag, Some("true"), Any),
    property("AutoURL", Flag, Some("true"), Any),
    property(
        "Group",
        Choice(&[
            "Horizontal",
            "Vertical",
            "HorizontalIfPossible",
            "AlwaysHorizontal",
        ]),
        Some("Vertical"),
        Any,
    ),
    property(
        "ChildrenAlign",
        Choice(&[
            "Auto",
            "None",
            "ItemsLeftTitlesLeft",
            "ItemsRightTitlesLeft",
            "ItemsLeftTitlesRight",
            "ItemsRightTitlesRight",
            "TitlesLeftDataLeft",
            "TitlesLeftDataRight",
            "TitlesRightDataLeft",
            "TitlesRightDataRight",
            "TitlesLeftDataAuto",
        ]),
        Some("Auto"),
        Any,
    ),
    property("HorizontalSpacing", Choice(AUTO_SPACING), Some("Auto"), Any),
    property("VerticalSpacing", Choice(AUTO_SPACING), Some("Auto"), Any),
    property(
        "HorizontalAlign",
        Choice(&["Left", "Center", "Right", "Auto"]),
        Some("Auto"),
        Any,
    ),
    property(
        "VerticalAlign",
        Choice(&["Top", "Center", "Bottom", "Auto"]),
        Some("Auto"),
        Any,
    ),
    property(
        "ChildItemsWidth",
        Choice(&[
            "Auto",
            "Equal",
            "LeftWide",
            "LeftWidest",
            "LeftNarrow",
            "LeftNarrowest",
        ]),
        Some("Auto"),
        Any,
    ),
    property("AutoFillCheck", Flag, Some("true"), Any),
    property("Customizable", Flag, Some("true"), Any),
    property("Enabled", Flag, Some("true"), Any),
    property("ReadOnly", Flag, Some("false"), Any),
    property(
        "CommandBarLocation",
        Choice(&["None", "Auto", "Top", "Bottom"]),
        Some("Auto"),
        Any,
    ),
    property(
        "VerticalScroll",
        Choice(&["auto", "use", "useIfNecessary", "useWithoutStretch"]),
        Some("auto"),
        Any,
    ),
    property(
        "ScalingMode",
        Choice(&["Auto", "Normal", "Compact"]),
        Some("Auto"),
        Any,
    ),
    property("Scale", Size, Some("100"), Any),
    property(
        "ConversationsRepresentation",
        Choice(&["Auto", "Show", "DontShow"]),
        Some("Auto"),
        Any,
    ),
    property("ShowTitle", Flag, Some("true"), Any),
    property("ShowCloseButton", Flag, Some("true"), Any),
    property(
        "CollapseItemsByImportanceVariant",
        Choice(&["Auto", "Use", "DontUse"]),
        Some("Auto"),
        Any,
    ),
    property(
        "UseForFoldersAndItems",
        Choice(&["Items", "Folders", "FoldersAndItems"]),
        None,
        Catalog,
    ),
    property("GroupList", Reference, None, Any),
    property(
        "AutoTime",
        Choice(&[
            "DontUse",
            "Last",
            "First",
            "CurrentOrLast",
            "CurrentOrFirst",
        ]),
        None,
        Document,
    ),
    property(
        "UsePostingMode",
        Choice(&["Regular", "RealTime", "Ask", "Auto"]),
        None,
        Document,
    ),
    property("RepostOnWrite", Flag, None, Document),
    property("ReportResult", Reference, None, Report),
    property("DetailsData", Reference, None, Report),
    property(
        "ReportFormType",
        Choice(&["Main", "Settings", "Variant"]),
        None,
        Report,
    ),
    property("VariantAppearance", Reference, None, Report),
    property(
        "AutoShowState",
        Choice(&["Auto", "DontShow", "Show", "ShowOnComposition"]),
        None,
        Report,
    ),
    property("CustomSettingsFolder", Reference, None, Report),
    property(
        "ReportResultViewMode",
        Choice(&["Auto", "Default", "Compact"]),
        None,
        Report,
    ),
    property(
        "ViewModeApplicationOnSetReportResult",
        Choice(&["Auto", "Apply", "DontApply"]),
        None,
        Report,
    ),
];

/// Позиция и описание свойства по имени элемента `Form.xml`.
pub fn root_property(name: &str) -> Option<(usize, &'static RootProperty)> {
    ROOT_PROPERTIES
        .iter()
        .enumerate()
        .find(|(_, property)| property.name == name)
}

/// Свойства, которые пишет `props.set` формы: ссылки на реквизиты, элементы
/// и объекты он не пишет.
pub fn writable_root_properties() -> impl Iterator<Item = &'static RootProperty> {
    ROOT_PROPERTIES
        .iter()
        .filter(|property| property.value != RootValue::Reference)
}

/// Корневые элементы формы, которые стоят среди свойств, но сами
/// свойствами `props.set` не являются: их место нужно, чтобы вставить
/// свойство рядом с ними. Каждый стоит перед названным свойством, в порядке
/// этого списка.
pub const INTERLEAVED_ROOT_ELEMENTS: &[(&str, &str)] = &[
    ("MobileDeviceCommandBarContent", "ShowTitle"),
    ("CommandSet", "ShowTitle"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_names_are_unique_and_defaults_are_values() {
        let mut names = std::collections::BTreeSet::new();
        for property in ROOT_PROPERTIES {
            assert!(names.insert(property.name), "{}", property.name);
            if let (Some(default), Choice(values)) = (property.default, property.value) {
                assert!(values.contains(&default), "{}", property.name);
            }
            if let (Some(default), Flag) = (property.default, property.value) {
                assert!(["true", "false"].contains(&default), "{}", property.name);
            }
        }
        for (element, before) in INTERLEAVED_ROOT_ELEMENTS {
            assert!(root_property(element).is_none(), "{element}");
            assert!(root_property(before).is_some(), "{before}");
        }
    }

    #[test]
    fn scopes_follow_the_main_attribute_type() {
        assert!(Document.admits(Some("DocumentObject.Заказ")));
        assert!(!Document.admits(Some("CatalogObject.Товары")));
        assert!(Catalog.admits(Some("ChartOfCharacteristicTypesObject.Виды")));
        assert!(Report.admits(Some("ExternalReportObject.Отчет")));
        assert!(Report.admits(Some("ReportObject")));
        assert!(!Report.admits(Some("ReportObjectX.Отчет")));
        assert!(!Report.admits(None));
        assert!(Any.admits(None));
    }
}
