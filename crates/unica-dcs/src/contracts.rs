use crate::arguments::*;
use crate::operations::Primitive;
use serde_json::{json, Value};
use unica_operation_contract::{object, text, OperationContract};

pub const DCS_ROLE_FLAGS: &[&str] = &[
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

fn index() -> Value {
    json!({"type":"integer", "minimum":0, "maximum":i64::MAX})
}

fn field_head(names: &[&str], required: &[&str]) -> Value {
    let mut fields: Vec<_> = names.iter().map(|name| (*name, text(true))).collect();
    fields.extend([("type", text(false)), ("title", text(false))]);
    aliases(object(&fields, required), names)
}

fn value_modifier(mut schema: Value, payloads: &[&str]) -> Value {
    schema["not"] = json!({"required":["valueType"], "not":{"anyOf":payloads.iter().map(|key| json!({"required":[key]})).collect::<Vec<_>>()}});
    schema
}

fn order_shape(mut schema: Value) -> Value {
    schema["properties"]["direction"]
        .as_object_mut()
        .unwrap()
        .remove("default");
    let mut field = schema.clone();
    field["properties"]["kind"] = json!({"type":"string", "enum":[OrderKind::Field.as_str()], "default":OrderKind::Field.as_str()});
    field["properties"]["direction"]["default"] = json!(OrderDirection::Asc.as_str());
    let mut auto = schema.clone();
    auto["properties"]["kind"] = json!({"type":"string", "enum":[OrderKind::Auto.as_str()]});
    auto["properties"]
        .as_object_mut()
        .unwrap()
        .remove("direction");
    schema["x-unica-discriminator"] = json!("kind");
    schema["anyOf"] = json!([field, auto]);
    schema
}

fn canonicalize_indexes(schema: &Value, value: &mut Value) {
    if schema["type"] == "integer" {
        let integer = value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
            .or_else(|| {
                let number = value.as_f64()?;
                if number.is_finite()
                    && number.fract() == 0.0
                    && (0.0..9_223_372_036_854_775_808.0).contains(&number)
                {
                    Some(number as i64)
                } else {
                    None
                }
            });
        if let Some(integer) = integer {
            *value = json!(integer);
        }
    }
    if let Some(object) = value.as_object_mut() {
        // Mutate values in place: insertion order determines existing refusal priority.
        for (key, value) in object {
            if let Some(child) = schema["properties"].get(key) {
                canonicalize_indexes(child, value);
            }
        }
    }
    if let (Some(item), Some(values)) = (schema.get("items"), value.as_array_mut()) {
        for value in values {
            canonicalize_indexes(item, value);
        }
    }
}

fn selected_defaults(schema: &Value, value: &mut Value) {
    if let Some(key) = schema["x-unica-discriminator"].as_str() {
        if let Some(branch) = schema["anyOf"].as_array().and_then(|branches| {
            branches.iter().find(|branch| {
                branch["properties"][key]["enum"]
                    .as_array()
                    .is_some_and(|choices| choices.contains(&value[key]))
            })
        }) {
            unica_operation_contract::apply_defaults(branch, value);
        }
    }
    if let Some(object) = value.as_object_mut() {
        for (key, value) in object {
            if let Some(child) = schema["properties"].get(key) {
                selected_defaults(child, value);
            }
        }
    }
    if let (Some(item), Some(values)) = (schema.get("items"), value.as_array_mut()) {
        for value in values {
            selected_defaults(item, value);
        }
    }
}

pub trait DcsContractExt {
    fn normalize_dcs(
        &self,
        args: &Value,
        path: &str,
    ) -> Result<Value, unica_operation_contract::ContractError>;
}

impl DcsContractExt for OperationContract {
    fn normalize_dcs(
        &self,
        args: &Value,
        path: &str,
    ) -> Result<Value, unica_operation_contract::ContractError> {
        let mut canonical = args.clone();
        canonicalize_indexes(&self.schema, &mut canonical);
        let mut normalized = self.normalize(&canonical, path)?;
        selected_defaults(&self.schema, &mut normalized);
        Ok(normalized)
    }
}

pub fn dcs(op: Primitive) -> OperationContract {
    let schema_target = "The DCS Template itself. Read the template props and its DataSet/Setting branches before choosing selectors.";
    let data_target =
        "Existing DCS Template.DataSet.<name>; repeat DataSet.<name> for a nested Union member.";
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
    let scalar =
        json!({"anyOf":[{"type":"string"},{"type":"number"},{"type":"boolean"},{"type":"null"}]});
    let localized_value =
        json!({"type":"object","minProperties":1,"additionalProperties":{"type":"string"}});
    let typed_value = json!({"anyOf":[scalar.clone(),localized_value.clone(),object(&[("variant",text(true)),("startDate",text(true)),("endDate",text(true))],&["variant"])]});
    let appearance_value = json!({"anyOf":[typed_value.clone(),object(&[("valueType",text(true)),("value",typed_value.clone())],&["valueType","value"])]});
    let strings = items(text(true));
    let comparison = json!({"type":"string","default":FilterComparison::Equal.as_str(),"enum":FilterComparison::VALUES});
    let filter = || {
        value_modifier(
            aliases(
                object(
                    &[
                        ("field", text(true)),
                        ("dataPath", text(true)),
                        (
                            "comparison",
                            if op == Primitive::FilterAdd {
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
                        ("viewMode", json!({"type":"string","enum":ViewMode::VALUES})),
                        ("userSettingID", text(true)),
                        ("userSettingPresentation", text(false)),
                    ],
                    &[],
                ),
                &["field", "dataPath"],
            ),
            &["value"],
        )
    };
    let structure = || {
        let mut schema = object(
            &[
                ("name", text(true)),
                (
                    "kind",
                    json!({"type":"string","enum":StructureKind::VALUES,"default":StructureKind::Group.as_str()}),
                ),
                ("groupBy", strings.clone()),
                (
                    "axis",
                    json!({"type":"string","enum":StructureAxis::VALUES}),
                ),
                ("viewMode", json!({"type":"string","enum":ViewMode::VALUES})),
            ],
            &["name"],
        );
        schema["not"] = json!({"required":["kind","groupBy"], "properties":{"kind":{"enum":[StructureKind::Table.as_str(),StructureKind::Chart.as_str()]}}});
        schema
    };
    let restriction = object(
        &["field", "condition", "group", "order"]
            .iter()
            .map(|key| (*key, json!({"type":"boolean"})))
            .collect::<Vec<_>>(),
        &[],
    );
    let parameter = || {
        value_modifier(
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
                    ("use", json!({"type":"string","enum":ParameterUse::VALUES})),
                ],
                &["name"],
            ),
            &["value", "values", "availableValues"],
        )
    };
    let (key, inner, target, effect, example_args) = match op {
        Primitive::DataSourceAdd => {
            let kind = json!({"type":"string","enum":DataSourceKind::VALUES,"default":DataSourceKind::Local.as_str()});
            ("items",object(&[("name",text(true)),("kind",kind)],&["name"]),schema_target,"Add a new named source; an existing name is refused. Change it later at Template.DataSource.<name>.",json!({"items":[{"name":"Database"}]}))
        }
        Primitive::DataSourceSet => ("values",object(&[("kind",json!({"type":"string","enum":DataSourceKind::VALUES}))],&["kind"]),data_source_target,"Change only the kind of the addressed source.",json!({"values":{"kind":"Local"}})),
        Primitive::DataSourceRemove => ("",json!({}),data_source_target,"Remove the addressed source. References are checked against the final batch image.",json!({})),
        Primitive::DataSetAdd => {
            let inner=object(&[("name",text(true)),("kind",json!({"type":"string","enum":DataSetKind::VALUES,"default":DataSetKind::Query.as_str()})),("dataSource",text(true)),("query",text(true)),("objectName",text(true))],&["name"]);
            ("items",inner,"DCS Template adds a top-level dataset; an existing Union DataSet adds one member.","Create one Query/Object/Union dataset. Query requires dataSource and query; Object requires dataSource and objectName; Union members are separate dataSet.add operations. Add fields separately.",json!({"items":[{"name":"Data","kind":"Query","dataSource":"ИсточникДанных1","query":"ВЫБРАТЬ 1 КАК Amount"}]}))
        }
        Primitive::DataSetSet => ("values",object(&[("dataSource",text(true)),("objectName",text(true)),("autoFillFields",json!({"type":"boolean"}))],&[]),data_target,"Update only supplied dataset properties; changing its kind is refused.",json!({"values":{"autoFillFields":false}})),
        Primitive::DataSetRemove => ("",json!({}),data_target,"Remove the addressed dataset. No dependent fields, links or variant settings are implicitly rewritten.",json!({})),
        Primitive::FieldAdd => {
            let mut inner=field_head(&["dataPath","name"],&[]);inner["properties"]["title"]=title.clone();inner["properties"]["field"]=text(true);
            for key in ["useRestriction","attributeUseRestriction"]{inner["properties"][key]=restriction.clone();}
            inner["properties"]["presentationExpression"]=text(false);
            ("items",inner,data_target,"Add dataset fields. Existing dataPath is unchanged. Query fields derive their type from the query and refuse type; describe Object field types explicitly. Selection is a separate operation.",json!({"items":[{"dataPath":"Added","title":"Added [шт]"}]}))
        }
        Primitive::FieldSet => {
            let mut inner=object(&[("type",text(false)),("title",title.clone()),("useRestriction",restriction.clone()),("attributeUseRestriction",restriction.clone()),("presentationExpression",text(false)),("sourceField",text(true))],&[]);
            inner["minProperties"]=json!(1);
            ("values",inner,field_target,"Update only supplied field mapping/title/type/restrictions/presentation expression. Query fields derive their type from query and refuse type. Other field patches remove stale valueType on the addressed Query field. Other field data remains.",json!({"values":{"title":{"ru":"Сумма","en":"Amount"}}}))
        }
        Primitive::FieldRemove | Primitive::ParameterRemove => ("",json!({}),if op==Primitive::FieldRemove{field_target}else{parameter_target},"Remove only the named terminal Field or Parameter. References and selections are separate operations.",json!({})),
        Primitive::FieldRoleSet => {
            let flags=object(&DCS_ROLE_KEYS.iter().map(|key|(*key,if DCS_ROLE_FLAGS.contains(key){json!({"type":"boolean"})}else{text(true)})).collect::<Vec<_>>(),&[]);
            ("values",object(&[("role",flags)],&["role"]),field_target,"Replace the addressed field's role with the supplied object. Values are validated against platform types; omitted role entries are removed.",json!({"values":{"role":{"dimension":true}}}))
        }
        Primitive::ParameterAdd => ("items",parameter(),schema_target,"Add a new schema parameter. Parameters for dates and their expressions are explicit separate operations.",json!({"items":[{"name":"Period","type":"date"}]})),
        Primitive::ParameterSet => {
            let mut inner=parameter();
            inner["properties"].as_object_mut().expect("parameter properties").remove("name");
            inner["required"]=json!([]);
            inner["minProperties"]=json!(1);
            ("values",inner,parameter_target,"Patch only supplied properties of the addressed schema parameter.",json!({"values":{"title":"Period"}}))
        }
        Primitive::ParameterRename => ("values",object(&[("newName",text(true))],&["newName"]),parameter_target,"Rename the addressed parameter declaration only. References are explicit query/expression operations in the same batch.",json!({"values":{"newName":"ReportPeriod"}})),
        Primitive::ParameterReorder => ("items",object(&[("name",text(true))],&["name"]),schema_target,"Reorder all schema parameter elements by the supplied complete unique list. Comments and all other nodes stay in their original slots.",json!({"items":[{"name":"Period"}]})),
        Primitive::CalculatedFieldAdd => {
            let mut inner=aliases(object(&[("name",text(true)),("dataPath",text(true)),("expression",text(true)),("title",title.clone()),("type",text(false))],&["expression"]),&["name","dataPath"]);
            inner["properties"]["useRestriction"]=restriction.clone();
            ("items",inner,schema_target,"Add a calculated field; selection is independent. Change it later at Template.CalculatedField.<dataPath>.",json!({"items":[{"name":"DoubleAmount","expression":"Amount * 2"}]}))
        }
        Primitive::TotalAdd => ("items",aliases(object(&[("field",text(true)),("dataPath",text(true)),("expression",text(false)),("group",json!({"anyOf":[text(true),strings.clone()]}))],&[]),&["field","dataPath"]),schema_target,"Add a total with explicit expression/group. Omitted expression is Сумма(field). Change it later at Template.Total.<dataPath>.",json!({"items":[{"field":"Amount","expression":"Сумма(Amount)"}]})),
        Primitive::CalculatedFieldRemove => ("",json!({}),calculated_target,"Remove the addressed calculated field only; references to it are separate operations.",json!({})),
        Primitive::TotalRemove => ("",json!({}),total_target,"Remove the addressed total only.",json!({})),
        Primitive::VariantAdd => ("items",optional_aliases(object(&[("name",text(true)),("title",title.clone()),("presentation",title.clone())],&["name"]),&["title","presentation"]),schema_target,"Create an empty settings variant. Configure selection and structure separately.",json!({"items":[{"name":"Additional","title":"Additional"}]})),
        Primitive::VariantSet => ("values",optional_aliases(object(&[("title",title.clone()),("presentation",title.clone())],&[]),&["title","presentation"]),setting_target,"Change the presentation of the addressed settings variant. A missing variant is refused; create it with variant.add on the template.",json!({"values":{"title":"Report"}})),
        Primitive::VariantRemove => ("",json!({}),setting_target,"Remove the addressed variant.",json!({})),
        Primitive::QuerySet => ("values",aliases(object(&[("query",text(true)),("text",text(true))],&[]),&["query","text"]),query_target,"Replace query text. Supply literal text, not @file.",json!({"values":{"query":"ВЫБРАТЬ 1 КАК Amount"}})),
        Primitive::QueryPatch => ("values",object(&[("find",text(true)),("replace",json!({"type":"string","default":""})),("once",json!({"type":"boolean","default":false}))],&["find"]),query_target,"Patch decoded query text. No matches fail; once=true requires exactly one. Comments/PI are preserved; a marker within replaced text anchors to replacement start.",json!({"values":{"find":"1 КАК Amount","replace":"2 КАК Amount","once":true}})),
        Primitive::FilterAdd | Primitive::FilterSet => (if matches!(op, Primitive::FilterAdd){"items"}else{"values"},filter(),container_target,"Add a filter or patch one uniquely selected by field in the addressed settings container. Literal strings retain @, brackets and comparison words. Set requires a unique match.",if matches!(op, Primitive::FilterAdd){json!({"items":[{"field":"Amount","comparison":"Greater","value":0}]})}else{json!({"values":{"field":"Amount","comparison":"Greater","value":1}})}),
        Primitive::FilterRemove => ("values",object(&[("field",text(true))],&["field"]),container_target,"Remove one uniquely selected filter; multiple matches fail.",json!({"values":{"field":"Amount"}})),
        Primitive::SelectionAdd | Primitive::OrderAdd => {
            let mut inner=optional_aliases(object(&[("field",text(true)),("dataPath",text(true)),("kind",json!({"type":"string","enum":OrderKind::VALUES,"default":OrderKind::Field.as_str()}))],&[]),&["field","dataPath"]);
            if op==Primitive::OrderAdd {inner["properties"]["direction"]=json!({"type":"string","enum":OrderDirection::VALUES,"default":OrderDirection::Asc.as_str()});}
            ("items",inner,container_target,"Add one explicit Field or Auto selection/order item. Field requires field; Auto has no field. Only Field order accepts direction. Use index selectors for anonymous Auto items.",json!({"items":[{"field":"Amount"}]}))
        }
        Primitive::FilterClear | Primitive::SelectionClear | Primitive::OrderClear | Primitive::ConditionalAppearanceClear => ("values",object(&[],&[]),container_target,"Clear the selected collection's elements. The collection's attributes/comments stay; an absent collection is unchanged.",json!({"values":{}})),
        Primitive::DataParameterAdd | Primitive::DataParameterSet | Primitive::OutputParameterSet => (if op==Primitive::DataParameterAdd{"items"}else{"values"},object(&[("name",text(true)),("value",typed_value.clone()),("valueType",text(true)),("use",json!({"type":"boolean"})),("viewMode",json!({"type":"string","enum":ViewMode::VALUES})),("userSettingID",text(true)),("userSettingPresentation",title.clone())],if op==Primitive::DataParameterAdd{&["name","value"][..]}else{&["name"][..]}),if op==Primitive::OutputParameterSet{container_target}else{setting_target},"Set only supplied variant data/output parameter properties. Typed defaults, NULL, periods, references and localized values preserve their platform type; schema parameter declarations remain separate.",if op==Primitive::DataParameterAdd{json!({"items":[{"name":"Period","value":"2026-01-01T00:00:00","valueType":"xs:dateTime"}]})}else{json!({"values":{"name":"Period","value":"2026-01-01T00:00:00","valueType":"xs:dateTime"}})}),
        Primitive::ConditionalAppearanceAdd => {
            let mut condition = filter();
            condition["properties"]["comparison"]["default"] = json!("Equal");
            ("items",object(&[("fields",strings.clone()),("filter",json!({"type":"array","items":condition})),("use",json!({"type":"boolean"})),("appearance",json!({"type":"object","minProperties":1,"additionalProperties":appearance_value.clone()}))],&["fields","appearance"]),container_target,"Add conditional appearance with explicit use, comparisons and typed value atoms. Formatting does not change field values.",json!({"items":[{"fields":["Amount"],"use":false,"appearance":{"Формат":{"valueType":"v8:LocalStringType","value":{"ru":"ЧДЦ=2"}}}}]}))
        }
        Primitive::StructureAdd => ("items",structure(),container_target,"Add one named group/table/chart into the addressed variant or structure item; axis selects the row/column/point/series of an addressed table/chart. The name must be new in the variant. groupBy belongs to group; table/chart grouping is added through its axes. Add each child with a separate operation at its parent's address.",json!({"items":[{"name":"ByAmount","kind":"group","groupBy":["Amount"]}]})),
        Primitive::StructureSet => ("values",object(&[("name",text(true)),("groupBy",json!({"type":"array","items":text(true),"default":[]})),("details",json!({"type":"boolean","default":true}))],&[]),setting_target,"Replace the variant structure with one group and optional nested details. groupBy=[] is a details group; use structure.add for tables/charts and nested composition.",json!({"values":{"name":"ByAmount","groupBy":["Amount"],"details":true}})),
        Primitive::StructurePatch => ("values",object(&[("groupBy",strings.clone())],&[]),structure_target,"Patch groupBy and supplied view properties of the addressed structure item. Other structure and settings are preserved.",json!({"values":{"groupBy":["Amount"]}})),
        Primitive::StructureRemove => ("",json!({}),structure_target,"Remove the addressed structure item and its children.",json!({})),
        Primitive::CalculatedFieldSet => {
            let mut inner=object(&[("expression",text(true)),("title",title.clone()),("type",text(true)),("useRestriction",restriction.clone())],&[]);
            inner["minProperties"]=json!(1);
            ("values",inner,calculated_target,"Patch only supplied properties of the addressed calculated field.",json!({"values":{"expression":"Amount * 3"}}))
        }
        Primitive::TotalSet => {
            let mut inner=object(&[("expression",text(true)),("group",json!({"anyOf":[text(true),strings.clone()]}))],&[]);
            inner["minProperties"]=json!(1);
            ("values",inner,total_target,"Patch supplied expression or grouping list of the addressed total.",json!({"values":{"expression":"Максимум(Amount)"}}))
        }
        Primitive::SelectionSet | Primitive::OrderSet => {
            let mut inner=object(&[("field",text(true)),("index",index()),("use",json!({"type":"boolean"})),("viewMode",json!({"type":"string","enum":ViewMode::VALUES})),("userSettingID",text(true)),("userSettingPresentation",text(false))],&[]);
            if op==Primitive::OrderSet{inner["properties"]["direction"]=json!({"type":"string","enum":OrderDirection::VALUES});}
            ("values",inner,container_target,"Patch one uniquely selected field item, preserving unspecified settings.",json!({"values":{"field":"Amount","use":false}}))
        }
        Primitive::SelectionRemove | Primitive::OrderRemove => ("values",object(&[("field",text(true)),("index",index())],&[]),container_target,"Remove one uniquely selected field item.",json!({"values":{"field":"Amount"}})),
        Primitive::DataParameterRemove | Primitive::OutputParameterRemove => ("values",object(&[("name",text(true))],&["name"]),if op==Primitive::OutputParameterRemove{container_target}else{setting_target},"Remove the uniquely named settings parameter.",json!({"values":{"name":"Period"}})),
        Primitive::ConditionalAppearanceRemove => ("values",object(&[("index",index())],&["index"]),container_target,"Remove the zero-based appearance item from the captured settings image; read props before selecting.",json!({"values":{"index":0}})),
        Primitive::DataSetLinkSet => ("values",object(&[("selector",object(&[("source",text(true)),("destination",text(true)),("sourceExpression",text(true)),("destinationExpression",text(true))],&["source","destination","sourceExpression","destinationExpression"])),("parameter",text(true)),("parameterListAllowed",json!({"type":"boolean"})),("required",json!({"type":"boolean"})),("condition",text(false)),("startExpression",text(false))],&["selector"]),schema_target,"Select one unique dataset link by its endpoints and expressions. Patch only supplied properties.",json!({"values":{"selector":{"source":"Left","destination":"Right","sourceExpression":"Amount","destinationExpression":"Amount"},"condition":"Amount > 0"}})),
        Primitive::DataSetLinkRemove => ("values",object(&[("selector",object(&[("source",text(true)),("destination",text(true)),("sourceExpression",text(true)),("destinationExpression",text(true))],&["source","destination","sourceExpression","destinationExpression"]))],&["selector"]),schema_target,"Remove one unique dataset link selected by its endpoints and expressions. Patch properties are refused.",json!({"values":{"selector":{"source":"Left","destination":"Right","sourceExpression":"Amount","destinationExpression":"Amount"}}})),
        Primitive::DataSetLinkAdd => ("items",object(&[("source",text(true)),("destination",text(true)),("sourceExpression",text(true)),("destinationExpression",text(true)),("parameter",text(true)),("parameterListAllowed",json!({"type":"boolean"})),("required",json!({"type":"boolean"})),("condition",text(false)),("startExpression",text(false))],&["source","destination","sourceExpression","destinationExpression"]),schema_target,"Add one dataset link between existing top-level datasets.",json!({"items":[{"source":"Left","destination":"Right","sourceExpression":"Amount","destinationExpression":"Amount"}]})),

    };
    let mut inner = inner;
    if matches!(
        op,
        Primitive::DataParameterAdd | Primitive::DataParameterSet | Primitive::OutputParameterSet
    ) {
        inner = value_modifier(inner, &["value"]);
    }
    if op == Primitive::OrderAdd {
        inner = order_shape(inner);
    }
    if matches!(
        op,
        Primitive::SelectionAdd | Primitive::SelectionSet | Primitive::SelectionRemove
    ) {
        inner["properties"]["parentIndexes"] = items(index());
        if op == Primitive::SelectionAdd {
            inner["properties"]["kind"]["enum"] = json!(SelectionKind::VALUES);
            inner["properties"]["kind"]["default"] = json!(SelectionKind::Field.as_str());
            inner["properties"]["title"] = title.clone();
        }
    }
    if op == Primitive::VariantSet {
        inner["anyOf"] = json!([{"required":["title"]},{"required":["presentation"]}]);
    }
    if matches!(
        op,
        Primitive::StructureAdd | Primitive::StructurePatch | Primitive::StructureSet
    ) {
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
            props.insert(key.into(), json!({"type":"string","enum":ViewMode::VALUES}));
        }
        props.insert("userSettingID".into(), text(true));
        props.insert("userSettingPresentation".into(), title.clone());
        if op == Primitive::StructurePatch {
            inner["minProperties"] = json!(1);
        }
    }
    let inner = if key == "items" { items(inner) } else { inner };
    let mut fields = vec![("at", text(true))];
    let required = if key.is_empty() {
        vec![]
    } else {
        fields.push((key, inner));
        if matches!(
            op,
            Primitive::FilterClear
                | Primitive::SelectionClear
                | Primitive::OrderClear
                | Primitive::ConditionalAppearanceClear
        ) {
            vec![]
        } else {
            vec![key]
        }
    };
    OperationContract{schema:object(&fields,&required),target,effect,
        notes:"Operands are structured values. Where accepted, valueType requires its value or value list, including an empty list. There is no shorthand or whole-schema JSON input. Unspecified properties and unrelated XML bytes are preserved; create template.add first, then compose operations. Preview makes no writes; execute its executionToken, read the post-image and check the template.",example_args,help:REPORTS_HELP,
        format_validators: &[]}
}

const REPORTS_HELP: &str = "references/use-cases/reports-printing.md";
