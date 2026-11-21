use super::*;
use crate::infrastructure::v13_read_projection::{
    project_ordinary_interface, project_registered_metadata_branch_names, MemberSupport,
};
use unica_source_designer::layout::ORDINARY_INTERFACE_XML_TAG;

impl LogicalViewReadAuthority<'_> {
    pub(super) fn ordinary_interface_view(
        &self,
        at: &QualifiedAddress,
        name: Option<&str>,
        admitted: &ViewSourceSnapshot,
    ) -> Result<NodeViewData, ViewError> {
        if self.read.source_set_kind() != SourceSetKind::Configuration {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                "ordinary application interfaces require a configuration source set",
            ));
        }
        let registrations = self.streamed_configuration_registration_index(admitted)?;
        let mut checkpoint = || self.read_checkpoint();
        if let Some(name) = name {
            if !registrations.contains(ORDINARY_INTERFACE_XML_TAG, name, &checkpoint)? {
                return Err(ViewError::new(
                    RefusalCode::NotFound,
                    format!("ordinary application interface `{name}` is not registered"),
                ));
            }
            let synonym = self
                .read
                .ordinary_interface_synonym(name, &mut checkpoint)?;
            return Ok(project_ordinary_interface(at, name, synonym));
        }
        let names = registrations.names_for_kind(ORDINARY_INTERFACE_XML_TAG, &checkpoint)?;
        for name in &names {
            if !QualifiedAddress::parse(&format!("{at}.{name}")).is_ok_and(
                |member| matches!(member.segments(), [segment] if segment.name() == Some(name)),
            ) {
                return Err(ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "registered ordinary application interface has an invalid name",
                ));
            }
            self.read
                .ordinary_interface_synonym(name, &mut checkpoint)?;
        }
        project_registered_metadata_branch_names(at, &names, MemberSupport::default())
    }
}
