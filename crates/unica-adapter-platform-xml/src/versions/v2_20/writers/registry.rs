use unica_format_core::commands::{FormEditEvidence, MutationMode, WriterCommand};

use crate::{application::NativeWriterResult, operations::PlatformWriterSession};

use super::{
    cf, cfe, common, dcs, external, form, help, interface, meta, mxl, role, subsystem, support,
    template,
};

pub(crate) fn execute(
    command: &WriterCommand,
    mode: MutationMode,
    session: &PlatformWriterSession,
) -> Result<(NativeWriterResult, Option<FormEditEvidence>), String> {
    let context = session.context();
    preflight_family_owned_command(command, session)?;
    if mode.is_preview() {
        return preview(command, session);
    }
    let outcome = match command {
        WriterCommand::ConfigurationInitialize(command) => {
            cf::initialize_configuration(command, session, context)
        }
        WriterCommand::ConfigurationEdit(command) => {
            cf::edit_configuration(command, session, context)
        }
        WriterCommand::ExtensionInitialize(command) => {
            cfe::initialize_extension(command, session, context)
        }
        WriterCommand::ExtensionBorrow(command) => {
            cfe::borrow_into_extension(command, session, context)
        }
        WriterCommand::ExtensionPatchMethod(command) => {
            let emitter = session
                .extension_emitter()
                .ok_or_else(|| "extension method patch requires a host BSL emitter".to_string())?;
            cfe::patch_extension_method_typed(
                command,
                session,
                context,
                emitter,
                MutationMode::Apply,
            )
        }
        WriterCommand::ExternalProcessorInitialize(command) => {
            external::initialize_processor(command, session, context, MutationMode::Apply)
        }
        WriterCommand::ExternalReportInitialize(command) => {
            external::initialize_report(command, session, context, MutationMode::Apply)
        }
        WriterCommand::MetadataCreate(command) => meta::create_metadata(command, session, context),
        WriterCommand::MetadataEdit(command) => meta::edit_metadata(command, session, context),
        WriterCommand::MetadataRemove(command) => meta::remove_metadata(command, session, context),
        WriterCommand::FormCreate(command) => form::create_form(command, session, context),
        WriterCommand::FormCompile(command) => form::compile_form_typed(command, session, context),
        WriterCommand::FormEdit(command) => {
            let (outcome, evidence) =
                form::apply_typed_with_data(command, session, context).into_core_parts();
            return Ok((outcome, evidence));
        }
        WriterCommand::FormRemove(command) => form::remove_form_typed(command, session, context),
        WriterCommand::TemplateCreate(command) => {
            template::create_template(command, session, context)
        }
        WriterCommand::TemplateRemove(command) => {
            template::remove_template_typed(command, session, context)
        }
        WriterCommand::HelpCreate(command) => help::create_help(command, session, context),
        WriterCommand::InterfaceEdit(command) => {
            interface::edit_interface_typed(command, session, context)
        }
        WriterCommand::RoleCreate(command) => role::create_role(command, session, context),
        WriterCommand::SubsystemCreate(command) => {
            subsystem::create_subsystem(command, session, context)
        }
        WriterCommand::SubsystemEdit(command) => {
            subsystem::edit_subsystem_typed(command, session, context)
        }
        WriterCommand::SupportEdit(command) => {
            support::edit_support_typed(command, session, context)
        }
        WriterCommand::DataCompositionCreate(command) => {
            dcs::create_data_composition(command, session, context)
        }
        WriterCommand::DataCompositionEdit(command) => {
            dcs::edit_data_composition(command, session, context)
        }
        WriterCommand::SpreadsheetCreate(command) => {
            mxl::create_spreadsheet(command, session, context)
        }
    };
    Ok((outcome, None))
}

fn preflight_family_owned_command(
    command: &WriterCommand,
    session: &PlatformWriterSession,
) -> Result<(), String> {
    use unica_format_core::commands::WriterSourceRole;

    match command {
        WriterCommand::DataCompositionCreate(_) => {
            let destination = session.required_source(
                WriterSourceRole::DestinationArtifact,
                "data composition destination",
            )?;
            common::preflight_active_format_dependencies_for_create(
                &[],
                &[destination],
                session.context(),
            )
        }
        WriterCommand::DataCompositionEdit(_) => {
            let template = session.required_source(
                WriterSourceRole::Template,
                "data composition mutation target",
            )?;
            common::preflight_active_format_dependencies(&[template], session.context())
        }
        WriterCommand::SpreadsheetCreate(_) => {
            let destination = session.required_source(
                WriterSourceRole::DestinationArtifact,
                "spreadsheet destination",
            )?;
            common::preflight_active_format_dependencies_for_create(
                &[],
                &[destination],
                session.context(),
            )
        }
        _ => Ok(()),
    }
}

fn preview(
    command: &WriterCommand,
    session: &PlatformWriterSession,
) -> Result<(NativeWriterResult, Option<FormEditEvidence>), String> {
    let context = session.context();
    let outcome = match command {
        WriterCommand::ExtensionPatchMethod(command) => {
            let emitter = session
                .extension_emitter()
                .ok_or_else(|| "extension method patch requires a host BSL emitter".to_string())?;
            cfe::patch_extension_method_typed(
                command,
                session,
                context,
                emitter,
                MutationMode::Preview,
            )
        }
        WriterCommand::ExternalProcessorInitialize(command) => {
            external::initialize_processor(command, session, context, MutationMode::Preview)
        }
        WriterCommand::ExternalReportInitialize(command) => {
            external::initialize_report(command, session, context, MutationMode::Preview)
        }
        WriterCommand::FormCompile(command) => {
            form::preview_form_compile_typed(command, session, context)?
        }
        WriterCommand::FormEdit(command) => {
            let (outcome, evidence) =
                form::preview_typed_with_data(command, session, context).into_core_parts();
            return Ok((outcome, evidence));
        }
        WriterCommand::MetadataCreate(command) => {
            meta::preview_meta_compile_typed(command, session, context)?
        }
        WriterCommand::MetadataEdit(command) => {
            meta::validate_meta_edit_preview_typed(command, session, context)?;
            preview_placeholder()
        }
        WriterCommand::RoleCreate(command) => {
            role::preview_role_compile_typed(command, session, context)?
        }
        WriterCommand::SubsystemCreate(command) => {
            subsystem::preview_subsystem_compile_typed(command, session, context)?
        }
        _ => preview_placeholder(),
    };
    Ok((outcome, None))
}

fn preview_placeholder() -> NativeWriterResult {
    NativeWriterResult {
        ok: true,
        summary: "semantic writer preview".to_string(),
        changes: Vec::new(),
        warnings: Vec::new(),
        errors: Vec::new(),
        artifacts: Vec::new(),
        stdout: None,
        stderr: None,
    }
}
