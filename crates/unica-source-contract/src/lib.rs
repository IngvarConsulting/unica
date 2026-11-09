//! Source-set identity and observed profile facts shared by component owners.
//! Format probing, storage layout and semantic capability admission live elsewhere.

pub mod context;
pub mod route;

use serde::Serialize;
/// The canonical id of the Designer export profile 8.3.27 / 2.20.
pub const PLATFORM_XML_8_3_27_FORMAT_2_20: &str = "platform-xml-8.3.27-format-2.20";

/// Built-in script language, independent of multilingual presentation values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ScriptVariant {
    #[default]
    Russian,
    English,
}

pub const ACTIVE_PLATFORM_LINE: &str = "8.3.27";
pub const ACTIVE_SERIALIZATION_FORMAT: &str = "2.20";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSourceMap {
    pub workspace_root: String,
    pub config_path: Option<String>,
    pub source_sets: Vec<ProjectSourceSet>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_source_set: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_source_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_selection_error: Option<String>,
    #[serde(skip_serializing)]
    pub configured_format_raw: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSourceSet {
    pub name: String,
    pub kind: SourceSetKind,
    pub path: String,
    pub source_format: SourceFormat,
    pub source_state: SourceSetState,
    pub format_evidence: Vec<String>,
    #[serde(skip_serializing)]
    pub format_probe_error: Option<String>,
}

/// Что в наборе исходников на самом деле: три состояния, которые читатель
/// обязан различать без догадок.
///
/// Различие уже лежало в данных, но названо не было: у наполненного набора
/// доказательство формата указывает на файл, у пустого — на объявление в
/// `v8project.yaml`. Первое наблюдено, второе лишь заявлено, и по виду
/// доказательства читателю приходилось это выводить.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceSetState {
    /// Формат наблюдён в файлах и поддерживается — с набором работаем.
    Supported,
    /// Формат наблюдён и не поддерживается — с набором не работаем.
    Unsupported,
    /// Набор объявлен, но каталог пуст: формат лишь заявлен. Такой набор
    /// надо наполнить каркасом, и действующим он быть не может.
    Declared,
}

impl SourceSetState {
    /// Наблюдался ли формат в файлах, а не взят из объявления.
    pub const fn is_observed(self) -> bool {
        matches!(self, Self::Supported | Self::Unsupported)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceSetKind {
    Configuration,
    Extension,
    ExternalProcessor,
    ExternalReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    PlatformXml,
    Edt,
    Unknown,
    Invalid,
}

impl SourceSetKind {
    pub const fn stable_discriminant(self) -> u8 {
        match self {
            Self::Configuration => 1,
            Self::Extension => 2,
            Self::ExternalProcessor => 3,
            Self::ExternalReport => 4,
        }
    }
}

impl SourceFormat {
    /// The format family this observed format names, by its wire name;
    /// `None` for a set whose format is unknown or contradictory.
    pub const fn family_id(self) -> Option<format_identity::FormatFamilyId> {
        match self {
            Self::PlatformXml => Some(format_identity::FormatFamilyId::new("platform_xml")),
            Self::Edt => Some(format_identity::FormatFamilyId::new("edt")),
            Self::Unknown | Self::Invalid => None,
        }
    }

    /// Работаем ли мы с этим форматом. Сегодня работаем с одним — выгрузкой
    /// платформы; EDT не поддержан, а `unknown` и `invalid` форматом не
    /// являются вовсе.
    pub const fn is_supported(self) -> bool {
        matches!(self, Self::PlatformXml)
    }

    pub const fn stable_discriminant(self) -> u8 {
        match self {
            Self::PlatformXml => 1,
            Self::Edt => 2,
            Self::Unknown => 3,
            Self::Invalid => 4,
        }
    }
}

/// Closed semantic profile carried by an actor-owned source binding.
///
/// `SourceFormat` remains a separate discriminant because the physical source
/// representation and the exact platform/serialization capability are
/// independently planner-significant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SourceProfile {
    PlatformXml8_3_27Format2_20,
    /// A Designer export format beyond 2.20 with its own dialects.
    PlatformXmlRelease(format_identity::PlatformXmlRelease),
    LegacyWorkspaceServiceCompatibility,
    #[cfg(any(test, feature = "test-support"))]
    TestPlatform8_3_28Format2_20,
    #[cfg(any(test, feature = "test-support"))]
    TestPlatform8_3_27Format2_21,
}

impl SourceProfile {
    pub const fn platform_xml_8_3_27_format_2_20() -> Self {
        Self::PlatformXml8_3_27Format2_20
    }

