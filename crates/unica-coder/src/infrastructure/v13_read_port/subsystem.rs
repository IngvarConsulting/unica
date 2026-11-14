use super::{path_text, ProviderReadAuthority, ViewError};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use unica_address::address::QualifiedAddress;
use unica_address::source_target::MetadataAddress;
use unica_refusal::{RefusalCode, RefusalDetail};
use unica_source_contract::portable_path::PortablePathExt;

impl ProviderReadAuthority {
    pub(crate) fn subsystem_payload(
        &self,
        target: &MetadataAddress,
        nested: &[String],
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Value, ViewError> {
        let (descriptor_relative, descriptor_bytes) =
            self.registered_subsystem_descriptor(target, nested, checkpoint)?;
        let descriptor = std::str::from_utf8(&descriptor_bytes).map_err(|_| {
            ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "subsystem descriptor is not UTF-8",
            )
        })?;
        let ci_relative = if nested.is_empty() {
            self.attached_resource_relative(target, "CommandInterface.xml")?
        } else {
            nested_subsystem_resource(&descriptor_relative, "CommandInterface.xml")
        };
        let command_interface = self
            .read_optional_relative_whole(&ci_relative)?
            .map(|bytes| {
                let text = std::str::from_utf8(&bytes).map_err(|_| {
                    ViewError::detailed(
                        RefusalDetail::SourceUnreadable,
                        "CommandInterface.xml is not UTF-8",
                    )
                })?;
                crate::infrastructure::family_codecs::metadata_read_codec()
                    .subsystem_command_interface(&ci_relative, text)
                    .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))
            })
            .transpose()?;
        let (data, _) = crate::infrastructure::family_codecs::metadata_read_codec()
            .subsystem_info(
                if nested.is_empty() {
                    Path::new(target.as_str())
                } else {
                    descriptor_relative.as_path()
                },
                descriptor,
                command_interface.is_some(),
            )
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
        if let Some(expected) = nested.last() {
            if &data.name != expected {
                return Err(ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    format!(
                        "subsystem descriptor `{}` names `{}` instead of `{expected}`",
                        descriptor_relative.portable(),
                        data.name
                    ),
                ));
            }
        }
        // Признак поддержки берётся из дескриптора читаемой подсистемы: у
        // вложенной он свой, а не корня.
        let support =
            self.object_support_with_descriptor(target, Some(&descriptor_bytes), checkpoint)?;
        let result = crate::infrastructure::family_codecs::metadata_read_codec().subsystem_result(
            data,
            None,
            command_interface,
            support,
        );
        serde_json::to_value(result)
            .map_err(|error| ViewError::new(RefusalCode::ProviderUnavailable, error.to_string()))
    }

    /// Спускается от корневой подсистемы к вложенной только по регистрации:
    /// каждое следующее имя должно стоять в `ChildObjects` родителя. Файл,
    /// лежащий на месте незарегистрированной подсистемы, подсистемой не
    /// является. Читаются только дескрипторы цепочки, а не вся структура.
    fn registered_subsystem_descriptor(
        &self,
        target: &MetadataAddress,
        nested: &[String],
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<(PathBuf, Vec<u8>), ViewError> {
        let mut relative = self.metadata_descriptor_relative(target)?;
        let mut descriptor = self.metadata_descriptor(target)?;
        for (depth, name) in nested.iter().enumerate() {
            checkpoint()?;
            let text = std::str::from_utf8(&descriptor).map_err(|_| {
                ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "subsystem descriptor is not UTF-8",
                )
            })?;
            let (parent, _) = crate::infrastructure::family_codecs::metadata_read_codec()
                .subsystem_info(&relative, text, false)
                .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
            // Промежуточный дескриптор обязан называть ту подсистему, по
            // регистрации которой к нему пришли; корень сверен владельцем.
            if let Some(expected) = depth.checked_sub(1).map(|index| &nested[index]) {
                if &parent.name != expected {
                    return Err(ViewError::detailed(
                        RefusalDetail::SourceUnreadable,
                        format!(
                            "subsystem descriptor `{}` names `{}` instead of `{expected}`",
                            relative.portable(),
                            parent.name
                        ),
                    ));
                }
            }
            if !parent.child_names.iter().any(|child| child == name) {
                return Err(ViewError::new(
                    RefusalCode::NotFound,
                    format!(
                        "subsystem `{name}` is not registered in `{}`",
                        relative.portable()
                    ),
                ));
            }
            relative = nested_subsystem_descriptor(&relative, name);
            descriptor = self
                .read_optional_relative_whole(&relative)?
                .ok_or_else(|| {
                    ViewError::new(
                        RefusalCode::NotFound,
                        format!("subsystem descriptor `{}` is absent", relative.portable()),
                    )
                })?;
        }
        Ok((relative, descriptor))
    }

    /// Check the selected descriptor and its direct dependencies under the same
    /// retained root used by view; legacy metadata paths cannot name this chain.
    pub(crate) fn validate_subsystem(
        &self,
        address: &QualifiedAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<crate::application::v13::check::NativeCheckOutcome, ViewError> {
        use crate::infrastructure::subsystem_composition::subsystem_validation_outcome;
        use unica_metadata::subsystem::SubsystemInfoFormatDocument;
        // Маршрут адреса подсистемы строится по маршрутам модулей и от линии
        // не зависит; каталог событий здесь не спрашивают.
        let route = crate::infrastructure::logical_tree::route_logical_address(
            address,
            unica_bsl::profile::PlatformProfile::for_line_or_base(None),
        )
        .map_err(|error| ViewError::new(RefusalCode::BadValue, error.to_string()))?;
        let target = route.reader_metadata_path().ok_or_else(|| {
            ViewError::new(RefusalCode::BadValue, "subsystem check has no descriptor")
        })?;
        let (relative, bytes) =
            self.registered_subsystem_descriptor(target, route.nested_subsystems(), checkpoint)?;
        let text = std::str::from_utf8(&bytes).map_err(|error| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
        })?;
        let (data, _) = crate::infrastructure::family_codecs::metadata_read_codec()
            .subsystem_info(&relative, text, false)
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
        let expected = address
            .segments()
            .last()
            .and_then(|segment| segment.name())
            .unwrap_or_default();
        if data.name != expected {
            return Err(ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                format!(
                    "subsystem descriptor names `{}` instead of `{expected}`",
                    data.name
                ),
            ));
        }
        let ci_relative = nested_subsystem_resource(&relative, "CommandInterface.xml");
        let ci = self.read_optional_relative_with_checkpoint(&ci_relative, checkpoint)?;
        let mut documents = vec![SubsystemInfoFormatDocument {
            path: self.root_path().join(&relative),
            bytes: bytes.clone(),
        }];
        if let Some(bytes) = &ci {
            documents.push(SubsystemInfoFormatDocument {
                path: self.root_path().join(&ci_relative),
                bytes: bytes.clone(),
            });
        }
        let guard = crate::infrastructure::format_guard::evaluate_prepared_subsystem_validation_format_guard(&documents);
        let mut source_error = None;
        let result = crate::infrastructure::family_codecs::metadata_read_codec()
            .check_subsystem_content(
                text,
                relative,
                false,
                &mut |path| {
                    let result = (|| {
                        checkpoint()?;
                        match self.root.open_relative_regular_nofollow(path) {
                            Ok(_) => Ok(true),
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
                            Err(error) => Err(ViewError::detailed(
                                RefusalDetail::SourceUnreadable,
                                error.to_string(),
                            )),
                        }
                    })();
                    result.map_err(|error| {
                        let message = error.to_string();
                        source_error = Some(error);
                        message
                    })
                },
                &mut |_| {
                    ci.as_ref()
                        .map(|bytes| {
                            String::from_utf8(bytes.clone()).map_err(|error| error.to_string())
                        })
                        .transpose()
                },
            );
        if let Some(error) = source_error {
            return Err(error);
        }
        checkpoint()?;
        Ok(
            crate::infrastructure::native_operations::v13_analysis::subsystem_validation_outcome(
                guard,
                subsystem_validation_outcome(result),
            ),
        )
    }

    pub(crate) fn nested_subsystem_export_path(
        &self,
        target: &MetadataAddress,
        nested: &[String],
        resource: Option<&str>,
    ) -> Result<String, ViewError> {
        let mut relative = self.metadata_descriptor_relative(target)?;
        for name in nested {
            relative = nested_subsystem_descriptor(&relative, name);
        }
        path_text(match resource {
            Some(resource) => nested_subsystem_resource(&relative, resource),
            None => relative,
        })
    }

    /// The registered subsystem topology of this source set, retained by the
    /// workspace across calls while its files keep their stamps.
    pub(crate) fn subsystem_topology(
        &self,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Arc<unica_metadata::subsystem_topology::SubsystemTopology>, ViewError> {
        self.artifacts.subsystem_topology(&self.root, checkpoint)
    }
}

/// `Subsystems/A.xml` → `Subsystems/A/Subsystems/B.xml`.
fn nested_subsystem_descriptor(parent: &Path, name: &str) -> PathBuf {
    parent
        .with_extension("")
        .join("Subsystems")
        .join(format!("{name}.xml"))
}

/// `Subsystems/A/Subsystems/B.xml` → `Subsystems/A/Subsystems/B/Ext/<resource>`.
fn nested_subsystem_resource(descriptor: &Path, resource: &str) -> PathBuf {
    descriptor.with_extension("").join("Ext").join(resource)
}
