//! A saved workspace module and the identity used by the platform debugger.

use unica_address::{
    address::{AddressError, NodeKind, QualifiedAddress},
    module_routes::{has_nested_module_role, ModuleRole},
};
use uuid::Uuid;

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
    pub fn from_presentation(presentation: &str) -> Option<Self> {
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
            ["МодульУправляемогоПриложения" | "ManagedApplicationModule"] => {
                Some(Self("Module.ManagedApplication".to_owned()))
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
        let form = DebugFrameModule::from_presentation(
            "Документ.РеализацияТоваровУслуг.Форма.ФормаДокумента.Форма.ПриСозданииНаСервере(Отказ = Ложь, СтандартнаяОбработка = Истина)",
        )
        .unwrap();
        assert_eq!(
            form.in_source_set("main").unwrap().to_string(),
            "main:Document.РеализацияТоваровУслуг.Form.ФормаДокумента.Module.Form"
        );
        assert_eq!(
            DebugFrameModule::from_presentation("Document.РеализацияТоваровУслуг.Form.ФормаДокумента.Form.ПриОткрытии(Объект.Реквизит)"),
            Some(form)
        );
        let application = DebugFrameModule::from_presentation(
            "МодульУправляемогоПриложения.ПередНачаломРаботыСистемы()",
        )
        .unwrap();
        assert_eq!(
            application.in_source_set("ext").unwrap().to_string(),
            "ext:Module.ManagedApplication"
        );
    }

    #[test]
    fn incomplete_or_unsupported_frame_names_have_no_address_hint() {
        for presentation in [
            "ПриСозданииНаСервере()",
            "Документ.Заказ.Форма.ФормаДокумента.Подпись.ПриСозданииНаСервере()",
            "Документ.Заказ.Команда.ФормаДокумента.Форма.Открыть()",
            "Документ.Заказ.Форма.Форма/Списка.Форма.Открыть()",
            "Документ.Заказ.Форма.ФормаДокумента.Форма.()",
            "Документ.Заказ.Форма.ФормаДокумента.Форма.Открыть(арг",
        ] {
            assert_eq!(
                DebugFrameModule::from_presentation(presentation),
                None,
                "{presentation}"
            );
        }
    }
}
