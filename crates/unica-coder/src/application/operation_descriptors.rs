#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SupportGuardRequirement {
    Editable,
    Removed,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OperationDescriptor {
    pub operation: &'static str,
    pub required_args: &'static [&'static str],
    pub write_path_args: &'static [&'static str],
    pub source_path_args: &'static [&'static str],
    pub format_path_policy: FormatPathPolicy,
    pub format_guard: FormatGuardPolicy,
    pub support_guard: Option<SupportGuardPolicy>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FormatPathPolicy {
    DeclaredArgs,
    HandlerResolved,
    DefaultSrcObject,
    FormCompile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FormatGuardPolicy {
    ExistingDump,
    OptionalExistingBase,
    NewDump,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum SupportGuardPolicy {
    HandlerResolved {
        requirement: SupportGuardRequirement,
    },
    PathArgs {
        names: &'static [&'static str],
        requirement: SupportGuardRequirement,
    },
    ObjectName {
        requirement: SupportGuardRequirement,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PathAliasGroup {
    pub canonical: &'static str,
    pub aliases: &'static [&'static str],
}

const EMPTY: &[&str] = &[];
pub(crate) const CF_PATH: &[&str] = &["ConfigPath", "configPath", "Path", "path"];
const OUTPUT_DIR: &[&str] = &["OutputDir", "outputDir"];
const EXTENSION_PATH: &[&str] = &["ExtensionPath", "extensionPath"];
pub(crate) const CFE_VALIDATE_PATH: &[&str] = &["ExtensionPath", "extensionPath", "Path", "path"];
const CFE_BORROW_SOURCE: &[&str] = &["ExtensionPath", "ConfigPath", "extensionPath", "configPath"];
const CFE_INIT_BASE: &[&str] = &["ConfigPath", "configPath"];
const CFE_INIT_OUTPUT: &[&str] = &["OutputDir", "outputDir", "ExtensionPath", "extensionPath"];
pub(crate) const OBJECT_PATH: &[&str] = &["ObjectPath", "objectPath", "Path", "path"];
const SRC_DIR: &[&str] = &["SrcDir", "srcDir"];
pub(crate) const FORM_PATH: &[&str] = &["FormPath", "formPath", "Path", "path"];
const FORM_PATH_REQUIRED: &[&str] = &["FormPath"];
const CI_PATH: &[&str] = &["CIPath", "ciPath", "path", "Path"];
const CI_PATH_REQUIRED: &[&str] = &["CIPath"];
pub(crate) const SUBSYSTEM_PATH: &[&str] = &["SubsystemPath", "subsystemPath", "Path", "path"];
const SUBSYSTEM_PATH_REQUIRED: &[&str] = &["SubsystemPath"];
const SUBSYSTEM_COMPILE_WRITE: &[&str] = &["OutputDir", "outputDir", "Parent", "parent"];
const SUBSYSTEM_COMPILE_GUARD: &[&str] = &["Parent", "parent", "OutputDir", "outputDir"];
const OUTPUT_PATH: &[&str] = &["OutputPath", "outputPath"];
pub(crate) const TEMPLATE_PATH: &[&str] = &["TemplatePath", "templatePath", "Path", "path"];
const TEMPLATE_PATH_REQUIRED: &[&str] = &["TemplatePath"];
pub(crate) const RIGHTS_PATH: &[&str] = &["RightsPath", "rightsPath", "Path", "path"];
const RIGHTS_PATH_REQUIRED: &[&str] = &["RightsPath"];
const SUPPORT_PATH: &[&str] = &["Path", "path", "TargetPath", "targetPath"];
const CFE_DIFF_REQUIRED: &[&str] = &["ExtensionPath", "ConfigPath"];
const CFE_BORROW_REQUIRED: &[&str] = &["ExtensionPath", "ConfigPath", "Object"];
const CFE_PATCH_METHOD_REQUIRED: &[&str] = &[
    "ExtensionPath",
    "ModulePath",
    "MethodName",
    "InterceptorType",
];
const CFE_VALIDATE_REQUIRED: &[&str] = &["ExtensionPath"];
const OBJECT_NAME_REQUIRED: &[&str] = &["ObjectName"];
const FORM_COMPILE_REQUIRED: &[&str] = &["OutputPath"];
const FORM_EDIT_REQUIRED: &[&str] = &["FormPath"];
const SUBSYSTEM_COMPILE_REQUIRED: &[&str] = &["OutputDir"];
const MXL_COMPILE_REQUIRED: &[&str] = &["JsonPath", "OutputPath"];
const ROLE_COMPILE_REQUIRED: &[&str] = &["JsonPath", "OutputDir"];
const ROLE_EDIT_REQUIRED: &[&str] = &["sourceSet", "metadataPath", "operations"];
const EXTERNAL_INIT_REQUIRED: &[&str] = &["Name", "OutputDir"];
// `selector` and `position` are required only by selector-based operations, so
// the descriptor cannot demand them for every call; `initialize` deliberately
// has neither and `validate_code_patch_arguments` enforces each operation.
const CODE_PATCH_REQUIRED: &[&str] = &["sourceSet", "metadataPath", "operation", "content"];
const XDTO_INFO_REQUIRED: &[&str] = &["sourceSet", "metadataPath"];
const XDTO_EDIT_REQUIRED: &[&str] = &["sourceSet", "metadataPath", "operation"];

const JSON_PATH: &[&str] = &["JsonPath", "jsonPath"];
const DEFINITION_FILE: &[&str] = &["DefinitionFile", "definitionFile"];
const MODULE_PATH: &[&str] = &["ModulePath", "modulePath"];
const PARENT_PATH: &[&str] = &["Parent", "parent"];

const CF_PATH_GROUP: PathAliasGroup = path_alias_group("ConfigPath", CF_PATH);
const OUTPUT_DIR_GROUP: PathAliasGroup = path_alias_group("OutputDir", OUTPUT_DIR);
const EXTENSION_PATH_GROUP: PathAliasGroup = path_alias_group("ExtensionPath", EXTENSION_PATH);
const CFE_VALIDATE_PATH_GROUP: PathAliasGroup =
    path_alias_group("ExtensionPath", CFE_VALIDATE_PATH);
const CFE_INIT_OUTPUT_GROUP: PathAliasGroup = path_alias_group("OutputDir", CFE_INIT_OUTPUT);
const OBJECT_PATH_GROUP: PathAliasGroup = path_alias_group("ObjectPath", OBJECT_PATH);
const SRC_DIR_GROUP: PathAliasGroup = path_alias_group("SrcDir", SRC_DIR);
const FORM_PATH_GROUP: PathAliasGroup = path_alias_group("FormPath", FORM_PATH);
const CI_PATH_GROUP: PathAliasGroup = path_alias_group("CIPath", CI_PATH);
const SUBSYSTEM_PATH_GROUP: PathAliasGroup = path_alias_group("SubsystemPath", SUBSYSTEM_PATH);
const OUTPUT_PATH_GROUP: PathAliasGroup = path_alias_group("OutputPath", OUTPUT_PATH);
const TEMPLATE_PATH_GROUP: PathAliasGroup = path_alias_group("TemplatePath", TEMPLATE_PATH);
const RIGHTS_PATH_GROUP: PathAliasGroup = path_alias_group("RightsPath", RIGHTS_PATH);
const SUPPORT_PATH_GROUP: PathAliasGroup = path_alias_group("Path", SUPPORT_PATH);
const JSON_PATH_GROUP: PathAliasGroup = path_alias_group("JsonPath", JSON_PATH);
const DEFINITION_FILE_GROUP: PathAliasGroup = path_alias_group("DefinitionFile", DEFINITION_FILE);
const MODULE_PATH_GROUP: PathAliasGroup = path_alias_group("ModulePath", MODULE_PATH);
const PARENT_PATH_GROUP: PathAliasGroup = path_alias_group("Parent", PARENT_PATH);

const CF_EDIT_PATH_GROUPS: &[PathAliasGroup] = &[CF_PATH_GROUP, DEFINITION_FILE_GROUP];
const CF_READ_PATH_GROUPS: &[PathAliasGroup] = &[CF_PATH_GROUP];
const CF_INIT_PATH_GROUPS: &[PathAliasGroup] = &[OUTPUT_DIR_GROUP];
const SUPPORT_PATH_GROUPS: &[PathAliasGroup] = &[SUPPORT_PATH_GROUP];
const CFE_TWO_ROOT_PATH_GROUPS: &[PathAliasGroup] = &[EXTENSION_PATH_GROUP, CF_PATH_GROUP];
const CFE_INIT_PATH_GROUPS: &[PathAliasGroup] = &[CF_PATH_GROUP, CFE_INIT_OUTPUT_GROUP];
const CFE_PATCH_METHOD_PATH_GROUPS: &[PathAliasGroup] = &[EXTENSION_PATH_GROUP, MODULE_PATH_GROUP];
const CFE_VALIDATE_PATH_GROUPS: &[PathAliasGroup] = &[CFE_VALIDATE_PATH_GROUP];
const COMPILE_TO_DIR_PATH_GROUPS: &[PathAliasGroup] = &[JSON_PATH_GROUP, OUTPUT_DIR_GROUP];
const SRC_DIR_PATH_GROUPS: &[PathAliasGroup] = &[SRC_DIR_GROUP];
const OBJECT_PATH_GROUPS: &[PathAliasGroup] = &[OBJECT_PATH_GROUP];
const FORM_COMPILE_PATH_GROUPS: &[PathAliasGroup] =
    &[JSON_PATH_GROUP, OBJECT_PATH_GROUP, OUTPUT_PATH_GROUP];
const FORM_EDIT_PATH_GROUPS: &[PathAliasGroup] = &[FORM_PATH_GROUP, JSON_PATH_GROUP];
const FORM_READ_PATH_GROUPS: &[PathAliasGroup] = &[FORM_PATH_GROUP];
const INTERFACE_EDIT_PATH_GROUPS: &[PathAliasGroup] = &[CI_PATH_GROUP, DEFINITION_FILE_GROUP];
const INTERFACE_READ_PATH_GROUPS: &[PathAliasGroup] = &[CI_PATH_GROUP];
const SUBSYSTEM_COMPILE_PATH_GROUPS: &[PathAliasGroup] =
    &[OUTPUT_DIR_GROUP, PARENT_PATH_GROUP, DEFINITION_FILE_GROUP];
const SUBSYSTEM_EDIT_PATH_GROUPS: &[PathAliasGroup] =
    &[SUBSYSTEM_PATH_GROUP, DEFINITION_FILE_GROUP];
const SUBSYSTEM_READ_PATH_GROUPS: &[PathAliasGroup] = &[SUBSYSTEM_PATH_GROUP];
const DCS_COMPILE_PATH_GROUPS: &[PathAliasGroup] = &[OUTPUT_PATH_GROUP, DEFINITION_FILE_GROUP];
const DCS_EDIT_PATH_GROUPS: &[PathAliasGroup] = &[TEMPLATE_PATH_GROUP, DEFINITION_FILE_GROUP];
const DCS_READ_PATH_GROUPS: &[PathAliasGroup] = &[TEMPLATE_PATH_GROUP];
const MXL_READ_PATH_GROUPS: &[PathAliasGroup] = &[TEMPLATE_PATH_GROUP, SRC_DIR_GROUP];
/// `unica.mxl.info` requires `TemplatePath`, so the `SrcDir` half of the
/// composite address it never reached is not published (#298).
const MXL_INFO_PATH_GROUPS: &[PathAliasGroup] = &[TEMPLATE_PATH_GROUP];
const COMPILE_TO_PATH_GROUPS: &[PathAliasGroup] = &[JSON_PATH_GROUP, OUTPUT_PATH_GROUP];
const RIGHTS_READ_PATH_GROUPS: &[PathAliasGroup] = &[RIGHTS_PATH_GROUP];

pub(crate) fn native_path_alias_groups(operation: &str) -> &'static [PathAliasGroup] {
    match operation {
        "cf-edit" => CF_EDIT_PATH_GROUPS,
        "cf-info" | "cf-validate" => CF_READ_PATH_GROUPS,
        "cf-init" => CF_INIT_PATH_GROUPS,
        "support-edit" => SUPPORT_PATH_GROUPS,
        "cfe-borrow" | "cfe-diff" => CFE_TWO_ROOT_PATH_GROUPS,
        "cfe-init" => CFE_INIT_PATH_GROUPS,
        "cfe-patch-method" => CFE_PATCH_METHOD_PATH_GROUPS,
        "cfe-validate" => CFE_VALIDATE_PATH_GROUPS,
        "role-compile" => COMPILE_TO_DIR_PATH_GROUPS,
        "help-add" | "form-remove" | "template-add" | "template-remove" => SRC_DIR_PATH_GROUPS,
        "form-add" => OBJECT_PATH_GROUPS,
        "form-compile" => FORM_COMPILE_PATH_GROUPS,
        "form-edit" => FORM_EDIT_PATH_GROUPS,
        "form-info" | "form-validate" => FORM_READ_PATH_GROUPS,
        "interface-edit" => INTERFACE_EDIT_PATH_GROUPS,
        "interface-validate" => INTERFACE_READ_PATH_GROUPS,
        "subsystem-compile" => SUBSYSTEM_COMPILE_PATH_GROUPS,
        "subsystem-edit" => SUBSYSTEM_EDIT_PATH_GROUPS,
        "subsystem-info" | "subsystem-validate" => SUBSYSTEM_READ_PATH_GROUPS,
        "dcs-compile" => DCS_COMPILE_PATH_GROUPS,
        "dcs-edit" => DCS_EDIT_PATH_GROUPS,
        "dcs-info" | "dcs-validate" => DCS_READ_PATH_GROUPS,
        "mxl-info" => MXL_INFO_PATH_GROUPS,
        "mxl-decompile" | "mxl-validate" => MXL_READ_PATH_GROUPS,
        "mxl-compile" => COMPILE_TO_PATH_GROUPS,
        "role-info" | "role-validate" => RIGHTS_READ_PATH_GROUPS,
        _ => &[],
    }
}

pub(crate) fn native_operation_descriptor(operation: &str) -> Option<&'static OperationDescriptor> {
    NATIVE_OPERATION_DESCRIPTORS
        .iter()
        .find(|descriptor| descriptor.operation == operation)
}

