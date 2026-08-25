use crate::domain::address::{NodeKind, QualifiedAddress};
use crate::domain::platform_profile::{ModuleCapability, PlatformProfile};
use crate::domain::source_target::MetadataAddress;
use crate::infrastructure::native_operations::common::typed_role_reader_target;
use crate::infrastructure::native_operations::dcs::typed_dcs_reader_target;
use crate::infrastructure::native_operations::form::typed_form_reader_target;
use crate::infrastructure::native_operations::logical_selector::typed_reader_metadata_target;
use crate::infrastructure::native_operations::meta::{
    accepts_logical_metadata_address, logical_metadata_reader_target,
};
use crate::infrastructure::native_operations::mxl::typed_mxl_reader_target;
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
    Subsystem,
    Interface,
    Dcs,
    Mxl,
    Xdto,
    Module,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LogicalTreeRoute {
    at: QualifiedAddress,
    reader: LogicalReader,
    reader_metadata_path: Option<MetadataAddress>,
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

    pub(crate) fn reader_metadata_path(&self) -> Option<&MetadataAddress> {
        self.reader_metadata_path.as_ref()
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
        return Ok(route(address, LogicalReader::Module, None, Some(module)));
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
        return Ok(route(address, LogicalReader::Configuration, None, None));
    }
    if segments
        .first()
        .is_some_and(|segment| segment.kind() == NodeKind::Role)
    {
        let target = typed_role_reader_target(address).ok_or_else(|| not_found(address))?;
        return Ok(route(address, LogicalReader::Role, Some(target), None));
    }
    if segments
        .first()
        .is_some_and(|segment| segment.kind() == NodeKind::Subsystem)
    {
        let target = typed_reader_metadata_target(address, &["Subsystem"])
            .ok_or_else(|| not_found(address))?;
        let reader = if segments
            .iter()
            .any(|segment| segment.kind() == NodeKind::Interface)
        {
            LogicalReader::Interface
        } else {
            LogicalReader::Subsystem
        };
        return Ok(route(address, reader, Some(target), None));
    }
    if segments
        .first()
        .is_some_and(|segment| segment.kind() == NodeKind::CommonForm)
        || segments
            .iter()
            .any(|segment| segment.kind() == NodeKind::Form)
    {
        let target = typed_form_reader_target(address).ok_or_else(|| not_found(address))?;
        return Ok(route(address, LogicalReader::Form, Some(target), None));
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
        let target = typed_dcs_reader_target(address).ok_or_else(|| not_found(address))?;
        return Ok(route(address, LogicalReader::Dcs, Some(target), None));
    }
    if segments
        .iter()
        .any(|segment| segment.kind() == NodeKind::Area)
    {
        let target = typed_mxl_reader_target(address).ok_or_else(|| not_found(address))?;
        return Ok(route(address, LogicalReader::Mxl, Some(target), None));
    }
    if segments
        .first()
        .is_some_and(|segment| segment.kind() == NodeKind::XdtoPackage)
    {
        let target = typed_reader_metadata_target(address, &["XDTOPackage"])
            .ok_or_else(|| not_found(address))?;
        return Ok(route(address, LogicalReader::Xdto, Some(target), None));
    }
    if accepts_logical_metadata_address(address) {
        return Ok(route(
            address,
            LogicalReader::Metadata,
            logical_metadata_reader_target(address),
            None,
        ));
    }
    Err(not_found(address))
}

fn route(
    address: &QualifiedAddress,
    reader: LogicalReader,
    reader_metadata_path: Option<MetadataAddress>,
    module: Option<ModuleCapability>,
) -> LogicalTreeRoute {
    LogicalTreeRoute {
        at: address.clone(),
        reader,
        reader_metadata_path,
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
    fn deep_invalid_module_suffix_cannot_hide_below_a_valid_module_prefix() {
        let address = QualifiedAddress::parse(
            "main:Document.Заказ.Module.Object.Method.Проверить.Module.Service",
        )
        .unwrap();

        let error = route_logical_address(&address, PlatformProfile::v8_3_27()).unwrap_err();
        assert_eq!(error.code(), LogicalTreeErrorCode::NotFound);
    }

    #[test]
    fn logical_tree_delegates_representative_addresses_to_current_typed_reader_adapters() {
        let profile = PlatformProfile::v8_3_27();
        let cases = [
            (
                "main:Document.Заказ.Attribute.Контрагент",
                LogicalReader::Metadata,
                "Document.Заказ",
            ),
            (
                "main:Document.Заказ.Form.ФормаДокумента.Attribute.Объект",
                LogicalReader::Form,
                "Document.Заказ.Form.ФормаДокумента",
            ),
            (
                "main:Role.Кладовщик.Right.Catalog_Товары",
                LogicalReader::Role,
                "Role.Кладовщик",
            ),
            (
                "main:Report.Продажи.Template.ОсновнаяСхема.DataSet.Продажи",
                LogicalReader::Dcs,
                "Report.Продажи.Template.ОсновнаяСхема",
            ),
            (
                "main:Report.Продажи.Template.Печать.Area.Шапка",
                LogicalReader::Mxl,
                "Report.Продажи.Template.Печать",
            ),
        ];

        for (raw, reader, expected_target) in cases {
            let address = QualifiedAddress::parse(raw).unwrap();
            let route = route_logical_address(&address, profile).unwrap();
            assert_eq!(route.reader(), reader, "{raw}");
            assert_eq!(
                route.reader_metadata_path().map(|target| target.as_str()),
                Some(expected_target),
                "{raw}"
            );
        }
    }

    #[test]
    fn task14_profiles_route_to_their_real_typed_readers_without_skips() {
        let profile = PlatformProfile::v8_3_27();
        let cases = [
            ("main:Configuration", LogicalReader::Configuration),
            (
                "main:Document.Заказ.Attribute.Контрагент",
                LogicalReader::Metadata,
            ),
            (
                "main:Document.Заказ.Form.ФормаДокумента.Item.Товары",
                LogicalReader::Form,
            ),
            (
                "main:Role.Кладовщик.RLS.Document_Заказ",
                LogicalReader::Role,
            ),
            ("main:Subsystem.Продажи", LogicalReader::Subsystem),
            ("main:Subsystem.Продажи.Interface", LogicalReader::Interface),
            (
                "main:Report.Продажи.Template.ОсновнаяСхема.DataSet.Продажи",
                LogicalReader::Dcs,
            ),
            (
                "main:Report.Продажи.Template.Печать.Area.Шапка",
                LogicalReader::Mxl,
            ),
            ("main:XDTOPackage.Обмен.Type.Заказ", LogicalReader::Xdto),
            ("main:Document.Заказ.Module.Object", LogicalReader::Module),
        ];

        for (raw, expected) in cases {
            let address = QualifiedAddress::parse(raw).unwrap();
            let route = route_logical_address(&address, profile)
                .unwrap_or_else(|error| panic!("{raw}: {error}"));
            assert_eq!(route.reader(), expected, "{raw}");
            assert!(route.diagnostic_file().is_none(), "{raw}");
        }
    }

    #[test]
    fn qualified_logical_tree_core_contract_is_complete() {
        logical_tree_routes_branches_to_existing_typed_readers();
        logical_tree_delegates_representative_addresses_to_current_typed_reader_adapters();
        task14_profiles_route_to_their_real_typed_readers_without_skips();
        platform_capability_controls_logical_existence_without_filesystem_evidence();
        deep_invalid_module_suffix_cannot_hide_below_a_valid_module_prefix();
    }
}
