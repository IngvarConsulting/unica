//! Bounded registration sorting for the root Configuration view. The root
//! publishes counts, while the owner proof still needs exact set semantics.

use crate::application::v13::view::ViewError;
use crate::domain::address::NodeKind;
use crate::domain::refusal::RefusalDetail;
use crate::infrastructure::native_operations::common::SupportState;
use crate::infrastructure::platform::filesystem::{FileStamp, RetainedDirectoryCapability};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use xml::name::OwnedName;
use xml::reader::{ParserConfig, XmlEvent};

const REGISTRATION_RUN_BYTES: usize = 1024 * 1024;
const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

type Registration = (String, String);

fn scratch_error(error: io::Error) -> ViewError {
    ViewError::detailed(
        RefusalDetail::BackendBroken,
        format!("temporary Configuration registration sort failed: {error}"),
    )
}

fn scratch_incomplete() -> ViewError {
    ViewError::detailed(
        RefusalDetail::BackendBroken,
        "temporary Configuration registration index is incomplete",
    )
}

fn source_io_error(error: io::Error) -> ViewError {
    ViewError::detailed(
        RefusalDetail::SourceUnreadable,
        format!("Configuration.xml read failed: {error}"),
    )
}

/// Binary-counter external sort: at most one anonymous run per level and
/// hence O(log N) open scratch handles. A run is owned by its File, with no
/// persistent path on Unix and delete-on-close semantics on Windows.
pub(super) struct RegistrationSorter {
    batch: Vec<Registration>,
    batch_bytes: usize,
    levels: Vec<Option<File>>,
}

impl RegistrationSorter {
    pub(super) fn new() -> Self {
        Self {
            batch: Vec::new(),
            batch_bytes: 0,
            levels: Vec::new(),
        }
    }

    pub(super) fn push(
        &mut self,
        kind: String,
        name: String,
        checkpoint: &dyn Fn() -> Result<(), ViewError>,
    ) -> Result<(), ViewError> {
        checkpoint()?;
        let charge = kind
            .len()
            .checked_add(name.len())
            .and_then(|length| length.checked_add(8))
            .ok_or_else(|| {
                ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "registration size overflow",
                )
            })?;
        if !self.batch.is_empty()
            && self.batch_bytes.saturating_add(charge) > REGISTRATION_RUN_BYTES
        {
            self.flush(checkpoint)?;
        }
        self.batch_bytes = self.batch_bytes.checked_add(charge).ok_or_else(|| {
            ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "registration batch size overflow",
            )
        })?;
        self.batch.push((kind, name));
        Ok(())
    }

    fn flush(&mut self, checkpoint: &dyn Fn() -> Result<(), ViewError>) -> Result<(), ViewError> {
        if self.batch.is_empty() {
            return Ok(());
        }
        checkpoint()?;
        self.batch.sort_unstable();
        self.batch.dedup();
        let file = tempfile::tempfile().map_err(scratch_error)?;
        let mut writer = BufWriter::new(file);
        for item in &self.batch {
            checkpoint()?;
            write_registration(&mut writer, item).map_err(scratch_error)?;
        }
        let mut file = writer
            .into_inner()
            .map_err(|error| scratch_error(error.into_error()))?;
        file.seek(SeekFrom::Start(0)).map_err(scratch_error)?;
        self.batch.clear();
        self.batch_bytes = 0;
        self.insert_run(file, checkpoint)
    }

    fn insert_run(
        &mut self,
        mut run: File,
        checkpoint: &dyn Fn() -> Result<(), ViewError>,
    ) -> Result<(), ViewError> {
        let mut level = 0;
        loop {
            checkpoint()?;
            if level == self.levels.len() {
                self.levels.push(Some(run));
                return Ok(());
            }
            match self.levels[level].take() {
                None => {
                    self.levels[level] = Some(run);
                    return Ok(());
                }
                Some(previous) => {
                    run = merge_runs(previous, run, checkpoint)?;
                    level += 1;
                }
            }
        }
    }

    pub(super) fn for_each_unique(
        mut self,
        checkpoint: &dyn Fn() -> Result<(), ViewError>,
        mut visit: impl FnMut(&str, &str) -> Result<(), ViewError>,
    ) -> Result<(), ViewError> {
        // The common small corpus never needs a scratch file.
        if self.levels.is_empty() {
            self.batch.sort_unstable();
            self.batch.dedup();
            for (kind, name) in self.batch {
                checkpoint()?;
                visit(&kind, &name)?;
            }
            return Ok(());
        }
        self.flush(checkpoint)?;
        let mut combined = None;
        for run in self.levels.into_iter().flatten() {
            checkpoint()?;
            combined = Some(match combined {
                Some(previous) => merge_runs(previous, run, checkpoint)?,
                None => run,
            });
        }
        let Some(run) = combined else {
            return Ok(());
        };
        let mut reader = BufReader::new(run);
        while let Some((kind, name)) = read_registration(&mut reader).map_err(scratch_error)? {
            checkpoint()?;
            visit(&kind, &name)?;
        }
        Ok(())
    }

    fn into_sorted_file(
        mut self,
        checkpoint: &dyn Fn() -> Result<(), ViewError>,
    ) -> Result<File, ViewError> {
        self.flush(checkpoint)?;
        let mut combined = None;
        for run in self.levels.into_iter().flatten() {
            checkpoint()?;
            combined = Some(match combined {
                Some(previous) => merge_runs(previous, run, checkpoint)?,
                None => run,
            });
        }
        match combined {
            Some(file) => Ok(file),
            None => tempfile::tempfile().map_err(scratch_error),
        }
    }
}

impl Default for RegistrationSorter {
    fn default() -> Self {
        Self::new()
    }
}

/// Revision-scoped, anonymous disk index for root registration membership.
/// The data run is sorted and unique; a second file stores fixed-width offsets
/// so a named read requires logarithmically many seeks without keeping every
/// registration key in process memory. Both files disappear on drop.
pub(crate) struct RegistrationIndex {
    inner: Mutex<RegistrationIndexFiles>,
    count: u64,
}

