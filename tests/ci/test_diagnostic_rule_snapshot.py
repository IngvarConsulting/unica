"""Offline rule docs retain their tool-lock provenance and immutable source bytes."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('rule_snapshot', ROOT / 'scripts/ci/sync-diagnostic-rule-docs.py')
snapshot = importlib.util.module_from_spec(spec)
spec.loader.exec_module(snapshot)

class DiagnosticRuleSnapshotTests(unittest.TestCase):
    def test_bundled_snapshot_matches_lock_and_all_content_digests(self):
        data = json.loads((ROOT / snapshot.SNAPSHOT).read_text(encoding='utf-8'))
        snapshot.validate(ROOT, data)
        for code in ('UnresolvedMethodCall', 'AmbiguousFieldInQuery', 'UnresolvedField'):
            for language in ('ru', 'en'):
                self.assertTrue(any(d['code'] == code and d['language'] == language for d in data['documents']))
        changed = {**data, 'version': 'unaccepted'}
        with self.assertRaisesRegex(ValueError, 'differs from tools.lock'):
            snapshot.validate(ROOT, changed)
        changed = {**data, 'documents': [{**data['documents'][0], 'text': 'replaced'}]}
        with self.assertRaisesRegex(ValueError, 'digest differs'):
            snapshot.validate(ROOT, changed)

    def test_refresh_reads_locked_git_objects_instead_of_modified_worktree(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            repo = root / 'upstream'
            repo.mkdir()
            def git(*args):
                return subprocess.check_output(['git', '-C', str(repo), *args], stderr=subprocess.DEVNULL).decode().strip()
            git('init')
            document = repo / snapshot.PREFIX / 'ru' / 'SyntheticRule.md'
            document.parent.mkdir(parents=True)
            document.write_text('# Synthetic rule\n\nOriginal description.\n', encoding='utf-8')
            git('add', '.')
            git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-m', 'fixture')
            commit = git('rev-parse', 'HEAD')
            lock = root / snapshot.LOCK
            lock.parent.mkdir(parents=True)
            lock.write_text(json.dumps({'tools': [{'name': 'bsl-analyzer', 'repository': 'https://example.invalid/analyzer', 'sourceCommit': commit, 'version': 'fixture', 'license': 'test'}]}), encoding='utf-8')
            document.write_text('Uncommitted replacement must never be accepted.', encoding='utf-8')
            data = snapshot.from_upstream(root, repo)
            snapshot.validate(root, data)
            self.assertEqual(data['documents'][0]['text'], '# Synthetic rule\n\nOriginal description.\n')
            self.assertEqual(data['sourceCommit'], commit)
            self.assertEqual(len(data['documents']), 1)

if __name__ == '__main__':
    unittest.main()
