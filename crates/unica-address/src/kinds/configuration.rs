//! «Конфигурация», `Configuration`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Конфигурация"],
    named: false,
    ..KindDeclaration::new("Configuration")
};
