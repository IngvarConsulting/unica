//! Argument contracts shared by the operation hint and the apply boundary.
//! The closed operation registry owns names and applicability; this module
//! describes and normalizes the arguments consumed by supported planners.

use serde_json::{json, Map, Value};

pub(crate) const MXL_MAX_ROW: i64 = 10_000;
pub(crate) const MXL_MAX_COLUMN: i64 = 1_000;

/// Preserve the existing integer spelling accepted at the apply boundary,
/// including leading zeros and a leading plus; hints and planner share it.
pub(crate) fn parse_mxl_cell_address(key: &str) -> Option<(i64, i64)> {
    let rest = key.strip_prefix('R')?;
    let (row, column) = rest.split_once('C')?;
    let row = row.parse::<i64>().ok()?;
    let column = column.parse::<i64>().ok()?;
    ((1..=MXL_MAX_ROW).contains(&row) && (1..=MXL_MAX_COLUMN).contains(&column))
        .then_some((row, column))
}

pub(crate) const DCS_ROLE_FLAGS: &[&str] = &[
    "period",
    "dimension",
    "account",
    "balance",
    "ignoreNullValues",
    "required",
    "dimensionAttribute",
];
const DCS_ROLE_KEYS: &[&str] = &[
    "parentDimension",
    "accountTypeExpression",
    "balanceGroupName",
    "accountField",
    "periodNumber",
    "periodType",
    "dimension",
    "account",
    "balance",
    "ignoreNullValues",
    "required",
    "dimensionAttribute",
    "balanceType",
    "accountingBalanceType",
];

pub(crate) struct OperationContract {
    pub(crate) schema: Value,
    pub(crate) target: &'static str,
    pub(crate) effect: &'static str,
    pub(crate) notes: &'static str,
    pub(crate) example_args: Value,
}

#[derive(Debug, PartialEq)]
pub(crate) struct ContractError {
    pub(crate) path: String,
    pub(crate) message: String,
}

fn text(nonempty: bool) -> Value {
    if nonempty {
        json!({"type":"string", "pattern":r"\S"})
    } else {
        json!({"type":"string"})
    }
}

fn object(fields: &[(&str, Value)], required: &[&str]) -> Value {
    json!({"type":"object", "additionalProperties":false,
        "properties":fields.iter().map(|(key,value)| (key.to_string(),value.clone())).collect::<Map<_,_>>(),
        "required":required})
}

fn aliases(mut schema: Value, names: &[&str]) -> Value {
    schema["x-unica-aliases"] = json!(names);
    for name in names {
        schema["properties"][*name] = text(false);
    }
    schema["anyOf"] = Value::Array(
        names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let mut choice = json!({"required":[name],"properties":{*name:text(true)}});
                if index > 0 {
                    choice["not"] = json!({"anyOf":names[..index].iter().map(|previous| json!({"required":[previous]})).collect::<Vec<_>>()});
                }
                choice
            })
            .collect(),
    );
    schema
}

fn optional_aliases(mut schema: Value, names: &[&str]) -> Value {
    schema["x-unica-aliases"] = json!(names);
    schema
}

fn items(item: Value) -> Value {
    json!({"type":"array", "items":item})
}

fn field_head(names: &[&str], required: &[&str]) -> Value {
    let mut fields: Vec<_> = names.iter().map(|name| (*name, text(true))).collect();
    fields.extend([("type", text(false)), ("title", text(false))]);
    aliases(object(&fields, required), names)
}

