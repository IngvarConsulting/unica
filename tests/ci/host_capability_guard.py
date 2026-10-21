"""Detector for tests that exit early on a missing host capability (#1893).

A test that needs a host capability (symlinks, a case-insensitive volume,
Unix permission bits...) reports its absence through
`unica_process::test_support::skip_without_host_capability` and returns
right after it. Any other early exit next to a capability probe is a silent
PASS. The probe registry below is closed: a new helper that answers "can this
host do X" is added here, so its callers are checked too.
"""
from __future__ import annotations

import importlib.util
import re
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SOURCE_ROOT = REPO_ROOT / "crates"

_spec = importlib.util.spec_from_file_location("conditional_test_skips", Path(__file__).with_name("test_conditional_test_skips.py"))
_skips = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_skips)

# Helpers and raw probes whose answer decides whether a test can run here.
CAPABILITY_PROBES = (
    "create_file_symlink_for_test", "create_dir_symlink_for_test",
    "create_file_link_fixture_for_test", "create_directory_link_fixture_for_test",
    "create_file_link_or_skip", "create_directory_symlink_or_skip", "create_file_symlink_or_skip",
    "create_file_link_fixture", "directory_link", "set_unix_mode_for_test",
    "can_swap_named_child_behind_retained_handle_for_test",
    "can_rename_parent_with_retained_cleanup_child_for_test",
    "supports_retained_root_replacement_test", "attempt_retained_directory_replacement_for_test",
    "host_filesystem_case_sensitive", "case_distinct_provider_paths_for_test", "child_names_equivalent",
    "path_lock_identity", "non_utf8_relative_path_for_test", "distinct_non_utf8_provider_paths_for_test",
    "distinct_non_unicode_paths_for_test", "windows_extended_length_path_for_test",
    "short_private_runtime_dir", "unix_runtime_authority_tests_supported", "JobAttachGateForTest",
    "age_tree", "set_modified", "ERROR_PRIVILEGE_NOT_HELD", "WindowsPrivilegeUnavailable",
    "PreventedByRetainedHandle", "MAIN_SEPARATOR", "symlink_file", "symlink_dir", "PermissionDenied",
    "canonical_delivery_fixture_binary",
)
PROBE = re.compile(r"\b(?:" + "|".join(CAPABILITY_PROBES) + r")\b|\bsymlink\s*\(|\b1314\b")
EXIT = re.compile(r"\b(?:return|continue)\b")
WINDOW_LINES = 15
SKIP = re.compile(r"\bskip_without_host_capability\s*\(")


def in_test_file(path: str) -> bool:
    name = path.rsplit("/", 1)[-1]
    return "/tests/" in path or name in ("tests.rs", "test_support.rs", "testing.rs") or name.endswith("_tests.rs")


_boundary_spec = importlib.util.spec_from_file_location(
    "rust_platform_boundary", REPO_ROOT / "scripts/ci/check-rust-platform-boundary.py")
_boundary = importlib.util.module_from_spec(_boundary_spec)
_boundary_spec.loader.exec_module(_boundary)


def without_strings(code: str) -> str:
    """Blank comments and literals, keeping lines: generated programs inside raw strings are not tests."""
    return _boundary._mask_non_code(code)


def offenses(text: str, path: str) -> list[str]:
    """`path::function` of every test function that exits early beside a capability probe."""
    code = without_strings(text)
    found = []
    for item in _skips.functions(code):
        if not (_skips.is_test(item) or in_test_file(path)):
            continue
        body = item["body"]
        if not PROBE.search(body):
            continue
        previous_exit = 0
        for exit_match in EXIT.finditer(body):
            before = body[:exit_match.start()]
            # The exit decides on the probe only when the probe is just above it:
            # `let .. = probe(..) else { return }`, `if !probe() { return }`, a match arm.
            window_start = max(0, len(before) - len("\n".join(before.splitlines()[-WINDOW_LINES:])))
            probes = list(PROBE.finditer(body, window_start, exit_match.start()))
            # The helper may come before clean-up, but after the probe that decided
            # and after the previous exit: one skip excuses one exit.
            if probes and not SKIP.search(body, max(probes[-1].end(), previous_exit), exit_match.start()):
                found.append(f"{path}::{item['name']}")
                break
            previous_exit = exit_match.end()
    return found


def repository_offenses() -> set[str]:
    found = set()
    for source in _skips.rust_sources():
        found.update(offenses(source.read_text(encoding="utf-8", errors="replace"),
                              source.relative_to(REPO_ROOT).as_posix()))
    return found
