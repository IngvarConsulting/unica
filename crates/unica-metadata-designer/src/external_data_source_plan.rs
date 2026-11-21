//! Retained planning for external data source descriptors.

use std::path::{Path, PathBuf};

use roxmltree::{Document, Node};
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_address::source_target::MetadataAddress;
use unica_metadata::external_data_source::{Changes, Field, PropertyValue};
use unica_metadata::{
    MetaFillValue, MetaObservedPropertyValue, MetaValueProfileContext, MetadataType,
};
use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyProposal, ProvisionalApplyEffect,
};
use unica_source_contract::SourceSetKind;
use unica_xml_format::prefix_form::PrefixForm;

use crate::descriptor_comment::{descriptor_properties, direct};
use crate::external_data_source_seed::{self as seed, Seed};
use crate::external_data_source_write::{self as xml, ChildStorage, Property};

struct Descriptor {
    path: PathBuf,
    bytes: Vec<u8>,
}

impl Descriptor {
    fn text(&self, site: &str) -> Result<&str, ApplyPlanError> {
        let bytes = self
            .bytes
            .strip_prefix(b"\xef\xbb\xbf")
            .unwrap_or(&self.bytes);
        std::str::from_utf8(bytes).map_err(|error| damaged(site, error.to_string()))
    }

    fn image(&self, text: &str) -> Vec<u8> {
        let mut image = Vec::new();
        if self.bytes.starts_with(b"\xef\xbb\xbf") {
            image.extend_from_slice(b"\xef\xbb\xbf");
        }
        image.extend_from_slice(text.as_bytes());
        image
    }
}

struct Located {
    descriptor: Descriptor,
    version: String,
    owner: (NodeKind, String),
    member: Option<(NodeKind, String)>,
}

impl Located {
    fn properties<'a, 'input>(
        &self,
        source: &str,
        document: &'a Document<'input>,
        site: &str,
    ) -> Result<Node<'a, 'input>, ApplyPlanError> {
        let (kind, name) = &self.owner;
        let props = xml::properties(source, document, *kind, name, &self.version)
            .map_err(|error| damaged(site, error))?;
        let Some((kind, name)) = &self.member else {
            return Ok(props);
        };
        let member = xml::registered_child(
            props
                .parent_element()
                .expect("canonical Properties has its owner"),
            *kind,
            name,
            ChildStorage::Inline,
        )
        .map_err(|error| damaged(site, error))?
        .ok_or_else(|| missing(site, format!("{}.{name} is not registered", kind.as_str())))?;
        direct(member, "Properties").map_err(|error| damaged(site, error))
    }
}

fn refusal(kind: ApplyPlanErrorKind, site: &str, message: impl Into<String>) -> ApplyPlanError {
    ApplyPlanError::new(kind, message).at_path(site.to_string())
}

fn damaged(site: &str, error: String) -> ApplyPlanError {
    refusal(ApplyPlanErrorKind::InvalidSource, site, error)
}

fn missing(site: &str, error: String) -> ApplyPlanError {
    refusal(ApplyPlanErrorKind::NotFound, site, error)
}

fn read_descriptor(
    staged: &mut ApplyProposal<'_>,
    path: PathBuf,
    site: &str,
) -> Result<Descriptor, ApplyPlanError> {
    let bytes = staged
        .read(&path)
        .map_err(|error| ApplyPlanError::staging(error, site.to_string()))?
        .ok_or_else(|| {
            damaged(
                site,
                format!("registered descriptor {} was not found", path.display()),
            )
        })?;
    Ok(Descriptor { path, bytes })
}

fn parse<'a>(source: &'a str, site: &str) -> Result<Document<'a>, ApplyPlanError> {
    Document::parse(source).map_err(|error| damaged(site, error.to_string()))
}

