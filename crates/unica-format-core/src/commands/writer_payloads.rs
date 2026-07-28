use serde::{Deserialize, Serialize};

use super::{validate_text, SemanticValueError};

semantic_text!(ConfigurationName, 128);
semantic_text!(ExtensionName, 128);
semantic_text!(ExternalArtifactName, 128);
semantic_text!(MetadataObjectReference, 256);
semantic_text!(MetadataKindName, 128);
semantic_text!(MetadataFieldName, 128);
semantic_text!(MetadataChildName, 128);
semantic_text!(MetadataPropertyText, 4096);
semantic_text!(MetadataTypeTargetName, 256);
semantic_text!(MetadataMethodReference, 256);
semantic_text!(MetadataEventName, 128);
semantic_text!(MetadataJobKey, 256);
semantic_text!(MetadataUrlRoot, 512);
semantic_text!(MetadataUrlTemplatePath, 2048);
semantic_text!(MetadataHttpMethodName, 64);
semantic_text!(MetadataServiceNamespace, 2048);
semantic_text!(MetadataServiceTypeName, 256);
semantic_text!(MetadataServiceOperationName, 128);
semantic_text!(MetadataServiceParameterName, 128);
semantic_text!(FormOwnerReference, 256);
semantic_text!(FormName, 128);
semantic_text!(FormAttributeName, 128);
semantic_text!(FormCommandName, 128);
semantic_text!(FormParameterName, 128);
semantic_text!(FormEventName, 128);
semantic_text!(FormHandlerName, 256);
semantic_text!(FormElementPath, 512);
semantic_text!(TemplateOwnerReference, 256);
semantic_text!(TemplateName, 128);
semantic_text!(TemplateText, 65_536);
semantic_text!(HelpOwnerReference, 256);
semantic_text!(HelpText, 65_536);
semantic_text!(RoleName, 128);
semantic_text!(RoleObjectReference, 256);
semantic_text!(RoleRestrictionText, 65_536);
semantic_text!(RoleTemplateName, 128);
semantic_text!(SubsystemName, 128);
semantic_text!(CommonModuleName, 128);
semantic_text!(MethodName, 128);
semantic_text!(SynonymText, 512);
semantic_text!(CommentText, 4096);
semantic_text!(DescriptionText, 4096);
semantic_text!(VendorName, 256);
semantic_text!(ArtifactVersion, 64);
semantic_text!(LanguageCode, 16);
semantic_text!(NamePrefix, 128);
semantic_text!(ConfigurationTextValue, 4096);
semantic_text!(InterfaceItemName, 256);
semantic_text!(InterfaceGroupName, 256);
semantic_text!(DataSetName, 128);
semantic_text!(DataSourceName, 128);
semantic_text!(DataFieldPath, 512);
semantic_text!(DataCompositionExpression, 65_536);
semantic_text!(DataCompositionQueryText, 65_536);
semantic_text!(DataCompositionParameterName, 128);
semantic_text!(VariantName, 128);
semantic_text!(ProcessorName, 128);
semantic_text!(FormElementName, 128);
semantic_text!(SpreadsheetAreaName, 128);
semantic_text!(SpreadsheetStyleName, 128);
semantic_text!(SpreadsheetFontName, 128);
semantic_text!(SpreadsheetCellText, 65_536);
semantic_text!(SpreadsheetNumberFormat, 256);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionNumber {
    components: Vec<u16>,
}

impl VersionNumber {
    pub fn new(components: Vec<u16>) -> Result<Self, SemanticValueError> {
        if !(2..=4).contains(&components.len()) || components.iter().all(|value| *value == 0) {
            return Err(SemanticValueError::InvalidCombination);
        }
        Ok(Self { components })
    }

    pub fn components(&self) -> &[u16] {
        &self.components
    }
}

