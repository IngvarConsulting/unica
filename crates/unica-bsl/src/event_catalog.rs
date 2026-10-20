//! Module capability selects its records from the same immutable catalogue.
use crate::profile::ModuleCapability;
use unica_metadata::event_catalog::{platform_event_catalog_fixture, PlatformEventSpec};

pub fn module_event_catalog_8_3_27(capability: ModuleCapability) -> &'static [PlatformEventSpec] {
    let owner_kind = capability.owner_kind().as_str();
    let role = capability.role().as_str();
    platform_event_catalog_fixture()
        .module_catalogs
        .iter()
        .find(|catalog| {
            catalog.module_role == role
                && (catalog.owner_kind == owner_kind || catalog.owner_kind == "*")
        })
        .map_or(&[], |catalog| catalog.events.as_slice())
}
