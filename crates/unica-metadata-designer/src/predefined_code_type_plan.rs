//! Final CodeType context for explicitly coupled calculation-chart edits.
use crate::decode::{MetadataPlanKind, MetadataPlanOperation};
use crate::predefined::PredefinedCodeType;
use std::collections::{BTreeMap, BTreeSet};
use unica_address::source_target::MetadataAddress;
use unica_metadata::{
    MetaEditOperation, MetaInfoPropertyValue, MetaPropertyKey, MetaPropertyValue, MetadataKind,
};
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyProposal, IndexedPlanOperation,
};
use unica_source_contract::SourceSetKind;
use unica_source_designer::layout::metadata_descriptor_relative;

#[derive(Default)]
pub struct PredefinedCodeTypePlan {
    render_types: BTreeMap<usize, PredefinedCodeType>,
    final_owners: BTreeMap<String, OwnerIntent>,
}
struct OwnerIntent {
    owner: MetadataAddress,
    index: usize,
    created: bool,
    code_type: PredefinedCodeType,
}
#[derive(Default)]
struct OwnerFold {
    created: bool,
    intent: Option<OwnerIntent>,
    predefined: Vec<usize>,
}
impl OwnerFold {
    fn finish(self, render_types: &mut BTreeMap<usize, PredefinedCodeType>) -> Option<OwnerIntent> {
        let intent = self.intent?;
        for index in self.predefined {
            render_types.insert(index, intent.code_type);
        }
        Some(intent)
    }
}
impl PredefinedCodeTypePlan {
    pub fn from_operations<'a>(
        operations: impl IntoIterator<Item = &'a IndexedPlanOperation<MetadataPlanOperation>>,
    ) -> Self {
        let mut plan = Self::default();
        let mut owners = BTreeMap::<String, OwnerFold>::new();
        for operation in operations {
            match &operation.operation().kind {
                MetadataPlanKind::Create {
                    kind: MetadataKind::ChartOfCalculationTypes,
                    name,
                } => {
                    let key = format!("ChartOfCalculationTypes.{name}");
                    if let Some(previous) = owners.remove(&key) {
                        previous.finish(&mut plan.render_types);
                    }
                    owners.insert(
                        key,
                        OwnerFold {
                            created: true,
                            ..OwnerFold::default()
                        },
                    );
                }
                MetadataPlanKind::Remove { target, .. } if is_calculation_chart(target) => {
                    if let Some(previous) = owners.remove(target.as_str()) {
                        previous.finish(&mut plan.render_types);
                    }
                }
                MetadataPlanKind::Edit {
                    target,
                    operation: MetaEditOperation::SetProperties { values },
                    ..
                } if is_calculation_chart(target) => {
                    let Some((_, MetaPropertyValue::String(value))) = values
                        .entries()
                        .iter()
                        .find(|(key, _)| *key == MetaPropertyKey::CodeType)
                    else {
                        continue;
                    };
                    let code_type = match value.as_str() {
                        "String" => PredefinedCodeType::String,
                        "Number" => PredefinedCodeType::Number,
                        _ => unreachable!(
                            "typed CodeType values are validated by the property codec"
                        ),
                    };
                    let owner = owners.entry(target.as_str().to_string()).or_default();
                    owner.intent = Some(OwnerIntent {
                        owner: target.clone(),
                        index: operation.index(),
                        created: owner.created,
                        code_type,
                    });
                }
                MetadataPlanKind::Predefined {
                    owner,
                    kind: MetadataKind::ChartOfCalculationTypes,
                    ..
                } => {
                    owners
                        .entry(owner.as_str().to_string())
                        .or_default()
                        .predefined
                        .push(operation.index());
                }
                _ => {}
            }
        }
        for (key, owner) in owners {
            if let Some(intent) = owner.finish(&mut plan.render_types) {
                plan.final_owners.insert(key, intent);
            }
        }
        plan
    }

    pub(crate) fn render_type(&self, operation_index: usize) -> Option<PredefinedCodeType> {
        self.render_types.get(&operation_index).copied()
    }

    /// Check only the final live incarnation, after all operation families.
    pub fn validate_final(
        &self,
        staged: &mut ApplyProposal<'_>,
        source_kind: SourceSetKind,
    ) -> Result<(), ApplyPlanError> {
        let changed_paths = staged
            .planned_changes()
            .into_iter()
            .map(|change| change.relative_path)
            .collect::<BTreeSet<_>>();
        for intent in self.final_owners.values() {
            let path = format!("ops[{}].args.values.CodeType", intent.index);
            let invalid_source = |message| {
                ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message).at_path(&path)
            };
            let relative = metadata_descriptor_relative(&intent.owner, source_kind)
                .map_err(invalid_source)?;
            let Some(current) = staged
                .read(&relative)
                .map_err(|error| ApplyPlanError::staging(error, &path))?
            else {
                continue;
            };
            let observed = observe_code_type(&current).map_err(invalid_source)?;
            let original = staged
                .original_image(&relative)
                .map(observe_code_type)
                .transpose()
                .map_err(invalid_source)?;
            let changed_type = intent.created
                || original
                    .as_ref()
                    .is_none_or(|original| original.raw != observed.raw);
            let companion = relative.with_extension("").join("Ext/Predefined.xml");
            let changed_companion = changed_paths.contains(&companion);
            // A no-op does not read or repair an unchanged companion.
            if !changed_type && !changed_companion {
                continue;
            }
            if changed_type
                && source_kind == SourceSetKind::Extension
                && super::final_object_inherits_composition(
                    MetadataKind::ChartOfCalculationTypes,
                    &current,
                )
            {
                return Err(invalid_source(
                    "cannot change borrowed chart CodeType: local data does not prove compatibility of inherited predefined codes".to_string(),
                ));
            }
            if !intent.created {
                original
                    .ok_or_else(|| {
                        invalid_source("CodeType source preimage is unavailable".to_string())
                    })?
                    .value
                    .map_err(invalid_source)?;
            }
            let code_type = observed.value.map_err(invalid_source)?;
            if code_type != intent.code_type {
                return Err(ApplyPlanError::new(
                    ApplyPlanErrorKind::Postcondition,
                    "final calculation-chart CodeType does not match the typed request",
                )
                .at_path(&path));
            }
            let Some(bytes) = staged
                .read(&companion)
                .map_err(|error| ApplyPlanError::staging(error, &path))?
            else {
                continue;
            };
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| invalid_source("Predefined.xml is not UTF-8".to_string()))?
                .trim_start_matches('\u{feff}');
            crate::predefined::validate_predefined_text_for_code_type(MetadataKind::ChartOfCalculationTypes, code_type, text).map_err(|message| {
                ApplyPlanError::new(ApplyPlanErrorKind::BadValue, format!("final CodeType is incompatible with Predefined.xml: {message}; explicitly update the affected predefined codes in the same apply")).at_path(&path)
            })?;
        }
        Ok(())
    }
}
fn is_calculation_chart(target: &MetadataAddress) -> bool {
    let mut parts = target.as_str().split('.');
    parts.next() == Some("ChartOfCalculationTypes")
        && parts.next().is_some()
        && parts.next().is_none()
}
struct CodeTypeObservation {
    raw: Vec<String>,
    value: Result<PredefinedCodeType, String>,
}
fn observe_code_type(bytes: &[u8]) -> Result<CodeTypeObservation, String> {
    use crate::info_projection::{
        direct_children_with_namespace, observe_meta_info_property, MD_CLASSES_NAMESPACE,
    };
    let (source, document) = unica_xml_format::read::parse_metadata_image(bytes)?;
    let root = document.root_element();
    if !root.has_tag_name((MD_CLASSES_NAMESPACE, "MetaDataObject")) {
        return Err("CodeType owner is not an MDClasses MetaDataObject".to_string());
    }
    let objects = root
        .children()
        .filter(roxmltree::Node::is_element)
        .collect::<Vec<_>>();
    let [object] = objects.as_slice() else {
        return Err("CodeType owner must contain exactly one calculation chart".to_string());
    };
    if !object.has_tag_name((MD_CLASSES_NAMESPACE, "ChartOfCalculationTypes")) {
        return Err("CodeType owner is not a calculation chart".to_string());
    }
    let properties = direct_children_with_namespace(*object, MD_CLASSES_NAMESPACE, "Properties");
    let [properties] = properties.as_slice() else {
        return Err("CodeType owner must contain exactly one Properties block".to_string());
    };
    let raw = properties
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "CodeType")
        .map(|node| source[node.range()].to_string())
        .collect();
    let value = observe_meta_info_property(
        *properties,
        MetadataKind::ChartOfCalculationTypes,
        "CodeType",
    )
    .map_err(|error| error.message)
    .and_then(|value| match value {
        Some(MetaInfoPropertyValue::String(value)) if value == "String" => {
            Ok(PredefinedCodeType::String)
        }
        Some(MetaInfoPropertyValue::String(value)) if value == "Number" => {
            Ok(PredefinedCodeType::Number)
        }
        None => Err(crate::predefined::MISSING_PREDEFINED_CODE_TYPE.to_string()),
        _ => Err("calculation-chart CodeType is not String or Number".to_string()),
    });
    Ok(CodeTypeObservation { raw, value })
}
