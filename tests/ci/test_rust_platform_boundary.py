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

    def test_allows_domain_instance_methods_and_pure_path_operations(self) -> None:
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
