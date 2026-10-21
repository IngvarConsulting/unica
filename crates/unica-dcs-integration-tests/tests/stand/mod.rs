//! Набор исходников с одним языком и предложение workspace над ним: запись
//! кодека проверяется на настоящем стадированном состоянии.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use unica_dcs::codec::DcsCodec;
use unica_dcs::edit::{plan_edit, DcsEdit};
use unica_dcs::operations::Primitive;
use unica_dcs_designer::DesignerDcsCodec;
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, IndexedPlanOperation, ProvisionalApplyEffect,
};
use unica_source_contract::read::{
    BoundSourceArtifactPolicy, RevisionArtifactDisposition, SourceArtifactClassifier,
};
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_contract::{SourceFormat, SourceProfile, SourceSetKind};
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::actor::{
    ApplyAdmission, DcsMxlApplyAuthority, ProviderRootBinding, WorkspaceActor, WorkspaceIdentity,
    WorkspaceSourceSetInput,
};
use unica_workspace::apply::ApplyStagedState;
use unica_workspace::policies::WorkspacePolicies;

/// Every source file is revision content: the stand stages, it never
/// classifies a source tree.
struct EveryFileIsContent;

impl BoundSourceArtifactPolicy for EveryFileIsContent {
    fn classify(&self, _relative: &Path) -> RevisionArtifactDisposition {
        RevisionArtifactDisposition::Content
    }
}

impl SourceArtifactClassifier for EveryFileIsContent {
    fn bind(
        &self,
        _kind: SourceSetKind,
        _format: SourceFormat,
        _profile: SourceProfile,
    ) -> Result<Arc<dyn BoundSourceArtifactPolicy>, String> {
        Ok(Arc::new(EveryFileIsContent))
    }
}

fn policies() -> WorkspacePolicies {
    WorkspacePolicies::new(
        Arc::new(
            unica_source_designer::project_sources::ActorSourceMapDiscovery::new(
                unica_workspace::paths::host_path_text,
            ),
        ),
        Arc::new(EveryFileIsContent),
    )
}

pub struct Stand {
    root: tempfile::TempDir,
    actor: WorkspaceActor,
    pub binding: ProviderRootBinding,
}

impl Stand {
    pub fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("src");
        std::fs::create_dir_all(source.join("Languages")).unwrap();
        std::fs::write(
            root.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Languages/Русский.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>Русский</Name><Comment/><LanguageCode>ru</LanguageCode></Properties></Language></MetaDataObject>"#,
        )
        .unwrap();
        let workspace_root = std::fs::canonicalize(root.path()).unwrap();
        let source = std::fs::canonicalize(source).unwrap();
        let context = WorkspaceContext {
            cwd: workspace_root.clone(),
            workspace_root: workspace_root.clone(),
            cache_root: workspace_root.join(".build/unica"),
            workspace_epoch: 1,
        };
        let identity = WorkspaceIdentity::new(
            &context,
            [WorkspaceSourceSetInput::new(
                "main",
                &source,
                SourceSetKind::Configuration,
                SourceFormat::PlatformXml,
                SourceProfile::platform_xml_8_3_27_format_2_20(),
            )],
            "dcs-codec-stage-test",
        )
        .unwrap();
        let actor = WorkspaceActor::new(identity, context)
            .unwrap()
            .with_policies(policies());
        let binding = actor.bind_provider_root("main", &source).unwrap();
        Self {
            root,
            actor,
            binding,
        }
    }

    /// The physical `src` root, for tests that lay out resources.
    pub fn source_dir(&self) -> PathBuf {
        std::fs::canonicalize(self.root.path().join("src")).unwrap()
    }

    /// Switches the only language from Russian to English.
    pub fn use_english_only_language(&self) {
        let configuration = self.source_dir().join("Configuration.xml");
        let text = std::fs::read_to_string(&configuration)
            .unwrap()
            .replace("Language.Русский", "Language.English")
            .replace(
                "<Language>Русский</Language>",
                "<Language>English</Language>",
            );
        std::fs::write(&configuration, text).unwrap();
        std::fs::remove_file(self.source_dir().join("Languages/Русский.xml")).unwrap();
        std::fs::write(
            self.source_dir().join("Languages/English.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>English</Name><Comment/><LanguageCode>en</LanguageCode></Properties></Language></MetaDataObject>"#,
        )
        .unwrap();
    }

    pub fn admission(&self) -> ApplyAdmission {
        self.actor
            .admit_apply(
                &self.binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap()
    }
}

/// The typed edit of one DCS operation, as the product planner decodes it.
pub fn plan(
    operation: &str,
    args: &serde_json::Value,
    op_index: usize,
) -> Result<DcsEdit, ApplyPlanError> {
    let primitive = Primitive::named(operation).expect("a DCS primitive");
    plan_edit(
        primitive,
        args.as_object().expect("object args"),
        op_index,
        "main",
    )
}

/// Stages the edits in order through the codec port.
pub fn stage_all(
    mut staged: ApplyStagedState,
    authority: DcsMxlApplyAuthority<'_>,
    operations: &[IndexedPlanOperation<DcsEdit>],
) -> Result<(ApplyStagedState, Vec<ProvisionalApplyEffect>), ApplyPlanError> {
    if !authority.owns_staged_state(&staged) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "DCS planner authority does not own the staged state",
        ));
    }
    let mut provisional = Vec::new();
    for operation in operations {
        DesignerDcsCodec.stage(
            &mut staged.proposal(),
            &authority,
            operation.operation(),
            operation.index(),
            &mut provisional,
        )?;
    }
    Ok((staged, provisional))
}
