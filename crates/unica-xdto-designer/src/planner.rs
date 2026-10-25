//! Canonical XDTO proposal planning; final source publication belongs to workspace.
use crate::validation::{validate, Finding, FindingSeverity, FindingState, ValidationDiff};
use crate::{decode, descriptor::descriptor_fields, encode_like, model::PackageModel, writer};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_metadata_designer::support::{
    parse_support_state_compat_bytes, support_root_uuid_from_bytes,
};
use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyProposal, ApplySourceAuthority, IndexedPlanOperation,
    ProvisionalApplyEffect,
};
use unica_source_contract::SourceSetKind;
use unica_source_designer::layout::{attached_resource_relative, metadata_descriptor_relative};
use unica_source_designer::owner::{
    prove_already_read_metadata_owner, prove_already_read_source_set_owner,
    PlatformXmlSourceSetOwnerEvidence,
};
use unica_xdto::operations::{
    XdtoOperation, XdtoPlanOperation, XdtoPropertyPath, XdtoRenamePropertyArgs,
};

pub fn decode_xdto_plan_operation(
    operation: XdtoOperation,
    value: &Value,
    op_index: usize,
    source_set_name: &str,
) -> Result<XdtoPlanOperation, ApplyPlanError> {
    operation
        .decode(value, &format!("ops[{op_index}].args"), source_set_name)
        .map_err(|error| bad_xdto_argument(&error.path, error.message))
}

fn bad_xdto_argument(path: &str, message: impl Into<String>) -> ApplyPlanError {
    ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(path)
}

pub fn plan_xdto_batch(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    operations: &[IndexedPlanOperation<XdtoPlanOperation>],
    validation_ledger: &mut XdtoValidationLedger,
) -> Result<Vec<ProvisionalApplyEffect>, ApplyPlanError> {
    if !authority.owns_staged_state(staged) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "staged source belongs to another apply admission",
        ));
    }
    if operations.is_empty() {
        return Err(bad_xdto_argument(
            "ops",
            "XDTO batch must contain at least one operation",
        ));
    }

    let mut package: Option<StagedXdtoPackage> = None;
    let mut provisional_effects = Vec::new();
    for indexed in operations {
        let op_index = indexed.index();
        let operation = indexed.operation();
        let at_path = format!("ops[{op_index}].args.at");
        let selected = staged_xdto_package(
            operation,
            authority.source_set_name(),
            authority.source_kind(),
            &at_path,
        )?;
        if package
            .as_ref()
            .is_some_and(|current| current.owner_at != selected.owner_at)
        {
            return Err(bad_xdto_argument(
                &at_path,
                "one XDTO batch cannot address more than one package",
            ));
        }
        package.get_or_insert_with(|| selected.clone());

        let descriptor_namespace = prove_staged_xdto_owner(staged, authority, &selected, &at_path)?;
        prove_staged_xdto_support(staged, authority.support_policy_mode(), &selected, &at_path)?;
        let before = staged
            .read(&selected.resource_relative)
            .map_err(|error| ApplyPlanError::staging(error, &at_path))?
            .ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::NotFound,
                    "XDTO package resource is absent",
                )
                .at_path(&at_path)
            })?;
        let before_text = decode(&before).map_err(|_| invalid_staged_xdto(&at_path))?;
        let before_model =
            PackageModel::parse(&before_text).map_err(|_| invalid_staged_xdto(&at_path))?;
        if before_model.target_namespace() != Some(descriptor_namespace.as_str()) {
            return Err(invalid_staged_xdto(&at_path));
        }
        validation_ledger.retain_original_baseline(
            staged,
            &selected,
            &before_model,
            &before,
            op_index,
            &at_path,
        )?;
        let writer_operation = staged_writer_operation(operation);
        let writer_plan = writer::plan_typed(&before_text, writer_operation)
            .map_err(|error| staged_writer_error(error, operation, op_index))?;
        let after_model = PackageModel::parse(&writer_plan.after)
            .map_err(|_| postcondition_xdto(&at_path, "writer produced an invalid XDTO package"))?;
        if after_model.target_namespace() != Some(descriptor_namespace.as_str()) {
            return Err(postcondition_xdto(
                &at_path,
                "writer changed the admitted XDTO namespace identity",
            ));
        }
        if writer_plan.blocks() {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidState,
                "XDTO operation conflicts with the package postimage",
            )
            .at_path(xdto_operation_identity_path(operation, op_index)));
        }
        let after = encode_like(&before, &writer_plan.after);
        if before != after {
            staged
                .replace(&selected.resource_relative, &before, after.clone())
                .map_err(|error| ApplyPlanError::staging(error, &at_path))?;
            if staged
                .read(&selected.resource_relative)
                .map_err(|error| ApplyPlanError::staging(error, &at_path))?
                .as_deref()
                != Some(after.as_slice())
            {
                return Err(postcondition_xdto(
                    &at_path,
                    "staged XDTO postimage did not remain exact",
                ));
            }
            provisional_effects.push(ProvisionalApplyEffect::single(
                selected.resource_relative.clone(),
                DomainEvent::new(DomainEventKind::MetadataChanged, selected.owner_at.clone()),
                op_index,
            ));
        }
        prove_staged_xdto_addition_repeat(&writer_plan.after, operation, &at_path)?;
        if let XdtoPlanOperation::RenameProperty(args) = operation {
            let keys = renamed_property_key(&before_model, args, args.name.as_str())
                .zip(renamed_property_key(
                    &after_model,
                    args,
                    args.new_name.as_str(),
                ))
                .ok_or_else(|| {
                    postcondition_xdto(&at_path, "renamed XDTO property was not found by its names")
                })?;
            validation_ledger.rename_property(&selected, &keys.0, &keys.1);
        }
        validation_ledger.record_postimage(&selected, validate(&after_model), &after, op_index);
    }

    Ok(provisional_effects)
}