impl OperationContract {
    pub(crate) fn dcs(op: &str) -> Option<Self> {
        let schema_target = "The DCS Template itself. Read the template props and its DataSet/Setting branches before choosing selectors.";
        let data_target = "Existing DCS Template.DataSet.<name>; repeat DataSet.<name> for a nested Union member. Query.<name> is a dataset alias.";
        let setting_target = "Existing DCS Template.Setting.<name>. Optional group selects a unique named structure item; read Setting.Item first.";
        let title = json!({"anyOf":[{"type":"string"},{"type":"object","additionalProperties":{"type":"string"}}]});
        let scalar = json!({"anyOf":[{"type":"string"},{"type":"number"},{"type":"boolean"},{"type":"null"}]});
        let localized_value =
            json!({"type":"object","minProperties":1,"additionalProperties":{"type":"string"}});
        let typed_value = json!({"anyOf":[scalar.clone(),localized_value.clone(),object(&[("variant",text(true)),("startDate",text(true)),("endDate",text(true))],&["variant"])]});
        let appearance_value = json!({"anyOf":[typed_value.clone(),object(&[("valueType",text(true)),("value",typed_value.clone())],&["valueType","value"])]});
        let strings = items(text(true));
        let comparison = json!({"type":"string","default":"Equal","enum":["Equal","NotEqual","Greater","GreaterOrEqual","Less","LessOrEqual","Contains","NotContains","BeginsWith","NotBeginsWith","InList","NotInList","InHierarchy","InListByHierarchy","Filled","NotFilled"]});
        let filter = || {
            aliases(
                object(
                    &[
                        ("field", text(true)),
                        ("dataPath", text(true)),
                        (
                            "comparison",
                            if op == "filter.add" {
                                comparison.clone()
                            } else {
                                let mut value = comparison.clone();
                                value.as_object_mut().unwrap().remove("default");
                                value
                            },
                        ),
                        ("value", scalar.clone()),
                        ("valueType", text(true)),
                        ("use", json!({"type":"boolean"})),
                        (
                            "viewMode",
                            json!({"type":"string","enum":["Normal","Inaccessible","QuickAccess"]}),
                        ),
                        ("userSettingID", text(true)),
                        ("userSettingPresentation", text(false)),
                        ("group", text(true)),
                    ],
                    &[],
                ),
                &["field", "dataPath"],
            )
        };
        let structure = || {
            object(
                &[
                    ("name", text(true)),
                    (
                        "kind",
                        json!({"type":"string","enum":["group","table","chart"],"default":"group"}),
                    ),
                    ("groupBy", strings.clone()),
                    ("parent", text(true)),
                    (
                        "axis",
                        json!({"type":"string","enum":["row","column","point","series"]}),
                    ),
                    (
                        "viewMode",
                        json!({"type":"string","enum":["Normal","Inaccessible","QuickAccess"]}),
                    ),
                ],
                &["name"],
            )
        };
        let restriction = object(
            &["field", "condition", "group", "order"]
                .iter()
                .map(|key| (*key, json!({"type":"boolean"})))
                .collect::<Vec<_>>(),
            &[],
        );
        let parameter = || {
            object(
                &[
                    ("name", text(true)),
                    ("type", text(false)),
                    ("title", title.clone()),
                    ("value", typed_value.clone()),
                    (
                        "values",
                        json!({"type":"array","items":typed_value.clone()}),
                    ),
                    (
                        "availableValues",
                        json!({"type":"array","items":object(&[("value",typed_value.clone()),("presentation",title.clone())],&["value"])}),
                    ),
                    ("valueType", text(true)),
                    ("expression", text(false)),
                    ("useRestriction", json!({"type":"boolean"})),
                    ("valueListAllowed", json!({"type":"boolean"})),
                    ("availableAsField", json!({"type":"boolean"})),
                    ("denyIncompleteValues", json!({"type":"boolean"})),
                    (
                        "use",
                        json!({"type":"string","enum":["Always","Auto","Never"]}),
                    ),
                ],
                &["name"],
            )
        };
        let (key, inner, target, effect, example_args) = match op {
            "dataSource.add" | "dataSource.set" => {
                let mut kind = json!({"type":"string","enum":["Local","External"]});
                if op.ends_with("add") {kind["default"]=json!("Local");}
                (if op.ends_with("add"){"items"}else{"values"},object(&[("name",text(true)),("kind",kind)],if op.ends_with("add"){&["name"][..]}else{&["name","kind"][..]}),schema_target,if op.ends_with("add"){"Add a new named source; an existing name is refused."}else{"Change only the kind of an existing named source. A missing name is refused; create it with dataSource.add."},if op.ends_with("add"){json!({"items":[{"name":"Database"}]})}else{json!({"values":{"name":"Database","kind":"Local"}})})
            }
            "dataSource.remove" => ("values",object(&[("name",text(true))],&["name"]),schema_target,"Remove an unused named source; a referenced source is refused.",json!({"values":{"name":"Unused"}})),
            "dataSet.add" => {
                let inner=object(&[("name",text(true)),("kind",json!({"type":"string","enum":["Query","Object","Union"],"default":"Query"})),("dataSource",text(true)),("query",text(true)),("objectName",text(true))],&["name"]);
                ("items",inner,"DCS Template adds a top-level dataset; an existing Union DataSet adds one member.","Create one Query/Object/Union dataset. Query requires dataSource and query; Object requires dataSource and objectName; Union members are separate dataSet.add operations. Add fields separately.",json!({"items":[{"name":"Data","kind":"Query","dataSource":"ИсточникДанных1","query":"ВЫБРАТЬ 1 КАК Amount"}]}))
            }
            "dataSet.set" => ("values",object(&[("dataSource",text(true)),("objectName",text(true)),("autoFillFields",json!({"type":"boolean"}))],&[]),data_target,"Update only supplied dataset properties; changing its kind is refused.",json!({"values":{"autoFillFields":false}})),
            "dataSet.remove" => ("",json!({}),data_target,"Remove the addressed dataset. No dependent fields, links or variant settings are implicitly rewritten.",json!({})),
            "field.add" => {
                let mut inner=field_head(&["dataPath","name"],&[]);inner["properties"]["title"]=title.clone();inner["properties"]["field"]=text(true);
                for key in ["useRestriction","attributeUseRestriction"]{inner["properties"][key]=restriction.clone();}
                inner["properties"]["presentationExpression"]=text(false);
                ("items",inner,data_target,"Add dataset fields. Existing dataPath is unchanged. Selection is a separate operation.",json!({"items":[{"dataPath":"Added","title":"Added [шт]"}]}))
            }
            "field.set" => {
                let mut inner=field_head(&["field","dataPath","name"],&[]);inner["properties"]["title"]=title.clone();
                for key in ["useRestriction","attributeUseRestriction"]{inner["properties"][key]=restriction.clone();}
                inner["properties"]["presentationExpression"]=text(false);inner["properties"]["sourceField"]=text(true);
                ("values",inner,data_target,"Update only supplied field mapping/title/type/restrictions/presentation expression. Query fields derive their type from query; field.set removes stale valueType on the selected Query field. Other field data remains.",json!({"values":{"field":"Amount","title":{"ru":"Сумма","en":"Amount"}}}))
            }
            "field.remove" | "parameter.remove" => ("",json!({}),if op=="field.remove"{data_target}else{schema_target},"Remove only the named terminal Field or Parameter. References and selections are separate operations.",json!({})),
            "fieldRole.set" => {
                let flags=object(&DCS_ROLE_KEYS.iter().map(|key|(*key,if DCS_ROLE_FLAGS.contains(key){json!({"type":"boolean"})}else{text(true)})).collect::<Vec<_>>(),&[]);
                ("values",aliases(object(&[("field",text(true)),("dataPath",text(true)),("name",text(true)),("role",flags)],&["role"]),&["field","dataPath","name"]),data_target,"Replace field role with the supplied object. Values are validated against platform types; omitted role entries are removed.",json!({"values":{"field":"Amount","role":{"dimension":true}}}))
            }
            "parameter.add" | "parameter.set" => (if op.ends_with("add"){"items"}else{"values"},parameter(),schema_target,if op.ends_with("add"){"Add a new schema parameter. Parameters for dates and their expressions are explicit separate operations."}else{"Patch only supplied properties of an existing schema parameter. A missing parameter is refused; create it with parameter.add."},if op.ends_with("add"){json!({"items":[{"name":"Period","type":"date"}]})}else{json!({"values":{"name":"Period","title":"Period"}})}),
            "parameter.rename" => ("values",object(&[("name",text(true)),("newName",text(true))],&["name","newName"]),schema_target,"Rename the parameter declaration only. References are explicit query/expression operations in the same batch.",json!({"values":{"name":"Period","newName":"ReportPeriod"}})),
            "parameter.reorder" => ("items",object(&[("name",text(true))],&["name"]),schema_target,"Reorder all schema parameter elements by the supplied complete unique list. Comments and all other nodes stay in their original slots.",json!({"items":[{"name":"Period"}]})),
            "calculatedField.add" => {
                let mut inner=aliases(object(&[("name",text(true)),("dataPath",text(true)),("expression",text(true)),("title",title.clone()),("type",text(false))],&["expression"]),&["name","dataPath"]);
                inner["properties"]["useRestriction"]=restriction.clone();
                ("items",inner,schema_target,"Add a calculated field; selection is independent.",json!({"items":[{"name":"DoubleAmount","expression":"Amount * 2"}]}))
            }
            "total.add" => ("items",aliases(object(&[("field",text(true)),("dataPath",text(true)),("expression",text(false)),("group",json!({"anyOf":[text(true),strings.clone()]}))],&[]),&["field","dataPath"]),schema_target,"Add a total with explicit expression/group. Omitted expression is Сумма(field).",json!({"items":[{"field":"Amount","expression":"Сумма(Amount)"}]})),
            "total.remove" | "calculatedField.remove" => ("values",object(&[("name",text(true))],&["name"]),schema_target,"Remove the named schema total/calculation only.",json!({"values":{"name":"Amount"}})),
            "variant.add" | "variant.set" => (if op.ends_with("add"){"items"}else{"values"},optional_aliases(object(&[("name",text(true)),("title",title.clone()),("presentation",title.clone())],&["name"]),&["title","presentation"]),schema_target,if op.ends_with("add"){"Create an empty settings variant. Configure selection and structure separately."}else{"Change the presentation of an existing settings variant. A missing variant is refused; create it with variant.add."},if op.ends_with("add"){json!({"items":[{"name":"Additional","title":"Additional"}]})}else{json!({"values":{"name":"Additional","title":"Report"}})}),
            "variant.remove" => ("values",object(&[("name",text(true))],&["name"]),schema_target,"Remove the selected variant.",json!({"values":{"name":"Additional"}})),
            "query.set" => ("values",aliases(object(&[("dataSet",text(false)),("query",text(true)),("text",text(true))],&[]),&["query","text"]),data_target,"Replace query text. Supply literal text, not @file.",json!({"values":{"query":"ВЫБРАТЬ 1 КАК Amount"}})),
            "query.patch" => ("values",object(&[("dataSet",text(false)),("find",text(true)),("replace",json!({"type":"string","default":""})),("once",json!({"type":"boolean","default":false}))],&["find"]),data_target,"Patch decoded query text. No matches fail; once=true requires exactly one. Comments/PI are preserved; a marker within replaced text anchors to replacement start.",json!({"values":{"find":"1 КАК Amount","replace":"2 КАК Amount","once":true}})),
            "filter.add" | "filter.set" => (if op.ends_with("add"){"items"}else{"values"},filter(),setting_target,"Add a filter or patch one uniquely selected by field. Literal strings retain @, brackets and comparison words. Set requires a unique match.",if op.ends_with("add"){json!({"items":[{"field":"Amount","comparison":"Greater","value":0}]})}else{json!({"values":{"field":"Amount","comparison":"Greater","value":1}})}),
            "filter.remove" => ("values",object(&[("field",text(true)),("group",text(true))],&["field"]),setting_target,"Remove one uniquely selected filter; multiple matches fail.",json!({"values":{"field":"Amount"}})),
            "selection.add" | "order.add" => {
                let mut inner=optional_aliases(object(&[("field",text(true)),("dataPath",text(true)),("kind",json!({"type":"string","enum":["Field","Auto"],"default":"Field"})),("group",text(true))],&[]),&["field","dataPath"]);
                if op=="order.add" {inner["properties"]["direction"]=json!({"type":"string","enum":["Asc","Desc"],"default":"Asc"});}
                ("items",inner,setting_target,"Add one explicit Field or Auto selection/order item. Field requires field; Auto has no field. Use index selectors for anonymous Auto items.",json!({"items":[{"field":"Amount"}]}))
            }
            "filter.clear" | "selection.clear" | "order.clear" | "conditionalAppearance.clear" => ("values",object(&[("group",text(true))],&[]),setting_target,"Clear the selected collection's elements. The collection's attributes/comments stay; an absent collection is unchanged.",json!({"values":{}})),
            "dataParameter.add" | "dataParameter.set" | "outputParameter.set" => (if op=="dataParameter.add"{"items"}else{"values"},object(&[("name",text(true)),("value",typed_value.clone()),("valueType",text(true)),("use",json!({"type":"boolean"})),("viewMode",json!({"type":"string","enum":["Normal","Inaccessible","QuickAccess"]})),("userSettingID",text(true)),("userSettingPresentation",title.clone()),("group",text(true))],if op=="dataParameter.add"{&["name","value"][..]}else{&["name"][..]}),setting_target,"Set only supplied variant data/output parameter properties. Typed defaults, NULL, periods, references and localized values preserve their platform type; schema parameter declarations remain separate.",if op=="dataParameter.add"{json!({"items":[{"name":"Period","value":"2026-01-01T00:00:00","valueType":"xs:dateTime"}]})}else{json!({"values":{"name":"Period","value":"2026-01-01T00:00:00","valueType":"xs:dateTime"}})}),
            "conditionalAppearance.add" => {
                let mut condition = filter();
                condition["properties"].as_object_mut().unwrap().remove("group");
                condition["properties"]["comparison"]["default"] = json!("Equal");
                ("items",object(&[("fields",strings.clone()),("filter",json!({"type":"array","items":condition})),("use",json!({"type":"boolean"})),("appearance",json!({"type":"object","minProperties":1,"additionalProperties":appearance_value.clone()})),("group",text(true))],&["fields","appearance"]),setting_target,"Add conditional appearance with explicit use, comparisons and typed value atoms. Formatting does not change field values.",json!({"items":[{"fields":["Amount"],"use":false,"appearance":{"Формат":{"valueType":"v8:LocalStringType","value":{"ru":"ЧДЦ=2"}}}}]}))
            }
            "structure.add" => ("items",structure(),setting_target,"Add one named group/table/chart; parent selects an existing unique item, axis selects its row/column/point/series. Add each child with a separate operation.",json!({"items":[{"name":"ByAmount","kind":"group","groupBy":["Amount"]}]})),
            "structure.set" => ("values",object(&[("variant",text(false)),("name",text(true)),("groupBy",json!({"type":"array","items":text(true),"default":[]})),("details",json!({"type":"boolean","default":true}))],&[]),setting_target,"Replace the variant structure with one group and optional nested details. groupBy=[] is a details group; use structure.add for tables/charts and nested composition.",json!({"values":{"name":"ByAmount","groupBy":["Amount"],"details":true}})),
            "structure.patch" => ("values",object(&[("variant",text(false)),("name",text(true)),("groupBy",strings.clone())],&["name","groupBy"]),setting_target,"Patch groupBy of one uniquely named group. Other structure and settings are preserved. Names and fields are separate operands.",json!({"values":{"name":"ExistingGroup","groupBy":["Amount"]}})),
            "structure.remove" => ("values",object(&[("name",text(true))],&["name"]),setting_target,"Remove a unique named structure item and its children.",json!({"values":{"name":"ExistingGroup"}})),
            "calculatedField.set" => ("values",object(&[("name",text(true)),("expression",text(true)),("title",title.clone()),("type",text(true)),("useRestriction",restriction.clone())],&["name"]),schema_target,"Patch only supplied calculation properties.",json!({"values":{"name":"DoubleAmount","expression":"Amount * 3"}})),
            "total.set" => ("values",object(&[("field",text(true)),("expression",text(true)),("group",json!({"anyOf":[text(true),strings.clone()]}))],&["field"]),schema_target,"Patch supplied total expression/group.",json!({"values":{"field":"Amount","expression":"Максимум(Amount)"}})),
            "selection.set" | "order.set" => {
                let mut inner=object(&[("field",text(true)),("index",json!({"type":"integer","minimum":0})),("group",text(true)),("use",json!({"type":"boolean"})),("viewMode",json!({"type":"string","enum":["Normal","Inaccessible","QuickAccess"]})),("userSettingID",text(true)),("userSettingPresentation",text(false))],&[]);
                if op=="order.set"{inner["properties"]["direction"]=json!({"type":"string","enum":["Asc","Desc"]});}
                ("values",inner,setting_target,"Patch one uniquely selected field item, preserving unspecified settings.",json!({"values":{"field":"Amount","use":false}}))
            }
            "selection.remove" | "order.remove" => ("values",object(&[("field",text(true)),("index",json!({"type":"integer","minimum":0})),("group",text(true))],&[]),setting_target,"Remove one uniquely selected field item.",json!({"values":{"field":"Amount"}})),
            "dataParameter.remove" | "outputParameter.remove" => ("values",object(&[("name",text(true)),("group",text(true))],&["name"]),setting_target,"Remove the uniquely named settings parameter.",json!({"values":{"name":"Period"}})),
            "conditionalAppearance.remove" => ("values",object(&[("index",json!({"type":"integer","minimum":0})),("group",text(true))],&["index"]),setting_target,"Remove the zero-based appearance item from the captured settings image; read props before selecting.",json!({"values":{"index":0}})),
            "dataSetLink.set" | "dataSetLink.remove" => ("values",object(&[("selector",object(&[("source",text(true)),("destination",text(true)),("sourceExpression",text(true)),("destinationExpression",text(true))],&["source","destination","sourceExpression","destinationExpression"])),("parameter",text(true)),("parameterListAllowed",json!({"type":"boolean"})),("required",json!({"type":"boolean"})),("condition",text(false)),("startExpression",text(false))],&["selector"]),schema_target,"Select one unique dataset link by its endpoints and expressions. Patch only supplied properties or remove the selected link.",json!({"values":{"selector":{"source":"Left","destination":"Right","sourceExpression":"Amount","destinationExpression":"Amount"},"condition":"Amount > 0"}})),
            "dataSetLink.add" => ("items",object(&[("source",text(true)),("destination",text(true)),("sourceExpression",text(true)),("destinationExpression",text(true)),("parameter",text(true)),("parameterListAllowed",json!({"type":"boolean"})),("required",json!({"type":"boolean"})),("condition",text(false)),("startExpression",text(false))],&["source","destination","sourceExpression","destinationExpression"]),schema_target,"Add one dataset link between existing top-level datasets.",json!({"items":[{"source":"Left","destination":"Right","sourceExpression":"Amount","destinationExpression":"Amount"}]})),
            _ => return None,
        };
        let mut inner = inner;
        if op.starts_with("selection.")
            && matches!(op, "selection.add" | "selection.set" | "selection.remove")
        {
            inner["properties"]["parentIndexes"] =
                json!({"type":"array","items":{"type":"integer","minimum":0}});
            if op == "selection.add" {
                inner["properties"]["kind"]["enum"] = json!(["Field", "Auto", "Folder"]);
                inner["properties"]["title"] = title.clone();
            }
        }
        if op == "variant.set" {
            inner["anyOf"] = json!([{"required":["title"]},{"required":["presentation"]}]);
        }
        if matches!(op, "structure.add" | "structure.patch" | "structure.set") {
            let props = inner["properties"]
                .as_object_mut()
                .expect("structure object");
            props.insert("use".into(), json!({"type":"boolean"}));
            for key in [
                "columnsViewMode",
                "rowsViewMode",
                "pointsViewMode",
                "seriesViewMode",
                "itemsViewMode",
                "viewMode",
            ] {
                props.insert(
                    key.into(),
                    json!({"type":"string","enum":["Normal","Inaccessible","QuickAccess"]}),
                );
            }
            props.insert("userSettingID".into(), text(true));
            props.insert("userSettingPresentation".into(), title.clone());
            if op == "structure.patch" {
                inner["required"] = json!(["name"]);
            }
        }
        let inner = if key == "items" { items(inner) } else { inner };
        let mut fields = vec![("at", text(true))];
        let required = if key.is_empty() {
            vec![]
        } else {
            fields.push((key, inner));
            if op.ends_with("clear") {
                vec![]
            } else {
                vec![key]
            }
        };
        Some(Self{schema:object(&fields,&required),target,effect,
            notes:"Operands are structured values. There is no shorthand or whole-schema JSON input. Unspecified properties and unrelated XML bytes are preserved; create template.add first, then compose operations. Preview makes no writes; execute its executionToken, read the post-image and check the template.",example_args})
    }