    /// The profile of a Designer export recording `raw` in its root: 2.20 or a
    /// later release with dialects; `None` for any other version.
    pub fn platform_xml_for_export_format(raw: &str) -> Option<Self> {
        format_identity::platform_xml_profile_for_export_format(raw)
    }

    pub const fn legacy_workspace_service_compatibility() -> Self {
        Self::LegacyWorkspaceServiceCompatibility
    }

    pub const fn stable_discriminants(self) -> [u8; 3] {
        match self {
            Self::PlatformXml8_3_27Format2_20 => [1, 1, 1],
            Self::PlatformXmlRelease(release) => release.stable_discriminants(),
            Self::LegacyWorkspaceServiceCompatibility => [2, 0, 0],
            #[cfg(any(test, feature = "test-support"))]
            Self::TestPlatform8_3_28Format2_20 => [1, 2, 1],
            #[cfg(any(test, feature = "test-support"))]
            Self::TestPlatform8_3_27Format2_21 => [1, 1, 2],
        }
    }

    pub const fn platform_line(self) -> Option<&'static str> {
        match self {
            Self::PlatformXml8_3_27Format2_20 => Some(ACTIVE_PLATFORM_LINE),
            Self::PlatformXmlRelease(release) => Some(release.platform_line_text()),
            Self::LegacyWorkspaceServiceCompatibility => None,
            #[cfg(any(test, feature = "test-support"))]
            Self::TestPlatform8_3_28Format2_20 => Some("8.3.28-test"),
            #[cfg(any(test, feature = "test-support"))]
            Self::TestPlatform8_3_27Format2_21 => Some(ACTIVE_PLATFORM_LINE),
        }
    }

