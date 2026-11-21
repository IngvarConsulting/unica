//! Platform XML seeds for the registered ExternalDataSource child shapes.
//! Defaults follow the all_kinds exports in formats 2.20–2.22.

use unica_address::address::NodeKind;
use unica_metadata::{MetadataType, MetadataTypeVariant};
use unica_source_designer::owner::MD_CLASSES_NS;
use unica_xml_format::text::escape_xml;

use crate::external_data_source_write::TypeNamespaces;

#[derive(Debug, Clone, Copy)]
pub(crate) enum TableSource<'a> {
    Table(&'a str),
    Expression(&'a str),
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Seed<'a> {
    Table {
        source: TableSource<'a>,
        data_type: &'a str,
    },
    Cube {
        source_name: &'a str,
    },
    DimensionTable {
        source_name: &'a str,
    },
    Field {
        source_name: &'a str,
        field_type: &'a MetadataType,
    },
    Function {
        expression: &'a str,
        return_value: bool,
        return_type: &'a MetadataType,
    },
    Dimension {
        type_qname: &'a str,
    },
    Resource {
        source_name: &'a str,
        field_type: &'a MetadataType,
    },
}

impl Seed<'_> {
    pub(crate) fn kind(self) -> NodeKind {
        match self {
            Self::Table { .. } => NodeKind::Table,
            Self::Cube { .. } => NodeKind::Cube,
            Self::DimensionTable { .. } => NodeKind::DimensionTable,
            Self::Field { .. } => NodeKind::Field,
            Self::Function { .. } => NodeKind::Function,
            Self::Dimension { .. } => NodeKind::Dimension,
            Self::Resource { .. } => NodeKind::Resource,
        }
    }

    fn separate_descriptor(self) -> bool {
        matches!(
            self,
            Self::Table { .. } | Self::Cube { .. } | Self::DimensionTable { .. }
        )
    }
}

