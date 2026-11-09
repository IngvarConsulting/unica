"""What each crate declares about itself under `[package.metadata.unica]`.

Guards derive their package sets from these declarations: a new crate states
its kind in its own manifest and does not edit a shared list.
"""

from __future__ import annotations

import functools
import tomllib
from pathlib import Path
from typing import NamedTuple

ROOT = Path(__file__).resolve().parents[2]


class Declaration(NamedTuple):
    """`component` and `file-io` of one package; `None` when not declared."""

    component: str | None
    file_io: str | None


def read(root: Path) -> dict[str, Declaration]:
    """Package name → its declaration, for every crate under `crates/`."""
    found: dict[str, Declaration] = {}
    for manifest in sorted(root.glob("crates/*/Cargo.toml")):
        package = tomllib.loads(manifest.read_text(encoding="utf-8")).get("package", {})
        name = package.get("name")
        if not isinstance(name, str):
            continue
        unica = package.get("metadata", {}).get("unica", {})
        found[name] = Declaration(unica.get("component"), unica.get("file-io"))
    return found


@functools.lru_cache(maxsize=1)
def repository() -> dict[str, Declaration]:
    """The declarations of this repository's crates."""
    return read(ROOT)


def packages(declarations: dict[str, Declaration], *, component: str | None = None,
             file_io: str | None = None) -> frozenset[str]:
    """Package names whose declaration has the given `component` and/or `file-io`."""
    return frozenset(
        name for name, declared in declarations.items()
        if (component is None or declared.component == component)
        and (file_io is None or declared.file_io == file_io)
    )
