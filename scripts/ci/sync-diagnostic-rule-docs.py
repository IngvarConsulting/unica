#!/usr/bin/env python3
"""Refresh/check analyzer rule documentation from the tool lock's exact commit."""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
LOCK = Path('plugins/unica/third-party/tools.lock.json')
SNAPSHOT = Path('crates/unica-documentation-sources/data/diagnostic-rules.json')
PREFIX = 'crates/ide-diagnostics/docs/'

def tool(root):
    data = json.loads((root / LOCK).read_text(encoding='utf-8'))
    return next(t for t in data['tools'] if t['name'] == 'bsl-analyzer')

def from_upstream(root, upstream):
    entry = tool(root)
    def git(*args):
        return subprocess.check_output(['git', '-C', str(upstream), *args])
    commit = entry['sourceCommit']
    paths = git('ls-tree', '-r', '--name-only', commit, '--', PREFIX).decode().splitlines()
    documents = []
    for path in paths:
        parts = path.split('/')
        if len(parts) != 5 or parts[3] not in ('ru', 'en') or not path.endswith('.md'):
            continue
        raw = git('show', f'{commit}:{path}')
        documents.append(dict(code=Path(path).stem, language=parts[3], path=path,
                              sha256=hashlib.sha256(raw).hexdigest(), text=raw.decode('utf-8')))
    if not documents:
        raise ValueError('the locked commit contains no rule documentation')
    return dict(toolLockRef=str(LOCK) + '#bsl-analyzer', repository=entry['repository'],
                sourceCommit=commit, version=entry['version'], license=entry['license'],
                documents=documents)

def validate(root, snapshot):
    entry = tool(root)
    for field in ('repository', 'sourceCommit', 'version', 'license'):
        if snapshot[field] != entry[field]:
            raise ValueError(f'{field} differs from tools.lock; refresh the snapshot')
    if snapshot['toolLockRef'] != str(LOCK) + '#bsl-analyzer':
        raise ValueError('snapshot must name its tool-lock owner')
    seen = set()
    for document in snapshot['documents']:
        code, language = document['code'], document['language']
        if not code.isascii() or not code.isalnum() or not code[0].isalpha() or language not in ('ru', 'en'):
            raise ValueError('invalid rule identity')
        if document['path'] != f'{PREFIX}{language}/{code}.md' or (language, code) in seen:
            raise ValueError('invalid/duplicate rule path')
        seen.add((language, code))
        if not document['text'].strip() or hashlib.sha256(document['text'].encode()).hexdigest() != document['sha256']:
            raise ValueError('rule content digest differs')
    if not seen:
        raise ValueError('empty rule snapshot')

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--upstream', type=Path, help='local analyzer Git repository; bytes read via git show')
    parser.add_argument('--write', action='store_true', help='replace snapshot using --upstream')
    args = parser.parse_args()
    if args.write and args.upstream is None:
        parser.error('--write requires --upstream')
    path = ROOT / SNAPSHOT
    if args.write:
        snapshot = from_upstream(ROOT, args.upstream)
        validate(ROOT, snapshot)
        path.write_text(json.dumps(snapshot, ensure_ascii=False, indent=2) + '\n', encoding='utf-8', newline='\n')
    else:
        snapshot = json.loads(path.read_text(encoding='utf-8'))
        validate(ROOT, snapshot)
        if args.upstream and snapshot != from_upstream(ROOT, args.upstream):
            raise ValueError('snapshot differs from the locked upstream bytes')
    print(f'diagnostic rule snapshot: {len(snapshot["documents"])} documents, locked {snapshot["version"]}')

if __name__ == '__main__':
    main()
