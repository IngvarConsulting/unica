//! Compare only choice settings of retained XML owners around a full pull.
use roxmltree::{Document, Node};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use unica_source_contract::read::SecureTreeCaptureLimits;
use unica_work_control::cancellation::CancellationToken;
use unica_workspace::read::visit_root_relative_regular_files_bounded;

const FORM_NS: &str = "http://v8.1c.ru/8.3/xcf/logform";
const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
const XR_NS: &str = "http://v8.1c.ru/8.3/xcf/readable";
const PROPERTIES: [&str; 3] = ["ListChoiceMode", "ChoiceList", "ChoiceParameterLinks"];

// resource-limit: pull.choice-warning-examples
// class: result
// unit: count
// scope: request
// value: 20
// resource: Примеры исчезнувших настроек выбора в квитанции полной выгрузки.
// status: unproven
// origin: #826
// problem: При большом числе изменений квитанция должна оставаться компактной.
// on-reach: квитанция содержит первые примеры и полное число найденных исчезновений
const MAX_EXAMPLES: usize = 20;

// resource-limit: pull.choice-scan-depth
// class: format
// unit: count
// scope: request
// value: 32
// resource: Глубина обхода дерева исходников при проверке настроек выбора.
// status: unproven
// origin: #826
// problem: Ограничивает обход дерева при нетипичной глубине каталогов.
// on-reach: квитанция сообщает о неполной проверке настроек
const MAX_DEPTH: usize = 32;

// resource-limit: pull.choice-scan-bytes
// class: step
// unit: bytes
// scope: request
// value: 33554432
// resource: Суммарный объём XML в одном снимке настроек выбора.
// status: unproven
// origin: #826
// problem: Чтение исходников перед выгрузкой обязано быть ограничено.
// on-reach: квитанция сообщает о неполной проверке настроек
const MAX_SCAN_BYTES: usize = 32 * 1024 * 1024;

// resource-limit: pull.choice-warning-identity-bytes
// class: result
// unit: bytes
// scope: request
// value: 512
// resource: Длина пути и имени владельца в одном примере исчезнувшей настройки.
// status: unproven
// origin: #826
// problem: Длинные имена из XML не должны раздувать квитанцию.
// on-reach: пример опускается, но исчезновение входит в общий счётчик
const MAX_EXAMPLE_IDENTITY_BYTES: usize = 512;

// resource-limit: pull.choice-owner-parts
// class: retention
// unit: count
// scope: request
// value: 64000
// resource: Сумма частей имён владельцев в одном снимке настроек.
// status: unproven
// origin: #826
// problem: Узлы XML могут многократно повторять имена предков в снимке.
// on-reach: квитанция сообщает о неполной проверке настроек
const MAX_OWNER_PARTS: usize = 64_000;

// resource-limit: pull.choice-xml-nodes
// class: format
// unit: count
// scope: file
// value: 200000
// resource: Число узлов, разбираемых в одном XML для проверки настроек.
// status: unproven
// origin: #826
// problem: Компактный XML может создавать много узлов в памяти.
// on-reach: файл пропускается с предупреждением о неполной проверке
const MAX_XML_NODES: u32 = 200_000;

type Owner = Vec<(String, String)>;
type Settings = [bool; PROPERTIES.len()];
type Owners = BTreeMap<Owner, Option<Settings>>;

#[derive(Default)]
pub(super) struct ChoiceSnapshot {
    files: BTreeMap<String, Owners>,
    incomplete: bool,
}

impl ChoiceSnapshot {
    pub(super) fn capture(root: &Path, target: &Path, cancellation: &CancellationToken) -> Self {
        let mut snapshot = Self::default();
        let mut remaining_parts = MAX_OWNER_PARTS;
        let walked = visit_root_relative_regular_files_bounded(
            root,
            target,
            SecureTreeCaptureLimits {
                maximum_depth: MAX_DEPTH,
            },
            Some(MAX_SCAN_BYTES),
            |path| {
                path.file_name()
                    .is_some_and(|name| !name.to_string_lossy().starts_with('.'))
            },
            |path| {
                path.ends_with("Ext/Form.xml")
                    || (path.extension().is_some_and(|ext| ext == "xml")
                        && path.parent().is_some_and(|parent| {
                            parent.file_name().is_some_and(|name| name == "Documents")
                        }))
            },
            |file| {
                match std::str::from_utf8(&file.bytes)
                    .ok()
                    .and_then(|xml| parse_owners(xml, &mut remaining_parts))
                {
                    Some((owners, incomplete)) => {
                        snapshot.incomplete |= incomplete;
                        snapshot.files.insert(file.logical_path, owners);
                    }
                    None => snapshot.incomplete = true,
                }
                Ok(())
            },
            || {
                if cancellation.is_cancelled() {
                    Err(io::Error::from(io::ErrorKind::Interrupted))
                } else {
                    Ok(())
                }
            },
        );
        if walked.is_err() {
            return Self {
                incomplete: true,
                ..Self::default()
            };
        }
        snapshot
    }