/// Semantic references may be completed or removed by later operations. Only
/// the whole admitted batch postimage is compared with the retained original.
pub fn validate_final_xdto_resources<'a>(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    ledger: &XdtoValidationLedger,
    operations: impl Iterator<Item = &'a IndexedPlanOperation<XdtoPlanOperation>>,
) -> Result<(), ApplyPlanError> {
    if !authority.owns_staged_state(staged) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "staged source belongs to another apply admission",
        ));
    }
    let operations = operations
        .map(|indexed| (indexed.index(), indexed.operation()))
        .collect::<HashMap<_, _>>();
    let mut first_conflict: Option<(usize, ApplyPlanError)> = None;
    for resource in ledger.resources.values() {
        let at_path = format!("ops[{}].args.at", resource.last_operation_index);
        let namespace = prove_staged_xdto_owner(staged, authority, &resource.package, &at_path)?;
        prove_staged_xdto_support(
            staged,
            authority.support_policy_mode(),
            &resource.package,
            &at_path,
        )?;
        let after = staged
            .read(&resource.package.resource_relative)
            .map_err(|error| ApplyPlanError::staging(error, &at_path))?
            .ok_or_else(|| postcondition_xdto(&at_path, "final XDTO package is absent"))?;
        let digest: [u8; 32] = Sha256::digest(&after).into();
        if digest != resource.postimage_digest {
            return Err(postcondition_xdto(
                &at_path,
                "final XDTO package has no operation provenance",
            ));
        }
        let text = decode(&after).map_err(|_| invalid_staged_xdto(&at_path))?;
        let model = PackageModel::parse(&text).map_err(|_| invalid_staged_xdto(&at_path))?;
        if model.target_namespace() != Some(namespace.as_str()) {
            return Err(invalid_staged_xdto(&at_path));
        }
        let validation = ValidationDiff::between(&resource.baseline, validate(&model));
        let mut origins = resource.origins.clone();
        for finding in validation.findings {
            let key = (finding.code, finding.location.key);
            let origin = origins
                .get_mut(&key)
                .and_then(VecDeque::pop_front)
                .ok_or_else(|| {
                    postcondition_xdto(&at_path, "final XDTO finding has no provenance")
                })?;
            if finding.state != FindingState::Introduced
                || finding.severity != FindingSeverity::Error
            {
                continue;
            }
            let index = origin.ok_or_else(|| {
                postcondition_xdto(
                    &at_path,
                    "introduced XDTO finding has no operation provenance",
                )
            })?;
            let operation = operations.get(&index).ok_or_else(|| {
                postcondition_xdto(
                    &at_path,
                    "XDTO finding origin is outside the admitted batch",
                )
            })?;
            if first_conflict
                .as_ref()
                .is_none_or(|(first, _)| index < *first)
            {
                first_conflict = Some((
                    index,
                    ApplyPlanError::new(
                        ApplyPlanErrorKind::InvalidState,
                        "XDTO operation conflicts with the package postimage",
                    )
                    .at_path(xdto_validation_error_path(operation, index)),
                ));
            }
        }
    }
    match first_conflict {
        Some((_, error)) => Err(error),
        None => Ok(()),
    }
}

