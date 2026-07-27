use unica_format_core::{
    ports::{
        EffectiveSupportRule, FormatCompatibility, FormatInspectionMode,
        FormatInspectionRequest, FormatInspectionResult, SupportEvidence,
        SupportInspectionRequest, SupportSourceState, SupportVendorEvidence,
    },
    source::{FormatVersion, SourceAdapterError, SourceAdapterErrorKind, SourceFamily},
};

use crate::safe_root::{ArtifactReadLimit, SafeRootError, SafeSourceRoot};

use super::{profile, support, EXPORT_FORMAT};

pub(crate) fn inspect_format(
    request: &FormatInspectionRequest,
) -> Result<FormatInspectionResult, SourceAdapterError> {
    let raw = read_target(&request.source)?;
    let source = std::str::from_utf8(&raw).map_err(|_| malformed())?;
    let source = source.trim_start_matches('\u{feff}');
    let document = roxmltree::Document::parse(source).map_err(|_| malformed())?;
    let root = document.root_element();
    let version = root
        .attributes()
        .find(|attribute| attribute.namespace().is_none() && attribute.name() == "version")
        .and_then(|attribute| source.get(attribute.range_value()));
    match request.mode {
        FormatInspectionMode::Versionless if version.is_some() => Err(malformed()),
        FormatInspectionMode::Versionless => Ok(FormatInspectionResult {
            compatibility: None,
        }),
        FormatInspectionMode::Versioned => {
            let compatibility =
                profile::classify_root_version(version).map_err(|_| malformed())?;
            let actual = FormatVersion::parse(&compatibility.actual().to_string())?;
            let target = FormatVersion::parse(EXPORT_FORMAT)?;
            Ok(FormatInspectionResult {
                compatibility: Some(match compatibility {
                    profile::FormatCompatibility::Older { .. } => {
                        FormatCompatibility::Older { actual, target }
                    }
                    profile::FormatCompatibility::Supported { .. } => {
                        FormatCompatibility::Supported { actual, target }
                    }
                    profile::FormatCompatibility::Newer { .. } => {
                        FormatCompatibility::Newer { actual, target }
                    }
                }),
            })
        }
    }
}

pub(crate) fn inspect_support(
    request: &SupportInspectionRequest,
) -> Result<SupportEvidence, SourceAdapterError> {
    let root = authorized_root(&request.source)?;
    let facts = match root.read_relative(
        "Ext/ParentConfigurations.bin",
        ArtifactReadLimit::SupportEvidence,
    ) {
        Ok(read) => support::read_support_facts_bytes(Some(read.bytes())),
        Err(SafeRootError::Missing) => support::read_support_facts_bytes(None),
        Err(_) => support::read_support_facts_bytes(Some(b"<unreadable")),
    };
    let object = request
        .object
        .as_ref()
        .map(|object| object.as_str())
        .unwrap_or("");
    Ok(support_evidence(facts, object))
}

fn read_target(
    source: &unica_format_core::source::SourceContext,
) -> Result<Vec<u8>, SourceAdapterError> {
    let root = authorized_root(source)?;
    let target = root
        .bind_target(source.location().target(), false)
        .map_err(source_error)?;
    root.read_bound(&target, ArtifactReadLimit::Descriptor)
        .map(|read| read.into_bytes())
        .map_err(source_error)
}

fn authorized_root(
    source: &unica_format_core::source::SourceContext,
) -> Result<SafeSourceRoot, SourceAdapterError> {
    if source.declared_family() != &SourceFamily::PlatformXml {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::SourceUnavailable,
            "source family is incompatible",
        ));
    }
    SafeSourceRoot::capture(
        source.location().workspace_root(),
        source.location().source_root(),
    )
    .map_err(source_error)
}

fn support_evidence(facts: support::SupportFacts, object_uuid: &str) -> SupportEvidence {
    use support::{EffectiveSupportRule as NativeRule, SupportSourceState as NativeState};
    let effective = facts.effective_rule_for(object_uuid);
    let source = match &facts.source {
        NativeState::Absent => SupportSourceState::Absent,
        NativeState::Removed => SupportSourceState::Removed,
        NativeState::Parsed => SupportSourceState::Parsed,
        NativeState::Unreadable { error } => SupportSourceState::Unreadable {
            context: error.context.to_string(),
            offset: error.offset,
        },
    };
    SupportEvidence {
        source,
        effective_rule: match effective {
            NativeRule::Absent => EffectiveSupportRule::Absent,
            NativeRule::Removed => EffectiveSupportRule::Removed,
            NativeRule::Editable => EffectiveSupportRule::Editable,
            NativeRule::Locked => EffectiveSupportRule::Locked,
            NativeRule::ConfigurationReadOnly => EffectiveSupportRule::ConfigurationReadOnly,
            NativeRule::UnknownReadOnly => EffectiveSupportRule::UnknownReadOnly,
            NativeRule::Unreadable => EffectiveSupportRule::Unreadable,
        },
        authorability: facts.authorability_for(object_uuid),
        global_editing_enabled: facts.global_editing_enabled(),
        rule_counts: facts.rule_counts(),
        vendors: facts
            .vendors()
            .iter()
            .map(|vendor| SupportVendorEvidence {
                version: vendor.version.clone(),
                vendor: vendor.vendor.clone(),
                name: vendor.name.clone(),
            })
            .collect(),
    }
}

fn source_error(_error: SafeRootError) -> SourceAdapterError {
    SourceAdapterError::new(
        SourceAdapterErrorKind::SourceUnavailable,
        "authorized source artifact is unavailable",
    )
}

fn malformed() -> SourceAdapterError {
    SourceAdapterError::new(
        SourceAdapterErrorKind::DecodeCorrupted,
        "source revision evidence is malformed",
    )
}
