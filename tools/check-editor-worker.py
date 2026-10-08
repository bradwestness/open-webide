#!/usr/bin/env python3
"""Exercise the actual production Rust/WASM worker in Chrome, without an account."""
import functools
import http.server
import json
import os
from pathlib import Path
import runpy
import tempfile
import threading

ROOT = Path(__file__).resolve().parent.parent
Browser = runpy.run_path(str(ROOT / 'tools/check-editor-recovery.py'))['Browser']


def check():
    assert os.environ.get('CHROMEDRIVER'), 'Set CHROMEDRIVER to a compatible driver'
    dist = ROOT / 'frontend/dist'
    assert (dist / 'editor-worker.js').is_file(), 'Build the frontend first'
    assert '"/editor-worker.js"' in (dist / 'service-worker.js').read_text(), 'Worker missing from PWA shell'
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(dist))
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix='openwebide-worker-') as directory:
            browser = Browser(directory)
            try:
                browser.call('POST', '/url', {'url': f'http://127.0.0.1:{server.server_port}/offline.html'})
                result = browser.call('POST', '/execute/async', {'script': r'''
                    const done = arguments[0];
                    const worker = new Worker('/editor-worker.js', {type: 'module'});
                    const waiting = new Map(), bases = new Map(), sources = new Map(), structures = new Map(); let ticket = 0;
                    let wake; const ready = new Promise(resolve => {wake = resolve;});
                    worker.onerror = e => { worker.terminate(); done({error: e.message || 'Module worker failed to load', eventType: e.type, filename: e.filename || null}); };
                    worker.onmessage = event => {
                        if (event.data === 'openwebide-editor-ready:6') {wake(); return;}
                        const reply = JSON.parse(event.data);
                        waiting.get(reply.ticket)?.(reply); waiting.delete(reply.ticket);
                    };
                    async function request(document, language, source, span = null) {
                        await ready;
                        return new Promise(resolve => {
                            const id = ++ticket;
                            waiting.set(id, reply => {
                                if (reply.analysis) {
                                    const published = reply.analysis.source;
                                    if (typeof published !== 'string') {
                                        const old = sources.get(document);
                                        // Protocol offsets are UTF-8 bytes, not JavaScript characters.
                                        const encoder = new TextEncoder(), decoder = new TextDecoder('utf-8', {fatal:true});
                                        const bytes = encoder.encode(old), inserted = encoder.encode(published.text);
                                        const combined = new Uint8Array(published.start + inserted.length + bytes.length - published.end);
                                        combined.set(bytes.subarray(0, published.start));
                                        combined.set(inserted, published.start);
                                        combined.set(bytes.subarray(published.end), published.start + inserted.length);
                                        reply.analysis.sourceDelta = true;
                                        reply.analysis.source = decoder.decode(combined);
                                    }
                                    if (reply.analysis.structure) {
                                        const publication = reply.analysis.structure;
                                        reply.analysis.structureBytes = JSON.stringify(publication).length;
                                        if (publication.changes) {
                                            const old = structures.get(document), changes = publication.changes;
                                            if (!old || old.language !== changes.language) throw new Error('invalid structural base');
                                            const restored = {language: changes.language};
                                            for (const key of ['scopes','selections','opaque_starts','protected','brackets']) {
                                                const span = changes[key];
                                                restored[key] = Array.isArray(span) ? span :
                                                    [...old[key].slice(0,span.start), ...span.items, ...old[key].slice(span.end)];
                                            }
                                            reply.analysis.structure = restored;
                                            reply.analysis.structureDelta = true;
                                        }
                                        structures.set(document, reply.analysis.structure);
                                    } else { structures.delete(document); }
                                    sources.set(document, reply.analysis.source);
                                    bases.set(document, reply.ticket);
                                } else { bases.delete(document); sources.delete(document); structures.delete(document); }
                                resolve(reply);
                            });
                            worker.postMessage(JSON.stringify({version:6,ticket:id,document,language,source:span || source,tab_width:4,base_ticket:bases.get(document)}));
                        });
                    }
                    (async () => {
                        const source = 'fn main() {\r\n call("文😀");\r\n}\r\n';
                        const first = await request('rust', 'Rust', source);
                        const next = await request('rust', 'Rust', source.replace('文😀','😀 changed'));
                        const fixtures = {
                            TypeScript: 'function main(): number {\n return 1;\n}',
                            Tsx: 'function Main() {\n return <div>文😀</div>;\n}',
                            JavaScript: 'function main() {\n return `文 ${call()}`;\n}',
                            Jsx: 'function Main() {\n return <div>文😀</div>;\n}',
                            Python: 'def main():\n    return "文😀"\n',
                            Java: 'class Main {\n void call() { }\n}',
                            CSharp: 'class Main {\n void Call() { }\n}',
                            Cpp: 'int main() {\n return 1;\n}',
                            Php: '<?php function main() {\n return "文😀";\n}',
                            Shell: 'main() {\n echo "文😀"\n}',
                            C: 'int main() {\n return 1;\n}',
                            Go: 'package main\nfunc main() {\n println("文😀")\n}',
                            Html: '<script>function call() {\n return "文😀";\n}</script><style>a {color:red}</style>',
                            Css: 'a {\n color: red;\n}'
                        };
                        const providers = [];
                        for (const [language, text] of Object.entries(fixtures)) {
                            const reply = await request(language, language, text);
                            providers.push({language, status: reply.status, source: reply.analysis?.source,
                                structure: !!reply.analysis?.structure, paint: !!reply.analysis?.highlights,
                                sourceMatches: reply.analysis?.source === text});
                        }
                        const configs = [];
                        for (const [language, text] of Object.entries({
                            Json: '{\r\n "name": "文😀", "value": 42\r\n}',
                            Toml: '[section]\r\nname = "文😀"\r\n',
                            Yaml: 'section:\r\n  name: 文😀\r\n',
                            Sql: '/* first\r\nstill comment */\r\nSELECT \'文😀\';',
                            Markdown: '# Header\r\n' + Array.from({length: 1000}, (_, index) => `Paragraph ${index}: **文😀** and \`code\`.\r\n\r\n`).join(''),
                            Ini: '[section]\r\nname = 文😀\r\n',
                            Xml: '<section>\r\n<name>文😀</name>\r\n</section>'
                        })) {
                            const reply = await request(language, language, text);
                            const revised = text.replace('文😀', '😀 changed');
                            const update = await request(language, language, revised);
                            configs.push({language, status: reply.status, structure: !!reply.analysis?.structure,
                                plain: reply.analysis?.highlights?.every(row => Array.isArray(row) && row.every(token => token[1] === 'Plain')),
                                paint: !!reply.analysis?.highlights, sourceMatches: reply.analysis?.source === text,
                                updateStatus: update.status, updatePaint: !!update.analysis?.highlights,
                                updateSourceMatches: update.analysis?.source === revised,
                                reusedRows: update.analysis?.highlights?.reduce((total, row) => total + (!Array.isArray(row) && Number.isInteger(row.reuse) ? row.count : 0), 0)});
                        }
                        const heavy = 'fn call() {\n if true { println!("文😀"); }\n}\n'.repeat(1000);
                        let uiEvent = false;
                        setTimeout(() => {uiEvent = true;}, 0);
                        const prepared = await request('large', 'Rust', heavy);
                        const byteStart = new TextEncoder().encode(heavy.slice(0, heavy.indexOf('文😀'))).length;
                        const heavyUpdate = await request('large', 'Rust', heavy.replace('文😀', '🦀 changed'),
                            {start:byteStart,end:byteStart+new TextEncoder().encode('文😀').length,text:'🦀 changed'});
                        const structuralText = heavy.replace('文😀', '🦀 changed').replace('fn call()', 'fn test()');
                        const structuralUpdate = await request('large', 'Rust', structuralText);
                        const structuralDelta = structuralUpdate.analysis?.structureDelta === true &&
                            JSON.stringify(structuralUpdate.analysis.structure) === JSON.stringify(heavyUpdate.analysis.structure) &&
                            structuralUpdate.analysis.structureBytes * 10 < JSON.stringify(heavyUpdate.analysis.structure).length;
                        // TypeScript's acknowledged base was evicted by the subsequent documents.
                        const resyncText = fixtures.TypeScript + '\n';
                        const resync = await request('TypeScript', 'TypeScript', resyncText,
                            {start:new TextEncoder().encode(fixtures.TypeScript).length,
                             end:new TextEncoder().encode(fixtures.TypeScript).length,text:'\n'});
                        const resynced = await request('TypeScript', 'TypeScript', resyncText);
                        const nested = [];
                        for (const [language, open, member, close] of [
                            ['Rust', 'impl Example {\r\n', 'fn fINDEX() {\r\n call("文😀");\r\n}\r\n', '}\r\n'],
                            ['Java', 'class Example {\r\n', 'void fINDEX() {\r\n call("文😀");\r\n}\r\n', '}\r\n'],
                            ['CSharp', 'class Example {\r\n', 'void F_INDEX() {\r\n Call("文😀");\r\n}\r\n', '}\r\n'],
                            ['JavaScript', 'class Example {\r\n', 'fINDEX() {\r\n call(`文😀 ${inner("value")}`);\r\n}\r\n', '}\r\n'],
                            ['Python', 'class Example:\r\n', '    def fINDEX(self):\r\n        call("文😀")\r\n', ''],
                            ['Html', '<div>\r\n', '<section id="INDEX">\r\n<p>文😀</p>\r\n</section>\r\n', '</div>\r\n']
                        ]) {
                            const text = open + Array.from({length:200}, (_, i) => member.replace('INDEX', i)).join('') + close;
                            const seed = await request('nested-' + language, language, text);
                            const changed = text.replace('文😀', '😀 changed文');
                            const warm = await request('nested-' + language, language, changed);
                            const cold = await request('fresh-' + language, language, changed);
                            const colors = warm.analysis?.highlights?.flatMap(row => Array.isArray(row) ? [row] :
                                seed.analysis.highlights.slice(row.reuse, row.reuse + row.count));
                            nested.push({language, incremental: warm.status?.Ready?.incremental === true,
                                fresh: cold.status?.Ready?.incremental === false,
                                source: warm.analysis?.source === changed && cold.analysis?.source === changed,
                                folds: JSON.stringify(warm.analysis?.folds) === JSON.stringify(cold.analysis?.folds),
                                colors: !!colors && JSON.stringify(colors) === JSON.stringify(cold.analysis?.highlights),
                                structure: !!warm.analysis?.structure && ['language','scopes','selections','opaque_starts','protected','brackets'].every(key =>
                                    JSON.stringify(warm.analysis.structure[key]) === JSON.stringify(cold.analysis?.structure?.[key]))});
                        }
                        const oversized = await request('large', 'Rust', 'x'.repeat(2*1024*1024+1));
                        worker.terminate();
                        done({first, next, providers, configs, nested, structuralDelta, heavyStatus: prepared.status,
                            heavySource: prepared.analysis?.source === heavy, uiEvent,
                            heavyDelta: heavyUpdate.analysis?.sourceDelta === true,
                            heavyRowRuns: heavyUpdate.analysis?.highlights?.length,
                            heavyUpdateSource: heavyUpdate.analysis?.source === heavy.replace('文😀', '🦀 changed'),
                            resyncStatus: resync.status, resyncedSource: resynced.analysis?.source === resyncText,
                            oversizedStatus: oversized.status, oversizedAnalysis: oversized.analysis});
                    })().catch(error => {worker.terminate(); done({error:String(error)});});
                ''', 'args': []})
                assert 'error' not in result, result
                assert result['heavyDelta'] and result['heavyUpdateSource'], result
                assert result['structuralDelta'], 'Structural update did not preserve metadata with a compact patch'
                assert result['heavyRowRuns'] == 3, result['heavyRowRuns']
                assert result['resyncStatus'] == 'NeedsSource' and result['resyncedSource'], result
                assert result['first']['status'] == {'Ready': {'incremental': False}}, result['first']['status']
                assert result['next']['status'] == {'Ready': {'incremental': True}}, result['next']['status']
                assert result['first']['analysis']['source'] != result['next']['analysis']['source']
                assert result['first']['analysis']['folds']
                for provider in result['providers']:
                    assert 'Ready' in provider['status'] and provider['structure'] and provider['paint'] and provider['sourceMatches'], provider
                for config in result['configs']:
                    assert 'Ready' in config['status'] and config['paint'] and config['sourceMatches'], config
                    assert config['structure'] == (config['language'] != 'Sql'), config
                    if config['language'] == 'Sql':
                        assert config['plain'], config
                    assert 'Ready' in config['updateStatus'] and config['updatePaint'] and config['updateSourceMatches'] and config['reusedRows'] > 0, config
                assert 'Ready' in result['heavyStatus'] and result['heavySource'] and result['uiEvent'], result
                for nested in result['nested']:
                    assert all(nested[key] for key in ['incremental', 'fresh', 'source', 'folds', 'structure', 'colors']), nested
                assert result['oversizedStatus'] == 'TooLarge' and result['oversizedAnalysis'] is None, result
                print(json.dumps({'providers': len(result['providers'])+1, 'config_languages': len(result['configs']) - 1, 'plain_fallback_languages': 1, 'nested_languages': len(result['nested']), 'incremental': True, 'structural_delta': result['structuralDelta'],
                                  'ui_event_during_worker': result['uiEvent'], 'oversize_fallback': True, 'changed_row_records': result['heavyRowRuns']}))
            finally:
                browser.stop()
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


if __name__ == '__main__':
    check()
