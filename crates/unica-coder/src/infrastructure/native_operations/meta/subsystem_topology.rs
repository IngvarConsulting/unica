use super::validation_context::inspect_metadata_registration_image;
use super::xml_model::{
    meta_info_child, meta_info_child_text, meta_info_children, meta_info_inner_text,
    parse_metadata_image,
};
use crate::domain::subsystem_address::SubsystemAddress;
use std::collections::BTreeMap;

const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

/// Читатель дескриптора по пути вложенности. Отсутствие образа — не пустая
/// подсистема, а недостающее доказательство, поэтому `Ok(None)` отличается от
/// `Err`.
pub(crate) type SubsystemDescriptorReader<'a> =
    dyn FnMut(&[String]) -> Result<Option<Vec<u8>>, String> + 'a;

/// Предельная вложенность зарегистрированного дерева подсистем.
pub(crate) const SUBSYSTEM_TOPOLOGY_MAX_NESTING: usize = 8;

/// Роль подсистемы в конфигурации. Две роли — не два представления одного
/// флага, а разные семантики отношения: интерфейсная задаёт видимость и
/// образует раздел командного интерфейса, функциональная группирует объекты для
/// разработки (v8std 543 п.1.1–1.3, v8std 705 п.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubsystemRole {
    Interface,
    Functional,
}

#[derive(Debug)]
struct SubsystemEntry {
    role: SubsystemRole,
    content: Vec<String>,
}

/// Доказанное зарегистрированное дерево подсистем. Строится по регистрациям, а
/// не по раскладке файлов: наличие XML в каталоге членства в конфигурации не
/// доказывает.
#[derive(Debug)]
pub(crate) struct SubsystemTopology {
    entries: BTreeMap<SubsystemAddress, SubsystemEntry>,
}

impl SubsystemTopology {
    pub(crate) fn addresses_with_role(&self, role: SubsystemRole) -> Vec<&SubsystemAddress> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.role == role)
            .map(|(address, _)| address)
            .collect()
    }

    /// Достигает ли объект хотя бы одного раздела командного интерфейса.
    pub(crate) fn reaches_command_interface(&self, object_reference: &str) -> bool {
        self.entries.values().any(|entry| {
            entry.role == SubsystemRole::Interface
                && entry
                    .content
                    .iter()
                    .any(|item| item.as_str() == object_reference)
        })
    }
}

struct SubsystemFacts {
    name: String,
    own_include: bool,
    children: Vec<String>,
    content: Vec<String>,
}

