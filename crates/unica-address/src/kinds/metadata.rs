//! Локальное описание формы; содержимое остаётся в корне формы.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Метаданные"],
    named: false,
    ..KindDeclaration::new("Metadata")
};
