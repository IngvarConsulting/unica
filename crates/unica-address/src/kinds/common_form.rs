//! «ОбщаяФорма», `CommonForm`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ОбщаяФорма"],
    metadata: Some(MetadataDeclaration::new(&["CommonForms", "ОбщиеФормы"])),
    root: Some(RootPlace {
        child_objects: 2900,
        tree: Some(1800),
    }),
    layout: Some(LayoutDeclaration {
        directory: "CommonForms",
        display_name_ru: "Общие формы",
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    creation: Some(CreationDeclaration {
        main_configuration_only: true,
        ..CreationDeclaration::properties(&[
            "<FormType>Managed</FormType>",
            "<IncludeHelpInContents>false</IncludeHelpInContents>",
            "<UsePurposes>",
            "\t<v8:Value xsi:type=\"app:ApplicationUsePurpose\">PlatformApplication</v8:Value>",
            "\t<v8:Value xsi:type=\"app:ApplicationUsePurpose\">MobilePlatformApplication</v8:Value>",
            "</UsePurposes>",
            "<UseStandardCommands>false</UseStandardCommands>",
            "<ExtendedPresentation/>",
            "<Explanation/>",
        ])
    }),
    ..KindDeclaration::new("CommonForm")
};
