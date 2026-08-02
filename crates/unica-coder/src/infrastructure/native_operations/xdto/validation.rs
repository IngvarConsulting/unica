use super::model::{
    AnonymousObject, NamedType, PackageModel, Property, QNameRef, SourceSpan, TopLevelKind,
    TypeKind, XML_SCHEMA_NS,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};

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
        let mut baseline = HashMap::<(String, String), VecDeque<&Finding>>::new();
        for finding in before {
            let (code, key) = finding.semantic_key();
            baseline
                .entry((code.to_string(), key.to_string()))
                .or_default()
                .push_back(finding);
        }
        let findings = after
            .into_iter()
            .map(|finding| {
                let pre_existing = baseline
                    .entry((finding.code.clone(), finding.location.key.clone()))
                    .or_default()
                    .pop_front();
                if let Some(pre_existing) = pre_existing {
                    ClassifiedFinding {
                        code: pre_existing.code.clone(),
                        severity: pre_existing.severity,
                        state: FindingState::PreExisting,
                        message: pre_existing.message.clone(),
                        location: pre_existing.location.clone(),
                    }
                } else {
                    ClassifiedFinding {
                        code: finding.code,
                        severity: finding.severity,
                        state: FindingState::Introduced,
                        message: finding.message,
                        location: finding.location,
                    }
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
            ReferenceSite {
                kind: ReferenceKind::Type,
                key: &format!("{}/@base", named.key),
            },
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
            ReferenceSite {
                kind: ReferenceKind::Property,
                key: &format!("{}/@ref", property.key),
            },
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
            ReferenceSite {
                kind: ReferenceKind::Type,
                key: &format!("{}/@type", property.key),
            },
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

struct ReferenceSite<'a> {
    kind: ReferenceKind,
    key: &'a str,
}

fn validate_qname(
    qname: &QNameRef,
    site: ReferenceSite<'_>,
    target_namespace: &str,
    imports: &HashSet<&str>,
    local_types: &HashSet<&str>,
    global_properties: &HashSet<&str>,
    findings: &mut Vec<Finding>,
) {
    let ReferenceSite { kind, key } = site;
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
        .map_or("1", |value| value.value.as_str());
    let upper = property
        .upper_bound
        .as_ref()
        .map_or("1", |value| value.value.as_str());
    let lower_is_valid = matches!(lower, "0" | "1");
    let upper_is_unbounded = upper == "-1";
    let upper_is_finite = is_canonical_positive_decimal(upper);
    let valid = lower_is_valid
        && (upper_is_unbounded || (upper_is_finite && decimal_less_than_or_equal(lower, upper)));
    if !valid {
        let span = if !lower_is_valid {
            property
                .lower_bound
                .as_ref()
                .map_or(&property.span, |value| &value.span)
        } else {
            property
                .upper_bound
                .as_ref()
                .map_or(&property.span, |value| &value.span)
        };
        findings.push(Finding::error(
            "invalid_bounds",
            format!("{}/@bounds", property.key),
            span,
            "effective bounds require lowerBound 0 or 1, upperBound -1 or >=1, and lower <= finite upper"
                .to_string(),
        ));
    }
}

fn is_canonical_positive_decimal(value: &str) -> bool {
    value
        .as_bytes()
        .first()
        .is_some_and(|first| matches!(first, b'1'..=b'9'))
        && value.as_bytes()[1..]
            .iter()
            .all(|byte| byte.is_ascii_digit())
}

fn decimal_less_than_or_equal(left: &str, right: &str) -> bool {
    left.len() < right.len() || (left.len() == right.len() && left <= right)
}

pub(super) fn is_ncname(value: &str) -> bool {
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

#[cfg(test)]
mod tests {
    use super::{validate, Finding};
    use crate::infrastructure::native_operations::xdto::model::PackageModel;

    const ROOT: &str = r#"xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:tns="urn:test" targetNamespace="urn:test""#;

    fn package(body: &str) -> String {
        format!("<package {ROOT}>{body}</package>")
    }

    fn findings(xml: &str) -> Vec<Finding> {
        validate(&PackageModel::parse(xml).unwrap())
    }

    fn codes(xml: &str) -> Vec<String> {
        findings(xml)
            .into_iter()
            .map(|finding| finding.code)
            .collect()
    }

    #[test]
    fn xdto_validation_rejects_noncanonical_bound_literals() {
        for (label, attributes) in [
            ("lower leading zero", r#"lowerBound="00""#),
            ("lower noncanonical one", r#"lowerBound="01""#),
            ("lower plus", r#"lowerBound="+1""#),
            ("lower negative zero", r#"lowerBound="-01""#),
            ("upper zero", r#"upperBound="0""#),
            ("upper leading zero", r#"upperBound="00""#),
            ("upper noncanonical one", r#"upperBound="01""#),
            ("upper plus", r#"upperBound="+1""#),
            ("upper negative sentinel", r#"upperBound="-01""#),
            ("upper negative finite", r#"upperBound="-2""#),
        ] {
            let xml = package(&format!(
                r#"<objectType name="T"><property name="P" {attributes}/></objectType>"#
            ));
            assert_eq!(codes(&xml), vec!["invalid_bounds"], "{label}");
        }
    }

    #[test]
    fn xdto_validation_accepts_unbounded_defaults_and_arbitrary_size_finite_upper() {
        for (label, attributes) in [
            ("defaults", ""),
            ("minimum finite", r#"lowerBound="1" upperBound="1""#),
            ("unbounded", r#"lowerBound="0" upperBound="-1""#),
            (
                "above i64 max",
                r#"lowerBound="1" upperBound="9223372036854775808""#,
            ),
            (
                "arbitrary precision",
                r#"lowerBound="0" upperBound="999999999999999999999999999999999999999999""#,
            ),
        ] {
            let xml = package(&format!(
                r#"<objectType name="T"><property name="P" {attributes}/></objectType>"#
            ));
            assert!(findings(&xml).is_empty(), "{label}: {:#?}", findings(&xml));
        }
    }

    #[test]
    fn xdto_validation_ignores_only_whitespace_character_data_in_supported_containers() {
        let xml = package(
            r#"
	<!-- package comment with data --><?package instruction?>
	<import namespace="urn:external">
		<!-- import comment --><?import instruction?>
	</import>
	<valueType name="V" base="xs:string">
		<!-- named type comment --><?type instruction?>
	</valueType>
	<objectType name="T">
		<property name="Nested">
			<!-- property comment --><?property instruction?>
			<typeDef xsi:type="ObjectType">
				<!-- typeDef comment --><?typedef instruction?>
				<property name="Leaf" type="xs:string"/>
			</typeDef>
		</property>
	</objectType>
"#,
        );

        assert!(findings(&xml).is_empty(), "{:#?}", findings(&xml));
    }

    #[test]
    fn xdto_validation_rejects_text_and_cdata_in_every_supported_container_with_span() {
        let cases = [
            (
                "package text",
                package("unexpected-package<objectType name=\"T\"/>"),
            ),
            (
                "import text",
                package("<import namespace=\"urn:external\">unexpected-import</import>"),
            ),
            (
                "named type text",
                package("<objectType name=\"T\">unexpected-type</objectType>"),
            ),
            (
                "value type text",
                package("<valueType name=\"V\">unexpected-value-type</valueType>"),
            ),
            (
                "property text",
                package(
                    "<objectType name=\"T\"><property name=\"P\">unexpected-property</property></objectType>",
                ),
            ),
            (
                "typeDef text",
                package(
                    "<objectType name=\"T\"><property name=\"P\"><typeDef xsi:type=\"ObjectType\">unexpected-typedef</typeDef></property></objectType>",
                ),
            ),
            (
                "package CDATA",
                package("<![CDATA[unexpected-package]]><objectType name=\"T\"/>"),
            ),
            (
                "import CDATA",
                package(
                    "<import namespace=\"urn:external\"><![CDATA[unexpected-import]]></import>",
                ),
            ),
            (
                "named type CDATA",
                package("<objectType name=\"T\"><![CDATA[unexpected-type]]></objectType>"),
            ),
            (
                "value type CDATA",
                package(
                    "<valueType name=\"V\"><![CDATA[unexpected-value-type]]></valueType>",
                ),
            ),
            (
                "property CDATA",
                package(
                    "<objectType name=\"T\"><property name=\"P\"><![CDATA[unexpected-property]]></property></objectType>",
                ),
            ),
            (
                "typeDef CDATA",
                package(
                    "<objectType name=\"T\"><property name=\"P\"><typeDef xsi:type=\"ObjectType\"><![CDATA[unexpected-typedef]]></typeDef></property></objectType>",
                ),
            ),
        ];

        for (label, xml) in cases {
            let unsupported = findings(&xml)
                .into_iter()
                .filter(|finding| finding.code == "unsupported_node")
                .collect::<Vec<_>>();
            assert_eq!(unsupported.len(), 1, "{label}: {unsupported:#?}");
            let span = &unsupported[0].location.span;
            assert!(
                unsupported[0]
                    .location
                    .key
                    .ends_with("/unsupported:character-data"),
                "{label}: {:?}",
                unsupported[0].location.key
            );
            assert!(span.start < span.end, "{label}: {span:#?}");
            assert!(
                xml[span.start..span.end].contains("unexpected-"),
                "{label}: {:?}",
                &xml[span.start..span.end]
            );
        }
    }

    #[test]
    fn xdto_validation_does_not_treat_non_xml_unicode_space_as_formatting() {
        let xml = package("\u{00a0}<objectType name=\"T\"/>");

        let unsupported = findings(&xml)
            .into_iter()
            .filter(|finding| finding.code == "unsupported_node")
            .collect::<Vec<_>>();

        assert_eq!(unsupported.len(), 1, "{unsupported:#?}");
        let span = &unsupported[0].location.span;
        assert_eq!(&xml[span.start..span.end], "\u{00a0}");
    }

    #[test]
    fn xdto_validation_does_not_use_default_namespace_for_bare_qname_values() {
        let xml = package(r#"<valueType name="V" base="string"/>"#);

        assert_eq!(codes(&xml), vec!["invalid_qname"]);
    }

    #[test]
    fn xdto_validation_accepts_node_scope_prefix_for_imported_external_type() {
        let xml = package(
            r#"<import namespace="urn:external"/>
	<objectType name="T"><property xmlns:ext="urn:external" name="P" type="ext:Remote"/></objectType>"#,
        );

        assert!(findings(&xml).is_empty(), "{:#?}", findings(&xml));
    }

    #[test]
    fn xdto_validation_treats_xsi_type_only_as_object_type_discriminator() {
        let xml = package(
            r#"<objectType name="T"><property name="P"><typeDef xsi:type="ObjectType"/></property></objectType>"#,
        );

        assert!(findings(&xml).is_empty(), "{:#?}", findings(&xml));
    }

    #[test]
    fn xdto_validation_local_ref_requires_global_property_not_same_named_type() {
        let type_only = package(
            r#"<valueType name="Shared" base="xs:string"/>
	<objectType name="T"><property ref="tns:Shared"/></objectType>"#,
        );
        assert_eq!(codes(&type_only), vec!["unknown_property_reference"]);

        let with_global_property = package(
            r#"<property name="Shared" type="xs:string"/>
	<valueType name="Shared" base="xs:string"/>
	<objectType name="T"><property ref="tns:Shared"/></objectType>"#,
        );
        assert!(
            findings(&with_global_property).is_empty(),
            "{:#?}",
            findings(&with_global_property)
        );
    }

    #[test]
    fn xdto_validation_rejects_unknown_local_base() {
        let xml = package(r#"<valueType name="V" base="tns:Missing"/>"#);

        assert_eq!(codes(&xml), vec!["unknown_type_reference"]);
    }

    #[test]
    fn xdto_validation_recurses_through_nested_object_type_defs() {
        let xml = package(
            r#"<objectType name="T">
	<property name="Outer"><typeDef xsi:type="ObjectType">
		<property name="Inner"><typeDef xsi:type="ObjectType">
			<property name="Leaf" type="tns:Missing"/>
		</typeDef></property>
	</typeDef></property>
</objectType>"#,
        );

        let validation = findings(&xml);
        assert_eq!(validation.len(), 1, "{validation:#?}");
        assert_eq!(validation[0].code, "unknown_type_reference");
        assert!(
            validation[0]
                .location
                .key
                .contains("property:Outer/typeDef/property:Inner/typeDef/property:Leaf/@type"),
            "{:?}",
            validation[0].location.key
        );
    }

    #[test]
    fn xdto_validation_accepts_every_supported_container_edge() {
        let xml = package(
            r#"<import namespace="urn:external"/>
	<property name="Global" type="xs:string"/>
	<valueType name="V" base="xs:string"/>
	<objectType name="T">
		<property name="Nested"><typeDef xsi:type="ObjectType">
			<property ref="tns:Global"/>
		</typeDef></property>
	</objectType>"#,
        );

        assert!(findings(&xml).is_empty(), "{:#?}", findings(&xml));
    }

    #[test]
    fn xdto_validation_rejects_each_explicitly_unsupported_edge() {
        for (label, body) in [
            (
                "valueType enumeration",
                r#"<valueType name="V" base="xs:string"><enumeration/></valueType>"#,
            ),
            (
                "valueType pattern",
                r#"<valueType name="V" base="xs:string"><pattern/></valueType>"#,
            ),
            (
                "valueType typeDef",
                r#"<valueType name="V"><typeDef xsi:type="ValueType"/></valueType>"#,
            ),
            (
                "property ValueType typeDef",
                r#"<objectType name="T"><property name="P"><typeDef xsi:type="ValueType"/></property></objectType>"#,
            ),
            (
                "valueType memberTypes",
                r#"<valueType name="V" memberTypes="xs:string"/>"#,
            ),
            (
                "import child",
                r#"<import namespace="urn:external"><property name="P"/></import>"#,
            ),
            (
                "property child",
                r#"<objectType name="T"><property name="P"><objectType name="Nested"/></property></objectType>"#,
            ),
            (
                "typeDef child",
                r#"<objectType name="T"><property name="P"><typeDef xsi:type="ObjectType"><valueType name="V"/></typeDef></property></objectType>"#,
            ),
        ] {
            assert!(
                codes(&package(body))
                    .iter()
                    .any(|code| code == "unsupported_node"),
                "{label}"
            );
        }
    }
}
