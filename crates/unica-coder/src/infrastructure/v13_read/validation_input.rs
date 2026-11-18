//! Входы проверок XDTO, подсистем, форм, СКД и табличных документов.

use super::*;

impl<'a> LogicalViewReadAuthority<'a> {
    /// One retained read prepares the complete package verdict. A separate
    /// public view would parse and validate the same XDTO model a second time.
    pub fn check_xdto(
        &self,
        at: &QualifiedAddress,
    ) -> Result<
        (
            ViewSourceSnapshot,
            Option<crate::application::v13::check::NativeCheckOutcome>,
        ),
        ViewError,
    > {
        if !matches!(at.segments(), [owner] if owner.kind() == NodeKind::XdtoPackage && owner.name().is_some())
        {
            return Err(ViewError::new(
                RefusalCode::BadValue,
                "XDTO check requires a named package root",
            ));
        }
        let admitted = self.snapshot(at)?;
        let canonical = self.canonical_address(at, &admitted)?;
        let route = route_logical_address(&canonical, self.profile)
            .map_err(|error| ViewError::new(RefusalCode::BadValue, error.to_string()))?;
        let target = route
            .reader_metadata_path()
            .ok_or_else(|| ViewError::new(RefusalCode::BadValue, "XDTO check has no descriptor"))?;
        self.verify_registered_owner(target, &admitted)?;
        if self
            .read
            .inherited_resource(target, "Package.bin")?
            .is_some()
        {
            self.read_checkpoint()?;
            return Ok((admitted, None));
        }
        let outcome = self
            .read
            .xdto_check_outcome(target, &mut || self.read_checkpoint())?;
        self.read_checkpoint()?;
        Ok((admitted, Some(outcome)))
    }

    pub fn validate_subsystem(
        &self,
        at: &QualifiedAddress,
    ) -> Result<crate::application::v13::check::NativeCheckOutcome, ViewError> {
        let admitted = self.snapshot(at)?;
        let canonical = self.canonical_address(at, &admitted)?;
        let route = route_logical_address(&canonical, self.profile)
            .map_err(|error| ViewError::new(RefusalCode::BadValue, error.to_string()))?;
        let target = route.reader_metadata_path().ok_or_else(|| {
            ViewError::new(RefusalCode::BadValue, "subsystem check has no descriptor")
        })?;
        self.verify_registered_owner(target, &admitted)?;
        self.read
            .validate_subsystem(&canonical, &mut || self.read_checkpoint())
    }

    /// Проверка формы использует ту же регистрацию и удержанный корень, что view.
    pub fn form_validation_input(
        &self,
        at: &QualifiedAddress,
    ) -> Result<crate::infrastructure::v13_read_port::FormValidationInput, ViewError> {
        let admitted = self.snapshot(at)?;
        let canonical = self.canonical_address(at, &admitted)?;
        let target = crate::infrastructure::form_composition::typed_form_reader_target(&canonical)
            .ok_or_else(|| ViewError::new(RefusalCode::BadValue, "Проверка требует адрес формы"))?;
        self.verify_registered_owner(&target, &admitted)?;
        self.read_checkpoint()?;
        let input = self.read.form_validation_input(
            &target,
            Some(self.platform_profile().line()),
            &mut || self.read_checkpoint(),
        )?;
        self.read_checkpoint()?;
        Ok(input)
    }

    /// Proves a DCS template through the same registration and descriptor
    /// authority as view, then reads its body through the retained source root.
    pub fn dcs_validation_input(
        &self,
        at: &QualifiedAddress,
    ) -> Result<crate::infrastructure::v13_read_port::TemplateValidationInput, ViewError> {
        let admitted = self.snapshot(at)?;
        let canonical = self.canonical_address(at, &admitted)?;
        let target =
            crate::infrastructure::native_operations::dcs::typed_dcs_reader_target(&canonical)
                .ok_or_else(|| {
                    ViewError::new(RefusalCode::BadValue, "DCS validation requires a template")
                })?;
        self.verify_registered_owner(&target, &admitted)?;
        if !matches!(
            self.read.metadata_child_profile(&target)?,
            unica_metadata::validation_subject::MetadataChildProfile::Template(
                unica_metadata::validation_subject::MetadataTemplateType::DataCompositionSchema
            )
        ) {
            return Err(ViewError::new(
                RefusalCode::BadValue,
                "template is not a data composition schema",
            ));
        }
        self.read_checkpoint()?;
        let input = self
            .read
            .dcs_validation_input(&target, &mut || self.read_checkpoint())?;
        self.read_checkpoint()?;
        Ok(input)
    }

    /// Proves a MXL template through the same registration and descriptor
    /// authority as view, then reads its body through the retained source root.
    pub fn mxl_validation_input(
        &self,
        at: &QualifiedAddress,
    ) -> Result<crate::infrastructure::v13_read_port::TemplateValidationInput, ViewError> {
        let admitted = self.snapshot(at)?;
        let canonical = self.canonical_address(at, &admitted)?;
        let target =
            crate::infrastructure::native_operations::mxl_composition::typed_mxl_reader_target(
                &canonical,
            )
            .ok_or_else(|| {
                ViewError::new(RefusalCode::BadValue, "MXL validation requires a template")
            })?;
        self.verify_registered_owner(&target, &admitted)?;
        if !matches!(
            self.read.metadata_child_profile(&target)?,
            unica_metadata::validation_subject::MetadataChildProfile::Template(
                unica_metadata::validation_subject::MetadataTemplateType::SpreadsheetDocument
            )
        ) {
            return Err(ViewError::new(
                RefusalCode::BadValue,
                "template is not a spreadsheet document",
            ));
        }
        self.read_checkpoint()?;
        let input = self
            .read
            .mxl_validation_input(&target, &mut || self.read_checkpoint())?;
        self.read_checkpoint()?;
        Ok(input)
    }
}
