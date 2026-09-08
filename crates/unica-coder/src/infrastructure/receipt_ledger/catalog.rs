//! Каталог квитанций: проверка и фиксация вставки, удаления и замены
//! записей, а также признаки истёкших записей.

use super::*;

pub(super) fn validate_catalog_insert(
    catalog: &ReceiptCatalog,
    entry: &CatalogEntry,
    recovering: bool,
) -> Result<(), ReceiptLedgerError> {
    if !entry.is_tombstone() && catalog.live_count() >= MAX_LIVE_RECEIPTS {
        return Err(if recovering {
            ReceiptLedgerError::Corrupt("receipt catalog exceeds the live-record limit")
        } else {
            ReceiptLedgerError::CapacityExceeded
        });
    }
    if entry.is_tombstone() && catalog.tombstone_count() >= MAX_ACKNOWLEDGED_TOMBSTONES {
        return Err(if recovering {
            ReceiptLedgerError::Corrupt("receipt catalog exceeds the tombstone-record limit")
        } else {
            ReceiptLedgerError::TombstoneCapacityExceeded
        });
    }
    if catalog.records.contains_key(&entry.record.key_digest) {
        return Err(if recovering {
            ReceiptLedgerError::Corrupt("receipt catalog contains a duplicate key digest")
        } else {
            ReceiptLedgerError::ReceiptDigestCollision
        });
    }
    if catalog
        .invocation_index
        .contains_key(&entry.record.key.invocation_id())
    {
        return Err(if recovering {
            ReceiptLedgerError::Corrupt("receipt catalog contains a duplicate invocation id")
        } else {
            ReceiptLedgerError::InvocationIdentityMismatch
        });
    }
    if catalog
        .reserved_task_index
        .contains_key(&entry.record.key.reserved_task_id())
    {
        return Err(if recovering {
            ReceiptLedgerError::Corrupt("receipt catalog contains a duplicate reserved task id")
        } else {
            ReceiptLedgerError::ReservedTaskIdentityMismatch
        });
    }
    if !entry.is_tombstone()
        && catalog
            .records
            .values()
            .filter(|stored| !stored.is_tombstone())
            .any(|stored| stored.record.mutation_sequence == entry.record.mutation_sequence)
    {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt catalog contains a duplicate mutation sequence",
        ));
    }
    let next_actual_bytes = catalog
        .actual_bytes
        .checked_add(entry.live_actual_bytes())
        .ok_or(ReceiptLedgerError::CapacityExceeded)?;
    let next_reserved_bytes = catalog
        .reserved_result_bytes
        .checked_add(entry.reserved_result_bytes())
        .ok_or(ReceiptLedgerError::CapacityExceeded)?;
    if next_actual_bytes
        .checked_add(next_reserved_bytes)
        .filter(|total| *total <= MAX_LIVE_RECEIPT_BYTES)
        .is_none()
    {
        return Err(if recovering {
            ReceiptLedgerError::Corrupt("receipt catalog exceeds the byte entitlement limit")
        } else {
            ReceiptLedgerError::CapacityExceeded
        });
    }
    let next_tombstone_bytes = catalog
        .tombstone_bytes
        .checked_add(entry.tombstone_bytes())
        .ok_or(ReceiptLedgerError::TombstoneCapacityExceeded)?;
    if next_tombstone_bytes > MAX_ACKNOWLEDGED_TOMBSTONE_POOL_BYTES {
        return Err(if recovering {
            ReceiptLedgerError::Corrupt("receipt catalog exceeds the tombstone byte limit")
        } else {
            ReceiptLedgerError::TombstoneCapacityExceeded
        });
    }
    Ok(())
}

