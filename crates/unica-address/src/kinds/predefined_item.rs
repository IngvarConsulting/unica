//! Предопределённый элемент объекта: носит имя и идентификатор, поэтому
//! адресуется — в отличие от характеристики, названной парой источников.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ПредопределённыйЭлемент"],
    ..KindDeclaration::new("PredefinedItem")
};
