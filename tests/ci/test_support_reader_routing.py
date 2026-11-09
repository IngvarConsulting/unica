"""Читатели поддержки объекта идут только через логический порт.

Состояние поддержки читает `SupportStateReader`. Модули-читатели и
координатор метаданных не зовут прямые чтения состояния поддержки, а
читатели, которым поддержка нужна, зовут её через порт. Доменный порт
поддержки не знает файловых путей.
"""

from __future__ import annotations

import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
INFRASTRUCTURE = "crates/unica-coder/src/infrastructure"

# Читатель, его исходник и вызовы, через которые он обязан брать поддержку.
READERS: tuple[tuple[str, str, tuple[str, ...]], ...] = (
    ("cf", f"{INFRASTRUCTURE}/native_operations/cf.rs", (".configuration_support(",)),
    ("role", f"{INFRASTRUCTURE}/native_operations/role.rs", (".object_support(",)),
    ("mxl", f"{INFRASTRUCTURE}/native_operations/mxl_composition.rs", (".object_support(",)),
    ("mxl-codec", "crates/unica-templates-designer/src/mxl_codec.rs", ()),
    ("mxl-root", "crates/unica-templates-designer/src/mxl_root.rs", ()),
    ("mxl-cells", "crates/unica-templates-designer/src/mxl_cells.rs", ()),
    ("dcs", f"{INFRASTRUCTURE}/native_operations/dcs.rs", (".object_support(",)),
    ("form", f"{INFRASTRUCTURE}/form_composition.rs", (".object_support(",)),
    ("form-codec", "crates/unica-forms-designer/src/codec.rs", ()),
    ("subsystem", f"{INFRASTRUCTURE}/subsystem_composition.rs", (".object_support(", ".subsystem_support(")),
    ("subsystem-codec", "crates/unica-metadata-designer/src/subsystem.rs", ()),
    ("meta", f"{INFRASTRUCTURE}/native_operations/meta/info.rs",
     ("unica_metadata_designer::info_read::read_typed_meta_info(",)),
    ("meta-codec", "crates/unica-metadata-designer/src/info_read.rs", (".object_support(",)),
)
COORDINATOR = f"{INFRASTRUCTURE}/metadata_operations.rs"
DOMAIN_PORT = "crates/unica-metadata/src/support_state.rs"
FORBIDDEN = ("object_support_state(", "support_state_data(", "support_status_for_path(")
PATH_TYPES = ("std::path", "&Path", "PathBuf")


def read(relative: str, root: Path = REPO_ROOT) -> str:
    return (root / relative).read_text(encoding="utf-8")


def violations(root: Path = REPO_ROOT) -> list[str]:
    found = []
    for name, relative, required_calls in READERS:
        source = read(relative, root)
        found += [f"{name} bypasses SupportStateReader through {forbidden}"
                  for forbidden in FORBIDDEN if forbidden in source]
        found += [f"{name} must route support through {required}"
                  for required in required_calls if required not in source]
    coordinator = read(COORDINATOR, root)
    found += [f"metadata coordinator bypasses SupportStateReader through {forbidden}"
              for forbidden in FORBIDDEN if forbidden in coordinator]
    port = read(DOMAIN_PORT, root)
    found += [f"the domain support port names a file path through {name}" for name in PATH_TYPES if name in port]
    return found


class SupportReaderRoutingTests(unittest.TestCase):
    def test_support_readers_cannot_bypass_the_logical_port(self) -> None:
        self.assertEqual(violations(), [])


if __name__ == "__main__":
    unittest.main()
