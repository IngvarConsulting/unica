use super::model::{
    AnonymousObject, NamedType, PackageModel, Property, QNameRef, SourceSpan, TopLevelKind,
    TypeKind, XML_SCHEMA_NS,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum FindingSeverity {
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct FindingLocation {
    pub(super) key: String,
    pub(super) span: SourceSpanDto,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct SourceSpanDto {
    pub(super) start: usize,
    pub(super) end: usize,
}

impl From<&SourceSpan> for SourceSpanDto {
    fn from(span: &SourceSpan) -> Self {
        Self {
            start: span.start,
            end: span.end,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct Finding {
    pub(super) code: String,
    pub(super) severity: FindingSeverity,
    pub(super) message: String,
    pub(super) location: FindingLocation,
}

impl Finding {
    fn error(code: &str, key: String, span: &SourceSpan, message: String) -> Self {
        Self {
            code: code.to_string(),
            severity: FindingSeverity::Error,
            message,
            location: FindingLocation {
                key,
                span: span.into(),
            },
        }
    }

    fn semantic_key(&self) -> (&str, &str) {
        (&self.code, &self.location.key)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum FindingState {
    PreExisting,
    Introduced,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct ClassifiedFinding {
    pub(super) code: String,
    pub(super) severity: FindingSeverity,
    pub(super) state: FindingState,
    pub(super) message: String,
    pub(super) location: FindingLocation,
}

#[derive(Clone, Debug)]
pub(super) struct ValidationDiff {
    pub(super) findings: Vec<ClassifiedFinding>,
}

impl ValidationDiff {
    pub(super) fn between(before: &[Finding], after: Vec<Finding>) -> Self {
        let mut baseline = HashMap::new();
        for finding in before {
            let (code, key) = finding.semantic_key();
            *baseline
                .entry((code.to_string(), key.to_string()))
                .or_insert(0_usize) += 1;
        }
        let findings = after
            .into_iter()
            .map(|finding| {
                let remaining = baseline
                    .entry((finding.code.clone(), finding.location.key.clone()))
                    .or_insert(0);
                let state = if *remaining > 0 {
                    *remaining -= 1;
                    FindingState::PreExisting
                } else {
                    FindingState::Introduced
                };
                ClassifiedFinding {
                    code: finding.code,
                    severity: finding.severity,
                    state,
                    message: finding.message,
                    location: finding.location,
                }
            })
            .collect();
        Self { findings }
    }

    pub(super) fn blocks(&self) -> bool {
        self.findings.iter().any(|finding| {
            finding.state == FindingState::Introduced && finding.severity == FindingSeverity::Error
        })
    }
}

pub(super) fn validate(model: &PackageModel) -> Vec<Finding> {
    let mut findings = Vec::new();
    validate_target_namespace(model, &mut findings);
    validate_group_order(model, &mut findings);
    for unsupported in &model.unsupported {
        findings.push(Finding::error(
            "unsupported_node",
            unsupported.key.clone(),
            &unsupported.span,
            unsupported.description.clone(),
        ));
    }

    let target_namespace = model.target_namespace().unwrap_or("");
    let imported_namespaces = model
        .imports
        .iter()
        .filter_map(|import| import.namespace.as_ref())
        .filter(|namespace| !namespace.value.is_empty())
        .map(|namespace| namespace.value.as_str())
        .collect::<HashSet<_>>();
    for import in &model.imports {
        if import
            .namespace
            .as_ref()
            .is_none_or(|namespace| namespace.value.is_empty())
        {
            findings.push(Finding::error(
                "missing_import",
                format!("{}/@namespace", import.key),
                &import.span,
                "import must declare a non-empty namespace URI".to_string(),
            ));
        }
    }

    let local_types = model
        .types
        .iter()
        .filter_map(|named| named.name.as_ref())
        .map(|name| name.value.as_str())
        .collect::<HashSet<_>>();
    let global_properties = model
        .global_properties
        .iter()
        .filter_map(|property| property.name.as_ref())
        .map(|name| name.value.as_str())
        .collect::<HashSet<_>>();

    validate_type_uniqueness(model, &mut findings);
    for property in &model.global_properties {
        validate_property(
            property,
            target_namespace,
            &imported_namespaces,
            &local_types,
            &global_properties,
            &mut findings,
        );
    }
    for named in &model.types {
        validate_named_type(
            named,
            target_namespace,
            &imported_namespaces,
            &local_types,
            &global_properties,
            &mut findings,
        );
    }
    findings
}

fn validate_target_namespace(model: &PackageModel, findings: &mut Vec<Finding>) {
    if model
        .target_namespace
        .as_ref()
        .is_none_or(|namespace| namespace.value.is_empty())
    {
        findings.push(Finding::error(
            "target_namespace_required",
            "$package/@targetNamespace".to_string(),
            &model.root_span,
            "package must declare a non-empty targetNamespace".to_string(),
        ));
    }
}

fn validate_group_order(model: &PackageModel, findings: &mut Vec<Finding>) {
    let mut greatest_rank = 0;
    for node in &model.top_level {
        let rank = match node.kind {
            TopLevelKind::Import => 1,
            TopLevelKind::Property => 2,
            TopLevelKind::ValueType => 3,
            TopLevelKind::ObjectType => 4,
            TopLevelKind::Unsupported => continue,
        };
        if rank < greatest_rank {
            findings.push(Finding::error(
                "invalid_group_order",
                format!("{}/@group-order", node.key),
                &node.span,
                "package children must be grouped as import, property, valueType, objectType"
                    .to_string(),
            ));
        } else {
            greatest_rank = rank;
        }
    }
}

fn validate_type_uniqueness(model: &PackageModel, findings: &mut Vec<Finding>) {
    let mut seen = HashMap::new();
    for named in &model.types {
        let Some(name) = &named.name else { continue };
        if seen.insert(name.value.as_str(), named.kind).is_some() {
            findings.push(Finding::error(
                "duplicate_type",
                format!("$package/type:{}/@unique", name.value),
                &name.span,
                format!(
                    "valueType and objectType names must be jointly unique; `{}` is repeated",
                    name.value
                ),
            ));
        }
    }
}

fn validate_named_type(
    named: &NamedType,
    target_namespace: &str,
    imports: &HashSet<&str>,
    local_types: &HashSet<&str>,
    global_properties: &HashSet<&str>,
    findings: &mut Vec<Finding>,
) {
    match &named.name {
        Some(name) if !is_ncname(&name.value) => findings.push(Finding::error(
            "invalid_ncname",
            format!("{}/@name", named.key),
            &name.span,
            format!("`{}` is not an NCName", name.value),
        )),
        None => findings.push(Finding::error(
            "invalid_ncname",
            format!("{}/@name", named.key),
            &named.span,
            format!("{} must declare name", named.kind.element_name()),
        )),
        _ => {}
    }
    if let Some(base) = &named.base {
        validate_qname(
            base,
            ReferenceKind::Type,
            &format!("{}/@base", named.key),
            target_namespace,
            imports,
            local_types,
            global_properties,
            findings,
        );
    }
    if named.kind == TypeKind::Object {
        validate_property_list(
            &named.key,
            &named.properties,
            target_namespace,
            imports,
            local_types,
            global_properties,
            findings,
        );
    }
}

fn validate_property_list(
    owner_key: &str,
    properties: &[Property],
    target_namespace: &str,
    imports: &HashSet<&str>,
    local_types: &HashSet<&str>,
    global_properties: &HashSet<&str>,
    findings: &mut Vec<Finding>,
) {
    let mut identities = HashSet::new();
    for property in properties {
        let identity = property.logical_identity();
        if !identities.insert(identity.clone()) {
            findings.push(Finding::error(
                "duplicate_property",
                format!("{owner_key}/property:{identity}/@unique"),
                &property.span,
                format!("property identity `{identity}` is repeated in one object type"),
            ));
        }
        validate_property(
            property,
            target_namespace,
            imports,
            local_types,
            global_properties,
            findings,
        );
    }
}

fn validate_property(
    property: &Property,
    target_namespace: &str,
    imports: &HashSet<&str>,
    local_types: &HashSet<&str>,
    global_properties: &HashSet<&str>,
    findings: &mut Vec<Finding>,
) {
    let has_name = property.name.is_some();
    let has_ref = property.reference.is_some();
    if has_name == has_ref {
        findings.push(Finding::error(
            "invalid_property_identity",
            format!("{}/@identity", property.key),
            &property.span,
            "property must declare exactly one of name or ref".to_string(),
        ));
    }
    if let Some(name) = &property.name {
        if !is_ncname(&name.value) {
            findings.push(Finding::error(
                "invalid_ncname",
                format!("{}/@name", property.key),
                &name.span,
                format!("`{}` is not an NCName", name.value),
            ));
        }
    }
    if let Some(reference) = &property.reference {
        validate_qname(
            reference,
            ReferenceKind::Property,
            &format!("{}/@ref", property.key),
            target_namespace,
            imports,
            local_types,
            global_properties,
            findings,
        );
    }
    if let Some(type_ref) = &property.type_ref {
        validate_qname(
            type_ref,
            ReferenceKind::Type,
            &format!("{}/@type", property.key),
            target_namespace,
            imports,
            local_types,
            global_properties,
            findings,
        );
    }

    let owned_type_count = usize::from(property.type_ref.is_some()) + property.type_defs.len();
    if owned_type_count > 1 || (has_ref && owned_type_count > 0) {
        findings.push(Finding::error(
            "type_definition_conflict",
            format!("{}/@owned-type", property.key),
            &property.span,
            "property type, typeDef:ObjectType, and ref are mutually exclusive".to_string(),
        ));
    }
    if property.type_defs.len() > 1 {
        findings.push(Finding::error(
            "unsupported_node",
            format!("{}/typeDef/@cardinality", property.key),
            &property.span,
            "property supports at most one typeDef:ObjectType".to_string(),
        ));
    }
    for type_def in &property.type_defs {
        validate_type_def(
            type_def,
            target_namespace,
            imports,
            local_types,
            global_properties,
            findings,
        );
    }
    validate_bounds(property, findings);
}

fn validate_type_def(
    type_def: &AnonymousObject,
    target_namespace: &str,
    imports: &HashSet<&str>,
    local_types: &HashSet<&str>,
    global_properties: &HashSet<&str>,
    findings: &mut Vec<Finding>,
) {
    if type_def
        .discriminator
        .as_ref()
        .is_none_or(|discriminator| discriminator.value != "ObjectType")
    {
        findings.push(Finding::error(
            "unsupported_node",
            format!("{}/@xsi:type", type_def.key),
            type_def
                .discriminator
                .as_ref()
                .map_or(&type_def.span, |discriminator| &discriminator.span),
            "typeDef must have the exact discriminator xsi:type=\"ObjectType\"".to_string(),
        ));
    }
    validate_property_list(
        &type_def.key,
        &type_def.properties,
        target_namespace,
        imports,
        local_types,
        global_properties,
        findings,
    );
}

#[derive(Clone, Copy)]
enum ReferenceKind {
    Type,
    Property,
}

fn validate_qname(
    qname: &QNameRef,
    kind: ReferenceKind,
    key: &str,
    target_namespace: &str,
    imports: &HashSet<&str>,
    local_types: &HashSet<&str>,
    global_properties: &HashSet<&str>,
    findings: &mut Vec<Finding>,
) {
    if !qname.lexical_valid
        || qname
            .prefix
            .as_deref()
            .is_none_or(|prefix| !is_ncname(prefix))
        || !is_ncname(&qname.local)
    {
        findings.push(Finding::error(
            "invalid_qname",
            key.to_string(),
            &qname.span,
            format!("`{}` is not a supported prefixed QName", qname.raw),
        ));
        return;
    }
    let Some(namespace) = qname.namespace.as_deref() else {
        findings.push(Finding::error(
            "undeclared_prefix",
            key.to_string(),
            &qname.span,
            format!(
                "QName prefix in `{}` is not declared in node scope",
                qname.raw
            ),
        ));
        return;
    };

    if namespace == target_namespace {
        let exists = match kind {
            ReferenceKind::Type => local_types.contains(qname.local.as_str()),
            ReferenceKind::Property => global_properties.contains(qname.local.as_str()),
        };
        if !exists {
            let (code, noun) = match kind {
                ReferenceKind::Type => ("unknown_type_reference", "type"),
                ReferenceKind::Property => ("unknown_property_reference", "global property"),
            };
            findings.push(Finding::error(
                code,
                key.to_string(),
                &qname.span,
                format!("local {noun} `{}` does not exist", qname.local),
            ));
        }
        return;
    }

    if matches!(kind, ReferenceKind::Type) && namespace == XML_SCHEMA_NS {
        return;
    }
    if !imports.contains(namespace) {
        findings.push(Finding::error(
            "missing_import",
            key.to_string(),
            &qname.span,
            format!("QName namespace `{namespace}` is not declared by package/import"),
        ));
    }
}

fn validate_bounds(property: &Property, findings: &mut Vec<Finding>) {
    let lower = property
        .lower_bound
        .as_ref()
        .map(|value| {
            value
                .value
                .parse::<i64>()
                .ok()
                .filter(|parsed| matches!(parsed, 0 | 1))
        })
        .unwrap_or(Some(1));
    let upper = property
        .upper_bound
        .as_ref()
        .map(|value| {
            value
                .value
                .parse::<i64>()
                .ok()
                .filter(|parsed| *parsed == -1 || *parsed >= 1)
        })
        .unwrap_or(Some(1));
    let valid = match (lower, upper) {
        (Some(lower), Some(-1)) => lower >= 0,
        (Some(lower), Some(upper)) => lower <= upper,
        _ => false,
    };
    if !valid {
        let span = property
            .lower_bound
            .as_ref()
            .map(|value| &value.span)
            .or_else(|| property.upper_bound.as_ref().map(|value| &value.span))
            .unwrap_or(&property.span);
        findings.push(Finding::error(
            "invalid_bounds",
            format!("{}/@bounds", property.key),
            span,
            "effective bounds require lowerBound 0 or 1, upperBound -1 or >=1, and lower <= finite upper"
                .to_string(),
        ));
    }
}

fn is_ncname(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !is_ncname_start(first) {
        return false;
    }
    characters.all(is_ncname_char)
}

fn is_ncname_start(character: char) -> bool {
    matches!(
        character,
        'A'..='Z'
            | '_'
            | 'a'..='z'
            | '\u{00c0}'..='\u{00d6}'
            | '\u{00d8}'..='\u{00f6}'
            | '\u{00f8}'..='\u{02ff}'
            | '\u{0370}'..='\u{037d}'
            | '\u{037f}'..='\u{1fff}'
            | '\u{200c}'..='\u{200d}'
            | '\u{2070}'..='\u{218f}'
            | '\u{2c00}'..='\u{2fef}'
            | '\u{3001}'..='\u{d7ff}'
            | '\u{f900}'..='\u{fdcf}'
            | '\u{fdf0}'..='\u{fffd}'
            | '\u{10000}'..='\u{effff}'
    )
}

fn is_ncname_char(character: char) -> bool {
    is_ncname_start(character)
        || matches!(
            character,
            '-' | '.' | '0'..='9' | '\u{00b7}' | '\u{0300}'..='\u{036f}' | '\u{203f}'..='\u{2040}'
        )
}
