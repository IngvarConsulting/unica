# Platform XML Source Adapter Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the PR #210 Platform XML navigation prototype with a version-evidenced source adapter core and one fail-closed Platform XML 2.20 read adapter behind `unica.meta.info`.

**Architecture:** Domain contracts describe source descriptors, snapshots, semantic identity, relations, capabilities, and structured failures. Infrastructure supplies deterministic probe/reader selection, then the Platform XML family implements provider, probe, decoder, support-state reader, and semantic projector as separate modules. `unica.meta.info` returns only a versioned navigation envelope; unsupported versions never select a guessed decoder and no legacy text analyzer remains.

**Tech Stack:** Rust, serde/serde_json, roxmltree, sha2, uuid, existing `unica-coder` hexagonal boundaries and inline Rust test conventions.

## Global Constraints

- The public MCP boundary remains one server named `unica` with `unica.*` tools.
- Do not add a generic public `execute(action)` tool.
- Direct CF and file-database access remains read-only and is outside this plan.
- Do not introduce a Rust dynamic-library ABI.
- Decoder selection must use explicit evidence and declared compatible ranges; nearest-version fallback is forbidden.
- Public object and relation references must not contain physical paths, database offsets, or parser handles.
- A readable source is not automatically writable.
- Mutation is never `executable` without a specialized tool binding and a compatible mutation adapter.
- Replace the legacy `meta.info` text contract outright; backward compatibility
  is not a requirement.
- Remove `Mode`, `Name`, `Limit`, `Offset`, and `OutFile` from the
  `unica.meta.info` input schema.
- Return the result only through the versioned `data.navigation` envelope;
  `meta.info` must not populate `stdout`.
- Use JSON as the only canonical tool-response representation.
- Return every semantic property with an explicit value type and value state.
- Replace legacy drill-down and offset pagination with `objectRef`,
  `snapshotRevision`, `select`, and snapshot-bound cursor requests.
- Default relation page size is 25; maximum page size is 100.
- Platform XML 2.20 is the only read format certified by this plan.
- For other formats, return an explicit unavailable navigation envelope without
  invoking a legacy analyzer.
- Fix source-map, support-state, identity, and relation failures at their source; do not mask them with fallback identities or permissive capabilities.
- Do not change `plugins/unica/.mcp.json`, `plugins/unica/.codex-plugin/plugin.json`, or `plugins/unica/third-party/tools.lock.json` in this plan.

---

## Scope Decomposition

This plan is the first independently reviewable implementation slice from
`spec/designs/2026-07-26-versioned-source-adapter-architecture.md`.

It includes:

- source descriptor, version, snapshot, and error contracts;
- semantic identity and capability corrections;
- deterministic built-in adapter registry;
- Platform XML provider, probe, decoder, support reader, and projector;
- integration with `unica.meta.info`;
- reusable read-adapter certification tests.

It excludes these independently testable programs of work:

- Platform XML form-content projection;
- the first specialized Platform XML writer;
- EDT readers;
- binary CF readers;
- file-database readers;
- the external adapter process protocol.

Each excluded area requires its own design-derived implementation plan after
this plan proves the shared contracts.

## Planned File Structure

### Domain

- `crates/unica-coder/src/domain/source_adapters.rs`
  - source families, versions, descriptors, snapshots, manifests, maturity,
    coverage, and structured error categories.
- `crates/unica-coder/src/domain/navigation.rs`
  - semantic object/relation identity, navigation envelope, graph, capability
    vector, typed properties, lazy query selection, cursor pages, action
    availability, atomicity, and operation bindings.
- `crates/unica-coder/src/domain/mod.rs`
  - exports the source-adapter domain module.

### Infrastructure core

- `crates/unica-coder/src/infrastructure/source_adapters/mod.rs`
  - source probe and read-adapter facade traits plus built-in registry entrypoint.
- `crates/unica-coder/src/infrastructure/source_adapters/registry.rs`
  - deterministic probe and adapter selection.
- `crates/unica-coder/src/infrastructure/source_adapters/certification.rs`
  - reusable test-only read-adapter certification assertions.
- `crates/unica-coder/src/infrastructure/mod.rs`
  - exports the infrastructure source-adapter module.

### Platform XML family

- `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/mod.rs`
  - Platform XML facade and manifest.
- `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/provider.rs`
  - bounded aggregate read set, relative-path validation, immutable bytes, and
    revision evidence.
- `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/probe.rs`
  - root evidence and exact Platform XML 2.20 recognition.
- `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/native_model.rs`
  - Platform XML native snapshot types.
- `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/decoder.rs`
  - descriptor and content decoding with identity and uniqueness checks.
- `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/support.rs`
  - strict `ParentConfigurations.bin` state and rule parsing.
- `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/projector.rs`
  - native-to-semantic projection and capability facts.

### Existing integration points

- `crates/unica-coder/src/infrastructure/native_operations/meta.rs`
  - delegates navigation to the built-in registry and retains legacy text.
- `crates/unica-coder/src/infrastructure/native_operations/typed_result.rs`
  - serializes the navigation envelope as typed data.
- `crates/unica-coder/src/application/operation_descriptors.rs`
  - removes `OutFile` from the `meta-info` operation descriptor.
- `crates/unica-coder/src/infrastructure/native_operations/common.rs`
  - removes the duplicated permissive support parser.
- `crates/unica-coder/src/infrastructure/support_guard.rs`
  - consumes the shared strict support facts.
- `plugins/unica/skills/meta-info/SKILL.md`
  - replaces legacy text-mode, drill-down, pagination, and output-file guidance
    with the typed semantic-navigation contract.

---

### Task 1: Source descriptor, version, snapshot, and error contracts

**Files:**
- Create: `crates/unica-coder/src/domain/source_adapters.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`
- Test: inline tests in `crates/unica-coder/src/domain/source_adapters.rs`

**Interfaces:**
- Consumes: no new interfaces.
- Produces:
  - `FormatVersion::parse(&str) -> Result<FormatVersion, SourceAdapterError>`
  - `FormatRange::contains(&self, &FormatVersion) -> bool`
  - `SourceDescriptor`
  - `SourceSnapshot`
  - `AdapterManifest`
  - `SourceAdapterError`

- [ ] **Step 1: Write failing version, range, and serialization tests**

```rust
#[test]
fn format_ranges_are_explicit_and_do_not_select_nearest_versions() {
    let range = FormatRange::exact(FormatVersion::parse("2.20").unwrap());

    assert!(range.contains(&FormatVersion::parse("2.20").unwrap()));
    assert!(!range.contains(&FormatVersion::parse("2.19").unwrap()));
    assert!(!range.contains(&FormatVersion::parse("2.21").unwrap()));
}

#[test]
fn invalid_format_versions_are_structured_failures() {
    let error = FormatVersion::parse("2.latest").unwrap_err();

    assert_eq!(error.kind, SourceAdapterErrorKind::FormatUnsupported);
    assert_eq!(error.code(), "format_unsupported");
}

#[test]
fn snapshot_serialization_does_not_expose_physical_locations() {
    let snapshot = SourceSnapshot {
        source_id: SourceId::new("workspace:main").unwrap(),
        revision: SourceRevision::new("sha256:abc").unwrap(),
        consistency: SnapshotConsistency::Consistent,
        adapter_id: "platform-xml-2.20".to_string(),
    };
    let value = serde_json::to_value(snapshot).unwrap();
    let text = value.to_string();

    assert!(!text.contains("/Users/"));
    assert!(!text.contains("C:\\\\"));
}
```

- [ ] **Step 2: Run the focused tests and confirm the module is missing**

Run:

```bash
cargo test -p unica-coder domain::source_adapters::tests -- --nocapture
```

Expected: compilation fails because `domain::source_adapters` and its types do
not exist.

- [ ] **Step 3: Implement the source contracts**

Use ordered numeric version components instead of semantic-version inference:

```rust
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FormatVersion(Vec<u32>);

impl FormatVersion {
    pub(crate) fn parse(raw: &str) -> Result<Self, SourceAdapterError> {
        let parts = raw
            .split('.')
            .map(str::parse::<u32>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| SourceAdapterError::new(
                SourceAdapterErrorKind::FormatUnsupported,
                format!("invalid format version `{raw}`"),
            ))?;
        if parts.is_empty() || parts.iter().all(|part| *part == 0) {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::FormatUnsupported,
                format!("invalid format version `{raw}`"),
            ));
        }
        Ok(Self(parts))
    }
}

impl std::fmt::Display for FormatVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let rendered = self
            .0
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(".");
        formatter.write_str(&rendered)
    }
}

impl Serialize for FormatVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FormatRange {
    pub(crate) min_inclusive: FormatVersion,
    pub(crate) max_inclusive: FormatVersion,
}

impl FormatRange {
    pub(crate) fn exact(version: FormatVersion) -> Self {
        Self {
            min_inclusive: version.clone(),
            max_inclusive: version,
        }
    }

    pub(crate) fn contains(&self, version: &FormatVersion) -> bool {
        self.min_inclusive <= *version && *version <= self.max_inclusive
    }
}
```

Define the remaining contracts with exact serialized names:

```rust
pub(crate) enum SourceFamily {
    PlatformXml,
    Edt,
    Cf,
    FileDatabase,
}

pub(crate) enum SnapshotConsistency {
    Consistent,
    Partial,
    Changed,
    Unverifiable,
}

pub(crate) enum AdapterMaturity {
    Experimental,
    ProbeComplete,
    ReadCompatible,
    SemanticParity,
    WriteSafe,
}

pub(crate) enum SourceAccess {
    ReadOnly,
    ReadWrite,
}

pub(crate) struct SourceDescriptor {
    pub(crate) source_id: SourceId,
    pub(crate) family: SourceFamily,
    pub(crate) format_version: FormatVersion,
    pub(crate) producer_version: Option<FormatVersion>,
    pub(crate) detected_features: BTreeSet<String>,
    pub(crate) probe_evidence: Vec<String>,
}

pub(crate) struct SourceSnapshot {
    pub(crate) source_id: SourceId,
    pub(crate) revision: SourceRevision,
    pub(crate) consistency: SnapshotConsistency,
    pub(crate) adapter_id: String,
}

pub(crate) struct AdapterManifest {
    pub(crate) adapter_id: &'static str,
    pub(crate) adapter_version: &'static str,
    pub(crate) source_family: SourceFamily,
    pub(crate) supported_formats: Vec<FormatRange>,
    pub(crate) required_features: BTreeSet<String>,
    pub(crate) excluded_features: BTreeSet<String>,
    pub(crate) source_access: SourceAccess,
    pub(crate) maturity: AdapterMaturity,
}
```

Define every accepted error code from the design:

```rust
pub(crate) enum SourceAdapterErrorKind {
    SourceUnavailable,
    ProbeAmbiguous,
    FormatUnsupported,
    SnapshotInconsistent,
    SnapshotStale,
    DecodeCorrupted,
    ProjectionAmbiguous,
    IdentityCollision,
    CapabilityBlocked,
    MutationConflict,
    ValidationFailed,
    RecoveryRequired,
}
```

`SourceId` and `SourceRevision` constructors must reject empty strings and
control characters. Their `Serialize` implementations are transparent.

- [ ] **Step 4: Export the module and run the focused tests**

Add to `domain/mod.rs`:

```rust
pub(crate) mod source_adapters;
```

Run:

```bash
cargo test -p unica-coder domain::source_adapters::tests -- --nocapture
```

Expected: all source-adapter domain tests pass.

- [ ] **Step 5: Commit the domain contracts**

```bash
git add crates/unica-coder/src/domain/mod.rs \
  crates/unica-coder/src/domain/source_adapters.rs
git commit -m "feat: add source adapter domain contracts"
```

---

### Task 2: Versioned navigation identity and truthful capabilities

**Files:**
- Modify: `crates/unica-coder/src/domain/navigation.rs:11`
- Test: inline tests in `crates/unica-coder/src/domain/navigation.rs`

**Interfaces:**
- Consumes:
  - `SourceId`
  - `SourceRevision`
  - `SnapshotConsistency`
  - `SourceAccess`
- Produces:
  - `ObjectRef`
  - `RelationRef`
  - `NavigationEnvelope`
  - `SemanticProperty`
  - `NavigationQuery`
  - `NavigationCursor`
  - `CapabilityVector`
  - `SemanticAction`

- [ ] **Step 1: Add failing identity, relation, and capability tests**

```rust
#[test]
fn object_identity_is_not_derived_from_display_name_alone() {
    let left = ObjectRef::new(
        source_id("workspace:main"),
        ObjectKey::new("uuid:11111111-1111-1111-1111-111111111111").unwrap(),
        IdentityStrength::Persistent,
        NodeKind::Document,
        "Order",
    );
    let renamed = ObjectRef::new(
        source_id("workspace:main"),
        ObjectKey::new("uuid:11111111-1111-1111-1111-111111111111").unwrap(),
        IdentityStrength::Persistent,
        NodeKind::Document,
        "CustomerOrder",
    );

    assert_eq!(left.identity(), renamed.identity());
}

#[test]
fn resolved_authorable_but_format_incompatible_is_not_executable() {
    let capability = CapabilityVector {
        resolution: ResolutionState::Resolved,
        identity: IdentityStrength::Persistent,
        consistency: SnapshotConsistency::Consistent,
        coverage: CoverageState::Complete,
        format: FormatCompatibility::Incompatible,
        source_access: SourceAccess::ReadWrite,
        authorability: Authorability::Authorable,
    };

    assert!(!capability.permits_mutation());
    assert_eq!(
        capability.blocking_reasons(),
        vec![CapabilityBlockReason::FormatIncompatible]
    );
}

#[test]
fn clone_requires_an_explicit_owning_relation() {
    let action = SemanticAction::modeled_clone(node_ref(), None);

    assert_eq!(action.availability, ActionAvailability::Blocked);
    assert_eq!(
        action.blocking_reasons,
        vec![CapabilityBlockReason::OwningRelationMissing]
    );
}

#[test]
fn navigation_envelope_always_has_a_schema_version_and_status() {
    let envelope = NavigationEnvelope::unavailable(
        SourceAdapterError::new(
            SourceAdapterErrorKind::FormatUnsupported,
            "Platform XML 2.19 has no certified reader",
        ),
    );
    let value = serde_json::to_value(envelope).unwrap();

    assert_eq!(value["schemaVersion"], "1");
    assert_eq!(value["status"], "unavailable");
    assert_eq!(value["diagnostics"][0]["code"], "format_unsupported");
}

#[test]
fn properties_preserve_type_value_and_value_state() {
    let property = SemanticProperty::explicit(
        PropertyType::Integer,
        PropertyValue::Integer(11),
        PropertyProvenance::Descriptor,
    )
    .unwrap();
    let value = serde_json::to_value(property).unwrap();

    assert_eq!(value["type"], "integer");
    assert_eq!(value["value"], 11);
    assert_eq!(value["valueState"], "explicit");
}

#[test]
fn incompatible_property_type_and_value_are_rejected() {
    let error = SemanticProperty::explicit(
        PropertyType::Integer,
        PropertyValue::String("11".to_string()),
        PropertyProvenance::Descriptor,
    )
    .unwrap_err();

    assert_eq!(error.kind, SourceAdapterErrorKind::ProjectionAmbiguous);
}

#[test]
fn relation_page_size_is_bounded() {
    assert_eq!(RelationSelection::new("attributes", None).unwrap().page_size, 25);
    assert!(RelationSelection::new("attributes", Some(101)).is_err());
}

#[test]
fn cursor_is_bound_to_snapshot_revision() {
    let cursor = NavigationCursor::issue(
        source_id("workspace:main"),
        SourceRevision::new("sha256:one").unwrap(),
        relation_ref(),
        25,
    );

    let error = cursor
        .resume(&SourceRevision::new("sha256:two").unwrap())
        .unwrap_err();
    assert_eq!(error.kind, SourceAdapterErrorKind::SnapshotStale);
}
```