fn xdto_operation_at(operation: &XdtoPlanOperation) -> &QualifiedAddress {
    match operation {
        XdtoPlanOperation::AddValueType(args) => &args.at,
        XdtoPlanOperation::AddObjectType(args) => &args.at,
        XdtoPlanOperation::AddProperty(args) => &args.at,
        XdtoPlanOperation::RemoveType(args) => &args.at,
        XdtoPlanOperation::RemoveProperty(args) => &args.at,
        XdtoPlanOperation::RenameProperty(args) => &args.at,
    }
}

fn staged_xdto_package(
    operation: &XdtoPlanOperation,
    source_set: &str,
    source_kind: SourceSetKind,
    at_path: &str,
) -> Result<StagedXdtoPackage, ApplyPlanError> {
    let at = xdto_operation_at(operation);
    if at.source_set() != source_set {
        return Err(bad_xdto_argument(
            at_path,
            "at belongs to another actor-admitted source set",
        ));
    }
    let type_target = matches!(
        operation,
        XdtoPlanOperation::AddProperty(_)
            | XdtoPlanOperation::RemoveType(_)
            | XdtoPlanOperation::RemoveProperty(_)
            | XdtoPlanOperation::RenameProperty(_)
    );
    let package_name = match (type_target, at.segments()) {
        (false, [package]) if package.kind() == NodeKind::XdtoPackage => package.name(),
        (true, [package, type_segment])
            if package.kind() == NodeKind::XdtoPackage
                && type_segment.kind() == NodeKind::Type
                && type_segment.name().is_some() =>
        {
            package.name()
        }
        _ => None,
    }
    .ok_or_else(|| bad_xdto_argument(at_path, "at does not identify the required XDTO node"))?;
    let metadata = MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("XDTOPackage.{package_name}"),
    )
    .map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::Postcondition,
            "logical XDTO address did not map to a metadata identity",
        )
        .at_path(at_path)
    })?;
    let descriptor_relative =
        metadata_descriptor_relative(&metadata, source_kind).map_err(|_| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::ProviderUnavailable,
                "XDTO descriptor layout is unavailable for the admitted source",
            )
            .at_path(at_path)
        })?;
    let resource_relative = attached_resource_relative(&metadata, "Package.bin", source_kind)
        .map_err(|_| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::ProviderUnavailable,
                "XDTO package layout is unavailable for the admitted source",
            )
            .at_path(at_path)
        })?;
    Ok(StagedXdtoPackage {
        owner_at: format!("{source_set}:XDTOPackage.{package_name}"),
        package_name: package_name.to_string(),
        descriptor_relative,
        resource_relative,
    })
}

