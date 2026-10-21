//! Metadata argument shapes shared by Can and staged decoding.

use crate::{MetadataKind, RelationEditMode};
use serde_json::{json, Map, Value};
use unica_operation_contract::{object, text, OperationContract};

const METADATA_HELP: &str = "skills/meta-edit/SKILL.md";

pub fn metadata_object_create_input_schema() -> &'static Value {
    static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    SCHEMA.get_or_init(|| {
        object(
            &[
                ("at", text(true)),
                ("values", object(&[
                    ("kind", json!({"type":"string","enum":crate::MetadataKind::ALL.iter().map(|kind| kind.as_str()).collect::<Vec<_>>()})),
                    // The identifier is checked during staging. Raw decoding
                    // must not move it ahead of a later operation's error.
                    ("name", json!({"type":"string","minLength":1})),
                ], &["kind", "name"])),
            ],
            &["values"],
        )
    })
}

pub fn metadata_object_remove_input_schema() -> &'static Value {
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

pub fn metadata_borrow_input_schema() -> &'static Value {
    static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    SCHEMA.get_or_init(|| object(&[
        ("at", text(true)),
        ("from", json!({"type":"string","minLength":1})),
        ("overrides", json!({"type":"array","uniqueItems":true,"items":{"type":"string","minLength":1}})),
    ], &["from"]))
}

pub fn metadata_help_create_input_schema() -> &'static Value {
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

