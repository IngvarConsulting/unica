//! Shared section encoding; the file backend uses one framed aggregate.
use crate::invocation_store::{MAX_CANONICAL_RESULT_BYTES, MAX_TASK_RECORD_ENVELOPE_BYTES};
use sha2::{Digest, Sha256};
use std::io::{self, Read, Write};
use std::sync::Arc;
use unica_execution_contract::storage::{codec as raw, StoredRecord};

const MAGIC: &[u8; 8] = b"UNICAEX1";
pub(crate) const HEADER_BYTES: usize = 128;
// Each independently encoded role may contain a whole result plus its closed
// wrapper. This is deliberately not a one-result aggregate allocation bound.
const BODY_BOUND: usize = MAX_CANONICAL_RESULT_BYTES + MAX_TASK_RECORD_ENVELOPE_BYTES;
const METADATA_BOUND: usize = raw::MAX_METADATA_BYTES;

#[derive(Debug)]
pub(crate) enum CodecError {
    Io(io::Error),
    Invalid,
    TooLarge,
}
impl From<io::Error> for CodecError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
impl From<raw::DecodeError> for CodecError {
    fn from(error: raw::DecodeError) -> Self {
        match error {
            raw::DecodeError::InvalidRecord => Self::Invalid,
            raw::DecodeError::TooLarge => Self::TooLarge,
        }
    }
}

pub(crate) struct EncodedFrame<'a> {
    header: [u8; HEADER_BYTES],
    metadata: Vec<u8>,
    producer: &'a [u8],
    primary_override: &'a [u8],
}
impl<'a> EncodedFrame<'a> {
    pub(crate) fn prepare(
        stored: &'a StoredRecord,
        namespace: [u8; 16],
    ) -> Result<Self, CodecError> {
        raw::validate_result_sizes(stored, MAX_CANONICAL_RESULT_BYTES)?;
        let metadata = stored.record().encode();
        let producer = stored.producer().map_or(&[][..], |o| o.bytes());
        let primary_override = stored.primary_override().map_or(&[][..], |o| o.bytes());
        if metadata.len() > METADATA_BOUND
            || producer.len() > BODY_BOUND
            || primary_override.len() > BODY_BOUND
        {
            return Err(CodecError::TooLarge);
        }
        let mut header = [0; HEADER_BYTES];
        header[..8].copy_from_slice(MAGIC);
        header[8..10].copy_from_slice(&1_u16.to_le_bytes());
        header[12..16].copy_from_slice(&(HEADER_BYTES as u32).to_le_bytes());
        header[16..32].copy_from_slice(&namespace);
        header[32..64].copy_from_slice(&stored.record().key().digest());
        let total = HEADER_BYTES + metadata.len() + producer.len() + primary_override.len();
        for (offset, length) in [
            (64, metadata.len()),
            (72, producer.len()),
            (80, primary_override.len()),
            (88, total),
        ] {
            header[offset..offset + 8].copy_from_slice(&(length as u64).to_le_bytes());
        }
        let mut hash = Sha256::new();
        hash.update(&header[..96]);
        hash.update(&metadata);
        hash.update(producer);
        hash.update(primary_override);
        header[96..].copy_from_slice(&hash.finalize());
        Ok(Self {
            header,
            metadata,
            producer,
            primary_override,
        })
    }
    pub(crate) fn write_to(&self, writer: &mut impl Write) -> io::Result<()> {
        writer.write_all(&self.header)?;
        writer.write_all(&self.metadata)?;
        writer.write_all(self.producer)?;
        writer.write_all(self.primary_override)
    }
}

fn section(reader: &mut impl Read, length: usize) -> Result<Vec<u8>, CodecError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| CodecError::TooLarge)?;
    bytes.resize(length, 0);
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

pub(crate) fn read_frame(
    reader: &mut impl Read,
    physical_length: u64,
    namespace: [u8; 16],
) -> Result<StoredRecord, CodecError> {
    let mut header = [0; HEADER_BYTES];
    reader.read_exact(&mut header)?;
    if &header[..8] != MAGIC
        || header[8..10] != 1_u16.to_le_bytes()
        || header[10..12] != [0, 0]
        || header[12..16] != (HEADER_BYTES as u32).to_le_bytes()
        || header[16..32] != namespace
    {
        return Err(CodecError::Invalid);
    }
    let length =
        |offset| u64::from_le_bytes(header[offset..offset + 8].try_into().expect("fixed header"));
    let lengths = [length(64), length(72), length(80)];
    let total = lengths
        .iter()
        .try_fold(HEADER_BYTES as u64, |n, next| n.checked_add(*next))
        .ok_or(CodecError::Invalid)?;
    if total != physical_length
        || total != length(88)
        || lengths[0] > METADATA_BOUND as u64
        || lengths[1..].iter().any(|n| *n > BODY_BOUND as u64)
    {
        return Err(CodecError::Invalid);
    }
    let metadata = section(reader, lengths[0] as usize)?;
    let producer = section(reader, lengths[1] as usize)?;
    let primary_override = section(reader, lengths[2] as usize)?;
    let mut hash = Sha256::new();
    hash.update(&header[..96]);
    hash.update(&metadata);
    hash.update(&producer);
    hash.update(&primary_override);
    if hash.finalize().as_slice() != &header[96..] {
        return Err(CodecError::Invalid);
    }
    let mut trailing = [0];
    if reader.read(&mut trailing)? != 0 {
        return Err(CodecError::Invalid);
    }
    let body = |bytes: Vec<u8>| (!bytes.is_empty()).then(|| Arc::<[u8]>::from(bytes));
    let stored = raw::decode_parts(
        &metadata,
        body(producer),
        body(primary_override),
        MAX_CANONICAL_RESULT_BYTES,
    )?;
    if stored.record().key().digest() != header[32..64] {
        return Err(CodecError::Invalid);
    }
    Ok(stored)
}
