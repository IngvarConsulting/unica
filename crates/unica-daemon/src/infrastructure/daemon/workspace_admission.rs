//! Допуск рабочей области к актору: один и тот же для вызова MCP и папки
//! редактора. Оба получают актор из общего реестра по одним входам, поэтому
//! на одной рабочей области это один актор (`actor-identity`).

use super::super::super::v13_workspace_bootstrap::project_config_present;
use super::{
    closed_daemon_source_relative_path, UnadmittedCause, WorkspaceAdmissionError,
    WORKSPACE_ACTOR_SCOPE,
};
use crate::infrastructure::source_workspace::{
    discover_project_source_admission, WorkspaceActor, WorkspaceActorRegistry,
};
use crate::infrastructure::workspace::discover_workspace;
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use unica_execution_contract::invocation::SafeIdentityHash;
use unica_source_contract::read::FileIdentity;
use unica_source_contract::{ProjectSourceSet, SourceFormat, SourceProfile, SourceSetKind};
use unica_workspace::actor::{
    ProviderRootBinding, WorkspaceActorRegistryError, WorkspaceSourceSetInput,
};
use unica_workspace::selection_evidence::ResolvedProjectSourceAdmission;

// resource-limit: daemon.admission-root-descriptor-bytes
// class: format
// unit: bytes
// scope: file
// value: 67108864
// resource: Размер корня набора Configuration.xml, читаемого при допуске, чтобы выбрать профиль по версии корня.
// status: unproven
// origin: #2730
// problem: неизвестно
// on-reach: набор допускается с профилем 2.20; чтение сообщает о версии корня само
const ROOT_DESCRIPTOR_MAX_BYTES: usize = 64 * 1024 * 1024;
const BYTE_ORDER_MARK: char = '\u{feff}';

/// Профиль набора XML-выгрузки по версии его корня `Configuration.xml`:
/// 2.20 или выпуск со своими диалектами (2.21, 2.22). Набор, чей корень не
/// прочитан или записан в другой версии, получает профиль 2.20: чтение
/// сообщает о версии само, а правку отклоняет охрана формата.
fn platform_xml_source_profile(kind: SourceSetKind, root: &Path) -> SourceProfile {
    let fallback = SourceProfile::platform_xml_8_3_27_format_2_20();
    if !matches!(
        kind,
        SourceSetKind::Configuration | SourceSetKind::Extension
    ) {
        return fallback;
    }
    unica_workspace::read::open_source_directory(root)
        .and_then(|directory| {
            directory.read_relative_regular_bounded(
                Path::new("Configuration.xml"),
                ROOT_DESCRIPTOR_MAX_BYTES,
            )
        })
        .ok()
        .and_then(|raw| String::from_utf8(raw).ok())
        .and_then(|source| root_export_format(&source))
        .and_then(|version| SourceProfile::platform_xml_for_export_format(&version))
        .unwrap_or(fallback)
}

fn root_export_format(source: &str) -> Option<String> {
    let source = source.trim_start_matches(BYTE_ORDER_MARK);
    let document = roxmltree::Document::parse(source).ok()?;
    unica_source_designer::roots::root_version_literal(source, document.root_element())
}

/// Набор, допущенный к актору: что объявлено и какой корень удержан при обходе.
#[derive(Clone)]
pub(super) struct DiscoveredActorSource {
    pub(super) name: String,
    pub(super) kind: SourceSetKind,
    pub(super) source_format: SourceFormat,
    pub(super) source_profile: SourceProfile,
    pub(super) root: PathBuf,
    pub(super) retained_identity: FileIdentity,
}

/// Актор рабочей области и привязки его наборов, в порядке имён.
pub(in crate::infrastructure::daemon) struct AdmittedActor {
    pub(in crate::infrastructure::daemon) actor: Arc<WorkspaceActor>,
    pub(in crate::infrastructure::daemon) bindings: Vec<ProviderRootBinding>,
    pub(in crate::infrastructure::daemon) identity: SafeIdentityHash,
    pub(super) source_admission: ResolvedProjectSourceAdmission,
    pub(super) workspace_root: PathBuf,
}

