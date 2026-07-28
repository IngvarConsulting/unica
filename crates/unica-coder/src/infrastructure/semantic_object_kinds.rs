use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_format_core::ports::ObjectKindSelector;

pub(crate) fn task8_metadata_kind(
    value: &str,
) -> Option<unica_format_core::semantic_ids::SemanticObjectKind> {
    let registration = PlatformXmlAdapterFactory::new().operational_registration();
    registration
        .object_kinds()
        .resolve(&ObjectKindSelector::new(value).ok()?)
}

pub(crate) fn task8_metadata_kind_by_directory(
    value: &str,
) -> Option<unica_format_core::semantic_ids::SemanticObjectKind> {
    task8_metadata_kind(value)
}

pub(crate) fn task8_metadata_kind_tags() -> Vec<&'static str> {
    let registration = PlatformXmlAdapterFactory::new().operational_registration();
    registration
        .object_kinds()
        .ordered_kinds()
        .into_iter()
        .filter_map(|kind| registration.object_kinds().lease(kind))
        .filter_map(|lease| registration.object_kinds().project(&lease))
        .map(|projection| projection.canonical_selector().as_str())
        .collect()
}

pub(crate) fn task8_metadata_kind_tag(
    kind: unica_format_core::semantic_ids::SemanticObjectKind,
) -> Option<&'static str> {
    let registration = PlatformXmlAdapterFactory::new().operational_registration();
    let lease = registration.object_kinds().lease(kind)?;
    registration
        .object_kinds()
        .project(&lease)
        .map(|projection| projection.canonical_selector().as_str())
}

pub(crate) fn task8_metadata_kind_directory(value: &str) -> Option<&'static str> {
    let registration = PlatformXmlAdapterFactory::new().operational_registration();
    let kind = registration
        .object_kinds()
        .resolve(&ObjectKindSelector::new(value).ok()?)?;
    let lease = registration.object_kinds().lease(kind)?;
    registration
        .object_kinds()
        .project(&lease)
        .map(|projection| projection.collection_selector().as_str())
}
