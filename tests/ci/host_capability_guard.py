"""Detector for tests that silently omit host-dependent checks (#1893, #1925).

A test that needs a host capability (symlinks, a case-insensitive volume,
Unix permission bits...) reports its absence through
`unica_process::test_support::skip_without_host_capability` and returns
right after it. Any other early exit next to a capability probe is a silent
PASS. Positive gates that omit assertions are checked too, including local
aliases of the probe result. The probe registry below is closed: a new helper that answers "can this
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
ASSERTION_PROBE = re.compile(PROBE.pattern + r"|\b(?:hard_link_count|file_identity_for_test)\b")
EXIT = re.compile(r"\b(?:return|continue)\b")
WINDOW_LINES = 15
# A contract in a crate without the helper returns a typed skip instead;
# its `#[test]` wrapper reports it (`CapabilitySkip`, unica-workspace).
SKIP = re.compile(r"\bskip_without_host_capability\s*\(|\bCapabilitySkip\s*\{")


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


def partial_capability_assertions(body: str) -> bool:
    """Find positive capability gates whose other path omits the assertions.

    Track local probe aliases, so moving the probe away from the assertion
    does not hide the gate. Balanced blocks keep a neighbouring skip from
    excusing a different conditional branch.
    """
    aliases: set[str] = set()
    assignments = list(re.finditer(r"\blet\s+(?:mut\s+)?(\w+)\s*=\s*([^;]+);", body))
    while True:
        added = {m[1] for m in assignments if ASSERTION_PROBE.search(m[2]) or
                 any(re.search(rf"\b{re.escape(alias)}\b", m[2]) for alias in aliases)}
        if added <= aliases:
            break
        aliases |= added
    for match in re.finditer(r"\bif\s+([^{};]+)\{", body):
        condition = match[1].strip()
        if "=>" in condition:
            continue
        # Selecting path syntax alone is not a capability gate. Combining it
        # with a real probe must still be checked.
        condition_probes = [probe for probe in ASSERTION_PROBE.finditer(condition)
                            if probe.group() != "MAIN_SEPARATOR"]
        if re.search(r"\bMAIN_SEPARATOR\s*==", condition) and not condition_probes and not any(
                re.search(rf"\b{re.escape(alias)}\b", condition) for alias in aliases):
            continue
        if not (ASSERTION_PROBE.search(condition) or
                any(re.search(rf"\b{re.escape(alias)}\b", condition) for alias in aliases)):
            continue
        # Negative guards are checked by the exit detector below. Comparisons
        # of identities are assertions about the API, not availability probes.
        positive = (condition in aliases or
                    any(re.search(rf"(?:^|&&|\|\|)\s*\(*\s*{re.escape(alias)}\s*\)*(?:\s*$|\s*(?:&&|\|\|))", condition)
                        for alias in aliases) or
                    any(re.fullmatch(rf"{re.escape(alias)}\s*\.is_(?:some|ok)\s*\(\s*\)", condition)
                        for alias in aliases) or
                    re.match(r"let\s+(?:Some|Ok)\s*\(", condition) or
                    "::Created" in condition or
                    # The `!` of a macro such as `matches!` is not a negation.
                    (ASSERTION_PROBE.search(condition) and
                     not re.search(r"(?<!\w)!(?!=)|is_(?:err|none)\s*\(|!=", condition)))
        if not positive:
            continue
        opening = match.end() - 1
        guarded = _skips.block(body, opening)
        if not re.search(r"\b(?:assert(?:_eq|_ne)?|expect)\b", guarded):
            continue
        rest = body[opening + len(guarded):]
        fallback = re.match(r"\s*else\s*(?=\{)", rest)
        other = _skips.block(rest, fallback.end()) if fallback else ""
        if not SKIP.search(other) and not re.search(r"\b(?:assert(?:_eq|_ne)?|panic|unreachable)\s*!", other):
            return True
    return False


def offenses(text: str, path: str) -> list[str]:
    """`path::function` of tests with silent exits or partial capability assertions."""
    code = without_strings(text)
    found = []
    for item in _skips.functions(code):
        if not (_skips.is_test(item) or in_test_file(path)):
            continue
        body = item["body"]
        if not ASSERTION_PROBE.search(body):
            continue
        if partial_capability_assertions(body):
            found.append(f"{path}::{item['name']}")
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
            # `return Err(CapabilitySkip { .. })` carries the skip in the exit itself.
            exit_line_end = body.find("\n", exit_match.end())
            exit_line_end = len(body) if exit_line_end < 0 else exit_line_end
            if probes and not SKIP.search(body, max(probes[-1].end(), previous_exit), exit_line_end):
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
