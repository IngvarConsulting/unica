//! A saved workspace module and the identity used by the platform debugger.

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
    pub fn from_presentation(presentation: &str, property: Uuid) -> Option<Self> {
        let (call, arguments) = presentation.split_once('(')?;
        if !arguments.ends_with(')') {
            return None;
        }
        let (module, method) = call.rsplit_once('.')?;
        let parts: Vec<_> = module.splitn(6, '.').collect();
        if !parts.iter().copied().chain([method]).all(is_identifier) {
            return None;
        }
        match parts.as_slice() {
            [name] if property == COMMON_MODULE_PROPERTY_ID => {
                Some(Self(format!("CommonModule.{name}")))
            }
            ["МодульУправляемогоПриложения" | "ManagedApplicationModule"] => {
                Some(Self("Module.ManagedApplication".to_owned()))
            }
            [kind, name, role]
                if property == COMMON_MODULE_PROPERTY_ID
                    && NodeKind::parse(kind).ok() == Some(NodeKind::CommonModule)
                    && NodeKind::parse(role).ok() == Some(NodeKind::Module) =>
            {
                Some(Self(format!("CommonModule.{name}")))
            }
            [kind, name, "МодульОбъекта" | "ObjectModule"]
                if property == OBJECT_MODULE_PROPERTY_ID =>
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

    #[test]
    fn frame_module_parses_the_module_independently_of_method_and_arguments() {
        let form_property = Uuid::from_u128(0x32e087ab_1491_49b6_aba7_43571b41ac2b);
        let form = DebugFrameModule::from_presentation(
            "Документ.РеализацияТоваровУслуг.Форма.ФормаДокумента.Форма.ПриСозданииНаСервере(Отказ = Ложь, СтандартнаяОбработка = Истина)",
            form_property,
        )
        .unwrap();
        assert_eq!(
            form.in_source_set("main").unwrap().to_string(),
            "main:Document.РеализацияТоваровУслуг.Form.ФормаДокумента.Module.Form"
        );
        assert_eq!(
            DebugFrameModule::from_presentation("Document.РеализацияТоваровУслуг.Form.ФормаДокумента.Form.ПриОткрытии(Объект.Реквизит)", form_property),
            Some(form)
        );
        let application = DebugFrameModule::from_presentation(
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
            let module =
                DebugFrameModule::from_presentation(name, COMMON_MODULE_PROPERTY_ID).unwrap();
            assert_eq!(
                module.in_source_set("main").unwrap().to_string(),
                "main:CommonModule.ИсправлениеДокументов"
            );
            assert_eq!(DebugFrameModule::from_presentation(name, Uuid::nil()), None);
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
            let module =
                DebugFrameModule::from_presentation(name, OBJECT_MODULE_PROPERTY_ID).unwrap();
            assert_eq!(module.in_source_set("main").unwrap().to_string(), address);
            assert_eq!(
                DebugFrameModule::from_presentation(name, COMMON_MODULE_PROPERTY_ID),
                None
            );
        }
        for name in [
            "РегистрСведений.Цены.МодульОбъекта.Выполнить()",
            "Обработка.Проверка.МодульМенеджера.Выполнить()",
            "ВнешняяОбработка.Проверка.МодульОбъекта.Выполнить()",
            "Проверка.Выполнить()",
        ] {
            assert_eq!(
                DebugFrameModule::from_presentation(name, OBJECT_MODULE_PROPERTY_ID),
                None,
                "{name}"
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
                DebugFrameModule::from_presentation(presentation, COMMON_MODULE_PROPERTY_ID),
                None,
                "{presentation}"
            );
        }
    }
}
