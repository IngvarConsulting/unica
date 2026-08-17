//! ADR-0072: планировщик фасета встроенной справки. `addHelp` материализует
//! `Ext/Help.xml`, `Ext/Help/<lang>.html` владельца и флаг
//! `IncludeHelpInContents` его форм через тот же приватный post-image план,
//! которым пользуется каждая типизированная операция: ни один файл не
//! стейджится до успешного построения полного плана.

use crate::application::metadata::MetaFailure;
use crate::domain::format_profile::ACTIVE_FORMAT_PROFILE;
use crate::domain::metadata::{
    MetaDiagnostic, MetaDiagnosticCode, MetaEditOperation, MetaPublicationAction,
    MetaPublicationPlanEntry, MetaPublicationResource,
};
use crate::domain::source_target::MetadataAddress;
use crate::infrastructure::native_operations::common::{read_utf8_sig_snapshot, utf8_bom_bytes};
use crate::infrastructure::native_operations::help::{
    form_with_include_help, help_metadata_xml, help_page_html, validate_help_form_owner_8_3_27,
    validate_help_xml,
};
use std::fs;
use std::path::Path;

use super::edit::{TypedChildFileMutation, TypedChildResourcePlan};

fn failure(
    owner: &MetadataAddress,
    code: MetaDiagnosticCode,
    message: impl Into<String>,
) -> MetaFailure {
    MetaFailure::from(MetaDiagnostic::error(code, message.into()).with_metadata_path(owner.clone()))
}

/// Спланировать фасет справки для операции `addHelp`. План create-only:
/// существующий `Ext/Help.xml` отклоняет пакет до стейджинга любого файла,
/// а нетронутые формы охраняются точными преимиджами.
pub(super) fn plan_help_resource_after_descriptor_edit(
    descriptor_path: &Path,
    owner: &MetadataAddress,
    object_name: &str,
    operations: &[MetaEditOperation],
) -> Result<TypedChildResourcePlan, MetaFailure> {
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
    if !ext_dir.is_dir() {
        return Err(failure(
            owner,
            MetaDiagnosticCode::TargetNotFound,
            "owner payload directory has no Ext facet directory",
        ));
    }
    let help_xml_path = ext_dir.join("Help.xml");
    let help_html_path = ext_dir.join("Help").join(format!("{lang}.html"));
    for existing in [&help_xml_path, &help_html_path] {
        if existing.exists() {
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
        let bytes = utf8_bom_bytes(text);
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
    if forms_dir.is_dir() {
        let unreadable = |error: std::io::Error| {
            failure(
                owner,
                MetaDiagnosticCode::ProviderUnavailable,
                format!("cannot read owner forms: {error}"),
            )
        };
        let mut entries = fs::read_dir(&forms_dir)
            .map_err(unreadable)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(unreadable)?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let form_path = entry.path();
            if form_path.extension().and_then(|value| value.to_str()) != Some("xml")
                || !form_path.is_file()
            {
                continue;
            }
            let snapshot = read_utf8_sig_snapshot(&form_path).map_err(|message| {
                failure(owner, MetaDiagnosticCode::ProviderUnavailable, message)
            })?;
            match form_with_include_help(&snapshot.text) {
                Some(updated) => {
                    validate_help_form_owner_8_3_27(&form_path, &updated).map_err(|message| {
                        failure(owner, MetaDiagnosticCode::ValidationFailed, message)
                    })?;
                    let bytes = utf8_bom_bytes(&updated);
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
