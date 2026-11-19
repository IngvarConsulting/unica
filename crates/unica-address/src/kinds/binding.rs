//! `Binding`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    named: false,
    ..KindDeclaration::new("Binding")
};