pub(super) fn validate_catalog_insert_batch(
    catalog: &ReceiptCatalog,
    entries: &[CatalogEntry],
) -> Result<(), ReceiptLedgerError> {
    let mut live_count = catalog.live_count();
    let mut tombstone_count = catalog.tombstone_count();
    let mut actual_bytes = catalog.actual_bytes;
    let mut reserved_result_bytes = catalog.reserved_result_bytes;
    let mut tombstone_bytes = catalog.tombstone_bytes;
    let mut mutation_sequences = HashSet::with_capacity(entries.len());
    for entry in entries {
        if catalog.records.contains_key(&entry.record.key_digest) {
            return Err(ReceiptLedgerError::ReceiptDigestCollision);
        }
        if catalog
            .invocation_index
            .contains_key(&entry.record.key.invocation_id())
        {
            return Err(ReceiptLedgerError::InvocationIdentityMismatch);
        }
        if catalog
            .reserved_task_index
            .contains_key(&entry.record.key.reserved_task_id())
        {
            return Err(ReceiptLedgerError::ReservedTaskIdentityMismatch);
        }
        if !mutation_sequences.insert(entry.record.mutation_sequence) {
            return Err(ReceiptLedgerError::Corrupt(
                "receipt insertion batch reuses a mutation sequence",
            ));
        }
        if entry.is_tombstone() {
            tombstone_count = tombstone_count
                .checked_add(1)
                .ok_or(ReceiptLedgerError::TombstoneCapacityExceeded)?;
        } else {
            live_count = live_count
                .checked_add(1)
                .ok_or(ReceiptLedgerError::CapacityExceeded)?;
        }
        actual_bytes = actual_bytes
            .checked_add(entry.live_actual_bytes())
            .ok_or(ReceiptLedgerError::CapacityExceeded)?;
        reserved_result_bytes = reserved_result_bytes
            .checked_add(entry.reserved_result_bytes())
            .ok_or(ReceiptLedgerError::CapacityExceeded)?;
        tombstone_bytes = tombstone_bytes
            .checked_add(entry.tombstone_bytes())
            .ok_or(ReceiptLedgerError::TombstoneCapacityExceeded)?;
    }
    if live_count > MAX_LIVE_RECEIPTS
        || actual_bytes
            .checked_add(reserved_result_bytes)
            .is_none_or(|bytes| bytes > MAX_LIVE_RECEIPT_BYTES)
    {
        return Err(ReceiptLedgerError::CapacityExceeded);
    }
    if tombstone_count > MAX_ACKNOWLEDGED_TOMBSTONES
        || tombstone_bytes > MAX_ACKNOWLEDGED_TOMBSTONE_POOL_BYTES
    {
        return Err(ReceiptLedgerError::TombstoneCapacityExceeded);
    }
    Ok(())
}

pub(super) fn commit_catalog_insert(catalog: &mut ReceiptCatalog, entry: CatalogEntry) {
    catalog.actual_bytes += entry.live_actual_bytes();
    catalog.reserved_result_bytes += entry.reserved_result_bytes();
    catalog.tombstone_bytes += entry.tombstone_bytes();
    if entry.is_tombstone() {
        catalog.tombstone_records += 1;
    } else {
        catalog.live_records += 1;
    }
    catalog.invocation_index.insert(
        entry.record.key.invocation_id(),
        entry.record.key_digest.clone(),
    );
    catalog.reserved_task_index.insert(
        entry.record.key.reserved_task_id(),
        entry.record.key_digest.clone(),
    );
    catalog
        .records
        .insert(entry.record.key_digest.clone(), entry);
}

pub(super) fn insert_catalog_entry(
    catalog: &mut ReceiptCatalog,
    entry: CatalogEntry,
    recovering: bool,
) -> Result<(), ReceiptLedgerError> {
    validate_catalog_insert(catalog, &entry, recovering)?;
    commit_catalog_insert(catalog, entry);
    Ok(())
}

pub(super) fn catalog_entry_is_expired_identity_reclaimable(
    catalog: &ReceiptCatalog,
    digest: &ReceiptKeyDigest,
    observed_at_epoch_ms: u64,
) -> Result<bool, ReceiptLedgerError> {
    let entry = catalog
        .records
        .get(digest)
        .ok_or(ReceiptLedgerError::Corrupt(
            "receipt identity index points outside the catalog",
        ))?;
    Ok(
        entry_is_expired_cancel_reserved(entry, observed_at_epoch_ms)
            || entry_is_expired_tombstone(entry, observed_at_epoch_ms)
            || entry_is_expired_direct_terminal(entry, observed_at_epoch_ms),
    )
}

pub(super) fn entry_is_expired_cancel_reserved(
    entry: &CatalogEntry,
    observed_at_epoch_ms: u64,
) -> bool {
    matches!(
        &entry.record.lifecycle,
        StoredActiveLifecycleV1::CancelReserved {
            expires_at_epoch_ms,
            ..
        } if observed_at_epoch_ms >= *expires_at_epoch_ms
    )
}

