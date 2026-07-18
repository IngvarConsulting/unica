use super::contract::{
    ArtifactRef, CfePatchMethodArguments, DiscoverRequest, MutationIntent, Proposal, StableTag,
};
use super::model::{
    BindingDetails, Check, DefinitionParameter, DefinitionShape, DiscoveryReport, DiscoverySource,
    Evidence, EvidenceRecord, LinkedSourceSnapshot, PlatformCallbackShape, ProviderFact,
    ProviderOutcomeSnapshot, ProviderReadiness,
};
use crate::domain::project_sources::SourceFormat;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::BTreeMap;
#[cfg(test)]
use std::collections::BTreeSet;
use std::fmt;

const CANONICAL_FORMAT_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DeterminismError {
    InvalidComponent(String),
    IdentifierCollision { id: String },
}

impl fmt::Display for DeterminismError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidComponent(message) => formatter.write_str(message),
            Self::IdentifierCollision { id } => {
                write!(formatter, "stable identifier collision for {id}")
            }
        }
    }
}

impl std::error::Error for DeterminismError {}

struct CanonicalEncoder {
    hasher: Sha256,
}

impl CanonicalEncoder {
    fn new(domain: &str) -> Result<Self, DeterminismError> {
        if domain.is_empty() {
            return Err(DeterminismError::InvalidComponent(
                "canonical domain must not be empty".to_string(),
            ));
        }
        let mut encoder = Self {
            hasher: Sha256::new(),
        };
        encoder.hasher.update(b"unica.discovery.canonical\0");
        encoder.write_u16(CANONICAL_FORMAT_VERSION);
        encoder.write_string(domain)?;
        Ok(encoder)
    }

    fn write_u8(&mut self, value: u8) {
        self.hasher.update([0x01, value]);
    }

    fn write_bool(&mut self, value: bool) {
        self.hasher.update([0x02, u8::from(value)]);
    }

    fn write_u16(&mut self, value: u16) {
        self.hasher.update([0x03]);
        self.hasher.update(value.to_be_bytes());
    }

    fn write_u32(&mut self, value: u32) {
        self.hasher.update([0x04]);
        self.hasher.update(value.to_be_bytes());
    }

    fn write_u64(&mut self, value: u64) {
        self.hasher.update([0x05]);
        self.hasher.update(value.to_be_bytes());
    }

    fn write_len(&mut self, value: u64) {
        self.write_u64(value);
    }

    fn write_string(&mut self, value: &str) -> Result<(), DeterminismError> {
        self.hasher.update([0x06]);
        self.write_len(canonical_len(value.len())?);
        self.hasher.update(value.as_bytes());
        Ok(())
    }

    fn write_optional_string(&mut self, value: Option<&str>) -> Result<(), DeterminismError> {
        match value {
            Some(value) => {
                self.write_u8(1);
                self.write_string(value)?;
            }
            None => self.write_u8(0),
        }
        Ok(())
    }

    fn finish(self) -> String {
        lowercase_hex(&self.hasher.finalize())
    }
}

fn canonical_len<T>(value: T) -> Result<u64, DeterminismError>
where
    T: TryInto<u64>,
{
    value.try_into().map_err(|_| {
        DeterminismError::InvalidComponent("canonical collection is too large".to_string())
    })
}

fn lowercase_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from_digit(u32::from(byte >> 4), 16).expect("nibble is hexadecimal"));
        output.push(char::from_digit(u32::from(byte & 0x0f), 16).expect("nibble is hexadecimal"));
    }
    output
}

fn normalized_identity(value: &str) -> String {
    value.chars().flat_map(char::to_lowercase).collect()
}

fn encode_artifact(
    encoder: &mut CanonicalEncoder,
    artifact: &ArtifactRef,
) -> Result<(), DeterminismError> {
    encoder.write_u16(artifact.kind.stable_tag());
    encoder.write_string(&normalized_identity(&artifact.canonical_ref))
}

fn encode_string_set(
    encoder: &mut CanonicalEncoder,
    values: &[String],
) -> Result<(), DeterminismError> {
    let mut values = values.to_vec();
    values.sort();
    encoder.write_len(canonical_len(values.len())?);
    for value in values {
        encoder.write_string(&value)?;
    }
    Ok(())
}

fn encode_proposal(
    encoder: &mut CanonicalEncoder,
    proposal: &Proposal,
) -> Result<(), DeterminismError> {
    encoder.write_string(&proposal.id)?;
    encode_artifact(encoder, &proposal.target)?;
    encoder.write_string(&proposal.intent)?;
    match &proposal.mutation_intent {
        Some(MutationIntent::CfePatchMethod {
            destination_source_set,
            arguments,
        }) => {
            encoder.write_u8(1);
            encoder.write_string("unica.cfe.patch_method")?;
            encoder.write_string(destination_source_set)?;
            encode_cfe_patch_arguments(encoder, arguments)?;
        }
        None => encoder.write_u8(0),
    }
    Ok(())
}

fn encode_cfe_patch_arguments(
    encoder: &mut CanonicalEncoder,
    arguments: &CfePatchMethodArguments,
) -> Result<(), DeterminismError> {
    encoder.write_string(&arguments.extension_path)?;
    encoder.write_string(&arguments.module_path)?;
    encoder.write_string(&arguments.method_name)?;
    encoder.write_u16(arguments.interceptor_type.stable_tag());
    encoder.write_u16(arguments.context.stable_tag());
    encoder.write_bool(arguments.is_function);
    Ok(())
}

