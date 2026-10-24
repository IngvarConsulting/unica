//! Exact admitted XDTO byte decoding and logical read projections.
use crate::descriptor::descriptor_identity;
use crate::model::PackageModel;
use crate::validation::validate;
use crate::{decode, model, validation, writer};
use unica_address::source_target::{MetadataAddress, TargetKind};
use unica_xdto::results::*;
/// Parse the exact XDTO bytes admitted by a caller-owned read capability.
///
/// This is the shared construction core for the V12 filesystem wrapper and
/// retained V13 readers. It deliberately accepts no workspace or path, so it
/// cannot re-resolve a source set after admission.
pub fn parse_xdto_info_bytes(
    package_bytes: &[u8],
    descriptor_bytes: &[u8],
    source_set: &str,
    metadata_address: &MetadataAddress,
    type_name: Option<&str>,
) -> Result<XdtoInfoData, XdtoInfoError> {
    if source_set.trim().is_empty() || source_set != source_set.trim() {
        return Err("source_set_unknown: sourceSet must be a non-empty string"
            .to_string()
            .into());
    }
    let identity = AdmittedXdtoIdentity {
        source_set,
        metadata_path: metadata_address.as_str(),
    };
    let model = parse_xdto_info_model(package_bytes, descriptor_bytes, &identity)?;
    build_xdto_info(&identity, &model, type_name, None)
}
pub fn parse_xdto_info_model(
    package_bytes: &[u8],
    descriptor_bytes: &[u8],
    identity: &impl XdtoIdentity,
) -> Result<PackageModel, String> {
    let expected_name = xdto_metadata_name(identity.metadata_path())?;
    let text = decode(package_bytes)?;
    let model = PackageModel::parse(&text).map_err(model_error)?;
    let descriptor = descriptor_identity(descriptor_bytes)?;
    if descriptor.name != expected_name {
        return Err("not_an_xdto_package: descriptor Name does not match metadataPath".to_string());
    }
    if model.target_namespace() != Some(descriptor.namespace.as_str()) {
        return Err(
            "namespace_mismatch: descriptor Namespace must equal package targetNamespace"
                .to_string(),
        );
    }
    Ok(model)
}
pub fn build_xdto_info(
    identity: &impl XdtoIdentity,
    model: &PackageModel,
    requested: Option<&str>,
    page: Option<(usize, usize, Option<String>)>,
) -> Result<XdtoInfoData, XdtoInfoError> {
    validate_xdto_type_name(requested)?;
    let value_types = model
        .types
        .iter()
        .filter(|named| named.kind == model::TypeKind::Value)
        .count();
    let counts = XdtoTypeCounts {
        total: model.types.len(),
        value_types,
        object_types: model.types.len() - value_types,
        global_properties: model.global_properties.len(),
    };
    let (selected, type_detail, next_cursor) = if let Some(name) = requested {
        let matches = model
            .types
            .iter()
            .filter(|named| named.name.as_ref().is_some_and(|value| value.value == name))
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(if matches.is_empty() {
                XdtoInfoError::TypeNotFound(name.to_string())
            } else {
                XdtoInfoError::Source(format!("duplicate_type: XDTO type `{name}` is ambiguous"))
            });
        }
        (
            Vec::new(),
            Some(type_info(identity, model, matches[0])),
            None,
        )
    } else {
        let (start, end, next_cursor) = page.unwrap_or((0, model.types.len(), None));
        (
            model.types[start..end].iter().collect::<Vec<_>>(),
            None,
            next_cursor,
        )
    };
    let location = addressed_location(identity);
    Ok(XdtoInfoData {
        source_set: identity.source_set().to_string(),
        metadata_path: identity.metadata_path().to_string(),
        location,
        target_namespace: model
            .target_namespace
            .as_ref()
            .map(|value| value.value.clone()),
        imports: model
            .imports
            .iter()
            .map(|import| XdtoImportInfo {
                namespace: import
                    .namespace
                    .as_ref()
                    .map(|namespace| namespace.value.clone()),
                location: internal_location(identity, &import.key, &import.span),
            })
            .collect(),
        counts,
        global_properties: model
            .global_properties
            .iter()
            .map(|property| property_info(identity, property))
            .collect(),
        types: selected
            .into_iter()
            .map(|named| type_summary(identity, named))
            .collect(),
        type_detail,
        findings: validate(model)
            .into_iter()
            .map(|finding| baseline_finding(identity, finding))
            .collect(),
        next_cursor,
    })
}
pub fn validate_xdto_type_name(requested: Option<&str>) -> Result<(), String> {
    if requested.is_some_and(|name| !validation::is_ncname(name)) {
        return Err("xdto info `typeName` must be an XML NCName".to_string());
    }
    Ok(())
}
pub fn addressed_location(package: &impl XdtoIdentity) -> XdtoLocation {
    XdtoLocation::Addressed {
        source_set: package.source_set().to_string(),
        metadata_path: package.metadata_path().to_string(),
        target_kind: TargetKind::MetadataObject,
    }
}
fn internal_location(
    package: &impl XdtoIdentity,
    node_key: &str,
    span: &model::SourceSpan,
) -> XdtoLocation {
    XdtoLocation::Unaddressable {
        source_set: package.source_set().to_string(),
        owner_metadata_path: package.metadata_path().to_string(),
        node_key: node_key.to_string(),
        body_byte_range: XdtoByteRange {
            start: span.start,
            end: span.end,
        },
    }
}
pub(crate) fn qname_info(qname: &model::QNameRef) -> XdtoQNameInfo {
    XdtoQNameInfo {
        raw: qname.raw.clone(),
        prefix: qname.prefix.clone(),
        local: qname.local.clone(),
        namespace: qname.namespace.clone(),
    }
}
fn local_base_address(
    package: &impl XdtoIdentity,
    model: &PackageModel,
    named: &model::NamedType,
) -> Option<String> {
    let base = named.base.as_ref()?;
    if base.namespace.as_deref() != model.target_namespace() {
        return None;
    }
    let mut candidates = model.types.iter().filter(|candidate| {
        candidate
            .name
            .as_ref()
            .is_some_and(|name| name.value == base.local)
    });
    let parent = candidates.next()?;
    if candidates.next().is_some() || parent.kind != named.kind {
        return None;
    }
    let at = format!(
        "{}:{}.Type.{}",
        package.source_set(),
        package.metadata_path(),
        unica_address::address::escape_name(&base.local)
    );
    let address = unica_address::address::QualifiedAddress::parse(&at).ok()?;
    (address.segments().len() == 2 && address.segments()[1].name() == Some(base.local.as_str()))
        .then_some(at)
}

