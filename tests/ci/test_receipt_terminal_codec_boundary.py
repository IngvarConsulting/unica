from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT_PATH = REPO_ROOT / "scripts" / "ci" / "check-receipt-terminal-codec-boundary.py"

APPLICATION_PATH = Path("crates/unica-coder/src/application/receipt_ledger.rs")
STORE_PATH = Path("crates/unica-coder/src/infrastructure/receipt_ledger.rs")
CODEC_PATH = Path(
    "crates/unica-coder/src/infrastructure/daemon/terminal_codec_v5.rs"
)
DAEMON_MOD_PATH = Path("crates/unica-coder/src/infrastructure/daemon/mod.rs")


COMPLIANT_SOURCES = {
    APPLICATION_PATH: (
        "pub(crate) struct PreparedReceiptRecord;\n"
        "pub(crate) struct PreparedWireFrame;\n"
        "pub(crate) struct PreparedReceiptTerminalPublication;\n"
    ),
    STORE_PATH: (
        "fn publish() {\n"
        "    let _slot = DirectReceiptWriteSlot::new(key);\n"
        "}\n"
        "fn serialize_reserved_record() {\n"
        "    let _encoded = serde_json::to_vec(&record);\n"
        "}\n"
        "#[cfg(test)]\n"
        "mod tests {\n"
        "    fn fixture() {\n"
        "        let _ = canonical_v5_terminal(outcome);\n"
        "        let _ = b\"{\\\"resultType\\\":\\\"direct\\\"}\";\n"
        "        let _ = b\"{\\\"state\\\":\\\"direct_terminal_unacked\\\"}\";\n"
        "    }\n"
        "}\n"
    ),
    CODEC_PATH: (
        "impl DirectReceiptWriteSlot { fn new() -> Self { Self } }\n"
        "fn prepare() {\n"
        "    let _ = PreparedReceiptRecord::new(record);\n"
        "    let _ = PreparedWireFrame::new(frame);\n"
        "    let _ = PreparedReceiptTerminalPublication::new(record, frame);\n"
        "}\n"
        "#[cfg(test)]\n"
        "mod tests {\n"
        "    fn golden() { let _ = DirectReceiptWriteSlot::new(key); }\n"
        "}\n"
    ),
    DAEMON_MOD_PATH: "pub(crate) mod terminal_codec_v5;\n",
}


def write_tree(root: Path, overrides: dict[Path, str] | None = None) -> None:
    sources = dict(COMPLIANT_SOURCES)
    if overrides:
        sources.update(overrides)
    for relative_path, source in sources.items():
        path = root / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(source, encoding="utf-8")


def run_guard(root: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["python3", str(SCRIPT_PATH), "--root", str(root)],
        text=True,
        capture_output=True,
        check=False,
    )


