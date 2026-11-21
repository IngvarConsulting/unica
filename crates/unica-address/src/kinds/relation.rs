//! Ссылка объекта метаданных наружу: владелец, движение, основание,
//! источник события. Элемент этой ветви адресует чужой объект, а не
//! потомка.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Связь"],
    ..KindDeclaration::new("Relation")
};
