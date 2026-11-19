//! «Интерфейс», `Interface`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Интерфейс"],
    named: false,
    ..KindDeclaration::new("Interface")
};
