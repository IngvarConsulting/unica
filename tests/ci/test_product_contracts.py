from __future__ import annotations

import copy
from contextlib import ExitStack, contextmanager, nullcontext
import gc
import importlib.util
import json
import os
import re
import signal
import subprocess
import sys
import tempfile
import time
import tomllib
import unittest
import warnings
from pathlib import Path
from unittest.mock import patch

from tree_sitter import Language, Parser
import tree_sitter_rust

# Размер тестов модуля (scripts/ci/python_sizes.py).
# Контракт продукта: тест зовёт `cargo metadata`.
MEDIUM = ("ProductContractTests.test_rmcp_transport_is_confined_to_mcp_interface",)


REPO_ROOT = Path(__file__).resolve().parents[2]
RMCP_OWNER = "crates/unica-mcp/src/lib.rs"
RMCP_PACKAGE = "unica-mcp"
RELATIVE_TOOL_CONTRACT_TIMEOUT_SECONDS = 15
RMCP_INTERFACE_ROOT = "crates/unica-mcp/src/"
RMCP_CRATES_IO_SOURCE = "registry+https://github.com/rust-lang/crates.io-index"
RMCP_ALLOWED_FEATURES = ["server", "transport-io"]
TEST_ONLY_ATTRIBUTES = frozenset({"#[cfg(test)]"})
# Граница экспорта пропускает и внутренности двери, открытые тестам демона.
EXPORT_TEST_ONLY_ATTRIBUTES = TEST_ONLY_ATTRIBUTES | {'#[cfg(any(test,feature="test-support"))]'}
# Единственный параметр `run_stdio`: подключение к демону передаёт `unica-product`.
RUN_STDIO_PARAMETER = "fn()->Result<V5DaemonClient,String>"


def cfg_test_item_ranges(
    source: bytes, tree, attributes: frozenset[str] = TEST_ONLY_ATTRIBUTES
) -> list[tuple[int, int]]:
    ranges = []

    def collect(parent) -> None:
        children = parent.named_children
        if parent.type == "source_file" and any(
            child.type == "inner_attribute_item"
            and re.sub(r"\s+", "", source[child.start_byte : child.end_byte].decode()) == "#![cfg(test)]"
            for child in children
        ):
            # Файл тестового модуля целиком вне производственной сборки.
            ranges.append((0, len(source)))
            return
        index = 0
        while index < len(children):
            child = children[index]
            compact = re.sub(
                r"\s+",
                "",
                source[child.start_byte : child.end_byte].decode(),
            )
            if child.type == "attribute_item" and compact in attributes:
                end = index + 1
                while end < len(children) and children[end].type == "attribute_item":
                    end += 1
                if end < len(children):
                    ranges.append((child.start_byte, children[end].end_byte))
                    index = end + 1
                    continue
            collect(child)
            index += 1

    collect(tree.root_node)
    return ranges


def productive_rust_code(source: bytes) -> bytes:
    parser = Parser(Language(tree_sitter_rust.language()))
    tree = parser.parse(source)
    ignored = {
        "block_comment",
        "char_literal",
        "line_comment",
        "raw_string_literal",
        "string_literal",
    }
    code = bytearray(source)
    for start, end in cfg_test_item_ranges(source, tree):
        code[start:end] = b" " * (end - start)
    stack = [tree.root_node]
    while stack:
        node = stack.pop()
        if node.type in ignored:
            code[node.start_byte : node.end_byte] = b" " * (node.end_byte - node.start_byte)
            continue
        stack.extend(node.children)
    return bytes(code)


def direct_git_command_calls(source: bytes) -> list[int]:
    """Return productive direct std::process git spawn offsets."""
    parser = Parser(Language(tree_sitter_rust.language()))
    tree = parser.parse(source)
    test_ranges = cfg_test_item_ranges(source, tree)

    def is_test_only(node) -> bool:
        return any(
            node.start_byte >= start and node.end_byte <= end
            for start, end in test_ranges
        )

    def is_git_literal(node) -> bool:
        literal = source[node.start_byte : node.end_byte]
        if node.type == "string_literal":
            return literal == b'"git"'
        if node.type != "raw_string_literal":
            return False
        match = re.fullmatch(rb'r(?P<hashes>#{0,255})"git"(?P=hashes)', literal)
        return match is not None

    calls = []
    stack = [tree.root_node]
    while stack:
        node = stack.pop()
        if is_test_only(node):
            continue
        if node.type == "call_expression":
            function = node.child_by_field_name("function")
            arguments = node.child_by_field_name("arguments")
            if (
                function is not None
                and source[function.start_byte : function.end_byte]
                == b"std::process::Command::new"
                and arguments is not None
                and arguments.named_child_count == 1
                and is_git_literal(arguments.named_child(0))
            ):
                calls.append(node.start_byte)
        stack.extend(node.named_children)
    return sorted(calls)


def cargo_workspace_metadata(repo_root: Path) -> dict:
    result = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=repo_root,
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout)


def tracked_workspace_production_rust_sources(repo_root: Path) -> dict[str, bytes]:
    metadata = cargo_workspace_metadata(repo_root)
    workspace_members = set(metadata["workspace_members"])
    root = repo_root.resolve()
    source_roots = []
    for package in metadata["packages"]:
        if package["id"] not in workspace_members:
            continue
        package_root = Path(package["manifest_path"]).resolve().parent
        source_root = package_root / "src"
        source_roots.append(source_root.relative_to(root))

    tracked = subprocess.run(
        ["git", "ls-files", "-z"],
        cwd=repo_root,
        check=True,
        capture_output=True,
    ).stdout.split(b"\0")
    sources = {}
    for raw_path in tracked:
        if not raw_path:
            continue
        path = raw_path.decode()
        if not path.endswith(".rs"):
            continue
        tracked_path = Path(path)
        if any(
            tracked_path == source_root or tracked_path.is_relative_to(source_root)
            for source_root in source_roots
        ):
            sources[path] = (repo_root / path).read_bytes()
    return sources


def rmcp_reference_confinement_errors(
    sources: dict[str, bytes], owner: str
) -> list[str]:
    productive = {
        path: productive_rust_code(source) for path, source in sources.items()
    }
    return [
        f"{path}: productive rmcp reference outside transport owner"
        for path, code in sorted(productive.items())
        if not (
            (owner.endswith("/") and path.startswith(owner))
            or (not owner.endswith("/") and path == owner)
        )
        and re.search(rb"\brmcp\b", code)
    ]


def workspace_rmcp_reference_confinement_errors(repo_root: Path) -> list[str]:
    return rmcp_reference_confinement_errors(
        tracked_workspace_production_rust_sources(repo_root),
        RMCP_INTERFACE_ROOT,
    )


def rmcp_owner_export_boundary_errors(source: bytes, owner: str) -> list[str]:
    parser = Parser(Language(tree_sitter_rust.language()))
    tree = parser.parse(source)
    if tree.root_node.has_error:
        return [f"{owner}: Rust parser could not resolve the export boundary"]
    test_ranges = cfg_test_item_ranges(source, tree, EXPORT_TEST_ONLY_ATTRIBUTES)

    def is_test_only(node) -> bool:
        return any(
            node.start_byte >= start and node.end_byte <= end
            for start, end in test_ranges
        )

    def node_text(node) -> bytes:
        return source[node.start_byte : node.end_byte]

    errors = []
    root_run_stdio_count = 0
    stack = [tree.root_node]
    while stack:
        node = stack.pop()
        if is_test_only(node):
            continue
        if node.type == "attribute_item":
            attribute = next(
                (child for child in node.named_children if child.type == "attribute"),
                None,
            )
            attribute_nodes = [attribute] if attribute is not None else []
            exports_macro = False
            while attribute_nodes:
                attribute_node = attribute_nodes.pop()
                if (
                    attribute_node.type == "identifier"
                    and node_text(attribute_node) == b"macro_export"
                ):
                    exports_macro = True
                    break
                attribute_nodes.extend(attribute_node.named_children)
            if exports_macro:
                errors.append(f"{owner}: macro_export leaves the transport module")

        visibility = next(
            (child for child in node.named_children if child.type == "visibility_modifier"),
            None,
        )
        if visibility is not None:
            name = node.child_by_field_name("name")
            public_name = node_text(name) if name is not None else b""
            legacy_export = (
                node.parent is not None
                and node.parent.type == "source_file"
                and node_text(visibility) == b"pub"
                and (node.type, public_name)
                in {
                    ("struct_item", b"UnicaServer"),
                    ("function_item", b"tool_definitions"),
                    ("function_item", b"run_stdio"),
                }
            )
            root_run_stdio = (
                legacy_export
                and node.type == "function_item"
                and public_name == b"run_stdio"
            )
            if root_run_stdio:
                root_run_stdio_count += 1
                parameters = node.child_by_field_name("parameters")
                return_type = node.child_by_field_name("return_type")
                type_parameters = node.child_by_field_name("type_parameters")
                where_clause = next(
                    (
                        child
                        for child in node.named_children
                        if child.type == "where_clause"
                    ),
                    None,
                )
                parameter_types = (
                    [
                        re.sub(rb"\s+", b"", node_text(parameter.child_by_field_name("type"))).decode()
                        for parameter in parameters.named_children
                        if parameter.type == "parameter"
                    ]
                    if parameters is not None
                    else []
                )
                if (
                    parameters is None
                    or parameters.named_child_count != 1
                    or parameter_types != [RUN_STDIO_PARAMETER]
                    or return_type is not None
                    or type_parameters is not None
                    or where_clause is not None
                ):
                    errors.append(
                        f"{owner}: run_stdio must be non-generic, take only the daemon "
                        "connector fn() -> Result<V5DaemonClient, String>, and return unit"
                    )
            elif not legacy_export:
                nested = (
                    "nested "
                    if node.type == "function_item"
                    and node.parent is not None
                    and node.parent.type != "source_file"
                    else ""
                )
                errors.append(
                    f"{owner}: {nested}public {node.type} is outside run_stdio"
                )
        stack.extend(reversed(node.named_children))
    if root_run_stdio_count != 1:
        errors.append(
            f"{owner}: expected exactly one root pub fn run_stdio, "
            f"found {root_run_stdio_count}"
        )
    return errors


def workspace_rmcp_owner_export_boundary_errors(repo_root: Path) -> list[str]:
    sources = tracked_workspace_production_rust_sources(repo_root)
    if RMCP_OWNER not in sources:
        return [f"{RMCP_OWNER}: owner is not a tracked workspace production source"]
    return rmcp_owner_export_boundary_errors(sources[RMCP_OWNER], RMCP_OWNER)


def rmcp_dependency_contract_errors(metadata: dict) -> list[str]:
    workspace_members = set(metadata["workspace_members"])
    packages = sorted(
        (
            package
            for package in metadata["packages"]
            if package["id"] in workspace_members
        ),
        key=lambda package: package["name"],
    )
    errors = []
    owner_dependencies = []
    for package in packages:
        dependencies = [
            dependency
            for dependency in package["dependencies"]
            if dependency["name"] == "rmcp"
        ]
        if package["name"] == RMCP_PACKAGE:
            owner_dependencies.extend(dependencies)
            continue
        for dependency in dependencies:
            alias = dependency.get("rename") or "rmcp"
            errors.append(
                f"{package['name']}: rmcp dependency is outside {RMCP_PACKAGE} "
                f"(alias {alias})"
            )

    if len(owner_dependencies) != 1:
        errors.append(
            f"{RMCP_PACKAGE}: expected exactly one direct rmcp dependency, "
            f"found {len(owner_dependencies)}"
        )
        return errors

    dependency = owner_dependencies[0]
    if dependency.get("rename") is not None:
        errors.append(f"{RMCP_PACKAGE}: rmcp dependency must use its canonical name")
    if dependency.get("source") != RMCP_CRATES_IO_SOURCE:
        errors.append(f"{RMCP_PACKAGE}: rmcp dependency must come from crates.io")
    if dependency.get("uses_default_features") is not False:
        errors.append(f"{RMCP_PACKAGE}: rmcp default features must be disabled")
    features = sorted(dependency.get("features", []))
    if features != RMCP_ALLOWED_FEATURES:
        errors.append(
            f"{RMCP_PACKAGE}: rmcp features {features!r} differ from "
            f"{RMCP_ALLOWED_FEATURES!r}"
        )
    return errors


def workspace_rmcp_dependency_errors(repo_root: Path) -> list[str]:
    return rmcp_dependency_contract_errors(cargo_workspace_metadata(repo_root))