fn descriptor_path(logical: &str, site: &str) -> Result<PathBuf, ApplyPlanError> {
    let at = MetadataAddress::parse_logical(logical)
        .map_err(|error| refusal(ApplyPlanErrorKind::BadValue, site, error.to_string()))?;
    unica_source_designer::layout::metadata_descriptor_relative(&at, SourceSetKind::Configuration)
        .map_err(|error| refusal(ApplyPlanErrorKind::BadValue, site, error))
}

fn registered(
    parent: Node<'_, '_>,
    child: NodeKind,
    name: &str,
    storage: ChildStorage,
    site: &str,
) -> Result<(), ApplyPlanError> {
    xml::registered_child(parent, child, name, storage)
        .map_err(|error| damaged(site, error))?
        .ok_or_else(|| missing(site, format!("{}.{name} is not registered", child.as_str())))?;
    Ok(())
}

fn locate(
    staged: &mut ApplyProposal<'_>,
    at: &QualifiedAddress,
    site: &str,
) -> Result<Located, ApplyPlanError> {
    let [root, children @ ..] = at.segments() else {
        return Err(refusal(
            ApplyPlanErrorKind::BadValue,
            site,
            "address must name an external data source",
        ));
    };
    let root_name = root
        .name()
        .filter(|_| root.kind() == NodeKind::ExternalDataSource)
        .ok_or_else(|| {
            refusal(
                ApplyPlanErrorKind::BadValue,
                site,
                "address must name an external data source",
            )
        })?;
    let configuration =
        read_descriptor(staged, Path::new("Configuration.xml").to_path_buf(), site)?;
    let configuration_text = configuration.text(site)?;
    let document = parse(configuration_text, site)?;
    let version = unica_source_designer::roots::root_version_literal(
        configuration_text,
        document.root_element(),
    );
    if let Some(error) = unica_source_designer::format_profile::writable_profile_refusal(
        "Configuration.xml".to_string(),
        version.as_deref(),
    ) {
        return Err(damaged(site, error.message));
    }
    let version =
        version.ok_or_else(|| damaged(site, "configuration has no writable XML format".into()))?;
    let config =
        descriptor_properties(&document, "Configuration").map_err(|error| damaged(site, error))?;
    registered(
        config
            .parent_element()
            .expect("canonical Properties has its owner"),
        root.kind(),
        root_name,
        ChildStorage::Reference,
        site,
    )?;
    let mut logical = format!("ExternalDataSource.{root_name}");
    let mut result = Located {
        descriptor: read_descriptor(staged, descriptor_path(&logical, site)?, site)?,
        version,
        owner: (root.kind(), root_name.to_string()),
        member: None,
    };
    for (index, segment) in children.iter().enumerate() {
        let child = segment.kind();
        let name = segment.name().ok_or_else(|| {
            refusal(
                ApplyPlanErrorKind::BadValue,
                site,
                "address must name an object, not a branch",
            )
        })?;
        let parent = result.owner.0;
        let order = unica_address::kinds::declared(parent.as_str())
            .and_then(|(_, declaration)| declaration.child_order);
        if !order.is_some_and(|order| order.contains(&child.as_str())) {
            return Err(refusal(
                ApplyPlanErrorKind::BadValue,
                site,
                "the child is outside the external source writer profile",
            ));
        }
        let physical = unica_address::kinds::nested_place(parent.as_str(), child.as_str())
            .is_some_and(|place| place.directory.is_some());
        let storage = if physical {
            ChildStorage::Reference
        } else {
            ChildStorage::Inline
        };
        let text = result.descriptor.text(site)?;
        let document = parse(text, site)?;
        let props = result.properties(text, &document, site)?;
        registered(
            props
                .parent_element()
                .expect("canonical Properties has its owner"),
            child,
            name,
            storage,
            site,
        )?;
        if !physical {
            if index + 1 != children.len() {
                return Err(refusal(
                    ApplyPlanErrorKind::BadValue,
                    site,
                    "an inline member has no nested metadata writer",
                ));
            }
            result.member = Some((child, name.to_string()));
            return Ok(result);
        }
        logical.push_str(&format!(".{}.{name}", child.as_str()));
        result.descriptor = read_descriptor(staged, descriptor_path(&logical, site)?, site)?;
        result.owner = (child, name.to_string());
    }
    let text = result.descriptor.text(site)?;
    result.properties(text, &parse(text, site)?, site)?;
    Ok(result)
}