/// Разобранные артефакты корня одного набора исходников: индекс регистраций
/// `Configuration.xml`, полезная нагрузка корня и состояние поддержки. Актор
/// держит один такой кеш на набор между вызовами. Запись годна, пока штамп
/// её файлов (физическая идентичность, длина, время изменения) совпадает с
/// текущим; изменённый файл разбирается заново, а результат, чей файл
/// изменился во время разбора, не сохраняется. Обхода дерева нет.
#[derive(Default)]
pub(crate) struct RegistrationCache {
    registrations: Mutex<Option<(FileStamp, Arc<RegistrationIndex>)>>,
    configuration: Mutex<Option<(ConfigurationStamps, Arc<Value>)>>,
    support: Mutex<Option<RetainedSupportState>>,
    #[cfg(test)]
    registration_builds: std::sync::atomic::AtomicUsize,
}

/// A support state with the stamp of its marker (`None` when absent).
type RetainedSupportState = (Option<FileStamp>, Option<Arc<SupportState>>);

/// Stamps of every file the configuration-root payload is built from.
pub(crate) type ConfigurationStamps = [Option<FileStamp>; 4];

/// Files the configuration-root payload reads, in stamp order.
pub(crate) const CONFIGURATION_PAYLOAD_FILES: [&str; 4] = [
    "Configuration.xml",
    "Ext/ParentConfigurations.bin",
    "Ext/HomePageWorkArea.xml",
    "Ext/CommandInterface.xml",
];

fn cache_poisoned() -> ViewError {
    ViewError::detailed(
        RefusalDetail::CachePoisoned,
        "source artifact cache is poisoned",
    )
}

fn unreadable(error: io::Error) -> ViewError {
    ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
}

impl RegistrationCache {
    /// The registration index of `Configuration.xml` under `root`, parsed
    /// again only when the file's stamp changed.
    pub(crate) fn registrations(
        &self,
        root: &RetainedDirectoryCapability,
        checkpoint: &dyn Fn() -> Result<(), ViewError>,
    ) -> Result<Arc<RegistrationIndex>, ViewError> {
        checkpoint()?;
        let file = root
            .open_relative_regular_nofollow(std::path::Path::new("Configuration.xml"))
            .map_err(unreadable)?;
        let stamp = FileStamp::of(&file).map_err(unreadable)?;
        if let Some((cached, index)) = self
            .registrations
            .lock()
            .map_err(|_| cache_poisoned())?
            .as_ref()
        {
            if *cached == stamp {
                return Ok(Arc::clone(index));
            }
        }
        #[cfg(test)]
        self.registration_builds
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let observed = file.try_clone().map_err(unreadable)?;
        let built = Arc::new(read_configuration_registration_index(file, checkpoint)?);
        checkpoint()?;
        if FileStamp::of(&observed).map_err(unreadable)? == stamp {
            *self.registrations.lock().map_err(|_| cache_poisoned())? =
                Some((stamp, Arc::clone(&built)));
        }
        Ok(built)
    }

    /// The configuration-root payload, rebuilt only when one of its files
    /// changed. `build` reads the files itself; its result is retained only
    /// when no stamp moved while it ran.
    pub(crate) fn configuration_payload(
        &self,
        root: &RetainedDirectoryCapability,
        build: impl FnOnce() -> Result<Value, ViewError>,
    ) -> Result<Arc<Value>, ViewError> {
        let stamps = configuration_stamps(root)?;
        if let Some((cached, payload)) = self
            .configuration
            .lock()
            .map_err(|_| cache_poisoned())?
            .as_ref()
        {
            if *cached == stamps {
                return Ok(Arc::clone(payload));
            }
        }
        let built = Arc::new(build()?);
        if configuration_stamps(root)? == stamps {
            *self.configuration.lock().map_err(|_| cache_poisoned())? =
                Some((stamps, Arc::clone(&built)));
        }
        Ok(built)
    }

    /// The parsed support state of `Ext/ParentConfigurations.bin` (`None`
    /// when absent), parsed again only when the file's stamp changed.
    pub(crate) fn support_state(
        &self,
        root: &RetainedDirectoryCapability,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
        parse: impl FnOnce(
            File,
            &mut dyn FnMut() -> Result<(), ViewError>,
        ) -> Result<SupportState, ViewError>,
    ) -> Result<Option<Arc<SupportState>>, ViewError> {
        checkpoint()?;
        let file = match root
            .open_relative_regular_nofollow(std::path::Path::new("Ext/ParentConfigurations.bin"))
        {
            Ok(file) => Some(file),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(unreadable(error)),
        };
        let stamp = file
            .as_ref()
            .map(FileStamp::of)
            .transpose()
            .map_err(unreadable)?;
        if let Some((cached, state)) = self.support.lock().map_err(|_| cache_poisoned())?.as_ref() {
            if *cached == stamp {
                return Ok(state.clone());
            }
        }
        let (state, unchanged) = match file {
            None => (None, true),
            Some(file) => {
                let observed = file.try_clone().map_err(unreadable)?;
                let state = Arc::new(parse(file, checkpoint)?);
                let unchanged = Some(FileStamp::of(&observed).map_err(unreadable)?) == stamp;
                (Some(state), unchanged)
            }
        };
        checkpoint()?;
        if unchanged {
            *self.support.lock().map_err(|_| cache_poisoned())? = Some((stamp, state.clone()));
        }
        Ok(state)
    }

