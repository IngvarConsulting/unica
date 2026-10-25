//! Bounded metadata reference/subsystem removal planning over issued read observations.
use roxmltree::Document;
use std::collections::HashSet;
use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use unica_source_contract::discovery::{DiscoveryMetadata, FollowMode, ProjectDiscoveryRead};
use unica_source_designer::read::{file_stem_string, read_utf8_sig_snapshot, relative_display};

use unica_source_contract::proposal::DirectoryTopologyEntryKind;

pub struct MetaRemoveSubsystemReplacement {
    pub path: PathBuf,
    pub original: Vec<u8>,
    pub replacement: Vec<u8>,
}

pub struct MetaRemoveTextRead {
    pub path: PathBuf,
    pub raw: Vec<u8>,
}

// #1119: the reference scan visits the whole source root, however many
// entries the configuration holds; depth still guards against cycles.
const META_REMOVE_MAX_TRAVERSAL_DEPTH: usize = 256;
const META_REMOVE_MAX_TRAVERSAL_ENTRIES: usize = usize::MAX;

#[derive(Clone, Copy)]
pub struct MetaRemoveTraversalLimits {
    pub max_depth: usize,
    pub max_entries: usize,
}

fn meta_remove_preserved_utf8_image(original: &[u8], updated: String) -> Vec<u8> {
    let mut image = Vec::with_capacity(original.len());
    if original.starts_with(b"\xef\xbb\xbf") {
        image.extend_from_slice(b"\xef\xbb\xbf");
    }
    image.extend_from_slice(updated.as_bytes());
    image
}