    pub const fn serialization_format(self) -> Option<&'static str> {
        match self {
            Self::PlatformXml8_3_27Format2_20 => Some(ACTIVE_SERIALIZATION_FORMAT),
            Self::PlatformXmlRelease(release) => Some(release.export_format()),
            Self::LegacyWorkspaceServiceCompatibility => None,
            #[cfg(any(test, feature = "test-support"))]
            Self::TestPlatform8_3_28Format2_20 => Some(ACTIVE_SERIALIZATION_FORMAT),
            #[cfg(any(test, feature = "test-support"))]
            Self::TestPlatform8_3_27Format2_21 => Some("2.21-test"),
        }
    }

    /// The platform line of the format this profile identifies, when that
    /// format is a known one (INV.SOURCE.FORMAT-IDENTITY): the line follows
    /// from the format. Legacy and test identities name no known format.
    pub const fn semantic_line(self) -> Option<format_identity::PlatformLine> {
        match self {
            Self::PlatformXml8_3_27Format2_20 => Some(format_identity::PlatformLine::new(8, 3, 27)),
            Self::PlatformXmlRelease(release) => Some(release.platform_line()),
            Self::LegacyWorkspaceServiceCompatibility => None,
            #[cfg(any(test, feature = "test-support"))]
            Self::TestPlatform8_3_28Format2_20 | Self::TestPlatform8_3_27Format2_21 => None,
        }
    }

    pub const fn canonical_id(self) -> &'static str {
        match self {
            Self::PlatformXml8_3_27Format2_20 => PLATFORM_XML_8_3_27_FORMAT_2_20,
            Self::PlatformXmlRelease(release) => release.canonical_id(),
            Self::LegacyWorkspaceServiceCompatibility => "legacy-workspace-service-compatibility",
            #[cfg(any(test, feature = "test-support"))]
            Self::TestPlatform8_3_28Format2_20 => "platform-xml-8.3.28-test-format-2.20",
            #[cfg(any(test, feature = "test-support"))]
            Self::TestPlatform8_3_27Format2_21 => "platform-xml-8.3.27-format-2.21-test",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn source_map_serializes_observed_identity_without_private_probe_inputs() {
        let map = ProjectSourceMap {
            workspace_root: "/workspace".into(),
            config_path: Some("/workspace/v8project.yaml".into()),
            source_sets: vec![ProjectSourceSet {
                name: "main".into(),
                kind: SourceSetKind::Configuration,
                path: "src/cf".into(),
                source_format: SourceFormat::PlatformXml,
                source_state: SourceSetState::Supported,
                format_evidence: vec!["Configuration.xml".into()],
                format_probe_error: Some("private probe input".into()),
            }],
            effective_source_set: Some("main".into()),
            effective_source_root: Some("/workspace/src/cf".into()),
            source_selection_error: None,
            configured_format_raw: Some("private configured spelling".into()),
        };
        assert_eq!(
            serde_json::to_value(map).unwrap(),
            json!({
                "workspaceRoot":"/workspace",
                "configPath":"/workspace/v8project.yaml",
                "sourceSets":[{
                    "name":"main","kind":"configuration","path":"src/cf",
                    "sourceFormat":"platform_xml","sourceState":"supported",
                    "formatEvidence":["Configuration.xml"]
                }],
                "effectiveSourceSet":"main","effectiveSourceRoot":"/workspace/src/cf"
            })
        );
    }

    #[test]
    fn source_profile_axes_and_legacy_identity_keep_distinct_stable_discriminants() {
        let active = SourceProfile::platform_xml_8_3_27_format_2_20();
        assert_eq!(active.platform_line(), Some("8.3.27"));
        assert_eq!(active.serialization_format(), Some("2.20"));
        assert_eq!(active.canonical_id(), "platform-xml-8.3.27-format-2.20");
        assert_eq!(active.stable_discriminants(), [1, 1, 1]);
        let legacy = SourceProfile::legacy_workspace_service_compatibility();
        assert_eq!(legacy.platform_line(), None);
        assert_eq!(legacy.serialization_format(), None);
        assert_eq!(legacy.stable_discriminants(), [2, 0, 0]);
        assert_eq!(
            SourceProfile::TestPlatform8_3_28Format2_20.stable_discriminants(),
            [1, 2, 1]
        );
        assert_eq!(
            SourceProfile::TestPlatform8_3_27Format2_21.stable_discriminants(),
            [1, 1, 2]
        );
        for (raw, line, id, discriminants) in [
            ("2.21", "8.5.1", "platform-xml-8.5.1-format-2.21", [1, 3, 3]),
            ("2.22", "8.5.4", "platform-xml-8.5.4-format-2.22", [1, 4, 4]),
        ] {
            let release = SourceProfile::platform_xml_for_export_format(raw)
                .expect("a format with dialects has a profile row");
            assert_eq!(release.platform_line(), Some(line));
            assert_eq!(release.serialization_format(), Some(raw));
            assert_eq!(
                release.semantic_line().map(|line| line.to_string()),
                Some(line.to_string())
            );
            assert_eq!(release.canonical_id(), id);
            assert_eq!(release.stable_discriminants(), discriminants);
        }
        assert_eq!(
            SourceProfile::platform_xml_for_export_format("2.20"),
            Some(active)
        );
        for raw in ["2.19", "2.23", "2.21.0", "02.21", ""] {
            assert_eq!(
                SourceProfile::platform_xml_for_export_format(raw),
                None,
                "{raw}"
            );
        }
        assert!(!SourceFormat::Edt.is_supported());
        assert!(!SourceFormat::Unknown.is_supported());
        assert!(!SourceFormat::Invalid.is_supported());
    }
}

pub mod location;
pub mod workspace;

pub mod events;
pub mod resources;

pub mod portable_path;
pub mod read;

pub mod proposal;

pub mod discovery;

pub mod compatibility;
pub mod format_identity;