pub(crate) fn dimension_type(
    staged: &mut ApplyProposal<'_>,
    at: &QualifiedAddress,
    reference: &QualifiedAddress,
    site: &str,
) -> Result<String, ApplyPlanError> {
    let scope = at.segments();
    let cube = if scope
        .last()
        .is_some_and(|segment| segment.kind() == NodeKind::Dimension)
    {
        &scope[..scope.len() - 1]
    } else {
        scope
    };
    let target = reference.segments();
    if reference.source_set() != at.source_set()
        || cube
            .last()
            .is_none_or(|segment| segment.kind() != NodeKind::Cube)
        || target.len() != cube.len() + 1
        || !target.starts_with(cube)
        || target
            .last()
            .is_none_or(|segment| segment.kind() != NodeKind::DimensionTable)
    {
        return Err(refusal(
            ApplyPlanErrorKind::BadValue,
            site,
            "type must name a registered DimensionTable of the same cube",
        ));
    }
    locate(staged, reference, site)?;
    let names = target
        .iter()
        .filter_map(|segment| segment.name())
        .collect::<Vec<_>>()
        .join(".");
    Ok(format!("ExternalDataSourceCubeDimensionTableRef.{names}"))
}

pub(crate) fn add(
    staged: &mut ApplyProposal<'_>,
    owner: &QualifiedAddress,
    name: &str,
    item: Seed<'_>,
    site: &str,
    op_index: usize,
) -> Result<ProvisionalApplyEffect, ApplyPlanError> {
    let located = locate(staged, owner, site)?;
    if located.member.is_some() {
        return Err(refusal(
            ApplyPlanErrorKind::BadValue,
            site,
            "an inline member cannot contain external data source metadata",
        ));
    }
    let kind = item.kind();
    let address = format!("{owner}.{}.{name}", kind.as_str());
    let generated_name = owner
        .segments()
        .iter()
        .filter_map(|segment| segment.name())
        .chain(std::iter::once(name))
        .collect::<Vec<_>>()
        .join(".");
    let separate = unica_address::kinds::nested_place(located.owner.0.as_str(), kind.as_str())
        .is_some_and(|place| place.directory.is_some());
    let storage = if separate {
        ChildStorage::Reference
    } else {
        ChildStorage::Inline
    };
    let (text, form) = PrefixForm::normalize(located.descriptor.text(site)?)
        .map_err(|error| damaged(site, error))?;
    let document = parse(&text, site)?;
    let props = located.properties(&text, &document, site)?;
    let owner_node = props
        .parent_element()
        .expect("canonical Properties has its owner");
    if xml::registered_child(owner_node, kind, name, storage)
        .map_err(|error| damaged(site, error))?
        .is_some()
    {
        return Err(refusal(
            ApplyPlanErrorKind::InvalidState,
            site,
            format!("{address} already exists"),
        ));
    }
    let children = direct(owner_node, "ChildObjects").map_err(|error| damaged(site, error))?;
    let namespaces = xml::TypeNamespaces::for_scope(Some(children));
    let fragment = seed::emit(item, name, &generated_name, &namespaces, &mut || {
        uuid::Uuid::new_v4().to_string()
    });
    let registration = if separate {
        xml::physical_reference(kind, name)
    } else {
        fragment.clone()
    };
    let updated = xml::insert_child(&text, owner_node, kind, name, storage, &registration)
        .and_then(|updated| form.restore(&updated))
        .map_err(|error| damaged(site, error))?;
    let mut paths = vec![located.descriptor.path.clone()];
    if separate {
        let logical = format!("{}.{}.{name}", owner.logical_path(), kind.as_str());
        let path = descriptor_path(&logical, site)?;
        if staged
            .read(&path)
            .map_err(|error| ApplyPlanError::staging(error, site.to_string()))?
            .is_some()
        {
            return Err(refusal(
                ApplyPlanErrorKind::InvalidState,
                site,
                format!("{address} has an existing descriptor"),
            ));
        }
        staged
            .create(&path, seed::standalone(&fragment, &located.version))
            .map_err(|error| ApplyPlanError::staging(error, site.to_string()))?;
        paths.push(path);
    }
    staged
        .replace(
            &located.descriptor.path,
            &located.descriptor.bytes,
            located.descriptor.image(&updated),
        )
        .map_err(|error| ApplyPlanError::staging(error, site.to_string()))?;
    Ok(ProvisionalApplyEffect::spanning(
        paths,
        DomainEvent::new(DomainEventKind::MetadataChanged, address),
        op_index,
    ))
}

