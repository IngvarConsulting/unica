#!/usr/bin/env python3
"""Keep v5 Direct terminal bytes owned by the sole preflight codec."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path, PurePosixPath


SOURCE_ROOT = PurePosixPath("crates/unica-coder/src")
APPLICATION_PATH = PurePosixPath(
    "crates/unica-coder/src/application/receipt_ledger.rs"
)
STORE_PATH = PurePosixPath("crates/unica-coder/src/infrastructure/receipt_ledger.rs")
CODEC_PATH = PurePosixPath(
    "crates/unica-coder/src/infrastructure/daemon/terminal_codec_v5.rs"
)
DAEMON_MOD_PATH = PurePosixPath(
    "crates/unica-coder/src/infrastructure/daemon/mod.rs"
)
REQUIRED_PATHS = (APPLICATION_PATH, STORE_PATH, CODEC_PATH, DAEMON_MOD_PATH)

PREPARED_CONSTRUCTORS = (
    "PreparedReceiptRecord",
    "PreparedWireFrame",
    "PreparedReceiptTerminalPublication",
)
LINEAR_ARTIFACTS = PREPARED_CONSTRUCTORS
FORBIDDEN_SERIALIZER = "serialize_direct_terminal_record"
DIRECT_SLOT = "DirectReceiptWriteSlot"
RUST_IDENTIFIER = r"[A-Za-z_][A-Za-z0-9_]*"
TEST_MODULE = re.compile(
    rf"(?m)^[ \t]*#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\][ \t]*\r?\n"
    rf"[ \t]*(?:pub(?:\s*\([^)]*\))?[ \t]+)?mod[ \t]+{RUST_IDENTIFIER}[ \t]*\{{"
)


def _line_number(source: str, offset: int) -> int:
    return source.count("\n", 0, offset) + 1


def _diagnostic(path: PurePosixPath, source: str, offset: int, reason: str) -> str:
    return f"{path.as_posix()}:{_line_number(source, offset)}: {reason}"


def _mask_range(masked: list[str], start: int, end: int) -> None:
    for index in range(start, end):
        if masked[index] not in {"\n", "\r"}:
            masked[index] = " "


def _raw_string_end(source: str, start: int) -> int | None:
    marker = start
    if source.startswith("br", start):
        marker += 1
    elif not source.startswith("r", start):
        return None
    if start and (source[start - 1].isalnum() or source[start - 1] == "_"):
        return None

    hashes_start = marker + 1
    quote = hashes_start
    while quote < len(source) and source[quote] == "#":
        quote += 1
    if quote >= len(source) or source[quote] != '"':
        return None
    hashes = source[hashes_start:quote]
    terminator = '"' + hashes
    closing = source.find(terminator, quote + 1)
    return len(source) if closing == -1 else closing + len(terminator)


def _quoted_string_end(source: str, start: int) -> int:
    index = start + 1
    while index < len(source):
        if source[index] == "\\":
            index += 2
            continue
        if source[index] == '"':
            return index + 1
        index += 1
    return len(source)


def _char_literal_end(source: str, start: int) -> int | None:
    """Recognize ordinary Rust char literals without consuming lifetimes."""
    index = start + 1
    if index >= len(source) or source[index] in {"'", "\n", "\r"}:
        return None
    if source[index] == "\\":
        index += 2
    else:
        index += 1
    return index + 1 if index < len(source) and source[index] == "'" else None


def _mask_non_code(source: str) -> str:
    """Mask comments and literals while preserving offsets and line numbers."""
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
            end = _quoted_string_end(source, index)
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


def _string_literals(source: str) -> list[tuple[int, str]]:
    """Return Rust string literal payloads, ignoring comments."""
    literals: list[tuple[int, str]] = []
    index = 0
    while index < len(source):
        if source.startswith("//", index):
            end = source.find("\n", index)
            index = len(source) if end == -1 else end
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
            index = end
            continue

        raw_end = _raw_string_end(source, index)
        if raw_end is not None:
            marker = index + (1 if source.startswith("br", index) else 0)
            quote = marker + 1
            while source[quote] == "#":
                quote += 1
            hashes = source[marker + 1 : quote]
            payload_end = raw_end - 1 - len(hashes)
            literals.append((index, source[quote + 1 : payload_end]))
            index = raw_end
            continue
        if source[index] == '"':
            end = _quoted_string_end(source, index)
            payload = source[index + 1 : max(index + 1, end - 1)]
            payload = payload.replace('\\"', '"').replace("\\\\", "\\")
            literals.append((index, payload))
            index = end
            continue
        if source[index] == "'":
            end = _char_literal_end(source, index)
            if end is not None:
                index = end
                continue
        index += 1
    return literals


def _brace_range(masked: str, opening_brace: int) -> tuple[int, int]:
    depth = 0
    for index in range(opening_brace, len(masked)):
        if masked[index] == "{":
            depth += 1
        elif masked[index] == "}":
            depth -= 1
            if depth == 0:
                return opening_brace, index + 1
    return opening_brace, len(masked)


def _delimiter_range(masked: str, opening: int) -> tuple[int, int]:
    pairs = {"{": "}", "[": "]", "(": ")"}
    stack: list[str] = []
    for index in range(opening, len(masked)):
        token = masked[index]
        if token in pairs:
            stack.append(pairs[token])
        elif stack and token == stack[-1]:
            stack.pop()
            if not stack:
                return opening, index + 1
    return opening, len(masked)


def _test_module_ranges(masked: str) -> list[tuple[int, int]]:
    ranges: list[tuple[int, int]] = []
    for match in TEST_MODULE.finditer(masked):
        opening_brace = masked.rfind("{", match.start(), match.end())
        if opening_brace != -1:
            ranges.append(_brace_range(masked, opening_brace))
    return ranges


def _function_ranges(masked: str, function_name: str) -> list[tuple[int, int]]:
    ranges: list[tuple[int, int]] = []
    declaration = re.compile(rf"\bfn\s+{re.escape(function_name)}\b")
    for match in declaration.finditer(masked):
        if masked.count("{", 0, match.start()) != masked.count(
            "}", 0, match.start()
        ):
            continue
        opening_brace = masked.find("{", match.end())
        semicolon = masked.find(";", match.end())
        if opening_brace == -1 or (semicolon != -1 and semicolon < opening_brace):
            continue
        ranges.append(_brace_range(masked, opening_brace))
    return ranges


def _inside_ranges(offset: int, ranges: list[tuple[int, int]]) -> bool:
    return any(start <= offset < end for start, end in ranges)


def _constructor_pattern(type_name: str) -> re.Pattern[str]:
    return re.compile(
        rf"\b{re.escape(type_name)}\s*(?:>\s*)?::\s*new\b"
    )


def _source_diagnostics(
    path: PurePosixPath, source: str, masked: str
) -> list[str]:
    diagnostics: list[str] = []
    test_ranges = _test_module_ranges(masked)
    for type_name in PREPARED_CONSTRUCTORS:
        if path == CODEC_PATH:
            continue
        for match in _constructor_pattern(type_name).finditer(masked):
            diagnostics.append(
                _diagnostic(
                    path,
                    source,
                    match.start(),
                    f"{type_name}::new is owned by terminal_codec_v5",
                )
            )

    if path != CODEC_PATH:
        for type_name in (*PREPARED_CONSTRUCTORS, DIRECT_SLOT):
            import_alias = re.compile(
                rf"\b(?P<artifact>{re.escape(type_name)})\s+as\s+{RUST_IDENTIFIER}\b"
            )
            for match in import_alias.finditer(masked):
                diagnostics.append(
                    _diagnostic(
                        path,
                        source,
                        match.start("artifact"),
                        f"{type_name} alias is forbidden outside terminal_codec_v5",
                    )
                )

            type_alias = re.compile(
                rf"\btype\s+{RUST_IDENTIFIER}(?:\s*<[^;=]*>)?\s*=\s*"
                rf"(?P<rhs>[^;]+);"
            )
            for match in type_alias.finditer(masked):
                artifact = re.search(
                    rf"\b{re.escape(type_name)}\b", match.group("rhs")
                )
                if artifact is None:
                    continue
                diagnostics.append(
                    _diagnostic(
                        path,
                        source,
                        match.start("rhs") + artifact.start(),
                        f"{type_name} alias is forbidden outside terminal_codec_v5",
                    )
                )

    slot_calls = list(_constructor_pattern(DIRECT_SLOT).finditer(masked))
    if path == STORE_PATH:
        slot_calls = []
    elif path == CODEC_PATH:
        slot_calls = [
            match for match in slot_calls if not _inside_ranges(match.start(), test_ranges)
        ]
    for match in slot_calls:
        diagnostics.append(
            _diagnostic(
                path,
                source,
                match.start(),
                "DirectReceiptWriteSlot::new is owned by the receipt ledger store",
            )
        )

    for match in re.finditer(rf"\b{FORBIDDEN_SERIALIZER}\b", masked):
        diagnostics.append(
            _diagnostic(
                path,
                source,
                match.start(),
                f"{FORBIDDEN_SERIALIZER} is forbidden",
            )
        )
    return diagnostics


def _store_diagnostics(source: str, masked: str) -> list[str]:
    diagnostics: list[str] = []
    test_ranges = _test_module_ranges(masked)
    for match in re.finditer(r"\bcanonical_v5_terminal\s*\(", masked):
        if _inside_ranges(match.start(), test_ranges):
            continue
        diagnostics.append(
            _diagnostic(
                STORE_PATH,
                source,
                match.start(),
                "receipt ledger production must not canonicalize a Direct terminal",
            )
        )

    reserved_encoder_ranges = _function_ranges(masked, "serialize_reserved_record")
    serde_json_use = re.compile(r"\bserde_json\b")
    allowed_deserializer = re.compile(r"serde_json\s*::\s*from_slice\b")
    for match in serde_json_use.finditer(masked):
        if _inside_ranges(match.start(), test_ranges) or _inside_ranges(
            match.start(), reserved_encoder_ranges
        ):
            continue
        if allowed_deserializer.match(masked, match.start()) is not None:
            continue
        diagnostics.append(
            _diagnostic(
                STORE_PATH,
                source,
                match.start(),
                "receipt ledger production must not reserialize an active receipt record",
            )
        )
    return diagnostics


def _direct_literal_diagnostics(
    path: PurePosixPath, source: str, masked: str
) -> list[str]:
    if path == CODEC_PATH:
        return []

    diagnostics: list[str] = []
    test_ranges = _test_module_ranges(masked)
    manual_markers = (
        (
            '"resultType":"direct"',
            "production must not handwrite Direct wire JSON outside terminal_codec_v5",
        ),
        (
            '"state":"direct_terminal_unacked"',
            "production must not handwrite Direct record JSON outside terminal_codec_v5",
        ),
    )
    if path == STORE_PATH:
        manual_markers = (
            (
                '"resultType":"direct"',
                "receipt ledger production must not handwrite Direct wire JSON",
            ),
            (
                '"state":"direct_terminal_unacked"',
                "receipt ledger production must not handwrite Direct record JSON",
            ),
        )
    literals = _string_literals(source)
    for offset, payload in literals:
        if _inside_ranges(offset, test_ranges):
            continue
        compact = re.sub(r"\s+", "", payload)
        for marker, reason in manual_markers:
            if marker in compact:
                diagnostics.append(_diagnostic(path, source, offset, reason))

    json_macro = re.compile(
        r"\b(?:serde_json\s*::\s*)?json\s*!\s*(?P<opening>[\{\[\(])"
    )
    macro_markers = (
        (
            ("state", "direct_terminal_unacked"),
            "production must not handwrite Direct record JSON outside terminal_codec_v5",
        ),
        (
            ("resultType", "direct"),
            "production must not handwrite Direct wire JSON outside terminal_codec_v5",
        ),
    )
    for match in json_macro.finditer(masked):
        if _inside_ranges(match.start(), test_ranges):
            continue
        start, end = _delimiter_range(masked, match.start("opening"))
        payloads = [
            payload for offset, payload in literals if start <= offset < end
        ]
        adjacent = set(zip(payloads, payloads[1:]))
        for marker, reason in macro_markers:
            if marker in adjacent:
                diagnostics.append(_diagnostic(path, source, match.start(), reason))
    return diagnostics


def _linear_artifact_diagnostics(
    path: PurePosixPath, source: str, masked: str
) -> list[str]:
    diagnostics: list[str] = []
    for artifact in LINEAR_ARTIFACTS:
        declaration = re.compile(
            rf"(?m)(?P<attrs>(?:^[ \t]*#\s*\[[^\]]*\][ \t]*(?:\r?\n|$))*)"
            rf"^[ \t]*(?:pub(?:\s*\([^)]*\))?[ \t]+)?struct[ \t]+"
            rf"(?P<name>{re.escape(artifact)})\b"
        )
        for match in declaration.finditer(masked):
            attrs = match.group("attrs")
            derives_clone = any(
                re.search(r"(?:^|,)\s*Clone\s*(?:,|$)", derive.group("body"))
                for derive in re.finditer(r"\bderive\s*\((?P<body>[^)]*)\)", attrs)
            )
            if not derives_clone:
                continue
            diagnostics.append(
                _diagnostic(
                    path,
                    source,
                    match.start("name"),
                    f"{artifact} must remain linear and must not derive Clone",
                )
            )

        qualified_path = rf"(?:::)?(?:{RUST_IDENTIFIER}\s*::\s*)*"
        clone_impl = re.compile(
            rf"\bimpl(?:\s*<[^>{{}};]*>)?\s+"
            rf"{qualified_path}Clone\s+for\s+"
            rf"(?:\(\s*)*{qualified_path}(?P<artifact>{re.escape(artifact)})\b"
            rf"(?:\s*\))*"
        )
        for match in clone_impl.finditer(masked):
            diagnostics.append(
                _diagnostic(
                    path,
                    source,
                    match.start(),
                    f"{artifact} must remain linear and must not implement Clone",
                )
            )
    return diagnostics


def _module_diagnostics(source: str, masked: str) -> list[str]:
    registered = re.search(
        r"(?m)^[ \t]*(?:pub(?:\s*\([^)]*\))?[ \t]+)?"
        r"mod[ \t]+terminal_codec_v5[ \t]*;",
        masked,
    )
    if registered is not None:
        return []
    return [f"{DAEMON_MOD_PATH.as_posix()}:1: terminal_codec_v5 module is not registered"]


def collect_sources(root: Path) -> dict[PurePosixPath, str]:
    source_directory = root.joinpath(*SOURCE_ROOT.parts)
    sources: dict[PurePosixPath, str] = {}
    if source_directory.is_dir():
        for absolute_path in sorted(source_directory.rglob("*.rs")):
            if absolute_path.is_file():
                relative_path = PurePosixPath(absolute_path.relative_to(root).as_posix())
                sources[relative_path] = absolute_path.read_text(encoding="utf-8")
    return sources


def check_root(root: Path) -> list[str]:
    sources = collect_sources(root)
    diagnostics: list[str] = []
    for required_path in REQUIRED_PATHS:
        if required_path not in sources:
            diagnostics.append(f"{required_path.as_posix()}:1: required source file is missing")

    masked_sources = {path: _mask_non_code(source) for path, source in sources.items()}
    for path in sorted(sources, key=PurePosixPath.as_posix):
        diagnostics.extend(
            _source_diagnostics(path, sources[path], masked_sources[path])
        )
        diagnostics.extend(
            _linear_artifact_diagnostics(
                path, sources[path], masked_sources[path]
            )
        )
        diagnostics.extend(
            _direct_literal_diagnostics(
                path, sources[path], masked_sources[path]
            )
        )
    if STORE_PATH in sources:
        diagnostics.extend(_store_diagnostics(sources[STORE_PATH], masked_sources[STORE_PATH]))
    if DAEMON_MOD_PATH in sources:
        diagnostics.extend(
            _module_diagnostics(sources[DAEMON_MOD_PATH], masked_sources[DAEMON_MOD_PATH])
        )
    return sorted(
        diagnostics,
        key=lambda diagnostic: (
            diagnostic.split(":", 1)[0],
            int(diagnostic.split(":", 2)[1]),
            diagnostic,
        ),
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path("."))
    args = parser.parse_args()
    try:
        diagnostics = check_root(args.root.resolve())
    except (OSError, UnicodeDecodeError, ValueError) as error:
        print(f"receipt terminal codec boundary error: {error}", file=sys.stderr)
        return 1
    for diagnostic in diagnostics:
        print(diagnostic)
    return 1 if diagnostics else 0


if __name__ == "__main__":
    raise SystemExit(main())
