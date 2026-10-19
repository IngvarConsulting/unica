//! Metadata argument shapes shared by Can and staged decoding.

use super::{object, text, OperationContract};
use serde_json::{json, Map, Value};

const METADATA_HELP: &str = "skills/meta-edit/SKILL.md";

pub(crate) fn metadata_object_create_input_schema() -> &'static Value {
    static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    SCHEMA.get_or_init(|| {
        object(
            &[
                ("at", text(true)),
                ("values", object(&[
                    ("kind", json!({"type":"string","enum":crate::domain::metadata::MetadataKind::ALL.iter().map(|kind| kind.as_str()).collect::<Vec<_>>()})),
                    // The identifier is checked during staging. Raw decoding
                    // must not move it ahead of a later operation's error.
                    ("name", json!({"type":"string","minLength":1})),
                ], &["kind", "name"])),
            ],
            &["values"],
        )
    })
}

pub(crate) fn metadata_object_remove_input_schema() -> &'static Value {
    static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    SCHEMA.get_or_init(|| {
        object(
            &[
                ("at", text(true)),
                ("force", json!({"type":"boolean","default":false})),
            ],
            &[],
        )
    })
}

pub(crate) fn metadata_borrow_input_schema() -> &'static Value {
    static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    SCHEMA.get_or_init(|| object(&[
        ("at", text(true)),
        ("from", json!({"type":"string","minLength":1})),
        ("overrides", json!({"type":"array","uniqueItems":true,"items":{"type":"string","minLength":1}})),
    ], &["from"]))
}

pub(crate) fn metadata_help_create_input_schema() -> &'static Value {
    static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    SCHEMA.get_or_init(|| {
        object(
            &[
                ("at", text(true)),
                (
                    "values",
                    object(
                        &[(
                            "lang",
                            json!({"type":"string","minLength":1,"pattern":"^[A-Za-z0-9]+$"}),
                        )],
                        &["lang"],
                    ),
                ),
            ],
            &["values"],
        )
    })
}

pub(crate) fn metadata_relation_input_schema() -> &'static Value {
    static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    SCHEMA.get_or_init(|| {
        object(
            &[
                ("at", text(true)),
                (
                    "values",
                    crate::domain::metadata::metadata_relation_values_input_schema().clone(),
                ),
            ],
            &["values"],
        )
    })
}

/// The node a metadata member contract is asked for: the owner kind and
/// whether the address lies inside a tabular section. Field legality that
/// depends on them (fill values, predefined-item fields) is published only
/// for the node it holds on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MetadataSite {
    pub(crate) owner: crate::domain::metadata::MetadataKind,
    pub(crate) in_section: bool,
}

/// `fillValue` is accepted for `collection` at `site`; without a site, for any
/// owner that has the collection.
fn fill_value_allowed(
    collection: crate::domain::metadata::MetaCollection,
    site: Option<MetadataSite>,
) -> bool {
    use crate::domain::metadata::{
        metadata_collection_spec, metadata_fill_value_is_allowed, MetaElementScope,
    };
    match site {
        None => metadata_collection_spec(collection).allows_fill_value,
        Some(site) => metadata_fill_value_is_allowed(
            site.owner,
            collection,
            if site.in_section {
                MetaElementScope::TabularSection
            } else {
                MetaElementScope::TopLevel
            },
        ),
    }
}

pub(super) const IDENTIFIER_PATTERN: &str = r"^[_A-Za-zА-Яа-яЁё][_A-Za-zА-Яа-яЁё0-9]*$";

/// Field definitions for both the raw typed input and the contextual Can.
/// Raw decoding knows the union; legality belongs to MetaCollectionSpec and
/// conversion, so an inapplicable field does not reorder an earlier error.
fn metadata_element_base_fields() -> &'static Map<String, Value> {
    use crate::domain::metadata::{
        fill_value_schema, metadata_type_schema, position_schema, MetaTemplateKind,
    };
    static FIELDS: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
    FIELDS.get_or_init(|| Map::from_iter([
        ("name".into(), json!({"type":"string","minLength":1})),
        ("newName".into(), json!({"type":"string","minLength":1})),
        ("synonym".into(), json!({"type":"string"})),
        ("comment".into(), json!({"type":"string"})),
        ("type".into(), metadata_type_schema()),
        ("required".into(), json!({"type":"boolean"})),
        ("fillValue".into(), fill_value_schema()),
        ("position".into(), position_schema()),
        (
            "indexing".into(),
            json!({"type":"string","enum":["DontIndex","Index","IndexWithAdditionalOrder"]}),
        ),
        (
            "templateType".into(),
            json!({"type":"string","enum":MetaTemplateKind::ALL.iter().map(|kind| kind.as_str()).collect::<Vec<_>>(),
            "description":"Template kind for a new templates element; omitted it defaults to SpreadsheetDocument (ADR-0072)."}),
        ),
    ]))
}