fn prove_staged_xdto_owner(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    package: &StagedXdtoPackage,
    at_path: &str,
) -> Result<String, ApplyPlanError> {
    let root_relative = Path::new("Configuration.xml");
    let root = staged
        .read(root_relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "source-set owner descriptor is absent",
            )
            .at_path(at_path)
        })?;
    let root_evidence =
        prove_already_read_source_set_owner(root_relative, &root, authority.source_kind())
            .map_err(|_| invalid_staged_xdto(at_path))?;
    require_staged_xdto_format(&root_evidence, authority.expected_format(), at_path)?;
    if !root_evidence.registers("XDTOPackage", &package.package_name) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::NotFound,
            "XDTO package is not registered by its source-set owner",
        )
        .at_path(at_path));
    }
    let descriptor = staged
        .read(&package.descriptor_relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path))?
        .ok_or_else(|| {
            ApplyPlanError::new(ApplyPlanErrorKind::NotFound, "XDTO descriptor is absent")
                .at_path(at_path)
        })?;
    let descriptor_evidence =
        prove_already_read_metadata_owner(&package.descriptor_relative, &descriptor)
            .map_err(|_| invalid_staged_xdto(at_path))?;
    require_staged_xdto_format(&descriptor_evidence, authority.expected_format(), at_path)?;
    if descriptor_evidence.artifact_kind() != "XDTOPackage"
        || descriptor_evidence.artifact_name() != Some(package.package_name.as_str())
        || support_root_uuid_from_bytes(&descriptor).is_none()
    {
        return Err(invalid_staged_xdto(at_path));
    }
    descriptor_fields(&descriptor)
        .map(|fields| fields.namespace)
        .map_err(|_| invalid_staged_xdto(at_path))
}

fn require_staged_xdto_format(
    evidence: &PlatformXmlSourceSetOwnerEvidence,
    expected_format: &str,
    at_path: &str,
) -> Result<(), ApplyPlanError> {
    if evidence.version() == Some(expected_format) {
        Ok(())
    } else {
        Err(invalid_staged_xdto(at_path))
    }
}

fn prove_staged_xdto_support(
    staged: &mut ApplyProposal<'_>,
    support_policy: unica_source_contract::proposal::SupportPolicyMode,
    package: &StagedXdtoPackage,
    at_path: &str,
) -> Result<(), ApplyPlanError> {
    let marker = staged
        .read(Path::new("Ext/ParentConfigurations.bin"))
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    if support_policy != unica_source_contract::proposal::SupportPolicyMode::Deny {
        return Ok(());
    }
    let Some(state) = parse_support_state_compat_bytes(marker.as_deref()) else {
        return Ok(());
    };
    if state.removed() {
        return Ok(());
    }
    if !state.global_editing_enabled() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "actor support policy denies editing this protected source",
        )
        .at_path(at_path));
    }
    let descriptor = staged
        .read(&package.descriptor_relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    if descriptor
        .as_deref()
        .and_then(support_root_uuid_from_bytes)
        .and_then(|uuid| state.object_rule(&uuid))
        == Some(0)
    {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "actor support policy denies editing this protected XDTO package",
        )
        .at_path(at_path));
    }
    Ok(())
}