fn encode_request(
    encoder: &mut CanonicalEncoder,
    request: &DiscoverRequest,
) -> Result<(), DeterminismError> {
    encoder.write_u16(request.mode.stable_tag());
    encoder.write_string(&request.task)?;
    encode_string_set(encoder, &request.concepts)?;
    encode_string_set(encoder, &request.search_terms)?;

    let mut known_artifacts = request.known_artifacts.clone();
    known_artifacts.sort();
    encoder.write_len(canonical_len(known_artifacts.len())?);
    for artifact in &known_artifacts {
        encode_artifact(encoder, artifact)?;
    }

    let mut proposals = request.proposals.clone();
    proposals.sort_by(|left, right| left.id.cmp(&right.id));
    encoder.write_len(canonical_len(proposals.len())?);
    for proposal in &proposals {
        encode_proposal(encoder, proposal)?;
    }

    encoder.write_optional_string(request.source_set.as_deref())?;
    encoder.write_u16(request.limits.max_candidates);
    encoder.write_u8(request.limits.max_graph_depth);
    encoder.write_u16(request.limits.max_evidence);
    // cwd is a workspace-resolution transport argument. The resolved source
    // identity below binds the workspace content and is the stable boundary.
    Ok(())
}

fn snapshot_sort_key(snapshot: &LinkedSourceSnapshot) -> (u16, String, String) {
    (
        snapshot.role.stable_tag(),
        snapshot.source_set.clone(),
        snapshot.source_fingerprint.clone(),
    )
}

fn source_format_tag(source_format: SourceFormat) -> u16 {
    match source_format {
        SourceFormat::PlatformXml => 1,
        SourceFormat::Edt => 2,
        SourceFormat::Unknown => 3,
        SourceFormat::Invalid => 4,
    }
}

fn encode_source(
    encoder: &mut CanonicalEncoder,
    source: &DiscoverySource,
) -> Result<(), DeterminismError> {
    validate_fingerprint(&source.composite_source_fingerprint)?;
    encoder.write_string(&source.analysis_source_set)?;
    encoder.write_u16(source_format_tag(source.source_format));
    let mut snapshots = source.linked_source_snapshots.clone();
    snapshots.sort_by_key(snapshot_sort_key);
    encoder.write_len(canonical_len(snapshots.len())?);
    for snapshot in snapshots {
        validate_fingerprint(&snapshot.source_fingerprint)?;
        encoder.write_string(&snapshot.source_set)?;
        encoder.write_u16(snapshot.role.stable_tag());
        encoder.write_string(&snapshot.source_fingerprint)?;
    }
    encoder.write_string(&source.composite_source_fingerprint)?;
    // workspaceEpoch is diagnostic-only and deliberately excluded.
    Ok(())
}

fn provider_sort_key(
    provider: &ProviderOutcomeSnapshot,
) -> (u16, String, String, u16, u16, String, Vec<String>) {
    let mut record_digests = provider.record_digests.clone();
    record_digests.sort();
    (
        provider.port.stable_tag(),
        provider.name.clone(),
        provider.version.clone(),
        provider.readiness.stable_tag(),
        provider.coverage.stable_tag(),
        provider.reason_code.clone().unwrap_or_default(),
        record_digests,
    )
}

fn encode_provider_outcome(
    encoder: &mut CanonicalEncoder,
    provider: &ProviderOutcomeSnapshot,
) -> Result<(), DeterminismError> {
    if provider.name.trim().is_empty() || provider.version.trim().is_empty() {
        return Err(DeterminismError::InvalidComponent(
            "provider identity contains an empty component".to_string(),
        ));
    }
    encoder.write_u16(provider.port.stable_tag());
    encoder.write_string(&provider.name)?;
    encoder.write_string(&provider.version)?;
    encoder.write_u16(provider.readiness.stable_tag());
    encoder.write_u16(provider.coverage.stable_tag());
    encoder.write_optional_string(provider.reason_code.as_deref())?;
    let mut record_digests = provider.record_digests.clone();
    record_digests.sort();
    encoder.write_len(canonical_len(record_digests.len())?);
    for digest in record_digests {
        validate_fingerprint(&digest)?;
        encoder.write_string(&digest)?;
    }
    Ok(())
}

pub(crate) fn analysis_id(
    request: &DiscoverRequest,
    analysis_contract_version: &str,
    source: &DiscoverySource,
    provider_outcomes: &[ProviderOutcomeSnapshot],
) -> Result<String, DeterminismError> {
    if analysis_contract_version.trim().is_empty() || analysis_contract_version.len() > 128 {
        return Err(DeterminismError::InvalidComponent(
            "analysis contract version must contain 1..=128 bytes".to_string(),
        ));
    }
    let mut encoder = CanonicalEncoder::new("analysis-id")?;
    encoder.write_string(analysis_contract_version)?;
    encode_request(&mut encoder, request)?;
    encode_source(&mut encoder, source)?;
    let mut outcomes = provider_outcomes.to_vec();
    outcomes.sort_by_key(provider_sort_key);
    encoder.write_len(canonical_len(outcomes.len())?);
    for outcome in &outcomes {
        outcome
            .validate()
            .map_err(DeterminismError::InvalidComponent)?;
        encode_provider_outcome(&mut encoder, outcome)?;
    }
    Ok(format!("analysis_{}", encoder.finish()))
}

fn encode_location(
    encoder: &mut CanonicalEncoder,
    record: &EvidenceRecord,
) -> Result<(), DeterminismError> {
    match &record.location {
        Some(location) => {
            encoder.write_u8(1);
            encoder.write_string(&location.path)?;
            match location.line {
                Some(line) => {
                    encoder.write_u8(1);
                    encoder.write_u32(line);
                }
                None => encoder.write_u8(0),
            }
            match location.column {
                Some(column) => {
                    encoder.write_u8(1);
                    encoder.write_u32(column);
                }
                None => encoder.write_u8(0),
            }
        }
        None => encoder.write_u8(0),
    }
    Ok(())
}

