use serde::Serialize;
use std::fmt;

use crate::address::NodeKind;

#[cfg(test)]
use unica_xml_format::names::xml_ncname_is_valid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceTarget {
    pub source_set: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_path: Option<MetadataAddress>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct MetadataAddress(String);

impl MetadataAddress {
    /// A logical metadata address, `Kind.Name[.Kind.Name…]`. Its syntax and its
    /// vocabulary of kinds do not depend on the format version or the format
    /// family (INV.SOURCE.FORMAT-IDENTITY); whether a kind exists in a source
    /// set is a question for the set's profile, not for the address.
    pub fn parse_logical(raw: &str) -> Result<Self, SourceTargetError> {
        parse_address(raw)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Arity decides the kind, not the spelling of the last segment: `parse`
    /// accepts a module terminal only on an odd segment count, so an object
    /// legitimately named `Module` stays a metadata object instead of being
    /// read as the module role of a nameless owner.
    pub fn target_kind(&self) -> TargetKind {
        if self.0.split('.').count() % 2 == 1 {
            TargetKind::Module
        } else {
            TargetKind::MetadataObject
        }
    }

    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split('.')
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MetadataAddressPrefix(String);

impl MetadataAddressPrefix {
    /// A logical address prefix; see [`MetadataAddress::parse_logical`].
    pub fn parse_logical(raw: &str) -> Result<Self, SourceTargetError> {
        parse_prefix(raw)
    }
}

impl fmt::Display for MetadataAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TargetKind {
    SourceRoot,
    MetadataObject,
    Module,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedTarget {
    pub source_set: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_path: Option<MetadataAddress>,
    pub target_kind: TargetKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceTargetErrorCode {
    SourceSetRequired,
    SourceSetNotFound,
    SourceRootNotAddressable,
    MetadataAddressInvalid,
    MetadataAddressNotFound,
    TargetKindMismatch,
    ContainmentDenied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceTargetErrorReason {
    General,
    SourceFormatUnsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceTargetError {
    pub code: SourceTargetErrorCode,
    pub message: String,
    #[serde(skip)]
    reason: SourceTargetErrorReason,
}

impl SourceTargetError {
    pub fn new(code: SourceTargetErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            reason: SourceTargetErrorReason::General,
        }
    }

    pub fn source_format_unsupported(message: impl Into<String>) -> Self {
        Self {
            code: SourceTargetErrorCode::SourceRootNotAddressable,
            message: message.into(),
            reason: SourceTargetErrorReason::SourceFormatUnsupported,
        }
    }

    pub fn reason(&self) -> SourceTargetErrorReason {
        self.reason
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self::new(SourceTargetErrorCode::MetadataAddressInvalid, message)
    }
}

impl fmt::Display for SourceTargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for SourceTargetError {}

fn parse_address(raw: &str) -> Result<MetadataAddress, SourceTargetError> {
    let parts = raw.split('.').collect::<Vec<_>>();
    if parts.is_empty() || parts.iter().any(|part| part.is_empty()) {
        return Err(SourceTargetError::invalid(
            "metadata address contains an empty segment",
        ));
    }

    if parts.len() == 1 {
        return if is_root_module_terminal(parts[0]) {
            Ok(MetadataAddress(parts[0].to_string()))
        } else {
            Err(SourceTargetError::invalid(
                "metadata address must contain a kind and application name",
            ))
        };
    }

    // `Вид.Имя` pairs alternate at any depth; the depth is bounded by the
    // nested kinds each owner admits, not by a segment count.
    let terminal = (parts.len() % 2 == 1).then(|| parts[parts.len() - 1]);
    let pairs = &parts[..parts.len() - usize::from(terminal.is_some())];
    let mut canonical = Vec::with_capacity(parts.len());
    let mut owner = canonical_kind(pairs[0])?;
    canonical.push(owner);
    canonical.push(pairs[1]);
    for (depth, pair) in pairs[2..].chunks(2).enumerate() {
        owner = nested_kind(owner, depth + 1, pair[0])?;
        canonical.push(owner);
        canonical.push(pair[1]);
    }
    if let Some(terminal) = terminal {
        if !is_module_terminal(terminal) {
            return Err(SourceTargetError::invalid(format!(
                "unsupported metadata address terminal `{terminal}`"
            )));
        }
        canonical.push(terminal);
    }
    Ok(MetadataAddress(canonical.join(".")))
}

fn parse_prefix(raw: &str) -> Result<MetadataAddressPrefix, SourceTargetError> {
    let parts = raw.split('.').collect::<Vec<_>>();
    if parts.is_empty() || parts.iter().any(|part| part.is_empty()) {
        return Err(SourceTargetError::invalid(
            "metadata address prefix contains an empty segment",
        ));
    }

    if parts.len() == 1 {
        if let Ok(kind) = canonical_kind_or_collection(parts[0]) {
            return Ok(MetadataAddressPrefix(kind.to_string()));
        }
        if is_root_module_terminal(parts[0]) {
            return Ok(MetadataAddressPrefix(parts[0].to_string()));
        }
        if is_canonical_token_prefix(parts[0]) {
            return Ok(MetadataAddressPrefix(parts[0].to_string()));
        }
        if is_alias_prefix(parts[0]) {
            return Err(SourceTargetError::invalid(format!(
                    "metadata address prefix `{}` matches only a partial alias; use the complete kind token or its canonical English prefix",
                    parts[0]
                )));
        }
        return Err(SourceTargetError::invalid(format!(
            "unknown metadata address prefix root `{}`",
            parts[0]
        )));
    }

    let mut canonical = Vec::with_capacity(parts.len());
    let mut owner = canonical_kind_or_collection(parts[0])?;
    canonical.push(owner);
    canonical.push(parts[1]);
    let mut index = 2;
    while index < parts.len() {
        let depth = index / 2;
        let last = index + 1 == parts.len();
        match nested_kind(owner, depth, parts[index]) {
            Ok(child) => {
                owner = child;
                canonical.push(child);
                if !last {
                    canonical.push(parts[index + 1]);
                }
            }
            Err(error) if !last => return Err(error),
            Err(_) => {
                let terminal = parts[index];
                let mut terminals = if depth == 1 {
                    Box::new(MODULE_TERMINALS.iter().copied())
                        as Box<dyn Iterator<Item = &'static str>>
                } else {
                    Box::new(module_terminal_after(owner))
                };
                if !terminals.any(|role| role.starts_with(terminal)) {
                    return Err(SourceTargetError::invalid(if depth == 1 {
                        format!("unknown metadata address prefix transition `{terminal}`")
                    } else {
                        format!(
                            "metadata address prefix terminal `{terminal}` is invalid after `{owner}`"
                        )
                    }));
                }
                canonical.push(terminal);
            }
        }
        index += 2;
    }
    Ok(MetadataAddressPrefix(canonical.join(".")))
}

/// The module roles a nested object owns: a form its form module, a command
/// its command module, a declared nested kind the roles it declares.
/// A template owns none.
fn module_terminal_after(kind: &'static str) -> impl Iterator<Item = &'static str> {
    let roles: &'static [&'static str] = match kind {
        "Form" => &["FormModule"],
        "Command" => &["CommandModule"],
        _ => NodeKind::parse(kind)
            .ok()
            .and_then(|kind| kind.declaration().nested.as_ref())
            .map_or(&[], |place| place.modules),
    };
    roles.iter().copied()
}

#[derive(Clone, Copy)]
struct AddressKind {
    canonical: &'static str,
    russian_aliases: &'static [&'static str],
    collection_aliases: &'static [&'static str],
    /// Its manager is a property of the global context: `Справочники`.
    manager: bool,
    /// Its objects carry attributes and tabular sections.
    object: bool,
}

/// The metadata kinds of `crate::kinds` with their address spellings.
fn address_kinds() -> impl Iterator<Item = AddressKind> {
    NodeKind::metadata_kinds().iter().filter_map(|kind| {
        let declaration = kind.declaration();
        declaration.metadata.map(|metadata| AddressKind {
            canonical: declaration.tag,
            russian_aliases: declaration.russian,
            collection_aliases: metadata.collections,
            manager: metadata.manager,
            object: metadata.object,
        })
    })
}

fn canonical_kind(raw: &str) -> Result<&'static str, SourceTargetError> {
    if let Some(kind) =
        address_kinds().find(|kind| kind.canonical == raw || kind.russian_aliases.contains(&raw))
    {
        return Ok(kind.canonical);
    }
    if address_kinds().any(|kind| kind.collection_aliases.contains(&raw)) {
        return Err(SourceTargetError::invalid(format!(
            "metadata collection `{raw}` is not an addressable kind"
        )));
    }
    Err(SourceTargetError::invalid(format!(
        "unknown metadata kind `{raw}`"
    )))
}

/// Non-allocating twin of `metadata_address_kind_spellings` for the hot
/// address-parsing path: it answers whether `raw` is the canonical spelling
/// or a Russian alias of `canonical`.
pub fn metadata_address_kind_matches(canonical: &str, raw: &str) -> bool {
    address_kinds()
        .find(|kind| kind.canonical == canonical)
        .is_some_and(|kind| kind.canonical == raw || kind.russian_aliases.contains(&raw))
}

/// The collection names of a canonical kind: English first, then Russian.
pub fn metadata_collection_names(canonical: &str) -> Option<&'static [&'static str]> {
    address_kinds()
        .find(|kind| kind.canonical == canonical)
        .map(|kind| kind.collection_aliases)
}

/// Kinds whose managers are properties of the global context, as code names
/// them: `Справочники`, `Документы`.
pub fn metadata_manager_kinds() -> impl Iterator<Item = &'static str> {
    address_kinds()
        .filter(|kind| kind.manager)
        .map(|kind| kind.canonical)
}

/// Whether objects of a canonical kind carry attributes and tabular sections.
pub fn metadata_kind_has_objects(canonical: &str) -> bool {
    address_kinds().any(|kind| kind.canonical == canonical && kind.object)
}

pub fn metadata_address_kind_spellings(canonical: &str) -> Option<Vec<&'static str>> {
    address_kinds()
        .find(|kind| kind.canonical == canonical)
        .map(|kind| {
            std::iter::once(kind.canonical)
                .chain(kind.russian_aliases.iter().copied())
                .collect()
        })
}