/// Describe object type additions from the final staged bytes, including
/// later operations from other runs in the same apply request and equivalent
/// no-op additions.
pub fn final_object_addition_details(
    staged: &mut ApplyProposal<'_>,
    source_set: &str,
    source_kind: SourceSetKind,
    operations: &[XdtoPlanOperation],
) -> Result<Vec<unica_xdto::results::XdtoObjectPlanDetail>, ApplyPlanError> {
    let mut seen = std::collections::BTreeSet::new();
    let mut details = Vec::new();
    for operation in operations {
        let XdtoPlanOperation::AddObjectType(args) = operation else {
            continue;
        };
        if !seen.insert((args.at.to_string(), args.name.as_str().to_string())) {
            continue;
        }
        let selected = staged_xdto_package(operation, source_set, source_kind, "ops")?;
        let mut detail = unica_xdto::results::XdtoObjectPlanDetail {
            at: args.at.to_string(),
            name: args.name.as_str().to_string(),
            present: false,
            base: None,
            previous: None,
            next: None,
        };
        if let Some(bytes) = staged
            .read(&selected.resource_relative)
            .map_err(|error| ApplyPlanError::staging(error, "ops"))?
        {
            let text = decode(&bytes).map_err(|_| invalid_staged_xdto("ops"))?;
            let model = PackageModel::parse(&text).map_err(|_| invalid_staged_xdto("ops"))?;
            let objects = model
                .types
                .iter()
                .filter(|named| named.kind == crate::model::TypeKind::Object)
                .collect::<Vec<_>>();
            if let Some(index) = objects.iter().position(|named| {
                named
                    .name
                    .as_ref()
                    .is_some_and(|name| name.value == args.name.as_str())
            }) {
                detail.present = true;
                detail.base = objects[index].base.as_ref().map(crate::info::qname_info);
                detail.previous = index
                    .checked_sub(1)
                    .and_then(|index| objects[index].name.as_ref())
                    .map(|name| name.value.clone());
                detail.next = objects
                    .get(index + 1)
                    .and_then(|named| named.name.as_ref())
                    .map(|name| name.value.clone());
            }
        }
        details.push(detail);
    }
    Ok(details)
}

pub fn staged_writer_operation(operation: &XdtoPlanOperation) -> writer::TypedWriterOperation<'_> {
    match operation {
        XdtoPlanOperation::AddValueType(args) => writer::TypedWriterOperation::AddValueType {
            name: args.name.as_str(),
            base: args.base.as_str(),
            facets: Some(&args.facets),
        },
        XdtoPlanOperation::AddObjectType(args) => writer::TypedWriterOperation::AddObjectType {
            name: args.name.as_str(),
            base: args.base.as_ref().map(|base| base.as_str()),
            before: args.before.as_ref().map(|before| before.as_str()),
            after: args.after.as_ref().map(|after| after.as_str()),
        },
        XdtoPlanOperation::AddProperty(args) => writer::TypedWriterOperation::AddPropertyTree {
            type_name: args.at.segments()[1]
                .name()
                .expect("closed XDTO type target has a name"),
            property: &args.property,
            property_path: args.property_path.as_ref().map(XdtoPropertyPath::as_str),
        },
        XdtoPlanOperation::RemoveType(args) => writer::TypedWriterOperation::RemoveType {
            name: args.at.segments()[1]
                .name()
                .expect("closed XDTO type target has a name"),
        },
        XdtoPlanOperation::RemoveProperty(args) => writer::TypedWriterOperation::RemoveProperty {
            type_name: args.at.segments()[1]
                .name()
                .expect("closed XDTO type target has a name"),
            name: args.name.as_str(),
            property_path: args.property_path.as_ref().map(XdtoPropertyPath::as_str),
        },
        XdtoPlanOperation::RenameProperty(args) => writer::TypedWriterOperation::RenameProperty {
            type_name: args.at.segments()[1]
                .name()
                .expect("closed XDTO type target has a name"),
            name: args.name.as_str(),
            property_path: args.property_path.as_ref().map(XdtoPropertyPath::as_str),
            new_name: args.new_name.as_str(),
        },
    }
}

