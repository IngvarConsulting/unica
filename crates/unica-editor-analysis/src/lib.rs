//! Служба LSP демона: документы редактора по адресам `unica://` и ответы о них.
//!
//! Порт [`unica_lsp_contract::host::LspHost`] и типы сообщений задаёт контракт;
//! транспорт принадлежит демону, и служба от него не зависит.
//! Допуск папки, чтение модуля, службы символов и диагностик приходят от
//! продукта через [`LspWorkspaces`].

mod completion;
mod diagnostics;
#[allow(dead_code)]
mod document;
#[allow(dead_code)]
mod documents;
mod host;
mod module_document;
#[allow(dead_code)]
mod position;
mod symbols;
#[allow(dead_code)]
mod uri;

pub use completion::{LspCompletion, LspCompletionAnswer, LspCompletionRequest};
pub use diagnostics::{
    LspAnalysis, LspAnalysisProgress, LspDiagnosticGap, LspDiagnostics, LspFinding, LspSeverity,
};
pub use documents::{ModuleSourceRead, SourceSetLayout};
pub use host::{AdmittedWorkspace, LspActor, LspModuleReads, LspWorkspaces, UnicaLspHost};
pub use module_document::ModuleDocument;
pub use symbols::{LspSymbolAnswer, LspSymbolOperation, LspSymbolRequest, LspSymbols};

#[cfg(test)]
mod service_tests;