pub(crate) fn emit<F>(
    seed: Seed<'_>,
    name: &str,
    generated_name: &str,
    namespaces: &TypeNamespaces,
    next_uuid: &mut F,
) -> String
where
    F: FnMut() -> String,
{
    let kind = seed.kind().as_str();
    let uuid = next_uuid();
    let standalone = seed.separate_descriptor();
    let open = if standalone {
        format!("<{kind} uuid=\"{uuid}\">")
    } else {
        let declarations = namespaces.declarations(None);
        format!("<{kind} xmlns=\"{MD_CLASSES_NS}\"{declarations} uuid=\"{uuid}\">")
    };
    let mut lines = vec![open];
    if standalone {
        crate::template_catalog::emit_meta_internal_info(
            &mut lines,
            "\t",
            kind,
            generated_name,
            next_uuid,
        );
    }
    lines.push("\t<Properties>".into());
    scalar(&mut lines, "Name", name);
    defaults(&mut lines, &["<Synonym/>", "<Comment/>"]);
    match seed {
        Seed::Table { source, data_type } => {
            let (table_type, source_name, expression) = match source {
                TableSource::Table(source_name) => ("Table", source_name, ""),
                TableSource::Expression(expression) => ("Expression", "", expression),
            };
            scalar(&mut lines, "TableType", table_type);
            scalar(&mut lines, "NameInDataSource", source_name);
            scalar(&mut lines, "ExpressionInDataSource", expression);
            scalar(&mut lines, "TableDataType", data_type);
            defaults(&mut lines, TABLE_DEFAULTS);
        }
        Seed::Cube { source_name } => {
            scalar(&mut lines, "NameInDataSource", source_name);
            defaults(&mut lines, CUBE_DEFAULTS);
        }
        Seed::DimensionTable { source_name } => {
            scalar(&mut lines, "NameInDataSource", source_name);
            defaults(&mut lines, DIMENSION_TABLE_DEFAULTS);
        }
        Seed::Field {
            source_name,
            field_type,
        } => {
            lines.push(namespaces.type_xml(field_type, "\t\t", "\n"));
            inline_defaults(&mut lines, FIELD_DEFAULTS_BEFORE_FILL, namespaces);
            let xsi = namespaces.xsi();
            let fill = if matches!(
                field_type.variants.as_slice(),
                [MetadataTypeVariant::String { .. }]
            ) {
                let xs = namespaces.xs();
                format!(r#"<FillValue {xsi}:type="{xs}:string"/>"#)
            } else {
                format!(r#"<FillValue {xsi}:nil="true"/>"#)
            };
            defaults(&mut lines, &[&fill]);
            defaults(&mut lines, FIELD_DEFAULTS_AFTER_FILL);
            scalar(&mut lines, "NameInDataSource", source_name);
            defaults(
                &mut lines,
                &["<ReadOnly>false</ReadOnly>", "<AllowNull>true</AllowNull>"],
            );
        }
        Seed::Function {
            expression,
            return_value,
            return_type,
        } => {
            scalar(
                &mut lines,
                "ReturnValue",
                if return_value { "true" } else { "false" },
            );
            lines.push(namespaces.type_xml(return_type, "\t\t", "\n"));
            scalar(&mut lines, "ExpressionInDataSource", expression);
        }
        Seed::Dimension { type_qname } => {
            lines.push("\t\t<Type>".into());
            lines.push(format!(
                "\t\t\t{}",
                namespaces.dimension_type_xml(type_qname)
            ));
            lines.push("\t\t</Type>".into());
            inline_defaults(&mut lines, DIMENSION_DEFAULTS, namespaces);
        }
        Seed::Resource {
            source_name,
            field_type,
        } => {
            lines.push(namespaces.type_xml(field_type, "\t\t", "\n"));
            defaults(&mut lines, RESOURCE_DEFAULTS);
            scalar(&mut lines, "NameInDataSource", source_name);
        }
    }
    lines.push("\t</Properties>".into());
    if standalone {
        lines.push("\t<ChildObjects/>".into());
    }
    lines.push(format!("</{kind}>"));
    lines.join("\n")
}

pub(crate) fn standalone(fragment: &str, version: &str) -> Vec<u8> {
    let lines = fragment
        .lines()
        .map(|line| format!("\t{line}"))
        .collect::<Vec<_>>();
    let child = lines.join("\n");
    let wrapped = crate::edit::wrap_standalone_descriptor(&child, version);
    let mut bytes = vec![0xef, 0xbb, 0xbf];
    bytes.extend(wrapped);
    bytes.push(b'\n');
    bytes
}

fn inline_defaults(lines: &mut Vec<String>, values: &[&str], namespaces: &TypeNamespaces) {
    let xsi_prefix = format!(" {}:", namespaces.xsi());
    lines.extend(
        values
            .iter()
            .map(|xml| format!("\t\t{}", xml.replace(" xsi:", &xsi_prefix))),
    );
}

fn scalar(lines: &mut Vec<String>, tag: &str, value: &str) {
    let value = escape_xml(value)
        .replace('\r', "&#13;")
        .replace('\n', "&#10;")
        .replace('\t', "&#9;");
    lines.push(if value.is_empty() {
        format!("\t\t<{tag}/>")
    } else {
        format!("\t\t<{tag}>{value}</{tag}>")
    });
}

fn defaults(lines: &mut Vec<String>, values: &[&str]) {
    lines.extend(values.iter().map(|xml| format!("\t\t{xml}")));
}

const TABLE_DEFAULTS: &[&str] = &[
    "<KeyFields/>",
    "<PresentationField/>",
    "<ParentField/>",
    r#"<UnfilledParentValue xsi:nil="true"/>"#,
    "<Characteristics/>",
    "<UseStandardCommands>true</UseStandardCommands>",
    "<QuickChoice>false</QuickChoice>",
    "<InputByString/>",
    "<CreateOnInput>Auto</CreateOnInput>",
    "<SearchStringModeOnInputByString>Begin</SearchStringModeOnInputByString>",
    "<ChoiceDataGetModeOnInputByString>Directly</ChoiceDataGetModeOnInputByString>",
    "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
    "<DefaultObjectForm/>",
    "<DefaultRecordForm/>",
    "<DefaultListForm/>",
    "<DefaultChoiceForm/>",
    "<ObjectPresentation/>",
    "<ExtendedObjectPresentation/>",
    "<RecordPresentation/>",
    "<ExtendedRecordPresentation/>",
    "<ListPresentation/>",
    "<ExtendedListPresentation/>",
    "<Explanation/>",
    "<IncludeHelpInContents>false</IncludeHelpInContents>",
    "<ReadOnly>false</ReadOnly>",
    "<TransactionsIsolationLevel>Auto</TransactionsIsolationLevel>",
    "<DataVersionField/>",
    "<EditType>InDialog</EditType>",
    "<BasedOn/>",
    "<DataLockFields/>",
    "<DataLockControlMode>Automatic</DataLockControlMode>",
];

const CUBE_DEFAULTS: &[&str] = &[
    "<Characteristics/>",
    "<UseStandardCommands>false</UseStandardCommands>",
    "<DefaultRecordForm/>",
    "<DefaultListForm/>",
    "<RecordPresentation/>",
    "<ExtendedRecordPresentation/>",
    "<ListPresentation/>",
    "<ExtendedListPresentation/>",
    "<Explanation/>",
    "<IncludeHelpInContents>false</IncludeHelpInContents>",
];

const DIMENSION_TABLE_DEFAULTS: &[&str] = &[
    "<PresentationField/>",
    "<HierarchyNameInDataSource/>",
    "<LevelNumber>0</LevelNumber>",
    "<Hierarchical>false</Hierarchical>",
    r#"<UnfilledParentValue xsi:nil="true"/>"#,
    "<UseStandardCommands>false</UseStandardCommands>",
    "<QuickChoice>false</QuickChoice>",
    "<DefaultObjectForm/>",
    "<DefaultListForm/>",
    "<DefaultChoiceForm/>",
    "<ObjectPresentation/>",
    "<ExtendedObjectPresentation/>",
    "<ListPresentation/>",
    "<ExtendedListPresentation/>",
    "<Explanation/>",
    "<IncludeHelpInContents>false</IncludeHelpInContents>",
];

const FIELD_DEFAULTS_BEFORE_FILL: &[&str] = &[
    "<PasswordMode>false</PasswordMode>",
    "<Format/>",
    "<EditFormat/>",
    "<ToolTip/>",
    "<MarkNegatives>false</MarkNegatives>",
    "<Mask/>",
    "<MultiLine>false</MultiLine>",
    "<ExtendedEdit>false</ExtendedEdit>",
    r#"<MinValue xsi:nil="true"/>"#,
    r#"<MaxValue xsi:nil="true"/>"#,
    "<FillFromFillingValue>false</FillFromFillingValue>",
];

const FIELD_DEFAULTS_AFTER_FILL: &[&str] = &[
    "<FillChecking>DontCheck</FillChecking>",
    "<ChoiceParameterLinks/>",
    "<ChoiceParameters/>",
    "<QuickChoice>Auto</QuickChoice>",
    "<CreateOnInput>Auto</CreateOnInput>",
    "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
    "<ChoiceForm/>",
];

const DIMENSION_DEFAULTS: &[&str] = &[
    "<PasswordMode>false</PasswordMode>",
    "<Format/>",
    "<EditFormat/>",
    "<ToolTip/>",
    "<MarkNegatives>false</MarkNegatives>",
    "<Mask/>",
    "<MultiLine>false</MultiLine>",
    "<ExtendedEdit>false</ExtendedEdit>",
    r#"<MinValue xsi:nil="true"/>"#,
    r#"<MaxValue xsi:nil="true"/>"#,
    "<FillFromFillingValue>false</FillFromFillingValue>",
    r#"<FillValue xsi:nil="true"/>"#,
    "<FillChecking>DontCheck</FillChecking>",
    "<ChoiceFoldersAndItems>Items</ChoiceFoldersAndItems>",
    "<ChoiceParameterLinks/>",
    "<ChoiceParameters/>",
    "<QuickChoice>Auto</QuickChoice>",
    "<CreateOnInput>Auto</CreateOnInput>",
    "<ChoiceForm/>",
    "<LinkByType/>",
    "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
];

const RESOURCE_DEFAULTS: &[&str] = &[
    "<PasswordMode>false</PasswordMode>",
    "<Format/>",
    "<EditFormat/>",
    "<ToolTip/>",
    "<MarkNegatives>false</MarkNegatives>",
    "<Mask/>",
    "<MultiLine>false</MultiLine>",
    "<ChoiceParameterLinks/>",
    "<ChoiceParameters/>",
    "<QuickChoice>Auto</QuickChoice>",
    "<ChoiceForm/>",
    "<ExtendedEdit>false</ExtendedEdit>",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_external_cube_seed_has_platform_generated_types_and_owns_its_child_registrations() {
        let mut running = 0;
        let mut id = || {
            running += 1;
            format!("00000000-0000-4000-8000-{running:012x}")
        };
        let seed = Seed::Cube {
            source_name: "sales & costs",
        };
        let namespaces = TypeNamespaces::for_scope(None);
        let xml = standalone(
            &emit(seed, "Sales", "Warehouse.Sales", &namespaces, &mut id),
            "2.22",
        );
        let text = std::str::from_utf8(&xml)
            .unwrap()
            .trim_start_matches('\u{feff}');
        let doc = roxmltree::Document::parse(text).unwrap();
        let node = doc.root_element().first_element_child().unwrap();
        assert!(node.has_tag_name((MD_CLASSES_NS, "Cube")));
        assert_eq!(doc.root_element().attribute("version"), Some("2.22"));
        let generated: Vec<_> = node
            .descendants()
            .filter_map(|n| n.attribute("name"))
            .collect();
        assert_eq!(generated.len(), 7);
        assert!(
            generated.contains(&"ExternalDataSourceCubeDimensionsTablesManager.Warehouse.Sales")
        );
        assert!(node
            .children()
            .any(|child| child.has_tag_name("ChildObjects")));
        assert!(node
            .descendants()
            .any(|child| child.has_tag_name("NameInDataSource")
                && child.text() == Some("sales & costs")));
    }
}
