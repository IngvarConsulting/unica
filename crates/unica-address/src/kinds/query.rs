//! «Запрос», `Query`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Запрос"],
    ..KindDeclaration::new("Query")
};
