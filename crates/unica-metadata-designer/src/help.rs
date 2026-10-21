//! Embedded-help facet images and planned reads; publication remains outside the codec.
use roxmltree::Document;

pub fn help_metadata_xml(lang: &str, format_version: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<Help xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"{}\">\n\
\t<Page>{}</Page>\n\
</Help>",
        unica_xml_format::text::escape_xml(format_version),
        unica_xml_format::text::escape_xml(lang)
    )
}

pub fn help_page_html(object_name: &str) -> String {
    format!(
        "<!DOCTYPE html PUBLIC \"-//W3C//DTD HTML 4.0 Transitional//EN\">\
<html><head>\
<meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\"></meta>\
<link rel=\"stylesheet\" type=\"text/css\" href=\"v8help://service_book/service_style\"></link>\
</head><body>\n    <h1>{}</h1>\n    <p>Описание.</p>\n</body></html>",
        unica_xml_format::text::escape_xml(object_name)
    )
}

pub fn validate_help_xml(path: &Path, text: &str) -> Result<(), String> {
    Document::parse(text.trim_start_matches('\u{feff}'))
        .map(|_| ())
        .map_err(|error| format!("XML parse error in {}: {error}", path.display()))
}

pub fn validate_help_form_owner_8_3_27(path: &Path, text: &str) -> Result<(), String> {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

    let document = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("XML parse error in {}: {error}", path.display()))?;
    let root = document.root_element();
    if root.tag_name().name() != "MetaDataObject" || root.tag_name().namespace() != Some(MD_NS) {
        return Err(format!(
            "help.add form owner {} must have the fixed 8.3.27 MetaDataObject root",
            path.display()
        ));
    }
    if root.attribute("version") != Some(ACTIVE_FORMAT_PROFILE.export_format) {
        return Err(format!(
            "help.add form owner {} must use export format {}",
            path.display(),
            ACTIVE_FORMAT_PROFILE.export_format
        ));
    }

    let forms = root
        .children()
        .filter(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_NS)
                && node.tag_name().name() == "Form"
        })
        .collect::<Vec<_>>();
    if forms.len() != 1 {
        return Err(format!(
            "help.add form owner {} must contain exactly one MDClasses Form element",
            path.display()
        ));
    }
    let form = forms[0];
    let properties = form
        .children()
        .find(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_NS)
                && node.tag_name().name() == "Properties"
        })
        .ok_or_else(|| {
            format!(
                "help.add form owner {} is missing Form/Properties",
                path.display()
            )
        })?;
    let property = |name: &str| {
        properties.children().find(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_NS)
                && node.tag_name().name() == name
        })
    };

    let form_type = property("FormType")
        .and_then(|node| node.text())
        .unwrap_or("");
    if !matches!(form_type, "Managed" | "Ordinary") {
        return Err(format!(
            "help.add form owner {} property Form.FormType value '{form_type}' is not valid for the fixed 8.3.27 contract; expected Managed or Ordinary",
            path.display()
        ));
    }
    if let Some(include_help) = property("IncludeHelpInContents") {
        let value = include_help.text().unwrap_or("");
        if !matches!(value, "true" | "false") {
            return Err(format!(
                "help.add form owner {} property Form.IncludeHelpInContents value '{value}' is not a canonical xs:boolean for the fixed 8.3.27 contract; expected true or false",
                path.display()
            ));
        }
    }

    Ok(())
}

pub fn form_with_include_help(text: &str) -> Option<String> {
    if text.contains("<IncludeHelpInContents>") {
        return None;
    }
    let insert_at = text
        .find("</FormType>")
        .map(|index| index + "</FormType>".len())?;
    let mut updated = text.to_string();
    updated.insert_str(
        insert_at,
        "\n\t\t\t<IncludeHelpInContents>false</IncludeHelpInContents>",
    );
    if !updated.ends_with('\n') {
        updated.push('\n');
    }
    Some(updated)
}

use crate::resource_plan::{TypedChildFileMutation, TypedChildResourcePlan};
// ADR-0072: планировщик фасета встроенной справки. `addHelp` материализует
// `Ext/Help.xml`, `Ext/Help/<lang>.html` владельца и флаг
// `IncludeHelpInContents` его форм через тот же приватный post-image план,
// которым пользуется каждая типизированная операция: ни один файл не
// стейджится до успешного построения полного плана.

use unica_metadata::{
    MetaDiagnostic, MetaDiagnosticCode, MetaEditOperation, MetaPublicationAction,
    MetaPublicationPlanEntry, MetaPublicationResource,
};
use unica_source_contract::discovery::ProjectDiscoveryRead;
use unica_source_designer::read::{path_exists, path_is_dir, read_utf8_sig_snapshot};

use std::path::Path;
use unica_address::source_target::MetadataAddress;
use unica_source_designer::format_profile::ACTIVE_FORMAT_PROFILE;

fn failure(
    owner: &MetadataAddress,
    code: MetaDiagnosticCode,
    message: impl Into<String>,
) -> unica_metadata::request::MetaFailure {
    unica_metadata::request::MetaFailure::from(
        MetaDiagnostic::error(code, message.into()).with_metadata_path(owner.clone()),
    )
}

