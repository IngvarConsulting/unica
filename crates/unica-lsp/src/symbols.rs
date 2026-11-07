//! Символы модуля для редактора: где объявлено имя, что оно такое и где
//! его используют.
//!
//! Ответ даёт служба символов Unica в своей модели: логические адреса,
//! диапазоны и коды неполноты. Её составляет продукт за портом
//! [`LspSymbols`], а служба LSP переводит места в адреса `unica://`
//! пространства документа и отбрасывает те, что редактор не откроет.

use unica_code_intelligence::symbols::{
    Incompleteness, SymbolAnswer, SymbolInfo, SymbolLocation, SymbolPosition, SymbolRequestContext,
};
use unica_lsp_contract::protocol::V5LspFailure;

/// Что спрашивают о позиции в модуле.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LspSymbolOperation {
    Definition,
    Hover,
    References,
}

/// Вопрос о позиции в модуле одного набора исходников.
#[derive(Debug, Clone, Copy)]
pub struct LspSymbolRequest<'a> {
    pub source_set: &'a str,
    /// Адрес модуля, тело которого — документ: `main:CommonModule.X`,
    /// `main:Catalog.X.Module.Object`. Места ответа называют модули так же.
    pub module: &'a str,
    /// Открытый буфер сессии; `None` — модуль читается сохранённым.
    pub buffer: Option<&'a str>,
    pub position: SymbolPosition,
    pub operation: LspSymbolOperation,
}

/// Ответ службы символов: места или сведения об имени.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LspSymbolAnswer {
    Places(SymbolAnswer<Vec<SymbolLocation>>),
    Info(SymbolAnswer<Option<SymbolInfo>>),
}

/// Служба символов допущенной рабочей области. Реализует продукт: у
/// редактора и у MCP одни поставщик, индекс имён и чтение исходников.
pub trait LspSymbols: Send + Sync {
    /// Поставщик получает срок и отмену запроса редактора и уступает
    /// анализатор вызову MCP.
    fn ask(
        &self,
        request: LspSymbolRequest<'_>,
        context: &SymbolRequestContext<'_>,
    ) -> Result<LspSymbolAnswer, V5LspFailure>;
}

/// Код причины неполноты, как его видит редактор.
pub(crate) fn reason_code(reason: &Incompleteness) -> &'static str {
    match reason {
        Incompleteness::Stale => "stale",
        Incompleteness::ProviderNotReady => "provider_not_ready",
        Incompleteness::ProviderUnavailable => "provider_unavailable",
        Incompleteness::Capped => "capped",
        Incompleteness::OtherModules => "other_modules",
        Incompleteness::NamesNotIndexed => "names_not_indexed",
        Incompleteness::UntypedReferences => "untyped_references",
        Incompleteness::Unaddressable(_) => "unaddressable",
    }
}
