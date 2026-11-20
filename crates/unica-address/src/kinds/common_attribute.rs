//! «ОбщийРеквизит», `CommonAttribute`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ОбщийРеквизит"],
    metadata: Some(MetadataDeclaration::new(&[
        "CommonAttributes",
        "ОбщиеРеквизиты",
    ])),
    root: Some(RootPlace {
        child_objects: 1100,
        tree: Some(500),
    }),
    layout: Some(LayoutDeclaration {
        directory: "CommonAttributes",
        display_name_ru: "Общие реквизиты",
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    // Значения, которые выгрузка 8.3.27, 8.5.1 и 8.5.4 одинаково пишет
    // объекту с опущенными в описании свойствами.
    creation: Some(CreationDeclaration {
        properties: &[
            "<Type>",
            "\t<v8:Type>xs:string</v8:Type>",
            "\t<v8:StringQualifiers>",
            "\t\t<v8:Length>10</v8:Length>",
            "\t\t<v8:AllowedLength>Variable</v8:AllowedLength>",
            "\t</v8:StringQualifiers>",
            "</Type>",
            "<PasswordMode>false</PasswordMode>",
            "<Format/>",
            "<EditFormat/>",
            "<ToolTip/>",
            "<MarkNegatives>false</MarkNegatives>",
            "<Mask/>",
            "<MultiLine>false</MultiLine>",
            "<ExtendedEdit>false</ExtendedEdit>",
            "<MinValue xsi:nil=\"true\"/>",
            "<MaxValue xsi:nil=\"true\"/>",
            "<FillFromFillingValue>false</FillFromFillingValue>",
            "<FillValue xsi:type=\"xs:string\"/>",
            "<FillChecking>DontCheck</FillChecking>",
            "<ChoiceFoldersAndItems>Items</ChoiceFoldersAndItems>",
            "<ChoiceParameterLinks/>",
            "<ChoiceParameters/>",
            "<QuickChoice>Auto</QuickChoice>",
            "<CreateOnInput>Auto</CreateOnInput>",
            "<ChoiceForm/>",
            "<LinkByType/>",
            "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
            "<Content/>",
            "<AutoUse>DontUse</AutoUse>",
            "<DataSeparation>DontUse</DataSeparation>",
            "<SeparatedDataUse>Independently</SeparatedDataUse>",
            "<DataSeparationValue/>",
            "<DataSeparationUse/>",
            "<ConditionalSeparation/>",
            "<UsersSeparation>DontUse</UsersSeparation>",
            "<AuthenticationSeparation>DontUse</AuthenticationSeparation>",
            "<ConfigurationExtensionsSeparation>DontUse</ConfigurationExtensionsSeparation>",
            "<Indexing>DontIndex</Indexing>",
            "<FullTextSearch>Use</FullTextSearch>",
            "<DataHistory>Use</DataHistory>",
        ],
    }),
    ..KindDeclaration::new("CommonAttribute")
};