/// The kind a BSL manager collection names in code: `Справочники.Валюты` and
/// `Catalogs.Валюты` both name a `Catalog`. BSL names are case-insensitive, so
/// `справочники` matches too.
pub fn metadata_collection_kind(raw: &str) -> Option<&'static str> {
    let raw = raw.to_lowercase();
    address_kinds()
        .find(|kind| {
            kind.collection_aliases
                .iter()
                .any(|alias| alias.to_lowercase() == raw)
        })
        .map(|kind| kind.canonical)
}

fn canonical_kind_or_collection(raw: &str) -> Result<&'static str, SourceTargetError> {
    address_kinds()
        .find(|kind| {
            kind.canonical == raw
                || kind.russian_aliases.contains(&raw)
                || kind.collection_aliases.contains(&raw)
        })
        .map(|kind| kind.canonical)
        .ok_or_else(|| SourceTargetError::invalid(format!("unknown metadata kind `{raw}`")))
}

/// A partial prefix segment is matched against canonical English spelling only.
/// A partial alias has no single canonical form to expand into, so it is
/// reported instead of being silently passed through unnormalized.
fn is_canonical_token_prefix(raw: &str) -> bool {
    address_kinds().any(|kind| kind.canonical.starts_with(raw))
        || ROOT_MODULE_TERMINALS
            .iter()
            .any(|terminal| terminal.starts_with(raw))
}