    #[cfg(test)]
    pub(crate) fn registration_builds_for_test(&self) -> usize {
        self.registration_builds
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

fn configuration_stamps(
    root: &RetainedDirectoryCapability,
) -> Result<ConfigurationStamps, ViewError> {
    let mut stamps = [None; 4];
    for (stamp, relative) in stamps.iter_mut().zip(CONFIGURATION_PAYLOAD_FILES) {
        *stamp = root
            .relative_file_stamp(std::path::Path::new(relative))
            .map_err(unreadable)?;
    }
    Ok(stamps)
}

struct RegistrationIndexFiles {
    data: File,
    offsets: File,
}

impl RegistrationIndex {
    fn from_sorter(
        sorter: RegistrationSorter,
        checkpoint: &dyn Fn() -> Result<(), ViewError>,
    ) -> Result<Self, ViewError> {
        let mut data = sorter.into_sorted_file(checkpoint)?;
        let mut offsets = BufWriter::new(tempfile::tempfile().map_err(scratch_error)?);
        let mut count = 0_u64;
        loop {
            checkpoint()?;
            let position = data.stream_position().map_err(scratch_error)?;
            if read_registration(&mut data)
                .map_err(scratch_error)?
                .is_none()
            {
                break;
            }
            offsets
                .write_all(&position.to_le_bytes())
                .map_err(scratch_error)?;
            count = count.checked_add(1).ok_or_else(|| {
                ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "registration count overflow",
                )
            })?;
        }
        let mut offsets = offsets
            .into_inner()
            .map_err(|error| scratch_error(error.into_error()))?;
        data.seek(SeekFrom::Start(0)).map_err(scratch_error)?;
        offsets.seek(SeekFrom::Start(0)).map_err(scratch_error)?;
        Ok(Self {
            inner: Mutex::new(RegistrationIndexFiles { data, offsets }),
            count,
        })
    }

    pub(crate) fn contains(
        &self,
        kind: &str,
        name: &str,
        checkpoint: &dyn Fn() -> Result<(), ViewError>,
    ) -> Result<bool, ViewError> {
        let mut files = self.inner.lock().map_err(|_| {
            ViewError::detailed(
                RefusalDetail::CachePoisoned,
                "registration index is poisoned",
            )
        })?;
        let mut low = 0_u64;
        let mut high = self.count;
        while low < high {
            checkpoint()?;
            let middle = low + (high - low) / 2;
            files
                .offsets
                .seek(SeekFrom::Start(middle.checked_mul(8).ok_or_else(|| {
                    ViewError::detailed(RefusalDetail::BackendBroken, "index offset overflow")
                })?))
                .map_err(scratch_error)?;
            let mut offset = [0_u8; 8];
            files
                .offsets
                .read_exact(&mut offset)
                .map_err(scratch_error)?;
            files
                .data
                .seek(SeekFrom::Start(u64::from_le_bytes(offset)))
                .map_err(scratch_error)?;
            let candidate = read_registration(&mut files.data)
                .map_err(scratch_error)?
                .ok_or_else(scratch_incomplete)?;
            match (candidate.0.as_str(), candidate.1.as_str()).cmp(&(kind, name)) {
                std::cmp::Ordering::Less => low = middle + 1,
                std::cmp::Ordering::Equal => return Ok(true),
                std::cmp::Ordering::Greater => high = middle,
            }
        }
        Ok(false)
    }

    /// Return the complete sorted, duplicate-free branch. The collection
    /// response itself holds these names, so only output-sized memory is used.
    pub(crate) fn names_for_kind(
        &self,
        kind: &str,
        checkpoint: &dyn Fn() -> Result<(), ViewError>,
    ) -> Result<Vec<String>, ViewError> {
        let mut files = self.inner.lock().map_err(|_| {
            ViewError::detailed(
                RefusalDetail::CachePoisoned,
                "registration index is poisoned",
            )
        })?;
        let mut low = 0_u64;
        let mut high = self.count;
        while low < high {
            checkpoint()?;
            let middle = low + (high - low) / 2;
            files
                .offsets
                .seek(SeekFrom::Start(middle.checked_mul(8).ok_or_else(|| {
                    ViewError::detailed(RefusalDetail::BackendBroken, "index offset overflow")
                })?))
                .map_err(scratch_error)?;
            let mut offset = [0_u8; 8];
            files
                .offsets
                .read_exact(&mut offset)
                .map_err(scratch_error)?;
            files
                .data
                .seek(SeekFrom::Start(u64::from_le_bytes(offset)))
                .map_err(scratch_error)?;
            let candidate = read_registration(&mut files.data)
                .map_err(scratch_error)?
                .ok_or_else(scratch_incomplete)?;
            if candidate.0.as_str() < kind {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        if low == self.count {
            return Ok(Vec::new());
        }
        files
            .offsets
            .seek(SeekFrom::Start(low.checked_mul(8).ok_or_else(|| {
                ViewError::detailed(RefusalDetail::BackendBroken, "index offset overflow")
            })?))
            .map_err(scratch_error)?;
        let mut offset = [0_u8; 8];
        files
            .offsets
            .read_exact(&mut offset)
            .map_err(scratch_error)?;
        files
            .data
            .seek(SeekFrom::Start(u64::from_le_bytes(offset)))
            .map_err(scratch_error)?;
        let mut names = Vec::new();
        while let Some((candidate_kind, name)) =
            read_registration(&mut files.data).map_err(scratch_error)?
        {
            checkpoint()?;
            if candidate_kind != kind {
                break;
            }
            names.push(name);
        }
        Ok(names)
    }
}

fn merge_runs(
    left: File,
    right: File,
    checkpoint: &dyn Fn() -> Result<(), ViewError>,
) -> Result<File, ViewError> {
    let mut left = BufReader::new(left);
    let mut right = BufReader::new(right);
    let mut first = read_registration(&mut left).map_err(scratch_error)?;
    let mut second = read_registration(&mut right).map_err(scratch_error)?;
    let mut writer = BufWriter::new(tempfile::tempfile().map_err(scratch_error)?);
    let mut last = None;
    while first.is_some() || second.is_some() {
        checkpoint()?;
        let next = match (&first, &second) {
            (Some(a), Some(b)) if a <= b => {
                let value = first.take().expect("left item is present");
                first = read_registration(&mut left).map_err(scratch_error)?;
                value
            }
            (Some(_), Some(_)) | (None, Some(_)) => {
                let value = second.take().expect("right item is present");
                second = read_registration(&mut right).map_err(scratch_error)?;
                value
            }
            (Some(_), None) => {
                let value = first.take().expect("left item is present");
                first = read_registration(&mut left).map_err(scratch_error)?;
                value
            }
            (None, None) => break,
        };
        if last.as_ref() != Some(&next) {
            write_registration(&mut writer, &next).map_err(scratch_error)?;
            last = Some(next);
        }
    }
    let mut merged = writer
        .into_inner()
        .map_err(|error| scratch_error(error.into_error()))?;
    merged.seek(SeekFrom::Start(0)).map_err(scratch_error)?;
    Ok(merged)
}

fn write_registration(writer: &mut impl Write, item: &Registration) -> io::Result<()> {
    for value in [&item.0, &item.1] {
        let length = u64::try_from(value.len()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "registration field is too long")
        })?;
        writer.write_all(&length.to_le_bytes())?;
        writer.write_all(value.as_bytes())?;
    }
    Ok(())
}

fn read_registration(reader: &mut impl Read) -> io::Result<Option<Registration>> {
    let mut first = [0_u8; 8];
    if reader.read(&mut first[..1])? == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut first[1..])?;
    let kind = read_field(reader, first)?;
    let mut second = [0_u8; 8];
    reader.read_exact(&mut second)?;
    let name = read_field(reader, second)?;
    Ok(Some((kind, name)))
}

