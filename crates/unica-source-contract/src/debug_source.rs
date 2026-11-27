//! A saved workspace module and the identity used by the platform debugger.

mod endpoint;
pub use endpoint::DebugServerUrl;

use unica_address::{
    address::{AddressError, NodeKind, QualifiedAddress},
    module_routes::{has_direct_module_route, has_nested_module_role, ModuleRole},
};
use uuid::Uuid;

/// RDBG property ID of a common module's BSL.
pub const COMMON_MODULE_PROPERTY_ID: Uuid = Uuid::from_u128(0xd5963243_262e_4398_b4d7_fb16d06484f6);
/// RDBG property ID of an object's BSL module.
pub const OBJECT_MODULE_PROPERTY_ID: Uuid = Uuid::from_u128(0xa637f77f_3840_441d_a1c3_699c8c5cb7e0);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DebugModuleId {
    pub extension: Option<String>,
    pub object: Uuid,
    pub property: Uuid,
}

#[derive(Debug)]
pub struct DebugModuleSource {
    pub address: QualifiedAddress,
    pub runtime_id: DebugModuleId,
}

/// Logical module named by the platform's stack frame, without a workspace source set.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DebugFrameModule(String);

impl DebugFrameModule {
    pub fn from_presentation(presentation: &str, id: &DebugModuleId) -> Option<Self> {
        let parts = frame_module_parts(presentation, id.extension.as_deref())?;
        match parts.as_slice() {
            [name] if id.property == COMMON_MODULE_PROPERTY_ID => {
                Some(Self(format!("CommonModule.{name}")))
            }
            ["МодульУправляемогоПриложения" | "ManagedApplicationModule"] => {
                Some(Self("Module.ManagedApplication".to_owned()))
            }
            [kind, name, role]
                if id.property == COMMON_MODULE_PROPERTY_ID
                    && NodeKind::parse(kind).ok() == Some(NodeKind::CommonModule)
                    && NodeKind::parse(role).ok() == Some(NodeKind::Module) =>
            {
                Some(Self(format!("CommonModule.{name}")))
            }
            [kind, name, "МодульОбъекта" | "ObjectModule"]
                if id.property == OBJECT_MODULE_PROPERTY_ID =>
            {
                let kind = NodeKind::parse(kind).ok()?;
                (kind.declaration().root.is_some()
                    && has_direct_module_route(kind, ModuleRole::Object))
                .then(|| Self(format!("{}.{name}.Module.Object", kind.as_str())))
            }
            [kind, owner, collection, form, role] => {
                let kind = NodeKind::parse(kind).ok()?;
                (has_nested_module_role(kind, ModuleRole::Form)
                    && NodeKind::parse(collection).ok() == Some(NodeKind::Form)
                    && NodeKind::parse(role).ok() == Some(NodeKind::Form))
                .then(|| Self(format!("{}.{owner}.Form.{form}.Module.Form", kind.as_str())))
            }
            _ => None,
        }
    }

    pub fn in_source_set(&self, source_set: &str) -> Result<QualifiedAddress, AddressError> {
        QualifiedAddress::parse(&format!("{source_set}:{}", self.0))
    }
}

/// Only the qualified object-module frame of a dynamically loaded processor names its owner.
pub fn external_processor_frame_name<'a>(
    presentation: &'a str,
    id: &DebugModuleId,
) -> Option<&'a str> {
    if id.extension.is_some() || id.property != OBJECT_MODULE_PROPERTY_ID || id.object.is_nil() {
        return None;
    }
    let parts = frame_module_parts(presentation, None)?;
    match parts.as_slice() {
        [kind, name, "МодульОбъекта" | "ObjectModule"]
            if NodeKind::parse(kind).ok() == Some(NodeKind::ExternalDataProcessor) =>
        {
            Some(*name)
        }
        _ => None,
    }
}

fn frame_module_parts<'a>(presentation: &'a str, extension: Option<&str>) -> Option<Vec<&'a str>> {
    let (call, arguments) = presentation.split_once('(')?;
    if !arguments.ends_with(')') {
        return None;
    }
    let call = match call.split_once(' ') {
        Some((prefix, call)) if !prefix.is_empty() && extension == Some(prefix) => call,
        Some(_) => return None,
        None => call,
    };
    let (module, method) = call.rsplit_once('.')?;
    let parts: Vec<_> = module.splitn(6, '.').collect();
    parts
        .iter()
        .copied()
        .chain([method])
        .all(is_identifier)
        .then_some(parts)
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_alphanumeric())
}