fn is_alias_prefix(raw: &str) -> bool {
    address_kinds().any(|kind| {
        kind.russian_aliases
            .iter()
            .chain(kind.collection_aliases.iter())
            .any(|alias| alias.starts_with(raw))
    })
}

/// The kind of a nested pair under `owner`. A form, template or command
/// belongs to a top-level object, a recalculation to a calculation register;
/// any other nested kind declares its owners in its own declaration.
fn nested_kind(owner: &str, depth: usize, raw: &str) -> Result<&'static str, SourceTargetError> {
    if depth == 1 && owner == "CalculationRegister" && RECALCULATION_KIND_SPELLINGS.contains(&raw) {
        return Ok("Recalculation");
    }
    let unsupported =
        || SourceTargetError::invalid(format!("unsupported nested metadata kind `{raw}`"));
    let canonical = canonical_kind_or_collection(raw).map_err(|_| unsupported())?;
    let typical = depth == 1 && matches!(canonical, "Form" | "Template" | "Command");
    if typical || crate::kinds::nested_place(owner, canonical).is_some() {
        Ok(canonical)
    } else {
        Err(unsupported())
    }
}

pub const RECALCULATION_KIND_SPELLINGS: &[&str] = &[
    "Recalculation",
    "Recalculations",
    "Перерасчет",
    "Перерасчёт",
    "Перерасчеты",
    "Перерасчёты",
];