pub fn staged_writer_error(
    error: writer::WriterError,
    operation: &XdtoPlanOperation,
    op_index: usize,
) -> ApplyPlanError {
    let kind = match error.cause() {
        writer::WriterErrorCause::BadValue => ApplyPlanErrorKind::BadValue,
        writer::WriterErrorCause::NotFound => ApplyPlanErrorKind::NotFound,
        writer::WriterErrorCause::AmbiguousTarget => ApplyPlanErrorKind::InvalidState,
        writer::WriterErrorCause::InvalidSource => ApplyPlanErrorKind::InvalidSource,
        writer::WriterErrorCause::Postcondition => ApplyPlanErrorKind::Postcondition,
    };
    let base = format!("ops[{op_index}].args");
    let path = match error.field() {
        writer::WriterErrorField::Source | writer::WriterErrorField::TypeTarget => {
            format!("{base}.at")
        }
        writer::WriterErrorField::Base => format!("{base}.values.base"),
        writer::WriterErrorField::Before => format!("{base}.values.before"),
        writer::WriterErrorField::After => format!("{base}.values.after"),
        writer::WriterErrorField::Facets => format!("{base}.values.facets"),
        // The writer checks the whole tree; a top-level anonymous ValueType
        // is named exactly, a nested one by the property that holds it.
        writer::WriterErrorField::ValueType => match operation {
            XdtoPlanOperation::AddProperty(args)
                if matches!(
                    args.property.definition,
                    unica_xdto::operations::XdtoPropertyDefinition::Value { .. }
                ) =>
            {
                format!("{base}.values.property.valueType")
            }
            _ => format!("{base}.values.property"),
        },
        // Цепочка свойств и имя удаляемого свойства задаются адресом.
        writer::WriterErrorField::PropertyPath => format!("{base}.at"),
        writer::WriterErrorField::PropertyName => match operation {
            XdtoPlanOperation::AddProperty(_) => format!("{base}.values.property.name"),
            _ => format!("{base}.at"),
        },
    };
    // Цепочка свойств адреса не дошла до вложенного типа: узел есть, но
    // свойства в него не кладутся — это говорится прямо, а не общим отказом.
    let message = match error.field() {
        writer::WriterErrorField::PropertyPath => "the addressed XDTO property chain does not reach a single nested anonymous type (typeDef); address a type or a property that declares one".to_string(),
        // An unsupported base and facet profile is the caller's to fix, so
        // the refusal says which profiles are supported.
        writer::WriterErrorField::Facets | writer::WriterErrorField::ValueType => error.to_string(),
        _ => "XDTO writer rejected the typed operation".to_string(),
    };
    ApplyPlanError::new(kind, message).at_path(path)
}

fn xdto_operation_identity_path(operation: &XdtoPlanOperation, op_index: usize) -> String {
    let base = format!("ops[{op_index}].args");
    match operation {
        XdtoPlanOperation::AddValueType(_) | XdtoPlanOperation::AddObjectType(_) => {
            format!("{base}.values.name")
        }
        XdtoPlanOperation::AddProperty(_) => format!("{base}.values.property.name"),
        XdtoPlanOperation::RenameProperty(_) => format!("{base}.values.newName"),
        XdtoPlanOperation::RemoveType(_) | XdtoPlanOperation::RemoveProperty(_) => {
            format!("{base}.at")
        }
    }
}

fn xdto_validation_error_path(operation: &XdtoPlanOperation, op_index: usize) -> String {
    let base = format!("ops[{op_index}].args");
    match operation {
        XdtoPlanOperation::AddValueType(_) => format!("{base}.values.base"),
        XdtoPlanOperation::AddObjectType(_) => format!("{base}.at"),
        // The final package names no child, so an anonymous type answers
        // at the field that declares its children.
        XdtoPlanOperation::AddProperty(args) => match args.property.definition {
            unica_xdto::operations::XdtoPropertyDefinition::Named(_) => {
                format!("{base}.values.property.type")
            }
            unica_xdto::operations::XdtoPropertyDefinition::Object(_) => {
                format!("{base}.values.property.objectType")
            }
            unica_xdto::operations::XdtoPropertyDefinition::Value { .. } => {
                format!("{base}.values.property.valueType")
            }
        },
        XdtoPlanOperation::RenameProperty(_) => format!("{base}.values.newName"),
        XdtoPlanOperation::RemoveType(_) | XdtoPlanOperation::RemoveProperty(_) => {
            format!("{base}.at")
        }
    }
}

