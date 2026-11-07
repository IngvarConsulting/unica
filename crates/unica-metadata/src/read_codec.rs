//! Семейный порт чтения представления метаданных. Семейство владеет фактами,
//! кодек разбирает своё представление дескрипторов. Код чтения в композиции
//! обращается к разбору только через [`MetadataReadCodec`], поэтому конкретный
//! кодек не входит в его граф вызовов.
//!
//! Порт растёт по видам чтения. Каждый метод получает уже прочитанные байты
//! или текст: файлы и снимок источника остаются у композиции.
use crate::style_color::StyleItemPropertiesObservation;
use crate::wsdl::Definition;
use crate::{MetaExchangeContentItem, MetaObservedProperty};
use serde_json::{Map, Value};
use std::path::Path;
use unica_address::source_target::MetadataAddress;

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

    /// Дополняет ответ чтения элемента стиля его видом и значением.
    fn style_item(&self, descriptor: &[u8], payload: &mut Value) -> Result<(), String>;

    /// Комментарий и цвет элемента стиля `name`. `relative` — путь
    /// дескриптора в источнике: по нему кодек узнаёт профиль записи.
    fn style_properties(
        &self,
        descriptor: &str,
        name: &str,
        relative: &Path,
    ) -> Result<StyleItemPropertiesObservation, String>;

    /// Свойства формы и ограничения чтения, которые не прерывают его.
    fn form_metadata(
        &self,
        descriptor: &[u8],
        target: &MetadataAddress,
    ) -> Result<(Map<String, Value>, Vec<String>), String>;

    /// Состав плана обмена из `Ext/Content.xml`.
    fn exchange_content(&self, text: &str) -> Result<ExchangeContentFacts, String>;

    /// Объявления WSDL из `WSDefinition.xml`. Нечитаемый документ кодек
    /// описывает в самом [`Definition`]; ошибкой метод завершается только
    /// при остановке через `checkpoint`.
    fn wsdl(
        &self,
        bytes: &[u8],
        checkpoint: &mut dyn FnMut() -> Result<(), ReadStopped>,
    ) -> Result<Definition, ReadStopped>;
}

/// Чтение остановлено вызывающим через `checkpoint`. Причину остановки
/// хранит вызывающий.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadStopped;

/// Элементы состава плана обмена и ошибки элементов, которые не удалось прочитать.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExchangeContentFacts {
    pub items: Vec<MetaExchangeContentItem>,
    pub errors: Vec<String>,
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
