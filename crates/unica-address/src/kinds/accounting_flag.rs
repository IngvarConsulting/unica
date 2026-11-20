//! «ПризнакУчета», `AccountingFlag`: признак учёта плана счетов.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ПризнакУчета"],
    ..KindDeclaration::new("AccountingFlag")
};