    pub(crate) fn mxl(op: &str) -> Option<Self> {
        let scalar = json!({"anyOf":[{"type":"string"},{"type":"number"},{"type":"boolean"},{"type":"null"}]});
        let (schema, target, effect, notes, example_args) = match op {
            "mxl.set" => {
                let cells = json!({"type":"object","minProperties":1,
                    "propertyNames":{"type":"string","format":"unica-mxl-cell-address",
                        "description":format!("R<row>C<column>, 1-based inside the area; row <= {MXL_MAX_ROW}, column <= {MXL_MAX_COLUMN}; leading zeros and plus are accepted")},
                    "additionalProperties":scalar});
                (object(&[("at",text(true)),("values",object(&[("area",text(true)),
                    ("cells",cells),("columns",json!({"type":"integer","format":"unica-json-i64","minimum":1,"maximum":i64::MAX,
                        "description":"Positive JSON integer serialized without a decimal point or exponent, in the signed 64-bit range; 1 is accepted, 1.0 and 1e0 are refused."}))], &["area","cells"]))], &["values"]),
                "The existing SpreadsheetDocument Template itself: <source-set>:<Owner>.<name>.Template.<name>; not its Area or Body child.",
                "Update an existing named Rows area or append a new area. Addressed parameter/template cells become ordinary text. Unaddressed cells and areas remain. Width grows to max(existing width, written column, columns); it never shrinks.",
                "Read Template -> Area -> Body for ordered nonempty text cells; Body index is a reading ordinal, not RnCn. Parameter lists parameter names. Readability is broader than editability: drawings, non-Rows areas, overlapping/reordered areas, outside rows, multiple column sets and multilingual content the writer cannot preserve are refused before writing. Strings remain text; numbers/booleans are serialized as text; null is empty text. Full DSL, fonts, styles, merges and page properties are not public inputs. Create and execute template.add before addressing a new template.",
                json!({"values":{"area":"Header","columns":4,"cells":{"R1C1":"Item","R1C2":"Amount","R2C1":true,"R2C2":null}}}))
            }
            "template.add" => {
                let kinds: Vec<_> = crate::domain::metadata::MetaTemplateKind::ALL
                    .iter()
                    .map(|kind| kind.as_str())
                    .collect();
                let kind = json!({"default":"SpreadsheetDocument","anyOf":[{"type":"string","enum":kinds},{"not":{"type":"string"}}]});
                (object(&[("at",text(true)),("items",json!({"type":"array","minItems":1,"items":object(&[
                    ("name",text(true)),("templateType",kind),("synonym",json!({}))], &["name"])}))], &["items"]),
                "An existing metadata owner that exposes template.add in can. Read its Template collection before choosing a new valid 1C identifier.",
                "Register a new template and its initial content. The new address exists only after execution of the saved plan; preview does not create it.",
                "The kind vocabulary comes from MetaTemplateKind. Current initial-content synthesis supports SpreadsheetDocument and DataCompositionSchema; HTMLDocument, TextDocument and BinaryData are typed refusals (tracked gap #992). Missing or non-string templateType uses SpreadsheetDocument for compatibility. synonym is accepted but currently ignored. A new DCS has no dataset. For MXL, execute creation first, then preview/execute mxl.set; view returns a node projection, not a round-trip JSON DSL.",
                json!({"items":[{"name":"PrintLayout","templateType":"SpreadsheetDocument"}]}))
            }
            _ => return None,
        };
        Some(Self {
            schema,
            target,
            effect,
            notes,
            example_args,
        })
    }