fn metadata_element_fields(attributes: bool) -> Map<String, Value> {
    let mut fields = metadata_element_base_fields().clone();
    if attributes {
        fields.insert(
            "attributes".into(),
            metadata_element_attributes_schema(metadata_element_input_schema(false, false).clone()),
        );
    }
    fields
}

fn metadata_element_attributes_schema(items: Value) -> Value {
    json!({"type":"array","minItems":1,"items":items})
}

/// Existing internal input: update also carries the synthetic selector name.
/// The finite cached shapes are shared with the productive decoder; this is
/// not a second operation registry or a new argument language.
pub(crate) fn metadata_element_input_schema(update: bool, attributes: bool) -> &'static Value {
    use std::sync::OnceLock;
    static ADD: OnceLock<Value> = OnceLock::new();
    static ADD_NESTED: OnceLock<Value> = OnceLock::new();
    static UPDATE: OnceLock<Value> = OnceLock::new();
    let cache = if update {
        &UPDATE
    } else if attributes {
        &ADD
    } else {
        &ADD_NESTED
    };
    cache.get_or_init(|| {
        let mut fields = metadata_element_fields(!update && attributes);
        if !update {
            fields.remove("newName");
            if !attributes {
                fields.remove("templateType");
            }
        } else {
            fields.remove("templateType");
        }
        json!({"type":"object","additionalProperties":false,"required":["name"],"properties":fields})
    })
}

/// Preserve the host-visible internal envelope while deriving its field
/// definitions from the same input owner. The old envelope is deliberately
/// open and describes nested attributes shallowly; it is not the Can contract.
pub(crate) fn metadata_host_element_schema() -> Value {
    let mut fields = metadata_element_fields(true);
    fields.remove("indexing");
    fields["type"] = json!({"$ref":"#/$defs/metadataType"});
    fields["fillValue"] = json!({"$ref":"#/$defs/fillValue"});
    fields["position"] = json!({"$ref":"#/$defs/position"});
    fields["attributes"]["items"] = json!({"type":"object","required":["name"]});
    json!({"type":"object","required":metadata_element_input_schema(false, false)["required"],"properties":fields,
        "description":"Collection element; the legal fields depend on the object kind and collection."})
}