fn encode_evidence_record(
    encoder: &mut CanonicalEncoder,
    record: &EvidenceRecord,
) -> Result<(), DeterminismError> {
    validate_fingerprint(&record.freshness.source_fingerprint)?;
    encode_provider_fact(encoder, &record.fact)?;
    encode_location(encoder, record)?;
    encoder.write_u16(record.provider.port.stable_tag());
    encoder.write_string(&record.provider.name)?;
    encoder.write_string(&record.provider.version)?;
    encoder.write_u16(record.coverage.stable_tag());
    encoder.write_string(&record.freshness.source_set)?;
    encoder.write_string(&record.freshness.source_fingerprint)?;
    // workspaceEpoch is diagnostic-only and deliberately excluded.
    Ok(())
}

fn encode_definition_parameter(
    encoder: &mut CanonicalEncoder,
    parameter: &DefinitionParameter,
) -> Result<(), DeterminismError> {
    if parameter.name.trim().is_empty() {
        return Err(DeterminismError::InvalidComponent(
            "definition parameter name must not be blank".to_string(),
        ));
    }
    encoder.write_string(&parameter.name)?;
    encoder.write_bool(parameter.by_value);
    encoder.write_bool(parameter.has_default);
    Ok(())
}

fn encode_definition_shape(
    encoder: &mut CanonicalEncoder,
    definition: &DefinitionShape,
) -> Result<(), DeterminismError> {
    encoder.write_bool(definition.is_function);
    encoder.write_bool(definition.exported);
    encoder.write_len(canonical_len(definition.parameters.len())?);
    for parameter in &definition.parameters {
        encode_definition_parameter(encoder, parameter)?;
    }
    Ok(())
}

fn encode_callback_shape(
    encoder: &mut CanonicalEncoder,
    callback: &PlatformCallbackShape,
) -> Result<(), DeterminismError> {
    for (name, value) in [
        ("platformVariant", callback.platform_variant.as_str()),
        ("metadataKind", callback.metadata_kind.as_str()),
        ("moduleKind", callback.module_kind.as_str()),
        ("methodName", callback.method_name.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(DeterminismError::InvalidComponent(format!(
                "callback {name} must not be blank"
            )));
        }
        encoder.write_string(value)?;
    }
    encoder.write_bool(callback.exported);
    encoder.write_len(canonical_len(callback.parameters.len())?);
    for parameter in &callback.parameters {
        encode_definition_parameter(encoder, parameter)?;
    }
    Ok(())
}

fn encode_provider_fact(
    encoder: &mut CanonicalEncoder,
    fact: &ProviderFact,
) -> Result<(), DeterminismError> {
    encoder.write_u16(fact.stable_tag());
    match fact {
        ProviderFact::MetadataPresent { subject } => {
            encode_artifact(encoder, subject)?;
        }
        ProviderFact::MetadataAbsent { subject } => {
            encode_artifact(encoder, subject)?;
        }
        ProviderFact::CodeOccurrence {
            subject,
            search_term,
        } => {
            if search_term.trim().is_empty() {
                return Err(DeterminismError::InvalidComponent(
                    "code occurrence search term must not be blank".to_string(),
                ));
            }
            encode_artifact(encoder, subject)?;
            encoder.write_string(search_term)?;
        }
        ProviderFact::DefinitionPresent {
            subject,
            definition,
        } => {
            encode_artifact(encoder, subject)?;
            encode_definition_shape(encoder, definition)?;
        }
        ProviderFact::DefinitionAbsent { subject } => {
            encode_artifact(encoder, subject)?;
        }
        ProviderFact::Binding {
            subject,
            object,
            relation,
            details,
        } => {
            encode_artifact(encoder, subject)?;
            encode_artifact(encoder, object)?;
            encoder.write_u16(relation.stable_tag());
            encode_binding_details(encoder, details)?;
        }
        ProviderFact::Call {
            subject,
            object,
            resolution,
            call_type,
            context,
        } => {
            encode_artifact(encoder, subject)?;
            encode_artifact(encoder, object)?;
            encoder.write_u16(resolution.stable_tag());
            encoder.write_u16(call_type.stable_tag());
            encoder.write_u16(context.stable_tag());
        }
        ProviderFact::PlatformCallback {
            subject,
            object,
            callback,
        } => {
            encode_artifact(encoder, subject)?;
            encode_artifact(encoder, object)?;
            encode_callback_shape(encoder, callback)?;
        }
        ProviderFact::Support { subject, state } => {
            encode_artifact(encoder, subject)?;
            encoder.write_u16(state.stable_tag());
        }
    }
    Ok(())
}

fn encode_binding_details(
    encoder: &mut CanonicalEncoder,
    details: &BindingDetails,
) -> Result<(), DeterminismError> {
    encoder.write_u16(details.stable_tag());
    match details {
        BindingDetails::Structural => {}
        BindingDetails::EventSubscription { event, context } => {
            encode_binding_component(encoder, event, "event")?;
            encoder.write_u16(context.stable_tag());
        }
        BindingDetails::FormCommand { action, context } => {
            encode_binding_component(encoder, action, "action")?;
            encoder.write_u16(context.stable_tag());
        }
        BindingDetails::CommonCommand { action, context } => {
            encode_binding_component(encoder, action, "action")?;
            encoder.write_u16(context.stable_tag());
        }
        BindingDetails::ScheduledJob { enabled, context } => {
            encoder.write_bool(*enabled);
            encoder.write_u16(context.stable_tag());
        }
        BindingDetails::HttpRoute {
            verb,
            url_template,
            context,
        } => {
            encoder.write_u16(verb.stable_tag());
            encode_binding_component(encoder, url_template, "url template")?;
            encoder.write_u16(context.stable_tag());
        }
        BindingDetails::ExchangePlan { event, context } => {
            encode_binding_component(encoder, event, "event")?;
            encoder.write_u16(context.stable_tag());
        }
    }
    Ok(())
}

