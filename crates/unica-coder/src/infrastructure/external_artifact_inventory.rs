//! Shared external-owner observations over the actor's retained source directory.

use std::path::{Path, PathBuf};
use unica_execution_contract::view_error::ViewError;
use unica_refusal::RefusalDetail;
use unica_source_contract::{read::SourceDirectory, SourceSetKind};
use unica_source_designer::external_artifact::{
    ExternalArtifactBinding, ExternalArtifactInventory,
};
use unica_source_designer::owner::prove_already_read_source_set_owner;
use unica_source_designer::probe::{
    classify_already_read_config_dump_info_xml, ConfigDumpInfoXmlKind,
};

pub(crate) fn read_inventory(
    root: &SourceDirectory,
    source_kind: SourceSetKind,
    checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
) -> Result<ExternalArtifactInventory, ViewError> {
    checkpoint()?;
    let names = root
        .read_immediate_names_with_limit(None, || {
            checkpoint().map_err(|error| {
                std::io::Error::new(std::io::ErrorKind::Interrupted, error.to_string())
            })
        })
        .map_err(|error| {
            checkpoint().err().unwrap_or_else(|| {
                ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
            })
        })?;
    let mut inventory = ExternalArtifactInventory::default();
    for name in names {
        checkpoint()?;
        let relative = PathBuf::from(&name);
        if relative
            .extension()
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("xml"))
        {
            continue;
        }
        let mut bytes = Vec::new();
        let mut interrupted = None;
        let visited = root.visit_relative_regular_chunks(
            &relative,
            || {
                checkpoint().map_err(|error| {
                    interrupted = Some(error);
                    std::io::Error::other("external descriptor read interrupted")
                })
            },
            |chunk| {
                bytes
                    .try_reserve(chunk.len())
                    .map_err(std::io::Error::other)?;
                bytes.extend_from_slice(chunk);
                Ok(())
            },
        );
        if let Some(error) = interrupted {
            return Err(error);
        }
        visited.map_err(|error| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
        })?;
        checkpoint()?;
        if Path::new(&name)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("ConfigDumpInfo.xml"))
            && classify_already_read_config_dump_info_xml(&bytes)
                == ConfigDumpInfoXmlKind::RuntimeSidecar
        {
            continue;
        }
        let evidence = prove_already_read_source_set_owner(&relative, &bytes, source_kind)
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.message))?;
        let name = evidence.artifact_name().ok_or_else(|| {
            ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "external descriptor has no non-empty Properties/Name",
            )
        })?;
        let binding = ExternalArtifactBinding::new(source_kind, name, &relative)
            .map_err(|message| ViewError::detailed(RefusalDetail::SourceUnreadable, message))?;
        if evidence.artifact_kind() != binding.kind() {
            return Err(ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "external descriptor has the wrong owner kind",
            ));
        }
        inventory
            .insert(binding)
            .map_err(|message| ViewError::detailed(RefusalDetail::SourceUnreadable, message))?;
    }
    checkpoint()?;
    Ok(inventory)
}