RMCP_HANDLER_ASSERTION = "assert_unica_server_implements_official_rmcp_server_handler"
def top_level_rust_items(source: bytes) -> tuple[list[tuple[str, str, list[str], bytes]], bool]:
    """Top-level items with their outer attributes, and whether a file-level cfg gates them."""
    code = productive_rust_code(source)
    tree = Parser(Language(tree_sitter_rust.language())).parse(code)
    items, attributes, gated = [], [], False
    for node in tree.root_node.named_children:
        text = re.sub(rb"\s+", b"", code[node.start_byte : node.end_byte])
        if node.type == "inner_attribute_item":
            gated = gated or text.startswith(b"#![cfg")
        elif node.type == "attribute_item":
            attributes.append(text.decode())
        elif node.type.endswith("comment"):
            continue
        else:
            name = node.child_by_field_name("name")
            items.append((
                node.type,
                code[name.start_byte : name.end_byte].decode() if name else "",
                attributes,
                text,
            ))
            attributes = []
    return items, gated


def production_rmcp_handler_assertion_errors(sources: dict[str, bytes]) -> list[str]:
    # Любая не-тестовая сборка библиотеки проверяет эту границу компилятором;
    # здесь — только что граница и путь модулей к ней не закрыты cfg.
    # Владелец — корень крейта: путь модулей к нему не закрыть cfg.
    errors = []
    items, gated = top_level_rust_items(sources[RMCP_OWNER])
    bounds = [
        (attrs, text)
        for kind, name, attrs, text in items
        if (kind, name) == ("function_item", RMCP_HANDLER_ASSERTION)
    ]
    if (
        gated
        or len(bounds) != 1
        or bounds[0][0] != ["#[allow(dead_code)]"]
        or not bounds[0][1].endswith(b"whereUnicaServer:::rmcp::ServerHandler,{}")
    ):
        errors.append(
            f"{RMCP_OWNER}: production code must assert UnicaServer: rmcp::ServerHandler "
            "once at module level without cfg"
        )
    return errors


def load_ci_rust_lanes_module():
    scripts = Path(__file__).resolve().parents[2] / "scripts" / "ci"
    spec = importlib.util.spec_from_file_location("ci_rust_lanes", scripts / "ci_rust_lanes.py")
    if spec is None or spec.loader is None:
        raise RuntimeError("failed to load ci_rust_lanes.py")
    module = importlib.util.module_from_spec(spec)
    with patch("sys.path", [str(scripts), *sys.path]):
        spec.loader.exec_module(module)
    return module


