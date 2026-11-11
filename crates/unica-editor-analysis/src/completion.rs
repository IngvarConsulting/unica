//! Дополнение для редактора: имена, которые может употребить код в месте
//! ввода.
//!
//! Кандидатов собирает служба дополнения Unica по тексту, который видит
//! соединение, метаданным набора и глобальному контексту платформы. Её
//! составляет продукт за портом [`LspCompletion`]; служба LSP переводит
//! позиции в UTF-16 и строит элементы протокола.

use unica_code_model::completion::{Completion, CompletionMethod};
use unica_lsp_contract::protocol::V5LspFailure;
use unica_work_control::cancellation::CancellationToken;

/// Вопрос о месте ввода в модуле.
#[derive(Debug, Clone, Copy)]
pub struct LspCompletionRequest<'a> {
    /// Сессия, буферы которой видны чтению модулей.
    pub session: &'a str,
    pub source_set: &'a str,
    /// Модуль, тело которого — документ.
    pub module: &'a crate::ModuleDocument,
    /// Текст, который видит соединение.
    pub text: &'a str,
    /// Байт текста, на котором стоит курсор.
    pub offset: usize,
}

/// Кандидаты и полнота списка.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LspCompletionAnswer {
    /// `None` — в этом месте дополнения нет (комментарий, строка).
    pub completion: Option<Completion>,
    /// `false`, пока источник не готов (глобальный контекст грузится) или
    /// не ответил (метаданные не прочитались): список неполон, и редактор
    /// переспросит.
    pub complete: bool,
}

/// Служба дополнения допущенной рабочей области. Реализует продукт.
pub trait LspCompletion: Send + Sync {
    /// Чтения источников прерываются, как только `cancellation` отменён.
    fn complete(
        &self,
        request: LspCompletionRequest<'_>,
        cancellation: &CancellationToken,
    ) -> Result<LspCompletionAnswer, V5LspFailure>;

    /// Описание метода, который называет элемент: из этого модуля или из
    /// общего модуля, который его экспортирует.
    fn describe(
        &self,
        session: &str,
        source_set: &str,
        module: &crate::ModuleDocument,
        method: &CompletionMethod,
        cancellation: &CancellationToken,
    ) -> Option<String>;
}