fn encode_binding_component(
    encoder: &mut CanonicalEncoder,
    value: &str,
    field: &str,
) -> Result<(), DeterminismError> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        return Err(DeterminismError::InvalidComponent(format!(
            "binding {field} must not be blank"
        )));
    }
    encoder.write_string(value)
}

fn evidence_record_hex(record: &EvidenceRecord) -> Result<String, DeterminismError> {
    let mut encoder = CanonicalEncoder::new("evidence-record")?;
    encode_evidence_record(&mut encoder, record)?;
    Ok(encoder.finish())
}

pub(crate) fn evidence_record_digest(record: &EvidenceRecord) -> Result<String, DeterminismError> {
    Ok(format!("sha256:{}", evidence_record_hex(record)?))
}

pub(crate) fn evidence_id(record: &EvidenceRecord) -> Result<String, DeterminismError> {
    Ok(format!("ev_{}", evidence_record_hex(record)?))
}

impl ProviderOutcomeSnapshot {
    pub(crate) fn from_records(
        port: super::model::EvidencePort,
        name: &str,
        version: &str,
        coverage: super::model::Coverage,
        reason_code: Option<String>,
        records: &[EvidenceRecord],
    ) -> Result<Self, String> {
        let record_digests = records
            .iter()
            .map(evidence_record_digest)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Self::new(
            port,
            name,
            version,
            ProviderReadiness::Ready,
            coverage,
            reason_code,
            record_digests,
        )
    }
}

pub(crate) fn canonicalize_evidence(
    records: Vec<EvidenceRecord>,
) -> Result<Vec<Evidence>, DeterminismError> {
    canonicalize_evidence_by(records, evidence_id)
}

pub(crate) fn canonicalize_evidence_with_id<F>(
    records: Vec<EvidenceRecord>,
    mut identifier: F,
) -> Result<Vec<Evidence>, DeterminismError>
where
    F: FnMut(&EvidenceRecord) -> String,
{
    canonicalize_evidence_by(records, |record| Ok(identifier(record)))
}

fn canonicalize_evidence_by<F>(
    records: Vec<EvidenceRecord>,
    mut identifier: F,
) -> Result<Vec<Evidence>, DeterminismError>
where
    F: FnMut(&EvidenceRecord) -> Result<String, DeterminismError>,
{
    let mut by_id: BTreeMap<String, EvidenceRecord> = BTreeMap::new();
    for record in records {
        let id = identifier(&record)?;
        if let Some(existing) = by_id.get(&id) {
            if !records_are_byte_identical(existing, &record) {
                return Err(DeterminismError::IdentifierCollision { id });
            }
            continue;
        }
        by_id.insert(id, record);
    }
    Ok(by_id
        .into_iter()
        .map(|(id, record)| Evidence::from_record(id, record))
        .collect())
}

fn exact_artifact(left: &ArtifactRef, right: &ArtifactRef) -> bool {
    left.kind == right.kind && left.canonical_ref == right.canonical_ref
}

fn records_are_byte_identical(left: &EvidenceRecord, right: &EvidenceRecord) -> bool {
    provider_facts_are_byte_identical(&left.fact, &right.fact)
        && left.location == right.location
        && left.provider == right.provider
        && left.coverage == right.coverage
        && left.freshness == right.freshness
}

fn provider_facts_are_byte_identical(left: &ProviderFact, right: &ProviderFact) -> bool {
    match (left, right) {
        (
            ProviderFact::MetadataPresent { subject: left },
            ProviderFact::MetadataPresent { subject: right },
        )
        | (
            ProviderFact::MetadataAbsent { subject: left },
            ProviderFact::MetadataAbsent { subject: right },
        )
        | (
            ProviderFact::DefinitionAbsent { subject: left },
            ProviderFact::DefinitionAbsent { subject: right },
        ) => exact_artifact(left, right),
        (
            ProviderFact::CodeOccurrence {
                subject: left_subject,
                search_term: left_term,
            },
            ProviderFact::CodeOccurrence {
                subject: right_subject,
                search_term: right_term,
            },
        ) => exact_artifact(left_subject, right_subject) && left_term == right_term,
        (
            ProviderFact::DefinitionPresent {
                subject: left_subject,
                definition: left_definition,
            },
            ProviderFact::DefinitionPresent {
                subject: right_subject,
                definition: right_definition,
            },
        ) => exact_artifact(left_subject, right_subject) && left_definition == right_definition,
        (
            ProviderFact::Binding {
                subject: left_subject,
                object: left_object,
                relation: left_relation,
                details: left_details,
            },
            ProviderFact::Binding {
                subject: right_subject,
                object: right_object,
                relation: right_relation,
                details: right_details,
            },
        ) => {
            exact_artifact(left_subject, right_subject)
                && exact_artifact(left_object, right_object)
                && left_relation == right_relation
                && left_details == right_details
        }
        (
            ProviderFact::Call {
                subject: left_subject,
                object: left_object,
                resolution: left_resolution,
                call_type: left_call_type,
                context: left_context,
            },
            ProviderFact::Call {
                subject: right_subject,
                object: right_object,
                resolution: right_resolution,
                call_type: right_call_type,
                context: right_context,
            },
        ) => {
            exact_artifact(left_subject, right_subject)
                && exact_artifact(left_object, right_object)
                && left_resolution == right_resolution
                && left_call_type == right_call_type
                && left_context == right_context
        }
        (
            ProviderFact::PlatformCallback {
                subject: left_subject,
                object: left_object,
                callback: left_callback,
            },
            ProviderFact::PlatformCallback {
                subject: right_subject,
                object: right_object,
                callback: right_callback,
            },
        ) => {
            exact_artifact(left_subject, right_subject)
                && exact_artifact(left_object, right_object)
                && left_callback == right_callback
        }
        (
            ProviderFact::Support {
                subject: left_subject,
                state: left_state,
            },
            ProviderFact::Support {
                subject: right_subject,
                state: right_state,
            },
        ) => exact_artifact(left_subject, right_subject) && left_state == right_state,
        _ => false,
    }
}

