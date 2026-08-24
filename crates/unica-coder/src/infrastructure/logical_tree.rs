use crate::domain::address::{NodeKind, QualifiedAddress};
use crate::domain::platform_profile::{ModuleCapability, PlatformProfile};
use crate::infrastructure::native_operations::meta::accepts_logical_metadata_address;
use serde::Deserialize;
use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum LogicalReader {
    Configuration,
    Metadata,
    Form,
    Role,
    Dcs,
    Mxl,
    Module,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LogicalTreeRoute {
    at: QualifiedAddress,
    reader: LogicalReader,
    module: Option<ModuleCapability>,
    diagnostic_file: Option<PathBuf>,
}

impl LogicalTreeRoute {
    pub(crate) fn at(&self) -> &QualifiedAddress {
        &self.at
    }

    pub(crate) const fn reader(&self) -> LogicalReader {
        self.reader
    }

    pub(crate) const fn module(&self) -> Option<ModuleCapability> {
        self.module
    }

    pub(crate) fn diagnostic_file(&self) -> Option<&std::path::Path> {
        self.diagnostic_file.as_deref()
    }
}

pub(crate) fn route_logical_address(
    address: &QualifiedAddress,
    profile: PlatformProfile,
) -> Result<LogicalTreeRoute, LogicalTreeError> {
    if let Some(module) = profile.module_prefix_capability(address) {
        let _internal_layout = module.source_layout();
        return Ok(route(address, LogicalReader::Module, Some(module)));
    }

    let segments = address.segments();
    if segments
        .iter()
        .any(|segment| segment.kind() == NodeKind::Module)
    {
        return Err(not_found(address));
    }
    if matches!(segments, [root] if root.kind() == NodeKind::Configuration && root.name().is_none())
    {
        return Ok(route(address, LogicalReader::Configuration, None));
    }
    if segments
        .first()
        .is_some_and(|segment| segment.kind() == NodeKind::Role)
    {
        return Ok(route(address, LogicalReader::Role, None));
    }
    if segments
        .first()
        .is_some_and(|segment| segment.kind() == NodeKind::CommonForm)
        || segments
            .iter()
            .any(|segment| segment.kind() == NodeKind::Form)
    {
        return Ok(route(address, LogicalReader::Form, None));
    }
    if segments.iter().any(|segment| {
        matches!(
            segment.kind(),
            NodeKind::DataSet
                | NodeKind::Field
                | NodeKind::Query
                | NodeKind::Calculation
                | NodeKind::Setting
        )
    }) {
        return Ok(route(address, LogicalReader::Dcs, None));
    }
    if segments
        .iter()
        .any(|segment| segment.kind() == NodeKind::Area)
    {
        return Ok(route(address, LogicalReader::Mxl, None));
    }
    if accepts_logical_metadata_address(address) {
        return Ok(route(address, LogicalReader::Metadata, None));
    }
    Err(not_found(address))
}

fn route(
    address: &QualifiedAddress,
    reader: LogicalReader,
    module: Option<ModuleCapability>,
) -> LogicalTreeRoute {
    LogicalTreeRoute {
        at: address.clone(),
        reader,
        module,
        diagnostic_file: None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LogicalTreeErrorCode {
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LogicalTreeError {
    code: LogicalTreeErrorCode,
    message: String,
}

impl LogicalTreeError {
    pub(crate) const fn code(&self) -> LogicalTreeErrorCode {
        self.code
    }
}

fn not_found(address: &QualifiedAddress) -> LogicalTreeError {
    LogicalTreeError {
        code: LogicalTreeErrorCode::NotFound,
        message: format!("logical address `{address}` is not available in the platform profile"),
    }
}

impl fmt::Display for LogicalTreeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for LogicalTreeError {}

#[cfg(test)]
mod tests {
    use super::{route_logical_address, LogicalReader, LogicalTreeErrorCode};
    use crate::domain::address::QualifiedAddress;
    use crate::domain::platform_profile::PlatformProfile;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Fixture {
        valid_addresses: Vec<AddressCase>,
        module_capabilities: Vec<ModuleCase>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct AddressCase {
        case: String,
        input: String,
        route: LogicalReader,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ModuleCase {
        case: String,
        at: String,
        exists: bool,
    }

    fn fixture() -> Fixture {
        serde_json::from_str(include_str!(
            "../../../../tests/fixtures/v013/address-profile-8.3.27.json"
        ))
        .expect("the checked logical tree fixture must be valid JSON")
    }

    #[test]
    fn logical_tree_routes_branches_to_existing_typed_readers() {
        let profile = PlatformProfile::v8_3_27();
        for case in fixture().valid_addresses {
            let address = QualifiedAddress::parse(&case.input).unwrap();
            let route = route_logical_address(&address, profile)
                .unwrap_or_else(|error| panic!("{}: {error}", case.case));
            assert_eq!(route.reader(), case.route, "{}", case.case);
            assert_eq!(route.at().to_string(), address.to_string(), "{}", case.case);
            assert!(route.diagnostic_file().is_none(), "{}", case.case);
        }
    }

    #[test]
    fn platform_capability_controls_logical_existence_without_filesystem_evidence() {
        let profile = PlatformProfile::v8_3_27();
        for case in fixture().module_capabilities {
            let address = QualifiedAddress::parse(&case.at).unwrap();
            match route_logical_address(&address, profile) {
                Ok(route) => {
                    assert!(case.exists, "{} unexpectedly exists", case.case);
                    assert_eq!(route.reader(), LogicalReader::Module, "{}", case.case);
                    assert!(route.diagnostic_file().is_none(), "{}", case.case);
                }
                Err(error) => {
                    assert!(!case.exists, "{} unexpectedly failed: {error}", case.case);
                    assert_eq!(
                        error.code(),
                        LogicalTreeErrorCode::NotFound,
                        "{}",
                        case.case
                    );
                }
            }
        }
    }

    #[test]
    fn qualified_logical_tree_core_contract_is_complete() {
        logical_tree_routes_branches_to_existing_typed_readers();
        platform_capability_controls_logical_existence_without_filesystem_evidence();
    }
}
