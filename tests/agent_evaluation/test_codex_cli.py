"""Совместимость общей команды приёмки с установленным Codex CLI."""
import json
import os
import subprocess
import unittest
import uuid

from tests.agent_evaluation.dcs_driver import REPO, audit_review, codex_command, disabled_skills, run_cli


class CodexCliCompatibilityTests(unittest.TestCase):
    def test_prepared_command_completes_without_configuration_errors(self):
        proof = REPO / ".session-temp/acceptance-agent" / ("CodexCLI-" + uuid.uuid4().hex)
        workspace = proof / "workspace"
        workspace.mkdir(parents=True)
        schema = proof / "output.schema.json"
        schema.write_text(json.dumps({
            "type": "object", "properties": {"ready": {"type": "boolean"}},
            "required": ["ready"], "additionalProperties": False,
        }))
        final = proof / "final.json"
        command = codex_command(workspace, schema, final, disabled_skills())
        command += ["--disable", "shell_tool"]
        prompt = 'Не используй инструменты. Верни JSON {"ready": true}.'
        (proof / "prompt.md").write_text(prompt)
        (proof / "manifest.json").write_text(json.dumps({
            "codexVersion": subprocess.check_output(["codex", "--version"], text=True).strip(),
            "command": command,
        }, indent=2))
        events, _, _ = run_cli(command, prompt, dict(os.environ), proof / "agent", timeout=120)
        audit_review(events)
        self.assertEqual(json.loads(final.read_text()), {"ready": True})
        print(f"accepted Codex CLI compatibility proof: {proof}", flush=True)
