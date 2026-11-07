//! Семейный порт чтения представления метаданных. Семейство владеет фактами,
//! кодек разбирает своё представление дескрипторов. Код чтения в композиции
//! обращается к разбору только через [`MetadataReadCodec`], поэтому конкретный
//! кодек не входит в его граф вызовов.
//!
//! Порт растёт по видам чтения. Каждый метод получает уже прочитанные байты
//! или текст: файлы и снимок источника остаются у композиции.
use crate::MetaObservedProperty;
use serde_json::Value;

/// Разбор дескрипторов метаданных для чтения.
pub trait MetadataReadCodec: Send + Sync {
    /// Расписание регламентного задания из текста `Schedule.xml`.
    fn job_schedule(&self, text: &str) -> Result<Value, String>;

    /// `LocationURL` WS-ссылки. Свойства нет — `None`.
    fn ws_reference_location(&self, descriptor: &[u8]) -> Result<Option<String>, String>;

    /// Комментарий объекта вида `kind`. Комментария нет — `None`.
    fn descriptor_comment(&self, descriptor: &[u8], kind: &str) -> Result<Option<String>, String>;

    /// Код языка из дескриптора `Language`.
    fn language_code(&self, descriptor: &[u8]) -> Result<String, String>;

    /// Состав и режим функциональной опции `name`.
    fn functional_option(
        &self,
        descriptor: &str,
        name: &str,
    ) -> Result<FunctionalOptionFacts, String>;

    /// Свойства нумератора документов. Нечитаемое свойство не прерывает
    /// чтение остальных и попадает в `unreadable`.
    fn numerator(&self, descriptor: &[u8]) -> Result<NumeratorFacts, String>;
}

/// Свойства функциональной опции, которые показывает чтение.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionalOptionFacts {
    /// Путь хранения значения опции.
    pub location: String,
    /// `PrivilegedGetMode`.
    pub privileged_get_mode: bool,
}

/// Свойства нумератора документов и поля, которые не удалось прочитать.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumeratorFacts {
    pub properties: Vec<MetaObservedProperty>,
    pub unreadable: Vec<ReadFinding>,
}

/// Поле, которое кодек не смог прочитать, и причина.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadFinding {
    pub field: String,
    pub message: String,
}