    pub(crate) fn normalize(&self, args: &Value, path: &str) -> Result<Value, ContractError> {
        validate(&self.schema, args, path)?;
        let mut args = args.clone();
        defaults(&self.schema, &mut args);
        normalize_aliases(&self.schema, &mut args);
        Ok(args)
    }

    pub(crate) fn details(&self) -> Value {
        json!({"argsSchema":self.schema,"target":self.target,"effect":self.effect,
            "notes":self.notes,"exampleArgs":self.example_args,
            "exampleUse":"Read existing targets and required names through view. For creation, choose a new unused name. These argument examples do not establish that their names exist in this workspace.",
            "help":{"scope":"plugin","path":"references/use-cases/reports-printing.md"},
            "execution":"Preview with at/ops; execute only data.executionToken from the successful plan, then read the post-image and check. Observe the same Task to terminal state; never repeat a mutation to wait for it."})
    }
}

fn error(path: &str, message: impl Into<String>) -> ContractError {
    ContractError {
        path: path.into(),
        message: message.into(),
    }
}

fn validate(schema: &Value, value: &Value, path: &str) -> Result<(), ContractError> {
    if schema
        .get("not")
        .is_some_and(|excluded| validate(excluded, value, path).is_ok())
    {
        return Err(error(path, "argument selects an excluded alternative"));
    }
    let valid_type = match schema["type"].as_str() {
        Some("object") => value.is_object(),
        Some("array") => value.is_array(),
        Some("string") => value.is_string(),
        Some("boolean") => value.is_boolean(),
        Some("number") => value.is_number(),
        Some("integer") => value.as_i64().is_some(),
        Some("null") => value.is_null(),
        None => true,
        other => unreachable!("unsupported internal schema type: {other:?}"),
    };
    if !valid_type {
        return Err(error(path, format!("expected {}", schema["type"])));
    }
    if let Some(minimum) = schema["minimum"].as_i64() {
        if !value.as_i64().is_some_and(|number| number >= minimum) {
            return Err(error(path, "integer is below the supported minimum"));
        }
    }
    if schema["format"] == "unica-mxl-cell-address"
        && !value
            .as_str()
            .is_some_and(|key| parse_mxl_cell_address(key).is_some())
    {
        return Err(error(
            path,
            format!("expected area-local R<row>C<column>, up to R{MXL_MAX_ROW}C{MXL_MAX_COLUMN}"),
        ));
    }
    if let Some(pattern) = schema["pattern"].as_str() {
        if !value.as_str().is_some_and(|text| {
            regex::Regex::new(pattern)
                .expect("owned contract pattern")
                .is_match(text)
        }) {
            return Err(error(
                path,
                "text does not match the supported argument grammar",
            ));
        }
    }
    if let Some(values) = schema["enum"].as_array() {
        if !values.contains(value) {
            return Err(error(
                path,
                "value is not one of the supported alternatives",
            ));
        }
    }
    if let Some(alternatives) = schema["anyOf"].as_array() {
        if !alternatives
            .iter()
            .any(|choice| validate(choice, value, path).is_ok())
        {
            return Err(error(
                path,
                "one of the supported argument alternatives must match",
            ));
        }
    }
    if let Some(object) = value.as_object() {
        if schema["minProperties"]
            .as_u64()
            .is_some_and(|minimum| (object.len() as u64) < minimum)
        {
            return Err(error(path, "object must not be empty"));
        }
        if let Some(names) = schema.get("propertyNames") {
            for key in object.keys() {
                validate(names, &Value::String(key.clone()), &format!("{path}.{key}"))?;
            }
        }
        if schema["additionalProperties"].is_object() {
            for (key, value) in object {
                if schema["properties"].get(key).is_none() {
                    validate(
                        &schema["additionalProperties"],
                        value,
                        &format!("{path}.{key}"),
                    )?;
                }
            }
        }
        if let Some(required) = schema["required"].as_array() {
            for key in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(key) {
                    return Err(error(
                        &format!("{path}.{key}"),
                        "required argument is missing",
                    ));
                }
            }
        }
        if let Some(properties) = schema["properties"].as_object() {
            for (key, value) in object {
                match properties.get(key) {
                    Some(child) => validate(child, value, &format!("{path}.{key}"))?,
                    None if schema["additionalProperties"] == false => {
                        return Err(error(&format!("{path}.{key}"), "unknown argument"))
                    }
                    None => {}
                }
            }
        }
    }
    if let Some(values) = value.as_array() {
        if schema["minItems"]
            .as_u64()
            .is_some_and(|minimum| (values.len() as u64) < minimum)
        {
            return Err(error(path, "array must not be empty"));
        }
    }
    if let (Some(items), Some(values)) = (schema.get("items"), value.as_array()) {
        for (index, value) in values.iter().enumerate() {
            validate(items, value, &format!("{path}[{index}]"))?;
        }
    }
    Ok(())
}

