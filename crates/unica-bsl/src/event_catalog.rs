//! Module capability selects its records from the catalogue of its line.
use crate::profile::{ModuleCapability, PlatformProfile};
use unica_metadata::event_catalog::{platform_event_catalog_fixture_for_line, PlatformEventSpec};

/// Возможные события роли модуля у её владельца в наборе линии `platform`.
/// Каталог выбирает линия: таблицы событий разных линий не совпадают, и
/// обработчик, верный для своей линии, не сверяется с чужой подписью.
pub fn module_event_catalog(
    platform: PlatformProfile,
    capability: ModuleCapability,
) -> &'static [PlatformEventSpec] {
    let owner_kind = capability.owner_kind().as_str();
    let role = capability.role().as_str();
    // У линии без закреплённого каталога событий нет вовсе: пустой ответ
    // виден, а чужой состав притворился бы своим.
    let Some(fixture) = platform_event_catalog_fixture_for_line(Some(platform.line())) else {
        return &[];
    };
    fixture
        .module_catalogs
        .iter()
        .find(|catalog| {
            catalog.module_role == role
                && (catalog.owner_kind == owner_kind || catalog.owner_kind == "*")
        })
        .map_or(&[], |catalog| catalog.events.as_slice())
}
