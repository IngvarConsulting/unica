use serde::Serialize;
use std::fmt;
use std::path::PathBuf;

pub const SUBSYSTEM_ADDRESS_MAX_DEPTH: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct SubsystemAddress(String);

impl SubsystemAddress {
    pub fn parse(raw: &str) -> Result<Self, SubsystemAddressError> {
        Self::from_names(raw.split('.'))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_names<'a>(
        names: impl IntoIterator<Item = &'a str>,
    ) -> Result<Self, SubsystemAddressError> {
        let names = names.into_iter().collect::<Vec<_>>();
        if names.is_empty() || names.len() > SUBSYSTEM_ADDRESS_MAX_DEPTH {
            return Err(SubsystemAddressError::new(format!(
                "subsystem address must contain from 1 to {SUBSYSTEM_ADDRESS_MAX_DEPTH} names"
            )));
        }
        for name in &names {
            if !crate::identifier::is_1c_identifier(name) {
                return Err(SubsystemAddressError::new(format!(
                    "subsystem address contains invalid name `{name}`"
                )));
            }
        }
        Ok(Self(names.join(".")))
    }
}

impl fmt::Display for SubsystemAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubsystemAddressError {
    message: String,
}

impl SubsystemAddressError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for SubsystemAddressError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SubsystemAddressError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectiveSubsystemRole {
    Functional,
    Interface,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bsl_address_serializes_without_platform_kind_tokens() {
        let address = SubsystemAddress::parse("СтандартныеПодсистемы.Обсуждения").unwrap();

        assert_eq!(address.as_str(), "СтандартныеПодсистемы.Обсуждения");
        assert_eq!(
            serde_json::to_value(address).unwrap(),
            serde_json::json!("СтандартныеПодсистемы.Обсуждения")
        );
    }

    #[test]
    fn address_preserves_the_spelling_of_application_names() {
        let address = SubsystemAddress::parse("Sales.eCommerce").unwrap();

        assert_eq!(address.as_str(), "Sales.eCommerce");
    }

    #[test]
    fn address_rejects_empty_and_non_identifier_segments() {
        for raw in ["", ".A", "A.", "A..B", "1Root", "A.With space", "A.A-B"] {
            assert!(
                SubsystemAddress::parse(raw).is_err(),
                "invalid subsystem address was accepted: {raw}"
            );
        }
    }

    #[test]
    fn depth_budget_counts_subsystem_names_not_a_kind_token() {
        let maximum = "A.B.C.D.E.F.G.H";
        assert_eq!(SubsystemAddress::parse(maximum).unwrap().as_str(), maximum);
        assert!(SubsystemAddress::parse("A.B.C.D.E.F.G.H.I").is_err());
    }

    #[test]
    fn registered_names_build_the_same_public_address() {
        let address =
            SubsystemAddress::from_names(["СтандартныеПодсистемы", "Обсуждения"]).unwrap();

        assert_eq!(address.as_str(), "СтандартныеПодсистемы.Обсуждения");
    }
}

use crate::support_state::ObjectSupportData as DomainObjectSupportData;
pub struct SubsystemInfoData {
    pub name: String,
    pub synonym: String,
    pub comment: Option<String>,
    pub include_ci: String,
    pub use_one_command: String,
    pub explanation: Option<String>,
    pub picture: String,
    pub content_items: Vec<String>,
    pub groups: Vec<(String, Vec<String>)>,
    pub child_names: Vec<String>,
    pub has_ci: bool,
    pub property_limits: Vec<String>,
}

pub struct SubsystemEditModel {
    pub version: String,
    pub uuid: String,
    pub name: String,
    pub synonym: String,
    pub comment: String,
    pub include_help: String,
    pub include_ci: String,
    pub use_one_command: String,
    pub explanation: String,
    pub picture: String,
    pub picture_load_transparent: Option<String>,
    pub content: Vec<String>,
    pub children: Vec<String>,
}

#[derive(Default)]
pub struct SubsystemEditCounters {
    pub added: usize,
    pub removed: usize,
    pub modified: usize,
}

