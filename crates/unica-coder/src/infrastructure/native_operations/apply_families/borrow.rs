use std::path::{Path, PathBuf};
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_metadata::contracts::metadata_borrow_input_schema;
use unica_metadata_designer::borrowed_enum_value::{
    enum_value_properties, enum_value_source_uuid, plan_borrow_enum_value, EnumValueLookupError,
    EnumValueLookupErrorKind,
};
use unica_operation_contract::ArgumentShape;
use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::proposal::ProvisionalApplyEffect;
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind};
use unica_source_contract::SourceSetKind;
use unica_workspace::actor::{ApplyAdmission, ProviderRootBinding};
use unica_workspace::apply::ApplyStagedState;

#[derive(Debug, Clone)]
pub(super) struct BorrowOperation {
    parent: QualifiedAddress,
    overrides: Option<Vec<String>>,
    index: usize,
}

pub(super) fn parse(
    args: &serde_json::Value,
    index: usize,
    binding: &ProviderRootBinding,
) -> Result<BorrowOperation, ApplyPlanError> {
    let fail = |message: &str| {
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message)
            .at_path(format!("ops[{index}].args"))
    };
    if binding.source_kind() != SourceSetKind::Extension {
        return Err(fail("object.borrow requires an extension destination"));
    }
    super::validate_platform_xml_binding(binding, "object.borrow", index)?;
    let args = args
        .as_object()
        .ok_or_else(|| fail("borrow args must be an object"))?;
    let shape = ArgumentShape::new(metadata_borrow_input_schema());
    if let Some(key) = args.keys().find(|key| shape.property(key).is_none()) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!("unknown object.borrow argument `{key}`"),
        )
        .at_path(format!("ops[{index}].args.{key}")));
    }
    let target = QualifiedAddress::parse(
        args.get("at")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default(),
    )
    .map_err(|error| fail(&error.to_string()))?;
    if target.segments().len() != 1 || target.segments()[0].kind() != NodeKind::Configuration {
        return Err(fail(
            "object.borrow target must be the extension Configuration root",
        ));
    }
    let parent = QualifiedAddress::parse(
        args.get("from")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default(),
    )
    .map_err(|error| fail(&error.to_string()))?;
    if !crate::domain::apply::valid_borrow_source(&parent, binding.source_set_name()) {
        return Err(fail(
            "from must name a top-level object, Enum.EnumValue or object Form in a distinct parent source set",
        ));
    }
    shape
        .property("from")
        .expect("borrow from field")
        .validate_field(&args["from"], &format!("ops[{index}].args"))
        .map_err(|error| fail(&error.message))?;
    if parent.segments().len() == 2 && args.contains_key("overrides") {
        return Err(fail(
            "overrides are not supported when borrowing an EnumValue or a Form",
        ));
    }
    let overrides = args
        .get("overrides")
        .map(|value| {
            let array = value
                .as_array()
                .ok_or_else(|| fail("overrides must be an array of property names"))?;
            let overrides_shape = shape.property("overrides").expect("borrow overrides field");
            overrides_shape
                .validate_field(value, &format!("ops[{index}].args"))
                .map_err(|error| fail(&error.message))?;
            let item_shape = overrides_shape.items().expect("borrow override item");
            array
                .iter()
                .map(|value| -> Result<String, ApplyPlanError> {
                    let name = value
                        .as_str()
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                        .ok_or_else(|| fail("override must be a nonempty property name"))?;
                    item_shape
                        .validate_field(value, &format!("ops[{index}].args"))
                        .map_err(|error| fail(&error.message))?;
                    Ok(name)
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    if let Some(overrides) = &overrides {
        unica_metadata_designer::borrowed_object::validate_requested_overrides(overrides)
            .map_err(|error| fail(&error.to_string()))?;
    }
    Ok(BorrowOperation {
        parent,
        overrides,
        index,
    })
}

fn registered(owner: &[u8], kind: &str, name: &str) -> Result<bool, ApplyPlanError> {
    let text = std::str::from_utf8(owner)
        .map_err(|error| ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error.to_string()))?
        .trim_start_matches('\u{feff}');
    let document = roxmltree::Document::parse(text).map_err(|error| {
        ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error.to_string())
    })?;
    const NS: &str = "http://v8.1c.ru/8.3/MDClasses";
    let root = document.root_element();
    let objects = root
        .children()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    if !root.has_tag_name((NS, "MetaDataObject"))
        || root.attribute("version") != Some("2.20")
        || objects.len() != 1
        || !objects[0].has_tag_name((NS, "Configuration"))
    {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "parent/destination owner is not a supported Configuration descriptor",
        ));
    }
    let blocks = objects[0]
        .children()
        .filter(|node| node.has_tag_name((NS, "ChildObjects")))
        .collect::<Vec<_>>();
    if blocks.len() != 1 {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "configuration must contain one ChildObjects block",
        ));
    }
    let count = blocks[0]
        .children()
        .filter(|node| node.has_tag_name((NS, kind)) && node.text() == Some(name))
        .count();
    if count > 1 {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "configuration registers the parent object ambiguously",
        ));
    }
    Ok(count == 1)
}

fn enum_value_lookup_error(
    error: EnumValueLookupError,
    parent: &QualifiedAddress,
) -> ApplyPlanError {
    let kind = match error.kind {
        EnumValueLookupErrorKind::NotFound => ApplyPlanErrorKind::NotFound,
        EnumValueLookupErrorKind::InvalidSource => ApplyPlanErrorKind::InvalidSource,
    };
    ApplyPlanError::new(kind, error.to_string()).at_path(parent.to_string())
}

pub(super) fn plan(
    staged: &mut ApplyStagedState,
    admission: &ApplyAdmission,
    binding: &ProviderRootBinding,
    operation: &BorrowOperation,
) -> Result<
    (
        Vec<ProvisionalApplyEffect>,
        unica_source_contract::proposal::BorrowPlanDetail,
    ),
    ApplyPlanError,
> {
    admission.metadata_planning_authority(binding)?;
    if parent_form_name(&operation.parent).is_some() {
        return plan_form(staged, admission, binding, operation);
    }
    plan_object(
        staged,
        admission,
        binding,
        &operation.parent,
        operation.overrides.as_deref(),
        operation.index,
    )
}

fn parent_form_name(parent: &QualifiedAddress) -> Option<&str> {
    parent
        .segments()
        .get(1)
        .filter(|segment| segment.kind() == NodeKind::Form)
        .and_then(|segment| segment.name())
}

/// Borrows or refreshes a top-level object or one of an enum's values.
fn plan_object(
    staged: &mut ApplyStagedState,
    admission: &ApplyAdmission,
    binding: &ProviderRootBinding,
    parent: &QualifiedAddress,
    requested_overrides: Option<&[String]>,
    index: usize,
) -> Result<
    (
        Vec<ProvisionalApplyEffect>,
        unica_source_contract::proposal::BorrowPlanDetail,
    ),
    ApplyPlanError,