fn validate_fingerprint(value: &str) -> Result<(), DeterminismError> {
    let Some(digest) = value.strip_prefix("sha256:") else {
        return Err(DeterminismError::InvalidComponent(
            "fingerprint must start with sha256:".to_string(),
        ));
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(DeterminismError::InvalidComponent(
            "fingerprint must contain 64 lowercase hexadecimal characters".to_string(),
        ));
    }
    Ok(())
}

fn sort_dedup(values: &mut Vec<String>) {
    values.sort();
    values.dedup();
}

fn cmp_artifact(left: &ArtifactRef, right: &ArtifactRef) -> Ordering {
    left.cmp(right)
        .then_with(|| left.canonical_ref.cmp(&right.canonical_ref))
}

fn cmp_optional_artifact(left: Option<&ArtifactRef>, right: Option<&ArtifactRef>) -> Ordering {
    match (left, right) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(left), Some(right)) => cmp_artifact(left, right),
    }
}

fn location_sort_key(
    location: Option<&super::model::SourceLocation>,
) -> (u8, String, Option<u32>, Option<u32>) {
    match location {
        Some(location) => (1, location.path.clone(), location.line, location.column),
        None => (0, String::new(), None, None),
    }
}

fn cmp_evidence(left: &Evidence, right: &Evidence) -> Ordering {
    left.id
        .cmp(&right.id)
        .then_with(|| {
            left.evidence_type
                .stable_tag()
                .cmp(&right.evidence_type.stable_tag())
        })
        .then_with(|| cmp_artifact(&left.subject, &right.subject))
        .then_with(|| left.fact_code.cmp(&right.fact_code))
        .then_with(|| cmp_optional_artifact(left.object.as_ref(), right.object.as_ref()))
        .then_with(|| {
            location_sort_key(left.location.as_ref())
                .cmp(&location_sort_key(right.location.as_ref()))
        })
        .then_with(|| {
            left.provider
                .port
                .stable_tag()
                .cmp(&right.provider.port.stable_tag())
        })
        .then_with(|| left.provider.name.cmp(&right.provider.name))
        .then_with(|| left.provider.version.cmp(&right.provider.version))
        .then_with(|| left.coverage.stable_tag().cmp(&right.coverage.stable_tag()))
        .then_with(|| left.freshness.source_set.cmp(&right.freshness.source_set))
        .then_with(|| {
            left.freshness
                .source_fingerprint
                .cmp(&right.freshness.source_fingerprint)
        })
        .then_with(|| {
            left.freshness
                .workspace_epoch
                .cmp(&right.freshness.workspace_epoch)
        })
}

fn cmp_check(left: &Check, right: &Check) -> Ordering {
    left.code
        .cmp(&right.code)
        .then_with(|| left.provider.cmp(&right.provider))
        .then_with(|| left.state.stable_tag().cmp(&right.state.stable_tag()))
        .then_with(|| left.outcome.stable_tag().cmp(&right.outcome.stable_tag()))
        .then_with(|| left.coverage.stable_tag().cmp(&right.coverage.stable_tag()))
        .then_with(|| left.severity.stable_tag().cmp(&right.severity.stable_tag()))
        .then_with(|| left.affects.cmp(&right.affects))
        .then_with(|| left.reason_code.cmp(&right.reason_code))
        .then_with(|| left.retryable.cmp(&right.retryable))
        .then_with(|| left.details.cmp(&right.details))
        .then_with(|| left.evidence_ids.cmp(&right.evidence_ids))
}

