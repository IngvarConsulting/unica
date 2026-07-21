use crate::domain::discovery::ArtifactId;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

const DISCOVER_ALLOWED_ARGS: &[&str] = &[
    "concepts",
    "cwd",
    "limits",
    "mode",
    "objects",
    "searchTerms",
    "sourceDir",
    "task",
];

const MAX_TASK_BYTES: usize = 8_192;
const MAX_CONCEPTS: usize = 64;
const MAX_SEARCH_TERMS: usize = 128;
const MAX_OBJECTS: usize = 128;
const MAX_ARRAY_TEXT_BYTES: usize = 256;
const MAX_OBJECT_BYTES: usize = 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiscoveryMode {
    Explore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiscoverRequest {
    cwd: Option<PathBuf>,
    mode: DiscoveryMode,
    task: String,
    source_dir: Option<PathBuf>,
    concepts: Vec<String>,
    search_terms: Vec<String>,
    objects: Vec<ArtifactId>,
    limits: DiscoveryLimits,
}

impl DiscoverRequest {
    pub(crate) fn cwd(&self) -> Option<&Path> {
        self.cwd.as_deref()
    }

    pub(crate) fn mode(&self) -> DiscoveryMode {
        self.mode
    }

    pub(crate) fn task(&self) -> &str {
        &self.task
    }

    pub(crate) fn source_dir(&self) -> Option<&Path> {
        self.source_dir.as_deref()
    }

    pub(crate) fn concepts(&self) -> &[String] {
        &self.concepts
    }

    pub(crate) fn search_terms(&self) -> &[String] {
        &self.search_terms
    }

    pub(crate) fn objects(&self) -> &[ArtifactId] {
        &self.objects
    }

    pub(crate) fn limits(&self) -> DiscoveryLimits {
        self.limits
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DiscoveryLimits {
    max_files: MaxFiles,
    max_bytes: MaxBytes,
    max_evidence: MaxEvidence,
    max_candidates: MaxCandidates,
    max_graph_depth: MaxGraphDepth,
}

impl DiscoveryLimits {
    pub(crate) fn max_files(self) -> MaxFiles {
        self.max_files
    }

    pub(crate) fn max_bytes(self) -> MaxBytes {
        self.max_bytes
    }

    pub(crate) fn max_evidence(self) -> MaxEvidence {
        self.max_evidence
    }

    pub(crate) fn max_candidates(self) -> MaxCandidates {
        self.max_candidates
    }

    pub(crate) fn max_graph_depth(self) -> MaxGraphDepth {
        self.max_graph_depth
    }
}

impl Default for DiscoveryLimits {
    fn default() -> Self {
        Self {
            max_files: MaxFiles(20_000),
            max_bytes: MaxBytes(268_435_456),
            max_evidence: MaxEvidence(2_000),
            max_candidates: MaxCandidates(100),
            max_graph_depth: MaxGraphDepth(12),
        }
    }
}

macro_rules! limit_type {
    ($name:ident, $inner:ty) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) struct $name($inner);

        impl $name {
            pub(crate) fn get(self) -> $inner {
                self.0
            }
        }
    };
}

limit_type!(MaxFiles, u32);
limit_type!(MaxBytes, u64);
limit_type!(MaxEvidence, u16);
limit_type!(MaxCandidates, u16);
limit_type!(MaxGraphDepth, u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiscoveryContractErrorCode {
    UnknownField,
    MissingField,
    InvalidType,
    InvalidMode,
    TextBytesOutOfRange,
    TooManyItems,
    DuplicateValue,
    InvalidArtifactId,
    InvalidSourceDir,
    LimitOutOfRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiscoveryContractError {
    code: DiscoveryContractErrorCode,
    message: String,
}

impl DiscoveryContractError {
    fn new(code: DiscoveryContractErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) fn code(&self) -> DiscoveryContractErrorCode {
        self.code
    }
}

impl fmt::Display for DiscoveryContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DiscoveryContractError {}

pub(crate) fn discover_allowed_args() -> &'static [&'static str] {
    DISCOVER_ALLOWED_ARGS
}

pub(crate) fn parse_discover_request(
    args: &Map<String, Value>,
) -> Result<DiscoverRequest, DiscoveryContractError> {
    reject_unknown_fields(args)?;
    let mode = parse_mode(args)?;
    let task = required_bounded_text(args, "task", MAX_TASK_BYTES)?;
    let cwd = optional_path(args, "cwd")?;
    let source_dir = optional_source_dir(args)?;
    let concepts = parse_text_array(args, "concepts", MAX_CONCEPTS, MAX_ARRAY_TEXT_BYTES)?;
    let search_terms =
        parse_text_array(args, "searchTerms", MAX_SEARCH_TERMS, MAX_ARRAY_TEXT_BYTES)?;
    let objects = parse_artifact_array(args)?;
    let limits = parse_limits(args)?;

    Ok(DiscoverRequest {
        cwd,
        mode,
        task,
        source_dir,
        concepts,
        search_terms,
        objects,
        limits,
    })
}

fn reject_unknown_fields(args: &Map<String, Value>) -> Result<(), DiscoveryContractError> {
    if let Some(key) = args
        .keys()
        .find(|key| !DISCOVER_ALLOWED_ARGS.contains(&key.as_str()))
    {
        return Err(DiscoveryContractError::new(
            DiscoveryContractErrorCode::UnknownField,
            format!("unica.project.discover does not accept argument `{key}`"),
        ));
    }
    Ok(())
}

fn parse_mode(args: &Map<String, Value>) -> Result<DiscoveryMode, DiscoveryContractError> {
    match args.get("mode") {
        None => Err(DiscoveryContractError::new(
            DiscoveryContractErrorCode::MissingField,
            "unica.project.discover requires `mode`",
        )),
        Some(Value::String(mode)) if mode == "explore" => Ok(DiscoveryMode::Explore),
        Some(Value::String(mode)) => Err(DiscoveryContractError::new(
            DiscoveryContractErrorCode::InvalidMode,
            format!("unica.project.discover mode `{mode}` is invalid; expected `explore`"),
        )),
        Some(value) => Err(invalid_type("mode", "string", value)),
    }
}

fn required_bounded_text(
    args: &Map<String, Value>,
    field: &str,
    maximum_bytes: usize,
) -> Result<String, DiscoveryContractError> {
    let value = args.get(field).ok_or_else(|| {
        DiscoveryContractError::new(
            DiscoveryContractErrorCode::MissingField,
            format!("unica.project.discover requires `{field}`"),
        )
    })?;
    let text = value
        .as_str()
        .ok_or_else(|| invalid_type(field, "string", value))?
        .trim();
    validate_text_bytes(field, text, maximum_bytes)?;
    Ok(text.to_string())
}

fn validate_text_bytes(
    field: &str,
    text: &str,
    maximum_bytes: usize,
) -> Result<(), DiscoveryContractError> {
    if text.is_empty() || text.len() > maximum_bytes {
        return Err(DiscoveryContractError::new(
            DiscoveryContractErrorCode::TextBytesOutOfRange,
            format!(
                "unica.project.discover `{field}` must contain 1..={maximum_bytes} UTF-8 bytes"
            ),
        ));
    }
    Ok(())
}

fn optional_path(
    args: &Map<String, Value>,
    field: &str,
) -> Result<Option<PathBuf>, DiscoveryContractError> {
    let Some(value) = args.get(field) else {
        return Ok(None);
    };
    let text = value
        .as_str()
        .ok_or_else(|| invalid_type(field, "string", value))?;
    Ok(Some(PathBuf::from(text)))
}

fn optional_source_dir(
    args: &Map<String, Value>,
) -> Result<Option<PathBuf>, DiscoveryContractError> {
    let Some(value) = args.get("sourceDir") else {
        return Ok(None);
    };
    let text = value
        .as_str()
        .ok_or_else(|| invalid_type("sourceDir", "string", value))?
        .trim();
    if text.is_empty() || text.starts_with(['/', '\\']) || text.contains(':') || text.contains('\0')
    {
        return Err(invalid_source_dir());
    }

    let portable = text.replace('\\', "/");
    let mut normalized = Vec::new();
    for component in portable.split('/') {
        match component {
            "" | ".." => return Err(invalid_source_dir()),
            "." => {}
            normal => normalized.push(normal),
        }
    }
    let normalized = if normalized.is_empty() {
        ".".to_string()
    } else {
        normalized.join("/")
    };
    Ok(Some(PathBuf::from(normalized)))
}

fn invalid_source_dir() -> DiscoveryContractError {
    DiscoveryContractError::new(
        DiscoveryContractErrorCode::InvalidSourceDir,
        "unica.project.discover `sourceDir` must be a non-empty contained relative path",
    )
}

fn parse_text_array(
    args: &Map<String, Value>,
    field: &str,
    maximum_items: usize,
    maximum_bytes: usize,
) -> Result<Vec<String>, DiscoveryContractError> {
    let Some(value) = args.get(field) else {
        return Ok(Vec::new());
    };
    let items = value
        .as_array()
        .ok_or_else(|| invalid_type(field, "array", value))?;
    validate_item_count(field, items.len(), maximum_items)?;
    let mut normalized = BTreeSet::new();
    items
        .iter()
        .map(|value| {
            let text = value
                .as_str()
                .ok_or_else(|| invalid_type(field, "string array item", value))?
                .trim();
            validate_text_bytes(field, text, maximum_bytes)?;
            reject_duplicate(field, text, &mut normalized)?;
            Ok(text.to_string())
        })
        .collect()
}

fn parse_artifact_array(
    args: &Map<String, Value>,
) -> Result<Vec<ArtifactId>, DiscoveryContractError> {
    let Some(value) = args.get("objects") else {
        return Ok(Vec::new());
    };
    let items = value
        .as_array()
        .ok_or_else(|| invalid_type("objects", "array", value))?;
    validate_item_count("objects", items.len(), MAX_OBJECTS)?;
    let mut normalized = BTreeSet::new();
    items
        .iter()
        .map(|value| {
            let text = value
                .as_str()
                .ok_or_else(|| invalid_type("objects", "string array item", value))?
                .trim();
            validate_text_bytes("objects", text, MAX_OBJECT_BYTES)?;
            let artifact = ArtifactId::parse(text).map_err(|error| {
                DiscoveryContractError::new(
                    DiscoveryContractErrorCode::InvalidArtifactId,
                    format!("unica.project.discover object `{text}` is invalid: {error}"),
                )
            })?;
            if !normalized.insert(artifact.clone()) {
                return Err(DiscoveryContractError::new(
                    DiscoveryContractErrorCode::DuplicateValue,
                    format!("unica.project.discover `objects` contains duplicate value `{text}`"),
                ));
            }
            Ok(artifact)
        })
        .collect()
}

fn validate_item_count(
    field: &str,
    count: usize,
    maximum: usize,
) -> Result<(), DiscoveryContractError> {
    if count > maximum {
        return Err(DiscoveryContractError::new(
            DiscoveryContractErrorCode::TooManyItems,
            format!("unica.project.discover `{field}` accepts at most {maximum} items"),
        ));
    }
    Ok(())
}

fn reject_duplicate(
    field: &str,
    text: &str,
    normalized: &mut BTreeSet<String>,
) -> Result<(), DiscoveryContractError> {
    let identity = text
        .chars()
        .flat_map(char::to_lowercase)
        .collect::<String>();
    if !normalized.insert(identity) {
        return Err(DiscoveryContractError::new(
            DiscoveryContractErrorCode::DuplicateValue,
            format!("unica.project.discover `{field}` contains duplicate value `{text}`"),
        ));
    }
    Ok(())
}

fn parse_limits(args: &Map<String, Value>) -> Result<DiscoveryLimits, DiscoveryContractError> {
    let Some(value) = args.get("limits") else {
        return Ok(DiscoveryLimits::default());
    };
    let limits = value
        .as_object()
        .ok_or_else(|| invalid_type("limits", "object", value))?;
    const ALLOWED: &[&str] = &[
        "maxFiles",
        "maxBytes",
        "maxEvidence",
        "maxCandidates",
        "maxGraphDepth",
    ];
    if let Some(field) = limits
        .keys()
        .find(|field| !ALLOWED.contains(&field.as_str()))
    {
        return Err(DiscoveryContractError::new(
            DiscoveryContractErrorCode::UnknownField,
            format!("unica.project.discover `limits` does not accept `{field}`"),
        ));
    }

    let defaults = DiscoveryLimits::default();
    Ok(DiscoveryLimits {
        max_files: MaxFiles(limit_u64(limits, "maxFiles", defaults.max_files.get().into())? as u32),
        max_bytes: MaxBytes(limit_u64(limits, "maxBytes", defaults.max_bytes.get())?),
        max_evidence: MaxEvidence(limit_u64(
            limits,
            "maxEvidence",
            defaults.max_evidence.get().into(),
        )? as u16),
        max_candidates: MaxCandidates(limit_u64(
            limits,
            "maxCandidates",
            defaults.max_candidates.get().into(),
        )? as u16),
        max_graph_depth: MaxGraphDepth(limit_u64(
            limits,
            "maxGraphDepth",
            defaults.max_graph_depth.get().into(),
        )? as u8),
    })
}

fn limit_u64(
    limits: &Map<String, Value>,
    field: &str,
    maximum: u64,
) -> Result<u64, DiscoveryContractError> {
    let Some(value) = limits.get(field) else {
        return Ok(maximum);
    };
    let Some(number) = value.as_u64() else {
        return Err(invalid_type(field, "positive integer", value));
    };
    if number == 0 || number > maximum {
        return Err(DiscoveryContractError::new(
            DiscoveryContractErrorCode::LimitOutOfRange,
            format!("unica.project.discover `limits.{field}` must be between 1 and {maximum}"),
        ));
    }
    Ok(number)
}

fn invalid_type(field: &str, expected: &str, value: &Value) -> DiscoveryContractError {
    DiscoveryContractError::new(
        DiscoveryContractErrorCode::InvalidType,
        format!(
            "unica.project.discover `{field}` must be {expected}, got {}",
            json_type(value)
        ),
    )
}

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

pub(crate) fn discover_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "concepts": {
                "type": "array",
                "maxItems": MAX_CONCEPTS,
                "uniqueItems": true,
                "items": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Runtime validation trims each concept, requires 1..=256 UTF-8 bytes, and enforces array uniqueness ignoring case. JSON Schema uniqueItems covers exact JSON values only."
                }
            },
            "cwd": {"type": "string"},
            "limits": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "maxFiles": {"type": "integer", "minimum": 1, "maximum": 20_000, "default": 20_000},
                    "maxBytes": {"type": "integer", "minimum": 1, "maximum": 268_435_456_u64, "default": 268_435_456_u64},
                    "maxEvidence": {"type": "integer", "minimum": 1, "maximum": 2_000, "default": 2_000},
                    "maxCandidates": {"type": "integer", "minimum": 1, "maximum": 100, "default": 100},
                    "maxGraphDepth": {"type": "integer", "minimum": 1, "maximum": 12, "default": 12}
                }
            },
            "mode": {"type": "string", "enum": ["explore"]},
            "objects": {
                "type": "array",
                "maxItems": MAX_OBJECTS,
                "uniqueItems": true,
                "items": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Runtime validation requires a canonical dot-separated artifact identifier with at least a kind and name, no empty dot segments or path separators, and 1..=1024 UTF-8 bytes; normalized identities must be unique ignoring case."
                }
            },
            "searchTerms": {
                "type": "array",
                "maxItems": MAX_SEARCH_TERMS,
                "uniqueItems": true,
                "items": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Runtime validation trims each search term, requires 1..=256 UTF-8 bytes, and enforces array uniqueness ignoring case. JSON Schema uniqueItems covers exact JSON values only."
                }
            },
            "sourceDir": {
                "type": "string",
                "minLength": 1,
                "description": "Runtime validation accepts a non-empty contained portable relative path using / or \\ separators, normalizes it to /, and rejects absolute, prefixed, escaping, or ambiguous forms."
            },
            "task": {
                "type": "string",
                "minLength": 1,
                "description": "Runtime validation trims the task and requires 1..=8192 UTF-8 bytes. JSON Schema maxLength is omitted because it counts characters."
            }
        },
        "required": ["mode", "task"]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Map, Value};

    fn parse(value: Value) -> Result<DiscoverRequest, DiscoveryContractError> {
        match value {
            Value::Object(args) => parse_discover_request(&args),
            other => panic!("test request must be an object, got {other:?}"),
        }
    }

    fn request_with_task(task: String) -> Value {
        json!({"mode": "explore", "task": task})
    }

    fn valid_request() -> Value {
        json!({"mode": "explore", "task": "Find extension points"})
    }

    #[test]
    fn task_only_request_derives_a_typed_explore_request() {
        let request = parse(json!({
            "cwd": "/workspace",
            "mode": "explore",
            "task": "При поступлении контролировать срок годности серий"
        }))
        .expect("task-only request");

        assert_eq!(request.cwd(), Some(std::path::Path::new("/workspace")));
        assert_eq!(request.mode(), DiscoveryMode::Explore);
        assert_eq!(
            request.task(),
            "При поступлении контролировать срок годности серий"
        );
        assert_eq!(request.limits().max_files().get(), 20_000);
        assert_eq!(request.limits().max_bytes().get(), 268_435_456);
        assert_eq!(request.limits().max_evidence().get(), 2_000);
        assert_eq!(request.limits().max_candidates().get(), 100);
        assert_eq!(request.limits().max_graph_depth().get(), 12);
        assert!(request.concepts().is_empty());
        assert!(request.search_terms().is_empty());
        assert!(request.objects().is_empty());
        assert_eq!(request.source_dir(), None);
    }

    #[test]
    fn cyrillic_task_limit_is_measured_in_utf8_bytes() {
        assert!(parse(request_with_task("я".repeat(4_096))).is_ok());
        let error = parse(request_with_task("я".repeat(4_097))).unwrap_err();
        assert_eq!(
            error.code(),
            DiscoveryContractErrorCode::TextBytesOutOfRange
        );
    }

    #[test]
    fn request_rejects_invalid_top_level_fields_and_mode() {
        let cases = [
            (
                json!({"task": "x"}),
                DiscoveryContractErrorCode::MissingField,
            ),
            (
                json!({"mode": "inspect", "task": "x"}),
                DiscoveryContractErrorCode::InvalidMode,
            ),
            (
                json!({"mode": 1, "task": "x"}),
                DiscoveryContractErrorCode::InvalidType,
            ),
            (
                json!({"mode": "explore"}),
                DiscoveryContractErrorCode::MissingField,
            ),
            (
                json!({"mode": "explore", "task": ""}),
                DiscoveryContractErrorCode::TextBytesOutOfRange,
            ),
            (
                json!({"mode": "explore", "task": "x", "dryRun": true}),
                DiscoveryContractErrorCode::UnknownField,
            ),
            (
                json!({"mode": "explore", "task": "x", "confirm": true}),
                DiscoveryContractErrorCode::UnknownField,
            ),
            (
                json!({"mode": "explore", "task": "x", "sourceSet": "main"}),
                DiscoveryContractErrorCode::UnknownField,
            ),
            (
                json!({"mode": "explore", "task": "x", "proposedExtensionPoints": []}),
                DiscoveryContractErrorCode::UnknownField,
            ),
            (
                json!({"mode": "explore", "task": "x", "receipt": "r"}),
                DiscoveryContractErrorCode::UnknownField,
            ),
            (
                json!({"mode": "explore", "task": "x", "unknown": 1}),
                DiscoveryContractErrorCode::UnknownField,
            ),
        ];

        for (payload, expected) in cases {
            let error = parse(payload).unwrap_err();
            assert_eq!(error.code(), expected, "{error}");
        }
    }

    #[test]
    fn arrays_are_trimmed_bounded_and_unique_ignoring_case() {
        let request = parse(json!({
            "mode": "explore",
            "task": "x",
            "concepts": ["  Series  "],
            "searchTerms": [" FindMe "],
            "objects": [" Document.Order "]
        }))
        .expect("normalized arrays");
        assert_eq!(request.concepts(), &["Series"]);
        assert_eq!(request.search_terms(), &["FindMe"]);
        assert_eq!(request.objects()[0].as_str(), "document.order");

        for field in ["concepts", "searchTerms", "objects"] {
            let mut payload = valid_request();
            payload[field] = if field == "objects" {
                json!(["Kind.Duplicate", " kind.duplicate "])
            } else {
                json!(["Duplicate", " duplicate "])
            };
            let error = parse(payload).unwrap_err();
            assert_eq!(
                error.code(),
                DiscoveryContractErrorCode::DuplicateValue,
                "{field}: {error}"
            );
        }

        for (field, count) in [("concepts", 65), ("searchTerms", 129), ("objects", 129)] {
            let values = (0..count)
                .map(|index| format!("Kind.Value{index}"))
                .collect::<Vec<_>>();
            let mut payload = valid_request();
            payload[field] = json!(values);
            let error = parse(payload).unwrap_err();
            assert_eq!(
                error.code(),
                DiscoveryContractErrorCode::TooManyItems,
                "{field}: {error}"
            );
        }
    }

    #[test]
    fn array_text_limits_are_measured_in_utf8_bytes() {
        for field in ["concepts", "searchTerms"] {
            let mut accepted = valid_request();
            accepted[field] = json!(["я".repeat(128)]);
            assert!(parse(accepted).is_ok(), "{field}");

            let mut rejected = valid_request();
            rejected[field] = json!(["я".repeat(129)]);
            let error = parse(rejected).unwrap_err();
            assert_eq!(
                error.code(),
                DiscoveryContractErrorCode::TextBytesOutOfRange
            );
        }

        let mut accepted = valid_request();
        accepted["objects"] = json!([format!("Kind.{}", "я".repeat(509))]);
        assert!(parse(accepted).is_ok());
        let mut rejected = valid_request();
        rejected["objects"] = json!([format!("Kind.{}", "я".repeat(510))]);
        let error = parse(rejected).unwrap_err();
        assert_eq!(
            error.code(),
            DiscoveryContractErrorCode::TextBytesOutOfRange
        );
    }

    #[test]
    fn malformed_artifact_ids_are_rejected() {
        for object in [
            "Document",
            ".Document.Order",
            "Document.Order.",
            "Document..Order",
            "Document/Order",
            "Document\\Order",
        ] {
            let mut payload = valid_request();
            payload["objects"] = json!([object]);
            let error = parse(payload).unwrap_err();
            assert_eq!(
                error.code(),
                DiscoveryContractErrorCode::InvalidArtifactId,
                "{object}: {error}"
            );
        }
    }

    #[test]
    fn source_dir_normalizes_portable_relative_paths() {
        for (source_dir, expected) in [
            ("src/./configuration", "src/configuration"),
            ("src\\configuration", "src/configuration"),
            ("./src\\nested/./configuration", "src/nested/configuration"),
            (".", "."),
        ] {
            let mut payload = valid_request();
            payload["sourceDir"] = json!(source_dir);
            let request = parse(payload).expect("portable contained source dir");
            assert_eq!(
                request.source_dir(),
                Some(std::path::Path::new(expected)),
                "{source_dir}"
            );
        }
    }

    #[test]
    fn source_dir_rejects_portable_absolute_escaping_and_ambiguous_paths() {
        for source_dir in [
            "",
            "/absolute",
            "\\rooted",
            "//server/share",
            "\\\\server\\share",
            "C:\\absolute",
            "C:/absolute",
            "C:drive-relative",
            "\\\\?\\C:\\device",
            "\\\\.\\device",
            "../escape",
            "src/../escape",
            "src\\..\\escape",
            "src//ambiguous",
            "src\\\\ambiguous",
            "src/",
            "src:",
        ] {
            let mut payload = valid_request();
            payload["sourceDir"] = json!(source_dir);
            let error = parse(payload).unwrap_err();
            assert_eq!(
                error.code(),
                DiscoveryContractErrorCode::InvalidSourceDir,
                "{source_dir}: {error}"
            );
        }
    }

    #[test]
    fn limits_accept_only_positive_values_at_or_below_package_maxima() {
        let maxima = [
            ("maxFiles", 20_000_u64),
            ("maxBytes", 268_435_456),
            ("maxEvidence", 2_000),
            ("maxCandidates", 100),
            ("maxGraphDepth", 12),
        ];
        for (field, maximum) in maxima {
            let mut accepted = valid_request();
            accepted["limits"] = json!({field: maximum});
            assert!(parse(accepted).is_ok(), "{field} at maximum");

            for invalid in [0, maximum + 1] {
                let mut rejected = valid_request();
                rejected["limits"] = json!({field: invalid});
                let error = parse(rejected).unwrap_err();
                assert_eq!(
                    error.code(),
                    DiscoveryContractErrorCode::LimitOutOfRange,
                    "{field}={invalid}: {error}"
                );
            }
        }

        for limits in [json!({"unknown": 1}), json!([]), json!({"maxFiles": 1.5})] {
            let mut payload = valid_request();
            payload["limits"] = limits;
            assert!(parse(payload).is_err());
        }
    }

    #[test]
    fn schema_describes_the_exact_strict_request_contract() {
        let schema = discover_input_schema();
        let properties = schema["properties"].as_object().expect("properties object");
        assert_eq!(
            properties
                .keys()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>(),
            discover_allowed_args().iter().copied().collect()
        );
        assert_eq!(properties.len(), 8);
        assert_eq!(schema["required"], json!(["mode", "task"]));
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(properties["mode"]["enum"], json!(["explore"]));
        assert_eq!(properties["concepts"]["maxItems"], 64);
        assert_eq!(properties["searchTerms"]["maxItems"], 128);
        assert_eq!(properties["objects"]["maxItems"], 128);
        for field in ["concepts", "searchTerms", "objects"] {
            assert_eq!(properties[field]["uniqueItems"], true, "{field}");
        }
        for field in ["task", "concepts", "searchTerms", "objects"] {
            let text_schema = if field == "task" {
                &properties[field]
            } else {
                &properties[field]["items"]
            };
            assert!(text_schema.get("maxLength").is_none(), "{field}");
            assert_eq!(text_schema["minLength"], 1, "{field}");
            assert!(text_schema["description"]
                .as_str()
                .is_some_and(|description| description.contains("UTF-8 bytes")));
        }

        let limits = &properties["limits"];
        assert_eq!(limits["additionalProperties"], false);
        assert_eq!(limits["properties"]["maxFiles"]["maximum"], 20_000);
        assert_eq!(limits["properties"]["maxBytes"]["maximum"], 268_435_456_u64);
        assert_eq!(limits["properties"]["maxEvidence"]["maximum"], 2_000);
        assert_eq!(limits["properties"]["maxCandidates"]["maximum"], 100);
        assert_eq!(limits["properties"]["maxGraphDepth"]["maximum"], 12);
    }

    #[test]
    fn local_schema_evaluator_enforces_expressible_keywords() {
        let schema = json!({
            "type": "array",
            "minItems": 1,
            "maxItems": 2,
            "uniqueItems": true,
            "items": {
                "type": "string",
                "minLength": 2,
                "maxLength": 3,
                "enum": ["aa", "bbb"]
            }
        });

        assert!(schema_structurally_accepts(&schema, &json!(["aa", "bbb"])));
        for rejected in [
            json!([]),
            json!(["aa", "bbb", "aa"]),
            json!(["aa", "aa"]),
            json!(["a"]),
            json!(["bbbb"]),
            json!(["cc"]),
            json!([1]),
        ] {
            assert!(
                !schema_structurally_accepts(&schema, &rejected),
                "{rejected}"
            );
        }
    }

    #[test]
    fn expressible_schema_structure_and_runtime_parser_agree() {
        let schema = discover_input_schema();
        let cases = [
            valid_request(),
            json!({"mode": "explore", "task": "x", "sourceDir": "src", "concepts": ["a"]}),
            json!({"mode": "wrong", "task": "x"}),
            json!({"mode": "explore"}),
            json!({"mode": "explore", "task": ""}),
            json!({"mode": "explore", "task": "x", "extra": true}),
            json!({"mode": "explore", "task": "x", "concepts": "a"}),
            json!({"mode": "explore", "task": "x", "concepts": [""]}),
            json!({"mode": "explore", "task": "x", "concepts": ["a", "a"]}),
            json!({"mode": "explore", "task": "x", "concepts": vec!["a"; 65]}),
            json!({"mode": "explore", "task": "x", "limits": {"maxFiles": 20_001}}),
            json!({"mode": "explore", "task": "x", "limits": {"unknown": 1}}),
        ];
        for payload in cases {
            let schema_accepts = schema_structurally_accepts(&schema, &payload);
            let runtime_accepts = parse(payload.clone()).is_ok();
            assert_eq!(schema_accepts, runtime_accepts, "payload: {payload}");
        }
    }

    #[test]
    fn runtime_only_semantics_are_documented_and_runtime_authoritative() {
        let schema = discover_input_schema();
        let properties = schema["properties"].as_object().expect("properties object");
        for (field, required_words) in [
            ("task", &["Runtime validation", "UTF-8 bytes"][..]),
            ("concepts", &["Runtime validation", "ignoring case"]),
            ("searchTerms", &["Runtime validation", "ignoring case"]),
            (
                "objects",
                &[
                    "Runtime validation",
                    "kind and name",
                    "empty dot segments",
                    "path separators",
                ],
            ),
            ("sourceDir", &["Runtime validation", "contained"]),
        ] {
            let description = if matches!(field, "concepts" | "searchTerms" | "objects") {
                properties[field]["items"]["description"].as_str()
            } else {
                properties[field]["description"].as_str()
            }
            .expect("runtime-authoritative description");
            for word in required_words {
                assert!(description.contains(word), "{field}: {description}");
            }
        }

        for payload in [
            json!({"mode": "explore", "task": "x", "concepts": ["Series", " series "]}),
            json!({"mode": "explore", "task": "x", "searchTerms": ["Find", "find"]}),
            json!({"mode": "explore", "task": "x", "objects": ["Document"]}),
            json!({"mode": "explore", "task": "x", "objects": ["Document.Order", "document.order"]}),
            json!({"mode": "explore", "task": "x", "sourceDir": "../escape"}),
            request_with_task("я".repeat(4_097)),
        ] {
            assert!(
                schema_structurally_accepts(&schema, &payload),
                "standard JSON Schema remains structural for {payload}"
            );
            assert!(parse(payload.clone()).is_err(), "runtime rejects {payload}");
        }
    }

    fn schema_structurally_accepts(schema: &Value, value: &Value) -> bool {
        match schema["type"].as_str() {
            Some("object") => {
                let Some(object) = value.as_object() else {
                    return false;
                };
                let Some(properties) = schema["properties"].as_object() else {
                    return false;
                };
                if schema["additionalProperties"] == false
                    && object.keys().any(|key| !properties.contains_key(key))
                {
                    return false;
                }
                if schema["required"].as_array().is_some_and(|required| {
                    required
                        .iter()
                        .filter_map(Value::as_str)
                        .any(|key| !object.contains_key(key))
                }) {
                    return false;
                }
                object.iter().all(|(key, nested)| {
                    properties.get(key).is_some_and(|nested_schema| {
                        schema_structurally_accepts(nested_schema, nested)
                    })
                })
            }
            Some("array") => value.as_array().is_some_and(|array| {
                schema["minItems"]
                    .as_u64()
                    .is_none_or(|minimum| array.len() as u64 >= minimum)
                    && schema["maxItems"]
                        .as_u64()
                        .is_none_or(|maximum| array.len() as u64 <= maximum)
                    && array
                        .iter()
                        .all(|item| schema_structurally_accepts(&schema["items"], item))
                    && (schema["uniqueItems"] != true
                        || array
                            .iter()
                            .enumerate()
                            .all(|(index, item)| !array[..index].contains(item)))
            }),
            Some("string") => value.as_str().is_some_and(|text| {
                schema["minLength"]
                    .as_u64()
                    .is_none_or(|minimum| text.chars().count() as u64 >= minimum)
                    && schema["maxLength"]
                        .as_u64()
                        .is_none_or(|maximum| text.chars().count() as u64 <= maximum)
                    && schema["enum"]
                        .as_array()
                        .is_none_or(|values| values.iter().any(|value| value == text))
            }),
            Some("integer") => value.as_u64().is_some_and(|number| {
                schema["minimum"]
                    .as_u64()
                    .is_none_or(|minimum| number >= minimum)
                    && schema["maximum"]
                        .as_u64()
                        .is_none_or(|maximum| number <= maximum)
            }),
            _ => false,
        }
    }

    #[test]
    fn allowed_arguments_are_stable_and_do_not_include_common_mutation_fields() {
        assert_eq!(
            discover_allowed_args(),
            &[
                "concepts",
                "cwd",
                "limits",
                "mode",
                "objects",
                "searchTerms",
                "sourceDir",
                "task"
            ]
        );
        assert!(!discover_allowed_args().contains(&"dryRun"));
        assert!(!discover_allowed_args().contains(&"confirm"));
    }

    #[allow(dead_code)]
    fn _map_type_is_part_of_the_boundary(_: &Map<String, Value>) {}
}
