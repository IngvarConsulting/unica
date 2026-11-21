//! «Переменная», `Variable`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Переменная"],
    ..KindDeclaration::new("Variable")
};