> {
    let owner = &parent.segments()[0];
    let kind = owner.kind().as_str();
    let name = owner.name().expect("parsed borrow has object name");
    let at = parent.to_string();
    let layout = unica_source_designer::layout::metadata_kind(kind).ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "from is not a supported top-level metadata object",
        )
        .at_path(at.clone())
    })?;
    let relative = PathBuf::from(layout.directory).join(format!("{name}.xml"));
    let parent_owner =
        admission.read_apply_dependency(parent.source_set(), Path::new("Configuration.xml"))?;
    if !registered(&parent_owner, kind, name)? {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::NotFound,
            "parent object is not registered in its configuration",
        )
        .at_path(at));
    }
    let parent_bytes = admission.read_apply_dependency(parent.source_set(), &relative)?;
    if let Some(finding) = crate::infrastructure::format_guard::classify_staged_platform_xml_root(
        &relative,
        &parent_bytes,
    ) {
        return Err(
            ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, finding.message)
                .at_path(parent.to_string()),
        );
    }
    let owner_path = PathBuf::from("Configuration.xml");
    let destination_owner = staged
        .read(&owner_path)
        .map_err(|error| ApplyPlanError::staging(error, "Configuration"))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "extension Configuration descriptor is missing",
            )
        })?;
    let existing = staged
        .read(&relative)
        .map_err(|error| ApplyPlanError::staging(error, parent.to_string()))?;
    if let Some(finding) = existing.as_deref().and_then(|bytes| {
        crate::infrastructure::format_guard::classify_staged_platform_xml_root(&relative, bytes)
    }) {
        return Err(
            ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, finding.message)
                .at_path(format!("{}:{kind}.{name}", binding.source_set_name())),
        );
    }
    let is_registered = registered(&destination_owner, kind, name)?;
    if is_registered != existing.is_some() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "extension object registration and descriptor disagree",
        ));
    }
    let overrides =
        serde_json::to_vec(&requested_overrides).expect("parsed borrow overrides are serializable");
    let identity = plan_identity(
        staged,
        parent,
        &[
            binding.source_set_name().as_bytes(),
            at.as_bytes(),
            &parent_owner,
            &parent_bytes,
            &overrides,
        ],
        index,
    )?;
    let mut borrowed = unica_metadata_designer::borrowed_object::plan_borrow_object_with_identity(
        &parent_bytes,
        existing.as_deref(),
        requested_overrides,
        &identity,
        crate::infrastructure::source_ids::fresh_uuid,
    )
    .map_err(|error| {
        let kind = match error.kind {
            unica_metadata_designer::borrowed_object::BorrowObjectErrorKind::BadValue => {
                ApplyPlanErrorKind::BadValue
            }
            unica_metadata_designer::borrowed_object::BorrowObjectErrorKind::InvalidSource => {
                ApplyPlanErrorKind::InvalidSource
            }
        };
        ApplyPlanError::new(kind, error.to_string()).at_path(
            if kind == ApplyPlanErrorKind::BadValue {
                format!("ops[{index}].args")
            } else {
                parent.to_string()
            },
        )
    })?;
    if borrowed.kind != kind || borrowed.name != name {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "registered parent address and descriptor identity disagree",
        ));
    }
    let value_name = parent.segments().get(1).and_then(|value| value.name());
    let mut result_at = format!("{}:{kind}.{name}", binding.source_set_name());
    let mut result_parent_uuid = borrowed.parent_uuid.clone();
    let mut created = existing.is_none();
    let mut previous_value_properties = None;
    if let Some(value_name) = value_name {
        result_parent_uuid = enum_value_source_uuid(&parent_bytes, value_name)
            .map_err(|error| enum_value_lookup_error(error, parent))?;
        previous_value_properties = existing
            .as_deref()
            .map(|bytes| enum_value_properties(bytes, value_name))
            .transpose()
            .map_err(|error| enum_value_lookup_error(error, parent))?
            .flatten();
        created = previous_value_properties.is_none();
        borrowed.bytes = plan_borrow_enum_value(
            &parent_bytes,
            &borrowed.bytes,
            value_name,
            &identity.new_uuid(
                "enum-value",
                &parent.to_string(),
                crate::infrastructure::source_ids::fresh_uuid,
            ),
        )
        .map_err(|error| {
            ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error)
                .at_path(parent.to_string())
        })?;
        result_at.push_str(&format!(".EnumValue.{value_name}"));
    }
    let previous_properties = if value_name.is_some() {
        previous_value_properties.unwrap_or_default()
    } else {
        existing
            .as_deref()
            .map(descriptor_properties)
            .transpose()?
            .unwrap_or_default()
    };
    let next_properties = if let Some(value_name) = value_name {
        enum_value_properties(&borrowed.bytes, value_name)
            .map_err(|error| enum_value_lookup_error(error, parent))?
            .ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidSource,
                    "planned EnumValue is missing from the borrowed descriptor",
                )
                .at_path(parent.to_string())
            })?
    } else {
        descriptor_properties(&borrowed.bytes)?
    };
    let changed_properties = previous_properties
        .keys()
        .chain(next_properties.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|key| previous_properties.get(key) != next_properties.get(key))
        .collect();
    let detail = unica_source_contract::proposal::BorrowPlanDetail {
        index,
        at: result_at,
        from: parent.to_string(),
        parent_uuid: result_parent_uuid,
        created,
        changed_properties,
        protected_overrides: if value_name.is_some() {
            Vec::new()
        } else {
            unica_metadata_designer::xml_model::parse_meta_borrowing(&borrowed.bytes)
                .map_err(|error| ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error))?
                .overrides
        },
    };
    let mut effects = Vec::new();
    if existing.as_deref() != Some(borrowed.bytes.as_slice()) {
        match existing {
            Some(previous) => staged.replace(&relative, &previous, borrowed.bytes),
            None => staged.create(&relative, borrowed.bytes),
        }
        .map_err(|error| ApplyPlanError::staging(error, parent.to_string()))?;
        effects.push(ProvisionalApplyEffect::single(
            &relative,
            DomainEvent::new(
                DomainEventKind::MetadataChanged,
                format!("{}:{kind}.{name}", binding.source_set_name()),
            ),
            index,
        ));
    }
    if !is_registered {
        let updated = unica_source_designer::registration::owner_registration_image(
            &destination_owner,
            kind,
            name,
            true,
            index,
        )?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                "new borrow could not register its object",
            )
        })?;
        staged
            .replace(&owner_path, &destination_owner, updated)
            .map_err(|error| ApplyPlanError::staging(error, "Configuration"))?;
        effects.push(ProvisionalApplyEffect::single(
            owner_path,
            DomainEvent::new(
                DomainEventKind::MetadataChanged,
                format!("{}:Configuration", binding.source_set_name()),
            ),
            index,
        ));
    }
    Ok((effects, detail))
}

/// Borrows an object's managed form with its own minimal content, borrowing
/// the owner first when the extension does not have it yet (#1982).
fn plan_form(
    staged: &mut ApplyStagedState,
    admission: &ApplyAdmission,
    binding: &ProviderRootBinding,
    operation: &BorrowOperation,
) -> Result<
    (
        Vec<ProvisionalApplyEffect>,
        unica_source_contract::proposal::BorrowPlanDetail,
    ),
    ApplyPlanError,