pub(crate) fn canonicalize_report(report: &mut DiscoveryReport) {
    report
        .source
        .linked_source_snapshots
        .sort_by_key(snapshot_sort_key);
    report.source.linked_source_snapshots.dedup();

    for related in &mut report.related_artifacts {
        sort_dedup(&mut related.reason_codes);
        sort_dedup(&mut related.evidence_ids);
    }
    report.related_artifacts.sort_by(|left, right| {
        cmp_artifact(&left.artifact, &right.artifact)
            .then_with(|| {
                left.evidence_level
                    .stable_tag()
                    .cmp(&right.evidence_level.stable_tag())
            })
            .then_with(|| left.reason_codes.cmp(&right.reason_codes))
            .then_with(|| left.evidence_ids.cmp(&right.evidence_ids))
    });
    report.related_artifacts.dedup_by(|left, right| {
        exact_artifact(&left.artifact, &right.artifact)
            && left.evidence_level == right.evidence_level
            && left.reason_codes == right.reason_codes
            && left.evidence_ids == right.evidence_ids
    });

    for edge in &mut report.flow_edges {
        sort_dedup(&mut edge.evidence_ids);
    }
    report.flow_edges.sort_by(|left, right| {
        cmp_artifact(&left.from, &right.from)
            .then_with(|| cmp_artifact(&left.to, &right.to))
            .then_with(|| left.kind.stable_tag().cmp(&right.kind.stable_tag()))
            .then_with(|| left.evidence_ids.cmp(&right.evidence_ids))
    });
    report.flow_edges.dedup_by(|left, right| {
        exact_artifact(&left.from, &right.from)
            && exact_artifact(&left.to, &right.to)
            && left.kind == right.kind
            && left.evidence_ids == right.evidence_ids
    });

    for candidate in &mut report.extension_point_candidates {
        sort_dedup(&mut candidate.reason_codes);
        sort_dedup(&mut candidate.evidence_ids);
        sort_dedup(&mut candidate.blockers);
    }
    report.extension_point_candidates.sort_by(|left, right| {
        cmp_artifact(&left.target, &right.target)
            .then_with(|| {
                left.evidence_level
                    .stable_tag()
                    .cmp(&right.evidence_level.stable_tag())
            })
            .then_with(|| {
                left.support_state
                    .stable_tag()
                    .cmp(&right.support_state.stable_tag())
            })
            .then_with(|| left.reason_codes.cmp(&right.reason_codes))
            .then_with(|| left.evidence_ids.cmp(&right.evidence_ids))
            .then_with(|| left.blockers.cmp(&right.blockers))
    });
    report.extension_point_candidates.dedup_by(|left, right| {
        exact_artifact(&left.target, &right.target)
            && left.evidence_level == right.evidence_level
            && left.support_state == right.support_state
            && left.reason_codes == right.reason_codes
            && left.evidence_ids == right.evidence_ids
            && left.blockers == right.blockers
    });

    for verdict in &mut report.proposal_verdicts {
        sort_dedup(&mut verdict.evidence_ids);
        sort_dedup(&mut verdict.coverage_gaps);
        sort_dedup(&mut verdict.blockers);
    }
    report.proposal_verdicts.sort_by(|left, right| {
        left.proposal_id
            .cmp(&right.proposal_id)
            .then_with(|| left.verdict.stable_tag().cmp(&right.verdict.stable_tag()))
            .then_with(|| {
                left.facts
                    .exists
                    .stable_tag()
                    .cmp(&right.facts.exists.stable_tag())
            })
            .then_with(|| {
                left.facts
                    .runtime_reachable
                    .stable_tag()
                    .cmp(&right.facts.runtime_reachable.stable_tag())
            })
            .then_with(|| {
                left.facts
                    .support
                    .stable_tag()
                    .cmp(&right.facts.support.stable_tag())
            })
            .then_with(|| left.evidence_ids.cmp(&right.evidence_ids))
            .then_with(|| left.coverage_gaps.cmp(&right.coverage_gaps))
            .then_with(|| left.blockers.cmp(&right.blockers))
    });
    report.proposal_verdicts.dedup();

    report.evidence.sort_by(cmp_evidence);
    report
        .evidence
        .dedup_by(|left, right| left.is_byte_identical(right));

    for check in &mut report.checks {
        sort_dedup(&mut check.affects);
        sort_dedup(&mut check.details);
        sort_dedup(&mut check.evidence_ids);
    }
    report.checks.sort_by(cmp_check);
    report.checks.dedup();
    sort_dedup(&mut report.receipt_eligibility.blockers);
}

#[cfg(test)]
pub(crate) fn canonical_golden_vector() -> String {
    let mut encoder = CanonicalEncoder::new("golden-vector").expect("fixed domain is valid");
    encoder.write_u16(7);
    encoder
        .write_string("Привет")
        .expect("fixed string length is representable");
    encoder.write_bool(true);
    encoder
        .write_optional_string(None)
        .expect("fixed optional string is representable");
    encoder.finish()
}