fn prove_staged_xdto_addition_repeat(
    after: &str,
    operation: &XdtoPlanOperation,
    at_path: &str,
) -> Result<(), ApplyPlanError> {
    if !matches!(
        operation,
        XdtoPlanOperation::AddValueType(_)
            | XdtoPlanOperation::AddObjectType(_)
            | XdtoPlanOperation::AddProperty(_)
    ) {
        return Ok(());
    }
    let repeated = writer::plan_typed(after, staged_writer_operation(operation))
        .map_err(|_| postcondition_xdto(at_path, "XDTO addition cannot be repeated safely"))?;
    if repeated.after != after || !repeated.edits.is_empty() || repeated.blocks() {
        return Err(postcondition_xdto(
            at_path,
            "XDTO addition did not prove an exact repeat no-op",
        ));
    }
    Ok(())
}

/// The validation key of the renamed property under one of its names. Findings
/// below this key belong to the same declaration before and after the rename.
fn renamed_property_key(
    model: &PackageModel,
    args: &XdtoRenamePropertyArgs,
    name: &str,
) -> Option<String> {
    let named = |property: &&crate::model::Property, name: &str| {
        property
            .name
            .as_ref()
            .is_some_and(|value| value.value == name)
    };
    let type_name = args.at.segments()[1].name()?;
    let mut properties = &model
        .types
        .iter()
        .find(|named| {
            named.kind == crate::model::TypeKind::Object
                && named
                    .name
                    .as_ref()
                    .is_some_and(|value| value.value == type_name)
        })?
        .properties;
    for parent in args.property_path.iter().flat_map(XdtoPropertyPath::names) {
        properties = &properties
            .iter()
            .find(|property| named(property, &parent))?
            .type_defs
            .first()?
            .properties;
    }
    properties
        .iter()
        .find(|property| named(property, name))
        .map(|property| property.key.clone())
}

fn invalid_staged_xdto(path: &str) -> ApplyPlanError {
    ApplyPlanError::new(
        ApplyPlanErrorKind::InvalidSource,
        "admitted XDTO owner or package evidence is invalid",
    )
    .at_path(path)
}

fn postcondition_xdto(path: &str, message: impl Into<String>) -> ApplyPlanError {
    ApplyPlanError::new(ApplyPlanErrorKind::Postcondition, message).at_path(path)
}

#[derive(Clone)]
struct StagedXdtoPackage {
    owner_at: String,
    package_name: String,
    descriptor_relative: PathBuf,
    resource_relative: PathBuf,
}

/// Request-local provenance for the package's final semantic validation. This
/// survives separate XDTO runs within the same admitted public batch.
#[derive(Default)]
pub struct XdtoValidationLedger {
    resources: BTreeMap<PathBuf, XdtoResourceValidation>,
}

struct XdtoResourceValidation {
    package: StagedXdtoPackage,
    baseline: Vec<Finding>,
    origins: HashMap<(String, String), VecDeque<Option<usize>>>,
    postimage_digest: [u8; 32],
    last_operation_index: usize,
}

impl XdtoValidationLedger {
    pub fn is_empty(&self) -> bool {
        self.resources.is_empty()
    }

    fn retain_original_baseline(
        &mut self,
        staged: &ApplyProposal<'_>,
        package: &StagedXdtoPackage,
        before_model: &PackageModel,
        before: &[u8],
        op_index: usize,
        at_path: &str,
    ) -> Result<(), ApplyPlanError> {
        if self.resources.contains_key(&package.resource_relative) {
            return Ok(());
        }
        let baseline = match staged.original_image(&package.resource_relative) {
            Some(original) => {
                let text = decode(original).map_err(|_| invalid_staged_xdto(at_path))?;
                let model = PackageModel::parse(&text).map_err(|_| invalid_staged_xdto(at_path))?;
                validate(&model)
            }
            None => Vec::new(),
        };
        let mut origins = HashMap::<_, VecDeque<_>>::new();
        for finding in validate(before_model) {
            origins
                .entry((finding.code, finding.location.key))
                .or_default()
                .push_back(None);
        }
        self.resources.insert(
            package.resource_relative.clone(),
            XdtoResourceValidation {
                package: package.clone(),
                baseline,
                origins,
                postimage_digest: Sha256::digest(before).into(),
                last_operation_index: op_index,
            },
        );
        Ok(())
    }

