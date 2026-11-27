//! Встроенная справка объекта, страница которой именуется кодом языка.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Справка"],
    ..KindDeclaration::new("Help")
};