pub(super) const NATIVE_OPERATION_DESCRIPTORS: &[OperationDescriptor] = &[
    descriptor_with_paths(
        "xdto-info",
        XDTO_INFO_REQUIRED,
        EMPTY,
        EMPTY,
        FormatGuardPolicy::ExistingDump,
        FormatPathPolicy::HandlerResolved,
        None,
    ),
    descriptor_with_paths(
        "xdto-edit",
        XDTO_EDIT_REQUIRED,
        EMPTY,
        EMPTY,
        FormatGuardPolicy::ExistingDump,
        FormatPathPolicy::HandlerResolved,
        Some(handler_resolved_guard(SupportGuardRequirement::Editable)),
    ),
    descriptor_with_paths(
        "code-patch",
        CODE_PATCH_REQUIRED,
        EMPTY,
        EMPTY,
        FormatGuardPolicy::ExistingDump,
        FormatPathPolicy::HandlerResolved,
        Some(handler_resolved_guard(SupportGuardRequirement::Editable)),
    ),
    descriptor_with_paths(
        "cf-edit",
        EMPTY,
        CF_PATH,
        CF_PATH,
        FormatGuardPolicy::ExistingDump,
        FormatPathPolicy::HandlerResolved,
        Some(path_guard(CF_PATH, SupportGuardRequirement::Editable)),
    ),
    descriptor_with_paths(
        "cf-info",
        &["ConfigPath"],
        EMPTY,
        CF_PATH,
        FormatGuardPolicy::ExistingDump,
        FormatPathPolicy::HandlerResolved,
        None,
    ),
    descriptor_with_format(
        "cf-init",
        EMPTY,
        OUTPUT_DIR,
        OUTPUT_DIR,
        FormatGuardPolicy::NewDump,
        None,
    ),
    descriptor_with_paths(
        "cf-validate",
        &["ConfigPath"],
        EMPTY,
        CF_PATH,
        FormatGuardPolicy::ExistingDump,
        FormatPathPolicy::HandlerResolved,
        None,
    ),
    descriptor("support-edit", EMPTY, SUPPORT_PATH, SUPPORT_PATH, None),
    descriptor(
        "cfe-borrow",
        CFE_BORROW_REQUIRED,
        EXTENSION_PATH,
        CFE_BORROW_SOURCE,
        None,
    ),
    descriptor(
        "cfe-diff",
        CFE_DIFF_REQUIRED,
        EMPTY,
        &["ExtensionPath", "ConfigPath", "extensionPath", "configPath"],
        None,
    ),
    descriptor_with_format(
        "cfe-init",
        EMPTY,
        CFE_INIT_OUTPUT,
        CFE_INIT_BASE,
        FormatGuardPolicy::OptionalExistingBase,
        None,
    ),
    descriptor_with_format(
        "epf-init",
        EXTERNAL_INIT_REQUIRED,
        OUTPUT_DIR,
        OUTPUT_DIR,
        FormatGuardPolicy::NewDump,
        None,
    ),
    descriptor_with_format(
        "erf-init",
        EXTERNAL_INIT_REQUIRED,
        OUTPUT_DIR,
        OUTPUT_DIR,
        FormatGuardPolicy::NewDump,
        None,
    ),
    descriptor(
        "cfe-patch-method",
        CFE_PATCH_METHOD_REQUIRED,
        EXTENSION_PATH,
        EXTENSION_PATH,
        None,
    ),
    descriptor_with_paths(
        "cfe-validate",
        CFE_VALIDATE_REQUIRED,
        EMPTY,
        CFE_VALIDATE_PATH,
        FormatGuardPolicy::ExistingDump,
        FormatPathPolicy::HandlerResolved,
        None,
    ),
    descriptor_with_paths(
        "help-add",
        OBJECT_NAME_REQUIRED,
        SRC_DIR,
        SRC_DIR,
        FormatGuardPolicy::ExistingDump,
        FormatPathPolicy::DefaultSrcObject,
        Some(object_name_guard(SupportGuardRequirement::Editable)),
    ),
    descriptor_with_paths(
        "form-add",
        EMPTY,
        OBJECT_PATH,
        OBJECT_PATH,
        FormatGuardPolicy::ExistingDump,
        FormatPathPolicy::HandlerResolved,
        Some(path_guard(OBJECT_PATH, SupportGuardRequirement::Editable)),
    ),
    descriptor_with_paths(
        "form-compile",
        FORM_COMPILE_REQUIRED,
        OUTPUT_PATH,
        OUTPUT_PATH,
        FormatGuardPolicy::ExistingDump,
        FormatPathPolicy::FormCompile,
        Some(path_guard(OUTPUT_PATH, SupportGuardRequirement::Editable)),
    ),
    descriptor(
        "form-edit",
        FORM_EDIT_REQUIRED,
        FORM_PATH,
        FORM_PATH,
        Some(path_guard(FORM_PATH, SupportGuardRequirement::Editable)),
    ),
    descriptor_with_paths(
        "form-info",
        FORM_PATH_REQUIRED,
        EMPTY,
        FORM_PATH,
        FormatGuardPolicy::ExistingDump,
        FormatPathPolicy::HandlerResolved,
        None,
    ),
    descriptor_with_paths(
        "form-remove",
        EMPTY,
        SRC_DIR,
        SRC_DIR,
        FormatGuardPolicy::ExistingDump,
        FormatPathPolicy::DefaultSrcObject,
        Some(object_name_guard(SupportGuardRequirement::Editable)),
    ),
    descriptor_with_paths(
        "form-validate",
        FORM_PATH_REQUIRED,
        EMPTY,
        FORM_PATH,
        FormatGuardPolicy::ExistingDump,
        FormatPathPolicy::HandlerResolved,
        None,
    ),
    descriptor(
        "interface-edit",
        CI_PATH_REQUIRED,
        CI_PATH,
        CI_PATH,
        Some(path_guard(CI_PATH, SupportGuardRequirement::Editable)),
    ),
    descriptor_with_paths(
        "interface-validate",
        CI_PATH_REQUIRED,
        EMPTY,
        CI_PATH,
        FormatGuardPolicy::ExistingDump,
        FormatPathPolicy::HandlerResolved,
        None,
    ),
    descriptor(
        "subsystem-compile",
        SUBSYSTEM_COMPILE_REQUIRED,
        SUBSYSTEM_COMPILE_WRITE,
        SUBSYSTEM_COMPILE_WRITE,
        Some(path_guard(
            SUBSYSTEM_COMPILE_GUARD,
            SupportGuardRequirement::Editable,
        )),
    ),
    descriptor_with_paths(
        "subsystem-edit",
        SUBSYSTEM_PATH_REQUIRED,
        SUBSYSTEM_PATH,
        SUBSYSTEM_PATH,
        FormatGuardPolicy::ExistingDump,
        FormatPathPolicy::HandlerResolved,
        Some(path_guard(
            SUBSYSTEM_PATH,
            SupportGuardRequirement::Editable,
        )),
    ),
    descriptor(
        "subsystem-info",
        SUBSYSTEM_PATH_REQUIRED,
        EMPTY,
        SUBSYSTEM_PATH,
        None,
    ),
    descriptor(
        "subsystem-validate",
        SUBSYSTEM_PATH_REQUIRED,
        EMPTY,
        SUBSYSTEM_PATH,
        None,
    ),
    descriptor_with_paths(
        "template-add",
        EMPTY,
        SRC_DIR,
        SRC_DIR,
        FormatGuardPolicy::ExistingDump,
        FormatPathPolicy::DefaultSrcObject,
        Some(object_name_guard(SupportGuardRequirement::Editable)),
    ),
    descriptor_with_paths(
        "template-remove",
        EMPTY,
        SRC_DIR,
        SRC_DIR,
        FormatGuardPolicy::ExistingDump,
        FormatPathPolicy::DefaultSrcObject,
        Some(object_name_guard(SupportGuardRequirement::Editable)),
    ),
    descriptor(
        "dcs-compile",
        EMPTY,
        OUTPUT_PATH,
        OUTPUT_PATH,
        Some(path_guard(OUTPUT_PATH, SupportGuardRequirement::Editable)),
    ),
    descriptor_with_paths(
        "dcs-edit",
        TEMPLATE_PATH_REQUIRED,
        TEMPLATE_PATH,
        TEMPLATE_PATH,
        FormatGuardPolicy::ExistingDump,
        FormatPathPolicy::HandlerResolved,
        Some(path_guard(TEMPLATE_PATH, SupportGuardRequirement::Editable)),
    ),
    descriptor(
        "dcs-info",
        TEMPLATE_PATH_REQUIRED,
        EMPTY,
        TEMPLATE_PATH,
        None,
    ),
    descriptor_with_paths(
        "dcs-validate",
        TEMPLATE_PATH_REQUIRED,
        EMPTY,
        TEMPLATE_PATH,
        FormatGuardPolicy::ExistingDump,
        FormatPathPolicy::HandlerResolved,
        None,
    ),
    descriptor_with_format(
        "mxl-compile",
        MXL_COMPILE_REQUIRED,
        OUTPUT_PATH,
        OUTPUT_PATH,
        FormatGuardPolicy::ExistingDump,
        Some(path_guard(OUTPUT_PATH, SupportGuardRequirement::Editable)),
    ),
    descriptor(
        "mxl-decompile",
        TEMPLATE_PATH_REQUIRED,
        EMPTY,
        TEMPLATE_PATH,
        None,
    ),
    descriptor(
        "mxl-info",
        TEMPLATE_PATH_REQUIRED,
        EMPTY,
        TEMPLATE_PATH,
        None,
    ),
    descriptor_with_paths(
        "mxl-validate",
        TEMPLATE_PATH_REQUIRED,
        EMPTY,
        TEMPLATE_PATH,
        FormatGuardPolicy::ExistingDump,
        FormatPathPolicy::HandlerResolved,
        None,
    ),
    descriptor(
        "role-compile",
        ROLE_COMPILE_REQUIRED,
        OUTPUT_DIR,
        OUTPUT_DIR,
        Some(path_guard(OUTPUT_DIR, SupportGuardRequirement::Editable)),
    ),
    descriptor_with_paths(
        "role-edit",
        ROLE_EDIT_REQUIRED,
        EMPTY,
        EMPTY,
        FormatGuardPolicy::ExistingDump,
        FormatPathPolicy::HandlerResolved,
        Some(handler_resolved_guard(SupportGuardRequirement::Editable)),
    ),
    descriptor_with_paths(
        "role-info",
        RIGHTS_PATH_REQUIRED,
        EMPTY,
        RIGHTS_PATH,
        FormatGuardPolicy::ExistingDump,
        FormatPathPolicy::HandlerResolved,
        None,
    ),
    descriptor_with_paths(
        "role-validate",
        RIGHTS_PATH_REQUIRED,
        EMPTY,
        RIGHTS_PATH,
        FormatGuardPolicy::ExistingDump,
        FormatPathPolicy::HandlerResolved,
        None,
    ),
];