pub(crate) fn set(
    staged: &mut ApplyProposal<'_>,
    at: &QualifiedAddress,
    changes: &Changes,
    op_index: usize,
) -> Result<Option<ProvisionalApplyEffect>, ApplyPlanError> {
    let site = format!("ops[{op_index}].args.at");
    let located = locate(staged, at, &site)?;
    let (original, form) = PrefixForm::normalize(located.descriptor.text(&site)?)
        .map_err(|error| damaged(&site, error))?;
    let mut updated = original.clone();
    for (field, value) in changes {
        let tag = field.names()[0];
        let document = parse(&updated, &site)?;
        let props = located.properties(&updated, &document, &site)?;
        let field_path = format!("ops[{op_index}].args.values.{tag}");
        if located.owner.0 == NodeKind::Table && located.member.is_none() {
            validate_table_property(props, *field, value, &field_path)?;
        }
        let replacement = match value {
            PropertyValue::Text(value) => {
                xml::patch_property(&updated, props, tag, Property::Scalar(value))
            }
            PropertyValue::Bool(value) => {
                let text = if *value { "true" } else { "false" };
                xml::patch_property(&updated, props, tag, Property::Scalar(text))
            }
            PropertyValue::Synonym(items) => {
                let languages = items
                    .iter()
                    .map(|item| (item.language.clone(), item.content.clone()))
                    .collect::<Vec<_>>();
                xml::patch_property(&updated, props, tag, Property::Synonym(&languages))
            }
            PropertyValue::Type(value) => {
                let postimage = xml::patch_property(&updated, props, tag, Property::Type(value))
                    .map_err(|error| damaged(&field_path, error))?;
                if postimage != updated {
                    validate_dependent_values(props, value, &field_path)?;
                }
                Ok(postimage)
            }
            PropertyValue::DimensionTable(value) => {
                let wire = dimension_type(staged, at, value, &field_path)?;
                xml::patch_property(&updated, props, tag, Property::DimensionType(&wire))
            }
        };
        updated = replacement.map_err(|error| damaged(&field_path, error))?;
    }
    if updated == original {
        return Ok(None);
    }
    let updated = form
        .restore(&updated)
        .map_err(|error| damaged(&site, error))?;
    staged
        .replace(
            &located.descriptor.path,
            &located.descriptor.bytes,
            located.descriptor.image(&updated),
        )
        .map_err(|error| ApplyPlanError::staging(error, site))?;
    Ok(Some(ProvisionalApplyEffect::single(
        located.descriptor.path,
        DomainEvent::new(DomainEventKind::MetadataChanged, at.to_string()),
        op_index,
    )))
}

fn validate_table_property(
    props: Node<'_, '_>,
    field: Field,
    value: &PropertyValue,
    site: &str,
) -> Result<(), ApplyPlanError> {
    if !matches!(field, Field::SourceName | Field::Expression) {
        return Ok(());
    }
    let table_type = direct(props, "TableType").map_err(|error| damaged(site, error))?;
    let kind = unica_xml_format::read::direct_text(table_type);
    match (kind.as_str(), field, value) {
        ("Table", Field::SourceName, _) => Ok(()),
        ("Expression", Field::Expression, PropertyValue::Text(text)) if !text.trim().is_empty() => Ok(()),
        ("Table" | "Expression", _, _) => Err(refusal(
            ApplyPlanErrorKind::BadValue,
            site,
            "Table requires NameInDataSource and Expression requires nonempty ExpressionInDataSource",
        )),
        _ => Err(damaged(site, "unsupported or damaged TableType".into())),
    }
}