- [ ] **Step 2: Run focused navigation tests and confirm they fail**

Run:

```bash
cargo test -p unica-coder domain::navigation::tests -- --nocapture
```

Expected: compilation fails because the new identity, envelope, and capability
types do not exist.

- [ ] **Step 3: Replace path-shaped identity with opaque semantic identity**

Add:

```rust
pub(crate) struct ObjectRef {
    pub(crate) source_id: SourceId,
    pub(crate) object_key: ObjectKey,
    pub(crate) identity_strength: IdentityStrength,
    pub(crate) kind: NodeKind,
    pub(crate) display_name: String,
}

pub(crate) struct RelationRef {
    pub(crate) source_id: SourceId,
    pub(crate) relation_key: RelationKey,
    pub(crate) kind: RelationKind,
}

pub(crate) enum IdentityStrength {
    Persistent,
    Derived,
    SnapshotOnly,
}
```

`ObjectKey` and `RelationKey` are opaque serialized strings. Their constructors
reject empty values, control characters, and strings beginning with `/`,
`\\`, or a Windows drive prefix. Existing owner chains may remain in node
display metadata, but equality and relation endpoints use the opaque keys.

Add `NodeKind::SourceRoot` so an inspected metadata object has a semantic owner
without pretending that every source is a configuration.

- [ ] **Step 4: Implement capability and envelope truthfulness**

Use:

```rust
pub(crate) struct CapabilityVector {
    pub(crate) resolution: ResolutionState,
    pub(crate) identity: IdentityStrength,
    pub(crate) consistency: SnapshotConsistency,
    pub(crate) coverage: CoverageState,
    pub(crate) format: FormatCompatibility,
    pub(crate) source_access: SourceAccess,
    pub(crate) authorability: Authorability,
}

pub(crate) enum ActionAvailability {
    Modeled,
    Executable,
    Blocked,
}

pub(crate) enum Atomicity {
    SingleFileAtomicReplace,
    AggregateSwapWithRecovery,
    BackendTransaction,
    ReadOnly,
}

pub(crate) struct OperationBinding {
    pub(crate) tool: String,
    pub(crate) schema_version: String,
}

pub(crate) struct NavigationEnvelope {
    pub(crate) schema_version: String,
    pub(crate) status: NavigationStatus,
    pub(crate) snapshot: Option<SourceSnapshot>,
    pub(crate) root: Option<ObjectRef>,
    pub(crate) nodes: Vec<NavigationNode>,
    pub(crate) relations: Vec<NavigationRelationPage>,
    pub(crate) diagnostics: Vec<SourceAdapterDiagnostic>,
}
```

`CapabilityVector::permits_mutation()` returns true only for:

```rust
matches!(self.resolution, ResolutionState::Resolved)
    && !matches!(self.identity, IdentityStrength::SnapshotOnly)
    && matches!(self.consistency, SnapshotConsistency::Consistent)
    && matches!(self.coverage, CoverageState::Complete)
    && matches!(self.format, FormatCompatibility::Compatible)
    && matches!(self.source_access, SourceAccess::ReadWrite)
    && matches!(self.authorability, Authorability::Authorable)
```

An action is `Executable` only when `permits_mutation()` is true and
`operation_binding` is present. A known action without an operation binding is
`Modeled`. A failed precondition is `Blocked` with all applicable structured
reasons.

Migrate the PR #210 action-profile tests to the new assertions. Keep `remove`
unadvertised because authorability is not sufficient removal eligibility.

Define the typed property contract:

```rust
pub(crate) enum PropertyType {
    Boolean,
    Integer,
    Decimal,
    String,
    LocalizedString,
    Uuid,
    Enum { enum_type: String },
    Date,
    TypeSet,
    ObjectRef,
    List,
    Structure,
    Null,
    Unknown,
}

pub(crate) enum PropertyValue {
    Boolean(bool),
    Integer(i64),
    Decimal(String),
    String(String),
    LocalizedString(BTreeMap<String, String>),
    Uuid(Uuid),
    EnumSymbol(String),
    Date(String),
    TypeSet(TypeSetValue),
    ObjectRef(ObjectRef),
    List(Vec<PropertyValue>),
    Structure(BTreeMap<String, PropertyValue>),
    Null,
    Unknown { summary: String },
}

pub(crate) enum PropertyValueState {
    Explicit,
    Defaulted,
    Inherited,
    Computed,
    Absent,
    Unresolved,
}

pub(crate) struct SemanticProperty {
    pub(crate) value_type: PropertyType,
    pub(crate) value_state: PropertyValueState,
    pub(crate) value: Option<PropertyValue>,
    pub(crate) provenance: PropertyProvenance,
    pub(crate) capability: PropertyCapability,
}
```

Serialize `value_type` as the public `type` field and use camelCase enum names.
`SemanticProperty` constructors validate that the value variant matches
`value_type`. `Absent` and `Unresolved` carry no value. `Defaulted` is legal
only when the exact projector profile supplies the default.

Define the bounded query contract:

```rust
pub(crate) enum NavigationTarget {
    ObjectPath(String),
    ObjectRef {
        object_ref: ObjectRef,
        snapshot_revision: SourceRevision,
    },
    Cursor(NavigationCursor),
}

pub(crate) struct NavigationQuery {
    pub(crate) target: NavigationTarget,
    pub(crate) select: NavigationSelection,
}

pub(crate) struct NavigationSelection {
    pub(crate) properties: PropertySelection,
    pub(crate) facets: FacetSelection,
    pub(crate) relations: Vec<RelationSelection>,
}

pub(crate) struct RelationSelection {
    pub(crate) kind: Option<RelationKind>,
    pub(crate) role: String,
    pub(crate) page_size: u16,
}

pub(crate) struct NavigationCursor {
    pub(crate) schema_version: u16,
    pub(crate) source_id: SourceId,
    pub(crate) snapshot_revision: SourceRevision,
    pub(crate) target: ObjectKey,
    pub(crate) relation: RelationKey,
    pub(crate) selection_hash: String,
    pub(crate) next_position: u64,
}
```

`NavigationCursor` serializes as an opaque JSON object. Clients pass it back
unchanged and must not synthesize its fields. It contains no physical path.
Cursor decoding validates its schema version and normalized selection hash,
re-resolves its target and relation, and rejects a mismatched revision with
`SnapshotStale`.

- [ ] **Step 5: Run navigation tests**

Run:

```bash
cargo test -p unica-coder domain::navigation::tests -- --nocapture
```

Expected: all navigation tests pass, including existing action-profile and
atomicity cases.

- [ ] **Step 6: Commit the navigation contract**

```bash
git add crates/unica-coder/src/domain/navigation.rs
git commit -m "feat: version semantic navigation identity"
```

---

### Task 3: Deterministic source probe and read-adapter registry

**Files:**
- Create: `crates/unica-coder/src/infrastructure/source_adapters/mod.rs`
- Create: `crates/unica-coder/src/infrastructure/source_adapters/registry.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs:1`
- Test: inline tests in `crates/unica-coder/src/infrastructure/source_adapters/registry.rs`

**Interfaces:**
- Consumes:
  - `AdapterManifest`
  - `SourceDescriptor`
  - `SourceSnapshot`
  - `NavigationEnvelope`
- Produces:
  - `SourceInput`
  - `SourceProbe`
  - `SourceReadAdapter`
  - `BuiltInSourceAdapterRegistry::inspect(SourceInput)`

- [ ] **Step 1: Write failing deterministic-selection tests with fake adapters**