const fn descriptor(
    operation: &'static str,
    required_args: &'static [&'static str],
    write_path_args: &'static [&'static str],
    source_path_args: &'static [&'static str],
    support_guard: Option<SupportGuardPolicy>,
) -> OperationDescriptor {
    OperationDescriptor {
        operation,
        required_args,
        write_path_args,
        source_path_args,
        format_path_policy: FormatPathPolicy::DeclaredArgs,
        format_guard: FormatGuardPolicy::ExistingDump,
        support_guard,
    }
}

const fn descriptor_with_format(
    operation: &'static str,
    required_args: &'static [&'static str],
    write_path_args: &'static [&'static str],
    source_path_args: &'static [&'static str],
    format_guard: FormatGuardPolicy,
    support_guard: Option<SupportGuardPolicy>,
) -> OperationDescriptor {
    OperationDescriptor {
        operation,
        required_args,
        write_path_args,
        source_path_args,
        format_path_policy: FormatPathPolicy::DeclaredArgs,
        format_guard,
        support_guard,
    }
}

const fn descriptor_with_paths(
    operation: &'static str,
    required_args: &'static [&'static str],
    write_path_args: &'static [&'static str],
    source_path_args: &'static [&'static str],
    format_guard: FormatGuardPolicy,
    format_path_policy: FormatPathPolicy,
    support_guard: Option<SupportGuardPolicy>,
) -> OperationDescriptor {
    OperationDescriptor {
        operation,
        required_args,
        write_path_args,
        source_path_args,
        format_path_policy,
        format_guard,
        support_guard,
    }
}

const fn path_guard(
    names: &'static [&'static str],
    requirement: SupportGuardRequirement,
) -> SupportGuardPolicy {
    SupportGuardPolicy::PathArgs { names, requirement }
}

const fn handler_resolved_guard(requirement: SupportGuardRequirement) -> SupportGuardPolicy {
    SupportGuardPolicy::HandlerResolved { requirement }
}

const fn object_name_guard(requirement: SupportGuardRequirement) -> SupportGuardPolicy {
    SupportGuardPolicy::ObjectName { requirement }
}

const fn path_alias_group(
    canonical: &'static str,
    aliases: &'static [&'static str],
) -> PathAliasGroup {
    PathAliasGroup { canonical, aliases }
}