/// Typed answer of `unica.subsystem.edit` (ADR-0023). Every requested operation
/// is reported, including the ones that changed nothing, and a content
/// reference that had to be normalized says so instead of printing `[NORM]`.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsystemEditData {
    pub subsystem: String,
    pub operations: Vec<SubsystemEditOperationData>,
    pub added: usize,
    pub removed: usize,
    pub modified: usize,
    /// Child subsystem descriptors created because the edit referenced them.
    pub created_stubs: Vec<String>,
    /// The edit commits only through post-validation; this records that the
    /// validator actually ran over the written subsystem.
    pub validated: bool,
    pub mutation: unica_execution_contract::mutation::MutationData,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsystemEditOperationData {
    pub operation: String,
    pub target: Option<String>,
    pub applied: bool,
    /// Why nothing changed; `null` when the operation applied.
    pub reason: Option<String>,
    /// The reference as it was requested, when it had to be normalized to the
    /// canonical form; `null` when it was already canonical.
    pub normalized_from: Option<String>,
}

/// Typed answer of `unica.subsystem.info` (ADR-0023).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsystemInfoResult {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub property_limits: Vec<String>,
    pub name: String,
    pub synonym: Option<String>,
    pub comment: Option<String>,
    pub explanation: Option<String>,
    pub picture: Option<String>,
    pub include_in_command_interface: Option<String>,
    pub use_one_command: Option<String>,
    pub support: DomainObjectSupportData,
    pub content: Vec<String>,
    pub groups: Vec<SubsystemGroupData>,
    pub children: Vec<String>,
    /// A focused registered tree: one ancestor chain down to this subsystem,
    /// then its complete descendant tree. Standalone XML has no proved tree.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tree: Option<Vec<SubsystemTreeNode>>,
    /// `null` when the subsystem ships no `CommandInterface.xml`, which is a
    /// different fact from an interface that hides nothing.
    pub command_interface: Option<SubsystemCommandInterfaceData>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsystemGroupData {
    pub name: String,
    pub items: Vec<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsystemCommandInterfaceData {
    pub visibility: Vec<SubsystemCommandVisibilityData>,
    pub placement: Vec<SubsystemCommandPlacementData>,
    pub order: Vec<SubsystemGroupData>,
    /// Порядок групп панели. Отдельная секция, а не порядок ключей в
    /// `order`: группа может быть объявлена в порядке и не иметь ни одной
    /// команды.
    pub groups_order: Vec<String>,
    /// Порядок дочерних подсистем. У документа подсистемы эта секция своя и
    /// говорит о её детях, а не о корне конфигурации.
    pub subsystem_order: Vec<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsystemCommandVisibilityData {
    pub command: String,
    pub visible: bool,
    /// Сколько ролей переопределяют общее значение. Замер на боевой
    /// конфигурации: 99 блоков видимости из 1050 несут такие значения, то
    /// есть каждая одиннадцатая команда. Умолчать о них значило бы отдать
    /// `visible` за всю правду.
    pub role_overrides: usize,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsystemCommandPlacementData {
    pub command: String,
    pub group: Option<String>,
    pub placement: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsystemTreeNode {
    pub name: String,
    pub content: usize,
    pub children: Vec<SubsystemTreeNode>,
}

pub struct SubsystemInfoFormatDocument {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

impl SubsystemEditOperationData {
    pub fn applied(operation: &str, target: impl Into<Option<String>>) -> Self {
        Self {
            operation: operation.to_string(),
            target: target.into(),
            applied: true,
            reason: None,
            normalized_from: None,
        }
    }

    pub fn skipped(operation: &str, target: impl Into<Option<String>>, reason: &str) -> Self {
        Self {
            operation: operation.to_string(),
            target: target.into(),
            applied: false,
            reason: Some(reason.to_string()),
            normalized_from: None,
        }
    }

    pub fn normalized_from(mut self, original: String) -> Self {
        self.normalized_from = Some(original);
        self
    }
}

/// Collects what the subsystem edit did, replacing the `&mut String` the
/// helpers used to append prose to.
pub type SubsystemEditLog = Vec<SubsystemEditOperationData>;

/// The tool answers about whatever it was pointed at: one subsystem for a file,
/// the hierarchy for a `Subsystems/` folder.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase", untagged)]
pub enum SubsystemInfoAnswer {
    Subsystem(Box<SubsystemInfoResult>),
    Tree { tree: Vec<SubsystemTreeNode> },
}
