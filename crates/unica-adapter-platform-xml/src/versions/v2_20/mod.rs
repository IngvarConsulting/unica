#![allow(dead_code)]

use std::{any::Any, collections::BTreeSet};

use unica_format_core::{
    navigation::NavigationEnvelope,
    ports::{CaptureResult, CapturedSource, CapturedSourceSession, ProbeResult},
    source::{
        source_id_for_configured_source_set, AdapterManifest, AdapterMaturity, FormatRange,
        FormatVersion, SnapshotConsistency, SourceAccess, SourceAdapterError,
        SourceAdapterErrorKind, SourceBinding, SourceContext, SourceFamily, SourceSnapshot,
    },
};

pub(crate) mod decoder;
pub(crate) mod native_model;
pub(crate) mod probe;
pub(crate) mod profile;
pub(crate) mod projector;
pub(crate) mod provider;
pub(crate) mod schema;
pub(crate) mod semantic_map;
pub(crate) mod support;
pub(crate) mod xml;

pub(crate) type ProbeOutcome = ProbeResult;

pub(crate) struct SourceInput {
    pub(crate) workspace_root: std::path::PathBuf,
    pub(crate) source_root: std::path::PathBuf,
    pub(crate) target: std::path::PathBuf,
    pub(crate) configured_source_set: Option<String>,
    pub(crate) declared_family: SourceFamily,
    pub(crate) declared_format: Option<FormatVersion>,
}

pub(crate) struct PlatformXmlReadAdapter;

struct PlatformXmlCapturedSession {
    source: SourceContext,
    snapshot: SourceSnapshot,
    binding: SourceBinding,
    provider: provider::PlatformXmlProvider,
}

impl CapturedSourceSession for PlatformXmlCapturedSession {
    fn source(&self) -> &SourceContext {
        &self.source
    }

    fn snapshot(&self) -> &SourceSnapshot {
        &self.snapshot
    }

    fn binding(&self) -> &SourceBinding {
        &self.binding
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl PlatformXmlReadAdapter {
    pub(crate) const fn new() -> Self {
        Self
    }

    pub(crate) fn inspect_provider(
        &self,
        provider: &provider::PlatformXmlProvider,
        descriptor: &unica_format_core::source::SourceDescriptor,
    ) -> Result<NavigationEnvelope, SourceAdapterError> {
        let native = decoder::decode(provider, descriptor)?;
        let support_bytes = provider.parent_configurations_bytes();
        let support = match support_bytes.as_deref() {
            None => support::read_support_facts_bytes(None),
            Some(bytes) => match provider.configuration_uuid() {
                Ok(configuration_uuid) => support::read_support_facts_bytes_for_configuration(
                    Some(bytes),
                    &configuration_uuid,
                ),
                Err(_) => support::unreadable_configuration_evidence(),
            },
        };
        projector::project(&native, &support)
    }
}

pub(crate) fn manifest() -> AdapterManifest {
    AdapterManifest {
        adapter_id: "platform-xml-2.20",
        adapter_version: env!("CARGO_PKG_VERSION"),
        source_family: SourceFamily::PlatformXml,
        supported_formats: vec![FormatRange::exact(
            FormatVersion::parse("2.20").expect("constant version"),
        )],
        required_features: BTreeSet::new(),
        excluded_features: BTreeSet::new(),
        source_access: SourceAccess::ReadOnly,
        maturity: AdapterMaturity::ReadCompatible,
    }
}

fn capture_provider(
    source: &SourceContext,
) -> Result<(provider::PlatformXmlProvider, SourceBinding), SourceAdapterError> {
    if source.declared_family() != &SourceFamily::PlatformXml {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::SourceUnavailable,
            "Platform XML adapter cannot capture a different source family",
        ));
    }
    let provider = provider::PlatformXmlProvider::capture(
        source.location().target(),
        source.location().source_root(),
    )?;
    let source_id =
        source_id_for_configured_source_set(source.configured_source_set().ok_or_else(|| {
            SourceAdapterError::new(
                SourceAdapterErrorKind::SourceUnavailable,
                "Platform XML capture requires a configured source-set identity",
            )
        })?)?;
    let binding = SourceBinding::new(
        source_id,
        SourceFamily::PlatformXml,
        source.declared_format().cloned(),
        provider.target_identity().clone(),
        provider.revision()?,
    );
    Ok((provider, binding))
}

