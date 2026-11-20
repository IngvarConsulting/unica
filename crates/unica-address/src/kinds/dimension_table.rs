//! «ТаблицаИзмерения», `DimensionTable`: таблица измерения куба внешнего
//! источника данных.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ТаблицаИзмерения"],
    ..KindDeclaration::new("DimensionTable")
};