```rust
#[test]
fn exact_reader_is_selected_for_probe_evidence() {
    let registry = registry_with(
        vec![probe_match("2.20")],
        vec![reader("xml-2.20", exact("2.20"))],
    );

    let read = registry.inspect(input()).unwrap();

    assert_eq!(read.snapshot.unwrap().adapter_id, "xml-2.20");
}

#[test]
fn nearest_reader_is_never_selected() {
    let registry = registry_with(
        vec![probe_match("2.19")],
        vec![reader("xml-2.20", exact("2.20"))],
    );

    let envelope = registry.inspect(input()).unwrap();

    assert_eq!(envelope.status, NavigationStatus::Unavailable);
    assert_eq!(envelope.diagnostics[0].code, "format_unsupported");
}

#[test]
fn equally_specific_readers_are_ambiguous() {
    let registry = registry_with(
        vec![probe_match("2.20")],
        vec![
            reader("xml-a", exact("2.20")),
            reader("xml-b", exact("2.20")),
        ],
    );

    let error = registry.inspect(input()).unwrap_err();

    assert_eq!(error.kind, SourceAdapterErrorKind::ProbeAmbiguous);
}
```

- [ ] **Step 2: Run the registry tests and confirm the module is missing**

Run:

```bash
cargo test -p unica-coder source_adapters::registry::tests -- --nocapture
```

Expected: compilation fails because the infrastructure module does not exist.

- [ ] **Step 3: Define the facade traits**

In `source_adapters/mod.rs`:

```rust
pub(crate) struct SourceInput {
    pub(crate) workspace_root: PathBuf,
    pub(crate) target: PathBuf,
    pub(crate) configured_source_set: Option<String>,
}

pub(crate) enum ProbeOutcome {
    NoMatch,
    Match(SourceDescriptor),
}

pub(crate) trait SourceProbe: Send + Sync {
    fn probe(&self, input: &SourceInput)
        -> Result<ProbeOutcome, SourceAdapterError>;
}

pub(crate) trait SourceReadAdapter: Send + Sync {
    fn manifest(&self) -> &AdapterManifest;

    fn inspect(
        &self,
        input: &SourceInput,
        descriptor: &SourceDescriptor,
    ) -> Result<NavigationEnvelope, SourceAdapterError>;
}
```

The traits return semantic data. Native snapshots remain private to each source
family and cannot leak physical types into application or domain layers.

- [ ] **Step 4: Implement deterministic registry selection**

`BuiltInSourceAdapterRegistry` owns:

```rust
pub(crate) struct BuiltInSourceAdapterRegistry {
    probes: Vec<Box<dyn SourceProbe>>,
    readers: Vec<Box<dyn SourceReadAdapter>>,
}
```

`inspect` must:

1. collect all `ProbeOutcome::Match` results;
2. return `ProbeAmbiguous` if probes identify incompatible descriptors;
3. filter readers by exact family, range, required features, and exclusions;
4. select the compatible reader with the narrowest version range;
5. return `ProbeAmbiguous` when specificity is tied;
6. return `NavigationEnvelope::unavailable(FormatUnsupported)` when no reader
   is compatible;
7. never retry a different reader after a selected reader reports corruption.

- [ ] **Step 5: Export the module and run registry tests**

Add to `infrastructure/mod.rs`:

```rust
pub(crate) mod source_adapters;
```

Run:

```bash
cargo test -p unica-coder source_adapters::registry::tests -- --nocapture
```

Expected: all deterministic-selection tests pass.

- [ ] **Step 6: Commit the registry**

```bash
git add crates/unica-coder/src/infrastructure/mod.rs \
  crates/unica-coder/src/infrastructure/source_adapters/mod.rs \
  crates/unica-coder/src/infrastructure/source_adapters/registry.rs
git commit -m "feat: add deterministic source adapter registry"
```

---

### Task 4: Platform XML aggregate provider and exact 2.20 probe

**Files:**
- Create: `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/mod.rs`
- Create: `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/provider.rs`
- Create: `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/probe.rs`
- Modify: `crates/unica-coder/src/infrastructure/source_adapters/mod.rs`
- Test: inline tests in `provider.rs` and `probe.rs`

**Interfaces:**
- Consumes:
  - `SourceInput`
  - `SourceProbe`
  - `SourceDescriptor`
- Produces:
  - `PlatformXmlProvider`
  - `PlatformXmlProbe`
  - exact Platform XML 2.20 probe evidence

- [ ] **Step 1: Write failing provider boundary tests**

```rust
#[test]
fn provider_rejects_parent_traversal_before_io() {
    let provider = fixture_provider();
    let error = provider.read_relative("../outside.xml").unwrap_err();

    assert_eq!(error.kind, SourceAdapterErrorKind::SourceUnavailable);
}

#[test]
fn provider_returns_the_same_immutable_bytes_for_repeated_reads() {
    let provider = fixture_provider_with("Object.xml", b"<MetaDataObject/>");

    let first = provider.read_relative("Object.xml").unwrap();
    overwrite_fixture("Object.xml", b"changed");
    let second = provider.read_relative("Object.xml").unwrap();

    assert_eq!(first.as_ref(), second.as_ref());
}
```

- [ ] **Step 2: Write failing probe tests**

```rust
#[test]
fn probe_recognizes_exact_platform_xml_2_20() {
    let outcome = probe_fixture(
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"
             version="2.20"><Document uuid="11111111-1111-1111-1111-111111111111"/></MetaDataObject>"#,
    )
    .unwrap();

    let ProbeOutcome::Match(descriptor) = outcome else {
        panic!("expected Platform XML match");
    };
    assert_eq!(descriptor.family, SourceFamily::PlatformXml);
    assert_eq!(descriptor.format_version, FormatVersion::parse("2.20").unwrap());
}

#[test]
fn probe_reports_but_does_not_guess_platform_xml_2_19() {
    let outcome = probe_fixture(
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"
             version="2.19"><Document/></MetaDataObject>"#,
    )
    .unwrap();

    let ProbeOutcome::Match(descriptor) = outcome else {
        panic!("family and version should still be evidenced");
    };
    assert_eq!(descriptor.format_version, FormatVersion::parse("2.19").unwrap());
}
```

- [ ] **Step 3: Run focused tests and confirm the family module is missing**

Run:

```bash
cargo test -p unica-coder source_adapters::platform_xml -- --nocapture
```

Expected: compilation fails because the Platform XML family module does not
exist.

- [ ] **Step 4: Implement the bounded provider**

`PlatformXmlProvider` owns the aggregate root and a mutex-protected map of
validated relative paths to immutable `Arc<[u8]>` values:

```rust
pub(crate) struct PlatformXmlProvider {
    root: PathBuf,
    reads: Mutex<BTreeMap<PathBuf, Arc<[u8]>>>,
}

impl PlatformXmlProvider {
    pub(crate) fn read_relative(
        &self,
        raw: impl AsRef<Path>,
    ) -> Result<Arc<[u8]>, SourceAdapterError>;

    pub(crate) fn revision(&self) -> Result<SourceRevision, SourceAdapterError>;
}
```

`read_relative` rejects absolute paths, `..`, symlink escape, non-regular files,
and files outside the aggregate root. It reads each accepted file once.

`revision()` hashes, in sorted order, each read-set relative path, byte length,
and SHA-256 digest. Physical paths are never included in the serialized
revision.

- [ ] **Step 5: Implement the evidence-only probe**

`PlatformXmlProbe` reads only the requested root descriptor and proves:

- UTF-8 XML after optional BOM;
- root local name `MetaDataObject`;
- expected 1C metadata namespace;
- exactly one metadata-class child;
- non-empty `version`;
- valid UUID when the class declares one.

The probe recognizes Platform XML family and version even when no compatible
reader exists. Registry selection, not the probe, decides support.

For configured source sets, use `SourceId("workspace:<source-set>")`. For a
valid project map whose target is not a configured source set, use an ad-hoc ID
derived from root descriptor bytes. A project-map discovery error is returned
and never converted into an ad-hoc ID.

- [ ] **Step 6: Declare the first reader manifest**

In `platform_xml/mod.rs`:

```rust
pub(crate) fn manifest() -> AdapterManifest {
    AdapterManifest {
        adapter_id: "platform-xml-2.20",
        adapter_version: env!("CARGO_PKG_VERSION"),
        source_family: SourceFamily::PlatformXml,
        supported_formats: vec![FormatRange::exact(
            FormatVersion::parse("2.20").expect("constant version"),
        )],
        required_features: BTreeSet::new(),
        excluded_features: BTreeSet::new(),
        source_access: SourceAccess::ReadOnly,
        maturity: AdapterMaturity::ProbeComplete,
    }
}
```

- [ ] **Step 7: Run Platform XML provider and probe tests**

Run:

```bash
cargo test -p unica-coder source_adapters::platform_xml -- --nocapture
```

Expected: provider and probe tests pass.

- [ ] **Step 8: Commit provider and probe**

```bash
git add crates/unica-coder/src/infrastructure/source_adapters/mod.rs \
  crates/unica-coder/src/infrastructure/source_adapters/platform_xml
git commit -m "feat: probe Platform XML source versions"
```

---

### Task 5: Extract the Platform XML native decoder from `meta.rs`

**Files:**
- Create: `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/native_model.rs`
- Create: `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/decoder.rs`
- Modify: `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta.rs:4616`
- Test: inline tests moved from `meta.rs` to `decoder.rs`

**Interfaces:**
- Consumes:
  - `PlatformXmlProvider`
  - `SourceDescriptor`
  - `SourceSnapshot`
- Produces:
  - `PlatformXmlNativeSnapshot`
  - `decode(&PlatformXmlProvider, &SourceDescriptor)`

- [ ] **Step 1: Add failing decoder identity and uniqueness tests**

```rust
#[test]
fn duplicate_inline_child_names_are_identity_collisions() {
    let provider = document_fixture(
        r#"
        <Document uuid="11111111-1111-1111-1111-111111111111">
          <ChildObjects>
            <TabularSection><Properties><Name>Lines</Name></Properties></TabularSection>
            <TabularSection><Properties><Name>Lines</Name></Properties></TabularSection>
          </ChildObjects>
        </Document>
        "#,
    );

    let error = decode(&provider, &descriptor_2_20()).unwrap_err();

    assert_eq!(error.kind, SourceAdapterErrorKind::IdentityCollision);
}

#[test]
fn invalid_inline_child_name_is_not_promoted_to_a_mutable_native_node() {
    let provider = document_fixture(
        r#"
        <Document uuid="11111111-1111-1111-1111-111111111111">
          <ChildObjects>
            <Attribute><Properties><Name>../Bad</Name></Properties></Attribute>
          </ChildObjects>
        </Document>
        "#,
    );

    let error = decode(&provider, &descriptor_2_20()).unwrap_err();

    assert_eq!(error.kind, SourceAdapterErrorKind::DecodeCorrupted);
}

#[test]
fn conflicting_descriptor_fields_are_projection_ambiguity() {
    let provider = descriptor_fixture_with_conflicting_names();

    let error = decode(&provider, &descriptor_2_20()).unwrap_err();

    assert_eq!(error.kind, SourceAdapterErrorKind::ProjectionAmbiguous);
}
```

- [ ] **Step 2: Run decoder tests and confirm the decoder is missing**

Run:

```bash
cargo test -p unica-coder source_adapters::platform_xml::decoder::tests -- --nocapture
```

Expected: compilation fails because the decoder and native model do not exist.

- [ ] **Step 3: Define the native model**

Use source-family-specific native types:

```rust
pub(crate) struct PlatformXmlNativeSnapshot {
    pub(crate) source: SourceSnapshot,
    pub(crate) root: NativeMetadataObject,
    pub(crate) coverage: CoverageState,
}

pub(crate) struct NativeMetadataObject {
    pub(crate) class: NativeMetadataClass,
    pub(crate) uuid: Option<Uuid>,
    pub(crate) name: String,
    pub(crate) attributes: Vec<NativeNamedChild>,
    pub(crate) tabular_sections: Vec<NativeNamedChild>,
    pub(crate) commands: Vec<NativeNamedChild>,
    pub(crate) forms: Vec<NativeForm>,
    pub(crate) templates: Vec<NativeTemplate>,
    pub(crate) properties: BTreeMap<String, NativeProperty>,
}

pub(crate) struct NativeNamedChild {
    pub(crate) kind: NativeChildKind,
    pub(crate) uuid: Option<Uuid>,
    pub(crate) name: String,
}

pub(crate) struct NativeProperty {
    pub(crate) canonical_id: String,
    pub(crate) value: NativePropertyValue,
    pub(crate) provenance: NativePropertyProvenance,
}
```

`NativeForm` contains registration evidence, descriptor evidence, and validated
managed-form content state. `NativeTemplate` contains registration evidence,
descriptor type, canonical content evidence, and detected MXL root kind.

- [ ] **Step 4: Move and harden decoder logic**

Move, rather than copy, the PR #210 root, descriptor, Form, Template, and MXL
decoding logic currently centered around `meta.rs:4616`, `meta.rs:4933`, and
`meta.rs:5022`.

The decoder entrypoint is:

```rust
pub(crate) fn decode(
    provider: &PlatformXmlProvider,
    descriptor: &SourceDescriptor,
) -> Result<PlatformXmlNativeSnapshot, SourceAdapterError>
```

Add one helper for all named siblings:

```rust
fn collect_unique_named_children(
    nodes: impl Iterator<Item = roxmltree::Node<'_, '_>>,
    kind: NativeChildKind,
) -> Result<Vec<NativeNamedChild>, SourceAdapterError>
```

It must:

1. require exactly one unambiguous `Properties/Name`;
2. validate the 1C identifier before constructing any relative path;
3. reject duplicate names within the same owner and kind;
4. preserve a valid native UUID when present;
5. return `IdentityCollision`, `DecodeCorrupted`, or `ProjectionAmbiguous`
   instead of selecting the first matching value.

Keep the existing fail-closed Form and MXL structural checks. Continue requiring
the single canonical `Ext/Template.xml` for `SpreadsheetDocument`.

Decode scalar metadata properties into `NativeProperty` without formatting
them as display text. Preserve explicit, absent, and unresolved state. Apply
version-profile defaults only in the projector, where exact 2.20 semantics are
known.

- [ ] **Step 5: Move relevant tests from `meta.rs`**

Move the existing tests for:

- root namespace and class identity;
- filename/name correspondence;
- conflicting names and template types;
- Form descriptor registration;
- managed Form XML root;
- MXL roots and canonical content;
- traversal rejection;
- symlinks and non-regular files.

Delete the moved copies from `meta.rs`; do not retain duplicate suites.

- [ ] **Step 6: Run decoder and legacy meta tests**

Run:

```bash
cargo test -p unica-coder source_adapters::platform_xml::decoder::tests -- --nocapture
cargo test -p unica-coder native_operations::meta::tests -- --nocapture
```

Expected: decoder tests pass and obsolete legacy text-analysis tests have been
removed or replaced by decoder contract tests.

- [ ] **Step 7: Commit the decoder extraction**

```bash
git add crates/unica-coder/src/infrastructure/source_adapters/platform_xml \
  crates/unica-coder/src/infrastructure/native_operations/meta.rs
git commit -m "refactor: extract Platform XML native decoder"
```

---

### Task 6: Make support-state parsing fully fail-closed

**Files:**
- Create: `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/support.rs`
- Modify: `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/common.rs:1740`
- Modify: `crates/unica-coder/src/infrastructure/support_guard.rs:117`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta.rs:5232`
- Test: inline tests in `platform_xml/support.rs` and existing support-guard tests

**Interfaces:**
- Consumes: bounded `ParentConfigurations.bin` bytes.
- Produces:
  - `SupportFacts`
  - `SupportSourceState`
  - `parse_parent_configurations(&[u8])`
  - `read_support_facts(&Path)`

- [ ] **Step 1: Add failing malformed-body regression tests**

```rust
#[test]
fn valid_header_with_garbage_body_is_unreadable() {
    let facts = parse_parent_configurations(b"{6,0,1,garbage}");

    assert!(matches!(
        facts.source,
        SupportSourceState::Unreadable { .. }
    ));
}