pub(crate) fn capture(source: &SourceContext) -> Result<CaptureResult, SourceAdapterError> {
    if source.declared_family() != &SourceFamily::PlatformXml {
        return Ok(CaptureResult::NoMatch);
    }
    let (provider, binding) = capture_provider(source)?;
    let snapshot = SourceSnapshot {
        source_id: binding.source_id,
        revision: binding.revision,
        consistency: SnapshotConsistency::Consistent,
        adapter_id: manifest().adapter_id.to_string(),
    };
    let binding = SourceBinding::new(
        snapshot.source_id.clone(),
        SourceFamily::PlatformXml,
        source.declared_format().cloned(),
        provider.target_identity().clone(),
        snapshot.revision.clone(),
    );
    Ok(CaptureResult::Captured(CapturedSource::new(
        PlatformXmlCapturedSession {
            source: source.clone(),
            snapshot,
            binding,
            provider,
        },
    )))
}

pub(crate) fn probe(captured: &CapturedSource) -> Result<ProbeResult, SourceAdapterError> {
    let session = captured_session(captured)?;
    probe::probe_provider(
        &session.provider,
        session.provider.descriptor_key(),
        &session.binding,
    )
}

pub(crate) fn read(captured: &CapturedSource) -> Result<NavigationEnvelope, SourceAdapterError> {
    let session = captured_session(captured)?;
    if session.snapshot.adapter_id != manifest().adapter_id
        || session.snapshot.source_id != session.binding.source_id
        || session.snapshot.revision != session.binding.revision
    {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::SnapshotStale,
            "Platform XML captured session binding is inconsistent",
        ));
    }
    let descriptor = match probe::probe_provider(
        &session.provider,
        session.provider.descriptor_key(),
        &session.binding,
    )? {
        ProbeResult::Match(descriptor) => descriptor,
        ProbeResult::NoMatch => {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::FormatUnsupported,
                "source is not Platform XML",
            ));
        }
    };
    if descriptor.format_version.to_string() != "2.20" {
        return Ok(NavigationEnvelope::unavailable(SourceAdapterError::new(
            SourceAdapterErrorKind::FormatUnsupported,
            format!(
                "no reader supports PlatformXml format {}",
                descriptor.format_version
            ),
        )));
    }
    let native = decoder::decode(&session.provider, &descriptor)?;
    let support_bytes = session.provider.parent_configurations_bytes();
    let support = match support_bytes.as_deref() {
        None => support::read_support_facts_bytes(None),
        Some(bytes) => match session.provider.configuration_uuid() {
            Ok(configuration_uuid) => support::read_support_facts_bytes_for_configuration(
                Some(bytes),
                &configuration_uuid,
            ),
            Err(_) => support::unreadable_configuration_evidence(),
        },
    };
    projector::project(&native, &support)
}

fn captured_session(
    captured: &CapturedSource,
) -> Result<&PlatformXmlCapturedSession, SourceAdapterError> {
    captured
        .adapter_state::<PlatformXmlCapturedSession>()
        .ok_or_else(|| {
            SourceAdapterError::new(
                SourceAdapterErrorKind::SourceUnavailable,
                "Platform XML adapter received another adapter's captured session",
            )
        })
}

pub(crate) const fn metadata_classes() -> &'static [&'static str] {
    schema::LEGACY_TOP_LEVEL_METADATA_CLASSES
}

pub(crate) const PLATFORM_LINE: &str = profile::ACTIVE_FORMAT_PROFILE.platform_line;
pub(crate) const EXPORT_FORMAT: &str = profile::ACTIVE_FORMAT_PROFILE.export_format;

