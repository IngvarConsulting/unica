//! Closed, format-neutral mutation commands and outcomes.
//!
//! Commands contain semantic intent only. Source locations, serialized
//! definitions and adapter-specific compatibility data are captured in opaque
//! operational sessions before a command reaches a port.

use serde::Serialize;

use crate::{
    ports::{
        OperationalContractError, PublicationCancellation, PublicationCleanup, PublicationRecovery,
        PublicationRollback,
    },
    semantic_ids::SemanticObjectKind,
};

mod inspection;
pub use inspection::*;
mod module_locator;
pub use module_locator::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WriterFamily {
    Configuration,
    Extension,
    ExternalArtifact,
    Metadata,
    Form,
    Template,
    Help,
    Interface,
    Role,
    Subsystem,
    Support,
    DataComposition,
    Spreadsheet,
}

impl WriterFamily {
    pub const ALL: [Self; 13] = [
        Self::Configuration,
        Self::Extension,
        Self::ExternalArtifact,
        Self::Metadata,
        Self::Form,
        Self::Template,
        Self::Help,
        Self::Interface,
        Self::Role,
        Self::Subsystem,
        Self::Support,
        Self::DataComposition,
        Self::Spreadsheet,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationMode {
    Preview,
    Apply,
}

impl MutationMode {
    pub const fn is_preview(self) -> bool {
        matches!(self, Self::Preview)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticValueError {
    Empty,
    ControlCharacter,
    TooLong,
}

impl std::fmt::Display for SemanticValueError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "semantic value must not be empty",
            Self::ControlCharacter => "semantic value must not contain control characters",
            Self::TooLong => "semantic value is too long",
        })
    }
}

impl std::error::Error for SemanticValueError {}

fn validate_text(value: String, max: usize) -> Result<String, SemanticValueError> {
    if value.trim().is_empty() {
        return Err(SemanticValueError::Empty);
    }
    if value.chars().any(char::is_control) {
        return Err(SemanticValueError::ControlCharacter);
    }
    if value.chars().count() > max {
        return Err(SemanticValueError::TooLong);
    }
    Ok(value)
}