fn defaults(schema: &Value, value: &mut Value) {
    if let (Some(properties), Some(object)) =
        (schema["properties"].as_object(), value.as_object_mut())
    {
        for (key, child) in properties {
            if !object.contains_key(key) {
                if let Some(default) = child.get("default") {
                    object.insert(key.clone(), default.clone());
                }
            }
            if let Some(value) = object.get_mut(key) {
                defaults(child, value);
            }
        }
    }
    if let (Some(items), Some(values)) = (schema.get("items"), value.as_array_mut()) {
        for value in values {
            defaults(items, value);
        }
    }
}

fn normalize_aliases(schema: &Value, value: &mut Value) {
    if let Some(object) = value.as_object_mut() {
        if let Some(names) = schema["x-unica-aliases"].as_array() {
            let names: Vec<_> = names.iter().filter_map(Value::as_str).collect();
            let selected = names.iter().find_map(|name| object.get(*name).cloned());
            for name in &names {
                object.remove(*name);
            }
            if let (Some(canonical), Some(selected)) = (names.first(), selected) {
                object.insert((*canonical).to_string(), selected);
            }
        }
        if let Some(properties) = schema["properties"].as_object() {
            for (key, child) in properties {
                if let Some(value) = object.get_mut(key) {
                    normalize_aliases(child, value);
                }
            }
        }
    }
    if let (Some(items), Some(values)) = (schema.get("items"), value.as_array_mut()) {
        for value in values {
            normalize_aliases(items, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::apply::{OperationFamily, OperationRegistry, IMPLEMENTED_APPLY_OPERATIONS};

    #[test]
    fn mxl_and_template_contracts_cover_examples_and_keep_legacy_value_grammar() {
        let registry = OperationRegistry::closed();
        for name in ["mxl.set", "template.add"] {
            let descriptor = registry
                .descriptors()
                .iter()
                .find(|d| d.name() == name)
                .unwrap();
            let contract = descriptor.argument_contract().expect("shared contract");
            assert!(contract.normalize(&contract.example_args, "args").is_ok());
        }
    }

    #[test]
    fn mxl_contract_checks_dynamic_keys_scalars_and_exact_integer_values() {
        let contract = OperationContract::mxl("mxl.set").unwrap();
        for key in ["R1C1", "R01C01", "R+1C+1", "R10000C1000"] {
            assert!(
                contract
                    .normalize(
                        &json!({"values":{"area":"A","cells":{key:null},"columns":1001}}),
                        "args"
                    )
                    .is_ok(),
                "{key}"
            );
        }
        for values in [
            json!({"area":"A","cells":[]}),
            json!({"area":"A","cells":{}}),
            json!({"area":"A","cells":{"R0C1":"x"}}),
            json!({"area":"A","cells":{"R10001C1":"x"}}),
            json!({"area":"A","cells":{"R1C1001":"x"}}),
            json!({"area":"A","cells":{"R1C1":{}}}),
            json!({"area":"A","cells":{"R1C1":"x"},"columns":0}),
            json!({"area":"A","cells":{"R1C1":"x"},"columns":1.0}),
            serde_json::from_str(r#"{"area":"A","cells":{"R1C1":"x"},"columns":1e0}"#).unwrap(),
            json!({"area":"A","cells":{"R1C1":"x"},"columns":u64::MAX}),
            json!({"area":"A","cells":{"R1C1":"x"},"style":"bold"}),
        ] {
            assert!(
                contract
                    .normalize(&json!({"values":values}), "args")
                    .is_err(),
                "{values}"
            );
        }
        assert!(contract.normalize(&json!({"values":{"area":"A","cells":{"R1C1":"x","R1C2":42,"R1C3":true,"R1C4":null}}}), "args").is_ok());
        assert!(contract
            .normalize(
                &json!({"values":{"area":"A","cells":{"R1C1":"x"},"columns":i64::MAX}}),
                "args"
            )
            .is_ok());
        let creation = OperationContract::mxl("template.add").unwrap();
        for kind in [
            Value::Null,
            json!(7),
            json!(true),
            json!({}),
            json!("SpreadsheetDocument"),
        ] {
            assert!(creation
                .normalize(
                    &json!({"items":[{"name":"Layout","templateType":kind,"synonym":7}]}),
                    "args"
                )
                .is_ok());
        }
        assert_eq!(
            creation
                .normalize(&json!({"items":[{"name":"Layout"}]}), "args")
                .unwrap()["items"][0]["templateType"],
            "SpreadsheetDocument"
        );
        assert!(creation
            .normalize(
                &json!({"items":[{"name":"Layout","templateType":"Unknown"}]}),
                "args"
            )
            .is_err());
        assert!(creation.normalize(&json!({"items":[]}), "args").is_err());
        assert_eq!(parse_mxl_cell_address("R01C+01"), Some((1, 1)));
    }

    #[test]
    fn every_implemented_dcs_operation_has_a_valid_example_and_small_contract() {
        let registry = OperationRegistry::closed();
        let mut count = 0;
        for descriptor in registry.descriptors().iter().filter(|descriptor| {
            descriptor.family() == OperationFamily::Dcs
                && IMPLEMENTED_APPLY_OPERATIONS.contains(&descriptor.name())
        }) {
            let contract = descriptor
                .argument_contract()
                .unwrap_or_else(|| panic!("{} lacks its DCS contract", descriptor.name()));
            contract
                .normalize(&contract.example_args, "args")
                .unwrap_or_else(|error| panic!("{}: {error:?}", descriptor.name()));
            assert!(serde_json::to_vec(&contract.details()).unwrap().len() < 16 * 1024);
            count += 1;
        }
        assert_eq!(count, 53);
    }

    #[test]
    fn dcs_contract_defaults_aliases_and_roles_match_the_typed_boundary() {
        let query = OperationContract::dcs("query.patch").unwrap();
        assert_eq!(
            query
                .normalize(&json!({"values":{"find":"old"}}), "args")
                .unwrap(),
            json!({"values":{"find":"old","replace":"","once":false}})
        );
        let structure = OperationContract::dcs("structure.set").unwrap();
        assert_eq!(
            structure.normalize(&json!({"values":{}}), "args").unwrap(),
            json!({"values":{"groupBy":[],"details":true}})
        );
        let field = OperationContract::dcs("field.add").unwrap();
        for args in [
            json!({"items":[{"name":"Alias"}]}),
            json!({"items":[{"dataPath":"Path","name":"Alias","title":""}]}),
            json!({"items":[{"dataPath":"Path","name":"","title":""}]}),
        ] {
            assert!(field.normalize(&args, "args").is_ok());
        }
        for args in [
            json!({"items":[{}]}),
            json!({"items":[{"name":"  \n"}]}),
            json!({"items":[{"name":7}]}),
            json!({"items":[{"dataPath":"","name":"Fallback"}]}),
        ] {
            assert!(field.normalize(&args, "args").is_err());
        }
        let role = OperationContract::dcs("fieldRole.set").unwrap();
        for value in [
            json!({"dimension":true}),
            json!({"periodType":"Main"}),
            json!({"balanceType":"OpeningBalance"}),
            json!({"parentDimension":"Parent","accountField":"Account"}),
        ] {
            assert!(role
                .normalize(&json!({"values":{"field":"Amount","role":value}}), "args")
                .is_ok());
        }
        assert!(role
            .normalize(
                &json!({"values":{"field":"Amount","role":"dimension"}}),
                "args"
            )
            .is_err());
    }
}
