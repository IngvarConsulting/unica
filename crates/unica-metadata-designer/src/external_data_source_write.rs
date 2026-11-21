//! XML writes for the descriptor tree of one external data source.

use roxmltree::{Document, Node};
use unica_address::address::NodeKind;
use unica_metadata::{metadata_name_eq, MetadataType};
use unica_source_designer::owner::MD_CLASSES_NS;
use unica_xml_format::read::direct_text;
use unica_xml_format::text::escape_xml;

use crate::descriptor_comment::{descriptor_properties, direct};
use crate::edit::meta_edit_line_indent;
use crate::predefined::{opening_tag_end, render_type, FragmentXmlContext};

const CORE: &str = "http://v8.1c.ru/8.1/data/core";
const XML_SCHEMA: &str = "http://www.w3.org/2001/XMLSchema";
const CONFIG: &str = "http://v8.1c.ru/8.1/data/enterprise/current-config";

const XML_INSTANCE: &str = "http://www.w3.org/2001/XMLSchema-instance";

/// Bindings used by the generated fragment. Fixed names stay canonical when
/// available, but never shadow the owner's MDClasses prefix.
pub(crate) struct TypeNamespaces {
    bindings: [(String, &'static str); 4],
}

impl TypeNamespaces {
    pub(crate) fn for_scope(scope: Option<Node<'_, '_>>) -> Self {
        let mut context = scope.map_or_else(
            || FragmentXmlContext::from_namespaces(std::iter::empty()),
            FragmentXmlContext::for_node,
        );
        let canonical = [
            ("v8", CORE),
            ("xs", XML_SCHEMA),
            ("xsi", XML_INSTANCE),
            ("cfg", CONFIG),
        ];
        // An alias chosen for one namespace must not be a canonical name
        // that the type renderer will process for a different namespace.
        context
            .reserved_generated_prefixes
            .extend(canonical.map(|(prefix, _)| prefix.to_string()));
        let bindings = canonical.map(|(prefix, namespace)| {
            let selected = if context
                .in_scope_prefixes
                .get(prefix)
                .is_some_and(|bound| bound != namespace)
            {
                context.generated_prefix(namespace, prefix).0
            } else {
                prefix.to_string()
            };
            (selected, namespace)
        });
        Self { bindings }
    }

    pub(crate) fn declarations(&self, scope: Option<Node<'_, '_>>) -> String {
        let mut output = String::new();
        for (prefix, namespace) in &self.bindings {
            if scope.is_none_or(|node| node.lookup_namespace_uri(Some(prefix)) != Some(*namespace))
            {
                output.push_str(&format!(" xmlns:{prefix}=\"{namespace}\""));
            }
        }
        output
    }

    pub(crate) fn xsi(&self) -> &str {
        &self.bindings[2].0
    }

    pub(crate) fn xs(&self) -> &str {
        &self.bindings[1].0
    }

    pub(crate) fn type_xml(&self, value: &MetadataType, indent: &str, eol: &str) -> String {
        let context = FragmentXmlContext::from_namespaces(
            self.bindings
                .iter()
                .map(|(prefix, namespace)| (prefix.as_str(), *namespace)),
        );
        render_type(value, indent, eol, &context)
    }

    pub(crate) fn dimension_type_xml(&self, local: &str) -> String {
        let core = &self.bindings[0].0;
        let config = &self.bindings[3].0;
        format!("<{core}:Type>{config}:{}</{core}:Type>", escape_xml(local))
    }
}

fn canonical_name(properties: Node<'_, '_>) -> Result<String, String> {
    let name = direct(properties, "Name")?;
    if name.children().any(|node| node.is_element()) || name.attributes().len() != 0 {
        return Err("descriptor Name must be canonical scalar text".into());
    }
    let name = direct_text(name);
    if !unica_metadata::identifier::is_1c_identifier(&name) {
        return Err("descriptor Name is not a 1C identifier".into());
    }
    Ok(name)
}

pub(crate) fn properties<'a, 'input>(
    source: &str,
    document: &'a Document<'input>,
    kind: NodeKind,
    name: &str,
    version: &str,
) -> Result<Node<'a, 'input>, String> {
    let root = document.root_element();
    let kind = kind.as_str();
    if unica_source_designer::roots::root_version_literal(source, root).as_deref() != Some(version)
    {
        return Err(format!(
            "the {kind} descriptor must use its owner's format {version}"
        ));
    }
    let properties = descriptor_properties(document, kind)?;
    if canonical_name(properties)? != name {
        return Err(format!(
            "the {kind} descriptor names a different object than {name}"
        ));
    }
    Ok(properties)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChildStorage {
    Reference,
    Inline,
}

fn find_child<'a, 'input>(
    container: Node<'a, 'input>,
    kind: NodeKind,
    name: &str,
    storage: ChildStorage,
) -> Result<Option<Node<'a, 'input>>, String> {
    let kind = kind.as_str();
    let mut found = None;
    for child in container
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == kind)
    {
        if child.tag_name().namespace() != Some(MD_CLASSES_NS) {
            return Err(format!("{kind} has a foreign namespace in ChildObjects"));
        }
        let registered = match storage {
            ChildStorage::Inline => canonical_name(direct(child, "Properties")?)?,
            ChildStorage::Reference => {
                let value = direct_text(child);
                if child.children().any(|node| node.is_element())
                    || child.attributes().len() != 0
                    || !unica_metadata::identifier::is_1c_identifier(&value)
                {
                    return Err(format!("{kind} registration must be a 1C identifier"));
                }
                value
            }
        };
        if metadata_name_eq(&registered, name) {
            if found.is_some() {
                return Err(format!("ambiguous {kind} registration for {name}"));
            }
            found = Some(child);
        }
    }
    Ok(found)
}

pub(crate) fn registered_child<'a, 'input>(
    owner: Node<'a, 'input>,
    kind: NodeKind,
    name: &str,
    storage: ChildStorage,
) -> Result<Option<Node<'a, 'input>>, String> {
    let children = direct(owner, "ChildObjects")?;
    find_child(children, kind, name, storage)
}

fn element_shell(fragment: &str) -> Result<(&str, &str, usize), String> {
    let end = opening_tag_end(fragment)?;
    let head = fragment[..end].trim_end_matches('/').trim_end();
    let spelling = head[1..]
        .split_ascii_whitespace()
        .next()
        .ok_or_else(|| "element opening tag has no name".to_string())?;
    Ok((head, spelling, end))
}

pub(crate) fn physical_reference(kind: NodeKind, name: &str) -> String {
    let tag = kind.as_str();
    format!(
        "<{tag} xmlns=\"{MD_CLASSES_NS}\">{}</{tag}>",
        escape_xml(name)
    )
}

pub(crate) fn insert_child(
    source: &str,
    owner: Node<'_, '_>,
    kind: NodeKind,
    name: &str,
    storage: ChildStorage,
    fragment: &str,
) -> Result<String, String> {
    let children = direct(owner, "ChildObjects")?;
    if find_child(children, kind, name, storage)?.is_some() {
        return Err(format!(
            "the owner already contains {}.{name}",
            kind.as_str()
        ));
    }
    let kind = kind.as_str();
    let order = unica_address::kinds::declared(owner.tag_name().name())
        .and_then(|(_, declaration)| declaration.child_order)
        .ok_or_else(|| "owner has no child object order".to_string())?;
    let slot = order
        .iter()
        .position(|tag| *tag == kind)
        .ok_or_else(|| format!("{kind} is not a child of this owner"))?;
    let raw = &source[children.range()];
    let (opening, spelling, end) = element_shell(raw)?;
    let eol = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let indent = meta_edit_line_indent(source, children.range().start);
    let child_indent = format!("{eol}{indent}\t");
    let fragment = fragment.replace('\n', &child_indent);
    let mut updated = source.to_string();
    if raw[..end].ends_with('/') {
        updated.replace_range(
            children.range(),
            &format!("{opening}>{eol}{indent}\t{fragment}{eol}{indent}</{spelling}>"),
        );
    } else {
        let later = children.children().find(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_CLASSES_NS)
                && order
                    .iter()
                    .position(|tag| *tag == node.tag_name().name())
                    .is_some_and(|position| position > slot)
        });
        let (at, after_indent) = match later {
            Some(node) => (
                node.range().start,
                meta_edit_line_indent(source, node.range().start),
            ),
            None => (
                children.range().start + raw.rfind("</").ok_or("ChildObjects has no close")?,
                indent.clone(),
            ),
        };
        let extra_indent = if later.is_none() { "\t" } else { "" };
        updated.insert_str(at, &format!("{extra_indent}{fragment}{eol}{after_indent}"));
    }
    Document::parse(&updated).map_err(|error| format!("invalid child postimage: {error}"))?;
    Ok(updated)
}