pub(super) fn entry_is_expired_tombstone(entry: &CatalogEntry, observed_at_epoch_ms: u64) -> bool {
    matches!(
        &entry.record.lifecycle,
        StoredActiveLifecycleV1::AcknowledgedTombstone {
            acknowledged_at_epoch_ms,
            ..
        } if acknowledged_at_epoch_ms
            .checked_add(ACKNOWLEDGED_TOMBSTONE_TTL_MS)
            .is_some_and(|expires_at_epoch_ms| observed_at_epoch_ms >= expires_at_epoch_ms)
    )
}

pub(super) fn entry_is_expired_direct_terminal(
    entry: &CatalogEntry,
    observed_at_epoch_ms: u64,
) -> bool {
    matches!(
        &entry.record.lifecycle,
        StoredActiveLifecycleV1::DirectTerminalUnacked {
            terminal_epoch_ms,
            ..
        } if terminal_epoch_ms
            .checked_add(DIRECT_TERMINAL_RETENTION_MS)
            .is_some_and(|expires_at_epoch_ms| observed_at_epoch_ms >= expires_at_epoch_ms)
    )
}

pub(super) fn entry_is_expired_task_receipt_terminal(
    entry: &CatalogEntry,
    observed_at_epoch_ms: u64,
) -> bool {
    matches!(
        &entry.record.lifecycle,
        StoredActiveLifecycleV1::TaskTerminalReceiptBacked {
            terminal_epoch_ms,
            ttl_ms,
            ..
        } if terminal_epoch_ms
            .checked_add(*ttl_ms)
            .is_some_and(|expires_at_epoch_ms| observed_at_epoch_ms >= expires_at_epoch_ms)
    )
}

pub(super) fn ack_tombstone_has_capacity(
    catalog: &ReceiptCatalog,
    replacement: &CatalogEntry,
) -> bool {
    catalog
        .tombstone_count()
        .checked_add(usize::from(replacement.is_tombstone()))
        .is_some_and(|count| count <= MAX_ACKNOWLEDGED_TOMBSTONES)
        && catalog
            .tombstone_bytes
            .checked_add(replacement.tombstone_bytes())
            .is_some_and(|bytes| bytes <= MAX_ACKNOWLEDGED_TOMBSTONE_POOL_BYTES)
}

pub(super) fn validate_catalog_remove(
    catalog: &ReceiptCatalog,
    expected: &CatalogEntry,
) -> Result<(), ReceiptLedgerError> {
    let digest = &expected.record.key_digest;
    if catalog.records.get(digest) != Some(expected) {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt catalog changed before exact removal",
        ));
    }
    if catalog
        .invocation_index
        .get(&expected.record.key.invocation_id())
        != Some(digest)
    {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt invocation index changed before exact removal",
        ));
    }
    if catalog
        .reserved_task_index
        .get(&expected.record.key.reserved_task_id())
        != Some(digest)
    {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt task index changed before exact removal",
        ));
    }
    catalog
        .actual_bytes
        .checked_sub(expected.live_actual_bytes())
        .ok_or(ReceiptLedgerError::Corrupt(
            "receipt catalog actual-byte accounting underflowed",
        ))?;
    catalog
        .reserved_result_bytes
        .checked_sub(expected.reserved_result_bytes())
        .ok_or(ReceiptLedgerError::Corrupt(
            "receipt catalog reserved-byte accounting underflowed",
        ))?;
    catalog
        .tombstone_bytes
        .checked_sub(expected.tombstone_bytes())
        .ok_or(ReceiptLedgerError::Corrupt(
            "receipt catalog tombstone-byte accounting underflowed",
        ))?;
    Ok(())
}

pub(super) fn commit_catalog_remove(catalog: &mut ReceiptCatalog, expected: &CatalogEntry) {
    let digest = &expected.record.key_digest;
    catalog.actual_bytes -= expected.live_actual_bytes();
    catalog.reserved_result_bytes -= expected.reserved_result_bytes();
    catalog.tombstone_bytes -= expected.tombstone_bytes();
    if expected.is_tombstone() {
        catalog.tombstone_records -= 1;
    } else {
        catalog.live_records -= 1;
    }
    catalog
        .invocation_index
        .remove(&expected.record.key.invocation_id());
    catalog
        .reserved_task_index
        .remove(&expected.record.key.reserved_task_id());
    catalog.records.remove(digest);
}