#[test]
fn truncated_object_rule_count_is_unreadable() {
    let facts = parse_parent_configurations(b"{6,0,2,{1,0}}");

    assert!(matches!(
        facts.source,
        SupportSourceState::Unreadable { .. }
    ));
}

#[test]
fn unknown_object_rule_blocks_authorability() {
    let facts = parsed_fixture_with_unknown_rule("Document.Order");

    assert_eq!(
        facts.authorability_for("Document.Order"),
        Authorability::UnknownSupportState
    );
}
```

- [ ] **Step 2: Run focused support tests and confirm the regression**

Run:

```bash
cargo test -p unica-coder support::tests -- --nocapture
```

Expected: the garbage-body case fails because the current parser accepts the
header and silently produces an empty rule map.

- [ ] **Step 3: Implement one strict parser shared by discovery and mutation**

Define:

```rust
pub(crate) enum SupportSourceState {
    Absent,
    Removed,
    Parsed,
    Unreadable { reason: String },
}

pub(crate) struct SupportFacts {
    pub(crate) source: SupportSourceState,
    pub(crate) object_rules: BTreeMap<String, SupportRule>,
}
```

The parser must consume the complete bounded input, validate declared counts,
reject trailing tokens, reject duplicate rules, and return `Unreadable` for
unknown grammar. It must never return `Parsed` after a partial parse.

Preserve these established meanings:

- missing support file: source is unsupported;
- empty or whitespace support file: support was removed;
- directory, symlink, unreadable file, malformed header, malformed body, or
  trailing data: unreadable and blocking under the default policy.

- [ ] **Step 4: Replace both old consumers**

Remove the permissive parser from `native_operations/common.rs`. Make both:

- semantic projection in `meta.rs`;
- mutation guard in `support_guard.rs`;

consume `platform_xml::support::SupportFacts`.

The default support guard remains `Deny` for missing/invalid project policy.
Explicit `warn` and `off` alter mutation enforcement but do not rewrite
semantic support facts as authorable.

- [ ] **Step 5: Run support, common, and meta tests**

Run:

```bash
cargo test -p unica-coder source_adapters::platform_xml::support::tests -- --nocapture
cargo test -p unica-coder support_guard::tests -- --nocapture
cargo test -p unica-coder native_operations::common::tests -- --nocapture
cargo test -p unica-coder native_operations::meta::tests -- --nocapture
```

Expected: all support-state cases pass and malformed bodies remain blocking.

- [ ] **Step 6: Commit the shared strict support reader**

```bash
git add crates/unica-coder/src/infrastructure/source_adapters/platform_xml/support.rs \
  crates/unica-coder/src/infrastructure/source_adapters/platform_xml/mod.rs \
  crates/unica-coder/src/infrastructure/native_operations/common.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta.rs \
  crates/unica-coder/src/infrastructure/support_guard.rs
git commit -m "fix: parse Platform XML support state strictly"
```

---

### Task 7: Project Platform XML into the semantic graph

**Files:**
- Create: `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/projector.rs`
- Modify: `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/source_adapters/registry.rs`
- Modify: `crates/unica-coder/src/domain/navigation.rs:178`
- Test: inline tests in `platform_xml/projector.rs`

**Interfaces:**
- Consumes:
  - `PlatformXmlNativeSnapshot`
  - `SupportFacts`
  - versioned navigation types
- Produces:
  - `project(&PlatformXmlNativeSnapshot, &SupportFacts)`
  - registered `PlatformXmlReadAdapter`

- [ ] **Step 1: Add failing projection invariants**

```rust
#[test]
fn root_metadata_object_has_an_owning_relation() {
    let envelope = project_fixture(document_fixture()).unwrap();
    let document = envelope.node_named(NodeKind::Document, "Order").unwrap();
    let owning = envelope.owning_relation(&document.object_ref).unwrap();

    assert_eq!(owning.kind, RelationKind::Contains);
    assert_eq!(
        envelope.node(&owning.source).unwrap().kind,
        NodeKind::SourceRoot
    );
}

#[test]
fn serialized_graph_contains_no_physical_paths() {
    let envelope = project_fixture(document_fixture()).unwrap();
    let text = serde_json::to_string(&envelope).unwrap();

    assert!(!text.contains("/tmp/"));
    assert!(!text.contains("\\\\Users\\\\"));
    assert!(!text.contains("Ext/Template.xml"));
}

#[test]
fn no_writer_means_mutations_are_modeled_not_executable() {
    let envelope = project_fixture(document_fixture()).unwrap();
    let clone = envelope
        .action(ActionKind::Clone, "Order")
        .unwrap();

    assert_eq!(clone.availability, ActionAvailability::Modeled);
    assert!(clone.operation_binding.is_none());
    assert!(clone.owning_relation.is_some());
}

#[test]
fn format_compatibility_is_part_of_every_node_capability() {
    let envelope = project_fixture(document_fixture()).unwrap();

    assert!(envelope
        .nodes
        .iter()
        .all(|node| node.capability.format == FormatCompatibility::Compatible));
}

#[test]
fn document_properties_are_typed_for_ai_consumption() {
    let envelope = project_fixture(document_fixture()).unwrap();
    let document = envelope
        .node_named(NodeKind::Document, "Order")
        .unwrap();

    assert_eq!(
        document.properties["numberLength"].value_type,
        PropertyType::Integer
    );
    assert_eq!(
        document.properties["numberLength"].value,
        Some(PropertyValue::Integer(11))
    );
    assert_eq!(
        document.properties["numberLength"].value_state,
        PropertyValueState::Explicit
    );
}

#[test]
fn one_c_type_descriptions_are_structured_not_strings() {
    let envelope = project_fixture(attribute_fixture()).unwrap();
    let attribute = envelope
        .node_named(NodeKind::Attribute, "Product")
        .unwrap();

    let PropertyValue::TypeSet(type_set) =
        attribute.properties["dataType"].value.clone().unwrap()
    else {
        panic!("expected structured type set");
    };
    assert_eq!(
        type_set.variants[0],
        TypeVariant::Reference {
            target: "Catalog.Products".to_string(),
        }
    );
}
```

- [ ] **Step 2: Run projector tests and confirm the module is missing**

Run:

```bash
cargo test -p unica-coder source_adapters::platform_xml::projector::tests -- --nocapture
```

Expected: compilation fails because the projector does not exist.

- [ ] **Step 3: Implement deterministic semantic keys**

Use persistent UUIDs when available:

```rust
fn object_key(
    source_id: &SourceId,
    owner: Option<&ObjectKey>,
    kind: NodeKind,
    native_uuid: Option<Uuid>,
    validated_name: &str,
) -> (ObjectKey, IdentityStrength)
```

When `native_uuid` exists, return `uuid:<uuid>` with
`IdentityStrength::Persistent`. Otherwise hash:

```text
sourceId NUL ownerKey NUL canonicalKind NUL validatedName
```

and return `derived:sha256:<digest>` with `IdentityStrength::Derived`.

Create relation keys by hashing source ID, owner key, relation kind, and target
key. Reject duplicate generated object or relation keys before graph
serialization.

- [ ] **Step 4: Build the graph without path leakage**

`project` creates:

1. one `SourceRoot` node;
2. one root metadata-object node;
3. a `contains` relation from source root to metadata object;
4. nodes and `contains` relations for attributes, tabular sections, commands,
   forms, and templates;
5. MXL profile only from validated descriptor and content evidence;
6. inspection-only profiles for unknown metadata classes and non-MXL templates.

Form internals are not projected by this plan. Form nodes report partial facet
coverage and cannot advertise FormElement, move, bind, or handler actions.

Project every known scalar property into `SemanticProperty`. Project 1C type
descriptions as `TypeSetValue` with structured primitive qualifiers and
metadata-reference targets. Never expose a raw XML type-expression string as
the canonical value.

Set each capability from:

- decoder resolution;
- identity strength;
- snapshot consistency;
- declared coverage;
- exact 2.20 compatibility;
- read-only manifest;
- strict support facts;
- absence of an operation binding.

- [ ] **Step 5: Register the built-in Platform XML reader**

Implement:

```rust
pub(crate) struct PlatformXmlReadAdapter;