> {
    use unica_metadata_designer::borrowed_form::{self, BorrowFormError};
    let index = operation.index;
    let parent = &operation.parent;
    let from = parent.to_string();
    let owner_address = parent.prefix(1);
    let kind = owner_address.segments()[0].kind().as_str();
    let name = owner_address.segments()[0]
        .name()
        .expect("parsed borrow has object name");
    let form = parent_form_name(parent).expect("form borrow has a form name");
    let layout = unica_source_designer::layout::metadata_kind(kind).ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "from is not a form of a supported top-level metadata object",
        )
        .at_path(from.clone())
    })?;
    let owner_relative = PathBuf::from(layout.directory).join(format!("{name}.xml"));
    let forms = PathBuf::from(layout.directory).join(name).join("Forms");
    let form_relative = forms.join(format!("{form}.xml"));
    let content_relative = forms.join(form).join("Ext").join("Form.xml");
    let form_error = |error: BorrowFormError| {
        let kind = match error {
            BorrowFormError::NotManaged => ApplyPlanErrorKind::BadValue,
            BorrowFormError::InvalidSource(_) => ApplyPlanErrorKind::InvalidSource,
        };
        ApplyPlanError::new(kind, error.to_string()).at_path(from.clone())
    };

    let parent_configuration =
        admission.read_apply_dependency(parent.source_set(), Path::new("Configuration.xml"))?;
    if !registered(&parent_configuration, kind, name)? {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::NotFound,
            "parent object is not registered in its configuration",
        )
        .at_path(from));
    }
    let parent_owner = admission.read_apply_dependency(parent.source_set(), &owner_relative)?;
    let parent_owner_uuid = object_ids(&parent_owner).0.ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "parent object descriptor has no uuid",
        )
        .at_path(from.clone())
    })?;
    if !object_registers_form(&parent_owner, form)? {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::NotFound,
            "parent object does not register this form",
        )
        .at_path(from));
    }
    let parent_form_bytes = admission.read_apply_dependency(parent.source_set(), &form_relative)?;
    if let Some(finding) = crate::infrastructure::format_guard::classify_staged_platform_xml_root(
        &form_relative,
        &parent_form_bytes,
    ) {
        return Err(
            ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, finding.message).at_path(from),
        );
    }
    let parent_form = borrowed_form::parent_form(&parent_form_bytes, form).map_err(form_error)?;

    let mut effects = Vec::new();
    let owner_at = format!("{}:{kind}.{name}", binding.source_set_name());
    let owner_existing = staged
        .read(&owner_relative)
        .map_err(|error| ApplyPlanError::staging(error, owner_at.clone()))?;
    match owner_existing.as_deref() {
        None => {
            let (owner_effects, _) =
                plan_object(staged, admission, binding, &owner_address, None, index)?;
            effects.extend(owner_effects);
        }
        Some(owner) => {
            if let Some(finding) =
                crate::infrastructure::format_guard::classify_staged_platform_xml_root(
                    &owner_relative,
                    owner,
                )
            {
                return Err(ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidSource,
                    finding.message,
                )
                .at_path(owner_at));
            }
            let configuration = staged
                .read(Path::new("Configuration.xml"))
                .map_err(|error| ApplyPlanError::staging(error, "Configuration"))?
                .ok_or_else(|| {
                    ApplyPlanError::new(
                        ApplyPlanErrorKind::NotFound,
                        "extension Configuration descriptor is missing",
                    )
                })?;
            if !registered(&configuration, kind, name)? {
                return Err(ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidSource,
                    "extension object registration and descriptor disagree",
                )
                .at_path(owner_at));
            }
            if object_ids(owner).1.as_deref() != Some(parent_owner_uuid.as_str()) {
                return Err(ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidSource,
                    "the extension object is not borrowed from the form's parent owner",
                )
                .at_path(owner_at));
            }
        }
    }

    let at = format!("{}:{kind}.{name}.Form.{form}", binding.source_set_name());
    let existing = staged
        .read(&form_relative)
        .map_err(|error| ApplyPlanError::staging(error, at.clone()))?;
    let existing_content = staged
        .read(&content_relative)
        .map_err(|error| ApplyPlanError::staging(error, at.clone()))?;
    let created = existing.is_none();
    match existing.as_deref() {
        Some(bytes) => {
            let local = borrowed_form::extension_form(bytes, form).map_err(form_error)?;
            match local.parent {
                Some(uuid) if uuid == parent_form.uuid => {}
                Some(_) => {
                    return Err(ApplyPlanError::new(
                        ApplyPlanErrorKind::InvalidSource,
                        "the extension form is borrowed from a different parent form",
                    )
                    .at_path(at))
                }
                None => {
                    return Err(ApplyPlanError::new(
                        ApplyPlanErrorKind::InvalidState,
                        "the extension already owns a form with this name",
                    )
                    .at_path(at))
                }
            }
            if existing_content.is_none() {
                return Err(ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidState,
                    "the form is already borrowed without its own Form.xml; its content is inherited from the parent",
                )
                .at_path(at));
            }
            if !local.managed {
                return Err(ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidSource,
                    "the borrowed form descriptor lacks FormType Managed, without which the platform drops its Form.xml",
                )
                .at_path(at));
            }
        }
        None => {
            if existing_content.is_some() {
                return Err(ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidSource,
                    "the extension has the form's Form.xml without its descriptor",
                )
                .at_path(at));
            }
            let identity = plan_identity(
                staged,
                parent,
                &[
                    binding.source_set_name().as_bytes(),
                    from.as_bytes(),
                    &parent_owner,
                    &parent_form_bytes,
                ],
                index,
            )?;
            let uuid =
                identity.new_uuid("form", &from, crate::infrastructure::source_ids::fresh_uuid);
            let mut content = b"\xef\xbb\xbf".to_vec();
            content.extend_from_slice(
                unica_forms_designer::codec::borrowed_form_content_xml(&parent_form.version)
                    .as_bytes(),
            );
            for (relative, bytes) in [
                (
                    &form_relative,
                    borrowed_form::borrowed_form_descriptor(form, &uuid, &parent_form),
                ),
                (&content_relative, content),
            ] {
                staged
                    .create(relative, bytes)
                    .map_err(|error| ApplyPlanError::staging(error, at.clone()))?;
            }
            effects.push(ProvisionalApplyEffect::spanning(
                vec![form_relative.clone(), content_relative.clone()],
                DomainEvent::new(DomainEventKind::FormChanged, at.clone()),
                index,
            ));
        }
    }

    let owner = staged
        .read(&owner_relative)
        .map_err(|error| ApplyPlanError::staging(error, owner_at.clone()))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidState,
                "borrowed owner is missing",
            )
            .at_path(owner_at.clone())
        })?;
    if !object_registers_form(&owner, form)? {
        let source = unica_source_designer::registration::utf8_body(
            &owner,
            "the form owner descriptor",
            &owner_at,
        )?;
        let updated = unica_forms_designer::codec::register_form_in_object_text(&source, form);
        let image = unica_source_designer::registration::preserve_descriptor_image(
            &owner, &source, &updated,
        );
        staged
            .replace(&owner_relative, &owner, image)
            .map_err(|error| ApplyPlanError::staging(error, owner_at.clone()))?;
        effects.push(ProvisionalApplyEffect::single(
            &owner_relative,
            DomainEvent::new(DomainEventKind::MetadataChanged, owner_at),
            index,
        ));
    }

    let previous_properties = existing
        .as_deref()
        .map(descriptor_properties)
        .transpose()?
        .unwrap_or_default();
    let next = staged
        .read(&form_relative)
        .map_err(|error| ApplyPlanError::staging(error, at.clone()))?
        .unwrap_or_default();
    let next_properties = descriptor_properties(&next)?;
    let changed_properties = next_properties
        .iter()
        .filter(|(key, value)| previous_properties.get(*key) != Some(*value))
        .map(|(key, _)| key.clone())
        .collect();
    let detail = unica_source_contract::proposal::BorrowPlanDetail {
        index,
        at,
        from,
        parent_uuid: parent_form.uuid,
        created,
        changed_properties,
        protected_overrides: Vec::new(),
    };
    Ok((effects, detail))
}