pub fn metadata_relation_input_schema() -> &'static Value {
    static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    SCHEMA.get_or_init(|| {
        object(
            &[
                ("at", text(true)),
                (
                    "values",
                    crate::metadata_relation_values_input_schema().clone(),
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
pub struct MetadataSite {
    pub owner: crate::MetadataKind,
    pub in_section: bool,
}

/// `fillValue` is accepted for `collection` at `site`; without a site, for any
/// owner that has the collection.
fn fill_value_allowed(collection: crate::MetaCollection, site: Option<MetadataSite>) -> bool {
    use crate::{metadata_collection_spec, metadata_fill_value_is_allowed, MetaElementScope};
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

pub const IDENTIFIER_PATTERN: &str = r"^[_A-Za-zА-Яа-яЁё][_A-Za-zА-Яа-яЁё0-9]*$";

/// Field definitions for both the raw typed input and the contextual Can.
/// Raw decoding knows the union; legality belongs to MetaCollectionSpec and
/// conversion, so an inapplicable field does not reorder an earlier error.
fn metadata_element_base_fields() -> &'static Map<String, Value> {
    use crate::{fill_value_schema, metadata_type_schema, position_schema, MetaTemplateKind};
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
pub fn metadata_element_input_schema(update: bool, attributes: bool) -> &'static Value {
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
pub fn metadata_host_element_schema() -> Value {
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
    collection: crate::MetaCollection,
    update: bool,
    site: Option<MetadataSite>,
) -> Value {
    use crate::{metadata_collection_spec, MetaCollection};
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

/// The existing host input knows every predefined field. Identifier, UUID
/// and owner legality remain with the typed converters in their old order.
pub fn metadata_predefined_element_input_schema(update: bool) -> &'static Value {
    use crate::{metadata_type_schema, MetaPredefinedAccountType};
    static ADD: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    static UPDATE: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    let cache = if update { &UPDATE } else { &ADD };
    cache.get_or_init(|| {
        let name = json!({"type":"string","minLength":1});
        let flags = json!({"type":"object","additionalProperties":{"type":"boolean"}});
        let fields = Map::from_iter([
            ("id".into(), json!({"type":"string","minLength":1})),
            ("name".into(), name.clone()),
            ("code".into(), text(false)),
            ("description".into(), text(false)),
            ("isFolder".into(), json!({"type":"boolean"})),
            ("type".into(), metadata_type_schema()),
            ("accountType".into(), json!({"type":"string","enum":MetaPredefinedAccountType::ALL.iter().map(|kind| kind.as_str()).collect::<Vec<_>>()})),
            ("offBalance".into(), json!({"type":"boolean"})),
            ("order".into(), text(false)),
            ("accountingFlags".into(), flags.clone()),
            ("extDimensionTypes".into(), json!({"type":"array","items":{"type":"object","additionalProperties":false,
                "required":["name"],"properties":{"name":name,"turnover":{"type":"boolean"},"accountingFlags":flags}}})),
            ("actionPeriodIsBase".into(), json!({"type":"boolean"})),
        ]);
        json!({"type":"object","additionalProperties":false,
            "required":if update { json!(["id"]) } else { json!(["id", "name"]) },
            "properties":fields})
    })
}

/// The canonical address selects the item; the host's optional `name`
/// becomes `newName`. Publish only the fields the owner kind accepts.
fn predefined_item_schema(update: bool, owner: Option<crate::MetadataKind>) -> Value {
    use crate::metadata_predefined_kind_fields;
    let input = metadata_predefined_element_input_schema(update);
    let input_fields = input["properties"]
        .as_object()
        .expect("predefined input fields");
    let mut name = input_fields["name"].clone();
    name.as_object_mut()
        .expect("name field")
        .remove("minLength");
    name["pattern"] = json!(IDENTIFIER_PATTERN);
    let mut fields = Map::new();
    if update {
        fields.insert("newName".into(), name);
    } else {
        let mut id = input_fields["id"].clone();
        id.as_object_mut().expect("id field").remove("minLength");
        id["pattern"] =
            json!("^[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}$");
        fields.insert("id".into(), id);
        fields.insert("name".into(), name);
    }
    let kind_fields = owner.map(metadata_predefined_kind_fields);
    for (name, schema) in input_fields {
        if matches!(name.as_str(), "id" | "name") {
            continue;
        }
        let common = matches!(name.as_str(), "code" | "description");
        if common || kind_fields.is_none_or(|allowed| allowed.contains(&name.as_str())) {
            fields.insert(name.clone(), schema.clone());
        }
    }
    let mut schema = json!({"type":"object","additionalProperties":false,"properties":fields});
    if update {
        schema["minProperties"] = json!(1);
    } else {
        schema["required"] = input["required"].clone();
    }
    schema
}

pub fn metadata_member_add(
    collection: crate::MetaCollection,
    site: Option<MetadataSite>,
) -> OperationContract {
    use crate::MetaCollection;
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
    OperationContract {
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
        format_validators: &[],
    }
}

pub fn metadata_member_set(
    collection: crate::MetaCollection,
    site: Option<MetadataSite>,
) -> OperationContract {
    OperationContract {
        schema: object(
            &[
                ("at", text(true)),
                ("values", metadata_element_schema(collection, true, site)),
            ],
            &["values"],
        ),
        target: if collection == crate::MetaCollection::Attributes {
            "The existing attribute: <source-set>:<Kind>.<Name>.Attribute.<A> or <source-set>:<Kind>.<Name>.TabularSection.<T>.Attribute.<A>."
        } else {
            "The existing member at its own address: <source-set>:<Kind>.<Name>.<Leaf>.<Member>."
        },
        effect: "Change the listed fields of the member; newName renames it. Unlisted fields stay.",
        notes: "The member is named by its address, not by values.name. Read the member first. Cross-field rules — fillValue must match the member type — are checked by the planner, not by this schema.",
        example_args: json!({"values":{"synonym":"Sample"}}),
        help: METADATA_HELP,
        format_validators: &[],
    }
}

pub fn metadata_member_remove() -> OperationContract {
    OperationContract {
        schema: object(&[("at", text(true))], &[]),
        target:
            "The existing member at its own address: <source-set>:<Kind>.<Name>.<Leaf>.<Member>.",
        effect: "Remove the member from its owner.",
        notes: "The member is named by its address; the operation takes no other arguments.",
        example_args: json!({}),
        help: METADATA_HELP,
        format_validators: &[],
    }
}

pub fn metadata_object_create() -> OperationContract {
    let mut schema = metadata_object_create_input_schema().clone();
    schema["properties"]["values"]["properties"]["name"]["pattern"] = json!(IDENTIFIER_PATTERN);
    OperationContract {
        schema,
        target: "The Configuration root: <source-set>:Configuration.",
        effect: "Create and register a new metadata object with its minimal supported files.",
        notes: "Choose an unused valid 1C identifier. The operation accepts kind and name; configure properties and members with subsequent operations in the same apply call.",
        example_args: json!({"values":{"kind":"Catalog","name":"Sample"}}),
        help: "skills/meta-add/SKILL.md",
        format_validators: &[],
    }
}

pub fn metadata_object_remove() -> OperationContract {
    OperationContract {
        schema: metadata_object_remove_input_schema().clone(),
        target: "The existing metadata object: <source-set>:<Kind>.<Name>.",
        effect: "Remove the object descriptor, its resource directory and its registration; update subsystem membership.",
        notes: "References from other objects are checked. force=true keeps those references and reports references_kept; it does not repair them. Under the deny support policy, a vendor object must be taken off support in a separate apply call first.",
        example_args: json!({}),
        help: METADATA_HELP,
        format_validators: &[],
    }
}

pub fn metadata_object_borrow() -> OperationContract {
    OperationContract {
        schema: metadata_borrow_input_schema().clone(),
        target: "The extension Configuration root: <extension-source-set>:Configuration.",
        effect: "Borrow or refresh the named parent object, preserving the extension's identity and local data.",
        notes: "exampleArgs is a template: replace parent and Sample with an existing object in a distinct admitted parent source set. from is a qualified address of a named top-level object or Enum.EnumValue; read that source first. overrides names supported transferred properties; an empty array is allowed for a top-level object but any overrides field is refused for EnumValue. Refresh preserves the established override set.",
        example_args: json!({"from":"parent:Catalog.Sample"}),
        help: "skills/cfe-borrow/SKILL.md",
        format_validators: &[],
    }
}

pub fn metadata_help_create() -> OperationContract {
    OperationContract {
        schema: metadata_help_create_input_schema().clone(),
        target: "The existing metadata object: <source-set>:<Kind>.<Name>.",
        effect: "Create Ext/Help.xml and Ext/Help/<lang>.html; add IncludeHelpInContents=false to owner forms that lack that property.",
        notes: "lang is required and contains only ASCII letters and digits, for example ru or en. Creation is refused if help already exists; existing help and forms are preserved. Plan this root-inspecting operation before other changes in the batch.",
        example_args: json!({"values":{"lang":"ru"}}),
        help: METADATA_HELP,
        format_validators: &[],
    }
}

pub fn metadata_relation(mode: RelationEditMode, owner: Option<MetadataKind>) -> OperationContract {
    use crate::{metadata_relation_specs, metadata_relation_values_schema, MetaRelation};
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
    OperationContract {
        schema,
        target: "The existing metadata object that owns this relation: <source-set>:<Kind>.<Name>.",
        effect: match mode {
            RelationEditMode::Add => {
                "Add the listed relation targets, preserving existing targets."
            }
            RelationEditMode::Remove => {
                "Remove the listed relation targets, preserving other targets."
            }
            RelationEditMode::Replace => {
                "Replace the relation's logical targets with the listed set."
            }
        },
        notes: if relation.is_none() {
            "This owner kind has no supported relations. Its relation field rejects every value; there is no executable argument example on this node."
        } else {
            "exampleArgs is a template: replace Sample targets with existing objects or fields allowed for this owner. The owner kind narrows the relation and target shape. targets is nonempty; mode is selected by the operation name. Read targets first. Field existence, same-owner paths, source-class compatibility and the final nonempty EventSubscription source are checked by the planner. Existing XML namespaces and unrelated data are preserved."
        },
        example_args,
        help: METADATA_HELP,
        format_validators: &[],
    }
}

pub fn metadata_predefined_add(owner: Option<MetadataKind>) -> OperationContract {
    OperationContract {
        schema: object(&[("at", text(true)), ("items", json!({"type":"array","minItems":1,"items":predefined_item_schema(false, owner)}))], &["items"]),
        target: "A metadata object with predefined items: <source-set>:<Kind>.<Name>. Read its PredefinedItem branch first; a new item needs a new UUID id and a new name.",
        effect: "Add predefined items to Ext/Predefined.xml of the owner; the file is created with the first item.",
        notes: "Predefined items are named by their address; there is no id or name selector argument. Item names are unique within the owner at every depth; an add or rename onto a used name is refused. The fields beyond code and description depend on the owner kind.",
        example_args: json!({"items":[{"id":"a1b2c3d4-1111-4111-8111-111111111111","name":"Sample","description":"Sample item"}]}),
        help: METADATA_HELP,
        format_validators: &[],
    }
}

pub fn metadata_predefined_set(owner: Option<MetadataKind>) -> OperationContract {
    OperationContract {
        schema: object(&[("at", text(true)), ("values", predefined_item_schema(true, owner))], &["values"]),
        target: "The existing item: <source-set>:<Kind>.<Name>.PredefinedItem.<ItemName>. The id is not an input.",
        effect: "Change the listed fields of the item; newName renames it. Unlisted fields stay.",
        notes: "Predefined items are named by their address; there is no id or name selector argument. Item names are unique within the owner at every depth; an add or rename onto a used name is refused. The fields beyond code and description depend on the owner kind.",
        example_args: json!({"values":{"description":"Sample item"}}),
        help: METADATA_HELP,
        format_validators: &[],
    }
}

pub fn metadata_predefined_remove() -> OperationContract {
    OperationContract {
        schema: object(&[("at", text(true))], &[]),
        target: "The existing item: <source-set>:<Kind>.<Name>.PredefinedItem.<ItemName>.",
        effect: "Remove the item with its nested items; the file is removed with the last item.",
        notes: "Predefined items are named by their address; there is no id or name selector argument. Item names are unique within the owner at every depth; an add or rename onto a used name is refused. The fields beyond code and description depend on the owner kind.",
        example_args: json!({}),
        help: METADATA_HELP,
        format_validators: &[],
    }
}

pub fn metadata_template_add() -> OperationContract {
    let kinds: Vec<_> = crate::MetaTemplateKind::ALL
        .iter()
        .map(|kind| kind.as_str())
        .collect();
    let kind = json!({"default":"SpreadsheetDocument","anyOf":[{"type":"string","enum":kinds},{"not":{"type":"string"}}]});
    OperationContract {
        schema: object(&[("at",text(true)),("items",json!({"type":"array","minItems":1,"items":object(&[
            ("name",text(true)),("templateType",kind),("synonym",json!({}))], &["name"])}))], &["items"]),
        target: "An existing metadata owner that exposes template.add in can. Read its Template collection before choosing a new valid 1C identifier.",
        effect: "Register a new template and its initial content. The new address exists only after execution of the saved plan; preview does not create it.",
        notes: "The kind vocabulary comes from MetaTemplateKind. Current initial-content synthesis supports SpreadsheetDocument and DataCompositionSchema; HTMLDocument, TextDocument and BinaryData are typed refusals (tracked gap #992). Missing or non-string templateType uses SpreadsheetDocument for compatibility. synonym is accepted but currently ignored. A new DCS has no dataset. For MXL, add areas with area.add on the template, then change their cells with mxl.set at Template.<name>.Area.<area>; view returns a node projection, not a round-trip JSON DSL.",
        example_args: json!({"items":[{"name":"PrintLayout","templateType":"SpreadsheetDocument"}]}),
        help: REPORTS_HELP,
        format_validators: &[],
    }
}

pub fn metadata_template_set() -> OperationContract {
    OperationContract {
        schema: object(
            &[
                ("at", text(true)),
                (
                    "values",
                    json!({"type":"object","additionalProperties":false,"minProperties":1,
            "properties":{"synonym":{"type":"string"},"comment":{"type":"string"},
                "templateType":{"type":"string","enum":crate::MetaTemplateKind::ALL.iter().map(|kind| kind.as_str()).collect::<Vec<_>>()}}}),
                ),
            ],
            &["values"],
        ),
        target: "The existing template: <source-set>:<Kind>.<Name>.Template.<T>.",
        effect:
            "Change the listed properties of the template descriptor. Unlisted properties stay.",
        notes: "The template is named by its address; there is no name argument.",
        example_args: json!({"values":{"synonym":"Sample"}}),
        help: METADATA_HELP,
        format_validators: &[],
    }
}

pub fn metadata_template_remove() -> OperationContract {
    OperationContract {
        schema: object(&[("at", text(true))], &[]),
        target: "The existing template: <source-set>:<Kind>.<Name>.Template.<T>.",
        effect: "Remove the template descriptor and its content, and clear owner properties that pointed at it.",
        notes: "The template is named by its address; there is no name argument.",
        example_args: json!({}),
        help: METADATA_HELP,
        format_validators: &[],
    }
}

const REPORTS_HELP: &str = "references/use-cases/reports-printing.md";
