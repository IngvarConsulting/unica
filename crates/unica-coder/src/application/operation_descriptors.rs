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
    pub support_guard: Option<SupportGuardPolicy>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum SupportGuardPolicy {
    PathArgs {
        names: &'static [&'static str],
        requirement: SupportGuardRequirement,
    },
    MetaRemove {
        requirement: SupportGuardRequirement,
    },
    ObjectName {
        requirement: SupportGuardRequirement,
    },
}

const EMPTY: &[&str] = &[];
const CF_PATH: &[&str] = &["ConfigPath", "configPath", "Path", "path"];
const CONFIG_PATH: &[&str] = &["ConfigPath", "configPath"];
const CONFIG_DIR: &[&str] = &["ConfigDir", "configDir"];
const OUTPUT_DIR: &[&str] = &["OutputDir", "outputDir"];
const OUT_FILE: &[&str] = &["OutFile", "outFile"];
const EXTENSION_PATH: &[&str] = &["ExtensionPath", "extensionPath"];
const CFE_BORROW_SOURCE: &[&str] = &["ExtensionPath", "ConfigPath", "extensionPath", "configPath"];
const OBJECT_PATH: &[&str] = &["ObjectPath", "objectPath", "Path", "path"];
const OBJECT_PATH_REQUIRED: &[&str] = &["ObjectPath"];
const SRC_DIR: &[&str] = &["SrcDir", "srcDir"];
const FORM_PATH: &[&str] = &["FormPath", "formPath"];
const FORM_PATH_REQUIRED: &[&str] = &["FormPath"];
const CI_PATH: &[&str] = &["CIPath", "ciPath", "path", "Path"];
const CI_PATH_REQUIRED: &[&str] = &["CIPath"];
const SUBSYSTEM_PATH: &[&str] = &["SubsystemPath", "subsystemPath"];
const SUBSYSTEM_PATH_REQUIRED: &[&str] = &["SubsystemPath"];
const SUBSYSTEM_COMPILE_WRITE: &[&str] = &["OutputDir", "outputDir", "Parent", "parent"];
const OUTPUT_PATH: &[&str] = &["OutputPath", "outputPath"];
const TEMPLATE_PATH: &[&str] = &["TemplatePath", "templatePath"];
const TEMPLATE_PATH_REQUIRED: &[&str] = &["TemplatePath"];
const RIGHTS_PATH: &[&str] = &["RightsPath", "rightsPath"];
const RIGHTS_PATH_REQUIRED: &[&str] = &["RightsPath"];
const SUPPORT_PATH: &[&str] = &["Path", "path", "TargetPath", "targetPath"];
const META_REMOVE_REQUIRED: &[&str] = EMPTY;
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
const META_COMPILE_REQUIRED: &[&str] = &["JsonPath", "OutputDir"];
const FORM_COMPILE_REQUIRED: &[&str] = &["OutputPath"];
const FORM_EDIT_REQUIRED: &[&str] = &["FormPath"];
const SUBSYSTEM_COMPILE_REQUIRED: &[&str] = &["OutputDir"];
const MXL_COMPILE_REQUIRED: &[&str] = &["JsonPath", "OutputPath"];
const ROLE_COMPILE_REQUIRED: &[&str] = &["JsonPath", "OutputDir"];
const EXTERNAL_INIT_REQUIRED: &[&str] = &["Name", "OutputDir"];
const CODE_PATCH_PATH: &[&str] = &["path"];
const CODE_PATCH_REQUIRED: &[&str] = &["path", "operation", "selector", "content", "position"];

pub(crate) fn native_operation_descriptor(operation: &str) -> Option<&'static OperationDescriptor> {
    NATIVE_OPERATION_DESCRIPTORS
        .iter()
        .find(|descriptor| descriptor.operation == operation)
}

