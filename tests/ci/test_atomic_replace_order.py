"""Атомарная замена файла сбрасывает переименованный дескриптор до каталога.

`replace_regular_child_atomically` переименовывает подготовленный файл через
дескриптор каталога, затем сбрасывает ещё открытый на запись подготовленный
дескриптор и только потом синхронизирует каталог. Windows-замена идёт через
переименование открытого дескриптора с POSIX-семантикой, без `MoveFileExW`.
"""

from __future__ import annotations

import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SOURCE = "crates/unica-workspace/src/platform/filesystem.rs"


def between(source: str, start: str, end: str) -> str | None:
    """Срез от первого `start` до первого `end` после него."""
    begin = source.find(start)
    if begin < 0:
        return None
    finish = source.find(end, begin)
    return None if finish < 0 else source[begin:finish]


def violations(source: str) -> list[str]:
    source = source.replace("\r\n", "\n")
    found = []
    publisher = between(source, "pub(crate) fn replace_regular_child_atomically(",
                        "pub(crate) fn read_relative_regular_bounded(")
    if publisher is None:
        found.append("the retained atomic publisher has no bounded source slice")
    else:
        rename = publisher.find("replace_identity_bound_regular_child(")
        flush = publisher.find("sync_renamed_regular_child(")
        directory = publisher.find("sync_directory(")
        if min(rename, flush, directory) < 0:
            found.append("the publisher lacks its rename, renamed-handle flush or directory sync")
        elif not rename < flush < directory:
            found.append("rename must precede descriptor flush, which must precede directory sync")
        if "sync_renamed_regular_child(&staged.file)" not in publisher:
            found.append("the still-writable staged handle must flush the physical object after rename")
    replace = between(source, "#[cfg(windows)]\npub(crate) fn replace_identity_bound_regular_child(",
                      "#[cfg(not(any(unix, windows)))]")
    if replace is None:
        found.append("the Windows descriptor-relative replacement has no bounded source slice")
    else:
        if "rename_open_child(" not in replace:
            found.append("the Windows replacement must rename the open handle")
        if "MoveFileExW" in replace:
            found.append("the Windows replacement must not rename by path through MoveFileExW")
    rename_open = between(source, "#[cfg(windows)]\nfn rename_open_child(",
                          "#[cfg(unix)]\npub(crate) fn create_owner_only_directory_child(")
    if rename_open is None:
        found.append("the Windows handle-bound rename has no bounded source slice")
    else:
        for name in ("FILE_RENAME_INFORMATION_EX_CLASS", "FILE_RENAME_POSIX_SEMANTICS"):
            if name not in rename_open:
                found.append(f"the Windows handle-bound rename must use {name}")
    return found


class AtomicReplaceOrderTests(unittest.TestCase):
    def test_retained_atomic_replace_flushes_renamed_handle_before_directory_sync(self) -> None:
        self.assertEqual(violations((REPO_ROOT / SOURCE).read_text(encoding="utf-8")), [])


if __name__ == "__main__":
    unittest.main()
