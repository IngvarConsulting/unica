//! Метод, который зовёт этот метод. Элемент этой ветви адресует чужой
//! метод, а не потомка: граф вызовов показывает наружу.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Вызывающий"],
    ..KindDeclaration::new("Caller")
};
