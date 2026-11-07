//! Дополнение редактора на стороне продукта: метаданные набора, тексты
//! общих модулей с буферами сессии и глобальный контекст платформы из той же
//! справки, что у `unica.docs`.

use crate::infrastructure::v13_large_configuration::RegistrationIndex;
use crate::infrastructure::v13_read::LogicalViewReadAuthority;
use crate::infrastructure::v13_read_port::ProviderReadAuthority;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};
use unica_address::address::QualifiedAddress;
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_code_intelligence::completion::{
    CompletionSources, ModuleOwner, ObjectMembers, PlatformMember,
};
use unica_documentation_sources::platform_help::provider::{
    global_context_members, GlobalContextMemberKind,
};

/// Сколько неудачная загрузка глобального контекста ждёт следующей попытки:
/// установку могут ещё дописывать.
const PLATFORM_CONTEXT_RETRY_DELAY: Duration = Duration::from_secs(60);

/// Глобальный контекст каждой установки платформы, загруженный один раз в
/// фоне: холодный индекс синтакс-помощника строится много дольше запроса.
#[derive(Default)]
pub(super) struct PlatformContexts {
    loaded: Mutex<HashMap<PathBuf, PlatformContext>>,
}

#[derive(Clone)]
enum PlatformContext {
    Loading,
    Ready(Arc<[PlatformMember]>),
    Unavailable(Instant),
}

impl PlatformContexts {
    /// Члены установки `root` или `None`, пока они неизвестны. Первый вопрос
    /// начинает загрузку; неудачная повторяется после задержки.
    pub(super) fn members(self: &Arc<Self>, root: &Path) -> Option<Arc<[PlatformMember]>> {
        let mut loaded = lock(&self.loaded);
        match loaded.get(root) {
            Some(PlatformContext::Ready(members)) => return Some(Arc::clone(members)),
            Some(PlatformContext::Loading) => return None,
            Some(PlatformContext::Unavailable(at))
                if at.elapsed() < PLATFORM_CONTEXT_RETRY_DELAY =>
            {
                return None
            }
            Some(PlatformContext::Unavailable(_)) | None => {}
        }
        loaded.insert(root.to_path_buf(), PlatformContext::Loading);
        drop(loaded);
        let (contexts, owned) = (Arc::clone(self), root.to_path_buf());
        let spawned = std::thread::Builder::new()
            .name("unica-lsp-global-context".into())
            .spawn(move || {
                // A panic in the help index must not leave the root loading
                // for the life of the daemon.
                let context = match std::panic::catch_unwind(|| global_context_members(&owned)) {
                    Ok(Ok(members)) => PlatformContext::Ready(
                        members
                            .into_iter()
                            .map(|member| PlatformMember {
                                method: member.kind == GlobalContextMemberKind::Method,
                                name: member.name_ru,
                                parameters: member.parameters,
                            })
                            .collect(),
                    ),
                    Ok(Err(_)) | Err(_) => PlatformContext::Unavailable(Instant::now()),
                };
                lock(&contexts.loaded).insert(owned, context);
            });
        if spawned.is_err() {
            lock(&self.loaded).remove(root);
        }
        None
    }

    #[cfg(test)]
    pub(super) fn ready_for_test(&self, root: PathBuf, members: Vec<PlatformMember>) {
        lock(&self.loaded).insert(root, PlatformContext::Ready(Arc::from(members)));
    }
}

/// What completion of one module knows beyond its text, read as the session
/// sees the source set: its buffers stand for their files.
pub(super) struct SessionSources<'a> {
    pub(super) source_set: &'a str,
    /// The module being edited.
    pub(super) module: &'a str,
    pub(super) registrations: Option<Arc<RegistrationIndex>>,
    pub(super) descriptors: ProviderReadAuthority,
    pub(super) view: LogicalViewReadAuthority<'a>,
    pub(super) platform: Arc<[PlatformMember]>,
    pub(super) cancelled: &'a dyn Fn() -> bool,
}

impl CompletionSources for SessionSources<'_> {
    fn platform_members(&self) -> &[PlatformMember] {
        &self.platform
    }

    fn metadata_names(&self, kind: &str) -> Vec<String> {
        self.registrations
            .as_ref()
            .and_then(|index| index.names_for_kind(kind, &|| Ok(())).ok())
            .unwrap_or_default()
    }

    fn common_module_text(&self, name: &str) -> Option<String> {
        let module =
            QualifiedAddress::parse(&format!("{}:CommonModule.{name}", self.source_set)).ok()?;
        self.view.module_text(&module).ok()
    }

    fn object_members(&self, kind: &str, name: &str) -> Option<ObjectMembers> {
        let target =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &format!("{kind}.{name}"))
                .ok()?;
        let descriptor = self.descriptors.metadata_descriptor(&target).ok()?;
        object_members(std::str::from_utf8(&descriptor).ok()?)
    }

    fn module_owner(&self) -> Option<ModuleOwner> {
        module_owner(self.module)
    }

    fn cancelled(&self) -> bool {
        (self.cancelled)()
    }
}

/// The object whose object module or form module `module` is: the object
/// module and the forms of an object reach its members.
pub(super) fn module_owner(module: &str) -> Option<ModuleOwner> {
    let form = module.ends_with(".Module.Form") && module.contains(".Form.");
    if !(module.ends_with(".Module.Object") || form) {
        return None;
    }
    let at = QualifiedAddress::parse(module).ok()?;
    let owner = at.segments().first()?;
    Some(ModuleOwner {
        kind: owner.kind().as_str().to_owned(),
        name: owner.name()?.to_owned(),
        form,
    })
}

/// Attributes and tabular sections, with their columns, of an object's
/// descriptor.
pub(super) fn object_members(descriptor: &str) -> Option<ObjectMembers> {
    let document = roxmltree::Document::parse(descriptor).ok()?;
    let object = document
        .root_element()
        .children()
        .find(roxmltree::Node::is_element)?;
    Some(ObjectMembers {
        attributes: children(object, "Attribute")
            .into_iter()
            .filter_map(name)
            .collect(),
        tabular_sections: children(object, "TabularSection")
            .into_iter()
            .filter_map(|section| {
                Some((
                    name(section)?,
                    children(section, "Attribute")
                        .into_iter()
                        .filter_map(name)
                        .collect(),
                ))
            })
            .collect(),
    })
}

/// The `kind` elements in a node's `ChildObjects`.
fn children<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    kind: &'static str,
) -> Vec<roxmltree::Node<'a, 'input>> {
    node.children()
        .find(|child| child.has_tag_name("ChildObjects"))
        .into_iter()
        .flat_map(|list| list.children().filter(|child| child.has_tag_name(kind)))
        .collect()
}

/// The `Properties/Name` of a child object.
fn name(node: roxmltree::Node<'_, '_>) -> Option<String> {
    node.children()
        .find(|child| child.has_tag_name("Properties"))?
        .children()
        .find(|child| child.has_tag_name("Name"))?
        .text()
        .map(str::to_owned)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
#[path = "lsp_completion/tests.rs"]
mod tests;