def load_contract_module():
    module_path = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "check-tool-contracts.py"
    spec = importlib.util.spec_from_file_location("check_tool_contracts", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {module_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ProductContractTests(unittest.TestCase):
    def test_rmcp_confinement_scans_other_tracked_workspace_crates(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            owner = root / "crates" / "unica-mcp" / "src" / "lib.rs"
            sibling = root / "crates" / "unica-bootstrap" / "src" / "lib.rs"
            owner.parent.mkdir(parents=True)
            sibling.parent.mkdir(parents=True)
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["crates/unica-bootstrap", "crates/unica-mcp"]\n',
                encoding="utf-8",
            )
            for package_name in ("unica-bootstrap", "unica-mcp"):
                (root / "crates" / package_name / "Cargo.toml").write_text(
                    f'[package]\nname = "{package_name}"\nversion = "0.1.0"\n'
                    'edition = "2021"\n',
                    encoding="utf-8",
                )
            owner.write_text(
                "use rmcp::ServerHandler;\n"
                "struct UnicaServer;\n"
                "impl ServerHandler for UnicaServer {}\n",
                encoding="utf-8",
            )
            sibling.write_text("use rmcp::model::ProtocolVersion;\n", encoding="utf-8")
            subprocess.run(["git", "init", "-q"], cwd=root, check=True)
            subprocess.run(["git", "add", "."], cwd=root, check=True)

            self.assertEqual(
                workspace_rmcp_reference_confinement_errors(root),
                [
                    "crates/unica-bootstrap/src/lib.rs: productive rmcp "
                    "reference outside transport owner"
                ],
            )

    def test_tracked_workspace_sources_include_root_package(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "src" / "lib.rs"
            source.parent.mkdir(parents=True)
            (root / "Cargo.toml").write_text(
                '[package]\nname = "root-package"\nversion = "0.1.0"\n'
                '[workspace]\nmembers = []\n',
                encoding="utf-8",
            )
            source.write_text("use rmcp::model::ProtocolVersion;\n", encoding="utf-8")
            subprocess.run(["git", "init", "-q"], cwd=root, check=True)
            subprocess.run(["git", "add", "."], cwd=root, check=True)

            self.assertEqual(
                set(tracked_workspace_production_rust_sources(root)),
                {"src/lib.rs"},
            )

    def test_tracked_workspace_sources_expand_member_globs_and_excludes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            included = root / "crates" / "included" / "src" / "lib.rs"
            excluded = root / "crates" / "excluded" / "src" / "lib.rs"
            included.parent.mkdir(parents=True)
            excluded.parent.mkdir(parents=True)
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["crates/*"]\n'
                'exclude = ["crates/excluded"]\n',
                encoding="utf-8",
            )
            for package_name in ("included", "excluded"):
                (root / "crates" / package_name / "Cargo.toml").write_text(
                    f'[package]\nname = "{package_name}"\nversion = "0.1.0"\n'
                    'edition = "2021"\n',
                    encoding="utf-8",
                )
            included.write_text("pub struct Included;\n", encoding="utf-8")
            excluded.write_text("pub struct Excluded;\n", encoding="utf-8")
            subprocess.run(["git", "init", "-q"], cwd=root, check=True)
            subprocess.run(["git", "add", "."], cwd=root, check=True)

            self.assertEqual(
                set(tracked_workspace_production_rust_sources(root)),
                {"crates/included/src/lib.rs"},
            )

    def test_tracked_workspace_sources_include_implicit_path_dependency_member(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            app = root / "app"
            shared = root / "shared"
            (app / "src").mkdir(parents=True)
            (shared / "src").mkdir(parents=True)
            (root / "Cargo.toml").write_text(
                '[workspace]\nresolver = "2"\nmembers = ["app"]\n',
                encoding="utf-8",
            )
            (app / "Cargo.toml").write_text(
                '[package]\nname = "app"\nversion = "0.1.0"\nedition = "2021"\n'
                '[dependencies]\nshared = { path = "../shared" }\n',
                encoding="utf-8",
            )
            (shared / "Cargo.toml").write_text(
                '[package]\nname = "shared"\nversion = "0.1.0"\nedition = "2021"\n',
                encoding="utf-8",
            )
            (app / "src" / "lib.rs").write_text(
                "pub fn app() {}\n", encoding="utf-8"
            )
            (shared / "src" / "lib.rs").write_text(
                "pub fn shared() {}\n", encoding="utf-8"
            )
            subprocess.run(["git", "init", "-q"], cwd=root, check=True)
            subprocess.run(["git", "add", "."], cwd=root, check=True)

            self.assertEqual(
                set(tracked_workspace_production_rust_sources(root)),
                {"app/src/lib.rs", "shared/src/lib.rs"},
            )

    def test_rmcp_dependency_contract_rejects_alias_in_sibling_package(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            coder = root / "unica-mcp"
            sibling = root / "sibling"
            vendor = root / "vendor" / "rmcp"
            for package in (coder, sibling, vendor):
                (package / "src").mkdir(parents=True)
                (package / "src" / "lib.rs").write_text("", encoding="utf-8")
            (root / "Cargo.toml").write_text(
                '[workspace]\nresolver = "2"\n'
                'members = ["unica-mcp", "sibling"]\n'
                '[workspace.dependencies]\n'
                'rmcp = { version = "3.1.2", default-features = false, '
                'features = ["server", "transport-io"] }\n'
                'sdk_alias = { package = "rmcp", version = "3.1.2", '
                'default-features = false, features = ["server", "transport-io"] }\n'
                '[patch.crates-io]\nrmcp = { path = "vendor/rmcp" }\n',
                encoding="utf-8",
            )
            (coder / "Cargo.toml").write_text(
                '[package]\nname = "unica-mcp"\nversion = "0.1.0"\n'
                'edition = "2021"\n[dependencies]\nrmcp.workspace = true\n',
                encoding="utf-8",
            )
            (sibling / "Cargo.toml").write_text(
                '[package]\nname = "sibling"\nversion = "0.1.0"\n'
                'edition = "2021"\n[dependencies]\nsdk_alias.workspace = true\n',
                encoding="utf-8",
            )
            (vendor / "Cargo.toml").write_text(
                '[package]\nname = "rmcp"\nversion = "3.1.2"\nedition = "2021"\n'
                '[features]\ndefault = []\nserver = []\ntransport-io = []\n',
                encoding="utf-8",
            )
            subprocess.run(["git", "init", "-q"], cwd=root, check=True)
            subprocess.run(["git", "add", "."], cwd=root, check=True)

            self.assertEqual(
                workspace_rmcp_dependency_errors(root),
                ["sibling: rmcp dependency is outside unica-mcp (alias sdk_alias)"],
            )

    def test_rmcp_dependency_contract_rejects_renamed_local_macro_dependency(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            coder = root / "unica-mcp"
            vendor = root / "vendor" / "rmcp"
            for package in (coder, vendor):
                (package / "src").mkdir(parents=True)
                (package / "src" / "lib.rs").write_text("", encoding="utf-8")
            (root / "Cargo.toml").write_text(
                '[workspace]\nresolver = "2"\nmembers = ["unica-mcp"]\n',
                encoding="utf-8",
            )
            (coder / "Cargo.toml").write_text(
                '[package]\nname = "unica-mcp"\nversion = "0.1.0"\n'
                'edition = "2021"\n[dependencies]\n'
                'sdk_alias = { package = "rmcp", path = "../vendor/rmcp", '
                'features = ["server", "transport-io", "macros"] }\n',
                encoding="utf-8",
            )
            (vendor / "Cargo.toml").write_text(
                '[package]\nname = "rmcp"\nversion = "3.1.2"\nedition = "2021"\n'
                '[features]\ndefault = []\nserver = []\ntransport-io = []\nmacros = []\n',
                encoding="utf-8",
            )
            subprocess.run(["git", "init", "-q"], cwd=root, check=True)
            subprocess.run(["git", "add", "."], cwd=root, check=True)

            self.assertEqual(
                workspace_rmcp_dependency_errors(root),
                [
                    "unica-mcp: rmcp dependency must use its canonical name",
                    "unica-mcp: rmcp dependency must come from crates.io",
                    "unica-mcp: rmcp default features must be disabled",
                    "unica-mcp: rmcp features ['macros', 'server', "
                    "'transport-io'] differ from ['server', 'transport-io']",
                ],
            )

    def test_rmcp_owner_rejects_public_reexport_and_type_alias(self) -> None:
        owner = RMCP_OWNER
        errors = rmcp_owner_export_boundary_errors(
            b"""
                pub fn run_stdio(connect: fn() -> Result<V5DaemonClient, String>) {}
                pub use rmcp::model::Tool as ExportedTool;
                pub type SdkTool = rmcp::model::Tool;
            """,
            owner,
        )

        self.assertEqual(
            errors,
            [
                f"{owner}: public use_declaration is outside run_stdio",
                f"{owner}: public type_item is outside run_stdio",
            ],
        )

    def test_rmcp_owner_rejects_other_public_items_members_and_macros(self) -> None:
        owner = RMCP_OWNER
        errors = rmcp_owner_export_boundary_errors(
            b"""
                pub(crate) const LIMIT: usize = 1;
                struct Internal { pub value: usize }
                pub struct Exported;
                #[macro_export]
                macro_rules! exported { () => {}; }
                mod nested { pub fn run_stdio(connect: fn() -> Result<V5DaemonClient, String>) {} }
                pub fn run_stdio(_: rmcp::model::Tool) -> rmcp::model::Tool {
                    unreachable!()
                }
            """,
            owner,
        )

        self.assertEqual(
            errors,
            [
                f"{owner}: public const_item is outside run_stdio",
                f"{owner}: public field_declaration is outside run_stdio",
                f"{owner}: public struct_item is outside run_stdio",
                f"{owner}: macro_export leaves the transport module",
                f"{owner}: nested public function_item is outside run_stdio",
                f"{owner}: run_stdio must be non-generic, take only the daemon "
                "connector fn() -> Result<V5DaemonClient, String>, and return unit",
            ],
        )

    def test_rmcp_owner_ignores_exports_inside_exact_cfg_test_item(self) -> None:
        owner = RMCP_OWNER
        errors = rmcp_owner_export_boundary_errors(
            b"""
                pub fn run_stdio(connect: fn() -> Result<V5DaemonClient, String>) {}
                #[cfg(test)]
                mod tests {
                    pub use rmcp::model::Tool;
                    pub type SdkTool = rmcp::model::Tool;
                }
            """,
            owner,
        )

        self.assertEqual(errors, [])

    def test_rmcp_owner_ignores_exports_inside_test_support_item(self) -> None:
        """rule: INV.WIRE.SDK-MODULE-EXPORTS"""
        owner = RMCP_OWNER
        errors = rmcp_owner_export_boundary_errors(
            b"""
                pub fn run_stdio(connect: fn() -> Result<V5DaemonClient, String>) {}
                #[cfg(any(test, feature = "test-support"))]
                pub mod test_support {
                    pub use rmcp;
                }
                #[cfg(any(test, feature = "test-support"))]
                impl UnicaServer {
                    pub fn with_canonical_v13_router(router: Router) -> Self { todo!() }
                }
                #[cfg(feature = "test-support")]
                pub fn only_with_the_feature() {}
            """,
            owner,
        )

        self.assertEqual(
            errors, [f"{owner}: public function_item is outside run_stdio"]
        )

    def test_rmcp_owner_requires_the_daemon_connector_parameter(self) -> None:
        """rule: INV.WIRE.SDK-MODULE-EXPORTS"""
        owner = RMCP_OWNER
        expected = [
            f"{owner}: run_stdio must be non-generic, take only the daemon "
            "connector fn() -> Result<V5DaemonClient, String>, and return unit"
        ]
        for name, source in {
            "no_parameter": b"pub fn run_stdio() {}",
            "other_type": b"pub fn run_stdio(connect: fn() -> V5DaemonClient) {}",
            "two_parameters": (
                b"pub fn run_stdio(connect: fn() -> Result<V5DaemonClient, String>, "
                b"extra: u8) {}"
            ),
        }.items():
            with self.subTest(name=name):
                self.assertEqual(rmcp_owner_export_boundary_errors(source, owner), expected)

    def test_rmcp_owner_export_guard_fails_closed_on_invalid_rust(self) -> None:
        owner = RMCP_OWNER

        self.assertEqual(
            rmcp_owner_export_boundary_errors(b"pub fn run_stdio(", owner),
            [f"{owner}: Rust parser could not resolve the export boundary"],
        )

    def test_rmcp_owner_rejects_parameterized_macro_export(self) -> None:
        owner = RMCP_OWNER

        self.assertEqual(
            rmcp_owner_export_boundary_errors(
                b"""
                    pub fn run_stdio(connect: fn() -> Result<V5DaemonClient, String>) {}
                    #[macro_export(local_inner_macros)]
                    macro_rules! exported { () => {}; }
                """,
                owner,
            ),
            [f"{owner}: macro_export leaves the transport module"],
        )

    def test_rmcp_owner_rejects_conditional_macro_export(self) -> None:
        owner = RMCP_OWNER

        self.assertEqual(
            rmcp_owner_export_boundary_errors(
                b"""
                    pub fn run_stdio(connect: fn() -> Result<V5DaemonClient, String>) {}
                    #[cfg_attr(not(test), macro_export)]
                    macro_rules! exported { () => {}; }
                """,
                owner,
            ),
            [f"{owner}: macro_export leaves the transport module"],
        )

    def test_rmcp_owner_requires_exactly_one_root_run_stdio(self) -> None:
        owner = RMCP_OWNER
        fixtures = {
            "missing": (
                b"struct Internal;",
                [f"{owner}: expected exactly one root pub fn run_stdio, found 0"],
            ),
            "duplicate": (
                b"pub fn run_stdio(connect: fn() -> Result<V5DaemonClient, String>) {} pub fn run_stdio(connect: fn() -> Result<V5DaemonClient, String>) {}",
                [f"{owner}: expected exactly one root pub fn run_stdio, found 2"],
            ),
            "nested-only": (
                b"mod nested { pub fn run_stdio(connect: fn() -> Result<V5DaemonClient, String>) {} }",
                [
                    f"{owner}: nested public function_item is outside run_stdio",
                    f"{owner}: expected exactly one root pub fn run_stdio, found 0",
                ],
            ),
        }

        for name, (source, expected) in fixtures.items():
            with self.subTest(name=name):
                self.assertEqual(
                    rmcp_owner_export_boundary_errors(source, owner), expected
                )

    def test_rmcp_owner_rejects_sdk_bound_on_generic_run_stdio(self) -> None:
        owner = RMCP_OWNER

        self.assertEqual(
            rmcp_owner_export_boundary_errors(
                b"pub fn run_stdio<T: rmcp::ServerHandler>(connect: fn() -> Result<V5DaemonClient, String>) {}",
                owner,
            ),
            [
                f"{owner}: run_stdio must be non-generic, take only the daemon "
                "connector fn() -> Result<V5DaemonClient, String>, and return unit"
            ],
        )

    def test_rmcp_dependency_is_owned_by_unica_coder_without_macro_features(
        self,
    ) -> None:
        """rule: INV.WIRE.SDK-TRANSPORT"""
        self.assertEqual(workspace_rmcp_dependency_errors(REPO_ROOT), [])

    def test_production_rmcp_handler_assertion_rejects_test_only_or_gated_bound(
        self,
    ) -> None:
        """rule: INV.WIRE.SDK-TRANSPORT"""
        bound = (
            b"#[allow(dead_code)]\n"
            b"fn assert_unica_server_implements_official_rmcp_server_handler()\n"
            b"where\n    UnicaServer: ::rmcp::ServerHandler,\n{\n}\n"
        )
        owner_error = [
            f"{RMCP_OWNER}: production code must assert UnicaServer: rmcp::ServerHandler "
            "once at module level without cfg"
        ]
        cfg = b'#[cfg(feature = "research")]\n'
        fixtures = {
            "production": ({RMCP_OWNER: b"struct UnicaServer;\n" + bound}, []),
            "absent": ({RMCP_OWNER: b"struct UnicaServer;\n"}, owner_error),
            "test_only": ({RMCP_OWNER: b"#[cfg(test)]\n" + bound}, owner_error),
            "commented": ({RMCP_OWNER: b"/*\n" + bound + b"*/\n"}, owner_error),
            "duplicated": ({RMCP_OWNER: bound + bound}, owner_error),
            "feature_gated": ({RMCP_OWNER: cfg + bound}, owner_error),
            "nested_gated_module": (
                {RMCP_OWNER: cfg + b"mod gated {\n" + bound + b"}\n"},
                owner_error,
            ),
            "file_gated": ({RMCP_OWNER: b'#![cfg(feature = "research")]\n' + bound}, owner_error),
            "weakened_bound": (
                {RMCP_OWNER: bound.replace(b"::rmcp::ServerHandler", b"Clone")},
                owner_error,
            ),
        }
        for name, (sources, expected) in fixtures.items():
            with self.subTest(name=name):
                self.assertEqual(
                    production_rmcp_handler_assertion_errors(sources), expected
                )

    def test_unica_coder_production_library_satisfies_rmcp_handler_bound(
        self,
    ) -> None:
        # Граница — проверка компилятора в production-коде библиотеки. Python-набор
        # её не собирает: в каждом профиле, включая PR, это делает ordinary-архив
        # Rust (не-тестовый бинарь `unica`, фичи объединены с dev-зависимостями
        # тестов), а вне PR и push в main — шаг `product` точным
        # production-набором фич (`cargo build -p unica-product --bin unica`).
        """rule: INV.WIRE.SDK-TRANSPORT"""
        sources = {RMCP_OWNER: (REPO_ROOT / RMCP_OWNER).read_bytes()}
        self.assertEqual(production_rmcp_handler_assertion_errors(sources), [])

        lanes = load_ci_rust_lanes_module()
        for profile in ("pr", "queue", "main", "release", "large", "all"):
            with self.subTest(profile=profile):
                self.assertIn("ordinary", lanes.lanes(profile))
        command = lanes.archive_command("ordinary", Path("tests.tar.zst"))
        self.assertEqual(lanes.selection("ordinary"), ("--workspace",))
        self.assertFalse(
            {"--features", "--all-features", "--no-default-features", "--lib", "--tests"}
            & set(command)
        )
        self.assertEqual(lanes.prepared_binary.ORDINARY_FEATURES, ["default"])

    def test_rmcp_module_preserves_legacy_public_exports_only(self) -> None:
        """rule: INV.WIRE.SDK-MODULE-EXPORTS"""
        source = tracked_workspace_production_rust_sources(REPO_ROOT)[RMCP_OWNER]
        for declaration in (
            b"pub struct UnicaServer",
            b"pub fn tool_definitions(",
            b"pub fn run_stdio(connect: fn() -> Result<V5DaemonClient, String>)",
        ):
            with self.subTest(declaration=declaration):
                self.assertIn(declaration, source)
        self.assertEqual(workspace_rmcp_owner_export_boundary_errors(REPO_ROOT), [])

    def test_rmcp_confinement_ignores_comments_literals_and_exact_cfg_test(
        self,
    ) -> None:
        owner = RMCP_OWNER
        outside = "crates/other/src/lib.rs"
        errors = rmcp_reference_confinement_errors(
            {
                owner: b"use rmcp::ServerHandler;",
                outside: b'''
                    // use rmcp::ServerHandler;
                    const TEXT: &str = "rmcp::ServerHandler";
                    const RAW: &str = r#"rmcp::ServerHandler"#;
                    #[cfg(test)]
                    mod tests { use rmcp::ServerHandler; }
                ''',
            },
            owner,
        )

        self.assertEqual(errors, [])

    def test_rmcp_confinement_ignores_a_test_only_file(self) -> None:
        """rule: INV.WIRE.SDK-TRANSPORT"""
        owner = RMCP_INTERFACE_ROOT
        errors = rmcp_reference_confinement_errors(
            {
                "crates/unica-daemon/src/interfaces/mcp_door_tests/mcp.rs": (
                    b"#![cfg(test)]\nuse unica_mcp::test_support::rmcp::model::Tool;\n"
                ),
                "crates/unica-daemon/src/interfaces/daemon.rs": (
                    b'#![cfg(feature = "test-support")]\nuse rmcp::model::Tool;\n'
                ),
            },
            owner,
        )

        self.assertEqual(
            errors,
            [
                "crates/unica-daemon/src/interfaces/daemon.rs: productive rmcp "
                "reference outside transport owner"
            ],
        )

    def test_rmcp_transport_is_confined_to_mcp_interface(self) -> None:
        """rule: INV.WIRE.SDK-TRANSPORT"""
        self.assertEqual(
            workspace_rmcp_reference_confinement_errors(REPO_ROOT),
            [],
            "productive rmcp references must stay in unica-mcp",
        )

    def test_native_validators_do_not_expose_internal_local_owner_only_switch(
        self,
    ) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        offenders = []
        roots = [repo_root / "crates" / crate / "src" for crate in ("unica-coder", "unica-daemon")]
        for path in sorted(path for root in roots for path in root.rglob("*.rs")):
            text = path.read_text(encoding="utf-8")
            for marker in ("InternalLocalOwnerOnly", "internalLocalOwnerOnly"):
                if marker in text:
                    offenders.append(
                        f"{path.relative_to(repo_root).as_posix()}: {marker}"
                    )
        self.assertEqual(offenders, [])

    def test_v8_runner_partial_load_list_requires_bom_crlf_and_cyrillic_path(self) -> None:
        module = load_contract_module()
        expected_path = str(
            Path("Catalogs.Товары") / "Ext" / "ObjectModule.bsl"
        )
        payload = b"\xef\xbb\xbf" + expected_path.encode("utf-8") + b"\r\n"

        self.assertEqual(
            module.validate_v8_runner_partial_load_list(payload, expected_path),
            [],
        )
        self.assertIn(
            "UTF-8 BOM",
            "\n".join(
                module.validate_v8_runner_partial_load_list(
                    payload.removeprefix(b"\xef\xbb\xbf"),
                    expected_path,
                )
            ),
        )
        self.assertIn(
            "CRLF",
            "\n".join(
                module.validate_v8_runner_partial_load_list(
                    b"\xef\xbb\xbf" + expected_path.encode("utf-8") + b"\n",
                    expected_path,
                )
            ),
        )

    def test_v8_runner_full_pull_windows_verbatim_paths_keep_the_exact_target(self) -> None:
        module = load_contract_module()
        normalize = module._ordinary_v8_runner_publication_path
        for ordinary, reported in [
            (r"C:\guard\1cv8.exe", r"\\?\C:\guard\1cv8.exe"),
            (r"\\server\share\sources", r"\\?\UNC\server\share\sources"),
            (r"C:\guard\stage", "//?/C:/guard/stage"),
        ]:
            with self.subTest(reported=reported):
                self.assertEqual(normalize(reported), ordinary)
        self.assertEqual(normalize(r"C:\guard\1cv8.exe"), r"C:\guard\1cv8.exe")
        self.assertNotEqual(normalize(r"\\?\C:\other\1cv8.exe"), r"C:\guard\1cv8.exe")

    def test_v8_runner_full_pull_frames_require_nonce_executor_and_prepared_target(self) -> None:
        module = load_contract_module()
        nonce = "11111111-1111-4111-8111-111111111111"
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw).resolve()
            source, stage, platform = [root / name for name in ("sources", "stage", "1cv8")]
            selected = {"phase": "selected", "version": 1, "nonce": nonce,
                        "provider": {"selected": "designer", "origin": {"kind": "override", "file": "v8project.yaml"}},
                        "executor": {"kind": "local", "executable": str(platform), "endpoint": None}}
            ready = {"phase": "ready", "version": 1, "nonce": nonce, "command": "pull",
                     "requested_mode": "FULL", "mode": "FULL", "mode_reason": None,
                     "extension": None, "provider_dispatched": True, "source_set": "main",
                     "target_path": str(source), "stage_path": str(stage),
                     "snapshot": "prepared", "snapshot_warning": None}
            terminal = {"phase": "terminal", "version": 1, "nonce": nonce,
                        "publication": "published", "memory": "saved", "result": {
                            "ok": True, "command": "pull", "duration_ms": 0,
                            "data": {}, "warnings": [], "steps": []}}
            for value in [selected, ready, terminal]:
                self.assertEqual(module.validate_v8_runner_publication_frame(
                    value, value["phase"], nonce, source, stage, platform), [])
            defects = [
                (selected, {"nonce": "not-a-uuid"}),
                (selected, {"executor": {**selected["executor"], "kind": "attached"}}),
                (selected, {"executor": {**selected["executor"], "executable": str(root / "other")}}),
                (selected, {"executor": {**selected["executor"], "endpoint": "foreign:1545"}}),
                (selected, {"provider": {"selected": "agent"}}),
                (ready, {"nonce": "22222222-2222-4222-8222-222222222222"}),
                (ready, {"version": True}),
                (ready, {"version": 2}),
                (ready, {"phase": "terminal"}),
                (ready, {"target_path": str(root / "other")}),
                (ready, {"stage_path": "relative-stage"}),
                (ready, {"snapshot": "unavailable"}),
                (ready, {"provider_dispatched": False}),
                (ready, {"source_set": "other"}),
                (ready, {"unexpected": True}),
                (terminal, {"nonce": "22222222-2222-4222-8222-222222222222"}),
            ]
            for value, defect in defects:
                with self.subTest(phase=value["phase"], fields=sorted(defect)):
                    self.assertTrue(module.validate_v8_runner_publication_frame(
                        {**value, **defect}, value["phase"], nonce, source, stage, platform))

    def test_v8_runner_full_pull_selected_requires_closed_configured_provider_receipt(self) -> None:
        module = load_contract_module()
        nonce = "11111111-1111-4111-8111-111111111111"
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw).resolve()
            source, stage, platform = [root / name for name in ("sources", "stage", "1cv8")]
            origin = {"kind": "override", "file": "v8project.yaml"}
            provider = {"selected": "designer", "origin": origin}
            selected = {"phase": "selected", "version": 1, "nonce": nonce,
                        "provider": provider,
                        "executor": {"kind": "local", "executable": str(platform), "endpoint": None}}
            for receipt in [provider, {**provider, "skipped": [], "endpoint": None}]:
                self.assertEqual(module.validate_v8_runner_publication_frame(
                    {**selected, "provider": receipt}, "selected", nonce, source, stage, platform), [])
            invalid_receipts = [
                {"selected": "designer"},
                {**provider, "origin": None},
                {**provider, "origin": {"kind": "override"}},
                {**provider, "origin": {"kind": "default"}},
                {**provider, "origin": {**origin, "file": "v8project.local.yaml"}},
                {**provider, "origin": {**origin, "file": None}},
                {**provider, "origin": {**origin, "unexpected": True}},
                {**provider, "unexpected": True},
                {**provider, "skipped": None},
                {**provider, "skipped": [{"provider": "agent", "reason": "not this explicit override"}]},
                {**provider, "endpoint": {"mode": "attached", "address": "other:1545"}},
            ]
            for receipt in invalid_receipts:
                with self.subTest(receipt=receipt):
                    self.assertTrue(module.validate_v8_runner_publication_frame(
                        {**selected, "provider": receipt}, "selected", nonce, source, stage, platform))
            for executor in [
                {"kind": "local", "executable": str(platform)},
                {**selected["executor"], "unexpected": True},
            ]:
                with self.subTest(executor=executor):
                    self.assertTrue(module.validate_v8_runner_publication_frame(
                        {**selected, "executor": executor}, "selected", nonce, source, stage, platform))

    def test_v8_runner_full_pull_terminal_requires_closed_common_envelope(self) -> None:
        module = load_contract_module()
        nonce = "11111111-1111-4111-8111-111111111111"
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw).resolve()
            source, stage, platform = [root / name for name in ("sources", "stage", "1cv8")]
            result = {"ok": True, "command": "pull", "duration_ms": 0,
                      "data": {}, "warnings": [], "steps": []}
            terminal = {"phase": "terminal", "version": 1, "nonce": nonce,
                        "publication": "published", "memory": "saved", "result": result}
            failure = {**result, "ok": False, "error": {
                "code": "runtime_failure", "kind": "runtime", "message": "control input closed"}}
            for envelope in [result, {**result, "error": None}, {**result, "warnings": ["diagnostic"]}, failure]:
                self.assertEqual(module.validate_v8_runner_publication_frame(
                    {**terminal, "result": envelope}, "terminal", nonce, source, stage, platform), [])
            invalid_results = [
                {key: value for key, value in result.items() if key != removed}
                for removed in result
            ] + [
                {**result, "unexpected": True},
                {**result, "ok": 1},
                {**result, "command": None},
                {**result, "duration_ms": True},
                {**result, "duration_ms": -1},
                {**result, "duration_ms": "0"},
                {**result, "warnings": {}},
                {**result, "warnings": [1]},
                {**result, "steps": {}},
                {**result, "steps": [{}]},
                {**result, "error": []},
                {**result, "ok": False},
                {**failure, "error": {"code": "runtime_failure", "kind": "runtime"}},
                {**failure, "error": {**failure["error"], "unexpected": True}},
                {**failure, "error": {**failure["error"], "code": True}},
                {**failure, "error": {**failure["error"], "kind": "platform"}},
                {**failure, "error": {**failure["error"], "message": ""}},
            ]
            for envelope in invalid_results:
                with self.subTest(envelope=envelope):
                    self.assertTrue(module.validate_v8_runner_publication_frame(
                        {**terminal, "result": envelope}, "terminal", nonce, source, stage, platform))

    def test_v8_runner_full_pull_preview_cannot_hide_missing_memory_or_a_late_edit(self) -> None:
        module = load_contract_module()
        def envelope(mode):
            return {"ok": True, "command": "push", "data": {
                "ok": True, "provider_dispatched": False,
                "steps": [{"source_set": "main", "mode": mode, "ok": True, "applied": False}]}}
        unchanged = envelope("skipped")
        changed = envelope({"partial": {"file_count": 1}})
        full = envelope("full")
        self.assertEqual(module.validate_v8_runner_post_pull_preview(unchanged, 0, changed=False), [])
        self.assertEqual(module.validate_v8_runner_post_pull_preview(changed, 0, changed=True), [])
        self.assertEqual(module.validate_v8_runner_post_pull_preview(full, 0, changed=True), [])
        cases = [
            (unchanged, 0, True), (changed, 0, False),
            (full, 0, False),
            ({**unchanged, "ok": False, "error": {"code": "no_memory"}}, 2, False),
            ({**unchanged, "data": {**unchanged["data"], "steps": []}}, 0, False),
            ({**unchanged, "data": {**unchanged["data"], "provider_dispatched": True}}, 0, False),
            (envelope(None), 0, True),
            (envelope({"partial": {"file_count": 0}}), 0, True),
            (envelope({"partial": {"file_count": True}}), 0, True),
        ]
        for index, (value, status, expect_changed) in enumerate(cases):
            with self.subTest(case=index):
                self.assertTrue(module.validate_v8_runner_post_pull_preview(value, status, changed=expect_changed))

    def test_v8_runner_full_pull_preserves_every_prepared_file(self) -> None:
        module = load_contract_module()
        prepared = {"Configuration.xml": b"configuration", "Module.bsl": b"prepared",
                    "ConfigDumpInfo.xml": b"dump info"}
        late_edit = {**prepared, "Module.bsl": b"late edit"}
        self.assertEqual(module.validate_v8_runner_publication_tree(dict(prepared), prepared), [])
        self.assertEqual(module.validate_v8_runner_publication_tree(dict(late_edit), late_edit), [])
        defects = [
            {name: payload for name, payload in prepared.items() if name != "Configuration.xml"},
            {**prepared, "Configuration.xml": b"corrupted configuration"},
            {name: payload for name, payload in prepared.items() if name != "ConfigDumpInfo.xml"},
            {**prepared, "ConfigDumpInfo.xml": b"corrupted dump info"},
            {**prepared, "unexpected.bsl": b"extra"},
            {**prepared, "Module.bsl": b"unexpected edit"},
        ]
        for index, tree in enumerate(defects):
            with self.subTest(case=index):
                self.assertTrue(module.validate_v8_runner_publication_tree(tree, prepared))

    def test_v8_runner_full_pull_invalid_frame_still_closes_each_owned_process(self) -> None:
        from io import StringIO
        from queue import Queue
        from types import SimpleNamespace
        module = load_contract_module()
        sessions = []
        def start(command, environment, timeout_seconds, *, cwd, deadline):
            del timeout_seconds, deadline
            calls = Path(environment["UNICA_FULL_PULL_CALLS"])
            if not calls.exists():
                calls.write_bytes(b"probe\n")
            output = {"ok": True} if "pull" in command else {"ok": True, "command": "push", "data": {
                "ok": True, "provider_dispatched": False,
                "steps": [{"source_set": "main", "mode": "skipped", "ok": True, "applied": False}]}}
            if "--publication-stdio" in command:
                output = {"phase": "selected"}  # Invalid before any reply or publication.
            lines = Queue()
            lines.put(json.dumps(output) + "\n")
            lines.put("")
            reader = SimpleNamespace(join=lambda timeout: None, is_alive=lambda: False)
            session = SimpleNamespace(
                process=SimpleNamespace(stdin=StringIO(), wait=lambda timeout: 0),
                lines=lines, reader=reader, error_reader=reader, diagnostics=[])
            sessions.append(session)
            return session
        def close(session, root):
            del root
            session.process.stdin.close()
        with tempfile.TemporaryDirectory() as raw:
            runner = Path(raw) / "v8-runner"
            runner.write_bytes(b"test-only supplied binary stand-in")
            with patch.object(module, "compile_rust_platform_stub", return_value=[]), patch.object(
                module, "load_shared_mcp_smoke_module", return_value=SimpleNamespace(McpSession=start)
            ), patch.object(module, "terminate_shared_mcp_session", side_effect=close) as cleanup:
                errors = module.check_v8_runner_full_pull_publication_contract(runner, "linux-x64")
        self.assertEqual(len(sessions), 3)
        self.assertEqual(cleanup.call_count, 3)
        self.assertTrue(all(session.process.stdin.closed for session in sessions))
        self.assertEqual(len(errors), 1)
        self.assertIn("selected frame", errors[0])

    def test_v8_runner_platform_stub_compilation_timeout_is_bounded(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "platform-stub.rs"
            output = root / "platform-stub.exe"
            source.write_text("fn main() {}\n", encoding="utf-8")
            with patch.object(
                module.subprocess,
                "run",
                side_effect=subprocess.TimeoutExpired(["rustc"], 60),
            ) as compile_run:
                errors = module.compile_rust_platform_stub(
                    source,
                    output,
                    root,
                    "v8-runner fixture",
                )

        self.assertEqual(
            errors,
            [
                "v8-runner fixture: platform stub compilation timed out "
                "after 60 seconds"
            ],
        )
        self.assertEqual(compile_run.call_args.kwargs["timeout"], 60)

    def test_v8_runner_partial_load_smoke_rejects_missing_binary(self) -> None:
        module = load_contract_module()

        errors = module.check_v8_runner_partial_load_contract(
            Path("/missing/v8-runner"),
            "linux-x64",
        )

        self.assertEqual(
            errors,
            ["v8-runner partial-load contract: binary not found: /missing/v8-runner"],
        )

    def test_v8_runner_failed_partial_receipt_requires_exit_four_and_closed_shape(
        self,
    ) -> None:
        module = load_contract_module()
        validator = getattr(
            module,
            "validate_v8_runner_failed_partial_receipt",
            None,
        )
        self.assertIsNotNone(validator)
        message = (
            "load failed for source-set 'main' with exit code 1; "
            "platform log: rejected; platform log path: /tmp/out.log; "
            "partial load list path: /tmp/partial.lst"
        )
        envelope = {
            "ok": False,
            "command": "push",
            "duration_ms": 12,
            "data": {
                "ok": False,
                # v8-runner 0.9.0 (ADR-0028 форка): признак есть у каждого
                # ответа `build` (с 0.12 — `push`); `platform_failure` удостоверяет, что
                # провайдер был вызван, — значит здесь он обязан быть `true`.
                "provider_dispatched": True,
                "provider": {"selected": "designer", "origin": {"kind": "default"}},
                "steps": [
                    {
                        "source_set": "main",
                        "mode": {"partial": {"file_count": 1}},
                        "ok": False,
                        "applied": False,
                        "message": f"platform error: {message}",
                        "duration_ms": 0,
                    }
                ],
                "duration_ms": 12,
            },
            "warnings": [],
            "steps": [],
            "error": {
                "code": "platform_failure",
                "kind": "platform",
                "message": message,
            },
        }

        self.assertEqual(validator(envelope, 4, "main"), [])
        self.assertTrue(
            any("exit code 4" in error for error in validator(envelope, 1, "main"))
        )
        invalid_duration = copy.deepcopy(envelope)
        invalid_duration["data"]["steps"][0]["duration_ms"] = "0"
        self.assertTrue(
            any(
                "duration_ms" in error
                for error in validator(invalid_duration, 4, "main")
            )
        )
        unknown_field = copy.deepcopy(envelope)
        unknown_field["data"]["steps"][0]["mode"]["partial"]["unknown"] = True
        self.assertTrue(
            any("closed" in error for error in validator(unknown_field, 4, "main"))
        )
        undispatched = copy.deepcopy(envelope)
        undispatched["data"]["provider_dispatched"] = False
        self.assertTrue(
            any(
                "provider_dispatched" in error
                for error in validator(undispatched, 4, "main")
            )
        )
        # С 0.15.1 шаг обязан назвать `applied`; отказавшая загрузка ничего
        # не применила.
        pre_0_15_1 = copy.deepcopy(envelope)
        del pre_0_15_1["data"]["steps"][0]["applied"]
        self.assertTrue(
            any("closed" in error for error in validator(pre_0_15_1, 4, "main"))
        )
        applied = copy.deepcopy(envelope)
        applied["data"]["steps"][0]["applied"] = True
        self.assertTrue(
            any("applied=false" in error for error in validator(applied, 4, "main"))
        )
        pre_0_9_0 = copy.deepcopy(envelope)
        del pre_0_9_0["data"]["provider_dispatched"]
        self.assertTrue(
            any("closed" in error for error in validator(pre_0_9_0, 4, "main"))
        )
        # С 0.12 отказ может назвать шаг выхода; прочие лишние ключи — нет.
        with_next = copy.deepcopy(envelope)
        with_next["error"]["next"] = {"command": "push main --full"}
        self.assertEqual(validator(with_next, 4, "main"), [])
        unknown_error_field = copy.deepcopy(envelope)
        unknown_error_field["error"]["hint"] = "retry"
        self.assertTrue(
            any("closed" in error for error in validator(unknown_error_field, 4, "main"))
        )
        # С 0.13 шаг ещё называет судьбу записи поколения набора; другая
        # приписка к сообщению шага — уже не та квитанция.
        generation_note = copy.deepcopy(envelope)
        generation_note["data"]["steps"][0]["message"] = (
            f"platform error: {message}; the configuration generation of "
            "source-set 'main' is not recorded after the failed load"
        )
        self.assertEqual(validator(generation_note, 4, "main"), [])
        other_note = copy.deepcopy(envelope)
        other_note["data"]["steps"][0]["message"] = f"platform error: {message}; retried"
        self.assertTrue(
            any("does not match" in error for error in validator(other_note, 4, "main"))
        )
        previous_command = copy.deepcopy(envelope)
        previous_command["command"] = "build"
        self.assertTrue(
            any("command=push" in error for error in validator(previous_command, 4, "main"))
        )

    def test_v8_runner_zero_source_preview_requires_an_undispatched_launch_receipt(self) -> None:
        module = load_contract_module()
        envelope = {
            "ok": True,
            "command": "launch",
            "data": {
                "ok": True,
                "mode": "thin",
                "plan": {"program": "/fixture/platform/bin/1cv8c"},
                "provider_dispatched": False,
                "pid": None,
            },
        }
        self.assertEqual(module.validate_v8_runner_zero_source_preview(envelope, 0), [])
        self.assertTrue(module.validate_v8_runner_zero_source_preview(envelope, 2))
        for malformed in [None, [], {}, {"data": []}]:
            with self.subTest(malformed=malformed):
                self.assertTrue(module.validate_v8_runner_zero_source_preview(malformed, 0))
        for scope, field, invalid in [
            ((), "ok", [False, 1, "true", None]),
            ((), "command", ["build", None]),
            (("data",), "ok", [False, 1, "true", None]),
            (("data",), "mode", ["thick", None]),
            (("data",), "plan", [None, [], {}, {"program": ""}, {"program": 1}]),
            (("data",), "provider_dispatched", [True, 0, "false", None]),
            (("data",), "pid", [1, 0, False, ""]),
        ]:
            for value in invalid:
                with self.subTest(field=field, value=value):
                    changed = copy.deepcopy(envelope)
                    container = changed[scope[0]] if scope else changed
                    container[field] = value
                    self.assertTrue(module.validate_v8_runner_zero_source_preview(changed, 0))
            with self.subTest(field=field, missing=True):
                changed = copy.deepcopy(envelope)
                container = changed[scope[0]] if scope else changed
                del container[field]
                self.assertTrue(module.validate_v8_runner_zero_source_preview(changed, 0))

    def test_v8_runner_native_contract_checks_zero_source_preview_before_waited_control(self) -> None:
        module = load_contract_module()
        for failure in [
            None, "preview_dispatch", "invalid_json", "invalid_receipt", "missing_dispatch",
            "changed_config", "changed_infobase",
        ]:
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as tmp:
                runner = Path(tmp) / "runner"
                runner.write_bytes(b"runner")
                calls = []

                def compile_stub(source, binary, root, label):
                    binary.write_bytes(b"platform")
                    return []

                def run(command, **kwargs):
                    calls.append(command)
                    root = kwargs["cwd"]
                    marker = root / "platform" / "bin" / "client-dispatched"
                    config = Path(command[command.index("--config") + 1]).read_text()
                    if "--dry-run" in command:
                        self.assertEqual(command[-3:], ["launch", "thin", "--dry-run"])
                        self.assertIn("source-set: []\n", config)
                        self.assertNotIn("CONFIGURATION", config)
                        # Legacy preview logging is allowed; input mutation is not.
                        (root / "work" / "preview.log").write_text("previewed")
                        if failure == "changed_config":
                            Path(command[command.index("--config") + 1]).write_text(
                                config + "# rewritten by provider\n"
                            )
                        if failure == "changed_infobase":
                            (root / "ib" / "1Cv8.1CD").write_bytes(b"unexpected database")
                        if failure == "preview_dispatch":
                            marker.write_bytes(b"dispatched")
                        payload = {
                            "ok": True, "command": "launch",
                            "data": {"ok": True, "mode": "thin", "pid": None,
                                     "provider_dispatched": False,
                                     "plan": {"program": "/fixture/platform/bin/1cv8c"}},
                        }
                        if failure == "invalid_receipt":
                            payload["data"]["provider_dispatched"] = True
                    else:
                        self.assertEqual(len(calls), 2)
                        self.assertIn("type: CONFIGURATION", config)
                        self.assertIn("--wait-for-exit", command)
                        if failure != "missing_dispatch":
                            marker.write_bytes(b"dispatched")
                        execute = command[command.index("--execute") + 1]
                        output = command[command.index("--output") + 1]
                        stderr = command[command.index("--stderr-output") + 1]
                        Path(output).write_text(module.V8_RUNNER_BOUNDED_OUTPUT_MARKER)
                        Path(stderr).write_text(module.V8_RUNNER_BOUNDED_STDERR_MARKER)
                        payload = {"data": {"external_epf_wait": {
                            "pid": 123, "execute_path": execute, "exit_code": 7,
                            "timed_out": False, "output_path": output, "stderr_path": stderr,
                        }}}
                    stdout = "{" if failure == "invalid_json" else json.dumps(payload)
                    return subprocess.CompletedProcess(command, 0, stdout, "")

                with (
                    patch.object(module, "compile_rust_platform_stub", side_effect=compile_stub),
                    patch.object(module.subprocess, "run", side_effect=run),
                ):
                    errors = module.check_v8_runner_bounded_external_epf_contract(runner, "linux-x64")
                if failure is None:
                    self.assertEqual(errors, [])
                else:
                    expected = {
                        "preview_dispatch": "preview dispatched",
                        "invalid_json": "preview returned invalid JSON",
                        "invalid_receipt": "provider_dispatched must be false",
                        "missing_dispatch": "control did not dispatch",
                        "changed_config": "preview changed its configuration",
                        "changed_infobase": "preview changed the initially empty infobase",
                    }[failure]
                    self.assertTrue(any(expected in error for error in errors), errors)
                self.assertEqual(
                    len(calls),
                    2 if failure in {None, "missing_dispatch"} else 1,
                )

    def test_v8_runner_bounded_external_epf_result_accepts_exit_seven_artifacts(
        self,
    ) -> None:
        module = load_contract_module()
        validator = getattr(
            module,
            "validate_v8_runner_bounded_external_epf_result",
            None,
        )
        self.assertIsNotNone(validator)

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            execute = root / "processor.epf"
            output = root / "platform.log"
            stderr_output = root / "client.stderr.log"
            execute.write_bytes(b"epf")
            output.write_text("bounded-platform-out\n", encoding="utf-8")
            stderr_output.write_text("bounded-client-stderr\n", encoding="utf-8")
            envelope = {
                "data": {
                    "external_epf_wait": {
                        "pid": 123,
                        "execute_path": str(execute),
                        "exit_code": 7,
                        "timed_out": False,
                        "output_path": str(output),
                        "stderr_path": str(stderr_output),
                    }
                }
            }

            self.assertEqual(
                validator(
                    envelope,
                    execute,
                    output,
                    stderr_output,
                    "bounded-platform-out",
                    "bounded-client-stderr",
                ),
                [],
            )

    def test_v8_runner_bounded_external_epf_result_rejects_broken_wait_contract(
        self,
    ) -> None:
        module = load_contract_module()
        validator = getattr(
            module,
            "validate_v8_runner_bounded_external_epf_result",
            None,
        )
        self.assertIsNotNone(validator)

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            execute = root / "processor.epf"
            output = root / "platform.log"
            stderr_output = root / "client.stderr.log"
            execute.write_bytes(b"epf")
            output.write_text("bounded-platform-out\n", encoding="utf-8")
            stderr_output.write_text("bounded-client-stderr\n", encoding="utf-8")
            envelope = {
                "data": {
                    "external_epf_wait": {
                        "pid": 123,
                        "execute_path": str(execute),
                        "exit_code": 7,
                        "timed_out": False,
                        "output_path": str(output),
                        "stderr_path": str(stderr_output),
                    }
                }
            }
            mutations = [
                ("pid", 0, "pid"),
                ("execute_path", str(root / "other.epf"), "execute_path"),
                ("exit_code", 0, "exit_code"),
                ("timed_out", True, "timed_out"),
                ("output_path", str(root / "other.log"), "output_path"),
                ("stderr_path", str(root / "other.stderr.log"), "stderr_path"),
            ]

            for field, value, expected_error in mutations:
                with self.subTest(field=field):
                    broken = json.loads(json.dumps(envelope))
                    broken["data"]["external_epf_wait"][field] = value
                    errors = validator(
                        broken,
                        execute,
                        output,
                        stderr_output,
                        "bounded-platform-out",
                        "bounded-client-stderr",
                    )
                    self.assertTrue(
                        any(expected_error in error for error in errors),
                        errors,
                    )

            stderr_output.write_text("unexpected stderr\n", encoding="utf-8")
            errors = validator(
                envelope,
                execute,
                output,
                stderr_output,
                "bounded-platform-out",
                "bounded-client-stderr",
            )
            self.assertTrue(
                any("stderr artifact" in error for error in errors),
                errors,
            )

            output.write_text(
                "bounded-platform-out\nbounded-client-stderr\n",
                encoding="utf-8",
            )
            stderr_output.write_text(
                "bounded-client-stderr\nbounded-platform-out\n",
                encoding="utf-8",
            )
            errors = validator(
                envelope,
                execute,
                output,
                stderr_output,
                "bounded-platform-out",
                "bounded-client-stderr",
            )
            self.assertTrue(
                any("platform /Out artifact" in error for error in errors),
                errors,
            )
            self.assertTrue(
                any("stderr artifact" in error for error in errors),
                errors,
            )

    def test_v8_runner_windows_external_publication_result_accepts_clean_epf(
        self,
    ) -> None:
        module = load_contract_module()
        validator = getattr(
            module,
            "validate_v8_runner_windows_external_publication_result",
            None,
        )
        self.assertIsNotNone(validator)

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            output = root / "Deploy"
            epf = output / "Alpha.epf"
            output.mkdir()
            epf.write_bytes(b"issue-310-current")
            envelope = {
                "ok": True,
                "command": "make",
                "data": {
                    "ok": True,
                    "mode": "external_data_processor_epf",
                    "source_set": "external-processors",
                    "output_path": "Deploy",
                    "artifacts": {
                        "root_dir": "Deploy",
                        "items": [
                            {
                                "kind": "package",
                                "path": str(Path("Deploy") / "Alpha.epf"),
                                "role": "package_file",
                            }
                        ],
                    },
                    "execution": {
                        "status": "succeeded",
                        "payload": {
                            "artifact_type": "external_data_processor_epf",
                            "output_path": "Deploy",
                            "file_names": ["Alpha.epf"],
                            "published": True,
                        },
                    },
                },
            }

            self.assertEqual(
                validator(envelope, output, epf, b"issue-310-current", root),
                [],
            )

    def test_v8_runner_windows_external_publication_result_rejects_failed_or_dirty_publish(
        self,
    ) -> None:
        module = load_contract_module()
        validator = module.validate_v8_runner_windows_external_publication_result

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            output = root / "Deploy"
            epf = output / "Alpha.epf"
            output.mkdir()
            epf.write_bytes(b"issue-310-stale")
            (root / ".artifacts-stage-leftover").mkdir()
            envelope = {
                "ok": False,
                "data": {
                    "ok": False,
                    "mode": "external_data_processor_epf",
                    "source_set": "external-processors",
                    "output_path": str(output),
                    "execution": {"status": "failed"},
                },
            }

            errors = validator(
                envelope,
                output,
                epf,
                b"issue-310-current",
                root,
            )

        self.assertTrue(any("envelope" in error for error in errors), errors)
        self.assertTrue(any("unexpected bytes" in error for error in errors), errors)
        self.assertTrue(any("temporary state" in error for error in errors), errors)

    def test_v8_runner_external_build_calls_accept_a_build_over_the_configuration(
        self,
    ) -> None:
        """Раннер 0.14 собирает обработку во временной базе поверх конфигурации."""
        module = load_contract_module()
        validator = module.validate_v8_runner_external_build_calls

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            configuration = root / "src" / "main"
            work = root / "work"
            (work / "temp" / "throwaway-infobases").mkdir(parents=True)
            calls = [
                "create",
                f"load-config\t{configuration}\textension=false",
                "load-external",
                "dump-external",
            ]

            self.assertEqual(validator(calls, configuration, work), [])

    def test_v8_runner_external_build_calls_reject_a_wrong_or_leaky_build(
        self,
    ) -> None:
        module = load_contract_module()
        validator = module.validate_v8_runner_external_build_calls

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            configuration = root / "src" / "main"
            work = root / "work"
            retained = work / "temp" / "throwaway-infobases" / "base-1"
            retained.mkdir(parents=True)

            cases = {
                "no configuration load": (
                    ["create", "load-external", "dump-external"],
                    "platform steps",
                ),
                "load after the external package": (
                    [
                        "create",
                        "load-external",
                        f"load-config\t{configuration}\textension=false",
                        "dump-external",
                    ],
                    "platform steps",
                ),
                "another source-set": (
                    [
                        "create",
                        f"load-config\t{root / 'src' / 'other'}\textension=false",
                        "load-external",
                        "dump-external",
                    ],
                    "CONFIGURATION source-set",
                ),
                "loaded as an extension": (
                    [
                        "create",
                        f"load-config\t{configuration}\textension=true",
                        "load-external",
                        "dump-external",
                    ],
                    "-Extension",
                ),
            }
            for name, (calls, expected) in cases.items():
                with self.subTest(name=name):
                    errors = validator(calls, configuration, work)
                    self.assertTrue(
                        any(expected in error for error in errors), errors
                    )
                    self.assertTrue(
                        any("throwaway infobase was retained" in error for error in errors),
                        errors,
                    )

    def test_v8_runner_external_build_calls_reject_each_defect_alone(self) -> None:
        """Каждое нарушение ловится само по себе, а не за компанию с другим."""
        module = load_contract_module()
        validator = module.validate_v8_runner_external_build_calls

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            configuration = root / "src" / "main"
            good = [
                "create",
                f"load-config\t{configuration}\textension=false",
                "load-external",
                "dump-external",
            ]
            clean = root / "clean"
            (clean / "temp" / "throwaway-infobases").mkdir(parents=True)
            leaky = root / "leaky"
            (leaky / "temp" / "throwaway-infobases" / "base-1").mkdir(parents=True)
            rootless = root / "rootless"
            rootless.mkdir()

            cases = {
                "retained base": (good, leaky, "throwaway infobase was retained"),
                "no throwaway root": (good, rootless, "root was not found"),
                "malformed load": (
                    ["create", f"load-config\t{configuration}", "load-external", "dump-external"],
                    clean,
                    "malformed",
                ),
            }
            for name, (calls, work, expected) in cases.items():
                with self.subTest(name=name):
                    errors = validator(calls, configuration, work)
                    self.assertEqual(len(errors), 1, errors)
                    self.assertIn(expected, errors[0])

    def test_targeted_tool_contracts_run_all_v8_runner_behavioral_smokes(self) -> None:
        module = load_contract_module()
        checks = [
            "check_v8_runner_partial_load_contract",
            "check_v8_runner_bounded_external_epf_contract",
            "check_v8_runner_windows_external_publication_contract",
            "check_v8_runner_full_pull_publication_contract",
        ]
        with tempfile.TemporaryDirectory() as tmp:
            tools_dir = Path(tmp)
            runner = tools_dir / "v8-runner"
            runner.write_bytes(b"runner")
            with ExitStack() as patches:
                patches.enter_context(patch.object(module, "TOOL_HELP_CHECKS", []))
                mocks = [patches.enter_context(patch.object(module, name, return_value=[name]))
                         for name in checks]
                errors = module.check_tool_contracts(tools_dir, "linux-x64")
        self.assertEqual(errors, checks)
        for check in mocks:
            check.assert_called_once_with(runner.resolve(), "linux-x64")

    def test_tool_contracts_find_the_runner_inside_its_publisher_archive_directory(
        self,
    ) -> None:
        """Архив издателя кладёт бинарь в свой каталог, и проверка его находит."""
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            tools_dir = Path(tmp)
            for target, folder, name in [
                ("linux-x64", "v8-runner-linux-x86_64-musl", "v8-runner"),
                ("win-x64", "v8-runner-windows-x86_64", "v8-runner.exe"),
            ]:
                with self.subTest(target=target):
                    nested = tools_dir / target / folder / name
                    nested.parent.mkdir(parents=True)
                    nested.write_bytes(b"runner")
                    self.assertEqual(
                        module.tool_executable(tools_dir / target, "v8-runner", target),
                        nested,
                    )
            ambiguous = tools_dir / "ambiguous"
            for folder in ["one", "two"]:
                (ambiguous / folder).mkdir(parents=True)
                (ambiguous / folder / "v8-runner").write_bytes(b"runner")
            self.assertEqual(
                module.tool_executable(ambiguous, "v8-runner", "linux-x64"),
                ambiguous / "v8-runner",
            )

    def test_targeted_tool_contracts_run_windows_external_publication_smoke(
        self,
    ) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            tools_dir = Path(tmp)
            runner = tools_dir / "v8-runner.exe"
            runner.write_bytes(b"runner")
            with (
                patch.object(module, "TOOL_HELP_CHECKS", []),
                patch.object(
                    module,
                    "check_v8_runner_partial_load_contract",
                    return_value=[],
                ),
                patch.object(
                    module,
                    "check_v8_runner_bounded_external_epf_contract",
                    return_value=[],
                ),
                patch.object(
                    module,
                    "check_v8_runner_windows_external_publication_contract",
                    return_value=["windows publication failure"],
                ) as publication_check,
                patch.object(
                    module,
                    "check_v8_runner_full_pull_publication_contract",
                    return_value=[],
                ),
            ):
                errors = module.check_tool_contracts(tools_dir, "win-x64")

        self.assertEqual(errors, ["windows publication failure"])
        publication_check.assert_called_once_with(runner.resolve(), "win-x64")

    BSL_ANALYZER_HELP = (
        "#!/usr/bin/env sh\n"
        "case \"$*\" in\n"
        "  'analyze --help') printf '%s\\n' '--source-dir --format jsonl' ;;\n"
        "  'mcp serve --help') printf '%s\\n' '--profile --source-dir --mode stdio' ;;\n"
        "  *) exit 1 ;;\n"
        "esac\n"
    )

    def test_local_research_material_is_ignored_by_git(self) -> None:
        """`docs-local/` — местные материалы: git не берёт в индекс ничего оттуда."""
        for probe in ("docs-local/note.md", "docs-local/nested/page.html"):
            with self.subTest(probe=probe):
                ignored = subprocess.run(
                    ["git", "check-ignore", "--quiet", "--no-index", probe],
                    cwd=REPO_ROOT,
                    check=False,
                )
                self.assertEqual(ignored.returncode, 0, f"{probe} is not ignored")

    def test_marketplace_card_uses_unica_product_legal_links(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        plugin = json.loads(
            (repo_root / "plugins/unica/.codex-plugin/plugin.json").read_text(
                encoding="utf-8"
            )
        )

        self.assertEqual(
            plugin["interface"]["websiteURL"],
            "https://ingvar.pro/products/unica/en",
        )
        self.assertEqual(
            plugin["interface"]["privacyPolicyURL"],
            "https://ingvar.pro/products/unica/privacy/en",
        )
        self.assertEqual(
            plugin["interface"]["termsOfServiceURL"],
            "https://ingvar.pro/products/unica/terms/en",
        )

    def test_the_catalog_moves_only_behind_green_consumer_installs(self) -> None:
        """The linear pipeline replaces the warden's greenness check.

        The marketplace default branch has no protection rules, so the job
        ordering here is the only thing standing between unverified bytes and
        every consumer: promote must require the install checks, the anchor tag
        must exist before them, and staging must never touch the catalog.
        """
        repo_root = Path(__file__).resolve().parents[2]
        publish = (repo_root / ".github/workflows/release.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("needs: [gate, stage, anchor, verify-fresh-install, verify-upgrade]", publish)
        self.assertIn("needs: [gate, stage, anchor]", publish)
        # A published tag never moves; a rerun proves sameness instead.
        marketplace = publish[publish.index("  gate:" + chr(10)):]
        self.assertNotIn("git tag -f", marketplace)
        self.assertNotIn("--force", marketplace)
        # No scheduled babysitter exists anymore; the pipeline is one pass.
        self.assertFalse((repo_root / ".github/workflows/release-warden.yml").exists())
        self.assertFalse((repo_root / "scripts/ci/release-warden.py").exists())

    def test_release_tag_is_not_hardcoded_in_the_build_workflow(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        release = (repo_root / ".github/workflows/release.yml").read_text(
            encoding="utf-8"
        )
        # A hardcoded release version is a location no contract check covers, and
        # packaging fails on every later pull request once it drifts. A tag-shaped
        # literal is wrong anywhere in the file, however quoted, and this also
        # catches suffixed forms such as v1.2.3-rc1 by matching their prefix.
        # The version comment beside a commit hash documents an action pin, not
        # a release tag: Dependabot moves it with the hash, and nothing reads it.
        without_pins = re.sub(r"@[0-9a-f]{40} # v\d+\.\d+\.\d+", "", release)
        tag_literals = sorted(set(re.findall(r"v\d+\.\d+\.\d+", without_pins)))
        # An unprefixed literal is only wrong inside the step that derives the
        # tag, including in an intermediate variable it reads. The file elsewhere
        # pins other tools by bare version, so this cannot be a whole-file rule.
        step_name = "Resolve the release tag for non-tag builds"
        start = release.find(step_name)
        self.assertNotEqual(start, -1, "the workflow no longer derives the release tag")
        following = re.search(r"(?m)^      - (name|uses|run):", release[start:])
        step = release[start : start + following.start()] if following else release[start:]
        unprefixed = sorted(set(re.findall(r"\d+\.\d+\.\d+", step)))

        self.assertEqual(tag_literals, [])
        self.assertEqual(unprefixed, [])

    def test_bump_version_writes_every_contract_location(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        module_path = repo_root / "scripts" / "dev" / "bump-version.py"
        spec = importlib.util.spec_from_file_location("bump_version", module_path)
        assert spec is not None and spec.loader is not None
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)

        contract = importlib.util.spec_from_file_location(
            "check_version_contract", repo_root / "scripts" / "ci" / "check-version-contract.py"
        )
        assert contract is not None and contract.loader is not None
        contract_module = importlib.util.module_from_spec(contract)
        contract.loader.exec_module(contract_module)

        with tempfile.TemporaryDirectory() as tmp:
            work = Path(tmp) / "repo"
            for relative in (
                "Cargo.toml",
                "plugins/unica/.codex-plugin/plugin.json",
                "plugins/unica/third-party/tools.lock.json",
            ):
                target = work / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(
                    (repo_root / relative).read_text(encoding="utf-8"), encoding="utf-8"
                )
            for directory in (".claude-plugin", ".zcode-plugin"):
                manifest = work / "plugins/unica" / directory / "plugin.json"
                manifest.parent.mkdir(parents=True, exist_ok=True)
                manifest.write_text(
                    json.dumps({"name": "unica", "version": "0.0.0"}) + "\n", encoding="utf-8"
                )

            changed = module.bump(work, "9.8.7")
            values = contract_module.read_version_contract(work)
            for directory in (".codex-plugin", ".claude-plugin", ".zcode-plugin"):
                manifest = work / "plugins/unica" / directory / "plugin.json"
                self.assertEqual(json.loads(manifest.read_text(encoding="utf-8"))["version"], "9.8.7")
                self.assertIn(f"plugins/unica/{directory}/plugin.json", changed)
            self.assertEqual(module.bump(work, "9.8.7"), [])

        self.assertEqual(set(values.values()), {"9.8.7"}, values)

    def test_bump_version_refuses_missing_or_malformed_zcode_without_writes(self) -> None:
        module_path = REPO_ROOT / "scripts/dev/bump-version.py"
        spec = importlib.util.spec_from_file_location("bump_version", module_path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        for payload in (None, "{invalid-json"):
            with self.subTest(zcode=payload), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                before = {}
                for relative in (
                    "Cargo.toml", "plugins/unica/.codex-plugin/plugin.json",
                    "plugins/unica/.claude-plugin/plugin.json", "plugins/unica/third-party/tools.lock.json",
                ):
                    target = root / relative
                    target.parent.mkdir(parents=True, exist_ok=True)
                    before[relative] = (REPO_ROOT / relative).read_bytes()
                    target.write_bytes(before[relative])
                if payload is not None:
                    zcode = root / "plugins/unica/.zcode-plugin/plugin.json"
                    zcode.parent.mkdir()
                    zcode.write_text(payload, encoding="utf-8")
                    before[zcode.relative_to(root).as_posix()] = zcode.read_bytes()
                with self.assertRaises((SystemExit, json.JSONDecodeError)):
                    module.bump(root, "9.8.7")
                self.assertEqual(
                    {relative: (root / relative).read_bytes() for relative in before}, before
                )

    def test_bump_version_writes_nothing_when_a_later_file_is_malformed(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        module_path = repo_root / "scripts" / "dev" / "bump-version.py"
        spec = importlib.util.spec_from_file_location("bump_version", module_path)
        assert spec is not None and spec.loader is not None
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)

        with tempfile.TemporaryDirectory() as tmp:
            work = Path(tmp) / "repo"
            cargo = work / "Cargo.toml"
            cargo.parent.mkdir(parents=True, exist_ok=True)
            cargo.write_text(
                (repo_root / "Cargo.toml").read_text(encoding="utf-8"), encoding="utf-8"
            )
            for directory in (".codex-plugin", ".claude-plugin", ".zcode-plugin"):
                manifest = work / "plugins/unica" / directory / "plugin.json"
                manifest.parent.mkdir(parents=True, exist_ok=True)
                manifest.write_text(json.dumps({"version": "0.0.1"}), encoding="utf-8")
            lock = work / "plugins/unica/third-party/tools.lock.json"
            lock.parent.mkdir(parents=True, exist_ok=True)
            # Two unica entries: valid JSON, but no single version to set.
            lock.write_text(
                json.dumps({"tools": [{"name": "unica"}, {"name": "unica"}]}), encoding="utf-8"
            )
            before = cargo.read_text(encoding="utf-8")

            with self.assertRaises(SystemExit):
                module.bump(work, "9.8.7")

            # Straddling two versions is the exact state the contract forbids, so
            # a failure part-way through has to leave everything untouched.
            self.assertEqual(cargo.read_text(encoding="utf-8"), before)

    def test_the_candidate_anchor_names_the_staging_commit(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        publish = (repo_root / ".github/workflows/release.yml").read_text(
            encoding="utf-8"
        )

        # The install checks need an immutable ref on the staged bytes before
        # the catalog moves. The release tag cannot be it: its catalogs name
        # the release, and that commit exists only once promote builds it. The
        # anchor `candidate-vX` takes that role on the staging commit; the
        # release tag is created in promote, as test_publish_channels runs.
        self.assertIn('tag -a "$anchor" "$STAGING_SHA"', publish)
        self.assertNotIn('tag -a "$RELEASE_TAG" "$STAGING_SHA"', publish)
        self.assertIn('tag -a "$RELEASE_TAG" "$release_commit"', publish)

    def test_release_gate_pins_the_oldest_supported_client(self) -> None:
        from tests.ci.test_unica_workflow import RELEASE_WORKFLOW, job, load, script, steps

        package = job(load(RELEASE_WORKFLOW), "package-thin")
        pins = [
            str(step["env"]["CLAUDE_CLI_VERSION"])
            for step in steps(package)
            if "CLAUDE_CLI_VERSION" in (step.get("env") or {})
        ]
        self.assertEqual(pins, ["2.1.69"])
        shell = script(package)
        ordered = (
            'npm install -g "@anthropic-ai/claude-code@${CLAUDE_CLI_VERSION}"',
            'test "$(claude --version | cut -d\' \' -f1)" = "$CLAUDE_CLI_VERSION"',
            "claude plugin validate dist/thin/marketplace/plugins/unica",
            "claude plugin validate dist/thin/marketplace",
        )
        positions = [shell.index(command) for command in ordered]
        self.assertEqual(positions, sorted(positions))
        self.assertIn("python scripts/ci/package-unica-plugin.py", shell)
        self.assertLess(
            shell.index("python scripts/ci/package-unica-plugin.py"),
            positions[0],
        )

    def test_publish_workflow_promotes_both_host_catalogs(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        publish = (repo_root / ".github/workflows/release.yml").read_text(
            encoding="utf-8"
        )
        release = (repo_root / ".github/workflows/release.yml").read_text(
            encoding="utf-8"
        )

        # Staging must carry both manifests, and promotion must move both
        # catalogs together, or one host would be left pointing at a stale tag.
        self.assertIn("payload/plugins/unica/.claude-plugin/plugin.json", publish)
        self.assertIn("payload/.claude-plugin/marketplace.json", publish)
        self.assertIn("codex=.agents/plugins/marketplace.json", publish)
        self.assertIn("claude=.claude-plugin/marketplace.json", publish)
        self.assertIn(
            'python3 "$rules" write-catalogs "$target" "$RELEASE_TAG" payload marketplace',
            publish,
        )
        # Writing is not enough: an unstaged catalog would leave the promotion
        # commit without the Claude entry while the write assertion still passed.
        self.assertIn('git -C marketplace add "$codex" "$claude"', publish)
        # The gate is pinned to the compatibility floor, not to the latest CLI.
        self.assertIn("@anthropic-ai/claude-code@${CLAUDE_CLI_VERSION}", release)
        self.assertIn("CLAUDE_CLI_VERSION: 2.1.69", release)
        self.assertIn("claude plugin validate", release)

    def test_every_declared_refusal_detail_is_constructed_somewhere(self) -> None:
        """Объявленное уточнение без источника — обещание, которого провод не держит.

        Карта code→detail внутри типа такого не ловит: она удовлетворяется одними
        объявлениями. Код тогда молча отвечает своим умолчанием, и различие, ради
        которого уточнение заведено, до читателя не доходит.
        """
        repo_root = Path(__file__).resolve().parents[2]
        dictionary = (
            repo_root / "crates" / "unica-refusal" / "src" / "lib.rs"
        )
        sources = [
            path
            for crate in (repo_root / "crates").iterdir()
            for path in (crate / "src").rglob("*.rs")
            if path != dictionary
        ]
        corpus = "".join(path.read_text(encoding="utf-8") for path in sources)
        detail_names = re.findall(
            r"^pub enum RefusalDetail \{(.*?)^\}",
            dictionary.read_text(encoding="utf-8"),
            re.MULTILINE | re.DOTALL,
        )
        self.assertEqual(len(detail_names), 1, "не нашли объявление RefusalDetail")
        variants = re.findall(r"^\s{4}(\w+),$", detail_names[0], re.MULTILINE)
        self.assertGreaterEqual(len(variants), 8)

        orphans = [
            variant for variant in variants if f"RefusalDetail::{variant}" not in corpus
        ]
        self.assertEqual(
            orphans,
            [],
            "уточнения объявлены, но нигде не ставятся: их код так и будет "
            "отвечать умолчанием",
        )

    def test_application_layer_does_not_spawn_git_directly(self) -> None:
        """rule: INV.APP.DEPENDENCY-DIRECTION"""
        repo_root = Path(__file__).resolve().parents[2]
        application_root = (
            repo_root / "crates" / "unica-application" / "src" / "application"
        )
        offenders = []
        for path in application_root.rglob("*.rs"):
            if direct_git_command_calls(path.read_bytes()):
                offenders.append(str(path.relative_to(repo_root)))

        self.assertEqual(offenders, [])

    def test_direct_git_command_guard_matches_calls_before_masking_literals(self) -> None:
        """rule: INV.APP.DEPENDENCY-DIRECTION"""
        source = br'''
fn productive() { std::process::Command::new("git"); }
fn harmless() {
    let mention = "std::process::Command::new(\"git\")";
    // std::process::Command::new("git");
}
#[cfg(test)]
fn test_only() { std::process::Command::new("git"); }
'''

        self.assertEqual(len(direct_git_command_calls(source)), 1)

    def write_executable(self, tools_dir: Path, name: str, body: str) -> None:
        commands = {
            "bsl-analyzer": [("analyze", "--help"), ("mcp", "serve", "--help")],
            "v8-runner": [("--version",), ("push", "--help")],
        }[name]
        routed_outputs = {
            tuple(route.split()): output
            for route, output in re.findall(
                r"'([^']+)'\) printf '%s\\n' '([^']*)'",
                body,
            )
        }
        fallback_outputs = re.findall(r"printf '%s\\n' '([^']*)'", body)
        fallback = fallback_outputs[0] if fallback_outputs else ""
        routes = {" ".join(command): routed_outputs.get(command, fallback) for command in commands}
        path = tools_dir / f"{name}.py"
        path.write_text(
            "#!/usr/bin/env python3\n"
            "import json\n"
            "import sys\n"
            f"ROUTES = json.loads({json.dumps(json.dumps(routes))})\n"
            "key = ' '.join(sys.argv[1:])\n"
            "if key not in ROUTES:\n"
            "    raise SystemExit(1)\n"
            "print(ROUTES[key])\n",
            encoding="utf-8",
        )
        path.chmod(path.stat().st_mode | 0o755)

    def process_is_alive(self, pid: int) -> bool:
        if os.name == "nt":
            result = subprocess.run(
                ["tasklist", "/FI", f"PID eq {pid}", "/FO", "CSV", "/NH"],
                text=True,
                capture_output=True,
                check=False,
            )
            return result.returncode == 0 and f'"{pid}"' in result.stdout
        # Отсутствие сообщает ядро; `ps` нужен только чтобы отличить зомби,
        # и его сбой не выдаёт живой процесс за мёртвый.
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return False
        except PermissionError:
            return True
        result = subprocess.run(
            ["ps", "-o", "stat=", "-p", str(pid)],
            text=True,
            capture_output=True,
            check=False,
        )
        return not result.stdout.strip().startswith("Z")

    def kill_process_tree_best_effort(self, parent_pid: int, child_pid: int) -> None:
        if os.name == "nt":
            for pid in dict.fromkeys((parent_pid, child_pid)):
                subprocess.run(
                    ["taskkill", "/PID", str(pid), "/T", "/F"],
                    text=True,
                    capture_output=True,
                    check=False,
                )
            return
        try:
            if parent_pid and os.getpgid(parent_pid) == parent_pid:
                os.killpg(parent_pid, signal.SIGKILL)
                return
        except ProcessLookupError:
            pass
        for pid in (child_pid, parent_pid):
            if not pid:
                continue
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass

    def cleanup_process_tree_best_effort(self, parent_pid: int, child_pid: int) -> None:
        if parent_pid or child_pid:
            self.kill_process_tree_best_effort(parent_pid, child_pid)

    def test_windows_best_effort_cleanup_targets_surviving_child_independently(
        self,
    ) -> None:
        with patch.object(os, "name", "nt"), patch.object(
            subprocess,
            "run",
        ) as run:
            self.kill_process_tree_best_effort(101, 202)

        self.assertEqual(
            [call.args[0] for call in run.call_args_list],
            [
                ["taskkill", "/PID", "101", "/T", "/F"],
                ["taskkill", "/PID", "202", "/T", "/F"],
            ],
        )

    def test_tool_help_contracts_pass_with_expected_cli_surface(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            tools_dir = Path(tmp)
            self.write_executable(
                tools_dir,
                "bsl-analyzer",
                self.BSL_ANALYZER_HELP,
            )
            self.write_executable(
                tools_dir,
                "v8-runner",
                "#!/usr/bin/env sh\nprintf '%s\\n' 'v8-runner 0.5.1 version Usage: v8-runner push'\n",
            )

            errors = module.check_tool_contracts(tools_dir)

        self.assertEqual(errors, [])

    def test_tool_help_failure_includes_bounded_sanitized_diagnostics(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            tools_dir = Path(tmp)
            tool = tools_dir / "helper-tool.py"
            tool.write_text(
                "from pathlib import Path\n"
                "import sys\n"
                "print(f'OUT-ДИАГНОСТИКА cwd={Path.cwd()} ' + 'x' * 6000)\n"
                "print(f'ERR-ДИАГНОСТИКА cwd={Path.cwd()}', file=sys.stderr)\n"
                "raise SystemExit(7)\n",
                encoding="utf-8",
            )
            with patch.object(
                module,
                "TOOL_HELP_CHECKS",
                [("helper-tool build", "helper-tool", ["index", "build"], ["build"])],
            ):
                errors = module.check_tool_contracts(tools_dir)

        self.assertEqual(len(errors), 1)
        error = errors[0]
        self.assertIn("command exited with 7", error)
        self.assertIn("helper-tool.py index build", error)
        self.assertIn("OUT-ДИАГНОСТИКА", error)
        self.assertIn("ERR-ДИАГНОСТИКА", error)
        self.assertNotIn(str(tools_dir), error)
        self.assertLess(len(error), 5_000)

    @contextmanager
    def relative_tool_contract_fixture(self):
        with tempfile.TemporaryDirectory() as tmp:
            cwd = Path(tmp)
            tools_dir = cwd / "tools"
            tools_dir.mkdir()
            self.write_executable(
                tools_dir,
                "bsl-analyzer",
                self.BSL_ANALYZER_HELP,
            )
            self.write_executable(
                tools_dir,
                "v8-runner",
                "#!/usr/bin/env sh\nprintf '%s\\n' 'v8-runner 0.5.1 version Usage: v8-runner push'\n",
            )

            yield cwd, tools_dir.relative_to(cwd)

    def check_relative_tool_contracts(self, cwd: Path, tools_dir: Path):
        self.assertFalse(tools_dir.is_absolute())
        module_path = REPO_ROOT / "scripts" / "ci" / "check-tool-contracts.py"
        try:
            checked = subprocess.run(
                [sys.executable, "-c", "\n".join([
                    "import importlib.util, json, sys",
                    "from pathlib import Path",
                    "spec = importlib.util.spec_from_file_location('contracts', sys.argv[1])",
                    "module = importlib.util.module_from_spec(spec)",
                    "spec.loader.exec_module(module)",
                    "print(json.dumps(module.check_tool_contracts(Path(sys.argv[2]))))",
                ]), str(module_path), str(tools_dir)],
                cwd=cwd,
                check=False,
                capture_output=True,
                text=True,
                timeout=RELATIVE_TOOL_CONTRACT_TIMEOUT_SECONDS,
            )
        except subprocess.TimeoutExpired as error:
            self.fail(
                f"relative tools child timed out after {error.timeout}s\n"
                f"stdout: {error.stdout!r}\nstderr: {error.stderr!r}"
            )
        self.assertEqual(
            checked.returncode, 0,
            f"relative tools child failed\nstdout:\n{checked.stdout}\nstderr:\n{checked.stderr}",
        )
        return json.loads(checked.stdout)

    def test_tool_help_contracts_accept_relative_tools_dir(self) -> None:
        with self.relative_tool_contract_fixture() as (cwd, tools_dir):
            errors = self.check_relative_tool_contracts(cwd, tools_dir)

        self.assertEqual(errors, [])

    def test_relative_tool_contract_child_failure_preserves_captured_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            checker = root / "scripts" / "ci" / "check-tool-contracts.py"
            checker.parent.mkdir(parents=True)
            checker.write_text(
                "print('unexpected-child-stdout', flush=True)\n"
                "raise RuntimeError('unexpected-child-stderr')\n",
                encoding="utf-8",
            )
            with patch.dict(globals(), {"REPO_ROOT": root}):
                with self.assertRaises(Exception) as failure:
                    self.check_relative_tool_contracts(root, Path("tools"))
            self.assertIsInstance(failure.exception, AssertionError)
            self.assertIn("unexpected-child-stdout", str(failure.exception))
            self.assertIn("unexpected-child-stderr", str(failure.exception))

    def test_relative_tool_contract_child_timeout_preserves_captured_output(self):
        timeout = subprocess.TimeoutExpired(
            ["child"], 15, output=b"timeout-child-stdout", stderr=b"timeout-child-stderr",
        )
        with patch.object(subprocess, "run", side_effect=timeout):
            with self.assertRaises(Exception) as failure:
                self.check_relative_tool_contracts(Path.cwd(), Path("tools"))
        self.assertIsInstance(failure.exception, AssertionError)
        self.assertIn("timeout-child-stdout", str(failure.exception))
        self.assertIn("timeout-child-stderr", str(failure.exception))

    def test_tool_help_contracts_report_missing_expected_flag(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            tools_dir = Path(tmp)
            self.write_executable(tools_dir, "bsl-analyzer", "#!/usr/bin/env sh\nprintf '%s\\n' 'analyze'\n")
            self.write_executable(tools_dir, "v8-runner", "#!/usr/bin/env sh\nprintf '%s\\n' 'v8-runner version Usage: v8-runner push'\n")

            errors = module.check_tool_contracts(tools_dir)

        self.assertTrue(any("--source-dir" in error for error in errors), errors)

    def test_analyze_help_cannot_borrow_tokens_from_mcp_serve_help(self) -> None:
        module = load_contract_module()

        with tempfile.TemporaryDirectory() as tmp:
            tools_dir = Path(tmp)
            self.write_executable(
                tools_dir,
                "bsl-analyzer",
                "#!/usr/bin/env sh\n"
                "case \"$*\" in\n"
                "  'analyze --help') printf '%s\\n' '--format jsonl' ;;\n"
                "  'mcp serve --help') printf '%s\\n' '--profile --source-dir --mode stdio' ;;\n"
                "  *) exit 1 ;;\n"
                "esac\n",
            )
            self.write_executable(
                tools_dir,
                "v8-runner",
                "#!/usr/bin/env sh\nprintf '%s\\n' 'v8-runner version Usage: v8-runner push'\n",
            )

            errors = module.check_tool_contracts(tools_dir)

        self.assertTrue(
            any("bsl-analyzer analyze" in error and "--source-dir" in error for error in errors),
            errors,
        )


    def test_the_release_checks_every_address_it_publishes(self) -> None:
        """Ядро выпуск сверяет побайтно, поставки — только по адресу.

        Их байты сверены на сборке, когда CI их качал, а адрес после этого не
        трогает никто: опечатка в теге дожила бы до первого вызова движка у
        пользователя. Шаг легко выпасть незамеченным, поэтому он закреплён.
        """
        repo_root = Path(__file__).resolve().parents[2]
        release = (repo_root / ".github/workflows/release.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("scripts/ci/verify-delivery-reachable.py", release)
        self.assertIn("scripts/ci/verify-release-assets.py", release)
        # Выкладывается только то, у чего есть читатель. Подстановочный знак
        # тянул сюда описания поставок: они обещали ассеты, которых на релизе
        # нет, и не читал их никто.
        self.assertIn("dist/runtime/unica-runtime-*.tar.gz", release)
        self.assertIn("dist/runtime/unica-runtime-*.json", release)
        self.assertNotIn("dist/runtime/*-runtime-*", release)
        # Один сквозной прогрев на выпуск: адрес, сумма и раскладка вместе.
        self.assertIn("prefetch --plugin-root", release)
        self.assertTrue(
            (repo_root / "scripts/ci/verify-delivery-reachable.py").is_file()
        )

    def test_both_sides_of_the_wire_approve_the_same_release_origins(self) -> None:
        """Адрес пишет упаковщик, а сверяет bootstrap.

        Разойдись эти списки — выпуск соберётся, а установка откажет уже у
        пользователя: «origin is outside the approved release origin». Ловить
        это надо здесь, а не в поле.

        rule: INV.PKG.ENGINE-RELEASE-SOURCES
        """
        repo_root = Path(__file__).resolve().parents[2]
        packager = (repo_root / "scripts/ci/package-unica-plugin.py").read_text(
            encoding="utf-8"
        )
        validator = (
            repo_root / "crates/unica-artifact-delivery/src/manifest.rs"
        ).read_text(encoding="utf-8")

        approved = {
            "https://github.com/IngvarConsulting/unica",
            "https://github.com/IngvarConsulting/unica-toolchain",
            "https://github.com/IngvarConsulting/v8-runner-rust",
        }
        for origin in approved:
            with self.subTest(origin=origin):
                self.assertIn(f'"{origin}"', packager)
                self.assertIn(f'"{origin}/releases/download/"', validator)

        emitted_origins = set(
            re.findall(
                r'^(?:SOURCE_REPOSITORY|TOOLCHAIN_REPOSITORY|V8_RUNNER_REPOSITORY) = "([^"]+)"$',
                packager,
                re.MULTILINE,
            )
        )
        self.assertEqual(emitted_origins, approved)

        self.assertIn(
            'V8_RUNNER_REPOSITORY if artifact == "v8-runner" else TOOLCHAIN_REPOSITORY',
            packager,
        )
        self.assertIn(
            '(ArtifactRole::Engine, "v8-runner") => V8_RUNNER_RELEASE_ORIGIN',
            validator,
        )

        # Список закрыт с обеих сторон: следующий адрес — новая запись реестра.
        self.assertEqual(
            len(re.findall(r'"https://github\.com/IngvarConsulting/[\w-]+/releases/download/"', validator)),
            len(approved),
        )

    def test_startup_documentation_separates_core_blocking_from_engine_delivery(self) -> None:
        readme = (
            Path(__file__).resolve().parents[2] / "plugins/unica/README.md"
        ).read_text(encoding="utf-8")

        self.assertIn("host waits for the core", readme)
        self.assertIn("engine delivery is non-blocking", readme)
        self.assertNotIn("session is not held up while it runs", readme)


if __name__ == "__main__":
    unittest.main()
