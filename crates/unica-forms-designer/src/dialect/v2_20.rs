//! The form dialect of Designer export format 2.20 (platform 8.3.27): the
//! values of enumerated form properties the platform corpus of that version
//! proves.

/// The values an enumerated form property admits.
pub(crate) fn enum_values(xml_name: &str) -> Option<&'static [&'static str]> {
    match xml_name {
        "TableRepresentation" => Some(&["List", "HierarchicalList", "Tree"]),
        "FormChildrenGroup" => Some(&[
            "Horizontal",
            "Vertical",
            "HorizontalIfPossible",
            "AlwaysHorizontal",
        ]),
        "UsualGroupBehavior" => Some(&["Usual", "Collapsible", "PopUp", "Auto"]),
        "UsualGroupRepresentation" => Some(&[
            "None",
            "StrongSeparation",
            "WeakSeparation",
            "NormalSeparation",
            "GroupBox",
            "Line",
            "Margin",
        ]),
        "PagesRepresentation" => Some(&[
            "None",
            "TabsOnTop",
            "TabsOnBottom",
            "TabsOnLeftHorizontal",
            "TabsOnRightHorizontal",
            "Swipe",
            "Auto",
        ]),
        "CurrentRowUse" => Some(&["Use", "DontUse", "Auto"]),
        "InitialTreeView" => Some(&["NoExpand", "ExpandTopLevel", "ExpandAllLevels"]),
        "ChoiceFoldersAndItems" => Some(&["Items", "Folders", "FoldersAndItems"]),
        "UpdateOnDataChange" => Some(&["Auto", "DontUpdate"]),
        "CheckBoxType" => Some(&["Auto", "CheckBox", "Tumbler", "Switcher"]),
        "FormElementTitleLocation" => Some(&["None", "Auto", "Left", "Top", "Right", "Bottom"]),
        "ManagedFormButtonType" => Some(&[
            "CommandBarButton",
            "UsualButton",
            "Hyperlink",
            "CommandBarHyperlink",
        ]),
        "ButtonRepresentation" => Some(&["Text", "Picture", "PictureAndText", "Auto"]),
        // `logform:FormCommand.Representation` is `DefaultRepresentation`,
        // not the button set: 8.3.27 refuses `PictureAndText` on a command.
        "DefaultRepresentation" => Some(&["Text", "Picture", "TextPicture", "Auto"]),
        "ButtonLocationInCommandBar" => Some(&[
            "Auto",
            "InAdditionalSubmenu",
            "InCommandBar",
            "InCommandBarAndInAdditionalSubmenu",
        ]),
        // Свойства корня формы: перечисления общей таблицы 8.3.27.
        _ => unica_forms::root_properties::root_property(xml_name).and_then(|(_, property)| {
            match property.value {
                unica_forms::root_properties::RootValue::Choice(choices) => Some(choices),
                _ => None,
            }
        }),
    }
}
