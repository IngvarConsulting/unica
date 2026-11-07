//! Дополнение для редактора: имена, которые может употребить код в месте
//! ввода.
//!
//! Кандидатов собирает служба дополнения Unica по тексту, который видит
//! соединение, метаданным набора и глобальному контексту платформы. Её
//! составляет продукт за портом [`LspCompletion`]; служба LSP переводит
//! позиции в UTF-16 и строит элементы протокола.

use unica_code_intelligence::completion::{Completion, CompletionMethod};
use unica_lsp_contract::protocol::V5LspFailure;

/// Вопрос о месте ввода в модуле.
#[derive(Debug, Clone, Copy)]
pub struct LspCompletionRequest<'a> {
    /// Сессия, буферы которой видны чтению модулей.
    pub session: &'a str,
    pub source_set: &'a str,
    /// Адрес модуля, тело которого — документ.
    pub module: &'a str,
    /// Текст, который видит соединение.
    pub text: &'a str,
    /// Байт текста, на котором стоит курсор.
    pub offset: usize,
}

/// Кандидаты и то, готов ли глобальный контекст платформы.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LspCompletionAnswer {
    /// `None` — в этом месте дополнения нет (комментарий, строка).
    pub completion: Option<Completion>,
    /// `false`, пока члены глобального контекста ещё грузятся: список
    /// неполон, и редактор переспросит.
    pub platform_ready: bool,
}

/// Служба дополнения допущенной рабочей области. Реализует продукт.
pub trait LspCompletion: Send + Sync {
    fn complete(
        &self,
        request: LspCompletionRequest<'_>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<LspCompletionAnswer, V5LspFailure>;

    /// Описание метода, который называет элемент: из этого модуля или из
    /// общего модуля, который его экспортирует.
    fn describe(
        &self,
        session: &str,
        source_set: &str,
        module: &str,
        method: &CompletionMethod,
    ) -> Option<String>;
}
