//! Итог (ресурс) схемы: `Template.T.Total.<dataPath>`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Итог"],
    ..KindDeclaration::new("Total")
};