#[derive(Clone, Copy)]
pub(crate) enum Property<'a> {
    Scalar(&'a str),
    Synonym(&'a [(String, String)]),
    Type(&'a MetadataType),
    DimensionType(&'a str),
}

pub(crate) fn patch_property(
    source: &str,
    properties: Node<'_, '_>,
    tag: &str,
    change: Property<'_>,
) -> Result<String, String> {
    let property = direct(properties, tag)?;
    let range = property.range();
    let raw = &source[range.clone()];
    let (opening, spelling, _) = element_shell(raw)?;
    let indent = meta_edit_line_indent(source, range.start);
    let eol = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let replacement = match change {
        Property::Scalar(value) => {
            if property.children().any(|node| node.is_element()) || property.attributes().len() != 0
            {
                return Err(format!("{tag} must be canonical scalar text"));
            }
            if direct_text(property) == value {
                return Ok(source.into());
            }
            if value.is_empty() {
                format!("{opening}/>")
            } else {
                let escaped = escape_xml(value)
                    .replace('\r', "&#13;")
                    .replace('\n', "&#10;")
                    .replace('\t', "&#9;");
                format!("{opening}>{escaped}</{spelling}>")
            }
        }
        Property::Synonym(values) => {
            crate::descriptor_fields::synonym(property)?;
            unica_xml_format::localized::replace_language_values(source, property, &indent, values)
        }
        Property::Type(value) => {
            let observed =
                crate::info_projection::parse_observed_metadata_type_container_node(property)
                    .map_err(|error| error.message)?;
            if MetadataType::try_from(observed).as_ref().ok() == Some(value) {
                return Ok(source.into());
            }
            let namespaces = TypeNamespaces::for_scope(Some(property));
            let declarations = namespaces.declarations(Some(property));
            let xml = namespaces.type_xml(value, &indent, eol);
            let (head, tail) = xml.split_once(eol).ok_or("generated Type has no body")?;
            let body = tail
                .strip_suffix(&format!("{eol}{indent}</Type>"))
                .ok_or("generated Type has no closing element")?;
            debug_assert_eq!(head.trim_start(), "<Type>");
            format!("{opening}{declarations}>{eol}{body}{eol}{indent}</{spelling}>")
        }
        Property::DimensionType(local_name) => {
            let items = property
                .children()
                .filter(Node::is_element)
                .collect::<Vec<_>>();
            let [item] = items.as_slice() else {
                return Err("Dimension Type must contain exactly one qualified type".into());
            };
            let actual = direct_text(*item);
            if !item.has_tag_name((CORE, "Type"))
                || item.attributes().len() != 0
                || item.children().any(|child| child.is_element())
            {
                return Err("Dimension Type must contain canonical QName text".into());
            }
            let (namespace, local) =
                crate::info_projection::expand_observed_type_qname(*item, &actual)
                    .map_err(|error| error.message)?;
            if namespace != CONFIG || !local.starts_with("ExternalDataSourceCubeDimensionTableRef.")
            {
                return Err("Dimension Type must be a reference to a dimension table".into());
            }
            if local_name == local {
                return Ok(source.into());
            }
            let namespaces = TypeNamespaces::for_scope(Some(property));
            let declarations = namespaces.declarations(Some(property));
            let element = namespaces.dimension_type_xml(local_name);
            format!("{opening}{declarations}>{eol}{indent}\t{element}{eol}{indent}</{spelling}>")
        }
    };
    let mut updated = source.to_string();
    updated.replace_range(range, &replacement);
    Document::parse(&updated).map_err(|error| format!("invalid {tag} postimage: {error}"))?;
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_member_update_keeps_both_identities_and_untouched_properties() {
        let source = format!(
            "<MetaDataObject xmlns='{MD_CLASSES_NS}' version='2.20'>\r\n<Table uuid='old-table'><Properties><Name>Items</Name></Properties><ChildObjects><Field uuid='first'><Properties><Name>Code</Name><Comment>old</Comment></Properties></Field><Field uuid='second'><Properties><Name>Unchanged</Name><Comment>old</Comment></Properties></Field></ChildObjects></Table></MetaDataObject>"
        );
        let document = Document::parse(&source).unwrap();
        let table_properties =
            properties(&source, &document, NodeKind::Table, "Items", "2.20").unwrap();
        let field = registered_child(
            table_properties.parent_element().unwrap(),
            NodeKind::Field,
            "Code",
            ChildStorage::Inline,
        )
        .unwrap()
        .unwrap();
        let updated = patch_property(
            &source,
            direct(field, "Properties").unwrap(),
            "Comment",
            Property::Scalar("ru & en"),
        )
        .unwrap();
        assert_eq!(
            updated,
            source.replacen(
                "<Comment>old</Comment>",
                "<Comment>ru &amp; en</Comment>",
                1
            )
        );
        assert!(registered_child(
            table_properties.parent_element().unwrap(),
            NodeKind::Field,
            "Other",
            ChildStorage::Inline
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn external_child_registration_inserts_only_in_the_owner_and_respects_kind_order() {
        let source = format!("<MetaDataObject xmlns='{MD_CLASSES_NS}' version='2.20'><ExternalDataSource><Properties><Name>Warehouse</Name></Properties><ChildObjects><Cube>Sales</Cube><Function><Properties><Name>Total</Name></Properties></Function></ChildObjects></ExternalDataSource></MetaDataObject>");
        let document = Document::parse(&source).unwrap();
        let owner = document.root_element().first_element_child().unwrap();
        let updated = insert_child(
            &source,
            owner,
            NodeKind::Table,
            "NewTable",
            ChildStorage::Reference,
            &physical_reference(NodeKind::Table, "NewTable"),
        )
        .unwrap();
        assert!(updated.contains(&format!(
            "<ChildObjects><Table xmlns=\"{MD_CLASSES_NS}\">NewTable</Table>\n<Cube>"
        )));
        let document = Document::parse(&updated).unwrap();
        let owner = document.root_element().first_element_child().unwrap();
        assert!(insert_child(
            &updated,
            owner,
            NodeKind::Table,
            "newtable",
            ChildStorage::Reference,
            "ignored"
        )
        .is_err());
    }

    #[test]
    fn editing_one_external_table_field_changes_only_type_and_table_synonym() {
        let original = format!(
            r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" xmlns:v8="{CORE}" xmlns:xs="{XML_SCHEMA}" xmlns:cfg="{CONFIG}" version="2.20">
    <Table uuid="table"><Properties><Name>Items</Name><Synonym/><TableType>Table</TableType></Properties>
        <ChildObjects>
            <Field uuid="field"><Properties><Name>Code</Name>
                <Type><v8:Type>xs:decimal</v8:Type><v8:NumberQualifiers><v8:Digits>10</v8:Digits><v8:FractionDigits>2</v8:FractionDigits><v8:AllowedSign>Any</v8:AllowedSign></v8:NumberQualifiers></Type>
                <NameInDataSource>code</NameInDataSource><ReadOnly>false</ReadOnly>
            </Properties></Field>
            <Field uuid="untouched"><Properties><Name>Other</Name><Comment>same</Comment></Properties></Field>
        </ChildObjects>
    </Table>
</MetaDataObject>"#
        );
        fn edit_site<'a, 'input>(
            source: &str,
            document: &'a Document<'input>,
        ) -> (Node<'a, 'input>, Node<'a, 'input>) {
            let table = properties(source, document, NodeKind::Table, "Items", "2.20").unwrap();
            let field = registered_child(
                table.parent_element().unwrap(),
                NodeKind::Field,
                "Code",
                ChildStorage::Inline,
            )
            .unwrap()
            .unwrap();
            (table, direct(field, "Properties").unwrap())
        }
        fn mask(source: &str) -> String {
            let document = Document::parse(source).unwrap();
            let (table, field) = edit_site(source, &document);
            let mut masked = source.to_string();
            masked.replace_range(direct(field, "Type").unwrap().range(), "$type");
            masked.replace_range(direct(table, "Synonym").unwrap().range(), "$synonym");
            masked
        }
        let second_table = original
            .replace("Items", "OtherItems")
            .replace("uuid=\"field\"", "uuid=\"other-code\"");
        let document = Document::parse(&original).unwrap();
        let (table, _) = edit_site(&original, &document);
        let edits = vec![
            ("ru".to_string(), "Синоним".to_string()),
            ("en".to_string(), "Synonym".to_string()),
        ];
        let next = patch_property(&original, table, "Synonym", Property::Synonym(&edits)).unwrap();
        let document = Document::parse(&next).unwrap();
        let (_, field) = edit_site(&next, &document);
        let boolean =
            MetadataType::new(vec![unica_metadata::MetadataTypeVariant::Boolean]).unwrap();
        let post = patch_property(&next, field, "Type", Property::Type(&boolean)).unwrap();
        assert_eq!(mask(&post), mask(&original));
        assert!(post.contains("<v8:Type>xs:boolean</v8:Type>"));
        assert!(second_table.contains("<v8:Type>xs:decimal</v8:Type>"));
        assert!(second_table.contains("uuid=\"other-code\""));
    }
}
