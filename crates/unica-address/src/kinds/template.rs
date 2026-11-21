//! «Макет», `Template`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Макет"],
    metadata: Some(MetadataDeclaration::new(&["Templates", "Макеты"])),
    ..KindDeclaration::new("Template")
};