impl<'de> Deserialize<'de> for VersionNumber {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            components: Vec<u16>,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.components).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "selection",
    content = "requirement",
    deny_unknown_fields
)]
pub enum CapabilityRequirement {
    Preserve,
    AdapterDefault,
    Explicit(VersionNumber),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfigurationProperty {
    Name,
    Synonym,
    Comment,
    Vendor,
    Version,
    DefaultLanguage,
    BriefInformation,
    DetailedInformation,
    Copyright,
    VendorInformationAddress,
    ConfigurationInformationAddress,
    UpdateCatalogAddress,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "kind",
    content = "value",
    deny_unknown_fields
)]
pub enum ConfigurationPropertyValue {
    Text(ConfigurationTextValue),
    Language(LanguageCode),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationPropertyPatch {
    property: ConfigurationProperty,
    value: ConfigurationPropertyValue,
}

impl ConfigurationPropertyPatch {
    pub fn new(
        property: ConfigurationProperty,
        value: ConfigurationPropertyValue,
    ) -> Result<Self, SemanticValueError> {
        let valid = match property {
            ConfigurationProperty::DefaultLanguage => {
                matches!(value, ConfigurationPropertyValue::Language(_))
            }
            _ => matches!(value, ConfigurationPropertyValue::Text(_)),
        };
        if !valid {
            return Err(SemanticValueError::InvalidCombination);
        }
        Ok(Self { property, value })
    }
    pub const fn property(&self) -> ConfigurationProperty {
        self.property
    }
    pub const fn value(&self) -> &ConfigurationPropertyValue {
        &self.value
    }
}

impl<'de> Deserialize<'de> for ConfigurationPropertyPatch {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            property: ConfigurationProperty,
            value: ConfigurationPropertyValue,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.property, wire.value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfigurationPanel {
    Sections,
    Functions,
    Favorites,
    History,
    Open,
    Tools,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigurationPanelPlacement {
    panel: ConfigurationPanel,
    order: u16,
    visible: bool,
}

impl ConfigurationPanelPlacement {
    pub const fn new(panel: ConfigurationPanel, order: u16, visible: bool) -> Self {
        Self {
            panel,
            order,
            visible,
        }
    }
    pub const fn panel(&self) -> ConfigurationPanel {
        self.panel
    }
    pub const fn order(&self) -> u16 {
        self.order
    }
    pub const fn visible(&self) -> bool {
        self.visible
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigurationHomePageItem {
    object: MetadataObjectReference,
    order: u16,
}

impl ConfigurationHomePageItem {
    pub const fn new(object: MetadataObjectReference, order: u16) -> Self {
        Self { object, order }
    }
    pub const fn object(&self) -> &MetadataObjectReference {
        &self.object
    }
    pub const fn order(&self) -> u16 {
        self.order
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfigurationPanelSide {
    Top,
    Left,
    Right,
    Bottom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "kind",
    content = "value",
    deny_unknown_fields
)]
pub enum ConfigurationPanelEntry {
    Panel(ConfigurationPanel),
    Group(#[serde(deserialize_with = "deserialize_non_empty_vec")] Vec<ConfigurationPanelEntry>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigurationPanelSection {
    side: ConfigurationPanelSide,
    #[serde(deserialize_with = "deserialize_non_empty_vec")]
    entries: Vec<ConfigurationPanelEntry>,
}

impl ConfigurationPanelSection {
    pub fn new(
        side: ConfigurationPanelSide,
        entries: Vec<ConfigurationPanelEntry>,
    ) -> Result<Self, SemanticValueError> {
        if entries.is_empty() {
            return Err(SemanticValueError::Empty);
        }
        Ok(Self { side, entries })
    }
    pub const fn side(&self) -> ConfigurationPanelSide {
        self.side
    }
    pub fn entries(&self) -> &[ConfigurationPanelEntry] {
        &self.entries
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigurationPanelLayout {
    #[serde(deserialize_with = "deserialize_non_empty_vec")]
    sections: Vec<ConfigurationPanelSection>,
}

impl ConfigurationPanelLayout {
    pub fn new(sections: Vec<ConfigurationPanelSection>) -> Result<Self, SemanticValueError> {
        if sections.is_empty() {
            return Err(SemanticValueError::Empty);
        }
        Ok(Self { sections })
    }
    pub fn sections(&self) -> &[ConfigurationPanelSection] {
        &self.sections
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfigurationHomePageTemplate {
    OneColumn,
    TwoColumnsEqualWidth,
    TwoColumnsVariableWidth,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigurationHomePageRoleVisibility {
    role: RoleName,
    visible: bool,
}

impl ConfigurationHomePageRoleVisibility {
    pub const fn new(role: RoleName, visible: bool) -> Self {
        Self { role, visible }
    }
    pub const fn role(&self) -> &RoleName {
        &self.role
    }
    pub const fn visible(&self) -> bool {
        self.visible
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigurationHomePageEntry {
    form: MetadataObjectReference,
    #[serde(deserialize_with = "deserialize_non_zero_u16")]
    height: u16,
    visible: bool,
    role_visibility: Vec<ConfigurationHomePageRoleVisibility>,
}

impl ConfigurationHomePageEntry {
    pub const fn new(form: MetadataObjectReference) -> Self {
        Self {
            form,
            height: 10,
            visible: true,
            role_visibility: Vec::new(),
        }
    }
    pub fn with_height(mut self, value: u16) -> Result<Self, SemanticValueError> {
        if value == 0 {
            return Err(SemanticValueError::Empty);
        }
        self.height = value;
        Ok(self)
    }
    pub const fn visible(mut self, value: bool) -> Self {
        self.visible = value;
        self
    }
    pub fn with_role_visibility(mut self, value: Vec<ConfigurationHomePageRoleVisibility>) -> Self {
        self.role_visibility = value;
        self
    }
    pub const fn form(&self) -> &MetadataObjectReference {
        &self.form
    }
    pub const fn height(&self) -> u16 {
        self.height
    }
    pub const fn is_visible(&self) -> bool {
        self.visible
    }
    pub fn role_visibility(&self) -> &[ConfigurationHomePageRoleVisibility] {
        &self.role_visibility
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationHomePageLayout {
    template: ConfigurationHomePageTemplate,
    left: Vec<ConfigurationHomePageEntry>,
    right: Vec<ConfigurationHomePageEntry>,
}

impl ConfigurationHomePageLayout {
    pub fn new(
        template: ConfigurationHomePageTemplate,
        left: Vec<ConfigurationHomePageEntry>,
        right: Vec<ConfigurationHomePageEntry>,
    ) -> Result<Self, SemanticValueError> {
        if template == ConfigurationHomePageTemplate::OneColumn && !right.is_empty() {
            return Err(SemanticValueError::Empty);
        }
        Ok(Self {
            template,
            left,
            right,
        })
    }
    pub const fn template(&self) -> ConfigurationHomePageTemplate {
        self.template
    }
    pub fn left(&self) -> &[ConfigurationHomePageEntry] {
        &self.left
    }
    pub fn right(&self) -> &[ConfigurationHomePageEntry] {
        &self.right
    }
}

impl<'de> Deserialize<'de> for ConfigurationHomePageLayout {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            template: ConfigurationHomePageTemplate,
            #[serde(default)]
            left: Vec<ConfigurationHomePageEntry>,
            #[serde(default)]
            right: Vec<ConfigurationHomePageEntry>,
        }

        let value = Wire::deserialize(deserializer)?;
        Self::new(value.template, value.left, value.right).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "operation",
    content = "payload",
    deny_unknown_fields
)]
pub enum ConfigurationMutation {
    SetProperty(ConfigurationPropertyPatch),
    RemoveChild(MetadataObjectReference),
    AddChild(MetadataObjectReference),
    SetDefaultRoles(Vec<RoleName>),
    AddDefaultRole(RoleName),
    RemoveDefaultRole(RoleName),
    SetPanels(Vec<ConfigurationPanelPlacement>),
    SetHomePage(Vec<ConfigurationHomePageItem>),
    SetPanelLayout(ConfigurationPanelLayout),
    SetHomePageLayout(ConfigurationHomePageLayout),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigurationPatchSet {
    #[serde(deserialize_with = "deserialize_non_empty_vec")]
    operations: Vec<ConfigurationMutation>,
}

impl ConfigurationPatchSet {
    pub fn new(operations: Vec<ConfigurationMutation>) -> Result<Self, SemanticValueError> {
        if operations.is_empty() {
            return Err(SemanticValueError::Empty);
        }
        Ok(Self { operations })
    }
    pub fn operations(&self) -> &[ConfigurationMutation] {
        &self.operations
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigurationInitialize {
    name: ConfigurationName,
    synonym: Option<SynonymText>,
    vendor: Option<VendorName>,
    version: Option<ArtifactVersion>,
    omit_default_role: bool,
    compatibility: CapabilityRequirement,
}

impl ConfigurationInitialize {
    pub const fn new(name: ConfigurationName) -> Self {
        Self {
            name,
            synonym: None,
            vendor: None,
            version: None,
            omit_default_role: false,
            compatibility: CapabilityRequirement::AdapterDefault,
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
    pub fn with_compatibility(mut self, value: CapabilityRequirement) -> Self {
        self.compatibility = value;
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
    pub const fn compatibility(&self) -> &CapabilityRequirement {
        &self.compatibility
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "source",
    content = "patch",
    deny_unknown_fields
)]
pub enum ConfigurationEdit {
    Patch(ConfigurationPatchSet),
}

impl ConfigurationEdit {
    pub fn mutate(mutation: ConfigurationMutation) -> Self {
        Self::Patch(ConfigurationPatchSet {
            operations: vec![mutation],
        })
    }
    pub const fn from_patch(patch: ConfigurationPatchSet) -> Self {
        Self::Patch(patch)
    }
    pub fn operations(&self) -> &[ConfigurationMutation] {
        match self {
            Self::Patch(value) => value.operations(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BorrowScope {
    None,
    Form,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BorrowSelection {
    Include,
    Exclude,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BorrowConflictPolicy {
    Reject,
    Replace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InterceptorKind {
    Before,
    After,
    Instead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExecutionContext {
    Automatic,
    Client,
    Server,
    ServerWithoutContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtensionInitialize {
    name: ExtensionName,
    synonym: Option<SynonymText>,
    purpose: Option<ExtensionPurpose>,
    prefix: Option<NamePrefix>,
    vendor: Option<VendorName>,
    version: Option<ArtifactVersion>,
    omit_default_role: bool,
    compatibility: CapabilityRequirement,
}

impl ExtensionInitialize {
    pub const fn new(name: ExtensionName) -> Self {
        Self {
            name,
            synonym: None,
            purpose: None,
            prefix: None,
            vendor: None,
            version: None,
            omit_default_role: false,
            compatibility: CapabilityRequirement::Preserve,
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
    pub fn with_compatibility(mut self, value: CapabilityRequirement) -> Self {
        self.compatibility = value;
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
    pub const fn vendor(&self) -> Option<&VendorName> {
        self.vendor.as_ref()
    }
    pub const fn version(&self) -> Option<&ArtifactVersion> {
        self.version.as_ref()
    }
    pub const fn omits_default_role(&self) -> bool {
        self.omit_default_role
    }
    pub const fn compatibility(&self) -> &CapabilityRequirement {
        &self.compatibility
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtensionBorrow {
    object: MetadataObjectReference,
    main_attribute: BorrowScope,
    selection: BorrowSelection,
    conflict: BorrowConflictPolicy,
}

impl ExtensionBorrow {
    pub const fn new(object: MetadataObjectReference) -> Self {
        Self {
            object,
            main_attribute: BorrowScope::None,
            selection: BorrowSelection::Include,
            conflict: BorrowConflictPolicy::Reject,
        }
    }
    pub const fn with_main_attribute(mut self, value: Option<BorrowScope>) -> Self {
        self.main_attribute = match value {
            Some(value) => value,
            None => BorrowScope::None,
        };
        self
    }
    pub const fn exclude_selection(mut self, value: bool) -> Self {
        self.selection = if value {
            BorrowSelection::Exclude
        } else {
            BorrowSelection::Include
        };
        self
    }
    pub const fn force(mut self, value: bool) -> Self {
        self.conflict = if value {
            BorrowConflictPolicy::Replace
        } else {
            BorrowConflictPolicy::Reject
        };
        self
    }
    pub const fn object(&self) -> &MetadataObjectReference {
        &self.object
    }
    pub const fn main_attribute(&self) -> Option<BorrowScope> {
        match self.main_attribute {
            BorrowScope::None => None,
            value => Some(value),
        }
    }
    pub const fn excludes_selection(&self) -> bool {
        matches!(self.selection, BorrowSelection::Exclude)
    }
    pub const fn is_forced(&self) -> bool {
        matches!(self.conflict, BorrowConflictPolicy::Replace)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CallableKind {
    Procedure,
    Function,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExtensionObjectModuleRole {
    Object,
    Manager,
    RecordSet,
    ValueManager,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", deny_unknown_fields)]
pub enum ExtensionModuleTarget {
    Common {
        module: CommonModuleName,
    },
    Object {
        owner: MetadataObjectReference,
        role: ExtensionObjectModuleRole,
    },
    Form {
        owner: MetadataObjectReference,
        form: FormName,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtensionPatchMethod {
    module: ExtensionModuleTarget,
    method: MethodName,
    interceptor: InterceptorKind,
    context: ExecutionContext,
    callable: CallableKind,
}

impl ExtensionPatchMethod {
    pub const fn new(
        module: ExtensionModuleTarget,
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
            callable: if function {
                CallableKind::Function
            } else {
                CallableKind::Procedure
            },
        }
    }
    pub const fn module(&self) -> &ExtensionModuleTarget {
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
        matches!(self.callable, CallableKind::Function)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExternalArtifactKind {
    Processor,
    Report,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalFormDefinition {
    name: FormName,
    purpose: FormPurpose,
}

impl ExternalFormDefinition {
    pub const fn new(name: FormName, purpose: FormPurpose) -> Self {
        Self { name, purpose }
    }
    pub const fn name(&self) -> &FormName {
        &self.name
    }
    pub const fn purpose(&self) -> FormPurpose {
        self.purpose
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalArtifactInitialize {
    name: ExternalArtifactName,
    synonym: Option<SynonymText>,
    primary_form: Option<ExternalFormDefinition>,
}

impl ExternalArtifactInitialize {
    pub const fn new(name: ExternalArtifactName) -> Self {
        Self {
            name,
            synonym: None,
            primary_form: None,
        }
    }
    pub fn with_synonym(mut self, value: Option<SynonymText>) -> Self {
        self.synonym = value;
        self
    }
    pub fn with_form(mut self, value: Option<FormName>) -> Self {
        self.primary_form =
            value.map(|name| ExternalFormDefinition::new(name, FormPurpose::Object));
        self
    }
    pub const fn name(&self) -> &ExternalArtifactName {
        &self.name
    }
    pub const fn synonym(&self) -> Option<&SynonymText> {
        self.synonym.as_ref()
    }
    pub const fn form(&self) -> Option<&FormName> {
        match self.primary_form.as_ref() {
            Some(value) => Some(value.name()),
            None => None,
        }
    }
    pub const fn primary_form(&self) -> Option<&ExternalFormDefinition> {
        self.primary_form.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataKind {
    CommonModule,
    SessionParameter,
    Role,
    CommonAttribute,
    ExchangePlan,
    XdtoPackage,
    WebService,
    HttpService,
    WsReference,
    StyleItem,
    CommonPicture,
    CommonTemplate,
    FilterCriterion,
    EventSubscription,
    ScheduledJob,
    FunctionalOption,
    FunctionalOptionsParameter,
    DefinedType,
    SettingsStorage,
    Language,
    CommandGroup,
    CommonCommand,
    DocumentNumerator,
    Sequence,
    Constant,
    Catalog,
    Document,
    Enum,
    Report,
    DataProcessor,
    ChartOfCharacteristicTypes,
    ChartOfAccounts,
    ChartOfCalculationTypes,
    InformationRegister,
    AccumulationRegister,
    AccountingRegister,
    CalculationRegister,
    BusinessProcess,
    Task,
    DocumentJournal,
}

impl MetadataKind {
    pub const ALL: [Self; 40] = [
        Self::CommonModule,
        Self::SessionParameter,
        Self::Role,
        Self::CommonAttribute,
        Self::ExchangePlan,
        Self::XdtoPackage,
        Self::WebService,
        Self::HttpService,
        Self::WsReference,
        Self::StyleItem,
        Self::CommonPicture,
        Self::CommonTemplate,
        Self::FilterCriterion,
        Self::EventSubscription,
        Self::ScheduledJob,
        Self::FunctionalOption,
        Self::FunctionalOptionsParameter,
        Self::DefinedType,
        Self::SettingsStorage,
        Self::Language,
        Self::CommandGroup,
        Self::CommonCommand,
        Self::DocumentNumerator,
        Self::Sequence,
        Self::Constant,
        Self::Catalog,
        Self::Document,
        Self::Enum,
        Self::Report,
        Self::DataProcessor,
        Self::ChartOfCharacteristicTypes,
        Self::ChartOfAccounts,
        Self::ChartOfCalculationTypes,
        Self::InformationRegister,
        Self::AccumulationRegister,
        Self::AccountingRegister,
        Self::CalculationRegister,
        Self::BusinessProcess,
        Self::Task,
        Self::DocumentJournal,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataValueType {
    String,
    Number,
    Boolean,
    Date,
    Uuid,
    Binary,
    ValueStorage,
    Any,
    CatalogReference,
    DocumentReference,
    EnumReference,
    DefinedType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataReferenceKind {
    Catalog,
    Document,
    Enum,
    DefinedType,
    ChartOfCharacteristicTypes,
    ChartOfAccounts,
    ChartOfCalculationTypes,
    BusinessProcess,
    Task,
    ExchangePlan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "kind",
    content = "value",
    deny_unknown_fields
)]
pub enum MetadataTypeExpression {
    String {
        length: Option<u32>,
    },
    Number {
        length: Option<u32>,
        precision: Option<u16>,
        nonnegative: bool,
    },
    Boolean,
    Date,
    Uuid,
    Binary,
    ValueStorage,
    Any,
    Reference {
        category: MetadataReferenceKind,
        target: MetadataTypeTargetName,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataFieldFlag {
    Required,
    Index,
    Master,
    MainFilter,
    DenyIncomplete,
    MultiLine,
    ExcludeFromTotals,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataIndexing {
    None,
    Index,
    IndexWithAdditionalOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataFillChecking {
    None,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataChoiceHistory {
    Automatic,
    Use,
    DoNotUse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataModuleContext {
    Client,
    Server,
    ClientAndServer,
    ExternalConnection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataReturnValuesReuse {
    DoNotUse,
    DuringRequest,
    DuringSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataHierarchyType {
    GroupsAndItems,
    Items,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataPeriodicity {
    Nonperiodical,
    Second,
    Day,
    Month,
    Quarter,
    Year,
    RecorderPosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataRegisterKind {
    Balance,
    Turnovers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataCalculationDependence {
    DoNotUse,
    OnActionPeriod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataSessionReuse {
    DoNotUse,
    Automatic,
    Use,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataServiceParameterDirection {
    Input,
    Output,
    InputOutput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataHttpMethodDefinition {
    name: MetadataHttpMethodName,
    method: MetadataHttpMethodName,
}

impl MetadataHttpMethodDefinition {
    pub const fn new(name: MetadataHttpMethodName, method: MetadataHttpMethodName) -> Self {
        Self { name, method }
    }
    pub const fn name(&self) -> &MetadataHttpMethodName {
        &self.name
    }
    pub const fn method(&self) -> &MetadataHttpMethodName {
        &self.method
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataUrlTemplateDefinition {
    name: MetadataChildName,
    template: MetadataUrlTemplatePath,
    methods: Vec<MetadataHttpMethodDefinition>,
}

impl MetadataUrlTemplateDefinition {
    pub const fn new(
        name: MetadataChildName,
        template: MetadataUrlTemplatePath,
        methods: Vec<MetadataHttpMethodDefinition>,
    ) -> Self {
        Self {
            name,
            template,
            methods,
        }
    }
    pub const fn name(&self) -> &MetadataChildName {
        &self.name
    }
    pub const fn template(&self) -> &MetadataUrlTemplatePath {
        &self.template
    }
    pub fn methods(&self) -> &[MetadataHttpMethodDefinition] {
        &self.methods
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataServiceParameterDefinition {
    name: MetadataServiceParameterName,
    value_type: MetadataServiceTypeName,
    nillable: bool,
    direction: MetadataServiceParameterDirection,
}

impl MetadataServiceParameterDefinition {
    pub const fn new(
        name: MetadataServiceParameterName,
        value_type: MetadataServiceTypeName,
    ) -> Self {
        Self {
            name,
            value_type,
            nillable: true,
            direction: MetadataServiceParameterDirection::Input,
        }
    }
    pub const fn name(&self) -> &MetadataServiceParameterName {
        &self.name
    }
    pub const fn value_type(&self) -> &MetadataServiceTypeName {
        &self.value_type
    }
    pub const fn nillable(&self) -> bool {
        self.nillable
    }
    pub const fn direction(&self) -> MetadataServiceParameterDirection {
        self.direction
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataServiceOperationDefinition {
    name: MetadataServiceOperationName,
    return_type: MetadataServiceTypeName,
    nillable: bool,
    transactioned: bool,
    handler: MetadataMethodReference,
    parameters: Vec<MetadataServiceParameterDefinition>,
}

impl MetadataServiceOperationDefinition {
    pub const fn new(
        name: MetadataServiceOperationName,
        return_type: MetadataServiceTypeName,
        handler: MetadataMethodReference,
        parameters: Vec<MetadataServiceParameterDefinition>,
    ) -> Self {
        Self {
            name,
            return_type,
            nillable: false,
            transactioned: false,
            handler,
            parameters,
        }
    }
    pub const fn name(&self) -> &MetadataServiceOperationName {
        &self.name
    }
    pub const fn return_type(&self) -> &MetadataServiceTypeName {
        &self.return_type
    }
    pub const fn nillable(&self) -> bool {
        self.nillable
    }
    pub const fn transactioned(&self) -> bool {
        self.transactioned
    }
    pub const fn handler(&self) -> &MetadataMethodReference {
        &self.handler
    }
    pub fn parameters(&self) -> &[MetadataServiceParameterDefinition] {
        &self.parameters
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataKindPropertyName {
    Hierarchical,
    LimitLevelCount,
    LevelCount,
    FoldersOnTop,
    CodeLength,
    DescriptionLength,
    NumberLength,
    CheckUnique,
    Autonumbering,
    QuickChoice,
    SequenceFilling,
    PostInPrivilegedMode,
    UnpostInPrivilegedMode,
    MainFilterOnPeriod,
    EnableTotalsSplitting,
    Correspondence,
    PeriodAdjustmentLength,
    ActionPeriod,
    BasePeriod,
    MaxExtDimensionCount,
    CodeMask,
    AutoOrderByCode,
    OrderLength,
    ActionPeriodUse,
    DistributedInfoBase,
    IncludeConfigurationExtensions,
    RestartCountOnFailure,
    RestartIntervalOnFailure,
    SessionMaxAge,
    Length,
    Precision,
    Nonnegative,
    CreateTaskInPrivilegedMode,
    ValueType,
    ValueTypes,
    Context,
    ReturnValuesReuse,
    HierarchyType,
    Periodicity,
    RegisterType,
    ChartOfAccounts,
    ChartOfCalculationTypes,
    ExtDimensionTypes,
    AccountingFlags,
    ExtDimensionAccountingFlags,
    DependenceOnCalculationTypes,
    BaseCalculationTypes,
    Task,
    Addressing,
    MainAddressingAttribute,
    RegisteredDocuments,
    MethodName,
    Description,
    Key,
    Use,
    Predefined,
    Source,
    Event,
    Handler,
    RootUrl,
    ReuseSessions,
    UrlTemplates,
    Namespace,
    Operations,
}

impl MetadataKindPropertyName {
    pub const ALL: [Self; 64] = [
        Self::Hierarchical,
        Self::LimitLevelCount,
        Self::LevelCount,
        Self::FoldersOnTop,
        Self::CodeLength,
        Self::DescriptionLength,
        Self::NumberLength,
        Self::CheckUnique,
        Self::Autonumbering,
        Self::QuickChoice,
        Self::SequenceFilling,
        Self::PostInPrivilegedMode,
        Self::UnpostInPrivilegedMode,
        Self::MainFilterOnPeriod,
        Self::EnableTotalsSplitting,
        Self::Correspondence,
        Self::PeriodAdjustmentLength,
        Self::ActionPeriod,
        Self::BasePeriod,
        Self::MaxExtDimensionCount,
        Self::CodeMask,
        Self::AutoOrderByCode,
        Self::OrderLength,
        Self::ActionPeriodUse,
        Self::DistributedInfoBase,
        Self::IncludeConfigurationExtensions,
        Self::RestartCountOnFailure,
        Self::RestartIntervalOnFailure,
        Self::SessionMaxAge,
        Self::Length,
        Self::Precision,
        Self::Nonnegative,
        Self::CreateTaskInPrivilegedMode,
        Self::ValueType,
        Self::ValueTypes,
        Self::Context,
        Self::ReturnValuesReuse,
        Self::HierarchyType,
        Self::Periodicity,
        Self::RegisterType,
        Self::ChartOfAccounts,
        Self::ChartOfCalculationTypes,
        Self::ExtDimensionTypes,
        Self::AccountingFlags,
        Self::ExtDimensionAccountingFlags,
        Self::DependenceOnCalculationTypes,
        Self::BaseCalculationTypes,
        Self::Task,
        Self::Addressing,
        Self::MainAddressingAttribute,
        Self::RegisteredDocuments,
        Self::MethodName,
        Self::Description,
        Self::Key,
        Self::Use,
        Self::Predefined,
        Self::Source,
        Self::Event,
        Self::Handler,
        Self::RootUrl,
        Self::ReuseSessions,
        Self::UrlTemplates,
        Self::Namespace,
        Self::Operations,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "kind",
    content = "value",
    deny_unknown_fields
)]
pub enum MetadataPropertyValue {
    Name(MetadataChildName),
    Synonym(SynonymText),
    Comment(CommentText),
    Text(MetadataPropertyText),
    Boolean(bool),
    Integer(i64),
    Object(MetadataObjectReference),
    Objects(Vec<MetadataObjectReference>),
    Texts(Vec<MetadataPropertyText>),
    Type(MetadataTypeExpression),
    Types(Vec<MetadataTypeExpression>),
    ModuleContext(MetadataModuleContext),
    ReturnValuesReuse(MetadataReturnValuesReuse),
    HierarchyType(MetadataHierarchyType),
    FillValue(MetadataObjectReference),
    FillChecking(MetadataFillChecking),
    Indexing(MetadataIndexing),
    ChoiceHistory(MetadataChoiceHistory),
    Periodicity(MetadataPeriodicity),
    RegisterKind(MetadataRegisterKind),
    CalculationDependence(MetadataCalculationDependence),
    SessionReuse(MetadataSessionReuse),
    Method(MetadataMethodReference),
    Event(MetadataEventName),
    JobKey(MetadataJobKey),
    UrlRoot(MetadataUrlRoot),
    ServiceNamespace(MetadataServiceNamespace),
    UrlTemplates(Vec<MetadataUrlTemplateDefinition>),
    ServiceOperations(Vec<MetadataServiceOperationDefinition>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataKindProperty {
    name: MetadataKindPropertyName,
    value: MetadataPropertyValue,
}

impl MetadataKindProperty {
    pub fn new(
        name: MetadataKindPropertyName,
        value: MetadataPropertyValue,
    ) -> Result<Self, SemanticValueError> {
        if !metadata_kind_property_pair_is_valid(name, &value) {
            return Err(SemanticValueError::InvalidCombination);
        }
        Ok(Self { name, value })
    }
    pub const fn name(&self) -> MetadataKindPropertyName {
        self.name
    }
    pub const fn value(&self) -> &MetadataPropertyValue {
        &self.value
    }
}

impl<'de> Deserialize<'de> for MetadataKindProperty {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            name: MetadataKindPropertyName,
            value: MetadataPropertyValue,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.name, wire.value).map_err(serde::de::Error::custom)
    }
}

fn metadata_kind_property_pair_is_valid(
    name: MetadataKindPropertyName,
    value: &MetadataPropertyValue,
) -> bool {
    use MetadataKindPropertyName as Name;
    use MetadataPropertyValue as Value;
    match name {
        Name::Hierarchical
        | Name::LimitLevelCount
        | Name::FoldersOnTop
        | Name::CheckUnique
        | Name::Autonumbering
        | Name::QuickChoice
        | Name::SequenceFilling
        | Name::PostInPrivilegedMode
        | Name::UnpostInPrivilegedMode
        | Name::MainFilterOnPeriod
        | Name::EnableTotalsSplitting
        | Name::Correspondence
        | Name::ActionPeriod
        | Name::BasePeriod
        | Name::AutoOrderByCode
        | Name::ActionPeriodUse
        | Name::DistributedInfoBase
        | Name::IncludeConfigurationExtensions
        | Name::Nonnegative
        | Name::CreateTaskInPrivilegedMode
        | Name::Use
        | Name::Predefined => matches!(value, Value::Boolean(_)),
        Name::LevelCount
        | Name::CodeLength
        | Name::DescriptionLength
        | Name::NumberLength
        | Name::PeriodAdjustmentLength
        | Name::MaxExtDimensionCount
        | Name::OrderLength
        | Name::RestartCountOnFailure
        | Name::RestartIntervalOnFailure
        | Name::SessionMaxAge
        | Name::Length
        | Name::Precision => matches!(value, Value::Integer(_)),
        Name::CodeMask | Name::Description | Name::MainAddressingAttribute => {
            matches!(value, Value::Text(_))
        }
        Name::ValueType | Name::Addressing => matches!(value, Value::Type(_)),
        Name::ValueTypes => matches!(value, Value::Types(_)),
        Name::Context => matches!(value, Value::ModuleContext(_)),
        Name::ReturnValuesReuse => matches!(value, Value::ReturnValuesReuse(_)),
        Name::HierarchyType => matches!(value, Value::HierarchyType(_)),
        Name::Periodicity => matches!(value, Value::Periodicity(_)),
        Name::RegisterType => matches!(value, Value::RegisterKind(_)),
        Name::ChartOfAccounts
        | Name::ChartOfCalculationTypes
        | Name::ExtDimensionTypes
        | Name::Task => matches!(value, Value::Object(_)),
        Name::AccountingFlags | Name::ExtDimensionAccountingFlags => {
            matches!(value, Value::Texts(_))
        }
        Name::DependenceOnCalculationTypes => matches!(value, Value::CalculationDependence(_)),
        Name::BaseCalculationTypes | Name::RegisteredDocuments | Name::Source => {
            matches!(value, Value::Objects(_))
        }
        Name::MethodName | Name::Handler => matches!(value, Value::Method(_)),
        Name::Key => matches!(value, Value::JobKey(_)),
        Name::Event => matches!(value, Value::Event(_)),
        Name::RootUrl => matches!(value, Value::UrlRoot(_)),
        Name::ReuseSessions => matches!(value, Value::SessionReuse(_)),
        Name::UrlTemplates => matches!(value, Value::UrlTemplates(_)),
        Name::Namespace => matches!(value, Value::ServiceNamespace(_)),
        Name::Operations => matches!(value, Value::ServiceOperations(_)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataAttributeDefinition {
    name: MetadataFieldName,
    synonym: Option<SynonymText>,
    comment: Option<CommentText>,
    value_type: Option<MetadataValueType>,
    type_expression: Option<MetadataTypeExpression>,
    length: Option<u32>,
    precision: Option<u16>,
    nonnegative: bool,
    flags: Vec<MetadataFieldFlag>,
    fill_checking: Option<MetadataFillChecking>,
    indexing: Option<MetadataIndexing>,
    choice_history: Option<MetadataChoiceHistory>,
    addressing_dimension: Option<MetadataObjectReference>,
    references: Vec<MetadataObjectReference>,
}

impl MetadataAttributeDefinition {
    pub const fn new(name: MetadataFieldName) -> Self {
        Self {
            name,
            synonym: None,
            comment: None,
            value_type: None,
            type_expression: None,
            length: None,
            precision: None,
            nonnegative: false,
            flags: Vec::new(),
            fill_checking: None,
            indexing: None,
            choice_history: None,
            addressing_dimension: None,
            references: Vec::new(),
        }
    }
    pub fn with_synonym(mut self, value: Option<SynonymText>) -> Self {
        self.synonym = value;
        self
    }
    pub fn with_comment(mut self, value: Option<CommentText>) -> Self {
        self.comment = value;
        self
    }
    pub fn with_type_expression(mut self, value: Option<MetadataTypeExpression>) -> Self {
        self.type_expression = value;
        self
    }
    pub fn with_flags(mut self, value: Vec<MetadataFieldFlag>) -> Self {
        self.flags = value;
        self
    }
    pub fn with_fill_checking(mut self, value: Option<MetadataFillChecking>) -> Self {
        self.fill_checking = value;
        self
    }
    pub fn with_indexing(mut self, value: Option<MetadataIndexing>) -> Self {
        self.indexing = value;
        self
    }
    pub fn with_choice_history(mut self, value: Option<MetadataChoiceHistory>) -> Self {
        self.choice_history = value;
        self
    }
    pub fn with_addressing_dimension(mut self, value: Option<MetadataObjectReference>) -> Self {
        self.addressing_dimension = value;
        self
    }
    pub fn with_references(mut self, value: Vec<MetadataObjectReference>) -> Self {
        self.references = value;
        self
    }
    pub const fn name(&self) -> &MetadataFieldName {
        &self.name
    }
    pub const fn synonym(&self) -> Option<&SynonymText> {
        self.synonym.as_ref()
    }
    pub const fn comment(&self) -> Option<&CommentText> {
        self.comment.as_ref()
    }
    pub const fn value_type(&self) -> Option<MetadataValueType> {
        self.value_type
    }
    pub const fn type_expression(&self) -> Option<&MetadataTypeExpression> {
        self.type_expression.as_ref()
    }
    pub const fn length(&self) -> Option<u32> {
        self.length
    }
    pub const fn precision(&self) -> Option<u16> {
        self.precision
    }
    pub const fn nonnegative(&self) -> bool {
        self.nonnegative
    }
    pub fn flags(&self) -> &[MetadataFieldFlag] {
        &self.flags
    }
    pub const fn fill_checking(&self) -> Option<MetadataFillChecking> {
        self.fill_checking
    }
    pub const fn indexing(&self) -> Option<MetadataIndexing> {
        self.indexing
    }
    pub const fn choice_history(&self) -> Option<MetadataChoiceHistory> {
        self.choice_history
    }
    pub const fn addressing_dimension(&self) -> Option<&MetadataObjectReference> {
        self.addressing_dimension.as_ref()
    }
    pub fn references(&self) -> &[MetadataObjectReference] {
        &self.references
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataTabularSectionDefinition {
    name: MetadataChildName,
    synonym: Option<SynonymText>,
    attributes: Vec<MetadataAttributeDefinition>,
}

impl MetadataTabularSectionDefinition {
    pub const fn new(
        name: MetadataChildName,
        attributes: Vec<MetadataAttributeDefinition>,
    ) -> Self {
        Self {
            name,
            synonym: None,
            attributes,
        }
    }
    pub fn with_synonym(mut self, value: Option<SynonymText>) -> Self {
        self.synonym = value;
        self
    }
    pub const fn name(&self) -> &MetadataChildName {
        &self.name
    }
    pub const fn synonym(&self) -> Option<&SynonymText> {
        self.synonym.as_ref()
    }
    pub fn attributes(&self) -> &[MetadataAttributeDefinition] {
        &self.attributes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataNamedChildKind {
    Attribute,
    TabularSection,
    Form,
    Template,
    Command,
    Dimension,
    Resource,
    Requisite,
    EnumValue,
    Column,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataNamedChildDefinition {
    kind: MetadataNamedChildKind,
    name: MetadataChildName,
    synonym: Option<SynonymText>,
}

impl MetadataNamedChildDefinition {
    pub const fn new(kind: MetadataNamedChildKind, name: MetadataChildName) -> Self {
        Self {
            kind,
            name,
            synonym: None,
        }
    }
    pub const fn kind(&self) -> MetadataNamedChildKind {
        self.kind
    }
    pub const fn name(&self) -> &MetadataChildName {
        &self.name
    }
    pub const fn synonym(&self) -> Option<&SynonymText> {
        self.synonym.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataCommonDefinition {
    name: MetadataChildName,
    synonym: Option<SynonymText>,
    comment: Option<CommentText>,
    attributes: Vec<MetadataAttributeDefinition>,
    tabular_sections: Vec<MetadataTabularSectionDefinition>,
    dimensions: Vec<MetadataAttributeDefinition>,
    resources: Vec<MetadataAttributeDefinition>,
    addressing_attributes: Vec<MetadataAttributeDefinition>,
    columns: Vec<MetadataAttributeDefinition>,
    children: Vec<MetadataNamedChildDefinition>,
}

impl MetadataCommonDefinition {
    pub const fn new(name: MetadataChildName) -> Self {
        Self {
            name,
            synonym: None,
            comment: None,
            attributes: Vec::new(),
            tabular_sections: Vec::new(),
            dimensions: Vec::new(),
            resources: Vec::new(),
            addressing_attributes: Vec::new(),
            columns: Vec::new(),
            children: Vec::new(),
        }
    }
    pub fn with_synonym(mut self, value: Option<SynonymText>) -> Self {
        self.synonym = value;
        self
    }
    pub fn with_comment(mut self, value: Option<CommentText>) -> Self {
        self.comment = value;
        self
    }
    pub fn with_attributes(mut self, value: Vec<MetadataAttributeDefinition>) -> Self {
        self.attributes = value;
        self
    }
    pub fn with_tabular_sections(mut self, value: Vec<MetadataTabularSectionDefinition>) -> Self {
        self.tabular_sections = value;
        self
    }
    pub fn with_dimensions(mut self, value: Vec<MetadataAttributeDefinition>) -> Self {
        self.dimensions = value;
        self
    }
    pub fn with_resources(mut self, value: Vec<MetadataAttributeDefinition>) -> Self {
        self.resources = value;
        self
    }
    pub fn with_addressing_attributes(mut self, value: Vec<MetadataAttributeDefinition>) -> Self {
        self.addressing_attributes = value;
        self
    }
    pub fn with_columns(mut self, value: Vec<MetadataAttributeDefinition>) -> Self {
        self.columns = value;
        self
    }
    pub fn with_children(mut self, value: Vec<MetadataNamedChildDefinition>) -> Self {
        self.children = value;
        self
    }
    pub const fn name(&self) -> &MetadataChildName {
        &self.name
    }
    pub const fn synonym(&self) -> Option<&SynonymText> {
        self.synonym.as_ref()
    }
    pub const fn comment(&self) -> Option<&CommentText> {
        self.comment.as_ref()
    }
    pub fn attributes(&self) -> &[MetadataAttributeDefinition] {
        &self.attributes
    }
    pub fn tabular_sections(&self) -> &[MetadataTabularSectionDefinition] {
        &self.tabular_sections
    }
    pub fn dimensions(&self) -> &[MetadataAttributeDefinition] {
        &self.dimensions
    }
    pub fn resources(&self) -> &[MetadataAttributeDefinition] {
        &self.resources
    }
    pub fn addressing_attributes(&self) -> &[MetadataAttributeDefinition] {
        &self.addressing_attributes
    }
    pub fn columns(&self) -> &[MetadataAttributeDefinition] {
        &self.columns
    }
    pub fn children(&self) -> &[MetadataNamedChildDefinition] {
        &self.children
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataKindDefinition {
    kind: MetadataKind,
    properties: Vec<MetadataKindProperty>,
}

impl MetadataKindDefinition {
    pub fn new(
        kind: MetadataKind,
        properties: Vec<MetadataKindProperty>,
    ) -> Result<Self, SemanticValueError> {
        if properties
            .iter()
            .any(|property| !metadata_kind_allows_property(kind, property.name()))
        {
            return Err(SemanticValueError::InvalidCombination);
        }
        Ok(Self { kind, properties })
    }
    pub const fn kind(&self) -> MetadataKind {
        self.kind
    }
    pub fn properties(&self) -> &[MetadataKindProperty] {
        &self.properties
    }
}

impl<'de> Deserialize<'de> for MetadataKindDefinition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            kind: MetadataKind,
            properties: Vec<MetadataKindProperty>,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.kind, wire.properties).map_err(serde::de::Error::custom)
    }
}

pub const fn metadata_kind_allows_property(
    kind: MetadataKind,
    property: MetadataKindPropertyName,
) -> bool {
    use MetadataKind as Kind;
    use MetadataKindPropertyName as Property;

    match property {
        Property::Hierarchical
        | Property::LimitLevelCount
        | Property::LevelCount
        | Property::FoldersOnTop => matches!(kind, Kind::Catalog | Kind::ChartOfAccounts),
        Property::CodeLength | Property::DescriptionLength => matches!(
            kind,
            Kind::Catalog
                | Kind::ExchangePlan
                | Kind::ChartOfCharacteristicTypes
                | Kind::ChartOfAccounts
                | Kind::ChartOfCalculationTypes
        ),
        Property::NumberLength => matches!(
            kind,
            Kind::Document | Kind::DocumentNumerator | Kind::BusinessProcess | Kind::Task
        ),
        Property::CheckUnique | Property::Autonumbering => matches!(
            kind,
            Kind::Catalog
                | Kind::ExchangePlan
                | Kind::Document
                | Kind::DocumentNumerator
                | Kind::ChartOfCharacteristicTypes
                | Kind::ChartOfAccounts
                | Kind::ChartOfCalculationTypes
                | Kind::BusinessProcess
                | Kind::Task
        ),
        Property::QuickChoice => matches!(
            kind,
            Kind::Catalog
                | Kind::ExchangePlan
                | Kind::Enum
                | Kind::ChartOfCharacteristicTypes
                | Kind::ChartOfAccounts
                | Kind::ChartOfCalculationTypes
        ),
        Property::SequenceFilling
        | Property::PostInPrivilegedMode
        | Property::UnpostInPrivilegedMode => matches!(kind, Kind::Document),
        Property::MainFilterOnPeriod => matches!(kind, Kind::InformationRegister),
        Property::Periodicity => {
            matches!(kind, Kind::InformationRegister | Kind::CalculationRegister)
        }
        Property::EnableTotalsSplitting | Property::RegisterType => {
            matches!(kind, Kind::AccumulationRegister)
        }
        Property::Correspondence | Property::ChartOfAccounts => {
            matches!(kind, Kind::AccountingRegister)
        }
        Property::PeriodAdjustmentLength
        | Property::ActionPeriod
        | Property::BasePeriod
        | Property::ChartOfCalculationTypes => matches!(kind, Kind::CalculationRegister),
        Property::MaxExtDimensionCount
        | Property::CodeMask
        | Property::AutoOrderByCode
        | Property::OrderLength
        | Property::ExtDimensionTypes
        | Property::AccountingFlags
        | Property::ExtDimensionAccountingFlags => matches!(kind, Kind::ChartOfAccounts),
        Property::ActionPeriodUse
        | Property::DependenceOnCalculationTypes
        | Property::BaseCalculationTypes => matches!(kind, Kind::ChartOfCalculationTypes),
        Property::DistributedInfoBase | Property::IncludeConfigurationExtensions => {
            matches!(kind, Kind::ExchangePlan)
        }
        Property::RestartCountOnFailure
        | Property::RestartIntervalOnFailure
        | Property::MethodName
        | Property::Key
        | Property::Use
        | Property::Predefined => matches!(kind, Kind::ScheduledJob),
        Property::SessionMaxAge | Property::ReuseSessions => {
            matches!(kind, Kind::HttpService | Kind::WebService)
        }
        Property::Length | Property::Precision | Property::Nonnegative => matches!(
            kind,
            Kind::SessionParameter
                | Kind::CommonAttribute
                | Kind::FilterCriterion
                | Kind::FunctionalOption
                | Kind::FunctionalOptionsParameter
                | Kind::Constant
        ),
        Property::CreateTaskInPrivilegedMode | Property::Task => {
            matches!(kind, Kind::BusinessProcess)
        }
        Property::ValueType => matches!(
            kind,
            Kind::SessionParameter
                | Kind::CommonAttribute
                | Kind::StyleItem
                | Kind::FilterCriterion
                | Kind::FunctionalOption
                | Kind::FunctionalOptionsParameter
                | Kind::Constant
        ),
        Property::ValueTypes => {
            matches!(kind, Kind::DefinedType | Kind::ChartOfCharacteristicTypes)
        }
        Property::Context | Property::ReturnValuesReuse => matches!(kind, Kind::CommonModule),
        Property::HierarchyType => matches!(kind, Kind::Catalog),
        Property::Addressing | Property::MainAddressingAttribute => matches!(kind, Kind::Task),
        Property::RegisteredDocuments => matches!(kind, Kind::DocumentJournal),
        Property::Description => matches!(
            kind,
            Kind::StyleItem
                | Kind::FunctionalOption
                | Kind::FunctionalOptionsParameter
                | Kind::CommandGroup
                | Kind::CommonCommand
                | Kind::ScheduledJob
        ),
        Property::Source | Property::Event | Property::Handler => {
            matches!(kind, Kind::EventSubscription)
        }
        Property::RootUrl | Property::UrlTemplates => matches!(kind, Kind::HttpService),
        Property::Namespace | Property::Operations => matches!(kind, Kind::WebService),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataDefinition {
    common: MetadataCommonDefinition,
    specific: MetadataKindDefinition,
}

impl MetadataDefinition {
    pub const fn new(common: MetadataCommonDefinition, specific: MetadataKindDefinition) -> Self {
        Self { common, specific }
    }
    pub const fn common(&self) -> &MetadataCommonDefinition {
        &self.common
    }
    pub const fn specific(&self) -> &MetadataKindDefinition {
        &self.specific
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataObjectProperty {
    Name,
    Synonym,
    Comment,
    ValueType,
    Length,
    Precision,
    Nonnegative,
    FillChecking,
    Indexing,
    MultiLine,
    HierarchyType,
    FillValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataPropertyPatch {
    property: MetadataObjectProperty,
    value: MetadataPropertyValue,
}

impl MetadataPropertyPatch {
    pub fn new(
        property: MetadataObjectProperty,
        value: MetadataPropertyValue,
    ) -> Result<Self, SemanticValueError> {
        let valid = match property {
            MetadataObjectProperty::Name => matches!(value, MetadataPropertyValue::Name(_)),
            MetadataObjectProperty::Synonym => matches!(value, MetadataPropertyValue::Synonym(_)),
            MetadataObjectProperty::Comment => matches!(value, MetadataPropertyValue::Comment(_)),
            MetadataObjectProperty::ValueType => matches!(value, MetadataPropertyValue::Type(_)),
            MetadataObjectProperty::Length | MetadataObjectProperty::Precision => {
                matches!(value, MetadataPropertyValue::Integer(_))
            }
            MetadataObjectProperty::Nonnegative | MetadataObjectProperty::MultiLine => {
                matches!(value, MetadataPropertyValue::Boolean(_))
            }
            MetadataObjectProperty::FillChecking => {
                matches!(value, MetadataPropertyValue::FillChecking(_))
            }
            MetadataObjectProperty::Indexing => {
                matches!(value, MetadataPropertyValue::Indexing(_))
            }
            MetadataObjectProperty::HierarchyType => {
                matches!(value, MetadataPropertyValue::HierarchyType(_))
            }
            MetadataObjectProperty::FillValue => {
                matches!(value, MetadataPropertyValue::FillValue(_))
            }
        };
        if !valid {
            return Err(SemanticValueError::InvalidCombination);
        }
        Ok(Self { property, value })
    }
    pub const fn property(&self) -> MetadataObjectProperty {
        self.property
    }
    pub const fn value(&self) -> &MetadataPropertyValue {
        &self.value
    }
}

impl<'de> Deserialize<'de> for MetadataPropertyPatch {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            property: MetadataObjectProperty,
            value: MetadataPropertyValue,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.property, wire.value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "kind",
    content = "definition",
    deny_unknown_fields
)]
pub enum MetadataChildDefinition {
    Attribute(MetadataAttributeDefinition),
    TabularSection(MetadataTabularSectionDefinition),
    Named(MetadataNamedChildDefinition),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataChildReference {
    kind: MetadataNamedChildKind,
    name: MetadataChildName,
    parent: Option<MetadataChildName>,
}

impl MetadataChildReference {
    pub const fn new(kind: MetadataNamedChildKind, name: MetadataChildName) -> Self {
        Self {
            kind,
            name,
            parent: None,
        }
    }
    pub fn with_parent(mut self, parent: Option<MetadataChildName>) -> Self {
        self.parent = parent;
        self
    }
    pub const fn kind(&self) -> MetadataNamedChildKind {
        self.kind
    }
    pub const fn name(&self) -> &MetadataChildName {
        &self.name
    }
    pub const fn parent(&self) -> Option<&MetadataChildName> {
        self.parent.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataChildPatch {
    target: MetadataChildReference,
    #[serde(deserialize_with = "deserialize_non_empty_vec")]
    changes: Vec<MetadataPropertyPatch>,
}

impl MetadataChildPatch {
    pub fn new(
        target: MetadataChildReference,
        changes: Vec<MetadataPropertyPatch>,
    ) -> Result<Self, SemanticValueError> {
        if changes.is_empty() {
            return Err(SemanticValueError::Empty);
        }
        Ok(Self { target, changes })
    }
    pub const fn target(&self) -> &MetadataChildReference {
        &self.target
    }
    pub fn changes(&self) -> &[MetadataPropertyPatch] {
        &self.changes
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct MetadataPropertyChanges(Vec<MetadataPropertyPatch>);

impl MetadataPropertyChanges {
    pub fn new(values: Vec<MetadataPropertyPatch>) -> Result<Self, SemanticValueError> {
        if values.is_empty() {
            return Err(SemanticValueError::Empty);
        }
        Ok(Self(values))
    }
    pub fn one(value: MetadataPropertyPatch) -> Self {
        Self(vec![value])
    }
    pub fn values(&self) -> &[MetadataPropertyPatch] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for MetadataPropertyChanges {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let values = Vec::<MetadataPropertyPatch>::deserialize(deserializer)?;
        Self::new(values).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct MetadataPropertiesToClear(Vec<MetadataObjectProperty>);

impl MetadataPropertiesToClear {
    pub fn new(values: Vec<MetadataObjectProperty>) -> Result<Self, SemanticValueError> {
        if values.is_empty() {
            return Err(SemanticValueError::Empty);
        }
        Ok(Self(values))
    }
    pub fn one(value: MetadataObjectProperty) -> Self {
        Self(vec![value])
    }
    pub fn values(&self) -> &[MetadataObjectProperty] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for MetadataPropertiesToClear {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let values = Vec::<MetadataObjectProperty>::deserialize(deserializer)?;
        Self::new(values).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "operation",
    content = "payload",
    deny_unknown_fields
)]
pub enum MetadataPatch {
    Replace(MetadataDefinition),
    SetProperties(MetadataPropertyChanges),
    ClearProperties(MetadataPropertiesToClear),
    AddChild(MetadataChildDefinition),
    RemoveChild(MetadataChildReference),
    ModifyChild(MetadataChildPatch),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataCreate {
    #[serde(deserialize_with = "deserialize_non_empty_vec")]
    definitions: Vec<MetadataDefinition>,
    omit_default_role: bool,
    assign_default_form: bool,
}

impl MetadataCreate {
    pub fn new(definition: MetadataDefinition) -> Self {
        Self {
            definitions: vec![definition],
            omit_default_role: false,
            assign_default_form: false,
        }
    }
    pub fn batch(definitions: Vec<MetadataDefinition>) -> Result<Self, SemanticValueError> {
        if definitions.is_empty() {
            return Err(SemanticValueError::Empty);
        }
        Ok(Self {
            definitions,
            omit_default_role: false,
            assign_default_form: false,
        })
    }
    pub const fn omit_default_role(mut self, value: bool) -> Self {
        self.omit_default_role = value;
        self
    }
    pub const fn assign_default_form(mut self, value: bool) -> Self {
        self.assign_default_form = value;
        self
    }
    pub fn definitions(&self) -> &[MetadataDefinition] {
        &self.definitions
    }
    pub fn definition(&self) -> &MetadataDefinition {
        &self.definitions[0]
    }
    pub const fn omits_default_role(&self) -> bool {
        self.omit_default_role
    }
    pub const fn assigns_default_form(&self) -> bool {
        self.assign_default_form
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataEdit {
    object: Option<MetadataObjectReference>,
    patch: MetadataPatch,
    create_if_missing: bool,
}

impl MetadataEdit {
    pub const fn new(object: MetadataObjectReference, patch: MetadataPatch) -> Self {
        Self {
            object: Some(object),
            patch,
            create_if_missing: false,
        }
    }
    pub const fn selected_object(patch: MetadataPatch) -> Self {
        Self {
            object: None,
            patch,
            create_if_missing: false,
        }
    }
    pub const fn create_if_missing(mut self, value: bool) -> Self {
        self.create_if_missing = value;
        self
    }
    pub const fn object(&self) -> Option<&MetadataObjectReference> {
        self.object.as_ref()
    }
    pub const fn patch(&self) -> &MetadataPatch {
        &self.patch
    }
    pub const fn creates_if_missing(&self) -> bool {
        self.create_if_missing
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DefaultFormAssignment {
    IfVacant,
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FormElementType {
    Input,
    Group,
    Table,
    Button,
    CommandBar,
    Label,
    Picture,
    Calendar,
    Pages,
    Page,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FormEventBinding {
    event: FormEventName,
    handler: FormHandlerName,
}
impl FormEventBinding {
    pub const fn new(event: FormEventName, handler: FormHandlerName) -> Self {
        Self { event, handler }
    }
    pub const fn event(&self) -> &FormEventName {
        &self.event
    }
    pub const fn handler(&self) -> &FormHandlerName {
        &self.handler
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FormAttributeDefinition {
    name: FormAttributeName,
    value_type: Option<MetadataValueType>,
    title: Option<SynonymText>,
    main: bool,
    columns: Vec<FormAttributeDefinition>,
}
impl FormAttributeDefinition {
    pub const fn new(name: FormAttributeName) -> Self {
        Self {
            name,
            value_type: None,
            title: None,
            main: false,
            columns: Vec::new(),
        }
    }
    pub const fn with_value_type(mut self, value: Option<MetadataValueType>) -> Self {
        self.value_type = value;
        self
    }
    pub fn with_title(mut self, value: Option<SynonymText>) -> Self {
        self.title = value;
        self
    }
    pub const fn as_main(mut self, value: bool) -> Self {
        self.main = value;
        self
    }
    pub fn with_columns(mut self, value: Vec<FormAttributeDefinition>) -> Self {
        self.columns = value;
        self
    }
    pub const fn name(&self) -> &FormAttributeName {
        &self.name
    }
    pub const fn value_type(&self) -> Option<MetadataValueType> {
        self.value_type
    }
    pub const fn title(&self) -> Option<&SynonymText> {
        self.title.as_ref()
    }
    pub const fn is_main(&self) -> bool {
        self.main
    }
    pub fn columns(&self) -> &[FormAttributeDefinition] {
        &self.columns
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FormCommandDefinition {
    name: FormCommandName,
    title: Option<SynonymText>,
    action: Option<FormHandlerName>,
}
impl FormCommandDefinition {
    pub const fn new(name: FormCommandName) -> Self {
        Self {
            name,
            title: None,
            action: None,
        }
    }
    pub fn with_title(mut self, value: Option<SynonymText>) -> Self {
        self.title = value;
        self
    }
    pub fn with_action(mut self, value: Option<FormHandlerName>) -> Self {
        self.action = value;
        self
    }
    pub const fn name(&self) -> &FormCommandName {
        &self.name
    }
    pub const fn title(&self) -> Option<&SynonymText> {
        self.title.as_ref()
    }
    pub const fn action(&self) -> Option<&FormHandlerName> {
        self.action.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FormParameterDefinition {
    name: FormParameterName,
    value_type: Option<MetadataValueType>,
}
impl FormParameterDefinition {
    pub const fn new(name: FormParameterName) -> Self {
        Self {
            name,
            value_type: None,
        }
    }
    pub const fn with_value_type(mut self, value: Option<MetadataValueType>) -> Self {
        self.value_type = value;
        self
    }
    pub const fn name(&self) -> &FormParameterName {
        &self.name
    }
    pub const fn value_type(&self) -> Option<MetadataValueType> {
        self.value_type
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FormElementDefinition {
    name: FormElementName,
    #[serde(rename = "type")]
    element_type: FormElementType,
    title: Option<SynonymText>,
    data_path: Option<FormElementPath>,
    command: Option<FormCommandName>,
    visible: bool,
    enabled: bool,
    read_only: bool,
    events: Vec<FormEventBinding>,
    children: Vec<FormElementDefinition>,
}
impl FormElementDefinition {
    pub const fn new(name: FormElementName, element_type: FormElementType) -> Self {
        Self {
            name,
            element_type,
            title: None,
            data_path: None,
            command: None,
            visible: true,
            enabled: true,
            read_only: false,
            events: Vec::new(),
            children: Vec::new(),
        }
    }
    pub fn with_title(mut self, value: Option<SynonymText>) -> Self {
        self.title = value;
        self
    }
    pub fn with_data_path(mut self, value: Option<FormElementPath>) -> Self {
        self.data_path = value;
        self
    }
    pub fn with_command(mut self, value: Option<FormCommandName>) -> Self {
        self.command = value;
        self
    }
    pub const fn visible(mut self, value: bool) -> Self {
        self.visible = value;
        self
    }
    pub const fn enabled(mut self, value: bool) -> Self {
        self.enabled = value;
        self
    }
    pub const fn read_only(mut self, value: bool) -> Self {
        self.read_only = value;
        self
    }
    pub fn with_events(mut self, value: Vec<FormEventBinding>) -> Self {
        self.events = value;
        self
    }
    pub fn with_children(mut self, value: Vec<FormElementDefinition>) -> Self {
        self.children = value;
        self
    }
    pub const fn name(&self) -> &FormElementName {
        &self.name
    }
    pub const fn element_type(&self) -> FormElementType {
        self.element_type
    }
    pub const fn title(&self) -> Option<&SynonymText> {
        self.title.as_ref()
    }
    pub const fn data_path(&self) -> Option<&FormElementPath> {
        self.data_path.as_ref()
    }
    pub const fn command(&self) -> Option<&FormCommandName> {
        self.command.as_ref()
    }
    pub const fn is_visible(&self) -> bool {
        self.visible
    }
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }
    pub const fn is_read_only(&self) -> bool {
        self.read_only
    }
    pub fn events(&self) -> &[FormEventBinding] {
        &self.events
    }
    pub fn children(&self) -> &[FormElementDefinition] {
        &self.children
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedFormDefinition {
    title: Option<SynonymText>,
    attributes: Vec<FormAttributeDefinition>,
    commands: Vec<FormCommandDefinition>,
    parameters: Vec<FormParameterDefinition>,
    elements: Vec<FormElementDefinition>,
    events: Vec<FormEventBinding>,
}
impl ManagedFormDefinition {
    pub const fn empty() -> Self {
        Self {
            title: None,
            attributes: Vec::new(),
            commands: Vec::new(),
            parameters: Vec::new(),
            elements: Vec::new(),
            events: Vec::new(),
        }
    }
    pub fn new(
        title: Option<SynonymText>,
        attributes: Vec<FormAttributeDefinition>,
        commands: Vec<FormCommandDefinition>,
        parameters: Vec<FormParameterDefinition>,
        elements: Vec<FormElementDefinition>,
        events: Vec<FormEventBinding>,
    ) -> Self {
        Self {
            title,
            attributes,
            commands,
            parameters,
            elements,
            events,
        }
    }
    pub const fn title(&self) -> Option<&SynonymText> {
        self.title.as_ref()
    }
    pub fn attributes(&self) -> &[FormAttributeDefinition] {
        &self.attributes
    }
    pub fn commands(&self) -> &[FormCommandDefinition] {
        &self.commands
    }
    pub fn parameters(&self) -> &[FormParameterDefinition] {
        &self.parameters
    }
    pub fn elements(&self) -> &[FormElementDefinition] {
        &self.elements
    }
    pub fn events(&self) -> &[FormEventBinding] {
        &self.events
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FormCreate {
    owner: FormOwnerReference,
    name: FormName,
    synonym: Option<SynonymText>,
    purpose: FormPurpose,
    default_assignment: DefaultFormAssignment,
    definition: ManagedFormDefinition,
}
impl FormCreate {
    pub const fn new(owner: FormOwnerReference, name: FormName) -> Self {
        Self {
            owner,
            name,
            synonym: None,
            purpose: FormPurpose::Object,
            default_assignment: DefaultFormAssignment::IfVacant,
            definition: ManagedFormDefinition::empty(),
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
    pub const fn definition(&self) -> &ManagedFormDefinition {
        &self.definition
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "source",
    content = "value",
    deny_unknown_fields
)]
pub enum FormCompileSource {
    Definition(ManagedFormDefinition),
    Object { purpose: Option<FormCompilePurpose> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FormCompile {
    source: FormCompileSource,
    skip_validation: bool,
}
impl FormCompile {
    pub const fn new(definition: ManagedFormDefinition, skip_validation: bool) -> Self {
        Self {
            source: FormCompileSource::Definition(definition),
            skip_validation,
        }
    }
    pub const fn from_object(purpose: Option<FormCompilePurpose>, skip_validation: bool) -> Self {
        Self {
            source: FormCompileSource::Object { purpose },
            skip_validation,
        }
    }
    pub const fn source(&self) -> &FormCompileSource {
        &self.source
    }
    pub const fn skips_validation(&self) -> bool {
        self.skip_validation
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "operation",
    content = "payload",
    deny_unknown_fields
)]
pub enum FormPatch {
    NoOp,
    Replace(ManagedFormDefinition),
    AddElement(FormElementDefinition),
    RemoveElement(FormElementName),
    UpsertAttribute(FormAttributeDefinition),
    RemoveAttribute(FormAttributeName),
    UpsertCommand(FormCommandDefinition),
    RemoveCommand(FormCommandName),
    BindEvent(FormEventBinding),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FormEdit {
    #[serde(deserialize_with = "deserialize_non_empty_vec")]
    patches: Vec<FormPatch>,
    skip_validation: bool,
}

fn deserialize_non_empty_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    let values = <Vec<T> as serde::Deserialize>::deserialize(deserializer)?;
    if values.is_empty() {
        return Err(<D::Error as serde::de::Error>::custom(
            "at least one semantic operation is required",
        ));
    }
    Ok(values)
}

fn deserialize_non_zero_u16<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = <u16 as serde::Deserialize>::deserialize(deserializer)?;
    if value == 0 {
        return Err(<D::Error as serde::de::Error>::custom(
            "semantic size must be greater than zero",
        ));
    }
    Ok(value)
}

fn deserialize_non_zero_u32<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = <u32 as serde::Deserialize>::deserialize(deserializer)?;
    if value == 0 {
        return Err(<D::Error as serde::de::Error>::custom(
            "semantic coordinate or size must be greater than zero",
        ));
    }
    Ok(value)
}

const fn one_u16() -> u16 {
    1
}

impl FormEdit {
    pub fn new(patches: Vec<FormPatch>, skip_validation: bool) -> Result<Self, SemanticValueError> {
        if patches.is_empty() {
            return Err(SemanticValueError::Empty);
        }
        Ok(Self {
            patches,
            skip_validation,
        })
    }
    pub fn patches(&self) -> &[FormPatch] {
        &self.patches
    }
    pub const fn skips_validation(&self) -> bool {
        self.skip_validation
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TemplateKind {
    DataComposition,
    Spreadsheet,
    Text,
    Html,
    Binary,
    Graphical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "kind",
    content = "content",
    deny_unknown_fields
)]
pub enum TemplateDefinition {
    Empty(TemplateKind),
    Text(TemplateText),
    Html(TemplateText),
}
impl TemplateDefinition {
    pub const fn kind(&self) -> TemplateKind {
        match self {
            Self::Empty(kind) => *kind,
            Self::Text(_) => TemplateKind::Text,
            Self::Html(_) => TemplateKind::Html,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemplateCreate {
    owner: TemplateOwnerReference,
    name: TemplateName,
    definition: TemplateDefinition,
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
            definition: TemplateDefinition::Empty(kind),
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
        self.definition.kind()
    }
    pub const fn definition(&self) -> &TemplateDefinition {
        &self.definition
    }
    pub const fn synonym(&self) -> Option<&SynonymText> {
        self.synonym.as_ref()
    }
    pub const fn assigns_main_data_composition(&self) -> bool {
        self.assign_main_data_composition
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "kind",
    content = "body",
    deny_unknown_fields
)]
pub enum HelpContent {
    Empty,
    Text(HelpText),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelpDefinition {
    language: Option<LanguageCode>,
    content: HelpContent,
}
impl HelpDefinition {
    pub const fn empty(language: Option<LanguageCode>) -> Self {
        Self {
            language,
            content: HelpContent::Empty,
        }
    }
    pub const fn language(&self) -> Option<&LanguageCode> {
        self.language.as_ref()
    }
    pub const fn content(&self) -> &HelpContent {
        &self.content
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelpCreate {
    owner: HelpOwnerReference,
    definition: HelpDefinition,
}
impl HelpCreate {
    pub const fn new(owner: HelpOwnerReference, language: Option<LanguageCode>) -> Self {
        Self {
            owner,
            definition: HelpDefinition::empty(language),
        }
    }
    pub const fn owner(&self) -> &HelpOwnerReference {
        &self.owner
    }
    pub const fn language(&self) -> Option<&LanguageCode> {
        self.definition.language()
    }
    pub const fn definition(&self) -> &HelpDefinition {
        &self.definition
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InterfaceItemKind {
    Command,
    Group,
    Subsystem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InterfaceItemReference {
    kind: InterfaceItemKind,
    name: InterfaceItemName,
}
impl InterfaceItemReference {
    pub const fn new(kind: InterfaceItemKind, name: InterfaceItemName) -> Self {
        Self { kind, name }
    }
    pub const fn kind(&self) -> InterfaceItemKind {
        self.kind
    }
    pub const fn name(&self) -> &InterfaceItemName {
        &self.name
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InterfacePlacement {
    item: InterfaceItemReference,
    group: InterfaceGroupName,
    order: u16,
}
impl InterfacePlacement {
    pub const fn new(item: InterfaceItemReference, group: InterfaceGroupName, order: u16) -> Self {
        Self { item, group, order }
    }
    pub const fn item(&self) -> &InterfaceItemReference {
        &self.item
    }
    pub const fn group(&self) -> &InterfaceGroupName {
        &self.group
    }
    pub const fn order(&self) -> u16 {
        self.order
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InterfaceCommandOrder {
    group: InterfaceGroupName,
    commands: Vec<InterfaceItemReference>,
}
impl InterfaceCommandOrder {
    pub const fn new(group: InterfaceGroupName, commands: Vec<InterfaceItemReference>) -> Self {
        Self { group, commands }
    }
    pub const fn group(&self) -> &InterfaceGroupName {
        &self.group
    }
    pub fn commands(&self) -> &[InterfaceItemReference] {
        &self.commands
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandInterfaceDefinition {
    items: Vec<InterfacePlacement>,
}
impl CommandInterfaceDefinition {
    pub const fn new(items: Vec<InterfacePlacement>) -> Self {
        Self { items }
    }
    pub fn items(&self) -> &[InterfacePlacement] {
        &self.items
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "operation",
    content = "payload",
    deny_unknown_fields
)]
pub enum InterfaceEdit {
    Replace(CommandInterfaceDefinition),
    Hide(InterfaceItemReference),
    Show(InterfaceItemReference),
    Place(InterfacePlacement),
    Order(InterfaceCommandOrder),
    OrderSubsystems(Vec<SubsystemName>),
    OrderGroups(Vec<InterfaceGroupName>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoleRight {
    Read,
    Insert,
    Update,
    Delete,
    View,
    Edit,
    InputByString,
    InteractiveInsert,
    InteractiveUpdate,
    InteractiveDelete,
    InteractiveDeleteMarked,
    InteractiveSetDeletionMark,
    InteractiveClearDeletionMark,
    Posting,
    UndoPosting,
    InteractivePosting,
    InteractivePostingRegular,
    InteractiveUndoPosting,
    InteractiveChangeOfPosted,
    Use,
    Execute,
    Get,
    Set,
    Administration,
    DataAdministration,
    ConfigurationAdministration,
    ThinClient,
    WebClient,
    MobileClient,
    Output,
    SaveUserData,
    MainWindowModeNormal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoleRightState {
    Allow,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoleRightAssignment {
    right: RoleRight,
    state: RoleRightState,
}
impl RoleRightAssignment {
    pub const fn new(right: RoleRight, state: RoleRightState) -> Self {
        Self { right, state }
    }
    pub const fn right(&self) -> RoleRight {
        self.right
    }
    pub const fn state(&self) -> RoleRightState {
        self.state
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoleObjectRights {
    object: RoleObjectReference,
    rights: Vec<RoleRightAssignment>,
    restriction: Option<RoleRestrictionText>,
}
impl RoleObjectRights {
    pub const fn new(object: RoleObjectReference, rights: Vec<RoleRightAssignment>) -> Self {
        Self {
            object,
            rights,
            restriction: None,
        }
    }
    pub fn with_restriction(mut self, value: Option<RoleRestrictionText>) -> Self {
        self.restriction = value;
        self
    }
    pub const fn object(&self) -> &RoleObjectReference {
        &self.object
    }
    pub fn rights(&self) -> &[RoleRightAssignment] {
        &self.rights
    }
    pub const fn restriction(&self) -> Option<&RoleRestrictionText> {
        self.restriction.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoleRestrictionTemplate {
    name: RoleTemplateName,
    condition: RoleRestrictionText,
}
impl RoleRestrictionTemplate {
    pub const fn new(name: RoleTemplateName, condition: RoleRestrictionText) -> Self {
        Self { name, condition }
    }
    pub const fn name(&self) -> &RoleTemplateName {
        &self.name
    }
    pub const fn condition(&self) -> &RoleRestrictionText {
        &self.condition
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoleDefinition {
    name: RoleName,
    synonym: Option<SynonymText>,
    comment: Option<CommentText>,
    set_for_new_objects: bool,
    set_for_attributes_by_default: bool,
    independent_rights_of_child_objects: bool,
    objects: Vec<RoleObjectRights>,
    templates: Vec<RoleRestrictionTemplate>,
}
impl RoleDefinition {
    pub const fn new(name: RoleName) -> Self {
        Self {
            name,
            synonym: None,
            comment: None,
            set_for_new_objects: false,
            set_for_attributes_by_default: true,
            independent_rights_of_child_objects: false,
            objects: Vec::new(),
            templates: Vec::new(),
        }
    }
    pub fn with_synonym(mut self, value: Option<SynonymText>) -> Self {
        self.synonym = value;
        self
    }
    pub fn with_comment(mut self, value: Option<CommentText>) -> Self {
        self.comment = value;
        self
    }
    pub const fn set_for_new_objects(mut self, value: bool) -> Self {
        self.set_for_new_objects = value;
        self
    }
    pub const fn set_for_attributes_by_default(mut self, value: bool) -> Self {
        self.set_for_attributes_by_default = value;
        self
    }
    pub const fn independent_rights_of_child_objects(mut self, value: bool) -> Self {
        self.independent_rights_of_child_objects = value;
        self
    }
    pub fn with_objects(mut self, value: Vec<RoleObjectRights>) -> Self {
        self.objects = value;
        self
    }
    pub fn with_templates(mut self, value: Vec<RoleRestrictionTemplate>) -> Self {
        self.templates = value;
        self
    }
    pub const fn name(&self) -> &RoleName {
        &self.name
    }
    pub const fn synonym(&self) -> Option<&SynonymText> {
        self.synonym.as_ref()
    }
    pub const fn comment(&self) -> Option<&CommentText> {
        self.comment.as_ref()
    }
    pub const fn sets_for_new_objects(&self) -> bool {
        self.set_for_new_objects
    }
    pub const fn sets_for_attributes_by_default(&self) -> bool {
        self.set_for_attributes_by_default
    }
    pub const fn has_independent_child_rights(&self) -> bool {
        self.independent_rights_of_child_objects
    }
    pub fn objects(&self) -> &[RoleObjectRights] {
        &self.objects
    }
    pub fn templates(&self) -> &[RoleRestrictionTemplate] {
        &self.templates
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoleCreate {
    definition: RoleDefinition,
}
impl RoleCreate {
    pub fn new(name: Option<RoleName>) -> Self {
        match name {
            Some(name) => Self {
                definition: RoleDefinition::new(name),
            },
            None => panic!("role definition requires a semantic name"),
        }
    }
    pub const fn from_definition(definition: RoleDefinition) -> Self {
        Self { definition }
    }
    pub const fn name(&self) -> Option<&RoleName> {
        Some(self.definition.name())
    }
    pub const fn definition(&self) -> &RoleDefinition {
        &self.definition
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubsystemDefinition {
    name: SubsystemName,
    synonym: Option<SynonymText>,
    comment: Option<CommentText>,
    explanation: Option<DescriptionText>,
    content: Vec<MetadataObjectReference>,
    children: Vec<SubsystemName>,
}
impl SubsystemDefinition {
    pub const fn new(name: SubsystemName) -> Self {
        Self {
            name,
            synonym: None,
            comment: None,
            explanation: None,
            content: Vec::new(),
            children: Vec::new(),
        }
    }
    pub const fn name(&self) -> &SubsystemName {
        &self.name
    }
    pub const fn synonym(&self) -> Option<&SynonymText> {
        self.synonym.as_ref()
    }
    pub const fn comment(&self) -> Option<&CommentText> {
        self.comment.as_ref()
    }
    pub const fn explanation(&self) -> Option<&DescriptionText> {
        self.explanation.as_ref()
    }
    pub fn content(&self) -> &[MetadataObjectReference] {
        &self.content
    }
    pub fn children(&self) -> &[SubsystemName] {
        &self.children
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubsystemCreate {
    definition: SubsystemDefinition,
}
impl SubsystemCreate {
    pub fn new(name: Option<SubsystemName>) -> Self {
        match name {
            Some(name) => Self {
                definition: SubsystemDefinition::new(name),
            },
            None => panic!("subsystem definition requires a semantic name"),
        }
    }
    pub const fn from_definition(definition: SubsystemDefinition) -> Self {
        Self { definition }
    }
    pub const fn name(&self) -> Option<&SubsystemName> {
        Some(self.definition.name())
    }
    pub const fn definition(&self) -> &SubsystemDefinition {
        &self.definition
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "operation",
    content = "value",
    deny_unknown_fields
)]
pub enum SubsystemPropertyPatch {
    SetSynonym(SynonymText),
    SetComment(CommentText),
    SetExplanation(DescriptionText),
    SetCommandInterfaceVisibility(bool),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "operation",
    content = "payload",
    deny_unknown_fields
)]
pub enum SubsystemEdit {
    Replace(SubsystemDefinition),
    AddContent(MetadataObjectReference),
    RemoveContent(MetadataObjectReference),
    AddChild(SubsystemName),
    RemoveChild(SubsystemName),
    SetProperty(SubsystemPropertyPatch),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SupportCapability {
    Enable,
    Disable,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SupportObjectRule {
    Locked,
    Editable,
    OffSupport,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "operation",
    content = "rule",
    deny_unknown_fields
)]
pub enum SupportEdit {
    Capability(SupportCapability),
    ObjectRule(SupportObjectRule),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataCompositionDataSourceKind {
    Local,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataCompositionDataSource {
    name: DataSourceName,
    kind: DataCompositionDataSourceKind,
}

impl DataCompositionDataSource {
    pub const fn new(name: DataSourceName, kind: DataCompositionDataSourceKind) -> Self {
        Self { name, kind }
    }
    pub const fn name(&self) -> &DataSourceName {
        &self.name
    }
    pub const fn kind(&self) -> DataCompositionDataSourceKind {
        self.kind
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataCompositionFieldDefinition {
    path: DataFieldPath,
    title: Option<SynonymText>,
    value_type: Option<MetadataValueType>,
    presentation_expression: Option<DataCompositionExpression>,
}

impl DataCompositionFieldDefinition {
    pub const fn new(path: DataFieldPath) -> Self {
        Self {
            path,
            title: None,
            value_type: None,
            presentation_expression: None,
        }
    }
    pub fn with_title(mut self, value: Option<SynonymText>) -> Self {
        self.title = value;
        self
    }
    pub const fn with_value_type(mut self, value: Option<MetadataValueType>) -> Self {
        self.value_type = value;
        self
    }
    pub fn with_presentation_expression(
        mut self,
        value: Option<DataCompositionExpression>,
    ) -> Self {
        self.presentation_expression = value;
        self
    }
    pub const fn path(&self) -> &DataFieldPath {
        &self.path
    }
    pub const fn title(&self) -> Option<&SynonymText> {
        self.title.as_ref()
    }
    pub const fn value_type(&self) -> Option<MetadataValueType> {
        self.value_type
    }
    pub const fn presentation_expression(&self) -> Option<&DataCompositionExpression> {
        self.presentation_expression.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataCompositionQueryDataSet {
    name: DataSetName,
    source: Option<DataSourceName>,
    query: DataCompositionQueryText,
    fields: Vec<DataCompositionFieldDefinition>,
    auto_fill_fields: bool,
}

impl DataCompositionQueryDataSet {
    pub const fn new(name: DataSetName, query: DataCompositionQueryText) -> Self {
        Self {
            name,
            source: None,
            query,
            fields: Vec::new(),
            auto_fill_fields: true,
        }
    }
    pub fn with_source(mut self, value: Option<DataSourceName>) -> Self {
        self.source = value;
        self
    }
    pub fn with_fields(mut self, value: Vec<DataCompositionFieldDefinition>) -> Self {
        self.fields = value;
        self
    }
    pub const fn auto_fill_fields(mut self, value: bool) -> Self {
        self.auto_fill_fields = value;
        self
    }
    pub const fn name(&self) -> &DataSetName {
        &self.name
    }
    pub const fn source(&self) -> Option<&DataSourceName> {
        self.source.as_ref()
    }
    pub const fn query(&self) -> &DataCompositionQueryText {
        &self.query
    }
    pub fn fields(&self) -> &[DataCompositionFieldDefinition] {
        &self.fields
    }
    pub const fn auto_fills_fields(&self) -> bool {
        self.auto_fill_fields
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataCompositionObjectDataSet {
    name: DataSetName,
    object: MetadataObjectReference,
    fields: Vec<DataCompositionFieldDefinition>,
}

impl DataCompositionObjectDataSet {
    pub const fn new(name: DataSetName, object: MetadataObjectReference) -> Self {
        Self {
            name,
            object,
            fields: Vec::new(),
        }
    }
    pub fn with_fields(mut self, value: Vec<DataCompositionFieldDefinition>) -> Self {
        self.fields = value;
        self
    }
    pub const fn name(&self) -> &DataSetName {
        &self.name
    }
    pub const fn object(&self) -> &MetadataObjectReference {
        &self.object
    }
    pub fn fields(&self) -> &[DataCompositionFieldDefinition] {
        &self.fields
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "kind",
    content = "definition",
    deny_unknown_fields
)]
pub enum DataCompositionDataSet {
    Query(DataCompositionQueryDataSet),
    Object(DataCompositionObjectDataSet),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataCompositionCalculatedField {
    path: DataFieldPath,
    expression: DataCompositionExpression,
    title: Option<SynonymText>,
    value_type: Option<MetadataValueType>,
}

impl DataCompositionCalculatedField {
    pub const fn new(path: DataFieldPath, expression: DataCompositionExpression) -> Self {
        Self {
            path,
            expression,
            title: None,
            value_type: None,
        }
    }
    pub const fn path(&self) -> &DataFieldPath {
        &self.path
    }
    pub const fn expression(&self) -> &DataCompositionExpression {
        &self.expression
    }
    pub const fn title(&self) -> Option<&SynonymText> {
        self.title.as_ref()
    }
    pub const fn value_type(&self) -> Option<MetadataValueType> {
        self.value_type
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataCompositionTotal {
    path: DataFieldPath,
    expression: DataCompositionExpression,
    groups: Vec<DataFieldPath>,
}

impl DataCompositionTotal {
    pub const fn new(path: DataFieldPath, expression: DataCompositionExpression) -> Self {
        Self {
            path,
            expression,
            groups: Vec::new(),
        }
    }
    pub const fn path(&self) -> &DataFieldPath {
        &self.path
    }
    pub const fn expression(&self) -> &DataCompositionExpression {
        &self.expression
    }
    pub fn groups(&self) -> &[DataFieldPath] {
        &self.groups
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "kind",
    content = "value",
    deny_unknown_fields
)]
pub enum DataCompositionValue {
    Null,
    Boolean(bool),
    Integer(i64),
    Text(MetadataPropertyText),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataCompositionParameter {
    name: DataCompositionParameterName,
    title: Option<SynonymText>,
    value_type: Option<MetadataValueType>,
    value: Option<DataCompositionValue>,
    expression: Option<DataCompositionExpression>,
    hidden: bool,
}
impl DataCompositionParameter {
    pub const fn new(name: DataCompositionParameterName) -> Self {
        Self {
            name,
            title: None,
            value_type: None,
            value: None,
            expression: None,
            hidden: false,
        }
    }
    pub const fn name(&self) -> &DataCompositionParameterName {
        &self.name
    }
    pub const fn title(&self) -> Option<&SynonymText> {
        self.title.as_ref()
    }
    pub const fn value_type(&self) -> Option<MetadataValueType> {
        self.value_type
    }
    pub const fn value(&self) -> Option<&DataCompositionValue> {
        self.value.as_ref()
    }
    pub const fn expression(&self) -> Option<&DataCompositionExpression> {
        self.expression.as_ref()
    }
    pub const fn is_hidden(&self) -> bool {
        self.hidden
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataCompositionSelection {
    field: DataFieldPath,
    title: Option<SynonymText>,
    enabled: bool,
}
impl DataCompositionSelection {
    pub const fn new(field: DataFieldPath) -> Self {
        Self {
            field,
            title: None,
            enabled: true,
        }
    }
    pub const fn field(&self) -> &DataFieldPath {
        &self.field
    }
    pub const fn title(&self) -> Option<&SynonymText> {
        self.title.as_ref()
    }
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataCompositionFilterOperator {
    Equal,
    NotEqual,
    Greater,
    Less,
    InList,
    Contains,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataCompositionFilter {
    field: DataFieldPath,
    operator: DataCompositionFilterOperator,
    value: DataCompositionValue,
    enabled: bool,
}
impl DataCompositionFilter {
    pub const fn new(
        field: DataFieldPath,
        operator: DataCompositionFilterOperator,
        value: DataCompositionValue,
    ) -> Self {
        Self {
            field,
            operator,
            value,
            enabled: true,
        }
    }
    pub const fn field(&self) -> &DataFieldPath {
        &self.field
    }
    pub const fn operator(&self) -> DataCompositionFilterOperator {
        self.operator
    }
    pub const fn value(&self) -> &DataCompositionValue {
        &self.value
    }
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataCompositionOrderDirection {
    Ascending,
    Descending,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataCompositionOrder {
    field: DataFieldPath,
    direction: DataCompositionOrderDirection,
    enabled: bool,
}
impl DataCompositionOrder {
    pub const fn new(field: DataFieldPath, direction: DataCompositionOrderDirection) -> Self {
        Self {
            field,
            direction,
            enabled: true,
        }
    }
    pub const fn field(&self) -> &DataFieldPath {
        &self.field
    }
    pub const fn direction(&self) -> DataCompositionOrderDirection {
        self.direction
    }
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataCompositionSettings {
    selection: Vec<DataCompositionSelection>,
    filters: Vec<DataCompositionFilter>,
    order: Vec<DataCompositionOrder>,
}
impl DataCompositionSettings {
    pub const fn empty() -> Self {
        Self {
            selection: Vec::new(),
            filters: Vec::new(),
            order: Vec::new(),
        }
    }
    pub fn selection(&self) -> &[DataCompositionSelection] {
        &self.selection
    }
    pub fn filters(&self) -> &[DataCompositionFilter] {
        &self.filters
    }
    pub fn order(&self) -> &[DataCompositionOrder] {
        &self.order
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataCompositionVariant {
    name: VariantName,
    presentation: Option<SynonymText>,
    settings: DataCompositionSettings,
}
impl DataCompositionVariant {
    pub const fn new(name: VariantName, settings: DataCompositionSettings) -> Self {
        Self {
            name,
            presentation: None,
            settings,
        }
    }
    pub const fn name(&self) -> &VariantName {
        &self.name
    }
    pub const fn presentation(&self) -> Option<&SynonymText> {
        self.presentation.as_ref()
    }
    pub const fn settings(&self) -> &DataCompositionSettings {
        &self.settings
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataCompositionDataSetLink {
    source: DataSetName,
    destination: DataSetName,
    source_expression: DataCompositionExpression,
    destination_expression: DataCompositionExpression,
    required: bool,
}
impl DataCompositionDataSetLink {
    pub const fn source(&self) -> &DataSetName {
        &self.source
    }
    pub const fn destination(&self) -> &DataSetName {
        &self.destination
    }
    pub const fn source_expression(&self) -> &DataCompositionExpression {
        &self.source_expression
    }
    pub const fn destination_expression(&self) -> &DataCompositionExpression {
        &self.destination_expression
    }
    pub const fn is_required(&self) -> bool {
        self.required
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataCompositionConditionalAppearance {
    condition: DataCompositionExpression,
    fields: Vec<DataFieldPath>,
    presentation: Option<SynonymText>,
}
impl DataCompositionConditionalAppearance {
    pub const fn condition(&self) -> &DataCompositionExpression {
        &self.condition
    }
    pub fn fields(&self) -> &[DataFieldPath] {
        &self.fields
    }
    pub const fn presentation(&self) -> Option<&SynonymText> {
        self.presentation.as_ref()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataCompositionDrilldown {
    field: DataFieldPath,
    target: MetadataObjectReference,
}
impl DataCompositionDrilldown {
    pub const fn field(&self) -> &DataFieldPath {
        &self.field
    }
    pub const fn target(&self) -> &MetadataObjectReference {
        &self.target
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataCompositionOutputParameter {
    name: DataCompositionParameterName,
    value: DataCompositionValue,
}
impl DataCompositionOutputParameter {
    pub const fn name(&self) -> &DataCompositionParameterName {
        &self.name
    }
    pub const fn value(&self) -> &DataCompositionValue {
        &self.value
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "kind",
    content = "value",
    deny_unknown_fields
)]
pub enum DataCompositionStructureItem {
    Field(DataFieldPath),
    Group {
        name: DataCompositionParameterName,
        title: Option<SynonymText>,
        items: Vec<DataCompositionStructureItem>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataCompositionStructure {
    items: Vec<DataCompositionStructureItem>,
}
impl DataCompositionStructure {
    pub const fn new(items: Vec<DataCompositionStructureItem>) -> Self {
        Self { items }
    }
    pub fn items(&self) -> &[DataCompositionStructureItem] {
        &self.items
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataCompositionFieldRole {
    Dimension,
    Attribute,
    Measure,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataCompositionFieldRoleAssignment {
    field: DataFieldPath,
    role: DataCompositionFieldRole,
}
impl DataCompositionFieldRoleAssignment {
    pub const fn field(&self) -> &DataFieldPath {
        &self.field
    }
    pub const fn role(&self) -> DataCompositionFieldRole {
        self.role
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "operation",
    content = "value",
    deny_unknown_fields
)]
pub enum DataCompositionParameterChange {
    SetTitle(SynonymText),
    SetValue(DataCompositionValue),
    SetExpression(DataCompositionExpression),
    SetHidden(bool),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataCompositionParameterPatch {
    name: DataCompositionParameterName,
    #[serde(deserialize_with = "deserialize_non_empty_vec")]
    changes: Vec<DataCompositionParameterChange>,
}
impl DataCompositionParameterPatch {
    pub fn new(
        name: DataCompositionParameterName,
        changes: Vec<DataCompositionParameterChange>,
    ) -> Result<Self, SemanticValueError> {
        if changes.is_empty() {
            return Err(SemanticValueError::Empty);
        }
        Ok(Self { name, changes })
    }
    pub const fn name(&self) -> &DataCompositionParameterName {
        &self.name
    }
    pub fn changes(&self) -> &[DataCompositionParameterChange] {
        &self.changes
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataCompositionParameterRename {
    from: DataCompositionParameterName,
    to: DataCompositionParameterName,
}
impl DataCompositionParameterRename {
    pub const fn from(&self) -> &DataCompositionParameterName {
        &self.from
    }
    pub const fn to(&self) -> &DataCompositionParameterName {
        &self.to
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct DataCompositionParameterOrder(Vec<DataCompositionParameterName>);

impl DataCompositionParameterOrder {
    pub fn new(values: Vec<DataCompositionParameterName>) -> Result<Self, SemanticValueError> {
        if values.is_empty() {
            return Err(SemanticValueError::Empty);
        }
        Ok(Self(values))
    }
    pub fn values(&self) -> &[DataCompositionParameterName] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for DataCompositionParameterOrder {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let values = Vec::<DataCompositionParameterName>::deserialize(deserializer)?;
        Self::new(values).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataCompositionDefinition {
    data_sources: Vec<DataCompositionDataSource>,
    #[serde(deserialize_with = "deserialize_non_empty_vec")]
    data_sets: Vec<DataCompositionDataSet>,
    data_set_links: Vec<DataCompositionDataSetLink>,
    calculated_fields: Vec<DataCompositionCalculatedField>,
    totals: Vec<DataCompositionTotal>,
    parameters: Vec<DataCompositionParameter>,
    variants: Vec<DataCompositionVariant>,
}
impl DataCompositionDefinition {
    pub fn new(data_sets: Vec<DataCompositionDataSet>) -> Result<Self, SemanticValueError> {
        if data_sets.is_empty() {
            return Err(SemanticValueError::Empty);
        }
        Ok(Self {
            data_sources: Vec::new(),
            data_sets,
            data_set_links: Vec::new(),
            calculated_fields: Vec::new(),
            totals: Vec::new(),
            parameters: Vec::new(),
            variants: Vec::new(),
        })
    }
    pub fn data_sources(&self) -> &[DataCompositionDataSource] {
        &self.data_sources
    }
    pub fn data_sets(&self) -> &[DataCompositionDataSet] {
        &self.data_sets
    }
    pub fn data_set_links(&self) -> &[DataCompositionDataSetLink] {
        &self.data_set_links
    }
    pub fn calculated_fields(&self) -> &[DataCompositionCalculatedField] {
        &self.calculated_fields
    }
    pub fn totals(&self) -> &[DataCompositionTotal] {
        &self.totals
    }
    pub fn parameters(&self) -> &[DataCompositionParameter] {
        &self.parameters
    }
    pub fn variants(&self) -> &[DataCompositionVariant] {
        &self.variants
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataCompositionCreate {
    definition: DataCompositionDefinition,
}
impl DataCompositionCreate {
    pub const fn new(definition: DataCompositionDefinition) -> Self {
        Self { definition }
    }
    pub const fn definition(&self) -> &DataCompositionDefinition {
        &self.definition
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataCompositionScope {
    Root,
    Variant,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataCompositionClearTarget {
    Selection,
    Order,
    Filter,
    ConditionalAppearance,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "operation",
    content = "payload",
    deny_unknown_fields
)]
pub enum DataCompositionMutation {
    AddField(DataCompositionFieldDefinition),
    AddTotal(DataCompositionTotal),
    AddCalculatedField(DataCompositionCalculatedField),
    AddParameter(DataCompositionParameter),
    AddFilter(DataCompositionFilter),
    AddDataParameter(DataCompositionParameter),
    SetQuery(DataCompositionQueryText),
    PatchQuery {
        find: DataCompositionQueryText,
        replace: DataCompositionQueryText,
    },
    Clear {
        target: DataCompositionClearTarget,
        scope: DataCompositionScope,
    },
    AddSelection(DataCompositionSelection),
    AddOrder(DataCompositionOrder),
    AddDataSetLink(DataCompositionDataSetLink),
    AddDataSet(DataCompositionDataSet),
    AddVariant(DataCompositionVariant),
    AddConditionalAppearance(DataCompositionConditionalAppearance),
    AddDrilldown(DataCompositionDrilldown),
    SetOutputParameter(DataCompositionOutputParameter),
    SetStructure(DataCompositionStructure),
    ModifyStructure(DataCompositionStructure),
    RemoveField(DataFieldPath),
    RemoveParameter(DataCompositionParameterName),
    ModifyField(DataCompositionFieldDefinition),
    SetFieldRole(DataCompositionFieldRoleAssignment),
    ModifyFilter(DataCompositionFilter),
    ModifyDataParameter(DataCompositionParameter),
    ModifyParameter(DataCompositionParameterPatch),
    RenameParameter(DataCompositionParameterRename),
    ReorderParameters(DataCompositionParameterOrder),
    RemoveTotal(DataFieldPath),
    RemoveCalculatedField(DataFieldPath),
    RemoveFilter(DataFieldPath),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpreadsheetPageOrientation {
    Portrait,
    Landscape,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpreadsheetHorizontalAlignment {
    Left,
    Center,
    Right,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpreadsheetVerticalAlignment {
    Top,
    Center,
    Bottom,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpreadsheetBorderStyle {
    None,
    Thin,
    Thick,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpreadsheetBorderSide {
    Left,
    Top,
    Right,
    Bottom,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpreadsheetFont {
    name: SpreadsheetFontName,
    face: Option<DescriptionText>,
    bold: bool,
    italic: bool,
    underline: bool,
    strikeout: bool,
    size: Option<u16>,
}
impl SpreadsheetFont {
    pub fn new(
        name: SpreadsheetFontName,
        face: Option<DescriptionText>,
        bold: bool,
        italic: bool,
        size: Option<u16>,
    ) -> Self {
        Self {
            name,
            face,
            bold,
            italic,
            underline: false,
            strikeout: false,
            size,
        }
    }
    pub const fn with_underline(mut self, value: bool) -> Self {
        self.underline = value;
        self
    }
    pub const fn with_strikeout(mut self, value: bool) -> Self {
        self.strikeout = value;
        self
    }
    pub const fn name(&self) -> &SpreadsheetFontName {
        &self.name
    }
    pub const fn face(&self) -> Option<&DescriptionText> {
        self.face.as_ref()
    }
    pub const fn is_bold(&self) -> bool {
        self.bold
    }
    pub const fn is_italic(&self) -> bool {
        self.italic
    }
    pub const fn is_underlined(&self) -> bool {
        self.underline
    }
    pub const fn is_struck_out(&self) -> bool {
        self.strikeout
    }
    pub const fn size(&self) -> Option<u16> {
        self.size
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpreadsheetStyle {
    name: SpreadsheetStyleName,
    font: Option<SpreadsheetFontName>,
    border: SpreadsheetBorderStyle,
    border_sides: Vec<SpreadsheetBorderSide>,
    horizontal: Option<SpreadsheetHorizontalAlignment>,
    vertical: Option<SpreadsheetVerticalAlignment>,
    wrap: bool,
    number_format: Option<SpreadsheetNumberFormat>,
}
impl SpreadsheetStyle {
    pub fn new(
        name: SpreadsheetStyleName,
        font: Option<SpreadsheetFontName>,
        border: SpreadsheetBorderStyle,
        horizontal: Option<SpreadsheetHorizontalAlignment>,
        vertical: Option<SpreadsheetVerticalAlignment>,
        wrap: bool,
        number_format: Option<SpreadsheetNumberFormat>,
    ) -> Self {
        let border_sides = if matches!(border, SpreadsheetBorderStyle::None) {
            Vec::new()
        } else {
            vec![
                SpreadsheetBorderSide::Left,
                SpreadsheetBorderSide::Top,
                SpreadsheetBorderSide::Right,
                SpreadsheetBorderSide::Bottom,
            ]
        };
        Self {
            name,
            font,
            border,
            border_sides,
            horizontal,
            vertical,
            wrap,
            number_format,
        }
    }
    pub fn with_border_sides(mut self, value: Vec<SpreadsheetBorderSide>) -> Self {
        self.border_sides = value;
        self
    }
    pub const fn name(&self) -> &SpreadsheetStyleName {
        &self.name
    }
    pub const fn font(&self) -> Option<&SpreadsheetFontName> {
        self.font.as_ref()
    }
    pub const fn border(&self) -> SpreadsheetBorderStyle {
        self.border
    }
    pub fn border_sides(&self) -> &[SpreadsheetBorderSide] {
        &self.border_sides
    }
    pub const fn horizontal(&self) -> Option<SpreadsheetHorizontalAlignment> {
        self.horizontal
    }
    pub const fn vertical(&self) -> Option<SpreadsheetVerticalAlignment> {
        self.vertical
    }
    pub const fn wraps(&self) -> bool {
        self.wrap
    }
    pub const fn number_format(&self) -> Option<&SpreadsheetNumberFormat> {
        self.number_format.as_ref()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "kind",
    content = "value",
    deny_unknown_fields
)]
pub enum SpreadsheetCellValue {
    Empty,
    Text(SpreadsheetCellText),
    Parameter(FormParameterName),
    Template(SpreadsheetCellText),
    Detail(SpreadsheetCellText),
    Composite(SpreadsheetCellContent),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpreadsheetCellContent {
    text: Option<SpreadsheetCellText>,
    parameter: Option<FormParameterName>,
    template: Option<SpreadsheetCellText>,
    detail: Option<SpreadsheetCellText>,
}

impl<'de> Deserialize<'de> for SpreadsheetCellContent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            text: Option<SpreadsheetCellText>,
            parameter: Option<FormParameterName>,
            template: Option<SpreadsheetCellText>,
            detail: Option<SpreadsheetCellText>,
        }

        let value = Wire::deserialize(deserializer)?;
        Self::new(value.text, value.parameter, value.template, value.detail)
            .map_err(serde::de::Error::custom)
    }
}
impl SpreadsheetCellContent {
    pub fn new(
        text: Option<SpreadsheetCellText>,
        parameter: Option<FormParameterName>,
        template: Option<SpreadsheetCellText>,
        detail: Option<SpreadsheetCellText>,
    ) -> Result<Self, SemanticValueError> {
        if text.is_none() && parameter.is_none() && template.is_none() && detail.is_none() {
            return Err(SemanticValueError::Empty);
        }
        Ok(Self {
            text,
            parameter,
            template,
            detail,
        })
    }
    pub const fn text(&self) -> Option<&SpreadsheetCellText> {
        self.text.as_ref()
    }
    pub const fn parameter(&self) -> Option<&FormParameterName> {
        self.parameter.as_ref()
    }
    pub const fn template(&self) -> Option<&SpreadsheetCellText> {
        self.template.as_ref()
    }
    pub const fn detail(&self) -> Option<&SpreadsheetCellText> {
        self.detail.as_ref()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpreadsheetCell {
    #[serde(alias = "col", deserialize_with = "deserialize_non_zero_u32")]
    column: u32,
    #[serde(
        alias = "span",
        default = "one_u16",
        deserialize_with = "deserialize_non_zero_u16"
    )]
    column_span: u16,
    #[serde(
        alias = "rowspan",
        default = "one_u16",
        deserialize_with = "deserialize_non_zero_u16"
    )]
    row_span: u16,
    style: Option<SpreadsheetStyleName>,
    value: SpreadsheetCellValue,
}
impl SpreadsheetCell {
    pub fn new(column: u32, value: SpreadsheetCellValue) -> Result<Self, SemanticValueError> {
        if column == 0 {
            return Err(SemanticValueError::Empty);
        }
        Ok(Self {
            column,
            column_span: 1,
            row_span: 1,
            style: None,
            value,
        })
    }
    pub fn with_span(
        mut self,
        column_span: u16,
        row_span: u16,
    ) -> Result<Self, SemanticValueError> {
        if column_span == 0 || row_span == 0 {
            return Err(SemanticValueError::Empty);
        }
        self.column_span = column_span;
        self.row_span = row_span;
        Ok(self)
    }
    pub fn with_style(mut self, style: Option<SpreadsheetStyleName>) -> Self {
        self.style = style;
        self
    }
    pub const fn column(&self) -> u32 {
        self.column
    }
    pub const fn column_span(&self) -> u16 {
        self.column_span
    }
    pub const fn row_span(&self) -> u16 {
        self.row_span
    }
    pub const fn style(&self) -> Option<&SpreadsheetStyleName> {
        self.style.as_ref()
    }
    pub const fn value(&self) -> &SpreadsheetCellValue {
        &self.value
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpreadsheetRow {
    height: Option<u32>,
    style: Option<SpreadsheetStyleName>,
    cells: Vec<SpreadsheetCell>,
}
impl SpreadsheetRow {
    pub const fn new(cells: Vec<SpreadsheetCell>) -> Self {
        Self {
            height: None,
            style: None,
            cells,
        }
    }
    pub const fn with_height(mut self, value: Option<u32>) -> Self {
        self.height = value;
        self
    }
    pub fn with_style(mut self, value: Option<SpreadsheetStyleName>) -> Self {
        self.style = value;
        self
    }
    pub const fn height(&self) -> Option<u32> {
        self.height
    }
    pub const fn style(&self) -> Option<&SpreadsheetStyleName> {
        self.style.as_ref()
    }
    pub fn cells(&self) -> &[SpreadsheetCell] {
        &self.cells
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpreadsheetArea {
    name: SpreadsheetAreaName,
    #[serde(deserialize_with = "deserialize_non_empty_vec")]
    rows: Vec<SpreadsheetRow>,
}
impl SpreadsheetArea {
    pub fn new(
        name: SpreadsheetAreaName,
        rows: Vec<SpreadsheetRow>,
    ) -> Result<Self, SemanticValueError> {
        if rows.is_empty() {
            return Err(SemanticValueError::Empty);
        }
        Ok(Self { name, rows })
    }
    pub const fn name(&self) -> &SpreadsheetAreaName {
        &self.name
    }
    pub fn rows(&self) -> &[SpreadsheetRow] {
        &self.rows
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpreadsheetColumnWidth {
    #[serde(deserialize_with = "deserialize_non_zero_u32")]
    column: u32,
    #[serde(deserialize_with = "deserialize_non_zero_u32")]
    width: u32,
}
impl SpreadsheetColumnWidth {
    pub fn new(column: u32, width: u32) -> Result<Self, SemanticValueError> {
        if column == 0 || width == 0 {
            return Err(SemanticValueError::Empty);
        }
        Ok(Self { column, width })
    }
    pub const fn column(&self) -> u32 {
        self.column
    }
    pub const fn width(&self) -> u32 {
        self.width
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpreadsheetDocument {
    #[serde(deserialize_with = "deserialize_non_zero_u32")]
    column_count: u32,
    #[serde(deserialize_with = "deserialize_non_zero_u32")]
    default_width: u32,
    page: Option<SpreadsheetPageOrientation>,
    fonts: Vec<SpreadsheetFont>,
    styles: Vec<SpreadsheetStyle>,
    column_widths: Vec<SpreadsheetColumnWidth>,
    #[serde(deserialize_with = "deserialize_non_empty_vec")]
    areas: Vec<SpreadsheetArea>,
}
impl SpreadsheetDocument {
    pub fn new(areas: Vec<SpreadsheetArea>) -> Result<Self, SemanticValueError> {
        if areas.is_empty() {
            return Err(SemanticValueError::Empty);
        }
        let column_count = areas
            .iter()
            .flat_map(|area| area.rows())
            .flat_map(|row| row.cells())
            .map(|cell| {
                cell.column()
                    .saturating_add(u32::from(cell.column_span()))
                    .saturating_sub(1)
            })
            .max()
            .unwrap_or(1);
        Ok(Self {
            column_count,
            default_width: 10,
            page: None,
            fonts: Vec::new(),
            styles: Vec::new(),
            column_widths: Vec::new(),
            areas,
        })
    }
    pub fn with_column_count(mut self, value: u32) -> Result<Self, SemanticValueError> {
        if value == 0 {
            return Err(SemanticValueError::Empty);
        }
        self.column_count = value;
        Ok(self)
    }
    pub fn with_default_width(mut self, value: u32) -> Result<Self, SemanticValueError> {
        if value == 0 {
            return Err(SemanticValueError::Empty);
        }
        self.default_width = value;
        Ok(self)
    }
    pub fn with_page(mut self, value: Option<SpreadsheetPageOrientation>) -> Self {
        self.page = value;
        self
    }
    pub fn with_fonts(mut self, value: Vec<SpreadsheetFont>) -> Self {
        self.fonts = value;
        self
    }
    pub fn with_styles(mut self, value: Vec<SpreadsheetStyle>) -> Self {
        self.styles = value;
        self
    }
    pub fn with_column_widths(mut self, value: Vec<SpreadsheetColumnWidth>) -> Self {
        self.column_widths = value;
        self
    }
    pub const fn column_count(&self) -> u32 {
        self.column_count
    }
    pub const fn default_width(&self) -> u32 {
        self.default_width
    }
    pub const fn page(&self) -> Option<SpreadsheetPageOrientation> {
        self.page
    }
    pub fn fonts(&self) -> &[SpreadsheetFont] {
        &self.fonts
    }
    pub fn styles(&self) -> &[SpreadsheetStyle] {
        &self.styles
    }
    pub fn column_widths(&self) -> &[SpreadsheetColumnWidth] {
        &self.column_widths
    }
    pub fn areas(&self) -> &[SpreadsheetArea] {
        &self.areas
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "source",
    content = "value",
    deny_unknown_fields
)]
pub enum SpreadsheetSource {
    Definition(SpreadsheetDocument),
    Object {
        processor: Option<ProcessorName>,
        template: Option<TemplateName>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpreadsheetCreate {
    source: SpreadsheetSource,
}
impl SpreadsheetCreate {
    pub const fn new(document: SpreadsheetDocument) -> Self {
        Self {
            source: SpreadsheetSource::Definition(document),
        }
    }
    pub const fn from_object(
        processor: Option<ProcessorName>,
        template: Option<TemplateName>,
    ) -> Self {
        Self {
            source: SpreadsheetSource::Object {
                processor,
                template,
            },
        }
    }
    pub const fn source(&self) -> &SpreadsheetSource {
        &self.source
    }
    pub const fn processor(&self) -> Option<&ProcessorName> {
        match &self.source {
            SpreadsheetSource::Object { processor, .. } => processor.as_ref(),
            _ => None,
        }
    }
    pub const fn template(&self) -> Option<&TemplateName> {
        match &self.source {
            SpreadsheetSource::Object { template, .. } => template.as_ref(),
            _ => None,
        }
    }
    pub const fn derives_from_object(&self) -> bool {
        matches!(self.source, SpreadsheetSource::Object { .. })
    }
}
