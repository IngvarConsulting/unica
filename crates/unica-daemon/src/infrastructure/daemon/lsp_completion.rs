//! Дополнение редактора на стороне продукта: метаданные набора, тексты
//! общих модулей с буферами сессии и глобальный контекст платформы из той же
//! справки, что у `unica.docs`.

use crate::infrastructure::v13_large_configuration::RegistrationIndex;
use crate::infrastructure::v13_read::LogicalViewReadAuthority;
use crate::infrastructure::v13_read_port::ProviderReadAuthority;
use std::cell::{Cell, OnceCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};
use unica_address::address::QualifiedAddress;
use unica_address::source_target::MetadataAddress;
use unica_code_intelligence::completion::{
    CompletionSources, ModuleOwner, ObjectMembers, PlatformMember,
};
use unica_documentation_sources::platform_help::provider::{
    global_context_members, GlobalContextMemberKind,
};
use unica_execution_contract::view_error::ViewError;
use unica_work_control::cancellation::CancellationToken;

/// Сколько неудачная загрузка глобального контекста ждёт следующей попытки:
/// установку могут ещё дописывать.
// resource-limit: lsp.platform-context-retry
// class: retry
// unit: ms
// scope: daemon
// value: 60000
// resource: Ожидание перед новой загрузкой глобального контекста платформы после неудачной.
// status: unproven
// origin: #2578
// problem: неизвестно
// on-reach: до следующей попытки списки дополнения без членов платформы помечены неполными
// issue: #2578
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
            // Help that failed to load has nothing to wait for until the
            // retry: the list is whole without it, so the editor does not ask
            // again on every keystroke.
            Some(PlatformContext::Unavailable(at))
                if at.elapsed() < PLATFORM_CONTEXT_RETRY_DELAY =>
            {
                return Some(Arc::from(Vec::new()))
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

    #[cfg(test)]
    pub(super) fn loading_for_test(&self, root: PathBuf) {
        lock(&self.loaded).insert(root, PlatformContext::Loading);
    }
}

/// What completion of one module knows beyond its text, read as the session
/// sees the source set: its buffers stand for their files.
pub(super) struct SessionSources<'a> {
    pub(super) source_set: &'a str,
    /// The module being edited.
    pub(super) module: &'a str,
    /// Read on the first question about metadata names.
    pub(super) registrations: OnceCell<Option<Arc<RegistrationIndex>>>,
    pub(super) descriptors: ProviderReadAuthority,
    pub(super) view: LogicalViewReadAuthority<'a>,
    pub(super) platform: Arc<[PlatformMember]>,
    pub(super) cancellation: &'a CancellationToken,
    /// A source failed to answer: the list lacks what it would have given.
    pub(super) missed: Cell<bool>,
}

impl SessionSources<'_> {
    /// Whether some source failed to answer this request.
    pub(super) fn missed(&self) -> bool {
        self.missed.get()
    }

    fn checkpoint(&self) -> Result<(), ViewError> {
        if self.cancellation.is_cancelled() {
            Err(ViewError::new(
                unica_refusal::RefusalCode::Cancelled,
                "completion was cancelled",
            ))
        } else {
            Ok(())
        }
    }

    fn registrations(&self) -> Option<&Arc<RegistrationIndex>> {
        self.registrations
            .get_or_init(|| {
                let read = self
                    .descriptors
                    .artifacts()
                    .registrations(self.descriptors.retained_root(), &|| self.checkpoint());
                self.missed.set(self.missed.get() || read.is_err());
                read.ok()
            })
            .as_ref()
    }
}

impl CompletionSources for SessionSources<'_> {
    fn platform_members(&self) -> &[PlatformMember] {
        &self.platform
    }

    fn metadata_names(&self, kind: &str) -> Vec<String> {
        let Some(index) = self.registrations() else {
            return Vec::new();
        };
        let names = index.names_for_kind(kind, &|| self.checkpoint());
        self.missed.set(self.missed.get() || names.is_err());
        names.unwrap_or_default()
    }

    fn common_module_text(&self, name: &str) -> Option<String> {
        let module =
            QualifiedAddress::parse(&format!("{}:CommonModule.{name}", self.source_set)).ok()?;
        let text = self.view.module_text(&module);
        self.missed.set(self.missed.get() || text.is_err());
        text.ok()
    }

    fn object_members(&self, kind: &str, name: &str) -> Option<ObjectMembers> {
        let target = MetadataAddress::parse_logical(&format!("{kind}.{name}")).ok()?;
        let members = self
            .descriptors
            .metadata_descriptor(&target)
            .ok()
            .and_then(|descriptor| object_members(&descriptor));
        self.missed.set(self.missed.get() || members.is_none());
        members
    }

    fn module_owner(&self) -> Option<ModuleOwner> {
        module_owner(self.module)
    }

    fn cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
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
/// descriptor, as the metadata family reads them.
pub(super) fn object_members(descriptor: &[u8]) -> Option<ObjectMembers> {
    let names = crate::infrastructure::family_codecs::metadata_read_codec()
        .object_member_names(descriptor)
        .ok()?;
    Some(ObjectMembers {
        attributes: names.attributes,
        tabular_sections: names.tabular_sections,
    })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
#[path = "lsp_completion/tests.rs"]
mod tests;
