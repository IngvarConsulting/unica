//! Группа командного интерфейса: место, куда платформа кладёт команду.
//! Не путать с `CommandGroup` — тот объект метаданных, а это ссылка на
//! группу панели, стандартную либо заведённую таким объектом.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Группа"],
    ..KindDeclaration::new("Group")
};
