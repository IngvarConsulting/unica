//! «Таблица», `Table`: таблица внешнего источника данных.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Таблица"],
    ..KindDeclaration::new("Table")
};
