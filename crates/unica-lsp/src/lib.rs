//! Служба LSP демона: документы редактора по адресам `unica://` и ответы о них.
//!
//! Порт [`unica_lsp_contract::host::LspHost`] и типы сообщений задаёт контракт;
//! транспорт принадлежит демону, и служба от него не зависит.
//! Допуск папки и чтение модуля приходят от продукта через [`LspWorkspaces`].

#[allow(dead_code)]
mod document;
#[allow(dead_code)]
mod documents;
mod host;
#[allow(dead_code)]
mod position;
#[allow(dead_code)]
mod uri;

pub use documents::{ModuleSourceRead, SourceSetLayout};
pub use host::{AdmittedWorkspace, LspActor, LspModuleReads, LspWorkspaces, UnicaLspHost};

#[cfg(test)]
mod service_tests;
