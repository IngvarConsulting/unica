//! Характеристика плана видов характеристик: пара источников типов и
//! значений. Прикладного имени у неё нет, поэтому элементы этой ветви
//! остаются строками данных.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Характеристика"],
    ..KindDeclaration::new("Characteristic")
};
