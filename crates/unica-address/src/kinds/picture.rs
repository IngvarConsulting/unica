//! The displayed resource of a registered CommonPicture.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    named: false,
    ..KindDeclaration::new("Picture")
};