/// Whether an object descriptor lists `<Form>name</Form>` in its own
/// `ChildObjects`.
fn object_registers_form(owner: &[u8], form: &str) -> Result<bool, ApplyPlanError> {
    const NS: &str = "http://v8.1c.ru/8.3/MDClasses";
    let text = std::str::from_utf8(owner)
        .map_err(|error| ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error.to_string()))?
        .trim_start_matches('\u{feff}');
    let document = roxmltree::Document::parse(text).map_err(|error| {
        ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error.to_string())
    })?;
    let object = document
        .root_element()
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                "owner descriptor has no object",
            )
        })?;
    Ok(object
        .children()
        .filter(|node| node.has_tag_name((NS, "ChildObjects")))
        .flat_map(|node| node.children())
        .any(|node| node.has_tag_name((NS, "Form")) && node.text() == Some(form)))
}

/// The object's own uuid and the parent uuid it borrows from, as written.
fn object_ids(bytes: &[u8]) -> (Option<String>, Option<String>) {
    let Some(text) = std::str::from_utf8(bytes).ok() else {
        return (None, None);
    };
    let Ok(document) = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')) else {
        return (None, None);
    };
    let Some(object) = document
        .root_element()
        .children()
        .find(|node| node.is_element())
    else {
        return (None, None);
    };
    let parent = object
        .children()
        .filter(|node| node.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "Properties")))
        .flat_map(|node| node.children())
        .find(|node| {
            node.has_tag_name((
                "http://v8.1c.ru/8.3/MDClasses",
                "ExtendedConfigurationObject",
            ))
        })
        .and_then(|node| node.text())
        .map(str::to_string);
    (object.attribute("uuid").map(str::to_string), parent)
}

/// Preview and publication regenerate the same identities from the same
/// planning inputs.
fn plan_identity(
    staged: &mut ApplyStagedState,
    parent: &QualifiedAddress,
    inputs: &[&[u8]],
    index: usize,
) -> Result<unica_metadata_designer::extension::CfeBorrowIdentity, ApplyPlanError> {
    use sha2::{Digest, Sha256};
    let mut seed = Sha256::new();
    seed.update(b"unica-canonical-borrow-plan-v1\0");
    staged
        .fingerprint_inputs(&mut seed)
        .map_err(|error| ApplyPlanError::staging(error, parent.to_string()))?;
    for bytes in inputs {
        seed.update((bytes.len() as u64).to_be_bytes());
        for chunk in bytes.chunks(64 * 1024) {
            staged
                .proposal()
                .checkpoint("borrow identity inputs")
                .map_err(|error| ApplyPlanError::staging(error, parent.to_string()))?;
            seed.update(chunk);
        }
    }
    seed.update((index as u64).to_be_bytes());
    Ok(unica_metadata_designer::extension::CfeBorrowIdentity::for_plan(seed.finalize().into()))
}