pub(super) fn validate_catalog_replace(
    catalog: &ReceiptCatalog,
    expected: &CatalogEntry,
    replacement: &CatalogEntry,
) -> Result<(), ReceiptLedgerError> {
    if catalog.records.get(&expected.record.key_digest) != Some(expected) {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt catalog changed before exact replacement",
        ));
    }
    if replacement.record.key_digest != expected.record.key_digest
        || replacement.record.key != expected.record.key
    {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt replacement changed its exact identity",
        ));
    }
    if !replacement.is_tombstone()
        && catalog.records.iter().any(|(digest, stored)| {
            digest != &expected.record.key_digest
                && !stored.is_tombstone()
                && stored.record.mutation_sequence == replacement.record.mutation_sequence
        })
    {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt replacement reuses a mutation sequence",
        ));
    }
    let next_actual_bytes = catalog
        .actual_bytes
        .checked_sub(expected.live_actual_bytes())
        .and_then(|bytes| bytes.checked_add(replacement.live_actual_bytes()))
        .ok_or(ReceiptLedgerError::Corrupt(
            "receipt catalog actual-byte accounting underflowed",
        ))?;
    let next_reserved_bytes = catalog
        .reserved_result_bytes
        .checked_sub(expected.reserved_result_bytes())
        .and_then(|bytes| bytes.checked_add(replacement.reserved_result_bytes()))
        .ok_or(ReceiptLedgerError::Corrupt(
            "receipt catalog reserved-byte accounting underflowed",
        ))?;
    if next_actual_bytes
        .checked_add(next_reserved_bytes)
        .filter(|total| *total <= MAX_LIVE_RECEIPT_BYTES)
        .is_none()
    {
        return Err(ReceiptLedgerError::CapacityExceeded);
    }
    let next_tombstone_count = catalog
        .tombstone_count()
        .checked_sub(usize::from(expected.is_tombstone()))
        .and_then(|count| count.checked_add(usize::from(replacement.is_tombstone())))
        .ok_or(ReceiptLedgerError::Corrupt(
            "receipt catalog tombstone count underflowed",
        ))?;
    if next_tombstone_count > MAX_ACKNOWLEDGED_TOMBSTONES {
        return Err(ReceiptLedgerError::TombstoneCapacityExceeded);
    }
    let next_tombstone_bytes = catalog
        .tombstone_bytes
        .checked_sub(expected.tombstone_bytes())
        .and_then(|bytes| bytes.checked_add(replacement.tombstone_bytes()))
        .ok_or(ReceiptLedgerError::Corrupt(
            "receipt catalog tombstone-byte accounting underflowed",
        ))?;
    if next_tombstone_bytes > MAX_ACKNOWLEDGED_TOMBSTONE_POOL_BYTES {
        return Err(ReceiptLedgerError::TombstoneCapacityExceeded);
    }
    Ok(())
}

