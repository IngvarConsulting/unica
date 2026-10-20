from contextlib import redirect_stderr
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from tests.ci import private_provider_failure_evidence as evidence


def record(**changes):
    return {"version": 1, "provider": "bsl_analyzer", "phase": "wait_response",
            "cause": "invalid_json", "io_kind": None, "service_pid": 101,
            "helper_pid": 102, "request_id": 2, **changes}


def line(value=None):
    return evidence.RECORD_PREFIX + json.dumps(
        record() if value is None else value, separators=(",", ":")
    ).encode() + b"\n"


def service(workspace, *, name="svc-0123456789abcdef", owner=None):
    workspace.mkdir(parents=True, exist_ok=True)
    (workspace / "v8project.yaml").write_text("", encoding="utf-8")
    directory = workspace / ".build/unica/services" / name
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "service.json").write_text(json.dumps(
        owner if owner is not None else {"pid": 101, "workspace_root": str(workspace.resolve()),
                                         "token": "PRIVATE_OWNER_TOKEN"}
    ), encoding="utf-8")
    log = directory / "service.stderr.log"
    log.write_bytes(b"")
    return log


class PrivateProviderFailureEvidenceTests(unittest.TestCase):
    def test_new_failure_excludes_old_records_and_raw_secret_text(self):
        with tempfile.TemporaryDirectory() as raw:
            workspace = Path(raw)
            log = service(workspace)
            log.write_bytes(line(record(request_id=2)))
            baseline = evidence.before_call(workspace, cache_is_default=True)
            with log.open("ab") as stream:
                stream.write(b"PRIVATE_STDERR_SECRET\nraw error PRIVATE_PAYLOAD\n")
                stream.write(line(record(request_id=3)))
            output = io.StringIO()
            evidence.emit_failure_evidence(workspace, baseline, cache_is_default=True,
                                           case_ordinal=1, step_ordinal=5, writer=output)
            captured = json.loads(output.getvalue().removeprefix(evidence.EVIDENCE_PREFIX))
            self.assertEqual(captured["record"], record(request_id=3))
            self.assertEqual(captured["capture_status"], "captured")
            for secret in ("PRIVATE_STDERR_SECRET", "PRIVATE_PAYLOAD", "PRIVATE_OWNER_TOKEN"):
                self.assertNotIn(secret, output.getvalue())
            self.assertNotIn(str(workspace), output.getvalue())

    def test_run_corpus_keeps_original_failure_and_copies_evidence_before_cleanup(self):
        from tests.ci import test_acceptance_scenarios as corpus_runner

        output, closed, workspaces = io.StringIO(), [], []

        class Server:
            provider_failure_cache_is_default = True

            def __init__(self, workspace, *_):
                self.workspace = workspace
                self.log = service(workspace)
                workspaces.append(workspace)

            def call(self, *_):
                self.log.write_bytes(b"PRIVATE_PAYLOAD\n" + line())
                return {}

            def close(self):
                closed.append("captured" in output.getvalue())

        corpus = {"workspace": "unused-owned-fixture", "protocolVersion": "2025-03-26",
                  "scenarios": [{"id": "fixed-case", "wire": [
                      {"tool": "unica.search", "args": {}, "expect": ["ok"]}]}]}

        def copy_workspace(_, destination):
            destination.mkdir(parents=True)

        with (patch.object(corpus_runner, "AcceptanceServer", Server),
              patch.object(corpus_runner.shutil, "copytree", copy_workspace),
              patch.object(corpus_runner, "classify", return_value=("provider", "original refusal")),
              redirect_stderr(output)):
            with self.assertRaisesRegex(AssertionError, "original refusal"):
                corpus_runner.run_corpus(self, corpus)
        self.assertTrue(closed[0], "safe record must reach the log before server cleanup")
        self.assertTrue(all(not path.exists() for path in workspaces))
        captured = json.loads(output.getvalue().removeprefix(evidence.EVIDENCE_PREFIX))
        self.assertEqual(captured["record"], record())
        self.assertNotIn("PRIVATE_PAYLOAD", output.getvalue())

    def test_run_corpus_success_writes_no_failure_diagnostic(self):
        from tests.ci import test_acceptance_scenarios as corpus_runner

        class Server:
            provider_failure_cache_is_default = True

            def __init__(self, workspace, *_):
                self.workspace = workspace
                service(workspace)

            def call(self, *_):
                return {}

            def close(self):
                pass

        corpus = {"workspace": "unused-owned-fixture", "protocolVersion": "2025-03-26",
                  "scenarios": [{"id": "paired-control", "wire": [
                      {"tool": "unica.search", "args": {}, "expect": ["ok"]}]}]}
        output = io.StringIO()
        with (patch.object(corpus_runner, "AcceptanceServer", Server),
              patch.object(corpus_runner.shutil, "copytree", lambda _, path: path.mkdir(parents=True)),
              patch.object(corpus_runner, "classify", return_value=("ok", "")),
              patch.object(corpus_runner, "content_mismatches", return_value=[]),
              patch.object(corpus_runner, "capture_values", return_value={}),
              redirect_stderr(output)):
            corpus_runner.run_corpus(self, corpus)
        self.assertEqual(output.getvalue(), "")

    def test_new_service_after_call_can_supply_the_record(self):
        with tempfile.TemporaryDirectory() as raw:
            workspace = Path(raw)
            (workspace / "v8project.yaml").write_text("", encoding="utf-8")
            baseline = evidence.before_call(workspace, cache_is_default=True)
            service(workspace).write_bytes(line())
            self.assertEqual(evidence.collect_failure(
                workspace, baseline, cache_is_default=True
            ), evidence.FailureCapture("captured", record()))

    def test_unsupported_cache_override_performs_no_discovery(self):
        from tests.ci import test_acceptance_scenarios as corpus_runner

        with tempfile.TemporaryDirectory() as raw:
            workspace = Path(raw)
            (workspace / "v8project.yaml").write_text("", encoding="utf-8")
            state = workspace / "state"
            state.mkdir()
            for override, expected_default in (
                (None, True), ("relative-cache", False),
                ("/private/external-cache", False), ("", False),
            ):
                environment = {} if override is None else {"UNICA_CACHE_DIR": override}
                process = SimpleNamespace(
                    stdin=io.BytesIO(),
                    stdout=io.BytesIO(b'{"jsonrpc":"2.0","id":1,"result":{}}\n'),
                    terminate=lambda: None,
                    wait=lambda **_: 0,
                )
                with self.subTest(override=override), patch.object(
                    corpus_runner.subprocess, "Popen", return_value=process
                ) as popen:
                    server = corpus_runner.AcceptanceServer(
                        workspace, state, "2025-03-26", environment=environment
                    )
                    try:
                        self.assertEqual(server.provider_failure_cache_is_default, expected_default)
                        popen.assert_called_once()
                        forwarded = popen.call_args.kwargs["env"]
                        if override is None:
                            self.assertNotIn("UNICA_CACHE_DIR", forwarded)
                            baseline = evidence.before_call(
                                workspace, cache_is_default=server.provider_failure_cache_is_default
                            )
                            self.assertEqual(baseline.status, "not_present")
                        else:
                            self.assertEqual(forwarded["UNICA_CACHE_DIR"], override)
                            with patch.object(Path, "resolve", side_effect=AssertionError):
                                baseline = evidence.before_call(
                                    workspace, cache_is_default=server.provider_failure_cache_is_default
                                )
                            self.assertEqual(baseline.status, "unlocated")
                    finally:
                        server.close()

    def test_multiple_services_and_multiple_new_records_are_ambiguous(self):
        with tempfile.TemporaryDirectory() as raw:
            workspace = Path(raw)
            log = service(workspace)
            baseline = evidence.before_call(workspace, cache_is_default=True)
            log.write_bytes(line() + line(record(request_id=3)))
            self.assertEqual(evidence.collect_failure(workspace, baseline, cache_is_default=True).status,
                             "ambiguous")
            service(workspace, name="svc-fedcba9876543210")
            self.assertEqual(evidence.before_call(workspace, cache_is_default=True).status, "ambiguous")

    def test_record_rejects_payloads_duplicate_keys_and_false_numeric_identities(self):
        candidates = [record(message="PRIVATE_SECRET"), record(phase="PRIVATE_SECRET"),
                      record(cause="PRIVATE_SECRET"), record(io_kind="PRIVATE_SECRET"),
                      record(service_pid=True), record(helper_pid=-1), record(request_id=2**63),
                      record(request_id="PRIVATE_SECRET"), record(cause="write_io"),
                      record(phase="decode_tool_result", cause="read_io", io_kind="other")]
        for candidate in candidates:
            with self.subTest(candidate=candidate), self.assertRaises(ValueError):
                evidence.validate_record(line(candidate))
        for malformed in (line()[:-1], line().replace(b'"version":1', b'"version":1,"version":1'),
                          evidence.RECORD_PREFIX + b'{"version":NaN}\n'):
            with self.subTest(line=malformed), self.assertRaises(ValueError):
                evidence.validate_record(malformed)

    def test_wrong_owner_and_truncated_log_give_status_without_raw_text(self):
        with tempfile.TemporaryDirectory() as raw:
            workspace = Path(raw)
            log = service(workspace)
            baseline = evidence.before_call(workspace, cache_is_default=True)
            for data, status in ((line(record(service_pid=999)), "invalid_record"),
                                 (line()[:-1], "invalid_record"),
                                 (b"PRIVATE_SECRET" * evidence.READ_LIMIT, "truncated")):
                with self.subTest(status=status):
                    log.write_bytes(data)
                    captured = evidence.collect_failure(workspace, baseline, cache_is_default=True)
                    self.assertEqual(captured, evidence.FailureCapture(status))

    def test_replaced_log_and_unrelated_owner_are_refused(self):
        with tempfile.TemporaryDirectory() as raw:
            workspace = Path(raw)
            log = service(workspace)
            baseline = evidence.before_call(workspace, cache_is_default=True)
            log.rename(log.with_suffix(".old"))
            log.write_bytes(line())
            self.assertEqual(evidence.collect_failure(workspace, baseline, cache_is_default=True).status,
                             "ambiguous")
            (log.parent / "service.json").write_text(
                json.dumps({"pid": 101, "workspace_root": "PRIVATE_OTHER_WORKSPACE"}), encoding="utf-8")
            self.assertEqual(evidence.before_call(workspace, cache_is_default=True).status, "unlocated")

    def test_log_write_failure_keeps_the_original_assertion(self):
        class BrokenWriter:
            def write(self, _):
                raise OSError("PRIVATE_IO_ERROR")

        with tempfile.TemporaryDirectory() as raw:
            workspace = Path(raw)
            baseline = evidence.before_call(workspace, cache_is_default=False)
            with self.assertRaisesRegex(AssertionError, "original assertion"):
                try:
                    raise AssertionError("original assertion")
                finally:
                    evidence.emit_failure_evidence(workspace, baseline, cache_is_default=False,
                                                   case_ordinal=0, step_ordinal=0, writer=BrokenWriter())


if __name__ == "__main__":
    unittest.main()
