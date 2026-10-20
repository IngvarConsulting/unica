use super::*;

impl DirectoryRemoval {
    pub(in crate::infrastructure::native_operations) fn capture_owned(
        root: Arc<RetainedDirectoryCapability>,
        payload: &Path,
        descriptor: &Path,
        collection: &Path,
        files: &BTreeMap<FileIdentity, RemovedFileImage>,
        deadline: ProviderDeadline,
        cancellation: CancellationToken,
    ) -> Result<Vec<Self>, ApplyStagingError> {
        if payload.parent() != Some(collection) || descriptor.parent() != Some(collection) {
            return Err(ApplyStagingError::new(
                ApplyStagingErrorKind::Invariant,
                "resource directory and descriptor must share their selected collection",
            ));
        }
        // The payload has exactly the same depth/entry budget as enumerate_tree.
        // Proving the optional parent never spends that budget on other owners.
        let plans = Self::capture(
            Arc::clone(&root),
            payload,
            descriptor,
            files,
            deadline,
            cancellation.clone(),
        )?;
        let Some((directory, route)) = retain_route(&root, collection, deadline, &cancellation)?
        else {
            return Ok(plans);
        };
        let mut names = Vec::new();
        let mut foreign = false;
        let mut stopped = None;
        let observed = directory.visit_immediate_names(|name| -> std::io::Result<()> {
            checkpoint(deadline, &cancellation).map_err(|error| {
                stopped = Some(error);
                std::io::Error::other("collection observation stopped")
            })?;
            let allowed = [payload, descriptor]
                .iter()
                .try_fold(false, |found, path| {
                    directory
                        .child_names_equivalent(&name, path.file_name().expect("non-root resource"))
                        .map(|same| found || same)
                })?;
            if !allowed || names.len() == 2 {
                foreign = true;
                return Err(std::io::Error::new(
                    ErrorKind::Interrupted,
                    "collection has another owner",
                ));
            }
            names.push(name);
            Ok(())
        });
        if foreign {
            return Ok(plans);
        }
        observed.map_err(|error| stopped.unwrap_or_else(|| io_error(error)))?;
        let mut children = BTreeMap::new();
        for name in names {
            match directory
                .retain_immediate_child_nofollow(&name)
                .map_err(io_error)?
            {
                RetainedChildCapability::Directory(child) => {
                    let Some(plan) = plans.iter().find(|plan| {
                        plan.relative == payload && plan.image.identity == child.identity()
                    }) else {
                        // A protected descendant keeps its ancestors in place.
                        return Ok(plans);
                    };
                    children.insert(name, ChildImage::Directory(plan.image.clone()));
                }
                RetainedChildCapability::RegularFile(file) => {
                    let Some(image) = files.get(&file.identity()) else {
                        return Ok(plans);
                    };
                    file_matches(&file, file.identity(), image)?;
                    children.insert(name, ChildImage::File(file.identity(), image.clone()));
                }
                _ => return Err(wrong_kind()),
            }
        }
        directory.validate_named_identity().map_err(io_error)?;
        let image = DirectoryImage {
            identity: directory.identity(),
            permissions: portable_permissions(
                &directory
                    .try_clone_directory()
                    .and_then(|file| file.metadata())
                    .map_err(io_error)?,
            ),
            children,
        };
        Ok(vec![Self {
            root,
            relative: collection.to_path_buf(),
            owner_descriptor: descriptor.to_path_buf(),
            route,
            image,
            deadline,
            cancellation,
        }])
    }
}
