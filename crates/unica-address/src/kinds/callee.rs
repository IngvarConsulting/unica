//! Метод, которого зовёт этот метод.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Вызываемый"],
    ..KindDeclaration::new("Callee")
};