/// Разбирает дескриптор подсистемы. Спецификация формата объявляет
/// `IncludeInCommandInterface` обязательным, поэтому его отсутствие и любое
/// значение вне `true`/`false` — повреждённое доказательство, а не значение по
/// умолчанию.
fn subsystem_facts(bytes: &[u8]) -> Result<SubsystemFacts, String> {
    let (_, document) = parse_metadata_image(bytes)?;
    let Some(object) = document.root_element().children().find(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(MD_CLASSES_NS)
            && node.tag_name().name() == "Subsystem"
    }) else {
        return Err("image is not a Subsystem descriptor".to_string());
    };
    let properties = meta_info_child(object, "Properties").ok_or("Subsystem has no Properties")?;
    let name = meta_info_child_text(properties, "Name")
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "Subsystem Name is missing".to_string())?;
    let own_include = match meta_info_child_text(properties, "IncludeInCommandInterface").as_deref()
    {
        Some("true") => true,
        Some("false") => false,
        Some(other) => {
            return Err(format!(
                "Subsystem {name} has a non-canonical IncludeInCommandInterface `{other}`"
            ))
        }
        None => return Err(format!("Subsystem {name} has no IncludeInCommandInterface")),
    };
    let content = meta_info_child(properties, "Content")
        .map(|content| {
            meta_info_children(content, "Item")
                .into_iter()
                .map(|item| meta_info_inner_text(item).trim().to_string())
                .filter(|item| !item.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let children = meta_info_child(object, "ChildObjects")
        .map(|children| {
            meta_info_children(children, "Subsystem")
                .into_iter()
                .map(|child| meta_info_inner_text(child).trim().to_string())
                .filter(|child| !child.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Ok(SubsystemFacts {
        name,
        own_include,
        children,
        content,
    })
}

/// Строит дерево от регистраций `Configuration/ChildObjects` и далее от
/// `ChildObjects` каждого зарегистрированного родителя.
///
/// Любое сомнение — отказ, а не пустое дерево: пропавший зарегистрированный
/// дескриптор, имя, не совпавшее с регистрацией, неканонический boolean и
/// превышение вложенности доказательством не являются.
pub(crate) fn build_subsystem_topology(
    configuration: &[u8],
    read_descriptor: &mut SubsystemDescriptorReader<'_>,
) -> Result<SubsystemTopology, String> {
    let registration = inspect_metadata_registration_image(configuration)?;
    let roots = registration
        .registrations
        .iter()
        .filter(|(kind, _)| kind == "Subsystem")
        .map(|(_, name)| name.clone())
        .collect::<Vec<_>>();
    let mut entries = BTreeMap::new();
    for root in roots {
        collect_subsystem(
            &mut vec![root],
            true,
            read_descriptor,
            &mut entries,
            SUBSYSTEM_TOPOLOGY_MAX_NESTING,
        )?;
    }
    Ok(SubsystemTopology { entries })
}

fn collect_subsystem(
    names: &mut Vec<String>,
    ancestors_included: bool,
    read_descriptor: &mut SubsystemDescriptorReader<'_>,
    entries: &mut BTreeMap<SubsystemAddress, SubsystemEntry>,
    remaining_nesting: usize,
) -> Result<(), String> {
    let address = SubsystemAddress::from_names(names)?;
    if remaining_nesting == 0 {
        return Err(format!(
            "subsystem {} nests deeper than {SUBSYSTEM_TOPOLOGY_MAX_NESTING} levels",
            address.as_str()
        ));
    }
    let Some(bytes) = read_descriptor(names)? else {
        return Err(format!(
            "registered subsystem {} has no descriptor",
            address.as_str()
        ));
    };
    let facts = subsystem_facts(&bytes)?;
    let registered_name = names.last().expect("address names are not empty");
    if &facts.name != registered_name {
        return Err(format!(
            "subsystem descriptor {} is registered as {registered_name}",
            facts.name
        ));
    }
    let include = ancestors_included && facts.own_include;
    entries.insert(
        address,
        SubsystemEntry {
            role: if include {
                SubsystemRole::Interface
            } else {
                SubsystemRole::Functional
            },
            content: facts.content,
        },
    );
    for child in facts.children {
        names.push(child);
        let outcome = collect_subsystem(
            names,
            include,
            read_descriptor,
            entries,
            remaining_nesting - 1,
        );
        names.pop();
        outcome?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configuration(children: &str) -> Vec<u8> {
        format!(
            concat!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">"#,
                "<Configuration><Properties><Name>Fixture</Name></Properties>",
                "<ChildObjects>{}</ChildObjects></Configuration></MetaDataObject>"
            ),
            children
        )
        .into_bytes()
    }

    fn descriptor(name: &str, include: &str, content: &str, children: &str) -> Vec<u8> {
        let include = if include.is_empty() {
            String::new()
        } else {
            format!("<IncludeInCommandInterface>{include}</IncludeInCommandInterface>")
        };
        format!(
            concat!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" "#,
                r#"xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" "#,
                r#"xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">"#,
                "<Subsystem><Properties><Name>{}</Name>{}",
                r#"<Content>{}</Content></Properties>"#,
                "<ChildObjects>{}</ChildObjects></Subsystem></MetaDataObject>"
            ),
            name, include, content, children
        )
        .into_bytes()
    }

    fn item(reference: &str) -> String {
        format!(r#"<xr:Item xsi:type="xr:MDObjectRef">{reference}</xr:Item>"#)
    }

    fn build(
        config: Vec<u8>,
        images: Vec<(Vec<&str>, Vec<u8>)>,
    ) -> Result<SubsystemTopology, String> {
        let images = images
            .into_iter()
            .map(|(names, bytes)| {
                (
                    names.into_iter().map(str::to_string).collect::<Vec<_>>(),
                    bytes,
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut read = |names: &[String]| -> Result<Option<Vec<u8>>, String> {
            Ok(images.get(names).cloned())
        };
        build_subsystem_topology(&config, &mut read)
    }

    #[test]
    fn an_unregistered_descriptor_never_reaches_the_command_interface() {
        // The counterexample: a stray file under Subsystems/ must not lend the
        // register a section it was never registered into.
        let topology = build(
            configuration(""),
            vec![(
                vec!["Stray"],
                descriptor("Stray", "true", &item("InformationRegister.Ledger"), ""),
            )],
        )
        .unwrap();

        assert!(!topology.reaches_command_interface("InformationRegister.Ledger"));
        assert!(topology
            .addresses_with_role(SubsystemRole::Interface)
            .is_empty());
    }

    #[test]
    fn an_unregistered_child_does_not_join_its_parent() {
        let topology = build(
            configuration("<Subsystem>Sales</Subsystem>"),
            vec![
                (vec!["Sales"], descriptor("Sales", "true", "", "")),
                (
                    vec!["Sales", "Hidden"],
                    descriptor("Hidden", "true", &item("InformationRegister.Ledger"), ""),
                ),
            ],
        )
        .unwrap();

        assert!(!topology.reaches_command_interface("InformationRegister.Ledger"));
    }

    #[test]
    fn a_registered_chain_that_stays_included_reaches_the_command_interface() {
        let topology = build(
            configuration("<Subsystem>Sales</Subsystem>"),
            vec![
                (
                    vec!["Sales"],
                    descriptor("Sales", "true", "", "<Subsystem>Orders</Subsystem>"),
                ),
                (
                    vec!["Sales", "Orders"],
                    descriptor("Orders", "true", &item("InformationRegister.Ledger"), ""),
                ),
            ],
        )
        .unwrap();

        assert!(topology.reaches_command_interface("InformationRegister.Ledger"));
        assert_eq!(
            topology
                .addresses_with_role(SubsystemRole::Interface)
                .iter()
                .map(|address| address.as_str())
                .collect::<Vec<_>>(),
            vec!["Sales", "Sales.Orders"]
        );
    }

    #[test]
    fn an_excluded_ancestor_makes_its_whole_subtree_functional() {
        let topology = build(
            configuration("<Subsystem>Library</Subsystem>"),
            vec![
                (
                    vec!["Library"],
                    descriptor("Library", "false", "", "<Subsystem>Service</Subsystem>"),
                ),
                (
                    vec!["Library", "Service"],
                    descriptor("Service", "true", &item("InformationRegister.Ledger"), ""),
                ),
            ],
        )
        .unwrap();

        assert!(!topology.reaches_command_interface("InformationRegister.Ledger"));
        assert_eq!(
            topology
                .addresses_with_role(SubsystemRole::Functional)
                .iter()
                .map(|address| address.as_str())
                .collect::<Vec<_>>(),
            vec!["Library", "Library.Service"]
        );
    }

    #[test]
    fn a_missing_registered_descriptor_is_not_a_proved_absence() {
        let error = build(configuration("<Subsystem>Sales</Subsystem>"), vec![]).unwrap_err();

        assert!(error.contains("has no descriptor"), "{error}");
    }

    #[test]
    fn a_descriptor_named_differently_from_its_registration_is_refused() {
        let error = build(
            configuration("<Subsystem>Sales</Subsystem>"),
            vec![(vec!["Sales"], descriptor("Renamed", "true", "", ""))],
        )
        .unwrap_err();

        assert!(error.contains("registered as Sales"), "{error}");
    }

    #[test]
    fn a_non_canonical_or_absent_flag_is_corrupt_evidence() {
        // The format spec declares the flag mandatory, so neither a typo nor an
        // absent tag may be read as a default.
        let typo = build(
            configuration("<Subsystem>Sales</Subsystem>"),
            vec![(vec!["Sales"], descriptor("Sales", "tru", "", ""))],
        )
        .unwrap_err();
        assert!(typo.contains("non-canonical"), "{typo}");

        let absent = build(
            configuration("<Subsystem>Sales</Subsystem>"),
            vec![(vec!["Sales"], descriptor("Sales", "", "", ""))],
        )
        .unwrap_err();
        assert!(absent.contains("no IncludeInCommandInterface"), "{absent}");
    }

    #[test]
    fn a_tree_past_the_nesting_budget_is_refused() {
        let mut images = vec![];
        let mut names = vec![];
        for level in 0..=SUBSYSTEM_TOPOLOGY_MAX_NESTING {
            let name = format!("S{level}");
            names.push(name);
        }
        let owned: Vec<Vec<String>> = (1..=names.len())
            .map(|depth| names[..depth].to_vec())
            .collect();
        for (index, path) in owned.iter().enumerate() {
            let child = names
                .get(index + 1)
                .map(|child| format!("<Subsystem>{child}</Subsystem>"))
                .unwrap_or_default();
            images.push((
                path.iter().map(String::as_str).collect::<Vec<_>>(),
                descriptor(path.last().unwrap(), "true", "", &child),
            ));
        }
        let error = build(configuration("<Subsystem>S0</Subsystem>"), images).unwrap_err();

        assert!(error.contains("nests deeper"), "{error}");
    }
}