fn type_info(
    package: &impl XdtoIdentity,
    model: &PackageModel,
    named: &model::NamedType,
) -> XdtoTypeInfo {
    XdtoTypeInfo {
        facets: named.facets.clone(),
        kind: match named.kind {
            model::TypeKind::Value => XdtoTypeKind::Value,
            model::TypeKind::Object => XdtoTypeKind::Object,
        },
        name: named.name.as_ref().map(|name| name.value.clone()),
        base: named.base.as_ref().map(qname_info),
        base_at: local_base_address(package, model, named),
        properties: named
            .properties
            .iter()
            .map(|property| property_info(package, property))
            .collect(),
        location: internal_location(package, &named.key, &named.span),
    }
}
fn type_summary(package: &impl XdtoIdentity, named: &model::NamedType) -> XdtoTypeSummary {
    XdtoTypeSummary {
        kind: match named.kind {
            model::TypeKind::Value => XdtoTypeKind::Value,
            model::TypeKind::Object => XdtoTypeKind::Object,
        },
        name: named.name.as_ref().map(|name| name.value.clone()),
        base: named.base.as_ref().map(qname_info),
        location: internal_location(package, &named.key, &named.span),
    }
}
fn property_info(package: &impl XdtoIdentity, property: &model::Property) -> XdtoPropertyInfo {
    let lower = property
        .lower_bound
        .as_ref()
        .map_or("1", |value| value.value.as_str());
    let upper = property
        .upper_bound
        .as_ref()
        .map_or("1", |value| value.value.as_str());
    XdtoPropertyInfo {
        name: property.name.as_ref().map(|name| name.value.clone()),
        reference: property.reference.as_ref().map(qname_info),
        type_name: property.type_ref.as_ref().map(qname_info),
        bounds: XdtoBoundsInfo {
            lower: lower.to_string(),
            upper: upper.to_string(),
            lower_explicit: property.lower_bound.is_some(),
            upper_explicit: property.upper_bound.is_some(),
            unbounded: upper == "-1",
        },
        type_defs: property
            .type_defs
            .iter()
            .map(|anonymous| anonymous_type_info(package, anonymous))
            .collect(),
        location: internal_location(package, &property.key, &property.span),
    }
}
fn anonymous_type_info(
    package: &impl XdtoIdentity,
    anonymous: &model::AnonymousObject,
) -> XdtoAnonymousTypeInfo {
    XdtoAnonymousTypeInfo {
        facets: anonymous.facets.clone(),
        base: anonymous.base.as_ref().map(qname_info),
        discriminator: anonymous
            .discriminator
            .as_ref()
            .map(|value| value.value.clone()),
        properties: anonymous
            .properties
            .iter()
            .map(|property| property_info(package, property))
            .collect(),
        location: internal_location(package, &anonymous.key, &anonymous.span),
    }
}
fn baseline_finding(package: &impl XdtoIdentity, finding: validation::Finding) -> XdtoFinding {
    XdtoFinding {
        code: finding.code,
        severity: XdtoFindingSeverity::Error,
        state: XdtoFindingState::PreExisting,
        phase: XdtoFindingPhase::Before,
        message: finding.message,
        location: XdtoLocation::Unaddressable {
            source_set: package.source_set().to_string(),
            owner_metadata_path: package.metadata_path().to_string(),
            node_key: finding.location.key,
            body_byte_range: XdtoByteRange {
                start: finding.location.span.start,
                end: finding.location.span.end,
            },
        },
    }
}
pub fn classified_finding(
    package: &impl XdtoIdentity,
    finding: &validation::ClassifiedFinding,
) -> XdtoFinding {
    XdtoFinding {
        code: finding.code.clone(),
        severity: XdtoFindingSeverity::Error,
        state: match finding.state {
            validation::FindingState::PreExisting => XdtoFindingState::PreExisting,
            validation::FindingState::Introduced => XdtoFindingState::Introduced,
        },
        phase: match finding.state {
            validation::FindingState::PreExisting => XdtoFindingPhase::Before,
            validation::FindingState::Introduced => XdtoFindingPhase::After,
        },
        message: finding.message.clone(),
        location: XdtoLocation::Unaddressable {
            source_set: package.source_set().to_string(),
            owner_metadata_path: package.metadata_path().to_string(),
            node_key: finding.location.key.clone(),
            body_byte_range: XdtoByteRange {
                start: finding.location.span.start,
                end: finding.location.span.end,
            },
        },
    }
}
pub fn writer_finding(package: &impl XdtoIdentity, finding: &writer::WriterFinding) -> XdtoFinding {
    XdtoFinding {
        code: finding.code.to_string(),
        severity: match finding.severity {
            writer::WriterFindingSeverity::Info => XdtoFindingSeverity::Info,
            writer::WriterFindingSeverity::Error => XdtoFindingSeverity::Error,
        },
        state: match finding.state {
            writer::WriterFindingState::PreExisting => XdtoFindingState::PreExisting,
            writer::WriterFindingState::Introduced => XdtoFindingState::Introduced,
        },
        phase: match finding.state {
            writer::WriterFindingState::PreExisting => XdtoFindingPhase::Before,
            writer::WriterFindingState::Introduced => XdtoFindingPhase::After,
        },
        message: finding.message.clone(),
        location: XdtoLocation::Unaddressable {
            source_set: package.source_set().to_string(),
            owner_metadata_path: package.metadata_path().to_string(),
            node_key: finding.location.key.clone(),
            body_byte_range: XdtoByteRange {
                start: finding.location.span.start,
                end: finding.location.span.end,
            },
        },
    }
}
pub fn model_error(error: model::ModelError) -> String {
    format!(
        "{}: {} at byte {}",
        error.code, error.message, error.span.start
    )
}
pub fn xdto_metadata_name(metadata_path: &str) -> Result<&str, String> {
    let parts = metadata_path.split('.').collect::<Vec<_>>();
    if parts.len() != 2 || parts[0] != "XDTOPackage" || parts[1].is_empty() {
        return Err("not_an_xdto_package: metadataPath must be XDTOPackage.<name>".to_string());
    }
    Ok(parts[1])
}
/// Selection absence remains distinct from failures to read an XDTO source.
#[derive(Debug)]
pub enum XdtoInfoError {
    TypeNotFound(String),
    Source(String),
}
impl std::fmt::Display for XdtoInfoError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TypeNotFound(name) => {
                write!(
                    formatter,
                    "target_not_found: XDTO type `{name}` was not found"
                )
            }
            Self::Source(message) => formatter.write_str(message),
        }
    }
}
impl std::error::Error for XdtoInfoError {}
impl From<String> for XdtoInfoError {
    fn from(message: String) -> Self {
        Self::Source(message)
    }
}
struct AdmittedXdtoIdentity<'a> {
    source_set: &'a str,
    metadata_path: &'a str,
}
impl XdtoIdentity for AdmittedXdtoIdentity<'_> {
    fn source_set(&self) -> &str {
        self.source_set
    }

    fn metadata_path(&self) -> &str {
        self.metadata_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode_like;
    use crate::test_support::{args, PACKAGE};
    use serde_json::json;
    use unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20;
    #[test]
    fn xdto_info_shared_core_uses_exact_descriptor_identity_and_admitted_bytes() {
        let address =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "XDTOPackage.Sample").unwrap();
        let descriptor = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><XDTOPackage><Properties><Name>Sample</Name><Namespace>urn:test</Namespace></Properties></XDTOPackage></MetaDataObject>"#;

        let data = parse_xdto_info_bytes(
            PACKAGE.as_bytes(),
            descriptor,
            "main",
            &address,
            Some("ЛюбаяСсылка"),
        )
        .unwrap();
        let value = serde_json::to_value(data).unwrap();
        assert_eq!(value["sourceSet"], "main");
        assert_eq!(value["metadataPath"], "XDTOPackage.Sample");
        assert_eq!(value["typeDetail"]["name"], "ЛюбаяСсылка");

        let wrong_owner = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><XDTOPackage><Properties><Name>Other</Name><Namespace>urn:test</Namespace></Properties></XDTOPackage></MetaDataObject>"#;
        let error = parse_xdto_info_bytes(PACKAGE.as_bytes(), wrong_owner, "main", &address, None)
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "not_an_xdto_package: descriptor Name does not match metadataPath"
        );

        let malformed = parse_xdto_info_bytes(
            PACKAGE.as_bytes(),
            b"<MetaDataObject",
            "main",
            &address,
            None,
        )
        .expect_err("malformed descriptor must fail");
        assert_eq!(
            malformed.to_string(),
            "not_an_xdto_package: XDTO descriptor is not valid XML"
        );

        let other_namespace = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><XDTOPackage><Properties><Name>Sample</Name><Namespace>urn:other</Namespace></Properties></XDTOPackage></MetaDataObject>"#;
        let mismatch =
            parse_xdto_info_bytes(PACKAGE.as_bytes(), other_namespace, "main", &address, None)
                .expect_err("namespace mismatch must fail");
        assert_eq!(
            mismatch.to_string(),
            "namespace_mismatch: descriptor Namespace must equal package targetNamespace"
        );
    }
    #[test]
    fn add_property_descends_through_property_path_and_is_idempotent() {
        let args = args(&[
            ("typeName", json!("ЛюбаяСсылка")),
            ("propertyPath", json!("СсылкаНаОбъект")),
            (
                "property",
                json!({"name":"Документ_Новый", "type":"tns:Документ_Новый", "minOccurs":0}),
            ),
        ]);
        let once = writer::plan(PACKAGE, &args, "add-property").unwrap();
        assert_eq!(
            once.after,
            r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:tns="urn:test" targetNamespace="urn:test">
	<objectType name="ЛюбаяСсылка">
		<property name="СсылкаНаОбъект">
			<typeDef xsi:type="ObjectType">
				<property name="Документ_Новый" type="tns:Документ_Новый" lowerBound="0"/>
			</typeDef>
		</property>
	</objectType>
	<objectType name="СоставнойЛюбойОбъект"/>
</package>"#
        );
        let repeated = writer::plan(&once.after, &args, "add-property").unwrap();
        assert_eq!(repeated.after, once.after);
        assert!(repeated.edits.is_empty());
        assert_eq!(repeated.finding.unwrap().code, "duplicate_property");
    }
    #[test]
    fn byte_encoding_keeps_bom_and_crlf() {
        let before = format!("\u{feff}{}", PACKAGE.replace('\n', "\r\n")).into_bytes();
        let text = decode(&before).unwrap();
        let after = writer::plan(
            &text,
            &args(&[("name", json!("Новый")), ("base", json!("xs:string"))]),
            "add-value-type",
        )
        .unwrap()
        .after;
        let bytes = encode_like(&before, &after);
        assert!(bytes.starts_with(&[0xef, 0xbb, 0xbf]));
        assert!(decode(&bytes).unwrap().contains("\r\n"));
    }
    #[test]
    fn xdto_validation_accepts_the_untouched_enterprise_data_fixture() {
        let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/xdto/enterprise-data-minimal/XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin"
    ));
        let text = decode(bytes).unwrap();
        let model = super::model::PackageModel::parse(&text).unwrap();

        let findings = super::validation::validate(&model);

        assert!(findings.is_empty(), "{findings:#?}");
    }
    #[test]
    fn xdto_validation_diff_uses_code_and_logical_key_not_message_or_span() {
        use super::validation::{
            Finding, FindingLocation, FindingSeverity, FindingState, SourceSpanDto, ValidationDiff,
        };
        let baseline = vec![Finding {
            code: "unknown_type_reference".to_string(),
            severity: FindingSeverity::Error,
            message: "old wording".to_string(),
            location: FindingLocation {
                key: "$package/objectType:Consumer/property:Value/@type".to_string(),
                span: SourceSpanDto { start: 10, end: 18 },
            },
        }];
        let candidate = vec![Finding {
            code: "unknown_type_reference".to_string(),
            severity: FindingSeverity::Error,
            message: "new wording".to_string(),
            location: FindingLocation {
                key: "$package/objectType:Consumer/property:Value/@type".to_string(),
                span: SourceSpanDto {
                    start: 110,
                    end: 118,
                },
            },
        }];

        let diff = ValidationDiff::between(&baseline, candidate);

        assert_eq!(diff.findings.len(), 1);
        assert_eq!(diff.findings[0].state, FindingState::PreExisting);
        assert_eq!(diff.findings[0].message, "old wording");
        assert_eq!(diff.findings[0].location.span.start, 10);
        assert!(!diff.blocks());
    }
    #[test]
    fn xdto_validation_diff_classifies_multiplicity_removal_and_fresh_keys() {
        use super::validation::{
            Finding, FindingLocation, FindingSeverity, FindingState, SourceSpanDto, ValidationDiff,
        };
        let finding = |start| Finding {
            code: "duplicate_property".to_string(),
            severity: FindingSeverity::Error,
            message: "duplicate".to_string(),
            location: FindingLocation {
                key: "$package/objectType:Consumer/property:Value/@unique".to_string(),
                span: SourceSpanDto {
                    start,
                    end: start + 1,
                },
            },
        };

        let diff = ValidationDiff::between(&[finding(10)], vec![finding(20), finding(30)]);

        assert_eq!(diff.findings.len(), 2);
        assert_eq!(diff.findings[0].state, FindingState::PreExisting);
        assert_eq!(diff.findings[1].state, FindingState::Introduced);
        assert!(diff.blocks());

        let removed = ValidationDiff::between(&[finding(40)], Vec::new());
        assert!(removed.findings.is_empty());
        assert!(!removed.blocks());

        let fresh = ValidationDiff::between(&[], vec![finding(50)]);
        assert_eq!(fresh.findings[0].state, FindingState::Introduced);
        assert!(fresh.blocks());
    }
    #[test]
    fn xdto_validation_requires_exact_root_and_target_namespace() {
        let wrong_root = super::model::PackageModel::parse(
            r#"<package xmlns="urn:not-xdto" targetNamespace="urn:test"/>"#,
        )
        .unwrap_err();
        assert_eq!(wrong_root.code, "not_an_xdto_package");

        let missing_namespace =
            super::model::PackageModel::parse(r#"<package xmlns="http://v8.1c.ru/8.1/xdto"/>"#)
                .unwrap();
        let findings = super::validation::validate(&missing_namespace);
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].code, "target_namespace_required");
        assert_eq!(findings[0].location.key, "$package/@targetNamespace");
    }
    #[test]
    fn xdto_validation_uses_xml_ncname_character_ranges() {
        let model = super::model::PackageModel::parse(
            r#"<package xmlns="http://v8.1c.ru/8.1/xdto" targetNamespace="urn:test">
	<objectType name="_ok"/>
	<objectType name="Имя"/>
	<objectType name="A·B"/>
	<objectType name="Á"/>
	<objectType name="1bad"/>
	<objectType name="bad:name"/>
</package>"#,
        )
        .unwrap();

        let invalid_names = super::validation::validate(&model)
            .into_iter()
            .filter(|finding| finding.code == "invalid_ncname")
            .map(|finding| finding.location.key)
            .collect::<Vec<_>>();

        assert_eq!(invalid_names.len(), 2, "{invalid_names:#?}");
        assert!(invalid_names.iter().any(|key| key.contains("1bad")));
        assert!(invalid_names.iter().any(|key| key.contains("bad:name")));
    }
    #[test]
    fn staged_xdto_writer_errors_are_typed_without_v12_drift() {
        let legacy = writer::plan(PACKAGE, &args(&[("name", json!("Missing"))]), "remove-type")
            .expect_err("legacy missing type unexpectedly succeeded");
        assert_eq!(legacy, "target_not_found: type does not exist");

        let typed = writer::plan_typed(
            PACKAGE,
            writer::TypedWriterOperation::RemoveType { name: "Missing" },
        )
        .expect_err("typed missing type unexpectedly succeeded");
        assert_eq!(typed.cause(), writer::WriterErrorCause::NotFound);
        assert_eq!(typed.to_string(), legacy);

        let ambiguous = r#"<package xmlns="http://v8.1c.ru/8.1/xdto" targetNamespace="urn:test"><objectType name="Same"/><objectType name="Same"/></package>"#;
        let typed = writer::plan_typed(
            ambiguous,
            writer::TypedWriterOperation::RemoveType { name: "Same" },
        )
        .expect_err("typed ambiguous type unexpectedly succeeded");
        assert_eq!(typed.cause(), writer::WriterErrorCause::AmbiguousTarget);
        assert_eq!(
            typed.to_string(),
            "unsupported_node: type identity is ambiguous"
        );
    }
}
