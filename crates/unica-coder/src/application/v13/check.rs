use crate::application::invocation_store::MAX_CANONICAL_RESULT_BYTES;
use crate::application::result_store::{ViewCursorBinding, ViewCursorStore};
use crate::application::v13::view::PREFERRED_PAGE_BYTES;
use crate::domain::address::QualifiedAddress;
use crate::domain::invocation::DomainResult;
use crate::domain::refusal::{RefusalCode, RefusalDetail};
use serde::Serialize;
use serde_json::{json, Value};
use std::fmt;

const CURSOR_SIZE_PLACEHOLDER: &str = "vc1.00000000000000000000000000000000";

/// Page the diagnostics of one completed node check. The immutable snapshot
/// holds the verdict and every finding; no first page is published unless
/// each later indivisible finding fits the transport and the snapshot fits
/// the bounded cursor store.
pub(crate) fn page_diagnostics(
    cursors: &ViewCursorStore,
    binding: ViewCursorBinding,
    fresh: Option<DomainResult>,
    cursor: Option<&str>,
) -> DomainResult {
    let at = Some(binding.canonical_at.clone());
    if let Some(cursor) = cursor {
        let stored = match cursors.read(cursor, &binding, &binding.source_revision) {
            Ok(stored) => stored,
            Err(error) => {
                return DomainResult::canonical_rejection(
                    at,
                    error.code(),
                    "check cursor is invalid or its source revision changed",
                )
            }
        };
        let page = match prepare_check_page(
            &stored.snapshot.node,
            &stored.snapshot.items,
            stored.offset,
            binding.page_limit,
        ) {
            Ok(page) => page,
            Err(message) => return check_page_refusal(at, message),
        };
        let next = if page.next_offset < stored.snapshot.items.len() {
            match cursors.insert_next(&stored, page.next_offset, cursor) {
                Some(next) => Some(next),
                None => return check_page_refusal(at, "check continuation could not be retained"),
            }
        } else {
            None
        };
        return check_page_result(&stored.snapshot.node, page.items, next, page.stopped_by);
    }

    let Some(mut result) = fresh else {
        return DomainResult::canonical_rejection(
            at,
            RefusalCode::InvalidState,
            "a new check page has no validation result",
        );
    };
    let Some(data) = result.data.as_mut().and_then(Value::as_object_mut) else {
        return DomainResult::canonical_rejection(
            at,
            RefusalCode::InvalidState,
            "check result has no diagnostic collection",
        );
    };
    let Some(Value::Array(items)) = data.remove("diagnostics") else {
        return DomainResult::canonical_rejection(
            at,
            RefusalCode::InvalidState,
            "check result has no diagnostic collection",
        );
    };
    data.insert("diagnosticCount".to_string(), json!(items.len()));
    let node = serde_json::to_value(&result).expect("a check result serializes");
    if let Err(message) = preflight_check_pages(&node, &items) {
        return check_page_refusal(at, message);
    }
    let page = match prepare_check_page(&node, &items, 0, binding.page_limit) {
        Ok(page) => page,
        Err(message) => return check_page_refusal(at, message),
    };
    let next = if page.next_offset < items.len() {
        match cursors.insert_snapshot(binding, node.clone(), items, page.next_offset) {
            Some(next) => Some(next),
            None => {
                return check_page_refusal(
                    at,
                    "check diagnostics exceed the bounded snapshot store",
                )
            }
        }
    } else {
        None
    };
    check_page_result(&node, page.items, next, page.stopped_by)
}

struct PreparedCheckPage {
    items: Vec<Value>,
    next_offset: usize,
    stopped_by: &'static str,
}

