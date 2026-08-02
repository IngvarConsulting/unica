#![allow(dead_code, unused_imports)]

use super::internal::*;

pub(crate) fn fresh_meta_compile_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub(crate) fn register_compiled_meta_in_configuration(
    output_dir: &Path,
    child_tag: &str,
    obj_name: &str,
) -> Result<Option<String>, String> {
    metadata_kind(child_tag).ok_or_else(|| format!("Unknown type '{child_tag}'"))?;
    let config_xml_path = output_dir.join("Configuration.xml");
    let mut transaction = CompileTransaction::new();
    let status = transaction.register_canonical_child(&config_xml_path, child_tag, obj_name)?;
    if status == RegistrationStatus::Added {
        transaction.commit()?;
    }
    Ok(Some(
        match status {
            RegistrationStatus::Added => "added",
            RegistrationStatus::AlreadyPresent => "already",
            RegistrationStatus::MissingTarget => "no-config",
        }
        .to_string(),
    ))
}

pub(crate) fn register_compiled_meta_in_transaction(
    transaction: &mut CompileTransaction,
    output_dir: &Path,
    child_tag: &str,
    object_name: &str,
) -> Result<RegistrationStatus, String> {
    transaction.register_canonical_child(
        output_dir.join("Configuration.xml"),
        child_tag,
        object_name,
    )
}

pub(crate) type MetaCompilePlan = (
    String,
    CompileTransaction,
    Vec<PathBuf>,
    Option<PathBuf>,
    Vec<PathBuf>,
);

pub(crate) fn prepare_meta_compile(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<MetaCompilePlan, String> {
    let output_dir_label = string_arg(args, &["outputDir", "OutputDir"])
        .ok_or_else(|| "missing required OutputDir argument".to_string())?
        .to_string();
    let output_dir = absolutize(PathBuf::from(&output_dir_label), &context.cwd);
    let config_path = output_dir.join("Configuration.xml");
    let config_owner = if config_path.is_file() {
        let snapshot = read_utf8_sig_snapshot(&config_path)?;
        require_meta_configuration_owner_validation(&config_path, context, "meta.compile")?;
        if fs::read(&config_path)
            .map_err(|error| format!("failed to read {}: {error}", config_path.display()))?
            != snapshot.raw
        {
            return Err(format!(
                "Configuration owner changed while planning: {}",
                config_path.display()
            ));
        }
        #[cfg(test)]
        run_meta_compile_after_owner_validation_hook(&config_path);
        Some((config_path, snapshot.raw))
    } else {
        None
    };
    let mut transaction = CompileTransaction::new();
    let defn = read_meta_compile_definition_guarded(args, context, &mut transaction)?;
    let event_subscription_dependencies =
        meta_compile_event_subscription_dependencies(&defn, &output_dir);
    let mut format_dependencies =
        meta_compile_definition_format_dependency_paths(&defn, &output_dir);
    #[cfg(test)]
    run_meta_compile_after_format_plan_hook();
    let (stdout, planned_artifacts) = compile_meta_value(
        defn,
        &output_dir_label,
        &output_dir,
        context,
        &mut transaction,
        &mut format_dependencies,
    )?;
    validate_meta_compile_event_subscription_dependencies(
        &event_subscription_dependencies,
        &transaction,
    )?;
    if let Some((config_owner, expected_preimage)) = &config_owner {
        transaction.guard_or_verify_exact_preimage(config_owner, expected_preimage)?;
    }
    Ok((
        stdout,
        transaction,
        planned_artifacts,
        config_owner.map(|(path, _)| path),
        format_dependencies,
    ))
}

fn validate_meta_compile_post_state(
    validation_paths: &[PathBuf],
    context: &WorkspaceContext,
) -> Result<(), String> {
    for path in validation_paths {
        if path.extension().and_then(|value| value.to_str()) != Some("xml") {
            continue;
        }
        let xml = read_utf8_sig(path)?;
        let document = Document::parse(xml.trim_start_matches('\u{feff}'))
            .map_err(|error| format!("XML parse error in {}: {error}", path.display()))?;
        if document.root_element().tag_name().name() == "MetaDataObject" {
            validate_metadata_owner_shape_8_3_27(path, context, "meta.compile")?;
        }
    }
    Ok(())
}

pub(crate) fn publish_meta_compile(
    planned: Result<MetaCompilePlan, String>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let write_result = planned.and_then(
        |(stdout, mut transaction, validation_paths, config_owner, format_dependencies)| {
            let format_dependencies = format_dependencies
                .iter()
                .map(PathBuf::as_path)
                .collect::<Vec<_>>();
            guard_active_format_dependencies(&mut transaction, &format_dependencies, context)?;
            let report = transaction.commit_with_post_validation(|| {
                if let Some(config_owner) = config_owner.as_deref() {
                    require_meta_configuration_owner_validation(
                        config_owner,
                        context,
                        "meta.compile",
                    )?;
                }
                validate_meta_compile_post_state(&validation_paths, context)
            })?;
            let mut changes = report
                .created
                .iter()
                .map(|path| format!("created {}", path.display()))
                .collect::<Vec<_>>();
            changes.extend(
                report
                    .updated
                    .iter()
                    .map(|path| format!("updated {}", path.display())),
            );
            Ok((stdout, report.created, changes, report.cleanup_warnings))
        },
    );

    match write_result {
        Ok((stdout, artifacts, changes, warnings)) => AdapterOutcome {
            ok: true,
            summary: "unica.meta.compile completed with native metadata compiler".to_string(),
            changes,
            warnings,
            errors: Vec::new(),
            artifacts: artifacts
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            stdout: Some(stdout),
            stderr: None,
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.meta.compile failed in native metadata compiler".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![error.clone()],
            artifacts: Vec::new(),
            stdout: None,
            stderr: Some(format!("{error}\n")),
            command: None,
        },
    }
}

pub(crate) fn preview_prepared_meta_compile(
    planned: Result<MetaCompilePlan, String>,
) -> Result<AdapterOutcome, String> {
    let (_stdout, transaction, _validation_paths, _config_owner, _format_dependencies) = planned?;
    Ok(AdapterOutcome {
        ok: true,
        summary: "dry run: unica.meta.compile planned native metadata compilation".to_string(),
        changes: transaction.dry_run_changes(),
        warnings: Vec::new(),
        errors: Vec::new(),
        artifacts: Vec::new(),
        stdout: Some(transaction.dry_run_stdout()),
        stderr: None,
        command: None,
    })
}