pub(crate) fn support_decision(
    bin_path: &std::path::Path,
    object_uuid: &str,
) -> Result<(&'static str, unica_format_core::navigation::Authorability), (String, Option<usize>)> {
    let facts = support::read_support_facts(bin_path);
    let effective = facts.effective_rule_for(object_uuid);
    if let Some(error) = facts.parse_error() {
        return Err((error.context.to_string(), error.offset));
    }
    let label = match effective {
        support::EffectiveSupportRule::Absent => "absent",
        support::EffectiveSupportRule::Removed => "removed",
        support::EffectiveSupportRule::Editable => "editable",
        support::EffectiveSupportRule::Locked => "locked",
        support::EffectiveSupportRule::ConfigurationReadOnly => "configuration_read_only",
        support::EffectiveSupportRule::UnknownReadOnly => "unknown_read_only",
        support::EffectiveSupportRule::Unreadable => {
            return Err(("ParentConfigurations.bin".to_string(), None));
        }
    };
    Ok((label, facts.authorability_for(object_uuid)))
}

pub(crate) fn support_summary_lines(bin_path: &std::path::Path, is_extension: bool) -> Vec<String> {
    let facts = support::read_support_facts(bin_path);
    match facts.source {
        support::SupportSourceState::Absent => {
            return vec![if is_extension {
                "Поддержка:      расширение (CFE), правки свободны".to_string()
            } else {
                "Поддержка:      не на поддержке (своя конфигурация)".to_string()
            }];
        }
        support::SupportSourceState::Unreadable { .. } => {
            return vec![
                "Поддержка:      состояние ParentConfigurations.bin не удалось прочитать — правки не подтверждены"
                    .to_string(),
            ];
        }
        support::SupportSourceState::Removed => {
            return vec!["Поддержка:      снята с поддержки полностью".to_string()];
        }
        support::SupportSourceState::Parsed => {}
    }
    if facts.global_editing_enabled() == Some(false) {
        return vec![
            "Поддержка:      на поддержке".to_string(),
            "  Возможность изменения: выключена — вся конфигурация read-only (правки заблокированы)"
                .to_string(),
            format!("  Конфигураций поставщика: {}", facts.vendors().len()),
        ];
    }
    let counts = facts.rule_counts();
    let mut lines = vec![
        "Поддержка:      на поддержке".to_string(),
        "  Возможность изменения: включена".to_string(),
        format!(
            "  Объектов: на замке {} / редактируется {} / снято {}",
            counts[0], counts[1], counts[2]
        ),
        format!("  Конфигураций поставщика: {}", facts.vendors().len()),
    ];
    if facts.vendors().len() > 1 {
        for vendor in facts.vendors() {
            lines.push(format!(
                "  Поставщик: {} — {} {}",
                vendor.vendor, vendor.name, vendor.version
            ));
        }
    }
    lines
}

pub(crate) fn support_status(bin_path: &std::path::Path, object_uuid: &str) -> String {
    match support_decision(bin_path, object_uuid) {
        Ok(("absent", _)) => "не на поддержке".to_string(),
        Ok(("removed", _)) => "снято с поддержки (правки свободны)".to_string(),
        Ok(("editable", _)) => "редактируется с сохранением поддержки".to_string(),
        Ok(("locked", _)) => "на замке — прямая правка сломает обновления; дорабатывай через cfe-* либо включи редактирование объекта".to_string(),
        Ok(("configuration_read_only", _)) => "конфигурация read-only (возможность изменения выключена) — правки невозможны без включения".to_string(),
        Ok(("unknown_read_only", _)) => "состояние нескольких поставщиков нельзя однозначно применить — правки не подтверждены".to_string(),
        Ok(_) | Err(_) => "состояние поддержки не удалось прочитать — правки не подтверждены".to_string(),
    }
}

pub(crate) fn support_header(text: &str) -> Option<(u8, usize)> {
    let facts = support::read_support_facts_bytes(Some(text.as_bytes()));
    if !matches!(facts.source, support::SupportSourceState::Parsed) {
        return None;
    }
    let global_flag = if facts.global_editing_enabled()? {
        0
    } else {
        1
    };
    Some((global_flag, facts.vendors().len()))
}
