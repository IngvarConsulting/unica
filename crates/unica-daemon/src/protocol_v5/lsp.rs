//! Маршрут LSP в протоколе демона: дверь редактора в тот же демон.
//!
//! Методы, уведомления и исходы — закрытые варианты, как и остальной
//! протокол v5. Тексты документов и адреса передаются как есть; их смысл
//! разбирает служба LSP за портом [`crate::lsp_lane::LspHost`].

use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Длиннейший адрес документа или рабочей папки.
const MAX_LSP_URI_BYTES: usize = 4 * 1024;
/// Рабочих папок у одного соединения.
const MAX_LSP_WORKSPACE_FOLDERS: usize = 64;
const MAX_LSP_FOLDER_NAME_BYTES: usize = 1024;
const MAX_LSP_LANGUAGE_ID_BYTES: usize = 64;
/// Правок в одном `didChange`.
const MAX_LSP_CONTENT_CHANGES: usize = 4 * 1024;
/// Сообщение уведомления редактору.
const MAX_LSP_LOG_MESSAGE_BYTES: usize = 4 * 1024;

/// Сессия LSP: выдаётся демоном на `lsp_open` и живёт, пока открыто
/// соединение, на котором она открыта.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct V5LspSessionId(String);

impl V5LspSessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(super) fn validate(&self) -> Result<(), String> {
        super::validate_uuid_v4(&self.0, "v5 lsp session")
    }
}

impl Default for V5LspSessionId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct V5LspWorkspaceFolder {
    pub uri: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct V5LspPosition {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct V5LspRange {
    pub start: V5LspPosition,
    pub end: V5LspPosition,
}

/// Запрос редактора, на который ждут ответа.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum V5LspCall {
    #[serde(rename = "textDocument/hover")]
    Hover {
        uri: String,
        position: V5LspPosition,
    },
}

impl V5LspCall {
    /// Документ, к состоянию которого относится ответ.
    pub fn document_uri(&self) -> Option<&str> {
        match self {
            Self::Hover { uri, .. } => Some(uri),
        }
    }

    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Hover { uri, .. } => validate_uri(uri),
        }
    }
}

/// Одна правка `didChange`: без диапазона — весь текст документа.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct V5LspContentChange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<V5LspRange>,
    pub text: String,
}

/// Уведомление редактора о документах. Порядок уведомлений одной сессии
/// сохраняется: они идут по соединению, открывшему сессию.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum V5LspNotification {
    #[serde(rename = "textDocument/didOpen")]
    DidOpen {
        uri: String,
        language_id: String,
        version: i32,
        text: String,
    },
    #[serde(rename = "textDocument/didChange")]
    DidChange {
        uri: String,
        version: i32,
        changes: Vec<V5LspContentChange>,
    },
    #[serde(rename = "textDocument/didClose")]
    DidClose { uri: String },
}

impl V5LspNotification {
    pub fn document_uri(&self) -> &str {
        match self {
            Self::DidOpen { uri, .. } | Self::DidChange { uri, .. } | Self::DidClose { uri } => uri,
        }
    }

    fn validate(&self) -> Result<(), String> {
        validate_uri(self.document_uri())?;
        match self {
            Self::DidOpen { language_id, .. } => {
                validate_short_text(language_id, MAX_LSP_LANGUAGE_ID_BYTES, "v5 lsp language id")
            }
            Self::DidChange { changes, .. } if changes.len() > MAX_LSP_CONTENT_CHANGES => {
                Err("v5 lsp didChange carries too many changes".to_string())
            }
            Self::DidChange { .. } | Self::DidClose { .. } => Ok(()),
        }
    }
}

// Тексты документов — исходники пользователя; в журнал они не попадают.
impl fmt::Debug for V5LspNotification {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DidOpen { version, .. } => formatter
                .debug_struct("V5LspDidOpen")
                .field("version", version)
                .finish_non_exhaustive(),
            Self::DidChange {
                version, changes, ..
            } => formatter
                .debug_struct("V5LspDidChange")
                .field("version", version)
                .field("changes", &changes.len())
                .finish_non_exhaustive(),
            Self::DidClose { .. } => formatter.write_str("V5LspDidClose"),
        }
    }
}

/// Как служба принимает тексты документов.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum V5LspTextSync {
    None,
    Full,
    Incremental,
}

/// Возможности, которые служба готова обслужить сейчас.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct V5LspCapabilities {
    pub text_document_sync: V5LspTextSync,
    pub hover: bool,
}

impl V5LspCapabilities {
    pub const NONE: Self = Self {
        text_document_sync: V5LspTextSync::None,
        hover: false,
    };
}

/// Почему запрос не получил ответа. Закрытый набор кодов Unica: поставщик
/// анализа сюда не просачивается.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum V5LspFailure {
    /// Метод не входит в объявленные возможности.
    NotSupported,
    /// Сессия не открыта или уже закрыта.
    SessionUnknown,
    /// Запрос с этим номером уже исполняется в сессии.
    DuplicateRequest,
    /// Очередь LSP полна.
    Busy,
    DeadlineExceeded,
    /// Служба не смогла ответить.
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "result",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum V5LspOutcome {
    /// Markdown подсказки; `None` — подсказки нет.
    Hover {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        contents: Option<String>,
    },
    /// Запрос отменён до ответа; его результат не выдаётся.
    Cancelled {},
    Failed {
        failure: V5LspFailure,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum V5LspMessageLevel {
    Error,
    Warning,
    Info,
}

/// Уведомление демона редактору.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum V5LspServerNotification {
    LogMessage {
        level: V5LspMessageLevel,
        message: String,
    },
}

impl V5LspServerNotification {
    pub(super) fn validate(&self) -> Result<(), String> {
        match self {
            Self::LogMessage { message, .. } if message.len() > MAX_LSP_LOG_MESSAGE_BYTES => {
                Err("v5 lsp log message exceeds its limit".to_string())
            }
            Self::LogMessage { .. } => Ok(()),
        }
    }
}

pub(super) fn validate_open(folders: &[V5LspWorkspaceFolder]) -> Result<(), String> {
    if folders.len() > MAX_LSP_WORKSPACE_FOLDERS {
        return Err("v5 lsp session has too many workspace folders".to_string());
    }
    for folder in folders {
        validate_uri(&folder.uri)?;
        if folder.name.len() > MAX_LSP_FOLDER_NAME_BYTES
            || folder.name.chars().any(char::is_control)
        {
            return Err("v5 lsp workspace folder name is not short text".to_string());
        }
    }
    Ok(())
}

pub(super) fn validate_call(call: &V5LspCall) -> Result<(), String> {
    call.validate()
}

pub(super) fn validate_notification(notification: &V5LspNotification) -> Result<(), String> {
    notification.validate()
}

fn validate_uri(uri: &str) -> Result<(), String> {
    validate_short_text(uri, MAX_LSP_URI_BYTES, "v5 lsp document uri")
}

fn validate_short_text(value: &str, limit: usize, field: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > limit || value.chars().any(char::is_control) {
        return Err(format!("{field} must be non-empty short text"));
    }
    Ok(())
}
