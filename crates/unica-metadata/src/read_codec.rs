//! Семейный порт чтения представления метаданных. Семейство владеет фактами,
//! кодек разбирает своё представление дескрипторов. Код чтения в композиции
//! обращается к разбору только через [`MetadataReadCodec`], поэтому конкретный
//! кодек не входит в его граф вызовов.
//!
//! Порт растёт по видам чтения. Каждый метод получает уже прочитанные байты
//! или текст: файлы и снимок источника остаются у композиции.
use crate::borrowing::MetaBorrowing;
use crate::configuration::{CfHomePageData, CfInfoData, CfInterfaceData};
use crate::request::MetaFailure;
use crate::style_color::StyleItemPropertiesObservation;
use crate::subsystem::{
    SubsystemCommandInterfaceData, SubsystemInfoData, SubsystemInfoResult, SubsystemTreeNode,
};
use crate::support_state::{
    ConfigurationSupportData, ObjectSupportData, SupportReadError, SupportState,
};
use crate::validation_subject::{
    CommonTemplateDescriptorFacts, MetaLocalInfo, MetadataChildProfile,
};
use crate::wsdl::Definition;
use crate::{
    MetaExchangeContentItem, MetaInfoPropertyValue, MetaObservedProperty, MetaPredefinedItemsData,
    MetaSupportStatus, MetadataKind,
};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use unica_address::source_target::MetadataAddress;
use unica_source_contract::SourceSetKind;

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

    /// Дополняет ответ чтения цвета палитры его синонимом и цветом.
    fn palette_color(&self, descriptor: &[u8], payload: &mut Value) -> Result<(), String>;

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

    /// Свойства корня из `Configuration.xml` вместе с уже прочитанными
    /// сопутствующими документами.
    fn configuration_info(
        &self,
        text: &str,
        support: ConfigurationSupportData,
        home_page: Option<CfHomePageData>,
        command_interface: Option<CfInterfaceData>,
        panels: Option<Value>,
    ) -> Result<CfInfoData, String>;

    /// Домашняя страница конфигурации из `Ext/HomePageWorkArea.xml`.
    fn configuration_home_page(&self, text: &str) -> Result<CfHomePageData, String>;

    /// Командный интерфейс конфигурации из `Ext/CommandInterface.xml`.
    fn configuration_command_interface(&self, text: &str) -> Result<CfInterfaceData, String>;

    /// Путь документа панелей клиентского приложения от корня конфигурации.
    fn client_interface_relative(&self) -> &'static str;

    /// Раскладка панелей в форме, которую принимает `panels.set`.
    fn client_panels(&self, text: &str) -> Result<Value, String>;

    /// Дескриптор подсистемы. `path` называет его в сообщениях;
    /// `has_command_interface` — есть ли у подсистемы командный интерфейс.
    /// Второй элемент — предупреждения разбора.
    fn subsystem_info(
        &self,
        path: &Path,
        text: &str,
        has_command_interface: bool,
    ) -> Result<(SubsystemInfoData, Vec<String>), String>;

    /// Командный интерфейс подсистемы. `path` называет его в сообщениях.
    fn subsystem_command_interface(
        &self,
        path: &Path,
        text: &str,
    ) -> Result<SubsystemCommandInterfaceData, String>;

    /// Ответ чтения подсистемы из разобранных частей.
    fn subsystem_result(
        &self,
        data: SubsystemInfoData,
        tree: Option<Vec<SubsystemTreeNode>>,
        command_interface: Option<SubsystemCommandInterfaceData>,
        support: ObjectSupportData,
    ) -> SubsystemInfoResult;

    /// Проверка содержимого подсистемы. Файлы открывает вызывающий через
    /// `exists` и `read_optional`. Итог: прошла ли проверка, отчёт и путь
    /// проверенного дескриптора.
    fn check_subsystem_content(
        &self,
        text: &str,
        xml_path: PathBuf,
        detailed: bool,
        exists: &mut dyn FnMut(&Path) -> Result<bool, String>,
        read_optional: &mut dyn FnMut(&Path) -> Result<Option<String>, String>,
    ) -> Result<(bool, String, PathBuf), String>;

    /// UUID корневого объекта в дескрипторе: по нему ищется правило поддержки.
    fn support_root_uuid(&self, descriptor: &[u8]) -> Option<String>;

    /// Потоковый разбор `Ext/ParentConfigurations.bin`. Файл читает
    /// вызывающий и отдаёт его частями.
    fn support_stream(&self) -> Box<dyn SupportStateStream>;

    /// Заимствование объекта расширением по его дескриптору.
    fn borrowing(&self, descriptor: &[u8]) -> Result<MetaBorrowing, String>;

    /// Дескриптор заимствует объект родителя. Нечитаемый дескриптор —
    /// не заимствование.
    fn extends_parent(&self, descriptor: &[u8]) -> bool;

    /// Свойства общего модуля. Повреждённая структура дескриптора —
    /// ошибка здесь, а не отсутствие свойства потом.
    fn common_module_properties<'a>(
        &self,
        descriptor: &'a str,
    ) -> Result<Box<dyn MetaPropertySource + 'a>, String>;

    /// Локальные свойства объекта из его дескриптора.
    fn local_info(
        &self,
        descriptor: &[u8],
        target: &MetadataAddress,
        support: MetaSupportStatus,
        source_kind: SourceSetKind,
    ) -> Result<MetaLocalInfo, MetaFailure>;

    /// Каталог объектов вида `kind` от корня источника.
    fn metadata_directory(&self, kind: MetadataKind) -> &'static str;

    /// Все предопределённые элементы из `Ext/Predefined.xml`. `code_type` —
    /// `CodeType` владельца как он записан; без него коды не проверить, и
    /// чтение отказывает. `checkpoint` может остановить разбор.
    fn predefined_items(
        &self,
        bytes: &[u8],
        kind: MetadataKind,
        code_type: Option<&str>,
        checkpoint: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<MetaPredefinedItemsData, String>;

    /// Факты общего макета из его дескриптора.
    fn common_template(
        &self,
        descriptor: &[u8],
        target: &MetadataAddress,
    ) -> Result<CommonTemplateDescriptorFacts, String>;

    /// Адрес и профиль подчинённого объекта по дескриптору. Дескриптор
    /// другого вида — `None`.
    fn child_profile(
        &self,
        descriptor: &[u8],
        owner: &MetadataAddress,
    ) -> Result<Option<(MetadataAddress, MetadataChildProfile)>, String>;

    /// Профиль содержимого формы по её дескриптору.
    fn form_profile(&self, descriptor: &[u8]) -> Result<MetadataChildProfile, String>;

    /// Имена реквизитов и табличных частей объекта с реквизитами каждой из
    /// них. Член без имени пропускается.
    fn object_member_names(&self, descriptor: &[u8]) -> Result<ObjectMemberNames, String>;
}

/// Имена членов объекта, которые видит код его модуля.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjectMemberNames {
    pub attributes: Vec<String>,
    /// Табличные части и имена их реквизитов.
    pub tabular_sections: Vec<(String, Vec<String>)>,
}

/// Прямые свойства одного разобранного дескриптора.
pub trait MetaPropertySource {
    /// Свойство из профиля `META_INFO_PROPERTY_PROFILE`. Нет свойства — `None`.
    fn info(&self, name: &str) -> Result<Option<MetaInfoPropertyValue>, ReadFinding>;

    /// `ObjectBelonging` или `ExtendedConfigurationObject`. Нет свойства — `None`.
    fn borrowing(&self, name: &str) -> Result<Option<String>, ReadFinding>;
}

/// Разбор состояния поддержки по частям файла.
pub trait SupportStateStream: Send {
    /// Следующая часть файла.
    fn feed(&mut self, chunk: &[u8]) -> Result<(), SupportReadError>;

    /// Итог после последней части.
    fn finish(self: Box<Self>) -> Result<SupportState, SupportReadError>;
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
