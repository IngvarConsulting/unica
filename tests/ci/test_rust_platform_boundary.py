from __future__ import annotations

import importlib.util
import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT_PATH = REPO_ROOT / "scripts" / "ci" / "check-rust-platform-boundary.py"


def load_checker_module():
    spec = importlib.util.spec_from_file_location("rust_platform_boundary", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {SCRIPT_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class RustPlatformBoundaryTests(unittest.TestCase):
    def test_rejects_platform_constructs_outside_facade_with_stable_lines(self) -> None:
        checker = load_checker_module()

        diagnostics = checker.check_source(
            "crates/unica-coder/src/infrastructure/process.rs",
            "#[cfg(windows)]\n"
            "use std::os::unix::fs::PermissionsExt;\n"
            "use windows_sys::Win32::Foundation::HANDLE;\n",
        )

        self.assertEqual(
            diagnostics,
            [
                "crates/unica-coder/src/infrastructure/process.rs:1: "
                "OS-specific cfg condition is outside a platform facade",
                "crates/unica-coder/src/infrastructure/process.rs:2: "
                "std::os platform module is outside a platform facade",
                "crates/unica-coder/src/infrastructure/process.rs:3: "
                "windows_sys is outside a platform facade",
            ],
        )

    def test_allows_platform_constructs_only_in_facades_and_nested_platform_tests(self) -> None:
        checker = load_checker_module()
        source = (
            "#[cfg(target_os = \"windows\")]\n"
            "use std::os::windows::io::AsRawHandle;\n"
            "use windows_sys::Win32::Foundation::HANDLE;\n"
        )

        self.assertEqual(
            checker.check_source(
                "crates/unica-coder/src/infrastructure/platform/windows.rs", source
            ),
            [],
        )
        self.assertEqual(
            checker.check_source("crates/unica-coder/tests/platform/windows.rs", source),
            [],
        )

    def test_rejects_top_level_platform_test_file(self) -> None:
        checker = load_checker_module()

        diagnostics = checker.check_source(
            "crates/unica-coder/tests/platform.rs",
            "#[cfg(unix)]\nuse std::os::unix::fs::PermissionsExt;\n",
        )

        self.assertEqual(
            diagnostics,
            [
                "crates/unica-coder/tests/platform.rs:1: "
                "OS-specific cfg condition is outside a platform facade",
                "crates/unica-coder/tests/platform.rs:2: "
                "std::os platform module is outside a platform facade",
            ],
        )

    def test_rejects_grouped_and_nested_std_os_modules(self) -> None:
        checker = load_checker_module()

        diagnostics = checker.check_source(
            "crates/unica-coder/src/infrastructure/process.rs",
            "use std::{os::unix::fs::PermissionsExt};\n"
            "use std::os::{freebsd::ffi::OsStrExt};\n"
            "use std::{\n"
            "    os::{\n"
            "        redox::fs::MetadataExt,\n"
            "        solaris::fs::MetadataExt as SolarisMetadataExt,\n"
            "    },\n"
            "};\n",
        )

        self.assertEqual(
            diagnostics,
            [
                f"crates/unica-coder/src/infrastructure/process.rs:{line}: "
                "std::os platform module is outside a platform facade"
                for line in (1, 2, 5, 6)
            ],
        )

    def test_rejects_std_os_through_direct_and_grouped_std_aliases(self) -> None:
        checker = load_checker_module()

        diagnostics = checker.check_source(
            "crates/unica-coder/src/infrastructure/process.rs",
            "use std as platform_std;\n"
            "use self::platform_std::os::unix::fs::PermissionsExt;\n"
            "use std::{self as grouped_std};\n"
            "use grouped_std::{os::windows::io::AsRawHandle};\n"
            "extern crate std as extern_std;\n"
            "use extern_std::os::linux::fs::MetadataExt;\n"
            "use {std as outer_std};\n"
            "use outer_std::os::macos::fs::MetadataExt as MacMetadataExt;\n",
        )

        self.assertEqual(
            diagnostics,
            [
                "crates/unica-coder/src/infrastructure/process.rs:2: "
                "std::os platform module is outside a platform facade",
                "crates/unica-coder/src/infrastructure/process.rs:4: "
                "std::os platform module is outside a platform facade",
                "crates/unica-coder/src/infrastructure/process.rs:6: "
                "std::os platform module is outside a platform facade",
                "crates/unica-coder/src/infrastructure/process.rs:8: "
                "std::os platform module is outside a platform facade",
            ],
        )

    def test_alias_shadowing_does_not_cross_lexical_scopes(self) -> None:
        checker = load_checker_module()

        diagnostics = checker.check_source(
            "crates/unica-coder/src/infrastructure/process.rs",
            "mod local { pub mod os { pub fn inspect() {} } }\n"
            "fn first() { use std as scoped; let _ = scoped::mem::size_of::<u8>(); }\n"
            "fn second() { use crate::local as scoped; scoped::os::inspect(); }\n",
        )

        self.assertEqual(diagnostics, [])

    def test_windows_sys_detection_requires_a_crate_path(self) -> None:
        checker = load_checker_module()

        diagnostics = checker.check_source(
            "crates/unica-coder/src/infrastructure/process.rs",
            "let windows_sys = 1;\n"
            "let _ = windows_sys;\n"
            "use crate::windows_sys::Local;\n"
            "use windows_sys::Win32::Foundation::HANDLE;\n"
            "use {windows_sys as windows};\n",
        )

        self.assertEqual(
            diagnostics,
            [
                "crates/unica-coder/src/infrastructure/process.rs:4: "
                "windows_sys is outside a platform facade",
                "crates/unica-coder/src/infrastructure/process.rs:5: "
                "windows_sys is outside a platform facade",
            ],
        )

    def test_cfg_parser_masks_non_code_and_handles_cfg_attr(self) -> None:
        checker = load_checker_module()

        diagnostics = checker.check_source(
            "crates/unica-coder/src/domain/project.rs",
            "// #[cfg(windows)] std::os::unix\n"
            "#[cfg(feature = \"windows\")]\n"
            "let text = \"cfg(unix) windows_sys\nstd::os::linux\";\n"
            "#[cfg_attr(target_arch = \"x86_64\", inline)]\n"
            "#[cfg(target_family = \"unix\")]\n"
            "#[cfg(target_env = \"gnu\")]\n"
            "#[cfg(target_vendor = \"apple\")]\n",
        )

        self.assertEqual(
            diagnostics,
            [
                "crates/unica-coder/src/domain/project.rs:5: "
                "OS-specific cfg condition is outside a platform facade",
                "crates/unica-coder/src/domain/project.rs:6: "
                "OS-specific cfg condition is outside a platform facade",
                "crates/unica-coder/src/domain/project.rs:7: "
                "OS-specific cfg condition is outside a platform facade",
                "crates/unica-coder/src/domain/project.rs:8: "
                "OS-specific cfg condition is outside a platform facade",
            ],
        )

    def test_lifetimes_labels_and_chars_do_not_hide_code(self) -> None:
        checker = load_checker_module()

        diagnostics = checker.check_source(
            "crates/unica-coder/src/domain/project.rs",
            r"fn inspect<'a>() { std::fs::read(path); rule.exists(); "
            r"let _ = crate::infrastructure::Store; #[cfg(windows)] let _ = 1; "
            r"let _: Option<&'a str> = None; let _ = ('x', '\n', '\u{41}', '\''); "
            r"'label: loop { break 'label; } }"
            "\n",
        )

        self.assertEqual(
            diagnostics,
            [
                "crates/unica-coder/src/domain/project.rs:1: "
                "OS-specific cfg condition is outside a platform facade",
                "crates/unica-coder/src/domain/project.rs:1: "
                "domain must not reference crate::infrastructure",
                "crates/unica-coder/src/domain/project.rs:1: "
                "domain must not access std::fs directly",
            ],
        )

    def test_rejects_direct_layer_references(self) -> None:
        checker = load_checker_module()

        domain_diagnostics = checker.check_source(
            "crates/unica-coder/src/domain/project.rs",
            "use crate::application::Port;\n"
            "use crate::infrastructure::Store;\n"
            "use crate::interfaces::Cli;\n",
        )
        application_diagnostics = checker.check_source(
            "crates/unica-coder/src/application/use_case.rs",
            "let store = crate :: infrastructure :: Store::new();\n"
            "let cli = super :: interfaces :: Cli::new();\n",
        )

        self.assertEqual(len(domain_diagnostics), 3)
        self.assertEqual(
            application_diagnostics,
            [
                "crates/unica-coder/src/application/use_case.rs:1: "
                "application must not reference crate::infrastructure",
                "crates/unica-coder/src/application/use_case.rs:2: "
                "application must not reference super::interfaces",
            ],
        )

    def test_rejects_grouped_layer_references_in_use_trees(self) -> None:
        checker = load_checker_module()

        diagnostics = checker.check_source(
            "crates/unica-coder/src/application/use_case.rs",
            "use crate::{infrastructure::Store, domain::Model};\n"
            "use super::{\n"
            "    interfaces::{Cli, Request},\n"
            "    application::Port,\n"
            "};\n",
        )

        self.assertEqual(
            diagnostics,
            [
                "crates/unica-coder/src/application/use_case.rs:1: "
                "application must not reference crate::infrastructure",
                "crates/unica-coder/src/application/use_case.rs:3: "
                "application must not reference super::interfaces",
            ],
        )

    def test_rejects_layer_references_through_root_aliases(self) -> None:
        checker = load_checker_module()

        diagnostics = checker.check_source(
            "crates/unica-coder/src/application/use_case.rs",
            "use crate as root;\n"
            "use self::root::infrastructure::Store;\n"
            "use super::{self as parent};\n"
            "use parent::{interfaces::Cli};\n",
        )

        self.assertEqual(
            diagnostics,
            [
                "crates/unica-coder/src/application/use_case.rs:2: "
                "application must not reference crate::infrastructure",
                "crates/unica-coder/src/application/use_case.rs:4: "
                "application must not reference super::interfaces",
            ],
        )

    def test_rejects_domain_std_io_in_direct_grouped_and_common_alias_forms(self) -> None:
        checker = load_checker_module()

        diagnostics = checker.check_source(
            "crates/unica-coder/src/domain/project.rs",
            "let text = ::std::fs::read_to_string(path)?;\n"
            "use std::{env as environment, process};\n"
            "use std as rust_std;\n"
            "rust_std::fs::read(path);\n"
            "use std::{self as grouped_std};\n"
            "grouped_std::env::current_dir();\n",
        )

        self.assertEqual(
            diagnostics,
            [
                "crates/unica-coder/src/domain/project.rs:1: "
                "domain must not access std::fs directly",
                "crates/unica-coder/src/domain/project.rs:2: "
                "domain must not access std::env directly",
                "crates/unica-coder/src/domain/project.rs:2: "
                "domain must not access std::process directly",
                "crates/unica-coder/src/domain/project.rs:4: "
                "domain must not access std::fs directly",
                "crates/unica-coder/src/domain/project.rs:6: "
                "domain must not access std::env directly",
            ],
        )

    def test_rejects_explicit_path_ufcs_and_common_import_aliases(self) -> None:
        checker = load_checker_module()

        diagnostics = checker.check_source(
            "crates/unica-coder/src/domain/project.rs",
            "use std::path::Path as P;\n"
            "P::exists(path);\n"
            "use std::path::{Path as Q, PathBuf as PB};\n"
            "<Q>::canonicalize(path);\n"
            "let metadata = PB::metadata;\n"
            "use std::{path::Path as NestedPath};\n"
            "NestedPath::read_link(path);\n"
            "std::path::Path::is_file(path);\n",
        )

        self.assertEqual(
            diagnostics,
            [
                f"crates/unica-coder/src/domain/project.rs:{line}: "
                f"domain must not call filesystem method .{method} directly"
                for line, method in (
                    (2, "exists"),
                    (4, "canonicalize"),
                    (5, "metadata"),
                    (7, "read_link"),
                    (8, "is_file"),
                )
            ],
        )

    def test_allows_business_instance_methods_and_pure_path_operations(self) -> None:
        checker = load_checker_module()

        diagnostics = checker.check_source(
            "crates/unica-coder/src/domain/project.rs",
            "use std::path::{Path, PathBuf};\n"
            "rule.exists();\n"
            "aggregate.metadata();\n"
            "decision.is_file();\n"
            "let child = Path::new(\"root\").join(\"child\");\n"
            "let _ = child.parent();\n"
            "let _ = child.starts_with(PathBuf::from(\"root\"));\n"
            "let _ = metadata.file_type().is_file();\n",
        )

        self.assertEqual(diagnostics, [])

    def test_masks_domain_io_text_in_comments_and_literals(self) -> None:
        checker = load_checker_module()

        diagnostics = checker.check_source(
            "crates/unica-coder/src/domain/project.rs",
            "// use std::fs; Path::exists(path);\n"
            "let text = \"std::env::current_dir Path::canonicalize(path)\";\n"
            "let raw = r#\"crate::infrastructure std::process::Command\"#;\n"
            "/* use std::{fs, env, process}; */\n",
        )

        self.assertEqual(diagnostics, [])

    def test_allows_io_apis_outside_domain(self) -> None:
        checker = load_checker_module()

        diagnostics = checker.check_source(
            "crates/unica-coder/src/infrastructure/discovery.rs",
            "use std::{fs, env, process};\n"
            "let _ = std::fs::read(path);\n"
            "let _ = Path::canonicalize(path);\n",
        )

        self.assertEqual(diagnostics, [])

    def test_rejects_host_names_outside_the_host_facade(self) -> None:
        checker = load_checker_module()

        diagnostics = checker.check_source(
            "crates/unica-bootstrap/src/main.rs",
            "let data = env::var_os(\"CLAUDE_PLUGIN_DATA\");\n"
            "let manifest = plugin_root.join(\".codex-plugin\").join(\"plugin.json\");\n"
            "fn codex_home_root() -> Result<PathBuf> { unimplemented!() }\n"
            "let home = env::var_os(\"CODEX_HOME\");\n"
            "let root = env::var_os(\"CLAUDE_PLUGIN_ROOT\");\n",
        )

        self.assertEqual(
            diagnostics,
            [
                "crates/unica-bootstrap/src/main.rs:1: "
                "host environment variable CLAUDE_PLUGIN_DATA is outside the host facade",
                "crates/unica-bootstrap/src/main.rs:2: "
                "host manifest directory .codex-plugin is outside the host facade",
                "crates/unica-bootstrap/src/main.rs:3: "
                "host name codex is outside the host facade",
                "crates/unica-bootstrap/src/main.rs:4: "
                "host environment variable CODEX_HOME is outside the host facade",
                "crates/unica-bootstrap/src/main.rs:5: "
                "host environment variable CLAUDE_PLUGIN_ROOT is outside the host facade",
            ],
        )

    def test_allows_host_names_only_in_the_host_facade_and_nested_host_tests(self) -> None:
        checker = load_checker_module()
        source = (
            "const HOME: &str = \"CODEX_HOME\";\n"
            "let manifest = root.join(\".claude-plugin\");\n"
            "fn codex_home_root() -> Result<PathBuf> { unimplemented!() }\n"
        )

        for path in (
            "crates/unica-bootstrap/src/host/mod.rs",
            "crates/unica-bootstrap/src/host/descriptor/codex.rs",
            "crates/unica-bootstrap/tests/host/cli_contract.rs",
        ):
            self.assertEqual(checker.check_source(path, source), [])

    def test_rejects_top_level_host_test_file(self) -> None:
        checker = load_checker_module()

        diagnostics = checker.check_source(
            "crates/unica-bootstrap/tests/host.rs",
            "let _ = env::var_os(\"CODEX_HOME\");\n",
        )

        self.assertEqual(
            diagnostics,
            [
                "crates/unica-bootstrap/tests/host.rs:1: "
                "host environment variable CODEX_HOME is outside the host facade"
            ],
        )

    def test_host_names_are_detected_in_literals_and_comments(self) -> None:
        checker = load_checker_module()

        diagnostics = checker.check_source(
            "crates/unica-bootstrap/src/main.rs",
            "// The package points the cache at ${CLAUDE_PLUGIN_DATA}.\n"
            "/* Claude Code scans skills/ on its own. */\n"
            "let variable = \"CODEX_HOME\";\n"
            "let raw = r#\".claude-plugin\"#;\n",
        )

        self.assertEqual(
            diagnostics,
            [
                "crates/unica-bootstrap/src/main.rs:1: "
                "host environment variable CLAUDE_PLUGIN_DATA is outside the host facade",
                "crates/unica-bootstrap/src/main.rs:2: "
                "host name Claude is outside the host facade",
                "crates/unica-bootstrap/src/main.rs:3: "
                "host environment variable CODEX_HOME is outside the host facade",
                "crates/unica-bootstrap/src/main.rs:4: "
                "host manifest directory .claude-plugin is outside the host facade",
            ],
        )

    def test_a_host_name_inside_a_longer_word_is_not_host_knowledge(self) -> None:
        """Имя хоста кончается там, где кончается его регистровый сегмент.

        Подстрочное совпадение объявляло host knowledge любое слово, внутри
        которого встретились эти буквы, а исключений по путям у стража нет
        (INV-PLATFORM-NO-PATH-EXEMPTIONS) — снять ложный диагноз было бы нечем.
        """
        checker = load_checker_module()

        diagnostics = checker.check_source(
            "crates/unica-coder/src/infrastructure/mineralogy.rs",
            "// claudetite and claudent are minerals, not hosts.\n"
            "let codexes = manuscripts.len();\n",
        )

        self.assertEqual(diagnostics, [])

    def test_a_host_name_ending_a_case_segment_is_still_host_knowledge(self) -> None:
        """Обратная сторона того же правила: camelCase и snake_case не теряются."""
        checker = load_checker_module()

        diagnostics = checker.check_source(
            "crates/unica-coder/src/infrastructure/plugin_runtime.rs",
            "struct CodexHost;\n"
            "fn claude_plugin_data() {}\n"
            "let catalog = \"codex-plugin\";\n",
        )

        self.assertEqual(
            diagnostics,
            [
                "crates/unica-coder/src/infrastructure/plugin_runtime.rs:1: "
                "host name Codex is outside the host facade",
                "crates/unica-coder/src/infrastructure/plugin_runtime.rs:2: "
                "host name claude is outside the host facade",
                "crates/unica-coder/src/infrastructure/plugin_runtime.rs:3: "
                "host name codex is outside the host facade",
            ],
        )

    def test_host_facade_root_is_not_granted_to_other_crates(self) -> None:
        checker = load_checker_module()

        diagnostics = checker.check_source(
            "crates/unica-coder/src/host/codex.rs",
            "let home = env::var_os(\"CODEX_HOME\");\n",
        )

        self.assertEqual(
            diagnostics,
            [
                "crates/unica-coder/src/host/codex.rs:1: "
                "host environment variable CODEX_HOME is outside the host facade"
            ],
        )

    def test_host_neutral_names_do_not_trip_the_host_boundary(self) -> None:
        checker = load_checker_module()

        diagnostics = checker.check_source(
            "crates/unica-coder/src/infrastructure/plugin_runtime.rs",
            "let root = env::var_os(\"UNICA_PLUGIN_ROOT\");\n"
            "let cache = env::var_os(\"UNICA_RUNTIME_CACHE_DIR\");\n"
            "let target = HostTarget::current()?;\n"
            "let fixture = root.join(\"marketplace/plugins/unica\");\n"
            "let manifest = root.join(\"third-party/manifest.json\");\n",
        )

        self.assertEqual(diagnostics, [])

    def test_collects_tracked_and_nonignored_untracked_rust_sources_only(self) -> None:
        checker = load_checker_module()
        with tempfile.TemporaryDirectory() as temporary_directory:
            repo_root = Path(temporary_directory)
            (repo_root / ".gitignore").write_text("ignored.rs\ntarget/\n", encoding="utf-8")
            (repo_root / "tracked.rs").write_text("tracked\n", encoding="utf-8")
            (repo_root / "untracked.rs").write_text("untracked\n", encoding="utf-8")
            (repo_root / "ignored.rs").write_text("ignored\n", encoding="utf-8")
            (repo_root / "target").mkdir()
            (repo_root / "target" / "generated.rs").write_text("generated\n", encoding="utf-8")
            subprocess.run(["git", "init", "-q", str(repo_root)], check=True)
            subprocess.run(
                ["git", "-C", str(repo_root), "add", ".gitignore", "tracked.rs"], check=True
            )

            self.assertEqual(
                checker.collect_repository_sources(repo_root),
                {"tracked.rs": "tracked\n", "untracked.rs": "untracked\n"},
            )

    def test_cli_returns_nonzero_and_prints_diagnostics(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            repo_root = Path(temporary_directory)
            source_path = repo_root / "crates/unica-coder/src/domain/project.rs"
            source_path.parent.mkdir(parents=True)
            source_path.write_text("let _ = std::fs::read(path);\n", encoding="utf-8")
            subprocess.run(["git", "init", "-q", str(repo_root)], check=True)
            subprocess.run(["git", "-C", str(repo_root), "add", "."], check=True)

            result = subprocess.run(
                ["python3", str(SCRIPT_PATH), "--repo-root", str(repo_root)],
                text=True,
                capture_output=True,
                check=False,
            )

        self.assertEqual(result.returncode, 1)
        self.assertEqual(
            result.stdout.splitlines(),
            [
                "crates/unica-coder/src/domain/project.rs:1: "
                "domain must not access std::fs directly"
            ],
        )

    def test_repository_currently_complies_with_platform_boundary(self) -> None:
        checker = load_checker_module()

        self.assertEqual(checker.check_repository(REPO_ROOT), [])


if __name__ == "__main__":
    unittest.main()
