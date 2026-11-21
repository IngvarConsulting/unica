//! «ЗначениеПеречисления», `EnumValue`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ЗначениеПеречисления"],
    ..KindDeclaration::new("EnumValue")
};