pub(super) const NATIVE_OPERATION_DESCRIPTORS: &[OperationDescriptor] = &[
    descriptor(
        "code-patch",
        CODE_PATCH_REQUIRED,
        CODE_PATCH_PATH,
        CODE_PATCH_PATH,
        Some(path_guard(
            CODE_PATCH_PATH,
            SupportGuardRequirement::Editable,
        )),
    ),
    descriptor(
        "cf-edit",
        EMPTY,
        CF_PATH,
        CF_PATH,
        Some(path_guard(CF_PATH, SupportGuardRequirement::Editable)),
    ),
    descriptor("cf-info", &["ConfigPath"], OUT_FILE, CONFIG_PATH, None),
    descriptor("cf-init", EMPTY, OUTPUT_DIR, OUTPUT_DIR, None),
    descriptor("cf-validate", &["ConfigPath"], OUT_FILE, CONFIG_PATH, None),
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
    descriptor("cfe-init", EMPTY, OUTPUT_DIR, OUTPUT_DIR, None),
    descriptor(
        "epf-init",
        EXTERNAL_INIT_REQUIRED,
        OUTPUT_DIR,
        OUTPUT_DIR,
        None,
    ),
    descriptor(
        "erf-init",
        EXTERNAL_INIT_REQUIRED,
        OUTPUT_DIR,
        OUTPUT_DIR,
        None,
    ),
    descriptor(
        "cfe-patch-method",
        CFE_PATCH_METHOD_REQUIRED,
        EXTENSION_PATH,
        EXTENSION_PATH,
        None,
    ),
    descriptor(
        "cfe-validate",
        CFE_VALIDATE_REQUIRED,
        OUT_FILE,
        EXTENSION_PATH,
        None,
    ),
    descriptor(
        "meta-compile",
        META_COMPILE_REQUIRED,
        OUTPUT_DIR,
        OUTPUT_DIR,
        Some(path_guard(OUTPUT_DIR, SupportGuardRequirement::Editable)),
    ),
    descriptor(
        "meta-edit",
        OBJECT_PATH_REQUIRED,
        OBJECT_PATH,
        OBJECT_PATH,
        Some(path_guard(OBJECT_PATH, SupportGuardRequirement::Editable)),
    ),
    descriptor(
        "meta-info",
        OBJECT_PATH_REQUIRED,
        OUT_FILE,
        OBJECT_PATH,
        None,
    ),
    descriptor(
        "meta-remove",
        META_REMOVE_REQUIRED,
        CONFIG_DIR,
        CONFIG_DIR,
        Some(meta_remove_guard()),
    ),
    descriptor(
        "meta-validate",
        OBJECT_PATH_REQUIRED,
        OUT_FILE,
        OBJECT_PATH,
        None,
    ),
    descriptor(
        "help-add",
        OBJECT_NAME_REQUIRED,
        SRC_DIR,
        SRC_DIR,
        Some(object_name_guard(SupportGuardRequirement::Editable)),
    ),
    descriptor(
        "form-add",
        EMPTY,
        OBJECT_PATH,
        OBJECT_PATH,
        Some(path_guard(OBJECT_PATH, SupportGuardRequirement::Editable)),
    ),
    descriptor(
        "form-compile",
        FORM_COMPILE_REQUIRED,
        OUTPUT_PATH,
        OUTPUT_PATH,
        Some(path_guard(OUTPUT_PATH, SupportGuardRequirement::Editable)),
    ),
    descriptor(
        "form-edit",
        FORM_EDIT_REQUIRED,
        FORM_PATH,
        FORM_PATH,
        Some(path_guard(FORM_PATH, SupportGuardRequirement::Editable)),
    ),
    descriptor("form-info", FORM_PATH_REQUIRED, EMPTY, FORM_PATH, None),
    descriptor(
        "form-remove",
        EMPTY,
        SRC_DIR,
        SRC_DIR,
        Some(object_name_guard(SupportGuardRequirement::Editable)),
    ),
    descriptor("form-validate", FORM_PATH_REQUIRED, EMPTY, FORM_PATH, None),
    descriptor(
        "interface-edit",
        CI_PATH_REQUIRED,
        CI_PATH,
        CI_PATH,
        Some(path_guard(CI_PATH, SupportGuardRequirement::Editable)),
    ),
    descriptor(
        "interface-validate",
        CI_PATH_REQUIRED,
        OUT_FILE,
        CI_PATH,
        None,
    ),
    descriptor(
        "subsystem-compile",
        SUBSYSTEM_COMPILE_REQUIRED,
        SUBSYSTEM_COMPILE_WRITE,
        SUBSYSTEM_COMPILE_WRITE,
        Some(path_guard(OUTPUT_DIR, SupportGuardRequirement::Editable)),
    ),
    descriptor(
        "subsystem-edit",
        SUBSYSTEM_PATH_REQUIRED,
        SUBSYSTEM_PATH,
        SUBSYSTEM_PATH,
        Some(path_guard(
            SUBSYSTEM_PATH,
            SupportGuardRequirement::Editable,
        )),
    ),
    descriptor(
        "subsystem-info",
        SUBSYSTEM_PATH_REQUIRED,
        OUT_FILE,
        SUBSYSTEM_PATH,
        None,
    ),
    descriptor(
        "subsystem-validate",
        SUBSYSTEM_PATH_REQUIRED,
        OUT_FILE,
        SUBSYSTEM_PATH,
        None,
    ),
    descriptor(
        "template-add",
        EMPTY,
        SRC_DIR,
        SRC_DIR,
        Some(object_name_guard(SupportGuardRequirement::Editable)),
    ),
    descriptor(
        "template-remove",
        EMPTY,
        SRC_DIR,
        SRC_DIR,
        Some(object_name_guard(SupportGuardRequirement::Editable)),
    ),
    descriptor(
        "dcs-compile",
        EMPTY,
        OUTPUT_PATH,
        OUTPUT_PATH,
        Some(path_guard(OUTPUT_PATH, SupportGuardRequirement::Editable)),
    ),
    descriptor(
        "dcs-edit",
        TEMPLATE_PATH_REQUIRED,
        TEMPLATE_PATH,
        TEMPLATE_PATH,
        Some(path_guard(TEMPLATE_PATH, SupportGuardRequirement::Editable)),
    ),
    descriptor(
        "dcs-info",
        TEMPLATE_PATH_REQUIRED,
        OUT_FILE,
        TEMPLATE_PATH,
        None,
    ),
    descriptor(
        "dcs-validate",
        TEMPLATE_PATH_REQUIRED,
        OUT_FILE,
        TEMPLATE_PATH,
        None,
    ),
    descriptor(
        "mxl-compile",
        MXL_COMPILE_REQUIRED,
        OUTPUT_PATH,
        OUTPUT_PATH,
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
    descriptor(
        "mxl-validate",
        TEMPLATE_PATH_REQUIRED,
        EMPTY,
        TEMPLATE_PATH,
        None,
    ),
    descriptor(
        "role-compile",
        ROLE_COMPILE_REQUIRED,
        OUTPUT_DIR,
        OUTPUT_DIR,
        Some(path_guard(OUTPUT_DIR, SupportGuardRequirement::Editable)),
    ),
    descriptor(
        "role-info",
        RIGHTS_PATH_REQUIRED,
        OUT_FILE,
        RIGHTS_PATH,
        None,
    ),
    descriptor(
        "role-validate",
        RIGHTS_PATH_REQUIRED,
        OUT_FILE,
        RIGHTS_PATH,
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
        support_guard,
    }
}

const fn path_guard(
    names: &'static [&'static str],
    requirement: SupportGuardRequirement,
) -> SupportGuardPolicy {
    SupportGuardPolicy::PathArgs { names, requirement }
}

const fn meta_remove_guard() -> SupportGuardPolicy {
    SupportGuardPolicy::MetaRemove {
        requirement: SupportGuardRequirement::Removed,
    }
}

const fn object_name_guard(requirement: SupportGuardRequirement) -> SupportGuardPolicy {
    SupportGuardPolicy::ObjectName { requirement }
}