fn descriptor_properties(
    bytes: &[u8],
) -> Result<std::collections::BTreeMap<String, String>, ApplyPlanError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error.to_string()))?
        .trim_start_matches('\u{feff}');
    let document = roxmltree::Document::parse(text).map_err(|error| {
        ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error.to_string())
    })?;
    let object = document
        .root_element()
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                "borrow descriptor has no object",
            )
        })?;
    Ok(object
        .children()
        .filter(|node| node.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "Properties")))
        .flat_map(|node| node.children())
        .filter(|node| node.is_element())
        .map(|node| {
            (
                node.tag_name().name().to_string(),
                text[node.range()].to_string(),
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::apply::ApplyRequest;
    use unica_source_contract::{SourceFormat, SourceProfile};

    use crate::infrastructure::source_workspace::WorkspaceActor;
    use std::time::Duration;
    use unica_source_contract::workspace::WorkspaceContext;
    use unica_work_control::cancellation::CancellationToken;
    use unica_work_control::provider_deadline::ProviderDeadline;
    use unica_workspace::actor::{WorkspaceIdentity, WorkspaceSourceSetInput};

    struct Fixture {
        _root: tempfile::TempDir,
        actor: WorkspaceActor,
        destination: ProviderRootBinding,
        parent: ProviderRootBinding,
        dst: PathBuf,
        src: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let workspace = std::fs::canonicalize(root.path()).unwrap();
            let dst = workspace.join("extension");
            let src = workspace.join("main");
            std::fs::create_dir_all(dst.join("Catalogs")).unwrap();
            std::fs::create_dir_all(src.join("Catalogs")).unwrap();
            std::fs::write(workspace.join("v8project.yaml"), "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: main\n  - name: extension\n    type: EXTENSION\n    path: extension\n").unwrap();
            for (path, children) in [(&src, "<Catalog>Orders</Catalog>"), (&dst, "")] {
                std::fs::write(path.join("Configuration.xml"),format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="10000000-0000-4000-8000-000000000001"><Properties><Name>Sample</Name></Properties><ChildObjects>{children}</ChildObjects></Configuration></MetaDataObject>"#)).unwrap();
            }
            std::fs::write(src.join("Catalogs/Orders.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="00000000-0000-4000-8000-000000000001"><Properties><Name>Orders</Name><Comment>parent comment</Comment><CodeLength>9</CodeLength><DescriptionLength>25</DescriptionLength><Hierarchical>false</Hierarchical></Properties><ChildObjects/></Catalog></MetaDataObject>"#).unwrap();
            let context = WorkspaceContext {
                cwd: workspace.clone(),
                workspace_root: workspace.clone(),
                cache_root: workspace.join(".build/unica"),
                workspace_epoch: 1,
            };
            let identity = WorkspaceIdentity::new(
                &context,
                [
                    WorkspaceSourceSetInput::new(
                        "main",
                        &src,
                        SourceSetKind::Configuration,
                        SourceFormat::PlatformXml,
                        SourceProfile::platform_xml_8_3_27_format_2_20(),
                    ),
                    WorkspaceSourceSetInput::new(
                        "extension",
                        &dst,
                        SourceSetKind::Extension,
                        SourceFormat::PlatformXml,
                        SourceProfile::platform_xml_8_3_27_format_2_20(),
                    ),
                ],
                "borrow-test",
            )
            .unwrap();
            let actor =
                crate::infrastructure::source_workspace::new_actor(identity, context).unwrap();
            let destination = actor.bind_provider_root("extension", &dst).unwrap();
            let parent = actor.bind_provider_root("main", &src).unwrap();
            Self {
                _root: root,
                actor,
                destination,
                parent,
                dst,
                src,
            }
        }
        fn run(
            &self,
            dry: bool,
            rev: Option<&str>,
            overrides: Option<serde_json::Value>,
        ) -> Result<unica_workspace::actor::ApplyPublicationResult, String> {
            self.run_from("main:Catalog.Orders", dry, rev, overrides)
        }
        fn run_from(
            &self,
            from: &str,
            dry: bool,
            rev: Option<&str>,
            overrides: Option<serde_json::Value>,
        ) -> Result<unica_workspace::actor::ApplyPublicationResult, String> {
            let mut args = serde_json::json!({"from":from});
            if let Some(overrides) = overrides {
                args["overrides"] = overrides;
            }
            let mut input = serde_json::json!({"at":"extension:Configuration","dryRun":dry,"ops":[{"op":"object.borrow","args":args}]});
            if let Some(rev) = rev {
                input["ifRev"] = serde_json::json!(rev);
            }
            let request = ApplyRequest::parse(input.as_object().unwrap(), &["main", "extension"])
                .map_err(|e| e.to_string())?;
            let admission = self
                .actor
                .admit_apply_with_dependencies(
                    &self.destination,
                    std::slice::from_ref(&self.parent),
                    request.if_rev(),
                    request.dry_run(),
                    ProviderDeadline::from_budget(Duration::from_secs(10)),
                    &CancellationToken::new(),
                )
                .map_err(|e| e.to_string())?;
            let (staged, effects) =
                super::super::plan_hidden_v13_apply(&request, &self.destination, &admission)
                    .map_err(|e| e.to_string())?;
            let prepared = admission
                .prepare_with_effects(staged, effects)
                .map_err(|e| e.to_string())?;
            self.actor
                .publish_prepared_apply(prepared)
                .map_err(|e| e.to_string())
        }

        fn plan_from(&self, from: &str) -> Result<(), ApplyPlanError> {
            let input = serde_json::json!({"at":"extension:Configuration","dryRun":true,"ops":[{"op":"object.borrow","args":{"from":from}}]});
            let request = ApplyRequest::parse(input.as_object().unwrap(), &["main", "extension"])
                .expect("test address must pass domain validation");
            let admission = self
                .actor
                .admit_apply_with_dependencies(
                    &self.destination,
                    std::slice::from_ref(&self.parent),
                    request.if_rev(),
                    request.dry_run(),
                    ProviderDeadline::from_budget(Duration::from_secs(10)),
                    &CancellationToken::new(),
                )
                .expect("test workspace must admit planning");
            super::super::plan_hidden_v13_apply(&request, &self.destination, &admission).map(|_| ())
        }
    }

    fn enum_fixture() -> Fixture {
        let fixture = Fixture::new();
        std::fs::create_dir_all(fixture.src.join("Enums")).unwrap();
        let owner = fixture.src.join("Configuration.xml");
        let text = std::fs::read_to_string(&owner).unwrap();
        std::fs::write(
            owner,
            text.replace(
                "<Catalog>Orders</Catalog>",
                "<Catalog>Orders</Catalog><Enum>Operations</Enum>",
            ),
        )
        .unwrap();
        std::fs::write(fixture.src.join("Enums/Operations.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Enum uuid="10000000-0000-4000-8000-000000000011"><Properties><Name>Operations</Name></Properties><ChildObjects><EnumValue uuid="10000000-0000-4000-8000-000000000012"><Properties><Name>Costs</Name><Comment>parent comment</Comment></Properties></EnumValue><EnumValue uuid="10000000-0000-4000-8000-000000000013"><Properties><Name>Other</Name></Properties></EnumValue></ChildObjects></Enum></MetaDataObject>"#).unwrap();
        fixture
    }

    #[test]
    fn canonical_enum_value_preview_refresh_and_noop_preserve_local_siblings() {
        let fixture = enum_fixture();
        let from = "main:Enum.Operations.EnumValue.Costs";
        let preview = fixture.run_from(from, true, None, None).unwrap();
        assert!(!fixture.dst.join("Enums/Operations.xml").exists());
        let committed = fixture
            .run_from(from, false, Some(preview.rev()), None)
            .unwrap();
        assert_eq!(committed.rev(), preview.rev());
        let path = fixture.dst.join("Enums/Operations.xml");
        let text = std::fs::read_to_string(&path).unwrap();
        let borrowed_uuid = roxmltree::Document::parse(&text)
            .unwrap()
            .descendants()
            .find(|node| {
                node.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "EnumValue"))
                    && node.descendants().any(|child| {
                        child.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "Name"))
                            && child.text() == Some("Costs")
                    })
            })
            .unwrap()
            .attribute("uuid")
            .unwrap()
            .to_string();
        assert!(text.contains("<Name>Costs</Name>"));
        assert!(!text.contains("<Name>Other</Name>"));
        assert!(text.contains("<ExtendedConfigurationObject>10000000-0000-4000-8000-000000000012</ExtendedConfigurationObject>"));
        let own = r#"<EnumValue uuid="10000000-0000-4000-8000-000000000019"><Properties><Name>Local</Name><Comment>keep local</Comment></Properties></EnumValue>"#;
        let local = text
            .replace(
                "<Comment/>\n\t\t\t\t\t<ExtendedConfigurationObject>10000000-0000-4000-8000-000000000012</ExtendedConfigurationObject>",
                "<Comment>local borrowed comment</Comment>\n\t\t\t\t\t<ExtendedConfigurationObject>10000000-0000-4000-8000-000000000012</ExtendedConfigurationObject>",
            )
            .replace("</ChildObjects>", &format!("{own}</ChildObjects>"));
        assert!(local.contains("local borrowed comment"));
        std::fs::write(&path, &local).unwrap();
        let parent = fixture.src.join("Enums/Operations.xml");
        std::fs::write(
            &parent,
            std::fs::read_to_string(&parent)
                .unwrap()
                .replace("parent comment", "updated parent comment"),
        )
        .unwrap();
        let refresh = fixture.run_from(from, true, None, None).unwrap();
        assert!(refresh.effects().events().is_empty());
        fixture
            .run_from(from, false, Some(refresh.rev()), None)
            .unwrap();
        let refreshed = std::fs::read_to_string(&path).unwrap();
        assert!(refreshed.contains("local borrowed comment"));
        assert!(!refreshed.contains("updated parent comment"));
        assert!(refreshed.contains(&format!("uuid=\"{borrowed_uuid}\"")));
        assert!(refreshed.contains("<Name>Local</Name>"));
        assert!(refreshed.contains("keep local"));
        let noop = fixture.run_from(from, true, None, None).unwrap();
        assert!(noop.effects().events().is_empty());
        fixture
            .run_from(from, false, Some(noop.rev()), None)
            .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), refreshed);
    }

    #[test]
    fn canonical_enum_value_missing_collision_and_overrides_refuse_without_writes() {
        let fixture = enum_fixture();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.dst);
        let missing = fixture
            .plan_from("main:Enum.Operations.EnumValue.Missing")
            .unwrap_err();
        assert_eq!(missing.kind(), ApplyPlanErrorKind::NotFound);
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.dst),
            before
        );
        assert!(fixture
            .run_from(
                "main:Enum.Operations.EnumValue.Costs",
                true,
                None,
                Some(serde_json::json!([]))
            )
            .unwrap_err()
            .contains("overrides"));
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.dst),
            before
        );
        let owner = fixture
            .run_from("main:Enum.Operations", true, None, None)
            .unwrap();
        fixture
            .run_from("main:Enum.Operations", false, Some(owner.rev()), None)
            .unwrap();
        let path = fixture.dst.join("Enums/Operations.xml");
        let text = std::fs::read_to_string(&path).unwrap().replace("<ChildObjects/>", r#"<ChildObjects><EnumValue uuid="10000000-0000-4000-8000-000000000019"><Properties><Name>Costs</Name></Properties></EnumValue></ChildObjects>"#);
        assert!(
            text.contains("<Name>Costs</Name>"),
            "collision fixture must contain the extension-owned value"
        );
        std::fs::write(&path, &text).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.dst);
        assert!(fixture
            .run_from("main:Enum.Operations.EnumValue.Costs", true, None, None)
            .is_err());
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.dst),
            before
        );
    }

    #[test]
    fn canonical_enum_value_owner_without_child_objects_refuses_without_panicking_or_writing() {
        let fixture = enum_fixture();
        let owner = fixture
            .run_from("main:Enum.Operations", true, None, None)
            .unwrap();
        fixture
            .run_from("main:Enum.Operations", false, Some(owner.rev()), None)
            .unwrap();
        let path = fixture.dst.join("Enums/Operations.xml");
        let damaged = std::fs::read_to_string(&path)
            .unwrap()
            .replace("\n\t\t<ChildObjects/>", "");
        std::fs::write(&path, damaged).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.dst);

        let error = fixture
            .plan_from("main:Enum.Operations.EnumValue.Costs")
            .expect_err("damaged owner must produce a typed refusal");

        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidSource);
        assert!(error.to_string().contains("ChildObjects"), "{error}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.dst),
            before
        );
    }

    #[test]
    fn canonical_borrow_preview_refresh_and_noop_keep_local_identity_and_events_honest() {
        let fixture = Fixture::new();
        let preview = fixture
            .run(true, None, Some(serde_json::json!(["DescriptionLength"])))
            .unwrap();
        assert!(!fixture.dst.join("Catalogs/Orders.xml").exists());
        let committed = fixture
            .run(
                false,
                Some(preview.rev()),
                Some(serde_json::json!(["DescriptionLength"])),
            )
            .unwrap();
        let descriptor = fixture.dst.join("Catalogs/Orders.xml");
        let before = std::fs::read_to_string(&descriptor).unwrap();
        let before_doc = roxmltree::Document::parse(&before).unwrap();
        let identity = before_doc
            .root_element()
            .children()
            .find(|n| n.is_element())
            .unwrap()
            .attribute("uuid")
            .unwrap()
            .to_string();
        assert_eq!(committed.rev(), preview.rev());
        let description = before_doc
            .descendants()
            .find(|node| node.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "DescriptionLength")))
            .unwrap()
            .children()
            .find(|node| node.is_text())
            .unwrap()
            .range();
        let mut local = before.clone();
        local.replace_range(description, "30");
        std::fs::write(&descriptor, local).unwrap();
        let parent_path = fixture.src.join("Catalogs/Orders.xml");
        let parent = std::fs::read_to_string(&parent_path).unwrap();
        std::fs::write(
            &parent_path,
            parent
                .replace("<CodeLength>9</CodeLength>", "<CodeLength>11</CodeLength>")
                .replace(
                    "<DescriptionLength>25</DescriptionLength>",
                    "<DescriptionLength>60</DescriptionLength>",
                )
                .replace("parent comment", "new parent comment"),
        )
        .unwrap();
        assert!(fixture.run(false, Some(committed.rev()), None).is_err());
        let refresh = fixture.run(true, None, None).unwrap();
        assert!(!refresh.effects().events().is_empty());
        let refreshed = fixture.run(false, Some(refresh.rev()), None).unwrap();
        assert_eq!(refreshed.rev(), refresh.rev());
        let after = std::fs::read_to_string(&descriptor).unwrap();
        let after_doc = roxmltree::Document::parse(&after).unwrap();
        let property = |name| {
            after_doc
                .descendants()
                .find(|node| node.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", name)))
                .unwrap()
                .text()
        };
        assert_eq!(property("CodeLength"), Some("11"));
        assert_eq!(property("DescriptionLength"), Some("30"));
        assert!(after.contains(&identity));
        assert!(!after.contains("new parent comment"));
        let file_identity =
            unica_workspace::test_support::file_identity_for_test(&descriptor).unwrap();
        let noop_preview = fixture.run(true, None, None).unwrap();
        assert!(noop_preview.effects().events().is_empty());
        let noop = fixture.run(false, Some(noop_preview.rev()), None).unwrap();
        assert_eq!(noop.rev(), noop_preview.rev());
        assert!(noop.effects().events().is_empty());
        assert_eq!(std::fs::read_to_string(&descriptor).unwrap(), after);
        assert_eq!(
            unica_workspace::test_support::file_identity_for_test(&descriptor).unwrap(),
            file_identity
        );
    }

    #[test]
    fn mixed_borrow_batch_refusal_names_the_original_operation() {
        let fixture = Fixture::new();
        let descriptor = fixture.dst.join("Configuration.xml");
        let text = std::fs::read_to_string(&descriptor).unwrap();
        std::fs::write(&descriptor, text.replace("</Name>", "</Name><Comment/>")).unwrap();
        let overrides = Some(serde_json::json!(["DescriptionLength"]));
        let preview = fixture.run(true, None, overrides.clone()).unwrap();
        fixture.run(false, Some(preview.rev()), overrides).unwrap();
        let before = std::fs::read(&descriptor).unwrap();
        let borrowed = fixture.dst.join("Catalogs/Orders.xml");
        let borrowed_before = std::fs::read(&borrowed).unwrap();
        let input = serde_json::json!({
            "at": "extension:Configuration",
            "ops": [
                {"op": "props.set", "args": {"values": {"Comment": "staged only"}}},
                {"op": "object.borrow", "args": {"from": "main:Catalog.Orders"}},
                {"op": "object.borrow", "args": {"from": "main:Catalog.Orders", "overrides": ["CodeLength"]}}
            ],
            "dryRun": true
        });
        let request =
            ApplyRequest::parse(input.as_object().unwrap(), &["main", "extension"]).unwrap();
        let admission = fixture
            .actor
            .admit_apply_with_dependencies(
                &fixture.destination,
                std::slice::from_ref(&fixture.parent),
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(10)),
                &CancellationToken::new(),
            )
            .unwrap();
        let error = super::super::plan_hidden_v13_apply(&request, &fixture.destination, &admission)
            .unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{error}");
        assert_eq!(error.path(), Some("ops[2].args"));
        assert_eq!(std::fs::read(descriptor).unwrap(), before);
        assert_eq!(std::fs::read(borrowed).unwrap(), borrowed_before);
    }

    #[test]
    fn canonical_borrow_bad_override_is_an_argument_error() {
        let fixture = Fixture::new();
        for overrides in [
            serde_json::json!(["Comment"]),
            serde_json::json!(["DescriptionLength", "DescriptionLength"]),
        ] {
            let error = parse(&serde_json::json!({"at":"extension:Configuration","from":"main:Catalog.Orders","overrides":overrides}), 0, &fixture.destination).expect_err("invalid override is rejected before source planning");
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
        }
        use unica_metadata_designer::borrowed_object::{
            test_support::plan_borrow_object, BorrowObjectErrorKind,
        };
        let parent = std::fs::read(fixture.src.join("Catalogs/Orders.xml")).unwrap();
        let unsupported = plan_borrow_object(
            &parent,
            None,
            Some(&["UnknownProperty".to_string()]),
            crate::infrastructure::source_ids::fresh_uuid,
        )
        .unwrap_err();
        assert_eq!(unsupported.kind, BorrowObjectErrorKind::BadValue);
        let first = plan_borrow_object(
            &parent,
            None,
            Some(&["DescriptionLength".to_string()]),
            crate::infrastructure::source_ids::fresh_uuid,
        )
        .unwrap();
        let changed = plan_borrow_object(
            &parent,
            Some(&first.bytes),
            Some(&[]),
            crate::infrastructure::source_ids::fresh_uuid,
        )
        .unwrap_err();
        assert_eq!(changed.kind, BorrowObjectErrorKind::BadValue);
    }

    #[test]
    fn canonical_borrow_refuses_parent_descriptor_outside_writable_profile() {
        let fixture = Fixture::new();
        let descriptor = fixture.src.join("Catalogs/Orders.xml");
        let parent = std::fs::read_to_string(&descriptor).unwrap();
        std::fs::write(
            &descriptor,
            parent.replace("version=\"2.20\"", "version=\"2.21\""),
        )
        .unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.dst);
        let error = fixture
            .run(true, None, None)
            .expect_err("a newer parent descriptor must refuse before emitting borrowed XML");
        assert!(error.contains("2.21"), "{error}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.dst),
            before
        );
        assert!(!fixture.actor.context().cache_root.exists());
    }

    #[test]
    fn canonical_borrow_refuses_unregistered_parent_and_destination_own_object() {
        let fixture = Fixture::new();
        let preview = fixture
            .run(true, None, Some(serde_json::json!(["DescriptionLength"])))
            .unwrap();
        fixture
            .run(
                false,
                Some(preview.rev()),
                Some(serde_json::json!(["DescriptionLength"])),
            )
            .unwrap();
        let descriptor = fixture.dst.join("Catalogs/Orders.xml");
        let parent = std::fs::read(fixture.src.join("Catalogs/Orders.xml")).unwrap();
        std::fs::write(&descriptor, &parent).unwrap();
        assert!(fixture.run(true, None, None).is_err());
        assert_eq!(std::fs::read(&descriptor).unwrap(), parent);
        let owner = fixture.src.join("Configuration.xml");
        std::fs::write(
            &owner,
            std::fs::read_to_string(&owner)
                .unwrap()
                .replace("<Catalog>Orders</Catalog>", ""),
        )
        .unwrap();
        assert!(fixture
            .run(true, None, None)
            .unwrap_err()
            .contains("not registered"));
    }
    fn platform(name: &str) -> Vec<u8> {
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/platform_8_3_27/cfe_borrow")
                .join(name),
        )
        .unwrap()
    }

    const FORM_FROM: &str = "main:Catalog.CorpusCatalog.Form.CorpusForm";
    const OWNER_FORMS: &str =
        "<ChildObjects>\r\n\t\t\t<Form>CorpusForm</Form>\r\n\t\t</ChildObjects>";

    /// Parent and extension exactly as the #1982 platform run exported them,
    /// with the borrowed catalog and form taken out of the extension.
    fn form_fixture() -> Fixture {
        let fixture = Fixture::new();
        let parent = fixture.src.join("Catalogs/CorpusCatalog");
        std::fs::create_dir_all(parent.join("Forms/CorpusForm/Ext/Form")).unwrap();
        for (relative, name) in [
            ("Catalogs/CorpusCatalog.xml", "parent-catalog-with-form.xml"),
            (
                "Catalogs/CorpusCatalog/Forms/CorpusForm.xml",
                "parent-catalog-form.xml",
            ),
            (
                "Catalogs/CorpusCatalog/Forms/CorpusForm/Ext/Form.xml",
                "parent-catalog-form-Form.xml",
            ),
            (
                "Catalogs/CorpusCatalog/Forms/CorpusForm/Ext/Form/Module.bsl",
                "parent-catalog-form-Module.bsl",
            ),
        ] {
            std::fs::write(fixture.src.join(relative), platform(name)).unwrap();
        }
        let owner = fixture.src.join("Configuration.xml");
        let text = std::fs::read_to_string(&owner).unwrap();
        std::fs::write(
            owner,
            text.replace(
                "<Catalog>Orders</Catalog>",
                "<Catalog>Orders</Catalog><Catalog>CorpusCatalog</Catalog>",
            ),
        )
        .unwrap();
        let configuration =
            String::from_utf8(platform("extension-configuration-with-borrowed-form.xml")).unwrap();
        let unregistered = configuration.replace("\r\n\t\t\t<Catalog>CorpusCatalog</Catalog>", "");
        assert_ne!(unregistered, configuration);
        std::fs::write(fixture.dst.join("Configuration.xml"), unregistered).unwrap();
        fixture
    }

    fn form_uuid(bytes: &[u8]) -> String {
        let text = std::str::from_utf8(bytes)
            .unwrap()
            .trim_start_matches('\u{feff}');
        roxmltree::Document::parse(text)
            .unwrap()
            .descendants()
            .find(|node| node.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "Form")))
            .unwrap()
            .attribute("uuid")
            .unwrap()
            .to_string()
    }

    /// The form descriptor and `Form.xml` equal the platform export; the
    /// only free byte is the extension's own fresh form uuid.
    fn assert_platform_form(fixture: &Fixture) {
        let forms = fixture.dst.join("Catalogs/CorpusCatalog/Forms");
        let descriptor = std::fs::read(forms.join("CorpusForm.xml")).unwrap();
        let expected = platform("extension-object-form-borrowed.xml");
        let own = form_uuid(&descriptor);
        assert_ne!(own, "3a1f0000-0000-4000-8000-000000001982");
        assert_eq!(
            String::from_utf8(descriptor).unwrap(),
            String::from_utf8(expected.clone())
                .unwrap()
                .replace(&form_uuid(&expected), &own)
        );
        assert_eq!(
            std::fs::read(forms.join("CorpusForm/Ext/Form.xml")).unwrap(),
            platform("extension-object-form-borrowed-Form.xml")
        );
        assert!(!forms.join("CorpusForm/Ext/Form/Module.bsl").exists());
    }

    #[test]
    fn canonical_form_borrow_previews_without_writes_then_publishes_platform_bytes_with_its_owner()
    {
        let fixture = form_fixture();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.dst);
        let preview = fixture.run_from(FORM_FROM, true, None, None).unwrap();
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.dst),
            before,
            "preview must not write"
        );
        assert!(!preview.effects().events().is_empty());
        let committed = fixture
            .run_from(FORM_FROM, false, Some(preview.rev()), None)
            .unwrap();
        assert_eq!(committed.rev(), preview.rev());

        assert_platform_form(&fixture);
        assert_eq!(
            std::fs::read(fixture.dst.join("Configuration.xml")).unwrap(),
            platform("extension-configuration-with-borrowed-form.xml"),
            "the auto-borrowed owner is registered as the platform exports it"
        );
        let owner =
            std::fs::read_to_string(fixture.dst.join("Catalogs/CorpusCatalog.xml")).unwrap();
        assert!(owner.contains(
            "<ExtendedConfigurationObject>1d6b8425-360c-4ab1-9bab-cc9a3b590bb2</ExtendedConfigurationObject>"
        ));
        assert_eq!(
            owner
                .replace("\r\n", "\n")
                .matches("<Form>CorpusForm</Form>")
                .count(),
            1
        );
        assert!(
            owner
                .replace("\r\n", "\n")
                .contains(&OWNER_FORMS.replace("\r\n", "\n")),
            "{owner}"
        );

        let content = fixture
            .dst
            .join("Catalogs/CorpusCatalog/Forms/CorpusForm/Ext/Form.xml");
        let identity = unica_workspace::test_support::file_identity_for_test(&content).unwrap();
        let tree = unica_workspace::test_support::tree_snapshot(&fixture.dst);
        let repeat = fixture.run_from(FORM_FROM, true, None, None).unwrap();
        assert!(repeat.effects().events().is_empty());
        let noop = fixture
            .run_from(FORM_FROM, false, Some(repeat.rev()), None)
            .unwrap();
        assert!(noop.effects().events().is_empty());
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.dst),
            tree
        );
        assert_eq!(
            unica_workspace::test_support::file_identity_for_test(&content).unwrap(),
            identity
        );
    }

    #[test]
    fn canonical_form_borrow_refuses_foreign_owner_own_form_and_orphan_content_without_writes() {
        let fixture = form_fixture();
        let preview = fixture.run_from(FORM_FROM, true, None, None).unwrap();
        fixture
            .run_from(FORM_FROM, false, Some(preview.rev()), None)
            .unwrap();
        let forms = fixture.dst.join("Catalogs/CorpusCatalog/Forms");
        let descriptor = forms.join("CorpusForm.xml");
        let content = forms.join("CorpusForm/Ext/Form.xml");
        let borrowed = std::fs::read_to_string(&descriptor).unwrap();
        let refuse = |kind: ApplyPlanErrorKind| {
            let before = unica_workspace::test_support::tree_snapshot(&fixture.dst);
            assert_eq!(fixture.plan_from(FORM_FROM).unwrap_err().kind(), kind);
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.dst),
                before
            );
        };

        std::fs::write(
            &descriptor,
            borrowed.replace("\t\t\t<FormType>Managed</FormType>\r\n", ""),
        )
        .unwrap();
        refuse(ApplyPlanErrorKind::InvalidSource);
        std::fs::write(
            &descriptor,
            borrowed.replace(
                "3a1f0000-0000-4000-8000-000000001982",
                "3a1f0000-0000-4000-8000-000000000001",
            ),
        )
        .unwrap();
        refuse(ApplyPlanErrorKind::InvalidSource);
        std::fs::write(
            &descriptor,
            borrowed.replace(
                "\t\t\t<ExtendedConfigurationObject>3a1f0000-0000-4000-8000-000000001982</ExtendedConfigurationObject>\r\n",
                "",
            ),
        )
        .unwrap();
        refuse(ApplyPlanErrorKind::InvalidState);
        std::fs::remove_file(&descriptor).unwrap();
        refuse(ApplyPlanErrorKind::InvalidSource);
        std::fs::write(&descriptor, &borrowed).unwrap();

        let owner = fixture.dst.join("Catalogs/CorpusCatalog.xml");
        let borrowed_owner = std::fs::read_to_string(&owner).unwrap();
        std::fs::write(
            &owner,
            borrowed_owner.replace(
                "1d6b8425-360c-4ab1-9bab-cc9a3b590bb2",
                "1d6b8425-360c-4ab1-9bab-000000000000",
            ),
        )
        .unwrap();
        refuse(ApplyPlanErrorKind::InvalidSource);
        std::fs::write(&owner, &borrowed_owner).unwrap();
        let configuration = fixture.dst.join("Configuration.xml");
        let registered = std::fs::read_to_string(&configuration).unwrap();
        std::fs::write(
            &configuration,
            registered.replace("\r\n\t\t\t<Catalog>CorpusCatalog</Catalog>", ""),
        )
        .unwrap();
        refuse(ApplyPlanErrorKind::InvalidSource);
        assert!(content.exists());
    }

    #[test]
    fn canonical_form_borrow_into_borrowed_owner_registers_the_form_as_the_platform_exports() {
        let fixture = form_fixture();
        std::fs::create_dir_all(fixture.dst.join("Catalogs")).unwrap();
        let expected_owner = platform("extension-catalog-with-borrowed-form.xml");
        let owner = String::from_utf8(expected_owner.clone())
            .unwrap()
            .replace(OWNER_FORMS, "<ChildObjects/>");
        assert_ne!(owner.as_bytes(), expected_owner.as_slice());
        std::fs::write(fixture.dst.join("Catalogs/CorpusCatalog.xml"), owner).unwrap();
        let configuration = platform("extension-configuration-with-borrowed-form.xml");
        std::fs::write(fixture.dst.join("Configuration.xml"), &configuration).unwrap();

        let preview = fixture.run_from(FORM_FROM, true, None, None).unwrap();
        fixture
            .run_from(FORM_FROM, false, Some(preview.rev()), None)
            .unwrap();

        assert_platform_form(&fixture);
        assert_eq!(
            std::fs::read(fixture.dst.join("Catalogs/CorpusCatalog.xml")).unwrap(),
            expected_owner
        );
        assert_eq!(
            std::fs::read(fixture.dst.join("Configuration.xml")).unwrap(),
            configuration
        );
    }

    #[test]
    fn canonical_form_borrow_refuses_missing_unmanaged_and_inherited_forms_without_writes() {
        let fixture = form_fixture();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.dst);
        let missing = fixture
            .plan_from("main:Catalog.CorpusCatalog.Form.Missing")
            .unwrap_err();
        assert_eq!(missing.kind(), ApplyPlanErrorKind::NotFound);
        assert!(fixture
            .run_from(FORM_FROM, true, None, Some(serde_json::json!([])))
            .unwrap_err()
            .contains("overrides"));
        let descriptor = fixture
            .src
            .join("Catalogs/CorpusCatalog/Forms/CorpusForm.xml");
        let managed = std::fs::read_to_string(&descriptor).unwrap();
        std::fs::write(
            &descriptor,
            managed.replace(
                "<FormType>Managed</FormType>",
                "<FormType>Ordinary</FormType>",
            ),
        )
        .unwrap();
        assert_eq!(
            fixture.plan_from(FORM_FROM).unwrap_err().kind(),
            ApplyPlanErrorKind::BadValue
        );
        std::fs::write(&descriptor, managed).unwrap();
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.dst),
            before
        );

        let preview = fixture.run_from(FORM_FROM, true, None, None).unwrap();
        fixture
            .run_from(FORM_FROM, false, Some(preview.rev()), None)
            .unwrap();
        std::fs::remove_file(
            fixture
                .dst
                .join("Catalogs/CorpusCatalog/Forms/CorpusForm/Ext/Form.xml"),
        )
        .unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.dst);
        assert_eq!(
            fixture.plan_from(FORM_FROM).unwrap_err().kind(),
            ApplyPlanErrorKind::InvalidState
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.dst),
            before
        );
    }
}
