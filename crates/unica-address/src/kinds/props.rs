//! Одно свойство узла целиком: `<узел>.Props.<имя>`. Сегмент
//! терминальный; так читается свойство, которое не поместилось в
//! `props` узла (#2088). Узел-владелец — адрес до `Props`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    ..KindDeclaration::new("Props")
};
