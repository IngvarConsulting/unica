"""The `apply` registry and the scenario registry document read from the corpus.

Split out of `test_acceptance_scenarios.py`, whose helpers it uses.
"""

# The helpers stay in `test_acceptance_scenarios`: tests elsewhere patch them
# there by module path, so they keep one home.
from tests.ci.test_acceptance_scenarios import *  # noqa: F401,F403
# Their own tests run in their own module, not again here.
del AcceptanceCorpusRunTests, AcceptanceCorpusScheduleTests  # noqa: F821

# Размер тестов модуля (scripts/ci/python_sizes.py).
PR_SKIP = "малым тестам нужен продукт"


class AcceptanceRegistryDocumentTests(unittest.TestCase):
    """The scenario registry is rendered from the corpus when the site is
    built; it is not committed, so two added scenarios never edit its counts
    and coverage lists in the same lines."""

    METADATA_CONTEXT = declared_variants("Metadata")
    FORM_CONTEXT = declared_variants("Form")
    CONTEXT_ROWS = [
        (name, "Metadata", applicability, skeleton)
        for _, name, applicability, skeleton in METADATA_CONTEXT
    ] + [
        (name, "Form", applicability, skeleton)
        for _, name, applicability, skeleton in FORM_CONTEXT
    ]
    CONTEXT_REGISTRATIONS = "".join(
        f"(unica_metadata::apply_operations::MetadataOperation::{variant})," for variant, _, _, _ in METADATA_CONTEXT
    ) + "".join(
        f"(FormOperation::{variant})," for variant, _, _, _ in FORM_CONTEXT
    )

    MXL_EVENT_ROWS = declared_rows("Mxl", "Event")
    MXL_EVENT_REGISTRATIONS = (
        declared_registrations("MxlOperation", "Mxl") + declared_registrations("EventOperation", "Event")
    )

    DCS_FIXTURE_METADATA = '''dcs_operations!(
        DataSourceAdd => ("dataSource.add", DcsSchema, Items, Vec<DataSourceSpec>),
        QuerySet => ("query.set", DcsQuery, Values, String),
        DataSetRemove => ("dataSet.remove", DcsData, Target, ()),
    );'''
    DCS_FIXTURE_REGISTRATIONS = "(Primitive::DataSourceAdd), (Primitive::QuerySet), (Primitive::DataSetRemove),"
    DCS_FIXTURE_ROWS = [
        ("dataSource.add", "Dcs", "DcsSchema", "Items"),
        ("query.set", "Dcs", "DcsQuery", "Values"),
        ("dataSet.remove", "Dcs", "DcsData", "Target"),
    ]
    PROPERTIES_ROWS = declared_rows("Properties")
    XDTO_ROWS = declared_rows("Xdto")
    XDTO_REGISTRATIONS = declared_registrations("XdtoOperation", "Xdto")
    ROLE_REGISTRATIONS = declared_registrations("RoleOperation", "Role")
    # Runs of one family in the composition table of `apply.rs`, in order.
    FAMILY_RUNS = [
        "Metadata", "Properties", "Metadata", "Form", "Metadata", "Form", "Role", "Dcs", "Mxl",
        "Xdto", "Subsystem", "Support", "Interface", "Code", "Event",
    ]

    def renderer(self):
        module_path = REPO_ROOT / "scripts" / "ci" / "render-acceptance-registry.py"
        spec = importlib.util.spec_from_file_location("render_acceptance_registry", module_path)
        self.assertIsNotNone(spec)
        assert spec is not None and spec.loader is not None
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def family_source(self, family):
        return self.renderer().FAMILY_SOURCES[family].read_text(encoding="utf-8")

    def test_registry_document_is_rendered_from_the_corpus(self) -> None:
        with patch("subprocess.Popen", side_effect=AssertionError("registry rendering must not launch Cargo or the product")):
            module = self.renderer()
            rendered = module.render_document()
        self.assertIn("## Покрытие реестра `apply`", rendered)
        corpus = load_acceptance_corpus()
        self.assertIn(f"**{len(corpus['scenarios'])} сценариев**", rendered)
        for scenario in corpus["scenarios"]:
            self.assertIn(f"\n| {scenario['id']} | ", rendered)
        page = module.render_html(rendered)
        self.assertIn('id="S047"', page)
        self.assertIn("описан в <code>tests/fixtures/acceptance/README.md</code>", page)

    def test_role_rows_keep_their_arguments_order_and_implemented_status(self) -> None:
        module = self.renderer()
        registry, implemented = module.load_apply_registry()
        self.assertEqual([entry for entry in registry if entry[1] == "Role"], declared_rows("Role"))
        rendered = module.render_document()
        for operation, _, _, skeleton in declared_rows("Role"):
            args = skeleton.lower()
            self.assertIn(operation, implemented)
            self.assertIn(f"| `{operation}` | `{args}` | ", rendered)

    SOURCE_ROWS = declared_rows("Code", "Subsystem", "Interface")
    SOURCE_REGISTRATIONS = (
        declared_registrations("CodeOperation", "Code")
        + declared_registrations("SubsystemOperation", "Subsystem")
        + declared_registrations("InterfaceOperation", "Interface")
    )

    def test_source_operations_fail_closed_when_typed_registration_is_incomplete(self):
        metadata = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        roles = self.ROLE_REGISTRATIONS
        table = roles + self.SOURCE_REGISTRATIONS
        registry, _ = self.registration_fixture(table, metadata, complete_source=False)
        self.assertEqual(
            len([entry for entry in registry if entry[1] in {"Code", "Subsystem", "Interface"}]),
            len(self.SOURCE_ROWS),
        )
        for typed, raw in [
            ("(CodeOperation::Insert)", '("code.insert", Code, CodeInsert, Text)'),
            ("(SubsystemOperation::Create)", '("subsystem.create", Subsystem, Configuration, Values)'),
            ("(InterfaceOperation::VisibilitySet)", '("commandVisibility.set", Interface, Interface, Items)'),
        ]:
            for broken in [table.replace(typed + ",", ""), table + typed + ",", table.replace(typed, raw), table.replace(typed, typed.replace("::", "::Unknown"))]:
                with self.subTest(typed=typed, broken=broken), self.assertRaises(RuntimeError):
                    self.registration_fixture(broken, metadata, complete_source=False)

    def registration_fixture(self, table, metadata, additional_metadata=None, *, complete_support=True, complete_context=True, complete_mxl_event=True, complete_dcs=True, complete_properties=True, complete_xdto=True, complete_source=True):
        # A family-focused fixture keeps unrelated typed families complete.
        module = self.renderer()
        families = dict(module.TYPED_FAMILIES)
        sources = dict(additional_metadata or {})
        sources.setdefault("dcs_operation.rs", self.DCS_FIXTURE_METADATA)
        for family in ["support", "metadata", "form", "mxl", "event", "properties", "xdto", "code", "subsystem", "interface"]:
            filename = f"{family}_operation.rs"
            if filename not in sources:
                sources[filename] = self.family_source(family.title())
        if complete_support:
            table += "\n(SupportOperation::CapabilitySet), (SupportOperation::RuleSet),"
        if complete_context:
            table += "\n" + self.CONTEXT_REGISTRATIONS
        if complete_mxl_event:
            table += "\n" + self.MXL_EVENT_REGISTRATIONS
        if complete_dcs:
            table += "\n" + self.DCS_FIXTURE_REGISTRATIONS
        if complete_properties:
            table += "\n(PropertiesOperation::Set),"
        if complete_xdto:
            table += "\n" + self.XDTO_REGISTRATIONS
        if complete_source:
            table += "\n" + self.SOURCE_REGISTRATIONS
        module = self.renderer()
        with tempfile.TemporaryDirectory(prefix="typed-registry-") as raw:
            root = Path(raw)
            registry = root / "apply.rs"
            registry.write_text(
                "\noperation_descriptors!(\n" + table + "\n);\n",
                encoding="utf-8",
            )
            if metadata is not None:
                (root / "role_operation.rs").write_text(metadata, encoding="utf-8")
            for filename, source in sources.items():
                if source is not None:
                    (root / filename).write_text(source, encoding="utf-8")
            paths = {family: root / f"{family.lower()}_operation.rs" for family in families}
            with patch.object(module, "APPLY_REGISTRY", registry), \
                    patch.object(module, "TYPED_FAMILIES", families), \
                    patch.object(module, "FAMILY_SOURCES", paths):
                return module.load_apply_registry()

    def product_registration_fixture(self, source, changed_metadata=None):
        module = self.renderer()
        with tempfile.TemporaryDirectory(prefix="product-registry-") as raw:
            root = Path(raw)
            registry = root / "apply.rs"
            registry.write_text(source, encoding="utf-8")
            paths = dict(module.FAMILY_SOURCES)
            for family, metadata in (changed_metadata or {}).items():
                paths[family] = root / f"{family.lower()}.rs"
                if metadata is not None:
                    paths[family].write_text(metadata, encoding="utf-8")
            with patch.object(module, "APPLY_REGISTRY", registry), \
                    patch.object(module, "FAMILY_SOURCES", paths):
                return module.load_apply_registry()

    def test_all_product_families_read_their_physical_owner_tables(self) -> None:
        module = self.renderer()
        self.assertEqual(set(module.TYPED_FAMILIES), set(module.FAMILY_SOURCES))
        self.assertEqual(len(module.TYPED_FAMILIES), 12)
        registry, _ = module.load_apply_registry()
        # Only the list of operations comes from the registry; what each one
        # is comes from the hand-written declaration of its family.
        declared_families = {path.stem for path in APPLY_DECLARATIONS.glob("*.json")}
        self.assertEqual(declared_families, {family.lower() for family in module.TYPED_FAMILIES})
        declared = {(row[0], row[1]): row for family in module.TYPED_FAMILIES for row in declared_rows(family)}
        registered = {(row[0], row[1]): row for row in registry}
        self.assertEqual(len(registered), len(registry), "an operation is registered twice in one family")
        self.assertEqual(
            sorted(set(registered) - set(declared)), [],
            "a registered apply operation is not declared in tests/ci/fixtures/apply-registry/<family>.json",
        )
        self.assertEqual(
            sorted(set(declared) - set(registered)), [],
            "a declared apply operation is not registered in crates/unica-domain/src/apply.rs",
        )
        for key, row in declared.items():
            with self.subTest(operation=key):
                self.assertEqual(registered[key], row)
        for family in module.TYPED_FAMILIES:
            with self.subTest(family=family):
                self.assertEqual([row for row in registry if row[1] == family], declared_rows(family))
                # The Rust variant behind each declared operation is the one
                # its family's typed metadata names.
                self.assertEqual(
                    {variant: name for variant, name, _, _ in declared_variants(family)},
                    {variant: entry[0] for variant, entry in module._registration_metadata(family).items()},
                )
        # Where each family sits in the composition table: consecutive runs of
        # one family, an operation added inside its run leaves them as they are.
        self.assertEqual(
            [family for family, _ in itertools.groupby(row[1] for row in registry)],
            self.FAMILY_RUNS,
        )

    def test_every_registered_apply_operation_has_a_published_integration_case(self) -> None:
        # The inventory is the closed `apply` registry of all typed families
        # against the `source` profile, which `AcceptanceCorpusRunTests`
        # executes in the mandatory queue.  A newly registered operation
        # without a published and checked case fails here, not as a skip.
        with patch("subprocess.Popen", side_effect=AssertionError("the inventory must not launch a process")):
            module = self.renderer()
            registry, _ = module.load_apply_registry()
        self.assertEqual({row[1] for row in registry}, set(module.TYPED_FAMILIES))
        cases = apply_operation_cases(select_profile(load_acceptance_corpus(), "source")["scenarios"], registry)
        missing = [(operation, family) for operation, family, _, _ in registry if (operation, family) not in cases]
        self.assertEqual(
            missing, [],
            "a registered apply operation is not published with checked files by the executed acceptance "
            "corpus; add a scenario under tests/fixtures/acceptance/scenarios/",
        )

    def test_form_property_inventory_distinguishes_columns_from_metadata_and_items(self):
        registry = [("props.set", "Properties", "Metadata", "Values"),
                    ("props.set", "Form", "FormProperties", "Values")]
        def scenario(at):
            return {"id": "column", "wire": [
                {"tool":"unica.apply", "args":{"at":at,"ops":[{"op":"props.set"}]}, "expect":["ok"]},
                {"tool":"unica.apply", "args":{"executionToken":"$executionToken"}, "expect":["ok"],
                 "fileAssertions":[{"path":"Form.xml", "kind":"xml", "select":"./Type", "eq":["T"]}]}]}
        for at in ["main:Catalog.X.Form.F", "main:Catalog.X.Form.F.Attribute.Rows.Column.Value"]:
            self.assertEqual(apply_operation_cases([scenario(at)], registry), {("props.set", "Form"): ["column"]})
        self.assertEqual(apply_operation_cases([scenario("main:Catalog.X")], registry), {("props.set", "Properties"): ["column"]})
        for at in ["main:Catalog.X.Form.F.Attribute.Rows", "main:Catalog.X.Form.F.Item.Value"]:
            with self.assertRaises(ValueError):
                apply_operation_cases([scenario(at)], registry)

    def test_column_creation_inventory_distinguishes_form_data_from_visual_columns(self):
        registry = [("column.add", "Metadata", "MetadataColumns", "Items"),
                    ("column.add", "Form", "FormAttribute", "Items")]
        def scenario(at):
            return {"id":"add", "wire":[
                {"tool":"unica.apply", "args":{"at":at,"ops":[{"op":"column.add"}]}, "expect":["ok"]},
                {"tool":"unica.apply", "args":{"executionToken":"$executionToken"}, "expect":["ok"],
                 "fileAssertions":[{"path":"Form.xml", "kind":"xml", "select":"./Column", "eq":["Added"]}]}]}
        self.assertEqual(apply_operation_cases([scenario("main:Catalog.X.Form.F.Attribute.Rows")], registry), {("column.add", "Form"): ["add"]})
        self.assertEqual(apply_operation_cases([scenario("main:DocumentJournal.Orders")], registry), {("column.add", "Metadata"): ["add"]})
        for at in ["main:Catalog.X.Form.F", "main:Catalog.X.Form.F.Item.Rows", "main:Catalog.X.Form.F.Attribute.Rows.Column"]:
            with self.assertRaises(ValueError):
                apply_operation_cases([scenario(at)], registry)

    def test_operation_inventory_counts_only_published_apply_plans_of_their_family(self) -> None:
        def preview(expect, at, *operations):
            return {"tool": "unica.apply", "expect": expect, "args": {"at": at, "ops": [{"op": name} for name in operations]}}

        pinned = {"path": "src/X.xml", "kind": "xml", "select": "./{*}Name", "eq": ["X"]}
        vague = [{"path": "src/X.xml", "kind": "exists", "eq": True},
                 {"path": "src/X.xml", "kind": "sha256", "notEq": "$capture.before"},
                 {"path": "src/X.xml", "kind": "sha256", "eq": "$capture.before"}]

        def publish(expect=("ok",), checked=True, token="$executionToken", assertions=(pinned,)):
            step = {"tool": "unica.apply", "expect": list(expect), "args": {"executionToken": token}}
            if checked:
                step["fileAssertions"] = list(assertions)
            return step

        registry = [
            ("props.set", "Properties", "Metadata", "Values"),
            ("field.add", "Dcs", "DcsField", "Values"),
            ("item.remove", "Form", "FormItem", "Target"),
            ("attribute.add", "Metadata", "MetadataAttributeContainer", "Items"),
            ("attribute.add", "Form", "Form", "Items"),
            ("attribute.remove", "Metadata", "MetadataAttribute", "Target"),
            ("attribute.remove", "Form", "FormAttribute", "Target"),
        ]
        catalog, form = "main:Catalog.X", "main:Catalog.X.Form.Y"
        scenarios = [
            {"id": "A", "wire": [preview(["ok"], catalog, "props.set", "attribute.add"),
                                 preview(["refused"], catalog, "field.add"), publish()]},
            {"id": "B", "wire": [
                preview(["ok"], catalog, "field.add"),
                preview(["ok"], form, "attribute.add"), publish(),
                publish(),
                preview(["ok"], form, "item.remove"), publish(checked=False),
                preview(["ok"], form, "item.remove"), publish(token="literal"),
                preview(["ok"], form, "item.remove"), publish(assertions=vague),
            ]},
            {"id": "H", "wire": [
                preview(["ok"], catalog, "field.add"), publish(expect=("refused",)),
                publish(assertions=[{"path": "src/X.xml", "kind": "exists", "eq": False}]),
            ]},
            {"id": "G", "wire": [
                preview(["ok", "task"], catalog, "props.set"), publish(),
                preview(["gap"], catalog, "props.set"), publish(),
                {"tool": "unica.view", "expect": ["ok"], "args": {"at": catalog, "ops": [{"op": "props.set"}]}},
                publish(),
            ]},
            {"id": "C", "wire": [{"tool": "unica.apply", "expect": ["ok"], "args": {
                "at": catalog, "ops": [{"op": "attribute.add", "args": {"at": form}}, {"op": "unregistered.add"}]}},
                publish()]},
            {"id": "D", "wire": [preview(["ok"], catalog, "props.set")]},
            {"id": "F", "wire": [
                {**preview(["ok"], form, "item.remove"), "captures": {"remove": {"pointer": "/data/executionToken"}}},
                preview(["ok"], catalog, "field.add"),
                publish(token="$capture.remove"),
            ]},
        ]
        scenarios.extend([
            {"id": "J", "wire": [preview(["ok"], catalog + ".Attribute.Z", "attribute.remove"), publish()]},
            {"id": "K", "wire": [preview(["ok"], form + ".Attribute.Z", "attribute.remove"), publish()]},
        ])
        self.assertEqual(apply_operation_cases(scenarios, registry), {
            ("props.set", "Properties"): ["A"],
            ("attribute.add", "Metadata"): ["A"],
            ("attribute.add", "Form"): ["B", "C"],
            ("item.remove", "Form"): ["F"],
            ("field.add", "Dcs"): ["H"],
            ("attribute.remove", "Metadata"): ["J"],
            ("attribute.remove", "Form"): ["K"],
        })
        self.assertTrue(pins_published_file({"path": "a", "kind": "sha256", "eq": "0" * 64}))
        self.assertTrue(names_form("main:CommonForm.Main"))
        self.assertIsNone(names_form("main:Catalog.X.Form.Y.Attribute.Z"))
        self.assertIs(names_form("main:Configuration"), False)
        nested = [{"id": "E", "wire": [preview(["ok"], "main:Catalog.X.Form.Y.Attribute.Z", "attribute.add")]}]
        with self.assertRaises(ValueError):
            apply_operation_cases(nested, registry)
        nested_remove = [{"id": "L", "wire": [preview(["ok"], form + ".Attribute.Rows.Column.Z", "attribute.remove")]}]
        with self.assertRaises(ValueError):
            apply_operation_cases(nested_remove, registry)
        with self.assertRaises(ValueError):
            apply_operation_cases(scenarios, registry + [("attribute.add", "Xdto", "XdtoType", "Items")])

    def test_metadata_registration_requires_the_actual_qualified_owner(self) -> None:
        with patch("subprocess.Popen", side_effect=AssertionError("registry inspection must not launch a process")):
            module = self.renderer()
            operation = "unica_metadata::apply_operations::MetadataOperation"
            self.assertEqual(module.TYPED_FAMILIES["Metadata"], operation)
            original = module.APPLY_REGISTRY.read_text(encoding="utf-8")
            registry, _ = module.load_apply_registry()
            self.assertEqual(
                [row for row in registry if row[1] == "Metadata"],
                [row for row in self.CONTEXT_ROWS if row[1] == "Metadata"],
            )
            self.assertEqual(len(registry), 131)
            self.assertEqual(len({row[0] for row in registry}), 126)
            registration = f"({operation}::ObjectBorrow),"
            self.assertEqual(original.count(registration), 1)
            replacements = {
                "old bare alias": "(MetadataOperation::ObjectBorrow),",
                "wrong crate same enum suffix": "(other_metadata::apply_operations::MetadataOperation::ObjectBorrow),",
                "wrong module same enum suffix": "(unica_metadata::request::MetadataOperation::ObjectBorrow),",
                "unknown prefix": f"(unknown::{operation}::ObjectBorrow),",
                "unknown variant": f"({operation}::Unknown),",
                "generic path": f"({operation}::<Other>::ObjectBorrow),",
                "missing": "",
                "duplicate": registration * 2,
                "raw replacement": '("object.borrow", Metadata, ExtensionRoot, From),',
                "unconsumed entry": registration + "unexpected_registration!(),",
            }
            for case, replacement in replacements.items():
                changed = original.replace(registration, replacement, 1)
                self.assertNotEqual(changed, original, f"{case} must alter a real registration")
                with self.subTest(case=case), self.assertRaises(RuntimeError):
                    self.product_registration_fixture(changed)

    def test_code_subsystem_and_interface_tables_fail_closed_in_product_composition(self) -> None:
        module = self.renderer()
        original = module.APPLY_REGISTRY.read_text(encoding="utf-8")
        for family in ["Code", "Subsystem", "Interface"]:
            operation = module.TYPED_FAMILIES[family]
            metadata = self.family_source(family)
            table_name = f"{family.lower()}_operations"
            entries = module._registration_metadata(family)
            for variant, (name, _, applicability, skeleton) in entries.items():
                registration = f"({operation}::{variant}),"
                row = f'{variant} => ("{name}", {applicability}, {skeleton})'
                raw = f'("{name}", {family}, {applicability}, {skeleton}),'
                self.assertEqual(original.count(registration), 1)
                self.assertEqual(metadata.count(row), 1)
                cases = {
                    "missing registration": (original.replace(registration, ""), metadata),
                    "duplicate registration": (original.replace(registration, registration * 2), metadata),
                    "unknown variant": (original.replace(registration, f"({operation}::Unknown),"), metadata),
                    "wrong family": (original.replace(registration, f"(OtherOperation::{variant}),"), metadata),
                    "raw replacement": (original.replace(registration, raw), metadata),
                    "raw beside": (original.replace(registration, registration + raw), metadata),
                    "raw future name": (original.replace(registration, registration + f'("future.operation", {family}, {applicability}, {skeleton}),'), metadata),
                    "metadata missing": (original, None),
                    "metadata empty": (original, f"{table_name}!();"),
                    "metadata row missing": (original, metadata.replace(row + ",", "")),
                    "metadata incomplete": (original, metadata.replace(row, f'{variant} => ("{name}", {applicability})')),
                    "metadata nonliteral": (original, metadata.replace(row, f'{variant} => (NAME, {applicability}, {skeleton})')),
                    "metadata repeated": (original, metadata + f'\n{table_name}!(Probe => ("future.operation", {applicability}, {skeleton}),);'),
                }
                for case, (source, changed) in cases.items():
                    with self.subTest(family=family, variant=variant, case=case), self.assertRaises(RuntimeError):
                        self.product_registration_fixture(source, {family: changed})

    def test_xdto_rows_have_complete_typed_metadata_and_preserve_registry_order(self) -> None:
        role_source = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        metadata = (REPO_ROOT / "crates/unica-xdto/src/operations.rs").read_text(encoding="utf-8")
        roles = self.ROLE_REGISTRATIONS
        table = roles + self.XDTO_REGISTRATIONS
        registry, implemented = self.renderer().load_apply_registry()
        self.assertEqual([entry for entry in registry if entry[1] == "Xdto"], self.XDTO_ROWS)
        self.assertLessEqual({entry[0] for entry in self.XDTO_ROWS}, implemented)
        row = 'AddValueType => ("valueType.add", XdtoPackage, Values)'
        self.assertIn(row, metadata)
        cases = {
            "missing registration": (table.replace("(XdtoOperation::AddProperty),", ""), metadata),
            "duplicate registration": (table + "(XdtoOperation::AddProperty),", metadata),
            "unknown variant": (table.replace("XdtoOperation::AddProperty", "XdtoOperation::Unknown"), metadata),
            "raw replacement": (table.replace("(XdtoOperation::AddValueType),", '("valueType.add", Xdto, XdtoPackage, Values),'), metadata),
            "raw unknown name": (table + '("future.operation", Xdto, XdtoPackage, Values),', metadata),
            "missing metadata": (table, None),
            "empty metadata": (table, "xdto_operations!();"),
            "missing metadata row": (table, metadata.replace(row + ",", "")),
            "nonliteral name": (table, metadata.replace(row, 'AddValueType => (NAME, XdtoPackage, Values)')),
        }
        for label, (source, changed_metadata) in cases.items():
            with self.subTest(label=label), self.assertRaises(RuntimeError):
                self.registration_fixture(source, role_source, {"xdto_operation.rs": changed_metadata}, complete_xdto=False)

    def test_properties_rows_use_complete_typed_metadata_and_fail_closed(self) -> None:
        role_source = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        metadata = (REPO_ROOT / "crates/unica-metadata/src/descriptor_operations.rs").read_text(encoding="utf-8")
        roles = self.ROLE_REGISTRATIONS
        registration = "(PropertiesOperation::Set),"
        table = roles + registration
        row = 'Set => ("props.set", MetadataOrRoleOrSubsystem, Values)'
        raw = '("props.set", Properties, MetadataOrRoleOrSubsystem, Values),'
        actual, _ = self.registration_fixture(table, role_source, complete_properties=False)
        self.assertEqual([entry for entry in actual if entry[1] == "Properties"],
                         [("props.set", "Properties", "MetadataOrRoleOrSubsystem", "Values")])
        cases = {
            "missing registration": (roles, metadata),
            "duplicate registration": (table + registration, metadata),
            "unknown variant": (table.replace("PropertiesOperation::Set", "PropertiesOperation::Unknown"), metadata),
            "wrong family": (table.replace("PropertiesOperation::Set", "RoleOperation::Set"), metadata),
            "raw replacement": (roles + raw, metadata),
            "raw beside complete typed": (table + raw, metadata),
            "raw unknown name": (table + '("future.operation", Properties, Form, Items),', metadata),
            "raw after literal rename": (table + raw, metadata.replace('"props.set"', '"props.probe"')),
            "raw renamed name": (table + '("props.probe", Properties, Form, Items),', metadata.replace('"props.set"', '"props.probe"')),
            "missing metadata": (table, None),
            "empty metadata": (table, "properties_operations!();"),
            "missing metadata row": (table, metadata.replace(row + ",", "")),
            "incomplete metadata": (table, metadata.replace(row, 'Set => ("props.set", MetadataOrRoleOrSubsystem)')),
            "nonliteral name": (table, metadata.replace(row, 'Set => (NAME, MetadataOrRoleOrSubsystem, Values)')),
            "duplicate metadata": (table, metadata + '\nproperties_operations!(Probe => ("props.probe", Form, Items),);'),
        }
        for label, (source, changed_metadata) in cases.items():
            with self.subTest(label=label), self.assertRaises(RuntimeError):
                self.registration_fixture(source, role_source, {"properties_operation.rs": changed_metadata}, complete_properties=False)
        registry, implemented = self.renderer().load_apply_registry()
        self.assertEqual([entry for entry in registry if entry[1] == "Properties"],
                         [("props.set", "Properties", "MetadataOrRoleOrSubsystem", "Values")])
        self.assertIn("props.set", implemented)

    def test_typed_support_rows_keep_their_metadata_and_registry_order(self) -> None:
        metadata = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        support_metadata = '''support_operations!(
            CapabilitySet => ("supportCapability.set", Configuration, Values),
            RuleSet => ("supportRule.set", SupportRule, Values),
        );'''
        registry, _ = self.registration_fixture(
            '(RoleOperation::Create), (RoleOperation::RightAdd), (RoleOperation::RightSet),\n'
            '(SupportOperation::CapabilitySet),\n'
            '("probe.add", Probe, Subsystem, Items),\n'
            '(SupportOperation::RuleSet),',
            metadata,
            {"support_operation.rs": support_metadata},
            complete_support=False,
        )
        self.assertEqual(registry, [
            ("role.create", "Role", "Configuration", "Values"),
            ("right.add", "Role", "Role", "Items"),
            ("right.set", "Role", "RoleRight", "Values"),
            ("supportCapability.set", "Support", "Configuration", "Values"),
            ("probe.add", "Probe", "Subsystem", "Items"),
            ("supportRule.set", "Support", "SupportRule", "Values"),
        ] + self.CONTEXT_ROWS + self.MXL_EVENT_ROWS + self.DCS_FIXTURE_ROWS + self.PROPERTIES_ROWS + self.XDTO_ROWS + self.SOURCE_ROWS)

    def test_support_rows_keep_their_arguments_order_and_implemented_status(self) -> None:
        module = self.renderer()
        registry, implemented = module.load_apply_registry()
        self.assertEqual([entry for entry in registry if entry[1] == "Support"], declared_rows("Support"))
        rendered = module.render_document()
        for operation, _, _, skeleton in declared_rows("Support"):
            self.assertIn(operation, implemented)
            self.assertIn(f"| `{operation}` | `{skeleton.lower()}` | ", rendered)

    def test_typed_rows_preserve_colliding_names_and_raw_neighbors(self) -> None:
        metadata = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        registry, _ = self.registration_fixture(
            '(RoleOperation::Create), (RoleOperation::RightAdd), (RoleOperation::RightSet),\n'
            '(unica_metadata::apply_operations::MetadataOperation::AttributeAdd),\n'
            '(unica_metadata::apply_operations::MetadataOperation::AttributeSet),\n'
            '(unica_metadata::apply_operations::MetadataOperation::ObjectCreate),\n'
            '("probe.add", Probe, Subsystem, Items),\n'
            '(FormOperation::Add), (FormOperation::Set), (FormOperation::Remove),\n'
            '(FormOperation::AttributeAdd), (FormOperation::AttributeRemove), (FormOperation::ColumnAdd),\n'
            '(unica_metadata::apply_operations::MetadataOperation::AttributeRemove),\n'
            '(unica_metadata::apply_operations::MetadataOperation::CommandAdd),\n'
            '(unica_metadata::apply_operations::MetadataOperation::CommandSet), (unica_metadata::apply_operations::MetadataOperation::CommandRemove),\n'
            '(FormOperation::ItemAdd),\n'
            '(FormOperation::CommandAdd),\n'
            '(FormOperation::ItemRemove), (FormOperation::ItemMove), (FormOperation::EventBind),\n'
            '(FormOperation::PropsSet),',
            metadata,
            {
                "metadata_operation.rs": '''metadata_operations!(
                    ObjectCreate => ("object.create", Configuration, Values),
                    AttributeAdd => ("attribute.add", MetadataAttributeContainer, Items),
                    AttributeSet => ("attribute.set", MetadataAttribute, Values),
                    AttributeRemove => ("attribute.remove", MetadataAttribute, Target),
                    CommandAdd => ("command.add", MetadataCommands, Items),
                    CommandSet => ("command.set", MetadataCommand, Values),
                    CommandRemove => ("command.remove", MetadataCommand, Target),
                );''',
            },
            complete_context=False,
        )
        self.assertEqual(registry, [
            ("role.create", "Role", "Configuration", "Values"),
            ("right.add", "Role", "Role", "Items"),
            ("right.set", "Role", "RoleRight", "Values"),
            ("attribute.add", "Metadata", "MetadataAttributeContainer", "Items"),
            ("attribute.set", "Metadata", "MetadataAttribute", "Values"),
            ("object.create", "Metadata", "Configuration", "Values"),
            ("probe.add", "Probe", "Subsystem", "Items"),
            ("form.add", "Form", "MetadataForms", "Items"),
            ("form.set", "Form", "MetadataForms", "Values"),
            ("form.remove", "Form", "Form", "Target"),
            ("attribute.add", "Form", "Form", "Items"),
            ("attribute.remove", "Form", "FormAttribute", "Target"),
            ("column.add", "Form", "FormAttribute", "Items"),
            ("attribute.remove", "Metadata", "MetadataAttribute", "Target"),
            ("command.add", "Metadata", "MetadataCommands", "Items"),
            ("command.set", "Metadata", "MetadataCommand", "Values"),
            ("command.remove", "Metadata", "MetadataCommand", "Target"),
            ("item.add", "Form", "FormItemContainer", "Items"),
            ("command.add", "Form", "Form", "Items"),
            ("item.remove", "Form", "FormItem", "Target"),
            ("item.move", "Form", "FormItem", "Values"),
            ("event.bind", "Form", "FormEvent", "Values"),
            ("props.set", "Form", "FormProperties", "Values"),
            ("supportCapability.set", "Support", "Configuration", "Values"),
            ("supportRule.set", "Support", "SupportRule", "Values"),
        ] + self.MXL_EVENT_ROWS + self.DCS_FIXTURE_ROWS + self.PROPERTIES_ROWS + self.XDTO_ROWS + self.SOURCE_ROWS)

    def test_fully_typed_metadata_and_form_keep_registry_rows(self) -> None:
        module = self.renderer()
        registry, implemented = module.load_apply_registry()
        for family, migrated_names, remaining in [
            ("Metadata", {name for _, name, _, _ in self.METADATA_CONTEXT}, 0),
            ("Form", {name for _, name, _, _ in self.FORM_CONTEXT}, 0),
        ]:
            self.assertEqual(
                [entry for entry in registry if entry[1] == family and entry[0] in migrated_names],
                [entry for entry in self.CONTEXT_ROWS if entry[1] == family],
            )
            self.assertEqual(sum(entry[1] == family and entry[0] not in migrated_names for entry in registry), remaining)
            self.assertLessEqual(migrated_names, implemented)
        self.assertIn(("content.set", "Metadata", "ExchangePlan", "Items"), registry)
        self.assertIn("content.set", implemented)
        self.assertEqual(len(registry), 131)
        self.assertEqual(len({entry[0] for entry in registry}), 126)

    def test_dcs_rows_keep_their_declared_metadata_and_family_order(self) -> None:
        module = self.renderer()
        registry, implemented = module.load_apply_registry()
        expected = declared_rows("Dcs")
        self.assertEqual([row for row in registry if row[1] == "Dcs"], expected)
        self.assertLessEqual({row[0] for row in expected}, implemented)

    def test_typed_dcs_rows_keep_payload_metadata_and_raw_registry_neighbors(self) -> None:
        role_source = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        registry, _ = self.registration_fixture(
            '(RoleOperation::Create), (RoleOperation::RightAdd), (RoleOperation::RightSet),\n'
            '(Primitive::QuerySet), ("probe.add", Probe, Subsystem, Items),\n'
            '(Primitive::DataSetRemove), ("probe.replace", Probe, Code, Text),\n'
            '(Primitive::DataSourceAdd),',
            role_source, complete_dcs=False,
        )
        self.assertEqual(registry, [
            ("role.create", "Role", "Configuration", "Values"),
            ("right.add", "Role", "Role", "Items"),
            ("right.set", "Role", "RoleRight", "Values"),
            ("query.set", "Dcs", "DcsQuery", "Values"),
            ("probe.add", "Probe", "Subsystem", "Items"),
            ("dataSet.remove", "Dcs", "DcsData", "Target"),
            ("probe.replace", "Probe", "Code", "Text"),
            ("dataSource.add", "Dcs", "DcsSchema", "Items"),
            ("supportCapability.set", "Support", "Configuration", "Values"),
            ("supportRule.set", "Support", "SupportRule", "Values"),
        ] + self.CONTEXT_ROWS + self.MXL_EVENT_ROWS + self.PROPERTIES_ROWS + self.XDTO_ROWS + self.SOURCE_ROWS)

    def test_dcs_metadata_and_registrations_are_complete_and_fail_closed(self) -> None:
        role_source = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        roles = "(RoleOperation::Create), (RoleOperation::RightAdd), (RoleOperation::RightSet),\n"
        table = roles + self.DCS_FIXTURE_REGISTRATIONS
        metadata = self.DCS_FIXTURE_METADATA
        self.registration_fixture(table, role_source, complete_dcs=False)
        row = 'QuerySet => ("query.set", DcsQuery, Values, String)'
        cases = {
            "unknown typed family": (table.replace("Primitive::QuerySet", "DcsOperation::QuerySet"), metadata),
            "unknown variant": (table.replace("Primitive::QuerySet", "Primitive::Unknown"), metadata),
            "wrong typed family": (table.replace("Primitive::QuerySet", "unica_metadata::apply_operations::MetadataOperation::QuerySet"), metadata),
            "all family registrations missing": (roles, metadata),
            "raw unknown name": (table + '("future.operation", Dcs, DcsSchema, Items),', metadata),
            "raw original name after rename": (
                table + '("query.set", Dcs, DcsQuery, Values),', metadata.replace('"query.set"', '"query.probe"'),
            ),
            "raw renamed family name": (
                table + '("query.probe", Dcs, DcsQuery, Values),', metadata.replace('"query.set"', '"query.probe"'),
            ),
            "unparsed registration": (table + "unexpected_registration!(),", metadata),
            "missing metadata file": (table, None),
            "missing metadata table": (table, metadata.replace("dcs_operations!", "previous_operations!")),
            "repeated metadata table": (table, metadata + '\ndcs_operations!(Extra => ("extra.add", DcsSchema, Items, String),);'),
            "empty metadata table": (table, "dcs_operations!();"),
            "unterminated metadata table": (table, 'dcs_operations!(QuerySet => ("query.set", DcsQuery, Values, String),'),
            "missing metadata row": (table, metadata.replace(row + ",", "")),
            "unsupported metadata row": (table, metadata.replace(row, "QuerySet => operation_metadata()")),
            "unparsed trailing metadata": (table, metadata.replace(row + ",", row + ", unexpected!(),")),
            "nonliteral metadata name": (table, metadata.replace(row, 'QuerySet => (QUERY_SET, DcsQuery, Values, String)')),
            "duplicate metadata variant": (table, metadata.replace("QuerySet =>", "DataSourceAdd =>")),
            "duplicate metadata name": (table, metadata.replace('"query.set"', '"dataSource.add"')),
        }
        for variant, (name, family, applicability, skeleton) in zip(
            ["DataSourceAdd", "QuerySet", "DataSetRemove"], self.DCS_FIXTURE_ROWS, strict=True,
        ):
            registration = f"(Primitive::{variant}),"
            raw = f'("{name}", {family}, {applicability}, {skeleton}),'
            cases[f"missing {name}"] = (table.replace(registration, ""), metadata)
            cases[f"duplicate {name}"] = (table + registration, metadata)
            cases[f"raw {name} replacement"] = (table.replace(registration, raw), metadata)
            cases[f"raw {name} beside complete typed"] = (table + raw, metadata)
        for case, (source, changed_metadata) in cases.items():
            with self.subTest(case=case), self.assertRaises(RuntimeError):
                self.registration_fixture(
                    source, role_source, {"dcs_operation.rs": changed_metadata}, complete_dcs=False,
                )

    def test_dcs_payload_type_syntax_is_consumed_without_skipping_metadata(self) -> None:
        role_source = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        roles = self.ROLE_REGISTRATIONS
        for payload in ["DataSourceSpec", "LocalizedText", "Vec<String>", "Vec < DataSourceSpec >", "()", "(\n )"]:
            changed = self.DCS_FIXTURE_METADATA.replace("Vec<DataSourceSpec>", payload)
            with self.subTest(payload=payload):
                registry, _ = self.registration_fixture(roles, role_source, {"dcs_operation.rs": changed})
                self.assertEqual([row for row in registry if row[1] == "Dcs"], self.DCS_FIXTURE_ROWS)
        for payload in ["", "Vec<>", "Vec<DataSourceSpec", "Vec<DataSourceSpec>>", "Vec<String, Other>", "payload()", "String, Extra", "("]:
            changed = self.DCS_FIXTURE_METADATA.replace("Vec<DataSourceSpec>", payload)
            with self.subTest(payload=payload), self.assertRaises(RuntimeError):
                self.registration_fixture(roles, role_source, {"dcs_operation.rs": changed})
        without_payload = self.DCS_FIXTURE_METADATA.replace(
            "Items, Vec<DataSourceSpec>", "Items",
        )
        with self.assertRaises(RuntimeError):
            self.registration_fixture(roles, role_source, {"dcs_operation.rs": without_payload})

    def test_contextual_typed_metadata_cannot_be_missing_duplicated_or_registered_as_raw(self) -> None:
        role_metadata = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        roles = "(RoleOperation::Create), (RoleOperation::RightAdd), (RoleOperation::RightSet),\n"
        table = roles + self.CONTEXT_REGISTRATIONS
        self.registration_fixture(table, role_metadata, complete_context=False)
        for family, applicability in [("Metadata", "MetadataCommands"), ("Form", "Form")]:
            filename = f"{family.lower()}_operation.rs"
            table_name = f"{family.lower()}_operations"
            operation_type = self.renderer().TYPED_FAMILIES[family]
            metadata = self.family_source(family)
            last_row = f'    CommandAdd => ("command.add", {applicability}, Items),\n'
            self.assertIn(last_row, metadata)
            raw = f'("command.add", {family}, {applicability}, Items),'
            other_type = self.renderer().TYPED_FAMILIES["Form" if family == "Metadata" else "Metadata"]
            operations = self.METADATA_CONTEXT if family == "Metadata" else self.FORM_CONTEXT
            without_family = table
            for variant, _, _, _ in operations:
                without_family = without_family.replace(f"({operation_type}::{variant}),", "")
            cases = {
                "unknown typed family": (table.replace(f"{operation_type}::AttributeAdd", "OtherOperation::AttributeAdd"), metadata),
                "unknown variant": (table.replace(f"{operation_type}::AttributeAdd", f"{operation_type}::Missing"), metadata),
                "wrong typed family": (table.replace(f"{operation_type}::AttributeAdd", f"{other_type}::AttributeAdd"), metadata),
                "missing registration": (table.replace(f"({operation_type}::CommandAdd),", ""), metadata),
                "all family registrations missing": (without_family, metadata),
                "both contextual families missing": (roles, metadata),
                "duplicate registration": (table + f"({operation_type}::CommandAdd),", metadata),
                "raw instead of typed": (table.replace(f"({operation_type}::CommandAdd),", raw), metadata),
                "raw beside complete typed": (table + raw, metadata),
                "missing metadata file": (table, None),
                "missing metadata table": (table, metadata.replace(f"\n{table_name}!(", "\nprevious_operations!(")),
                "repeated metadata table": (table, metadata + f'\n{table_name}!(Extra => ("extra.add", Form, Items),);\n'),
                "empty metadata table": (table, f"{table_name}!();"),
                "unterminated metadata table": (table, f'{table_name}!(CommandAdd => ("command.add", {applicability}, Items),'),
                "missing metadata row": (table, metadata.replace(last_row, "")),
                "incomplete metadata row": (table, metadata.replace(last_row, f'    CommandAdd => ("command.add", {applicability}),\n')),
                "unsupported metadata row": (table, metadata.replace(last_row, "    CommandAdd => operation_metadata(),\n")),
                "nonliteral metadata name": (table, metadata.replace(last_row, f"    CommandAdd => (COMMAND_ADD, {applicability}, Items),\n")),
                "duplicate metadata variant": (table, metadata.replace(last_row, f'    AttributeAdd => ("command.add", {applicability}, Items),\n')),
                "duplicate metadata name": (table, metadata.replace(last_row, f'    CommandAdd => ("attribute.add", {applicability}, Items),\n')),
            }
            for variant, name, applicability, skeleton in operations:
                registration = f"({operation_type}::{variant}),"
                raw_operation = f'("{name}", {family}, {applicability}, {skeleton}),'
                cases[f"wrong {name} typed family"] = (
                    table.replace(registration, f"({other_type}::{variant}),"), metadata,
                )
                if variant == "CommandAdd":
                    continue  # The common cases above already mutate this variant.
                cases[f"missing {name} registration"] = (table.replace(registration, ""), metadata)
                cases[f"duplicate {name} registration"] = (table + registration, metadata)
                cases[f"raw {name} instead of typed"] = (table.replace(registration, raw_operation), metadata)
                cases[f"raw {name} beside complete typed"] = (table + raw_operation, metadata)
            renamed_metadata = metadata.replace('"command.add"', '"command.probe"')
            cases["raw unknown family name"] = (table + f'("future.operation", {family}, Form, Items),', metadata)
            cases["raw original name after literal rename"] = (table + raw, renamed_metadata)
            cases["raw renamed family name"] = (table + f'("command.probe", {family}, Form, Items),', renamed_metadata)
            for case, (source, family_metadata) in cases.items():
                with self.subTest(family=family, case=case), self.assertRaises(RuntimeError):
                    self.registration_fixture(
                        source, role_metadata, {filename: family_metadata}, complete_context=False,
                    )

    def test_mxl_and_event_rows_are_complete_and_fail_closed(self) -> None:
        role_source = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        roles = self.ROLE_REGISTRATIONS
        table = roles + self.MXL_EVENT_REGISTRATIONS
        actual, _ = self.registration_fixture(table, role_source, complete_mxl_event=False)
        self.assertEqual([row for row in actual if row[1] in {"Mxl", "Event"}], self.MXL_EVENT_ROWS)
        for family, variants in [(family, declared_variants(family)) for family in ("Mxl", "Event")]:
            filename = f"{family.lower()}_operation.rs"
            metadata = self.family_source(family)
            for variant, name, applicability, skeleton in variants:
                registration = f"({family}Operation::{variant}),"
                row = f'{variant} => ("{name}", {applicability}, {skeleton})'
                raw = f'("{name}", {family}, {applicability}, {skeleton}),'
                cases = {
                    "missing": (table.replace(registration, ""), metadata),
                    "duplicate": (table + registration, metadata),
                    "wrong family": (table.replace(registration, f"(FormOperation::{variant}),"), metadata),
                    "unknown variant": (table.replace(registration, f"({family}Operation::Unknown),"), metadata),
                    "raw replacement": (table.replace(registration, raw), metadata),
                    "raw beside": (table + raw, metadata),
                    "raw unknown": (table + f'("future.operation", {family}, Form, Items),', metadata),
                    "raw renamed bypass": (table + raw, metadata.replace(f'"{name}"', '"future.operation"')),
                    "raw new spelling": (table + f'("future.operation", {family}, Form, Items),', metadata.replace(f'"{name}"', '"future.operation"')),
                    "metadata missing": (table, None),
                    "metadata empty": (table, f"{family.lower()}_operations!();"),
                    "metadata row missing": (table, metadata.replace(row + ",", "")),
                    "metadata incomplete": (table, metadata.replace(row, f'{variant} => ("{name}", {applicability})')),
                    "metadata nonliteral": (table, metadata.replace(row, f'{variant} => (NAME, {applicability}, {skeleton})')),
                    "metadata repeated": (table, metadata + f'\n{family.lower()}_operations!(Probe => ("future.operation", Form, Items),);'),
                }
                for case, (source, changed_metadata) in cases.items():
                    with self.subTest(family=family, variant=variant, case=case), self.assertRaises(RuntimeError):
                        self.registration_fixture(source, role_source, {filename: changed_metadata}, complete_mxl_event=False)
        registry, _ = self.renderer().load_apply_registry()
        self.assertEqual([row for row in registry if row[1] in {"Mxl", "Event"}], self.MXL_EVENT_ROWS)

    def test_typed_role_rows_stay_between_their_raw_registry_neighbors(self) -> None:
        metadata = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        registry, _ = self.registration_fixture(
            '("probe.add", Probe, Subsystem, Items),\n'
            '(RoleOperation::Create),\n'
            '(RoleOperation::RightAdd),\n'
            '// A comment is not a descriptor: (OtherOperation::Hidden),\n'
            '(RoleOperation::RightSet),\n'
            '("probe.replace", Probe, Code, Text),',
            metadata,
        )
        self.assertEqual(registry, [
            ("probe.add", "Probe", "Subsystem", "Items"),
            ("role.create", "Role", "Configuration", "Values"),
            ("right.add", "Role", "Role", "Items"),
            ("right.set", "Role", "RoleRight", "Values"),
            ("probe.replace", "Probe", "Code", "Text"),
            ("supportCapability.set", "Support", "Configuration", "Values"),
            ("supportRule.set", "Support", "SupportRule", "Values"),
        ] + self.CONTEXT_ROWS + self.MXL_EVENT_ROWS + self.DCS_FIXTURE_ROWS + self.PROPERTIES_ROWS + self.XDTO_ROWS + self.SOURCE_ROWS)

    def test_role_metadata_survives_split_impls_and_unrelated_helpers(self) -> None:
        metadata = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        contract = "    pub fn contract(self) -> OperationContract {"
        self.assertEqual(metadata.count(contract), 1)
        metadata = metadata.replace(
            contract,
            '}\n\n// role_operations!(Commented => ("ignored.operation", Configuration, Values),);\n'
            'impl RoleOperation {\n'
            '    #[allow(dead_code)]\n'
            '    fn unrelated_metadata_hint(self) -> &\'static str { "role_operations!(NotATable)" }\n\n'
            + contract,
        )
        registry, _ = self.registration_fixture(
            "(RoleOperation::Create), (RoleOperation::RightAdd), (RoleOperation::RightSet),",
            metadata,
        )
        self.assertEqual(registry, [
            ("role.create", "Role", "Configuration", "Values"),
            ("right.add", "Role", "Role", "Items"),
            ("right.set", "Role", "RoleRight", "Values"),
            ("supportCapability.set", "Support", "Configuration", "Values"),
            ("supportRule.set", "Support", "SupportRule", "Values"),
        ] + self.CONTEXT_ROWS + self.MXL_EVENT_ROWS + self.DCS_FIXTURE_ROWS + self.PROPERTIES_ROWS + self.XDTO_ROWS + self.SOURCE_ROWS)

    def test_unknown_or_incomplete_typed_metadata_cannot_silently_drop_a_role(self) -> None:
        metadata = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        table = '("probe.add", Probe, Subsystem, Items),\n' + "\n".join(
            f"(RoleOperation::{variant})," for variant in ["Create", "RightAdd", "RightSet"]
        )
        last_row = '    RightSet => ("right.set", RoleRight, Values),\n'
        self.assertIn(last_row, metadata)
        cases = {
            "unknown typed family": (table.replace("RoleOperation::Create", "OtherOperation::Create"), metadata),
            "unknown variant": (table.replace("RoleOperation::Create", "RoleOperation::Missing"), metadata),
            "missing registration": (table.replace("(RoleOperation::RightSet),", ""), metadata),
            "all typed registrations missing": ('("probe.add", Probe, Subsystem, Items),', metadata),
            "duplicate registration": (table + "\n(RoleOperation::Create),", metadata),
            "missing metadata file": (table, None),
            "missing metadata table": (table, metadata.replace("\nrole_operations!(", "\nprevious_role_operations!(")),
            "repeated metadata table": (table, metadata + '\nrole_operations!(Extra => ("role.extra", Configuration, Values),);\n'),
            "empty metadata table": (table, "role_operations!();"),
            "unterminated metadata table": (table, 'role_operations!(Create => ("role.create", Configuration, Values),'),
            "missing metadata row": (table, metadata.replace(last_row, "")),
            "incomplete metadata row": (table, metadata.replace(last_row, '    RightSet => ("right.set", RoleRight),\n')),
            "unsupported metadata row": (table, metadata.replace(last_row, '    RightSet => role_metadata(),\n')),
            "nonliteral metadata name": (table, metadata.replace(last_row, '    RightSet => (RIGHT_SET, RoleRight, Values),\n')),
            "duplicate metadata variant": (table, metadata.replace(last_row, '    RightAdd => ("right.set", RoleRight, Values),\n')),
            "duplicate metadata name": (table, metadata.replace(last_row, '    RightSet => ("right.add", RoleRight, Values),\n')),
        }
        for case, (source, role_metadata) in cases.items():
            with self.subTest(case=case), self.assertRaises(RuntimeError):
                self.registration_fixture(source, role_metadata)

    def test_unknown_or_incomplete_typed_metadata_cannot_silently_drop_support(self) -> None:
        metadata = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        support_metadata = (REPO_ROOT / "crates/unica-metadata/src/support.rs").read_text(encoding="utf-8")
        roles = "(RoleOperation::Create), (RoleOperation::RightAdd), (RoleOperation::RightSet),\n"
        table = roles + "(SupportOperation::CapabilitySet), (SupportOperation::RuleSet),"
        last_row = '    RuleSet => ("supportRule.set", SupportRule, Values),\n'
        self.assertIn(last_row, support_metadata)
        cases = {
            "unknown typed family": (table.replace("SupportOperation::CapabilitySet", "OtherOperation::CapabilitySet"), support_metadata),
            "unknown variant": (table.replace("SupportOperation::CapabilitySet", "SupportOperation::Missing"), support_metadata),
            "missing registration": (table.replace("(SupportOperation::RuleSet),", ""), support_metadata),
            "all typed registrations missing": (roles, support_metadata),
            "duplicate registration": (table + "\n(SupportOperation::CapabilitySet),", support_metadata),
            "missing metadata file": (table, None),
            "missing metadata table": (table, support_metadata.replace("\nsupport_operations!(", "\nprevious_support_operations!(")),
            "repeated metadata table": (table, support_metadata + '\nsupport_operations!(Extra => ("support.extra", Configuration, Values),);\n'),
            "empty metadata table": (table, "support_operations!();"),
            "unterminated metadata table": (table, 'support_operations!(CapabilitySet => ("supportCapability.set", Configuration, Values),'),
            "missing metadata row": (table, support_metadata.replace(last_row, "")),
            "incomplete metadata row": (table, support_metadata.replace(last_row, '    RuleSet => ("supportRule.set", SupportRule),\n')),
            "unsupported metadata row": (table, support_metadata.replace(last_row, '    RuleSet => support_metadata(),\n')),
            "nonliteral metadata name": (table, support_metadata.replace(last_row, '    RuleSet => (SUPPORT_RULE_SET, SupportRule, Values),\n')),
            "duplicate metadata variant": (table, support_metadata.replace(last_row, '    CapabilitySet => ("supportRule.set", SupportRule, Values),\n')),
            "duplicate metadata name": (table, support_metadata.replace(last_row, '    RuleSet => ("supportCapability.set", SupportRule, Values),\n')),
        }
        for case, (source, support_source) in cases.items():
            with self.subTest(case=case), self.assertRaises(RuntimeError):
                self.registration_fixture(
                    source, metadata, {"support_operation.rs": support_source}, complete_support=False,
                )

    def test_support_metadata_survives_split_impls_and_unrelated_helpers(self) -> None:
        metadata = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        support_metadata = (REPO_ROOT / "crates/unica-metadata/src/support.rs").read_text(encoding="utf-8")
        contract = "    pub fn contract(self) -> OperationContract {"
        self.assertEqual(support_metadata.count(contract), 1)
        support_metadata = support_metadata.replace(
            contract,
            '}\n\n// support_operations!(Commented => ("ignored.operation", Configuration, Values),);\n'
            'impl SupportOperation {\n'
            '    #[allow(dead_code)]\n'
            '    fn unrelated_metadata_hint(self) -> &\'static str { "support_operations!(NotATable)" }\n\n'
            + contract,
        )
        registry, _ = self.registration_fixture(
            "(RoleOperation::Create), (RoleOperation::RightAdd), (RoleOperation::RightSet),",
            metadata, {"support_operation.rs": support_metadata},
        )
        self.assertEqual([entry for entry in registry if entry[1] == "Support"], [
            ("supportCapability.set", "Support", "Configuration", "Values"),
            ("supportRule.set", "Support", "SupportRule", "Values"),
        ])

    def test_raw_typed_family_rows_cannot_bypass_complete_typed_registrations(self) -> None:
        metadata = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        roles = self.ROLE_REGISTRATIONS
        support = "(SupportOperation::CapabilitySet), (SupportOperation::RuleSet),"
        raw_roles = (
            '("role.create", Role, Configuration, Values),'
            '("right.add", Role, Role, Items),'
            '("right.set", Role, RoleRight, Values),'
        )
        raw_support = (
            '("supportCapability.set", Support, Configuration, Values),'
            '("supportRule.set", Support, SupportRule, Values),'
        )
        for table in [
            roles + support + raw_roles,
            roles + support + raw_support,
            raw_roles + support,
            roles + raw_support,
        ]:
            with self.subTest(table=table), self.assertRaises(RuntimeError):
                self.registration_fixture(table, metadata, complete_support=False)

    def test_metadata_table_locator_ignores_comments_strings_and_character_literals(self) -> None:
        metadata = (REPO_ROOT / "crates/unica-roles/src/operations.rs").read_text(encoding="utf-8")
        decoy = 'role_operations!(Decoy => ("ignored.operation", Configuration, Values),);'
        prefix = (
            '// ' + decoy + '\n'
            '/* outer /* nested */ ' + decoy + ' */\n'
            'const ORDINARY: &str = ' + json.dumps(decoy) + ';\n'
            'const RAW: &str = r###"a quoted \" prefix ' + decoy + '"###;\n'
            'const RAW_BYTES: &[u8] = br##"a quoted \" prefix ' + decoy + '"##;\n'
            'fn unused_c_literal() { let _ = cr#"left " role_operations!(NotATable); right"#; }\n'
            'fn quote_marker() -> char { \'"\' }\n'
            'fn byte_quote_marker() -> u8 { b\'"\' }\n'
            "fn lifetime_probe<'a>(value: &'a str) -> &'a str { value }\n"
        )
        table = "(RoleOperation::Create), (RoleOperation::RightAdd), (RoleOperation::RightSet),"
        expected, implemented = self.registration_fixture(table, metadata)
        for family in ["Role", "Support", "Metadata", "Form", "Mxl", "Event", "Dcs"]:
            family_prefix = prefix.replace("role_operations", f"{family.lower()}_operations")
            filename = f"{family.lower()}_operation.rs"
            family_metadata = self.DCS_FIXTURE_METADATA if family == "Dcs" else self.family_source(family)
            with self.subTest(family=family):
                role_source = family_prefix + metadata if family == "Role" else metadata
                actual, actual_implemented = self.registration_fixture(
                    table, role_source,
                    {} if family == "Role" else {filename: family_prefix + family_metadata},
                )
                self.assertEqual(actual, expected)
                self.assertEqual(actual_implemented, implemented)
                with self.assertRaises(RuntimeError):
                    self.registration_fixture(
                        table, family_prefix if family == "Role" else metadata,
                        {} if family == "Role" else {filename: family_prefix},
                    )


if __name__ == "__main__":
    unittest.main()