fn read_field(reader: &mut impl Read, encoded_length: [u8; 8]) -> io::Result<String> {
    let length = usize::try_from(u64::from_le_bytes(encoded_length)).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidData, "registration field is too long")
    })?;
    let mut bytes = vec![0_u8; length];
    reader.read_exact(&mut bytes)?;
    String::from_utf8(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "scratch text is not UTF-8"))
}

pub(crate) struct StreamedConfigurationRoot {
    pub(crate) payload: Value,
    pub(crate) counts: BTreeMap<String, usize>,
}

/// Reads the whole root owner through the retained descriptor. The XML parser
/// validates the tail before any answer is published; root facts and a bounded
/// run of registration keys are held in memory. An individual XML token still
/// occupies parser memory until its event is emitted. The caller
/// confirms the admitted revision after projecting the returned root.
pub(super) fn read_configuration_root(
    file: File,
    support: Value,
    home_page: Value,
    interface: Value,
    checkpoint: &dyn Fn() -> Result<(), ViewError>,
) -> Result<StreamedConfigurationRoot, ViewError> {
    let state = parse_configuration_owner(file, checkpoint)?;
    let RootState {
        properties,
        version,
        total_objects,
        registrations,
        ..
    } = state;
    let mut counts = BTreeMap::<String, usize>::new();
    registrations.for_each_unique(checkpoint, |kind, _name| {
        let Ok(kind_value) = NodeKind::parse(kind) else {
            return Ok(());
        };
        if !kind_value.is_metadata_kind() {
            return Ok(());
        }
        checkpoint()?;
        let count = counts.entry(kind.to_string()).or_default();
        *count = count.checked_add(1).ok_or_else(|| {
            ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "Configuration branch count overflow",
            )
        })?;
        Ok(())
    })?;
    let property = |name: &str| properties.get(name).cloned();
    let optional = |name: &str| property(name).filter(|value| !value.is_empty());
    let payload = json!({
        "format": version,
        "name": property("Name").unwrap_or_default(),
        "synonym": optional("Synonym"),
        "version": optional("Version"),
        "vendor": optional("Vendor"),
        "extensionPurpose": optional("ConfigurationExtensionPurpose"),
        "support": support,
        "properties": {
            "compatibilityMode": optional("CompatibilityMode"),
            "defaultRunMode": optional("DefaultRunMode"),
            "scriptVariant": optional("ScriptVariant"),
            "defaultLanguage": optional("DefaultLanguage"),
            "dataLockControlMode": optional("DataLockControlMode"),
            "modalityUseMode": optional("ModalityUseMode"),
            "interfaceCompatibilityMode": optional("InterfaceCompatibilityMode"),
            "extensionCompatibilityMode": optional("ConfigurationExtensionCompatibilityMode"),
            "objectAutonumerationMode": optional("ObjectAutonumerationMode"),
            "synchronousCallUseMode": optional("SynchronousPlatformExtensionAndAddInCallUseMode"),
            "databaseTablespacesUseMode": optional("DatabaseTablespacesUseMode"),
            "mainWindowMode": optional("MainClientApplicationWindowMode"),
            "comment": optional("Comment"),
            "namePrefix": optional("NamePrefix"),
            "updateCatalogAddress": optional("UpdateCatalogAddress"),
        },
        "totalObjects": total_objects,
        "homePage": home_page,
        "interface": interface,
    });
    Ok(StreamedConfigurationRoot { payload, counts })
}

pub(crate) fn read_configuration_registration_index(
    file: File,
    checkpoint: &dyn Fn() -> Result<(), ViewError>,
) -> Result<RegistrationIndex, ViewError> {
    let state = parse_configuration_owner(file, checkpoint)?;
    RegistrationIndex::from_sorter(state.registrations, checkpoint)
}

