//! «ПризнакУчетаСубконто», `ExtDimensionAccountingFlag`: признак учёта
//! субконто плана счетов.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ПризнакУчетаСубконто"],
    ..KindDeclaration::new("ExtDimensionAccountingFlag")
};
