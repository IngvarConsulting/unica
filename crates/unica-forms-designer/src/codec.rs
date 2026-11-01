//! Designer form parser, validator, compiler and in-memory editor.
use crate::event_context::context_from_root;
use crate::root::{form_edit_opening_tag_end, require_form_root, FORM_LOGFORM_NS};
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use unica_forms::events::*;
use unica_forms::facts::Utf8Bom;
use unica_forms::facts::*;
use unica_metadata::support_state::ObjectSupportData as DomainObjectSupportData;
use unica_operation_contract::input::*;
use unica_source_contract::discovery::ProjectDiscoveryRead;
use unica_source_designer::format_profile::{
    classify_root_version, format_compatibility_warning, FormatCompatibility,
};
use unica_source_designer::read::{path_exists, read_string, read_utf8_sig_snapshot};
use unica_source_designer::roots::root_version_literal;
use unica_xml_format::text::*;
use unica_xml_format::text::{form_edit_line_indent, form_edit_line_indent_at};

mod conditional_appearance;

pub const FORM_V8_NS: &str = "http://v8.1c.ru/8.1/data/core";

pub struct FormValidationReporter {
    pub errors: usize,
    pub warnings: usize,
    pub ok_count: usize,
    pub stopped: bool,
    pub max_errors: usize,
    pub detailed: bool,
    pub lines: Vec<String>,
    /// Warnings `check` reports as diagnostics. Older form warnings stay in
    /// the text report only, so their visibility does not change here.
    pub surfaced_warnings: Vec<String>,
}

impl FormValidationReporter {
    pub fn new(form_name: &str, max_errors: usize, detailed: bool) -> Self {
        Self {
            errors: 0,
            warnings: 0,
            ok_count: 0,
            stopped: false,
            max_errors,
            detailed,
            surfaced_warnings: Vec::new(),
            lines: vec![
                format!("=== Validation: Form.{form_name} ==="),
                String::new(),
            ],
        }
    }

    pub fn ok(&mut self, message: impl Into<String>) {
        self.ok_count += 1;
        if self.detailed {
            self.lines.push(format!("[OK]    {}", message.into()));
        }
    }

    pub fn error(&mut self, message: impl Into<String>) {
        self.errors += 1;
        self.lines.push(format!("[ERROR] {}", message.into()));
        if self.errors >= self.max_errors {
            self.stopped = true;
        }
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        self.warnings += 1;
        self.lines.push(format!("[WARN]  {}", message.into()));
    }

    fn warn_surfaced(&mut self, message: String) {
        self.surfaced_warnings.push(message.clone());
        self.warn(message);
    }

    pub fn finalize(mut self, form_name: &str) -> (bool, String, Vec<String>) {
        let checks = self.ok_count + self.errors + self.warnings;
        let ok = self.errors == 0;
        if ok && self.warnings == 0 && !self.detailed {
            return (
                true,
                format!("=== Validation OK: Form.{form_name} ({checks} checks) ===\n"),
                Vec::new(),
            );
        }
        self.lines.push(String::new());
        self.lines.push(format!(
            "=== Result: {} errors, {} warnings ({checks} checks) ===",
            self.errors, self.warnings
        ));
        let errors = self
            .lines
            .iter()
            .filter(|line| line.starts_with("[ERROR]"))
            .cloned()
            .collect::<Vec<_>>();
        (ok, format!("{}\n", self.lines.join("\n")), errors)
    }
}

#[derive(Clone)]
pub struct FormElementInfo<'a> {
    pub name: String,
    pub tag: String,
    pub id: String,
    pub node: roxmltree::Node<'a, 'a>,
}

/// Whether form validation compares bound handlers with their events.
/// `form.edit` discards warnings, so it skips the module read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandlerSignatures {
    Check,
    Skip,
}

/// Verdict, text report, validated file, errors and surfaced warnings.
pub type FormValidationRun = (bool, String, PathBuf, Vec<String>, Vec<String>);

pub fn form_validation_name(form_path: &Path) -> String {
    let parent = form_path.parent();
    if parent
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        == Some("Ext")
    {
        if let Some(form_dir) = parent
            .and_then(|path| path.parent())
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
        {
            return form_dir.to_string();
        }
    }
    form_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("Form")
        .to_string()
}

pub fn form_collect_elements<'a>(
    node: roxmltree::Node<'a, 'a>,
    elements: &mut Vec<FormElementInfo<'a>>,
    element_ids: &mut HashMap<String, String>,
    element_names: &mut HashMap<String, String>,
    report: &mut FormValidationReporter,
) {
    for child in node.children().filter(|child| child.is_element()) {
        let name = child.attribute("name").unwrap_or("");
        let id = child.attribute("id").unwrap_or("");
        if !name.is_empty() && !id.is_empty() {
            let tag = child.tag_name().name().to_string();
            elements.push(FormElementInfo {
                name: name.to_string(),
                tag,
                id: id.to_string(),
                node: child,
            });
            if id != "-1" {
                if let Some(existing) = element_ids.get(id) {
                    report.error(format!(
                        "Duplicate element id={id}: '{name}' and '{existing}'"
                    ));
                } else {
                    element_ids.insert(id.to_string(), name.to_string());
                }
                if let Some(existing) = element_names.get(name) {
                    report.error(format!(
                        "Duplicate element name '{name}': id={id} and id={existing}"
                    ));
                } else {
                    element_names.insert(name.to_string(), id.to_string());
                }
            }
        }
        if let Some(child_items) = form_validation_child(child, "ChildItems") {
            form_collect_elements(child_items, elements, element_ids, element_names, report);
        }
    }
}

pub fn form_validate_companions(
    elements: &[FormElementInfo<'_>],
    report: &mut FormValidationReporter,
) {
    let mut companion_errors = 0usize;
    let mut companion_checked = 0usize;
    for element in elements {
        let required = match element.tag.as_str() {
            "InputField" | "CheckBoxField" | "LabelDecoration" | "LabelField"
            | "PictureDecoration" | "PictureField" | "CalendarField" => {
                &["ContextMenu", "ExtendedTooltip"][..]
            }
            "UsualGroup" | "Pages" | "Page" | "Button" => &["ExtendedTooltip"][..],
            "Table" => &[
                "ContextMenu",
                "AutoCommandBar",
                "SearchStringAddition",
                "ViewStatusAddition",
                "SearchControlAddition",
            ][..],
            _ => continue,
        };
        companion_checked += 1;
        for tag in required {
            if form_validation_child(element.node, tag).is_none() {
                report.error(format!(
                    "[{}] '{}': missing companion <{}>",
                    element.tag, element.name, tag
                ));
                companion_errors += 1;
            }
        }
        if report.stopped {
            return;
        }
    }
    if companion_errors == 0 && companion_checked > 0 {
        report.ok(format!(
            "Companion elements: {companion_checked} elements checked"
        ));
    }
}

pub fn form_validate_data_paths(
    elements: &[FormElementInfo<'_>],
    attr_map: &HashMap<String, roxmltree::Node<'_, '_>>,
    has_base_form: bool,
    report: &mut FormValidationReporter,
) {
    let skip_tags = [
        "ContextMenu",
        "ExtendedTooltip",
        "AutoCommandBar",
        "SearchStringAddition",
        "ViewStatusAddition",
        "SearchControlAddition",
    ];
    let mut path_errors = 0usize;
    let mut path_checked = 0usize;
    let mut path_base_skipped = 0usize;
    for element in elements {
        if skip_tags.contains(&element.tag.as_str()) {
            continue;
        }
        if has_base_form
            && !element.id.is_empty()
            && element
                .id
                .parse::<i64>()
                .map(|id| id < 1_000_000)
                .unwrap_or(false)
        {
            path_base_skipped += 1;
            continue;
        }
        for (_, binding_tag) in FORM_BINDING_PATH_PROPERTIES {
            let Some(data_path) = form_validation_child_text(element.node, binding_tag) else {
                continue;
            };
            let data_path = data_path.trim();
            if data_path.is_empty() || is_opaque_form_binding(data_path) {
                continue;
            }
            path_checked += 1;

            let resolution = resolve_form_binding_path(data_path, |table_name| {
                let Some(table_element) = elements
                    .iter()
                    .find(|candidate| candidate.tag == "Table" && candidate.name == table_name)
                else {
                    return FormBindingTablePath::Missing;
                };
                match form_validation_child_text(table_element.node, "DataPath") {
                    Some(path) => FormBindingTablePath::Bound(path),
                    None => FormBindingTablePath::Unbound,
                }
            });
            let root_attr = match resolution {
                FormBindingPathResolution::Skip => continue,
                FormBindingPathResolution::UnknownItemsShape => {
                    report.warn(format!(
                        "[{}] '{}': {}='{}' — unknown Items.* shape, expected Items.<Table>.CurrentData.*",
                        element.tag, element.name, binding_tag, data_path
                    ));
                    continue;
                }
                FormBindingPathResolution::MissingTable(table_name) => {
                    report.error(format!(
                        "[{}] '{}': {}='{}' — table element '{}' not found",
                        element.tag, element.name, binding_tag, data_path, table_name
                    ));
                    path_errors += 1;
                    if report.stopped {
                        return;
                    }
                    continue;
                }
                FormBindingPathResolution::Attribute(root_attr) => root_attr,
            };

            if !attr_map.contains_key(root_attr.as_str()) {
                report.error(format!(
                    "[{}] '{}': {}='{}' — attribute '{}' not found",
                    element.tag, element.name, binding_tag, data_path, root_attr
                ));
                path_errors += 1;
            }
            if report.stopped {
                return;
            }
        }
    }
    let mut path_msg = String::new();
    if path_checked > 0 {
        path_msg = format!("{path_checked} paths checked");
    }
    if path_base_skipped > 0 {
        let skip_note = format!("{path_base_skipped} base skipped");
        path_msg = if path_msg.is_empty() {
            skip_note
        } else {
            format!("{path_msg}, {skip_note}")
        };
    }
    if path_errors == 0 && !path_msg.is_empty() {
        report.ok(format!("Binding path references: {path_msg}"));
    }
}

pub const FORM_BINDING_PATH_PROPERTIES: [(&str, &str); 8] = [
    ("path", "DataPath"),
    ("titleDataPath", "TitleDataPath"),
    ("footerDataPath", "FooterDataPath"),
    ("headerDataPath", "HeaderDataPath"),
    ("multipleValueDataPath", "MultipleValueDataPath"),
    (
        "multipleValuePresentDataPath",
        "MultipleValuePresentDataPath",
    ),
    ("rowPictureDataPath", "RowPictureDataPath"),
    (
        "multipleValuePictureDataPath",
        "MultipleValuePictureDataPath",
    ),
];

enum FormBindingTablePath {
    Missing,
    Unbound,
    Bound(String),
}

enum FormBindingPathResolution {
    Skip,
    Attribute(String),
    UnknownItemsShape,
    MissingTable(String),
}

fn resolve_form_binding_path(
    data_path: &str,
    table_path: impl FnOnce(&str) -> FormBindingTablePath,
) -> FormBindingPathResolution {
    let data_path = data_path.trim();
    if data_path.is_empty() || is_opaque_form_binding(data_path) {
        return FormBindingPathResolution::Skip;
    }

    let clean_path = strip_form_binding_prefixes(data_path);
    let root_attr = clean_path.split('.').next().unwrap_or("");
    if root_attr != "Items" {
        return FormBindingPathResolution::Attribute(root_attr.to_string());
    }

    let Some(table_name) = form_items_current_data_target(&clean_path) else {
        return FormBindingPathResolution::UnknownItemsShape;
    };

    match table_path(&table_name) {
        FormBindingTablePath::Missing => FormBindingPathResolution::MissingTable(table_name),
        FormBindingTablePath::Unbound => FormBindingPathResolution::Skip,
        FormBindingTablePath::Bound(table_path) => {
            let table_path = table_path.trim();
            if table_path.is_empty() || is_opaque_form_binding(table_path) {
                FormBindingPathResolution::Skip
            } else {
                let clean_table_path = strip_form_binding_prefixes(table_path);
                FormBindingPathResolution::Attribute(
                    clean_table_path.split('.').next().unwrap_or("").to_string(),
                )
            }
        }
    }
}

fn form_items_current_data_target(value: &str) -> Option<String> {
    let path = value.strip_prefix("Items.")?;
    let (target, remainder) = path.rsplit_once(".CurrentData")?;
    if target.is_empty() || (!remainder.is_empty() && !remainder.starts_with('.')) {
        return None;
    }
    Some(target.to_string())
}

pub fn strip_form_binding_prefixes(value: &str) -> String {
    strip_numeric_indexes(value)
        .trim_start_matches('~')
        .to_string()
}

pub fn is_opaque_form_binding(value: &str) -> bool {
    if value.chars().all(|ch| ch.is_ascii_digit()) {
        return true;
    }
    let Some((prefix, uuid)) = value.split_once(':') else {
        return false;
    };
    let Some((left, right)) = prefix.split_once('/') else {
        return false;
    };
    !left.is_empty()
        && !right.is_empty()
        && left.chars().all(|ch| ch.is_ascii_digit())
        && right.chars().all(|ch| ch.is_ascii_digit())
        && !uuid.is_empty()
        && uuid.chars().all(|ch| ch.is_ascii_hexdigit() || ch == '-')
}

pub fn strip_numeric_indexes(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '[' {
            let mut digits = String::new();
            while let Some(next) = chars.peek().copied() {
                chars.next();
                if next == ']' {
                    break;
                }
                digits.push(next);
            }
            if digits.chars().all(|digit| digit.is_ascii_digit()) {
                continue;
            }
            result.push('[');
            result.push_str(&digits);
            result.push(']');
        } else {
            result.push(ch);
        }
    }
    result
}

pub fn form_validate_button_commands(
    elements: &[FormElementInfo<'_>],
    cmd_map: &HashMap<String, roxmltree::Node<'_, '_>>,
    report: &mut FormValidationReporter,
) {
    let mut cmd_errors = 0usize;
    let mut cmd_checked = 0usize;
    for element in elements.iter().filter(|element| element.tag == "Button") {
        let Some(cmd_ref) = form_validation_child_text(element.node, "CommandName") else {
            continue;
        };
        let Some(cmd_name) = cmd_ref.strip_prefix("Form.Command.") else {
            continue;
        };
        cmd_checked += 1;
        if !cmd_map.contains_key(cmd_name) {
            report.error(format!(
                "[Button] '{}': CommandName='{}' — command '{}' not found in Commands",
                element.name, cmd_ref, cmd_name
            ));
            cmd_errors += 1;
        }
        if report.stopped {
            return;
        }
    }
    if cmd_errors == 0 && cmd_checked > 0 {
        report.ok(format!("Command references: {cmd_checked} buttons checked"));
    }
}

/// Lazily read facts for comparing bound handlers with their events: the
/// form module next to `Ext/Form.xml` and the owner's event lists.
pub struct HandlerSignatureCheck<'reader> {
    reader: Option<&'reader dyn ProjectDiscoveryRead>,
    module_path: Option<PathBuf>,
    module_text: Option<&'reader str>,
    module: Option<Option<Vec<unica_bsl_parser::BslMethodParameters>>>,
    metadata_owner: Option<unica_address::address::NodeKind>,
    owners: HashMap<(FormEventOwnerKind, Option<String>), Vec<FormEventParameterForms>>,
}

impl<'reader> HandlerSignatureCheck<'reader> {
    pub fn new(reader: &'reader dyn ProjectDiscoveryRead, form_path: &Path) -> Self {
        Self {
            reader: Some(reader),
            module_path: form_path
                .parent()
                .map(|ext| ext.join("Form").join("Module.bsl")),
            module_text: None,
            module: None,
            metadata_owner: form_metadata_owner(form_path),
            owners: HashMap::new(),
        }
    }

    /// Проверяет сохранённый модуль без повторного обращения к исходникам.
    pub fn from_source(
        module: Option<&'reader str>,
        metadata_owner: Option<unica_address::address::NodeKind>,
    ) -> Self {
        Self {
            reader: None,
            module_path: None,
            module_text: module,
            module: None,
            metadata_owner,
            owners: HashMap::new(),
        }
    }

    /// The module methods, parsed on first use. An absent or unparsable
    /// module gives `None`: handler presence and BSL syntax have their own
    /// checks, and a guessed method list must not produce findings.
    fn module(&mut self) -> Option<&[unica_bsl_parser::BslMethodParameters]> {
        let path = &self.module_path;
        let reader = self.reader;
        let module_text = self.module_text;
        self.module
            .get_or_insert_with(|| {
                let text = match module_text {
                    Some(text) => std::borrow::Cow::Borrowed(text),
                    None => std::borrow::Cow::Owned(read_string(reader?, path.as_ref()?).ok()?),
                };
                unica_bsl_parser::bsl_method_parameters(text.trim_start_matches('\u{feff}')).ok()
            })
            .as_deref()
    }

    fn owner_events(
        &mut self,
        context: &FormEventContext,
        owner: FormEventOwnerKind,
        data_path: Option<&str>,
    ) -> &[FormEventParameterForms] {
        let metadata_owner = self.metadata_owner;
        self.owners
            .entry((owner, data_path.map(str::to_string)))
            .or_insert_with(|| {
                let mut context = context.clone();
                context.metadata_owner = metadata_owner;
                form_owner_event_parameters_8_3_27(&context, owner, data_path)
            })
    }
}

/// The metadata kind that owns a dumped form, from its
/// `<Category>/<Object>/Forms/<Form>/Ext/Form.xml` path. Other layouts, such
/// as external processors, give `None`, and the form-kind-specific event
/// lists are then not selected.
fn form_metadata_owner(form_path: &Path) -> Option<unica_address::address::NodeKind> {
    let ext = form_path.parent()?;
    let forms = ext.parent()?.parent()?;
    if ext.file_name()? != "Ext" || forms.file_name()? != "Forms" {
        return None;
    }
    let category = forms.parent()?.parent()?.file_name()?.to_str()?;
    let kind = match category {
        "Catalogs" => "Catalog",
        "Documents" => "Document",
        "ExchangePlans" => "ExchangePlan",
        "ChartsOfAccounts" => "ChartOfAccounts",
        "ChartsOfCharacteristicTypes" => "ChartOfCharacteristicTypes",
        "ChartsOfCalculationTypes" => "ChartOfCalculationTypes",
        "BusinessProcesses" => "BusinessProcess",
        "Tasks" => "Task",
        "DataProcessors" => "DataProcessor",
        "Reports" => "Report",
        "InformationRegisters" => "InformationRegister",
        "AccumulationRegisters" => "AccumulationRegister",
        "AccountingRegisters" => "AccountingRegister",
        "CalculationRegisters" => "CalculationRegister",
        "DocumentJournals" => "DocumentJournal",
        "Enums" => "Enum",
        "FilterCriteria" => "FilterCriterion",
        "SettingsStorages" => "SettingsStorage",
        _ => return None,
    };
    unica_address::address::NodeKind::parse(kind).ok()
}

pub fn form_validate_events(
    root: roxmltree::Node<'_, '_>,
    signatures: Option<HandlerSignatureCheck<'_>>,
    metadata_owner: Option<unica_address::address::NodeKind>,
    report: &mut FormValidationReporter,
) {
    form_validate_events_inner(root, signatures, metadata_owner, report);
}

/// Определяет необходимость модуля теми же правилами, что проверка событий.
/// Непригодный XML остаётся предметом основной проверки формы.
pub fn form_validation_requires_module(
    text: &str,
    metadata_owner: Option<unica_address::address::NodeKind>,
) -> bool {
    let Ok(document) = Document::parse(text.trim_start_matches('\u{feff}')) else {
        return false;
    };
    let root = document.root_element();
    if require_form_root(root).is_err() {
        return false;
    }
    form_validate_events_inner(
        root,
        None,
        metadata_owner,
        // Без предела ошибок обход видит каждую привязку при любом maxErrors проверки.
        &mut FormValidationReporter::new("", usize::MAX, false),
    )
}

fn form_validate_events_inner(
    root: roxmltree::Node<'_, '_>,
    signatures: Option<HandlerSignatureCheck<'_>>,
    metadata_owner: Option<unica_address::address::NodeKind>,
    report: &mut FormValidationReporter,
) -> bool {
    struct EventValidationState<'report, 'reader> {
        context: FormEventContext,
        signatures: Option<HandlerSignatureCheck<'reader>>,
        report: &'report mut FormValidationReporter,
        errors: usize,
        checked: usize,
        unverified: usize,
        module_required: bool,
    }

    impl EventValidationState<'_, '_> {
        /// A bound handler whose signature belongs to another event (#791).
        /// Shorter signatures are legal earlier forms and are not checked.
        /// `callType` bindings of extension forms are skipped: their handlers
        /// wrap a borrowed one and are not compared here. The platform does not check
        /// parameter names, so the finding is a warning, not a failed check.
        fn validate_handler_signature(
            &mut self,
            owner_node: roxmltree::Node<'_, '_>,
            target: FormEventTarget,
            target_label: &str,
            binding: &FormEventBinding<'_>,
        ) {
            if binding.call_type.is_some() {
                return;
            }
            self.module_required = true;
            let Some(check) = self.signatures.as_mut() else {
                return;
            };
            let Some(module) = check.module() else {
                return;
            };
            let folded = binding.handler.to_lowercase();
            let mut methods = module
                .iter()
                .filter(|method| method.name.to_lowercase() == folded);
            let (Some(method), None) = (methods.next(), methods.next()) else {
                return;
            };
            let parameters = method
                .parameters
                .iter()
                .map(|(name, optional)| (name.clone(), *optional))
                .collect::<Vec<_>>();
            let declared = method
                .parameters
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let handler_name = method.name.clone();
            let owner = match target {
                FormEventTarget::Form => FormEventOwnerKind::Form,
                FormEventTarget::Element(FormElementKind::Table) => FormEventOwnerKind::Table,
                FormEventTarget::Element(kind) => FormEventOwnerKind::Element(kind),
            };
            let data_path = (owner == FormEventOwnerKind::Table)
                .then(|| form_validation_child_text(owner_node, "DataPath"))
                .flatten();
            let owner_events = check.owner_events(&self.context, owner, data_path.as_deref());
            let message = match foreign_handler_signature_8_3_27(owner_events, binding.name, &parameters.iter().map(|parameter| (parameter.0.as_str(),parameter.1)).collect::<Vec<_>>())
            {
                None => return,
                Some(HandlerSignatureFinding::ForeignEvent {
                    bound_ru,
                    expected,
                    other_ru,
                    other,
                }) => format!(
                    "Event '{bound_ru}' of {target_label}: handler '{handler_name}' ({declared}) matches the signature of event '{other_ru}' ({}), not the bound event ({})",
                    other.join(", "),
                    expected.join(", ")
                ),
                Some(HandlerSignatureFinding::ExtraRequired {
                    bound_ru,
                    required,
                    expected,
                }) => format!(
                    "Event '{bound_ru}' of {target_label}: handler '{handler_name}' ({declared}) requires {required} parameter(s), more than the platform passes ({})",
                    expected.join(", ")
                ),
            };
            self.report.warn_surfaced(message);
        }

        fn validate_owner(
            &mut self,
            owner: roxmltree::Node<'_, '_>,
            target: Option<FormEventTarget>,
            target_label: &str,
        ) {
            let Some(events) = form_validation_child(owner, "Events") else {
                return;
            };
            let mut names = HashSet::<String>::new();
            for event in form_validation_children(events, "Event") {
                let name = event.attribute("name").unwrap_or("");
                self.checked += 1;
                if !names.insert(name.to_string()) {
                    self.report.error(form_edit_event_diagnostic(
                        FormEventDiagnosticCode::Duplicate,
                        target_label,
                        name,
                        "event names must be unique within an Events section",
                    ));
                    self.errors += 1;
                }
                let handler = event.text().unwrap_or("").trim();
                let binding = if let Some(call_type) = event.attribute("callType") {
                    FormEventBinding::new(name, handler).with_call_type(call_type)
                } else {
                    FormEventBinding::new(name, handler)
                };
                let data_path = form_child_text(owner, "DataPath");
                let binding = binding.with_data_path(data_path.as_deref());
                let validation = target.map_or_else(
                    || {
                        Err(FormEventDiagnostic::new(
                            FormEventDiagnosticCode::EventNotAllowed,
                            target_label,
                            name,
                        )
                        .with_detail("event owner has no registered platform event matrix"))
                    },
                    |target| {
                        validate_event_owner_node(owner, target, &binding)
                            .and_then(|_| validate_event(&self.context, target, &binding))
                    },
                );
                if let (Ok(()), Some(target)) = (&validation, target) {
                    self.validate_handler_signature(owner, target, target_label, &binding);
                }
                if let Err(mut diagnostic) = validation {
                    diagnostic.target = target_label.to_string();
                    if diagnostic.code == FormEventDiagnosticCode::ContextUnknown
                        && self.context.main_attribute_provenance
                            == MainAttributeProvenance::InheritedBaseFormUnavailable
                    {
                        self.report.warn(format!(
                        "{}; inherited base-form context is unavailable, so this binding was not verified",
                        diagnostic
                    ));
                        self.unverified += 1;
                    } else {
                        self.report.error(diagnostic.to_string());
                        self.errors += 1;
                    }
                }
                if self.report.stopped {
                    return;
                }
            }
        }
    }

    let base_form = form_validation_child(root, "BaseForm");
    // Вид владельца формы выбирает его каталог событий, как при планировании
    // привязки: иначе итоговая проверка отвергла бы принятое событие.
    let mut context = context_from_root(root);
    context.metadata_owner = metadata_owner;
    let mut state = EventValidationState {
        context,
        signatures,
        report,
        errors: 0,
        checked: 0,
        unverified: 0,
        module_required: false,
    };
    state.validate_owner(root, Some(FormEventTarget::Form), "form");

    for owner in root.descendants().skip(1).filter(|node| {
        node.is_element()
            && form_validation_child(*node, "Events").is_some()
            && !base_form.is_some_and(|base| {
                *node == base || node.ancestors().any(|ancestor| ancestor == base)
            })
    }) {
        let tag = owner.tag_name().name();
        let name = owner.attribute("name").unwrap_or("");
        let target_label = if name.is_empty() {
            format!("{tag} element")
        } else {
            format!("{tag} element '{name}'")
        };
        let target = FormElementKind::from_xml_tag(tag).map(FormEventTarget::Element);
        state.validate_owner(owner, target, &target_label);
        if state.report.stopped {
            return state.module_required;
        }
    }

    if state.errors == 0 && state.unverified == 0 && state.checked > 0 {
        state
            .report
            .ok(format!("Event handlers: {} events checked", state.checked));
    }
    state.module_required
}

fn validate_event_owner_node(
    owner: roxmltree::Node<'_, '_>,
    target: FormEventTarget,
    binding: &FormEventBinding<'_>,
) -> Result<(), FormEventDiagnostic> {
    if target == FormEventTarget::Element(FormElementKind::Table)
        && form_validation_child_text(owner, "DataPath").is_none_or(|path| path.trim().is_empty())
    {
        return Err(FormEventDiagnostic::new(
            FormEventDiagnosticCode::EventNotAllowed,
            "table",
            binding.name,
        )
        .with_detail(
            "Table event bindings require a non-empty direct DataPath; the platform drops bindings on unbound tables",
        ));
    }
    Ok(())
}

pub fn form_validate_command_actions(
    cmd_nodes: &[roxmltree::Node<'_, '_>],
    report: &mut FormValidationReporter,
) {
    let mut action_errors = 0usize;
    let mut action_checked = 0usize;
    for command in cmd_nodes {
        let cmd_name = command.attribute("name").unwrap_or("");
        let actions = form_validation_children(*command, "Action");
        action_checked += 1;
        if actions
            .first()
            .and_then(|action| action.text())
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .is_none()
        {
            report.error(format!("Command '{cmd_name}': missing or empty Action"));
            action_errors += 1;
        }
        if report.stopped {
            return;
        }
    }
    if action_errors == 0 && action_checked > 0 {
        report.ok(format!(
            "Command actions: {action_checked} commands checked"
        ));
    }
}

pub fn form_validate_extension(
    root: roxmltree::Node<'_, '_>,
    _elements: &[FormElementInfo<'_>],
    attr_nodes: &[roxmltree::Node<'_, '_>],
    cmd_nodes: &[roxmltree::Node<'_, '_>],
    report: &mut FormValidationReporter,
) {
    let Some(base_form) = form_validation_child(root, "BaseForm") else {
        return;
    };
    if let Some(version) = base_form
        .attribute("version")
        .filter(|value| !value.is_empty())
    {
        report.ok(format!("BaseForm: version={version}"));
    } else {
        report.warn("BaseForm: version attribute missing");
    }

    let mut ct_errors = 0usize;
    let mut ct_checked = 0usize;
    for command in cmd_nodes {
        let cmd_name = command.attribute("name").unwrap_or("");
        for action in form_validation_children(*command, "Action") {
            if let Some(call_type) = action
                .attribute("callType")
                .filter(|value| !value.is_empty())
            {
                ct_checked += 1;
                if !form_valid_call_type(call_type) {
                    report.error(format!(
                        "Command '{cmd_name}' Action: invalid callType='{call_type}'"
                    ));
                    ct_errors += 1;
                }
            }
        }
    }
    if !report.stopped && ct_errors == 0 && ct_checked > 0 {
        report.ok(format!("callType values: {ct_checked} checked"));
    }

    let base_attr_names = form_validation_child(base_form, "Attributes")
        .map(|attrs| {
            form_validation_children(attrs, "Attribute")
                .into_iter()
                .filter_map(|attr| attr.attribute("name").map(ToOwned::to_owned))
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    let base_cmd_names = form_validation_child(base_form, "Commands")
        .map(|commands| {
            form_validation_children(commands, "Command")
                .into_iter()
                .filter_map(|cmd| cmd.attribute("name").map(ToOwned::to_owned))
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    let mut id_warn_count = 0usize;
    for attr in attr_nodes {
        let name = attr.attribute("name").unwrap_or("");
        let id = attr.attribute("id").unwrap_or("");
        if !name.is_empty()
            && !base_attr_names.contains(name)
            && id.parse::<i64>().map(|id| id < 1_000_000).unwrap_or(false)
        {
            report.warn(format!(
                "Attribute '{name}' (id={id}): extension-added attribute has id < 1000000"
            ));
            id_warn_count += 1;
        }
    }
    for command in cmd_nodes {
        let name = command.attribute("name").unwrap_or("");
        let id = command.attribute("id").unwrap_or("");
        if !name.is_empty()
            && !base_cmd_names.contains(name)
            && id.parse::<i64>().map(|id| id < 1_000_000).unwrap_or(false)
        {
            report.warn(format!(
                "Command '{name}' (id={id}): extension-added command has id < 1000000"
            ));
            id_warn_count += 1;
        }
    }
    if !report.stopped && id_warn_count == 0 {
        let ext_attr_count = attr_nodes
            .iter()
            .filter(|attr| {
                attr.attribute("name")
                    .is_some_and(|name| !base_attr_names.contains(name))
            })
            .count();
        let ext_cmd_count = cmd_nodes
            .iter()
            .filter(|cmd| {
                cmd.attribute("name")
                    .is_some_and(|name| !base_cmd_names.contains(name))
            })
            .count();
        if ext_attr_count + ext_cmd_count > 0 {
            report.ok(format!(
                "Extension ID ranges: {ext_attr_count} attr(s), {ext_cmd_count} cmd(s) — all >= 1000000"
            ));
        }
    }
}

pub fn form_valid_call_type(call_type: &str) -> bool {
    matches!(call_type, "Before" | "After" | "Override")
}

pub fn form_has_call_type(
    root: roxmltree::Node<'_, '_>,
    cmd_nodes: &[roxmltree::Node<'_, '_>],
) -> bool {
    form_validation_child(root, "Events")
        .map(|events| {
            form_validation_children(events, "Event")
                .iter()
                .any(|event| {
                    event
                        .attribute("callType")
                        .is_some_and(|value| !value.is_empty())
                })
        })
        .unwrap_or(false)
        || cmd_nodes.iter().any(|cmd| {
            form_validation_children(*cmd, "Action")
                .iter()
                .any(|action| {
                    action
                        .attribute("callType")
                        .is_some_and(|value| !value.is_empty())
                })
        })
}

pub fn form_validate_types(
    root: roxmltree::Node<'_, '_>,
    is_config_context: bool,
    report: &mut FormValidationReporter,
) {
    let type_nodes = root
        .descendants()
        .filter(|node| {
            node.is_element()
                && node.tag_name().name() == "Type"
                && form_is_data_type_declaration_type_node(*node)
        })
        .collect::<Vec<_>>();
    let mut type_error_count = 0usize;
    let mut type_warn_count = 0usize;
    for type_node in &type_nodes {
        let value = type_node.text().unwrap_or("").trim();
        if value.is_empty() {
            continue;
        }
        if value.contains(':') {
            if let Err(error) = require_form_type_qname_binding(*type_node, value) {
                report.error(format!("12. Type \"{value}\": {error}"));
                type_error_count += 1;
                if report.stopped {
                    return;
                }
                continue;
            }
        }
        if form_invalid_types().contains(&value) {
            report.error(format!(
                "12. Type \"{value}\": invalid runtime/UI type (not valid in XDTO schema)"
            ));
            type_error_count += 1;
        } else if form_valid_closed_types().contains(&value) {
        } else if let Some(suffix) = value.strip_prefix("cfg:") {
            let prefix = suffix.split('.').next().unwrap_or("");
            if unica_metadata::type_contract::valid_cfg_prefixes().contains(&prefix)
                || suffix == "DynamicList"
            {
                if is_config_context
                    && matches!(
                        prefix,
                        "ExternalDataProcessorObject" | "ExternalReportObject"
                    )
                {
                    report.error(format!(
                        "12. Type \"{value}\": External* type in configuration context (use DataProcessorObject/ReportObject instead)"
                    ));
                    type_error_count += 1;
                }
            } else {
                report.warn(format!("12. Type \"{value}\": unrecognized cfg prefix"));
                type_warn_count += 1;
            }
        } else if value.contains(':') {
        } else {
            report.error(format!(
                "12. Type \"{value}\": bare type without namespace prefix"
            ));
            type_error_count += 1;
        }
        if report.stopped {
            return;
        }
    }
    if type_error_count == 0 && type_warn_count == 0 {
        if type_nodes.is_empty() {
            report.ok("12. Types: no type values to check");
        } else {
            report.ok(format!("12. Types: {} values, all valid", type_nodes.len()));
        }
    }
}

pub fn require_form_type_qname_binding(
    node: roxmltree::Node<'_, '_>,
    value: &str,
) -> Result<(), String> {
    let Some((prefix, local_name)) = value.split_once(':') else {
        return Ok(());
    };
    if local_name.contains(':')
        || !unica_xml_format::names::xml_ncname_is_valid(prefix)
        || !unica_xml_format::names::xml_ncname_is_valid(local_name)
    {
        return Err("invalid QName syntax".to_string());
    }
    let namespace = node
        .lookup_namespace_uri(Some(prefix))
        .ok_or_else(|| format!("undeclared prefix '{prefix}'"))?;
    if let Some((_, expected)) = form_edit_emitter_namespaces()
        .into_iter()
        .find(|(known_prefix, _)| *known_prefix == prefix)
    {
        if namespace != expected {
            return Err(format!(
                "prefix '{prefix}' is bound to '{namespace}', expected '{expected}'"
            ));
        }
    }
    Ok(())
}

pub fn form_is_data_type_declaration_type_node(node: roxmltree::Node<'_, '_>) -> bool {
    let Some(parent) = node.parent_element() else {
        return false;
    };
    match parent.tag_name().name() {
        "Attribute" | "Parameter" | "Column" => true,
        "Type" => parent.parent_element().is_some_and(|grandparent| {
            matches!(
                grandparent.tag_name().name(),
                "Attribute" | "Parameter" | "Column"
            )
        }),
        _ => false,
    }
}

pub fn form_is_config_context(reader: &dyn ProjectDiscoveryRead, form_path: &Path) -> bool {
    // The owner of the form decides its context. Looking only for a
    // `Configuration.xml` above read an external artifact stored inside a
    // workspace that also holds a configuration as configuration context, and
    // then rejected the main attribute `epf.init` had just written (#340).
    if let Some(owner) = form_owner_artifact_name(reader, form_path) {
        return !matches!(
            owner.as_str(),
            "ExternalDataProcessor" | "ExternalReport" | "ExternalDataProcessorForm"
        );
    }
    let mut walk_dir = form_path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_path_buf();
    for _ in 0..15 {
        if reader.path_is_file(&walk_dir.join("Configuration.xml")) {
            return true;
        }
        let Some(parent) = walk_dir.parent() else {
            break;
        };
        if parent == walk_dir {
            break;
        }
        walk_dir = parent.to_path_buf();
    }
    false
}

/// The artifact tag of the object that owns this form, read from the object's
/// own descriptor next to its `Forms` directory. `None` when the form is not
/// stored in the object layout or the descriptor cannot be read, and the
/// caller falls back to the tree walk.
fn form_owner_artifact_name(reader: &dyn ProjectDiscoveryRead, form_path: &Path) -> Option<String> {
    let mut dir = form_path.parent()?;
    for _ in 0..15 {
        if dir.file_name() == Some(std::ffi::OsStr::new("Forms")) {
            let object_dir = dir.parent()?;
            let descriptor = object_dir.with_extension("xml");
            let text = read_string(reader, &descriptor).ok()?;
            let document = Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
            return document
                .root_element()
                .children()
                .find(|node| node.is_element())
                .map(|node| node.tag_name().name().to_string());
        }
        dir = dir.parent()?;
    }
    None
}

pub fn form_invalid_types() -> &'static [&'static str] {
    &[
        "FormDataStructure",
        "FormDataCollection",
        "FormDataTree",
        "FormDataTreeItem",
        "FormDataCollectionItem",
        "FormGroup",
        "FormField",
        "FormButton",
        "FormDecoration",
        "FormTable",
    ]
}

pub fn form_valid_closed_types() -> &'static [&'static str] {
    &[
        "xs:boolean",
        "xs:string",
        "xs:decimal",
        "xs:dateTime",
        "xs:binary",
        "xs:base64Binary",
        "v8:FillChecking",
        "v8:Null",
        "v8:StandardPeriod",
        "v8:StandardBeginningDate",
        "v8:Type",
        "v8:TypeDescription",
        "v8:UUID",
        "v8:ValueListType",
        "v8:ValueTable",
        "v8:ValueTree",
        "v8:Universal",
        "v8:FixedArray",
        "v8:FixedStructure",
        "v8ui:Color",
        "v8ui:Font",
        "v8ui:FormattedString",
        "v8ui:HorizontalAlign",
        "v8ui:Picture",
        "v8ui:SizeChangeMode",
        "v8ui:VerticalAlign",
        "dcsset:DataCompositionComparisonType",
        "dcsset:DataCompositionFieldPlacement",
        "dcsset:Filter",
        "dcsset:SettingsComposer",
        "dcsset:DataCompositionSettings",
        "dcssch:DataCompositionSchema",
        "dcscor:DataCompositionComparisonType",
        "dcscor:DataCompositionGroupType",
        "dcscor:DataCompositionPeriodAdditionType",
        "dcscor:DataCompositionSortDirection",
        "dcscor:Field",
        "ent:AccountType",
        "ent:AccumulationRecordType",
        "ent:AccountingRecordType",
    ]
}

fn form_info_optional(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn form_info_events_section(events: roxmltree::Node<'_, '_>) -> Vec<FormInfoEvent> {
    form_children(events, "Event")
        .into_iter()
        .map(|event| FormInfoEvent {
            name: event.attribute("name").unwrap_or("").to_string(),
            handler: event.text().unwrap_or("").to_string(),
            call_type: event.attribute("callType").map(str::to_string),
        })
        .collect()
}

fn form_info_binding_property(
    node: roxmltree::Node<'_, '_>,
    name: &str,
    limits: &mut Vec<String>,
) -> Option<String> {
    let values = node
        .children()
        .filter(|child| child.is_element() && child.tag_name().name() == name)
        .collect::<Vec<_>>();
    let value = match values.as_slice() {
        [] => return None,
        [value]
            if value.tag_name().namespace() == Some(FORM_LOGFORM_NS)
                && value.attributes().len() == 0
                && !value.children().any(|child| child.is_element()) =>
        {
            *value
        }
        _ => {
            limits.push(format!(
                "{name} is unreadable: expected one direct text property in the form namespace"
            ));
            return None;
        }
    };
    let text = unica_xml_format::read::direct_text(value);
    (!text.trim().is_empty()).then_some(text)
}

fn form_info_binding(
    data_path: Option<&str>,
    command_name: Option<&str>,
) -> Option<FormInfoBinding> {
    if let Some(data_path) = data_path {
        return Some(FormInfoBinding {
            kind: "dataPath".to_string(),
            target: data_path.trim().to_string(),
        });
    }
    let command_name = command_name?.trim();
    if let Some(name) = command_name.strip_prefix("Form.StandardCommand.") {
        Some(FormInfoBinding {
            kind: "standardCommand".to_string(),
            target: name.to_string(),
        })
    } else if let Some(name) = command_name.strip_prefix("Form.Command.") {
        Some(FormInfoBinding {
            kind: "command".to_string(),
            target: name.to_string(),
        })
    } else {
        Some(FormInfoBinding {
            kind: "other".to_string(),
            target: command_name.to_string(),
        })
    }
}

/// Typed counterpart of [`form_build_tree`]. The report collapsed deep pages
/// behind `-Expand`; data has no reason to hide them, so the whole tree is
/// always walked.
fn form_info_tree(child_items: roxmltree::Node<'_, '_>) -> Vec<FormInfoElement> {
    child_items
        .children()
        .filter(|child| {
            child.is_element() && !form_skip_elements().contains(&child.tag_name().name())
        })
        .map(form_info_element)
        .collect()
}

/// Элементы владельца, как их адресует `view`: его автоматическая командная
/// панель (у формы и таблицы), затем `ChildItems`. Панель — место кнопок,
/// которые добавляет `item.add` по её адресу (#1355); остальные
/// companions (`ContextMenu`, `ExtendedTooltip`, …) в дерево не входят.
fn form_info_children(owner: roxmltree::Node<'_, '_>) -> Vec<FormInfoElement> {
    form_child(owner, "AutoCommandBar")
        .filter(|bar| bar.attribute("name").is_some_and(|name| !name.is_empty()))
        .map(form_info_element)
        .into_iter()
        .chain(
            form_child(owner, "ChildItems")
                .map(form_info_tree)
                .unwrap_or_default(),
        )
        .collect()
}

fn form_info_element(child: roxmltree::Node<'_, '_>) -> FormInfoElement {
    let name = child.attribute("name").unwrap_or("").to_string();
    let mut binding_limits = Vec::new();
    let data_path = form_info_binding_property(child, "DataPath", &mut binding_limits);
    let command_name = form_info_binding_property(child, "CommandName", &mut binding_limits);
    FormInfoElement {
        tag: form_element_tag(child),
        event_kind: FormElementKind::from_xml_tag(child.tag_name().name()),
        title: form_title_differs(child, &name),
        name,
        binding: form_info_binding(data_path.as_deref(), command_name.as_deref()),
        data_path,
        command_name,
        binding_limits,
        visible: form_child_text(child, "Visible").as_deref() != Some("false"),
        enabled: form_child_text(child, "Enabled").as_deref() != Some("false"),
        read_only: form_child_text(child, "ReadOnly").as_deref() == Some("true"),
        events: form_child(child, "Events")
            .map(form_info_events_section)
            .unwrap_or_default(),
        children: form_info_children(child),
    }
}

fn form_info_attributes(attrs: roxmltree::Node<'_, '_>) -> Vec<FormInfoAttribute> {
    form_children(attrs, "Attribute")
        .into_iter()
        .map(|attr| {
            let type_name = crate::column_add::observed_value_table_type(attr).or_else(|| {
                form_child(attr, "Type")
                    .map(form_format_type)
                    .and_then(form_info_optional)
                    // The display formatter alone does not prove the table's
                    // namespace or complete type shape for column creation.
                    .filter(|type_name| type_name != "ValueTable")
            });
            let main_table = (type_name.as_deref() == Some("DynamicList"))
                .then(|| form_child(attr, "Settings").and_then(|s| form_child_text(s, "MainTable")))
                .flatten();
            let columns = if matches!(type_name.as_deref(), Some("ValueTable") | Some("ValueTree"))
            {
                form_child(attr, "Columns")
                    .map(|columns| {
                        form_children(columns, "Column")
                            .into_iter()
                            .map(|column| FormInfoColumn {
                                name: column.attribute("name").unwrap_or("").to_string(),
                                type_name: form_child(column, "Type")
                                    .map(form_format_type)
                                    .and_then(form_info_optional),
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            FormInfoAttribute {
                name: attr.attribute("name").unwrap_or("").to_string(),
                type_name,
                is_main: form_child_text(attr, "MainAttribute").as_deref() == Some("true"),
                main_table,
                columns,
            }
        })
        .collect()
}

fn form_info_parameters(params: roxmltree::Node<'_, '_>) -> Vec<FormInfoParameter> {
    form_children(params, "Parameter")
        .into_iter()
        .map(|param| FormInfoParameter {
            name: param.attribute("name").unwrap_or("").to_string(),
            type_name: form_child(param, "Type")
                .map(form_format_type)
                .and_then(form_info_optional),
            is_key: form_child_text(param, "KeyParameter").as_deref() == Some("true"),
        })
        .collect()
}

fn form_info_commands(commands: roxmltree::Node<'_, '_>) -> Vec<FormInfoCommand> {
    form_children(commands, "Command")
        .into_iter()
        .map(|command| FormInfoCommand {
            name: command.attribute("name").unwrap_or("").to_string(),
            actions: form_children(command, "Action")
                .into_iter()
                .map(|action| FormInfoEvent {
                    name: "Execute".to_string(),
                    handler: action.text().unwrap_or("").to_string(),
                    call_type: action.attribute("callType").map(str::to_string),
                })
                .collect(),
            title: form_child_text(command, "Title"),
            tooltip: form_child_text(command, "ToolTip"),
            shortcut: form_child_text(command, "Shortcut"),
            picture: form_child(command, "Picture")
                .and_then(|picture| form_child_text(picture, "Ref")),
            representation: form_child_text(command, "Representation"),
        })
        .collect()
}

fn form_event_evidence_from_root(root: roxmltree::Node<'_, '_>) -> FormEventEvidence {
    FormEventEvidence {
        context: context_from_root(root),
        events: form_child(root, "Events")
            .map(form_info_events_section)
            .unwrap_or_default(),
        elements: form_info_children(root),
        commands: form_child(root, "Commands")
            .map(form_info_commands)
            .unwrap_or_default(),
    }
}

/// Attribute facts used by the reader and operation admission share one projection.
pub fn parse_form_attributes_xml(text: &str) -> Result<Vec<FormInfoAttribute>, String> {
    let doc = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("Form XML parse error: {error}"))?;
    let root = doc.root_element();
    require_form_root(root)?;
    Ok(form_child(root, "Attributes")
        .map(form_info_attributes)
        .unwrap_or_default())
}

pub fn parse_form_event_evidence_xml(text: &str) -> Result<FormEventEvidence, String> {
    let doc = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("Form XML parse error: {err}"))?;
    let root = doc.root_element();
    require_form_root(root)?;
    Ok(form_event_evidence_from_root(root))
}

/// The address kinds a form reader accepts: a nested `Catalog.X.Form.Y` and a
/// top-level `CommonForm.Y`.
pub const FORM_KINDS: &[&str] = &["Form", "CommonForm"];

pub fn parse_form_info_xml(
    text: &str,
    form_name: String,
    object_context: String,
    support: DomainObjectSupportData,
) -> Result<FormInfoData, String> {
    let doc = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("Form XML parse error: {err}"))?;
    let root = doc.root_element();
    require_form_root(root)?;
    let base_form = form_child(root, "BaseForm");
    let is_extension = base_form.is_some();
    let form_title = form_child(root, "Title")
        .map(form_ml_text)
        .filter(|value| !value.is_empty());
    // Свойства корня в порядке 8.3.27: те же имена, что пишет `props.set`.
    let properties = unica_forms::root_properties::ROOT_PROPERTIES
        .iter()
        .map(|property| property.name)
        .filter_map(|name| {
            form_child(root, name).and_then(|node| {
                let value = form_ml_text(node);
                (!value.is_empty()).then(|| FormInfoProperty {
                    name: name.to_string(),
                    value,
                })
            })
        })
        .collect::<Vec<_>>();
    let command_bar_location = form_child_text(root, "CommandBarLocation")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "Auto".to_string());
    let auto_command_bar = if command_bar_location != "None" {
        form_child(root, "AutoCommandBar")
            .map(form_main_command_bar_lines)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let event_evidence = form_event_evidence_from_root(root);
    Ok(FormInfoData {
        name: form_name,
        title: form_title,
        object_context: (!object_context.is_empty()).then_some(object_context),
        is_extension,
        base_form_version: base_form
            .map(|node| node.attribute("version").unwrap_or("").to_string()),
        support,
        support_details: None,
        properties,
        events: event_evidence.events,
        auto_command_bar,
        command_bar_location,
        elements: event_evidence.elements,
        attributes: form_child(root, "Attributes")
            .map(form_info_attributes)
            .unwrap_or_default(),
        parameters: form_child(root, "Parameters")
            .map(form_info_parameters)
            .unwrap_or_default(),
        commands: event_evidence.commands,
        conditional_appearance: conditional_appearance::read(root),
        event_context: event_evidence.context,
    })
}

pub fn form_info_context(reader: &dyn ProjectDiscoveryRead, form_path: &Path) -> (String, String) {
    let resolved = reader
        .canonicalize(form_path)
        .unwrap_or_else(|_| form_path.to_path_buf());
    let parts = resolved
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(part) => part.to_str().map(ToOwned::to_owned),
            _ => None,
        })
        .collect::<Vec<_>>();
    if let Some(forms_idx) = parts.iter().rposition(|part| part == "Forms") {
        if forms_idx + 1 < parts.len() {
            let form_name = parts[forms_idx + 1].clone();
            let object_context = if forms_idx >= 2 {
                format!("{}.{}", parts[forms_idx - 2], parts[forms_idx - 1])
            } else {
                String::new()
            };
            return (form_name, object_context);
        }
    }
    if let Some(ext_idx) = parts.iter().rposition(|part| part == "Ext") {
        if ext_idx >= 2 {
            return (parts[ext_idx - 1].clone(), parts[ext_idx - 2].clone());
        }
    }
    (
        form_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("Form")
            .to_string(),
        String::new(),
    )
}

pub fn form_child<'a>(
    node: roxmltree::Node<'a, 'a>,
    local_name: &str,
) -> Option<roxmltree::Node<'a, 'a>> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == local_name)
}

pub fn form_child_in_ns<'a>(
    node: roxmltree::Node<'a, 'a>,
    local_name: &str,
    namespace: &str,
) -> Option<roxmltree::Node<'a, 'a>> {
    node.children().find(|child| {
        child.is_element()
            && child.tag_name().name() == local_name
            && child.tag_name().namespace() == Some(namespace)
    })
}

pub fn form_validation_child<'a>(
    node: roxmltree::Node<'a, 'a>,
    local_name: &str,
) -> Option<roxmltree::Node<'a, 'a>> {
    form_child_in_ns(node, local_name, FORM_LOGFORM_NS)
}

pub fn form_children<'a>(
    node: roxmltree::Node<'a, 'a>,
    local_name: &str,
) -> Vec<roxmltree::Node<'a, 'a>> {
    node.children()
        .filter(|child| child.is_element() && child.tag_name().name() == local_name)
        .collect()
}

pub fn form_children_in_ns<'a>(
    node: roxmltree::Node<'a, 'a>,
    local_name: &str,
    namespace: &str,
) -> Vec<roxmltree::Node<'a, 'a>> {
    node.children()
        .filter(|child| {
            child.is_element()
                && child.tag_name().name() == local_name
                && child.tag_name().namespace() == Some(namespace)
        })
        .collect()
}

pub fn form_validation_children<'a>(
    node: roxmltree::Node<'a, 'a>,
    local_name: &str,
) -> Vec<roxmltree::Node<'a, 'a>> {
    form_children_in_ns(node, local_name, FORM_LOGFORM_NS)
}

pub fn form_child_text(node: roxmltree::Node<'_, '_>, local_name: &str) -> Option<String> {
    form_child(node, local_name)
        .map(form_ml_text)
        .filter(|value| !value.is_empty())
}

pub fn form_validation_child_text(
    node: roxmltree::Node<'_, '_>,
    local_name: &str,
) -> Option<String> {
    form_validation_child(node, local_name)
        .map(form_ml_text)
        .filter(|value| !value.is_empty())
}

pub fn form_ml_text(node: roxmltree::Node<'_, '_>) -> String {
    for item in form_children(node, "item") {
        for child in item.children().filter(|child| child.is_element()) {
            if child.tag_name().name() == "content" {
                if let Some(text) = child.text() {
                    if !text.is_empty() {
                        return text.to_string();
                    }
                }
            }
        }
    }
    node.text().unwrap_or("").trim().to_string()
}

pub fn form_event_lines(events: roxmltree::Node<'_, '_>) -> Vec<String> {
    form_children(events, "Event")
        .into_iter()
        .map(|event| {
            let name = event.attribute("name").unwrap_or("");
            let handler = event.text().unwrap_or("");
            let call_type = event.attribute("callType").unwrap_or("");
            let call_type = if call_type.is_empty() {
                String::new()
            } else {
                format!("[{call_type}]")
            };
            format!("  {name}{call_type} -> {handler}")
        })
        .collect()
}

pub fn form_main_command_bar_lines(acb: roxmltree::Node<'_, '_>) -> Vec<String> {
    let autofill = form_child_text(acb, "Autofill")
        .map(|value| value != "false")
        .unwrap_or(true);
    let h_align = form_child_text(acb, "HorizontalAlign");
    let mut flags = vec![if autofill { "autofill" } else { "no-autofill" }.to_string()];
    if let Some(align) = h_align {
        flags.push(format!("align={align}"));
    }

    let mut buttons = Vec::new();
    if let Some(child_items) = form_child(acb, "ChildItems") {
        for button in child_items.children().filter(|child| {
            child.is_element() && !form_skip_elements().contains(&child.tag_name().name())
        }) {
            let name = button.attribute("name").unwrap_or("");
            let cmd_ref = form_child_text(button, "CommandName").unwrap_or_default();
            let loc = form_child_text(button, "LocationInCommandBar")
                .map(|value| format!(" [{value}]"))
                .unwrap_or_default();
            let tag = form_element_tag(button);
            if cmd_ref.is_empty() {
                buttons.push(format!("  {tag} {name}{loc}"));
            } else {
                buttons.push(format!("  {tag} {name} -> {cmd_ref}{loc}"));
            }
        }
    }
    if buttons.is_empty() && autofill && flags.len() == 1 {
        return vec!["AutoCommandBar [autofill]".to_string()];
    }
    let mut lines = vec![format!("AutoCommandBar [{}]", flags.join(", "))];
    lines.extend(buttons);
    lines
}

pub struct FormTreeState {
    pub has_collapsed: bool,
}

pub fn form_build_tree(
    child_items: roxmltree::Node<'_, '_>,
    prefix: &str,
    tree_lines: &mut Vec<String>,
    expand: &str,
    state: &mut FormTreeState,
) {
    let children = child_items
        .children()
        .filter(|child| {
            child.is_element() && !form_skip_elements().contains(&child.tag_name().name())
        })
        .collect::<Vec<_>>();

    for (index, child) in children.iter().enumerate() {
        let last = index + 1 == children.len();
        let connector = if last { "└─" } else { "├─" };
        let continuation = if last { "  " } else { "│ " };
        let tag = form_element_tag(*child);
        let name = child.attribute("name").unwrap_or("");
        let flags = form_flags(*child);
        let events = form_events_str(*child);
        let binding = form_binding(*child);
        let title = form_title_differs(*child, name)
            .map(|title| format!(" [title:{title}]"))
            .unwrap_or_default();
        tree_lines.push(format!(
            "{prefix}{connector} {tag} {name}{binding}{flags}{title}{events}"
        ));

        match child.tag_name().name() {
            "Page" => {
                let child_items = form_child(*child, "ChildItems");
                let page_name = child.attribute("name").unwrap_or("");
                let page_title = form_title_differs(*child, page_name);
                let should_expand = expand == "*"
                    || expand == page_name
                    || page_title.as_deref().is_some_and(|title| expand == title);
                if should_expand {
                    if let Some(child_items) = child_items {
                        form_build_tree(
                            child_items,
                            &format!("{prefix}{continuation}"),
                            tree_lines,
                            expand,
                            state,
                        );
                    }
                } else {
                    let count = child_items
                        .map(form_count_significant_children)
                        .unwrap_or(0);
                    if let Some(line) = tree_lines.last_mut() {
                        line.push_str(&format!(" ({count} items)"));
                    }
                    state.has_collapsed = true;
                }
            }
            "UsualGroup" | "Pages" | "Table" | "ColumnGroup" | "CommandBar" | "ButtonGroup"
            | "Popup" => {
                if let Some(child_items) = form_child(*child, "ChildItems") {
                    form_build_tree(
                        child_items,
                        &format!("{prefix}{continuation}"),
                        tree_lines,
                        expand,
                        state,
                    );
                }
            }
            _ => {}
        }
    }
}

pub fn form_skip_elements() -> &'static [&'static str] {
    &[
        "ExtendedTooltip",
        "ContextMenu",
        "AutoCommandBar",
        "SearchStringAddition",
        "ViewStatusAddition",
        "SearchControlAddition",
    ]
}

pub fn form_element_tag(node: roxmltree::Node<'_, '_>) -> String {
    match node.tag_name().name() {
        "UsualGroup" => {
            let orient = match form_child_text(node, "Group").as_deref() {
                Some("Vertical") => ":V",
                Some("Horizontal") => ":H",
                Some("AlwaysHorizontal") => ":AH",
                Some("AlwaysVertical") => ":AV",
                _ => "",
            };
            let collapse = if form_child_text(node, "Behavior").as_deref() == Some("Collapsible") {
                ",collapse"
            } else {
                ""
            };
            format!("[Group{orient}{collapse}]")
        }
        "InputField" => "[Input]".to_string(),
        "CheckBoxField" => "[Check]".to_string(),
        "LabelDecoration" => "[Label]".to_string(),
        "LabelField" => "[LabelField]".to_string(),
        "PictureDecoration" => "[Picture]".to_string(),
        "PictureField" => "[PicField]".to_string(),
        "CalendarField" => "[Calendar]".to_string(),
        "Table" => "[Table]".to_string(),
        "Button" => "[Button]".to_string(),
        "CommandBar" => "[CmdBar]".to_string(),
        "Pages" => "[Pages]".to_string(),
        "Page" => "[Page]".to_string(),
        "Popup" => "[Popup]".to_string(),
        "ButtonGroup" => "[BtnGroup]".to_string(),
        other => format!("[{other}]"),
    }
}

pub fn form_flags(node: roxmltree::Node<'_, '_>) -> String {
    let mut flags = Vec::new();
    if form_child_text(node, "Visible").as_deref() == Some("false") {
        flags.push("visible:false");
    }
    if form_child_text(node, "Enabled").as_deref() == Some("false") {
        flags.push("enabled:false");
    }
    if form_child_text(node, "ReadOnly").as_deref() == Some("true") {
        flags.push("ro");
    }
    if flags.is_empty() {
        String::new()
    } else {
        format!(" [{}]", flags.join(","))
    }
}

pub fn form_events_str(node: roxmltree::Node<'_, '_>) -> String {
    let Some(events) = form_child(node, "Events") else {
        return String::new();
    };
    let events = form_children(events, "Event")
        .into_iter()
        .map(|event| {
            let name = event.attribute("name").unwrap_or("");
            let call_type = event.attribute("callType").unwrap_or("");
            if call_type.is_empty() {
                name.to_string()
            } else {
                format!("{name}[{call_type}]")
            }
        })
        .collect::<Vec<_>>();
    if events.is_empty() {
        String::new()
    } else {
        format!(" {{{}}}", events.join(", "))
    }
}

pub fn form_binding(node: roxmltree::Node<'_, '_>) -> String {
    if let Some(data_path) = form_child_text(node, "DataPath") {
        return format!(" -> {data_path}");
    }
    let Some(command_name) = form_child_text(node, "CommandName") else {
        return String::new();
    };
    if let Some(name) = command_name.strip_prefix("Form.StandardCommand.") {
        format!(" -> {name} [std]")
    } else if let Some(name) = command_name.strip_prefix("Form.Command.") {
        format!(" -> {name} [cmd]")
    } else {
        format!(" -> {command_name}")
    }
}

pub fn form_title_differs(node: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    let title = form_child(node, "Title").map(form_ml_text)?;
    if title.is_empty() || title.replace(' ', "").to_lowercase() == name.to_lowercase() {
        None
    } else {
        Some(title)
    }
}

pub fn form_count_significant_children(child_items: roxmltree::Node<'_, '_>) -> usize {
    child_items
        .children()
        .filter(|child| {
            child.is_element() && !form_skip_elements().contains(&child.tag_name().name())
        })
        .count()
}

pub fn form_attribute_lines(attrs: roxmltree::Node<'_, '_>) -> Vec<String> {
    form_children(attrs, "Attribute")
        .into_iter()
        .map(|attr| {
            let name = attr.attribute("name").unwrap_or("");
            let type_str = form_child(attr, "Type")
                .map(form_format_type)
                .unwrap_or_default();
            let is_main = form_child_text(attr, "MainAttribute").as_deref() == Some("true");
            let prefix = if is_main { "*" } else { " " };
            let main_suffix = if is_main { " (main)" } else { "" };
            let mut dyn_table = String::new();
            if type_str == "DynamicList" {
                if let Some(settings) = form_child(attr, "Settings") {
                    if let Some(main_table) = form_child_text(settings, "MainTable") {
                        dyn_table = format!(" -> {main_table}");
                    }
                }
            }
            let mut col_str = String::new();
            if matches!(type_str.as_str(), "ValueTable" | "ValueTree") {
                if let Some(columns) = form_child(attr, "Columns") {
                    let cols = form_children(columns, "Column")
                        .into_iter()
                        .map(|column| {
                            let column_name = column.attribute("name").unwrap_or("");
                            let column_type = form_child(column, "Type")
                                .map(form_format_type)
                                .unwrap_or_default();
                            if column_type.is_empty() {
                                column_name.to_string()
                            } else {
                                format!("{column_name}: {column_type}")
                            }
                        })
                        .collect::<Vec<_>>();
                    if !cols.is_empty() {
                        col_str = format!(" [{}]", cols.join(", "));
                    }
                }
            }
            if type_str.is_empty() && col_str.is_empty() && dyn_table.is_empty() {
                format!("  {prefix}{name}{main_suffix}")
            } else {
                format!("  {prefix}{name}: {type_str}{col_str}{dyn_table}{main_suffix}")
            }
        })
        .collect()
}

pub fn form_parameter_lines(params: roxmltree::Node<'_, '_>) -> Vec<String> {
    form_children(params, "Parameter")
        .into_iter()
        .map(|param| {
            let name = param.attribute("name").unwrap_or("");
            let type_str = form_child(param, "Type")
                .map(form_format_type)
                .unwrap_or_default();
            let key_suffix = if form_child_text(param, "KeyParameter").as_deref() == Some("true") {
                " (key)"
            } else {
                ""
            };
            if type_str.is_empty() {
                format!("  {name}{key_suffix}")
            } else {
                format!("  {name}: {type_str}{key_suffix}")
            }
        })
        .collect()
}

pub fn form_command_lines(commands: roxmltree::Node<'_, '_>) -> Vec<String> {
    form_children(commands, "Command")
        .into_iter()
        .map(|command| {
            let name = command.attribute("name").unwrap_or("");
            let shortcut = form_child_text(command, "Shortcut")
                .map(|value| format!(" [{value}]"))
                .unwrap_or_default();
            let actions = form_children(command, "Action");
            let action = if actions.len() > 1 {
                let parts = actions
                    .into_iter()
                    .map(|action| {
                        let text = action.text().unwrap_or("");
                        let call_type = action.attribute("callType").unwrap_or("");
                        if call_type.is_empty() {
                            text.to_string()
                        } else {
                            format!("{text}[{call_type}]")
                        }
                    })
                    .collect::<Vec<_>>();
                format!(" -> {}", parts.join(", "))
            } else if actions.len() == 1 {
                let action_node = actions[0];
                let text = action_node.text().unwrap_or("");
                let call_type = action_node.attribute("callType").unwrap_or("");
                if call_type.is_empty() {
                    format!(" -> {text}")
                } else {
                    format!(" -> {text}[{call_type}]")
                }
            } else {
                String::new()
            };
            format!("  {name}{action}{shortcut}")
        })
        .collect()
}

pub fn form_format_type(type_node: roxmltree::Node<'_, '_>) -> String {
    let mut parts = Vec::new();
    for type_item in form_children(type_node, "Type") {
        let raw = type_item.text().unwrap_or("");
        let part = match raw {
            "xs:string" => {
                let length = form_child(type_node, "StringQualifiers")
                    .and_then(|node| form_child_text(node, "Length"))
                    .unwrap_or_else(|| "0".to_string());
                let fixed = form_child(type_node, "StringQualifiers")
                    .and_then(|node| form_child_text(node, "AllowedLength"))
                    .as_deref()
                    == Some("Fixed");
                if length != "0" {
                    if fixed {
                        format!("string({length},fixed)")
                    } else {
                        format!("string({length})")
                    }
                } else {
                    "string".to_string()
                }
            }
            "xs:decimal" => {
                if let Some(qualifiers) = form_child(type_node, "NumberQualifiers") {
                    let digits =
                        form_child_text(qualifiers, "Digits").unwrap_or_else(|| "0".to_string());
                    let fraction = form_child_text(qualifiers, "FractionDigits")
                        .unwrap_or_else(|| "0".to_string());
                    let sign = if form_child_text(qualifiers, "AllowedSign").as_deref()
                        == Some("Nonnegative")
                    {
                        ",nonneg"
                    } else {
                        ""
                    };
                    format!("decimal({digits},{fraction}{sign})")
                } else {
                    "decimal".to_string()
                }
            }
            "xs:boolean" => "boolean".to_string(),
            "xs:dateTime" => match form_child(type_node, "DateQualifiers")
                .and_then(|node| form_child_text(node, "DateFractions"))
                .as_deref()
            {
                Some("Date") => "date".to_string(),
                Some("Time") => "time".to_string(),
                _ => "dateTime".to_string(),
            },
            "xs:binary" | "xs:base64Binary" => "binary".to_string(),
            "v8:ValueTable" => "ValueTable".to_string(),
            "v8:ValueTree" => "ValueTree".to_string(),
            "v8:ValueListType" => "ValueList".to_string(),
            "v8:TypeDescription" => "TypeDescription".to_string(),
            "v8:Universal" => "Universal".to_string(),
            "v8:FixedArray" => "FixedArray".to_string(),
            "v8:FixedStructure" => "FixedStructure".to_string(),
            "v8ui:FormattedString" => "FormattedString".to_string(),
            "v8ui:Picture" => "Picture".to_string(),
            "v8ui:Color" => "Color".to_string(),
            "v8ui:Font" => "Font".to_string(),
            other if other.starts_with("cfg:") => other[4..].to_string(),
            other => other.to_string(),
        };
        parts.push(part);
    }
    for type_set in form_children(type_node, "TypeSet") {
        let raw = type_set.text().unwrap_or("").trim();
        if !raw.is_empty() {
            parts.push(raw.strip_prefix("cfg:").unwrap_or(raw).to_string());
        }
    }
    for type_id in form_children(type_node, "TypeId") {
        let raw = type_id.text().unwrap_or("").trim();
        if !raw.is_empty() {
            parts.push(format!("typeid:{raw}"));
        }
    }
    parts.join(" | ")
}

pub fn validate_form_metadata_path_name(argument: &str, value: &str) -> Result<(), String> {
    let mut components = Path::new(value).components();
    let is_single_path_component = matches!(
        components.next(),
        Some(std::path::Component::Normal(component))
            if component == std::ffi::OsStr::new(value)
    ) && components.next().is_none();

    if unica_xml_format::names::xml_ncname_is_valid(value) && is_single_path_component {
        Ok(())
    } else {
        Err(format!(
            "{argument} must be a valid Unicode XML NCName and a single path component: {value:?}"
        ))
    }
}

pub fn remove_form_reference_elements(xml_text: &str, suffix: &str) -> (String, usize) {
    remove_form_reference_elements_with_parent_context(xml_text, suffix)
        .unwrap_or_else(|| rewrite_simple_form_references(xml_text, suffix, true))
}

pub fn clear_form_slot_references(xml_text: &str, suffix: &str) -> (String, Vec<String>) {
    let (text, _cleared_count) = rewrite_simple_form_references(xml_text, suffix, false);
    let mut cleared = Vec::new();
    let mut cursor = 0;
    while let Some(open_rel) = xml_text[cursor..].find('<') {
        let open_start = cursor + open_rel;
        let Some(open_end_rel) = xml_text[open_start..].find('>') else {
            break;
        };
        let open_end = open_start + open_end_rel;
        let tag = xml_text[open_start + 1..open_end]
            .split_whitespace()
            .next()
            .unwrap_or("");
        let local_name = tag.rsplit_once(':').map(|(_, name)| name).unwrap_or(tag);
        if local_name != "Form" && local_name.ends_with("Form") {
            let close = format!("</{tag}>");
            let content_start = open_end + 1;
            if let Some(close_rel) = xml_text[content_start..].find(&close) {
                let close_start = content_start + close_rel;
                let content = &xml_text[content_start..close_start];
                if !content.contains('<') && content.trim().ends_with(suffix) {
                    let tag_name = local_name.to_string();
                    if !cleared.contains(&tag_name) {
                        cleared.push(tag_name);
                    }
                }
                cursor = close_start + close.len();
                continue;
            }
        }
        cursor = open_end + 1;
    }
    (text, cleared)
}

fn rewrite_simple_form_references(
    xml_text: &str,
    suffix: &str,
    remove_form_elements: bool,
) -> (String, usize) {
    let mut result = String::with_capacity(xml_text.len());
    let mut cursor = 0;
    let mut changed = 0;
    while let Some(open_rel) = xml_text[cursor..].find('<') {
        let open_start = cursor + open_rel;
        let Some(open_end_rel) = xml_text[open_start..].find('>') else {
            break;
        };
        let open_end = open_start + open_end_rel;
        let raw_tag = &xml_text[open_start + 1..open_end];
        if raw_tag.starts_with('/')
            || raw_tag.starts_with('?')
            || raw_tag.starts_with('!')
            || raw_tag.ends_with('/')
        {
            result.push_str(&xml_text[cursor..=open_end]);
            cursor = open_end + 1;
            continue;
        }
        let tag = raw_tag.split_whitespace().next().unwrap_or("");
        let local_name = tag.rsplit_once(':').map(|(_, name)| name).unwrap_or(tag);
        let should_consider = if remove_form_elements {
            local_name == "Form"
        } else {
            local_name != "Form" && local_name.ends_with("Form")
        };
        if !should_consider {
            result.push_str(&xml_text[cursor..=open_end]);
            cursor = open_end + 1;
            continue;
        }
        let close = format!("</{tag}>");
        let content_start = open_end + 1;
        let Some(close_rel) = xml_text[content_start..].find(&close) else {
            result.push_str(&xml_text[cursor..=open_end]);
            cursor = open_end + 1;
            continue;
        };
        let close_start = content_start + close_rel;
        let close_end = close_start + close.len();
        let content = &xml_text[content_start..close_start];
        let trimmed = content.trim();
        let short_name = suffix
            .rsplit_once('.')
            .map(|(_, name)| name)
            .unwrap_or(suffix);
        let matches_reference = if remove_form_elements {
            trimmed == short_name || trimmed.ends_with(suffix)
        } else {
            trimmed.ends_with(suffix)
        };
        if content.contains('<') || !matches_reference {
            result.push_str(&xml_text[cursor..content_start]);
            cursor = content_start;
            continue;
        }
        let prefix = &xml_text[cursor..open_start];
        if !(remove_form_elements && prefix.trim().is_empty()) {
            result.push_str(prefix);
        }
        if !remove_form_elements {
            result.push_str(&xml_text[open_start..content_start]);
            result.push_str(&xml_text[close_start..close_end]);
        }
        cursor = if remove_form_elements {
            skip_xml_whitespace(xml_text, close_end)
        } else {
            close_end
        };
        changed += 1;
    }
    result.push_str(&xml_text[cursor..]);
    (result, changed)
}

#[derive(Debug)]
struct XmlTextReplacement {
    range: Range<usize>,
    replacement: String,
}

fn remove_form_reference_elements_with_parent_context(
    xml_text: &str,
    suffix: &str,
) -> Option<(String, usize)> {
    let parse_text = xml_text.trim_start_matches('\u{feff}');
    let offset = xml_text.len() - parse_text.len();
    let doc = Document::parse(parse_text).ok()?;
    let short_name = suffix
        .rsplit_once('.')
        .map(|(_, name)| name)
        .unwrap_or(suffix);
    let mut replacements = Vec::new();

    for node in doc.descendants().filter(|node| node.is_element()) {
        if node.tag_name().name() != "Form" {
            continue;
        }
        let trimmed = node.text().unwrap_or("").trim();
        if trimmed != short_name && !trimmed.ends_with(suffix) {
            continue;
        }
        let range = offset_xml_range(node.range(), offset);
        let parent = node.parent()?;
        if parent.is_element()
            && parent.tag_name().name() == "ChildObjects"
            && parent.children().all(|child| {
                child == node || (child.is_text() && child.text().unwrap_or("").trim().is_empty())
            })
        {
            let parent_range = offset_xml_range(parent.range(), offset);
            replacements.push(XmlTextReplacement {
                replacement: self_closing_xml_element(xml_text, &parent_range)?,
                range: parent_range,
            });
        } else {
            replacements.push(XmlTextReplacement {
                range: xml_element_line_range(xml_text, range),
                replacement: String::new(),
            });
        }
    }

    if replacements.is_empty() {
        return Some((xml_text.to_string(), 0));
    }
    replacements.sort_by_key(|replacement| std::cmp::Reverse(replacement.range.start));
    let mut updated = xml_text.to_string();
    for replacement in &replacements {
        updated.replace_range(replacement.range.clone(), &replacement.replacement);
    }
    Some((updated, replacements.len()))
}

fn offset_xml_range(range: Range<usize>, offset: usize) -> Range<usize> {
    range.start + offset..range.end + offset
}

fn xml_element_line_range(xml_text: &str, range: Range<usize>) -> Range<usize> {
    let line_start = xml_text[..range.start].rfind('\n').map_or(0, |pos| pos + 1);
    let prefix_is_indent = xml_text[line_start..range.start]
        .chars()
        .all(|ch| ch == '\t' || ch == ' ');
    let start = if prefix_is_indent {
        line_start
    } else {
        range.start
    };
    let rest = &xml_text[range.end..];
    let end = if rest.starts_with("\r\n") {
        range.end + 2
    } else if rest.starts_with('\n') || rest.starts_with('\r') {
        range.end + 1
    } else {
        range.end
    };
    start..end
}

fn self_closing_xml_element(xml_text: &str, range: &Range<usize>) -> Option<String> {
    let open_end = range.start + xml_text[range.start..].find('>')?;
    let raw_tag = &xml_text[range.start + 1..open_end];
    Some(format!("<{}/>", raw_tag.trim_end()))
}

pub fn collapse_empty_xml_elements(xml_text: &str, local_names: &[&str]) -> String {
    let mut result = String::with_capacity(xml_text.len());
    let mut cursor = 0;
    while let Some(open_rel) = xml_text[cursor..].find('<') {
        let open_start = cursor + open_rel;
        let Some(open_end_rel) = xml_text[open_start..].find('>') else {
            break;
        };
        let open_end = open_start + open_end_rel;
        let raw_tag = &xml_text[open_start + 1..open_end];
        if raw_tag.starts_with('/')
            || raw_tag.starts_with('?')
            || raw_tag.starts_with('!')
            || raw_tag.trim_end().ends_with('/')
        {
            result.push_str(&xml_text[cursor..=open_end]);
            cursor = open_end + 1;
            continue;
        }
        let tag = raw_tag.split_whitespace().next().unwrap_or("");
        let local_name = tag.rsplit_once(':').map(|(_, name)| name).unwrap_or(tag);
        if !local_names.contains(&local_name) {
            result.push_str(&xml_text[cursor..=open_end]);
            cursor = open_end + 1;
            continue;
        }
        let close = format!("</{tag}>");
        let content_start = open_end + 1;
        let Some(close_rel) = xml_text[content_start..].find(&close) else {
            result.push_str(&xml_text[cursor..=open_end]);
            cursor = open_end + 1;
            continue;
        };
        let close_start = content_start + close_rel;
        let close_end = close_start + close.len();
        let content = &xml_text[content_start..close_start];
        if !content.trim().is_empty() {
            result.push_str(&xml_text[cursor..=open_end]);
            cursor = open_end + 1;
            continue;
        }
        result.push_str(&xml_text[cursor..open_start]);
        result.push('<');
        result.push_str(raw_tag.trim_end());
        result.push_str("/>");
        cursor = close_end;
    }
    result.push_str(&xml_text[cursor..]);
    result
}

fn skip_xml_whitespace(xml_text: &str, mut cursor: usize) -> usize {
    let bytes = xml_text.as_bytes();
    while cursor < bytes.len() && matches!(bytes[cursor], b' ' | b'\t' | b'\r' | b'\n') {
        cursor += 1;
    }
    cursor
}

pub fn form_compile_normalize_from_object_output_label(
    output_label: &str,
) -> Option<(String, String)> {
    let trimmed = output_label.trim_end_matches(['/', '\\']);
    if trimmed.ends_with("/Ext/Form.xml") || trimmed.ends_with("\\Ext\\Form.xml") {
        return None;
    }
    let normalized = if trimmed.ends_with("/Ext") || trimmed.ends_with("\\Ext") {
        format!("{trimmed}/Form.xml")
    } else {
        format!("{trimmed}/Ext/Form.xml")
    };
    Some((
        normalized.clone(),
        format!("[resolved] OutputPath -> {normalized}\n"),
    ))
}

pub fn form_compile_infer_from_object_target(
    output_path: &Path,
    reader: &dyn ProjectDiscoveryRead,
    cwd: &Path,
) -> (Option<PathBuf>, Option<&'static str>) {
    let components = output_path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    let Some(forms_index) = components
        .iter()
        .rposition(|component| component == "Forms")
    else {
        return (None, None);
    };
    if forms_index < 2 || forms_index + 1 >= components.len() {
        return (None, None);
    }

    let form_name = components[forms_index + 1].as_str();
    let purpose = match form_name {
        "ФормаЭлемента" | "ФормаДокумента" | "ФормаСчета" => {
            Some("Item")
        }
        "ФормаГруппы" => Some("Folder"),
        "ФормаСписка" => Some("List"),
        "ФормаВыбора" => Some("Choice"),
        "ФормаЗаписи" => Some("Record"),
        _ => None,
    };

    let object_name = components[forms_index - 1].as_str();
    let mut object_path = PathBuf::new();
    for component in &components[..forms_index - 1] {
        object_path.push(component);
    }
    object_path.push(format!("{object_name}.xml"));
    let object_path = absolutize(object_path, cwd);
    if path_exists(reader, &object_path) {
        (Some(object_path), purpose)
    } else {
        (None, purpose)
    }
}

pub fn form_compile_definition_from_object(
    args: &Map<String, Value>,
    reader: &dyn ProjectDiscoveryRead,
    cwd: &Path,
    output_path: &Path,
) -> Result<(Value, String, PathBuf, Utf8TextSnapshot), String> {
    let (inferred_object_path, inferred_purpose) =
        form_compile_infer_from_object_target(output_path, reader, cwd);
    let (object_path, mut stdout) = if let Some(object_path_raw) =
        path_arg(args, &["objectPath", "ObjectPath"])
    {
        let mut object_path = absolutize(object_path_raw, cwd);
        if object_path.extension().is_none() {
            object_path.set_extension("xml");
        }
        (object_path, String::new())
    } else if let Some(object_path) = inferred_object_path {
        (
            object_path.clone(),
            format!("[resolved] ObjectPath -> {}\n", object_path.display()),
        )
    } else {
        return Err(
            "Cannot derive object path from OutputPath. Use -ObjectPath explicitly.".to_string(),
        );
    };
    if !path_exists(reader, &object_path) {
        return Err(format!("Object file not found: {}", object_path.display()));
    }
    let object_snapshot = read_utf8_sig_snapshot(reader, &object_path)?;
    let meta = form_compile_parse_object_meta(&object_snapshot.text)?;
    let purpose = string_arg(args, &["purpose", "Purpose"])
        .or(inferred_purpose)
        .unwrap_or("Item");
    if string_arg(args, &["purpose", "Purpose"]).is_none() && inferred_purpose.is_some() {
        stdout.push_str(&format!("[resolved] Purpose -> {purpose}\n"));
    }

    let defn = match (meta.object_type.as_str(), purpose) {
        ("Catalog", "List") => form_compile_catalog_list_definition(&meta),
        ("Catalog", "Item") => form_compile_catalog_item_definition(&meta),
        ("Catalog", other) => {
            return Err(format!(
                "native form compiler from-object currently supports Catalog List, Catalog Item, Document List, and Document Item only; got Catalog {other}"
            ));
        }
        ("Document", "List") => form_compile_document_list_definition(&meta),
        ("Document", "Item") => form_compile_document_item_definition(&meta),
        ("Document", other) => {
            return Err(format!(
                "native form compiler from-object currently supports Document List and Document Item only; got Document {other}"
            ));
        }
        (other, _) => {
            return Err(format!(
                "Object type '{other}' not supported. Supported: Catalog, Document."
            ));
        }
    };
    stdout.push_str(&format!(
        "[from-object] Type={}, Name={}, Attrs={}, TS={}\n",
        meta.object_type,
        meta.name,
        meta.attributes.len(),
        meta.tabular_sections.len()
    ));
    Ok((defn, stdout, object_path, object_snapshot))
}

pub fn form_compile_parse_object_meta(object_text: &str) -> Result<FormCompileObjectMeta, String> {
    let doc = Document::parse(object_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let root = doc.root_element();
    let type_node = root
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| "Not a 1C metadata XML".to_string())?;
    let object_type = type_node.tag_name().name().to_string();
    let props = unica_xml_format::read::child_element(type_node, "Properties")
        .ok_or_else(|| "No <Properties> element found".to_string())?;
    let name = unica_xml_format::read::child_direct_text(props, "Name").unwrap_or_default();
    let synonym = form_compile_meta_synonym(props).unwrap_or_else(|| name.clone());
    let child_objects = unica_xml_format::read::child_element(type_node, "ChildObjects");
    let attributes = child_objects
        .map(|node| form_compile_object_fields(node, "Attribute"))
        .unwrap_or_default();
    let tabular_sections = child_objects
        .map(form_compile_object_tabular_sections)
        .unwrap_or_default();
    let code_length = unica_xml_format::read::child_direct_text(props, "CodeLength")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    let hierarchical =
        unica_xml_format::read::child_direct_text(props, "Hierarchical").as_deref() == Some("true");
    let hierarchy_type =
        unica_xml_format::read::child_direct_text(props, "HierarchyType").unwrap_or_default();
    let owners = unica_xml_format::read::child_element(props, "Owners")
        .map(form_compile_meta_collection_values)
        .unwrap_or_default();

    Ok(FormCompileObjectMeta {
        object_type,
        name,
        synonym,
        attributes,
        tabular_sections,
        code_length,
        hierarchical,
        hierarchy_type,
        owners,
    })
}

pub fn form_compile_meta_synonym(props: roxmltree::Node<'_, '_>) -> Option<String> {
    let synonym = unica_xml_format::read::child_element(props, "Synonym")?;
    for item in unica_xml_format::read::child_elements(synonym, "item") {
        let lang = unica_xml_format::read::child_direct_text(item, "lang").unwrap_or_default();
        if lang == "ru" {
            if let Some(content) = unica_xml_format::read::child_direct_text(item, "content") {
                if !content.is_empty() {
                    return Some(content);
                }
            }
        }
    }
    unica_xml_format::read::child_element(synonym, "content")
        .map(unica_xml_format::read::direct_text)
        .filter(|value| !value.is_empty())
}

pub fn form_compile_object_fields(
    child_objects: roxmltree::Node<'_, '_>,
    tag_name: &str,
) -> Vec<FormCompileObjectField> {
    unica_xml_format::read::child_elements(child_objects, tag_name)
        .into_iter()
        .filter_map(|field| {
            let props = unica_xml_format::read::child_element(field, "Properties")?;
            let name = unica_xml_format::read::child_direct_text(props, "Name")?;
            let type_name = unica_xml_format::read::child_element(props, "Type")
                .map(form_compile_type_xml_text)
                .unwrap_or_else(|| "string".to_string());
            Some(FormCompileObjectField { name, type_name })
        })
        .collect()
}

pub fn form_compile_object_tabular_sections(
    child_objects: roxmltree::Node<'_, '_>,
) -> Vec<FormCompileObjectTabularSection> {
    unica_xml_format::read::child_elements(child_objects, "TabularSection")
        .into_iter()
        .filter_map(|tabular_section| {
            let props = unica_xml_format::read::child_element(tabular_section, "Properties")?;
            let name = unica_xml_format::read::child_direct_text(props, "Name")?;
            let synonym = form_compile_meta_synonym(props).unwrap_or_else(|| name.clone());
            let columns = unica_xml_format::read::child_element(tabular_section, "ChildObjects")
                .map(|node| form_compile_object_fields(node, "Attribute"))
                .unwrap_or_default();
            Some(FormCompileObjectTabularSection {
                name,
                synonym,
                columns,
            })
        })
        .collect()
}

pub fn form_compile_meta_collection_values(node: roxmltree::Node<'_, '_>) -> Vec<String> {
    node.descendants()
        .filter(|child| child.is_element() && child != &node)
        .filter_map(|child| child.text())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub fn form_compile_type_xml_text(type_node: roxmltree::Node<'_, '_>) -> String {
    let type_name = form_format_type(type_node);
    if type_name.is_empty() {
        "string".to_string()
    } else {
        type_name
    }
}

pub fn form_compile_displayable_type(type_name: &str) -> bool {
    !["ValueStorage", "v8:ValueStorage", "ХранилищеЗначения"]
        .iter()
        .any(|needle| type_name.contains(needle))
}

pub fn form_compile_catalog_list_definition(meta: &FormCompileObjectMeta) -> Value {
    let mut columns = Vec::new();
    columns.push(json!({"labelField": "Наименование", "path": "Список.Description"}));
    if meta.code_length > 0 {
        columns.push(json!({"labelField": "Код", "path": "Список.Code"}));
    }
    for attr in &meta.attributes {
        if form_compile_displayable_type(&attr.type_name) {
            columns.push(json!({
                "labelField": attr.name,
                "path": format!("Список.{}", attr.name),
            }));
        }
    }
    columns.push(json!({
        "labelField": "Ссылка",
        "path": "Список.Ref",
        "userVisible": false,
    }));

    let mut table = json!({
        "table": "Список",
        "path": "Список",
        "rowPictureDataPath": "Список.DefaultPicture",
        "commandBarLocation": "None",
        "tableAutofill": false,
        "_dynList": true,
        "columns": columns,
    });
    if meta.hierarchical {
        if let Some(object) = table.as_object_mut() {
            object.insert("initialTreeView".to_string(), json!("ExpandTopLevel"));
            object.insert("enableStartDrag".to_string(), json!(true));
            object.insert("enableDrag".to_string(), json!(true));
        }
    }

    json!({
        "title": meta.synonym,
        "elements": [table],
        "attributes": [{
            "name": "Список",
            "type": "DynamicList",
            "main": true,
            "settings": {
                "mainTable": format!("Catalog.{}", meta.name),
                "dynamicDataRead": true,
            },
        }],
    })
}

pub fn form_compile_catalog_item_definition(meta: &FormCompileObjectMeta) -> Value {
    let mut header_children = Vec::new();
    if !meta.owners.is_empty() {
        header_children.push(json!({
            "input": "Владелец",
            "path": "Объект.Owner",
            "readOnly": true,
        }));
    }

    if meta.code_length > 0 {
        header_children.push(json!({
            "group": "horizontal",
            "name": "ГруппаКодНаименование",
            "showTitle": false,
            "representation": "none",
            "children": [
                {"input": "Наименование", "path": "Объект.Description"},
                {"input": "Код", "path": "Объект.Code"},
            ],
        }));
    } else {
        header_children.push(json!({"input": "Наименование", "path": "Объект.Description"}));
    }

    if meta.hierarchical {
        header_children.push(json!({
            "input": "Родитель",
            "path": "Объект.Parent",
            "title": "Входит в группу",
        }));
    }

    for attr in &meta.attributes {
        if form_compile_displayable_type(&attr.type_name) {
            header_children.push(form_compile_object_field_element(
                &attr.name,
                &format!("Объект.{}", attr.name),
                &attr.type_name,
            ));
        }
    }

    let mut root_elements = vec![json!({
        "group": "vertical",
        "name": "ГруппаШапка",
        "showTitle": false,
        "representation": "none",
        "children": header_children,
    })];

    for tabular_section in meta.tabular_sections.iter().filter(|section| {
        section.name != "ДополнительныеРеквизиты" && section.name != "Представления"
    }) {
        let mut columns = vec![json!({
            "labelField": format!("{}НомерСтроки", tabular_section.name),
            "path": format!("Объект.{}.LineNumber", tabular_section.name),
        })];
        for column in &tabular_section.columns {
            columns.push(form_compile_object_field_element(
                &format!("{}{}", tabular_section.name, column.name),
                &format!("Объект.{}.{}", tabular_section.name, column.name),
                &column.type_name,
            ));
        }
        root_elements.push(json!({
            "table": tabular_section.name,
            "path": format!("Объект.{}", tabular_section.name),
            "columns": columns,
        }));
    }

    root_elements.push(json!({
        "group": "vertical",
        "name": "ГруппаДополнительныеРеквизиты",
    }));

    let mut defn = json!({
        "title": meta.synonym,
        "properties": {},
        "elements": root_elements,
        "attributes": [{
            "name": "Объект",
            "type": format!("CatalogObject.{}", meta.name),
            "main": true,
        }],
    });
    if meta.hierarchical && meta.hierarchy_type == "HierarchyFoldersAndItems" {
        if let Some(properties) = defn.get_mut("properties").and_then(Value::as_object_mut) {
            properties.insert("useForFoldersAndItems".to_string(), json!("Items"));
        }
    }
    defn
}

pub fn form_compile_document_list_definition(meta: &FormCompileObjectMeta) -> Value {
    let mut columns = Vec::new();
    columns.push(json!({"labelField": "Номер", "path": "Список.Number"}));
    columns.push(json!({"labelField": "Дата", "path": "Список.Date"}));
    for attr in &meta.attributes {
        if form_compile_displayable_type(&attr.type_name) {
            columns.push(json!({
                "labelField": attr.name,
                "path": format!("Список.{}", attr.name),
            }));
        }
    }
    columns.push(json!({
        "labelField": "Ссылка",
        "path": "Список.Ref",
        "userVisible": false,
    }));

    json!({
        "title": meta.synonym,
        "properties": {},
        "elements": [{
            "table": "Список",
            "path": "Список",
            "rowPictureDataPath": "Список.DefaultPicture",
            "commandBarLocation": "None",
            "tableAutofill": false,
            "_dynList": true,
            "columns": columns,
        }],
        "attributes": [{
            "name": "Список",
            "type": "DynamicList",
            "main": true,
            "settings": {
                "mainTable": format!("Document.{}", meta.name),
                "dynamicDataRead": true,
            },
        }],
    })
}

pub fn form_compile_document_item_definition(meta: &FormCompileObjectMeta) -> Value {
    let footer_fields = ["Комментарий"];
    let mut claimed = HashSet::<&str>::new();
    for field in footer_fields {
        claimed.insert(field);
    }

    let unclaimed = meta
        .attributes
        .iter()
        .filter(|attr| {
            !claimed.contains(attr.name.as_str()) && form_compile_displayable_type(&attr.type_name)
        })
        .collect::<Vec<_>>();
    let half = unclaimed.len().div_ceil(2);
    let (left_attrs, right_attrs) = unclaimed.split_at(half);

    let number_date_group = json!({
        "group": "horizontal",
        "name": "ГруппаНомерДата",
        "showTitle": false,
        "children": [
            {"input": "Номер", "path": "Объект.Number", "autoMaxWidth": false, "width": 9},
            {"input": "Дата", "path": "Объект.Date", "title": "от"},
        ],
    });

    let mut left_children = vec![number_date_group];
    for attr in left_attrs {
        left_children.push(form_compile_object_field_element(
            &attr.name,
            &format!("Объект.{}", attr.name),
            &attr.type_name,
        ));
    }

    let mut right_children = Vec::new();
    for attr in right_attrs {
        right_children.push(form_compile_object_field_element(
            &attr.name,
            &format!("Объект.{}", attr.name),
            &attr.type_name,
        ));
    }

    let header_children = if right_children.is_empty() {
        vec![json!({
            "group": "vertical",
            "name": "ГруппаШапкаЛево",
            "showTitle": false,
            "children": left_children,
        })]
    } else {
        vec![
            json!({
                "group": "vertical",
                "name": "ГруппаШапкаЛево",
                "showTitle": false,
                "children": left_children,
            }),
            json!({
                "group": "vertical",
                "name": "ГруппаШапкаПраво",
                "showTitle": false,
                "children": right_children,
            }),
        ]
    };

    let header_group = json!({
        "group": "horizontal",
        "name": "ГруппаШапка",
        "showTitle": false,
        "representation": "none",
        "children": header_children,
    });

    let mut main_page_children = vec![header_group];
    for field in footer_fields {
        if let Some(attr) = meta.attributes.iter().find(|attr| attr.name == field) {
            main_page_children.push(form_compile_object_field_element(
                &attr.name,
                &format!("Объект.{}", attr.name),
                &attr.type_name,
            ));
        }
    }

    let mut pages_children = vec![json!({
        "page": "ГруппаОсновное",
        "title": "Основное",
        "children": main_page_children,
    })];

    for tabular_section in meta
        .tabular_sections
        .iter()
        .filter(|section| section.name != "ДополнительныеРеквизиты")
    {
        let mut columns = vec![json!({
            "labelField": format!("{}НомерСтроки", tabular_section.name),
            "path": format!("Объект.{}.LineNumber", tabular_section.name),
        })];
        for column in &tabular_section.columns {
            columns.push(form_compile_object_field_element(
                &format!("{}{}", tabular_section.name, column.name),
                &format!("Объект.{}.{}", tabular_section.name, column.name),
                &column.type_name,
            ));
        }
        pages_children.push(json!({
            "page": format!("Группа{}", tabular_section.name),
            "title": tabular_section.synonym,
            "children": [{
                "table": tabular_section.name,
                "path": format!("Объект.{}", tabular_section.name),
                "columns": columns,
            }],
        }));
    }

    pages_children.push(json!({
        "page": "ГруппаДополнительно",
        "title": "Дополнительно",
        "children": [
            {
                "group": "horizontal",
                "name": "ГруппаПараметры",
                "showTitle": false,
                "children": [
                    {"group": "vertical", "name": "ГруппаПараметрыЛево", "showTitle": false, "children": []},
                    {"group": "vertical", "name": "ГруппаПараметрыПраво", "showTitle": false, "children": []},
                ],
            },
            {"group": "vertical", "name": "ГруппаДополнительныеРеквизиты"},
        ],
    }));

    json!({
        "title": meta.synonym,
        "properties": {
            "autoTitle": false,
        },
        "elements": [{
            "pages": "ГруппаСтраницы",
            "children": pages_children,
        }],
        "attributes": [{
            "name": "Объект",
            "type": format!("DocumentObject.{}", meta.name),
            "main": true,
        }],
    })
}

pub fn form_compile_object_field_element(name: &str, path: &str, type_name: &str) -> Value {
    if type_name.trim() == "xs:boolean" || type_name == "boolean" || type_name.contains("Boolean") {
        json!({"check": name, "path": path})
    } else {
        json!({"input": name, "path": path})
    }
}

pub fn form_add_supported_object_types() -> &'static [&'static str] {
    &[
        "Document",
        "Catalog",
        "Enum",
        "DataProcessor",
        "Report",
        "ExternalDataProcessor",
        "ExternalReport",
        "InformationRegister",
        "AccumulationRegister",
        "ChartOfAccounts",
        "ChartOfCharacteristicTypes",
        "ExchangePlan",
        "BusinessProcess",
        "Task",
    ]
}

pub fn form_add_processor_like(object_type: &str) -> bool {
    matches!(
        object_type,
        "DataProcessor" | "Report" | "ExternalDataProcessor" | "ExternalReport"
    )
}

pub fn normalize_form_purpose(value: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    format!(
        "{}{}",
        first.to_uppercase().collect::<String>(),
        chars.as_str().to_lowercase()
    )
}

/// `lang` is the `LanguageCode` the synonym is written in.
pub fn form_add_metadata_xml(
    form_name: &str,
    synonym: &str,
    object_type: &str,
    format_version: &str,
    form_uuid: &str,
    lang: &str,
) -> String {
    let extended_presentation = if form_add_processor_like(object_type) {
        "\t\t\t<ExtendedPresentation/>\r\n"
    } else {
        ""
    };
    format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\"",
            " xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\"",
            " xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\"",
            " xmlns:cmi=\"http://v8.1c.ru/8.2/managed-application/cmi\"",
            " xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\"",
            " xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\"",
            " xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\"",
            " xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\"",
            " xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"",
            " xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\"",
            " xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\"",
            " xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\"",
            " xmlns:xen=\"http://v8.1c.ru/8.3/xcf/enums\"",
            " xmlns:xpr=\"http://v8.1c.ru/8.3/xcf/predef\"",
            " xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\"",
            " xmlns:xs=\"http://www.w3.org/2001/XMLSchema\"",
            " xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"",
            " version=\"{format_version}\">\r\n",
            "\t<Form uuid=\"{form_uuid}\">\r\n",
            "\t\t<Properties>\r\n",
            "\t\t\t<Name>{form_name}</Name>\r\n",
            "\t\t\t<Synonym>\r\n",
            "\t\t\t\t<v8:item>\r\n",
            "\t\t\t\t\t<v8:lang>{lang}</v8:lang>\r\n",
            "\t\t\t\t\t<v8:content>{synonym}</v8:content>\r\n",
            "\t\t\t\t</v8:item>\r\n",
            "\t\t\t</Synonym>\r\n",
            "\t\t\t<Comment/>\r\n",
            "\t\t\t<FormType>Managed</FormType>\r\n",
            "\t\t\t<IncludeHelpInContents>false</IncludeHelpInContents>\r\n",
            "\t\t\t<UsePurposes>\r\n",
            "\t\t\t\t<v8:Value xsi:type=\"app:ApplicationUsePurpose\">PlatformApplication</v8:Value>\r\n",
            "\t\t\t\t<v8:Value xsi:type=\"app:ApplicationUsePurpose\">MobilePlatformApplication</v8:Value>\r\n",
            "\t\t\t</UsePurposes>\r\n",
            "{extended_presentation}",
            "\t\t</Properties>\r\n",
            "\t</Form>\r\n",
            "</MetaDataObject>"
        ),
        format_version = unica_xml_format::text::escape_xml(format_version),
        form_uuid = unica_xml_format::text::escape_xml(form_uuid),
        form_name = unica_xml_format::text::escape_xml(form_name),
        synonym = unica_xml_format::text::escape_xml(synonym),
        extended_presentation = extended_presentation,
        lang = unica_xml_format::text::escape_xml(lang),
    )
}

/// Namespace declarations of a `Form.xml` root as 8.3.27 exports them.
const FORM_ROOT_NAMESPACES: &str = concat!(
    "xmlns=\"http://v8.1c.ru/8.3/xcf/logform\"",
    " xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\"",
    " xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\"",
    " xmlns:dcscor=\"http://v8.1c.ru/8.1/data-composition-system/core\"",
    " xmlns:dcssch=\"http://v8.1c.ru/8.1/data-composition-system/schema\"",
    " xmlns:dcsset=\"http://v8.1c.ru/8.1/data-composition-system/settings\"",
    " xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\"",
    " xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\"",
    " xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\"",
    " xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\"",
    " xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"",
    " xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\"",
    " xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\"",
    " xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\"",
    " xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\"",
    " xmlns:xs=\"http://www.w3.org/2001/XMLSchema\"",
    " xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\""
);

/// `Form.xml` of an object form borrowed into an extension, byte for byte
/// as 8.3.27 exports it (#1982): the own part and `BaseForm` hold only the
/// empty command bar and attributes, so the borrowed form changes nothing.
/// Parent events and commands are not copied: the platform would turn them
/// into `callType="Before"` interceptors of the extension.
pub fn borrowed_form_content_xml(format_version: &str) -> String {
    let version = unica_xml_format::text::escape_xml(format_version);
    [
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string(),
        format!("<Form {FORM_ROOT_NAMESPACES} version=\"{version}\">"),
        "\t<AutoCommandBar name=\"\" id=\"-1\"/>".to_string(),
        "\t<Attributes/>".to_string(),
        format!("\t<BaseForm version=\"{version}\">"),
        "\t\t<AutoCommandBar name=\"\" id=\"-1\"/>".to_string(),
        "\t\t<Attributes/>".to_string(),
        "\t</BaseForm>".to_string(),
        "</Form>".to_string(),
    ]
    .join("\r\n")
}

pub fn form_add_content_xml(
    object_type: &str,
    object_name: &str,
    purpose: &str,
    format_version: &str,
) -> Result<String, String> {
    let ns = FORM_ROOT_NAMESPACES;
    if matches!(purpose, "List" | "Choice") {
        let main_table = format!("{object_type}.{object_name}");
        return Ok(format!(
            concat!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
                "<Form {ns} version=\"{format_version}\">\r\n",
                "\t<AutoCommandBar name=\"ФормаКоманднаяПанель\" id=\"-1\"/>\r\n",
                "\t<Attributes>\r\n",
                "\t\t<Attribute name=\"Список\" id=\"1\">\r\n",
                "\t\t\t<Type>\r\n",
                "\t\t\t\t<v8:Type>cfg:DynamicList</v8:Type>\r\n",
                "\t\t\t</Type>\r\n",
                "\t\t\t<MainAttribute>true</MainAttribute>\r\n",
                "\t\t\t<Settings xsi:type=\"DynamicList\">\r\n",
                "\t\t\t\t<MainTable>{main_table}</MainTable>\r\n",
                "\t\t\t</Settings>\r\n",
                "\t\t</Attribute>\r\n",
                "\t</Attributes>\r\n",
                "</Form>"
            ),
            ns = ns,
            format_version = unica_xml_format::text::escape_xml(format_version),
            main_table = unica_xml_format::text::escape_xml(&main_table),
        ));
    }
    if purpose == "Record" {
        let main_attr_type = format!("InformationRegisterRecordManager.{object_name}");
        return Ok(format!(
            concat!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
                "<Form {ns} version=\"{format_version}\">\r\n",
                "\t<AutoCommandBar name=\"ФормаКоманднаяПанель\" id=\"-1\"/>\r\n",
                "\t<Attributes>\r\n",
                "\t\t<Attribute name=\"Запись\" id=\"1\">\r\n",
                "\t\t\t<Type>\r\n",
                "\t\t\t\t<v8:Type>cfg:{main_attr_type}</v8:Type>\r\n",
                "\t\t\t</Type>\r\n",
                "\t\t\t<MainAttribute>true</MainAttribute>\r\n",
                "\t\t\t<SavedData>true</SavedData>\r\n",
                "\t\t</Attribute>\r\n",
                "\t</Attributes>\r\n",
                "</Form>"
            ),
            ns = ns,
            format_version = unica_xml_format::text::escape_xml(format_version),
            main_attr_type = unica_xml_format::text::escape_xml(&main_attr_type),
        ));
    }

    let attr_prefix = match object_type {
        "Document" => "DocumentObject",
        "Catalog" => "CatalogObject",
        "DataProcessor" => "DataProcessorObject",
        "Report" => "ReportObject",
        "ExternalDataProcessor" => "ExternalDataProcessorObject",
        "ExternalReport" => "ExternalReportObject",
        "ChartOfAccounts" => "ChartOfAccountsObject",
        "ChartOfCharacteristicTypes" => "ChartOfCharacteristicTypesObject",
        "ExchangePlan" => "ExchangePlanObject",
        "BusinessProcess" => "BusinessProcessObject",
        "Task" => "TaskObject",
        "InformationRegister" => "InformationRegisterRecordManager",
        "AccumulationRegister" => "AccumulationRegisterRecordSet",
        other => return Err(format!("unsupported form object type: {other}")),
    };
    let main_attr_type = format!("{attr_prefix}.{object_name}");
    let saved_data_line = if form_add_processor_like(object_type) {
        ""
    } else {
        "\t\t\t<SavedData>true</SavedData>\r\n"
    };
    let root_defaults = match object_type {
        "Catalog" => "\t<UseForFoldersAndItems>Items</UseForFoldersAndItems>\r\n",
        // 1C 8.3.27 always exports the posting properties of a document form.
        "Document" => concat!(
            "\t<AutoTime>CurrentOrLast</AutoTime>\r\n",
            "\t<UsePostingMode>Auto</UsePostingMode>\r\n",
            "\t<RepostOnWrite>true</RepostOnWrite>\r\n"
        ),
        "Report" | "ExternalReport" => concat!(
            "\t<ReportFormType>Main</ReportFormType>\r\n",
            "\t<AutoShowState>Auto</AutoShowState>\r\n",
            "\t<ReportResultViewMode>Auto</ReportResultViewMode>\r\n",
            "\t<ViewModeApplicationOnSetReportResult>Auto</ViewModeApplicationOnSetReportResult>\r\n"
        ),
        _ => "",
    };
    Ok(format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
            "<Form {ns} version=\"{format_version}\">\r\n",
            "{root_defaults}",
            "\t<AutoCommandBar name=\"ФормаКоманднаяПанель\" id=\"-1\"/>\r\n",
            "\t<Attributes>\r\n",
            "\t\t<Attribute name=\"Объект\" id=\"1\">\r\n",
            "\t\t\t<Type>\r\n",
            "\t\t\t\t<v8:Type>cfg:{main_attr_type}</v8:Type>\r\n",
            "\t\t\t</Type>\r\n",
            "\t\t\t<MainAttribute>true</MainAttribute>\r\n",
            "{saved_data_line}",
            "\t\t</Attribute>\r\n",
            "\t</Attributes>\r\n",
            "</Form>"
        ),
        ns = ns,
        format_version = unica_xml_format::text::escape_xml(format_version),
        root_defaults = root_defaults,
        main_attr_type = unica_xml_format::text::escape_xml(&main_attr_type),
        saved_data_line = saved_data_line,
    ))
}

pub fn form_add_module_bsl() -> &'static str {
    concat!(
        "#Область ОбработчикиСобытийФормы\r\n\r\n",
        "#КонецОбласти\r\n\r\n",
        "#Область ОбработчикиСобытийЭлементовФормы\r\n\r\n",
        "#КонецОбласти\r\n\r\n",
        "#Область ОбработчикиКомандФормы\r\n\r\n",
        "#КонецОбласти\r\n\r\n",
        "#Область ОбработчикиОповещений\r\n\r\n",
        "#КонецОбласти\r\n\r\n",
        "#Область СлужебныеПроцедурыИФункции\r\n\r\n",
        "#КонецОбласти"
    )
}

#[cfg(test)]
use crate::test_support::assert_platform_text_uses_crlf_without_bare_lf;

pub fn form_default_property<'a>(object_type: &str, purpose: &'a str) -> &'a str {
    match purpose {
        "Object" => {
            if form_add_processor_like(object_type) {
                "DefaultForm"
            } else {
                "DefaultObjectForm"
            }
        }
        "List" => "DefaultListForm",
        "Choice" => "DefaultChoiceForm",
        "Record" => "DefaultRecordForm",
        _ => "DefaultForm",
    }
}

pub fn replace_form_default_property(
    text: &str,
    prop_name: &str,
    default_value: &str,
    overwrite: bool,
) -> (String, bool) {
    let empty = format!("<{prop_name}/>");
    if text.contains(&empty) {
        return (
            text.replacen(
                &empty,
                &format!("<{prop_name}>{default_value}</{prop_name}>"),
                1,
            ),
            true,
        );
    }
    let start_tag = format!("<{prop_name}>");
    let end_tag = format!("</{prop_name}>");
    let Some(start) = text.find(&start_tag) else {
        return (text.to_string(), false);
    };
    let value_start = start + start_tag.len();
    let Some(relative_end) = text[value_start..].find(&end_tag) else {
        return (text.to_string(), false);
    };
    let value_end = value_start + relative_end;
    if !overwrite && !text[value_start..value_end].trim().is_empty() {
        return (text.to_string(), false);
    }
    (
        format!(
            "{}{}{}",
            &text[..value_start],
            default_value,
            &text[value_end..]
        ),
        true,
    )
}

pub struct FormCompilePlan {
    pub output_label: String,
    pub output_path: PathBuf,
    pub stdout: String,
    pub xml: String,
    pub stats: FormCompileStats,
    pub derivation_inputs: Vec<FormCompileDerivationInput>,
}

pub struct FormCompileDerivationInput {
    pub path: PathBuf,
    pub snapshot: Utf8TextSnapshot,
    pub platform_xml: bool,
}

pub struct FormParentRegistrationPlan {
    pub path: PathBuf,
    pub original: Vec<u8>,
    pub replacement: Vec<u8>,
    pub stdout: String,
}

pub fn has_compile_payload(args: &Map<String, Value>) -> bool {
    const KEYS: &[&str] = &[
        "JsonPath",
        "jsonPath",
        "FromObject",
        "fromObject",
        "ObjectPath",
        "objectPath",
        "OutputPath",
        "outputPath",
    ];
    KEYS.iter().any(|key| args.contains_key(*key))
}

pub fn plan_form_compile(
    args: &Map<String, Value>,
    reader: &dyn ProjectDiscoveryRead,
    cwd: &Path,
    detect_format: &dyn Fn(&Path) -> Result<String, String>,
) -> Result<FormCompilePlan, String> {
    let json_path_raw = path_arg(args, &["jsonPath", "JsonPath"]);
    let from_object =
        unica_operation_contract::input::bool_arg(args, &["fromObject", "FromObject"]);
    if from_object && json_path_raw.is_some() {
        return Err("Cannot use both -JsonPath and -FromObject. Choose one mode.".to_string());
    }
    if !from_object && json_path_raw.is_none() {
        return Err("Either -JsonPath or -FromObject is required.".to_string());
    }

    let mut output_label = string_arg(args, &["outputPath", "OutputPath"])
        .ok_or_else(|| "missing required OutputPath argument".to_string())?
        .to_string();
    let mut stdout = String::new();
    if from_object {
        if let Some((normalized, resolved_line)) =
            form_compile_normalize_from_object_output_label(&output_label)
        {
            output_label = normalized;
            stdout.push_str(&resolved_line);
        }
    }
    let output_path = absolutize(PathBuf::from(&output_label), cwd);
    validate_form_compile_output_path(&output_path)?;
    let mut derivation_inputs = Vec::new();
    let defn = if from_object {
        let (defn, from_object_stdout, object_path, object_snapshot) =
            form_compile_definition_from_object(args, reader, cwd, &output_path)?;
        stdout.push_str(&from_object_stdout);
        derivation_inputs.push(FormCompileDerivationInput {
            path: object_path,
            snapshot: object_snapshot,
            platform_xml: true,
        });
        defn
    } else {
        let json_path_raw = json_path_raw
            .ok_or_else(|| "Either -JsonPath or -FromObject is required.".to_string())?;
        let json_path = absolutize(json_path_raw.clone(), cwd);
        if !path_exists(reader, &json_path) {
            return Err(format!("File not found: {}", json_path_raw.display()));
        }
        let json_snapshot = read_utf8_sig_snapshot(reader, &json_path)?;
        let definition = serde_json::from_str(json_snapshot.text.as_str())
            .map_err(|err| format!("failed to parse Form JSON: {err}"))?;
        derivation_inputs.push(FormCompileDerivationInput {
            path: json_path,
            snapshot: json_snapshot,
            platform_xml: false,
        });
        definition
    };

    let format_version = detect_format(output_path.parent().unwrap_or(cwd))?;
    let (xml, stats) = form_compile_xml(&defn, &format_version)?;
    Ok(FormCompilePlan {
        output_label,
        output_path,
        stdout,
        xml,
        stats,
        derivation_inputs,
    })
}

fn validate_form_compile_output_path(output_path: &Path) -> Result<(), String> {
    let components = output_path.components().collect::<Vec<_>>();
    if !matches!(
        components.last(),
        Some(std::path::Component::Normal(value))
            if *value == std::ffi::OsStr::new("Form.xml")
    ) || !matches!(
        components.get(components.len().saturating_sub(2)),
        Some(std::path::Component::Normal(value)) if *value == std::ffi::OsStr::new("Ext")
    ) {
        return Ok(());
    }
    let Some(forms_index) = components.iter().rposition(|component| {
        matches!(
            component,
            std::path::Component::Normal(value) if *value == std::ffi::OsStr::new("Forms")
        )
    }) else {
        return Ok(());
    };
    let suffix = &components[forms_index + 1..];
    if suffix.is_empty() {
        return Ok(());
    }
    if suffix.iter().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::CurDir
        )
    }) {
        return Err(format!(
            "OutputPath form name must be a valid Unicode XML NCName and a single path component: {:?}",
            output_path
        ));
    }
    let std::path::Component::Normal(form_name) = suffix[0] else {
        return Err(format!(
            "OutputPath form name must be a valid Unicode XML NCName and a single path component: {:?}",
            output_path
        ));
    };
    let form_name = form_name.to_str().ok_or_else(|| {
        format!(
            "OutputPath form name must be valid UTF-8, a Unicode XML NCName, and a single path component: {:?}",
            output_path
        )
    })?;
    validate_form_metadata_path_name("OutputPath form name", form_name)
}

pub fn append_form_compile_stats(stdout: &mut String, stats: &FormCompileStats) {
    stdout.push_str(&format!("     Elements+IDs: {}\n", stats.element_ids));
    if stats.attributes > 0 {
        stdout.push_str(&format!("     Attributes: {}\n", stats.attributes));
    }
    if stats.commands > 0 {
        stdout.push_str(&format!("     Commands: {}\n", stats.commands));
    }
    if stats.parameters > 0 {
        stdout.push_str(&format!("     Parameters: {}\n", stats.parameters));
    }
}

pub fn has_edit_payload(args: &Map<String, Value>) -> bool {
    const KEYS: &[&str] = &[
        "FormPath",
        "formPath",
        "Path",
        "path",
        "JsonPath",
        "jsonPath",
        "definition",
    ];
    args.keys().any(|key| KEYS.contains(&key.as_str()))
}

/// Applies one managed-form definition (`elements`, `attributes`,
/// `commands`, `events`, `removeElements`, `into`, `after`) to the form text
/// in memory. This is the single source of the form transforms: the legacy
/// editor and the v0.13 staged planner both route through it. The text must
/// already be a parsed managed form (`form_root_start` is its root offset).
pub fn form_edit_apply_definition(
    xml_text: &mut String,
    defn: &Value,
    form_path: &Path,
    form_root_start: usize,
    lang: Option<&str>,
) -> Result<FormEditApplied, String> {
    // The fragments are built with LF. A platform export ends every line
    // with CRLF, so in such a form each bare LF after the edit is a new one
    // and takes the form's line ending (#1996). A form with LF or mixed
    // endings keeps the inserted LF.
    let crlf_only = xml_text.contains("\r\n") && !has_bare_lf(xml_text) && !has_bare_cr(xml_text);
    let applied = form_edit_apply_definition_lf(xml_text, defn, form_path, form_root_start, lang)?;
    if crlf_only && has_bare_lf(xml_text) {
        *xml_text = xml_text.replace("\r\n", "\n").replace('\n', "\r\n");
    }
    Ok(applied)
}

fn has_bare_lf(text: &str) -> bool {
    text.match_indices('\n')
        .any(|(index, _)| index == 0 || text.as_bytes()[index - 1] != b'\r')
}

fn has_bare_cr(text: &str) -> bool {
    text.match_indices('\r')
        .any(|(index, _)| text.as_bytes().get(index + 1) != Some(&b'\n'))
}

fn form_edit_apply_definition_lf(
    xml_text: &mut String,
    defn: &Value,
    form_path: &Path,
    form_root_start: usize,
    lang: Option<&str>,
) -> Result<FormEditApplied, String> {
    if let Some(values) = defn.get("rootProperties").and_then(Value::as_object) {
        crate::form_root_properties::set_root_properties(xml_text, values, lang)?;
        return Ok(FormEditApplied {
            added_elements: Vec::new(),
            added_attrs: Vec::new(),
            added_cmds: Vec::new(),
            added_form_events: Vec::new(),
            added_element_events: Vec::new(),
            planned_removals: Vec::new(),
            companion_count: 0,
            form_name: form_edit_form_name(form_path),
        });
    }
    if let Some(attributes) = defn.get("attributes").and_then(Value::as_array) {
        form_edit_validate_named_objects(xml_text, attributes, "Attribute", "attribute")?;
    }
    if let Some(commands) = defn.get("commands").and_then(Value::as_array) {
        form_edit_validate_named_objects(xml_text, commands, "Command", "command")?;
    }
    let planned_removals = form_edit_plan_removals(defn, xml_text)?;
    form_edit_validate_removal_definition_conflicts(defn, &planned_removals)?;
    let planned_events = form_edit_plan_events(defn, xml_text, form_metadata_owner(form_path))?;
    form_edit_apply_planned_removals(xml_text, &planned_removals);
    let form_name = form_edit_form_name(form_path);
    let mut elem_ids = FormIdAllocator {
        next: form_edit_max_ui_id(xml_text),
    };
    let mut attr_ids = FormIdAllocator {
        next: form_edit_next_id(xml_text, &["Attribute", "Column"]),
    };
    let mut cmd_ids = FormIdAllocator {
        next: form_edit_next_id(xml_text, &["Command"]),
    };
    if form_edit_is_extension_form(xml_text) {
        elem_ids.next = elem_ids.next.max(999_999);
        attr_ids.next = attr_ids.next.max(999_999);
        cmd_ids.next = cmd_ids.next.max(999_999);
    }

    let mut added_elements = Vec::<FormEditAddedElement>::new();
    let mut emitted_fragments = String::new();
    let mut companion_count = 0usize;
    if let Some(elements) = defn.get("elements").and_then(Value::as_array) {
        if !elements.is_empty() {
            form_compile_validate_element_property_tree(elements)?;
            let elements_are_idempotent = form_edit_elements_are_idempotent_noop(
                xml_text,
                elements,
                defn.get("into").and_then(Value::as_str),
                defn.get("after").and_then(Value::as_str),
                lang,
            )?;
            if !elements_are_idempotent {
                form_edit_validate_element_names(xml_text, elements)?;
                let insert_target = form_edit_target_child_items_range(
                    xml_text,
                    defn.get("into").and_then(Value::as_str),
                    defn.get("after").and_then(Value::as_str),
                )?;
                let in_command_bar = {
                    let document = Document::parse(xml_text)
                        .map_err(|err| format!("[ERROR] XML parse error: {err}"))?;
                    form_edit_placement_is_command_bar(
                        document.root_element(),
                        defn.get("into").and_then(Value::as_str),
                        defn.get("after").and_then(Value::as_str),
                    )?
                };
                // Командная панель держит кнопки: поле или группа в ней
                // записались бы в XML, который платформа не загрузит (#1447).
                if in_command_bar {
                    for element in elements {
                        let Some(object) = element.as_object() else {
                            continue;
                        };
                        let kind = FormEditElementDefinitionKind::from_object(object)?;
                        if kind != FormEditElementDefinitionKind::Button {
                            return Err(format!(
                                "[ERROR] a command bar holds only buttons: add '{}' with type Button, or add it to a group or the form",
                                kind.name(object)?
                            ));
                        }
                    }
                }
                let element_indent = insert_target.child_indent().to_string();
                let start = elem_ids.next;
                let mut lines = Vec::<String>::new();
                for element in elements {
                    let record = form_edit_element_record(element);
                    emit_form_element_with_context(
                        &mut lines,
                        element,
                        &element_indent,
                        &mut elem_ids,
                        in_command_bar,
                        lang,
                    )?;
                    if let Some(record) = record {
                        added_elements.push(record);
                    }
                }
                emitted_fragments.push_str(&lines.join("\n"));
                form_edit_insert_lines_into_target(xml_text, insert_target, &lines)?;
                companion_count = elem_ids.next.saturating_sub(start + added_elements.len());
            }
        }
    }

    let mut added_attrs = Vec::<FormEditAddedAttribute>::new();
    if let Some(attrs) = defn.get("attributes").and_then(Value::as_array) {
        if !attrs.is_empty() {
            form_edit_validate_attribute_columns(attrs)?;
            let mut lines = Vec::<String>::new();
            for attr in attrs {
                let Some(object) = attr.as_object() else {
                    continue;
                };
                let Some(name) = object.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let id = attr_ids.allocate()?;
                emit_form_edit_attribute_item(&mut lines, object, name, id, "\t\t", lang)?;
                let type_name = object
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("(no type)");
                added_attrs.push(FormEditAddedAttribute {
                    name: name.to_string(),
                    type_name: type_name.to_string(),
                    id: id.to_string(),
                });
            }
            emitted_fragments.push_str(&lines.join("\n"));
            form_edit_insert_section_items(xml_text, "Attributes", &lines)?;
        }
    }

    let mut added_cmds = Vec::<FormEditAddedCommand>::new();
    if let Some(commands) = defn.get("commands").and_then(Value::as_array) {
        if !commands.is_empty() {
            let mut lines = Vec::<String>::new();
            for cmd in commands {
                let Some(object) = cmd.as_object() else {
                    continue;
                };
                let Some(name) = object.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let id = cmd_ids.allocate()?;
                emit_form_command(&mut lines, object, name, id, "\t\t", lang)?;
                added_cmds.push(FormEditAddedCommand {
                    name: name.to_string(),
                    action: object
                        .get("action")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    id: id.to_string(),
                });
            }
            emitted_fragments.push_str(&lines.join("\n"));
            form_edit_insert_section_items(xml_text, "Commands", &lines)?;
        }
    }

    let mut added_form_events = Vec::<FormEditAddedEvent>::new();
    let mut added_element_events = Vec::<FormEditAddedEvent>::new();
    for event in &planned_events {
        form_edit_apply_planned_event(xml_text, event)?;
        match &event.owner {
            FormEditEventOwner::Form => added_form_events.push(FormEditAddedEvent {
                element: None,
                name: event.name.clone(),
                handler: event.handler.clone(),
                call_type: event.call_type.clone(),
            }),
            FormEditEventOwner::Element(element) => added_element_events.push(FormEditAddedEvent {
                element: Some(element.clone()),
                name: event.name.clone(),
                handler: event.handler.clone(),
                call_type: event.call_type.clone(),
            }),
        }
    }

    let emitted_type_qnames = form_edit_collect_emitted_type_qnames(&emitted_fragments)?;
    form_edit_ensure_emitted_namespaces(xml_text, form_root_start, &emitted_fragments)?;
    let edited_document = Document::parse(xml_text)
        .map_err(|err| format!("[ERROR] XML parse error after edit: {err}"))?;
    let edited_root = edited_document.root_element();
    require_form_root(edited_root).map_err(|error| format!("[ERROR] {error}"))?;
    form_edit_validate_surviving_removal_references(edited_root, &planned_removals)?;
    form_edit_validate_emitted_type_qnames(edited_root, &emitted_type_qnames)?;
    form_validate_requested_text_document_bindings(edited_root, defn)?;

    Ok(FormEditApplied {
        added_elements,
        added_attrs,
        added_cmds,
        added_form_events,
        added_element_events,
        planned_removals,
        companion_count,
        form_name,
    })
}

pub fn form_edit_removed_elements(
    removals: &[FormEditPlannedRemoval],
) -> Vec<FormEditRemovedElement> {
    removals
        .iter()
        .flat_map(|removal| {
            std::iter::once(FormEditRemovedElement {
                name: removal.name.clone(),
                kind: removal.kind.clone(),
                reason: FormEditRemovalReason::Requested,
            })
            .chain(removal.contained.iter().map(|node| FormEditRemovedElement {
                name: node.name.clone(),
                kind: node.kind.clone(),
                reason: FormEditRemovalReason::Contained,
            }))
        })
        .collect()
}

fn form_edit_plan_removals(
    definition: &Value,
    xml_text: &str,
) -> Result<Vec<FormEditPlannedRemoval>, String> {
    let Some(requests) = definition.get("removeElements").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    if requests.is_empty() {
        return Ok(Vec::new());
    }

    let document =
        Document::parse(xml_text).map_err(|error| format!("[ERROR] XML parse error: {error}"))?;
    let root = document.root_element();
    require_form_root(root).map_err(|error| format!("[ERROR] {error}"))?;
    let base_form = form_validation_child(root, "BaseForm");
    let mut planned = Vec::with_capacity(requests.len());

    for request in requests {
        let name = request
            .get("name")
            .and_then(Value::as_str)
            .expect("validated removeElements request must contain a string name");
        let named_nodes = root
            .descendants()
            .filter(|node| {
                node.is_element()
                    && node.tag_name().namespace() == Some(FORM_LOGFORM_NS)
                    && node.attribute("name") == Some(name)
                    && !base_form.is_some_and(|base| {
                        *node == base || node.ancestors().any(|ancestor| ancestor == base)
                    })
            })
            .collect::<Vec<_>>();
        let public_nodes = named_nodes
            .iter()
            .copied()
            .filter(|node| {
                node.parent().is_some_and(|parent| {
                    parent.is_element()
                        && parent.tag_name().namespace() == Some(FORM_LOGFORM_NS)
                        && parent.tag_name().name() == "ChildItems"
                })
            })
            .collect::<Vec<_>>();

        let target = match public_nodes.as_slice() {
            [] if named_nodes.is_empty() => {
                return Err(format!(
                    "FORM_ELEMENT_NOT_FOUND: form element `{name}` was not found"
                ));
            }
            [] => {
                return Err(format!(
                    "FORM_EDIT_REMOVE_ELEMENT_PROTECTED: form element `{name}` is not directly owned by a logform ChildItems container"
                ));
            }
            [target] => *target,
            _ => {
                return Err(format!(
                    "FORM_EDIT_REMOVE_ELEMENT_AMBIGUOUS: form element `{name}` has {} public matches",
                    public_nodes.len()
                ));
            }
        };

        let range = xml_element_line_range(xml_text, target.range());
        if let Some(existing) = planned
            .iter()
            .find(|existing: &&FormEditPlannedRemoval| ranges_overlap(&existing.range, &range))
        {
            return Err(format!(
                "FORM_EDIT_REMOVE_ELEMENT_OVERLAP: removal `{name}` overlaps removal `{}`",
                existing.name
            ));
        }
        let contained = target
            .descendants()
            .skip(1)
            .filter_map(|node| {
                if !node.is_element() || node.tag_name().namespace() != Some(FORM_LOGFORM_NS) {
                    return None;
                }
                let name = node.attribute("name")?;
                // Form elements and structural companions have stable IDs.
                // Named properties such as <Event name="OnChange"> do not.
                node.attribute("id")?;
                Some(FormEditContainedNode {
                    name: name.to_string(),
                    kind: node.tag_name().name().to_string(),
                })
            })
            .collect();
        planned.push(FormEditPlannedRemoval {
            name: name.to_string(),
            kind: target.tag_name().name().to_string(),
            contained,
            range,
        });
    }

    Ok(planned)
}

fn form_edit_removed_node_names(removals: &[FormEditPlannedRemoval]) -> HashSet<&str> {
    removals
        .iter()
        .flat_map(|removal| {
            std::iter::once(removal.name.as_str())
                .chain(removal.contained.iter().map(|node| node.name.as_str()))
        })
        .collect()
}

fn form_edit_validate_removal_definition_conflicts(
    definition: &Value,
    removals: &[FormEditPlannedRemoval],
) -> Result<(), String> {
    if removals.is_empty() {
        return Ok(());
    }
    let removed_names = form_edit_removed_node_names(removals);

    for property in ["into", "after"] {
        if let Some(target) = definition.get(property).and_then(Value::as_str) {
            if removed_names.contains(target) {
                return Err(format!(
                    "FORM_EDIT_REMOVE_DEFINITION_CONFLICT: definition property `{property}` targets removed element `{target}`"
                ));
            }
        }
    }

    if let Some(events) = definition.get("elementEvents").and_then(Value::as_array) {
        for (index, event) in events.iter().enumerate() {
            let Some(target) = event
                .as_object()
                .and_then(|object| object.get("element"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            if removed_names.contains(target) {
                return Err(format!(
                    "FORM_EDIT_REMOVE_DEFINITION_CONFLICT: definition property `elementEvents[{index}].element` targets removed element `{target}`"
                ));
            }
        }
    }

    if let Some(elements) = definition.get("elements").and_then(Value::as_array) {
        form_edit_validate_removed_new_element_names(elements, "elements", &removed_names)?;
    }
    Ok(())
}

fn form_edit_validate_removed_new_element_names(
    elements: &[Value],
    property: &str,
    removed_names: &HashSet<&str>,
) -> Result<(), String> {
    for (index, element) in elements.iter().enumerate() {
        let element_property = format!("{property}[{index}]");
        if let Some(name) = form_edit_element_display_name(element) {
            if removed_names.contains(name.as_str()) {
                return Err(format!(
                    "FORM_EDIT_REMOVE_DEFINITION_CONFLICT: definition property `{element_property}` defines removed element `{name}`"
                ));
            }
        }
        let Some(object) = element.as_object() else {
            continue;
        };
        for nested_property in ["children", "columns"] {
            if let Some(nested) = object.get(nested_property).and_then(Value::as_array) {
                form_edit_validate_removed_new_element_names(
                    nested,
                    &format!("{element_property}.{nested_property}"),
                    removed_names,
                )?;
            }
        }
    }
    Ok(())
}

fn form_edit_validate_surviving_removal_references(
    root: roxmltree::Node<'_, '_>,
    removals: &[FormEditPlannedRemoval],
) -> Result<(), String> {
    if removals.is_empty() {
        return Ok(());
    }
    let removed_names = form_edit_removed_node_names(removals);
    let base_form = form_validation_child(root, "BaseForm");

    for node in root.descendants().filter(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(FORM_LOGFORM_NS)
            && !base_form.is_some_and(|base| {
                *node == base || node.ancestors().any(|ancestor| ancestor == base)
            })
    }) {
        let property = node.tag_name().name();
        if FORM_BINDING_PATH_PROPERTIES
            .iter()
            .any(|(_, xml_property)| *xml_property == property)
        {
            let value = node.text().unwrap_or("").trim();
            if let Some(target) = form_edit_items_binding_target(value) {
                if removed_names.contains(target.as_str()) {
                    return Err(form_edit_surviving_reference_diagnostic(
                        node, property, value, &target,
                    ));
                }
            }
        } else if property == "CommandName" {
            let value = node.text().unwrap_or("").trim();
            if let Some(target) = form_edit_item_standard_command_target(value) {
                if removed_names.contains(target) {
                    return Err(form_edit_surviving_reference_diagnostic(
                        node, property, value, target,
                    ));
                }
            }
        } else if property == "Item"
            && node.parent().is_some_and(|parent| {
                parent.is_element()
                    && parent.tag_name().namespace() == Some(FORM_LOGFORM_NS)
                    && parent.tag_name().name() == "AdditionSource"
            })
        {
            let value = node.text().unwrap_or("").trim();
            if removed_names.contains(value) {
                return Err(form_edit_surviving_reference_diagnostic(
                    node,
                    "AdditionSource/Item",
                    value,
                    value,
                ));
            }
        }
    }
    Ok(())
}

fn form_edit_items_binding_target(value: &str) -> Option<String> {
    let canonical = strip_form_binding_prefixes(value.trim());
    form_items_current_data_target(&canonical)
}

fn form_edit_item_standard_command_target(value: &str) -> Option<&str> {
    let value = value.trim().strip_prefix("Form.Item.")?;
    let (target, command) = value.rsplit_once(".StandardCommand.")?;
    (!target.is_empty() && !command.is_empty()).then_some(target)
}

fn form_edit_surviving_reference_diagnostic(
    reference: roxmltree::Node<'_, '_>,
    property: &str,
    value: &str,
    target: &str,
) -> String {
    let owner = reference
        .ancestors()
        .skip(1)
        .find(|ancestor| {
            ancestor.is_element()
                && ancestor.tag_name().namespace() == Some(FORM_LOGFORM_NS)
                && ancestor.attribute("name").is_some()
        })
        .and_then(|ancestor| ancestor.attribute("name"))
        .unwrap_or("<form>");
    format!(
        "FORM_EDIT_REMOVE_SURVIVING_REFERENCE: surviving element `{owner}` property `{property}` references removed element `{target}` (value `{value}`)"
    )
}

fn ranges_overlap(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

fn form_edit_apply_planned_removals(xml_text: &mut String, removals: &[FormEditPlannedRemoval]) {
    let mut ranges = removals
        .iter()
        .map(|removal| removal.range.clone())
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| std::cmp::Reverse(range.start));
    for range in ranges {
        xml_text.replace_range(range, "");
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FormEditEventSlot {
    owner: FormEditEventOwner,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FormEditRequestedBinding {
    handler: String,
    call_type: Option<String>,
}

struct FormEditEventPlanner {
    context: FormEventContext,
    requested: HashMap<FormEditEventSlot, FormEditRequestedBinding>,
    planned: Vec<FormEditPlannedEvent>,
}

impl FormEditEventPlanner {
    fn new(context: FormEventContext) -> Self {
        Self {
            context,
            requested: HashMap::new(),
            planned: Vec::new(),
        }
    }

    fn request(
        &mut self,
        owner_node: roxmltree::Node<'_, '_>,
        target: FormEventTarget,
        event: FormEditPlannedEvent,
    ) -> Result<(), String> {
        let binding = if let Some(call_type) = event.call_type.as_deref() {
            FormEventBinding::new(&event.name, &event.handler).with_call_type(call_type)
        } else {
            FormEventBinding::new(&event.name, &event.handler)
        };
        let data_path = form_child_text(owner_node, "DataPath");
        let binding = binding.with_data_path(data_path.as_deref());
        validate_event_owner_node(owner_node, target, &binding)
            .and_then(|_| validate_event(&self.context, target, &binding))
            .map_err(|diagnostic| diagnostic.to_string())?;

        let slot = FormEditEventSlot {
            owner: event.owner.clone(),
            name: event.name.clone(),
        };
        let requested_binding = FormEditRequestedBinding {
            handler: event.handler.clone(),
            call_type: event.call_type.clone(),
        };
        if let Some(seen) = self.requested.get(&slot) {
            if seen == &requested_binding {
                return Ok(());
            }
            return Err(form_edit_event_diagnostic(
                FormEventDiagnosticCode::BindingConflict,
                form_edit_event_owner_label(&event.owner),
                &event.name,
                "the request contains conflicting handlers or callType values for the same event slot",
            ));
        }
        self.requested.insert(slot, requested_binding);

        let existing = form_validation_child(owner_node, "Events")
            .map(|events| {
                form_validation_children(events, "Event")
                    .into_iter()
                    .filter(|candidate| candidate.attribute("name") == Some(event.name.as_str()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if existing.len() > 1 {
            return Err(form_edit_event_diagnostic(
                FormEventDiagnosticCode::Duplicate,
                form_edit_event_owner_label(&event.owner),
                &event.name,
                "the source form already contains duplicate bindings for this event slot",
            ));
        }
        if let Some(existing_event) = existing.first() {
            let existing_handler = existing_event.text().unwrap_or("").trim();
            let existing_call_type = existing_event.attribute("callType");
            if existing_handler == event.handler && existing_call_type == event.call_type.as_deref()
            {
                return Ok(());
            }
            return Err(form_edit_event_diagnostic(
                FormEventDiagnosticCode::BindingConflict,
                form_edit_event_owner_label(&event.owner),
                &event.name,
                format!(
                    "existing binding is handler='{existing_handler}', callType={existing_call_type:?}"
                ),
            ));
        }

        self.planned.push(event);
        Ok(())
    }

    fn finish(self) -> Vec<FormEditPlannedEvent> {
        self.planned
    }
}

pub fn form_edit_plan_events(
    definition: &Value,
    xml_text: &str,
    metadata_owner: Option<unica_address::address::NodeKind>,
) -> Result<Vec<FormEditPlannedEvent>, String> {
    let document = Document::parse(xml_text)
        .map_err(|err| format!("[ERROR] XML parse error while planning events: {err}"))?;
    let root = document.root_element();
    let direct_main_count = form_edit_direct_main_attribute_count(root);
    form_edit_validate_projected_main_attribute_count(direct_main_count, definition)?;
    let mut context = context_from_root(root);
    context.metadata_owner = metadata_owner;
    let context = form_project_event_context(context, direct_main_count, definition);
    let mut planner = FormEditEventPlanner::new(context.clone());

    if let Some(values) = definition.get("formEvents") {
        let events = values.as_array().ok_or_else(|| {
            form_edit_event_diagnostic(
                FormEventDiagnosticCode::EventNotAllowed,
                "form",
                "<definition>",
                "formEvents must be an array",
            )
        })?;
        for value in events {
            let (name, handler, call_type) = form_edit_definition_event(value, "name", "form")?;
            planner.request(
                root,
                FormEventTarget::Form,
                FormEditPlannedEvent {
                    owner: FormEditEventOwner::Form,
                    name,
                    handler,
                    call_type,
                },
            )?;
        }
    }

    if let Some(values) = definition.get("elementEvents") {
        let events = values.as_array().ok_or_else(|| {
            form_edit_event_diagnostic(
                FormEventDiagnosticCode::EventNotAllowed,
                "element",
                "<definition>",
                "elementEvents must be an array",
            )
        })?;
        for value in events {
            let object = value.as_object().ok_or_else(|| {
                form_edit_event_diagnostic(
                    FormEventDiagnosticCode::EventNotAllowed,
                    "element",
                    "<definition>",
                    "elementEvents items must be objects",
                )
            })?;
            let element_name = object
                .get("element")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| {
                    form_edit_event_diagnostic(
                        FormEventDiagnosticCode::TargetNotFound,
                        "element",
                        "<definition>",
                        "element must be a non-empty string",
                    )
                })?;
            let event_name = object
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("<definition>");
            let element_node = form_edit_resolve_element_node(root, element_name, event_name)?;
            let kind =
                FormElementKind::from_xml_tag(element_node.tag_name().name()).ok_or_else(|| {
                    form_edit_event_diagnostic(
                        FormEventDiagnosticCode::EventNotAllowed,
                        format!("element '{element_name}'"),
                        object
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("<definition>"),
                        format!(
                            "element tag '{}' has no registered event matrix",
                            element_node.tag_name().name()
                        ),
                    )
                })?;
            let (name, handler, call_type) =
                form_edit_definition_event(value, "name", &format!("element '{element_name}'"))?;
            planner.request(
                element_node,
                FormEventTarget::Element(kind),
                FormEditPlannedEvent {
                    owner: FormEditEventOwner::Element(element_name.to_string()),
                    name,
                    handler,
                    call_type,
                },
            )?;
        }
    }

    if let Some(elements) = definition.get("elements").and_then(Value::as_array) {
        form_edit_validate_new_element_event_tree(elements, &context)?;
    }

    Ok(planner.finish())
}

fn form_project_event_context(
    mut context: FormEventContext,
    direct_main_count: usize,
    definition: &Value,
) -> FormEventContext {
    if direct_main_count != 0 {
        return context;
    }

    let projected_main_attributes = definition
        .get("attributes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter(|attribute| attribute.get("main").and_then(Value::as_bool) == Some(true))
        .filter(|attribute| {
            attribute
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| !name.is_empty())
        })
        .collect::<Vec<_>>();

    if let [attribute] = projected_main_attributes.as_slice() {
        let type_name = attribute
            .get("type")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        context.main_attribute = type_name
            .map(MainAttributeKind::from_type_name)
            .unwrap_or(MainAttributeKind::Unknown);
        context.main_attribute_type = type_name.map(ToOwned::to_owned);
        context.main_attribute_provenance = MainAttributeProvenance::DirectForm;
    }
    context
}

fn form_edit_direct_main_attribute_count(root: roxmltree::Node<'_, '_>) -> usize {
    form_validation_child(root, "Attributes")
        .map(|attributes| {
            form_validation_children(attributes, "Attribute")
                .into_iter()
                .filter(|attribute| {
                    form_validation_child_text(*attribute, "MainAttribute").as_deref()
                        == Some("true")
                })
                .count()
        })
        .unwrap_or(0)
}

fn form_edit_validate_projected_main_attribute_count(
    direct_main_count: usize,
    definition: &Value,
) -> Result<(), String> {
    let added_main_count = definition
        .get("attributes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter(|attribute| attribute.get("main").and_then(Value::as_bool) == Some(true))
        .filter(|attribute| {
            attribute
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| !name.is_empty())
        })
        .count();
    let resulting_count = direct_main_count + added_main_count;
    if resulting_count > 1 {
        return Err(format!(
            "[ERROR] Resulting form would contain {resulting_count} direct MainAttribute=true entries; expected at most one"
        ));
    }
    Ok(())
}

pub fn form_edit_definition_event(
    value: &Value,
    name_key: &str,
    target: &str,
) -> Result<(String, String, Option<String>), String> {
    let object = value.as_object().ok_or_else(|| {
        form_edit_event_diagnostic(
            FormEventDiagnosticCode::EventNotAllowed,
            target,
            "<definition>",
            "event definition must be an object",
        )
    })?;
    let name = object
        .get(name_key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            form_edit_event_diagnostic(
                FormEventDiagnosticCode::EventNotAllowed,
                target,
                "<definition>",
                format!("{name_key} must be a non-empty string"),
            )
        })?
        .to_string();
    let handler = object
        .get("handler")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            form_edit_event_diagnostic(
                FormEventDiagnosticCode::EmptyHandler,
                target,
                &name,
                "handler must be a string",
            )
        })?
        .trim()
        .to_string();
    let call_type = match object.get("callType") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.clone()),
        Some(_) => {
            return Err(form_edit_event_diagnostic(
                FormEventDiagnosticCode::InvalidCallType,
                target,
                &name,
                "callType must be a string",
            ));
        }
    };
    Ok((name, handler, call_type))
}

fn form_edit_optional_event_handler(
    value: Option<&Value>,
    target: &str,
    event: &str,
) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(form_edit_event_diagnostic(
            FormEventDiagnosticCode::EmptyHandler,
            target,
            event,
            "handler must be a string or null",
        )),
    }
}

fn form_edit_validate_element_event_payload_types(
    object: &Map<String, Value>,
    kind: FormEditElementDefinitionKind,
    element_name: &str,
) -> Result<(), String> {
    let handlers = match object.get("handlers") {
        None | Some(Value::Null) => None,
        Some(Value::Object(values)) => Some(values),
        Some(_) => {
            return Err(form_edit_event_diagnostic(
                FormEventDiagnosticCode::EventNotAllowed,
                format!("new element '{element_name}'"),
                "<definition>",
                "handlers must be an object",
            ));
        }
    };
    let Some(events_value) = object.get("on") else {
        return Ok(());
    };
    let events = events_value.as_array().ok_or_else(|| {
        form_edit_event_diagnostic(
            FormEventDiagnosticCode::EventNotAllowed,
            format!("new element '{element_name}'"),
            "<definition>",
            "on must be an array",
        )
    })?;
    let target = format!("new element '{element_name}'");
    if kind == FormEditElementDefinitionKind::Table
        && !events.is_empty()
        && object
            .get("path")
            .and_then(Value::as_str)
            .is_none_or(|path| path.trim().is_empty())
    {
        return Err(form_edit_event_diagnostic(
            FormEventDiagnosticCode::EventNotAllowed,
            &target,
            "<definition>",
            "Table event bindings require a non-empty path/DataPath; the platform drops bindings on unbound tables",
        ));
    }
    for value in events {
        if let Some(event_name) = value.as_str() {
            form_edit_optional_event_handler(
                handlers.and_then(|values| values.get(event_name)),
                &target,
                event_name,
            )?;
            continue;
        }
        let event = value.as_object().ok_or_else(|| {
            form_edit_event_diagnostic(
                FormEventDiagnosticCode::EventNotAllowed,
                &target,
                "<definition>",
                "on items must be strings or objects",
            )
        })?;
        let event_name = event
            .get("event")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                form_edit_event_diagnostic(
                    FormEventDiagnosticCode::EventNotAllowed,
                    &target,
                    "<definition>",
                    "event must be a non-empty string",
                )
            })?;
        if form_edit_optional_event_handler(event.get("handler"), &target, event_name)?.is_none() {
            form_edit_optional_event_handler(
                handlers.and_then(|values| values.get(event_name)),
                &target,
                event_name,
            )?;
        }
    }
    Ok(())
}

pub fn form_edit_validate_new_element_event_tree(
    elements: &[Value],
    context: &FormEventContext,
) -> Result<(), String> {
    for element in elements {
        let Some(object) = element.as_object() else {
            continue;
        };
        let definition_kind = FormEditElementDefinitionKind::from_object(object)?;
        let element_name = definition_kind.name(object)?;
        form_edit_validate_element_event_payload_types(object, definition_kind, element_name)?;
        let kind = definition_kind.event_kind();
        let handlers = match object.get("handlers") {
            None | Some(Value::Null) => None,
            Some(Value::Object(values)) => Some(values),
            Some(_) => {
                return Err(form_edit_event_diagnostic(
                    FormEventDiagnosticCode::EventNotAllowed,
                    "new element",
                    "<definition>",
                    "handlers must be an object",
                ));
            }
        };
        if let Some(events_value) = object.get("on") {
            let events = events_value.as_array().ok_or_else(|| {
                form_edit_event_diagnostic(
                    FormEventDiagnosticCode::EventNotAllowed,
                    "new element",
                    "<definition>",
                    "on must be an array",
                )
            })?;
            let element_name = element_name.to_string();
            let mut seen = HashSet::<String>::new();
            for value in events {
                let (name, handler, call_type) = if let Some(name) = value.as_str() {
                    let target = format!("new element '{element_name}'");
                    let handler = form_edit_optional_event_handler(
                        handlers.and_then(|values| values.get(name)),
                        &target,
                        name,
                    )?
                    .unwrap_or_else(|| form_event_handler_name(&element_name, name));
                    (name.to_string(), handler, None)
                } else {
                    let object = value.as_object().ok_or_else(|| {
                        form_edit_event_diagnostic(
                            FormEventDiagnosticCode::EventNotAllowed,
                            format!("new element '{element_name}'"),
                            "<definition>",
                            "on items must be strings or objects",
                        )
                    })?;
                    let name = object
                        .get("event")
                        .and_then(Value::as_str)
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| {
                            form_edit_event_diagnostic(
                                FormEventDiagnosticCode::EventNotAllowed,
                                format!("new element '{element_name}'"),
                                "<definition>",
                                "event must be a non-empty string",
                            )
                        })?
                        .to_string();
                    let target = format!("new element '{element_name}'");
                    let handler = match form_edit_optional_event_handler(
                        object.get("handler"),
                        &target,
                        &name,
                    )? {
                        Some(handler) => Some(handler),
                        None => form_edit_optional_event_handler(
                            handlers.and_then(|values| values.get(&name)),
                            &target,
                            &name,
                        )?,
                    }
                    .unwrap_or_else(|| form_event_handler_name(&element_name, &name));
                    let call_type = match object.get("callType") {
                        None | Some(Value::Null) => None,
                        Some(Value::String(value)) => Some(value.clone()),
                        Some(_) => {
                            return Err(form_edit_event_diagnostic(
                                FormEventDiagnosticCode::InvalidCallType,
                                format!("new element '{element_name}'"),
                                &name,
                                "callType must be a string",
                            ));
                        }
                    };
                    (name, handler, call_type)
                };
                if !seen.insert(name.clone()) {
                    return Err(form_edit_event_diagnostic(
                        FormEventDiagnosticCode::Duplicate,
                        format!("new element '{element_name}'"),
                        &name,
                        "the on array contains the same event more than once",
                    ));
                }
                let binding = if let Some(call_type) = call_type.as_deref() {
                    FormEventBinding::new(&name, &handler).with_call_type(call_type)
                } else {
                    FormEventBinding::new(&name, &handler)
                };
                // Путь данных таблицы выбирает события источника динамического
                // списка, как у существующего элемента.
                let binding = binding.with_data_path(object.get("path").and_then(Value::as_str));
                validate_event(context, FormEventTarget::Element(kind), &binding)
                    .map_err(|diagnostic| diagnostic.to_string())?;
            }
        }
        for child_key in ["children", "columns"] {
            if let Some(children) = object.get(child_key).and_then(Value::as_array) {
                form_edit_validate_new_element_event_tree(children, context)?;
            }
        }
    }
    Ok(())
}

pub fn form_edit_event_owner_label(owner: &FormEditEventOwner) -> String {
    match owner {
        FormEditEventOwner::Form => "form".to_string(),
        FormEditEventOwner::Element(name) => format!("element '{name}'"),
    }
}

pub fn form_edit_event_diagnostic(
    code: FormEventDiagnosticCode,
    target: impl Into<String>,
    event: impl Into<String>,
    detail: impl Into<String>,
) -> String {
    FormEventDiagnostic::new(code, target, event)
        .with_detail(detail)
        .to_string()
}

pub fn form_edit_find_element_nodes<'a, 'input>(
    root: roxmltree::Node<'a, 'input>,
    name: &str,
) -> Vec<roxmltree::Node<'a, 'input>> {
    let auto_command_bar = root.children().find(|child| {
        child.is_element()
            && child.tag_name().namespace() == Some(FORM_LOGFORM_NS)
            && child.tag_name().name() == "AutoCommandBar"
    });
    let mut matches = auto_command_bar
        .filter(|node| node.attribute("name") == Some(name))
        .into_iter()
        .collect::<Vec<_>>();

    let mut containers = root
        .children()
        .filter(|child| {
            child.is_element()
                && child.tag_name().namespace() == Some(FORM_LOGFORM_NS)
                && child.tag_name().name() == "ChildItems"
        })
        .collect::<Vec<_>>();
    if let Some(child_items) = auto_command_bar.and_then(|node| {
        node.children().find(|child| {
            child.is_element()
                && child.tag_name().namespace() == Some(FORM_LOGFORM_NS)
                && child.tag_name().name() == "ChildItems"
        })
    }) {
        containers.push(child_items);
    }
    for container in containers {
        matches.extend(container.descendants().filter(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(FORM_LOGFORM_NS)
                && node.attribute("name") == Some(name)
                && FormElementKind::from_xml_tag(node.tag_name().name()).is_some()
        }));
    }
    matches
}

pub fn form_edit_resolve_element_node<'a, 'input>(
    root: roxmltree::Node<'a, 'input>,
    name: &str,
    event_name: &str,
) -> Result<roxmltree::Node<'a, 'input>, String> {
    let matches = form_edit_find_element_nodes(root, name);
    match matches.as_slice() {
        [] => Err(form_edit_event_diagnostic(
            FormEventDiagnosticCode::TargetNotFound,
            format!("element '{name}'"),
            event_name,
            "target element was not found in Form ChildItems or AutoCommandBar",
        )),
        [node] => Ok(*node),
        _ => Err(form_edit_event_diagnostic(
            FormEventDiagnosticCode::Duplicate,
            format!("element '{name}'"),
            event_name,
            format!(
                "target name is ambiguous: the source form contains {} matching elements",
                matches.len()
            ),
        )),
    }
}

pub fn form_edit_apply_planned_event(
    xml_text: &mut String,
    event: &FormEditPlannedEvent,
) -> Result<(), String> {
    enum InsertTarget {
        ExistingEvents(Range<usize>),
        AfterRootAutoCommandBar {
            pos: usize,
            indent: String,
        },
        IntoElement {
            range: Range<usize>,
            tag: String,
            indent: String,
        },
        BeforeChildItems {
            pos: usize,
            indent: String,
        },
    }

    let target = {
        let document = Document::parse(xml_text)
            .map_err(|err| format!("[ERROR] XML parse error while applying event: {err}"))?;
        let root = document.root_element();
        let owner_node = match &event.owner {
            FormEditEventOwner::Form => root,
            FormEditEventOwner::Element(name) => {
                form_edit_resolve_element_node(root, name, &event.name)?
            }
        };
        if let Some(events) = form_validation_child(owner_node, "Events") {
            InsertTarget::ExistingEvents(events.range())
        } else {
            match &event.owner {
                FormEditEventOwner::Form => {
                    let auto_command_bar = form_validation_child(root, "AutoCommandBar")
                        .ok_or_else(|| {
                            form_edit_event_diagnostic(
                                FormEventDiagnosticCode::TargetNotFound,
                                "form",
                                &event.name,
                                "AutoCommandBar is required to position the Events section",
                            )
                        })?;
                    InsertTarget::AfterRootAutoCommandBar {
                        pos: auto_command_bar.range().end,
                        indent: form_edit_whitespace_indent_at(
                            xml_text,
                            auto_command_bar.range().start,
                        ),
                    }
                }
                FormEditEventOwner::Element(_) => {
                    match form_validation_child(owner_node, "ChildItems")
                        .filter(|_| matches!(owner_node.tag_name().name(), "Pages" | "Table"))
                    {
                        Some(child_items) => {
                            let indent =
                                form_edit_whitespace_indent_at(xml_text, child_items.range().start);
                            InsertTarget::BeforeChildItems {
                                pos: child_items.range().start.saturating_sub(indent.len()),
                                indent,
                            }
                        }
                        None => InsertTarget::IntoElement {
                            range: owner_node.range(),
                            tag: owner_node.tag_name().name().to_string(),
                            indent: form_edit_whitespace_indent_at(
                                xml_text,
                                owner_node.range().start,
                            ),
                        },
                    }
                }
            }
        }
    };

    let call_type = event
        .call_type
        .as_deref()
        .map(|value| {
            format!(
                " callType=\"{}\"",
                unica_xml_format::text::escape_xml(value)
            )
        })
        .unwrap_or_default();
    let event_xml = format!(
        "<Event name=\"{}\"{}>{}</Event>",
        unica_xml_format::text::escape_xml(&event.name),
        call_type,
        unica_xml_format::text::escape_xml(&event.handler)
    );
    let eol = form_edit_eol(xml_text);
    match target {
        InsertTarget::ExistingEvents(range) => {
            form_edit_insert_event_into_events(xml_text, range, &event_xml, eol)
        }
        InsertTarget::AfterRootAutoCommandBar { pos, indent } => {
            let event_indent = format!("{indent}\t");
            let block =
                format!("{indent}<Events>{eol}{event_indent}{event_xml}{eol}{indent}</Events>");
            let suffix_has_eol = xml_text[pos..].starts_with(eol);
            let after = if suffix_has_eol {
                String::new()
            } else {
                format!("{eol}{indent}")
            };
            xml_text.insert_str(pos, &format!("{eol}{block}{after}"));
            Ok(())
        }
        InsertTarget::IntoElement { range, tag, indent } => {
            form_edit_insert_events_into_element(xml_text, range, &tag, &indent, &event_xml, eol)
        }
        InsertTarget::BeforeChildItems { pos, indent } => {
            let event_indent = format!("{indent}\t");
            let block = format!(
                "{indent}<Events>{eol}{event_indent}{event_xml}{eol}{indent}</Events>{eol}"
            );
            xml_text.insert_str(pos, &block);
            Ok(())
        }
    }
}

pub fn form_edit_insert_event_into_events(
    xml_text: &mut String,
    range: Range<usize>,
    event_xml: &str,
    eol: &str,
) -> Result<(), String> {
    let section = &xml_text[range.clone()];
    let events_indent = form_edit_whitespace_indent_at(xml_text, range.start);
    let event_indent = format!("{events_indent}\t");
    if section.trim_end().ends_with("/>") {
        let relative_close = section
            .rfind("/>")
            .ok_or_else(|| "Self-closing Events section has no '/>' terminator".to_string())?;
        let opening = section[..relative_close].trim_end();
        let replacement =
            format!("{opening}>{eol}{event_indent}{event_xml}{eol}{events_indent}</Events>");
        xml_text.replace_range(range, &replacement);
        return Ok(());
    }

    let relative_close = section
        .rfind("</Events>")
        .ok_or_else(|| "No closing </Events> found in form event section".to_string())?;
    let close_pos = range.start + relative_close;
    let line_start = xml_text[..close_pos]
        .rfind('\n')
        .map(|position| position + 1)
        .unwrap_or(close_pos);
    if xml_text[line_start..close_pos].trim().is_empty() {
        xml_text.insert_str(line_start, &format!("{event_indent}{event_xml}{eol}"));
    } else {
        xml_text.insert_str(
            close_pos,
            &format!("{eol}{event_indent}{event_xml}{eol}{events_indent}"),
        );
    }
    Ok(())
}

pub fn form_edit_insert_events_into_element(
    xml_text: &mut String,
    range: Range<usize>,
    tag: &str,
    element_indent: &str,
    event_xml: &str,
    eol: &str,
) -> Result<(), String> {
    let element = &xml_text[range.clone()];
    let section_indent = format!("{element_indent}\t");
    let event_indent = format!("{section_indent}\t");
    let block = format!(
        "{section_indent}<Events>{eol}{event_indent}{event_xml}{eol}{section_indent}</Events>"
    );
    if element.trim_end().ends_with("/>") {
        let relative_close = element
            .rfind("/>")
            .ok_or_else(|| "Self-closing form element has no '/>' terminator".to_string())?;
        let opening = element[..relative_close].trim_end();
        let replacement = format!("{opening}>{eol}{block}{eol}{element_indent}</{tag}>");
        xml_text.replace_range(range, &replacement);
        return Ok(());
    }

    let close = format!("</{tag}>");
    let relative_close = element
        .rfind(&close)
        .ok_or_else(|| format!("No closing {close} found in element target"))?;
    let close_pos = range.start + relative_close;
    let line_start = xml_text[..close_pos]
        .rfind('\n')
        .map(|position| position + 1)
        .unwrap_or(close_pos);
    if xml_text[line_start..close_pos].trim().is_empty() {
        xml_text.insert_str(line_start, &format!("{block}{eol}"));
    } else {
        xml_text.insert_str(close_pos, &format!("{eol}{block}{eol}{element_indent}"));
    }
    Ok(())
}

pub fn form_edit_eol(xml_text: &str) -> &'static str {
    if xml_text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

pub fn form_edit_whitespace_indent_at(xml_text: &str, pos: usize) -> String {
    let line_start = xml_text[..pos]
        .rfind('\n')
        .map(|position| position + 1)
        .unwrap_or(0);
    xml_text[line_start..pos]
        .chars()
        .take_while(|character| matches!(character, ' ' | '\t' | '\r'))
        .filter(|character| *character != '\r')
        .collect()
}

pub fn form_edit_form_name(path: &Path) -> String {
    if path.file_name().and_then(|value| value.to_str()) == Some("Form.xml")
        && path
            .parent()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            == Some("Ext")
    {
        if let Some(name) = path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
        {
            return name.to_string();
        }
    }
    path.file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Form")
        .to_string()
}

fn form_edit_max_ui_id(xml_text: &str) -> usize {
    let Ok(doc) = Document::parse(xml_text) else {
        return 0;
    };
    // Elements and their companions share IDs, unlike attributes and commands.
    // Walk the UI subtrees so imported element kinds need no allocator whitelist.
    doc.descendants()
        .filter(|node| {
            node.ancestors().any(|ancestor| {
                ancestor.has_tag_name("ChildItems") || ancestor.has_tag_name("AutoCommandBar")
            })
        })
        .filter_map(|node| node.attribute("id"))
        .filter_map(|value| value.parse::<usize>().ok())
        .max()
        .unwrap_or(0)
}

pub fn form_edit_next_id(xml_text: &str, tags: &[&str]) -> usize {
    let Ok(doc) = Document::parse(xml_text) else {
        return 0;
    };
    doc.descendants()
        .filter(|node| node.is_element() && tags.contains(&node.tag_name().name()))
        .filter_map(|node| node.attribute("id"))
        .filter(|value| *value != "-1")
        .filter_map(|value| value.parse::<usize>().ok())
        .max()
        .unwrap_or(0)
}

pub fn form_edit_is_extension_form(xml_text: &str) -> bool {
    Document::parse(xml_text).ok().is_some_and(|doc| {
        doc.descendants()
            .any(|node| node.is_element() && node.tag_name().name() == "BaseForm")
    })
}

pub fn form_edit_validate_element_names(xml_text: &str, elements: &[Value]) -> Result<(), String> {
    let mut names = HashSet::new();
    for element in elements {
        form_edit_validate_element_name_tree(xml_text, element, &mut names)?;
    }
    Ok(())
}

pub fn form_edit_validate_element_name_tree(
    xml_text: &str,
    element: &Value,
    names: &mut HashSet<String>,
) -> Result<(), String> {
    if let Some(name) = form_edit_element_display_name(element) {
        if !names.insert(name.clone()) {
            return Err(format!(
                "[ERROR] Element '{name}' already exists in edit definition -- element names must be unique"
            ));
        }
        if form_edit_element_name_exists(xml_text, &name) {
            return Err(format!(
                "[ERROR] Element '{name}' already exists in form -- element names must be unique"
            ));
        }
    }
    let Some(object) = element.as_object() else {
        return Ok(());
    };
    for key in ["children", "columns"] {
        if let Some(children) = object.get(key).and_then(Value::as_array) {
            for child in children {
                form_edit_validate_element_name_tree(xml_text, child, names)?;
            }
        }
    }
    Ok(())
}

pub fn form_edit_validate_named_objects(
    xml_text: &str,
    values: &[Value],
    tag: &str,
    label: &str,
) -> Result<(), String> {
    let mut names = HashSet::new();
    for value in values {
        let Some(name) = value
            .as_object()
            .and_then(|object| object.get("name"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        if name.is_empty() {
            return Err(format!(
                "[ERROR] Empty {label} name in edit definition -- names must be non-empty"
            ));
        }
        if !names.insert(name.to_string()) {
            return Err(format!(
                "[ERROR] Duplicate {label} name '{name}' in edit definition -- names must be unique"
            ));
        }
        if form_edit_name_exists(xml_text, tag, name) {
            return Err(format!(
                "[ERROR] {tag} '{name}' already exists in form -- {label} names must be unique"
            ));
        }
    }
    Ok(())
}

pub fn form_edit_validate_attribute_columns(attrs: &[Value]) -> Result<(), String> {
    for attr in attrs {
        let Some(object) = attr.as_object() else {
            continue;
        };
        let Some(attr_name) = object.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(columns_value) = object.get("columns") else {
            continue;
        };
        let columns = columns_value
            .as_array()
            .ok_or_else(|| format!("[ERROR] Attribute '{attr_name}' columns must be an array"))?;
        let attr_type = object
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let normalized_attr_type = normalize_form_type(attr_type);
        if !matches!(normalized_attr_type.as_str(), "ValueTable" | "ValueTree") {
            return Err(format!(
                "[ERROR] Attribute '{attr_name}' of type '{attr_type}' cannot define columns; columns are supported only for ValueTable or ValueTree"
            ));
        }
        let mut names = HashSet::new();
        for (index, column) in columns.iter().enumerate() {
            let column = column.as_object().ok_or_else(|| {
                format!(
                    "[ERROR] Attribute '{attr_name}' column #{} must be an object",
                    index + 1
                )
            })?;
            let name = column
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| {
                    format!(
                        "[ERROR] Attribute '{attr_name}' column #{} requires non-empty name",
                        index + 1
                    )
                })?;
            column
                .get("type")
                .and_then(Value::as_str)
                .filter(|column_type| !column_type.trim().is_empty())
                .ok_or_else(|| {
                    format!(
                        "[ERROR] Attribute '{attr_name}' column '{name}' requires non-empty type"
                    )
                })?;
            if !names.insert(name.to_string()) {
                return Err(format!(
                    "[ERROR] Duplicate column name '{name}' in attribute '{attr_name}' edit definition -- column names must be unique"
                ));
            }
        }
    }
    Ok(())
}

pub fn form_edit_name_exists(xml_text: &str, tag: &str, name: &str) -> bool {
    let Ok(doc) = Document::parse(xml_text) else {
        return false;
    };
    doc.descendants().any(|node| {
        node.is_element() && node.tag_name().name() == tag && node.attribute("name") == Some(name)
    })
}

pub fn form_edit_element_name_exists(xml_text: &str, name: &str) -> bool {
    let Ok(doc) = Document::parse(xml_text) else {
        return false;
    };
    doc.descendants().any(|node| {
        node.is_element()
            && FormElementKind::from_xml_tag(node.tag_name().name()).is_some()
            && node.attribute("name") == Some(name)
    })
}

#[derive(Debug, PartialEq, Eq)]
struct FormEditElementFingerprint {
    namespace: Option<String>,
    tag: String,
    attributes: Vec<(Option<String>, String, String)>,
    content: Vec<FormEditElementFingerprintContent>,
}

#[derive(Debug, PartialEq, Eq)]
enum FormEditElementFingerprintContent {
    Element(FormEditElementFingerprint),
    Text(String),
}

fn form_edit_is_platform_omitted_default(node: roxmltree::Node<'_, '_>) -> bool {
    node.has_tag_name((FORM_LOGFORM_NS, "RowFilter"))
        && node.attribute(("http://www.w3.org/2001/XMLSchema-instance", "nil")) == Some("true")
        && node.attributes().len() == 1
        && node
            .children()
            .all(|child| child.is_text() && child.text().is_none_or(|text| text.trim().is_empty()))
}

fn form_edit_element_fingerprint(node: roxmltree::Node<'_, '_>) -> FormEditElementFingerprint {
    let mut attributes = node
        .attributes()
        .filter(|attribute| !(attribute.namespace().is_none() && attribute.name() == "id"))
        .map(|attribute| {
            (
                attribute.namespace().map(ToOwned::to_owned),
                attribute.name().to_string(),
                attribute.value().to_string(),
            )
        })
        .collect::<Vec<_>>();
    attributes.sort();
    let content = node
        .children()
        .filter_map(|child| {
            if child.is_element() {
                (!form_edit_is_platform_omitted_default(child)).then(|| {
                    FormEditElementFingerprintContent::Element(form_edit_element_fingerprint(child))
                })
            } else if child.is_text() {
                child
                    .text()
                    .filter(|text| !text.trim().is_empty())
                    .map(|text| FormEditElementFingerprintContent::Text(text.to_string()))
            } else {
                None
            }
        })
        .collect();
    FormEditElementFingerprint {
        namespace: node.tag_name().namespace().map(ToOwned::to_owned),
        tag: node.tag_name().name().to_string(),
        attributes,
        content,
    }
}

fn form_edit_expected_element_fingerprint(
    xml_text: &str,
    element: &Value,
    in_command_bar: bool,
    lang: Option<&str>,
) -> Result<FormEditElementFingerprint, String> {
    let mut lines = Vec::new();
    let mut ids = FormIdAllocator::new();
    emit_form_element_with_context(&mut lines, element, "\t", &mut ids, in_command_bar, lang)?;
    let fragment = lines.join("\n");

    let source_document =
        Document::parse(xml_text).map_err(|error| format!("[ERROR] XML parse error: {error}"))?;
    let root = source_document.root_element();
    let root_start = root.range().start;
    let open_end = xml_text[root_start..]
        .find('>')
        .map(|offset| root_start + offset + 1)
        .ok_or_else(|| "[ERROR] Form root start tag is incomplete".to_string())?;
    let wrapper = format!("{}\n{fragment}\n</Form>", &xml_text[root_start..open_end]);
    let fragment_document = Document::parse(&wrapper)
        .map_err(|error| format!("[ERROR] emitted form element XML is invalid: {error}"))?;
    let emitted = fragment_document
        .root_element()
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| "[ERROR] form element emitter produced no element".to_string())?;
    Ok(form_edit_element_fingerprint(emitted))
}

fn form_edit_elements_match_target(
    root: roxmltree::Node<'_, '_>,
    existing: &[roxmltree::Node<'_, '_>],
    requested_names: &[String],
    into_name: Option<&str>,
    after_name: Option<&str>,
) -> bool {
    let root_child_items = form_child(root, "ChildItems");
    let (target_child_items, required_predecessor) =
        if let Some(into_name) = into_name.filter(|name| !name.is_empty()) {
            let Some(target) = form_edit_find_placement_element(root, into_name)
                .ok()
                .flatten()
            else {
                return false;
            };
            let Some(child_items) = form_child(target, "ChildItems") else {
                return false;
            };
            (child_items, None)
        } else if let Some(after_name) = after_name.filter(|name| !name.is_empty()) {
            let Some(after) = form_edit_find_placement_element(root, after_name)
                .ok()
                .flatten()
            else {
                return false;
            };
            let Some(child_items) = after
                .parent()
                .filter(|parent| parent.has_tag_name((FORM_LOGFORM_NS, "ChildItems")))
            else {
                return false;
            };
            (child_items, Some(after_name))
        } else {
            let Some(root_child_items) = root_child_items else {
                return false;
            };
            (root_child_items, None)
        };
    if existing
        .iter()
        .any(|node| node.parent() != Some(target_child_items))
    {
        return false;
    }

    let child_names = target_child_items
        .children()
        .filter(|node| node.is_element())
        .filter_map(|node| node.attribute("name"))
        .collect::<Vec<_>>();
    let Some(start) = required_predecessor.map_or_else(
        || {
            child_names
                .windows(requested_names.len())
                .position(|window| {
                    window
                        .iter()
                        .zip(requested_names)
                        .all(|(actual, expected)| *actual == expected)
                })
        },
        |predecessor| {
            child_names
                .iter()
                .position(|name| *name == predecessor)
                .map(|index| index + 1)
        },
    ) else {
        return false;
    };
    child_names
        .get(start..start + requested_names.len())
        .is_some_and(|window| {
            window
                .iter()
                .zip(requested_names)
                .all(|(actual, expected)| *actual == expected)
        })
}

fn form_edit_elements_are_idempotent_noop(
    xml_text: &str,
    elements: &[Value],
    into_name: Option<&str>,
    after_name: Option<&str>,
    lang: Option<&str>,
) -> Result<bool, String> {
    let document =
        Document::parse(xml_text).map_err(|error| format!("[ERROR] XML parse error: {error}"))?;
    let root = document.root_element();
    let in_command_bar = form_edit_placement_is_command_bar(root, into_name, after_name)?;
    let mut requested_names = Vec::with_capacity(elements.len());
    let mut existing = Vec::with_capacity(elements.len());
    for element in elements {
        let Some(object) = element.as_object() else {
            return Ok(false);
        };
        let kind = FormEditElementDefinitionKind::from_object(object)?;
        if kind == FormEditElementDefinitionKind::AutoCommandBar {
            return Ok(false);
        }
        let name = kind.name(object)?.to_string();
        let matches = form_edit_find_element_nodes(root, &name);
        let [node] = matches.as_slice() else {
            return Ok(false);
        };
        let expected =
            form_edit_expected_element_fingerprint(xml_text, element, in_command_bar, lang)?;
        if form_edit_element_fingerprint(*node) != expected {
            return Ok(false);
        }
        requested_names.push(name);
        existing.push(*node);
    }
    Ok(form_edit_elements_match_target(
        root,
        &existing,
        &requested_names,
        into_name,
        after_name,
    ))
}

pub fn form_edit_element_display_name(element: &Value) -> Option<String> {
    let object = element.as_object()?;
    let kind = FormEditElementDefinitionKind::from_object(object).ok()?;
    kind.name(object).ok().map(ToOwned::to_owned)
}

/// Typed counterpart of [`form_edit_element_summary`] (ADR-0023): the same
/// element identity, as fields rather than a rendered line.
fn form_edit_element_record(element: &Value) -> Option<FormEditAddedElement> {
    let object = element.as_object()?;
    let kind = FormEditElementDefinitionKind::from_object(object).ok()?;
    if kind == FormEditElementDefinitionKind::AutoCommandBar {
        return None;
    }
    let is_table = kind == FormEditElementDefinitionKind::Table;
    Some(FormEditAddedElement {
        kind: format!("{kind:?}"),
        name: form_edit_element_display_name(element)?,
        path: object
            .get("path")
            .and_then(Value::as_str)
            .map(str::to_string),
        representation: is_table
            .then(|| {
                object
                    .get("representation")
                    .and_then(Value::as_str)
                    .and_then(form_compile_table_representation)
                    .map(str::to_string)
            })
            .flatten(),
        auto_insert_new_row: is_table
            .then(|| object.get("autoInsertNewRow").and_then(Value::as_bool))
            .flatten(),
    })
}

pub fn form_edit_element_summary(element: &Value) -> Option<String> {
    let object = element.as_object()?;
    let kind = FormEditElementDefinitionKind::from_object(object).ok()?;
    let tag = match kind {
        FormEditElementDefinitionKind::Table => "Table",
        FormEditElementDefinitionKind::Label => "Label",
        FormEditElementDefinitionKind::LabelField => "LabelField",
        FormEditElementDefinitionKind::Button => "Button",
        FormEditElementDefinitionKind::CommandBar => "CommandBar",
        FormEditElementDefinitionKind::Pages => "Pages",
        FormEditElementDefinitionKind::Page => "Page",
        FormEditElementDefinitionKind::Group => "Group",
        FormEditElementDefinitionKind::CheckBox => "CheckBox",
        FormEditElementDefinitionKind::InputField => "Input",
        FormEditElementDefinitionKind::HtmlDocumentField => "HTMLDocumentField",
        FormEditElementDefinitionKind::TextDocumentField => "TextDocumentField",
        FormEditElementDefinitionKind::AutoCommandBar => return None,
    };
    let name = form_edit_element_display_name(element)?;
    let path = object
        .get("path")
        .and_then(Value::as_str)
        .map(|value| format!(" -> {value}"))
        .unwrap_or_default();
    let table_properties = if kind == FormEditElementDefinitionKind::Table {
        let mut properties = Vec::new();
        if let Some(representation) = object
            .get("representation")
            .and_then(Value::as_str)
            .and_then(form_compile_table_representation)
        {
            properties.push(format!("representation={representation}"));
        }
        if let Some(auto_insert_new_row) = object.get("autoInsertNewRow").and_then(Value::as_bool) {
            properties.push(format!("autoInsertNewRow={auto_insert_new_row}"));
        }
        if properties.is_empty() {
            String::new()
        } else {
            format!(" [{}]", properties.join(", "))
        }
    } else {
        String::new()
    };
    let events = form_edit_element_events_summary(object);
    Some(format!(
        "  + [{tag}] {name}{path}{table_properties}{events}"
    ))
}

pub fn form_edit_element_events_summary(element: &Map<String, Value>) -> String {
    let Some(events) = element.get("on").and_then(Value::as_array) else {
        return String::new();
    };
    if events.is_empty() {
        return String::new();
    }
    let names = events
        .iter()
        .map(|event| {
            event
                .as_str()
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| form_edit_python_repr(event))
        })
        .collect::<Vec<_>>();
    format!(" {{{}}}", names.join(", "))
}

pub fn form_edit_python_repr(value: &Value) -> String {
    match value {
        Value::String(value) => format!("'{}'", form_edit_python_repr_string(value)),
        Value::Bool(value) => {
            if *value {
                "True".to_string()
            } else {
                "False".to_string()
            }
        }
        Value::Number(value) => value.to_string(),
        Value::Null => "None".to_string(),
        Value::Array(values) => {
            let items = values.iter().map(form_edit_python_repr).collect::<Vec<_>>();
            format!("[{}]", items.join(", "))
        }
        Value::Object(object) => {
            let items = object
                .iter()
                .map(|(key, value)| {
                    format!(
                        "'{}': {}",
                        form_edit_python_repr_string(key),
                        form_edit_python_repr(value)
                    )
                })
                .collect::<Vec<_>>();
            format!("{{{}}}", items.join(", "))
        }
    }
}

pub fn form_edit_python_repr_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

pub fn form_edit_insert_section_items(
    xml_text: &mut String,
    section: &str,
    lines: &[String],
) -> Result<(), String> {
    if lines.is_empty() {
        return Ok(());
    }
    let content = lines.join("\n");
    let empty = format!("<{section}/>");
    if xml_text.contains(&empty) {
        *xml_text = xml_text.replacen(
            &empty,
            &format!("<{section}>\n{content}\n\t</{section}>"),
            1,
        );
        return Ok(());
    }
    let Some(pos) = form_edit_find_section_close(xml_text, section) else {
        // A form the platform wrote without this section (a fresh form has no
        // commands or attributes yet): open the section before the root close.
        let Some(root_close) = xml_text.rfind("</Form>") else {
            return Err(format!("No <{section}> section found in form"));
        };
        let line_start = xml_text[..root_close]
            .rfind('\n')
            .map(|idx| idx + 1)
            .unwrap_or(root_close);
        xml_text.insert_str(
            line_start,
            &format!("\t<{section}>\n{content}\n\t</{section}>\n"),
        );
        return Ok(());
    };
    let insert_pos = xml_text[..pos]
        .rfind('\n')
        .map(|idx| idx + 1)
        .unwrap_or(pos);
    xml_text.insert_str(insert_pos, &format!("{content}\n"));
    Ok(())
}

pub fn form_edit_target_child_items_range(
    xml_text: &str,
    into_name: Option<&str>,
    after_name: Option<&str>,
) -> Result<FormEditInsertTarget, String> {
    let doc = Document::parse(xml_text).map_err(|err| format!("[ERROR] XML parse error: {err}"))?;
    let root = doc.root_element();
    let root_child_items = form_child(root, "ChildItems");
    if let Some(into_name) = into_name.filter(|name| !name.is_empty()) {
        let Some(target) = form_edit_find_placement_element(root, into_name)? else {
            return Err(format!("[ERROR] Target group '{into_name}' not found"));
        };
        if let Some(child_items) = form_child(target, "ChildItems") {
            return Ok(FormEditInsertTarget::ExistingChildItems {
                child_indent: form_edit_child_indent_for_section(xml_text, child_items.range()),
                range: child_items.range(),
            });
        }
        let element_indent = form_edit_line_indent_at(xml_text, target.range().start);
        let child_items_indent = format!("{element_indent}\t");
        let child_indent = format!("{child_items_indent}\t");
        return Ok(FormEditInsertTarget::ElementNeedsChildItems {
            range: target.range(),
            tag: target.tag_name().name().to_string(),
            element_indent,
            child_items_indent,
            child_indent,
        });
    }
    if let Some(after_name) = after_name.filter(|name| !name.is_empty()) {
        let Some(after_element) = form_edit_find_placement_element(root, after_name)? else {
            return Err(format!("[ERROR] Element '{after_name}' not found"));
        };
        let child_items = after_element
            .parent_element()
            .filter(|node| node.has_tag_name((FORM_LOGFORM_NS, "ChildItems")))
            .ok_or_else(|| {
                format!("No parent <ChildItems> section found for form element '{after_name}'")
            })?;
        return Ok(FormEditInsertTarget::AfterElement {
            child_indent: form_edit_child_indent_for_section(xml_text, child_items.range()),
            pos: after_element.range().end,
        });
    }
    let Some(child_items) = root_child_items else {
        // 8.3.27 orders the root as ChildItems -> Attributes -> Parameters ->
        // Commands and omits the section entirely while the form is empty, so
        // the new one goes before the earliest of those that is present.
        let anchor = ["Attributes", "Parameters", "Commands"]
            .iter()
            .filter_map(|tag| form_child(root, tag))
            .map(|node| node.range().start)
            .min()
            .or_else(|| {
                root.children()
                    .rfind(roxmltree::Node::is_element)
                    .map(|node| node.range().end)
            })
            .ok_or_else(|| "No <ChildItems> section found in form".to_string())?;
        let section_indent = form_edit_line_indent_at(xml_text, anchor);
        let child_indent = format!("{section_indent}\t");
        return Ok(FormEditInsertTarget::RootNeedsChildItems {
            pos: anchor,
            section_indent,
            child_indent,
        });
    };
    Ok(FormEditInsertTarget::ExistingChildItems {
        child_indent: form_edit_child_indent_for_section(xml_text, child_items.range()),
        range: child_items.range(),
    })
}

fn form_edit_placement_is_command_bar(
    root: roxmltree::Node<'_, '_>,
    into_name: Option<&str>,
    after_name: Option<&str>,
) -> Result<bool, String> {
    let container = if let Some(name) = into_name.filter(|name| !name.is_empty()) {
        form_edit_find_placement_element(root, name)?
    } else if let Some(name) = after_name.filter(|name| !name.is_empty()) {
        form_edit_find_placement_element(root, name)?
            .and_then(|node| node.parent_element())
            .filter(|node| node.has_tag_name((FORM_LOGFORM_NS, "ChildItems")))
            .and_then(|node| node.parent_element())
    } else {
        None
    };
    Ok(container
        .is_some_and(|node| matches!(node.tag_name().name(), "AutoCommandBar" | "CommandBar")))
}

fn form_edit_find_placement_element<'a, 'input>(
    root: roxmltree::Node<'a, 'input>,
    name: &str,
) -> Result<Option<roxmltree::Node<'a, 'input>>, String> {
    // Share the working-tree lookup used for event targets. It includes
    // root/table AutoCommandBar but never descends into the root BaseForm.
    let matches = form_edit_find_element_nodes(root, name);
    match matches.as_slice() {
        [] => Ok(None),
        [node] => Ok(Some(*node)),
        _ => Err(format!("[ERROR] Target element '{name}' is ambiguous")),
    }
}

pub fn form_edit_insert_lines_into_target(
    xml_text: &mut String,
    target: FormEditInsertTarget,
    lines: &[String],
) -> Result<(), String> {
    if lines.is_empty() {
        return Ok(());
    }
    match target {
        FormEditInsertTarget::ExistingChildItems { range, .. } => {
            form_edit_insert_lines_into_range(xml_text, range, "ChildItems", lines)
        }
        FormEditInsertTarget::ElementNeedsChildItems {
            range,
            tag,
            element_indent,
            child_items_indent,
            ..
        } => form_edit_insert_child_items_into_element(
            xml_text,
            range,
            &tag,
            &element_indent,
            &child_items_indent,
            lines,
        ),
        FormEditInsertTarget::AfterElement { pos, .. } => {
            let content = lines.join("\n");
            xml_text.insert_str(pos, &format!("\n{content}"));
            Ok(())
        }
        FormEditInsertTarget::RootNeedsChildItems {
            pos,
            section_indent,
            ..
        } => {
            let content = lines.join("\n");
            xml_text.insert_str(
                pos,
                &format!(
                    "<ChildItems>\n{content}\n{section_indent}</ChildItems>\n{section_indent}"
                ),
            );
            Ok(())
        }
    }
}

pub fn form_edit_insert_lines_into_range(
    xml_text: &mut String,
    range: std::ops::Range<usize>,
    section: &str,
    lines: &[String],
) -> Result<(), String> {
    let content = lines.join("\n");
    let child_indent = form_edit_line_indent(lines.first().map(String::as_str).unwrap_or(""));
    let parent_indent = form_edit_parent_indent(&child_indent);
    let section_text = &xml_text[range.clone()];
    if section_text.trim_end().ends_with("/>") {
        xml_text.replace_range(
            range,
            &format!("<{section}>\n{content}\n{parent_indent}</{section}>"),
        );
        return Ok(());
    }
    let close = format!("</{section}>");
    let Some(relative_pos) = section_text.rfind(&close) else {
        return Err(format!("No <{section}> section found in form target"));
    };
    let insert_pos = section_text[..relative_pos]
        .rfind('\n')
        .map(|idx| range.start + idx + 1)
        .unwrap_or(range.start + relative_pos);
    xml_text.insert_str(insert_pos, &format!("{content}\n"));
    Ok(())
}

pub fn form_edit_parent_indent(child_indent: &str) -> String {
    child_indent
        .strip_suffix('\t')
        .unwrap_or(child_indent)
        .to_string()
}

pub fn form_edit_child_indent_for_section(xml_text: &str, range: std::ops::Range<usize>) -> String {
    let section_text = &xml_text[range.clone()];
    let open_end = section_text.find('>').map(|idx| idx + 1).unwrap_or(0);
    if let Some(close_pos) = section_text.rfind("</ChildItems>") {
        let body = &section_text[open_end..close_pos];
        if let Some(indent) = form_edit_first_element_indent(body) {
            return indent;
        }
        if let Some(parent_indent) = form_edit_trailing_tab_indent(&section_text[..close_pos]) {
            return format!("{parent_indent}\t");
        }
    }
    format!("{}\t", form_edit_line_indent_at(xml_text, range.start))
}

pub fn form_edit_first_element_indent(text: &str) -> Option<String> {
    for (idx, _) in text.match_indices('<') {
        if text[idx..].starts_with("</") {
            continue;
        }
        if let Some(indent) = form_edit_trailing_tab_indent(&text[..idx]) {
            return Some(indent);
        }
    }
    None
}

pub fn form_edit_trailing_tab_indent(text: &str) -> Option<String> {
    let line = text.rsplit('\n').next()?;
    if line.chars().all(|ch| ch == '\t') {
        Some(line.to_string())
    } else {
        None
    }
}

pub fn form_edit_insert_child_items_into_element(
    xml_text: &mut String,
    range: std::ops::Range<usize>,
    tag: &str,
    element_indent: &str,
    child_items_indent: &str,
    lines: &[String],
) -> Result<(), String> {
    let content = lines.join("\n");
    let element_text = &xml_text[range.clone()];
    let open_tag_end = form_edit_opening_tag_end(element_text, 0)
        .ok_or_else(|| format!("No opening <{tag}> tag found in form target"))?;
    let opening_tag = &element_text[..=open_tag_end];
    if opening_tag.trim_end().ends_with("/>") {
        let relative_pos = opening_tag
            .rfind("/>")
            .ok_or_else(|| format!("Self-closing <{tag}> tag has no '/>' terminator"))?;
        let pos = range.start + relative_pos;
        xml_text.replace_range(
            pos..pos + 2,
            &format!(
                ">\n{child_items_indent}<ChildItems>\n{content}\n{child_items_indent}</ChildItems>\n{element_indent}</{tag}>"
            ),
        );
        return Ok(());
    }
    let close = format!("</{tag}>");
    let Some(relative_pos) = element_text.rfind(&close) else {
        return Err(format!("No closing </{tag}> found in form target"));
    };
    let insert_pos = element_text[..relative_pos]
        .rfind('\n')
        .map(|idx| range.start + idx + 1)
        .unwrap_or(range.start + relative_pos);
    xml_text.insert_str(
        insert_pos,
        &format!(
            "{child_items_indent}<ChildItems>\n{content}\n{child_items_indent}</ChildItems>\n"
        ),
    );
    Ok(())
}

pub fn form_edit_ensure_emitted_namespaces(
    xml_text: &mut String,
    root_start: usize,
    emitted_fragments: &str,
) -> Result<(), String> {
    if emitted_fragments.is_empty() {
        return Ok(());
    }
    let root_open_end = form_edit_opening_tag_end(xml_text, root_start)
        .ok_or_else(|| "No opening <Form> tag found in form".to_string())?;
    let additions = {
        let root_opening = &xml_text[root_start..=root_open_end];
        let mut standalone_root = root_opening[..root_opening.len() - 1].to_string();
        standalone_root.push_str("/>");
        let document = Document::parse(&standalone_root)
            .map_err(|error| format!("[ERROR] Invalid <Form> opening tag: {error}"))?;
        let root = document.root_element();
        require_form_root(root).map_err(|error| format!("[ERROR] {error}"))?;
        let mut additions = String::new();
        for (prefix, uri) in form_edit_emitter_namespaces() {
            let needed = emitted_fragments.contains(&format!("{prefix}:"));
            if !needed {
                continue;
            }
            match root.lookup_namespace_uri(Some(prefix)) {
                Some(bound) if bound == uri => {}
                Some(bound) => {
                    return Err(format!(
                        "[ERROR] Namespace prefix '{prefix}' is bound to '{bound}', expected '{uri}'"
                    ));
                }
                None => additions.push_str(&format!(" xmlns:{prefix}=\"{uri}\"")),
            }
        }
        additions
    };
    if !additions.is_empty() {
        xml_text.insert_str(root_open_end, &additions);
    }
    Ok(())
}

pub fn form_edit_emitter_namespaces() -> [(&'static str, &'static str); 11] {
    [
        ("app", "http://v8.1c.ru/8.2/managed-application/core"),
        ("cfg", "http://v8.1c.ru/8.1/data/enterprise/current-config"),
        (
            "dcssch",
            "http://v8.1c.ru/8.1/data-composition-system/schema",
        ),
        (
            "dcsset",
            "http://v8.1c.ru/8.1/data-composition-system/settings",
        ),
        ("dcscor", "http://v8.1c.ru/8.1/data-composition-system/core"),
        ("ent", "http://v8.1c.ru/8.1/data/enterprise"),
        ("v8", FORM_V8_NS),
        ("v8ui", "http://v8.1c.ru/8.1/data/ui"),
        ("xr", "http://v8.1c.ru/8.3/xcf/readable"),
        ("xs", "http://www.w3.org/2001/XMLSchema"),
        ("xsi", "http://www.w3.org/2001/XMLSchema-instance"),
    ]
}

pub fn form_edit_validate_emitted_type_qnames(
    root: roxmltree::Node<'_, '_>,
    emitted_type_qnames: &[String],
) -> Result<(), String> {
    for value in emitted_type_qnames {
        if value.contains(':') {
            // Особые типы объявляют префикс локально на `v8:Type`.
            // Проверяем узел, который несёт QName, а не корень формы.
            let nodes = root
                .descendants()
                .filter(|node| {
                    node.is_element()
                        && node.tag_name().name() == "Type"
                        && form_is_data_type_declaration_type_node(*node)
                        && node.text().is_some_and(|text| text.trim() == value)
                })
                .collect::<Vec<_>>();
            if nodes.is_empty() {
                return Err(format!(
                    "[ERROR] Emitted Type \"{value}\" is absent after edit"
                ));
            }
            for node in nodes {
                require_form_type_qname_binding(node, value)
                    .map_err(|error| format!("[ERROR] Emitted Type \"{value}\": {error}"))?;
            }
        }
    }
    Ok(())
}

pub fn form_edit_collect_emitted_type_qnames(
    emitted_fragments: &str,
) -> Result<Vec<String>, String> {
    if emitted_fragments.is_empty() {
        return Ok(Vec::new());
    }

    let mut wrapper = format!("<Form xmlns=\"{FORM_LOGFORM_NS}\"");
    for (prefix, uri) in form_edit_emitter_namespaces() {
        wrapper.push_str(&format!(" xmlns:{prefix}=\"{uri}\""));
    }
    wrapper.push('>');
    wrapper.push_str(emitted_fragments);
    wrapper.push_str("</Form>");

    let document = Document::parse(&wrapper)
        .map_err(|error| format!("[ERROR] XML parse error in emitted form fragment: {error}"))?;
    Ok(document
        .root_element()
        .descendants()
        .filter(|node| {
            node.is_element()
                && node.tag_name().name() == "Type"
                && form_is_data_type_declaration_type_node(*node)
        })
        .filter_map(|node| {
            let value = node.text().unwrap_or("").trim();
            (!value.is_empty()).then(|| value.to_string())
        })
        .collect())
}

pub fn form_edit_opening_tag_declares_namespace(opening: &str, prefix: &str) -> bool {
    let needle = format!("xmlns:{prefix}");
    let mut search_start = 0usize;
    while let Some(relative_start) = opening[search_start..].find(&needle) {
        let start = search_start + relative_start + needle.len();
        let remainder = opening[start..].trim_start();
        if remainder.starts_with('=') {
            return true;
        }
        search_start = start;
    }
    false
}

pub fn form_edit_find_section_close(xml_text: &str, section: &str) -> Option<usize> {
    let open = format!("<{section}");
    let close = format!("</{section}>");
    let mut offset = 0usize;
    let mut depth = 0usize;
    let mut started = false;

    loop {
        let next_open = xml_text[offset..].find(&open).map(|idx| offset + idx);
        let next_close = xml_text[offset..].find(&close).map(|idx| offset + idx);
        let next = (match (next_open, next_close) {
            (Some(open_idx), Some(close_idx)) => {
                Some((open_idx.min(close_idx), open_idx <= close_idx))
            }
            (Some(open_idx), None) => Some((open_idx, true)),
            (None, Some(close_idx)) => Some((close_idx, false)),
            (None, None) => None,
        })?;

        let (idx, is_open) = next;
        if is_open {
            let after_name = idx + open.len();
            let next_char = xml_text[after_name..].chars().next()?;
            if !(next_char == '>' || next_char == '/' || next_char.is_whitespace()) {
                offset = after_name;
                continue;
            }
            let tag_end = xml_text[idx..].find('>').map(|end| idx + end)?;
            let tag = &xml_text[idx..=tag_end];
            started = true;
            if !tag.trim_end().ends_with("/>") {
                depth += 1;
            }
            offset = tag_end + 1;
        } else {
            if started {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(idx);
                }
            }
            offset = idx + close.len();
        }
    }
}

pub fn emit_form_edit_attribute_item(
    lines: &mut Vec<String>,
    attr: &Map<String, Value>,
    name: &str,
    id: usize,
    indent: &str,
    lang: Option<&str>,
) -> Result<(), String> {
    lines.push(format!(
        "{indent}<Attribute name=\"{}\" id=\"{id}\">",
        unica_xml_format::text::escape_xml(name)
    ));
    let inner = format!("{indent}\t");
    if let Some(title) = attr.get("title").and_then(Value::as_str) {
        emit_form_mltext(lines, &inner, "Title", title, lang)?;
    }
    if let Some(type_name) = attr.get("type").and_then(Value::as_str) {
        emit_form_type(lines, type_name, &inner)?;
    } else {
        lines.push(format!("{inner}<Type/>"));
    }
    if attr.get("main").and_then(Value::as_bool) == Some(true) {
        lines.push(format!("{inner}<MainAttribute>true</MainAttribute>"));
    }
    if attr.get("savedData").and_then(Value::as_bool) == Some(true) {
        lines.push(format!("{inner}<SavedData>true</SavedData>"));
    }
    // Проверка заполнения реквизита формы в 8.3.27 — `<FillCheck>`:
    // `<FillChecking>` платформа у реквизита формы не знает (#1447).
    if let Some(fill_checking) = attr.get("fillChecking").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<FillCheck>{}</FillCheck>",
            unica_xml_format::text::escape_xml(fill_checking)
        ));
    }
    emit_form_attribute_columns(lines, attr.get("columns"), &inner, lang)?;
    lines.push(format!("{indent}</Attribute>"));
    Ok(())
}

/// One form `<Command>` for both the form compiler and `command.add`.
/// Properties follow the platform's FormCommand XDTO order: Title, ToolTip,
/// Use, Shortcut, Picture, Action, FunctionalOptions, Representation, ...
/// The element shapes match 8.3.27 dumps: multilingual Title and ToolTip,
/// Shortcut as text, Picture as `xr:Ref` with `xr:LoadTransparent`, and
/// Representation from the command's own DefaultRepresentation set.
pub fn emit_form_command(
    lines: &mut Vec<String>,
    cmd: &Map<String, Value>,
    name: &str,
    id: usize,
    indent: &str,
    lang: Option<&str>,
) -> Result<(), String> {
    lines.push(format!(
        "{indent}<Command name=\"{}\" id=\"{id}\">",
        unica_xml_format::text::escape_xml(name)
    ));
    let inner = format!("{indent}\t");
    if let Some(title) = cmd.get("title").and_then(Value::as_str) {
        emit_form_mltext(lines, &inner, "Title", title, lang)?;
    }
    if let Some(tooltip) = cmd.get("tooltip") {
        emit_form_mltext_value(lines, &inner, "ToolTip", tooltip, lang)?;
    }
    if let Some(value) = cmd.get("shortcut").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<Shortcut>{}</Shortcut>",
            unica_xml_format::text::escape_xml(value)
        ));
    }
    if let Some(picture) = cmd.get("picture").and_then(Value::as_str) {
        lines.push(format!("{inner}<Picture>"));
        lines.push(format!(
            "{inner}\t<xr:Ref>{}</xr:Ref>",
            unica_xml_format::text::escape_xml(picture)
        ));
        lines.push(format!(
            "{inner}\t<xr:LoadTransparent>true</xr:LoadTransparent>"
        ));
        lines.push(format!("{inner}</Picture>"));
    }
    if let Some(action) = cmd.get("action").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<Action>{}</Action>",
            unica_xml_format::text::escape_xml(action)
        ));
    }
    if let Some(value) = cmd.get("representation").and_then(Value::as_str) {
        let representation = form_compile_command_representation(value)
            .ok_or_else(|| form_command_representation_error(name, value))?;
        lines.push(format!(
            "{inner}<Representation>{representation}</Representation>"
        ));
    }
    lines.push(format!("{indent}</Command>"));
    Ok(())
}

struct FormCompileEvent {
    name: String,
    handler: String,
}

#[derive(Default)]
pub struct FormIdAllocator {
    pub next: usize,
}

impl FormIdAllocator {
    pub fn new() -> Self {
        Self { next: 0 }
    }

    pub fn allocate(&mut self) -> Result<usize, String> {
        self.next = self.next.checked_add(1).ok_or("Form ID space exhausted")?;
        Ok(self.next)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormEditElementDefinitionKind {
    Table,
    Label,
    LabelField,
    Button,
    CommandBar,
    Pages,
    Page,
    Group,
    CheckBox,
    InputField,
    HtmlDocumentField,
    TextDocumentField,
    AutoCommandBar,
}

impl FormEditElementDefinitionKind {
    fn from_object(object: &Map<String, Value>) -> Result<Self, String> {
        // `commandBar` is also a nested Table property, and `group` is also a
        // Page layout property. Prefer unambiguous primary discriminators before
        // considering those two standalone-element shorthands.
        let primary_candidates = [
            (Self::Table, "table", object.contains_key("table")),
            (Self::Label, "label", object.contains_key("label")),
            (
                Self::LabelField,
                "labelField",
                object.contains_key("labelField"),
            ),
            (Self::Button, "button", object.contains_key("button")),
            (Self::Pages, "pages", object.contains_key("pages")),
            (Self::Page, "page", object.contains_key("page")),
            (Self::CheckBox, "check", object.contains_key("check")),
            (Self::InputField, "input", object.contains_key("input")),
            (Self::HtmlDocumentField, "html", object.contains_key("html")),
            (
                Self::TextDocumentField,
                "textDoc",
                object.contains_key("textDoc"),
            ),
            (
                Self::AutoCommandBar,
                "autoCmdBar/autoCommandBar",
                object.contains_key("autoCmdBar") || object.contains_key("autoCommandBar"),
            ),
        ]
        .into_iter()
        .filter_map(|(kind, label, present)| present.then_some((kind, label)))
        .collect::<Vec<_>>();

        match primary_candidates.as_slice() {
            [(kind, _label)] => Ok(*kind),
            [] => {
                let command_bar =
                    object.contains_key("cmdBar") || object.contains_key("commandBar");
                let group = object.contains_key("group");
                match (command_bar, group, object.contains_key("name")) {
                    (true, false, _) => Ok(Self::CommandBar),
                    (false, true, _) => Ok(Self::Group),
                    (false, false, true) => Ok(Self::InputField),
                    (true, true, _) => Err(
                        "Form element has ambiguous commandBar and group discriminators"
                            .to_string(),
                    ),
                    (false, false, false) => Err(format!(
                        "Unsupported form element in native compiler: {}",
                        serde_json::to_string(object).unwrap_or_else(|_| "<invalid>".to_string())
                    )),
                }
            }
            multiple => Err(format!(
                "Form element must contain exactly one type discriminator; found: {}",
                multiple
                    .iter()
                    .map(|(_kind, label)| *label)
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    const fn event_kind(self) -> FormElementKind {
        match self {
            Self::Table => FormElementKind::Table,
            Self::Label => FormElementKind::LabelDecoration,
            Self::LabelField => FormElementKind::LabelField,
            Self::Button => FormElementKind::Button,
            Self::CommandBar | Self::AutoCommandBar => FormElementKind::CommandBar,
            Self::Pages => FormElementKind::Pages,
            Self::Page => FormElementKind::Page,
            Self::Group => FormElementKind::Group,
            Self::CheckBox => FormElementKind::CheckBoxField,
            Self::InputField => FormElementKind::InputField,
            Self::HtmlDocumentField => FormElementKind::HtmlDocumentField,
            Self::TextDocumentField => FormElementKind::TextDocumentField,
        }
    }

    fn name(self, object: &Map<String, Value>) -> Result<&str, String> {
        let (keys, description): (&[&str], &str) = match self {
            Self::Table => (&["table"], "table"),
            Self::Label => (&["label"], "label decoration"),
            Self::LabelField => (&["labelField"], "label field"),
            Self::Button => (&["button"], "button"),
            Self::CommandBar => (&["cmdBar", "commandBar"], "command bar"),
            Self::Pages => (&["pages"], "pages container"),
            Self::Page => (&["page"], "page"),
            Self::Group => (&["group"], "group"),
            Self::CheckBox => (&["check"], "checkbox"),
            Self::InputField => (&["input"], "input"),
            Self::HtmlDocumentField => (&["html"], "HTML document field"),
            Self::TextDocumentField => (&["textDoc"], "text document field"),
            Self::AutoCommandBar => (&["autoCmdBar", "autoCommandBar"], "auto command bar"),
        };
        form_edit_definition_element_name(object, keys, description)
    }
}

fn form_edit_definition_element_name<'a>(
    object: &'a Map<String, Value>,
    keys: &[&str],
    description: &str,
) -> Result<&'a str, String> {
    object
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| {
            keys.iter()
                .find_map(|key| object.get(*key).and_then(Value::as_str))
        })
        .ok_or_else(|| format!("Form {description} is missing name"))
}

fn form_validate_requested_text_document_bindings(
    root: roxmltree::Node<'_, '_>,
    definition: &Value,
) -> Result<(), String> {
    let Some(elements) = definition.get("elements").and_then(Value::as_array) else {
        return Ok(());
    };
    for element in elements.iter().filter_map(Value::as_object) {
        let Some(name) = element.get("textDoc").and_then(Value::as_str) else {
            continue;
        };
        let path = element
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .ok_or_else(|| {
                format!(
                    "TextDocumentField '{name}' requires non-empty path bound to a TextDocument attribute"
                )
            })?;
        let path = strip_form_binding_prefixes(path);
        let resolved_path = if let Some(rest) = path.strip_prefix("Items.") {
            let (table_name, column) = rest.split_once(".CurrentData.").ok_or_else(|| {
                format!("TextDocumentField '{name}' has unsupported binding path '{path}'")
            })?;
            let table = form_edit_find_element_nodes(root, table_name)
                .into_iter()
                .find(|node| node.has_tag_name((FORM_LOGFORM_NS, "Table")));
            let table_path = table
                .and_then(|node| form_child(node, "DataPath"))
                .and_then(|node| node.text())
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .ok_or_else(|| {
                    format!("TextDocumentField '{name}' table '{table_name}' has no DataPath")
                })?;
            format!("{}.{}", strip_form_binding_prefixes(table_path), column)
        } else {
            path.clone()
        };
        let mut parts = resolved_path.split('.');
        let first = parts.next().unwrap_or_default();
        let mut attribute = form_child(root, "Attributes").and_then(|items| {
            items.children().find(|node| {
                node.is_element()
                    && node.tag_name().name() == "Attribute"
                    && node.attribute("name") == Some(first)
            })
        });
        for part in parts {
            attribute = attribute.and_then(|owner| {
                form_child(owner, "Columns").and_then(|columns| {
                    columns.children().find(|node| {
                        node.is_element()
                            && node.tag_name().name() == "Column"
                            && node.attribute("name") == Some(part)
                    })
                })
            });
        }
        let valid = attribute
            .and_then(|node| form_child(node, "Type"))
            .is_some_and(|type_node| {
                let types = type_node
                    .descendants()
                    .filter(|node| node.has_tag_name((FORM_V8_NS, "Type")))
                    .collect::<Vec<_>>();
                !types.is_empty()
                    && types.iter().all(|node| {
                        node.text()
                            .and_then(|value| value.trim().split_once(':'))
                            .is_some_and(|(prefix, local)| {
                                local == "TextDocument"
                                    && node.lookup_namespace_uri(Some(prefix))
                                        == Some("http://v8.1c.ru/8.1/data/txtedt")
                            })
                    })
            });
        if !valid {
            return Err(format!(
                "TextDocumentField '{name}' path '{path}' requires a TextDocument attribute"
            ));
        }
    }
    Ok(())
}

pub fn form_compile_xml(
    defn: &Value,
    format_version: &str,
) -> Result<(String, FormCompileStats), String> {
    fn contains_text_document_field(value: &Value) -> bool {
        match value {
            Value::Object(object) => {
                object.contains_key("textDoc") || object.values().any(contains_text_document_field)
            }
            Value::Array(items) => items.iter().any(contains_text_document_field),
            _ => false,
        }
    }
    if contains_text_document_field(defn) {
        return Err(
            "TextDocumentField is created only by item.add; the retired form compiler does not support it"
                .to_string(),
        );
    }
    if let Some(attributes) = defn.get("attributes").and_then(Value::as_array) {
        form_edit_validate_attribute_columns(attributes)?;
    }
    form_compile_validate_data_paths(defn)?;
    form_compile_validate_element_properties(defn)?;
    form_compile_validate_commands(defn)?;
    let context = form_project_event_context(
        FormEventContext {
            definition: FormDefinitionKind::Regular,
            direct_part_writable: true,
            main_attribute: MainAttributeKind::Unknown,
            main_attribute_type: None,
            main_attribute_provenance: MainAttributeProvenance::Missing,
            main_attribute_name: None,
            metadata_owner: None,
        },
        0,
        defn,
    );
    let events = form_compile_plan_events(defn, &context)?;

    if let Some(elements) = defn.get("elements").and_then(Value::as_array) {
        form_edit_validate_new_element_event_tree(elements, &context)?;
    }

    let lang = Some(LEGACY_FORM_TEXT_LANGUAGE);
    let mut ids = FormIdAllocator::new();
    let mut lines = Vec::<String>::new();
    lines.push("<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string());
    lines.push(format!(
        "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\" xmlns:dcscor=\"http://v8.1c.ru/8.1/data-composition-system/core\" xmlns:dcssch=\"http://v8.1c.ru/8.1/data-composition-system/schema\" xmlns:dcsset=\"http://v8.1c.ru/8.1/data-composition-system/settings\" xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\" xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\" xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"{format_version}\">"
    ));

    let form_title = json_string_field(defn, "title").or_else(|| {
        defn.get("properties")
            .and_then(|props| json_string_field(props, "title"))
    });
    if let Some(title) = form_title.as_deref() {
        emit_form_mltext(&mut lines, "\t", "Title", title, lang)?;
    }

    let props_src = defn.get("properties").and_then(Value::as_object);
    let mut props = Map::new();
    let has_explicit_auto_title = props_src
        .is_some_and(|values| values.contains_key("autoTitle") || values.contains_key("AutoTitle"));
    if form_title.is_some() && !has_explicit_auto_title {
        props.insert("autoTitle".to_string(), Value::Bool(false));
    }
    let has_explicit_catalog_scope = props_src.is_some_and(|values| {
        values.contains_key("useForFoldersAndItems") || values.contains_key("UseForFoldersAndItems")
    });
    if !has_explicit_catalog_scope
        && context
            .main_attribute_type
            .as_deref()
            .is_some_and(|value| value.starts_with("CatalogObject."))
    {
        props.insert(
            "useForFoldersAndItems".to_string(),
            Value::String("Items".to_string()),
        );
    }
    let is_report_form = context.main_attribute_type.as_deref().is_some_and(|value| {
        value.starts_with("ReportObject.") || value.starts_with("ExternalReportObject.")
    });
    if is_report_form {
        for (json_key, xml_key, default_value) in FORM_REPORT_ROOT_DEFAULTS {
            let has_explicit_value = props_src.is_some_and(|values| {
                values.contains_key(json_key) || values.contains_key(xml_key)
            });
            if !has_explicit_value {
                props.insert(
                    json_key.to_string(),
                    Value::String(default_value.to_string()),
                );
            }
        }
    }
    if let Some(values) = props_src {
        for (key, value) in values {
            props.insert(key.clone(), value.clone());
        }
    }
    if !props.is_empty() {
        emit_form_properties(&mut lines, &props, "\t")?;
    }

    emit_form_auto_command_bar(&mut lines, defn, "\t");
    emit_form_events(&mut lines, &events, "\t");

    if let Some(elements) = defn.get("elements").and_then(Value::as_array) {
        if !elements.is_empty() {
            lines.push("\t<ChildItems>".to_string());
            for element in elements {
                emit_form_element(&mut lines, element, "\t\t", &mut ids, lang)?;
            }
            lines.push("\t</ChildItems>".to_string());
        }
    }

    let attributes = defn
        .get("attributes")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    emit_form_attributes(&mut lines, defn.get("attributes"), "\t", &mut ids, lang)?;

    let parameters = defn
        .get("parameters")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    emit_form_parameters(&mut lines, defn.get("parameters"), "\t")?;

    let commands = defn
        .get("commands")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    emit_form_commands(&mut lines, defn.get("commands"), "\t", &mut ids, lang)?;

    lines.push("</Form>".to_string());
    Ok((
        format!("{}\n", lines.join("\n")),
        FormCompileStats {
            element_ids: ids.next,
            attributes,
            commands,
            parameters,
        },
    ))
}

const FORM_REPORT_ROOT_DEFAULTS: [(&str, &str, &str); 4] = [
    ("reportFormType", "ReportFormType", "Main"),
    ("autoShowState", "AutoShowState", "Auto"),
    ("reportResultViewMode", "ReportResultViewMode", "Auto"),
    (
        "viewModeApplicationOnSetReportResult",
        "ViewModeApplicationOnSetReportResult",
        "Auto",
    ),
];

fn form_compile_validate_data_paths(defn: &Value) -> Result<(), String> {
    let attributes = defn
        .get("attributes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter_map(|attribute| {
            attribute
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .map(|name| (name, attribute))
        })
        .collect::<HashMap<_, _>>();

    if let Some(elements) = defn.get("elements").and_then(Value::as_array) {
        let mut table_paths = HashMap::<String, Option<String>>::new();
        form_compile_collect_table_paths(elements, &mut table_paths)?;
        form_compile_validate_element_data_paths(elements, &attributes, &table_paths)?;
    }
    Ok(())
}

fn form_compile_collect_table_paths(
    elements: &[Value],
    table_paths: &mut HashMap<String, Option<String>>,
) -> Result<(), String> {
    for element in elements {
        let Some(object) = element.as_object() else {
            continue;
        };
        let kind = FormEditElementDefinitionKind::from_object(object)?;
        if kind == FormEditElementDefinitionKind::Table {
            let name = kind.name(object)?.to_string();
            let path = object
                .get("path")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            table_paths.entry(name).or_insert(path);
        }
        for key in ["children", "columns"] {
            if let Some(nested) = object.get(key).and_then(Value::as_array) {
                form_compile_collect_table_paths(nested, table_paths)?;
            }
        }
    }
    Ok(())
}

fn form_compile_validate_element_data_paths(
    elements: &[Value],
    attributes: &HashMap<&str, &Map<String, Value>>,
    table_paths: &HashMap<String, Option<String>>,
) -> Result<(), String> {
    for element in elements {
        let Some(object) = element.as_object() else {
            continue;
        };
        let kind = FormEditElementDefinitionKind::from_object(object)?;
        let element_name = kind.name(object)?;
        for (json_key, xml_tag) in FORM_BINDING_PATH_PROPERTIES {
            let Some(path) = object.get(json_key).and_then(Value::as_str) else {
                continue;
            };
            if !form_compile_binding_property_supported(kind, json_key) {
                if kind == FormEditElementDefinitionKind::Group && json_key == "headerDataPath" {
                    return Err(format!(
                        "UsualGroup '{element_name}' does not support HeaderDataPath in 8.3.27; HeaderDataPath belongs to ColumnGroup, which the compiler does not support"
                    ));
                }
                return Err(format!(
                    "Form element '{element_name}' does not support {xml_tag}/{json_key}"
                ));
            }
            let resolution =
                resolve_form_binding_path(path, |table_name| match table_paths.get(table_name) {
                    None => FormBindingTablePath::Missing,
                    Some(None) => FormBindingTablePath::Unbound,
                    Some(Some(path)) => FormBindingTablePath::Bound(path.clone()),
                });
            match resolution {
                FormBindingPathResolution::Skip | FormBindingPathResolution::UnknownItemsShape => {}
                FormBindingPathResolution::MissingTable(table_name) => {
                    return Err(format!(
                        "Form element '{element_name}' {xml_tag}='{path}': table element '{table_name}' not found"
                    ));
                }
                FormBindingPathResolution::Attribute(root)
                    if !attributes.contains_key(root.as_str()) =>
                {
                    return Err(format!(
                        "Form element '{element_name}' {xml_tag}='{path}' references missing top-level form attribute '{root}'"
                    ));
                }
                FormBindingPathResolution::Attribute(_) => {}
            }
        }
        form_compile_validate_related_data_paths(
            object,
            kind,
            element_name,
            attributes,
            table_paths,
        )?;
        for key in ["children", "columns"] {
            if let Some(nested) = object.get(key).and_then(Value::as_array) {
                form_compile_validate_element_data_paths(nested, attributes, table_paths)?;
            }
        }
    }
    Ok(())
}

fn form_compile_validate_related_data_paths(
    object: &Map<String, Value>,
    kind: FormEditElementDefinitionKind,
    element_name: &str,
    attributes: &HashMap<&str, &Map<String, Value>>,
    table_paths: &HashMap<String, Option<String>>,
) -> Result<(), String> {
    if kind == FormEditElementDefinitionKind::InputField {
        let multiple_paths = [
            ("multipleValueDataPath", "MultipleValueDataPath"),
            (
                "multipleValuePictureDataPath",
                "MultipleValuePictureDataPath",
            ),
            (
                "multipleValuePresentDataPath",
                "MultipleValuePresentDataPath",
            ),
        ];
        if multiple_paths
            .iter()
            .any(|(json_key, _)| object.get(*json_key).and_then(Value::as_str).is_some())
        {
            let used_multiple_tags = multiple_paths
                .iter()
                .filter(|(json_key, _)| object.get(*json_key).and_then(Value::as_str).is_some())
                .map(|(_, xml_tag)| *xml_tag)
                .collect::<Vec<_>>()
                .join(", ");
            let data_path = object
                .get("path")
                .and_then(Value::as_str)
                .filter(|path| !path.trim().is_empty())
                .ok_or_else(|| {
                    format!(
                        "InputField '{element_name}' uses multiple-value data paths without DataPath/path"
                    )
                })?;
            let Some(data_segments) = form_compile_semantic_path_segments(data_path, table_paths)
            else {
                return Ok(());
            };
            let collection = form_compile_attribute_contract_at_path(&data_segments, attributes)
                .ok_or_else(|| {
                    format!(
                        "InputField '{element_name}' DataPath='{data_path}' does not identify a declared collection form attribute"
                    )
                })?;
            let collection_type = collection
                .get("type")
                .and_then(Value::as_str)
                .map(normalize_form_type)
                .unwrap_or_default();
            if !matches!(
                collection_type.as_str(),
                "ValueList" | "ValueTable" | "ValueTree"
            ) {
                return Err(format!(
                    "InputField '{element_name}' {used_multiple_tags} require a collection DataPath in 8.3.27, but '{data_path}' has type '{}'",
                    collection
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("<missing>")
                ));
            }

            for (json_key, xml_tag) in multiple_paths {
                let Some(path) = object.get(json_key).and_then(Value::as_str) else {
                    continue;
                };
                let Some(path_segments) = form_compile_semantic_path_segments(path, table_paths)
                else {
                    continue;
                };
                if !form_compile_is_strict_subpath(&data_segments, &path_segments) {
                    return Err(format!(
                        "InputField '{element_name}' {xml_tag}='{path}' must be a subpath of collection DataPath='{data_path}' in 8.3.27"
                    ));
                }
                if matches!(collection_type.as_str(), "ValueTable" | "ValueTree") {
                    let column_name = &path_segments[data_segments.len()];
                    let has_column = collection
                        .get("columns")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_object)
                        .any(|column| {
                            column.get("name").and_then(Value::as_str) == Some(column_name.as_str())
                        });
                    if !has_column {
                        return Err(format!(
                            "InputField '{element_name}' {xml_tag}='{path}' references undeclared collection column '{column_name}'"
                        ));
                    }
                }
            }
        }
    }

    if kind == FormEditElementDefinitionKind::Table {
        if let Some(row_picture_path) = object.get("rowPictureDataPath").and_then(Value::as_str) {
            let data_path = object
                .get("path")
                .and_then(Value::as_str)
                .filter(|path| !path.trim().is_empty())
                .ok_or_else(|| {
                    format!("Table '{element_name}' uses RowPictureDataPath without DataPath/path")
                })?;
            if let (Some(data_segments), Some(picture_segments)) = (
                form_compile_semantic_path_segments(data_path, table_paths),
                form_compile_semantic_path_segments(row_picture_path, table_paths),
            ) {
                if !form_compile_is_strict_subpath(&data_segments, &picture_segments) {
                    return Err(format!(
                        "Table '{element_name}' RowPictureDataPath='{row_picture_path}' must be a subpath of DataPath='{data_path}' in 8.3.27"
                    ));
                }
            }
        }
    }

    Ok(())
}

fn form_compile_semantic_path_segments(
    path: &str,
    table_paths: &HashMap<String, Option<String>>,
) -> Option<Vec<String>> {
    let path = path.trim();
    if path.is_empty() || is_opaque_form_binding(path) {
        return None;
    }
    let clean_path = strip_form_binding_prefixes(path);
    let segments = clean_path
        .split('.')
        .filter(|segment| !segment.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if segments.first().is_some_and(|segment| segment == "Items")
        && segments
            .get(2)
            .is_some_and(|segment| segment == "CurrentData")
    {
        let table_name = segments.get(1)?;
        let table_path = table_paths.get(table_name)?.as_deref()?;
        if table_path.trim().is_empty() || is_opaque_form_binding(table_path) {
            return None;
        }
        let mut resolved = strip_form_binding_prefixes(table_path)
            .split('.')
            .filter(|segment| !segment.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        resolved.extend(segments.into_iter().skip(3));
        Some(resolved)
    } else {
        Some(segments)
    }
}

fn form_compile_attribute_contract_at_path<'a>(
    segments: &[String],
    attributes: &HashMap<&str, &'a Map<String, Value>>,
) -> Option<&'a Map<String, Value>> {
    let mut contract = *attributes.get(segments.first()?.as_str())?;
    for segment in &segments[1..] {
        contract = contract
            .get("columns")
            .and_then(Value::as_array)?
            .iter()
            .filter_map(Value::as_object)
            .find(|column| column.get("name").and_then(Value::as_str) == Some(segment.as_str()))?;
    }
    Some(contract)
}

fn form_compile_is_strict_subpath(parent: &[String], candidate: &[String]) -> bool {
    candidate.len() > parent.len()
        && candidate
            .iter()
            .zip(parent)
            .all(|(candidate, parent)| candidate == parent)
}

fn form_compile_binding_property_supported(
    kind: FormEditElementDefinitionKind,
    json_key: &str,
) -> bool {
    match json_key {
        "path" => matches!(
            kind,
            FormEditElementDefinitionKind::Table
                | FormEditElementDefinitionKind::HtmlDocumentField
                | FormEditElementDefinitionKind::TextDocumentField
                | FormEditElementDefinitionKind::LabelField
                | FormEditElementDefinitionKind::CheckBox
                | FormEditElementDefinitionKind::InputField
        ),
        "titleDataPath" => matches!(
            kind,
            FormEditElementDefinitionKind::Group | FormEditElementDefinitionKind::Page
        ),
        "footerDataPath" => matches!(
            kind,
            FormEditElementDefinitionKind::LabelField
                | FormEditElementDefinitionKind::CheckBox
                | FormEditElementDefinitionKind::InputField
        ),
        "headerDataPath" => false,
        "multipleValueDataPath"
        | "multipleValuePresentDataPath"
        | "multipleValuePictureDataPath" => kind == FormEditElementDefinitionKind::InputField,
        "rowPictureDataPath" => kind == FormEditElementDefinitionKind::Table,
        _ => false,
    }
}

fn form_compile_plan_events(
    defn: &Value,
    context: &FormEventContext,
) -> Result<Vec<FormCompileEvent>, String> {
    let Some(value) = defn.get("events") else {
        return Ok(Vec::new());
    };
    let events = value.as_object().ok_or_else(|| {
        FormEventDiagnostic::new(FormEventDiagnosticCode::EventNotAllowed, "form", "events")
            .with_detail("events must be an object mapping event names to string handlers")
            .to_string()
    })?;

    events
        .iter()
        .map(|(name, value)| {
            let handler = value.as_str().ok_or_else(|| {
                let (code, detail) = if value
                    .as_object()
                    .is_some_and(|event| event.contains_key("callType"))
                {
                    (
                        FormEventDiagnosticCode::EventNotAllowed,
                        "events map accepts only string handlers; callType is not supported",
                    )
                } else {
                    (
                        FormEventDiagnosticCode::EmptyHandler,
                        "event handler must be a string",
                    )
                };
                FormEventDiagnostic::new(code, "form", name)
                    .with_detail(detail)
                    .to_string()
            })?;
            let binding = FormEventBinding::new(name, handler);
            validate_event(context, FormEventTarget::Form, &binding)
                .map_err(|diagnostic| diagnostic.to_string())?;
            Ok(FormCompileEvent {
                name: name.clone(),
                handler: handler.to_string(),
            })
        })
        .collect()
}

fn emit_form_events(lines: &mut Vec<String>, events: &[FormCompileEvent], indent: &str) {
    if events.is_empty() {
        return;
    }

    lines.push(format!("{indent}<Events>"));
    for event in events {
        lines.push(format!(
            "{indent}\t<Event name=\"{}\">{}</Event>",
            unica_xml_format::text::escape_xml(&event.name),
            unica_xml_format::text::escape_xml(&event.handler)
        ));
    }
    lines.push(format!("{indent}</Events>"));
}

pub fn emit_form_auto_command_bar(lines: &mut Vec<String>, defn: &Value, indent: &str) {
    let mut explicit_bar = None::<&Value>;
    if let Some(elements) = defn.get("elements").and_then(Value::as_array) {
        explicit_bar = elements.iter().find(|element| {
            element.as_object().is_some_and(|object| {
                object.contains_key("autoCmdBar") || object.contains_key("autoCommandBar")
            })
        });
    }

    let mut name = "ФормаКоманднаяПанель".to_string();
    let mut halign = None::<String>;
    let mut autofill = true;
    let mut has_children = false;
    if let Some(bar) = explicit_bar.and_then(Value::as_object) {
        if let Some(value) = bar
            .get("autoCmdBar")
            .or_else(|| bar.get("autoCommandBar"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            name = value.to_string();
        }
        if let Some(value) = bar.get("name").and_then(Value::as_str) {
            name = value.to_string();
        }
        halign = bar
            .get("horizontalAlign")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        autofill = bar.get("autofill").and_then(Value::as_bool).unwrap_or(true);
        has_children = bar
            .get("children")
            .and_then(Value::as_array)
            .is_some_and(|children| !children.is_empty());
    } else if let Some(elements) = defn.get("elements").and_then(Value::as_array) {
        if elements.iter().any(form_element_has_command_bar) {
            autofill = false;
        }
    }

    if halign.is_some() || !autofill || has_children {
        lines.push(format!(
            "{indent}<AutoCommandBar name=\"{}\" id=\"-1\">",
            unica_xml_format::text::escape_xml(&name)
        ));
        if let Some(halign) = halign {
            lines.push(format!(
                "{indent}\t<HorizontalAlign>{}</HorizontalAlign>",
                unica_xml_format::text::escape_xml(&halign)
            ));
        }
        if !autofill {
            lines.push(format!("{indent}\t<Autofill>false</Autofill>"));
        }
        lines.push(format!("{indent}</AutoCommandBar>"));
    } else {
        lines.push(format!(
            "{indent}<AutoCommandBar name=\"{}\" id=\"-1\"/>",
            unica_xml_format::text::escape_xml(&name)
        ));
    }
}

pub fn form_element_has_command_bar(element: &Value) -> bool {
    let Some(object) = element.as_object() else {
        return false;
    };
    if object.contains_key("cmdBar") || object.contains_key("commandBar") {
        return true;
    }
    for key in ["children", "columns"] {
        if let Some(children) = object.get(key).and_then(Value::as_array) {
            if children.iter().any(form_element_has_command_bar) {
                return true;
            }
        }
    }
    false
}

/// Language of multilingual values written by the pre-v0.13 form commands
/// (`unica.form.add`, `unica.form.compile`, `unica.form.edit`). They take no
/// source-set language profile and keep `ru`; the v0.13 form operations
/// write in the source set's default language instead (#909, #1257).
pub const LEGACY_FORM_TEXT_LANGUAGE: &str = "ru";

/// Refusal of a plain-string form text emitted without a language; see
/// [`form_edit_apply_definition`].
pub const FORM_TEXT_LANGUAGE_REQUIRED: &str = "a plain-string form text needs the default language";

/// Writes `text` as a multilingual `tag` in the `lang` language
/// (`LanguageCode`); an empty `text` writes the empty element. A non-empty
/// `text` without a language fails with [`FORM_TEXT_LANGUAGE_REQUIRED`].
pub fn emit_form_mltext(
    lines: &mut Vec<String>,
    indent: &str,
    tag: &str,
    text: &str,
    lang: Option<&str>,
) -> Result<(), String> {
    if text.is_empty() {
        lines.push(format!("{indent}<{tag}/>"));
        return Ok(());
    }
    let lang = lang
        .filter(|lang| !lang.is_empty())
        .ok_or_else(|| FORM_TEXT_LANGUAGE_REQUIRED.to_string())?;
    lines.push(format!("{indent}<{tag}>"));
    lines.push(format!("{indent}\t<v8:item>"));
    lines.push(format!(
        "{indent}\t\t<v8:lang>{}</v8:lang>",
        unica_xml_format::text::escape_xml(lang)
    ));
    lines.push(format!(
        "{indent}\t\t<v8:content>{}</v8:content>",
        unica_xml_format::text::escape_xml(text)
    ));
    lines.push(format!("{indent}\t</v8:item>"));
    lines.push(format!("{indent}</{tag}>"));
    Ok(())
}

/// Writes a multilingual `tag` from `value`: a plain string in the `lang`
/// language, a language-to-text object as given.
pub fn emit_form_mltext_value(
    lines: &mut Vec<String>,
    indent: &str,
    tag: &str,
    value: &Value,
    lang: Option<&str>,
) -> Result<(), String> {
    if let Some(text) = value.as_str() {
        if text.is_empty() {
            return Ok(());
        }
        return emit_form_mltext(lines, indent, tag, text, lang);
    }

    let Some(texts) = value.as_object() else {
        return Ok(());
    };
    let texts = texts
        .iter()
        .filter(|(language, _)| !matches!(language.as_str(), "text" | "formatted"))
        .filter_map(|(language, text)| {
            text.as_str()
                .filter(|text| !text.is_empty())
                .map(|text| (language, text))
        })
        .collect::<Vec<_>>();
    if texts.is_empty() {
        return Ok(());
    }

    lines.push(format!("{indent}<{tag}>"));
    for (language, text) in texts {
        lines.push(format!("{indent}\t<v8:item>"));
        lines.push(format!(
            "{indent}\t\t<v8:lang>{}</v8:lang>",
            unica_xml_format::text::escape_xml(language)
        ));
        lines.push(format!(
            "{indent}\t\t<v8:content>{}</v8:content>",
            unica_xml_format::text::escape_xml(text)
        ));
        lines.push(format!("{indent}\t</v8:item>"));
    }
    lines.push(format!("{indent}</{tag}>"));
    Ok(())
}

pub fn emit_form_element_tooltip(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    indent: &str,
    lang: Option<&str>,
) -> Result<(), String> {
    if let Some(tooltip) = element.get("tooltip") {
        emit_form_mltext_value(lines, indent, "ToolTip", tooltip, lang)?;
    }
    if let Some(representation) = element
        .get("tooltipRepresentation")
        .and_then(Value::as_str)
        .filter(|representation| !representation.is_empty())
    {
        lines.push(format!(
            "{indent}<ToolTipRepresentation>{}</ToolTipRepresentation>",
            unica_xml_format::text::escape_xml(representation)
        ));
    }
    Ok(())
}

pub fn emit_form_font(lines: &mut Vec<String>, font: &Value, indent: &str) {
    if let Some(reference) = font.as_str().filter(|reference| !reference.is_empty()) {
        lines.push(format!(
            "{indent}<Font ref=\"{}\" kind=\"StyleItem\"/>",
            unica_xml_format::text::escape_xml(reference)
        ));
        return;
    }
    let Some(attributes) = font.as_object() else {
        return;
    };
    let mut serialized = Vec::new();
    for name in [
        "ref",
        "faceName",
        "height",
        "bold",
        "italic",
        "underline",
        "strikeout",
        "kind",
        "scale",
    ] {
        let Some(value) = attributes.get(name).filter(|value| !value.is_null()) else {
            continue;
        };
        let value = if let Some(flag) = value.as_bool() {
            if flag { "true" } else { "false" }.to_string()
        } else {
            json_value_to_python_string(value)
        };
        serialized.push(format!(
            "{name}=\"{}\"",
            unica_xml_format::text::escape_xml(&value)
        ));
    }
    lines.push(format!("{indent}<Font {}/>", serialized.join(" ")));
}

/// Корневое свойство формы по имени из JSON (`width` или `Width`): его
/// место в порядке 8.3.27 и описание из общей таблицы.
fn form_root_xml_property(
    name: &str,
) -> Option<(usize, &'static unica_forms::root_properties::RootProperty)> {
    let mut characters = name.chars();
    let first = characters.next()?;
    let candidate = format!("{}{}", first.to_uppercase(), characters.as_str());
    unica_forms::root_properties::root_property(&candidate)
}

#[cfg(any(test, feature = "test-support"))]
pub fn form_8_3_27_enum_values_for_test(xml_name: &str) -> Option<&'static [&'static str]> {
    form_8_3_27_enum_values(xml_name)
}

fn form_8_3_27_enum_values(xml_name: &str) -> Option<&'static [&'static str]> {
    match xml_name {
        "TableRepresentation" => Some(&["List", "HierarchicalList", "Tree"]),
        "FormChildrenGroup" => Some(&[
            "Horizontal",
            "Vertical",
            "HorizontalIfPossible",
            "AlwaysHorizontal",
        ]),
        "UsualGroupBehavior" => Some(&["Usual", "Collapsible", "PopUp", "Auto"]),
        "UsualGroupRepresentation" => Some(&[
            "None",
            "StrongSeparation",
            "WeakSeparation",
            "NormalSeparation",
            "GroupBox",
            "Line",
            "Margin",
        ]),
        "PagesRepresentation" => Some(&[
            "None",
            "TabsOnTop",
            "TabsOnBottom",
            "TabsOnLeftHorizontal",
            "TabsOnRightHorizontal",
            "Swipe",
            "Auto",
        ]),
        "CurrentRowUse" => Some(&["Use", "DontUse", "Auto"]),
        "InitialTreeView" => Some(&["NoExpand", "ExpandTopLevel", "ExpandAllLevels"]),
        "ChoiceFoldersAndItems" => Some(&["Items", "Folders", "FoldersAndItems"]),
        "UpdateOnDataChange" => Some(&["Auto", "DontUpdate"]),
        "CheckBoxType" => Some(&["Auto", "CheckBox", "Tumbler", "Switcher"]),
        "FormElementTitleLocation" => Some(&["None", "Auto", "Left", "Top", "Right", "Bottom"]),
        "ManagedFormButtonType" => Some(&[
            "CommandBarButton",
            "UsualButton",
            "Hyperlink",
            "CommandBarHyperlink",
        ]),
        "ButtonRepresentation" => Some(&["Text", "Picture", "PictureAndText", "Auto"]),
        // `logform:FormCommand.Representation` is `DefaultRepresentation`,
        // not the button set: 8.3.27 refuses `PictureAndText` on a command.
        "DefaultRepresentation" => Some(&["Text", "Picture", "TextPicture", "Auto"]),
        "ButtonLocationInCommandBar" => Some(&[
            "Auto",
            "InAdditionalSubmenu",
            "InCommandBar",
            "InCommandBarAndInAdditionalSubmenu",
        ]),
        // Свойства корня формы: перечисления общей таблицы 8.3.27.
        _ => unica_forms::root_properties::root_property(xml_name).and_then(|(_, property)| {
            match property.value {
                unica_forms::root_properties::RootValue::Choice(choices) => Some(choices),
                _ => None,
            }
        }),
    }
}

fn form_compile_validate_element_properties(defn: &Value) -> Result<(), String> {
    let Some(elements) = defn.get("elements").and_then(Value::as_array) else {
        return Ok(());
    };
    form_compile_validate_element_property_tree(elements)
}

fn form_compile_validate_commands(defn: &Value) -> Result<(), String> {
    let Some(commands) = defn.get("commands").and_then(Value::as_array) else {
        return Ok(());
    };
    for command in commands {
        let Some(value) = command.get("representation") else {
            continue;
        };
        let name = command.get("name").and_then(Value::as_str).unwrap_or("");
        let text = value
            .as_str()
            .ok_or_else(|| format!("form command {name} representation must be a string"))?;
        if form_compile_command_representation(text).is_none() {
            return Err(form_command_representation_error(name, text));
        }
    }
    Ok(())
}

fn form_command_representation_error(name: &str, value: &str) -> String {
    let allowed = form_8_3_27_enum_values("DefaultRepresentation")
        .expect("form command representation has an 8.3.27 contract");
    format!(
        "Command '{name}': Representation '{value}' is not valid for 8.3.27; expected one of {}",
        allowed.join(", ")
    )
}

fn form_compile_validate_element_property_tree(elements: &[Value]) -> Result<(), String> {
    for element in elements {
        let Some(object) = element.as_object() else {
            continue;
        };
        let kind = FormEditElementDefinitionKind::from_object(object)?;
        match kind {
            FormEditElementDefinitionKind::AutoCommandBar => {
                form_compile_validate_element_enum(object, "horizontalAlign", "HorizontalAlign")?;
            }
            FormEditElementDefinitionKind::Pages => {
                form_compile_validate_element_enum(
                    object,
                    "pagesRepresentation",
                    "PagesRepresentation",
                )?;
                form_compile_validate_element_enum(object, "currentRowUse", "CurrentRowUse")?;
            }
            FormEditElementDefinitionKind::Table => {
                form_compile_validate_element_boolean(object, "autoInsertNewRow")?;
                form_compile_validate_normalized_element_enum(
                    object,
                    "representation",
                    "TableRepresentation",
                    form_compile_table_representation,
                )?;
                form_compile_validate_element_enum(
                    object,
                    "commandBarLocation",
                    "CommandBarLocation",
                )?;
                form_compile_validate_element_enum(object, "initialTreeView", "InitialTreeView")?;
                if object.get("_dynList").and_then(Value::as_bool) == Some(true) {
                    form_compile_validate_element_enum(
                        object,
                        "choiceFoldersAndItems",
                        "ChoiceFoldersAndItems",
                    )?;
                    form_compile_validate_element_enum(
                        object,
                        "updateOnDataChange",
                        "UpdateOnDataChange",
                    )?;
                }
            }
            FormEditElementDefinitionKind::Group => {
                if object.contains_key("name") {
                    form_compile_validate_normalized_element_enum(
                        object,
                        "group",
                        "FormChildrenGroup",
                        form_compile_group_orientation,
                    )?;
                }
                form_compile_validate_normalized_element_enum(
                    object,
                    "behavior",
                    "UsualGroupBehavior",
                    form_compile_group_behavior,
                )?;
                form_compile_validate_normalized_element_enum(
                    object,
                    "representation",
                    "UsualGroupRepresentation",
                    form_compile_group_representation,
                )?;
                if object
                    .get("behavior")
                    .and_then(Value::as_str)
                    .and_then(form_compile_group_behavior)
                    == Some("PopUp")
                    && object
                        .get("representation")
                        .and_then(Value::as_str)
                        .and_then(form_compile_group_representation)
                        == Some("Line")
                {
                    return Err(
                        "UsualGroup behavior PopUp is incompatible with representation Line in 8.3.27"
                            .to_string(),
                    );
                }
                form_compile_validate_element_enum(object, "currentRowUse", "CurrentRowUse")?;
            }
            FormEditElementDefinitionKind::CheckBox => {
                form_compile_validate_normalized_element_enum(
                    object,
                    "checkBoxType",
                    "CheckBoxType",
                    form_compile_check_box_type,
                )?;
                form_compile_validate_normalized_element_enum(
                    object,
                    "titleLocation",
                    "FormElementTitleLocation",
                    form_compile_title_location,
                )?;
            }
            FormEditElementDefinitionKind::InputField
            | FormEditElementDefinitionKind::HtmlDocumentField
            | FormEditElementDefinitionKind::TextDocumentField => {
                if kind == FormEditElementDefinitionKind::TextDocumentField
                    && object.contains_key("editMode")
                {
                    return Err("TextDocumentField property editMode is unsupported".to_string());
                }
                form_compile_validate_normalized_element_enum(
                    object,
                    "titleLocation",
                    "FormElementTitleLocation",
                    form_compile_title_location,
                )?;
            }
            FormEditElementDefinitionKind::Button => {
                form_compile_validate_normalized_element_enum(
                    object,
                    "type",
                    "ManagedFormButtonType",
                    form_compile_button_type,
                )?;
                form_compile_validate_normalized_element_enum(
                    object,
                    "representation",
                    "ButtonRepresentation",
                    form_compile_button_representation,
                )?;
                form_compile_validate_normalized_element_enum(
                    object,
                    "locationInCommandBar",
                    "ButtonLocationInCommandBar",
                    form_compile_button_location,
                )?;
            }
            _ => {}
        }
        for child_key in ["children", "columns"] {
            if let Some(children) = object.get(child_key).and_then(Value::as_array) {
                form_compile_validate_element_property_tree(children)?;
            }
        }
    }
    Ok(())
}

fn form_compile_validate_element_boolean(
    object: &Map<String, Value>,
    json_name: &str,
) -> Result<(), String> {
    let Some(value) = object.get(json_name) else {
        return Ok(());
    };
    value
        .as_bool()
        .map(|_| ())
        .ok_or_else(|| format!("form element property {json_name} must be a boolean"))
}

fn form_compile_validate_element_enum(
    object: &Map<String, Value>,
    json_name: &str,
    xml_name: &str,
) -> Result<(), String> {
    let Some(value) = object.get(json_name) else {
        return Ok(());
    };
    let text = value
        .as_str()
        .ok_or_else(|| format!("form element property {json_name} must be a string"))?;
    let allowed = form_8_3_27_enum_values(xml_name)
        .ok_or_else(|| format!("missing 8.3.27 enum contract for {json_name}"))?;
    if !allowed.contains(&text) {
        return Err(format!(
            "form element property {json_name} is not valid for 8.3.27: {text}; expected one of {}",
            allowed.join(", ")
        ));
    }
    Ok(())
}

fn form_compile_validate_normalized_element_enum(
    object: &Map<String, Value>,
    json_name: &str,
    contract_name: &str,
    normalize: fn(&str) -> Option<&'static str>,
) -> Result<(), String> {
    let Some(value) = object.get(json_name) else {
        return Ok(());
    };
    let text = value
        .as_str()
        .ok_or_else(|| format!("form element property {json_name} must be a string"))?;
    let allowed = form_8_3_27_enum_values(contract_name)
        .ok_or_else(|| format!("missing 8.3.27 enum contract for {json_name}"))?;
    let normalized = normalize(text);
    if normalized.is_none_or(|value| !allowed.contains(&value)) {
        return Err(format!(
            "form element property {json_name} is not valid for 8.3.27: {text}; expected one of {}",
            allowed.join(", ")
        ));
    }
    Ok(())
}

fn form_root_property_text(
    name: &str,
    property: &unica_forms::root_properties::RootProperty,
    value: &Value,
) -> Result<String, String> {
    use unica_forms::root_properties::RootValue;
    match property.value {
        RootValue::Flag => value
            .as_bool()
            .map(|flag| if flag { "true" } else { "false" }.to_string())
            .ok_or_else(|| format!("form root property {name} must be a boolean")),
        RootValue::Size => value
            .as_u64()
            .filter(|number| *number <= u32::MAX as u64)
            .map(|number| number.to_string())
            .ok_or_else(|| {
                format!("form root property {name} must be an integer in 0..=4294967295 for 8.3.27")
            }),
        RootValue::Choice(allowed) => {
            let text = value
                .as_str()
                .ok_or_else(|| format!("form root property {name} must be a string"))?;
            if !allowed.contains(&text) {
                return Err(format!(
                    "form root property {name} is not valid for 8.3.27: {text}; expected one of {}",
                    allowed.join(", ")
                ));
            }
            Ok(text.to_string())
        }
        RootValue::Reference => value
            .as_str()
            .filter(|text| !text.is_empty())
            .map(ToOwned::to_owned)
            .ok_or_else(|| format!("form root property {name} must be a non-empty string")),
    }
}

pub fn emit_form_properties(
    lines: &mut Vec<String>,
    props: &Map<String, Value>,
    indent: &str,
) -> Result<(), String> {
    let mut canonical = BTreeMap::<usize, (&str, &Value)>::new();
    for (name, value) in props {
        if name == "title" {
            continue;
        }
        let (order, property) = form_root_xml_property(name)
            .ok_or_else(|| format!("unsupported form root property for 8.3.27: {name}"))?;
        if let Some((previous, _)) = canonical.insert(order, (name, value)) {
            return Err(format!(
                "duplicate form root property {}: {previous} and {name}",
                property.name
            ));
        }
    }

    for (order, (name, value)) in canonical {
        let property = &unica_forms::root_properties::ROOT_PROPERTIES[order];
        let xml_name = property.name;
        if xml_name == "AutoTitle" && value.as_str() == Some("") {
            continue;
        }
        let text = form_root_property_text(name, property, value)?;
        lines.push(format!(
            "{indent}<{xml_name}>{}</{xml_name}>",
            unica_xml_format::text::escape_xml(&text)
        ));
    }
    Ok(())
}

pub fn emit_form_element(
    lines: &mut Vec<String>,
    element: &Value,
    indent: &str,
    ids: &mut FormIdAllocator,
    lang: Option<&str>,
) -> Result<(), String> {
    emit_form_element_with_context(lines, element, indent, ids, false, lang)
}

fn emit_form_element_with_context(
    lines: &mut Vec<String>,
    element: &Value,
    indent: &str,
    ids: &mut FormIdAllocator,
    in_command_bar: bool,
    lang: Option<&str>,
) -> Result<(), String> {
    let Some(object) = element.as_object() else {
        return Ok(());
    };
    let kind = FormEditElementDefinitionKind::from_object(object)?;
    form_edit_validate_element_event_payload_types(object, kind, kind.name(object)?)?;
    match kind {
        FormEditElementDefinitionKind::Table => {
            emit_form_table(lines, object, kind.name(object)?, indent, ids, lang)
        }
        FormEditElementDefinitionKind::Label => {
            emit_form_label_decoration(lines, object, kind.name(object)?, indent, ids, lang)
        }
        FormEditElementDefinitionKind::LabelField => {
            emit_form_label_field(lines, object, kind.name(object)?, indent, ids, lang)
        }
        FormEditElementDefinitionKind::Button => emit_form_button(
            lines,
            object,
            kind.name(object)?,
            indent,
            ids,
            in_command_bar,
            lang,
        ),
        FormEditElementDefinitionKind::CommandBar => {
            emit_form_command_bar_element(lines, object, kind.name(object)?, indent, ids, lang)
        }
        FormEditElementDefinitionKind::Pages => {
            emit_form_pages(lines, object, kind.name(object)?, indent, ids, lang)
        }
        FormEditElementDefinitionKind::Page => {
            emit_form_page(lines, object, kind.name(object)?, indent, ids, lang)
        }
        FormEditElementDefinitionKind::Group => {
            emit_form_group(lines, object, kind.name(object)?, indent, ids, lang)
        }
        FormEditElementDefinitionKind::CheckBox => {
            emit_form_check(lines, object, kind.name(object)?, indent, ids, lang)
        }
        FormEditElementDefinitionKind::InputField => {
            emit_form_input(lines, object, kind.name(object)?, indent, ids, lang)
        }
        FormEditElementDefinitionKind::HtmlDocumentField => emit_form_document_field(
            lines,
            object,
            kind.name(object)?,
            indent,
            ids,
            lang,
            "HTMLDocumentField",
        ),
        FormEditElementDefinitionKind::TextDocumentField => emit_form_document_field(
            lines,
            object,
            kind.name(object)?,
            indent,
            ids,
            lang,
            "TextDocumentField",
        ),
        FormEditElementDefinitionKind::AutoCommandBar => Ok(()),
    }
}

fn emit_form_binding_path_property(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    json_key: &str,
    xml_tag: &str,
    indent: &str,
) {
    if let Some(value) = element.get(json_key).and_then(Value::as_str) {
        if !value.is_empty() {
            lines.push(format!(
                "{indent}<{xml_tag}>{}</{xml_tag}>",
                unica_xml_format::text::escape_xml(value)
            ));
        }
    }
}

pub fn emit_form_group(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    name: &str,
    indent: &str,
    ids: &mut FormIdAllocator,
    lang: Option<&str>,
) -> Result<(), String> {
    let id = ids.allocate()?;
    lines.push(format!(
        "{indent}<UsualGroup name=\"{}\" id=\"{id}\">",
        unica_xml_format::text::escape_xml(name)
    ));
    let inner = format!("{indent}\t");
    if let Some(title) = element.get("title").and_then(Value::as_str) {
        emit_form_mltext(lines, &inner, "Title", title, lang)?;
    }
    if let Some(value) = element
        .get("group")
        .and_then(Value::as_str)
        .and_then(form_compile_group_orientation)
    {
        lines.push(format!("{inner}<Group>{value}</Group>"));
    }
    if let Some(value) = element.get("behavior").and_then(Value::as_str) {
        if let Some(behavior) = form_compile_group_behavior(value) {
            lines.push(format!("{inner}<Behavior>{behavior}</Behavior>"));
        }
    } else if element.get("group").and_then(Value::as_str) == Some("collapsible") {
        lines.push(format!("{inner}<Behavior>Collapsible</Behavior>"));
    }
    if element.get("collapsed").and_then(Value::as_bool) == Some(true) {
        lines.push(format!("{inner}<Collapsed>true</Collapsed>"));
    }
    if let Some(value) = element
        .get("representation")
        .and_then(Value::as_str)
        .and_then(form_compile_group_representation)
    {
        lines.push(format!(
            "{inner}<Representation>{}</Representation>",
            unica_xml_format::text::escape_xml(value)
        ));
    }
    emit_form_binding_path_property(lines, element, "titleDataPath", "TitleDataPath", &inner);
    if let Some(value) = element.get("currentRowUse").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<CurrentRowUse>{}</CurrentRowUse>",
            unica_xml_format::text::escape_xml(value)
        ));
    }
    if let Some(value) = element.get("showTitle").and_then(Value::as_bool) {
        lines.push(format!(
            "{inner}<ShowTitle>{}</ShowTitle>",
            if value { "true" } else { "false" }
        ));
    }
    emit_form_common_flags(lines, element, &inner);
    if let Some(value) = element.get("showLeftMargin").and_then(Value::as_bool) {
        lines.push(format!(
            "{inner}<ShowLeftMargin>{}</ShowLeftMargin>",
            if value { "true" } else { "false" }
        ));
    }
    emit_form_companion(
        lines,
        "ExtendedTooltip",
        &format!("{name}РасширеннаяПодсказка"),
        &inner,
        ids,
    )?;
    emit_form_children(lines, element, &inner, ids, lang)?;
    lines.push(format!("{indent}</UsualGroup>"));
    Ok(())
}

pub fn emit_form_pages(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    name: &str,
    indent: &str,
    ids: &mut FormIdAllocator,
    lang: Option<&str>,
) -> Result<(), String> {
    let id = ids.allocate()?;
    lines.push(format!(
        "{indent}<Pages name=\"{}\" id=\"{id}\">",
        unica_xml_format::text::escape_xml(name)
    ));
    let inner = format!("{indent}\t");
    if let Some(title) = element.get("title").and_then(Value::as_str) {
        emit_form_mltext(lines, &inner, "Title", title, lang)?;
    }
    if let Some(value) = element.get("pagesRepresentation").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<PagesRepresentation>{}</PagesRepresentation>",
            unica_xml_format::text::escape_xml(value)
        ));
    }
    if let Some(value) = element.get("currentRowUse").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<CurrentRowUse>{}</CurrentRowUse>",
            unica_xml_format::text::escape_xml(value)
        ));
    }
    emit_form_common_flags(lines, element, &inner);
    emit_form_companion(
        lines,
        "ExtendedTooltip",
        &format!("{name}РасширеннаяПодсказка"),
        &inner,
        ids,
    )?;
    emit_form_element_events(lines, element, name, &inner);
    emit_form_children(lines, element, &inner, ids, lang)?;
    lines.push(format!("{indent}</Pages>"));
    Ok(())
}

pub fn emit_form_page(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    name: &str,
    indent: &str,
    ids: &mut FormIdAllocator,
    lang: Option<&str>,
) -> Result<(), String> {
    let id = ids.allocate()?;
    lines.push(format!(
        "{indent}<Page name=\"{}\" id=\"{id}\">",
        unica_xml_format::text::escape_xml(name)
    ));
    let inner = format!("{indent}\t");
    if let Some(title) = element.get("title").and_then(Value::as_str) {
        emit_form_mltext(lines, &inner, "Title", title, lang)?;
    }
    emit_form_common_flags(lines, element, &inner);
    if let Some(value) = element
        .get("group")
        .and_then(Value::as_str)
        .and_then(form_compile_group_orientation)
    {
        lines.push(format!("{inner}<Group>{value}</Group>"));
    }
    if let Some(value) = element.get("showTitle").and_then(Value::as_bool) {
        lines.push(format!(
            "{inner}<ShowTitle>{}</ShowTitle>",
            if value { "true" } else { "false" }
        ));
    }
    emit_form_binding_path_property(lines, element, "titleDataPath", "TitleDataPath", &inner);
    emit_form_companion(
        lines,
        "ExtendedTooltip",
        &format!("{name}РасширеннаяПодсказка"),
        &inner,
        ids,
    )?;
    emit_form_children(lines, element, &inner, ids, lang)?;
    lines.push(format!("{indent}</Page>"));
    Ok(())
}

pub fn emit_form_children(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    indent: &str,
    ids: &mut FormIdAllocator,
    lang: Option<&str>,
) -> Result<(), String> {
    let Some(children) = element.get("children").and_then(Value::as_array) else {
        return Ok(());
    };
    if children.is_empty() {
        return Ok(());
    }
    lines.push(format!("{indent}<ChildItems>"));
    for child in children {
        emit_form_element(lines, child, &format!("{indent}\t"), ids, lang)?;
    }
    lines.push(format!("{indent}</ChildItems>"));
    Ok(())
}

pub fn form_compile_group_orientation(value: &str) -> Option<&'static str> {
    match value.to_lowercase().as_str() {
        "horizontal" => Some("Horizontal"),
        "vertical" | "collapsible" => Some("Vertical"),
        "alwayshorizontal" => Some("AlwaysHorizontal"),
        "horizontalifpossible" => Some("HorizontalIfPossible"),
        _ => None,
    }
}

pub fn form_compile_group_behavior(value: &str) -> Option<&'static str> {
    match value.to_lowercase().as_str() {
        "usual" => Some("Usual"),
        "collapsible" => Some("Collapsible"),
        "popup" => Some("PopUp"),
        "auto" => Some("Auto"),
        _ => None,
    }
}

pub fn form_compile_group_representation(value: &str) -> Option<&'static str> {
    match value.to_lowercase().as_str() {
        "none" => Some("None"),
        "normal" | "normalseparation" => Some("NormalSeparation"),
        "weak" | "weakseparation" => Some("WeakSeparation"),
        "strong" | "strongseparation" => Some("StrongSeparation"),
        "groupbox" => Some("GroupBox"),
        "line" => Some("Line"),
        "margin" => Some("Margin"),
        _ => None,
    }
}

fn form_compile_table_representation(value: &str) -> Option<&'static str> {
    match value.to_lowercase().as_str() {
        "list" => Some("List"),
        "hierarchicallist" => Some("HierarchicalList"),
        "tree" => Some("Tree"),
        _ => None,
    }
}

fn form_compile_check_box_type(value: &str) -> Option<&'static str> {
    match value.to_lowercase().as_str() {
        "auto" => Some("Auto"),
        "checkbox" => Some("CheckBox"),
        "switcher" => Some("Switcher"),
        "tumbler" => Some("Tumbler"),
        _ => None,
    }
}

fn form_compile_title_location(value: &str) -> Option<&'static str> {
    match value.to_lowercase().as_str() {
        "none" => Some("None"),
        "auto" => Some("Auto"),
        "left" => Some("Left"),
        "top" => Some("Top"),
        "right" => Some("Right"),
        "bottom" => Some("Bottom"),
        _ => None,
    }
}

fn form_compile_button_type(value: &str) -> Option<&'static str> {
    match value.to_lowercase().as_str() {
        "commandbar" | "commandbarbutton" => Some("CommandBarButton"),
        "usual" | "usualbutton" => Some("UsualButton"),
        "hyperlink" => Some("Hyperlink"),
        "commandbarhyperlink" => Some("CommandBarHyperlink"),
        _ => None,
    }
}

fn form_compile_button_representation(value: &str) -> Option<&'static str> {
    match value.to_lowercase().as_str() {
        "text" => Some("Text"),
        "picture" => Some("Picture"),
        "pictureandtext" => Some("PictureAndText"),
        "auto" => Some("Auto"),
        _ => None,
    }
}

fn form_compile_command_representation(value: &str) -> Option<&'static str> {
    match value.to_lowercase().as_str() {
        "text" => Some("Text"),
        "picture" => Some("Picture"),
        "textpicture" => Some("TextPicture"),
        "auto" => Some("Auto"),
        _ => None,
    }
}

fn form_compile_button_location(value: &str) -> Option<&'static str> {
    match value.to_lowercase().as_str() {
        "auto" => Some("Auto"),
        "inadditionalsubmenu" => Some("InAdditionalSubmenu"),
        "incommandbar" => Some("InCommandBar"),
        "incommandbarandinadditionalsubmenu" => Some("InCommandBarAndInAdditionalSubmenu"),
        _ => None,
    }
}

pub fn emit_form_check(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    name: &str,
    indent: &str,
    ids: &mut FormIdAllocator,
    lang: Option<&str>,
) -> Result<(), String> {
    let id = ids.allocate()?;
    lines.push(format!(
        "{indent}<CheckBoxField name=\"{}\" id=\"{id}\">",
        unica_xml_format::text::escape_xml(name)
    ));
    let inner = format!("{indent}\t");
    if let Some(path) = element.get("path").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<DataPath>{}</DataPath>",
            unica_xml_format::text::escape_xml(path)
        ));
    }
    if let Some(title) = element.get("title").and_then(Value::as_str) {
        emit_form_mltext(lines, &inner, "Title", title, lang)?;
    }
    emit_form_element_tooltip(lines, element, &inner, lang)?;
    emit_form_common_flags(lines, element, &inner);
    let title_location = element
        .get("titleLocation")
        .and_then(Value::as_str)
        .map(|value| form_compile_title_location(value).unwrap_or(value))
        .unwrap_or("Right");
    lines.push(format!(
        "{inner}<TitleLocation>{}</TitleLocation>",
        unica_xml_format::text::escape_xml(title_location)
    ));
    if let Some(value) = element.get("checkBoxType").and_then(Value::as_str) {
        if !value.is_empty() {
            let mapped = form_compile_check_box_type(value).unwrap_or(value);
            lines.push(format!(
                "{inner}<CheckBoxType>{}</CheckBoxType>",
                unica_xml_format::text::escape_xml(mapped)
            ));
        }
    } else {
        lines.push(format!("{inner}<CheckBoxType>Auto</CheckBoxType>"));
    }
    emit_form_binding_path_property(lines, element, "footerDataPath", "FooterDataPath", &inner);
    emit_form_companion(
        lines,
        "ContextMenu",
        &format!("{name}КонтекстноеМеню"),
        &inner,
        ids,
    )?;
    emit_form_companion(
        lines,
        "ExtendedTooltip",
        &format!("{name}РасширеннаяПодсказка"),
        &inner,
        ids,
    )?;
    emit_form_element_events(lines, element, name, &inner);
    lines.push(format!("{indent}</CheckBoxField>"));
    Ok(())
}

pub fn emit_form_input(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    name: &str,
    indent: &str,
    ids: &mut FormIdAllocator,
    lang: Option<&str>,
) -> Result<(), String> {
    let id = ids.allocate()?;
    lines.push(format!(
        "{indent}<InputField name=\"{}\" id=\"{id}\">",
        unica_xml_format::text::escape_xml(name)
    ));
    let inner = format!("{indent}\t");
    if let Some(path) = element.get("path").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<DataPath>{}</DataPath>",
            unica_xml_format::text::escape_xml(path)
        ));
    }
    if let Some(title) = element.get("title").and_then(Value::as_str) {
        emit_form_mltext(lines, &inner, "Title", title, lang)?;
    }
    emit_form_element_tooltip(lines, element, &inner, lang)?;
    emit_form_common_flags(lines, element, &inner);
    if let Some(value) = element.get("titleLocation").and_then(Value::as_str) {
        let location = form_compile_title_location(value).unwrap_or(value);
        lines.push(format!(
            "{inner}<TitleLocation>{}</TitleLocation>",
            unica_xml_format::text::escape_xml(location)
        ));
    }
    for (key, tag) in [("multiLine", "MultiLine"), ("passwordMode", "PasswordMode")] {
        if element.get(key).and_then(Value::as_bool) == Some(true) {
            lines.push(format!("{inner}<{tag}>true</{tag}>"));
        }
    }
    if let Some(value) = element.get("choiceButton").and_then(Value::as_bool) {
        lines.push(format!("{inner}<ChoiceButton>{value}</ChoiceButton>"));
    }
    for (key, tag) in [
        ("clearButton", "ClearButton"),
        ("spinButton", "SpinButton"),
        ("dropListButton", "DropListButton"),
        ("markIncomplete", "AutoMarkIncomplete"),
    ] {
        if element.get(key).and_then(Value::as_bool) == Some(true) {
            lines.push(format!("{inner}<{tag}>true</{tag}>"));
        }
    }
    if element.get("skipOnInput").and_then(Value::as_bool) == Some(true) {
        lines.push(format!("{inner}<SkipOnInput>true</SkipOnInput>"));
    }
    if let Some(value) = element.get("showInHeader").and_then(Value::as_bool) {
        lines.push(format!("{inner}<ShowInHeader>{value}</ShowInHeader>"));
    }
    if let Some(value) = element.get("headerHorizontalAlign").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<HeaderHorizontalAlign>{}</HeaderHorizontalAlign>",
            unica_xml_format::text::escape_xml(value)
        ));
    }
    for (key, tag) in [
        ("autoMaxWidth", "AutoMaxWidth"),
        ("autoMaxHeight", "AutoMaxHeight"),
    ] {
        if element.get(key).and_then(Value::as_bool) == Some(false) {
            lines.push(format!("{inner}<{tag}>false</{tag}>"));
        }
    }
    emit_form_binding_path_property(lines, element, "footerDataPath", "FooterDataPath", &inner);
    for (json_key, xml_tag) in [("width", "Width"), ("height", "Height")] {
        if let Some(value) = element.get(json_key) {
            let number = value
                .as_u64()
                .filter(|number| *number <= u32::MAX as u64)
                .ok_or_else(|| {
                    format!(
                        "form input property {json_key} must be an integer in 0..=4294967295 for 8.3.27"
                    )
                })?;
            lines.push(format!("{inner}<{xml_tag}>{number}</{xml_tag}>"));
        }
    }
    for (json_key, xml_tag) in [
        ("multipleValueDataPath", "MultipleValueDataPath"),
        (
            "multipleValuePictureDataPath",
            "MultipleValuePictureDataPath",
        ),
        (
            "multipleValuePresentDataPath",
            "MultipleValuePresentDataPath",
        ),
    ] {
        emit_form_binding_path_property(lines, element, json_key, xml_tag, &inner);
    }
    for (key, tag) in [
        ("horizontalStretch", "HorizontalStretch"),
        ("verticalStretch", "VerticalStretch"),
    ] {
        if element.get(key).and_then(Value::as_bool) == Some(true) {
            lines.push(format!("{inner}<{tag}>true</{tag}>"));
        }
    }
    if let Some(value) = element.get("horizontalAlign").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<HorizontalAlign>{}</HorizontalAlign>",
            unica_xml_format::text::escape_xml(value)
        ));
    }
    if let Some(hint) = element.get("inputHint").and_then(Value::as_str) {
        emit_form_mltext(lines, &inner, "InputHint", hint, lang)?;
    }
    emit_form_companion(
        lines,
        "ContextMenu",
        &format!("{name}КонтекстноеМеню"),
        &inner,
        ids,
    )?;
    emit_form_companion(
        lines,
        "ExtendedTooltip",
        &format!("{name}РасширеннаяПодсказка"),
        &inner,
        ids,
    )?;
    emit_form_element_events(lines, element, name, &inner);
    lines.push(format!("{indent}</InputField>"));
    Ok(())
}

pub fn emit_form_button(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    name: &str,
    indent: &str,
    ids: &mut FormIdAllocator,
    in_command_bar: bool,
    lang: Option<&str>,
) -> Result<(), String> {
    let id = ids.allocate()?;
    lines.push(format!(
        "{indent}<Button name=\"{}\" id=\"{id}\">",
        unica_xml_format::text::escape_xml(name)
    ));
    let inner = format!("{indent}\t");
    let button_type = element.get("type").and_then(Value::as_str);
    if let Some(mapped) = button_type
        .map(|button_type| {
            if in_command_bar {
                match button_type {
                    "usual" | "UsualButton" | "commandBar" | "CommandBarButton" => {
                        "CommandBarButton"
                    }
                    "hyperlink" | "Hyperlink" | "CommandBarHyperlink" => "CommandBarHyperlink",
                    other => other,
                }
            } else {
                form_compile_button_type(button_type).unwrap_or(button_type)
            }
        })
        .or(in_command_bar.then_some("CommandBarButton"))
    {
        lines.push(format!(
            "{inner}<Type>{}</Type>",
            unica_xml_format::text::escape_xml(mapped)
        ));
    }
    if let Some(representation) = element.get("representation").and_then(Value::as_str) {
        let representation =
            form_compile_button_representation(representation).unwrap_or(representation);
        lines.push(format!(
            "{inner}<Representation>{}</Representation>",
            unica_xml_format::text::escape_xml(representation)
        ));
    }
    let command_name = element
        .get("command")
        .and_then(Value::as_str)
        .filter(|command| !command.is_empty())
        .map(|command| format!("Form.Command.{command}"))
        .or_else(|| {
            element
                .get("commandName")
                .and_then(Value::as_str)
                .filter(|command_name| !command_name.is_empty())
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            element
                .get("stdCommand")
                .and_then(Value::as_str)
                .filter(|std_command| !std_command.is_empty())
                .map(|std_command| {
                    if let Some((item, command)) = std_command.rsplit_once('.') {
                        format!("Form.Item.{item}.StandardCommand.{command}")
                    } else {
                        format!("Form.StandardCommand.{std_command}")
                    }
                })
        });
    if let Some(command_name) = command_name {
        lines.push(format!(
            "{inner}<CommandName>{}</CommandName>",
            unica_xml_format::text::escape_xml(&command_name)
        ));
    }
    if let Some(title) = element.get("title").and_then(Value::as_str) {
        emit_form_mltext(lines, &inner, "Title", title, lang)?;
    }
    emit_form_element_tooltip(lines, element, &inner, lang)?;
    emit_form_common_flags(lines, element, &inner);
    if element.get("defaultButton").and_then(Value::as_bool) == Some(true) {
        lines.push(format!("{inner}<DefaultButton>true</DefaultButton>"));
    }
    if let Some(picture) = element.get("picture").and_then(Value::as_str) {
        lines.push(format!("{inner}<Picture>"));
        lines.push(format!(
            "{inner}\t<xr:Ref>{}</xr:Ref>",
            unica_xml_format::text::escape_xml(picture)
        ));
        lines.push(format!(
            "{inner}\t<xr:LoadTransparent>true</xr:LoadTransparent>"
        ));
        lines.push(format!("{inner}</Picture>"));
    }
    if let Some(location) = element.get("locationInCommandBar").and_then(Value::as_str) {
        let location = form_compile_button_location(location).unwrap_or(location);
        lines.push(format!(
            "{inner}<LocationInCommandBar>{}</LocationInCommandBar>",
            unica_xml_format::text::escape_xml(location)
        ));
    }
    if let Some(back_color) = element
        .get("backColor")
        .and_then(Value::as_str)
        .filter(|back_color| !back_color.is_empty())
    {
        lines.push(format!(
            "{inner}<BackColor>{}</BackColor>",
            unica_xml_format::text::escape_xml(back_color)
        ));
    }
    if let Some(font) = element.get("font") {
        emit_form_font(lines, font, &inner);
    }
    emit_form_companion(
        lines,
        "ExtendedTooltip",
        &format!("{name}РасширеннаяПодсказка"),
        &inner,
        ids,
    )?;
    emit_form_element_events(lines, element, name, &inner);
    lines.push(format!("{indent}</Button>"));
    Ok(())
}

pub fn emit_form_command_bar_element(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    name: &str,
    indent: &str,
    ids: &mut FormIdAllocator,
    lang: Option<&str>,
) -> Result<(), String> {
    let id = ids.allocate()?;
    lines.push(format!(
        "{indent}<CommandBar name=\"{}\" id=\"{id}\">",
        unica_xml_format::text::escape_xml(name)
    ));
    let inner = format!("{indent}\t");
    if let Some(command_source) = element
        .get("commandSource")
        .and_then(Value::as_str)
        .filter(|command_source| !command_source.is_empty())
    {
        lines.push(format!(
            "{inner}<CommandSource>{}</CommandSource>",
            unica_xml_format::text::escape_xml(command_source)
        ));
    }
    if element.get("autofill").and_then(Value::as_bool) == Some(true) {
        lines.push(format!("{inner}<Autofill>true</Autofill>"));
    }
    emit_form_common_flags(lines, element, &inner);
    if let Some(children) = element.get("children").and_then(Value::as_array) {
        if !children.is_empty() {
            lines.push(format!("{inner}<ChildItems>"));
            for child in children {
                emit_form_element_with_context(
                    lines,
                    child,
                    &format!("{inner}\t"),
                    ids,
                    true,
                    lang,
                )?;
            }
            lines.push(format!("{inner}</ChildItems>"));
        }
    }
    lines.push(format!("{indent}</CommandBar>"));
    Ok(())
}

/// `LabelDecoration` — the documented `label` element.
///
/// The platform writes a decoration layout-first: own content — flags,
/// hyperlink and geometry — comes before `Title`, unlike a field, whose title
/// leads. `Title` on a decoration carries the `formatted` attribute, taken
/// from the published `{text, formatted}` form or from the explicit
/// back-compat `formatted` key.
pub fn emit_form_label_decoration(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    name: &str,
    indent: &str,
    ids: &mut FormIdAllocator,
    lang: Option<&str>,
) -> Result<(), String> {
    let id = ids.allocate()?;
    lines.push(format!(
        "{indent}<LabelDecoration name=\"{}\" id=\"{id}\">",
        unica_xml_format::text::escape_xml(name)
    ));
    let inner = format!("{indent}\t");
    emit_form_common_flags(lines, element, &inner);
    if element.get("hyperlink").and_then(Value::as_bool) == Some(true) {
        lines.push(format!("{inner}<Hyperlink>true</Hyperlink>"));
    }
    for (json_key, xml_tag) in [("width", "Width"), ("height", "Height")] {
        if let Some(value) = element.get(json_key) {
            let number = value
                .as_u64()
                .filter(|number| *number <= u32::MAX as u64)
                .ok_or_else(|| {
                    format!(
                        "form label property {json_key} must be an integer in 0..=4294967295 for 8.3.27"
                    )
                })?;
            lines.push(format!("{inner}<{xml_tag}>{number}</{xml_tag}>"));
        }
    }
    for (key, tag) in [
        ("autoMaxWidth", "AutoMaxWidth"),
        ("autoMaxHeight", "AutoMaxHeight"),
    ] {
        if element.get(key).and_then(Value::as_bool) == Some(false) {
            lines.push(format!("{inner}<{tag}>false</{tag}>"));
        }
    }
    if let Some(value) = element.get("tooltipRepresentation").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<ToolTipRepresentation>{}</ToolTipRepresentation>",
            unica_xml_format::text::escape_xml(value)
        ));
    }
    emit_form_decoration_title(lines, element, name, &inner, lang)?;
    emit_form_companion(
        lines,
        "ContextMenu",
        &format!("{name}КонтекстноеМеню"),
        &inner,
        ids,
    )?;
    emit_form_companion(
        lines,
        "ExtendedTooltip",
        &format!("{name}РасширеннаяПодсказка"),
        &inner,
        ids,
    )?;
    emit_form_element_events(lines, element, name, &inner);
    lines.push(format!("{indent}</LabelDecoration>"));
    Ok(())
}

/// A decoration `Title` carries `formatted`. The flag comes from the published
/// `{text, formatted}` form, or from the back-compat `formatted` key beside a
/// plain title.
fn emit_form_decoration_title(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    name: &str,
    indent: &str,
    lang: Option<&str>,
) -> Result<(), String> {
    // A decoration without an explicit title takes its own name, the way the
    // platform does; writing nothing left the element unlabelled (#450 review).
    let fallback;
    let title = match element.get("title") {
        Some(title) => title,
        None => {
            fallback = Value::String(name.to_string());
            &fallback
        }
    };
    let formatted = title
        .get("formatted")
        .and_then(Value::as_bool)
        .or_else(|| element.get("formatted").and_then(Value::as_bool))
        .unwrap_or(false);
    let text = title.get("text").unwrap_or(title);
    let mut rendered = Vec::new();
    emit_form_mltext_value(&mut rendered, indent, "Title", text, lang)?;
    // A decoration title always carries the attribute, true or false; the
    // platform writes it either way, so omitting it when false would diverge.
    if let Some(first) = rendered.first_mut() {
        *first = first.replacen("<Title>", &format!("<Title formatted=\"{formatted}\">"), 1);
        *first = first.replacen(
            "<Title/>",
            &format!("<Title formatted=\"{formatted}\"/>"),
            1,
        );
    }
    lines.extend(rendered);
    Ok(())
}

fn emit_form_document_field(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    name: &str,
    indent: &str,
    ids: &mut FormIdAllocator,
    lang: Option<&str>,
    field_tag: &str,
) -> Result<(), String> {
    let id = ids.allocate()?;
    lines.push(format!(
        "{indent}<{field_tag} name=\"{}\" id=\"{id}\">",
        unica_xml_format::text::escape_xml(name)
    ));
    let inner = format!("{indent}\t");
    emit_form_binding_path_property(lines, element, "path", "DataPath", &inner);
    for key in [
        "visible",
        "hidden",
        "userVisible",
        "enabled",
        "disabled",
        "readOnly",
    ] {
        form_compile_validate_element_boolean(element, key)?;
    }
    if field_tag == "TextDocumentField" && element.contains_key("skipOnInput") {
        return Err("TextDocumentField property skipOnInput is unsupported".to_string());
    }
    emit_form_common_flags(lines, element, &inner);
    // Выгрузка 8.3.27 ставит SkipOnInput после Visible/Enabled/ReadOnly.
    if field_tag == "HTMLDocumentField" {
        form_compile_validate_element_boolean(element, "skipOnInput")?;
        if let Some(value) = element.get("skipOnInput").and_then(Value::as_bool) {
            lines.push(format!("{inner}<SkipOnInput>{value}</SkipOnInput>"));
        }
    }
    if let Some(title) = element.get("title").and_then(Value::as_str) {
        emit_form_mltext(lines, &inner, "Title", title, lang)?;
    }
    if let Some(value) = element.get("titleLocation").and_then(Value::as_str) {
        let location = form_compile_title_location(value).unwrap_or(value);
        lines.push(format!(
            "{inner}<TitleLocation>{}</TitleLocation>",
            unica_xml_format::text::escape_xml(location)
        ));
    }
    emit_form_element_tooltip(lines, element, &inner, lang)?;
    for (key, tag, auto_key, auto_tag) in [
        ("width", "Width", "autoMaxWidth", "AutoMaxWidth"),
        ("height", "Height", "autoMaxHeight", "AutoMaxHeight"),
    ] {
        if let Some(value) = element.get(key) {
            let number = value.as_u64().filter(|number| *number <= u32::MAX as u64)
                .ok_or_else(|| format!("form document field property {key} must be an integer in 0..=4294967295 for 8.3.27"))?;
            if key != "height" || number != 10 {
                lines.push(format!("{inner}<{tag}>{number}</{tag}>"));
            }
        }
        form_compile_validate_element_boolean(element, auto_key)?;
        // true — значение по умолчанию: платформа его не выгружает.
        if let Some(false) = element.get(auto_key).and_then(Value::as_bool) {
            lines.push(format!("{inner}<{auto_tag}>false</{auto_tag}>"));
        }
    }
    emit_form_companion(
        lines,
        "ContextMenu",
        &format!("{name}КонтекстноеМеню"),
        &inner,
        ids,
    )?;
    emit_form_companion(
        lines,
        "ExtendedTooltip",
        &format!("{name}РасширеннаяПодсказка"),
        &inner,
        ids,
    )?;
    emit_form_element_events(lines, element, name, &inner);
    lines.push(format!("{indent}</{field_tag}>"));
    Ok(())
}

pub fn emit_form_label_field(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    name: &str,
    indent: &str,
    ids: &mut FormIdAllocator,
    lang: Option<&str>,
) -> Result<(), String> {
    let id = ids.allocate()?;
    lines.push(format!(
        "{indent}<LabelField name=\"{}\" id=\"{id}\">",
        unica_xml_format::text::escape_xml(name)
    ));
    let inner = format!("{indent}\t");
    if let Some(path) = element.get("path").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<DataPath>{}</DataPath>",
            unica_xml_format::text::escape_xml(path)
        ));
    }
    emit_form_element_tooltip(lines, element, &inner, lang)?;
    emit_form_common_flags(lines, element, &inner);
    emit_form_binding_path_property(lines, element, "footerDataPath", "FooterDataPath", &inner);
    emit_form_companion(
        lines,
        "ContextMenu",
        &format!("{name}КонтекстноеМеню"),
        &inner,
        ids,
    )?;
    emit_form_companion(
        lines,
        "ExtendedTooltip",
        &format!("{name}РасширеннаяПодсказка"),
        &inner,
        ids,
    )?;
    emit_form_element_events(lines, element, name, &inner);
    lines.push(format!("{indent}</LabelField>"));
    Ok(())
}

pub fn emit_form_table(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    name: &str,
    indent: &str,
    ids: &mut FormIdAllocator,
    lang: Option<&str>,
) -> Result<(), String> {
    let id = ids.allocate()?;
    lines.push(format!(
        "{indent}<Table name=\"{}\" id=\"{id}\">",
        unica_xml_format::text::escape_xml(name)
    ));
    let inner = format!("{indent}\t");
    if let Some(representation) = element
        .get("representation")
        .and_then(Value::as_str)
        .and_then(form_compile_table_representation)
    {
        lines.push(format!(
            "{inner}<Representation>{representation}</Representation>"
        ));
    }
    if element.get("autoInsertNewRow").and_then(Value::as_bool) == Some(true) {
        lines.push(format!("{inner}<AutoInsertNewRow>true</AutoInsertNewRow>"));
    }
    if let Some(path) = element.get("path").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<DataPath>{}</DataPath>",
            unica_xml_format::text::escape_xml(path)
        ));
    }
    emit_form_common_flags(lines, element, &inner);
    if let Some(value) = element.get("commandBarLocation").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<CommandBarLocation>{}</CommandBarLocation>",
            unica_xml_format::text::escape_xml(value)
        ));
    }
    if let Some(value) = element.get("initialTreeView").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<InitialTreeView>{}</InitialTreeView>",
            unica_xml_format::text::escape_xml(value)
        ));
    }
    if element.get("enableDrag").and_then(Value::as_bool).is_some() {
        let value = if element.get("enableDrag").and_then(Value::as_bool) == Some(true) {
            "true"
        } else {
            "false"
        };
        lines.push(format!("{inner}<EnableDrag>{value}</EnableDrag>"));
    }
    if let Some(value) = element.get("rowPictureDataPath").and_then(Value::as_str) {
        lines.push(format!(
            "{inner}<RowPictureDataPath>{}</RowPictureDataPath>",
            unica_xml_format::text::escape_xml(value)
        ));
    }
    lines.push(format!("{inner}<RowFilter xsi:nil=\"true\"/>"));
    if element.get("_dynList").and_then(Value::as_bool) == Some(true) {
        emit_form_dynamic_list_table_block(lines, element, &inner);
    }
    emit_form_companion(
        lines,
        "ContextMenu",
        &format!("{name}КонтекстноеМеню"),
        &inner,
        ids,
    )?;
    if element.get("tableAutofill").is_some() {
        let id = ids.allocate()?;
        let value = if element.get("tableAutofill").and_then(Value::as_bool) == Some(true) {
            "true"
        } else {
            "false"
        };
        lines.push(format!(
            "{inner}<AutoCommandBar name=\"{}КоманднаяПанель\" id=\"{id}\">",
            unica_xml_format::text::escape_xml(name)
        ));
        lines.push(format!("{inner}\t<Autofill>{value}</Autofill>"));
        lines.push(format!("{inner}</AutoCommandBar>"));
    } else {
        emit_form_companion(
            lines,
            "AutoCommandBar",
            &format!("{name}КоманднаяПанель"),
            &inner,
            ids,
        )?;
    }
    emit_form_companion(
        lines,
        "ExtendedTooltip",
        &format!("{name}РасширеннаяПодсказка"),
        &inner,
        ids,
    )?;
    emit_form_table_addition(
        lines,
        "SearchStringAddition",
        name,
        "СтрокаПоиска",
        "SearchStringRepresentation",
        &inner,
        ids,
    )?;
    emit_form_table_addition(
        lines,
        "ViewStatusAddition",
        name,
        "СостояниеПросмотра",
        "ViewStatusRepresentation",
        &inner,
        ids,
    )?;
    emit_form_table_addition(
        lines,
        "SearchControlAddition",
        name,
        "УправлениеПоиском",
        "SearchControl",
        &inner,
        ids,
    )?;

    emit_form_element_events(lines, element, name, &inner);
    if let Some(columns) = element.get("columns").and_then(Value::as_array) {
        if !columns.is_empty() {
            lines.push(format!("{inner}<ChildItems>"));
            for column in columns {
                emit_form_element(lines, column, &format!("{inner}\t"), ids, lang)?;
            }
            lines.push(format!("{inner}</ChildItems>"));
        }
    }
    lines.push(format!("{indent}</Table>"));
    Ok(())
}

pub fn emit_form_dynamic_list_table_block(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    indent: &str,
) {
    let auto_refresh = if element.get("autoRefresh").and_then(Value::as_bool) == Some(true) {
        "true"
    } else {
        "false"
    };
    let auto_refresh_period = element
        .get("autoRefreshPeriod")
        .and_then(unica_operation_contract::input::json_i64_value)
        .unwrap_or(60);
    let choice = element
        .get("choiceFoldersAndItems")
        .and_then(Value::as_str)
        .unwrap_or("Items");
    let restore = if element.get("restoreCurrentRow").and_then(Value::as_bool) == Some(true) {
        "true"
    } else {
        "false"
    };
    let show_root = if element.get("showRoot").and_then(Value::as_bool) == Some(false) {
        "false"
    } else {
        "true"
    };
    let allow_root_choice = if element.get("allowRootChoice").and_then(Value::as_bool) == Some(true)
    {
        "true"
    } else {
        "false"
    };
    let update_on_data_change = element
        .get("updateOnDataChange")
        .and_then(Value::as_str)
        .unwrap_or("Auto");
    let allow_url = if element
        .get("allowGettingCurrentRowURL")
        .and_then(Value::as_bool)
        == Some(false)
    {
        "false"
    } else {
        "true"
    };

    lines.push(format!("{indent}<AutoRefresh>{auto_refresh}</AutoRefresh>"));
    lines.push(format!(
        "{indent}<AutoRefreshPeriod>{auto_refresh_period}</AutoRefreshPeriod>"
    ));
    lines.push(format!("{indent}<Period>"));
    lines.push(format!(
        "{indent}\t<v8:variant xsi:type=\"v8:StandardPeriodVariant\">Custom</v8:variant>"
    ));
    lines.push(format!(
        "{indent}\t<v8:startDate>0001-01-01T00:00:00</v8:startDate>"
    ));
    lines.push(format!(
        "{indent}\t<v8:endDate>0001-01-01T00:00:00</v8:endDate>"
    ));
    lines.push(format!("{indent}</Period>"));
    lines.push(format!(
        "{indent}<ChoiceFoldersAndItems>{}</ChoiceFoldersAndItems>",
        unica_xml_format::text::escape_xml(choice)
    ));
    lines.push(format!(
        "{indent}<RestoreCurrentRow>{restore}</RestoreCurrentRow>"
    ));
    lines.push(format!("{indent}<TopLevelParent xsi:nil=\"true\"/>"));
    lines.push(format!("{indent}<ShowRoot>{show_root}</ShowRoot>"));
    lines.push(format!(
        "{indent}<AllowRootChoice>{allow_root_choice}</AllowRootChoice>"
    ));
    lines.push(format!(
        "{indent}<UpdateOnDataChange>{}</UpdateOnDataChange>",
        unica_xml_format::text::escape_xml(update_on_data_change)
    ));
    lines.push(format!(
        "{indent}<AllowGettingCurrentRowURL>{allow_url}</AllowGettingCurrentRowURL>"
    ));
}

pub fn emit_form_table_addition(
    lines: &mut Vec<String>,
    tag: &str,
    table_name: &str,
    suffix: &str,
    source_type: &str,
    indent: &str,
    ids: &mut FormIdAllocator,
) -> Result<(), String> {
    let name = format!("{table_name}{suffix}");
    let id = ids.allocate()?;
    lines.push(format!(
        "{indent}<{tag} name=\"{}\" id=\"{id}\">",
        unica_xml_format::text::escape_xml(&name)
    ));
    let inner = format!("{indent}\t");
    lines.push(format!("{inner}<AdditionSource>"));
    lines.push(format!(
        "{inner}\t<Item>{}</Item>",
        unica_xml_format::text::escape_xml(table_name)
    ));
    lines.push(format!("{inner}\t<Type>{source_type}</Type>"));
    lines.push(format!("{inner}</AdditionSource>"));
    emit_form_companion(
        lines,
        "ContextMenu",
        &format!("{name}КонтекстноеМеню"),
        &inner,
        ids,
    )?;
    emit_form_companion(
        lines,
        "ExtendedTooltip",
        &format!("{name}РасширеннаяПодсказка"),
        &inner,
        ids,
    )?;
    lines.push(format!("{indent}</{tag}>"));
    Ok(())
}

pub fn emit_form_element_events(
    lines: &mut Vec<String>,
    element: &Map<String, Value>,
    element_name: &str,
    indent: &str,
) {
    let Some(events) = element.get("on").and_then(Value::as_array) else {
        return;
    };
    if events.is_empty() {
        return;
    }

    let handlers = element.get("handlers").and_then(Value::as_object);
    lines.push(format!("{indent}<Events>"));
    for event in events {
        let (event_name, handler, call_type) = if let Some(event_name) = event.as_str() {
            let handler = handlers
                .and_then(|values| values.get(event_name))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| form_event_handler_name(element_name, event_name));
            (event_name.to_string(), handler, None::<String>)
        } else if let Some(object) = event.as_object() {
            let event_name = object
                .get("event")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| json_value_to_python_string(event));
            let handler = object
                .get("handler")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| {
                    handlers
                        .and_then(|values| values.get(&event_name))
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .unwrap_or_else(|| form_event_handler_name(element_name, &event_name));
            let call_type = object
                .get("callType")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            (event_name, handler, call_type)
        } else {
            let event_name = json_value_to_python_string(event);
            let handler = handlers
                .and_then(|values| values.get(&event_name))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| form_event_handler_name(element_name, &event_name));
            (event_name, handler, None)
        };
        let call_type_attr = call_type
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(|value| {
                format!(
                    " callType=\"{}\"",
                    unica_xml_format::text::escape_xml(value)
                )
            })
            .unwrap_or_default();
        lines.push(format!(
            "{indent}\t<Event name=\"{}\"{}>{}</Event>",
            unica_xml_format::text::escape_xml(&event_name),
            call_type_attr,
            unica_xml_format::text::escape_xml(&handler)
        ));
    }
    lines.push(format!("{indent}</Events>"));
}

pub fn form_event_handler_name(element_name: &str, event_name: &str) -> String {
    let suffix = match event_name {
        "Click" => "Нажатие",
        "OnChange" => "ПриИзменении",
        "StartChoice" => "НачалоВыбора",
        "ChoiceProcessing" => "ОбработкаВыбора",
        "AutoComplete" => "АвтоПодбор",
        "Clearing" => "Очистка",
        "Opening" => "Открытие",
        "OnActivateRow" => "ПриАктивизацииСтроки",
        "BeforeAddRow" => "ПередНачаломДобавления",
        "BeforeDeleteRow" => "ПередУдалением",
        "BeforeRowChange" => "ПередНачаломИзменения",
        "OnStartEdit" => "ПриНачалеРедактирования",
        "OnEndEdit" => "ПриОкончанииРедактирования",
        "Selection" => "ВыборСтроки",
        "OnCurrentPageChange" => "ПриСменеСтраницы",
        "TextEditEnd" => "ОкончаниеВводаТекста",
        "URLProcessing" => "ОбработкаНавигационнойСсылки",
        "DragStart" => "НачалоПеретаскивания",
        "Drag" => "Перетаскивание",
        "DragCheck" => "ПроверкаПеретаскивания",
        "Drop" => "Помещение",
        "AfterDeleteRow" => "ПослеУдаления",
        _ => event_name,
    };
    format!("{element_name}{suffix}")
}

pub fn emit_form_common_flags(lines: &mut Vec<String>, element: &Map<String, Value>, indent: &str) {
    if element.get("visible").and_then(Value::as_bool) == Some(false)
        || element.get("hidden").and_then(Value::as_bool) == Some(true)
    {
        lines.push(format!("{indent}<Visible>false</Visible>"));
    }
    if element.get("userVisible").and_then(Value::as_bool) == Some(false) {
        lines.push(format!("{indent}<UserVisible>"));
        lines.push(format!("{indent}\t<xr:Common>false</xr:Common>"));
        lines.push(format!("{indent}</UserVisible>"));
    }
    if element.get("enabled").and_then(Value::as_bool) == Some(false)
        || element.get("disabled").and_then(Value::as_bool) == Some(true)
    {
        lines.push(format!("{indent}<Enabled>false</Enabled>"));
    }
    if element.get("readOnly").and_then(Value::as_bool) == Some(true) {
        lines.push(format!("{indent}<ReadOnly>true</ReadOnly>"));
    }
}

pub fn emit_form_companion(
    lines: &mut Vec<String>,
    tag: &str,
    name: &str,
    indent: &str,
    ids: &mut FormIdAllocator,
) -> Result<(), String> {
    let id = ids.allocate()?;
    lines.push(format!(
        "{indent}<{tag} name=\"{}\" id=\"{id}\"/>",
        unica_xml_format::text::escape_xml(name)
    ));
    Ok(())
}

pub fn form_compile_main_attribute_saves_data(type_name: &str) -> bool {
    [
        "CatalogObject.",
        "DocumentObject.",
        "ChartOfAccountsObject.",
        "ChartOfCalculationTypesObject.",
        "ChartOfCharacteristicTypesObject.",
        "ExchangePlanObject.",
        "BusinessProcessObject.",
        "TaskObject.",
    ]
    .iter()
    .any(|prefix| type_name.starts_with(prefix))
        || type_name.contains("RecordManager.")
}

pub fn emit_form_attributes(
    lines: &mut Vec<String>,
    attrs: Option<&Value>,
    indent: &str,
    ids: &mut FormIdAllocator,
    lang: Option<&str>,
) -> Result<(), String> {
    let Some(attrs) = attrs.and_then(Value::as_array) else {
        return Ok(());
    };
    if attrs.is_empty() {
        return Ok(());
    }
    lines.push(format!("{indent}<Attributes>"));
    for attr in attrs {
        let Some(object) = attr.as_object() else {
            continue;
        };
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "Form attribute is missing name".to_string())?;
        let attr_id = ids.allocate()?;
        lines.push(format!(
            "{indent}\t<Attribute name=\"{}\" id=\"{attr_id}\">",
            unica_xml_format::text::escape_xml(name)
        ));
        let inner = format!("{indent}\t\t");
        if let Some(title) = object.get("title").and_then(Value::as_str) {
            emit_form_mltext(lines, &inner, "Title", title, lang)?;
        }
        let type_name = object.get("type").and_then(Value::as_str);
        if let Some(type_name) = type_name {
            emit_form_type(lines, type_name, &inner)?;
        } else {
            lines.push(format!("{inner}<Type/>"));
        }
        let main_attribute = object.get("main").and_then(Value::as_bool) == Some(true);
        if main_attribute {
            lines.push(format!("{inner}<MainAttribute>true</MainAttribute>"));
        }
        let saved_data = if object.contains_key("savedData") {
            object.get("savedData").and_then(Value::as_bool) == Some(true)
        } else {
            main_attribute && type_name.is_some_and(form_compile_main_attribute_saves_data)
        };
        if saved_data {
            lines.push(format!("{inner}<SavedData>true</SavedData>"));
        }
        // 8.3.27 пишет FillCheck до Settings динамического списка.
        if let Some(fill_checking) = object.get("fillChecking").and_then(Value::as_str) {
            lines.push(format!(
                "{inner}<FillCheck>{}</FillCheck>",
                unica_xml_format::text::escape_xml(fill_checking)
            ));
        }
        if object.get("type").and_then(Value::as_str) == Some("DynamicList") {
            if let Some(settings) = object.get("settings").and_then(Value::as_object) {
                emit_form_dynamic_list_attribute_settings(lines, settings, &inner);
            }
        }
        emit_form_compile_attribute_columns(lines, object.get("columns"), &inner, ids, lang)?;
        lines.push(format!("{indent}\t</Attribute>"));
    }
    lines.push(format!("{indent}</Attributes>"));
    Ok(())
}

pub fn emit_form_attribute_columns(
    lines: &mut Vec<String>,
    columns: Option<&Value>,
    indent: &str,
    lang: Option<&str>,
) -> Result<(), String> {
    emit_form_attribute_columns_with_ids(lines, columns, indent, |index| Ok(index + 1), lang)
}

fn emit_form_compile_attribute_columns(
    lines: &mut Vec<String>,
    columns: Option<&Value>,
    indent: &str,
    ids: &mut FormIdAllocator,
    lang: Option<&str>,
) -> Result<(), String> {
    emit_form_attribute_columns_with_ids(lines, columns, indent, |_| ids.allocate(), lang)
}

fn emit_form_attribute_columns_with_ids(
    lines: &mut Vec<String>,
    columns: Option<&Value>,
    indent: &str,
    mut next_id: impl FnMut(usize) -> Result<usize, String>,
    lang: Option<&str>,
) -> Result<(), String> {
    let Some(columns) = columns else {
        return Ok(());
    };
    let columns = columns
        .as_array()
        .ok_or_else(|| "Form attribute columns must be an array".to_string())?;
    if columns.is_empty() {
        return Ok(());
    }

    lines.push(format!("{indent}<Columns>"));
    for (idx, column) in columns.iter().enumerate() {
        let object = column
            .as_object()
            .ok_or_else(|| format!("Form attribute column #{} must be an object", idx + 1))?;
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Form attribute column #{} is missing name", idx + 1))?;
        let column_indent = format!("{indent}\t");
        let id = next_id(idx)?;
        lines.push(format!(
            "{column_indent}<Column name=\"{}\" id=\"{id}\">",
            unica_xml_format::text::escape_xml(name),
        ));
        let inner = format!("{column_indent}\t");
        if let Some(title) = object.get("title").and_then(Value::as_str) {
            emit_form_mltext(lines, &inner, "Title", title, lang)?;
        }
        let type_name = object
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Form attribute column '{name}' is missing type"))?;
        emit_form_type(lines, type_name, &inner)?;
        lines.push(format!("{column_indent}</Column>"));
    }
    lines.push(format!("{indent}</Columns>"));
    Ok(())
}

pub fn emit_form_dynamic_list_attribute_settings(
    lines: &mut Vec<String>,
    settings: &Map<String, Value>,
    indent: &str,
) {
    const CANON_FILTER_ID: &str = "dfcece9d-5077-440b-b6b3-45a5cb4538eb";
    const CANON_ORDER_ID: &str = "88619765-ccb3-46c6-ac52-38e9c992ebd4";
    const CANON_CA_ID: &str = "b75fecce-942b-4aed-abc9-e6a02e460fb3";
    const CANON_ITEMS_ID: &str = "911b6018-f537-43e8-a417-da56b22f9aec";

    let manual_query = if settings.get("manualQuery").and_then(Value::as_bool) == Some(true) {
        "true"
    } else {
        "false"
    };
    let dynamic_data_read = if settings
        .get("dynamicDataRead")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "true"
    } else {
        "false"
    };

    lines.push(format!("{indent}<Settings xsi:type=\"DynamicList\">"));
    lines.push(format!(
        "{indent}\t<ManualQuery>{manual_query}</ManualQuery>"
    ));
    lines.push(format!(
        "{indent}\t<DynamicDataRead>{dynamic_data_read}</DynamicDataRead>"
    ));
    if let Some(main_table) = settings.get("mainTable").and_then(Value::as_str) {
        lines.push(format!(
            "{indent}\t<MainTable>{}</MainTable>",
            unica_xml_format::text::escape_xml(main_table)
        ));
    }
    lines.push(format!("{indent}\t<ListSettings>"));
    lines.push(format!("{indent}\t\t<dcsset:filter>"));
    lines.push(format!(
        "{indent}\t\t\t<dcsset:viewMode>Normal</dcsset:viewMode>"
    ));
    lines.push(format!(
        "{indent}\t\t\t<dcsset:userSettingID>{CANON_FILTER_ID}</dcsset:userSettingID>"
    ));
    lines.push(format!("{indent}\t\t</dcsset:filter>"));
    lines.push(format!("{indent}\t\t<dcsset:order>"));
    lines.push(format!(
        "{indent}\t\t\t<dcsset:viewMode>Normal</dcsset:viewMode>"
    ));
    lines.push(format!(
        "{indent}\t\t\t<dcsset:userSettingID>{CANON_ORDER_ID}</dcsset:userSettingID>"
    ));
    lines.push(format!("{indent}\t\t</dcsset:order>"));
    lines.push(format!("{indent}\t\t<dcsset:conditionalAppearance>"));
    lines.push(format!(
        "{indent}\t\t\t<dcsset:viewMode>Normal</dcsset:viewMode>"
    ));
    lines.push(format!(
        "{indent}\t\t\t<dcsset:userSettingID>{CANON_CA_ID}</dcsset:userSettingID>"
    ));
    lines.push(format!("{indent}\t\t</dcsset:conditionalAppearance>"));
    lines.push(format!(
        "{indent}\t\t<dcsset:itemsViewMode>Normal</dcsset:itemsViewMode>"
    ));
    lines.push(format!(
        "{indent}\t\t<dcsset:itemsUserSettingID>{CANON_ITEMS_ID}</dcsset:itemsUserSettingID>"
    ));
    lines.push(format!("{indent}\t</ListSettings>"));
    lines.push(format!("{indent}</Settings>"));
}

pub fn emit_form_parameters(
    lines: &mut Vec<String>,
    params: Option<&Value>,
    indent: &str,
) -> Result<(), String> {
    let Some(params) = params.and_then(Value::as_array) else {
        return Ok(());
    };
    if params.is_empty() {
        return Ok(());
    }
    lines.push(format!("{indent}<Parameters>"));
    for param in params {
        let Some(object) = param.as_object() else {
            continue;
        };
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "Form parameter is missing name".to_string())?;
        lines.push(format!(
            "{indent}\t<Parameter name=\"{}\">",
            unica_xml_format::text::escape_xml(name)
        ));
        let inner = format!("{indent}\t\t");
        emit_form_type(
            lines,
            object
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            &inner,
        )?;
        if object.get("key").and_then(Value::as_bool) == Some(true) {
            lines.push(format!("{inner}<KeyParameter>true</KeyParameter>"));
        }
        lines.push(format!("{indent}\t</Parameter>"));
    }
    lines.push(format!("{indent}</Parameters>"));
    Ok(())
}

pub fn emit_form_commands(
    lines: &mut Vec<String>,
    cmds: Option<&Value>,
    indent: &str,
    ids: &mut FormIdAllocator,
    lang: Option<&str>,
) -> Result<(), String> {
    let Some(cmds) = cmds.and_then(Value::as_array) else {
        return Ok(());
    };
    if cmds.is_empty() {
        return Ok(());
    }
    lines.push(format!("{indent}<Commands>"));
    for cmd in cmds {
        let Some(object) = cmd.as_object() else {
            continue;
        };
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "Form command is missing name".to_string())?;
        let cmd_id = ids.allocate()?;
        emit_form_command(lines, object, name, cmd_id, &format!("{indent}\t"), lang)?;
    }
    lines.push(format!("{indent}</Commands>"));
    Ok(())
}

pub fn emit_form_type(
    lines: &mut Vec<String>,
    type_name: &str,
    indent: &str,
) -> Result<(), String> {
    if type_name.trim().is_empty() {
        lines.push(format!("{indent}<Type/>"));
        return Ok(());
    }

    let raw_parts = type_name.split(['|', '+']).collect::<Vec<_>>();
    if raw_parts.iter().any(|part| part.trim().is_empty()) {
        return Err(format!(
            "form type '{type_name}' is not valid for 8.3.27: composite type contains an empty item"
        ));
    }
    let entries = raw_parts
        .iter()
        .map(|part| parse_form_type_entry(part.trim()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("form type '{type_name}' is not valid for 8.3.27: {error}"))?;

    let mut seen = BTreeMap::<(FormTypeNodeKind, String), &str>::new();
    for (raw, entry) in raw_parts.iter().zip(&entries) {
        let key = (entry.kind, entry.wire_name.clone());
        if let Some(previous) = seen.insert(key, raw.trim()) {
            return Err(format!(
                "form type '{type_name}' is not valid for 8.3.27: duplicate platform type '{previous}' and '{}' both map to v8:{} {}",
                raw.trim(),
                entry.kind.tag(),
                entry.wire_name
            ));
        }
    }

    // v8:TypeDescription is an XSD sequence. Validate the complete DSL value
    // before emitting anything, then serialize its repeated groups in schema
    // order instead of interleaving qualifiers with concrete types.
    lines.push(format!("{indent}<Type>"));
    let inner = format!("{indent}\t");
    for kind in [
        FormTypeNodeKind::Type,
        FormTypeNodeKind::TypeSet,
        FormTypeNodeKind::TypeId,
    ] {
        for entry in entries.iter().filter(|entry| entry.kind == kind) {
            emit_form_type_entry(lines, entry, &inner);
        }
    }
    for qualifier_rank in [0_u8, 1, 2] {
        for qualifier in entries.iter().filter_map(|entry| entry.qualifier) {
            if form_type_qualifier_rank(qualifier) == qualifier_rank {
                emit_form_type_qualifier(lines, qualifier, &inner);
            }
        }
    }
    lines.push(format!("{indent}</Type>"));
    Ok(())
}

pub fn parse_form_type_entry(type_name: &str) -> Result<FormTypeEntry, String> {
    let normalized = normalize_form_type(type_name);
    if normalized == "boolean" {
        return Ok(form_type_entry("xs:boolean", None));
    }
    if normalized == "string" {
        return Ok(form_type_qualified_entry(
            "xs:string",
            FormTypeQualifier::String {
                length: 0,
                fixed: false,
            },
        ));
    }
    if normalized.starts_with("string(") {
        let (length, fixed) = unica_metadata::type_contract::parse_string_contract(&normalized).ok_or_else(|| {
            format!(
                "type '{type_name}' must be string(integer length 0..=1024[,fixed|variable]); fixed requires length > 0"
            )
        })?;
        return Ok(form_type_qualified_entry(
            "xs:string",
            FormTypeQualifier::String { length, fixed },
        ));
    }
    if normalized == "decimal" {
        return Ok(form_type_qualified_entry(
            "xs:decimal",
            FormTypeQualifier::Number {
                digits: 10,
                fraction: 0,
                nonnegative: false,
            },
        ));
    }
    if normalized.starts_with("decimal(") {
        let (digits, fraction, nonnegative) =
            unica_metadata::type_contract::parse_decimal_contract(&normalized).ok_or_else(|| {
                format!(
                    "type '{type_name}' must be decimal(integer digits 0..=38, integer fraction 0..=digits[,nonneg])"
                )
            })?;
        return Ok(form_type_qualified_entry(
            "xs:decimal",
            FormTypeQualifier::Number {
                digits,
                fraction,
                nonnegative,
            },
        ));
    }
    if matches!(normalized.as_str(), "date" | "dateTime" | "time") {
        let fractions = match normalized.as_str() {
            "date" => "Date",
            "dateTime" => "DateTime",
            "time" => "Time",
            _ => unreachable!(),
        };
        return Ok(form_type_qualified_entry(
            "xs:dateTime",
            FormTypeQualifier::Date(fractions),
        ));
    }
    if let Some(type_id) = normalized.strip_prefix("typeid:") {
        if !unica_xml_format::names::is_valid_uuid(type_id) {
            return Err(format!("type '{type_name}' has an invalid TypeId UUID"));
        }
        return Ok(FormTypeEntry {
            kind: FormTypeNodeKind::TypeId,
            wire_name: type_id.to_string(),
            local_namespace: None,
            qualifier: None,
        });
    }

    let mapped = match normalized.as_str() {
        "ValueTable" => Some("v8:ValueTable"),
        "ValueTree" => Some("v8:ValueTree"),
        "ValueList" => Some("v8:ValueListType"),
        "TypeDescription" => Some("v8:TypeDescription"),
        "Universal" => Some("v8:Universal"),
        "FixedArray" => Some("v8:FixedArray"),
        "FixedStructure" => Some("v8:FixedStructure"),
        "FormattedString" => Some("v8ui:FormattedString"),
        "Picture" => Some("v8ui:Picture"),
        "Color" => Some("v8ui:Color"),
        "Font" => Some("v8ui:Font"),
        "DataCompositionSettings" | "DCS.DataCompositionSettings" => {
            Some("dcsset:DataCompositionSettings")
        }
        "DataCompositionSchema" | "DCS.DataCompositionSchema" => {
            Some("dcssch:DataCompositionSchema")
        }
        "DataCompositionComparisonType" | "DCS.DataCompositionComparisonType" => {
            Some("dcscor:DataCompositionComparisonType")
        }
        _ => None,
    };
    if let Some(mapped) = mapped {
        return Ok(form_type_entry(mapped, None));
    }

    if matches!(
        normalized.as_str(),
        "DynamicList" | "ConstantsSet" | "ReportObject"
    ) {
        return Ok(form_type_entry(&format!("cfg:{normalized}"), None));
    }
    if normalized.starts_with("DefinedType.") || normalized.starts_with("Characteristic.") {
        validate_form_configuration_type_name(type_name, &normalized)?;
        return Ok(FormTypeEntry {
            kind: FormTypeNodeKind::TypeSet,
            wire_name: format!("cfg:{normalized}"),
            local_namespace: None,
            qualifier: None,
        });
    }
    if form_type_set_names().contains(&normalized.as_str()) {
        return Ok(FormTypeEntry {
            kind: FormTypeNodeKind::TypeSet,
            wire_name: format!("cfg:{normalized}"),
            local_namespace: None,
            qualifier: None,
        });
    }
    if let Some((prefix, _)) = normalized.split_once('.') {
        if unica_metadata::type_contract::valid_cfg_prefixes().contains(&prefix) {
            validate_form_configuration_type_name(type_name, &normalized)?;
            return Ok(form_type_entry(&format!("cfg:{normalized}"), None));
        }
    }
    if let Some((prefix, namespace)) = form_special_type_namespace(&normalized) {
        return Ok(form_type_entry(&normalized, Some((prefix, namespace))));
    }
    if form_valid_closed_types().contains(&normalized.as_str()) {
        return Ok(form_type_entry(&normalized, None));
    }
    if form_invalid_types().contains(&normalized.as_str()) {
        return Err(format!(
            "type '{type_name}' is a runtime/UI type, not a data type in the 8.3.27 XDTO contract"
        ));
    }
    Err(format!(
        "type '{type_name}' is not supported by the fixed 8.3.27 form type contract"
    ))
}

fn form_type_entry(
    wire_name: &str,
    local_namespace: Option<(&'static str, &'static str)>,
) -> FormTypeEntry {
    FormTypeEntry {
        kind: FormTypeNodeKind::Type,
        wire_name: wire_name.to_string(),
        local_namespace,
        qualifier: None,
    }
}

fn form_type_qualified_entry(wire_name: &str, qualifier: FormTypeQualifier) -> FormTypeEntry {
    FormTypeEntry {
        qualifier: Some(qualifier),
        ..form_type_entry(wire_name, None)
    }
}

fn emit_form_type_entry(lines: &mut Vec<String>, entry: &FormTypeEntry, indent: &str) {
    let tag = entry.kind.tag();
    if let Some((prefix, namespace)) = entry.local_namespace {
        lines.push(format!(
            "{indent}<v8:{tag} xmlns:{prefix}=\"{namespace}\">{}</v8:{tag}>",
            unica_xml_format::text::escape_xml(&entry.wire_name)
        ));
    } else {
        lines.push(format!(
            "{indent}<v8:{tag}>{}</v8:{tag}>",
            unica_xml_format::text::escape_xml(&entry.wire_name)
        ));
    }
}

fn form_type_qualifier_rank(qualifier: FormTypeQualifier) -> u8 {
    match qualifier {
        FormTypeQualifier::Number { .. } => 0,
        FormTypeQualifier::String { .. } => 1,
        FormTypeQualifier::Date(_) => 2,
    }
}

fn emit_form_type_qualifier(lines: &mut Vec<String>, qualifier: FormTypeQualifier, indent: &str) {
    match qualifier {
        FormTypeQualifier::Number {
            digits,
            fraction,
            nonnegative,
        } => {
            lines.push(format!("{indent}<v8:NumberQualifiers>"));
            lines.push(format!("{indent}\t<v8:Digits>{digits}</v8:Digits>"));
            lines.push(format!(
                "{indent}\t<v8:FractionDigits>{fraction}</v8:FractionDigits>"
            ));
            lines.push(format!(
                "{indent}\t<v8:AllowedSign>{}</v8:AllowedSign>",
                if nonnegative { "Nonnegative" } else { "Any" }
            ));
            lines.push(format!("{indent}</v8:NumberQualifiers>"));
        }
        FormTypeQualifier::String { length, fixed } => {
            lines.push(format!("{indent}<v8:StringQualifiers>"));
            lines.push(format!("{indent}\t<v8:Length>{length}</v8:Length>"));
            lines.push(format!(
                "{indent}\t<v8:AllowedLength>{}</v8:AllowedLength>",
                if fixed { "Fixed" } else { "Variable" }
            ));
            lines.push(format!("{indent}</v8:StringQualifiers>"));
        }
        FormTypeQualifier::Date(fractions) => {
            lines.push(format!("{indent}<v8:DateQualifiers>"));
            lines.push(format!(
                "{indent}\t<v8:DateFractions>{fractions}</v8:DateFractions>"
            ));
            lines.push(format!("{indent}</v8:DateQualifiers>"));
        }
    }
}

fn validate_form_configuration_type_name(raw: &str, normalized: &str) -> Result<(), String> {
    let invalid_name = normalized
        .split_once('.')
        .is_none_or(|(_, name)| name.trim().is_empty() || name.contains('.'));
    if invalid_name || !unica_xml_format::names::xml_ncname_is_valid(normalized) {
        return Err(format!(
            "type '{raw}' has an invalid or empty configuration type name"
        ));
    }
    Ok(())
}

pub fn form_type_set_names() -> &'static [&'static str] {
    &[
        "AnyRef",
        "AnyIBRef",
        "CatalogRef",
        "DocumentRef",
        "EnumRef",
        "ExchangePlanRef",
        "TaskRef",
        "BusinessProcessRef",
        "ChartOfAccountsRef",
        "ChartOfCharacteristicTypesRef",
        "ChartOfCalculationTypesRef",
    ]
}

pub fn form_special_type_namespace(value: &str) -> Option<(&'static str, &'static str)> {
    match value {
        "mxl:SpreadsheetDocument" => Some(("mxl", "http://v8.1c.ru/8.2/data/spreadsheet")),
        "fd:FormattedDocument" => Some(("fd", "http://v8.1c.ru/8.2/data/formatted-document")),
        "d5p1:TextDocument" => Some(("d5p1", "http://v8.1c.ru/8.1/data/txtedt")),
        "d5p1:Chart" | "d5p1:GanttChart" | "d5p1:Dendrogram" => {
            Some(("d5p1", "http://v8.1c.ru/8.2/data/chart"))
        }
        "d5p1:FlowchartContextType" => Some(("d5p1", "http://v8.1c.ru/8.2/data/graphscheme")),
        "d5p1:DataAnalysisTimeIntervalUnitType" => {
            Some(("d5p1", "http://v8.1c.ru/8.2/data/data-analysis"))
        }
        "d5p1:GeographicalSchema" => Some(("d5p1", "http://v8.1c.ru/8.2/data/geo")),
        "pdfdoc:PDFDocument" => Some(("pdfdoc", "http://v8.1c.ru/8.3/data/pdf")),
        "pl:Planner" => Some(("pl", "http://v8.1c.ru/8.3/data/planner")),
        _ => None,
    }
}

pub fn normalize_form_type(type_name: &str) -> String {
    let stripped = type_name.strip_prefix("cfg:").unwrap_or(type_name);
    if let Some(open) = stripped.find('(') {
        if stripped.ends_with(')') {
            let base = stripped[..open].trim();
            let params = &stripped[open + 1..stripped.len() - 1];
            let normalized = normalize_form_type_base(base).unwrap_or(base);
            return format!("{normalized}({params})");
        }
    }
    if let Some(dot) = stripped.find('.') {
        let prefix = &stripped[..dot];
        let suffix = &stripped[dot..];
        if let Some(normalized) = normalize_form_type_base(prefix) {
            return format!("{normalized}{suffix}");
        }
    }
    normalize_form_type_base(stripped)
        .unwrap_or(stripped)
        .to_string()
}

pub fn normalize_form_type_base(base: &str) -> Option<&'static str> {
    match base.to_lowercase().as_str() {
        "строка" | "string" => Some("string"),
        "число" | "number" => Some("decimal"),
        "булево" | "boolean" | "bool" => Some("boolean"),
        "дата" | "date" => Some("date"),
        "датавремя" | "datetime" => Some("dateTime"),
        "время" | "time" => Some("time"),
        "binary" | "xs:binary" | "xs:base64binary" => Some("xs:base64Binary"),
        "справочникссылка" | "catalogref" => Some("CatalogRef"),
        "справочникобъект" | "catalogobject" => Some("CatalogObject"),
        "документссылка" | "documentref" => Some("DocumentRef"),
        "документобъект" | "documentobject" => Some("DocumentObject"),
        "перечислениессылка" | "enumref" => Some("EnumRef"),
        "плансчетовссылка" | "chartofaccountsref" => Some("ChartOfAccountsRef"),
        "планвидовхарактеристикссылка" | "chartofcharacteristictypesref" => {
            Some("ChartOfCharacteristicTypesRef")
        }
        "планвидоврасчётассылка" | "планвидоврасчетассылка" | "chartofcalculationtypesref" => {
            Some("ChartOfCalculationTypesRef")
        }
        "планобменассылка" | "exchangeplanref" => Some("ExchangePlanRef"),
        "бизнеспроцессссылка" | "businessprocessref" => {
            Some("BusinessProcessRef")
        }
        "задачассылка" | "taskref" => Some("TaskRef"),
        "определяемыйтип" | "definedtype" => Some("DefinedType"),
        "характеристика" | "characteristic" => Some("Characteristic"),
        "любаяссылка" | "anyref" => Some("AnyRef"),
        "любаяссылкаиб" | "anyibref" => Some("AnyIBRef"),
        "таблицазначений" | "valuetable" => Some("ValueTable"),
        "деревозначений" | "valuetree" => Some("ValueTree"),
        "списокзначений" | "valuelist" => Some("ValueList"),
        "описаниетипов" | "typedescription" => Some("TypeDescription"),
        "formattedstring" => Some("FormattedString"),
        "picture" => Some("Picture"),
        "color" => Some("Color"),
        "font" => Some("Font"),
        "standardperiod" | "стандартныйпериод" | "v8:standardperiod" => {
            Some("v8:StandardPeriod")
        }
        "standardbeginningdate" | "стандартнаядатаначала" | "v8:standardbeginningdate" => {
            Some("v8:StandardBeginningDate")
        }
        "uuid" | "уникальныйидентификатор" | "v8:uuid" => Some("v8:UUID"),
        _ => None,
    }
}

pub fn plan_form_registration_in_parent_object(
    output_path: &Path,
    object_xml_path: &Path,
    source: &Utf8TextSnapshot,
) -> Result<Option<FormParentRegistrationPlan>, String> {
    let Some(form_xml_dir) = output_path.parent() else {
        return Ok(None);
    };
    let Some(form_name_dir) = form_xml_dir.parent() else {
        return Ok(None);
    };
    let Some(forms_dir) = form_name_dir.parent() else {
        return Ok(None);
    };
    let Some(object_dir) = forms_dir.parent() else {
        return Ok(None);
    };
    let Some(form_name) = form_name_dir.file_name().and_then(|value| value.to_str()) else {
        return Ok(None);
    };
    let Some(object_name) = object_dir.file_name().and_then(|value| value.to_str()) else {
        return Ok(None);
    };
    let escaped_form_name = unica_xml_format::text::escape_xml(form_name);
    if source
        .text
        .contains(&format!("<Form>{escaped_form_name}</Form>"))
    {
        return Ok(None);
    }
    let replacement_text = register_form_in_object_text(&source.text, &escaped_form_name);
    if replacement_text == source.text {
        return Ok(None);
    }
    let replacement_text = preserve_source_final_newline(replacement_text, &source.text);
    Ok(Some(FormParentRegistrationPlan {
        path: object_xml_path.to_path_buf(),
        original: source.raw.clone(),
        replacement: utf8_bom_bytes(&replacement_text),
        stdout: format!("     Registered: <Form>{escaped_form_name}</Form> in {object_name}.xml\n"),
    }))
}

pub fn form_parent_metadata_owner_candidate(output_path: &Path) -> Result<Option<PathBuf>, String> {
    let Some(form_xml_dir) = output_path.parent() else {
        return Ok(None);
    };
    let Some(form_name_dir) = form_xml_dir.parent() else {
        return Ok(None);
    };
    let Some(forms_dir) = form_name_dir.parent() else {
        return Ok(None);
    };
    let Some(object_dir) = forms_dir.parent() else {
        return Ok(None);
    };
    let Some(type_plural_dir) = object_dir.parent() else {
        return Ok(None);
    };
    if forms_dir.file_name().and_then(|value| value.to_str()) != Some("Forms") {
        return Ok(None);
    }
    let Some(form_name) = form_name_dir.file_name().and_then(|value| value.to_str()) else {
        return Ok(None);
    };
    validate_form_metadata_path_name("OutputPath form name", form_name)?;
    let Some(object_name) = object_dir.file_name().and_then(|value| value.to_str()) else {
        return Ok(None);
    };
    let object_xml_path = type_plural_dir.join(format!("{object_name}.xml"));
    Ok(Some(object_xml_path))
}

pub fn validate_form_image(
    args: &Map<String, Value>,
    reader: &dyn ProjectDiscoveryRead,
    form_path: &Path,
    text: &str,
    signatures: HandlerSignatures,
) -> Result<FormValidationRun, String> {
    validate_form_image_inner(
        args,
        form_path,
        text,
        form_metadata_owner(form_path),
        (signatures == HandlerSignatures::Check)
            .then(|| HandlerSignatureCheck::new(reader, form_path)),
        || form_is_config_context(reader, form_path),
    )
}

/// Факты владельца и модуля, полученные через доказанное право чтения.
pub struct FormValidationContext<'a> {
    pub config_context: bool,
    pub metadata_owner: Option<unica_address::address::NodeKind>,
    pub module: Option<&'a str>,
}

/// Проверяет сохранённые исходники; путь служит только для имени результата.
pub fn validate_form_image_with_context(
    args: &Map<String, Value>,
    form_path: &Path,
    text: &str,
    context: FormValidationContext<'_>,
) -> Result<FormValidationRun, String> {
    validate_form_image_inner(
        args,
        form_path,
        text,
        context.metadata_owner,
        Some(HandlerSignatureCheck::from_source(
            context.module,
            context.metadata_owner,
        )),
        || context.config_context,
    )
}

fn validate_form_image_inner(
    args: &Map<String, Value>,
    form_path: &Path,
    text: &str,
    metadata_owner: Option<unica_address::address::NodeKind>,
    signature_check: Option<HandlerSignatureCheck<'_>>,
    config_context: impl FnOnce() -> bool,
) -> Result<FormValidationRun, String> {
    let form_path = form_path.to_path_buf();
    let detailed = unica_operation_contract::input::bool_arg(args, &["detailed", "Detailed"]);
    let max_errors = unica_operation_contract::input::int_arg(args, &["maxErrors", "MaxErrors"])
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or(30);
    let form_name = form_validation_name(&form_path);

    let source = text.trim_start_matches('\u{feff}');
    let doc = match Document::parse(source) {
        Ok(doc) => doc,
        Err(err) => {
            let stdout = format!("[ERROR] XML parse error: {err}\n\n---\nErrors: 1, Warnings: 0\n");
            return Ok((
                false,
                stdout,
                form_path,
                vec![format!("[ERROR] XML parse error: {err}")],
                Vec::new(),
            ));
        }
    };
    let root = doc.root_element();
    let mut report = FormValidationReporter::new(&form_name, max_errors, detailed);

    if let Err(error) = require_form_root(root) {
        report.error(error);
        let (ok, stdout, errors) = report.finalize(&form_name);
        return Ok((ok, stdout, form_path, errors, Vec::new()));
    }

    let has_base_form = form_validation_child(root, "BaseForm").is_some();
    let version_literal = root_version_literal(source, root);
    match classify_root_version(version_literal.as_deref()) {
        Ok(FormatCompatibility::Supported { .. }) => report.ok("Export format: 2.20"),
        Ok(compatibility) => report.warn(format_compatibility_warning(&compatibility)),
        Err(error) => report.error(error.to_string()),
    }

    if !report.stopped {
        if let Some(acb) = form_validation_child(root, "AutoCommandBar") {
            let acb_name = acb.attribute("name").unwrap_or("");
            let acb_id = acb.attribute("id").unwrap_or("");
            if acb_id == "-1" {
                report.ok(format!("AutoCommandBar: name='{acb_name}', id={acb_id}"));
            } else {
                report.error(format!("AutoCommandBar id='{acb_id}', expected '-1'"));
            }
        } else {
            report.error("AutoCommandBar element missing");
        }
    }

    let mut elements = Vec::new();
    let mut element_ids = HashMap::<String, String>::new();
    let mut element_names = HashMap::<String, String>::new();
    if let Some(child_items) = form_validation_child(root, "ChildItems") {
        form_collect_elements(
            child_items,
            &mut elements,
            &mut element_ids,
            &mut element_names,
            &mut report,
        );
    }
    if let Some(acb_children) = form_validation_child(root, "AutoCommandBar")
        .and_then(|acb| form_validation_child(acb, "ChildItems"))
    {
        form_collect_elements(
            acb_children,
            &mut elements,
            &mut element_ids,
            &mut element_names,
            &mut report,
        );
    }

    if !report.stopped {
        let mut id_counts = HashMap::<String, usize>::new();
        for element in &elements {
            if element.id == "-1" {
                continue;
            }
            *id_counts.entry(element.id.clone()).or_default() += 1;
        }
        if id_counts.values().all(|count| *count <= 1) {
            report.ok(format!(
                "Unique element IDs: {} elements",
                element_ids.len()
            ));
        }
    }

    let attr_nodes = form_validation_child(root, "Attributes")
        .map(|attrs| form_validation_children(attrs, "Attribute"))
        .unwrap_or_default();
    let mut attr_map = HashMap::<String, roxmltree::Node<'_, '_>>::new();
    let mut attr_ids = HashMap::<String, String>::new();
    for attr in &attr_nodes {
        let attr_name = attr.attribute("name").unwrap_or("");
        let attr_id = attr.attribute("id").unwrap_or("");
        if !attr_name.is_empty() {
            if let Some(existing) = attr_map.get(attr_name) {
                report.error(format!(
                    "Duplicate attribute name '{attr_name}': id={attr_id} and id={}",
                    existing.attribute("id").unwrap_or("")
                ));
            }
            attr_map.insert(attr_name.to_string(), *attr);
        }
        if !attr_id.is_empty() {
            if let Some(existing) = attr_ids.get(attr_id) {
                report.error(format!(
                    "Duplicate attribute id={attr_id}: '{attr_name}' and '{existing}'"
                ));
            } else {
                attr_ids.insert(attr_id.to_string(), attr_name.to_string());
            }
        }

        if let Some(columns) = form_validation_child(*attr, "Columns") {
            let mut col_ids = HashMap::<String, String>::new();
            let mut col_names = HashMap::<String, String>::new();
            for column in form_validation_children(columns, "Column") {
                let col_id = column.attribute("id").unwrap_or("");
                let col_name = column.attribute("name").unwrap_or("");
                if !col_id.is_empty() {
                    if let Some(existing) = col_ids.get(col_id) {
                        report.error(format!(
                                "Duplicate column id={col_id} in '{attr_name}': '{col_name}' and '{existing}'"
                            ));
                    } else {
                        col_ids.insert(col_id.to_string(), col_name.to_string());
                    }
                }
                if !col_name.is_empty() {
                    if let Some(existing) = col_names.get(col_name) {
                        report.error(format!(
                                "Duplicate column name '{col_name}' in '{attr_name}': id={col_id} and id={existing}"
                            ));
                    } else {
                        col_names.insert(col_name.to_string(), col_id.to_string());
                    }
                }
            }
        }
    }
    if !report.stopped && !attr_ids.is_empty() {
        report.ok(format!("Unique attribute IDs: {} entries", attr_ids.len()));
    }

    let cmd_nodes = form_validation_child(root, "Commands")
        .map(|commands| form_validation_children(commands, "Command"))
        .unwrap_or_default();
    let mut cmd_map = HashMap::<String, roxmltree::Node<'_, '_>>::new();
    let mut cmd_ids = HashMap::<String, String>::new();
    for cmd in &cmd_nodes {
        let cmd_name = cmd.attribute("name").unwrap_or("");
        let cmd_id = cmd.attribute("id").unwrap_or("");
        if !cmd_name.is_empty() {
            if let Some(existing) = cmd_map.get(cmd_name) {
                report.error(format!(
                    "Duplicate command name '{cmd_name}': id={cmd_id} and id={}",
                    existing.attribute("id").unwrap_or("")
                ));
            }
            cmd_map.insert(cmd_name.to_string(), *cmd);
        }
        if !cmd_id.is_empty() {
            if let Some(existing) = cmd_ids.get(cmd_id) {
                report.error(format!(
                    "Duplicate command id={cmd_id}: '{cmd_name}' and '{existing}'"
                ));
            } else {
                cmd_ids.insert(cmd_id.to_string(), cmd_name.to_string());
            }
        }
        // The platform loads a command `Representation` only from its own
        // `DefaultRepresentation` set; the button values break the load.
        // An empty element is not `Auto`: it is outside the set as well.
        if let Some(element) = form_validation_child(*cmd, "Representation") {
            let representation = form_ml_text(element);
            let representation = representation.trim();
            if form_8_3_27_enum_values("DefaultRepresentation")
                .is_some_and(|allowed| !allowed.contains(&representation))
            {
                report.error(form_command_representation_error(cmd_name, representation));
            }
        }
    }
    if !report.stopped && !cmd_ids.is_empty() {
        report.ok(format!("Unique command IDs: {} entries", cmd_ids.len()));
    }

    if !report.stopped {
        let mut param_names = HashSet::<String>::new();
        if let Some(params) = form_validation_child(root, "Parameters") {
            for param in form_validation_children(params, "Parameter") {
                let param_name = param.attribute("name").unwrap_or("");
                if !param_name.is_empty() && !param_names.insert(param_name.to_string()) {
                    report.error(format!("Duplicate parameter name '{param_name}'"));
                }
            }
        }
    }

    if !report.stopped {
        form_validate_companions(&elements, &mut report);
    }
    if !report.stopped {
        form_validate_data_paths(&elements, &attr_map, has_base_form, &mut report);
    }
    if !report.stopped {
        form_validate_button_commands(&elements, &cmd_map, &mut report);
    }
    if !report.stopped {
        form_validate_events(root, signature_check, metadata_owner, &mut report);
    }
    if !report.stopped {
        form_validate_command_actions(&cmd_nodes, &mut report);
    }
    if !report.stopped {
        let main_count = attr_nodes
            .iter()
            .filter(|attr| {
                form_validation_child_text(**attr, "MainAttribute").as_deref() == Some("true")
            })
            .count();
        if main_count <= 1 {
            let main_info = if main_count == 1 {
                "1 main attribute"
            } else {
                "no main attribute"
            };
            report.ok(format!("MainAttribute: {main_info}"));
        } else {
            report.error(format!(
                "Multiple MainAttribute=true ({main_count} found, expected 0 or 1)"
            ));
        }
    }
    if !report.stopped {
        if let Some(title) = form_validation_child(root, "Title") {
            let v8_items = form_children_in_ns(title, "item", FORM_V8_NS);
            if v8_items.is_empty() && !title.text().unwrap_or("").trim().is_empty() {
                report.error(format!(
                        "Form Title is plain text ('{}') — must be multilingual XML (<v8:item>). Use top-level 'title' key in form-compile DSL.",
                        title.text().unwrap_or("").trim()
                    ));
            } else {
                report.ok("Title: multilingual XML");
            }
        }
    }
    if !report.stopped && has_base_form {
        form_validate_extension(root, &elements, &attr_nodes, &cmd_nodes, &mut report);
    }
    if !report.stopped && !has_base_form && form_has_call_type(root, &cmd_nodes) {
        report.warn("callType attributes found but no BaseForm — possible incorrect structure");
    }
    if !report.stopped {
        form_validate_types(root, config_context(), &mut report);
    }

    let warnings = report.surfaced_warnings.clone();
    let (ok, stdout, errors) = report.finalize(&form_name);
    Ok((ok, stdout, form_path, errors, warnings))
}

pub fn form_edit_encoded_bytes(xml_text: &str, bom: Utf8Bom) -> Vec<u8> {
    let bom_len = if bom == Utf8Bom::Present { 3 } else { 0 };
    let mut bytes = Vec::with_capacity(xml_text.len() + bom_len);
    if bom == Utf8Bom::Present {
        bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
    }
    bytes.extend_from_slice(xml_text.as_bytes());
    bytes
}
pub fn form_edit_decode_image(
    original_bytes: &[u8],
    form_path: &Path,
) -> Result<(String, Utf8Bom, usize), String> {
    let bom = if original_bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        Utf8Bom::Present
    } else {
        Utf8Bom::Absent
    };
    let content_bytes = if bom == Utf8Bom::Present {
        &original_bytes[3..]
    } else {
        original_bytes
    };
    let xml_text = String::from_utf8(content_bytes.to_vec())
        .map_err(|err| format!("{} is not valid UTF-8: {err}", form_path.display()))?;
    let form_root_start = {
        let document =
            Document::parse(&xml_text).map_err(|err| format!("[ERROR] XML parse error: {err}"))?;
        let root = document.root_element();
        require_form_root(root).map_err(|error| format!("[ERROR] {error}"))?;
        root.range().start
    };

    Ok((xml_text, bom, form_root_start))
}

pub fn detect_form_add_object(object_text: &str) -> Result<(String, String), String> {
    let supported = form_add_supported_object_types();
    let doc = Document::parse(object_text)
        .map_err(|err| format!("XML parse error in object XML: {err}"))?;
    for node in doc.descendants().filter(roxmltree::Node::is_element) {
        let object_type = node.tag_name().name();
        if !supported.contains(&object_type) {
            continue;
        }
        let Some(props) = unica_xml_format::read::child_element(node, "Properties") else {
            continue;
        };
        let Some(object_name) = unica_xml_format::read::child_direct_text(props, "Name") else {
            continue;
        };
        if !object_name.is_empty() {
            return Ok((object_type.to_string(), object_name));
        }
    }
    Err(format!(
        "Не удалось определить тип объекта. Поддерживаемые типы: {}",
        supported.join(", ")
    ))
}
pub fn validate_form_purpose(object_type: &str, purpose: &str) -> Result<(), String> {
    const VALID: &[&str] = &["Object", "List", "Choice", "Record"];
    if !VALID.contains(&purpose) {
        return Err(format!(
            "Недопустимое назначение: {purpose}. Допустимые: Object, List, Choice, Record"
        ));
    }
    if purpose == "List" && object_type == "DataProcessor" {
        return Err("Purpose=List недопустим для DataProcessor".to_string());
    }
    if purpose == "Choice"
        && (form_add_processor_like(object_type) || object_type == "InformationRegister")
    {
        return Err(format!("Purpose=Choice недопустим для {object_type}"));
    }
    if purpose == "Record" && object_type != "InformationRegister" {
        return Err("Purpose=Record допустим только для InformationRegister".to_string());
    }
    Ok(())
}
/// Registers `<Form>form_name</Form>` in the owner's top-level `ChildObjects`
/// in the platform slot of forms (#1385): before the first entry of a kind the
/// export writes after forms, else after the last entry. An owner without a
/// known platform order keeps forms before its first template.
pub fn register_form_in_object_text(text: &str, form_name: &str) -> String {
    let entry = format!("\t\t\t<Form>{form_name}</Form>\n");
    if let Some(offset) =
        unica_source_designer::registration::platform_child_object_insertion_offset(text, "Form")
            .or_else(|| {
                unica_source_designer::registration::platform_child_object_order(&top_object_kind(
                    text,
                )?)
                .is_none()
                .then(|| {
                    unica_source_designer::registration::first_top_child_object_offset(
                        text, "Template",
                    )
                })
                .flatten()
            })
    {
        let mut updated = text.to_string();
        updated.insert_str(offset, &entry);
        return updated;
    }
    if let Some(close) = top_child_objects_close(text) {
        let line_start = text[..close].rfind('\n').map_or(0, |index| index + 1);
        let mut updated = text.to_string();
        if text[line_start..close]
            .chars()
            .all(|ch| ch == '\t' || ch == ' ')
        {
            updated.insert_str(line_start, &entry);
        } else {
            updated.insert_str(close, &format!("\n{entry}\t\t"));
        }
        return updated;
    }
    if text.contains("<ChildObjects/>") {
        return text.replacen(
            "<ChildObjects/>",
            &format!("<ChildObjects>\n{entry}\t\t</ChildObjects>"),
            1,
        );
    }
    text.to_string()
}
fn top_object_kind(text: &str) -> Option<String> {
    let doc = Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
    let object = doc
        .root_element()
        .children()
        .find(|node| node.is_element())?;
    Some(object.tag_name().name().to_string())
}
/// Byte offset of the closing tag of the metadata object's own
/// `ChildObjects`; a nested tabular-section block is not it.
fn top_child_objects_close(text: &str) -> Option<usize> {
    let body = text.trim_start_matches('\u{feff}');
    let shift = text.len() - body.len();
    let doc = Document::parse(body).ok()?;
    let object = doc
        .root_element()
        .children()
        .find(|node| node.is_element())?;
    let child_objects = object
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "ChildObjects")?;
    let range = child_objects.range();
    let block = &body[range.clone()];
    if block.ends_with("/>") {
        return None;
    }
    Some(shift + range.start + block.rfind("</")?)
}

#[cfg(test)]
#[path = "../tests/platform/form_codec.rs"]
mod tests;

/// Existing edit/planner verdict over the actual validator report, before terminal projection.
pub fn form_validation_require_valid(
    result: Result<FormValidationRun, String>,
) -> Result<(), String> {
    match result {
        Ok((true, _, _, _, _)) => Ok(()),
        Ok((false, stdout, _, errors, _)) => {
            let details = if errors.is_empty() {
                stdout
            } else {
                errors.join("; ")
            };
            Err(format!("form validation failed: {details}"))
        }
        Err(error) => Err(format!("form validation failed: {error}")),
    }
}