/// Contextual publication is a view of the input fields. The address and
/// collection constrain legality; they do not invent another raw key list.
fn metadata_element_schema(
    collection: crate::domain::metadata::MetaCollection,
    update: bool,
    site: Option<MetadataSite>,
) -> Value {
    use crate::domain::metadata::{metadata_collection_spec, MetaCollection};
    let spec = metadata_collection_spec(collection);
    let mut fields: Map<String, Value> = metadata_element_base_fields()
        .iter()
        .filter(|(name, _)| match name.as_str() {
            "name" => !update,
            "newName" => update,
            "type" => spec.allows_type,
            "required" => spec.allows_required,
            "fillValue" => fill_value_allowed(collection, site),
            "position" => spec.allows_position,
            "indexing" => spec.allows_indexing,
            "templateType" => false,
            "synonym" | "comment" => true,
            _ => unreachable!("unclassified Metadata input field: {name}"),
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let selector = if update { "newName" } else { "name" };
    fields[selector]
        .as_object_mut()
        .expect("identifier field")
        .remove("minLength");
    fields[selector]["pattern"] = json!(IDENTIFIER_PATTERN);
    if spec.allows_nested_attributes && !update {
        fields.insert(
            "attributes".into(),
            metadata_element_attributes_schema(metadata_element_schema(
                MetaCollection::Attributes,
                false,
                site.map(|site| MetadataSite {
                    in_section: true,
                    ..site
                }),
            )),
        );
    }
    let mut schema = json!({"type":"object","additionalProperties":false,"properties":fields});
    if update {
        schema["minProperties"] = json!(1);
    } else {
        schema["required"] = metadata_element_input_schema(false, false)["required"].clone();
    }
    schema
}

/// Fields of a predefined item: `add` names a new item with its `id`, `set`
/// changes an existing one, named by the address (a rename is `newName`).
/// With an owner kind only the fields that kind accepts are published.
fn predefined_item_schema(
    update: bool,
    owner: Option<crate::domain::metadata::MetadataKind>,
) -> Value {
    use crate::domain::metadata::{metadata_predefined_kind_fields, metadata_type_schema};
    let mut fields = Map::new();
    if update {
        fields.insert(
            "newName".into(),
            json!({"type":"string","pattern":IDENTIFIER_PATTERN}),
        );
    } else {
        fields.insert(
            "id".into(),
            json!({"type":"string","pattern":"^[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}$"}),
        );
        fields.insert(
            "name".into(),
            json!({"type":"string","pattern":IDENTIFIER_PATTERN}),
        );
    }
    let kind_fields = owner.map(metadata_predefined_kind_fields);
    for (name, schema) in [
        ("code", json!({"type":"string"})),
        ("description", json!({"type":"string"})),
        ("isFolder", json!({"type":"boolean"})),
        ("type", metadata_type_schema()),
        (
            "accountType",
            json!({"type":"string","enum":["Active","Passive","ActivePassive"]}),
        ),
        ("offBalance", json!({"type":"boolean"})),
        ("order", json!({"type":"string"})),
        (
            "accountingFlags",
            json!({"type":"object","additionalProperties":{"type":"boolean"}}),
        ),
        (
            "extDimensionTypes",
            json!({"type":"array","items":{"type":"object","additionalProperties":false,
                "required":["name"],"properties":{"name":{"type":"string","minLength":1},
                "turnover":{"type":"boolean"},
                "accountingFlags":{"type":"object","additionalProperties":{"type":"boolean"}}}}}),
        ),
        ("actionPeriodIsBase", json!({"type":"boolean"})),
    ] {
        let common = matches!(name, "code" | "description");
        if common || kind_fields.is_none_or(|allowed| allowed.contains(&name)) {
            fields.insert(name.into(), schema);
        }
    }
    let mut schema = json!({"type":"object","additionalProperties":false,"properties":fields});
    if update {
        schema["minProperties"] = json!(1);
    } else {
        schema["required"] = json!(["id", "name"]);
    }
    schema
}

impl OperationContract {
    /// Contracts of the member operations of a metadata object: `*.add` on
    /// the owner (attributes also on a tabular section), `*.set` and
    /// `*.remove` on the member itself. The schema publishes the fields the
    /// typed writer accepts for the collection; the planner parses the same
    /// arguments with the writer's own request parser.
    pub(crate) fn metadata(op: &str) -> Option<Self> {
        Self::metadata_at(op, None)
    }

    pub(in crate::domain) fn metadata_member_add(
        collection: crate::domain::metadata::MetaCollection,
        site: Option<MetadataSite>,
    ) -> Self {
        use crate::domain::metadata::MetaCollection;
        let target = if collection == MetaCollection::Attributes {
            "A metadata object with attributes: <source-set>:<Kind>.<Name> adds header attributes; <source-set>:<Kind>.<Name>.TabularSection.<T> adds attributes of that tabular section."
        } else {
            "A metadata object with this collection: <source-set>:<Kind>.<Name>."
        };
        let example_args = match collection {
            MetaCollection::Attributes | MetaCollection::Dimensions => {
                json!({"items":[{"name":"Sample",
                "type":{"variants":[{"kind":"string","length":50,"allowedLength":"variable"}]}}]})
            }
            // Ресурс регистра накопления числовой: пример годится всем регистрам.
            MetaCollection::Resources => json!({"items":[{"name":"Sample",
                "type":{"variants":[{"kind":"number","digits":15,"fraction":2,"sign":"any"}]}}]}),
            _ => json!({"items":[{"name":"Sample"}]}),
        };
        Self {
            schema: object(
                &[
                    ("at", text(true)),
                    ("items", json!({"type":"array","minItems":1,"items":metadata_element_schema(collection, false, site)})),
                ],
                &["items"],
            ),
            target,
            effect: "Add new members after the existing ones, or at position.before/position.after an existing sibling. A name already used in the collection is refused.",
            notes: "Read the collection branch of the target before choosing a new valid 1C identifier. A new node is addressable only after the plan is executed; within one apply call a later operation may already address it. Cross-field rules — fillValue needs an explicit type and must match it — are checked by the planner, not by this schema.",
            example_args,
            help: METADATA_HELP,
        }
    }

    pub(in crate::domain) fn metadata_member_set(
        collection: crate::domain::metadata::MetaCollection,
        site: Option<MetadataSite>,
    ) -> Self {
        Self {
            schema: object(
                &[
                    ("at", text(true)),
                    ("values", metadata_element_schema(collection, true, site)),
                ],
                &["values"],
            ),
            target: if collection == crate::domain::metadata::MetaCollection::Attributes {
                "The existing attribute: <source-set>:<Kind>.<Name>.Attribute.<A> or <source-set>:<Kind>.<Name>.TabularSection.<T>.Attribute.<A>."
            } else {
                "The existing member at its own address: <source-set>:<Kind>.<Name>.<Leaf>.<Member>."
            },
            effect: "Change the listed fields of the member; newName renames it. Unlisted fields stay.",
            notes: "The member is named by its address, not by values.name. Read the member first. Cross-field rules — fillValue must match the member type — are checked by the planner, not by this schema.",
            example_args: json!({"values":{"synonym":"Sample"}}),
            help: METADATA_HELP,
        }
    }

    pub(in crate::domain) fn metadata_member_remove() -> Self {
        Self {
            schema: object(&[("at", text(true))], &[]),
            target: "The existing member at its own address: <source-set>:<Kind>.<Name>.<Leaf>.<Member>.",
            effect: "Remove the member from its owner.",
            notes: "The member is named by its address; the operation takes no other arguments.",
            example_args: json!({}),
            help: METADATA_HELP,
        }
    }

    /// The contract on a concrete node: fields whose legality depends on the
    /// owner kind or on the tabular section are narrowed to that node.
    pub(crate) fn metadata_at(op: &str, site: Option<MetadataSite>) -> Option<Self> {
        let (family, verb) = op.split_once('.')?;
        let owner = site.map(|site| site.owner);
        let at = || ("at", text(true));
        if family == "object" {
            let (schema, target, effect, notes, example_args, help) = match verb {
                "create" => {
                    let mut schema = metadata_object_create_input_schema().clone();
                    schema["properties"]["values"]["properties"]["name"]["pattern"] = json!(IDENTIFIER_PATTERN);
                    (schema,
                        "The Configuration root: <source-set>:Configuration.",
                        "Create and register a new metadata object with its minimal supported files.",
                        "Choose an unused valid 1C identifier. The operation accepts kind and name; configure properties and members with subsequent operations in the same apply call.",
                        json!({"values":{"kind":"Catalog","name":"Sample"}}),
                        "skills/meta-add/SKILL.md")
                }
                "remove" => (metadata_object_remove_input_schema().clone(),
                    "The existing metadata object: <source-set>:<Kind>.<Name>.",
                    "Remove the object descriptor, its resource directory and its registration; update subsystem membership.",
                    "References from other objects are checked. force=true keeps those references and reports references_kept; it does not repair them. Under the deny support policy, a vendor object must be taken off support in a separate apply call first.",
                    json!({}), METADATA_HELP),
                "borrow" => (metadata_borrow_input_schema().clone(),
                    "The extension Configuration root: <extension-source-set>:Configuration.",
                    "Borrow or refresh the named parent object, preserving the extension's identity and local data.",
                    "from is a qualified address in a distinct admitted parent source set: a named top-level object or Enum.EnumValue. Read that source first. overrides names supported transferred properties; an empty array is allowed for a top-level object but any overrides field is refused for EnumValue. Refresh preserves the established override set.",
                    json!({"from":"parent:Catalog.Sample"}), "skills/cfe-borrow/SKILL.md"),
                _ => return None,
            };
            return Some(Self {
                schema,
                target,
                effect,
                notes,
                example_args,
                help,
            });
        }
        if family == "help" && verb == "create" {
            return Some(Self {
                schema: metadata_help_create_input_schema().clone(),
                target: "The existing metadata object: <source-set>:<Kind>.<Name>.",
                effect: "Create Ext/Help.xml and Ext/Help/<lang>.html; add IncludeHelpInContents=false to owner forms that lack that property.",
                notes: "lang is required and contains only ASCII letters and digits, for example ru or en. Creation is refused if help already exists; existing help and forms are preserved. Plan this root-inspecting operation before other changes in the batch.",
                example_args: json!({"values":{"lang":"ru"}}),
                help: METADATA_HELP,
            });
        }
        if family == "relation" && matches!(verb, "add" | "remove" | "replace") {
            use crate::domain::metadata::{
                metadata_relation_specs, metadata_relation_values_schema, MetaRelation,
            };
            let relation = owner
                .map(|kind| {
                    metadata_relation_specs(kind)
                        .first()
                        .map(|spec| spec.relation)
                })
                .unwrap_or(Some(MetaRelation::Owners));
            let example_args = relation
                .map(|relation| {
                    let targets = match relation {
                        MetaRelation::Owners | MetaRelation::BasedOn => {
                            json!([{"metadataPath":"Catalog.Sample"}])
                        }
                        MetaRelation::RegisterRecords => {
                            json!([{"metadataPath":"InformationRegister.Sample"}])
                        }
                        MetaRelation::InputByString => {
                            json!([{"fieldPath":"Catalog.Sample.Attribute.Code"}])
                        }
                        MetaRelation::Source => {
                            json!([{"kind":"family","sourceClass":"catalogObject"}])
                        }
                    };
                    json!({"values":{"relation":relation.as_str(),"targets":targets}})
                })
                .unwrap_or_else(|| json!({}));
            let mut schema = metadata_relation_input_schema().clone();
            schema["properties"]["values"] = metadata_relation_values_schema(owner);
            return Some(Self {
                schema,
                target: "The existing metadata object that owns this relation: <source-set>:<Kind>.<Name>.",
                effect: match verb {
                    "add" => "Add the listed relation targets, preserving existing targets.",
                    "remove" => "Remove the listed relation targets, preserving other targets.",
                    "replace" => "Replace the relation's logical targets with the listed set.",
                    _ => unreachable!("the relation verb was checked"),
                },
                notes: if relation.is_none() {
                    "This owner kind has no supported relations. Its relation field rejects every value; there is no executable argument example on this node."
                } else {
                    "The owner kind narrows the relation and target shape. targets is nonempty; mode is selected by the operation name. Read targets first. Field existence, same-owner paths, source-class compatibility and the final nonempty EventSubscription source are checked by the planner. Existing XML namespaces and unrelated data are preserved."
                },
                example_args,
                help: METADATA_HELP,
            });
        }
        if family == "predefinedItem" {
            let (schema, target, effect, example_args) = match verb {
                "add" => (
                    object(&[at(), ("items", json!({"type":"array","minItems":1,"items":predefined_item_schema(false, owner)}))], &["items"]),
                    "A metadata object with predefined items: <source-set>:<Kind>.<Name>. Read its PredefinedItem branch first; a new item needs a new UUID id and a new name.",
                    "Add predefined items to Ext/Predefined.xml of the owner; the file is created with the first item.",
                    json!({"items":[{"id":"a1b2c3d4-1111-4111-8111-111111111111","name":"Sample","description":"Sample item"}]}),
                ),
                "set" => (
                    object(&[at(), ("values", predefined_item_schema(true, owner))], &["values"]),
                    "The existing item: <source-set>:<Kind>.<Name>.PredefinedItem.<ItemName>. The id is not an input.",
                    "Change the listed fields of the item; newName renames it. Unlisted fields stay.",
                    json!({"values":{"description":"Sample item"}}),
                ),
                "remove" => (
                    object(&[at()], &[]),
                    "The existing item: <source-set>:<Kind>.<Name>.PredefinedItem.<ItemName>.",
                    "Remove the item with its nested items; the file is removed with the last item.",
                    json!({}),
                ),
                _ => return None,
            };
            return Some(Self {
                schema,
                target,
                effect,
                notes: "Predefined items are named by their address; there is no id or name selector argument. Item names are unique within the owner at every depth; an add or rename onto a used name is refused. The fields beyond code and description depend on the owner kind.",
                example_args,
                help: METADATA_HELP,
            });
        }
        if family == "template" {
            let (schema, effect, example_args) = match verb {
                "set" => (
                    object(&[at(), ("values", json!({"type":"object","additionalProperties":false,"minProperties":1,
                        "properties":{"synonym":{"type":"string"},"comment":{"type":"string"},
                            "templateType":{"type":"string","enum":crate::domain::metadata::MetaTemplateKind::ALL.iter().map(|kind| kind.as_str()).collect::<Vec<_>>()}}}))], &["values"]),
                    "Change the listed properties of the template descriptor. Unlisted properties stay.",
                    json!({"values":{"synonym":"Sample"}}),
                ),
                "remove" => (
                    object(&[at()], &[]),
                    "Remove the template descriptor and its content, and clear owner properties that pointed at it.",
                    json!({}),
                ),
                _ => return None,
            };
            return Some(Self {
                schema,
                target: "The existing template: <source-set>:<Kind>.<Name>.Template.<T>.",
                effect,
                notes: "The template is named by its address; there is no name argument.",
                example_args,
                help: METADATA_HELP,
            });
        }
        None
    }
}
