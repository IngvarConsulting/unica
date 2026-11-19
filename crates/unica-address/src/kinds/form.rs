//! «Форма», `Form`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Форма"],
    metadata: Some(MetadataDeclaration::new(&["Forms", "Формы"])),
    ..KindDeclaration::new("Form")
};