impl SourceReadAdapter for PlatformXmlReadAdapter {
    fn manifest(&self) -> &AdapterManifest;

    fn inspect(
        &self,
        input: &SourceInput,
        descriptor: &SourceDescriptor,
    ) -> Result<NavigationEnvelope, SourceAdapterError> {
        let provider = PlatformXmlProvider::open(input)?;
        let native = decoder::decode(&provider, descriptor)?;
        let support = support::read_support_facts(native.support_path());
        projector::project(&native, &support)
    }
}
```

`BuiltInSourceAdapterRegistry::new()` registers exactly:

- `PlatformXmlProbe`;
- `PlatformXmlReadAdapter`.

- [ ] **Step 6: Run projector and registry tests**

Run:

```bash
cargo test -p unica-coder source_adapters::platform_xml::projector::tests -- --nocapture
cargo test -p unica-coder source_adapters::registry::tests -- --nocapture
```

Expected: projection invariants and built-in selection pass.

- [ ] **Step 7: Commit semantic projection**

```bash
git add crates/unica-coder/src/domain/navigation.rs \
  crates/unica-coder/src/infrastructure/source_adapters
git commit -m "feat: project Platform XML semantic navigation"
```

---

### Task 8: Replace `unica.meta.info` with the typed navigation contract

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta.rs:4616`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/typed_result.rs:19`
- Modify: `crates/unica-coder/src/application/mod.rs:3553`
- Modify: `crates/unica-coder/src/application/operation_descriptors.rs:96`
- Modify: `plugins/unica/skills/meta-info/SKILL.md`
- Test: existing inline tests in those modules

**Interfaces:**
- Consumes:
  - `BuiltInSourceAdapterRegistry::inspect`
  - `NavigationEnvelope`
- Produces:
  - `data.navigation.schemaVersion = "1"`
  - explicit ready/unavailable navigation status
  - no legacy `stdout`
  - typed properties
  - `ObjectPath`, `objectRef + snapshotRevision`, or `cursor` target modes
  - semantic `select` and cursor pagination
  - no legacy mode, drill-down, offset pagination, or output-file inputs

- [ ] **Step 1: Add failing public-envelope tests**

```rust
#[test]
fn meta_info_returns_ready_navigation_for_platform_xml_2_20() {
    let result = invoke_meta_info(platform_xml_2_20_fixture()).unwrap();

    assert!(result.adapter.ok);
    assert!(result.adapter.stdout.is_none());
    assert_eq!(result.data["navigation"]["schemaVersion"], "1");
    assert_eq!(result.data["navigation"]["status"], "ready");
    assert!(result.data["navigation"]["root"].is_object());
    assert!(result.data["navigation"]["nodes"].is_array());
    assert!(result.data["navigation"]["relations"].is_array());
}

#[test]
fn unsupported_version_returns_only_navigation_unavailability() {
    let result = invoke_meta_info(platform_xml_2_19_fixture()).unwrap();

    assert!(result.adapter.ok);
    assert!(result.adapter.stdout.is_none());
    assert_eq!(result.data["navigation"]["status"], "unavailable");
    assert_eq!(
        result.data["navigation"]["diagnostics"][0]["code"],
        "format_unsupported"
    );
    assert!(result.data["navigation"]["graph"].is_null());
}

#[test]
fn project_map_failure_is_not_replaced_with_ad_hoc_identity() {
    let result = invoke_meta_info_with_invalid_project_map().unwrap();

    assert_eq!(result.data["navigation"]["status"], "unavailable");
    assert_eq!(
        result.data["navigation"]["diagnostics"][0]["code"],
        "source_unavailable"
    );
    assert!(!result.data.to_string().contains("ad-hoc:"));
}

#[test]
fn meta_info_schema_has_no_legacy_text_controls() {
    let schema = input_schema_for_tool(tool("unica.meta.info"));
    let properties = schema["properties"].as_object().unwrap();

    for removed in ["Mode", "Name", "Limit", "Offset", "OutFile"] {
        assert!(!properties.contains_key(removed), "{removed} must be removed");
    }
}

#[test]
fn meta_info_schema_supports_semantic_lazy_navigation() {
    let schema = input_schema_for_tool(tool("unica.meta.info"));
    let properties = schema["properties"].as_object().unwrap();

    for field in ["ObjectPath", "objectRef", "snapshotRevision", "select", "cursor"] {
        assert!(properties.contains_key(field), "{field} must be present");
    }
}

#[test]
fn meta_info_relation_cursor_returns_the_next_snapshot_bound_page() {
    let first = invoke_meta_info_with_relation_page_size(
        platform_xml_2_20_fixture(),
        "attributes",
        1,
    )
    .unwrap();
    let cursor = first.data["navigation"]["relations"][0]["nextCursor"].clone();

    let second = invoke_meta_info_cursor(cursor).unwrap();

    assert_eq!(second.data["navigation"]["status"], "ready");
    assert_ne!(
        first.data["navigation"]["relations"][0]["items"][0],
        second.data["navigation"]["relations"][0]["items"][0]
    );
}

#[test]
fn meta_info_dry_run_uses_the_same_typed_read_contract() {
    let result = invoke_meta_info_dry_run(platform_xml_2_20_fixture()).unwrap();

    assert!(result.adapter.stdout.is_none());
    assert_eq!(result.data["navigation"]["status"], "ready");
}

