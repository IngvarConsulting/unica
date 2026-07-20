#!/usr/bin/env python3
"""Enforce ADR-0009's Rust platform and dependency boundaries."""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from collections.abc import Mapping
from pathlib import Path, PurePosixPath


OS_CFG_TERM = re.compile(
    r"\b(?:windows|unix|target_(?:os|arch|family|env|vendor))\b"
)
CFG_INVOCATION = re.compile(r"\bcfg(?:_attr)?!?\s*\(")
STD_OS = re.compile(r"\bstd\s*::\s*os\b")
WINDOWS_SYS = re.compile(r"\bwindows_sys\b")
LAYER_REFERENCE = re.compile(
    r"\b(?P<prefix>crate|super)\s*::\s*"
    r"(?P<module>application|infrastructure|interfaces)\b"
)
FORBIDDEN_DOMAIN_IMPORTS = ("application", "infrastructure", "interfaces")
FORBIDDEN_APPLICATION_IMPORTS = ("infrastructure", "interfaces")
RUST_IDENTIFIER_PATTERN = r"[A-Za-z_][A-Za-z0-9_]*"
DOMAIN_STD_IO_MODULES = frozenset({"fs", "env", "process"})
DOMAIN_PATH_IO_METHODS = (
    "canonicalize",
    "exists",
    "try_exists",
    "is_file",
    "is_dir",
    "metadata",
    "symlink_metadata",
    "read_dir",
    "read_link",
    "is_symlink",
)
DOMAIN_PATH_IO_METHOD_PATTERN = "|".join(DOMAIN_PATH_IO_METHODS)


def _repository_path(path: str) -> PurePosixPath:
    candidate = PurePosixPath(path)
    if candidate.is_absolute() or ".." in candidate.parts or not candidate.parts:
        raise ValueError(f"path must be repository-relative: {path}")
    return candidate


def _is_platform_test(path: PurePosixPath) -> bool:
    parts = path.parts
    return (
        len(parts) >= 5
        and parts[0] == "crates"
        and parts[2] == "tests"
        and parts[3] == "platform"
    )


def _is_platform_facade(path: PurePosixPath) -> bool:
    normalized = path.as_posix()
    return normalized.startswith("crates/unica-coder/src/infrastructure/platform/") or normalized.startswith(
        "crates/unica-bootstrap/src/platform/"
    )


def _line_number(source: str, index: int) -> int:
    return source.count("\n", 0, index) + 1


def _cfg_expression_end(source: str, opening_parenthesis: int) -> int:
    depth = 0
    for index in range(opening_parenthesis, len(source)):
        char = source[index]
        if char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
            if depth == 0:
                return index + 1
    return len(source)


def _mask_range(masked: list[str], start: int, end: int) -> None:
    for index in range(start, end):
        if masked[index] != "\n":
            masked[index] = " "


def _raw_string_end(source: str, start: int) -> int | None:
    raw_start = start
    if source.startswith("br", start):
        raw_start += 1
    elif source[start] != "r":
        return None

    if start and (source[start - 1].isalnum() or source[start - 1] == "_"):
        return None

    delimiter_end = raw_start + 1
    while delimiter_end < len(source) and source[delimiter_end] == "#":
        delimiter_end += 1
    if delimiter_end == len(source) or source[delimiter_end] != '"':
        return None

    terminator = '"' + source[raw_start + 1 : delimiter_end]
    end = source.find(terminator, delimiter_end + 1)
    return len(source) if end == -1 else end + len(terminator)


def _double_quoted_end(source: str, start: int) -> int:
    index = start + 1
    while index < len(source):
        if source[index] == "\n":
            index += 1
            continue
        if source[index] == "\\":
            index += 2
            continue
        if source[index] == '"':
            return index + 1
        index += 1
    return len(source)


def _char_literal_end(source: str, start: int) -> int | None:
    """Return a Rust char literal end without mistaking lifetimes or labels for chars."""
    index = start + 1
    if index >= len(source) or source[index] in {"'", "\n", "\r"}:
        return None

    if source[index] != "\\":
        index += 1
    else:
        index += 1
        if index >= len(source):
            return None
        escape = source[index]
        if escape in {"n", "r", "t", "\\", "0", "'", '"'}:
            index += 1
        elif escape == "x":
            digits = source[index + 1 : index + 3]
            if len(digits) != 2 or any(char not in "0123456789abcdefABCDEF" for char in digits):
                return None
            index += 3
        elif escape == "u" and index + 1 < len(source) and source[index + 1] == "{":
            closing_brace = source.find("}", index + 2)
            if closing_brace == -1:
                return None
            digits = source[index + 2 : closing_brace]
            hexadecimal_digits = digits.replace("_", "")
            if not 1 <= len(hexadecimal_digits) <= 6 or any(
                char not in "0123456789abcdefABCDEF" for char in hexadecimal_digits
            ):
                return None
            index = closing_brace + 1
        else:
            return None

    return index + 1 if index < len(source) and source[index] == "'" else None