class ReceiptTerminalCodecBoundaryTests(unittest.TestCase):
    def test_compliant_layout_passes_through_root_cli(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            write_tree(root)

            result = run_guard(root)

        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertEqual(result.stdout, "")
        self.assertEqual(result.stderr, "")

    def test_prepared_artifact_constructors_are_owned_only_by_the_codec(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            rogue_path = Path("crates/unica-coder/src/infrastructure/rogue.rs")
            write_tree(
                root,
                {
                    rogue_path: (
                        "fn bypass() {\n"
                        "    PreparedReceiptRecord :: new(record);\n"
                        "    PreparedWireFrame::new(frame);\n"
                        "    PreparedReceiptTerminalPublication::new(record, frame);\n"
                        "}\n"
                    )
                },
            )

            result = run_guard(root)

        self.assertEqual(result.returncode, 1)
        self.assertEqual(
            result.stdout.splitlines(),
            [
                f"{rogue_path.as_posix()}:2: PreparedReceiptRecord::new is owned by terminal_codec_v5",
                f"{rogue_path.as_posix()}:3: PreparedWireFrame::new is owned by terminal_codec_v5",
                f"{rogue_path.as_posix()}:4: PreparedReceiptTerminalPublication::new is owned by terminal_codec_v5",
            ],
        )

    def test_constructor_function_pointers_cannot_bypass_codec_ownership(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            rogue_path = Path("crates/unica-coder/src/infrastructure/constructor_pointer.rs")
            write_tree(
                root,
                {
                    rogue_path: (
                        "fn bypass() {\n"
                        "    let record_ctor = PreparedReceiptRecord::new;\n"
                        "    let frame_ctor = <PreparedWireFrame>::new;\n"
                        "}\n"
                    )
                },
            )

            result = run_guard(root)

        self.assertEqual(result.returncode, 1)
        self.assertEqual(
            result.stdout.splitlines(),
            [
                f"{rogue_path.as_posix()}:2: PreparedReceiptRecord::new is owned by terminal_codec_v5",
                f"{rogue_path.as_posix()}:3: PreparedWireFrame::new is owned by terminal_codec_v5",
            ],
        )

    def test_aliases_of_prepared_artifacts_are_forbidden_outside_the_codec(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            rogue_path = Path("crates/unica-coder/src/infrastructure/aliased_artifact.rs")
            write_tree(
                root,
                {
                    rogue_path: (
                        "use crate::application::receipt_ledger::{\n"
                        "    PreparedReceiptRecord as Record,\n"
                        "};\n"
                        "type Frame = crate::application::receipt_ledger::PreparedWireFrame;\n"
                        "type Slot = crate::infrastructure::daemon::terminal_codec_v5::DirectReceiptWriteSlot;\n"
                    )
                },
            )

            result = run_guard(root)

        self.assertEqual(result.returncode, 1)
        self.assertEqual(
            result.stdout.splitlines(),
            [
                f"{rogue_path.as_posix()}:2: PreparedReceiptRecord alias is forbidden outside terminal_codec_v5",
                f"{rogue_path.as_posix()}:4: PreparedWireFrame alias is forbidden outside terminal_codec_v5",
                f"{rogue_path.as_posix()}:5: DirectReceiptWriteSlot alias is forbidden outside terminal_codec_v5",
            ],
        )

    def test_generic_aliases_cannot_hide_linear_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            rogue_path = Path("crates/unica-coder/src/infrastructure/generic_alias.rs")
            write_tree(
                root,
                {
                    rogue_path: (
                        "struct Identity<T>(T);\n"
                        "type Record = Identity<PreparedReceiptRecord>;\n"
                        "impl Clone for Record {\n"
                        "    fn clone(&self) -> Self { todo!() }\n"
                        "}\n"
                        "fn bypass() { let _ = Record::new(record); }\n"
                    )
                },
            )

            result = run_guard(root)

        self.assertEqual(result.returncode, 1)
        self.assertEqual(
            result.stdout.splitlines(),
            [
                f"{rogue_path.as_posix()}:2: PreparedReceiptRecord alias is forbidden outside terminal_codec_v5",
            ],
        )

    def test_direct_write_slot_can_only_be_minted_by_store_or_codec(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            rogue_path = Path("crates/unica-coder/src/infrastructure/daemon/server.rs")
            write_tree(
                root,
                {
                    rogue_path: (
                        "fn bypass() {\n"
                        "    DirectReceiptWriteSlot /* hidden */ :: new(key);\n"
                        "}\n"
                    )
                },
            )

            result = run_guard(root)

        self.assertEqual(result.returncode, 1)
        self.assertEqual(
            result.stdout.splitlines(),
            [
                f"{rogue_path.as_posix()}:2: DirectReceiptWriteSlot::new is owned by the receipt ledger store",
            ],
        )

    def test_codec_may_define_the_slot_and_use_it_in_golden_tests_but_not_mint_it(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            write_tree(
                root,
                {
                    CODEC_PATH: (
                        "impl DirectReceiptWriteSlot { fn new() -> Self { Self } }\n"
                        "fn bypass() { let _ = DirectReceiptWriteSlot::new(key); }\n"
                        "#[cfg(test)]\n"
                        "mod tests {\n"
                        "    fn golden() { let _ = DirectReceiptWriteSlot::new(key); }\n"
                        "}\n"
                    )
                },
            )

            result = run_guard(root)

        self.assertEqual(result.returncode, 1)
        self.assertEqual(
            result.stdout.splitlines(),
            [
                f"{CODEC_PATH.as_posix()}:2: DirectReceiptWriteSlot::new is owned by the receipt ledger store",
            ],
        )

    def test_removed_direct_serializer_symbol_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            rogue_path = Path("crates/unica-coder/src/infrastructure/legacy.rs")
            write_tree(
                root,
                {rogue_path: "fn serialize_direct_terminal_record() {}\n"},
            )

            result = run_guard(root)

        self.assertEqual(result.returncode, 1)
        self.assertEqual(
            result.stdout.splitlines(),
            [
                f"{rogue_path.as_posix()}:1: serialize_direct_terminal_record is forbidden",
            ],
        )

    def test_store_production_cannot_recanonicalize_or_handwrite_direct_json(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            write_tree(
                root,
                {
                    STORE_PATH: (
                        "fn bypass() {\n"
                        "    let _ = canonical_v5_terminal(outcome);\n"
                        "    let _ = b\"{\\\"resultType\\\":\\\"direct\\\"}\";\n"
                        "    let _ = r#\"{\"state\":\"direct_terminal_unacked\"}\"#;\n"
                        "}\n"
                        "#[cfg(test)]\n"
                        "mod tests {\n"
                        "    fn fixture() { let _ = canonical_v5_terminal(outcome); }\n"
                        "}\n"
                    )
                },
            )

            result = run_guard(root)

        self.assertEqual(result.returncode, 1)
        self.assertEqual(
            result.stdout.splitlines(),
            [
                f"{STORE_PATH.as_posix()}:2: receipt ledger production must not canonicalize a Direct terminal",
                f"{STORE_PATH.as_posix()}:3: receipt ledger production must not handwrite Direct wire JSON",
                f"{STORE_PATH.as_posix()}:4: receipt ledger production must not handwrite Direct record JSON",
            ],
        )

    def test_direct_json_literals_are_forbidden_in_every_production_module(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            rogue_path = Path("crates/unica-coder/src/infrastructure/daemon/server.rs")
            write_tree(
                root,
                {
                    rogue_path: (
                        "fn bypass() {\n"
                        "    let _wire = b\"{\\\"resultType\\\":\\\"direct\\\"}\";\n"
                        "    let _record = r#\"{\"state\":\"direct_terminal_unacked\"}\"#;\n"
                        "}\n"
                        "#[cfg(test)]\n"
                        "mod tests {\n"
                        "    const WIRE: &str = r#\"{\"resultType\":\"direct\"}\"#;\n"
                        "    const RECORD: &str = r#\"{\"state\":\"direct_terminal_unacked\"}\"#;\n"
                        "}\n"
                    )
                },
            )

            result = run_guard(root)

        self.assertEqual(result.returncode, 1)
        self.assertEqual(
            result.stdout.splitlines(),
            [
                f"{rogue_path.as_posix()}:2: production must not handwrite Direct wire JSON outside terminal_codec_v5",
                f"{rogue_path.as_posix()}:3: production must not handwrite Direct record JSON outside terminal_codec_v5",
            ],
        )

    def test_direct_json_macros_are_forbidden_in_every_production_module(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            rogue_path = Path("crates/unica-coder/src/infrastructure/daemon/server.rs")
            write_tree(
                root,
                {
                    rogue_path: (
                        "fn bypass() {\n"
                        "    let _record = serde_json::json!({\"state\": \"direct_terminal_unacked\"});\n"
                        "    let _wire = json!({\"resultType\": \"direct\"});\n"
                        "}\n"
                        "#[cfg(test)]\n"
                        "mod tests {\n"
                        "    fn fixture() { let _ = json!({\"resultType\": \"direct\"}); }\n"
                        "}\n"
                    )
                },
            )

            result = run_guard(root)

        self.assertEqual(result.returncode, 1)
        self.assertEqual(
            result.stdout.splitlines(),
            [
                f"{rogue_path.as_posix()}:2: production must not handwrite Direct record JSON outside terminal_codec_v5",
                f"{rogue_path.as_posix()}:3: production must not handwrite Direct wire JSON outside terminal_codec_v5",
            ],
        )

    def test_production_after_an_earlier_test_module_is_still_checked(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            write_tree(
                root,
                {
                    STORE_PATH: (
                        "#[cfg(test)]\n"
                        "mod tests {\n"
                        "    fn fixture() { let _ = canonical_v5_terminal(outcome); }\n"
                        "}\n"
                        "fn bypass() { let _ = canonical_v5_terminal(outcome); }\n"
                    )
                },
            )

            result = run_guard(root)

        self.assertEqual(result.returncode, 1)
        self.assertEqual(
            result.stdout.splitlines(),
            [
                f"{STORE_PATH.as_posix()}:5: receipt ledger production must not canonicalize a Direct terminal",
            ],
        )

    def test_active_record_readback_cannot_reserialize_outside_reserved_encoder(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            write_tree(
                root,
                {
                    STORE_PATH: (
                        "fn read_active_record_from() {\n"
                        "    let canonical = serde_json :: to_vec ( & record );\n"
                        "}\n"
                        "fn serialize_reserved_record() {\n"
                        "    let encoded = serde_json::to_vec(&record);\n"
                        "}\n"
                        "#[cfg(test)]\n"
                        "mod tests {\n"
                        "    fn fixture() { let duplicate = serde_json::to_vec(&record); }\n"
                        "}\n"
                    )
                },
            )

            result = run_guard(root)

        self.assertEqual(result.returncode, 1)
        self.assertEqual(
            result.stdout.splitlines(),
            [
                f"{STORE_PATH.as_posix()}:2: receipt ledger production must not reserialize an active receipt record",
            ],
        )

    def test_store_cannot_use_alternate_serde_serializers_outside_reserved_encoder(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            write_tree(
                root,
                {
                    STORE_PATH: (
                        "fn bypass() {\n"
                        "    serde_json::to_writer(writer, &record);\n"
                        "    let _ = serde_json::to_string_pretty(&record);\n"
                        "    let _ = serde_json::to_value(&record);\n"
                        "    let _ = serde_json::Serializer::new(writer);\n"
                        "}\n"
                        "fn serialize_reserved_record() {\n"
                        "    serde_json::to_writer(writer, &record);\n"
                        "    let _ = serde_json::Serializer::new(writer);\n"
                        "}\n"
                        "#[cfg(test)]\n"
                        "mod tests {\n"
                        "    fn fixture() { let _ = serde_json::to_value(&record); }\n"
                        "}\n"
                    )
                },
            )

            result = run_guard(root)

        self.assertEqual(result.returncode, 1)
        self.assertEqual(
            result.stdout.splitlines(),
            [
                f"{STORE_PATH.as_posix()}:2: receipt ledger production must not reserialize an active receipt record",
                f"{STORE_PATH.as_posix()}:3: receipt ledger production must not reserialize an active receipt record",
                f"{STORE_PATH.as_posix()}:4: receipt ledger production must not reserialize an active receipt record",
                f"{STORE_PATH.as_posix()}:5: receipt ledger production must not reserialize an active receipt record",
            ],
        )

    def test_nested_function_cannot_impersonate_the_reserved_encoder_allowlist(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            write_tree(
                root,
                {
                    STORE_PATH: (
                        "fn serialize_reserved_record() {\n"
                        "    let encoded = serde_json::to_vec(&record);\n"
                        "}\n"
                        "fn bypass() {\n"
                        "    fn serialize_reserved_record() {\n"
                        "        let duplicate = serde_json::to_vec(&record);\n"
                        "    }\n"
                        "}\n"
                    )
                },
            )

            result = run_guard(root)

        self.assertEqual(result.returncode, 1)
        self.assertEqual(
            result.stdout.splitlines(),
            [
                f"{STORE_PATH.as_posix()}:6: receipt ledger production must not reserialize an active receipt record",
            ],
        )

    def test_commented_test_module_cannot_hide_store_production(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            write_tree(
                root,
                {
                    STORE_PATH: (
                        "/*\n"
                        "#[cfg(test)]\n"
                        "mod tests {\n"
                        "*/\n"
                        "fn bypass() { let _ = canonical_v5_terminal(outcome); }\n"
                        "#[cfg(test)]\n"
                        "mod tests {}\n"
                    )
                },
            )

            result = run_guard(root)

        self.assertEqual(result.returncode, 1)
        self.assertEqual(
            result.stdout.splitlines(),
            [
                f"{STORE_PATH.as_posix()}:5: receipt ledger production must not canonicalize a Direct terminal",
            ],
        )

    def test_prepared_artifacts_are_linear_and_cannot_derive_clone(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            write_tree(
                root,
                {
                    APPLICATION_PATH: (
                        "#[derive(Debug, Clone)]\n"
                        "pub(crate) struct PreparedReceiptRecord;\n"
                        "#[derive(Clone)]\n"
                        "pub(crate) struct PreparedWireFrame;\n"
                        "#[derive(PartialEq,\n"
                        "         Clone)]\n"
                        "pub(crate) struct PreparedReceiptTerminalPublication;\n"
                    )
                },
            )

            result = run_guard(root)

        self.assertEqual(result.returncode, 1)
        self.assertEqual(
            result.stdout.splitlines(),
            [
                f"{APPLICATION_PATH.as_posix()}:2: PreparedReceiptRecord must remain linear and must not derive Clone",
                f"{APPLICATION_PATH.as_posix()}:4: PreparedWireFrame must remain linear and must not derive Clone",
                f"{APPLICATION_PATH.as_posix()}:7: PreparedReceiptTerminalPublication must remain linear and must not derive Clone",
            ],
        )

    def test_manual_clone_impl_is_forbidden_for_linear_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            rogue_path = Path("crates/unica-coder/src/infrastructure/manual_clone.rs")
            write_tree(
                root,
                {
                    rogue_path: (
                        "impl Clone for PreparedReceiptRecord {\n"
                        "    fn clone(&self) -> Self { todo!() }\n"
                        "}\n"
                        "impl ::core::clone::Clone for crate::application::receipt_ledger::PreparedWireFrame {\n"
                        "    fn clone(&self) -> Self { todo!() }\n"
                        "}\n"
                        "#[allow(unused_parens)]\n"
                        "impl Clone for (PreparedReceiptTerminalPublication) {\n"
                        "    fn clone(&self) -> Self { todo!() }\n"
                        "}\n"
                    )
                },
            )

            result = run_guard(root)

        self.assertEqual(result.returncode, 1)
        self.assertEqual(
            result.stdout.splitlines(),
            [
                f"{rogue_path.as_posix()}:1: PreparedReceiptRecord must remain linear and must not implement Clone",
                f"{rogue_path.as_posix()}:4: PreparedWireFrame must remain linear and must not implement Clone",
                f"{rogue_path.as_posix()}:8: PreparedReceiptTerminalPublication must remain linear and must not implement Clone",
            ],
        )

    def test_moving_an_artifact_definition_cannot_bypass_linear_ownership(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            moved_path = Path("crates/unica-coder/src/infrastructure/moved_artifact.rs")
            write_tree(
                root,
                {
                    moved_path: (
                        "#[derive(Clone)]\n"
                        "pub(crate) struct PreparedReceiptRecord;\n"
                    )
                },
            )

            result = run_guard(root)

        self.assertEqual(result.returncode, 1)
        self.assertEqual(
            result.stdout.splitlines(),
            [
                f"{moved_path.as_posix()}:2: PreparedReceiptRecord must remain linear and must not derive Clone",
            ],
        )

    def test_terminal_codec_module_must_remain_registered(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            write_tree(root, {DAEMON_MOD_PATH: "pub(crate) mod server;\n"})

            result = run_guard(root)

        self.assertEqual(result.returncode, 1)
        self.assertEqual(
            result.stdout.splitlines(),
            [
                f"{DAEMON_MOD_PATH.as_posix()}:1: terminal_codec_v5 module is not registered",
            ],
        )

    def test_comments_and_ordinary_strings_do_not_trigger_code_symbol_rules(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            harmless_path = Path("crates/unica-coder/src/infrastructure/notes.rs")
            write_tree(
                root,
                {
                    harmless_path: (
                        "// PreparedReceiptRecord::new and DirectReceiptWriteSlot::new\n"
                        "const NOTE: &str = \"serialize_direct_terminal_record\";\n"
                    )
                },
            )

            result = run_guard(root)

        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)

    def test_current_repository_complies_with_terminal_codec_boundary(self) -> None:
        result = run_guard(REPO_ROOT)

        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)


if __name__ == "__main__":
    unittest.main()
