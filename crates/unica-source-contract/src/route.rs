//! Observed logical reader route; classification remains with its semantic owner.

use serde::Deserialize;

/// Дерево, которому принадлежит узел, — и читатель, который его проецирует.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RouteClass {
    /// Корень набора исходников.
    Configuration,
    /// Объект метаданных, ветвь их видов и члены объекта: реквизиты, табличные
    /// части, измерения, команды, макеты как узлы владельца.
    Metadata,
    /// Форма и её подузлы: элементы, реквизиты, команды, события формы.
    Form,
    /// Роль и её права.
    Role,
    /// Подсистема и её состав.
    Subsystem,
    /// Командный интерфейс подсистемы.
    Interface,
    /// Схема компоновки данных внутри макета.
    Dcs,
    /// Табличный документ внутри макета.
    Mxl,
    /// Пакет XDTO.
    Xdto,
    /// Модуль и узлы его текста.
    Module,
}