    pub(super) fn warnings(self, after: Self) -> Vec<Value> {
        let mut incomplete = self.incomplete || after.incomplete;
        let mut count = 0;
        let mut examples = Vec::new();
        for (path, owners) in self.files {
            let Some(new_owners) = after.files.get(&path) else {
                continue;
            };
            for (owner, old) in owners {
                let Some(old) = old else {
                    incomplete = true;
                    continue;
                };
                let Some(new) = new_owners.get(&owner) else {
                    continue;
                };
                let Some(new) = new else {
                    incomplete |= old.iter().any(|present| *present);
                    continue;
                };
                for (i, property) in PROPERTIES.iter().enumerate() {
                    if old[i] && !new[i] {
                        count += 1;
                        if examples.len() < MAX_EXAMPLES {
                            let at = owner
                                .iter()
                                .map(|(kind, name)| format!("{kind}.{name}"))
                                .collect::<Vec<_>>()
                                .join(".");
                            if path.len() + at.len() <= MAX_EXAMPLE_IDENTITY_BYTES {
                                examples
                                    .push(json!({"path": path, "owner": at, "property": property}));
                            }
                        }
                    }
                }
            }
        }
        let mut warnings = Vec::new();
        if count != 0 {
            warnings.push(json!({
                "code": "pull_choice_settings_disappeared",
                "message": "Choice settings present in the previous sources are absent or disabled after pull. Review the changes before committing.",
                "count": count,
                "examples": examples,
            }));
        }
        if incomplete {
            warnings.push(incomplete_warning());
        }
        warnings
    }
}

pub(super) fn incomplete_warning() -> Value {
    json!({
        "code": "pull_choice_settings_check_incomplete",
        "message": "Unica could not compare all supported choice settings before and after pull.",
    })
}

fn named_owner<'a>(node: Node<'a, '_>, namespace: &str) -> Option<&'a str> {
    if node.tag_name().namespace() != Some(namespace) {
        return None;
    }
    if namespace == FORM_NS {
        return node.attribute("name").filter(|name| !name.is_empty());
    }
    child(node, MD_NS, "Properties")
        .and_then(|properties| child(properties, MD_NS, "Name"))
        .and_then(|name| name.text())
        .filter(|name| !name.is_empty())
}

fn child<'a, 'input>(node: Node<'a, 'input>, ns: &str, name: &str) -> Option<Node<'a, 'input>> {
    node.children().find(|child| child.has_tag_name((ns, name)))
}

fn parse_owners(xml: &str, remaining_parts: &mut usize) -> Option<(Owners, bool)> {
    let options = roxmltree::ParsingOptions {
        nodes_limit: MAX_XML_NODES,
        ..Default::default()
    };
    let doc = Document::parse_with_options(xml.trim_start_matches('\u{feff}'), options).ok()?;
    let root = doc.root_element();
    let namespace = if root.has_tag_name((FORM_NS, "Form")) {
        FORM_NS
    } else if root.has_tag_name((MD_NS, "MetaDataObject"))
        && child(root, MD_NS, "Document").is_some()
    {
        MD_NS
    } else {
        return None;
    };
    let mut owners = Owners::new();
    let mut incomplete = false;
    for node in root.descendants().filter(|node| node.is_element()) {
        if (namespace == MD_NS && node.tag_name().name() != "Attribute")
            || (namespace == FORM_NS && node.attribute("id").is_none())
        {
            continue;
        }
        let Some(name) = named_owner(node, namespace) else {
            continue;
        };
        let mut owner = Vec::new();
        let mut identity_bytes = 0;
        for (kind, name) in std::iter::once((node.tag_name().name(), name)).chain(
            node.ancestors().skip(1).filter_map(|parent| {
                named_owner(parent, namespace).map(|name| (parent.tag_name().name(), name))
            }),
        ) {
            identity_bytes += kind.len() + name.len() + 2;
            if identity_bytes > MAX_EXAMPLE_IDENTITY_BYTES {
                break;
            }
            owner.push((kind.to_owned(), name.to_owned()));
        }
        if identity_bytes > MAX_EXAMPLE_IDENTITY_BYTES {
            incomplete = true;
            continue;
        }
        if *remaining_parts < owner.len() {
            incomplete = true;
            break;
        }
        *remaining_parts -= owner.len();
        owner.reverse();
        let properties = if namespace == MD_NS {
            child(node, MD_NS, "Properties")?
        } else {
            node
        };
        let list_mode = child(properties, namespace, "ListChoiceMode")
            .and_then(|mode| mode.text())
            .is_some_and(|value| matches!(value.trim(), "true" | "1"));
        let nonempty = |property, entry| {
            child(properties, namespace, property)
                .is_some_and(|value| child(value, XR_NS, entry).is_some())
        };
        let settings = [
            list_mode,
            nonempty("ChoiceList", "Item"),
            nonempty("ChoiceParameterLinks", "Link"),
        ];
        owners
            .entry(owner)
            .and_modify(|old| *old = None)
            .or_insert(Some(settings));
    }
    Some((owners, incomplete))
}