fn prepare_check_page(
    node: &Value,
    items: &[Value],
    mut offset: usize,
    limit: usize,
) -> Result<PreparedCheckPage, &'static str> {
    if offset > items.len() {
        return Err("check cursor offset is invalid");
    }
    let base = check_page_result(node, Vec::new(), None, "complete");
    let prefer_bytes = serialized_result_size(&base) <= PREFERRED_PAGE_BYTES;
    let mut page_items = Vec::new();
    let stopped_by = loop {
        if offset == items.len() {
            break "complete";
        }
        if page_items.len() == limit {
            break "limit";
        }
        let mut candidate = page_items.clone();
        candidate.push(items[offset].clone());
        let probe = check_page_result(
            node,
            candidate.clone(),
            (offset + 1 < items.len()).then(|| CURSOR_SIZE_PLACEHOLDER.to_string()),
            "complete",
        );
        let bytes = serialized_result_size(&probe);
        if bytes > MAX_CANONICAL_RESULT_BYTES {
            if page_items.is_empty() {
                return Err("one check diagnostic exceeds the transport result limit");
            }
            break "bytes";
        }
        if prefer_bytes && bytes > PREFERRED_PAGE_BYTES && !page_items.is_empty() {
            break "bytes";
        }
        page_items = candidate;
        offset += 1;
    };
    Ok(PreparedCheckPage {
        items: page_items,
        next_offset: offset,
        stopped_by,
    })
}

fn preflight_check_pages(node: &Value, items: &[Value]) -> Result<(), &'static str> {
    if serialized_result_size(&check_page_result(node, Vec::new(), None, "complete"))
        > MAX_CANONICAL_RESULT_BYTES
    {
        return Err("check result metadata exceed the transport result limit");
    }
    for (index, item) in items.iter().enumerate() {
        let cursor = (index + 1 < items.len()).then(|| CURSOR_SIZE_PLACEHOLDER.to_string());
        // `complete` is the longest stop reason. The last page may be three
        // bytes larger than an intermediate page even without a cursor.
        let probe = check_page_result(node, vec![item.clone()], cursor, "complete");
        if serialized_result_size(&probe) > MAX_CANONICAL_RESULT_BYTES {
            return Err("one check diagnostic exceeds the transport result limit");
        }
    }
    Ok(())
}

fn check_page_result(
    node: &Value,
    items: Vec<Value>,
    cursor: Option<String>,
    stopped_by: &'static str,
) -> DomainResult {
    let mut result: DomainResult =
        serde_json::from_value(node.clone()).expect("stored check result is a domain result");
    result.data.as_mut().expect("check has data")["diagnostics"] = Value::Array(items);
    result.cursor = cursor;
    result.page = Some(json!({"stoppedBy": stopped_by}));
    result
}

fn serialized_result_size(result: &DomainResult) -> usize {
    serde_json::to_vec(result)
        .expect("a check result containing JSON values serializes")
        .len()
}

fn check_page_refusal(at: Option<String>, message: &'static str) -> DomainResult {
    DomainResult::canonical_rejection(at, RefusalCode::ResultTooLarge, message)
}

/// The native validators `unica.check` can run. The list is closed: a node
/// kind owns its validators, and the caller never names one on the wire.
pub(crate) const NATIVE_VALIDATORS: &[CheckValidator] = &[
    CheckValidator::Cf,
    CheckValidator::Cfe,
    CheckValidator::Form,
    CheckValidator::Dcs,
    CheckValidator::Mxl,
    CheckValidator::Role,
    CheckValidator::Subsystem,
    CheckValidator::Interface,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckValidator {
    Cf,
    Cfe,
    Form,
    Dcs,
    Mxl,
    Role,
    Subsystem,
    Interface,
}

impl CheckValidator {
    /// The native validator this step runs, by its operation name. The
    /// read-only format guard of the retired `*.validate` tools is keyed by
    /// the same names, so the canonical check inherits it unchanged.
    pub(crate) const fn native_operation(self) -> &'static str {
        match self {
            Self::Cf => "cf-validate",
            Self::Cfe => "cfe-validate",
            Self::Form => "form-validate",
            Self::Dcs => "dcs-validate",
            Self::Mxl => "mxl-validate",
            Self::Role => "role-validate",
            Self::Subsystem => "subsystem-validate",
            Self::Interface => "interface-validate",
        }
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Cf => "cf",
            Self::Cfe => "cfe",
            Self::Form => "form",
            Self::Dcs => "dcs",
            Self::Mxl => "mxl",
            Self::Role => "role",
            Self::Subsystem => "subsystem",
            Self::Interface => "interface",
        }
    }

    pub(crate) fn supported() -> &'static [Self] {
        NATIVE_VALIDATORS
    }

    /// The validator that owns an address before the node is read: the
    /// export-format guard names it when the read port cannot open the target.
    pub(crate) fn for_unread_address(at: &QualifiedAddress, extension: bool) -> Option<Self> {
        let kind = at
            .segments()
            .last()
            .map(|segment| segment.kind().as_str())
            .unwrap_or_default();
        match kind {
            "Configuration" if extension => Some(Self::Cfe),
            "Configuration" => Some(Self::Cf),
            "Form" => Some(Self::Form),
            "Role" => Some(Self::Role),
            "Subsystem" => Some(Self::Subsystem),
            "Interface" | "CommandInterface" => Some(Self::Interface),
            _ => None,
        }
    }
}

