use super::*;
use crate::infrastructure::logical_tree::{route_logical_address, LogicalReader};

fn address(source_set: &str, kind: SourceSetKind, relative: &str) -> Option<String> {
    module_address_for_relative(
        source_set,
        kind,
        PlatformProfile::v8_3_27(),
        Path::new(relative),
    )
    .map(|at| at.to_string())
}

/// Search hits name the module that `view` reads at the same path: every
/// module role maps to one logical address, and that address routes back
/// to the module reader.
#[test]
fn module_paths_map_to_the_logical_module_view_reads() {
    let cases = [
        (
            "main",
            SourceSetKind::Configuration,
            "CommonModules/Общий/Ext/Module.bsl",
            "main:CommonModule.Общий",
        ),
        (
            "main",
            SourceSetKind::Configuration,
            "Catalogs/Валюты/Ext/ObjectModule.bsl",
            "main:Catalog.Валюты.Module.Object",
        ),
        (
            "main",
            SourceSetKind::Configuration,
            "Documents/Заказ/Ext/ManagerModule.bsl",
            "main:Document.Заказ.Module.Manager",
        ),
        (
            "main",
            SourceSetKind::Configuration,
            "InformationRegisters/Курсы/Ext/RecordSetModule.bsl",
            "main:InformationRegister.Курсы.Module.RecordSet",
        ),
        (
            "main",
            SourceSetKind::Configuration,
            "Constants/Флаг/Ext/ValueManagerModule.bsl",
            "main:Constant.Флаг.Module.ValueManager",
        ),
        (
            "main",
            SourceSetKind::Configuration,
            "Documents/Заказ/Forms/ФормаДокумента/Ext/Form/Module.bsl",
            "main:Document.Заказ.Form.ФормаДокумента.Module.Form",
        ),
        (
            "main",
            SourceSetKind::Configuration,
            "Catalogs/Валюты/Commands/Обновить/Ext/CommandModule.bsl",
            "main:Catalog.Валюты.Command.Обновить.Module.Command",
        ),
        (
            "main",
            SourceSetKind::Configuration,
            "CommonForms/Настройки/Ext/Form/Module.bsl",
            "main:CommonForm.Настройки.Module.Form",
        ),
        (
            "main",
            SourceSetKind::Configuration,
            "CommonCommands/Открыть/Ext/CommandModule.bsl",
            "main:CommonCommand.Открыть.Module.Command",
        ),
        (
            "main",
            SourceSetKind::Configuration,
            "HTTPServices/API/Ext/Module.bsl",
            "main:HTTPService.API.Module.HTTPService",
        ),
        (
            "main",
            SourceSetKind::Configuration,
            "Ext/ManagedApplicationModule.bsl",
            "main:Module.ManagedApplication",
        ),
        (
            "main",
            SourceSetKind::Configuration,
            "Ext/SessionModule.bsl",
            "main:Module.Session",
        ),
        (
            "main",
            SourceSetKind::Configuration,
            "WebServices/Обмен/Ext/Module.bsl",
            "main:WebService.Обмен.Module.WebService",
        ),
        (
            "main",
            SourceSetKind::Configuration,
            "Bots/Помощник/Ext/Module.bsl",
            "main:Bot.Помощник.Module.Bot",
        ),
        (
            "main",
            SourceSetKind::Configuration,
            "IntegrationServices/Шина/Ext/Module.bsl",
            "main:IntegrationService.Шина.Module.IntegrationService",
        ),
        (
            "ext",
            SourceSetKind::Extension,
            "CommonModules/Общий/Ext/Module.bsl",
            "ext:CommonModule.Общий",
        ),
        (
            "ext",
            SourceSetKind::Extension,
            "Ext/ManagedApplicationModule.bsl",
            "ext:Module.ManagedApplication",
        ),
        (
            "erf",
            SourceSetKind::ExternalReport,
            "Продажи/Ext/ObjectModule.bsl",
            "erf:ExternalReport.Продажи.Module.Object",
        ),
        (
            "ext",
            SourceSetKind::Extension,
            "Catalogs/Валюты/Forms/ФормаЭлемента/Ext/Form/Module.bsl",
            "ext:Catalog.Валюты.Form.ФормаЭлемента.Module.Form",
        ),
        (
            "epf",
            SourceSetKind::ExternalProcessor,
            "Импорт/Ext/ObjectModule.bsl",
            "epf:ExternalDataProcessor.Импорт.Module.Object",
        ),
        (
            "erf",
            SourceSetKind::ExternalReport,
            "Продажи/Forms/Форма/Ext/Form/Module.bsl",
            "erf:ExternalReport.Продажи.Form.Форма.Module.Form",
        ),
        (
            "epf",
            SourceSetKind::ExternalProcessor,
            "Импорт/Commands/Загрузить/Ext/CommandModule.bsl",
            "epf:ExternalDataProcessor.Импорт.Command.Загрузить.Module.Command",
        ),
    ];
    for (source_set, kind, relative, expected) in cases {
        let found = address(source_set, kind, relative);
        assert_eq!(found.as_deref(), Some(expected), "{relative}");
        let at = QualifiedAddress::parse(expected).unwrap();
        let route = route_logical_address(&at, PlatformProfile::v8_3_27()).unwrap();
        assert_eq!(route.reader(), LogicalReader::Module, "{expected}");
        assert!(route.module().is_some(), "{expected}");
    }
}

/// A `.bsl` file outside the module layout gets no address: neither a
/// guess from its directory nor the address of a neighbouring module.
#[test]
fn files_outside_the_module_layout_have_no_logical_address() {
    for (kind, relative) in [
        (SourceSetKind::Configuration, "Ext/ParentConfigurations.bsl"),
        (
            SourceSetKind::Configuration,
            "CommonModules/Общий/Ext/Other.bsl",
        ),
        (
            SourceSetKind::Configuration,
            "CommonModules/Общий/Module.bsl",
        ),
        (
            SourceSetKind::Configuration,
            "Catalogs/Валюты/Ext/RecordSetModule.bsl",
        ),
        (
            SourceSetKind::Configuration,
            "Constants/Флаг/Ext/ObjectModule.bsl",
        ),
        (
            SourceSetKind::Configuration,
            "UnknownKinds/X/Ext/Module.bsl",
        ),
        (SourceSetKind::Configuration, "tests/Сценарий.bsl"),
        // The 8.3.27 profile does not specify a WebSocketClient module
        // layout, so `view` cannot read one either.
        (
            SourceSetKind::Configuration,
            "WebSocketClients/Клиент/Ext/Module.bsl",
        ),
        // The 8.3.27 profile has no SettingsStorage form module, so view
        // refuses that address too; the hit stays unaddressed.
        (
            SourceSetKind::Configuration,
            "SettingsStorages/Варианты/Forms/Выбор/Ext/Form/Module.bsl",
        ),
        (SourceSetKind::Configuration, "Импорт/Ext/ObjectModule.bsl"),
        (SourceSetKind::Extension, "Ext/Module.bsl"),
        (
            SourceSetKind::ExternalProcessor,
            "CommonModules/Общий/Ext/Module.bsl",
        ),
        (
            SourceSetKind::ExternalProcessor,
            "Импорт/Ext/ManagerModule.bsl",
        ),
        (
            SourceSetKind::ExternalReport,
            "Продажи/Forms/Форма/Ext/Module.bsl",
        ),
    ] {
        assert_eq!(address("main", kind, relative), None, "{relative}");
    }
}