/// How a composition entry names the object it lists.
#[derive(Clone, Copy)]
pub enum CompositionKey {
    /// The entry's own text: `<Item>Catalog.X</Item>`.
    Text,
    /// An attribute of the entry: `<Command name="Catalog.X.StandardCommand.OpenList">`.
    Attribute(&'static str),
    /// The text of a child element: `<object><name>Catalog.X</name>…</object>`.
    Child(&'static str),
}

/// One kind of entry a composition lists, optionally only directly under
/// a parent element of the given local name.
#[derive(Clone, Copy)]
pub struct CompositionRule {
    pub entry: &'static str,
    pub parent: Option<&'static str>,
    pub key: CompositionKey,
}

const SUBSYSTEM_CONTENT: &[CompositionRule] = &[CompositionRule {
    entry: "Item",
    parent: Some("Content"),
    key: CompositionKey::Text,
}];

/// The compositions that list other objects and are cleaned with the
/// removed object, as the configurator cleans them, and the property each
/// one is reported as. Subsystem descriptors are walked separately by
/// [`plan_meta_remove_subsystem_replacements`].
pub fn meta_remove_composition(
    relative: &Path,
) -> Option<(&'static [CompositionRule], &'static str)> {
    const RIGHTS: &[CompositionRule] = &[CompositionRule {
        entry: "object",
        parent: None,
        key: CompositionKey::Child("name"),
    }];
    const COMMAND_INTERFACE: &[CompositionRule] = &[
        CompositionRule {
            entry: "Command",
            parent: None,
            key: CompositionKey::Attribute("name"),
        },
        CompositionRule {
            entry: "Subsystem",
            parent: None,
            key: CompositionKey::Text,
        },
        CompositionRule {
            entry: "Value",
            parent: None,
            key: CompositionKey::Attribute("name"),
        },
    ];
    const HOME_PAGE: &[CompositionRule] = &[CompositionRule {
        entry: "Item",
        parent: None,
        key: CompositionKey::Child("Form"),
    }];
    const EXCHANGE_CONTENT: &[CompositionRule] = &[CompositionRule {
        entry: "Item",
        parent: None,
        key: CompositionKey::Child("Metadata"),
    }];
    const FUNCTIONAL_OPTION_CONTENT: &[CompositionRule] = &[CompositionRule {
        entry: "Object",
        parent: Some("Content"),
        key: CompositionKey::Text,
    }];
    let parts = relative
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>();
    match parts.as_slice() {
        ["Roles", _, "Ext", "Rights.xml"] => Some((RIGHTS, "Rights")),
        [.., "Ext", "CommandInterface.xml"] => Some((COMMAND_INTERFACE, "CommandInterface")),
        ["Ext", "MainSectionCommandInterface.xml"] => {
            Some((COMMAND_INTERFACE, "MainSectionCommandInterface"))
        }
        ["Ext", "HomePageWorkArea.xml"] => Some((HOME_PAGE, "HomePageWorkArea")),
        ["ExchangePlans", _, "Ext", "Content.xml"] => Some((EXCHANGE_CONTENT, "Content")),
        ["FunctionalOptions", descriptor] if descriptor.ends_with(".xml") => {
            Some((FUNCTIONAL_OPTION_CONTENT, "Content"))
        }
        _ => None,
    }
}

/// Removes every entry the rules recognise that names `qualified_object_name`
/// or one of its children (`Catalog.X.Attribute.Y`), with the line it stood
/// on; the rest of the text is kept byte for byte. The text has no BOM.
pub fn remove_composition_entries(
    xml_text: &str,
    rules: &[CompositionRule],
    qualified_object_name: &str,
) -> Result<(String, usize), String> {
    let document =
        Document::parse(xml_text).map_err(|error| format!("XML parse error: {error}"))?;
    let child_prefix = format!("{qualified_object_name}.");
    let names_object = |value: &str| {
        let value = value.trim();
        value == qualified_object_name || value.starts_with(&child_prefix)
    };
    let mut ranges = document
        .descendants()
        .filter(|node| node.is_element())
        .filter(|node| {
            rules.iter().any(|rule| {
                node.tag_name().name() == rule.entry
                    && rule.parent.is_none_or(|parent| {
                        node.parent_element()
                            .is_some_and(|element| element.tag_name().name() == parent)
                    })
                    && match rule.key {
                        CompositionKey::Text => node.text().is_some_and(names_object),
                        CompositionKey::Attribute(name) => {
                            node.attribute(name).is_some_and(names_object)
                        }
                        CompositionKey::Child(name) => node
                            .children()
                            .find(|child| child.is_element() && child.tag_name().name() == name)
                            .and_then(|child| child.text())
                            .is_some_and(names_object),
                    }
            })
        })
        .map(|node| node.range())
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| range.start);
    // An entry inside another removed entry goes with it.
    let mut outer: Vec<std::ops::Range<usize>> = Vec::new();
    for range in ranges {
        if outer.last().is_none_or(|last| range.start >= last.end) {
            outer.push(range);
        }
    }
    let removed = outer.len();
    let mut updated = xml_text.to_string();
    for range in outer.into_iter().rev() {
        let line_start = updated[..range.start]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        let leading_is_indent = updated[line_start..range.start]
            .chars()
            .all(|character| matches!(character, ' ' | '\t' | '\r'));
        let removal = if leading_is_indent && updated[range.end..].starts_with("\r\n") {
            line_start..range.end + 2
        } else if leading_is_indent && updated[range.end..].starts_with('\n') {
            line_start..range.end + 1
        } else {
            range
        };
        updated.replace_range(removal, "");
    }
    Ok((updated, removed))
}

/// The image of a cleaned composition, keeping the BOM of the original.
pub fn meta_remove_composition_image(original: &[u8], updated: String) -> Vec<u8> {
    meta_remove_preserved_utf8_image(original, updated)
}

fn meta_remove_path_is_link_or_reparse_point(
    path: &Path,
    metadata: &DiscoveryMetadata,
    hooks: RemoveObservationHooks,
) -> bool {
    metadata.is_link_or_reparse() || (hooks.additional_reparse_observation)(path)
}

/// Additional observations used by the existing deterministic source-race tests.
/// Product bindings use `NONE`; the actual no-follow metadata rejection always applies.
#[derive(Clone, Copy)]
pub struct RemoveObservationHooks {
    pub before_child_inspection: fn(&Path),
    pub additional_reparse_observation: fn(&Path) -> bool,
}
impl RemoveObservationHooks {
    pub const NONE: Self = Self {
        before_child_inspection: |_| {},
        additional_reparse_observation: |_| false,
    };
}

fn require_meta_remove_real_path(
    hooks: RemoveObservationHooks,
    path: &Path,
    metadata: &DiscoveryMetadata,
    role: &str,
) -> Result<(), String> {
    if meta_remove_path_is_link_or_reparse_point(path, metadata, hooks) {
        Err(format!(
            "{role} must not be a symbolic link or reparse point: {}",
            path.display()
        ))
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct MetaRemoveDirectoryWalkPolicy {
    limits: MetaRemoveTraversalLimits,
    label: &'static str,
    allow_absent_root: bool,
}

type InspectedMetaRemoveDirectoryEntry = (PathBuf, OsString, DirectoryTopologyEntryKind);

fn inspect_meta_remove_directory(
    reader: &dyn ProjectDiscoveryRead,
    hooks: RemoveObservationHooks,
    dir: &Path,
    depth: usize,
    policy: MetaRemoveDirectoryWalkPolicy,
    visited_directories: &mut HashSet<PathBuf>,
    visited_entries: &mut usize,
) -> Result<Option<Vec<InspectedMetaRemoveDirectoryEntry>>, String> {
    let limits = policy.limits;

    if depth > limits.max_depth {
        return Err(format!(
            "{} traversal exceeded the maximum depth of {}: {}",
            policy.label,
            limits.max_depth,
            dir.display()
        ));
    }
    let metadata = match reader.inspect_input(dir, FollowMode::NoFollow) {
        Ok(metadata) => metadata,
        Err(error)
            if policy.allow_absent_root && depth == 0 && error.kind() == ErrorKind::NotFound =>
        {
            return Ok(None);
        }
        Err(error) => {
            return Err(format!(
                "failed to inspect {} directory {}: {error}",
                policy.label,
                dir.display()
            ));
        }
    };
    require_meta_remove_real_path(
        hooks,
        dir,
        &metadata,
        &format!("{} directory", policy.label),
    )?;
    if !metadata.is_dir() {
        return Err(format!(
            "{} path is not a directory: {}",
            policy.label,
            dir.display()
        ));
    }
    let directory_identity = reader.canonicalize(dir).map_err(|error| {
        format!(
            "failed to resolve {} directory identity {}: {error}",
            policy.label,
            dir.display()
        )
    })?;
    if !visited_directories.insert(directory_identity) {
        return Err(format!(
            "{} traversal directory cycle or duplicate identity detected before traversal: {}",
            policy.label,
            dir.display()
        ));
    }

    let directory_entries = reader.read_entries(dir).map_err(|error| {
        format!(
            "failed to read {} directory {}: {error}",
            policy.label,
            dir.display()
        )
    })?;
    let mut entries = Vec::new();
    for entry in directory_entries {
        let entry = entry.map_err(|error| {
            format!(
                "failed to read an entry in {} directory {}: {error}",
                policy.label,
                dir.display()
            )
        })?;
        if *visited_entries >= limits.max_entries {
            return Err(format!(
                "{} traversal exceeded the maximum of {} entries: {}",
                policy.label,
                limits.max_entries,
                dir.display()
            ));
        }
        *visited_entries += 1;
        entries.push(entry);
    }
    entries.sort_by_key(|entry| entry.file_name());

    entries
        .into_iter()
        .map(|entry| {
            let path = entry.path();
            let metadata = reader
                .inspect_input(&path, FollowMode::NoFollow)
                .map_err(|error| {
                    format!(
                        "failed to inspect {} entry {}: {error}",
                        policy.label,
                        path.display()
                    )
                })?;
            require_meta_remove_real_path(
                hooks,
                &path,
                &metadata,
                &format!("{} entry", policy.label),
            )?;
            let kind = if metadata.is_dir() {
                DirectoryTopologyEntryKind::Directory
            } else if metadata.is_file() {
                DirectoryTopologyEntryKind::File
            } else {
                return Err(format!(
                    "{} entry has an unsupported filesystem type: {}",
                    policy.label,
                    path.display()
                ));
            };
            Ok((path, entry.file_name(), kind))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

pub fn plan_meta_remove_subsystem_replacements(
    reader: &dyn ProjectDiscoveryRead,
    hooks: RemoveObservationHooks,
    dir: &Path,
    qualified_object_name: &str,
    replacements: &mut Vec<MetaRemoveSubsystemReplacement>,
    descriptor_reads: &mut Vec<MetaRemoveTextRead>,
) -> Result<(), String> {
    let mut visited_directories = HashSet::new();
    let mut visited_entries = 0usize;
    plan_meta_remove_subsystem_replacements_bounded(
        reader,
        hooks,
        dir,
        qualified_object_name,
        replacements,
        descriptor_reads,
        0,
        MetaRemoveTraversalLimits {
            max_depth: META_REMOVE_MAX_TRAVERSAL_DEPTH,
            max_entries: META_REMOVE_MAX_TRAVERSAL_ENTRIES,
        },
        &mut visited_directories,
        &mut visited_entries,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn plan_meta_remove_subsystem_replacements_bounded(
    reader: &dyn ProjectDiscoveryRead,
    hooks: RemoveObservationHooks,
    dir: &Path,
    qualified_object_name: &str,
    replacements: &mut Vec<MetaRemoveSubsystemReplacement>,
    descriptor_reads: &mut Vec<MetaRemoveTextRead>,
    depth: usize,
    limits: MetaRemoveTraversalLimits,
    visited_directories: &mut HashSet<PathBuf>,
    visited_entries: &mut usize,
) -> Result<(), String> {
    let Some(entries) = inspect_meta_remove_directory(
        reader,
        hooks,
        dir,
        depth,
        MetaRemoveDirectoryWalkPolicy {
            limits,
            label: "subsystem",
            allow_absent_root: false,
        },
        visited_directories,
        visited_entries,
    )?
    else {
        return Err(format!(
            "subsystem directory disappeared before traversal: {}",
            dir.display()
        ));
    };
    let descriptors = entries
        .into_iter()
        .filter_map(|(path, _, kind)| {
            let is_xml = path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"));
            (is_xml && kind == DirectoryTopologyEntryKind::File).then_some(path)
        })
        .collect::<Vec<_>>();

    for path in descriptors {
        let snapshot = read_utf8_sig_snapshot(reader, &path)?;
        let (updated, removed_references) =
            remove_composition_entries(&snapshot.text, SUBSYSTEM_CONTENT, qualified_object_name)?;
        descriptor_reads.push(MetaRemoveTextRead {
            path: path.clone(),
            raw: snapshot.raw.clone(),
        });

        let child_dir = path
            .parent()
            .unwrap_or(dir)
            .join(file_stem_string(&path))
            .join("Subsystems");
        if removed_references > 0 {
            let replacement = meta_remove_preserved_utf8_image(&snapshot.raw, updated);
            replacements.push(MetaRemoveSubsystemReplacement {
                path: path.clone(),
                original: snapshot.raw,
                replacement,
            });
        }

        (hooks.before_child_inspection)(&child_dir);
        match reader.inspect_input(&child_dir, FollowMode::NoFollow) {
            Ok(metadata) => {
                require_meta_remove_real_path(hooks, &child_dir, &metadata, "subsystem directory")?;
                if metadata.is_dir() {
                    if depth >= limits.max_depth {
                        return Err(format!(
                            "subsystem traversal exceeded the maximum depth of {}: {}",
                            limits.max_depth,
                            child_dir.display()
                        ));
                    }
                    plan_meta_remove_subsystem_replacements_bounded(
                        reader,
                        hooks,
                        &child_dir,
                        qualified_object_name,
                        replacements,
                        descriptor_reads,
                        depth + 1,
                        limits,
                        visited_directories,
                        visited_entries,
                    )?;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(format!("failed to inspect {}: {err}", child_dir.display()));
            }
        }
    }

    Ok(())
}

pub fn meta_remove_should_skip_file(
    file: &Path,
    config_dir: &Path,
    obj_xml: &Path,
    obj_dir: &Path,
    has_xml: bool,
    has_dir: bool,
) -> bool {
    if has_xml && file == obj_xml {
        return true;
    }
    if has_dir && (file == obj_dir || file.starts_with(obj_dir)) {
        return true;
    }
    let relative = file.strip_prefix(config_dir).ok();
    let rel = relative_display(file, config_dir);
    rel == "Configuration.xml"
        || rel == "ConfigDumpInfo.xml"
        || relative.is_some_and(|path| {
            path.components().next()
                == Some(std::path::Component::Normal(std::ffi::OsStr::new(
                    "Subsystems",
                )))
        })
}

pub fn meta_remove_search_patterns(
    obj_type: &str,
    obj_name: &str,
    type_plural: &str,
) -> Vec<String> {
    let mut patterns = Vec::new();
    if let Some(ref_names) = meta_remove_type_ref_names(obj_type) {
        patterns.extend(ref_names.iter().map(|name| format!("{name}.{obj_name}")));
    }
    if let Some(manager) = meta_remove_ru_manager(obj_type) {
        patterns.push(format!("{manager}.{obj_name}"));
    }
    patterns.push(format!("{type_plural}.{obj_name}"));
    if obj_type == "CommonModule" {
        // Module code calls a common module by its bare name; metadata names
        // it `CommonModule.X`, which the caller adds for XML.
        patterns.push(format!("{obj_name}."));
    }
    if obj_type == "CommonTemplate" {
        // No manager names a common template; `Метаданные.ОбщиеМакеты.X` does.
        patterns.push(format!("ОбщиеМакеты.{obj_name}"));
    }
    patterns
}

pub fn meta_remove_type_ref_names(obj_type: &str) -> Option<&'static [&'static str]> {
    match obj_type {
        "Catalog" => Some(&["CatalogRef", "CatalogObject"]),
        "Document" => Some(&["DocumentRef", "DocumentObject"]),
        "Enum" => Some(&["EnumRef"]),
        "ExchangePlan" => Some(&["ExchangePlanRef", "ExchangePlanObject"]),
        "ChartOfAccounts" => Some(&["ChartOfAccountsRef", "ChartOfAccountsObject"]),
        "ChartOfCharacteristicTypes" => Some(&[
            "ChartOfCharacteristicTypesRef",
            "ChartOfCharacteristicTypesObject",
        ]),
        "ChartOfCalculationTypes" => Some(&[
            "ChartOfCalculationTypesRef",
            "ChartOfCalculationTypesObject",
        ]),
        "BusinessProcess" => Some(&["BusinessProcessRef", "BusinessProcessObject"]),
        "Task" => Some(&["TaskRef", "TaskObject"]),
        _ => None,
    }
}

pub fn meta_remove_ru_manager(obj_type: &str) -> Option<&'static str> {
    match obj_type {
        "Catalog" => Some("Справочники"),
        "Document" => Some("Документы"),
        "Enum" => Some("Перечисления"),
        "Constant" => Some("Константы"),
        "InformationRegister" => Some("РегистрыСведений"),
        "AccumulationRegister" => Some("РегистрыНакопления"),
        "AccountingRegister" => Some("РегистрыБухгалтерии"),
        "CalculationRegister" => Some("РегистрыРасчета"),
        "ChartOfAccounts" => Some("ПланыСчетов"),
        "ChartOfCharacteristicTypes" => Some("ПланыВидовХарактеристик"),
        "ChartOfCalculationTypes" => Some("ПланыВидовРасчета"),
        "BusinessProcess" => Some("БизнесПроцессы"),
        "Task" => Some("Задачи"),
        "ExchangePlan" => Some("ПланыОбмена"),
        "Report" => Some("Отчеты"),
        "DataProcessor" => Some("Обработки"),
        "DocumentJournal" => Some("ЖурналыДокументов"),
        _ => None,
    }
}

/// One place outside the removed object that names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetaRemoveReference {
    /// Another metadata object names it in a property of its XML source.
    Metadata { at: String, property: String },
    /// Module code names it on a line.
    Code { at: String, line: usize },
}

/// The references to the removed object found in one source file, addressed
/// logically: XML is metadata and refuses the removal, BSL is module code.
/// The text of a query held in XML — a dynamic list's `QueryText`, a data
/// composition `query` — is code too: it is searched with `query_patterns`
/// and reported by the line inside the query. A design-time value of data
/// composition settings is metadata searched with `query_patterns` too: the
/// dump writes it in the configuration's language.
pub fn meta_remove_references(
    source_set: &str,
    relative: &Path,
    text: &str,
    patterns: &[String],
    query_patterns: &[String],
) -> Vec<MetaRemoveReference> {
    let is_bsl = relative
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("bsl"));
    let (owner, descriptor) = meta_remove_referrer(relative);
    let mut found = Vec::new();
    if is_bsl {
        let module = unica_source_designer::module_layout::platform_xml_module_identity(relative)
            .ok()
            .and_then(|identity| {
                unica_source_designer::module_layout::logical_module_from_metadata(
                    identity.address.as_str(),
                )
            })
            .unwrap_or(owner);
        for (index, line) in text.lines().enumerate() {
            if patterns.iter().any(|pattern| mentions(line, pattern)) {
                found.push(MetaRemoveReference::Code {
                    at: format!("{source_set}:{module}"),
                    line: index + 1,
                });
            }
        }
        return found;
    }
    let property_of_file = file_stem_string(relative);
    let Ok(document) = Document::parse(text.trim_start_matches('\u{feff}')) else {
        if patterns.iter().any(|pattern| mentions(text, pattern)) {
            found.push(MetaRemoveReference::Metadata {
                at: format!("{source_set}:{owner}"),
                property: property_of_file,
            });
        }
        return found;
    };
    // A reference is a text or an attribute value: `<v8:Type>cfg:CatalogRef.X</v8:Type>`,
    // `<xr:Value name="Role.X">` or `<Command name="Catalog.X.StandardCommand.OpenList">`.
    let file_name = relative.file_name().and_then(|name| name.to_str());
    for node in document.descendants() {
        let holds_query = node.is_text()
            && node
                .parent_element()
                .is_some_and(|parent| query_text_element(file_name, parent));
        if holds_query {
            let at = query_text_address(source_set, &owner, node);
            for (index, line) in node.text().unwrap_or_default().lines().enumerate() {
                let reference = MetaRemoveReference::Code {
                    at: at.clone(),
                    line: index + 1,
                };
                if query_patterns.iter().any(|pattern| mentions(line, pattern))
                    && !found.contains(&reference)
                {
                    found.push(reference);
                }
            }
            continue;
        }
        // A design-time value — the value of a filter, a parameter or a
        // choice parameter in data composition settings — names a predefined
        // item or an enum value the way the dump is written, `Справочник.X.Y`
        // in a Russian configuration, so it is searched in both languages.
        if let Some(value) = node
            .parent_element()
            .filter(|parent| node.is_text() && design_time_value(*parent))
        {
            let text = node.text().unwrap_or_default();
            let reference = MetaRemoveReference::Metadata {
                at: query_text_address(source_set, &owner, node),
                property: design_time_property(value),
            };
            if query_patterns.iter().any(|pattern| mentions(text, pattern))
                && !found.contains(&reference)
            {
                found.push(reference);
            }
            continue;
        }
        let named = if node.is_text() {
            node.text()
                .is_some_and(|value| patterns.iter().any(|pattern| mentions(value, pattern)))
        } else {
            node.attributes().any(|attribute| {
                patterns
                    .iter()
                    .any(|pattern| mentions(attribute.value(), pattern))
            })
        };
        if !named {
            continue;
        }
        let reference = if descriptor {
            MetaRemoveReference::Metadata {
                at: nested_referrer(source_set, &owner, node),
                property: descriptor_property(node),
            }
        } else {
            MetaRemoveReference::Metadata {
                at: format!("{source_set}:{owner}"),
                property: property_of_file.clone(),
            }
        };
        if !found.contains(&reference) {
            found.push(reference);
        }
    }
    found
}

/// The namespaces of data composition, `…/data-composition-system/schema`,
/// `…/settings`, `…/common`, `…/area-template` and the others.
const DATA_COMPOSITION_NAMESPACE: &str = "http://v8.1c.ru/8.1/data-composition-system/";

/// The namespace of a data composition schema.
const DATA_COMPOSITION_SCHEMA_NAMESPACE: &str =
    "http://v8.1c.ru/8.1/data-composition-system/schema";

/// Whether an element of an XML source holds query-language text, which is
/// searched like code: a dynamic list's `QueryText` and a data composition
/// `query` anywhere, every expression of data composition — an element of a
/// data composition namespace named `expression` or `…Expression`, in a
/// template or in a dynamic list of a form — and the access restriction
/// `condition` of role rights.
fn query_text_element(file_name: Option<&str>, element: roxmltree::Node<'_, '_>) -> bool {
    let name = element.tag_name().name();
    match name {
        "QueryText" | "query" => true,
        "condition" => file_name == Some("Rights.xml"),
        _ => {
            matches!(file_name, Some("Template.xml" | "Form.xml"))
                && (name == "expression" || name.ends_with("Expression"))
                && element
                    .tag_name()
                    .namespace()
                    .is_some_and(|namespace| namespace.starts_with(DATA_COMPOSITION_NAMESPACE))
        }
    }
}

/// Whether an element holds a data composition design-time value,
/// `xsi:type="dcscor:DesignTimeValue"`.
fn design_time_value(element: roxmltree::Node<'_, '_>) -> bool {
    element
        .attribute(("http://www.w3.org/2001/XMLSchema-instance", "type"))
        .and_then(|value| value.split_once(':'))
        .is_some_and(|(prefix, name)| {
            name == "DesignTimeValue"
                && element.lookup_namespace_uri(Some(prefix))
                    == Some("http://v8.1c.ru/8.1/data-composition-system/core")
        })
}

/// The property holding a design-time value: the outermost settings section
/// around it — `Filter`, `ConditionalAppearance` of a list or of the whole
/// form, `DataParameters`, the `InputParameters` or `AvailableValue` of a
/// field or parameter — or the element itself, `Value` of a schema parameter.
fn design_time_property(value: roxmltree::Node<'_, '_>) -> String {
    let section = value
        .ancestors()
        .filter(|ancestor| {
            matches!(
                ancestor.tag_name().name(),
                "filter"
                    | "conditionalAppearance"
                    | "ConditionalAppearance"
                    | "dataParameters"
                    | "inputParameters"
                    | "availableValue"
                    | "userFields"
            )
        })
        .last()
        .unwrap_or(value);
    let name = section.tag_name().name();
    let mut chars = name.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// The address of the query text holding a reference: the form attribute of
/// a dynamic list, the data set, field, calculated field, total, parameter or
/// settings variant of a data composition schema, the right entry of a role, as far as the
/// address grammar names it; otherwise the form, template or role that owns
/// the file. The grammar has no node for a nested schema or an output
/// template of the schema, so whatever lies inside one is addressed by the
/// template.
fn query_text_address(source_set: &str, owner: &str, node: roxmltree::Node<'_, '_>) -> String {
    let child_text = |element: roxmltree::Node<'_, '_>, name: &str| {
        element
            .children()
            .find(|child| child.is_element() && child.tag_name().name() == name)
            .and_then(|child| child.text())
            .map(|text| text.trim().to_string())
    };
    let mut segments = Vec::new();
    for ancestor in node.ancestors().filter(|ancestor| ancestor.is_element()) {
        // A nested schema and an output template of the schema have no node
        // in the address grammar; their parameters are not the schema's.
        if ancestor.tag_name().namespace() == Some(DATA_COMPOSITION_SCHEMA_NAMESPACE)
            && matches!(ancestor.tag_name().name(), "nestedSchema" | "template")
        {
            segments.clear();
            continue;
        }
        let member = |kind: &str, name: Option<String>| {
            name.map(|name| unica_address::address::member_address(&format!(".{kind}"), &name))
        };
        let segment = match ancestor.tag_name().name() {
            "Attribute" => member("Attribute", ancestor.attribute("name").map(str::to_string)),
            "dataSet" => member("DataSet", child_text(ancestor, "name")),
            "settingsVariant" => member("Setting", child_text(ancestor, "name")),
            "field" => member("Field", child_text(ancestor, "dataPath")),
            "calculatedField" => member("CalculatedField", child_text(ancestor, "dataPath")),
            "totalField" => member("Total", child_text(ancestor, "dataPath")),
            "parameter" => member("Parameter", child_text(ancestor, "name")),
            // A role's right entry is addressed `Right.<Kind>_<Name…>`, as
            // `right.set` addresses it.
            "object" => member(
                "Right",
                child_text(ancestor, "name").map(|object| object.replacen('.', "_", 1)),
            ),
            _ => None,
        };
        segments.extend(segment);
    }
    segments.reverse();
    while !segments.is_empty() {
        let candidate = format!("{source_set}:{owner}{}", segments.concat());
        if unica_address::address::QualifiedAddress::parse(&candidate).is_ok() {
            return candidate;
        }
        segments.pop();
    }
    format!("{source_set}:{owner}")
}

/// The metadata object a source file belongs to and whether the file is its
/// own descriptor: `Catalogs/X.xml` and `Catalogs/X/Forms/F.xml` are
/// descriptors, `Catalogs/X/Forms/F/Ext/Form.xml` belongs to `Catalog.X.Form.F`.
/// Files outside a metadata collection belong to the configuration.
pub fn meta_remove_referrer(relative: &Path) -> (String, bool) {
    let parts = relative
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>();
    let layout = parts
        .first()
        .and_then(|directory| unica_source_designer::layout::metadata_kind_by_directory(directory));
    let (Some(layout), Some(owner)) = (layout, parts.get(1)) else {
        return ("Configuration".to_string(), false);
    };
    let strip = |part: &str| part.strip_suffix(".xml").unwrap_or(part).to_string();
    let mut address = format!("{}.{}", layout.tag, strip(owner));
    let mut rest = &parts[2..];
    while let [directory, child, tail @ ..] = rest {
        let kind = match *directory {
            "Forms" => "Form",
            "Templates" => "Template",
            "Commands" => "Command",
            "Recalculations" => "Recalculation",
            "Subsystems" => "Subsystem",
            _ => break,
        };
        address.push_str(&format!(".{kind}.{}", strip(child)));
        rest = tail;
    }
    (address, rest.is_empty())
}

/// The descriptor node holding the reference: the owner, or the attribute,
/// tabular section, dimension and so on it is declared in, as long as the
/// address grammar names that node.
fn nested_referrer(source_set: &str, owner: &str, node: roxmltree::Node<'_, '_>) -> String {
    let mut chain = declared_chain(node);
    while !chain.is_empty() {
        let candidate = format!("{source_set}:{owner}{}", chain.concat());
        if unica_address::address::QualifiedAddress::parse(&candidate).is_ok() {
            return candidate;
        }
        chain.pop();
    }
    format!("{source_set}:{owner}")
}

/// The name a descriptor node declares in its `<Properties><Name>`.
pub fn declared_name(node: roxmltree::Node<'_, '_>) -> Option<String> {
    node.children()
        .find(|child| child.has_tag_name("Properties"))?
        .children()
        .find(|child| child.has_tag_name("Name"))?
        .text()
        .map(|name| name.trim().to_string())
}

/// The `.Kind.Name` segments of the declared descriptor nodes from below the
/// described object down to `node` itself: an attribute of a tabular section
/// is `[".TabularSection.T", ".Attribute.A"]`, the object itself is `[]`.
pub fn declared_chain(node: roxmltree::Node<'_, '_>) -> Vec<String> {
    let mut chain = node
        .ancestors()
        .filter(|ancestor| ancestor.is_element())
        .filter_map(|ancestor| {
            declared_name(ancestor).map(|name| format!(".{}.{name}", ancestor.tag_name().name()))
        })
        .collect::<Vec<_>>();
    // The outermost declared node is the described object itself.
    chain.pop();
    chain.reverse();
    chain
}

/// The property of the descriptor node that holds the reference: the element
/// right under `<Properties>`, or the enclosing element when there is none.
fn descriptor_property(node: roxmltree::Node<'_, '_>) -> String {
    let holder = if node.is_element() {
        Some(node)
    } else {
        node.parent_element()
    };
    holder
        .into_iter()
        .flat_map(|holder| holder.ancestors())
        .filter(|ancestor| ancestor.is_element())
        .find(|ancestor| {
            ancestor
                .parent_element()
                .is_some_and(|parent| parent.has_tag_name("Properties"))
        })
        .or(holder)
        .map(|element| element.tag_name().name().to_string())
        .unwrap_or_default()
}

/// Whether `text` names the object by `pattern` as a whole name: `Catalog.X`
/// does not match `Catalog.XY` or `MyCatalog.X`. The bare common-module
/// pattern `X.` must not follow a dot, where it names a member of another object.
fn mentions(text: &str, pattern: &str) -> bool {
    let identifier = |ch: char| ch.is_alphanumeric() || ch == '_';
    let bare = pattern.ends_with('.') && !pattern[..pattern.len() - 1].contains('.');
    text.match_indices(pattern).any(|(start, _)| {
        let before = text[..start].chars().next_back();
        let after = text[start + pattern.len()..].chars().next();
        !before.is_some_and(|ch| identifier(ch) || (bare && ch == '.'))
            && (pattern.ends_with('.') || !after.is_some_and(identifier))
    })
}