/// Спланировать фасет справки для операции `addHelp`. План create-only:
/// существующий `Ext/Help.xml` отклоняет пакет до стейджинга любого файла,
/// а нетронутые формы охраняются точными преимиджами.
pub fn plan_help_resource_after_descriptor_edit<G>(
    reader: &dyn ProjectDiscoveryRead,
    descriptor_path: &Path,
    owner: &MetadataAddress,
    object_name: &str,
    operations: &[MetaEditOperation],
) -> Result<TypedChildResourcePlan<G>, unica_metadata::request::MetaFailure> {
    let mut langs = operations.iter().filter_map(|operation| match operation {
        MetaEditOperation::AddHelp { lang } => Some(lang.as_str()),
        _ => None,
    });
    let Some(lang) = langs.next() else {
        return Ok(TypedChildResourcePlan::default());
    };
    if langs.next().is_some() {
        return Err(failure(
            owner,
            MetaDiagnosticCode::ValidationFailed,
            "addHelp may appear at most once per call",
        ));
    }
    let owner_dir = descriptor_path.with_extension("");
    let ext_dir = owner_dir.join("Ext");
    if !path_is_dir(reader, &ext_dir) {
        return Err(failure(
            owner,
            MetaDiagnosticCode::TargetNotFound,
            "owner payload directory has no Ext facet directory",
        ));
    }
    let help_xml_path = ext_dir.join("Help.xml");
    let help_html_path = ext_dir.join("Help").join(format!("{lang}.html"));
    for existing in [&help_xml_path, &help_html_path] {
        if path_exists(reader, existing) {
            return Err(failure(
                owner,
                MetaDiagnosticCode::AlreadyExists,
                "embedded help already exists; addHelp is create-only",
            ));
        }
    }
    let format_version = ACTIVE_FORMAT_PROFILE.export_format.to_string();
    let help_xml = help_metadata_xml(lang, &format_version);
    validate_help_xml(&help_xml_path, &help_xml)
        .map_err(|message| failure(owner, MetaDiagnosticCode::ValidationFailed, message))?;
    let help_html = help_page_html(object_name);

    let mut resources = TypedChildResourcePlan::default();
    // Отдельный absent-guard не нужен: create-мутация сама несёт no-clobber,
    // а второй guard на тот же путь транзакция считает пересечением.
    for (path, text) in [(&help_xml_path, &help_xml), (&help_html_path, &help_html)] {
        let bytes = unica_xml_format::text::utf8_bom_bytes(text);
        resources.file_mutations.push(TypedChildFileMutation {
            path: path.clone(),
            pre_image: None,
            post_image: Some(bytes.clone()),
        });
        resources.expected_post_images.push((path.clone(), bytes));
    }
    resources.publication_plan.push(MetaPublicationPlanEntry {
        action: MetaPublicationAction::Create,
        resource: MetaPublicationResource::Help,
        metadata_path: Some(owner.clone()),
    });

    // Формы владельца получают IncludeHelpInContents ровно так же, как это
    // делал снятый help.add; нетронутая форма фиксируется точным преимиджем.
    let forms_dir = owner_dir.join("Forms");
    if path_is_dir(reader, &forms_dir) {
        let unreadable = |error: std::io::Error| {
            failure(
                owner,
                MetaDiagnosticCode::ProviderUnavailable,
                format!("cannot read owner forms: {error}"),
            )
        };
        let mut entries = reader
            .read_entries(&forms_dir)
            .map_err(unreadable)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(unreadable)?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let form_path = entry.path();
            if form_path.extension().and_then(|value| value.to_str()) != Some("xml")
                || !reader.path_is_file(&form_path)
            {
                continue;
            }
            let snapshot = read_utf8_sig_snapshot(reader, &form_path).map_err(|message| {
                failure(owner, MetaDiagnosticCode::ProviderUnavailable, message)
            })?;
            match form_with_include_help(&snapshot.text) {
                Some(updated) => {
                    validate_help_form_owner_8_3_27(&form_path, &updated).map_err(|message| {
                        failure(owner, MetaDiagnosticCode::ValidationFailed, message)
                    })?;
                    let bytes = unica_xml_format::text::utf8_bom_bytes(&updated);
                    resources.file_mutations.push(TypedChildFileMutation {
                        path: form_path.clone(),
                        pre_image: Some(snapshot.raw.clone()),
                        post_image: Some(bytes.clone()),
                    });
                    resources.publication_plan.push(MetaPublicationPlanEntry {
                        action: MetaPublicationAction::Update,
                        resource: MetaPublicationResource::Form,
                        metadata_path: Some(owner.clone()),
                    });
                    resources.expected_post_images.push((form_path, bytes));
                }
                None => resources
                    .exact_file_guards
                    .push((form_path, snapshot.raw.clone())),
            }
        }
    }
    Ok(resources)
}

/// One staged file change of the embedded help facet: `(path, preimage,
/// postimage)` with absolute paths under the source root.
pub type HelpFacetFileChange = (std::path::PathBuf, Option<Vec<u8>>, Option<Vec<u8>>);

/// Plans the create-only embedded help facet for one owner without a
/// transaction: `Ext/Help.xml`, `Ext/Help/<lang>.html` and the owner's forms
/// gaining `IncludeHelpInContents`.
pub fn plan_help_facet_files(
    reader: &dyn ProjectDiscoveryRead,
    descriptor_path: &std::path::Path,
    owner: &unica_address::source_target::MetadataAddress,
    object_name: &str,
    lang: &str,
) -> Result<Vec<HelpFacetFileChange>, unica_metadata::request::MetaFailure> {
    let operations = [unica_metadata::MetaEditOperation::AddHelp {
        lang: lang.to_string(),
    }];
    let plan = plan_help_resource_after_descriptor_edit::<()>(
        reader,
        descriptor_path,
        owner,
        object_name,
        &operations,
    )?;
    Ok(plan
        .file_mutations
        .into_iter()
        .map(|mutation| (mutation.path, mutation.pre_image, mutation.post_image))
        .collect())
}
