//! DCS operands keep their subject meaning after the public argument boundary.

use super::operation_contract::{ArgumentShape, ContractError, OperationContract};
use serde_json::{Map, Value};
use std::collections::HashSet;

macro_rules! vocabulary {
    ($name:ident { $($variant:ident => $wire:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum $name { $($variant),+ }
        impl $name {
            pub(crate) const VALUES: &'static [&'static str] = &[$($wire),+];
            pub(crate) const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $wire),+ }
            }
            fn parse(value: &str) -> Option<Self> {
                match value { $($wire => Some(Self::$variant),)+ _ => None }
            }
        }
    };
}

vocabulary!(DataSourceKind { Local => "Local", External => "External" });
vocabulary!(DataSetKind { Query => "Query", Object => "Object", Union => "Union" });
vocabulary!(SelectionKind { Field => "Field", Auto => "Auto", Folder => "Folder" });
vocabulary!(OrderKind { Field => "Field", Auto => "Auto" });
vocabulary!(OrderDirection { Asc => "Asc", Desc => "Desc" });
vocabulary!(ViewMode { Normal => "Normal", Inaccessible => "Inaccessible", QuickAccess => "QuickAccess" });
vocabulary!(StructureKind { Group => "group", Table => "table", Chart => "chart" });
vocabulary!(StructureAxis { Row => "row", Column => "column", Point => "point", Series => "series" });
vocabulary!(ParameterUse { Always => "Always", Auto => "Auto", Never => "Never" });
vocabulary!(FilterComparison {
    Equal => "Equal", NotEqual => "NotEqual", Greater => "Greater",
    GreaterOrEqual => "GreaterOrEqual", Less => "Less", LessOrEqual => "LessOrEqual",
    Contains => "Contains", NotContains => "NotContains", BeginsWith => "BeginsWith",
    NotBeginsWith => "NotBeginsWith", InList => "InList", NotInList => "NotInList",
    InHierarchy => "InHierarchy", InListByHierarchy => "InListByHierarchy",
    Filled => "Filled", NotFilled => "NotFilled"
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DcsIndex(pub(crate) usize);
impl DcsIndex {
    pub(crate) fn get(self) -> usize {
        self.0
    }
}

/// JSON here is one platform value, never an operation or item envelope.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ValueAtom(pub(crate) Value);
impl ValueAtom {
    pub(crate) fn value(&self) -> &Value {
        &self.0
    }
}

/// A plain string or an explicit language-to-text value.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LocalizedText(pub(crate) Value);
impl LocalizedText {
    pub(crate) fn value(&self) -> &Value {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DataSourceSpec {
    pub(crate) name: String,
    pub(crate) kind: DataSourceKind,
}
#[derive(Debug, Clone)]
pub(crate) struct DataSetSpec {
    pub(crate) name: String,
    pub(crate) kind: DataSetKind,
    pub(crate) data_source: Option<String>,
    pub(crate) query: Option<String>,
    pub(crate) object_name: Option<String>,
}
#[derive(Debug, Clone)]
pub(crate) struct DataSetPatch {
    pub(crate) data_source: Option<String>,
    pub(crate) object_name: Option<String>,
    pub(crate) auto_fill_fields: Option<bool>,
}
#[derive(Debug, Clone)]
pub(crate) struct Restriction {
    pub(crate) field: Option<bool>,
    pub(crate) condition: Option<bool>,
    pub(crate) group: Option<bool>,
    pub(crate) order: Option<bool>,
}
#[derive(Debug, Clone)]
pub(crate) struct FieldSpec {
    pub(crate) data_path: String,
    pub(crate) field: Option<String>,
    pub(crate) type_name: Option<String>,
    pub(crate) title: Option<LocalizedText>,
    pub(crate) use_restriction: Option<Restriction>,
    pub(crate) attribute_use_restriction: Option<Restriction>,
    pub(crate) presentation_expression: Option<String>,
}
#[derive(Debug, Clone)]
pub(crate) struct FieldPatch {
    pub(crate) source_field: Option<String>,
    pub(crate) type_name: Option<String>,
    pub(crate) title: Option<LocalizedText>,
    pub(crate) use_restriction: Option<Restriction>,
    pub(crate) attribute_use_restriction: Option<Restriction>,
    pub(crate) presentation_expression: Option<String>,
}
#[derive(Debug, Clone)]
pub(crate) struct FieldRole {
    pub(crate) parent_dimension: Option<String>,
    pub(crate) account_type_expression: Option<String>,
    pub(crate) balance_group_name: Option<String>,
    pub(crate) account_field: Option<String>,
    pub(crate) period_number: Option<String>,
    pub(crate) period_type: Option<String>,
    pub(crate) dimension: Option<bool>,
    pub(crate) account: Option<bool>,
    pub(crate) balance: Option<bool>,
    pub(crate) ignore_null_values: Option<bool>,
    pub(crate) required: Option<bool>,
    pub(crate) dimension_attribute: Option<bool>,
    pub(crate) balance_type: Option<String>,
    pub(crate) accounting_balance_type: Option<String>,
}
#[derive(Debug, Clone)]
pub(crate) struct AvailableValue {
    pub(crate) value: ValueAtom,
    pub(crate) presentation: Option<LocalizedText>,
}
#[derive(Debug, Clone)]
pub(crate) struct ParameterSpec {
    pub(crate) name: String,
    pub(crate) properties: ParameterPatch,
}
#[derive(Debug, Clone)]
pub(crate) struct ParameterPatch {
    pub(crate) type_name: Option<String>,
    pub(crate) title: Option<LocalizedText>,
    pub(crate) value_type: Option<String>,
    pub(crate) value: Option<ValueAtom>,
    pub(crate) values: Option<Vec<ValueAtom>>,
    pub(crate) available_values: Option<Vec<AvailableValue>>,
    pub(crate) expression: Option<String>,
    pub(crate) use_restriction: Option<bool>,
    pub(crate) value_list_allowed: Option<bool>,
    pub(crate) available_as_field: Option<bool>,
    pub(crate) deny_incomplete_values: Option<bool>,
    pub(crate) use_mode: Option<ParameterUse>,
}
#[derive(Debug, Clone)]
pub(crate) struct CalculatedFieldSpec {
    pub(crate) name: String,
    pub(crate) expression: String,
    pub(crate) title: Option<LocalizedText>,
    pub(crate) type_name: Option<String>,
    pub(crate) use_restriction: Option<Restriction>,
}
#[derive(Debug, Clone)]
pub(crate) struct CalculatedFieldPatch {
    pub(crate) expression: Option<String>,
    pub(crate) title: Option<LocalizedText>,
    pub(crate) type_name: Option<String>,
    pub(crate) use_restriction: Option<Restriction>,
}
#[derive(Debug, Clone)]
pub(crate) struct TotalSpec {
    pub(crate) field: String,
    pub(crate) expression: Option<String>,
    pub(crate) groups: Option<Vec<String>>,
}
#[derive(Debug, Clone)]
pub(crate) struct TotalPatch {
    pub(crate) expression: Option<String>,
    pub(crate) groups: Option<Vec<String>>,
}
#[derive(Debug, Clone)]
pub(crate) struct VariantSpec {
    pub(crate) name: String,
    pub(crate) title: LocalizedText,
}
#[derive(Debug, Clone)]
pub(crate) struct QueryPatch {
    pub(crate) find: String,
    pub(crate) replace: String,
    pub(crate) once: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct SettingsMeta<P = LocalizedText> {
    pub(crate) use_flag: Option<bool>,
    pub(crate) view_mode: Option<ViewMode>,
    pub(crate) user_setting_id: Option<String>,
    pub(crate) user_setting_presentation: Option<P>,
}
#[derive(Debug, Clone)]
pub(crate) struct FilterSpec {
    pub(crate) field: String,
    pub(crate) comparison: FilterComparison,
    pub(crate) value: Option<ValueAtom>,
    pub(crate) value_type: Option<String>,
    pub(crate) meta: SettingsMeta<String>,
}
#[derive(Debug, Clone)]
pub(crate) struct FilterPatch {
    pub(crate) field: String,
    pub(crate) comparison: Option<FilterComparison>,
    pub(crate) value: Option<ValueAtom>,
    pub(crate) value_type: Option<String>,
    pub(crate) meta: SettingsMeta<String>,
}
/// Presence is retained until the existing settings lookup and selector check.
#[derive(Debug, Clone)]
pub(crate) struct SelectionKey {
    pub(crate) field: Option<String>,
    pub(crate) index: Option<DcsIndex>,
}
#[derive(Debug, Clone)]
pub(crate) struct SelectionSpec {
    pub(crate) kind: SelectionKind,
    pub(crate) field: Option<String>,
    pub(crate) title: Option<LocalizedText>,
    pub(crate) parent_indexes: Vec<DcsIndex>,
}
#[derive(Debug, Clone)]
pub(crate) struct SelectionPatch {
    pub(crate) selector: SelectionKey,
    pub(crate) parent_indexes: Vec<DcsIndex>,
    pub(crate) meta: SettingsMeta<String>,
}
#[derive(Debug, Clone)]
pub(crate) struct SelectionRemove {
    pub(crate) selector: SelectionKey,
    pub(crate) parent_indexes: Vec<DcsIndex>,
}
#[derive(Debug, Clone)]
pub(crate) struct OrderSpec {
    pub(crate) kind: OrderKind,
    pub(crate) field: Option<String>,
    pub(crate) direction: Option<OrderDirection>,
}
#[derive(Debug, Clone)]
pub(crate) struct OrderPatch {
    pub(crate) selector: SelectionKey,
    pub(crate) direction: Option<OrderDirection>,
    pub(crate) meta: SettingsMeta<String>,
}
#[derive(Debug, Clone)]
pub(crate) struct SettingsParameterSpec {
    pub(crate) name: String,
    pub(crate) value: Option<ValueAtom>,
    pub(crate) value_type: Option<String>,
    pub(crate) meta: SettingsMeta,
}
#[derive(Debug, Clone)]
pub(crate) struct ConditionalAppearanceSpec {
    pub(crate) fields: Vec<String>,
    pub(crate) filters: Option<Vec<FilterSpec>>,
    pub(crate) appearance: Vec<(String, ValueAtom)>,
    pub(crate) use_flag: Option<bool>,
}
#[derive(Debug, Clone)]
pub(crate) struct StructureMeta {
    pub(crate) settings: SettingsMeta,
    pub(crate) columns_view_mode: Option<ViewMode>,
    pub(crate) rows_view_mode: Option<ViewMode>,
    pub(crate) points_view_mode: Option<ViewMode>,
    pub(crate) series_view_mode: Option<ViewMode>,
    pub(crate) items_view_mode: Option<ViewMode>,
}
#[derive(Debug, Clone)]
pub(crate) struct StructureSpec {
    pub(crate) name: Option<String>,
    pub(crate) kind: StructureKind,
    pub(crate) group_by: Option<Vec<String>>,
    pub(crate) axis: Option<StructureAxis>,
    pub(crate) meta: StructureMeta,
}
#[derive(Debug, Clone)]
pub(crate) struct StructureSet {
    pub(crate) item: StructureSpec,
    pub(crate) details: bool,
}
#[derive(Debug, Clone)]
pub(crate) struct StructurePatch {
    pub(crate) group_by: Option<Vec<String>>,
    pub(crate) meta: StructureMeta,
}
#[derive(Debug, Clone)]
pub(crate) struct DataSetLinkSelector {
    pub(crate) source: String,
    pub(crate) destination: String,
    pub(crate) source_expression: String,
    pub(crate) destination_expression: String,
}
#[derive(Debug, Clone)]
pub(crate) struct DataSetLinkPatch {
    pub(crate) parameter: Option<String>,
    pub(crate) parameter_list_allowed: Option<bool>,
    pub(crate) required: Option<bool>,
    pub(crate) condition: Option<String>,
    pub(crate) start_expression: Option<String>,
}
#[derive(Debug, Clone)]
pub(crate) struct DataSetLinkSpec {
    pub(crate) selector: DataSetLinkSelector,
    pub(crate) properties: DataSetLinkPatch,
}

type DecodeResult<T> = Result<T, ContractError>;

fn invalid(path: &str, message: &str) -> ContractError {
    ContractError {
        path: path.into(),
        message: message.into(),
    }
}

struct DcsArgumentReader<'a> {
    object: &'a Map<String, Value>,
    shape: ArgumentShape<'a>,
    path: String,
    consumed: HashSet<String>,
}

impl<'a> DcsArgumentReader<'a> {
    fn new(value: &'a Value, shape: ArgumentShape<'a>, path: &str) -> DecodeResult<Self> {
        shape.validate_type(value, path)?;
        Ok(Self {
            object: value
                .as_object()
                .ok_or_else(|| invalid(path, "expected an argument object"))?,
            shape,
            path: path.into(),
            consumed: HashSet::new(),
        })
    }

    fn read<T>(
        &mut self,
        key: &str,
        convert: impl FnOnce(&'a Value, ArgumentShape<'a>, &str) -> DecodeResult<T>,
    ) -> DecodeResult<Option<T>> {
        let Some(shape) = self.shape.property(key) else {
            return Ok(None);
        };
        self.consumed.insert(key.into());
        let aliases: Vec<_> = self.shape.aliases().collect();
        if aliases.first().copied() == Some(key) {
            for alias in aliases {
                self.consumed.insert(alias.into());
            }
        }
        let path = format!("{}.{key}", self.path);
        self.object
            .get(key)
            .map(|value| {
                shape.validate_field(value, &path)?;
                convert(value, shape, &path)
            })
            .transpose()
    }

    fn required<T>(
        &mut self,
        key: &str,
        convert: impl FnOnce(&'a Value, ArgumentShape<'a>, &str) -> DecodeResult<T>,
    ) -> DecodeResult<T> {
        self.read(key, convert)?.ok_or_else(|| {
            invalid(
                &format!("{}.{key}", self.path),
                "required decoded argument is missing",
            )
        })
    }

    fn string(&mut self, key: &str) -> DecodeResult<Option<String>> {
        self.read(key, |value, _, path| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| invalid(path, "expected text"))
        })
    }
    fn required_string(&mut self, key: &str) -> DecodeResult<String> {
        self.string(key)?.ok_or_else(|| {
            invalid(
                &format!("{}.{key}", self.path),
                "required decoded text is missing",
            )
        })
    }
    fn boolean(&mut self, key: &str) -> DecodeResult<Option<bool>> {
        self.read(key, |value, _, path| {
            value
                .as_bool()
                .ok_or_else(|| invalid(path, "expected boolean"))
        })
    }
    fn required_boolean(&mut self, key: &str) -> DecodeResult<bool> {
        self.boolean(key)?.ok_or_else(|| {
            invalid(
                &format!("{}.{key}", self.path),
                "required decoded boolean is missing",
            )
        })
    }
    fn enumeration<T>(
        &mut self,
        key: &str,
        parse: fn(&str) -> Option<T>,
    ) -> DecodeResult<Option<T>> {
        self.read(key, |value, _, path| {
            value
                .as_str()
                .and_then(parse)
                .ok_or_else(|| invalid(path, "value has no supported domain alternative"))
        })
    }
    fn required_enum<T>(&mut self, key: &str, parse: fn(&str) -> Option<T>) -> DecodeResult<T> {
        self.enumeration(key, parse)?.ok_or_else(|| {
            invalid(
                &format!("{}.{key}", self.path),
                "required decoded alternative is missing",
            )
        })
    }
    fn atom(&mut self, key: &str) -> DecodeResult<Option<ValueAtom>> {
        self.read(key, |value, _, _| Ok(ValueAtom(value.clone())))
    }
    fn localized(&mut self, key: &str) -> DecodeResult<Option<LocalizedText>> {
        self.read(key, |value, _, _| Ok(LocalizedText(value.clone())))
    }
    fn object<T>(
        &mut self,
        key: &str,
        parse: impl FnOnce(&mut DcsArgumentReader<'a>) -> DecodeResult<T>,
    ) -> DecodeResult<Option<T>> {
        self.read(key, |value, shape, path| {
            let mut reader = Self::new(value, shape, path)?;
            let parsed = parse(&mut reader)?;
            reader.finish(parsed)
        })
    }
    fn required_object<T>(
        &mut self,
        key: &str,
        parse: impl FnOnce(&mut DcsArgumentReader<'a>) -> DecodeResult<T>,
    ) -> DecodeResult<T> {
        self.object(key, parse)?.ok_or_else(|| {
            invalid(
                &format!("{}.{key}", self.path),
                "required decoded object is missing",
            )
        })
    }
    fn array<T>(
        &mut self,
        key: &str,
        parse: impl Fn(&'a Value, ArgumentShape<'a>, &str) -> DecodeResult<T>,
    ) -> DecodeResult<Option<Vec<T>>> {
        self.read(key, |value, shape, path| {
            let values = value
                .as_array()
                .ok_or_else(|| invalid(path, "expected an argument array"))?;
            let item = shape
                .items()
                .ok_or_else(|| invalid(path, "array has no item contract"))?;
            values
                .iter()
                .enumerate()
                .map(|(index, value)| parse(value, item, &format!("{path}[{index}]")))
                .collect()
        })
    }
    fn objects<T>(
        &mut self,
        key: &str,
        parse: impl Fn(&mut DcsArgumentReader<'a>) -> DecodeResult<T>,
    ) -> DecodeResult<Option<Vec<T>>> {
        self.array(key, |value, shape, path| {
            let mut reader = Self::new(value, shape, path)?;
            let parsed = parse(&mut reader)?;
            reader.finish(parsed)
        })
    }
    fn items<T>(
        &mut self,
        parse: impl Fn(&mut DcsArgumentReader<'a>) -> DecodeResult<T>,
    ) -> DecodeResult<Vec<T>> {
        self.objects("items", parse)?.ok_or_else(|| {
            invalid(
                &format!("{}.items", self.path),
                "required decoded items are missing",
            )
        })
    }
    fn strings(&mut self, key: &str) -> DecodeResult<Option<Vec<String>>> {
        self.array(key, |value, _, path| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| invalid(path, "expected text"))
        })
    }
    fn groups(&mut self, key: &str) -> DecodeResult<Option<Vec<String>>> {
        self.read(key, |value, _, path| {
            if let Some(value) = value.as_str() {
                return Ok(vec![value.into()]);
            }
            value
                .as_array()
                .ok_or_else(|| invalid(path, "expected a group list"))?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| invalid(path, "expected group text"))
                })
                .collect()
        })
    }
    fn index(&mut self, key: &str) -> DecodeResult<Option<DcsIndex>> {
        self.read(key, |value, _, path| decode_index(value, path))
    }
    fn indexes(&mut self) -> DecodeResult<Vec<DcsIndex>> {
        Ok(self
            .array("parentIndexes", |value, _, path| decode_index(value, path))?
            .unwrap_or_default())
    }
    fn finish<T>(self, parsed: T) -> DecodeResult<T> {
        for alias in self.shape.aliases() {
            if self.shape.property(alias).is_none() {
                return Err(invalid(
                    &format!("{}.{}", self.path, alias),
                    "alias has no declared argument shape",
                ));
            }
        }
        if let Some(key) = self
            .shape
            .property_names()
            .find(|key| !self.consumed.contains(*key))
        {
            return Err(invalid(
                &format!("{}.{key}", self.path),
                "declared argument has no domain decoder",
            ));
        }
        Ok(parsed)
    }
}

fn decode_index(value: &Value, path: &str) -> DecodeResult<DcsIndex> {
    value
        .as_i64()
        .and_then(|number| usize::try_from(number).ok())
        .map(DcsIndex)
        .ok_or_else(|| invalid(path, "index is outside the supported range"))
}

fn restriction(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<Restriction> {
    Ok(Restriction {
        field: reader.boolean("field")?,
        condition: reader.boolean("condition")?,
        group: reader.boolean("group")?,
        order: reader.boolean("order")?,
    })
}
fn data_source(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<DataSourceSpec> {
    Ok(DataSourceSpec {
        name: reader.required_string("name")?,
        kind: reader.required_enum("kind", DataSourceKind::parse)?,
    })
}
fn data_set(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<DataSetSpec> {
    Ok(DataSetSpec {
        name: reader.required_string("name")?,
        kind: reader.required_enum("kind", DataSetKind::parse)?,
        data_source: reader.string("dataSource")?,
        query: reader.string("query")?,
        object_name: reader.string("objectName")?,
    })
}
fn data_set_patch(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<DataSetPatch> {
    Ok(DataSetPatch {
        data_source: reader.string("dataSource")?,
        object_name: reader.string("objectName")?,
        auto_fill_fields: reader.boolean("autoFillFields")?,
    })
}
fn field(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<FieldSpec> {
    Ok(FieldSpec {
        data_path: reader.required_string("dataPath")?,
        field: reader.string("field")?,
        type_name: reader.string("type")?,
        title: reader.localized("title")?,
        use_restriction: reader.object("useRestriction", restriction)?,
        attribute_use_restriction: reader.object("attributeUseRestriction", restriction)?,
        presentation_expression: reader.string("presentationExpression")?,
    })
}
fn field_patch(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<FieldPatch> {
    Ok(FieldPatch {
        source_field: reader.string("sourceField")?,
        type_name: reader.string("type")?,
        title: reader.localized("title")?,
        use_restriction: reader.object("useRestriction", restriction)?,
        attribute_use_restriction: reader.object("attributeUseRestriction", restriction)?,
        presentation_expression: reader.string("presentationExpression")?,
    })
}
fn field_role(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<FieldRole> {
    Ok(FieldRole {
        parent_dimension: reader.string("parentDimension")?,
        account_type_expression: reader.string("accountTypeExpression")?,
        balance_group_name: reader.string("balanceGroupName")?,
        account_field: reader.string("accountField")?,
        period_number: reader.string("periodNumber")?,
        period_type: reader.string("periodType")?,
        dimension: reader.boolean("dimension")?,
        account: reader.boolean("account")?,
        balance: reader.boolean("balance")?,
        ignore_null_values: reader.boolean("ignoreNullValues")?,
        required: reader.boolean("required")?,
        dimension_attribute: reader.boolean("dimensionAttribute")?,
        balance_type: reader.string("balanceType")?,
        accounting_balance_type: reader.string("accountingBalanceType")?,
    })
}
fn parameter_patch(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<ParameterPatch> {
    Ok(ParameterPatch {
        type_name: reader.string("type")?,
        title: reader.localized("title")?,
        value_type: reader.string("valueType")?,
        value: reader.atom("value")?,
        values: reader.array("values", |value, _, _| Ok(ValueAtom(value.clone())))?,
        available_values: reader.objects("availableValues", |reader| {
            Ok(AvailableValue {
                value: reader
                    .atom("value")?
                    .ok_or_else(|| invalid(&reader.path, "available value is missing"))?,
                presentation: reader.localized("presentation")?,
            })
        })?,
        expression: reader.string("expression")?,
        use_restriction: reader.boolean("useRestriction")?,
        value_list_allowed: reader.boolean("valueListAllowed")?,
        available_as_field: reader.boolean("availableAsField")?,
        deny_incomplete_values: reader.boolean("denyIncompleteValues")?,
        use_mode: reader.enumeration("use", ParameterUse::parse)?,
    })
}
fn parameter(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<ParameterSpec> {
    Ok(ParameterSpec {
        name: reader.required_string("name")?,
        properties: parameter_patch(reader)?,
    })
}
fn calculated(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<CalculatedFieldSpec> {
    Ok(CalculatedFieldSpec {
        name: reader.required_string("name")?,
        expression: reader.required_string("expression")?,
        title: reader.localized("title")?,
        type_name: reader.string("type")?,
        use_restriction: reader.object("useRestriction", restriction)?,
    })
}
fn calculated_patch(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<CalculatedFieldPatch> {
    Ok(CalculatedFieldPatch {
        expression: reader.string("expression")?,
        title: reader.localized("title")?,
        type_name: reader.string("type")?,
        use_restriction: reader.object("useRestriction", restriction)?,
    })
}
fn total(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<TotalSpec> {
    Ok(TotalSpec {
        field: reader.required_string("field")?,
        expression: reader.string("expression")?,
        groups: reader.groups("group")?,
    })
}
fn total_patch(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<TotalPatch> {
    Ok(TotalPatch {
        expression: reader.string("expression")?,
        groups: reader.groups("group")?,
    })
}
fn variant(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<VariantSpec> {
    let name = reader.required_string("name")?;
    let title = reader
        .localized("title")?
        .unwrap_or_else(|| LocalizedText(Value::String(name.clone())));
    Ok(VariantSpec { name, title })
}
fn string_meta(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<SettingsMeta<String>> {
    Ok(SettingsMeta {
        use_flag: reader.boolean("use")?,
        view_mode: reader.enumeration("viewMode", ViewMode::parse)?,
        user_setting_id: reader.string("userSettingID")?,
        user_setting_presentation: reader.string("userSettingPresentation")?,
    })
}
fn localized_meta(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<SettingsMeta> {
    Ok(SettingsMeta {
        use_flag: reader.boolean("use")?,
        view_mode: reader.enumeration("viewMode", ViewMode::parse)?,
        user_setting_id: reader.string("userSettingID")?,
        user_setting_presentation: reader.localized("userSettingPresentation")?,
    })
}
fn filter_patch(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<FilterPatch> {
    Ok(FilterPatch {
        field: reader.required_string("field")?,
        comparison: reader.enumeration("comparison", FilterComparison::parse)?,
        value: reader.atom("value")?,
        value_type: reader.string("valueType")?,
        meta: string_meta(reader)?,
    })
}
fn filter(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<FilterSpec> {
    let parsed = filter_patch(reader)?;
    Ok(FilterSpec {
        field: parsed.field,
        comparison: parsed.comparison.ok_or_else(|| {
            invalid(
                &format!("{}.comparison", reader.path),
                "comparison default is missing",
            )
        })?,
        value: parsed.value,
        value_type: parsed.value_type,
        meta: parsed.meta,
    })
}
fn selection_key(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<SelectionKey> {
    Ok(SelectionKey {
        field: reader.string("field")?,
        index: reader.index("index")?,
    })
}
fn selection(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<SelectionSpec> {
    Ok(SelectionSpec {
        kind: reader.required_enum("kind", SelectionKind::parse)?,
        field: reader.string("field")?,
        title: reader.localized("title")?,
        parent_indexes: reader.indexes()?,
    })
}
fn selection_patch(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<SelectionPatch> {
    Ok(SelectionPatch {
        selector: selection_key(reader)?,
        parent_indexes: reader.indexes()?,
        meta: string_meta(reader)?,
    })
}
fn selection_remove(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<SelectionRemove> {
    Ok(SelectionRemove {
        selector: selection_key(reader)?,
        parent_indexes: reader.indexes()?,
    })
}
fn order(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<OrderSpec> {
    Ok(OrderSpec {
        kind: reader.required_enum("kind", OrderKind::parse)?,
        field: reader.string("field")?,
        direction: reader.enumeration("direction", OrderDirection::parse)?,
    })
}
fn order_patch(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<OrderPatch> {
    Ok(OrderPatch {
        selector: selection_key(reader)?,
        direction: reader.enumeration("direction", OrderDirection::parse)?,
        meta: string_meta(reader)?,
    })
}
fn settings_parameter(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<SettingsParameterSpec> {
    Ok(SettingsParameterSpec {
        name: reader.required_string("name")?,
        value: reader.atom("value")?,
        value_type: reader.string("valueType")?,
        meta: localized_meta(reader)?,
    })
}
fn appearance(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<ConditionalAppearanceSpec> {
    Ok(ConditionalAppearanceSpec {
        fields: reader
            .strings("fields")?
            .ok_or_else(|| invalid(&reader.path, "appearance fields are missing"))?,
        filters: reader.objects("filter", filter)?,
        appearance: reader.required("appearance", |value, shape, path| {
            let atom = shape
                .additional_properties()
                .ok_or_else(|| invalid(path, "appearance values have no contract"))?;
            value
                .as_object()
                .ok_or_else(|| invalid(path, "expected appearance properties"))?
                .iter()
                .map(|(name, value)| {
                    atom.validate_field(value, &format!("{path}.{name}"))?;
                    Ok((name.clone(), ValueAtom(value.clone())))
                })
                .collect()
        })?,
        use_flag: reader.boolean("use")?,
    })
}
fn structure_meta(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<StructureMeta> {
    Ok(StructureMeta {
        settings: localized_meta(reader)?,
        columns_view_mode: reader.enumeration("columnsViewMode", ViewMode::parse)?,
        rows_view_mode: reader.enumeration("rowsViewMode", ViewMode::parse)?,
        points_view_mode: reader.enumeration("pointsViewMode", ViewMode::parse)?,
        series_view_mode: reader.enumeration("seriesViewMode", ViewMode::parse)?,
        items_view_mode: reader.enumeration("itemsViewMode", ViewMode::parse)?,
    })
}
fn structure(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<StructureSpec> {
    Ok(StructureSpec {
        name: reader.string("name")?,
        kind: reader.required_enum("kind", StructureKind::parse)?,
        group_by: reader.strings("groupBy")?,
        axis: reader.enumeration("axis", StructureAxis::parse)?,
        meta: structure_meta(reader)?,
    })
}
fn structure_set(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<StructureSet> {
    Ok(StructureSet {
        item: StructureSpec {
            name: reader.string("name")?,
            kind: StructureKind::Group,
            group_by: reader.strings("groupBy")?,
            axis: None,
            meta: structure_meta(reader)?,
        },
        details: reader.required_boolean("details")?,
    })
}
fn structure_patch(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<StructurePatch> {
    Ok(StructurePatch {
        group_by: reader.strings("groupBy")?,
        meta: structure_meta(reader)?,
    })
}
fn link_selector(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<DataSetLinkSelector> {
    Ok(DataSetLinkSelector {
        source: reader.required_string("source")?,
        destination: reader.required_string("destination")?,
        source_expression: reader.required_string("sourceExpression")?,
        destination_expression: reader.required_string("destinationExpression")?,
    })
}
fn link_patch(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<DataSetLinkPatch> {
    Ok(DataSetLinkPatch {
        parameter: reader.string("parameter")?,
        parameter_list_allowed: reader.boolean("parameterListAllowed")?,
        required: reader.boolean("required")?,
        condition: reader.string("condition")?,
        start_expression: reader.string("startExpression")?,
    })
}
fn link(reader: &mut DcsArgumentReader<'_>) -> DecodeResult<DataSetLinkSpec> {
    Ok(DataSetLinkSpec {
        selector: link_selector(reader)?,
        properties: link_patch(reader)?,
    })
}

pub(super) fn decode(
    operation: super::dcs_operation::Primitive,
    args: &Value,
    path: &str,
) -> DecodeResult<super::dcs_operation::DcsIntent> {
    decode_with_contract(operation, &operation.contract(), args, path)
}

fn decode_with_contract(
    operation: super::dcs_operation::Primitive,
    contract: &OperationContract,
    args: &Value,
    path: &str,
) -> DecodeResult<super::dcs_operation::DcsIntent> {
    use super::dcs_operation::{DcsIntent as I, Primitive as P};
    let normalized = contract.normalize_dcs(args, path)?;
    let mut reader =
        DcsArgumentReader::new(&normalized, ArgumentShape::new(&contract.schema), path)?;
    reader.string("at")?;
    let intent = match operation {
        P::DataSourceAdd => I::DataSourceAdd(reader.items(data_source)?),
        P::DataSourceSet => I::DataSourceSet(reader.required_object("values", |reader| {
            reader.required_enum("kind", DataSourceKind::parse)
        })?),
        P::DataSourceRemove => I::DataSourceRemove(()),
        P::DataSetAdd => I::DataSetAdd(reader.items(data_set)?),
        P::DataSetSet => I::DataSetSet(reader.required_object("values", data_set_patch)?),
        P::DataSetRemove => I::DataSetRemove(()),
        P::FieldAdd => I::FieldAdd(reader.items(field)?),
        P::FieldSet => I::FieldSet(reader.required_object("values", field_patch)?),
        P::FieldRemove => I::FieldRemove(()),
        P::FieldRoleSet => I::FieldRoleSet(reader.required_object("values", |reader| {
            reader.required_object("role", field_role)
        })?),
        P::ParameterAdd => I::ParameterAdd(reader.items(parameter)?),
        P::ParameterSet => I::ParameterSet(reader.required_object("values", parameter_patch)?),
        P::ParameterRemove => I::ParameterRemove(()),
        P::ParameterRename => I::ParameterRename(
            reader.required_object("values", |reader| reader.required_string("newName"))?,
        ),
        P::ParameterReorder => {
            I::ParameterReorder(reader.items(|reader| reader.required_string("name"))?)
        }
        P::CalculatedFieldAdd => I::CalculatedFieldAdd(reader.items(calculated)?),
        P::CalculatedFieldRemove => I::CalculatedFieldRemove(()),
        P::TotalAdd => I::TotalAdd(reader.items(total)?),
        P::TotalRemove => I::TotalRemove(()),
        P::VariantAdd => I::VariantAdd(reader.items(variant)?),
        P::VariantSet => I::VariantSet(reader.required_object("values", |reader| {
            reader
                .localized("title")?
                .ok_or_else(|| invalid(&reader.path, "variant title is missing"))
        })?),
        P::VariantRemove => I::VariantRemove(()),
        P::QuerySet => {
            I::QuerySet(reader.required_object("values", |reader| reader.required_string("query"))?)
        }
        P::QueryPatch => I::QueryPatch(reader.required_object("values", |reader| {
            Ok(QueryPatch {
                find: reader.required_string("find")?,
                replace: reader.required_string("replace")?,
                once: reader.required_boolean("once")?,
            })
        })?),
        P::FilterAdd => I::FilterAdd(reader.items(filter)?),
        P::FilterSet => I::FilterSet(reader.required_object("values", filter_patch)?),
        P::FilterRemove => I::FilterRemove(
            reader.required_object("values", |reader| reader.required_string("field"))?,
        ),
        P::FilterClear => {
            reader.object("values", |_| Ok(()))?;
            I::FilterClear(())
        }
        P::SelectionAdd => I::SelectionAdd(reader.items(selection)?),
        P::SelectionClear => {
            reader.object("values", |_| Ok(()))?;
            I::SelectionClear(())
        }
        P::OrderAdd => I::OrderAdd(reader.items(order)?),
        P::OrderClear => {
            reader.object("values", |_| Ok(()))?;
            I::OrderClear(())
        }
        P::DataParameterAdd => I::DataParameterAdd(reader.items(settings_parameter)?),
        P::DataParameterSet => {
            I::DataParameterSet(reader.required_object("values", settings_parameter)?)
        }
        P::OutputParameterSet => {
            I::OutputParameterSet(reader.required_object("values", settings_parameter)?)
        }
        P::ConditionalAppearanceAdd => I::ConditionalAppearanceAdd(reader.items(appearance)?),
        P::ConditionalAppearanceClear => {
            reader.object("values", |_| Ok(()))?;
            I::ConditionalAppearanceClear(())
        }
        P::StructureAdd => I::StructureAdd(reader.items(structure)?),
        P::StructureSet => I::StructureSet(reader.required_object("values", structure_set)?),
        P::StructurePatch => I::StructurePatch(reader.required_object("values", structure_patch)?),
        P::StructureRemove => I::StructureRemove(()),
        P::DataSetLinkAdd => I::DataSetLinkAdd(reader.items(link)?),
        P::CalculatedFieldSet => {
            I::CalculatedFieldSet(reader.required_object("values", calculated_patch)?)
        }
        P::TotalSet => I::TotalSet(reader.required_object("values", total_patch)?),
        P::SelectionSet => I::SelectionSet(reader.required_object("values", selection_patch)?),
        P::SelectionRemove => {
            I::SelectionRemove(reader.required_object("values", selection_remove)?)
        }
        P::OrderSet => I::OrderSet(reader.required_object("values", order_patch)?),
        P::OrderRemove => I::OrderRemove(reader.required_object("values", selection_key)?),
        P::DataParameterRemove => I::DataParameterRemove(
            reader.required_object("values", |reader| reader.required_string("name"))?,
        ),
        P::OutputParameterRemove => I::OutputParameterRemove(
            reader.required_object("values", |reader| reader.required_string("name"))?,
        ),
        P::ConditionalAppearanceRemove => {
            I::ConditionalAppearanceRemove(reader.required_object("values", |reader| {
                reader
                    .index("index")?
                    .ok_or_else(|| invalid(&reader.path, "appearance index is missing"))
            })?)
        }
        P::DataSetLinkSet => I::DataSetLinkSet(reader.required_object("values", |reader| {
            Ok(DataSetLinkSpec {
                selector: reader.required_object("selector", link_selector)?,
                properties: link_patch(reader)?,
            })
        })?),
        P::DataSetLinkRemove => {
            I::DataSetLinkRemove(reader.required_object("values", |reader| {
                reader.required_object("selector", link_selector)
            })?)
        }
    };
    reader.finish(intent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::dcs_operation::{DcsIntent, Primitive};
    use serde_json::json;

    #[test]
    fn every_dcs_identity_decodes_its_example_and_rejects_unknown_arguments() {
        assert_eq!(Primitive::ALL.len(), 53);
        for operation in Primitive::ALL {
            let contract = operation.contract();
            let intent = operation
                .decode(&contract.example_args, "ops[3].args")
                .unwrap_or_else(|error| panic!("{}: {error:?}", operation.name()));
            assert_eq!(intent.operation(), *operation);
            let mut unknown = contract.example_args.clone();
            unknown["unmodeled"] = json!(true);
            let error = operation.decode(&unknown, "ops[3].args").unwrap_err();
            assert_eq!(error.path, "ops[3].args.unmodeled", "{}", operation.name());
            if operation.skeleton() != crate::domain::apply::OperationSkeleton::Target
                && !matches!(
                    operation,
                    Primitive::FilterClear
                        | Primitive::SelectionClear
                        | Primitive::OrderClear
                        | Primitive::ConditionalAppearanceClear
                )
            {
                assert!(
                    operation.decode(&json!({}), "args").is_err(),
                    "{}",
                    operation.name()
                );
            }
            let wrong = match operation.skeleton() {
                crate::domain::apply::OperationSkeleton::Items => json!({"items":false}),
                crate::domain::apply::OperationSkeleton::Values => json!({"values":false}),
                crate::domain::apply::OperationSkeleton::Target => json!({"at":false}),
                other => panic!("unexpected DCS skeleton {other:?}"),
            };
            assert!(
                operation.decode(&wrong, "args").is_err(),
                "{}",
                operation.name()
            );
        }
    }

    #[test]
    fn declared_optional_fields_and_unwired_aliases_cannot_escape_the_decoder() {
        let operation = Primitive::QueryPatch;
        let mut contract = operation.contract();
        contract.schema["properties"]["values"]["properties"]["extra"] = json!({"type":"boolean"});
        let error =
            decode_with_contract(operation, &contract, &contract.example_args, "args").unwrap_err();
        assert_eq!(error.path, "args.values.extra");
        assert_eq!(error.message, "declared argument has no domain decoder");

        let operation = Primitive::OrderAdd;
        let mut contract = operation.contract();
        contract.schema["properties"]["items"]["items"]["anyOf"][0]["properties"]["branchExtra"] =
            json!({"type":"boolean"});
        let error =
            decode_with_contract(operation, &contract, &contract.example_args, "args").unwrap_err();
        assert_eq!(error.path, "args.items[0].branchExtra");

        let operation = Primitive::QuerySet;
        let mut contract = operation.contract();
        contract.schema["properties"]["values"]["properties"]["newAlias"] =
            json!({"type":"string"});
        let error =
            decode_with_contract(operation, &contract, &contract.example_args, "args").unwrap_err();
        assert_eq!(error.path, "args.values.newAlias");
        contract.schema["properties"]["values"]["x-unica-aliases"] =
            json!(["newCanonical", "newAlias"]);
        let error =
            decode_with_contract(operation, &contract, &contract.example_args, "args").unwrap_err();
        assert_eq!(error.path, "args.values.newCanonical");
    }

    #[test]
    fn dcs_schema_type_and_default_changes_reach_the_productive_decoder() {
        let operation = Primitive::QueryPatch;
        let mut contract = operation.contract();
        contract.schema["properties"]["values"]["properties"]["once"]["default"] = json!(true);
        let DcsIntent::QueryPatch(patch) = decode_with_contract(
            operation,
            &contract,
            &json!({"values":{"find":"literal"}}),
            "args",
        )
        .unwrap() else {
            panic!("query patch expected")
        };
        assert!(patch.once);
        assert_eq!(patch.replace, "");
        contract.schema["properties"]["values"]["properties"]["find"]["type"] = json!("boolean");
        assert!(decode_with_contract(
            operation,
            &contract,
            &json!({"values":{"find":"literal"}}),
            "args"
        )
        .is_err());
    }

    #[test]
    fn dcs_integer_spellings_share_indices_without_changing_atom_numbers_or_key_order() {
        for operation in [
            Primitive::SelectionSet,
            Primitive::SelectionRemove,
            Primitive::OrderSet,
            Primitive::OrderRemove,
            Primitive::ConditionalAppearanceRemove,
        ] {
            let contract = operation.contract();
            assert_eq!(
                contract.schema["properties"]["values"]["properties"]["index"]["maximum"],
                json!(i64::MAX)
            );
        }
        for operation in [
            Primitive::SelectionAdd,
            Primitive::SelectionSet,
            Primitive::SelectionRemove,
        ] {
            let contract = operation.contract();
            let item = if operation == Primitive::SelectionAdd {
                &contract.schema["properties"]["items"]["items"]
            } else {
                &contract.schema["properties"]["values"]
            };
            assert_eq!(
                item["properties"]["parentIndexes"]["items"]["maximum"],
                json!(i64::MAX)
            );
        }
        let DcsIntent::SelectionRemove(remove) = Primitive::SelectionRemove
            .decode(&json!({"values":{"index":i64::MAX}}), "args")
            .unwrap()
        else {
            panic!("selection removal expected")
        };
        assert_eq!(
            remove.selector.index.unwrap().get(),
            usize::try_from(i64::MAX).unwrap()
        );
        for spelling in ["0", "0.0", "0e0", "-0.0"] {
            let args: Value = serde_json::from_str(&format!(
                "{{\"values\":{{\"index\":{spelling},\"parentIndexes\":[{spelling}]}}}}"
            ))
            .unwrap();
            let DcsIntent::SelectionRemove(remove) = Primitive::SelectionRemove
                .decode(&args, "ops[5].args")
                .unwrap()
            else {
                panic!("selection removal expected")
            };
            assert_eq!(remove.selector.index.unwrap().get(), 0);
            assert_eq!(remove.parent_indexes, vec![DcsIndex(0)]);
        }
        for spelling in [
            "-1",
            "-1.0",
            "0.5",
            "9223372036854775808",
            "9.223372036854776e18",
            "18446744073709551615",
            "null",
            "true",
            "\"0\"",
        ] {
            let args: Value =
                serde_json::from_str(&format!("{{\"values\":{{\"index\":{spelling}}}}}")).unwrap();
            assert!(
                Primitive::SelectionRemove
                    .decode(&args, "ops[5].args")
                    .is_err(),
                "{spelling}"
            );
        }
        let args: Value =
            serde_json::from_str("{\"values\":{\"typo\":true,\"index\":0.5}}").unwrap();
        assert_eq!(
            Primitive::SelectionRemove
                .decode(&args, "args")
                .unwrap_err()
                .path,
            "args.values.typo"
        );
        let args: Value =
            serde_json::from_str("{\"values\":{\"index\":0.5,\"typo\":true}}").unwrap();
        assert_eq!(
            Primitive::SelectionRemove
                .decode(&args, "args")
                .unwrap_err()
                .path,
            "args.values.index"
        );
        let args = json!({"items":[{"name":"P","value":1.5}]});
        let DcsIntent::ParameterAdd(parameters) =
            Primitive::ParameterAdd.decode(&args, "args").unwrap()
        else {
            panic!("parameter addition expected")
        };
        assert_eq!(
            parameters[0].properties.value.as_ref().unwrap().value(),
            &json!(1.5)
        );
    }

    #[test]
    fn typed_presence_keeps_null_empty_lists_alias_priority_and_late_selectors() {
        let DcsIntent::ParameterSet(patch) = Primitive::ParameterSet
            .decode(&json!({"values":{"value":null}}), "args")
            .unwrap()
        else {
            panic!("parameter patch expected")
        };
        assert!(patch.value.unwrap().value().is_null());
        assert!(patch.values.is_none());
        let DcsIntent::ParameterSet(patch) = Primitive::ParameterSet
            .decode(
                &json!({"values":{"valueType":"xs:string","values":[]}}),
                "args",
            )
            .unwrap()
        else {
            panic!("parameter patch expected")
        };
        assert_eq!(patch.values.unwrap().len(), 0);
        let DcsIntent::QuerySet(query) = Primitive::QuerySet
            .decode(
                &json!({"values":{"query":"primary","text":"secondary"}}),
                "args",
            )
            .unwrap()
        else {
            panic!("query text expected")
        };
        assert_eq!(query, "primary");
        let DcsIntent::SelectionRemove(remove) = Primitive::SelectionRemove
            .decode(&json!({"values":{"field":"Amount","index":0}}), "args")
            .unwrap()
        else {
            panic!("selection removal expected")
        };
        assert_eq!(remove.selector.field.as_deref(), Some("Amount"));
        assert_eq!(remove.selector.index, Some(DcsIndex(0)));
        let DcsIntent::OrderAdd(items) = Primitive::OrderAdd
            .decode(
                &json!({"items":[{"field":"Amount"},{"kind":"Auto"}]}),
                "args",
            )
            .unwrap()
        else {
            panic!("order addition expected")
        };
        assert_eq!(items[0].direction, Some(OrderDirection::Asc));
        assert_eq!(items[1].direction, None);
    }
}