fn parse_configuration_owner(
    mut file: File,
    checkpoint: &dyn Fn() -> Result<(), ViewError>,
) -> Result<RootState, ViewError> {
    skip_leading_boms(&mut file, checkpoint)?;
    let checkpoint_failure = Rc::new(RefCell::new(None));
    let checked = CheckedRead {
        file,
        checkpoint,
        failure: Rc::clone(&checkpoint_failure),
    };
    let mut config = ParserConfig::new()
        .coalesce_characters(false)
        .ignore_comments(false);
    config.allow_multiple_root_elements = false;
    // The parser's own defaults are size ceilings. A large source document
    // must not acquire a new rejection merely because one XML token crosses
    // such a ceiling. The parser still retains an individual token in memory;
    // the remaining single-token limitation is tracked separately.
    config.max_data_length = usize::MAX;
    config.max_attribute_length = usize::MAX;
    config.max_name_length = usize::MAX;
    config.max_attributes = usize::MAX;
    config.override_encoding = Some(xml::Encoding::Utf8);
    // The established parser receives an already-validated UTF-8 `&str` and
    // treats an XML declaration's encoding as syntax, not as an instruction to
    // reinterpret those bytes. Keep the same behavior for streamed bytes.
    config.ignore_invalid_encoding_declarations = true;
    let mut parser = config.create_reader(BufReader::new(checked));
    let mut state = RootState::new();
    loop {
        checkpoint()?;
        let event = match parser.next() {
            Ok(event) => event,
            Err(error) => {
                if let Some(reason) = checkpoint_failure.borrow_mut().take() {
                    return Err(reason);
                }
                return Err(ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    format!("Configuration XML parse error: {error}"),
                ));
            }
        };
        match event {
            XmlEvent::StartDocument { .. } => {}
            XmlEvent::StartElement {
                name, attributes, ..
            } => {
                state.start(name, attributes, checkpoint)?;
            }
            XmlEvent::EndElement { .. } => state.end(checkpoint)?,
            XmlEvent::Characters(text) | XmlEvent::CData(text) | XmlEvent::Whitespace(text) => {
                state.text(text);
            }
            XmlEvent::Doctype { .. } => {
                return Err(source_unreadable(
                    "Configuration.xml has an unsupported DOCTYPE",
                ));
            }
            XmlEvent::EndDocument => break,
            XmlEvent::ProcessingInstruction { .. } | XmlEvent::Comment(_) => {
                state.text_boundary();
            }
        }
    }
    if !state.saw_root
        || state.root_children != 1
        || !state.saw_configuration
        || !state.saw_properties
    {
        return Err(source_unreadable(
            "Configuration.xml has no single Configuration/Properties owner",
        ));
    }
    Ok(state)
}

fn source_unreadable(message: &str) -> ViewError {
    ViewError::detailed(RefusalDetail::SourceUnreadable, message)
}

fn skip_leading_boms(
    file: &mut File,
    checkpoint: &dyn Fn() -> Result<(), ViewError>,
) -> Result<(), ViewError> {
    loop {
        checkpoint()?;
        let start = file.stream_position().map_err(source_io_error)?;
        let mut bom = [0_u8; 3];
        match file.read_exact(&mut bom) {
            Ok(()) if bom == [0xef, 0xbb, 0xbf] => continue,
            Ok(()) => {
                file.seek(SeekFrom::Start(start)).map_err(source_io_error)?;
                return Ok(());
            }
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                file.seek(SeekFrom::Start(start)).map_err(source_io_error)?;
                return Ok(());
            }
            Err(error) => return Err(source_io_error(error)),
        }
    }
}

struct CheckedRead<'a> {
    file: File,
    checkpoint: &'a dyn Fn() -> Result<(), ViewError>,
    failure: Rc<RefCell<Option<ViewError>>>,
}

impl Read for CheckedRead<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if let Err(error) = (self.checkpoint)() {
            *self.failure.borrow_mut() = Some(error);
            return Err(io::Error::other("logical read checkpoint failed"));
        }
        let length = buffer.len().min(64 * 1024);
        self.file.read(&mut buffer[..length])
    }
}

#[derive(Default)]
struct RootState {
    stack: Vec<Frame>,
    // Unknown subtrees carry no facts. Keep only their nesting count in our
    // state instead of one application frame per XML element.
    ignored_depth: usize,
    saw_root: bool,
    root_children: usize,
    saw_configuration: bool,
    saw_properties: bool,
    saw_first_child_objects: bool,
    version: String,
    properties: BTreeMap<String, String>,
    total_objects: usize,
    registrations: RegistrationSorter,
}

impl RootState {
    fn new() -> Self {
        Self {
            registrations: RegistrationSorter::new(),
            ..Self::default()
        }
    }

    fn start(
        &mut self,
        name: OwnedName,
        attributes: Vec<xml::attribute::OwnedAttribute>,
        checkpoint: &dyn Fn() -> Result<(), ViewError>,
    ) -> Result<(), ViewError> {
        checkpoint()?;
        if self.ignored_depth != 0 {
            self.ignored_depth = self.ignored_depth.checked_add(1).ok_or_else(|| {
                ViewError::detailed(RefusalDetail::SourceUnreadable, "XML depth overflow")
            })?;
            return Ok(());
        }
        self.text_boundary();
        let md = name.namespace.as_deref() == Some(MD_NS);
        let mut role = FrameRole::Other;
        match self.stack.last_mut().map(|frame| &mut frame.role) {
            None => {
                if !md || name.local_name != "MetaDataObject" {
                    return Err(source_unreadable(
                        "source-set owner root must be MDClasses MetaDataObject",
                    ));
                }
                self.saw_root = true;
                self.version = attributes
                    .iter()
                    .find(|attribute| {
                        attribute.name.local_name == "version" && attribute.name.namespace.is_none()
                    })
                    .map(|attribute| attribute.value.clone())
                    .unwrap_or_default();
                role = FrameRole::Root;
            }
            Some(FrameRole::Root) => {
                self.root_children = self.root_children.checked_add(1).ok_or_else(|| {
                    ViewError::detailed(
                        RefusalDetail::SourceUnreadable,
                        "root artifact count overflow",
                    )
                })?;
                if md && name.local_name == "Configuration" {
                    self.saw_configuration = true;
                    role = FrameRole::Configuration;
                }
            }
            Some(FrameRole::Configuration)
                if md && name.local_name == "Properties" && !self.saw_properties =>
            {
                self.saw_properties = true;
                role = FrameRole::Properties;
            }
            Some(FrameRole::Configuration) if md && name.local_name == "ChildObjects" => {
                let primary = !self.saw_first_child_objects;
                self.saw_first_child_objects = true;
                role = FrameRole::ChildObjects { primary };
            }
            Some(FrameRole::Properties) if md => {
                if name.local_name == "Synonym" && !self.properties.contains_key("Synonym") {
                    role = FrameRole::Synonym {
                        ru: None,
                        fallback: None,
                    };
                } else if is_root_property(&name.local_name)
                    && !self.properties.contains_key(&name.local_name)
                {
                    role = FrameRole::Property(name.local_name);
                }
            }
            Some(FrameRole::ChildObjects { primary }) => {
                if *primary {
                    self.total_objects = self.total_objects.checked_add(1).ok_or_else(|| {
                        ViewError::detailed(
                            RefusalDetail::SourceUnreadable,
                            "Configuration object count overflow",
                        )
                    })?;
                }
                if md {
                    role = FrameRole::Registration {
                        kind: name.local_name,
                        fallback_name: None,
                        properties_seen: false,
                    };
                }
            }
            Some(FrameRole::Registration {
                properties_seen, ..
            }) if md && name.local_name == "Properties" && !*properties_seen => {
                *properties_seen = true;
                role = FrameRole::RegistrationProperties { name_seen: false };
            }
            Some(FrameRole::RegistrationProperties { name_seen })
                if md && name.local_name == "Name" && !*name_seen =>
            {
                *name_seen = true;
                role = FrameRole::RegistrationName;
            }
            Some(FrameRole::Synonym { .. }) => {
                role = FrameRole::SynonymItem {
                    lang: String::new(),
                    last_content: String::new(),
                    first_content: None,
                }
            }
            Some(FrameRole::SynonymItem { .. }) if name.local_name == "lang" => {
                role = FrameRole::SynonymLang
            }
            Some(FrameRole::SynonymItem { .. }) if name.local_name == "content" => {
                role = FrameRole::SynonymContent
            }
            _ => {}
        }
        if matches!(role, FrameRole::Other) {
            self.ignored_depth = 1;
            return Ok(());
        }
        self.stack.push(Frame {
            role,
            first_text: None,
            first_text_closed: false,
        });
        Ok(())
    }