def _mask_non_code(source: str) -> str:
    """Replace comments and literals with spaces while keeping source offsets stable."""
    masked = list(source)
    index = 0
    while index < len(source):
        if source.startswith("//", index):
            end = source.find("\n", index)
            end = len(source) if end == -1 else end
            _mask_range(masked, index, end)
            index = end
            continue

        if source.startswith("/*", index):
            end = index + 2
            depth = 1
            while end < len(source) and depth:
                if source.startswith("/*", end):
                    depth += 1
                    end += 2
                elif source.startswith("*/", end):
                    depth -= 1
                    end += 2
                else:
                    end += 1
            _mask_range(masked, index, end)
            index = end
            continue

        raw_end = _raw_string_end(source, index)
        if raw_end is not None:
            _mask_range(masked, index, raw_end)
            index = raw_end
            continue

        if source[index] == '"':
            end = _double_quoted_end(source, index)
            _mask_range(masked, index, end)
            index = end
            continue

        if source[index] == "'":
            end = _char_literal_end(source, index)
            if end is not None:
                _mask_range(masked, index, end)
                index = end
                continue

        index += 1
    return "".join(masked)


def _diagnostic(path: PurePosixPath, line: int, reason: str) -> str:
    return f"{path.as_posix()}:{line}: {reason}"


def _platform_diagnostics(path: PurePosixPath, source: str, masked: str) -> list[str]:
    if _is_platform_test(path) or _is_platform_facade(path):
        return []

    diagnostics: list[str] = []
    for match in CFG_INVOCATION.finditer(masked):
        expression_end = _cfg_expression_end(masked, match.end() - 1)
        if OS_CFG_TERM.search(masked[match.start() : expression_end]):
            diagnostics.append(
                _diagnostic(
                    path,
                    _line_number(source, match.start()),
                    "OS-specific cfg condition is outside a platform facade",
                )
            )

    for index in _std_os_references(masked):
        diagnostics.append(
            _diagnostic(
                path,
                _line_number(source, index),
                "std::os platform module is outside a platform facade",
            )
        )

    for match in WINDOWS_SYS.finditer(masked):
        diagnostics.append(
            _diagnostic(
                path,
                _line_number(source, match.start()),
                "windows_sys is outside a platform facade",
            )
        )
    return diagnostics


def _forbidden_imports(path: PurePosixPath) -> tuple[str, ...]:
    normalized = path.as_posix()
    if normalized.startswith("crates/unica-coder/src/domain/"):
        return FORBIDDEN_DOMAIN_IMPORTS
    if normalized.startswith("crates/unica-coder/src/application/"):
        return FORBIDDEN_APPLICATION_IMPORTS
    return ()


def _dependency_diagnostics(path: PurePosixPath, source: str, masked: str) -> list[str]:
    forbidden_imports = _forbidden_imports(path)
    if not forbidden_imports:
        return []

    layer = "domain" if "/src/domain/" in path.as_posix() else "application"
    diagnostics: list[str] = []
    for reference in LAYER_REFERENCE.finditer(masked):
        module = reference.group("module")
        if module in forbidden_imports:
            prefix = reference.group("prefix")
            diagnostics.append(
                _diagnostic(
                    path,
                    _line_number(source, reference.start()),
                    f"{layer} must not reference {prefix}::{module}",
                )
            )
    for index, prefix, module in _grouped_layer_references(masked):
        if module in forbidden_imports:
            diagnostics.append(
                _diagnostic(
                    path,
                    _line_number(source, index),
                    f"{layer} must not reference {prefix}::{module}",
                )
            )
    return diagnostics


def _std_root_pattern(root: str) -> str:
    return rf"(?<!\w)(?:::)?{re.escape(root)}(?!\w)"


def _grouped_use_pattern(roots: tuple[str, ...], suffix: str = "") -> re.Pattern[str]:
    root_options = "|".join(_std_root_pattern(root) for root in roots)
    return re.compile(rf"(?:{root_options})\s*::{suffix}\s*\{{")


def _grouped_use_entries(
    masked: str, grouped_use_pattern: re.Pattern[str]
) -> list[tuple[int, str]]:
    entries: list[tuple[int, str]] = []
    for grouped_use in grouped_use_pattern.finditer(masked):
        opening_brace = grouped_use.end() - 1
        entry_start = opening_brace + 1
        depth = 1
        index = entry_start
        while index < len(masked) and depth:
            char = masked[index]
            if char == "{":
                depth += 1
            elif char == "}":
                depth -= 1
                if depth == 0:
                    entries.append((entry_start, masked[entry_start:index]))
                    break
            elif char == "," and depth == 1:
                entries.append((entry_start, masked[entry_start:index]))
                entry_start = index + 1
            index += 1
    return entries


