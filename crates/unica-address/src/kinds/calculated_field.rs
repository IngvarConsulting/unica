//! Вычисляемое поле схемы: `Template.T.CalculatedField.<dataPath>`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ВычисляемоеПоле"],
    ..KindDeclaration::new("CalculatedField")
};