#[cfg(test)]
pub(crate) fn assert_unique_stable_tags() {
    fn unique(tags: &[u16]) {
        assert_eq!(
            tags.iter().copied().collect::<BTreeSet<_>>().len(),
            tags.len()
        );
        assert!(tags.iter().all(|tag| *tag != 0));
    }

    unique(
        &super::contract::ArtifactKind::ALL
            .iter()
            .map(|value| value.stable_tag())
            .collect::<Vec<_>>(),
    );
    unique(
        &super::model::EvidencePort::ALL
            .iter()
            .map(|value| value.stable_tag())
            .collect::<Vec<_>>(),
    );
    unique(&[
        super::contract::DiscoverMode::Explore.stable_tag(),
        super::contract::DiscoverMode::Validate.stable_tag(),
    ]);
    unique(&[
        super::contract::ExecutionContext::Server.stable_tag(),
        super::contract::ExecutionContext::Client.stable_tag(),
        super::contract::ExecutionContext::ServerWithoutContext.stable_tag(),
        super::contract::ExecutionContext::ClientAndServerWithoutContext.stable_tag(),
    ]);
    unique(&[
        super::contract::InterceptorType::Before.stable_tag(),
        super::contract::InterceptorType::After.stable_tag(),
        super::contract::InterceptorType::ModificationAndControl.stable_tag(),
    ]);
    unique(&[
        super::model::Coverage::Complete.stable_tag(),
        super::model::Coverage::Bounded.stable_tag(),
        super::model::Coverage::Unknown.stable_tag(),
    ]);
    unique(&[
        super::model::ProviderReadiness::Ready.stable_tag(),
        super::model::ProviderReadiness::Unavailable.stable_tag(),
        super::model::ProviderReadiness::Failed.stable_tag(),
    ]);
    unique(&[
        super::model::EvidenceType::Metadata.stable_tag(),
        super::model::EvidenceType::CodeOccurrence.stable_tag(),
        super::model::EvidenceType::Definition.stable_tag(),
        super::model::EvidenceType::Binding.stable_tag(),
        super::model::EvidenceType::Call.stable_tag(),
        super::model::EvidenceType::PlatformCallback.stable_tag(),
        super::model::EvidenceType::Support.stable_tag(),
    ]);
    unique(&[
        source_format_tag(SourceFormat::PlatformXml),
        source_format_tag(SourceFormat::Edt),
        source_format_tag(SourceFormat::Unknown),
        source_format_tag(SourceFormat::Invalid),
    ]);
    unique(&[
        super::model::SourceSnapshotRole::Analysis.stable_tag(),
        super::model::SourceSnapshotRole::Mutation.stable_tag(),
    ]);
    unique(&[
        super::model::CheckState::Passed.stable_tag(),
        super::model::CheckState::Unavailable.stable_tag(),
        super::model::CheckState::Failed.stable_tag(),
        super::model::CheckState::Skipped.stable_tag(),
    ]);
    unique(&[
        super::model::CheckOutcome::Satisfied.stable_tag(),
        super::model::CheckOutcome::NoMatch.stable_tag(),
        super::model::CheckOutcome::Inconclusive.stable_tag(),
        super::model::CheckOutcome::Conflict.stable_tag(),
        super::model::CheckOutcome::NotApplicable.stable_tag(),
    ]);
    unique(&[
        super::model::CheckSeverity::Info.stable_tag(),
        super::model::CheckSeverity::Warning.stable_tag(),
        super::model::CheckSeverity::Blocking.stable_tag(),
    ]);
    unique(&[
        super::model::FlowKind::Contains.stable_tag(),
        super::model::FlowKind::Defines.stable_tag(),
        super::model::FlowKind::Calls.stable_tag(),
        super::model::FlowKind::Handles.stable_tag(),
        super::model::FlowKind::Subscribes.stable_tag(),
        super::model::FlowKind::Uses.stable_tag(),
    ]);
    unique(&[
        super::model::CallResolution::Resolved.stable_tag(),
        super::model::CallResolution::Dynamic.stable_tag(),
        super::model::CallResolution::Ambiguous.stable_tag(),
        super::model::CallResolution::Unresolved.stable_tag(),
    ]);
    unique(&[
        super::model::CallType::Direct.stable_tag(),
        super::model::CallType::Method.stable_tag(),
        super::model::CallType::Callback.stable_tag(),
        super::model::CallType::Dynamic.stable_tag(),
    ]);
    unique(&[
        super::model::HttpVerb::Get.stable_tag(),
        super::model::HttpVerb::Post.stable_tag(),
        super::model::HttpVerb::Put.stable_tag(),
        super::model::HttpVerb::Patch.stable_tag(),
        super::model::HttpVerb::Delete.stable_tag(),
        super::model::HttpVerb::Head.stable_tag(),
        super::model::HttpVerb::Options.stable_tag(),
    ]);
    unique(&super::model::BindingDetails::VARIANT_STABLE_TAGS);
    unique(&[
        super::model::DiscoveryStatus::Complete.stable_tag(),
        super::model::DiscoveryStatus::Partial.stable_tag(),
        super::model::DiscoveryStatus::Insufficient.stable_tag(),
    ]);
    unique(&[
        super::model::EvidenceLevel::Lexical.stable_tag(),
        super::model::EvidenceLevel::Observed.stable_tag(),
        super::model::EvidenceLevel::Connected.stable_tag(),
        super::model::EvidenceLevel::Actionable.stable_tag(),
    ]);
    unique(&[
        super::model::SupportState::Editable.stable_tag(),
        super::model::SupportState::Locked.stable_tag(),
        super::model::SupportState::ConfigurationReadOnly.stable_tag(),
        super::model::SupportState::Removed.stable_tag(),
        super::model::SupportState::NotUnderSupport.stable_tag(),
        super::model::SupportState::ExtensionOwned.stable_tag(),
        super::model::SupportState::ExtensionRequired.stable_tag(),
        super::model::SupportState::Unknown.stable_tag(),
    ]);
    unique(&[
        super::model::Verdict::Supported.stable_tag(),
        super::model::Verdict::Contradicted.stable_tag(),
        super::model::Verdict::Unknown.stable_tag(),
    ]);
    unique(&[
        super::model::FactAnswer::Yes.stable_tag(),
        super::model::FactAnswer::No.stable_tag(),
        super::model::FactAnswer::Unknown.stable_tag(),
    ]);
    unique(&super::model::ProviderFact::VARIANT_STABLE_TAGS);

    // These exact pairs are part of the hash protocol. Uniqueness alone would
    // not catch a renumbering that silently changes persisted identifiers.
    assert_eq!(
        super::contract::ArtifactKind::ALL.map(StableTag::stable_tag),
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
    );
    assert_eq!(
        super::model::EvidencePort::ALL.map(StableTag::stable_tag),
        [1, 2, 3, 4, 5, 6]
    );
    assert_eq!(
        [
            super::contract::DiscoverMode::Explore.stable_tag(),
            super::contract::DiscoverMode::Validate.stable_tag(),
        ],
        [1, 2]
    );
    assert_eq!(
        [
            super::contract::ExecutionContext::Server.stable_tag(),
            super::contract::ExecutionContext::Client.stable_tag(),
            super::contract::ExecutionContext::ServerWithoutContext.stable_tag(),
            super::contract::ExecutionContext::ClientAndServerWithoutContext.stable_tag(),
        ],
        [1, 2, 3, 4]
    );
    assert_eq!(
        [
            super::contract::InterceptorType::Before.stable_tag(),
            super::contract::InterceptorType::After.stable_tag(),
            super::contract::InterceptorType::ModificationAndControl.stable_tag(),
        ],
        [1, 2, 3]
    );
    assert_eq!(
        [
            super::model::Coverage::Complete.stable_tag(),
            super::model::Coverage::Bounded.stable_tag(),
            super::model::Coverage::Unknown.stable_tag(),
        ],
        [1, 2, 3]
    );
    assert_eq!(
        [
            super::model::ProviderReadiness::Ready.stable_tag(),
            super::model::ProviderReadiness::Unavailable.stable_tag(),
            super::model::ProviderReadiness::Failed.stable_tag(),
        ],
        [1, 2, 3]
    );
    assert_eq!(
        [
            super::model::EvidenceType::Metadata.stable_tag(),
            super::model::EvidenceType::CodeOccurrence.stable_tag(),
            super::model::EvidenceType::Definition.stable_tag(),
            super::model::EvidenceType::Binding.stable_tag(),
            super::model::EvidenceType::Call.stable_tag(),
            super::model::EvidenceType::PlatformCallback.stable_tag(),
            super::model::EvidenceType::Support.stable_tag(),
        ],
        [1, 2, 3, 4, 5, 6, 7]
    );
    assert_eq!(
        [
            source_format_tag(SourceFormat::PlatformXml),
            source_format_tag(SourceFormat::Edt),
            source_format_tag(SourceFormat::Unknown),
            source_format_tag(SourceFormat::Invalid),
        ],
        [1, 2, 3, 4]
    );
    assert_eq!(
        [
            super::model::SourceSnapshotRole::Analysis.stable_tag(),
            super::model::SourceSnapshotRole::Mutation.stable_tag(),
        ],
        [1, 2]
    );
    assert_eq!(
        [
            super::model::CheckState::Passed.stable_tag(),
            super::model::CheckState::Unavailable.stable_tag(),
            super::model::CheckState::Failed.stable_tag(),
            super::model::CheckState::Skipped.stable_tag(),
        ],
        [1, 2, 3, 4]
    );
    assert_eq!(
        [
            super::model::CheckOutcome::Satisfied.stable_tag(),
            super::model::CheckOutcome::NoMatch.stable_tag(),
            super::model::CheckOutcome::Inconclusive.stable_tag(),
            super::model::CheckOutcome::Conflict.stable_tag(),
            super::model::CheckOutcome::NotApplicable.stable_tag(),
        ],
        [1, 2, 3, 4, 5]
    );
    assert_eq!(
        [
            super::model::CheckSeverity::Info.stable_tag(),
            super::model::CheckSeverity::Warning.stable_tag(),
            super::model::CheckSeverity::Blocking.stable_tag(),
        ],
        [1, 2, 3]
    );
    assert_eq!(
        [
            super::model::FlowKind::Contains.stable_tag(),
            super::model::FlowKind::Defines.stable_tag(),
            super::model::FlowKind::Calls.stable_tag(),
            super::model::FlowKind::Handles.stable_tag(),
            super::model::FlowKind::Subscribes.stable_tag(),
            super::model::FlowKind::Uses.stable_tag(),
        ],
        [1, 2, 3, 4, 5, 6]
    );
    assert_eq!(
        [
            super::model::CallResolution::Resolved.stable_tag(),
            super::model::CallResolution::Dynamic.stable_tag(),
            super::model::CallResolution::Ambiguous.stable_tag(),
            super::model::CallResolution::Unresolved.stable_tag(),
        ],
        [1, 2, 3, 4]
    );
    assert_eq!(
        [
            super::model::CallType::Direct.stable_tag(),
            super::model::CallType::Method.stable_tag(),
            super::model::CallType::Callback.stable_tag(),
            super::model::CallType::Dynamic.stable_tag(),
        ],
        [1, 2, 3, 4]
    );
    assert_eq!(
        [
            super::model::HttpVerb::Get.stable_tag(),
            super::model::HttpVerb::Post.stable_tag(),
            super::model::HttpVerb::Put.stable_tag(),
            super::model::HttpVerb::Patch.stable_tag(),
            super::model::HttpVerb::Delete.stable_tag(),
            super::model::HttpVerb::Head.stable_tag(),
            super::model::HttpVerb::Options.stable_tag(),
        ],
        [1, 2, 3, 4, 5, 6, 7]
    );
    assert_eq!(
        super::model::BindingDetails::VARIANT_STABLE_TAGS,
        [1, 2, 3, 4, 5, 6, 7]
    );
    assert_eq!(
        [
            super::model::DiscoveryStatus::Complete.stable_tag(),
            super::model::DiscoveryStatus::Partial.stable_tag(),
            super::model::DiscoveryStatus::Insufficient.stable_tag(),
        ],
        [1, 2, 3]
    );
    assert_eq!(
        [
            super::model::EvidenceLevel::Lexical.stable_tag(),
            super::model::EvidenceLevel::Observed.stable_tag(),
            super::model::EvidenceLevel::Connected.stable_tag(),
            super::model::EvidenceLevel::Actionable.stable_tag(),
        ],
        [1, 2, 3, 4]
    );
    assert_eq!(
        [
            super::model::SupportState::Editable.stable_tag(),
            super::model::SupportState::Locked.stable_tag(),
            super::model::SupportState::ConfigurationReadOnly.stable_tag(),
            super::model::SupportState::Removed.stable_tag(),
            super::model::SupportState::NotUnderSupport.stable_tag(),
            super::model::SupportState::ExtensionOwned.stable_tag(),
            super::model::SupportState::ExtensionRequired.stable_tag(),
            super::model::SupportState::Unknown.stable_tag(),
        ],
        [1, 2, 3, 4, 5, 6, 7, 8]
    );
    assert_eq!(
        [
            super::model::Verdict::Supported.stable_tag(),
            super::model::Verdict::Contradicted.stable_tag(),
            super::model::Verdict::Unknown.stable_tag(),
        ],
        [1, 2, 3]
    );
    assert_eq!(
        [
            super::model::FactAnswer::Yes.stable_tag(),
            super::model::FactAnswer::No.stable_tag(),
            super::model::FactAnswer::Unknown.stable_tag(),
        ],
        [1, 2, 3]
    );
    assert_eq!(
        super::model::ProviderFact::VARIANT_STABLE_TAGS,
        [1, 2, 3, 4, 5, 6, 7, 8, 9]
    );
}
