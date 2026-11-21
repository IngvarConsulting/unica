//! «Команда», `Command`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Команда"],
    metadata: Some(MetadataDeclaration::new(&["Commands", "Команды"])),
    ..KindDeclaration::new("Command")
};
