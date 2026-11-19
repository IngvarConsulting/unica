//! Read-only semantic declarations of a WSReference's local WSDL.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Определение"],
    named: false,
    ..KindDeclaration::new("Definition")
};
