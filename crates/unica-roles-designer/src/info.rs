//! Actual role XML projections and validation over an issued ordinary reader.
use roxmltree::Document;
use unica_metadata::support_state::ObjectSupportData as DomainObjectSupportData;
use unica_roles::{info::*, nested_right_shape_is_modelled, validate_nested_right};
use unica_source_designer::layout::role_read_layout;
#[derive(Clone)]
pub struct RoleRight {
    pub name: String,
    pub value: String,
    pub condition: Option<String>,
}
fn role_group_data(groups: Vec<RoleInfoGroup>) -> Vec<RoleGroupData> {
    groups
        .into_iter()
        .map(|group| RoleGroupData {
            kind: group.type_prefix,
            objects: group
                .objects
                .into_iter()
                .map(|object| RoleObjectData {
                    name: object.short_name,
                    rights: object
                        .rights
                        .into_iter()
                        .map(|right| RoleRightData {
                            name: right.name,
                            restricted: right.rls,
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect()
}
fn role_attribute(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}
pub fn parse_role_info_xml(
    rights_text: &str,
    descriptor_text: Option<&str>,
    fallback_name: &str,
    support: DomainObjectSupportData,
) -> Result<RoleInfoData, String> {
    let (role_name, role_synonym) = role_info_metadata_xml(descriptor_text, fallback_name);
    let doc = Document::parse(rights_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error in Rights.xml: {err}"))?;
    let root = doc.root_element();
    let set_for_new = root.attribute("setForNewObjects").unwrap_or("");
    let set_for_attrs = root.attribute("setForAttributesByDefault").unwrap_or("");
    let independent_child = root
        .attribute("independentRightsOfChildObjects")
        .unwrap_or("");
    let mut allowed = Vec::<RoleInfoGroup>::new();
    let mut denied = Vec::<RoleInfoGroup>::new();
    let mut rls_objects = Vec::<String>::new();
    let mut total_allowed = 0usize;
    let mut total_denied = 0usize;
    for obj in root.children().filter(|node| {
        unica_xml_format::names::element_has_name(
            *node,
            "object",
            Some("http://v8.1c.ru/8.2/roles"),
        )
    }) {
        let mut obj_name = String::new();
        let mut rights = Vec::<RoleRight>::new();
        for child in obj.children().filter(|node| node.is_element()) {
            if unica_xml_format::names::element_has_name(
                child,
                "name",
                Some("http://v8.1c.ru/8.2/roles"),
            ) {
                obj_name = child.text().unwrap_or("").to_string();
            }
            if unica_xml_format::names::element_has_name(
                child,
                "right",
                Some("http://v8.1c.ru/8.2/roles"),
            ) {
                let mut right_name = String::new();
                let mut right_value = String::new();
                let mut has_rls = false;
                for rc in child.children().filter(|node| node.is_element()) {
                    match rc.tag_name().name() {
                        "name" => right_name = rc.text().unwrap_or("").to_string(),
                        "value" => right_value = rc.text().unwrap_or("").to_string(),
                        "restrictionByCondition" => has_rls = true,
                        _ => {}
                    }
                }
                if !right_name.is_empty() && !right_value.is_empty() {
                    rights.push(RoleRight {
                        name: right_name,
                        value: right_value,
                        condition: has_rls.then(String::new),
                    });
                }
            }
        }
        if obj_name.is_empty() || rights.is_empty() {
            continue;
        }
        let Some(dot_idx) = obj_name.find('.') else {
            continue;
        };
        let type_prefix = &obj_name[..dot_idx];
        let short_name = &obj_name[dot_idx + 1..];
        for right in rights {
            if right.value == "true" {
                total_allowed += 1;
                if right.condition.is_some() {
                    rls_objects.push(format!("{type_prefix}.{short_name} ({})", right.name));
                }
                add_role_info_right(
                    &mut allowed,
                    type_prefix,
                    short_name,
                    RoleInfoRightSummary {
                        name: right.name,
                        rls: right.condition.is_some(),
                    },
                );
            } else {
                total_denied += 1;
                add_role_info_right(
                    &mut denied,
                    type_prefix,
                    short_name,
                    RoleInfoRightSummary {
                        name: right.name,
                        rls: false,
                    },
                );
            }
        }
    }
    let mut templates = Vec::<String>::new();
    for template in root.children().filter(|node| {
        unica_xml_format::names::element_has_name(
            *node,
            "restrictionTemplate",
            Some("http://v8.1c.ru/8.2/roles"),
        )
    }) {
        for child in template.children().filter(|node| node.is_element()) {
            if child.tag_name().name() == "name" {
                let mut name = child.text().unwrap_or("").to_string();
                if let Some(paren_idx) = name.find('(') {
                    if paren_idx > 0 {
                        name.truncate(paren_idx);
                    }
                }
                templates.push(name);
            }
        }
    }
    Ok(RoleInfoData {
        name: role_name,
        synonym: (!role_synonym.is_empty()).then_some(role_synonym),
        support,
        defaults: RoleDefaultsData {
            set_for_new_objects: role_attribute(set_for_new),
            set_for_attributes_by_default: role_attribute(set_for_attrs),
            independent_rights_of_child_objects: role_attribute(independent_child),
        },
        allowed: role_group_data(allowed),
        denied: role_group_data(denied),
        totals: RoleTotalsData {
            allowed: total_allowed,
            denied: total_denied,
        },
        restricted_objects: rls_objects,
        templates,
    })
}
pub fn role_info_metadata_xml(
    descriptor_text: Option<&str>,
    fallback_name: &str,
) -> (String, String) {
    let mut role_name = String::new();
    let mut role_synonym = String::new();
    if let Some(meta_text) = descriptor_text {
        if let Ok(meta_doc) = Document::parse(meta_text.trim_start_matches('\u{feff}')) {
            for role in meta_doc
                .descendants()
                .filter(|node| unica_xml_format::names::element_has_name(*node, "Role", None))
            {
                for props in role.children().filter(|node| {
                    unica_xml_format::names::element_has_name(*node, "Properties", None)
                }) {
                    if role_name.is_empty() {
                        role_name = props
                            .children()
                            .find(|node| {
                                unica_xml_format::names::element_has_name(*node, "Name", None)
                            })
                            .and_then(|node| node.text())
                            .unwrap_or("")
                            .to_string();
                    }
                    if role_synonym.is_empty() {
                        for synonym in props.children().filter(|node| {
                            unica_xml_format::names::element_has_name(*node, "Synonym", None)
                        }) {
                            for item in synonym.children().filter(|node| {
                                unica_xml_format::names::element_has_name(*node, "item", None)
                            }) {
                                let lang = item
                                    .children()
                                    .find(|node| {
                                        unica_xml_format::names::element_has_name(
                                            *node, "lang", None,
                                        )
                                    })
                                    .and_then(|node| node.text())
                                    .unwrap_or("");
                                if lang == "ru" {
                                    role_synonym = item
                                        .children()
                                        .find(|node| {
                                            unica_xml_format::names::element_has_name(
                                                *node, "content", None,
                                            )
                                        })
                                        .and_then(|node| node.text())
                                        .unwrap_or("")
                                        .to_string();
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if role_name.is_empty() {
        role_name = fallback_name.to_string();
    }

    (role_name, role_synonym)
}
pub struct RoleValidationReport {
    pub lines: Vec<String>,
    pub errors: usize,
    pub warnings: usize,
    pub ok_count: usize,
    pub detailed: bool,
}
impl RoleValidationReport {
    pub fn new(detailed: bool) -> Self {
        Self {
            lines: Vec::new(),
            errors: 0,
            warnings: 0,
            ok_count: 0,
            detailed,
        }
    }

    pub fn ok(&mut self, msg: impl AsRef<str>) {
        self.ok_count += 1;
        if self.detailed {
            self.lines.push(format!("[OK]    {}", msg.as_ref()));
        }
    }

    pub fn warn(&mut self, msg: impl AsRef<str>) {
        self.warnings += 1;
        self.lines.push(format!("[WARN]  {}", msg.as_ref()));
    }

    pub fn error(&mut self, msg: impl AsRef<str>) {
        self.errors += 1;
        self.lines.push(format!("[ERROR] {}", msg.as_ref()));
    }

    pub fn finish(mut self, role_name: &str) -> String {
        self.lines
            .insert(0, format!("=== Validation: Role.{role_name} ==="));
        let checks = self.ok_count + self.errors + self.warnings;
        if self.errors == 0 && self.warnings == 0 && !self.detailed {
            format!("=== Validation OK: Role.{role_name} ({checks} checks) ===")
        } else {
            self.lines.push(String::new());
            self.lines.push(format!(
                "=== Result: {} errors, {} warnings ({checks} checks) ===",
                self.errors, self.warnings
            ));
            self.lines.join("\n")
        }
    }
}
pub fn validate_role_source(
    reader: &dyn unica_source_contract::discovery::ProjectDiscoveryRead,
    rights_path: std::path::PathBuf,
    detailed: bool,
) -> Result<(bool, String, std::path::PathBuf), String> {
    let layout = role_read_layout(&rights_path);
    let metadata_path = layout.metadata_path;

    let mut report = RoleValidationReport::new(detailed);
    if !unica_source_designer::read::path_exists(reader, &rights_path) {
        report.error(format!("File not found: {}", rights_path.display()));
        let text = report.lines.join("\n");
        return Ok((false, text, rights_path));
    }

    let rights_text = unica_source_designer::read::read_string(reader, &rights_path)
        .map_err(|err| format!("failed to read {}: {err}", rights_path.display()))?;
    let doc = match Document::parse(rights_text.trim_start_matches('\u{feff}')) {
        Ok(doc) => {
            report.ok("XML well-formed");
            doc
        }
        Err(err) => {
            report.error(format!("XML parse error: {err}"));
            let text = report.lines.join("\n");
            return Ok((false, text, rights_path));
        }
    };

    let root = doc.root_element();
    let root_local = root.tag_name().name();
    let root_ns = root.tag_name().namespace().unwrap_or("");
    const RIGHTS_NS: &str = "http://v8.1c.ru/8.2/roles";

    if root_local != "Rights" {
        report.error(format!("Root element is '{root_local}', expected 'Rights'"));
    } else if root_ns != RIGHTS_NS {
        report.warn(format!("Namespace is '{root_ns}', expected '{RIGHTS_NS}'"));
    } else {
        report.ok("Root element: <Rights> with correct namespace");
    }

    let mut flags_found = 0usize;
    for flag in [
        "setForNewObjects",
        "setForAttributesByDefault",
        "independentRightsOfChildObjects",
    ] {
        if let Some(node) = root
            .children()
            .find(|node| unica_xml_format::names::element_has_name(*node, flag, Some(RIGHTS_NS)))
        {
            let value = node.text().unwrap_or("");
            if value != "true" && value != "false" {
                report.warn(format!("{flag} = '{value}' (expected 'true' or 'false')"));
            }
            flags_found += 1;
        } else {
            report.warn(format!("Missing global flag: {flag}"));
        }
    }
    if flags_found == 3 {
        report.ok("3 global flags present");
    }

    let objects = root
        .children()
        .filter(|node| unica_xml_format::names::element_has_name(*node, "object", Some(RIGHTS_NS)))
        .collect::<Vec<_>>();
    let mut right_count = 0usize;
    let mut rls_count = 0usize;

    for obj in &objects {
        let mut obj_name = "";
        for child in obj.children().filter(|node| node.is_element()) {
            if unica_xml_format::names::element_has_name(child, "name", Some(RIGHTS_NS)) {
                obj_name = child.text().unwrap_or("");
                break;
            }
        }

        if obj_name.is_empty() {
            report.error("Object without <name>");
            continue;
        }

        let object_type = role_validate_object_type(obj_name);
        let is_nested = obj_name.matches('.').count() >= 2;
        if !is_nested && role_validate_known_rights(object_type).is_empty() {
            report.warn(format!("{obj_name}: unknown object type '{object_type}'"));
        }

        for child in obj.children().filter(|node| node.is_element()) {
            if !unica_xml_format::names::element_has_name(child, "right", Some(RIGHTS_NS)) {
                continue;
            }

            let mut right_name = "";
            let mut right_value = "";
            for rc in child.children().filter(|node| node.is_element()) {
                if rc.tag_name().namespace() != Some(RIGHTS_NS) {
                    continue;
                }
                match rc.tag_name().name() {
                    "name" => right_name = rc.text().unwrap_or(""),
                    "value" => right_value = rc.text().unwrap_or(""),
                    "restrictionByCondition" => {
                        rls_count += 1;
                        let cond_node = rc.children().find(|node| {
                            unica_xml_format::names::element_has_name(
                                *node,
                                "condition",
                                Some(RIGHTS_NS),
                            )
                        });
                        if cond_node
                            .and_then(|node| node.text())
                            .unwrap_or("")
                            .is_empty()
                        {
                            report.warn(format!(
                                "{obj_name}: RLS condition for '{right_name}' is empty"
                            ));
                        }
                    }
                    _ => {}
                }
            }

            if right_name.is_empty() {
                report.error(format!("{obj_name}: <right> without <name>"));
                continue;
            }
            if right_value != "true" && right_value != "false" {
                report.error(format!(
                    "{obj_name}: right '{right_name}' has invalid value '{right_value}'"
                ));
                continue;
            }

            right_count += 1;
            if is_nested {
                // О раскладке вне закрытой модели наблюдение не судит:
                // иначе легальные адреса платформы получают ложное
                // предупреждение (см. `nested_right_shape_is_modelled`).
                if nested_right_shape_is_modelled(obj_name) {
                    if let Err(message) = validate_nested_right(obj_name, right_name) {
                        report.warn(message);
                    }
                }
            } else {
                let valid_rights = role_validate_known_rights(object_type);
                if !valid_rights.is_empty() && !valid_rights.contains(&right_name) {
                    let similar = role_validate_find_similar(right_name, valid_rights);
                    let suggestion = if similar.is_empty() {
                        String::new()
                    } else {
                        format!(" Did you mean: {}?", similar.join(", "))
                    };
                    report.warn(format!(
                        "{obj_name}: unknown right '{right_name}'.{suggestion}"
                    ));
                } else if !valid_rights.is_empty()
                    && right_value == "true"
                    && right_name.ends_with("PredefinedData")
                {
                    report.warn(format!(
                    "{obj_name}: '{right_name}' = true grants interactive changes to predefined data (predefined data is part of the configuration and should not be available to end users)"
                ));
                }
            }
        }
    }

    report.ok(format!("{} objects, {right_count} rights", objects.len()));
    if rls_count > 0 {
        report.ok(format!("{rls_count} RLS restrictions"));
    }

    let templates = root
        .children()
        .filter(|node| {
            unica_xml_format::names::element_has_name(*node, "restrictionTemplate", Some(RIGHTS_NS))
        })
        .collect::<Vec<_>>();
    if !templates.is_empty() {
        let mut template_names = Vec::<String>::new();
        for template in &templates {
            let mut template_name = "";
            let mut template_condition = "";
            for child in template.children().filter(|node| node.is_element()) {
                if child.tag_name().namespace() != Some(RIGHTS_NS) {
                    continue;
                }
                match child.tag_name().name() {
                    "name" => template_name = child.text().unwrap_or(""),
                    "condition" => template_condition = child.text().unwrap_or(""),
                    _ => {}
                }
            }
            if template_name.is_empty() {
                report.warn("Restriction template without <name>");
            } else {
                let short_name = template_name
                    .find('(')
                    .filter(|idx| *idx > 0)
                    .map(|idx| &template_name[..idx])
                    .unwrap_or(template_name);
                template_names.push(short_name.to_string());
            }
            if template_condition.is_empty() {
                report.warn(format!("Template '{template_name}': empty <condition>"));
            }
        }
        report.ok(format!(
            "{} templates: {}",
            templates.len(),
            template_names.join(", ")
        ));
    }

    let mut inferred_role_name = String::new();
    if reader.path_is_file(&metadata_path) {
        report.lines.push(String::new());
        match unica_source_designer::read::read_string(reader, &metadata_path) {
            Ok(meta_text) => match Document::parse(meta_text.trim_start_matches('\u{feff}')) {
                Ok(meta_doc) => {
                    if let Some(role_node) = meta_doc
                        .descendants()
                        .find(|node| unica_xml_format::names::element_has_name(*node, "Role", None))
                    {
                        let uuid_val = role_node.attribute("uuid").unwrap_or("");
                        if unica_xml_format::names::is_valid_uuid(uuid_val) {
                            report.ok(format!("Metadata: UUID valid ({uuid_val})"));
                        } else {
                            report.error(format!("Metadata: invalid UUID format '{uuid_val}'"));
                        }

                        let name_node = role_node.descendants().find(|node| {
                            unica_xml_format::names::element_has_name(*node, "Name", None)
                        });
                        if let Some(name_text) = name_node.and_then(|node| node.text()) {
                            if !name_text.is_empty() {
                                report.ok(format!("Metadata: Name = {name_text}"));
                                inferred_role_name = name_text.to_string();
                            } else {
                                report.error("Metadata: <Name> is empty or missing");
                            }
                        } else {
                            report.error("Metadata: <Name> is empty or missing");
                        }

                        let syn_node = role_node.descendants().find(|node| {
                            unica_xml_format::names::element_has_name(*node, "Synonym", None)
                        });
                        if syn_node
                            .map(|node| node.children().any(|child| child.is_element()))
                            .unwrap_or(false)
                        {
                            report.ok("Metadata: Synonym present");
                        } else {
                            report.warn("Metadata: <Synonym> is empty");
                        }
                    } else {
                        report.error("Metadata: <Role> element not found");
                    }
                }
                Err(err) => report.error(format!("Metadata XML parse error: {err}")),
            },
            Err(err) => report.error(format!("Metadata XML parse error: {err}")),
        }
    }

    let config_xml_path = layout.configuration_path;
    if inferred_role_name.is_empty() {
        inferred_role_name = layout.role_dir_name;
    }

    if unica_source_designer::read::path_exists(reader, &config_xml_path) {
        report.lines.push(String::new());
        match unica_source_designer::read::read_string(reader, &config_xml_path) {
            Ok(config_text) => match Document::parse(config_text.trim_start_matches('\u{feff}')) {
                Ok(cfg_doc) => {
                    if let Some(child_obj) = cfg_doc.descendants().find(|node| {
                        unica_xml_format::names::element_has_name(
                            *node,
                            "ChildObjects",
                            Some("http://v8.1c.ru/8.3/MDClasses"),
                        ) && node.ancestors().any(|ancestor| {
                            unica_xml_format::names::element_has_name(
                                ancestor,
                                "Configuration",
                                Some("http://v8.1c.ru/8.3/MDClasses"),
                            )
                        })
                    }) {
                        let found = child_obj.children().any(|node| {
                            unica_xml_format::names::element_has_name(
                                node,
                                "Role",
                                Some("http://v8.1c.ru/8.3/MDClasses"),
                            ) && node.text().unwrap_or("") == inferred_role_name
                        });
                        if found {
                            report.ok(format!(
                                "Configuration.xml: <Role>{inferred_role_name}</Role> registered"
                            ));
                        } else {
                            report.warn(format!(
                            "Configuration.xml: <Role>{inferred_role_name}</Role> NOT found in ChildObjects"
                        ));
                        }
                    }
                }
                Err(err) => report.warn(format!("Configuration.xml: parse error — {err}")),
            },
            Err(err) => report.warn(format!("Configuration.xml: parse error — {err}")),
        }
    }

    let ok = report.errors == 0;
    let text = report.finish(&inferred_role_name);
    Ok((ok, text, rights_path))
}