def _grouped_layer_references(masked: str) -> list[tuple[int, str, str]]:
    references: list[tuple[int, str, str]] = []
    for prefix in ("crate", "super"):
        for entry_start, entry in _grouped_use_entries(
            masked, _grouped_use_pattern((prefix,))
        ):
            module = re.match(
                r"\s*(?P<module>application|infrastructure|interfaces)\b", entry
            )
            if module is not None:
                references.append(
                    (
                        entry_start + module.start("module"),
                        prefix,
                        module.group("module"),
                    )
                )
    return references


def _std_os_references(masked: str) -> list[int]:
    references = {match.start() for match in STD_OS.finditer(masked)}
    for entry_start, entry in _grouped_use_entries(
        masked, _grouped_use_pattern(("std",))
    ):
        os_entry = re.match(r"\s*(?P<module>os)\b", entry)
        if os_entry is None:
            continue

        direct_module = re.match(
            rf"\s*os\s*::\s*(?P<module>{RUST_IDENTIFIER_PATTERN})\b", entry
        )
        if direct_module is not None:
            references.add(entry_start + direct_module.start("module"))
            continue

        nested_entries = _grouped_use_entries(entry, re.compile(r"\bos\s*::\s*\{"))
        if nested_entries:
            for nested_start, nested_entry in nested_entries:
                module = re.match(
                    rf"\s*(?P<module>{RUST_IDENTIFIER_PATTERN})\b", nested_entry
                )
                if module is not None:
                    references.add(entry_start + nested_start + module.start("module"))
            continue

        references.add(entry_start + os_entry.start("module"))

    return sorted(references)


def _std_root_aliases(masked: str) -> tuple[str, ...]:
    direct_alias = re.compile(
        rf"\buse\s+(?:::)?std\s+as\s+(?P<alias>{RUST_IDENTIFIER_PATTERN})\s*;"
    )
    aliases = {match.group("alias") for match in direct_alias.finditer(masked)}
    for _, entry in _grouped_use_entries(masked, _grouped_use_pattern(("std",))):
        alias = re.match(
            rf"\s*self\s+as\s+(?P<alias>{RUST_IDENTIFIER_PATTERN})\s*$", entry
        )
        if alias is not None:
            aliases.add(alias.group("alias"))
    return tuple(sorted(aliases))


def _direct_std_io_roots(masked: str, roots: tuple[str, ...]) -> list[tuple[int, str]]:
    references: list[tuple[int, str]] = []
    for root in roots:
        root_reference = re.compile(
            rf"{_std_root_pattern(root)}\s*::\s*(?P<module>fs|env|process)\b"
        )
        references.extend(
            (match.start(), match.group("module")) for match in root_reference.finditer(masked)
        )
    return references


def _grouped_std_io_roots(masked: str, roots: tuple[str, ...]) -> list[tuple[int, str]]:
    """Return top-level fs/env/process entries from grouped std use statements."""
    references: list[tuple[int, str]] = []
    for entry_start, entry in _grouped_use_entries(masked, _grouped_use_pattern(roots)):
        module = re.match(r"\s*(?P<module>[A-Za-z_][A-Za-z0-9_]*)\b", entry)
        if module is not None and module.group("module") in DOMAIN_STD_IO_MODULES:
            references.append((entry_start + module.start("module"), module.group("module")))
    return references


def _path_type_aliases(masked: str, roots: tuple[str, ...]) -> tuple[str, ...]:
    aliases: set[str] = set()
    root_options = "|".join(_std_root_pattern(root) for root in roots)
    direct_alias = re.compile(
        rf"(?:{root_options})\s*::\s*path\s*::\s*"
        rf"(?:PathBuf|Path)\s+as\s+(?P<alias>{RUST_IDENTIFIER_PATTERN})"
        r"(?=\s*[,};])"
    )
    aliases.update(match.group("alias") for match in direct_alias.finditer(masked))

    grouped_path_use = _grouped_use_pattern(roots, r"\s*path\s*::")
    for _, entry in _grouped_use_entries(masked, grouped_path_use):
        alias = re.match(
            rf"\s*(?:PathBuf|Path)\s+as\s+(?P<alias>{RUST_IDENTIFIER_PATTERN})\s*$",
            entry,
        )
        if alias is not None:
            aliases.add(alias.group("alias"))

    for _, entry in _grouped_use_entries(masked, _grouped_use_pattern(roots)):
        alias = re.match(
            r"\s*path\s*::\s*(?:PathBuf|Path)\s+as\s+"
            rf"(?P<alias>{RUST_IDENTIFIER_PATTERN})\s*$",
            entry,
        )
        if alias is not None:
            aliases.add(alias.group("alias"))
            continue

        nested_path_group = re.match(r"\s*path\s*::\s*\{(?P<body>.*)\}\s*$", entry, re.DOTALL)
        if nested_path_group is not None:
            aliases.update(
                match.group("alias")
                for match in re.finditer(
                    r"\b(?:PathBuf|Path)\s+as\s+"
                    rf"(?P<alias>{RUST_IDENTIFIER_PATTERN})(?!\w)",
                    nested_path_group.group("body"),
                )
            )
    return tuple(sorted(aliases))


