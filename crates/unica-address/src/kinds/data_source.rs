//! Источник данных схемы компоновки: `Template.T.DataSource.<имя>`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ИсточникДанных"],
    ..KindDeclaration::new("DataSource")
};
