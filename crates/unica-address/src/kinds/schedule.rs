//! «Расписание», `Schedule`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Расписание"],
    named: false,
    ..KindDeclaration::new("Schedule")
};
