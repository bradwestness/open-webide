#!/usr/bin/env python3
"""Materialize the pinned core plugins from Git; never follow a moving branch."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
LOCK = ROOT / 'plugins/bundled.json'
OUTPUT = ROOT / 'bridge/bundled/plugins.json'
LICENSE = ROOT / 'bridge/bundled/LICENSE'


def git(repo, *args):
    return subprocess.check_output(['git', '-C', str(repo), *args])


def materialize(repo, lock):
    commit = lock['commit']
    assert git(repo, 'rev-parse', '--verify', commit + '^{commit}').decode().strip() == commit
    plugins = []
    for directory in lock['paths']:
        files = []
        for entry in git(repo, 'ls-tree', '-r', '-z', f'{commit}:{directory}').split(b'\0'):
            if not entry:
                continue
            metadata, name = entry.split(b'\t', 1)
            mode, kind, object_id = metadata.split()
            assert mode in (b'100644', b'100755') and kind == b'blob', 'Unsupported bundled file'
            files.append({'path': name.decode(), 'content': git(repo, 'cat-file', 'blob', object_id.decode()).decode(), 'executable': mode == b'100755'})
        plugins.append({'path': directory, 'files': files})
    return json.dumps({'repository': lock['repository'], 'commit': commit, 'plugins': plugins}, indent=2) + '\n', git(repo, 'show', f'{commit}:LICENSE').decode()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repository', type=Path, help='Existing upstream clone for offline regeneration')
    parser.add_argument('--check', action='store_true', help='Verify checked-in snapshots against pinned upstream Git objects')
    args = parser.parse_args()
    lock = json.loads(LOCK.read_text())
    with tempfile.TemporaryDirectory(prefix='openwebide-bundled-plugins-') as temp:
        repo = args.repository or Path(temp)
        if args.repository is None:
            git(repo, 'init', '--bare', '--quiet')
            git(repo, 'fetch', '--quiet', '--depth=1', '--no-tags', '--', lock['repository'], lock['commit'])
        snapshot, license_text = materialize(repo, lock)
        if args.check:
            assert OUTPUT.read_text() == snapshot, 'Bundled plugins differ; run tools/bundle_plugins.py'
            assert LICENSE.read_text() == license_text, 'Bundled plugin license differs'
        else:
            OUTPUT.parent.mkdir(parents=True, exist_ok=True)
            OUTPUT.write_text(snapshot)
            LICENSE.write_text(license_text)
    print('Pinned core plugin snapshots verified' if args.check else 'Pinned core plugin snapshots generated')


if __name__ == '__main__':
    main()
