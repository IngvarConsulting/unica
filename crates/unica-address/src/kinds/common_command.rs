//! «ОбщаяКоманда», `CommonCommand`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ОбщаяКоманда"],
    metadata: Some(MetadataDeclaration::new(&[
        "CommonCommands",
        "ОбщиеКоманды",
    ])),
    root: Some(RootPlace {
        child_objects: 2600,
        tree: Some(1600),
    }),
    layout: Some(LayoutDeclaration {
        directory: "CommonCommands",
        display_name_ru: "Общие команды",
        requires_child_objects: false,
        ..LayoutDeclaration::EMPTY
    }),
    generated_types: Some(&[]),
    // Новый объект — как его пишет Конфигуратор 8.3.27 по «Добавить»: группа
    // пуста, и платформа не загрузит команду, пока её не задать; модуль —
    // шаблон обработчика на варианте встроенного языка конфигурации.
    creation: Some(CreationDeclaration {
        module: Some(CreationModule {
            file: "CommandModule.bsl",
            russian: "\
            \r\n\
            &НаКлиенте\r\n\
            Процедура ОбработкаКоманды(ПараметрКоманды, ПараметрыВыполненияКоманды)\r\n\
            \t//Вставить содержимое обработчика.\r\n\
            \t//ПараметрыФормы = Новый Структура(\"\", );\r\n\
            \t//ОткрытьФорму(\"ОбщаяФорма.\", ПараметрыФормы, ПараметрыВыполненияКоманды.Источник, ПараметрыВыполненияКоманды.Уникальность, ПараметрыВыполненияКоманды.Окно, ПараметрыВыполненияКоманды.НавигационнаяСсылка);\r\n\
            КонецПроцедуры\r\n",
            english: "\
            \r\n\
            &AtClient\r\n\
            Procedure CommandProcessing(CommandParameter, CommandExecuteParameters)\r\n\
            \t//Вставить содержимое обработчика.\r\n\
            \t//FormParameters = New Structure(\"\", );\r\n\
            \t//OpenForm(\"CommonForm.\", FormParameters, CommandExecuteParameters.Source, CommandExecuteParameters.Uniqueness, CommandExecuteParameters.Window, CommandExecuteParameters.URL);\r\n\
            EndProcedure\r\n",
        }),
        ..CreationDeclaration::properties(&[
        "<Group/>",
        "<Representation>Auto</Representation>",
        "<ToolTip/>",
        "<Picture/>",
        "<Shortcut/>",
        "<IncludeHelpInContents>false</IncludeHelpInContents>",
        "<CommandParameterType/>",
        "<ParameterUseMode>Single</ParameterUseMode>",
        "<ModifiesData>false</ModifiesData>",
        "<OnMainServerUnavalableBehavior>Auto</OnMainServerUnavalableBehavior>",
        ])
    }),
    ..KindDeclaration::new("CommonCommand")
};
