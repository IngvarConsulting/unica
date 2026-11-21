//! Операция словаря узла: `<узел>.Can.<op>`. Сегмент терминальный, и
//! его имя — весь остаток адреса, потому что имя операции само содержит
//! точку (`query.set`). Узел, чей словарь спрашивают, — адрес до `Can`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    ..KindDeclaration::new("Can")
};