pub(super) fn validate_catalog_replace_batch(
    catalog: &ReceiptCatalog,
    replacements: &[(CatalogEntry, CatalogEntry)],
) -> Result<(), ReceiptLedgerError> {
    let mut live_count = catalog.live_count();
    let mut tombstone_count = catalog.tombstone_count();
    let mut actual_bytes = catalog.actual_bytes;
    let mut reserved_result_bytes = catalog.reserved_result_bytes;
    let mut tombstone_bytes = catalog.tombstone_bytes;
    let mut digests = HashSet::with_capacity(replacements.len());
    let mut mutation_sequences = HashSet::with_capacity(replacements.len());
    for (expected, replacement) in replacements {
        if catalog.records.get(&expected.record.key_digest) != Some(expected) {
            return Err(ReceiptLedgerError::Corrupt(
                "receipt catalog changed before exact batch replacement",
            ));
        }
        if replacement.record.key_digest != expected.record.key_digest
            || replacement.record.key != expected.record.key
            || !digests.insert(expected.record.key_digest.clone())
        {
            return Err(ReceiptLedgerError::Corrupt(
                "receipt batch replacement changed or duplicated its exact identity",
            ));
        }
        if !mutation_sequences.insert(replacement.record.mutation_sequence) {
            return Err(ReceiptLedgerError::Corrupt(
                "receipt replacement batch reuses a mutation sequence",
            ));
        }
        live_count = live_count
            .checked_sub(usize::from(!expected.is_tombstone()))
            .and_then(|count| count.checked_add(usize::from(!replacement.is_tombstone())))
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt batch live count underflowed",
            ))?;
        tombstone_count = tombstone_count
            .checked_sub(usize::from(expected.is_tombstone()))
            .and_then(|count| count.checked_add(usize::from(replacement.is_tombstone())))
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt batch tombstone count underflowed",
            ))?;
        actual_bytes = actual_bytes
            .checked_sub(expected.live_actual_bytes())
            .and_then(|bytes| bytes.checked_add(replacement.live_actual_bytes()))
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt batch actual-byte accounting underflowed",
            ))?;
        reserved_result_bytes = reserved_result_bytes
            .checked_sub(expected.reserved_result_bytes())
            .and_then(|bytes| bytes.checked_add(replacement.reserved_result_bytes()))
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt batch reserved-byte accounting underflowed",
            ))?;
        tombstone_bytes = tombstone_bytes
            .checked_sub(expected.tombstone_bytes())
            .and_then(|bytes| bytes.checked_add(replacement.tombstone_bytes()))
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt batch tombstone-byte accounting underflowed",
            ))?;
    }
    if live_count > MAX_LIVE_RECEIPTS
        || actual_bytes
            .checked_add(reserved_result_bytes)
            .is_none_or(|bytes| bytes > MAX_LIVE_RECEIPT_BYTES)
    {
        return Err(ReceiptLedgerError::CapacityExceeded);
    }
    if tombstone_count > MAX_ACKNOWLEDGED_TOMBSTONES
        || tombstone_bytes > MAX_ACKNOWLEDGED_TOMBSTONE_POOL_BYTES
    {
        return Err(ReceiptLedgerError::TombstoneCapacityExceeded);
    }
    Ok(())
}

pub(super) fn commit_catalog_replace(catalog: &mut ReceiptCatalog, replacement: CatalogEntry) {
    let digest = replacement.record.key_digest.clone();
    let replacement_live_actual_bytes = replacement.live_actual_bytes();
    let replacement_reserved_result_bytes = replacement.reserved_result_bytes();
    let replacement_tombstone_bytes = replacement.tombstone_bytes();
    let previous = catalog
        .records
        .insert(digest, replacement)
        .expect("validated receipt replacement has an existing catalog entry");
    catalog.actual_bytes =
        catalog.actual_bytes - previous.live_actual_bytes() + replacement_live_actual_bytes;
    catalog.reserved_result_bytes = catalog.reserved_result_bytes
        - previous.reserved_result_bytes()
        + replacement_reserved_result_bytes;
    catalog.tombstone_bytes =
        catalog.tombstone_bytes - previous.tombstone_bytes() + replacement_tombstone_bytes;
    match (previous.is_tombstone(), replacement_tombstone_bytes > 0) {
        (false, true) => {
            catalog.live_records -= 1;
            catalog.tombstone_records += 1;
        }
        (true, false) => {
            catalog.tombstone_records -= 1;
            catalog.live_records += 1;
        }
        (false, false) | (true, true) => {}
    }
}

pub(super) fn latch_catalog_error<T>(
    catalog: &mut ReceiptCatalog,
    error: ReceiptLedgerError,
) -> Result<T, ReceiptLedgerError> {
    catalog.unavailable = true;
    Err(error)
}

pub(super) fn latch_catalog_result<T>(
    catalog: &mut ReceiptCatalog,
    result: Result<T, ReceiptLedgerError>,
) -> Result<T, ReceiptLedgerError> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => latch_catalog_error(catalog, error),
    }
}

pub(super) fn cleanup_staged_file(
    parent: &File,
    name: &OsStr,
    identity: FileIdentity,
    file: &File,
) -> Result<(), ReceiptLedgerError> {
    remove_identity_bound_regular_child(parent, name, identity, file)
        .map_err(|error| storage_error("clean up receipt staging file", error))
}