/// Допустить рабочую область, найденную от `hint`.
///
/// `stop` вызывается на шагах обхода с корнем рабочей области; `true`
/// обрывает обход причиной [`UnadmittedCause::AdmissionCancelled`].
/// `skip_unavailable_parents` по объявленным наборам решает, пропускать ли
/// конфигурацию, чей корень недоступен: недоступный возможный родитель не
/// должен скрывать исправное расширение. `after_actor_admission` видит наборы
/// после выдачи актора и до привязки.
pub(super) fn admit_workspace_actor(
    hint: &Path,
    actors: &WorkspaceActorRegistry,
    stop: &mut dyn FnMut(&Path) -> bool,
    skip_unavailable_parents: impl FnOnce(&[ProjectSourceSet]) -> bool,
    after_actor_admission: impl FnOnce(&mut [DiscoveredActorSource]),
) -> Result<AdmittedActor, WorkspaceAdmissionError> {
    let context = discover_workspace(Some(hint.to_path_buf()))
        .map_err(|reason| WorkspaceAdmissionError::WorkspaceUndiscoverable { reason })?;
    let unadmitted = |cause| WorkspaceAdmissionError::Unadmitted {
        workspace_root: context.workspace_root.clone(),
        cause,
    };
    // Остановленный обход возвращает ту же ошибку, что и сорванный, и отличить
    // их может только сам чекпоинт: остановку не описывают как сбой обхода.
    let stopped = Cell::new(false);
    let mut checkpoint = || {
        if stop(&context.workspace_root) {
            stopped.set(true);
            Err("workspace admission was stopped".to_string())
        } else {
            Ok(())
        }
    };
    let source_admission =
        discover_project_source_admission(&context.workspace_root, &mut checkpoint).map_err(
            |reason| {
                if stopped.get() {
                    unadmitted(UnadmittedCause::AdmissionCancelled)
                } else if project_config_present(&context.workspace_root) {
                    unadmitted(UnadmittedCause::ProjectConfigInvalid(reason))
                } else {
                    unadmitted(UnadmittedCause::SourceDiscoveryFailed(reason))
                }
            },
        )?;
    let declared_sources = &source_admission.map().source_sets;
    let skip_unavailable_parents = skip_unavailable_parents(declared_sources);
    let mut admitted_sources = declared_sources
        .iter()
        .filter(|source| source.source_format == SourceFormat::PlatformXml)
        .filter(|source| {
            // The skipped parent's name stays declared, so resolution through
            // it is reported incomplete rather than silently narrowed.
            !(skip_unavailable_parents
                && source.kind == SourceSetKind::Configuration
                && closed_daemon_source_relative_path(&source.path)
                    .ok()
                    .and_then(|relative| source_admission.source_root_identity(&relative))
                    .is_none())
        })
        .map(|source| {
            let unreadable = |reason| {
                unadmitted(UnadmittedCause::SourceRootUnreadable {
                    source_set: source.name.clone(),
                    path: source.path.clone(),
                    reason,
                })
            };
            let relative = closed_daemon_source_relative_path(&source.path)
                .map_err(|_| unreadable("its declared path leaves the workspace root"))?;
            let retained_identity = source_admission
                .source_root_identity(&relative)
                .ok_or_else(|| unreadable("its declared root is absent or cannot be retained"))?;
            let root = context.workspace_root.join(&relative);
            Ok(DiscoveredActorSource {
                name: source.name.clone(),
                kind: source.kind,
                source_format: source.source_format,
                source_profile: platform_xml_source_profile(source.kind, &root),
                root,
                retained_identity,
            })
        })
        .collect::<Result<Vec<_>, WorkspaceAdmissionError>>()?;
    if admitted_sources.is_empty() {
        return Err(unadmitted(if declared_sources.is_empty() {
            UnadmittedCause::Uninitialized
        } else {
            UnadmittedCause::NoPlatformXmlSourceSet(declared_sources.clone())
        }));
    }
    admitted_sources.sort_by(|left, right| left.name.cmp(&right.name));
    let actor = actors
        .get_or_create(
            &context,
            admitted_sources.iter().map(|source| {
                WorkspaceSourceSetInput::new(
                    source.name.clone(),
                    source.root.clone(),
                    source.kind,
                    source.source_format,
                    source.source_profile,
                )
            }),
            WORKSPACE_ACTOR_SCOPE,
        )
        .map_err(|error| match error {
            WorkspaceActorRegistryError::InvalidIdentity(_) => {
                unadmitted(UnadmittedCause::ActorBindingFailed {
                    stage: "the actor registry rejected the workspace identity",
                })
            }
            WorkspaceActorRegistryError::NamedRootUnavailable => {
                unadmitted(UnadmittedCause::ActorRootUnavailable)
            }
            WorkspaceActorRegistryError::Poisoned => WorkspaceAdmissionError::RegistryFailed,
        })?;
    after_actor_admission(&mut admitted_sources);
    let bindings = admitted_sources
        .iter()
        .map(|source| {
            actor
                .bind_provider_root(&source.name, &source.root)
                .and_then(|binding| {
                    if binding.retained_root().identity() != source.retained_identity {
                        return Err("actor binding does not match retained source-map admission"
                            .to_string());
                    }
                    Ok(binding)
                })
                .map_err(|_| {
                    unadmitted(UnadmittedCause::ActorBindingFailed {
                        stage: "an admitted source root changed while the actor was binding it",
                    })
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let identity = actor.safe_identity_hash().map_err(|_| {
        unadmitted(UnadmittedCause::ActorBindingFailed {
            stage: "the workspace identity hash could not be computed",
        })
    })?;
    Ok(AdmittedActor {
        actor,
        bindings,
        identity,
        source_admission,
        workspace_root: context.workspace_root.clone(),
    })
}

/// Папка редактора допускается тем же путём, что `workspace_hint` вызова MCP.
/// Обход останавливается на сроке редактора.
///
/// Недоступная конфигурация отказывает всей папке и при объявленном
/// расширении, хотя `view` MCP такое расширение читает. Сессия держит актор
/// между запросами: актор без конфигурации, допущенный на время пропажи её
/// корня, остался бы у сессии и после возвращения корня, а открытые буферы
/// конфигурации пропали бы при переносе на него.
pub(in crate::infrastructure::daemon) fn admit_editor_folder(
    folder: &Path,
    actors: &WorkspaceActorRegistry,
    deadline: Instant,
) -> Option<AdmittedActor> {
    admit_workspace_actor(
        folder,
        actors,
        &mut |_| Instant::now() >= deadline,
        |_| false,
        |_| {},
    )
    .ok()
}

#[cfg(test)]
mod tests;