    /// A rename moves the declaration's findings to its new key. Its own
    /// pre-existing findings stay pre-existing and keep their provenance.
    fn rename_property(&mut self, package: &StagedXdtoPackage, old: &str, new: &str) {
        if old == new {
            return;
        }
        let resource = self
            .resources
            .get_mut(&package.resource_relative)
            .expect("the XDTO preimage retained its original baseline");
        let moved = |key: &str| {
            key.strip_prefix(old)
                .filter(|rest| rest.is_empty() || rest.starts_with('/'))
                .map(|rest| format!("{new}{rest}"))
        };
        for finding in &mut resource.baseline {
            if let Some(key) = moved(&finding.location.key) {
                finding.location.key = key;
            }
        }
        resource.origins = std::mem::take(&mut resource.origins).into_iter().fold(
            HashMap::new(),
            |mut origins, ((code, key), queue)| {
                let key = moved(&key).unwrap_or(key);
                origins
                    .entry((code, key))
                    .or_insert_with(VecDeque::new)
                    .extend(queue);
                origins
            },
        );
    }

    fn record_postimage(
        &mut self,
        package: &StagedXdtoPackage,
        findings: Vec<Finding>,
        after: &[u8],
        op_index: usize,
    ) {
        let resource = self
            .resources
            .get_mut(&package.resource_relative)
            .expect("the XDTO preimage retained its original baseline");
        let mut prior = std::mem::take(&mut resource.origins);
        for finding in findings {
            let key = (finding.code, finding.location.key);
            let origin = prior
                .get_mut(&key)
                .and_then(VecDeque::pop_front)
                .unwrap_or(Some(op_index));
            resource.origins.entry(key).or_default().push_back(origin);
        }
        resource.postimage_digest = Sha256::digest(after).into();
        resource.last_operation_index = op_index;
    }
}

#[cfg(test)]
mod tests {
    use super::{staged_writer_error, staged_writer_operation};
    use crate::writer::plan_typed;
    use serde_json::json;
    use unica_source_contract::proposal::ApplyPlanErrorKind;
    use unica_xdto::operations::XdtoOperation;

    const PACKAGE: &str = r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:tns="urn:test" targetNamespace="urn:test"><objectType name="T"/></package>"#;

    /// An unsupported base and facet profile is a refusal of the request at
    /// the field that asked for it, with the supported profiles named.
    #[test]
    fn unsupported_facet_profiles_are_refused_at_their_request_fields() {
        for (operation, args, path, message) in [
            (
                XdtoOperation::AddValueType,
                json!({"at":"main:XDTOPackage.Sample","values":{"name":"V","base":"xs:string","facets":{"fractionDigits":2}}}),
                "ops[3].args.values.facets",
                "supported profiles are string and decimal",
            ),
            (
                XdtoOperation::AddProperty,
                json!({"at":"main:XDTOPackage.Sample.Type.T","values":{"property":{"name":"P","valueType":{"base":"tns:T"}}}}),
                "ops[3].args.values.property.valueType",
                "anonymous ValueType supports only XML Schema string or decimal bases",
            ),
            (
                XdtoOperation::AddProperty,
                json!({"at":"main:XDTOPackage.Sample.Type.T","values":{"property":{"name":"P","objectType":{"properties":[{"name":"C","valueType":{"base":"tns:T"}}]}}}}),
                "ops[3].args.values.property",
                "anonymous ValueType supports only XML Schema string or decimal bases",
            ),
        ] {
            let decoded = operation.decode(&args, "ops[3].args", "main").unwrap();
            let error = match plan_typed(PACKAGE, staged_writer_operation(&decoded)) {
                Err(error) => error,
                Ok(_) => panic!("{args}: an unsupported profile is refused"),
            };
            let refusal = staged_writer_error(error, &decoded, 3);
            assert_eq!(refusal.kind(), ApplyPlanErrorKind::BadValue, "{args}");
            assert_eq!(refusal.path(), Some(path), "{args}");
            assert!(refusal.to_string().contains(message), "{refusal}");
        }
    }
}