fn validate_dependent_values(
    props: Node<'_, '_>,
    metadata_type: &MetadataType,
    site: &str,
) -> Result<(), ApplyPlanError> {
    for tag in ["MinValue", "MaxValue", "FillValue"] {
        let matches = props
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == tag)
            .count();
        if matches == 0 {
            continue;
        }
        let node = direct(props, tag).map_err(|error| damaged(site, error))?;
        let observed = crate::info_projection::parse_typed_or_nil_value(node, tag)
            .map_err(|error| damaged(site, error.message))?;
        let value = match observed {
            MetaObservedPropertyValue::Nil {} => continue,
            MetaObservedPropertyValue::Typed { r#type, value }
                if r#type.namespace == "http://www.w3.org/2001/XMLSchema" =>
            {
                match r#type.local_name.as_str() {
                    // The platform uses an empty xs:string value as its string default,
                    // and rounds it to nil when an incompatible type is imported.
                    "string" if value.is_empty() => continue,
                    "string" => MetaFillValue::String(value),
                    "decimal" => MetaFillValue::Number(value),
                    "boolean" => match value.as_str() {
                        "true" | "1" => MetaFillValue::Boolean(true),
                        "false" | "0" => MetaFillValue::Boolean(false),
                        _ => return Err(damaged(site, format!("{tag} is not a valid boolean"))),
                    },
                    _ => return Err(damaged(site, format!("cannot prove the type of {tag}"))),
                }
            }
            _ => return Err(damaged(site, format!("cannot prove the type of {tag}"))),
        };
        unica_metadata::validate_metadata_element_value_profile(
            Some(metadata_type),
            Some(&value),
            MetaValueProfileContext::Patch,
        )
        .map_err(|error| {
            refusal(
                ApplyPlanErrorKind::BadValue,
                site,
                format!(
                    "the requested Type would invalidate the retained {tag}: {}",
                    error.message
                ),
            )
        })?;
    }
    Ok(())
}

pub(crate) fn add_items(
    staged: &mut ApplyProposal<'_>,
    owner: &QualifiedAddress,
    items: &[unica_metadata::external_data_source::AddItem],
    op_index: usize,
) -> Result<Vec<ProvisionalApplyEffect>, ApplyPlanError> {
    use unica_metadata::external_data_source::{AddContent, TableSource};
    let mut effects = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let site = format!("ops[{op_index}].args.items[{index}]");
        let qname;
        let seed = match &item.content {
            AddContent::Table { source, data_type } => Seed::Table {
                source: match source {
                    TableSource::Table(name) => seed::TableSource::Table(name),
                    TableSource::Expression(expression) => {
                        seed::TableSource::Expression(expression)
                    }
                },
                data_type: data_type.as_str(),
            },
            AddContent::Cube(source_name) => Seed::Cube { source_name },
            AddContent::DimensionTable(source_name) => Seed::DimensionTable { source_name },
            AddContent::Field {
                source_name,
                field_type,
            } => Seed::Field {
                source_name,
                field_type,
            },
            AddContent::Function {
                expression,
                return_value,
                return_type,
            } => Seed::Function {
                expression,
                return_value: *return_value,
                return_type,
            },
            AddContent::Dimension(reference) => {
                qname = dimension_type(staged, owner, reference, &format!("{site}.type"))?;
                Seed::Dimension { type_qname: &qname }
            }
            AddContent::Resource {
                source_name,
                field_type,
            } => Seed::Resource {
                source_name,
                field_type,
            },
        };
        effects.push(add(staged, owner, &item.name, seed, &site, op_index)?);
    }
    Ok(effects)
}