macro_rules! semantic_text {
    ($name:ident, $max:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, SemanticValueError> {
                validate_text(value.into(), $max).map(Self)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

semantic_text!(ConfigurationName, 128);
semantic_text!(ExtensionName, 128);
semantic_text!(ExternalArtifactName, 128);
semantic_text!(MetadataObjectReference, 256);
semantic_text!(MetadataKindName, 128);
semantic_text!(FormOwnerReference, 256);
semantic_text!(FormName, 128);
semantic_text!(TemplateOwnerReference, 256);
semantic_text!(TemplateName, 128);
semantic_text!(HelpOwnerReference, 256);
semantic_text!(RoleName, 128);
semantic_text!(SubsystemName, 128);
semantic_text!(ModuleReference, 256);
semantic_text!(MethodName, 128);
semantic_text!(SynonymText, 512);
semantic_text!(VendorName, 256);
semantic_text!(ArtifactVersion, 64);
semantic_text!(LanguageCode, 16);
semantic_text!(NamePrefix, 128);
semantic_text!(ConfigurationPropertyChange, 4096);
semantic_text!(ConfigurationChildReference, 4096);
semantic_text!(ConfigurationRoleSet, 4096);
semantic_text!(ConfigurationRoleReference, 4096);
semantic_text!(ConfigurationPanelArrangement, 4096);
semantic_text!(ConfigurationHomePageArrangement, 4096);
semantic_text!(MetadataPropertyOperand, 4096);
semantic_text!(MetadataAttributeOperand, 4096);
semantic_text!(MetadataTabularSectionOperand, 4096);
semantic_text!(MetadataTabularSectionAttributeOperand, 4096);
semantic_text!(MetadataFormOperand, 4096);
semantic_text!(MetadataTemplateOperand, 4096);
semantic_text!(MetadataCommandOperand, 4096);
semantic_text!(MetadataDimensionOperand, 4096);
semantic_text!(MetadataResourceOperand, 4096);
semantic_text!(MetadataRequisiteOperand, 4096);
semantic_text!(MetadataEnumValueOperand, 4096);
semantic_text!(InterfaceItemReference, 4096);
semantic_text!(InterfacePlacement, 4096);
semantic_text!(InterfaceItemOrder, 4096);
semantic_text!(InterfaceSubsystemOrder, 4096);
semantic_text!(InterfaceGroupOrder, 4096);
semantic_text!(SubsystemContentReference, 4096);
semantic_text!(ChildSubsystemReference, 4096);
semantic_text!(SubsystemPropertyChange, 4096);
semantic_text!(DataCompositionFieldEdit, 65_536);
semantic_text!(DataCompositionTotalEdit, 65_536);
semantic_text!(DataCompositionCalculatedFieldEdit, 65_536);
semantic_text!(DataCompositionParameterEdit, 65_536);
semantic_text!(DataCompositionFilterEdit, 65_536);
semantic_text!(DataCompositionDataParameterEdit, 65_536);
semantic_text!(DataCompositionQueryText, 65_536);
semantic_text!(DataCompositionQueryPatch, 65_536);
semantic_text!(DataCompositionScopeReference, 65_536);
semantic_text!(DataCompositionSelectionEdit, 65_536);
semantic_text!(DataCompositionOrderEdit, 65_536);
semantic_text!(DataCompositionDataSetLinkEdit, 65_536);
semantic_text!(DataCompositionDataSetEdit, 65_536);
semantic_text!(DataCompositionVariantEdit, 65_536);
semantic_text!(DataSetName, 128);
semantic_text!(VariantName, 128);
semantic_text!(ProcessorName, 128);
semantic_text!(FormElementName, 128);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WriterSourceRole {
    Configuration,
    ConfigurationDirectory,
    Extension,
    DestinationDirectory,
    Definition,
    Object,
    SourceCollection,
    Form,
    Interface,
    Subsystem,
    DestinationArtifact,
    ParentSubsystem,
    Template,
    Rights,
    SupportTarget,
    Module,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigurationMutation {
    ModifyProperty(ConfigurationPropertyChange),
    RemoveChild(ConfigurationChildReference),
    AddChild(ConfigurationChildReference),
    SetDefaultRoles(ConfigurationRoleSet),
    AddDefaultRole(ConfigurationRoleReference),
    RemoveDefaultRole(ConfigurationRoleReference),
    SetPanels(ConfigurationPanelArrangement),
    SetHomePage(ConfigurationHomePageArrangement),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationInitialize {
    name: ConfigurationName,
    synonym: Option<SynonymText>,
    vendor: Option<VendorName>,
    version: Option<ArtifactVersion>,
    omit_default_role: bool,
}

impl ConfigurationInitialize {
    pub const fn new(name: ConfigurationName) -> Self {
        Self {
            name,
            synonym: None,
            vendor: None,
            version: None,
            omit_default_role: false,
        }
    }
    pub fn with_synonym(mut self, value: Option<SynonymText>) -> Self {
        self.synonym = value;
        self
    }
    pub fn with_vendor(mut self, value: Option<VendorName>) -> Self {
        self.vendor = value;
        self
    }
    pub fn with_version(mut self, value: Option<ArtifactVersion>) -> Self {
        self.version = value;
        self
    }
    pub const fn omit_default_role(mut self, value: bool) -> Self {
        self.omit_default_role = value;
        self
    }
    pub const fn name(&self) -> &ConfigurationName {
        &self.name
    }
    pub const fn synonym(&self) -> Option<&SynonymText> {
        self.synonym.as_ref()
    }
    pub const fn vendor(&self) -> Option<&VendorName> {
        self.vendor.as_ref()
    }
    pub const fn version(&self) -> Option<&ArtifactVersion> {
        self.version.as_ref()
    }
    pub const fn omits_default_role(&self) -> bool {
        self.omit_default_role
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConfigurationEdit {
    mutation: Option<ConfigurationMutation>,
}

impl ConfigurationEdit {
    pub const fn from_definition() -> Self {
        Self { mutation: None }
    }
    pub const fn mutate(mutation: ConfigurationMutation) -> Self {
        Self {
            mutation: Some(mutation),
        }
    }
    pub const fn mutation(&self) -> Option<&ConfigurationMutation> {
        self.mutation.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowScope {
    Form,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterceptorKind {
    Before,
    After,
    Instead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionContext {
    Automatic,
    Client,
    Server,
    ServerWithoutContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionPurpose {
    Patch,
    Customization,
    AddOn,
}

impl ExtensionPurpose {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Patch => "Patch",
            Self::Customization => "Customization",
            Self::AddOn => "AddOn",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionInitialize {
    name: ExtensionName,
    synonym: Option<SynonymText>,
    purpose: Option<ExtensionPurpose>,
    prefix: Option<NamePrefix>,
}

impl ExtensionInitialize {
    pub const fn new(name: ExtensionName) -> Self {
        Self {
            name,
            synonym: None,
            purpose: None,
            prefix: None,
        }
    }
    pub fn with_synonym(mut self, value: Option<SynonymText>) -> Self {
        self.synonym = value;
        self
    }
    pub fn with_purpose(mut self, value: Option<ExtensionPurpose>) -> Self {
        self.purpose = value;
        self
    }
    pub fn with_prefix(mut self, value: Option<NamePrefix>) -> Self {
        self.prefix = value;
        self
    }
    pub const fn name(&self) -> &ExtensionName {
        &self.name
    }
    pub const fn synonym(&self) -> Option<&SynonymText> {
        self.synonym.as_ref()
    }
    pub const fn purpose(&self) -> Option<&ExtensionPurpose> {
        self.purpose.as_ref()
    }
    pub const fn prefix(&self) -> Option<&NamePrefix> {
        self.prefix.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionBorrow {
    object: MetadataObjectReference,
    main_attribute: Option<BorrowScope>,
    exclude_selection: bool,
    force: bool,
}

impl ExtensionBorrow {
    pub const fn new(object: MetadataObjectReference) -> Self {
        Self {
            object,
            main_attribute: None,
            exclude_selection: false,
            force: false,
        }
    }
    pub const fn with_main_attribute(mut self, value: Option<BorrowScope>) -> Self {
        self.main_attribute = value;
        self
    }
    pub const fn exclude_selection(mut self, value: bool) -> Self {
        self.exclude_selection = value;
        self
    }
    pub const fn force(mut self, value: bool) -> Self {
        self.force = value;
        self
    }
    pub const fn object(&self) -> &MetadataObjectReference {
        &self.object
    }
    pub const fn main_attribute(&self) -> Option<BorrowScope> {
        self.main_attribute
    }
    pub const fn excludes_selection(&self) -> bool {
        self.exclude_selection
    }
    pub const fn is_forced(&self) -> bool {
        self.force
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionPatchMethod {
    module: ModuleReference,
    method: MethodName,
    interceptor: InterceptorKind,
    context: ExecutionContext,
    function: bool,
}

impl ExtensionPatchMethod {
    pub const fn new(
        module: ModuleReference,
        method: MethodName,
        interceptor: InterceptorKind,
        context: ExecutionContext,
        function: bool,
    ) -> Self {
        Self {
            module,
            method,
            interceptor,
            context,
            function,
        }
    }
    pub const fn module(&self) -> &ModuleReference {
        &self.module
    }
    pub const fn method(&self) -> &MethodName {
        &self.method
    }
    pub const fn interceptor(&self) -> InterceptorKind {
        self.interceptor
    }
    pub const fn context(&self) -> ExecutionContext {
        self.context
    }
    pub const fn is_function(&self) -> bool {
        self.function
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionPatchEmissionPlan {
    prefix: NamePrefix,
    method: MethodName,
    interceptor: InterceptorKind,
    context: Option<ExecutionContext>,
}

impl ExtensionPatchEmissionPlan {
    pub const fn new(
        prefix: NamePrefix,
        method: MethodName,
        interceptor: InterceptorKind,
        context: Option<ExecutionContext>,
    ) -> Self {
        Self {
            prefix,
            method,
            interceptor,
            context,
        }
    }
    pub const fn prefix(&self) -> &NamePrefix {
        &self.prefix
    }
    pub const fn method(&self) -> &MethodName {
        &self.method
    }
    pub const fn interceptor(&self) -> InterceptorKind {
        self.interceptor
    }
    pub const fn context(&self) -> Option<ExecutionContext> {
        self.context
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalArtifactInitialize {
    name: ExternalArtifactName,
    synonym: Option<SynonymText>,
    form: Option<FormName>,
}

impl ExternalArtifactInitialize {
    pub const fn new(name: ExternalArtifactName) -> Self {
        Self {
            name,
            synonym: None,
            form: None,
        }
    }
    pub fn with_synonym(mut self, value: Option<SynonymText>) -> Self {
        self.synonym = value;
        self
    }
    pub fn with_form(mut self, value: Option<FormName>) -> Self {
        self.form = value;
        self
    }
    pub const fn name(&self) -> &ExternalArtifactName {
        &self.name
    }
    pub const fn synonym(&self) -> Option<&SynonymText> {
        self.synonym.as_ref()
    }
    pub const fn form(&self) -> Option<&FormName> {
        self.form.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataMutation {
    AddProperty(MetadataPropertyOperand),
    RemoveProperty(MetadataPropertyOperand),
    ModifyProperty(MetadataPropertyOperand),
    AddAttribute(MetadataAttributeOperand),
    RemoveAttribute(MetadataAttributeOperand),
    ModifyAttribute(MetadataAttributeOperand),
    AddTabularSection(MetadataTabularSectionOperand),
    RemoveTabularSection(MetadataTabularSectionOperand),
    ModifyTabularSection(MetadataTabularSectionOperand),
    AddTabularSectionAttribute(MetadataTabularSectionAttributeOperand),
    RemoveTabularSectionAttribute(MetadataTabularSectionAttributeOperand),
    ModifyTabularSectionAttribute(MetadataTabularSectionAttributeOperand),
    AddForm(MetadataFormOperand),
    RemoveForm(MetadataFormOperand),
    ModifyForm(MetadataFormOperand),
    AddTemplate(MetadataTemplateOperand),
    RemoveTemplate(MetadataTemplateOperand),
    ModifyTemplate(MetadataTemplateOperand),
    AddCommand(MetadataCommandOperand),
    RemoveCommand(MetadataCommandOperand),
    ModifyCommand(MetadataCommandOperand),
    AddDimension(MetadataDimensionOperand),
    RemoveDimension(MetadataDimensionOperand),
    ModifyDimension(MetadataDimensionOperand),
    AddResource(MetadataResourceOperand),
    RemoveResource(MetadataResourceOperand),
    ModifyResource(MetadataResourceOperand),
    AddRequisite(MetadataRequisiteOperand),
    RemoveRequisite(MetadataRequisiteOperand),
    ModifyRequisite(MetadataRequisiteOperand),
    AddEnumValue(MetadataEnumValueOperand),
    RemoveEnumValue(MetadataEnumValueOperand),
    ModifyEnumValue(MetadataEnumValueOperand),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MetadataCreate {
    omit_default_role: bool,
    assign_default_form: bool,
}

impl MetadataCreate {
    pub const fn new() -> Self {
        Self {
            omit_default_role: false,
            assign_default_form: false,
        }
    }
    pub const fn omit_default_role(mut self, value: bool) -> Self {
        self.omit_default_role = value;
        self
    }
    pub const fn assign_default_form(mut self, value: bool) -> Self {
        self.assign_default_form = value;
        self
    }
    pub const fn omits_default_role(&self) -> bool {
        self.omit_default_role
    }
    pub const fn assigns_default_form(&self) -> bool {
        self.assign_default_form
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataEdit {
    object: Option<MetadataObjectReference>,
    mutation: Option<MetadataMutation>,
    create_if_missing: bool,
}

impl MetadataEdit {
    pub const fn new(object: MetadataObjectReference) -> Self {
        Self {
            object: Some(object),
            mutation: None,
            create_if_missing: false,
        }
    }
    pub const fn selected_object() -> Self {
        Self {
            object: None,
            mutation: None,
            create_if_missing: false,
        }
    }
    pub fn with_mutation(mut self, value: Option<MetadataMutation>) -> Self {
        self.mutation = value;
        self
    }
    pub const fn create_if_missing(mut self, value: bool) -> Self {
        self.create_if_missing = value;
        self
    }
    pub const fn object(&self) -> Option<&MetadataObjectReference> {
        self.object.as_ref()
    }
    pub const fn mutation(&self) -> Option<&MetadataMutation> {
        self.mutation.as_ref()
    }
    pub const fn creates_if_missing(&self) -> bool {
        self.create_if_missing
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataRemove {
    object: MetadataObjectReference,
    keep_files: bool,
}

impl MetadataRemove {
    pub const fn new(object: MetadataObjectReference, keep_files: bool) -> Self {
        Self { object, keep_files }
    }
    pub const fn object(&self) -> &MetadataObjectReference {
        &self.object
    }
    pub const fn keeps_files(&self) -> bool {
        self.keep_files
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormCreate {
    owner: FormOwnerReference,
    name: FormName,
    synonym: Option<SynonymText>,
    purpose: FormPurpose,
    default_assignment: DefaultFormAssignment,
}

impl FormCreate {
    pub const fn new(owner: FormOwnerReference, name: FormName) -> Self {
        Self {
            owner,
            name,
            synonym: None,
            purpose: FormPurpose::Object,
            default_assignment: DefaultFormAssignment::IfVacant,
        }
    }
    pub fn with_synonym(mut self, value: Option<SynonymText>) -> Self {
        self.synonym = value;
        self
    }
    pub const fn with_purpose(mut self, value: FormPurpose) -> Self {
        self.purpose = value;
        self
    }
    pub const fn with_default_assignment(mut self, value: DefaultFormAssignment) -> Self {
        self.default_assignment = value;
        self
    }
    pub const fn assign_default(mut self, value: bool) -> Self {
        self.default_assignment = if value {
            DefaultFormAssignment::Always
        } else {
            DefaultFormAssignment::Never
        };
        self
    }
    pub const fn owner(&self) -> &FormOwnerReference {
        &self.owner
    }
    pub const fn name(&self) -> &FormName {
        &self.name
    }
    pub const fn synonym(&self) -> Option<&SynonymText> {
        self.synonym.as_ref()
    }
    pub const fn purpose(&self) -> FormPurpose {
        self.purpose
    }
    pub const fn default_assignment(&self) -> DefaultFormAssignment {
        self.default_assignment
    }
    pub const fn assigns_default(&self) -> bool {
        matches!(self.default_assignment, DefaultFormAssignment::Always)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormPurpose {
    Object,
    List,
    Choice,
    Record,
}

impl FormPurpose {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Object => "Object",
            Self::List => "List",
            Self::Choice => "Choice",
            Self::Record => "Record",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultFormAssignment {
    IfVacant,
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormCompilePurpose {
    Item,
    Folder,
    List,
    Choice,
    Record,
}

impl FormCompilePurpose {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Item => "Item",
            Self::Folder => "Folder",
            Self::List => "List",
            Self::Choice => "Choice",
            Self::Record => "Record",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormCompileSource {
    Definition,
    Object { purpose: Option<FormCompilePurpose> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormCompile {
    source: FormCompileSource,
    skip_validation: bool,
}

impl FormCompile {
    pub const fn new(skip_validation: bool) -> Self {
        Self {
            source: FormCompileSource::Definition,
            skip_validation,
        }
    }
    pub const fn from_object(purpose: Option<FormCompilePurpose>, skip_validation: bool) -> Self {
        Self {
            source: FormCompileSource::Object { purpose },
            skip_validation,
        }
    }
    pub const fn source(self) -> FormCompileSource {
        self.source
    }
    pub const fn skips_validation(self) -> bool {
        self.skip_validation
    }
}

impl Default for FormCompile {
    fn default() -> Self {
        Self::new(false)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FormEdit {
    skip_validation: bool,
}

impl FormEdit {
    pub const fn new(skip_validation: bool) -> Self {
        Self { skip_validation }
    }
    pub const fn skips_validation(self) -> bool {
        self.skip_validation
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormRemove {
    owner: FormOwnerReference,
    name: FormName,
}

impl FormRemove {
    pub const fn new(owner: FormOwnerReference, name: FormName) -> Self {
        Self { owner, name }
    }
    pub const fn owner(&self) -> &FormOwnerReference {
        &self.owner
    }
    pub const fn name(&self) -> &FormName {
        &self.name
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateKind {
    DataComposition,
    Spreadsheet,
    Text,
    Html,
    Binary,
    Graphical,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateCreate {
    owner: TemplateOwnerReference,
    name: TemplateName,
    kind: TemplateKind,
    synonym: Option<SynonymText>,
    assign_main_data_composition: bool,
}

impl TemplateCreate {
    pub const fn new(
        owner: TemplateOwnerReference,
        name: TemplateName,
        kind: TemplateKind,
    ) -> Self {
        Self {
            owner,
            name,
            kind,
            synonym: None,
            assign_main_data_composition: false,
        }
    }
    pub fn with_synonym(mut self, value: Option<SynonymText>) -> Self {
        self.synonym = value;
        self
    }
    pub const fn assign_main_data_composition(mut self, value: bool) -> Self {
        self.assign_main_data_composition = value;
        self
    }
    pub const fn owner(&self) -> &TemplateOwnerReference {
        &self.owner
    }
    pub const fn name(&self) -> &TemplateName {
        &self.name
    }
    pub const fn kind(&self) -> TemplateKind {
        self.kind
    }
    pub const fn synonym(&self) -> Option<&SynonymText> {
        self.synonym.as_ref()
    }
    pub const fn assigns_main_data_composition(&self) -> bool {
        self.assign_main_data_composition
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateRemove {
    owner: TemplateOwnerReference,
    name: TemplateName,
}

impl TemplateRemove {
    pub const fn new(owner: TemplateOwnerReference, name: TemplateName) -> Self {
        Self { owner, name }
    }
    pub const fn owner(&self) -> &TemplateOwnerReference {
        &self.owner
    }
    pub const fn name(&self) -> &TemplateName {
        &self.name
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpCreate {
    owner: HelpOwnerReference,
    language: Option<LanguageCode>,
}

impl HelpCreate {
    pub const fn new(owner: HelpOwnerReference, language: Option<LanguageCode>) -> Self {
        Self { owner, language }
    }
    pub const fn owner(&self) -> &HelpOwnerReference {
        &self.owner
    }
    pub const fn language(&self) -> Option<&LanguageCode> {
        self.language.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterfaceEdit {
    FromDefinition,
    Hide(InterfaceItemReference),
    Show(InterfaceItemReference),
    Place(InterfacePlacement),
    Order(InterfaceItemOrder),
    OrderSubsystems(InterfaceSubsystemOrder),
    OrderGroups(InterfaceGroupOrder),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RoleCreate {
    name: Option<RoleName>,
}

impl RoleCreate {
    pub const fn new(name: Option<RoleName>) -> Self {
        Self { name }
    }
    pub const fn name(&self) -> Option<&RoleName> {
        self.name.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SubsystemCreate {
    name: Option<SubsystemName>,
}

impl SubsystemCreate {
    pub const fn new(name: Option<SubsystemName>) -> Self {
        Self { name }
    }
    pub const fn name(&self) -> Option<&SubsystemName> {
        self.name.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubsystemEdit {
    FromDefinition,
    AddContent(SubsystemContentReference),
    RemoveContent(SubsystemContentReference),
    AddChild(ChildSubsystemReference),
    RemoveChild(ChildSubsystemReference),
    SetProperty(SubsystemPropertyChange),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportCapability {
    Enable,
    Disable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportObjectRule {
    Locked,
    Editable,
    OffSupport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportEdit {
    Capability(SupportCapability),
    ObjectRule(SupportObjectRule),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DataCompositionCreate;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataCompositionMutation {
    AddField(DataCompositionFieldEdit),
    AddTotal(DataCompositionTotalEdit),
    AddCalculatedField(DataCompositionCalculatedFieldEdit),
    AddParameter(DataCompositionParameterEdit),
    AddFilter(DataCompositionFilterEdit),
    AddDataParameter(DataCompositionDataParameterEdit),
    SetQuery(DataCompositionQueryText),
    PatchQuery(DataCompositionQueryPatch),
    ClearSelection(DataCompositionScopeReference),
    ClearOrder(DataCompositionScopeReference),
    ClearFilter(DataCompositionScopeReference),
    ClearConditionalAppearance(DataCompositionScopeReference),
    AddSelection(DataCompositionSelectionEdit),
    AddOrder(DataCompositionOrderEdit),
    AddDataSetLink(DataCompositionDataSetLinkEdit),
    AddDataSet(DataCompositionDataSetEdit),
    AddVariant(DataCompositionVariantEdit),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataCompositionEdit {
    mutation: DataCompositionMutation,
    data_set: Option<DataSetName>,
    variant: Option<VariantName>,
    omit_selection: bool,
    skip_validation: bool,
}

impl DataCompositionEdit {
    pub const fn new(mutation: DataCompositionMutation) -> Self {
        Self {
            mutation,
            data_set: None,
            variant: None,
            omit_selection: false,
            skip_validation: false,
        }
    }
    pub fn with_data_set(mut self, value: Option<DataSetName>) -> Self {
        self.data_set = value;
        self
    }
    pub fn with_variant(mut self, value: Option<VariantName>) -> Self {
        self.variant = value;
        self
    }
    pub const fn omit_selection(mut self, value: bool) -> Self {
        self.omit_selection = value;
        self
    }
    pub const fn skip_validation(mut self, value: bool) -> Self {
        self.skip_validation = value;
        self
    }
    pub const fn mutation(&self) -> &DataCompositionMutation {
        &self.mutation
    }
    pub const fn data_set(&self) -> Option<&DataSetName> {
        self.data_set.as_ref()
    }
    pub const fn variant(&self) -> Option<&VariantName> {
        self.variant.as_ref()
    }
    pub const fn omits_selection(&self) -> bool {
        self.omit_selection
    }
    pub const fn skips_validation(&self) -> bool {
        self.skip_validation
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpreadsheetCreate {
    processor: Option<ProcessorName>,
    template: Option<TemplateName>,
    derive_from_object: bool,
}

impl SpreadsheetCreate {
    pub const fn new() -> Self {
        Self {
            processor: None,
            template: None,
            derive_from_object: false,
        }
    }
    pub fn with_processor(mut self, value: Option<ProcessorName>) -> Self {
        self.processor = value;
        self
    }
    pub fn with_template(mut self, value: Option<TemplateName>) -> Self {
        self.template = value;
        self
    }
    pub const fn derive_from_object(mut self, value: bool) -> Self {
        self.derive_from_object = value;
        self
    }
    pub const fn processor(&self) -> Option<&ProcessorName> {
        self.processor.as_ref()
    }
    pub const fn template(&self) -> Option<&TemplateName> {
        self.template.as_ref()
    }
    pub const fn derives_from_object(&self) -> bool {
        self.derive_from_object
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriterCommand {
    ConfigurationInitialize(ConfigurationInitialize),
    ConfigurationEdit(ConfigurationEdit),
    ExtensionInitialize(ExtensionInitialize),
    ExtensionBorrow(ExtensionBorrow),
    ExtensionPatchMethod(ExtensionPatchMethod),
    ExternalProcessorInitialize(ExternalArtifactInitialize),
    ExternalReportInitialize(ExternalArtifactInitialize),
    MetadataCreate(MetadataCreate),
    MetadataEdit(MetadataEdit),
    MetadataRemove(MetadataRemove),
    FormCreate(FormCreate),
    FormCompile(FormCompile),
    FormEdit(FormEdit),
    FormRemove(FormRemove),
    TemplateCreate(TemplateCreate),
    TemplateRemove(TemplateRemove),
    HelpCreate(HelpCreate),
    InterfaceEdit(InterfaceEdit),
    RoleCreate(RoleCreate),
    SubsystemCreate(SubsystemCreate),
    SubsystemEdit(SubsystemEdit),
    SupportEdit(SupportEdit),
    DataCompositionCreate(DataCompositionCreate),
    DataCompositionEdit(DataCompositionEdit),
    SpreadsheetCreate(SpreadsheetCreate),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WriterCommandKind {
    ConfigurationInitialize,
    ConfigurationEdit,
    ExtensionInitialize,
    ExtensionBorrow,
    ExtensionPatchMethod,
    ExternalProcessorInitialize,
    ExternalReportInitialize,
    MetadataCreate,
    MetadataEdit,
    MetadataRemove,
    FormCreate,
    FormCompile,
    FormEdit,
    FormRemove,
    TemplateCreate,
    TemplateRemove,
    HelpCreate,
    InterfaceEdit,
    RoleCreate,
    SubsystemCreate,
    SubsystemEdit,
    SupportEdit,
    DataCompositionCreate,
    DataCompositionEdit,
    SpreadsheetCreate,
}

impl WriterCommandKind {
    pub const ALL: [Self; 25] = [
        Self::ConfigurationInitialize,
        Self::ConfigurationEdit,
        Self::ExtensionInitialize,
        Self::ExtensionBorrow,
        Self::ExtensionPatchMethod,
        Self::ExternalProcessorInitialize,
        Self::ExternalReportInitialize,
        Self::MetadataCreate,
        Self::MetadataEdit,
        Self::MetadataRemove,
        Self::FormCreate,
        Self::FormCompile,
        Self::FormEdit,
        Self::FormRemove,
        Self::TemplateCreate,
        Self::TemplateRemove,
        Self::HelpCreate,
        Self::InterfaceEdit,
        Self::RoleCreate,
        Self::SubsystemCreate,
        Self::SubsystemEdit,
        Self::SupportEdit,
        Self::DataCompositionCreate,
        Self::DataCompositionEdit,
        Self::SpreadsheetCreate,
    ];
}

impl WriterCommand {
    pub const fn kind(&self) -> WriterCommandKind {
        match self {
            Self::ConfigurationInitialize(_) => WriterCommandKind::ConfigurationInitialize,
            Self::ConfigurationEdit(_) => WriterCommandKind::ConfigurationEdit,
            Self::ExtensionInitialize(_) => WriterCommandKind::ExtensionInitialize,
            Self::ExtensionBorrow(_) => WriterCommandKind::ExtensionBorrow,
            Self::ExtensionPatchMethod(_) => WriterCommandKind::ExtensionPatchMethod,
            Self::ExternalProcessorInitialize(_) => WriterCommandKind::ExternalProcessorInitialize,
            Self::ExternalReportInitialize(_) => WriterCommandKind::ExternalReportInitialize,
            Self::MetadataCreate(_) => WriterCommandKind::MetadataCreate,
            Self::MetadataEdit(_) => WriterCommandKind::MetadataEdit,
            Self::MetadataRemove(_) => WriterCommandKind::MetadataRemove,
            Self::FormCreate(_) => WriterCommandKind::FormCreate,
            Self::FormCompile(_) => WriterCommandKind::FormCompile,
            Self::FormEdit(_) => WriterCommandKind::FormEdit,
            Self::FormRemove(_) => WriterCommandKind::FormRemove,
            Self::TemplateCreate(_) => WriterCommandKind::TemplateCreate,
            Self::TemplateRemove(_) => WriterCommandKind::TemplateRemove,
            Self::HelpCreate(_) => WriterCommandKind::HelpCreate,
            Self::InterfaceEdit(_) => WriterCommandKind::InterfaceEdit,
            Self::RoleCreate(_) => WriterCommandKind::RoleCreate,
            Self::SubsystemCreate(_) => WriterCommandKind::SubsystemCreate,
            Self::SubsystemEdit(_) => WriterCommandKind::SubsystemEdit,
            Self::SupportEdit(_) => WriterCommandKind::SupportEdit,
            Self::DataCompositionCreate(_) => WriterCommandKind::DataCompositionCreate,
            Self::DataCompositionEdit(_) => WriterCommandKind::DataCompositionEdit,
            Self::SpreadsheetCreate(_) => WriterCommandKind::SpreadsheetCreate,
        }
    }

    pub const fn family(&self) -> WriterFamily {
        match self {
            Self::ConfigurationInitialize(_) | Self::ConfigurationEdit(_) => {
                WriterFamily::Configuration
            }
            Self::ExtensionInitialize(_)
            | Self::ExtensionBorrow(_)
            | Self::ExtensionPatchMethod(_) => WriterFamily::Extension,
            Self::ExternalProcessorInitialize(_) | Self::ExternalReportInitialize(_) => {
                WriterFamily::ExternalArtifact
            }
            Self::MetadataCreate(_) | Self::MetadataEdit(_) | Self::MetadataRemove(_) => {
                WriterFamily::Metadata
            }
            Self::FormCreate(_)
            | Self::FormCompile(_)
            | Self::FormEdit(_)
            | Self::FormRemove(_) => WriterFamily::Form,
            Self::TemplateCreate(_) | Self::TemplateRemove(_) => WriterFamily::Template,
            Self::HelpCreate(_) => WriterFamily::Help,
            Self::InterfaceEdit(_) => WriterFamily::Interface,
            Self::RoleCreate(_) => WriterFamily::Role,
            Self::SubsystemCreate(_) | Self::SubsystemEdit(_) => WriterFamily::Subsystem,
            Self::SupportEdit(_) => WriterFamily::Support,
            Self::DataCompositionCreate(_) | Self::DataCompositionEdit(_) => {
                WriterFamily::DataComposition
            }
            Self::SpreadsheetCreate(_) => WriterFamily::Spreadsheet,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WriterFailureKind {
    InvalidRequest,
    UnsupportedState,
    GuardRejected,
    Conflict,
    Validation,
    Planning,
    Publication,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriterInterruption {
    cancellation: PublicationCancellation,
    rollback: PublicationRollback,
    cleanup: PublicationCleanup,
    recovery: PublicationRecovery,
}

impl WriterInterruption {
    pub fn new(
        cancellation: PublicationCancellation,
        rollback: PublicationRollback,
        cleanup: PublicationCleanup,
        recovery: PublicationRecovery,
    ) -> Result<Self, OperationalContractError> {
        let valid = cancellation != PublicationCancellation::NotRequested
            && cleanup == PublicationCleanup::Completed
            && recovery == PublicationRecovery::NotRequired
            && match cancellation {
                PublicationCancellation::DuringPublication => {
                    rollback == PublicationRollback::Performed
                }
                _ => rollback == PublicationRollback::NotNeeded,
            };
        if !valid {
            return Err(OperationalContractError::InvalidStateCombination);
        }
        Ok(Self {
            cancellation,
            rollback,
            cleanup,
            recovery,
        })
    }
    pub const fn cancellation(self) -> PublicationCancellation {
        self.cancellation
    }
    pub const fn rollback(self) -> PublicationRollback {
        self.rollback
    }
    pub const fn cleanup(self) -> PublicationCleanup {
        self.cleanup
    }
    pub const fn recovery(self) -> PublicationRecovery {
        self.recovery
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriterFailure {
    kind: WriterFailureKind,
    rollback: PublicationRollback,
    cleanup: PublicationCleanup,
    recovery: PublicationRecovery,
}

impl WriterFailure {
    pub fn new(
        kind: WriterFailureKind,
        rollback: PublicationRollback,
        cleanup: PublicationCleanup,
        recovery: PublicationRecovery,
    ) -> Result<Self, OperationalContractError> {
        if recovery == PublicationRecovery::Required
            && cleanup != PublicationCleanup::RetainedForRecovery
        {
            return Err(OperationalContractError::InvalidStateCombination);
        }
        Ok(Self {
            kind,
            rollback,
            cleanup,
            recovery,
        })
    }
    pub const fn kind(self) -> WriterFailureKind {
        self.kind
    }
    pub const fn rollback(self) -> PublicationRollback {
        self.rollback
    }
    pub const fn cleanup(self) -> PublicationCleanup {
        self.cleanup
    }
    pub const fn recovery(self) -> PublicationRecovery {
        self.recovery
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state", content = "detail")]
pub enum WriterLifecycle {
    Previewed,
    Applied,
    Rejected(WriterFailure),
    Cancelled(WriterInterruption),
}

impl WriterLifecycle {
    pub fn rejected(kind: WriterFailureKind) -> Self {
        Self::Rejected(
            WriterFailure::new(
                kind,
                PublicationRollback::NotNeeded,
                PublicationCleanup::Completed,
                PublicationRecovery::NotRequired,
            )
            .expect("rejected lifecycle is valid"),
        )
    }

    pub fn cancelled_before_execution() -> Self {
        Self::Cancelled(
            WriterInterruption::new(
                PublicationCancellation::BeforeExecution,
                PublicationRollback::NotNeeded,
                PublicationCleanup::Completed,
                PublicationRecovery::NotRequired,
            )
            .expect("pre-execution cancellation lifecycle is valid"),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SemanticChange {
    SourceCreated,
    SourceUpdated,
    SourceRemoved,
    RegistrationUpdated,
    SupportUpdated,
    ModuleUpdated,
    NoChange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SemanticArtifact {
    Configuration,
    Extension,
    ExternalProcessor,
    ExternalReport,
    MetadataObject,
    Form,
    Template,
    Help,
    Interface,
    Role,
    Subsystem,
    SupportState,
    DataComposition,
    Spreadsheet,
    Module,
    RecoveryState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticCode {
    Cancelled,
    InvalidRequest,
    InvalidDefinition,
    NotFound,
    AlreadyExists,
    UnsupportedState,
    UnsupportedFormat,
    AuthorabilityBlocked,
    SupportBlocked,
    NoDowngrade,
    Conflict,
    ValidationFailed,
    PlannerRejected,
    OwnerResolutionFailed,
    PublicationFailed,
    RollbackFailed,
    RecoveryRequired,
    PathRejected,
    ReadOnlyArtifact,
    AliasedArtifact,
    InvalidMutation,
    InvalidObjectReference,
    UnknownObjectKind,
    MissingFormCompanion,
    SupportCapabilityDisabled,
    InvalidModuleReference,
    ObjectNotBorrowed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticField {
    Name,
    Owner,
    Module,
    Method,
    Definition,
    Mutation,
    SupportRule,
    Artifact,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "value")]
pub enum DiagnosticDetail {
    Field(DiagnosticField),
    ObjectKind(SemanticObjectKind),
    Object(MetadataObjectReference),
    MetadataKind(MetadataKindName),
    Method(MethodName),
    FormElement(FormElementName),
    ConflictCount(u32),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriterDiagnostic {
    code: DiagnosticCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<DiagnosticDetail>,
}

impl WriterDiagnostic {
    pub const fn new(code: DiagnosticCode, detail: Option<DiagnosticDetail>) -> Self {
        Self { code, detail }
    }
    pub const fn code(&self) -> DiagnosticCode {
        self.code
    }
    pub const fn detail(&self) -> Option<&DiagnosticDetail> {
        self.detail.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "result")]
pub enum WriterEvidence {
    FormEdit(FormEditEvidence),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormEditEvidence {
    changed: bool,
    removed: Vec<FormEditRemoval>,
    validation: FormEditValidation,
}

impl FormEditEvidence {
    pub fn new(
        changed: bool,
        removed: Vec<FormEditRemoval>,
        validation: FormEditValidation,
    ) -> Self {
        Self {
            changed,
            removed,
            validation,
        }
    }
    pub const fn changed(&self) -> bool {
        self.changed
    }
    pub fn removed(&self) -> &[FormEditRemoval] {
        &self.removed
    }
    pub const fn validation(&self) -> FormEditValidation {
        self.validation
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormEditRemoval {
    name: FormElementName,
    #[serde(rename = "kind")]
    element_kind: FormElementKind,
    reason: FormEditRemovalReason,
}

impl FormEditRemoval {
    pub const fn new(
        name: FormElementName,
        element_kind: FormElementKind,
        reason: FormEditRemovalReason,
    ) -> Self {
        Self {
            name,
            element_kind,
            reason,
        }
    }
    pub const fn name(&self) -> &FormElementName {
        &self.name
    }
    pub const fn element_kind(&self) -> FormElementKind {
        self.element_kind
    }
    pub const fn reason(&self) -> FormEditRemovalReason {
        self.reason
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FormElementKind {
    Element,
    Input,
    ContextMenu,
    Tooltip,
    Group,
    Table,
    Button,
    CommandBar,
    Attribute,
    Command,
    Parameter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FormEditRemovalReason {
    Requested,
    Contained,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FormEditValidation {
    Passed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WriterMessageCode {
    Applied,
    PreviewPlanned,
    NoChange,
    Rejected,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriterResult {
    lifecycle: WriterLifecycle,
    message_code: WriterMessageCode,
    changes: Vec<SemanticChange>,
    artifacts: Vec<SemanticArtifact>,
    diagnostics: Vec<WriterDiagnostic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    evidence: Option<WriterEvidence>,
}

impl WriterResult {
    pub fn new(
        lifecycle: WriterLifecycle,
        changes: impl IntoIterator<Item = SemanticChange>,
        artifacts: impl IntoIterator<Item = SemanticArtifact>,
        diagnostics: impl IntoIterator<Item = DiagnosticCode>,
    ) -> Result<Self, OperationalContractError> {
        Self::with_diagnostics(
            lifecycle,
            changes,
            artifacts,
            diagnostics
                .into_iter()
                .map(|code| WriterDiagnostic::new(code, None)),
        )
    }

    pub fn with_diagnostics(
        lifecycle: WriterLifecycle,
        changes: impl IntoIterator<Item = SemanticChange>,
        artifacts: impl IntoIterator<Item = SemanticArtifact>,
        diagnostics: impl IntoIterator<Item = WriterDiagnostic>,
    ) -> Result<Self, OperationalContractError> {
        let changes = changes.into_iter().collect::<Vec<_>>();
        let artifacts = artifacts.into_iter().collect::<Vec<_>>();
        let diagnostics = diagnostics.into_iter().collect::<Vec<_>>();
        let valid = match lifecycle {
            WriterLifecycle::Previewed => {
                artifacts.is_empty()
                    && diagnostics.is_empty()
                    && !changes.is_empty()
                    && (changes == [SemanticChange::NoChange]
                        || !changes.contains(&SemanticChange::NoChange))
            }
            WriterLifecycle::Applied => {
                diagnostics.is_empty()
                    && !changes.is_empty()
                    && !artifacts.contains(&SemanticArtifact::RecoveryState)
            }
            WriterLifecycle::Rejected(failure) => {
                changes.is_empty()
                    && artifacts
                        == if failure.recovery() == PublicationRecovery::Required {
                            vec![SemanticArtifact::RecoveryState]
                        } else {
                            Vec::new()
                        }
                    && !diagnostics.is_empty()
                    && !diagnostics
                        .iter()
                        .any(|item| item.code() == DiagnosticCode::Cancelled)
            }
            WriterLifecycle::Cancelled(_) => {
                changes.is_empty()
                    && artifacts.is_empty()
                    && diagnostics.len() == 1
                    && diagnostics[0].code() == DiagnosticCode::Cancelled
            }
        };
        if !valid {
            return Err(OperationalContractError::InvalidStateCombination);
        }
        let message_code = match lifecycle {
            WriterLifecycle::Previewed if changes == [SemanticChange::NoChange] => {
                WriterMessageCode::NoChange
            }
            WriterLifecycle::Previewed => WriterMessageCode::PreviewPlanned,
            WriterLifecycle::Applied if changes == [SemanticChange::NoChange] => {
                WriterMessageCode::NoChange
            }
            WriterLifecycle::Applied => WriterMessageCode::Applied,
            WriterLifecycle::Rejected(_) => WriterMessageCode::Rejected,
            WriterLifecycle::Cancelled(_) => WriterMessageCode::Cancelled,
        };
        Ok(Self {
            lifecycle,
            message_code,
            changes,
            artifacts,
            diagnostics,
            evidence: None,
        })
    }

    pub fn previewed(changed: bool) -> Self {
        Self::new(
            WriterLifecycle::Previewed,
            [if changed {
                SemanticChange::SourceUpdated
            } else {
                SemanticChange::NoChange
            }],
            [],
            [],
        )
        .expect("preview outcome is valid")
    }

    pub fn previewed_with_changes(
        changes: impl IntoIterator<Item = SemanticChange>,
    ) -> Result<Self, OperationalContractError> {
        Self::new(WriterLifecycle::Previewed, changes, [], [])
    }

    pub fn cancelled() -> Self {
        Self::new(
            WriterLifecycle::cancelled_before_execution(),
            [],
            [],
            [DiagnosticCode::Cancelled],
        )
        .expect("cancelled outcome is valid")
    }

    pub fn cancelled_during_execution() -> Self {
        Self::new(
            WriterLifecycle::Cancelled(
                WriterInterruption::new(
                    PublicationCancellation::DuringExecution,
                    PublicationRollback::NotNeeded,
                    PublicationCleanup::Completed,
                    PublicationRecovery::NotRequired,
                )
                .expect("execution cancellation lifecycle is valid"),
            ),
            [],
            [],
            [DiagnosticCode::Cancelled],
        )
        .expect("cancelled outcome is valid")
    }

    pub fn cancelled_during_publication() -> Self {
        Self::new(
            WriterLifecycle::Cancelled(
                WriterInterruption::new(
                    PublicationCancellation::DuringPublication,
                    PublicationRollback::Performed,
                    PublicationCleanup::Completed,
                    PublicationRecovery::NotRequired,
                )
                .expect("publication cancellation lifecycle is valid"),
            ),
            [],
            [],
            [DiagnosticCode::Cancelled],
        )
        .expect("cancelled outcome is valid")
    }

    pub fn publication_recovery_required() -> Self {
        Self::new(
            WriterLifecycle::Rejected(
                WriterFailure::new(
                    WriterFailureKind::Publication,
                    PublicationRollback::Failed,
                    PublicationCleanup::RetainedForRecovery,
                    PublicationRecovery::Required,
                )
                .expect("publication recovery lifecycle is valid"),
            ),
            [],
            [SemanticArtifact::RecoveryState],
            [DiagnosticCode::RecoveryRequired],
        )
        .expect("publication recovery result is valid")
    }

    pub fn rejected(code: DiagnosticCode, kind: WriterFailureKind) -> Self {
        Self::new(WriterLifecycle::rejected(kind), [], [], [code])
            .expect("rejected outcome is valid")
    }

    pub fn rejected_with_diagnostic(diagnostic: WriterDiagnostic, kind: WriterFailureKind) -> Self {
        Self::with_diagnostics(WriterLifecycle::rejected(kind), [], [], [diagnostic])
            .expect("rejected outcome is valid")
    }

    pub fn with_evidence(mut self, evidence: Option<WriterEvidence>) -> Self {
        self.evidence = evidence;
        self
    }

    pub const fn lifecycle(&self) -> WriterLifecycle {
        self.lifecycle
    }
    pub const fn message_code(&self) -> WriterMessageCode {
        self.message_code
    }
    pub fn changes(&self) -> &[SemanticChange] {
        &self.changes
    }
    pub fn artifacts(&self) -> &[SemanticArtifact] {
        &self.artifacts
    }
    pub fn diagnostics(&self) -> &[WriterDiagnostic] {
        &self.diagnostics
    }
    pub const fn evidence(&self) -> Option<&WriterEvidence> {
        self.evidence.as_ref()
    }
}