/// Which saved module evidence the debugger needs for this request.
#[derive(Clone, Copy)]
pub enum DebugModuleQuery<'a> {
    Address(&'a str),
    /// Look up just the registered address named by the frame and verify the complete ID.
    Frame {
        module: &'a DebugFrameModule,
        id: &'a DebugModuleId,
    },
    /// ExtMDModule uses its own source sets, even when a config/extension UUID is identical.
    ExternalProcessorFrame {
        name: &'a str,
        id: &'a DebugModuleId,
    },
    Runtime(&'a [DebugModuleId]),
}

pub struct DebugModuleMatches {
    pub modules: Vec<DebugModuleSource>,
    /// Changes when the set of supported source roots observed by this resolver changes.
    pub source_sets_revision: u64,
}

/// A resolver belongs to one DAP session and may retain evidence for unchanged source files.
pub type ResolveDebugModules = Box<
    dyn FnMut(
        DebugModuleQuery<'_>,
    ) -> Result<DebugModuleMatches, Box<dyn std::error::Error + Send + Sync>>,
>;

#[cfg(test)]
mod tests {
    use super::*;

    fn configuration_frame(presentation: &str, property: Uuid) -> Option<DebugFrameModule> {
        DebugFrameModule::from_presentation(
            presentation,
            &DebugModuleId {
                extension: None,
                object: Uuid::nil(),
                property,
            },
        )
    }

    #[test]
    fn frame_module_parses_the_module_independently_of_method_and_arguments() {
        let form_property = Uuid::from_u128(0x32e087ab_1491_49b6_aba7_43571b41ac2b);
        let form = configuration_frame(
            "Документ.РеализацияТоваровУслуг.Форма.ФормаДокумента.Форма.ПриСозданииНаСервере(Отказ = Ложь, СтандартнаяОбработка = Истина)",
            form_property,
        )
        .unwrap();
        assert_eq!(
            form.in_source_set("main").unwrap().to_string(),
            "main:Document.РеализацияТоваровУслуг.Form.ФормаДокумента.Module.Form"
        );
        assert_eq!(
            configuration_frame("Document.РеализацияТоваровУслуг.Form.ФормаДокумента.Form.ПриОткрытии(Объект.Реквизит)", form_property),
            Some(form)
        );
        let application = configuration_frame(
            "МодульУправляемогоПриложения.ПередНачаломРаботыСистемы()",
            Uuid::from_u128(0xd22e852a_cf8a_4f77_8ccb_3548e7792bea),
        )
        .unwrap();
        assert_eq!(
            application.in_source_set("ext").unwrap().to_string(),
            "ext:Module.ManagedApplication"
        );
    }

    #[test]
    fn common_module_frame_requires_a_module_name_and_the_common_module_property() {
        for name in [
            "ИсправлениеДокументов.ПриСозданииНаСервере(ЭтаФорма)",
            "ОбщийМодуль.ИсправлениеДокументов.Модуль.ПриСозданииНаСервере()",
            "CommonModule.ИсправлениеДокументов.Module.Open()",
        ] {
            let module = configuration_frame(name, COMMON_MODULE_PROPERTY_ID).unwrap();
            assert_eq!(
                module.in_source_set("main").unwrap().to_string(),
                "main:CommonModule.ИсправлениеДокументов"
            );
            assert_eq!(configuration_frame(name, Uuid::nil()), None);
        }
    }

    #[test]
    fn object_module_frame_requires_a_registered_owner_kind_and_the_object_property() {
        for (name, address) in [
            (
                "Обработка.МассоваяПроверка.МодульОбъекта.Выполнить()",
                "main:DataProcessor.МассоваяПроверка.Module.Object",
            ),
            (
                "DataProcessor.МассоваяПроверка.ObjectModule.Run()",
                "main:DataProcessor.МассоваяПроверка.Module.Object",
            ),
            (
                "Справочник.Валюты.МодульОбъекта.ПриЗаписи(Отказ)",
                "main:Catalog.Валюты.Module.Object",
            ),
        ] {
            let module = configuration_frame(name, OBJECT_MODULE_PROPERTY_ID).unwrap();
            assert_eq!(module.in_source_set("main").unwrap().to_string(), address);
            assert_eq!(configuration_frame(name, COMMON_MODULE_PROPERTY_ID), None);
        }
        for name in [
            "РегистрСведений.Цены.МодульОбъекта.Выполнить()",
            "Обработка.Проверка.МодульМенеджера.Выполнить()",
            "ВнешняяОбработка.Проверка.МодульОбъекта.Выполнить()",
            "Проверка.Выполнить()",
        ] {
            assert_eq!(
                configuration_frame(name, OBJECT_MODULE_PROPERTY_ID),
                None,
                "{name}"
            );
        }
    }

    #[test]
    fn external_processor_frame_requires_its_own_qualified_object_and_role() {
        let id = DebugModuleId {
            extension: None,
            object: Uuid::from_u128(0x2df5362f_05be_5a3f_8bb8_c422d5b52470),
            property: OBJECT_MODULE_PROPERTY_ID,
        };
        let presentation = "ВнешняяОбработка.SyntheticProcessor.МодульОбъекта.Entry()";
        assert_eq!(
            external_processor_frame_name(presentation, &id),
            Some("SyntheticProcessor")
        );
        assert_eq!(
            external_processor_frame_name(
                "ExternalDataProcessor.Probe.ObjectModule.Open(One, Two)",
                &id
            ),
            Some("Probe")
        );
        for name in [
            "Обработка.SyntheticProcessor.МодульОбъекта.Entry()",
            "ВнешнийОтчет.Probe.МодульОбъекта.Open()",
            "Probe.Open()",
            "ВнешняяОбработка.Probe.Форма.Open()",
            "ВнешняяОбработка.Bad/Name.МодульОбъекта.Open()",
        ] {
            assert_eq!(external_processor_frame_name(name, &id), None, "{name}");
        }
        for wrong in [
            DebugModuleId {
                object: Uuid::nil(),
                ..id.clone()
            },
            DebugModuleId {
                property: COMMON_MODULE_PROPERTY_ID,
                ..id.clone()
            },
            DebugModuleId {
                extension: Some("External".to_string()),
                ..id.clone()
            },
        ] {
            assert_eq!(external_processor_frame_name(presentation, &wrong), None);
        }
    }

    #[test]
    fn extension_frame_accepts_only_its_own_platform_prefix() {
        let id = DebugModuleId {
            extension: Some("CorpusExtension".into()),
            object: Uuid::from_u128(0x2acafa86_e100_5e76_b674_040553243539),
            property: COMMON_MODULE_PROPERTY_ID,
        };
        let plain = "ОбщийМодуль.CorpusExtension_OwnProbe.Модуль.Entry()";
        let prefixed = "CorpusExtension ОбщийМодуль.CorpusExtension_OwnProbe.Модуль.Entry()";
        let module = DebugFrameModule::from_presentation(prefixed, &id).unwrap();
        assert_eq!(
            module.in_source_set("ext").unwrap().to_string(),
            "ext:CommonModule.CorpusExtension_OwnProbe"
        );
        assert_eq!(
            DebugFrameModule::from_presentation(plain, &id),
            Some(module)
        );
        assert_eq!(configuration_frame(prefixed, id.property), None);
        for prefix in [
            "Corpus ",
            "corpusextension ",
            "CorpusExtensionX ",
            "CorpusExtension  ",
            "CorpusExtension\t",
            "CorpusExtension.",
            "CorpusExtension",
        ] {
            assert_eq!(
                DebugFrameModule::from_presentation(&format!("{prefix}{plain}"), &id),
                None,
                "{prefix:?}"
            );
        }
    }

    #[test]
    fn incomplete_or_unsupported_frame_names_have_no_address_hint() {
        for presentation in [
            "ПриСозданииНаСервере()",
            "ОбщийМодуль.ИсправлениеДокументов.Форма.ПриСозданииНаСервере()",
            "Исправление/Документов.ПриСозданииНаСервере()",
            "Документ.Заказ.Форма.ФормаДокумента.Подпись.ПриСозданииНаСервере()",
            "Документ.Заказ.Команда.ФормаДокумента.Форма.Открыть()",
            "Документ.Заказ.Форма.Форма/Списка.Форма.Открыть()",
            "Документ.Заказ.Форма.ФормаДокумента.Форма.()",
            "Документ.Заказ.Форма.ФормаДокумента.Форма.Открыть(арг",
        ] {
            assert_eq!(
                configuration_frame(presentation, COMMON_MODULE_PROPERTY_ID),
                None,
                "{presentation}"
            );
        }
    }
}
