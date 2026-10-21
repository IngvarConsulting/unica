"""Prepared binary consumers reject stale, test-feature and substituted bytes."""
from __future__ import annotations
import copy
import io
import json
import os
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/ci"))
import prepared_binary as product


class PreparedBinaryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.identity = {"sha": "a" * 40, "sourceDigest": "source", "os": "Linux", "target": "x86_64-unknown-linux-gnu", "toolchain": "rustc pinned", "cfg": ["unix"], "rustflags": {}, "lockSha256": "lock"}
        self.prepared = self.root / "prepared"
        self.prepared.mkdir()
        self.binary = self.prepared / "unica"
        self.binary.write_bytes(b"ordinary-product-bytes")
        self.binary.chmod(0o555)
        self.manifest = self.prepared / "manifest.json"
        self.document = {"schemaVersion": 1, "kind": "ordinary-product", "identity": self.identity,
                         "profile": "dev", "features": ["default"], "binary": "unica", "mode": 0o555,
                         "sha256": product.digest(self.binary), "bytes": self.binary.stat().st_size}
        self.save()

    def save(self):
        self.manifest.write_text(json.dumps(self.document))

    def test_verified_consumer_does_not_call_cargo_and_uses_immutable_path(self):
        with patch.dict(os.environ, {product.MANIFEST_ENV: str(self.manifest)}), patch.object(product, "identity", return_value=self.identity), patch.object(product.subprocess, "run") as build:
            self.assertEqual(product.ensure_binary(self.root), self.binary.resolve())
            self.assertEqual(product.binary_path(self.root), self.binary.resolve())
        build.assert_not_called()

    def test_missing_or_empty_manifest_never_falls_back_to_build(self):
        for path in ("", str(self.root / "missing.json")):
            with self.subTest(path=path), patch.dict(os.environ, {product.MANIFEST_ENV: path}), patch.object(product.subprocess, "run") as build:
                with self.assertRaises((ValueError, FileNotFoundError)):
                    product.ensure_binary(self.root)
                build.assert_not_called()

    def test_ci_without_manifest_refuses_instead_of_building(self):
        environment = {key: value for key, value in os.environ.items() if key != product.MANIFEST_ENV}
        with patch.dict(os.environ, {**environment, "GITHUB_ACTIONS": "true"}, clear=True), patch.object(product.subprocess, "run") as build:
            with self.assertRaisesRegex(ValueError, product.MANIFEST_ENV):
                product.ensure_binary(self.root)
        build.assert_not_called()

    def test_local_build_without_manifest_is_announced_and_uses_the_one_ordinary_command(self):
        environment = {key: value for key, value in os.environ.items() if key not in (product.MANIFEST_ENV, "GITHUB_ACTIONS", "CARGO_TARGET_DIR")}
        binary = self.root / "target" / "debug" / ("unica.exe" if os.name == "nt" else "unica")
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b"local build")
        stderr = io.StringIO()
        with patch.dict(os.environ, environment, clear=True), patch.object(product.subprocess, "run") as build, patch.object(sys, "stderr", stderr):
            self.assertEqual(product.ensure_binary(self.root), binary.resolve())
        build.assert_called_once_with(product.LOCAL_BUILD, cwd=self.root, check=True)
        self.assertEqual(product.LOCAL_BUILD, ["cargo", "build", "--quiet", "--locked", "-p", "unica-coder", "--bin", "unica"])
        self.assertIn(" ".join(product.LOCAL_BUILD), stderr.getvalue())
        self.assertIn(f"{product.MANIFEST_ENV}=DIR/manifest.json", stderr.getvalue())

    def test_wrong_source_target_toolchain_features_or_path_are_rejected(self):
        changes = [("features", ["default", "receipt-ledger-test-support"]), ("features", []),
                   ("profile", "release"), ("binary", "../unica"), ("mode", 0o755), ("schemaVersion", 999)]
        for field, value in changes:
            with self.subTest(field=field, value=value):
                original = copy.deepcopy(self.document)
                self.document[field] = value
                self.save()
                with self.assertRaises(ValueError):
                    product.verify(self.manifest, self.root, expected=self.identity)
                self.document = original
        self.save()
        for field in self.identity:
            changed = {**self.identity, field: "different"}
            with self.subTest(identity=field), self.assertRaises(ValueError):
                product.verify(self.manifest, self.root, expected=changed)

    def test_substituted_bytes_and_writable_files_are_rejected(self):
        self.binary.chmod(0o755)
        if os.name != "nt":
            with self.assertRaises(ValueError):
                product.verify(self.manifest, self.root, expected=self.identity)
        self.binary.write_bytes(b"substituted")
        self.binary.chmod(0o555)
        with self.assertRaises(ValueError):
            product.verify(self.manifest, self.root, expected=self.identity)

    def test_archive_roundtrip_is_deterministic_and_checks_members_before_publication(self):
        first, second = self.root / "first.tar.gz", self.root / "second.tar.gz"
        with patch.object(product, "identity", return_value=self.identity):
            product.pack(self.manifest, first, self.root)
            product.pack(self.manifest, second, self.root)
            self.assertEqual(first.read_bytes(), second.read_bytes())
            binary = product.unpack(first, self.root / "extracted", self.root)
            self.assertEqual(binary.read_bytes(), self.binary.read_bytes())
            self.assertEqual(product.verify(binary.parent / "manifest.json", self.root), binary)
            for name, kind in (("../outside", tarfile.REGTYPE), ("unica", tarfile.SYMTYPE)):
                bad = self.root / "bad.tar.gz"
                with tarfile.open(bad, "w:gz") as archive:
                    member = tarfile.TarInfo(name)
                    member.type, member.size = kind, 1
                    archive.addfile(member, io.BytesIO(b"x"))
                destination = self.root / "not-published"
                with self.assertRaises(ValueError):
                    product.unpack(bad, destination, self.root)
                self.assertFalse(destination.exists())

    def test_source_digest_includes_new_modified_deleted_and_executable_inputs(self):
        source = self.root / "source.rs"
        source.write_text("before")
        with patch.object(product.subprocess, "check_output", return_value=b"source.rs\0"):
            before = product.source_digest(self.root)
            source.write_text("after")
            self.assertNotEqual(before, product.source_digest(self.root))
            source.unlink()
            self.assertNotEqual(before, product.source_digest(self.root))


if __name__ == "__main__":
    unittest.main()
