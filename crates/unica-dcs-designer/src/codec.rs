//! Designer-кодек схемы компоновки: `Ext/Template.xml` макета.
use unica_dcs::codec::{DcsCheckOptions, DcsCheckReport, DcsCodec};
use unica_dcs::edit::DcsEdit;
use unica_dcs::facts::DcsFacts;
use unica_metadata::support_state::ObjectSupportData;
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyProposal, ApplySourceAuthority, ProvisionalApplyEffect,
};
use unica_source_contract::SourceSetKind;

/// Схема макета в выгрузке Designer.
pub(crate) const SCHEMA_RESOURCE: &str = "Template.xml";

pub struct DesignerDcsCodec;

impl DcsCodec for DesignerDcsCodec {
    fn read(&self, text: &str, support: ObjectSupportData) -> Result<DcsFacts, String> {
        crate::schema::parse_dcs_info_xml(text, support)
    }

    fn check(&self, text: &str, options: DcsCheckOptions, label: &str) -> DcsCheckReport {
        crate::schema::validate_dcs_text(options, text, label)
    }

    fn stage(
        &self,
        staged: &mut ApplyProposal<'_>,
        authority: &dyn ApplySourceAuthority,
        edit: &DcsEdit,
        op_index: usize,
        provisional: &mut Vec<ProvisionalApplyEffect>,
    ) -> Result<(), ApplyPlanError> {
        crate::planner::stage_dcs_edit(staged, authority, edit, op_index, provisional)
    }

    fn check_staged(
        &self,
        staged: &mut ApplyProposal<'_>,
        source_kind: SourceSetKind,
        edits: &[(usize, &DcsEdit)],
    ) -> Result<(), ApplyPlanError> {
        crate::planner::validate_final_dcs_resources(staged, source_kind, edits.iter().copied())
    }
}