    fn text(&mut self, text: String) {
        if self.ignored_depth != 0 || text.is_empty() {
            return;
        }
        if let Some(frame) = self.stack.last_mut() {
            if !frame.first_text_closed
                && matches!(
                    &frame.role,
                    FrameRole::Property(_)
                        | FrameRole::Registration { .. }
                        | FrameRole::RegistrationName
                        | FrameRole::SynonymLang
                        | FrameRole::SynonymContent
                )
            {
                frame
                    .first_text
                    .get_or_insert_with(String::new)
                    .push_str(&text);
            }
        }
    }

    fn text_boundary(&mut self) {
        if self.ignored_depth == 0 {
            if let Some(frame) = self.stack.last_mut() {
                frame.first_text_closed = true;
            }
        }
    }

    fn end(&mut self, checkpoint: &dyn Fn() -> Result<(), ViewError>) -> Result<(), ViewError> {
        checkpoint()?;
        if self.ignored_depth != 0 {
            self.ignored_depth -= 1;
            return Ok(());
        }
        let Some(frame) = self.stack.pop() else {
            return Err(source_unreadable(
                "Configuration XML has an unmatched closing tag",
            ));
        };
        match frame.role {
            FrameRole::Property(name) => {
                self.properties
                    .entry(name)
                    .or_insert_with(|| frame.first_text.unwrap_or_default());
            }
            FrameRole::Synonym { ru, fallback } => {
                self.properties
                    .entry("Synonym".to_string())
                    .or_insert(ru.or(fallback).unwrap_or_default());
            }
            FrameRole::SynonymItem {
                lang,
                last_content,
                first_content,
            } => {
                if let Some(Frame {
                    role: FrameRole::Synonym { ru, fallback },
                    ..
                }) = self.stack.last_mut()
                {
                    if ru.is_none() && lang == "ru" && !last_content.is_empty() {
                        *ru = Some(last_content);
                    }
                    if fallback.is_none() {
                        *fallback = first_content;
                    }
                }
            }
            FrameRole::SynonymLang => {
                if let Some(Frame {
                    role: FrameRole::SynonymItem { lang, .. },
                    ..
                }) = self.stack.last_mut()
                {
                    *lang = frame.first_text.unwrap_or_default();
                }
            }
            FrameRole::SynonymContent => {
                if let Some(Frame {
                    role:
                        FrameRole::SynonymItem {
                            last_content,
                            first_content,
                            ..
                        },
                    ..
                }) = self.stack.last_mut()
                {
                    let content = frame.first_text.unwrap_or_default();
                    if first_content.is_none() && !content.is_empty() {
                        *first_content = Some(content.clone());
                    }
                    *last_content = content;
                }
            }
            FrameRole::RegistrationName => {
                if self.stack.len() >= 2 {
                    let parent_index = self.stack.len() - 2;
                    if let FrameRole::Registration { fallback_name, .. } =
                        &mut self.stack[parent_index].role
                    {
                        let value = frame.first_text.unwrap_or_default();
                        if fallback_name.is_none() && !value.trim().is_empty() {
                            *fallback_name = Some(value.trim().to_string());
                        }
                    }
                }
            }
            FrameRole::Registration {
                kind,
                fallback_name,
                ..
            } => {
                let name = frame
                    .first_text
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .or(fallback_name);
                if let Some(name) = name {
                    self.registrations.push(kind, name, checkpoint)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

struct Frame {
    role: FrameRole,
    first_text: Option<String>,
    first_text_closed: bool,
}

enum FrameRole {
    Other,
    Root,
    Configuration,
    Properties,
    Property(String),
    ChildObjects {
        primary: bool,
    },
    Registration {
        kind: String,
        fallback_name: Option<String>,
        properties_seen: bool,
    },
    RegistrationProperties {
        name_seen: bool,
    },
    RegistrationName,
    Synonym {
        ru: Option<String>,
        fallback: Option<String>,
    },
    SynonymItem {
        lang: String,
        last_content: String,
        first_content: Option<String>,
    },
    SynonymLang,
    SynonymContent,
}

fn is_root_property(name: &str) -> bool {
    matches!(
        name,
        "Name"
            | "Version"
            | "Vendor"
            | "ConfigurationExtensionPurpose"
            | "CompatibilityMode"
            | "DefaultRunMode"
            | "ScriptVariant"
            | "DefaultLanguage"
            | "DataLockControlMode"
            | "ModalityUseMode"
            | "InterfaceCompatibilityMode"
            | "ConfigurationExtensionCompatibilityMode"
            | "ObjectAutonumerationMode"
            | "SynchronousPlatformExtensionAndAddInCallUseMode"
            | "DatabaseTablespacesUseMode"
            | "MainClientApplicationWindowMode"
            | "Comment"
            | "NamePrefix"
            | "UpdateCatalogAddress"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::refusal::RefusalCode;
    use std::cell::Cell;

    #[test]
    fn temporary_index_failures_are_distinct_from_unreadable_source_xml() {
        let scratch = scratch_error(io::Error::other("temporary storage unavailable"));
        assert_eq!(scratch.detail(), Some(RefusalDetail::BackendBroken));
        assert_eq!(scratch.code(), RefusalCode::TaskBackendFailed);
        assert_eq!(
            scratch_incomplete().detail(),
            Some(RefusalDetail::BackendBroken)
        );

        let source = source_io_error(io::Error::other("Configuration.xml unreadable"));
        assert_eq!(source.detail(), Some(RefusalDetail::SourceUnreadable));
        assert_eq!(source.code(), RefusalCode::ProviderUnavailable);
    }

    fn parsed(
        bytes: &[u8],
        checkpoint: &dyn Fn() -> Result<(), ViewError>,
    ) -> Result<StreamedConfigurationRoot, ViewError> {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(bytes).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        read_configuration_root(
            file,
            json!({"state": "not_supported"}),
            Value::Null,
            Value::Null,
            checkpoint,
        )
    }

    #[test]
    fn streaming_xml_rejects_strict_parser_false_positive_classes() {
        let base = r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Demo</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#;
        let cases = [
            ("ordinary", base.to_string()),
            (
                "non-UTF-8 declaration over UTF-8 bytes",
                base.replace("encoding=\"UTF-8\"", "encoding=\"ISO-8859-1\""),
            ),
            ("prologue comment", base.replacen("<MetaDataObject", "<!--<Fake/>--><?note value?><MetaDataObject", 1)),
            ("duplicate expanded attribute", base.replacen("version=\"2.20\"", "xmlns:a=\"urn:x\" xmlns:b=\"urn:x\" a:value=\"1\" b:value=\"2\" version=\"2.20\"", 1)),
            ("literal angle in attribute", base.replacen("version=\"2.20\"", "version=\"<\"", 1)),
            ("reserved namespace", base.replacen("version=\"2.20\"", "xmlns:xml=\"urn:wrong\" version=\"2.20\"", 1)),
            ("unknown entity", base.replacen("<Name>Demo</Name>", "<Name>&missing;</Name>", 1)),
            ("malformed tail", base.replace("</MetaDataObject>", "<Broken></MetaDataObject>")),
        ];
        for (case, source) in cases {
            let strict = roxmltree::Document::parse(source.trim_start_matches('\u{feff}')).is_ok();
            let streamed = parsed(source.as_bytes(), &|| Ok(())).is_ok();
            assert_eq!(
                streamed, strict,
                "strict/streaming XML grammar differs for {case}"
            );
        }
    }

    #[test]
    fn streaming_xml_rejects_invalid_utf8_inside_an_ignored_subtree() {
        let mut source = format!(
            "<MetaDataObject xmlns=\"{MD_NS}\"><Configuration><Properties><Name>Demo</Name></Properties><Ignored>"
        )
        .into_bytes();
        source.push(0xff);
        source.extend_from_slice(b"</Ignored><ChildObjects/></Configuration></MetaDataObject>");
        assert!(parsed(&source, &|| Ok(())).is_err());
    }

    #[test]
    fn configuration_name_text_matches_roxmltree_across_split_events() {
        for (case, name_body) in [
            ("plain", "First"),
            ("text then CDATA", "First<![CDATA[Second]]>"),
            ("CDATA then text", "<![CDATA[First]]>Second"),
            ("adjacent CDATA", "<![CDATA[First]]><![CDATA[Second]]>"),
            ("comment boundary", "First<!-- ignored -->Second"),
            ("leading comment", "<!-- ignored -->Second"),
            (
                "CDATA comment boundary",
                "<![CDATA[First]]><!-- ignored -->Second",
            ),
            ("PI boundary", "First<?note ignored?>Second"),
            ("entity", "First&amp;Second"),
        ] {
            let source = format!(
                "<MetaDataObject xmlns=\"{MD_NS}\" version=\"2.20\"><Configuration><Properties><Name>{name_body}</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"
            );
            let document = roxmltree::Document::parse(&source).unwrap();
            let expected = document
                .descendants()
                .find(|node| node.is_element() && node.tag_name().name() == "Name")
                .and_then(|node| node.text())
                .unwrap_or("");
            let actual = parsed(source.as_bytes(), &|| Ok(())).unwrap();
            assert_eq!(actual.payload["name"], expected, "text parity for {case}");
        }
    }

    #[test]
    fn streaming_xml_cancellation_stops_during_large_comment() {
        let source = format!(
            "<MetaDataObject xmlns=\"{MD_NS}\" version=\"2.20\"><Configuration><Properties><Name>Demo</Name></Properties><ChildObjects/></Configuration><!--{}--></MetaDataObject>",
            "x".repeat(8 * 1024 * 1024),
        );
        let checks = Cell::new(0);
        let result = parsed(source.as_bytes(), &|| {
            checks.set(checks.get() + 1);
            if checks.get() > 100 {
                Err(ViewError::new(
                    RefusalCode::Cancelled,
                    "test cancelled the XML read",
                ))
            } else {
                Ok(())
            }
        });
        assert_eq!(
            result.err().expect("large XML should be cancelled").code(),
            RefusalCode::Cancelled
        );
        assert!(checks.get() > 100);
    }

    #[test]
    fn irrelevant_xml_frames_do_not_retain_long_text() {
        let long_text = " ".repeat(1024 * 1024);
        for role in [
            FrameRole::Root,
            FrameRole::Configuration,
            FrameRole::ChildObjects { primary: true },
            FrameRole::RegistrationProperties { name_seen: false },
        ] {
            let mut state = RootState::new();
            state.stack.push(Frame {
                role,
                first_text: None,
                first_text_closed: false,
            });
            state.text(long_text.clone());
            assert!(state.stack[0].first_text.is_none());
        }
        let mut state = RootState::new();
        state.stack.push(Frame {
            role: FrameRole::Property("Name".to_string()),
            first_text: None,
            first_text_closed: false,
        });
        state.text("Real name".to_string());
        assert_eq!(state.stack[0].first_text.as_deref(), Some("Real name"));
    }

    #[test]
    fn external_registration_sort_deduplicates_across_multiple_runs() {
        let mut sorter = RegistrationSorter::new();
        for index in (0..80_000).rev() {
            let name = format!("Object{index:06}");
            sorter
                .push("WebSocketClient".to_string(), name.clone(), &|| Ok(()))
                .unwrap();
            if index % 1000 == 0 {
                sorter
                    .push("WebSocketClient".to_string(), name, &|| Ok(()))
                    .unwrap();
            }
        }
        assert!(
            !sorter.levels.is_empty(),
            "fixture must cross the in-memory run size"
        );
        let mut count = 0;
        let mut previous = String::new();
        sorter
            .for_each_unique(&|| Ok(()), |kind, name| {
                assert_eq!(kind, "WebSocketClient");
                assert!(
                    previous.as_str() < name,
                    "sorted registration order changed"
                );
                previous = name.to_string();
                count += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(count, 80_000);
    }

    #[test]
    fn registration_index_lists_only_sorted_unique_names_for_kind() {
        let mut sorter = RegistrationSorter::new();
        for (kind, name) in [
            ("Catalog", "Zulu"),
            ("CommonModule", "Beta"),
            ("Catalog", "Alpha"),
            ("Catalog", "Alpha"),
            ("Document", "Order"),
        ] {
            sorter
                .push(kind.to_string(), name.to_string(), &|| Ok(()))
                .unwrap();
        }
        let index = RegistrationIndex::from_sorter(sorter, &|| Ok(())).unwrap();
        assert_eq!(
            index.names_for_kind("Catalog", &|| Ok(())).unwrap(),
            ["Alpha", "Zulu"]
        );
        assert_eq!(
            index.names_for_kind("CommonModule", &|| Ok(())).unwrap(),
            ["Beta"]
        );
        assert!(index
            .names_for_kind("Empty", &|| Ok(()))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn retained_root_artifacts_are_reused_until_their_file_changes() {
        use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;

        let directory = tempfile::tempdir().unwrap();
        let root_path = std::fs::canonicalize(directory.path()).unwrap();
        let xml = |name: &str| {
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects><CommonModule>{name}</CommonModule></ChildObjects></Configuration></MetaDataObject>"#
            )
        };
        std::fs::write(root_path.join("Configuration.xml"), xml("First")).unwrap();
        let root = RetainedDirectoryCapability::open(&root_path).unwrap();
        let cache = RegistrationCache::default();

        let first = cache.registrations(&root, &|| Ok(())).unwrap();
        let same = cache.registrations(&root, &|| Ok(())).unwrap();
        assert!(Arc::ptr_eq(&first, &same));
        assert_eq!(cache.registration_builds_for_test(), 1);
        assert!(first.contains("CommonModule", "First", &|| Ok(())).unwrap());

        // A rewrite of the same length keeps the inode but moves the time; a
        // replacement through rename changes the identity. Both are observed.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(root_path.join("Configuration.xml"), xml("Other")).unwrap();
        let rewritten = cache.registrations(&root, &|| Ok(())).unwrap();
        assert!(rewritten
            .contains("CommonModule", "Other", &|| Ok(()))
            .unwrap());
        assert!(!rewritten
            .contains("CommonModule", "First", &|| Ok(()))
            .unwrap());
        std::fs::write(root_path.join("Replacement.xml"), xml("Renamed")).unwrap();
        std::fs::rename(
            root_path.join("Replacement.xml"),
            root_path.join("Configuration.xml"),
        )
        .unwrap();
        let renamed = cache.registrations(&root, &|| Ok(())).unwrap();
        assert!(renamed
            .contains("CommonModule", "Renamed", &|| Ok(()))
            .unwrap());
        assert_eq!(cache.registration_builds_for_test(), 3);
        let old = Arc::downgrade(&first);
        drop((first, same, rewritten));
        assert!(
            old.upgrade().is_none(),
            "a replaced index must not be retained"
        );

        let parses = std::cell::Cell::new(0);
        let support = |cache: &RegistrationCache| {
            cache
                .support_state(&root, &mut || Ok(()), |_file, _checkpoint| {
                    parses.set(parses.get() + 1);
                    Err(ViewError::detailed(
                        RefusalDetail::SourceUnreadable,
                        "support marker parsed",
                    ))
                })
                .map(|state| state.is_some())
        };
        assert!(!support(&cache).unwrap(), "an absent marker is no support");
        assert!(!support(&cache).unwrap());
        assert_eq!(parses.get(), 0);
        std::fs::create_dir(root_path.join("Ext")).unwrap();
        std::fs::write(root_path.join("Ext/ParentConfigurations.bin"), b"marker").unwrap();
        assert!(support(&cache).is_err(), "a new marker must be read");
        assert!(support(&cache).is_err(), "a failed parse is never retained");
        assert_eq!(parses.get(), 2);
    }

    #[test]
    fn unknown_xml_subtree_uses_one_state_counter_and_cannot_supply_parent_text() {
        let mut state = RootState::new();
        let named = |name: &str| OwnedName {
            local_name: name.to_string(),
            namespace: Some(MD_NS.to_string()),
            prefix: None,
        };
        for name in ["MetaDataObject", "Configuration", "Properties", "Comment"] {
            state.start(named(name), Vec::new(), &|| Ok(())).unwrap();
        }
        for _ in 0..10_000 {
            state
                .start(named("Ignored"), Vec::new(), &|| Ok(()))
                .unwrap();
        }
        assert_eq!(state.stack.len(), 4);
        assert_eq!(state.ignored_depth, 10_000);
        state.text("not direct text".to_string());
        for _ in 0..10_000 {
            state.end(&|| Ok(())).unwrap();
        }
        for _ in 0..4 {
            state.end(&|| Ok(())).unwrap();
        }
        assert_eq!(
            state.properties.get("Comment").map(String::as_str),
            Some("")
        );
    }
}
