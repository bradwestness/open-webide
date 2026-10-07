#!/usr/bin/env python3
"""Compare a Rust/WASM layout candidate with Chrome using the same supplied TTF.

This is an isolated geometry experiment, not an application renderer or input
benchmark. Dependencies stay outside the production workspace. Generated source
is temporary, and all compilation uses the repository's shared target directory.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parent.parent


def run(args, env, output, phase):
    result = subprocess.run(args, cwd=ROOT, env=env, text=True, capture_output=True)
    cases = 0
    for line in result.stdout.splitlines():
        if line.startswith('{'):
            record = json.loads(line)
            record['phase'] = phase
            output.write(json.dumps(record) + '\n')
            cases += 'case' in record
    output.flush()
    if result.returncode:
        raise RuntimeError(result.stdout + result.stderr)
    expected = {'native': 10, 'browser': 20}.get(phase)
    if expected is not None and cases != expected:
        raise RuntimeError(f'{phase} produced {cases} cases; expected {expected}\n{result.stdout}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--font', type=Path, required=True, help='Variable TTF; the recorded sample uses Monaspace Neon v1.400')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lint', action='store_true', help='Check both targets using the repository lint policy')
    options = parser.parse_args()
    font = options.font.resolve(strict=True)
    versions = {p['name']: p['version'] for p in tomllib.loads((ROOT / 'Cargo.lock').read_text())['package']}
    with tempfile.TemporaryDirectory(prefix='openwebide-editor-layout-') as folder:
        experiment = Path(folder)
        shutil.copytree(ROOT / 'tools/editor-layout', experiment, dirs_exist_ok=True)
        manifest = experiment / 'Cargo.toml'
        with manifest.open('a') as target:
            policy = (ROOT / 'Cargo.toml').read_text()
            for section in re.findall(r'(?ms)^\[workspace\.lints\.[^\]]+\].*?(?=^\[|\Z)', policy):
                target.write('\n' + section)
            target.write("\n[target.'cfg(target_arch = \"wasm32\")'.dev-dependencies]\n")
            for name in ['wasm-bindgen', 'wasm-bindgen-test', 'wasm-bindgen-futures', 'js-sys']:
                target.write(f'{name} = "={versions[name]}"\n')
        env = dict(os.environ, CARGO_TARGET_DIR=str(ROOT / 'target'), OPENWEBIDE_LAYOUT_FONT=str(font))
        with options.output.open('w') as output:
            output.write(json.dumps({'host': platform.platform(), 'font_sha256': hashlib.sha256(font.read_bytes()).hexdigest(),
                                     'font_bytes': font.stat().st_size, 'parley': '0.11.1', 'measurement': 'isolated layout'}) + '\n')
            base = ['--manifest-path', str(manifest), '--locked']
            if options.lint:
                for platform_args in [[], ['--target', 'wasm32-unknown-unknown']]:
                    run(['cargo', 'clippy', *base, *platform_args, '--all-targets', '--', '-D', 'warnings'], env, output, 'lint')
            run(['cargo', 'run', *base, '--release'], env, output, 'native')
            run(['cargo', 'test', *base, '--release', '--target', 'wasm32-unknown-unknown', '--test', 'browser', '--', '--nocapture'], env, output, 'browser')


if __name__ == '__main__':
    main()