#[test]
fn meta_info_skill_describes_typed_navigation_only() {
    let skill =
        include_str!("../../../../plugins/unica/skills/meta-info/SKILL.md");

    assert!(skill.contains("data.navigation"));
    assert!(!skill.contains("stdout"));
    for removed in ["`Mode`", "`Name`", "`Limit`", "`Offset`", "`OutFile`"] {
        assert!(!skill.contains(removed), "{removed} must be removed");
    }
}
```

- [ ] **Step 2: Run focused integration tests and confirm current behavior fails**

Run:

```bash
cargo test -p unica-coder meta_info_returns_ready_navigation -- --nocapture
cargo test -p unica-coder unsupported_version_returns_only -- --nocapture
cargo test -p unica-coder project_map_failure_is_not_replaced -- --nocapture
cargo test -p unica-coder meta_info_schema_has_no_legacy -- --nocapture
cargo test -p unica-coder meta_info_schema_supports_semantic -- --nocapture
cargo test -p unica-coder meta_info_relation_cursor -- --nocapture
cargo test -p unica-coder meta_info_dry_run_uses_the_same -- --nocapture
```

Expected: tests fail because `meta.info` still calls the prototype analyzer and
still exposes the legacy text contract.

- [ ] **Step 3: Delete the legacy analyzer and expose one registry facade**

Replace the legacy analysis entrypoint in `meta.rs` with:

```rust
pub(crate) fn inspect_meta_navigation(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<NavigationEnvelope, String>
```

Its flow is:

1. parse exactly one of `ObjectPath`, `objectRef + snapshotRevision`, or
   `cursor`;
2. normalize `select` and enforce relation page size 1 through 100;
3. resolve or resume the target inside the bound snapshot;
4. construct `SourceInput`;
5. call `BuiltInSourceAdapterRegistry::new().inspect(input)`;
6. return a ready or unavailable envelope;
7. convert corruption, ambiguity, stale cursor, and source-map failures into structured
   unavailable diagnostics without inventing an identity.

Delete the old text rendering, `Mode`, `Name`, pagination, output-file, and
drill-down branches. Also remove graph-construction, descriptor, support, and
source-scope helpers that Tasks 5 through 7 moved out of `meta.rs`.

- [ ] **Step 4: Return only typed navigation data**

In `typed_result.rs`, keep the operation gate:

```rust
if operation == "meta-info" && !mutating {
    let navigation = meta::inspect_meta_navigation(args, context)?;
    return Ok(TypedNativeOperationResult {
        adapter: AdapterOutcome::ok("semantic metadata navigation inspected"),
        data: Some(json!({ "navigation": navigation })),
    });
}
```

Do not populate `stdout`, flatten graph fields into `data`, or omit diagnostics
for an unavailable adapter. `dryRun` does not alter this read-only operation;
it must use the same typed path rather than falling through to the generic
native-operation preview.

- [ ] **Step 5: Remove legacy input controls from the public schema**

Remove `Mode`, `Name`, `Limit`, `Offset`, and `OutFile` from the `meta.info`
tool definition in `application/mod.rs`. Add `objectRef`,
`snapshotRevision`, `select`, and `cursor`. The JSON Schema uses `oneOf` so a
request selects exactly one target mode.

Change the operation descriptor to:

```rust
descriptor(
    "meta-info",
    OBJECT_PATH_REQUIRED,
    EMPTY,
    OBJECT_PATH,
    None,
),
```

Delete tests dedicated only to removed arguments or text formatting.
`ObjectPath` bootstraps a source, `objectRef + snapshotRevision` expands a known
node, and `cursor` resumes a relation page. Do not add a new MCP tool.

- [ ] **Step 6: Rewrite the packaged `meta-info` skill**

Update `plugins/unica/skills/meta-info/SKILL.md` so it:

- describes typed semantic navigation rather than compact text;
- documents `ObjectPath`, `objectRef + snapshotRevision`, `select`, and
  `cursor`;
- explains `schemaVersion`, `status`, `snapshot`, `graph`, and `diagnostics`;
- explains typed properties, value states, structured 1C type sets, relation
  pages, and stale cursors;
- explains node identity, relations, capability availability, and blocking
  reasons;
- contains no `Mode`, `Name`, `Limit`, `Offset`, or `OutFile` examples;
- does not instruct the model to read `stdout`;
- remains MCP-first through `unica.meta.info`.

- [ ] **Step 7: Run meta, typed-result, application, and skill-contract tests**

Run:

```bash
cargo test -p unica-coder native_operations::meta::tests -- --nocapture
cargo test -p unica-coder native_operations::typed_result::tests -- --nocapture
cargo test -p unica-coder application::tests -- --nocapture
cargo test -p unica-coder meta_info_skill -- --nocapture
```

Expected: all tests pass; no test expects legacy text or removed inputs.

- [ ] **Step 8: Commit the breaking `meta.info` contract**

```bash
git add crates/unica-coder/src/application/mod.rs \
  crates/unica-coder/src/application/operation_descriptors.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta.rs \
  crates/unica-coder/src/infrastructure/native_operations/typed_result.rs \
  plugins/unica/skills/meta-info/SKILL.md
git commit -m "feat!: replace meta info text with navigation"
```

---

### Task 9: Reusable read-adapter certification and final verification

**Files:**
- Create: `crates/unica-coder/src/infrastructure/source_adapters/certification.rs`
- Modify: `crates/unica-coder/src/infrastructure/source_adapters/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/mod.rs`
- Test: inline certification tests

**Interfaces:**
- Consumes:
  - `SourceProbe`
  - `SourceReadAdapter`
  - `AdapterManifest`
- Produces:
  - `certify_read_adapter`
  - Platform XML 2.20 `ReadCompatible` maturity evidence for the declared
    object-level navigation feature set.

- [ ] **Step 1: Write the reusable certification contract**

```rust
#[cfg(test)]
pub(crate) struct ReadAdapterCertificationCase {
    pub(crate) supported: SourceInput,
    pub(crate) unsupported_version: SourceInput,
    pub(crate) corrupted: SourceInput,
    pub(crate) expected_adapter_id: &'static str,
}

#[cfg(test)]
pub(crate) fn certify_read_adapter(
    registry: &BuiltInSourceAdapterRegistry,
    case: ReadAdapterCertificationCase,
) {
    let ready = registry.inspect(case.supported).unwrap();
    assert_eq!(ready.status, NavigationStatus::Ready);
    assert_eq!(
        ready.snapshot.unwrap().adapter_id,
        case.expected_adapter_id
    );

    let unsupported = registry.inspect(case.unsupported_version).unwrap();
    assert_eq!(unsupported.status, NavigationStatus::Unavailable);
    assert_eq!(unsupported.diagnostics[0].code, "format_unsupported");

    let corrupted = registry.inspect(case.corrupted).unwrap_err();
    assert_eq!(corrupted.kind, SourceAdapterErrorKind::DecodeCorrupted);
}
```

- [ ] **Step 2: Add Platform XML certification fixtures**

The Platform XML certification module creates bounded temporary fixtures for:

- supported 2.20 Document with attribute, tabular section, command, Form, and
  MXL template;
- unsupported 2.19 equivalent;
- malformed root XML;
- duplicate child identity;
- conflicting Form descriptor;
- non-canonical MXL content;
- unreadable support state;
- invalid project source map.

Use `std::env::temp_dir`, process ID, timestamp, and an atomic nonce, matching
the repository's existing fixture convention. Do not add a new temp-directory
dependency.

- [ ] **Step 3: Run certification and focused regression suites**

Run:

```bash
cargo test -p unica-coder source_adapters::certification -- --nocapture
cargo test -p unica-coder source_adapters::platform_xml -- --nocapture
cargo test -p unica-coder native_operations::meta::tests -- --nocapture
cargo test -p unica-coder support_guard::tests -- --nocapture
```

Expected: all certification and regression tests pass.

- [ ] **Step 4: Promote only the proven manifest maturity**

Change the Platform XML manifest maturity from `ProbeComplete` to
`ReadCompatible` only after Step 3 passes.

Declare the certified read capability as object-level navigation. Do not claim
form-content coverage, semantic parity across source families, or write safety.

- [ ] **Step 5: Run repository-level verification**

Run:

```bash
cargo fmt --all -- --check
git diff --check
cargo test -p unica-coder
cargo build -p unica-coder
```

Expected:

- formatting check succeeds;
- no whitespace errors;
- all `unica-coder` unit and integration tests pass;
- `unica-coder` builds successfully.

- [ ] **Step 6: Confirm package-contract files are untouched**

Run:

```bash
git diff --name-only 0aacaaf3..HEAD -- \
  plugins/unica/.mcp.json \
  plugins/unica/.codex-plugin/plugin.json \
  plugins/unica/third-party/tools.lock.json
```

Expected: no output.

- [ ] **Step 7: Commit certification**

```bash
git add crates/unica-coder/src/infrastructure/source_adapters
git commit -m "test: certify Platform XML read adapter"
```

---

## Completion Criteria

The plan is complete when:

1. Platform XML 2.20 is selected through evidence and a deterministic registry.
2. Platform XML 2.19 is never routed to the 2.20 reader.
3. Unsupported versions expose structured navigation unavailability without
   invoking a legacy analyzer.
4. Public semantic references contain no physical paths.
5. Duplicate child identities fail closed.
6. Root clone discovery names an owning relation.
7. Format compatibility participates in capability evaluation.
8. No mutation action is executable because this plan supplies no writer.
9. Malformed support bodies block authorability and mutation guards.
10. Source-map discovery errors cannot become ad-hoc identities.
11. `meta.info` emits no legacy text and exposes none of the removed
    `Mode`, `Name`, `Limit`, `Offset`, or `OutFile` inputs.
12. The packaged `meta-info` skill describes only the typed navigation
    contract.
13. Every canonical property contains an explicit type and value state.
14. 1C type descriptions are structured JSON values, not display strings.
15. Large relations use snapshot-bound cursor pagination with a maximum page
    size of 100.
16. Platform XML read certification, the full `unica-coder` test suite, and the
    crate build pass.

## Follow-up Plan Boundaries

After this plan is complete, create separate implementation plans in this
order:

1. Platform XML form-content projector and relation/reference parity.
2. First specialized Platform XML writer with staging and recovery.
3. EDT version-family read adapters.
4. Binary CF read adapters.
5. File-database snapshot provider and read adapters.
6. External adapter process protocol after at least two source families prove
   the internal contracts.