/// What `check` knows about a readable node before choosing its validators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct NodeFacts {
    /// The node lives in a source set of kind `EXTENSION`.
    pub(crate) extension: bool,
    /// A template node's flavour, when the projection states it.
    pub(crate) template: Option<TemplateFlavour>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TemplateFlavour {
    DataCompositionSchema,
    SpreadsheetDocument,
}

/// One validation step of a check plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckStep {
    Native(CheckValidator),
    /// The typed metadata validator of one object descriptor.
    Meta,
    /// Диагностика BSL провайдером анализа. Отдельный шаг, а не «родной»
    /// валидатор: он поднимает внешний инструмент, и это надо видеть в плане.
    Bsl,
}

impl CheckStep {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Native(validator) => validator.name(),
            Self::Meta => "meta",
            Self::Bsl => "bsl",
        }
    }
}

/// The validators a readable node owns, in the order they run. An empty plan
/// means the node has no validator and `check` reports readability only.
pub(crate) fn plan_for_node(kind: &str, facts: NodeFacts) -> Vec<CheckStep> {
    match kind {
        "Configuration" if facts.extension => vec![CheckStep::Native(CheckValidator::Cfe)],
        "Configuration" => vec![CheckStep::Native(CheckValidator::Cf)],
        "Form" => vec![CheckStep::Native(CheckValidator::Form)],
        "Template" => match facts.template {
            Some(TemplateFlavour::DataCompositionSchema) => {
                vec![CheckStep::Native(CheckValidator::Dcs)]
            }
            Some(TemplateFlavour::SpreadsheetDocument) => {
                vec![CheckStep::Native(CheckValidator::Mxl)]
            }
            None => Vec::new(),
        },
        // The typed metadata validator reads object descriptors only; roles
        // and subsystems keep their own validators.
        "Role" => vec![CheckStep::Native(CheckValidator::Role)],
        "Subsystem" => vec![CheckStep::Native(CheckValidator::Subsystem)],
        "Interface" | "CommandInterface" => vec![CheckStep::Native(CheckValidator::Interface)],
        // Узел с кодом проверяется анализатором BSL. Прежде план у него был
        // пуст, и `check` отвечал «читается» — то есть молчал о находках,
        // ради которых его и зовут.
        "Module" | "Body" => vec![CheckStep::Bsl],
        other if crate::domain::metadata::MetadataKind::parse(other).is_ok() => {
            vec![CheckStep::Meta]
        }
        _ => Vec::new(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CheckError {
    BadValue { field: String, message: String },
    DependencyUnavailable,
}

impl CheckError {
    /// Уточнение отказа, когда код покрывает несколько исходов.
    ///
    /// Недоступная зависимость валидатора — это отсутствующий поставщик:
    /// собственного кода у неё нет с тех пор, как `dependency_unavailable`
    /// снят из словаря как синоним. Уточнение держит код при себе, поэтому
    /// разойтись они не могут.
    pub(crate) const fn detail(&self) -> Option<RefusalDetail> {
        match self {
            Self::BadValue { .. } => None,
            Self::DependencyUnavailable => Some(RefusalDetail::ProviderAbsent),
        }
    }

    pub(crate) const fn code(&self) -> RefusalCode {
        match self {
            Self::BadValue { .. } => RefusalCode::BadValue,
            Self::DependencyUnavailable => RefusalDetail::ProviderAbsent.code(),
        }
    }
}

impl fmt::Display for CheckError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadValue { field, message } => {
                write!(formatter, "{field}: {message}")
            }
            Self::DependencyUnavailable => {
                formatter.write_str("the selected validator dependency is unavailable")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CheckDiagnostic {
    severity: String,
    code: String,
    message: String,
}

impl CheckDiagnostic {
    /// A warning that keeps the validator verdict: the closed format codes of
    /// the export-format guard travel through here.
    pub(crate) fn warning(code: impl Into<String>, message: &str) -> Self {
        Self {
            severity: "warning".to_string(),
            code: code.into(),
            message: sanitize_message(message),
        }
    }

    pub(crate) fn severity(&self) -> &str {
        &self.severity
    }

    pub(crate) fn code(&self) -> &str {
        &self.code
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeCheckOutcome {
    ok: bool,
    diagnostics: Vec<CheckDiagnostic>,
    unavailable: bool,
}

impl NativeCheckOutcome {
    pub(crate) fn passed() -> Self {
        Self {
            ok: true,
            diagnostics: Vec::new(),
            unavailable: false,
        }
    }

    pub(crate) fn failed(
        diagnostics: impl IntoIterator<Item = (&'static str, &'static str, &'static str)>,
    ) -> Self {
        Self {
            ok: false,
            diagnostics: diagnostics
                .into_iter()
                .map(|(severity, code, message)| CheckDiagnostic {
                    severity: severity.to_string(),
                    code: code.to_string(),
                    message: sanitize_message(message),
                })
                .collect(),
            unavailable: false,
        }
    }

    /// Prepends one diagnostic without touching the verdict: a read-only
    /// format warning is reported first, the validator findings follow.
    pub(crate) fn with_leading_diagnostic(mut self, diagnostic: CheckDiagnostic) -> Self {
        self.diagnostics.insert(0, diagnostic);
        self
    }

    pub(crate) fn unavailable(_detail: &str) -> Self {
        Self {
            ok: false,
            diagnostics: Vec::new(),
            unavailable: true,
        }
    }

    pub(crate) fn from_adapter(outcome: &crate::application::AdapterOutcome) -> Self {
        let mut diagnostics = outcome
            .errors
            .iter()
            .map(|message| CheckDiagnostic {
                severity: "error".to_string(),
                code: "native_validation_error".to_string(),
                message: sanitize_message(message),
            })
            .collect::<Vec<_>>();
        diagnostics.extend(outcome.warnings.iter().map(|message| CheckDiagnostic {
            severity: "warning".to_string(),
            code: "native_validation_warning".to_string(),
            message: sanitize_message(message),
        }));
        Self {
            ok: outcome.ok,
            diagnostics,
            unavailable: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CheckResult {
    at: String,
    kind: String,
    validator: String,
    ok: bool,
    diagnostics: Vec<CheckDiagnostic>,
}

impl CheckResult {
    pub(crate) fn ok(&self) -> bool {
        self.ok
    }

    pub(crate) fn diagnostics(&self) -> &[CheckDiagnostic] {
        &self.diagnostics
    }

    pub(crate) fn validator(&self) -> &str {
        &self.validator
    }

    pub(crate) fn raw_stream(&self) -> Option<&str> {
        None
    }
}

pub(crate) fn normalize_native_outcome(
    at: &QualifiedAddress,
    kind: &str,
    validator: CheckValidator,
    native: NativeCheckOutcome,
) -> Result<CheckResult, CheckError> {
    if native.unavailable {
        return Err(CheckError::DependencyUnavailable);
    }
    Ok(CheckResult {
        at: at.to_string(),
        kind: kind.to_string(),
        validator: validator.name().to_string(),
        ok: native.ok,
        diagnostics: native.diagnostics,
    })
}

fn sanitize_message(message: &str) -> String {
    message
        .split_whitespace()
        .filter(|token| !token.contains('/') && !token.contains('\\'))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_native_outcome, page_diagnostics, plan_for_node, CheckStep, CheckValidator,
        NativeCheckOutcome, NodeFacts, TemplateFlavour,
    };
    use crate::application::result_store::{ViewCursorBinding, ViewCursorStore, DEFAULT_TTL};
    use crate::domain::address::QualifiedAddress;
    use crate::domain::invocation::DomainResult;
    use serde_json::json;

    fn check_binding(at: &str, revision: &str, limit: usize) -> ViewCursorBinding {
        ViewCursorBinding {
            canonical_at: at.to_string(),
            projection: "check".to_string(),
            normalized_filter: String::new(),
            source_set_identity: "workspace:main".to_string(),
            source_revision: revision.to_string(),
            page_limit: limit,
        }
    }

    fn check_result(count: usize) -> DomainResult {
        let mut result = DomainResult::success("validation reported findings");
        result.at = Some("main:CommonModule.Example.Module".to_string());
        result.rev = Some("revision-1".to_string());
        result.data = Some(json!({
            "at": "main:CommonModule.Example.Module",
            "kind": "Module",
            "status": "failed",
            "validators": ["bsl"],
            "diagnostics": (0..count).map(|index| json!({
                "severity": "error",
                "code": format!("finding-{index}"),
                "message": format!("finding {index}"),
            })).collect::<Vec<_>>(),
        }));
        result
    }

    #[test]
    fn check_pages_preserve_verdict_and_every_diagnostic_with_replay() {
        let store = ViewCursorStore::default();
        let binding = check_binding("main:CommonModule.Example.Module", "revision-1", 20);
        let mut page = page_diagnostics(&store, binding.clone(), Some(check_result(53)), None);
        assert!(page.ok, "{page:?}");
        assert_eq!(page.data.as_ref().unwrap()["status"], "failed");
        assert_eq!(page.data.as_ref().unwrap()["diagnosticCount"], 53);
        assert_eq!(page.page.as_ref().unwrap()["stoppedBy"], "limit");
        let mut found = Vec::new();
        loop {
            found.extend(
                page.data.as_ref().unwrap()["diagnostics"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|item| item["code"].as_str().unwrap().to_string()),
            );
            let Some(cursor) = page.cursor.clone() else {
                break;
            };
            let replay = page_diagnostics(&store, binding.clone(), None, Some(&cursor));
            assert_eq!(
                replay,
                page_diagnostics(&store, binding.clone(), None, Some(&cursor))
            );
            assert_eq!(replay.data.as_ref().unwrap()["status"], "failed");
            page = replay;
        }
        assert_eq!(
            found,
            (0..53)
                .map(|index| format!("finding-{index}"))
                .collect::<Vec<_>>()
        );
        assert_eq!(page.page.as_ref().unwrap()["stoppedBy"], "complete");
    }

    #[test]
    fn check_cursor_rejects_other_node_and_stale_source() {
        let store = ViewCursorStore::default();
        let binding = check_binding("main:CommonModule.Example.Module", "revision-1", 20);
        let first = page_diagnostics(&store, binding.clone(), Some(check_result(21)), None);
        let cursor = first.cursor.as_deref().unwrap();
        let other = page_diagnostics(
            &store,
            check_binding("main:CommonModule.Other.Module", "revision-1", 20),
            None,
            Some(cursor),
        );
        assert_eq!(other.diagnostics[0]["code"], "invalid_cursor");
        let stale = page_diagnostics(
            &store,
            check_binding("main:CommonModule.Example.Module", "revision-2", 20),
            None,
            Some(cursor),
        );
        assert_eq!(stale.diagnostics[0]["code"], "stale_cursor");
    }

    #[test]
    fn check_returns_whole_large_finding_and_refuses_late_oversize_before_page_one() {
        let store = ViewCursorStore::default();
        let binding = check_binding("main:CommonModule.Example.Module", "revision-1", 20);
        let mut large = check_result(2);
        large.data.as_mut().unwrap()["diagnostics"][0]["message"] = json!("x".repeat(70_000));
        let first = page_diagnostics(&store, binding.clone(), Some(large), None);
        assert!(first.ok, "{first:?}");
        assert_eq!(
            first.data.as_ref().unwrap()["diagnostics"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(first.page.as_ref().unwrap()["stoppedBy"], "bytes");
        let next = page_diagnostics(&store, binding.clone(), None, first.cursor.as_deref());
        assert_eq!(
            next.data.as_ref().unwrap()["diagnostics"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        let mut oversize = check_result(2);
        oversize.data.as_mut().unwrap()["diagnostics"][1]["message"] =
            json!("x".repeat(8 * 1024 * 1024));
        let refused = page_diagnostics(&store, binding, Some(oversize), None);
        assert!(!refused.ok);
        assert_eq!(refused.diagnostics[0]["code"], "result_too_large");
        assert!(refused.cursor.is_none());
    }

    #[test]
    fn check_refuses_before_first_page_when_snapshot_cannot_be_retained() {
        let store = ViewCursorStore::new(DEFAULT_TTL, 128, 400);
        let result = page_diagnostics(
            &store,
            check_binding("main:CommonModule.Example.Module", "revision-1", 20),
            Some(check_result(21)),
            None,
        );
        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], "result_too_large");
        assert!(result.cursor.is_none());
    }

    #[test]
    fn late_finding_at_transport_edge_is_refused_before_issuing_a_cursor() {
        let mut result = check_result(2);
        result.data.as_mut().unwrap()["diagnostics"][1]["message"] = json!("");
        let mut node_result = result.clone();
        let node_data = node_result.data.as_mut().unwrap().as_object_mut().unwrap();
        node_data.remove("diagnostics");
        node_data.insert("diagnosticCount".to_string(), json!(2));
        let node = serde_json::to_value(node_result).unwrap();
        let item = result.data.as_ref().unwrap()["diagnostics"][1].clone();
        let baseline = super::check_page_result(&node, vec![item], None, "bytes");
        let message_len = crate::application::invocation_store::MAX_CANONICAL_RESULT_BYTES
            - super::serialized_result_size(&baseline);
        result.data.as_mut().unwrap()["diagnostics"][1]["message"] = json!("x".repeat(message_len));

        let store = ViewCursorStore::default();
        let answer = page_diagnostics(
            &store,
            check_binding("main:CommonModule.Example.Module", "revision-1", 1),
            Some(result),
            None,
        );
        assert!(!answer.ok, "{answer:?}");
        assert_eq!(answer.diagnostics[0]["code"], "result_too_large");
        assert!(answer.cursor.is_none());
    }

    #[test]
    fn every_node_kind_owns_its_validators_without_a_caller_choice() {
        let plain = NodeFacts::default();
        assert_eq!(
            plan_for_node("Configuration", plain),
            [CheckStep::Native(CheckValidator::Cf)]
        );
        assert_eq!(
            plan_for_node(
                "Configuration",
                NodeFacts {
                    extension: true,
                    ..plain
                }
            ),
            [CheckStep::Native(CheckValidator::Cfe)]
        );
        assert_eq!(
            plan_for_node("Form", plain),
            [CheckStep::Native(CheckValidator::Form)]
        );
        assert_eq!(
            plan_for_node(
                "Template",
                NodeFacts {
                    template: Some(TemplateFlavour::DataCompositionSchema),
                    ..plain
                }
            ),
            [CheckStep::Native(CheckValidator::Dcs)]
        );
        assert_eq!(
            plan_for_node(
                "Template",
                NodeFacts {
                    template: Some(TemplateFlavour::SpreadsheetDocument),
                    ..plain
                }
            ),
            [CheckStep::Native(CheckValidator::Mxl)]
        );
        assert!(plan_for_node("Template", plain).is_empty());
        assert_eq!(
            plan_for_node("Role", plain),
            [CheckStep::Native(CheckValidator::Role)]
        );
        assert_eq!(
            plan_for_node("Subsystem", plain),
            [CheckStep::Native(CheckValidator::Subsystem)]
        );
        assert_eq!(
            plan_for_node("Interface", plain),
            [CheckStep::Native(CheckValidator::Interface)]
        );
        assert_eq!(plan_for_node("Catalog", plain), [CheckStep::Meta]);
        // Узел с кодом owns анализатор BSL. Прежде план у него был пуст, и
        // `check` отвечал «читается», молча пропуская находки, ради которых
        // его и зовут.
        assert_eq!(plan_for_node("Module", plain), [CheckStep::Bsl]);
        assert_eq!(plan_for_node("Body", plain), [CheckStep::Bsl]);
        // Метод разбирается не отдельно: анализатор читает модуль целиком, и
        // отдельный шаг на методе поднимал бы инструмент дважды на один файл.
        assert!(plan_for_node("Method", plain).is_empty());
    }

    #[test]
    fn the_validator_registry_maps_every_step_to_its_native_operation() {
        for validator in CheckValidator::supported() {
            assert!(validator.native_operation().ends_with("-validate"));
            assert!(!validator.name().is_empty());
        }
        assert_eq!(CheckValidator::Cf.native_operation(), "cf-validate");
        assert_eq!(CheckValidator::Form.native_operation(), "form-validate");
        assert_eq!(CheckStep::Meta.name(), "meta");
    }

    #[test]
    fn an_unread_address_still_names_the_validator_that_guards_its_format() {
        let root = QualifiedAddress::parse("main:Configuration").unwrap();
        assert_eq!(
            CheckValidator::for_unread_address(&root, false),
            Some(CheckValidator::Cf)
        );
        assert_eq!(
            CheckValidator::for_unread_address(&root, true),
            Some(CheckValidator::Cfe)
        );
        let form = QualifiedAddress::parse("main:Catalog.Items.Form.List").unwrap();
        assert_eq!(
            CheckValidator::for_unread_address(&form, false),
            Some(CheckValidator::Form)
        );
        let module = QualifiedAddress::parse("main:CommonModule.Common").unwrap();
        assert_eq!(CheckValidator::for_unread_address(&module, false), None);
    }

    #[test]
    fn check_result_normalizes_diagnostics_without_native_stream_or_path() {
        let at = QualifiedAddress::parse("main:Configuration").unwrap();
        let native = NativeCheckOutcome::failed(vec![
            (
                "error",
                "invalid_root",
                "XML parse failed in /private/workspace/Configuration.xml",
            ),
            (
                "warning",
                "format_warning",
                "provider /usr/local/bin/engine reported a warning",
            ),
        ]);
        let result =
            normalize_native_outcome(&at, "Configuration", CheckValidator::Cf, native).unwrap();
        assert!(!result.ok());
        assert_eq!(result.validator(), "cf");
        assert_eq!(result.diagnostics().len(), 2);
        assert_eq!(result.diagnostics()[0].code(), "invalid_root");
        assert!(!result.diagnostics()[0].message().contains("/private/"));
        assert!(!result.diagnostics()[1].message().contains("/usr/local/"));
        assert!(result.raw_stream().is_none());
    }

    #[test]
    fn unavailable_validator_is_typed_dependency_failure() {
        let at = QualifiedAddress::parse("main:Configuration").unwrap();
        let error = normalize_native_outcome(
            &at,
            "Configuration",
            CheckValidator::Cf,
            NativeCheckOutcome::unavailable("validator engine is not installed"),
        )
        .unwrap_err();
        assert_eq!(error.code().as_str(), "provider_unavailable");
        assert!(!error.to_string().contains("engine"));
    }
}
