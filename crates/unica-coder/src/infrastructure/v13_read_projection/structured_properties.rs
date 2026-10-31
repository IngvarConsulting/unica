//! Closed public read profiles. Observation does not grant mutation rights.
//! WriteName binds a name to one operand of one actual Can contract; it does
//! not promise that the observed value is a writable operand (QName/bounds).
use super::{bounded_prop, safe_prop, LogicalReader, Map, NodeKind, Value};

#[derive(Clone, Copy, Debug)]
pub(crate) enum WriteTarget {
    SelfNode,
    Parent,
    Package,
}

#[derive(Clone, Copy, Debug)]
#[allow(
    dead_code,
    reason = "read-profile role metadata is consumed by the Can parity guard"
)]
pub(crate) enum PropertyRole {
    WriteName {
        operation: &'static str,
        target: WriteTarget,
        payload: &'static str,
    },
    Observed(&'static str),
    Derived(&'static str),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PropertySpec {
    pub name: &'static str,
    // None: an existing specialized builder supplies this value.
    input: Option<&'static str>,
    #[allow(
        dead_code,
        reason = "read-profile role metadata is consumed by the Can parity guard"
    )]
    pub role: PropertyRole,
}

const fn write(name: &'static str, operation: &'static str) -> PropertySpec {
    PropertySpec {
        name,
        input: Some(name),
        role: PropertyRole::WriteName {
            operation,
            target: WriteTarget::SelfNode,
            payload: "/properties/values/properties",
        },
    }
}
const fn observed(name: &'static str, reason: &'static str) -> PropertySpec {
    PropertySpec {
        name,
        input: Some(name),
        role: PropertyRole::Observed(reason),
    }
}
const fn derived(name: &'static str, reason: &'static str) -> PropertySpec {
    PropertySpec {
        name,
        input: Some(name),
        role: PropertyRole::Derived(reason),
    }
}
const fn special(mut spec: PropertySpec) -> PropertySpec {
    spec.input = None;
    spec
}
const fn input(mut spec: PropertySpec, name: &'static str) -> PropertySpec {
    spec.input = Some(name);
    spec
}
const fn creation(
    name: &'static str,
    operation: &'static str,
    target: WriteTarget,
    payload: &'static str,
) -> PropertySpec {
    PropertySpec {
        name,
        input: None,
        role: PropertyRole::WriteName {
            operation,
            target,
            payload,
        },
    }
}

pub(crate) fn properties(reader: LogicalReader, kind: NodeKind) -> &'static [PropertySpec] {
    match (reader, kind) {
        (LogicalReader::Dcs, NodeKind::Template) => {
            const {
                &[special(derived(
                    "format",
                    "The reader identifies the schema format",
                ))]
            }
        }
        (LogicalReader::Dcs, NodeKind::DataSource) => const { &[write("kind", "dataSource.set")] },
        (LogicalReader::Dcs, NodeKind::DataSet) => {
            const {
                &[
                    derived(
                        "kind",
                        "Discriminator of the observed dataset, immutable in dataSet.set",
                    ),
                    write("objectName", "dataSet.set"),
                    write("dataSource", "dataSet.set"),
                ]
            }
        }
        (LogicalReader::Dcs, NodeKind::Field) => {
            const { &[write("field", "field.set"), write("title", "field.set")] }
        }
        (LogicalReader::Dcs, NodeKind::Parameter) => {
            const {
                &[
                    write("type", "parameter.set"),
                    write("value", "parameter.set"),
                    write("expression", "parameter.set"),
                    write("availableAsField", "parameter.set"),
                    input(write("useRestriction", "parameter.set"), "restricted"),
                ]
            }
        }
        (LogicalReader::Dcs, NodeKind::Setting) => {
            const {
                &[
                    write("presentation", "variant.set"),
                    derived(
                        "filters",
                        "Number of filter elements, not a filter definition",
                    ),
                ]
            }
        }
        (LogicalReader::Dcs, NodeKind::CalculatedField) => {
            const {
                &[
                    write("expression", "calculatedField.set"),
                    write("title", "calculatedField.set"),
                ]
            }
        }
        (LogicalReader::Dcs, NodeKind::Total) => {
            const {
                &[
                    write("expression", "total.set"),
                    write("group", "total.set"),
                    special(observed(
                        "dataPath",
                        "Identity of an unaddressable total row",
                    )),
                ]
            }
        }
        (LogicalReader::Dcs, NodeKind::Item) => {
            const {
                &[
                    special(derived(
                        "kind",
                        "Discriminator of the observed structure item",
                    )),
                    special(observed(
                        "axis",
                        "Position in the parent table/chart, not a patchable property",
                    )),
                    special(write("groupBy", "structure.patch")),
                    special(write("use", "structure.patch")),
                    special(write("viewMode", "structure.patch")),
                    special(write("columnsViewMode", "structure.patch")),
                    special(write("rowsViewMode", "structure.patch")),
                    special(write("pointsViewMode", "structure.patch")),
                    special(write("seriesViewMode", "structure.patch")),
                    special(write("itemsViewMode", "structure.patch")),
                    special(write("userSettingID", "structure.patch")),
                    special(write("userSettingPresentation", "structure.patch")),
                ]
            }
        }
        (LogicalReader::Xdto, NodeKind::XdtoPackage) => {
            const {
                &[
                    observed(
                        "elementFormQualified",
                        "Observed package element qualification; no write operand",
                    ),
                    observed(
                        "attributeFormQualified",
                        "Observed package attribute qualification; no write operand",
                    ),
                    observed(
                        "targetNamespace",
                        "Namespace identity of the package; no namespace writer",
                    ),
                    special(derived("total", "Count of package declarations")),
                    special(derived("valueTypes", "Count of package declarations")),
                    special(derived("objectTypes", "Count of package declarations")),
                    special(derived("globalProperties", "Count of package declarations")),
                ]
            }
        }
        (LogicalReader::Xdto, NodeKind::Namespace) => {
            const {
                &[special(observed(
                    "namespace",
                    "Namespace identity of the package; no namespace writer",
                ))]
            }
        }
        (LogicalReader::Xdto, NodeKind::Type) => {
            const {
                &[
                    derived("kind", "Named valueType/objectType discriminator"),
                    observed("abstract", "Observed type attribute preserved by type.set"),
                    observed("mixed", "Observed type attribute preserved by type.set"),
                    observed("open", "Observed named objectType flag; no write operand"),
                    special(write("base", "type.set")),
                    special(derived("baseAt", "Resolved address of one local base type")),
                    creation(
                        "facets",
                        "valueType.add",
                        WriteTarget::Package,
                        "/properties/values/properties",
                    ),
                ]
            }
        }
        (LogicalReader::Xdto, NodeKind::Property) => {
            const {
                &[
                    observed("form", "Observed XML Element/Attribute/Text form; no write operand"),
                    observed("nillable", "Observed nil permission; no write operand"),
                    observed("fixed", "Boolean flag fixing the default literal; no write operand"),
                    observed("default", "Observed default literal, including an explicit empty string; no write operand"),
                    input(
                        creation(
                            "type",
                            "property.add",
                            WriteTarget::Parent,
                            "/properties/values/properties/property/properties",
                        ),
                        "type",
                    ),
                    input(
                        observed(
                            "ref",
                            "QName of a global-property reference, outside the own-property writer",
                        ),
                        "reference",
                    ),
                    derived(
                        "bounds",
                        "Effective lexical bounds with explicit/default flags and unbounded status",
                    ),
                    creation(
                        "lowerBound",
                        "property.add",
                        WriteTarget::Parent,
                        "/properties/values/properties/property/properties",
                    ),
                    creation(
                        "upperBound",
                        "property.add",
                        WriteTarget::Parent,
                        "/properties/values/properties/property/properties",
                    ),
                    special(derived(
                        "anonymousType",
                        "Discriminator of the nested anonymous definition",
                    )),
                    creation(
                        "base",
                        "property.add",
                        WriteTarget::Parent,
                        "/properties/values/properties/property/properties/valueType/properties",
                    ),
                    creation(
                        "facets",
                        "property.add",
                        WriteTarget::Parent,
                        "/properties/values/properties/property/properties/valueType/properties",
                    ),
                ]
            }
        }
        _ => &[],
    }
}