const MODULE_TERMINALS: &[&str] = &[
    "Module",
    "ObjectModule",
    "ManagerModule",
    "RecordSetModule",
    "ValueManagerModule",
    "FormModule",
    "CommandModule",
    "ManagedApplicationModule",
    "OrdinaryApplicationModule",
    "SessionModule",
    "ExternalConnectionModule",
];

const ROOT_MODULE_TERMINALS: &[&str] = &[
    "ManagedApplicationModule",
    "OrdinaryApplicationModule",
    "SessionModule",
    "ExternalConnectionModule",
];

pub fn is_module_terminal(value: &str) -> bool {
    MODULE_TERMINALS.contains(&value)
}

fn is_root_module_terminal(value: &str) -> bool {
    ROOT_MODULE_TERMINALS.contains(&value)
}

#[cfg(test)]
mod tests {

    #[test]
    fn the_kind_table_names_global_managers_and_kinds_with_objects() {
        let managers = metadata_manager_kinds().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            managers,
            [
                "AccountingRegister",
                "AccumulationRegister",
                "BusinessProcess",
                "CalculationRegister",
                "Catalog",
                "ChartOfAccounts",
                "ChartOfCalculationTypes",
                "ChartOfCharacteristicTypes",
                "Constant",
                "DataProcessor",
                "Document",
                "DocumentJournal",
                "Enum",
                "ExchangePlan",
                "ExternalDataSource",
                "FilterCriterion",
                "InformationRegister",
                "Report",
                "Sequence",
                "SessionParameter",
                "SettingsStorage",
                "Task",
                "WSReference",
            ]
            .into_iter()
            .collect()
        );
        let objects = address_kinds()
            .map(|kind| kind.canonical)
            .filter(|kind| metadata_kind_has_objects(kind))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            objects,
            [
                "BusinessProcess",
                "Catalog",
                "ChartOfAccounts",
                "ChartOfCalculationTypes",
                "ChartOfCharacteristicTypes",
                "DataProcessor",
                "Document",
                "ExchangePlan",
                "Report",
                "Task",
            ]
            .into_iter()
            .collect()
        );
        assert!(objects.is_subset(&managers));
    }

    #[test]
    fn a_manager_collection_in_code_names_its_kind_in_any_case() {
        use super::metadata_collection_kind;
        assert_eq!(metadata_collection_kind("Справочники"), Some("Catalog"));
        assert_eq!(metadata_collection_kind("справочники"), Some("Catalog"));
        assert_eq!(metadata_collection_kind("Catalogs"), Some("Catalog"));
        assert_eq!(
            metadata_collection_kind("РегистрыСведений"),
            Some("InformationRegister")
        );
        assert_eq!(metadata_collection_kind("Справочник"), None);
        assert_eq!(metadata_collection_kind("ОбщийМодуль"), None);
    }

    use super::{
        address_kinds, metadata_kind_has_objects, metadata_manager_kinds, xml_ncname_is_valid,
        MetadataAddress, MetadataAddressPrefix, SourceTargetErrorCode, TargetKind,
    };

    /// rule: INV.SOURCE.LOGICAL-IDENTITY
    #[test]
    fn source_target_profile_emits_canonical_english_kind_tokens() {
        let address =
            MetadataAddress::parse_logical("Catalog.Counterparties.ObjectModule").unwrap();

        assert_eq!(address.as_str(), "Catalog.Counterparties.ObjectModule");
        assert_eq!(address.target_kind(), TargetKind::Module);
    }

    /// rule: INV.SOURCE.LOGICAL-IDENTITY
    #[test]
    fn source_target_profile_normalizes_only_registered_exact_russian_kind_aliases() {
        let address =
            MetadataAddress::parse_logical("Справочник.Контрагенты.Форма.Списка.FormModule")
                .unwrap();

        assert_eq!(
            address.as_str(),
            "Catalog.Контрагенты.Form.Списка.FormModule"
        );
        let error =
            MetadataAddress::parse_logical("справочник.Контрагенты.ObjectModule").unwrap_err();
        assert_eq!(error.code, SourceTargetErrorCode::MetadataAddressInvalid);
    }

    /// rule: INV.SOURCE.LOGICAL-IDENTITY
    #[test]
    fn source_target_profile_preserves_application_name_case() {
        let address = MetadataAddress::parse_logical("CommonModule.eBayHTTP.Module").unwrap();

        assert_eq!(address.as_str(), "CommonModule.eBayHTTP.Module");
    }

    #[test]
    fn source_target_profile_rejects_unsupported_terminal() {
        let error = MetadataAddress::parse_logical("Catalog.Items.UnknownModule").unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::MetadataAddressInvalid);
        assert!(error.message.contains("terminal"));
    }

    #[test]
    fn source_target_profile_rejects_empty_segments_and_collections() {
        for raw in ["Catalog..ObjectModule", "Catalogs.Items"] {
            let error = MetadataAddress::parse_logical(raw).unwrap_err();
            assert_eq!(
                error.code,
                SourceTargetErrorCode::MetadataAddressInvalid,
                "{raw}"
            );
        }
    }

    #[test]
    fn source_target_profile_rejects_unknown_kind() {
        let error = MetadataAddress::parse_logical("UnknownKind.Items.ObjectModule").unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::MetadataAddressInvalid);
        assert!(error.message.contains("kind"));
    }

    #[test]
    fn source_target_prefix_profile_rejects_unknown_transitions_and_malformed_forms() {
        for raw in [
            "",
            ".",
            "Catalog..Man",
            "Unknown.Items",
            "Catalog.Items.Unknown",
            "Catalog.Items.Form.Order.Unknown",
            "Catalog.Items.ManagerModule.Extra",
            "Catalog.Items.Command.Print.FormM",
            "Catalog.Items.Template.Print.FormModule",
        ] {
            let error = MetadataAddressPrefix::parse_logical(raw).unwrap_err();
            assert_eq!(
                error.code,
                SourceTargetErrorCode::MetadataAddressInvalid,
                "{raw}"
            );
        }
    }

    #[test]
    fn source_target_profile_classifies_by_arity_not_by_terminal_spelling() {
        for raw in [
            "CommonModule.Module",
            "Catalog.SessionModule",
            "Document.ObjectModule",
            "Catalog.Items.Form.FormModule",
        ] {
            let address = MetadataAddress::parse_logical(raw).unwrap();
            assert_eq!(
                address.target_kind(),
                TargetKind::MetadataObject,
                "{raw} names an object, its terminal segment is an application name"
            );
        }
        for raw in [
            "SessionModule",
            "CommonModule.Shared.Module",
            "Catalog.Items.Form.Order.FormModule",
        ] {
            let address = MetadataAddress::parse_logical(raw).unwrap();
            assert_eq!(address.target_kind(), TargetKind::Module, "{raw}");
        }
    }

    #[test]
    fn source_target_prefix_profile_names_partial_aliases_apart_from_unknown_roots() {
        let alias = MetadataAddressPrefix::parse_logical("Док").unwrap_err();
        assert_eq!(alias.code, SourceTargetErrorCode::MetadataAddressInvalid);
        assert!(alias.message.contains("partial alias"), "{}", alias.message);

        let unknown = MetadataAddressPrefix::parse_logical("Zz").unwrap_err();
        assert!(
            unknown
                .message
                .contains("unknown metadata address prefix root"),
            "{}",
            unknown.message
        );
    }

    /// rule: INV.SOURCE.FORMAT-IDENTITY
    #[test]
    fn a_logical_address_parses_without_a_format_version() {
        for (raw, canonical) in [
            ("Catalog.Items", "Catalog.Items"),
            ("Справочник.Items", "Catalog.Items"),
            ("Справочник.Items.Форма.Main", "Catalog.Items.Form.Main"),
            (
                "Catalog.Items.Form.Main.Module",
                "Catalog.Items.Form.Main.Module",
            ),
            ("ManagedApplicationModule", "ManagedApplicationModule"),
        ] {
            assert_eq!(
                MetadataAddress::parse_logical(raw).unwrap().as_str(),
                canonical,
                "{raw}"
            );
        }
        assert_eq!(
            MetadataAddressPrefix::parse_logical("Справочник")
                .unwrap()
                .0,
            "Catalog"
        );
        assert!(MetadataAddress::parse_logical("Catalog..Items").is_err());
    }

    #[test]
    fn xml_ncname_validation_covers_unicode_and_rejects_qname_or_path_syntax() {
        for valid in ["Name", "_Имя", "Имя-2", "A.B", "\u{10000}Name"] {
            assert!(xml_ncname_is_valid(valid), "{valid:?}");
        }
        for invalid in ["", "1Name", "Bad Name", "cfg:Name", ".Name"] {
            assert!(!xml_ncname_is_valid(invalid), "{invalid:?}");
        }
    }

    #[test]
    fn address_profile_accepts_proven_root_module_terminals() {
        let address = MetadataAddress::parse_logical("ManagedApplicationModule").unwrap();

        assert_eq!(address.as_str(), "ManagedApplicationModule");
        assert_eq!(address.target_kind(), TargetKind::Module);
    }

    /// rule: INV.SOURCE.LOGICAL-IDENTITY
    #[test]
    fn source_target_and_resolved_target_serialize_only_logical_identity() {
        let address = MetadataAddress::parse_logical("Справочник.Items").unwrap();
        let target = super::SourceTarget {
            source_set: "addOn".to_string(),
            metadata_path: Some(address.clone()),
        };
        let resolved = super::ResolvedTarget {
            source_set: "addOn".to_string(),
            metadata_path: Some(address),
            target_kind: TargetKind::MetadataObject,
        };

        assert_eq!(
            serde_json::to_value(target).unwrap(),
            serde_json::json!({
                "sourceSet": "addOn",
                "metadataPath": "Catalog.Items"
            })
        );
        assert_eq!(
            serde_json::to_value(resolved).unwrap(),
            serde_json::json!({
                "sourceSet": "addOn",
                "metadataPath": "Catalog.Items",
                "targetKind": "metadataObject"
            })
        );
    }

    /// Пары `Вид.Имя` чередуются на любой глубине: таблица измерения куба
    /// внешнего источника адресуется семью сегментами.
    #[test]
    fn nested_kinds_declared_by_their_owner_extend_the_address_at_any_depth() {
        for (raw, canonical, kind) in [
            (
                "ВнешнийИсточникДанных.S.Таблица.T",
                "ExternalDataSource.S.Table.T",
                TargetKind::MetadataObject,
            ),
            (
                "ExternalDataSource.S.Table.T.RecordSetModule",
                "ExternalDataSource.S.Table.T.RecordSetModule",
                TargetKind::Module,
            ),
            (
                "ExternalDataSource.S.Function.F",
                "ExternalDataSource.S.Function.F",
                TargetKind::MetadataObject,
            ),
            (
                "ExternalDataSource.S.Куб.C.ТаблицаИзмерения.D.ObjectModule",
                "ExternalDataSource.S.Cube.C.DimensionTable.D.ObjectModule",
                TargetKind::Module,
            ),
        ] {
            let address = MetadataAddress::parse_logical(raw).unwrap();
            assert_eq!(address.as_str(), canonical, "{raw}");
            assert_eq!(address.target_kind(), kind, "{raw}");
        }
        for raw in [
            "Catalog.X.Table.T",
            "ExternalDataSource.S.DimensionTable.D",
            "ExternalDataSource.S.Cube.C.Table.T",
            "ExternalDataSource.S.Table.T.Form.F",
            "ExternalDataSource.S.Cube.C.DimensionTable.D.Unknown",
        ] {
            assert!(MetadataAddress::parse_logical(raw).is_err(), "{raw}");
        }
        assert_eq!(
            MetadataAddressPrefix::parse_logical(
                "ExternalDataSource.S.Cube.C.DimensionTable.D.Obj"
            )
            .unwrap()
            .0,
            "ExternalDataSource.S.Cube.C.DimensionTable.D.Obj"
        );
        for raw in [
            "ExternalDataSource.S.Cube.C.Obj",
            "ExternalDataSource.S.Function.F.Man",
        ] {
            assert!(MetadataAddressPrefix::parse_logical(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn nested_subsystem_paths_do_not_extend_the_metadata_address_grammar() {
        assert!(MetadataAddress::parse_logical("Subsystem.A.B").is_err());
        assert!(MetadataAddressPrefix::parse_logical("Subsystem.A.B").is_err());

        let top_level = MetadataAddress::parse_logical("Subsystem.A").unwrap();
        assert_eq!(top_level.target_kind(), TargetKind::MetadataObject);
    }
}
