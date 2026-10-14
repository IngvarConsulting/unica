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
    /// Plugin reference that explains the use case of the operation.
    pub(crate) help: &'static str,
}

const REPORTS_HELP: &str = "references/use-cases/reports-printing.md";
const METADATA_HELP: &str = "skills/meta-edit/SKILL.md";
const ROLE_HELP: &str = "skills/role-edit/SKILL.md";
const FORM_HELP: &str = "skills/form-edit/SKILL.md";

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
        let data_target = "Existing DCS Template.DataSet.<name>; repeat DataSet.<name> for a nested Union member.";
        let query_target = "The query of an existing dataset: Template.DataSet.<name>.Query; repeat DataSet.<name> for a nested Union member. The address names the dataset; there is no dataset argument.";
        let setting_target = "Existing DCS Template.Setting.<name>; the address names the variant.";
        let container_target = "Existing DCS Template.Setting.<name> or its named structure item Setting.<name>.Item.<item>[.Item.<child>…]; the address names the settings container. A named item under unnamed groups is addressed from its nearest named ancestor; read Setting.<name>.Item first.";
        let structure_target = "Existing named structure item Template.Setting.<name>.Item.<item>[.Item.<child>…]; the address names the item. Read Setting.<name>.Item first.";
        let data_source_target = "Existing DCS Template.DataSource.<name>; the address names the source. Create a source with dataSource.add on the template.";
        let calculated_target = "Existing DCS Template.CalculatedField.<dataPath>; the address names the calculated field. A dataPath dot is escaped: CalculatedField.A\\.B.";
        let total_target = "Existing DCS Template.Total.<dataPath>; the address names the total. A dataPath dot is escaped: Total.A\\.B.";
        let field_target = "Existing DCS field Template.DataSet.<name>.Field.<dataPath>; repeat DataSet.<name> for a nested Union member. The address names the field; there is no field argument. A composite dataPath escapes its dot: Field.Owner\\.Code.";
        let parameter_target = "Existing DCS Template.Parameter.<name>; the address names the parameter. A missing parameter is refused; create it with parameter.add on the template.";
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
            "dataSource.add" => {
                let kind = json!({"type":"string","enum":["Local","External"],"default":"Local"});
                ("items",object(&[("name",text(true)),("kind",kind)],&["name"]),schema_target,"Add a new named source; an existing name is refused. Change it later at Template.DataSource.<name>.",json!({"items":[{"name":"Database"}]}))
            }
            "dataSource.set" => ("values",object(&[("kind",json!({"type":"string","enum":["Local","External"]}))],&["kind"]),data_source_target,"Change only the kind of the addressed source.",json!({"values":{"kind":"Local"}})),
            "dataSource.remove" => ("",json!({}),data_source_target,"Remove the addressed source; a source still referenced by a dataset is refused.",json!({})),
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
                let mut inner=object(&[("type",text(false)),("title",title.clone()),("useRestriction",restriction.clone()),("attributeUseRestriction",restriction.clone()),("presentationExpression",text(false)),("sourceField",text(true))],&[]);
                inner["minProperties"]=json!(1);
                ("values",inner,field_target,"Update only supplied field mapping/title/type/restrictions/presentation expression. Query fields derive their type from query; field.set removes stale valueType on the addressed Query field. Other field data remains.",json!({"values":{"title":{"ru":"Сумма","en":"Amount"}}}))
            }
            "field.remove" | "parameter.remove" => ("",json!({}),if op=="field.remove"{field_target}else{parameter_target},"Remove only the named terminal Field or Parameter. References and selections are separate operations.",json!({})),
            "fieldRole.set" => {
                let flags=object(&DCS_ROLE_KEYS.iter().map(|key|(*key,if DCS_ROLE_FLAGS.contains(key){json!({"type":"boolean"})}else{text(true)})).collect::<Vec<_>>(),&[]);
                ("values",object(&[("role",flags)],&["role"]),field_target,"Replace the addressed field's role with the supplied object. Values are validated against platform types; omitted role entries are removed.",json!({"values":{"role":{"dimension":true}}}))
            }
            "parameter.add" => ("items",parameter(),schema_target,"Add a new schema parameter. Parameters for dates and their expressions are explicit separate operations.",json!({"items":[{"name":"Period","type":"date"}]})),
            "parameter.set" => {
                let mut inner=parameter();
                inner["properties"].as_object_mut().expect("parameter properties").remove("name");
                inner["required"]=json!([]);
                inner["minProperties"]=json!(1);
                ("values",inner,parameter_target,"Patch only supplied properties of the addressed schema parameter.",json!({"values":{"title":"Period"}}))
            }
            "parameter.rename" => ("values",object(&[("newName",text(true))],&["newName"]),parameter_target,"Rename the addressed parameter declaration only. References are explicit query/expression operations in the same batch.",json!({"values":{"newName":"ReportPeriod"}})),
            "parameter.reorder" => ("items",object(&[("name",text(true))],&["name"]),schema_target,"Reorder all schema parameter elements by the supplied complete unique list. Comments and all other nodes stay in their original slots.",json!({"items":[{"name":"Period"}]})),
            "calculatedField.add" => {
                let mut inner=aliases(object(&[("name",text(true)),("dataPath",text(true)),("expression",text(true)),("title",title.clone()),("type",text(false))],&["expression"]),&["name","dataPath"]);
                inner["properties"]["useRestriction"]=restriction.clone();
                ("items",inner,schema_target,"Add a calculated field; selection is independent. Change it later at Template.CalculatedField.<dataPath>.",json!({"items":[{"name":"DoubleAmount","expression":"Amount * 2"}]}))
            }
            "total.add" => ("items",aliases(object(&[("field",text(true)),("dataPath",text(true)),("expression",text(false)),("group",json!({"anyOf":[text(true),strings.clone()]}))],&[]),&["field","dataPath"]),schema_target,"Add a total with explicit expression/group. Omitted expression is Сумма(field). Change it later at Template.Total.<dataPath>.",json!({"items":[{"field":"Amount","expression":"Сумма(Amount)"}]})),
            "calculatedField.remove" => ("",json!({}),calculated_target,"Remove the addressed calculated field only; references to it are separate operations.",json!({})),
            "total.remove" => ("",json!({}),total_target,"Remove the addressed total only.",json!({})),
            "variant.add" => ("items",optional_aliases(object(&[("name",text(true)),("title",title.clone()),("presentation",title.clone())],&["name"]),&["title","presentation"]),schema_target,"Create an empty settings variant. Configure selection and structure separately.",json!({"items":[{"name":"Additional","title":"Additional"}]})),
            "variant.set" => ("values",optional_aliases(object(&[("title",title.clone()),("presentation",title.clone())],&[]),&["title","presentation"]),setting_target,"Change the presentation of the addressed settings variant. A missing variant is refused; create it with variant.add on the template.",json!({"values":{"title":"Report"}})),
            "variant.remove" => ("",json!({}),setting_target,"Remove the addressed variant.",json!({})),
            "query.set" => ("values",aliases(object(&[("query",text(true)),("text",text(true))],&[]),&["query","text"]),query_target,"Replace query text. Supply literal text, not @file.",json!({"values":{"query":"ВЫБРАТЬ 1 КАК Amount"}})),
            "query.patch" => ("values",object(&[("find",text(true)),("replace",json!({"type":"string","default":""})),("once",json!({"type":"boolean","default":false}))],&["find"]),query_target,"Patch decoded query text. No matches fail; once=true requires exactly one. Comments/PI are preserved; a marker within replaced text anchors to replacement start.",json!({"values":{"find":"1 КАК Amount","replace":"2 КАК Amount","once":true}})),
            "filter.add" | "filter.set" => (if op.ends_with("add"){"items"}else{"values"},filter(),container_target,"Add a filter or patch one uniquely selected by field in the addressed settings container. Literal strings retain @, brackets and comparison words. Set requires a unique match.",if op.ends_with("add"){json!({"items":[{"field":"Amount","comparison":"Greater","value":0}]})}else{json!({"values":{"field":"Amount","comparison":"Greater","value":1}})}),
            "filter.remove" => ("values",object(&[("field",text(true))],&["field"]),container_target,"Remove one uniquely selected filter; multiple matches fail.",json!({"values":{"field":"Amount"}})),
            "selection.add" | "order.add" => {
                let mut inner=optional_aliases(object(&[("field",text(true)),("dataPath",text(true)),("kind",json!({"type":"string","enum":["Field","Auto"],"default":"Field"}))],&[]),&["field","dataPath"]);
                if op=="order.add" {inner["properties"]["direction"]=json!({"type":"string","enum":["Asc","Desc"],"default":"Asc"});}
                ("items",inner,container_target,"Add one explicit Field or Auto selection/order item. Field requires field; Auto has no field. Use index selectors for anonymous Auto items.",json!({"items":[{"field":"Amount"}]}))
            }
            "filter.clear" | "selection.clear" | "order.clear" | "conditionalAppearance.clear" => ("values",object(&[],&[]),container_target,"Clear the selected collection's elements. The collection's attributes/comments stay; an absent collection is unchanged.",json!({"values":{}})),
            "dataParameter.add" | "dataParameter.set" | "outputParameter.set" => (if op=="dataParameter.add"{"items"}else{"values"},object(&[("name",text(true)),("value",typed_value.clone()),("valueType",text(true)),("use",json!({"type":"boolean"})),("viewMode",json!({"type":"string","enum":["Normal","Inaccessible","QuickAccess"]})),("userSettingID",text(true)),("userSettingPresentation",title.clone())],if op=="dataParameter.add"{&["name","value"][..]}else{&["name"][..]}),if op=="outputParameter.set"{container_target}else{setting_target},"Set only supplied variant data/output parameter properties. Typed defaults, NULL, periods, references and localized values preserve their platform type; schema parameter declarations remain separate.",if op=="dataParameter.add"{json!({"items":[{"name":"Period","value":"2026-01-01T00:00:00","valueType":"xs:dateTime"}]})}else{json!({"values":{"name":"Period","value":"2026-01-01T00:00:00","valueType":"xs:dateTime"}})}),
            "conditionalAppearance.add" => {
                let mut condition = filter();
                condition["properties"]["comparison"]["default"] = json!("Equal");
                ("items",object(&[("fields",strings.clone()),("filter",json!({"type":"array","items":condition})),("use",json!({"type":"boolean"})),("appearance",json!({"type":"object","minProperties":1,"additionalProperties":appearance_value.clone()}))],&["fields","appearance"]),container_target,"Add conditional appearance with explicit use, comparisons and typed value atoms. Formatting does not change field values.",json!({"items":[{"fields":["Amount"],"use":false,"appearance":{"Формат":{"valueType":"v8:LocalStringType","value":{"ru":"ЧДЦ=2"}}}}]}))
            }
            "structure.add" => ("items",structure(),container_target,"Add one named group/table/chart into the addressed variant or structure item; axis selects the row/column/point/series of an addressed table/chart. The name must be new in the variant. Add each child with a separate operation at its parent's address.",json!({"items":[{"name":"ByAmount","kind":"group","groupBy":["Amount"]}]})),
            "structure.set" => ("values",object(&[("name",text(true)),("groupBy",json!({"type":"array","items":text(true),"default":[]})),("details",json!({"type":"boolean","default":true}))],&[]),setting_target,"Replace the variant structure with one group and optional nested details. groupBy=[] is a details group; use structure.add for tables/charts and nested composition.",json!({"values":{"name":"ByAmount","groupBy":["Amount"],"details":true}})),
            "structure.patch" => ("values",object(&[("groupBy",strings.clone())],&[]),structure_target,"Patch groupBy and supplied view properties of the addressed structure item. Other structure and settings are preserved.",json!({"values":{"groupBy":["Amount"]}})),
            "structure.remove" => ("",json!({}),structure_target,"Remove the addressed structure item and its children.",json!({})),
            "calculatedField.set" => {
                let mut inner=object(&[("expression",text(true)),("title",title.clone()),("type",text(true)),("useRestriction",restriction.clone())],&[]);
                inner["minProperties"]=json!(1);
                ("values",inner,calculated_target,"Patch only supplied properties of the addressed calculated field.",json!({"values":{"expression":"Amount * 3"}}))
            }
            "total.set" => {
                let mut inner=object(&[("expression",text(true)),("group",json!({"anyOf":[text(true),strings.clone()]}))],&[]);
                inner["minProperties"]=json!(1);
                ("values",inner,total_target,"Patch supplied expression or grouping list of the addressed total.",json!({"values":{"expression":"Максимум(Amount)"}}))
            }
            "selection.set" | "order.set" => {
                let mut inner=object(&[("field",text(true)),("index",json!({"type":"integer","minimum":0})),("use",json!({"type":"boolean"})),("viewMode",json!({"type":"string","enum":["Normal","Inaccessible","QuickAccess"]})),("userSettingID",text(true)),("userSettingPresentation",text(false))],&[]);
                if op=="order.set"{inner["properties"]["direction"]=json!({"type":"string","enum":["Asc","Desc"]});}
                ("values",inner,container_target,"Patch one uniquely selected field item, preserving unspecified settings.",json!({"values":{"field":"Amount","use":false}}))
            }
            "selection.remove" | "order.remove" => ("values",object(&[("field",text(true)),("index",json!({"type":"integer","minimum":0}))],&[]),container_target,"Remove one uniquely selected field item.",json!({"values":{"field":"Amount"}})),
            "dataParameter.remove" | "outputParameter.remove" => ("values",object(&[("name",text(true))],&["name"]),if op=="outputParameter.remove"{container_target}else{setting_target},"Remove the uniquely named settings parameter.",json!({"values":{"name":"Period"}})),
            "conditionalAppearance.remove" => ("values",object(&[("index",json!({"type":"integer","minimum":0}))],&["index"]),container_target,"Remove the zero-based appearance item from the captured settings image; read props before selecting.",json!({"values":{"index":0}})),
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
                inner["minProperties"] = json!(1);
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
            notes:"Operands are structured values. There is no shorthand or whole-schema JSON input. Unspecified properties and unrelated XML bytes are preserved; create template.add first, then compose operations. Preview makes no writes; execute its executionToken, read the post-image and check the template.",example_args,help:REPORTS_HELP})
    }

    pub(crate) fn mxl(op: &str) -> Option<Self> {
        let scalar = json!({"anyOf":[{"type":"string"},{"type":"number"},{"type":"boolean"},{"type":"null"}]});
        let (schema, target, effect, notes, example_args) = match op {
            "mxl.set" | "area.add" => {
                let cells = json!({"type":"object","minProperties":1,
                    "propertyNames":{"type":"string","format":"unica-mxl-cell-address",
                        "description":format!("R<row>C<column>, 1-based inside the area; row <= {MXL_MAX_ROW}, column <= {MXL_MAX_COLUMN}; leading zeros and plus are accepted")},
                    "additionalProperties":scalar});
                let columns = json!({"type":"integer","format":"unica-json-i64","minimum":1,"maximum":i64::MAX,
                    "description":"Positive JSON integer serialized without a decimal point or exponent, in the signed 64-bit range; 1 is accepted, 1.0 and 1e0 are refused."});
                let notes = "Read Template -> Area -> Body for ordered nonempty text cells; Body index is a reading ordinal, not RnCn. Parameter lists parameter names. Readability is broader than editability: drawings, non-Rows areas, overlapping/reordered areas, outside rows, multiple column sets and multilingual content the writer cannot preserve are refused before writing. Strings remain text; numbers/booleans are serialized as text; null is empty text. Full DSL, fonts, styles, merges and page properties are not public inputs. Create and execute template.add before addressing a new template.";
                if op == "mxl.set" {
                    (object(&[("at",text(true)),("values",object(&[("cells",cells),("columns",columns)], &["cells"]))], &["values"]),
                    "An existing named area of a SpreadsheetDocument Template: <source-set>:<Owner>.<name>.Template.<name>.Area.<area>. The address names the area; a missing area is refused, create it with area.add on the template.",
                    "Update cells of the existing named Rows area. Addressed parameter/template cells become ordinary text. Unaddressed cells and areas remain. Width grows to max(existing width, written column, columns); it never shrinks.",
                    notes,
                    json!({"values":{"columns":4,"cells":{"R1C1":"Item","R1C2":"Amount","R2C1":true,"R2C2":null}}}))
                } else {
                    (object(&[("at",text(true)),("items",json!({"type":"array","minItems":1,"items":object(&[("name",text(true)),("cells",cells),("columns",columns)], &["name","cells"])}))], &["items"]),
                    "The existing SpreadsheetDocument Template itself: <source-set>:<Owner>.<name>.Template.<name>. Read its Area collection before choosing a new area name.",
                    "Append new named Rows areas after the existing ones with their initial cells. An existing area name is refused; change it with mxl.set at its Area address. Width grows to max(existing width, written column, columns); it never shrinks.",
                    notes,
                    json!({"items":[{"name":"Header","columns":4,"cells":{"R1C1":"Item","R1C2":"Amount"}}]}))
                }
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
                "The kind vocabulary comes from MetaTemplateKind. Current initial-content synthesis supports SpreadsheetDocument and DataCompositionSchema; HTMLDocument, TextDocument and BinaryData are typed refusals (tracked gap #992). Missing or non-string templateType uses SpreadsheetDocument for compatibility. synonym is accepted but currently ignored. A new DCS has no dataset. For MXL, add areas with area.add on the template, then change their cells with mxl.set at Template.<name>.Area.<area>; view returns a node projection, not a round-trip JSON DSL.",
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
            help: REPORTS_HELP,
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
            "help":{"scope":"plugin","path":self.help},
            "execution":"Preview with at/ops; execute only data.executionToken from the successful plan, then read the post-image and check. Observe the same Task to terminal state; never repeat a mutation to wait for it."})
    }
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

