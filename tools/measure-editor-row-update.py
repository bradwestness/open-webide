#!/usr/bin/env python3
"""Compare retained text-node edits with whole-row DOM replacement.

Uses the built app's CSS and fonts in disposable Chrome/Spin state. This isolates
layout mutation; it does not measure application input or complete cold startup.
No Rust build or additional target directory is created.
"""
import argparse
import json
import os
from pathlib import Path
import platform
import runpy
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
SUPPORT = runpy.run_path(str(ROOT / 'tools/check-editor-input.py'))
Browser, Runtime, wait = (SUPPORT[key] for key in ['Browser', 'Runtime', 'wait'])

MEASURE = r"""
const [operation, repetition, wrapped, bytes, unicode] = arguments;
const unit = unicode ? '文😀 words ' : 'alpha beta gamma ';
const source = unit.repeat(Math.floor(bytes / new TextEncoder().encode(unit).length));
let at = source.indexOf(' ', Math.floor(source.length / 2));
if (at < 0) throw new Error('Missing safe edit position');
const next = source.slice(0, at) + 'X' + source.slice(at);
const root = document.createElement('div');
root.className = 'editor-code' + (wrapped ? ' editor-word-wrap' : '');
root.style.cssText = 'position:fixed;left:-10000px;top:0;width:300px;height:auto;visibility:hidden;contain:layout style paint;--editor-gutter-width:40px';
const probe = document.createElement('div');
probe.className = 'editor-highlight editor-row-measure';
probe.style.cssText = 'position:relative;inset:auto;height:auto;width:300px;visibility:hidden;white-space:' + (wrapped ? 'pre-wrap' : 'pre') + ';overflow-wrap:' + (wrapped ? 'anywhere' : 'normal');
const paint = document.createElement('div');
paint.className = 'editor-highlight-content';
paint.style.cssText = 'transform:none;will-change:auto;min-height:0;min-width:0;width:300px;padding-left:40px;padding-right:16px';
const row = document.createElement('span');
row.className = 'editor-source-line'; row.dataset.line = '1';
const production = operation.startsWith('production-');
const update = production ? operation.slice('production-'.length) : operation;
function install(text) {
    row.replaceChildren();
    if (!production) {row.appendChild(document.createTextNode(text)); return;}
    const encoder = new TextEncoder(), segmenter = new Intl.Segmenter('und', {granularity:'grapheme'});
    let start = 0, bytes = 0, firstBytes = 0;
    function append(end) {
        const span = document.createElement('span'); span.className = 'editor-text-run';
        span.appendChild(document.createTextNode(text.slice(start,end))); row.appendChild(span); start = end;
    }
    for (const glyph of segmenter.segment(text)) {
        if (bytes - firstBytes >= 512) {append(glyph.index); firstBytes = bytes;}
        bytes += encoder.encode(glyph.segment).length;
    }
    if (start < text.length) append(text.length);
}
function point(offset) {
    const walker = document.createTreeWalker(row, NodeFilter.SHOW_TEXT);
    let node, previous;
    while ((node = walker.nextNode())) {
        if (offset < node.length) return [node,offset];
        offset -= node.length; previous = node;
    }
    if (offset !== 0 || !previous) throw new Error('Invalid native source offset');
    return [previous,previous.length];
}
install(source);
if (update === 'split-replace-data') {
    let node = row.firstChild;
    while (node.length > 4096) {
        const boundary = node.data.indexOf(' ', 4096);
        if (boundary < 0) break;
        node = node.splitText(boundary + 1);
    }
}
paint.appendChild(row); probe.appendChild(paint); root.appendChild(probe); document.body.appendChild(root);
function geometry() {
    const box = row.getBoundingClientRect();
    const caret = document.createRange();
    const [node,offset] = point(at + 1);
    caret.setStart(node, offset); caret.collapse(true);
    const rect = caret.getBoundingClientRect();
    return {width: row.scrollWidth, height: box.height, caretX: rect.left - box.left, caretY: rect.top - box.top};
}
try {
    const cold = performance.now(); geometry(); const coldLayoutMs = performance.now() - cold;
    const started = performance.now();
    if (update === 'split-replace-data' || update === 'replace-data') {
        const [node,offset] = point(at); node.replaceData(offset, 0, 'X');
    }
    else if (update === 'set-data') {
        if (production) {const [node,offset] = point(at); node.data = node.data.slice(0,offset) + 'X' + node.data.slice(offset);}
        else row.firstChild.data = next;
    }
    else if (update === 'replace-node') install(next);
    else throw new Error('Unknown row update operation');
    const mutated = performance.now(); const result = geometry(); const finished = performance.now();
    if (row.textContent !== next) throw new Error('Row mutation lost source');
    return {operation, repetition, wrapped, unicode, sourceBytes: new TextEncoder().encode(source).length,
        mutationMs: mutated - started, layoutMs: finished - mutated, totalMs: finished - started,
        coldLayoutMs, ...result, font: getComputedStyle(row).fontFamily, features: getComputedStyle(row).fontFeatureSettings};
} finally { root.remove(); }
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bytes', type=int, default=1024 * 1024)
    parser.add_argument('--repeat', type=int, default=3)
    parser.add_argument('--timeout', type=int, default=60)
    parser.add_argument('--wrap', choices=['off', 'on'], nargs='+', default=['off', 'on'])
    parser.add_argument('--source', choices=['ascii', 'unicode'], nargs='+', default=['ascii', 'unicode'])
    parser.add_argument('--operations', choices=['replace-data', 'set-data', 'replace-node', 'split-replace-data', 'production-replace-data', 'production-set-data', 'production-replace-node'], nargs='+', default=['production-replace-data', 'production-set-data', 'production-replace-node'])
    options = parser.parse_args()
    if not 1 <= options.bytes <= 1024 * 1024 or not 1 <= options.repeat <= 10 or not 1 <= options.timeout <= 180:
        parser.error('Use 1..1 MiB of source, 1..10 repetitions and 1..180 seconds timeout')
    css = sorted((ROOT / 'frontend/dist').glob('styles-*.css'))
    if len(css) != 1:
        raise RuntimeError('Build the current frontend first; expected one styles bundle')
    with tempfile.TemporaryDirectory(prefix='openwebide-row-layout-') as state:
        runtime, browser = Runtime(state, frontend=True), None
        try:
            runtime.start()
            reference = {}
            header = False
            for source in options.source:
                for wrapped in options.wrap:
                    for repetition in range(1, options.repeat + 1):
                        for operation in options.operations:
                            case_state = Path(state) / f'{source}-{wrapped}-{repetition}-{operation}'
                            case_state.mkdir()
                            browser = Browser(case_state)
                            try:
                                browser.call('POST', '/url', {'url': runtime.url + '/offline.html'})
                                browser.script('''
                                    window.rowLayoutReady = false;
                                    const sheet = document.createElement('link'); sheet.rel = 'stylesheet';
                                    sheet.onload = () => document.fonts.load('13px "Monaspace Neon"').then(() => {window.rowLayoutReady = true;});
                                    sheet.href = ''' + json.dumps('/' + css[0].name) + '; document.head.appendChild(sheet);')
                                wait('production font and stylesheet', lambda: browser.script('return window.rowLayoutReady;'))
                                if not header:
                                    print(json.dumps({'host': platform.platform(), 'measurement': 'isolated retained row DOM mutation',
                                        'checkoutHead': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                                        'browser': browser.script('return navigator.userAgent;'), 'stylesheet': css[0].name,
                                        'image': os.environ.get('EDITOR_VIEW_IMAGE'), 'freshBrowserPerCase': True,
                                        'commandTimeoutSeconds': options.timeout}), flush=True)
                                    header = True
                                try:
                                    record = browser.call('POST', '/execute/sync', {'script': MEASURE,
                                        'args': [operation, repetition, wrapped == 'on', options.bytes, source == 'unicode']}, timeout=options.timeout)
                                except TimeoutError:
                                    print(json.dumps({'status': 'timeout', 'operation': operation, 'repetition': repetition,
                                        'wrapped': wrapped == 'on', 'unicode': source == 'unicode', 'sourceBytesNominal': options.bytes,
                                        'commandTimeoutSeconds': options.timeout}), flush=True)
                                    raise

                                key = (source, wrapped)
                                values = [record[name] for name in ['width', 'height', 'caretX', 'caretY']]
                                if key in reference and any(abs(a - b) > 0.25 for a, b in zip(reference[key], values)):
                                    raise AssertionError('Mutation methods produced different source geometry')
                                reference[key] = values
                                print(json.dumps(record), flush=True)
                            finally:
                                browser.stop()
                                browser = None
        finally:
            if browser is not None:
                browser.stop()
            runtime.stop()


if __name__ == '__main__':
    main()
