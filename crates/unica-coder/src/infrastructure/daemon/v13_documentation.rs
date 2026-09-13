//! `unica.docs` отвечает до допуска рабочей области.
//!
//! Справка — вопрос о платформе и о стандартах разработки, а не о наборах
//! исходников: поставщики читают установку платформы, сеть и политику
//! проекта, и ни один из них не адресуется логическим адресом. Единственный
//! свод, который читал бы рабочее пространство, —
//! `configuration-documentation`, — до actor-owned nofollow-читателя отвечает
//! типизированным `unsupported_source`, поэтому аренда исходников актору
//! ничего не даёт и на входе не требуется.
//!
//! Без этого маршрута дорога «до рабочей области» обрывалась: `unica.view {}`
//! и словарь `unica.run` объясняют, чего не хватает, а спросить справку о том,
//! как это завести, было нельзя — допуск отвечал общим отказом. Маршрут
//! повторяет форму выгрузок ИБ: подготовка до admission, исполнение обычным
//! конвейером — со своим cutoff и Task, потому что поставщики ходят в сеть.

use super::protocol::InvocationRequest;
use crate::application::invocation_store::ToolIdentity;
use crate::domain::cancellation::CancellationToken;
use crate::domain::invocation::{DomainResult, SafeIdentityHash};
use crate::domain::refusal::RefusalCode;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::workspace::discover_workspace;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub(super) struct PreparedDocumentationSearch {
    query: String,
    source: Option<String>,
    context: WorkspaceContext,
}

pub(super) enum Preparation {
    NotApplicable,
    Rejected(Box<DomainResult>),
    Ready(Arc<PreparedDocumentationSearch>),
}

pub(super) fn prepare(request: &InvocationRequest) -> Preparation {
    if request.tool() != ToolIdentity::Docs {
        return Preparation::NotApplicable;
    }
    let arguments = request.arguments();
    let Some(query) = arguments.get("query").and_then(Value::as_str) else {
        return reject(
            RefusalCode::BadValue,
            "docs requires string argument `query`",
        );
    };
    let source = match arguments.get("source") {
        None => None,
        Some(Value::String(source)) => Some(source.clone()),
        Some(_) => return reject(RefusalCode::BadValue, "docs source must be a string"),
    };
    // Корень нужен политике сети и закреплённой версии платформы, а не
    // допуску исходников: пустой каталог — это отсутствие ограничений, и
    // обнаружение здесь не отказывает по их отсутствию.
    let context = match discover_workspace(Some(PathBuf::from(request.workspace_hint()))) {
        Ok(context) => context,
        Err(error) => {
            return reject(
                RefusalCode::ProviderUnavailable,
                format!("workspace discovery failed: {error}"),
            )
        }
    };
    Preparation::Ready(Arc::new(PreparedDocumentationSearch {
        query: query.to_string(),
        source,
        context,
    }))
}

fn reject(code: RefusalCode, message: impl Into<String>) -> Preparation {
    Preparation::Rejected(Box::new(DomainResult::canonical_rejection(
        None, code, message,
    )))
}

impl PreparedDocumentationSearch {
    pub(super) fn workspace_identity_hash(&self) -> SafeIdentityHash {
        let mut hasher = Sha256::new();
        hasher.update(b"unica-v13-documentation-workspace-v1\0");
        hasher.update(self.context.workspace_root.as_os_str().as_encoded_bytes());
        SafeIdentityHash::from_sha256(hasher.finalize().into())
    }

    pub(super) fn execute(&self, cancellation: CancellationToken) -> DomainResult {
        crate::infrastructure::application_ports::canonical_v13_docs_search(
            &self.context,
            &self.query,
            self.source.as_deref(),
            &cancellation,
        )
    }
}