const IDENTIFIER_PATTERN: &str = r"^[_A-Za-zА-Яа-яЁё][_A-Za-zА-Яа-яЁё0-9]*$";

/// Typed collection of a member operation of a metadata object.
fn metadata_member_collection(family: &str) -> Option<crate::domain::metadata::MetaCollection> {
    use crate::domain::metadata::MetaCollection;
    Some(match family {
        "attribute" => MetaCollection::Attributes,
        "tabularSection" => MetaCollection::TabularSections,
        "dimension" => MetaCollection::Dimensions,
        "resource" => MetaCollection::Resources,
        "enumValue" => MetaCollection::EnumValues,
        "column" => MetaCollection::Columns,
        "command" => MetaCollection::Commands,
        _ => return None,
    })
}

/// JSON Schema of one collection element: exactly the fields the typed
/// metadata writer accepts for this collection. `update` describes the
/// changed fields of an existing member, named by the address.
fn metadata_element_schema(
    collection: crate::domain::metadata::MetaCollection,
    update: bool,
    site: Option<MetadataSite>,
) -> Value {
    use crate::domain::metadata::{
        fill_value_schema, metadata_collection_spec, metadata_type_schema, position_schema,
        MetaCollection,
    };
    let spec = metadata_collection_spec(collection);
    let identifier = json!({"type":"string","pattern":IDENTIFIER_PATTERN});
    let mut fields = Map::new();
    if update {
        fields.insert("newName".into(), identifier);
    } else {
        fields.insert("name".into(), identifier);
    }
    fields.insert("synonym".into(), json!({"type":"string"}));
    fields.insert("comment".into(), json!({"type":"string"}));
    if spec.allows_type {
        fields.insert("type".into(), metadata_type_schema());
    }
    if spec.allows_required {
        fields.insert("required".into(), json!({"type":"boolean"}));
    }
    if fill_value_allowed(collection, site) {
        fields.insert("fillValue".into(), fill_value_schema());
    }
    if spec.allows_position {
        fields.insert("position".into(), position_schema());
    }
    if spec.allows_indexing {
        fields.insert(
            "indexing".into(),
            json!({"type":"string","enum":["DontIndex","Index","IndexWithAdditionalOrder"]}),
        );
    }
    if spec.allows_nested_attributes && !update {
        fields.insert(
            "attributes".into(),
            json!({"type":"array","minItems":1,
                "items":metadata_element_schema(MetaCollection::Attributes, false,
                    site.map(|site| MetadataSite { in_section: true, ..site }))}),
        );
    }
    let mut schema = json!({"type":"object","additionalProperties":false,"properties":fields});
    if update {
        schema["minProperties"] = json!(1);
    } else {
        schema["required"] = json!(["name"]);
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

    /// The contract on a concrete node: fields whose legality depends on the
    /// owner kind or on the tabular section are narrowed to that node.
    pub(crate) fn metadata_at(op: &str, site: Option<MetadataSite>) -> Option<Self> {
        let (family, verb) = op.split_once('.')?;
        let owner = site.map(|site| site.owner);
        let at = || ("at", text(true));
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
        let collection = metadata_member_collection(family)?;
        let (schema, target, effect, notes, example_args) = match verb {
            "add" => {
                let target = if family == "attribute" {
                    "A metadata object with attributes: <source-set>:<Kind>.<Name> adds header attributes; <source-set>:<Kind>.<Name>.TabularSection.<T> adds attributes of that tabular section."
                } else {
                    "A metadata object with this collection: <source-set>:<Kind>.<Name>."
                };
                let example = match family {
                    "attribute" | "dimension" => json!({"items":[{"name":"Sample",
                        "type":{"variants":[{"kind":"string","length":50,"allowedLength":"variable"}]}}]}),
                    // Ресурс регистра накопления числовой: пример годится всем регистрам.
                    "resource" => json!({"items":[{"name":"Sample",
                        "type":{"variants":[{"kind":"number","digits":15,"fraction":2,"sign":"any"}]}}]}),
                    _ => json!({"items":[{"name":"Sample"}]}),
                };
                (
                    object(&[at(), ("items", json!({"type":"array","minItems":1,"items":metadata_element_schema(collection, false, site)}))], &["items"]),
                    target,
                    "Add new members after the existing ones, or at position.before/position.after an existing sibling. A name already used in the collection is refused.",
                    "Read the collection branch of the target before choosing a new valid 1C identifier. A new node is addressable only after the plan is executed; within one apply call a later operation may already address it. Cross-field rules — fillValue needs an explicit type and must match it — are checked by the planner, not by this schema.",
                    example,
                )
            }
            "set" => (
                object(&[at(), ("values", metadata_element_schema(collection, true, site))], &["values"]),
                match family {
                    "attribute" => "The existing attribute: <source-set>:<Kind>.<Name>.Attribute.<A> or <source-set>:<Kind>.<Name>.TabularSection.<T>.Attribute.<A>.",
                    _ => "The existing member at its own address: <source-set>:<Kind>.<Name>.<Leaf>.<Member>.",
                },
                "Change the listed fields of the member; newName renames it. Unlisted fields stay.",
                "The member is named by its address, not by values.name. Read the member first. Cross-field rules — fillValue must match the member type — are checked by the planner, not by this schema.",
                json!({"values":{"synonym":"Sample"}}),
            ),
            "remove" => (
                object(&[at()], &[]),
                "The existing member at its own address: <source-set>:<Kind>.<Name>.<Leaf>.<Member>.",
                "Remove the member from its owner.",
                "The member is named by its address; the operation takes no other arguments.",
                json!({}),
            ),
            _ => return None,
        };
        Some(Self {
            schema,
            target,
            effect,
            notes,
            example_args,
            help: METADATA_HELP,
        })
    }
}

impl OperationContract {
    /// Права роли (#1354): `right.add` создаёт записи на роли, `right.set`
    /// меняет одно право существующей записи по её адресу.
    pub(crate) fn role(op: &str) -> Option<Self> {
        let (schema, target, effect, notes, example_args) = match op {
            "right.add" => (
                object(
                    &[
                        ("at", text(true)),
                        (
                            "items",
                            json!({"type":"array","minItems":1,"items":object(&[
                                ("object",json!({"type":"string","pattern":r"^[^.\s]+\.[^.\s]+(\.[^.\s]+\.[^.\s]+)*$",
                                    "description":"A metadata object `Kind.Name` (or `Configuration.Name`), or a nested one such as `Kind.Name.Command.C`, without a rights entry in the role yet."})),
                                ("rights",json!({"type":"object","minProperties":1,"additionalProperties":{"type":"boolean"},
                                    "description":"Initial rights by platform name, e.g. Read, View, Use; values different from the role default for new objects are stored."})),
                            ], &["object","rights"])}),
                        ),
                    ],
                    &["items"],
                ),
                "The existing role itself: <source-set>:Role.<Name>. Read its Right branch before choosing objects without an entry.",
                "Create rights entries for objects the role does not list yet, each with its initial rights. An object that already has an entry is refused; change it with right.set at Role.<Name>.Right.<Kind>_<Name>.",
                "An entry whose rights all equal the role default would stay empty and is refused. Restrict records with right.set on the new entry in the same batch. A nested object (command, attribute) is one entry whose further dots are escaped: Role.<Name>.Right.<Kind>_<Name>\\.Command\\.<C>.",
                json!({"items":[{"object":"Catalog.Sample","rights":{"Read":true,"View":true}}]}),
            ),
            "right.set" => {
                let mut values = object(
                    &[
                        ("right", text(true)),
                        ("value", json!({"type":"boolean"})),
                        ("rls", text(true)),
                    ],
                    &["right"],
                );
                values["anyOf"] = json!([{"required":["value"]},{"required":["rls"]}]);
                (
                    object(&[("at", text(true)), ("values", values)], &["values"]),
                    "An existing rights entry by its canonical name: <source-set>:Role.<Name>.Right.<Kind>_<Name>. The address names the object; a missing entry is refused, create it with right.add on the role.",
                    "Set one right of the addressed entry and/or its record restriction. A value equal to the role default removes the right with its restriction; an emptied entry is removed.",
                    "rls alone also grants the right. Restriction templates are preserved but not created.",
                    json!({"values":{"right":"Read","value":true}}),
                )
            }
            _ => return None,
        };
        Some(Self {
            schema,
            target,
            effect,
            notes,
            example_args,
            help: ROLE_HELP,
        })
    }

    /// Контракт команды формы публикует ровно те ключи, которые писатель
    /// кладёт в `<Command>` (#1439): ключ, который он не пишет, отказывает
    /// здесь, а не теряется молча.
    pub(crate) fn form(op: &str) -> Option<Self> {
        let (schema, target, effect, notes, example_args) = match op {
            "formCommand.add" => (
                object(
                    &[
                        ("at", text(true)),
                        (
                            "items",
                            json!({"type":"array","minItems":1,"items":object(&[
                                ("name",json!({"type":"string","pattern":r"\S",
                                    "description":"New command name, unused in the form."})),
                                ("title",json!({"type":"string","pattern":r"\S",
                                    "description":"Title in the configuration language."})),
                                ("tooltip",json!({"type":"string","pattern":r"\S",
                                    "description":"Tooltip in the configuration language."})),
                                ("action",json!({"type":"string","pattern":r"\S",
                                    "description":"Handler procedure in the form module; defaults to the command name."})),
                                ("shortcut",json!({"type":"string","pattern":r"\S",
                                    "description":"Keyboard shortcut as the platform writes it, e.g. F5."})),
                                ("picture",json!({"type":"string","pattern":r"^(StdPicture|CommonPicture)\.[^.\s]+$",
                                    "description":"Picture reference StdPicture.<Name> or CommonPicture.<Name>; written with LoadTransparent=true."})),
                                ("representation",json!({"type":"string","enum":["Auto","Text","Picture","TextPicture"],
                                    "description":"Command DefaultRepresentation; the button set (PictureAndText, None, Compact) is not valid for a command."})),
                            ], &["name"])}),
                        ),
                    ],
                    &["items"],
                ),
                "The form itself: <source-set>:<Kind>.<Name>.Form.<Form>. Read its Command branch before choosing names.",
                "Add form commands with the listed properties, written in the platform's FormCommand order. Place a button bound to the command with element.add.",
                "A property outside this schema is refused before any write; use, functionalOptions, currentRowUse, modifiesSavedData and the associated table are not written by this operation.",
                json!({"items":[{"name":"Refresh","title":"Refresh","tooltip":"Refresh the list","shortcut":"F5","picture":"StdPicture.Refresh","representation":"TextPicture"}]}),
            ),
            _ => return None,
        };
        Some(Self {
            schema,
            target,
            effect,
            notes,
            example_args,
            help: FORM_HELP,
        })
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
        if value.as_i64().is_none_or(|number| number < minimum) {
            return Err(error(path, "integer is below the supported minimum"));
        }
    }
    if schema["format"] == "unica-mxl-cell-address"
        && value
            .as_str()
            .is_none_or(|key| parse_mxl_cell_address(key).is_none())
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
        for name in ["mxl.set", "area.add", "template.add"] {
            let descriptor = registry
                .descriptors()
                .iter()
                .find(|d| d.name() == name)
                .unwrap();
            let contract = descriptor.argument_contract().expect("shared contract");
            assert!(contract.normalize(&contract.example_args, "args").is_ok());
        }
    }

    /// Контракт прав публикует ту же форму, что принимает планировщик
    /// (#1354): объект — только `Kind.Name`, право — значение или
    /// ограничение, объект записи прав не аргумент `right.set`.
    #[test]
    fn role_contracts_publish_the_entry_shape() {
        let registry = OperationRegistry::closed();
        for op in ["right.add", "right.set"] {
            let contract = registry.lookup(op).unwrap().argument_contract().unwrap();
            assert!(
                contract.normalize(&contract.example_args, "args").is_ok(),
                "{op}"
            );
        }
        assert!(registry
            .lookup("role.create")
            .unwrap()
            .argument_contract()
            .is_none());
        let add = OperationContract::role("right.add").unwrap();
        for (args, path) in [
            (json!({"items":[]}), "args.items"),
            (
                json!({"items":[{"object":"Catalog.A","rights":{}}]}),
                "args.items[0].rights",
            ),
            (
                json!({"items":[{"object":"Catalog.A","rights":{"Read":"yes"}}]}),
                "args.items[0].rights.Read",
            ),
            (
                json!({"items":[{"object":"DataProcessor.A.Command","rights":{"Use":true}}]}),
                "args.items[0].object",
            ),
            (
                json!({"items":[{"object":"Catalog.A","rights":{"Read":true},"rls":"x"}]}),
                "args.items[0].rls",
            ),
        ] {
            let error = add.normalize(&args, "args").unwrap_err();
            assert_eq!(error.path, path, "{args}");
        }
        // Вложенный объект — одна запись роли (#1356).
        assert!(add
            .normalize(
                &json!({"items":[{"object":"DataProcessor.A.Command.B","rights":{"Use":true}}]}),
                "args"
            )
            .is_ok());
        let set = OperationContract::role("right.set").unwrap();
        assert!(set
            .normalize(
                &json!({"values":{"right":"Read","rls":"ГДЕ ИСТИНА"}}),
                "args"
            )
            .is_ok());
        for args in [
            json!({"values":{"right":"Read"}}),
            json!({"values":{"object":"Catalog.A","right":"Read","value":true}}),
            json!({"values":{"right":"Read","value":"true"}}),
        ] {
            assert!(set.normalize(&args, "args").is_err(), "{args}");
        }
    }

    #[test]
    fn mxl_contract_checks_dynamic_keys_scalars_and_exact_integer_values() {
        let contract = OperationContract::mxl("mxl.set").unwrap();
        for key in ["R1C1", "R01C01", "R+1C+1", "R10000C1000"] {
            assert!(
                contract
                    .normalize(
                        &json!({"values":{"cells":{key:null},"columns":1001}}),
                        "args"
                    )
                    .is_ok(),
                "{key}"
            );
        }
        for values in [
            json!({"cells":[]}),
            json!({"cells":{}}),
            json!({"cells":{"R0C1":"x"}}),
            json!({"cells":{"R10001C1":"x"}}),
            json!({"cells":{"R1C1001":"x"}}),
            json!({"cells":{"R1C1":{}}}),
            json!({"cells":{"R1C1":"x"},"columns":0}),
            json!({"cells":{"R1C1":"x"},"columns":1.0}),
            serde_json::from_str(r#"{"cells":{"R1C1":"x"},"columns":1e0}"#).unwrap(),
            json!({"cells":{"R1C1":"x"},"columns":u64::MAX}),
            json!({"cells":{"R1C1":"x"},"style":"bold"}),
        ] {
            assert!(
                contract
                    .normalize(&json!({"values":values}), "args")
                    .is_err(),
                "{values}"
            );
        }
        assert!(contract
            .normalize(
                &json!({"values":{"cells":{"R1C1":"x","R1C2":42,"R1C3":true,"R1C4":null}}}),
                "args"
            )
            .is_ok());
        assert!(contract
            .normalize(
                &json!({"values":{"cells":{"R1C1":"x"},"columns":i64::MAX}}),
                "args"
            )
            .is_ok());
        // Область называет адрес (#1354): ключ, который её называл, снят.
        let error = contract
            .normalize(&json!({"values":{"area":"A","cells":{"R1C1":"x"}}}), "args")
            .unwrap_err();
        assert_eq!(error.path, "args.values.area");
        // `area.add` называет новую область и её начальные ячейки.
        let area = OperationContract::mxl("area.add").unwrap();
        assert!(area
            .normalize(
                &json!({"items":[{"name":"A","cells":{"R1C1":"x"},"columns":2}]}),
                "args"
            )
            .is_ok());
        for args in [
            json!({"items":[]}),
            json!({"items":[{"name":"A"}]}),
            json!({"items":[{"cells":{"R1C1":"x"}}]}),
            json!({"items":[{"name":"A","cells":{"R0C1":"x"}}]}),
        ] {
            assert!(area.normalize(&args, "args").is_err(), "{args}");
        }
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
        // Набор и вариант называет адрес: ключи, которые его дублировали,
        // контракт не принимает (П2, #1353).
        for (op, values, key) in [
            (
                "query.set",
                json!({"dataSet":"D","query":"ВЫБРАТЬ 1"}),
                "dataSet",
            ),
            ("query.patch", json!({"dataSet":"D","find":"1"}), "dataSet"),
            ("structure.set", json!({"variant":"V"}), "variant"),
            (
                "structure.patch",
                json!({"variant":"V","name":"G","groupBy":["A"]}),
                "variant",
            ),
            // Подузел называет адрес (П3, #1354).
            ("variant.set", json!({"name":"V","title":"T"}), "name"),
            ("parameter.set", json!({"name":"P","title":"T"}), "name"),
            (
                "parameter.rename",
                json!({"name":"P","newName":"Q"}),
                "name",
            ),
            ("field.set", json!({"field":"F","title":"T"}), "field"),
            ("field.set", json!({"dataPath":"F","title":"T"}), "dataPath"),
            (
                "fieldRole.set",
                json!({"field":"F","role":{"dimension":true}}),
                "field",
            ),
        ] {
            let error = OperationContract::dcs(op)
                .unwrap()
                .normalize(&json!({ "values": values }), "args")
                .unwrap_err();
            assert_eq!(error.path, format!("args.values.{key}"), "{op}");
            assert_eq!(error.message, "unknown argument", "{op}");
        }
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
                .normalize(&json!({"values":{"role":value}}), "args")
                .is_ok());
        }
        assert!(role
            .normalize(&json!({"values":{"role":"dimension"}}), "args")
            .is_err());
        // Правка без свойств ничего не меняет и отвергается.
        for op in ["field.set", "parameter.set"] {
            assert!(
                OperationContract::dcs(op)
                    .unwrap()
                    .normalize(&json!({"values":{}}), "args")
                    .is_err(),
                "{op}"
            );
        }
        assert!(OperationContract::dcs("variant.remove")
            .unwrap()
            .normalize(&json!({"values":{"name":"V"}}), "args")
            .is_err());
    }
}