def _path_io_ufcs_method_pattern(
    type_names: tuple[str, ...], roots: tuple[str, ...]
) -> re.Pattern[str]:
    type_options = "|".join(re.escape(type_name) for type_name in type_names)
    root_options = "|".join(_std_root_pattern(root) for root in roots)
    qualified_std_path = (
        rf"(?:{root_options})\s*::\s*path\s*::\s*(?:PathBuf|Path)"
    )
    return re.compile(
        rf"(?<!\w)(?:"
        rf"<\s*(?:(?:{type_options})(?!\w)|{qualified_std_path})\s*>|"
        rf"(?:{type_options})(?!\w)"
        rf")\s*::\s*"
        rf"(?P<method>{DOMAIN_PATH_IO_METHOD_PATTERN})(?!\w)"
    )


def _domain_io_diagnostics(path: PurePosixPath, source: str, masked: str) -> list[str]:
    if not path.as_posix().startswith("crates/unica-coder/src/domain/"):
        return []

    diagnostics: list[str] = []
    std_roots = ("std", *_std_root_aliases(masked))
    for index, module in _direct_std_io_roots(masked, std_roots):
        diagnostics.append(
            _diagnostic(
                path,
                _line_number(source, index),
                f"domain must not access std::{module} directly",
            )
        )

    for index, module in _grouped_std_io_roots(masked, std_roots):
        diagnostics.append(
            _diagnostic(
                path,
                _line_number(source, index),
                f"domain must not access std::{module} directly",
            )
        )

    path_type_names = ("Path", "PathBuf", *_path_type_aliases(masked, std_roots))
    for method_call in _path_io_ufcs_method_pattern(path_type_names, std_roots).finditer(masked):
        method = method_call.group("method")
        diagnostics.append(
            _diagnostic(
                path,
                _line_number(source, method_call.start()),
                f"domain must not call filesystem method .{method} directly",
            )
        )
    return diagnostics


def check_source(path: str, source: str) -> list[str]:
    """Return stable ADR-0009 diagnostics for one repository-relative Rust file."""
    repository_path = _repository_path(path)
    if repository_path.suffix != ".rs":
        raise ValueError(f"path must name a Rust source file: {path}")
    masked = _mask_non_code(source)
    diagnostics = (
        _platform_diagnostics(repository_path, source, masked)
        + _dependency_diagnostics(repository_path, source, masked)
        + _domain_io_diagnostics(repository_path, source, masked)
    )
    return sorted(diagnostics, key=lambda diagnostic: int(diagnostic.split(":", 2)[1]))


def check_sources(sources: Mapping[str, str]) -> list[str]:
    """Return diagnostics sorted by repository path and then source line."""
    diagnostics: list[str] = []
    for path in sorted(sources):
        diagnostics.extend(check_source(path, sources[path]))
    return diagnostics


def collect_repository_sources(repo_root: Path) -> dict[str, str]:
    """Read tracked and nonignored untracked Rust sources without walking build output."""
    result = subprocess.run(
        [
            "git",
            "-C",
            str(repo_root),
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            "*.rs",
        ],
        text=False,
        capture_output=True,
        check=True,
    )
    sources: dict[str, str] = {}
    for raw_path in sorted(set(result.stdout.split(b"\0")) - {b""}):
        path = raw_path.decode("utf-8")
        absolute_path = repo_root / path
        if absolute_path.is_file():
            sources[path] = absolute_path.read_text(encoding="utf-8")
    return sources


def check_repository(repo_root: Path) -> list[str]:
    return check_sources(collect_repository_sources(repo_root))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-root", type=Path, default=Path("."))
    args = parser.parse_args()

    try:
        diagnostics = check_repository(args.repo_root.resolve())
    except (OSError, subprocess.CalledProcessError, UnicodeDecodeError, ValueError) as error:
        print(f"rust platform boundary error: {error}", file=sys.stderr)
        return 1

    for diagnostic in diagnostics:
        print(diagnostic)
    return 1 if diagnostics else 0


if __name__ == "__main__":
    raise SystemExit(main())