pub(super) fn direct_props(
    reader: LogicalReader,
    kind: NodeKind,
    value: &Value,
) -> Map<String, Value> {
    properties(reader, kind)
        .iter()
        .filter_map(|spec| {
            if reader == LogicalReader::Xdto
                && kind == NodeKind::Type
                && matches!(spec.name, "open" | "mixed")
                && value["kind"] != "objectType"
            {
                return None;
            }
            value
                .get(spec.input?)
                .and_then(|value| {
                    if reader == LogicalReader::Xdto && kind == NodeKind::Property {
                        bounded_prop(spec.name, value)
                    } else {
                        safe_prop(spec.name, value).then(|| value.clone())
                    }
                })
                .map(|value| (spec.name.to_string(), value))
        })
        .collect()
}

/// Flattened anonymous facts have a different writer operand from named types.
/// The discriminator is part of the read context, never a union of schemas.
#[allow(
    dead_code,
    reason = "contextual read-profile metadata is verified by the Can parity guard"
)]
pub(crate) fn property(
    reader: LogicalReader,
    kind: NodeKind,
    address: &unica_address::address::QualifiedAddress,
    props: &Value,
    name: &str,
) -> Option<PropertySpec> {
    if matches!(reader, LogicalReader::Dcs | LogicalReader::Xdto)
        && matches!(kind, NodeKind::Template | NodeKind::XdtoPackage)
    {
        let inherited = [
            derived("belonging", "Own/borrowed identity of the descriptor"),
            derived("parentId", "Identifier of the extended object"),
            derived("parentStatus", "Availability of the extended object"),
            derived("overridesCount", "Number of locally overridden properties"),
            derived(
                "overridesComplete",
                "Completeness of the compact override list",
            ),
            derived("overrides", "Names of locally overridden properties"),
            derived("content", "Local/inherited companion content status"),
            derived(
                "parentReason",
                "How to locate a borrowed child in the parent source set",
            ),
            derived("extends", "Resolved logical address of the extended object"),
        ];
        if let Some(spec) = inherited.iter().find(|spec| spec.name == name) {
            return Some(*spec);
        }
    }
    if reader == LogicalReader::Xdto {
        if kind == NodeKind::Type
            && matches!(name, "open" | "mixed")
            && props["kind"] != "objectType"
        {
            return None;
        }
        if kind == NodeKind::Type && name == "facets" && props["kind"] != "valueType" {
            return None;
        }
        if kind == NodeKind::Property
            && matches!(name, "base" | "facets")
            && props["anonymousType"] != "ValueType"
        {
            return None;
        }
    }
    let mut spec = *properties(reader, kind)
        .iter()
        .find(|spec| spec.name == name)?;
    // Package-level global declarations are observable; property.add creates
    // own properties of object types or anonymous object containers only.
    if reader == LogicalReader::Xdto
        && kind == NodeKind::Property
        && !address
            .segments()
            .iter()
            .any(|part| part.kind() == NodeKind::Type)
        && matches!(spec.role, PropertyRole::WriteName { .. })
    {
        spec.role =
            PropertyRole::Observed("Global property declaration; no global-property writer");
    }
    Some(spec)
}
