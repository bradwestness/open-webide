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
                    const waiting = new Map(), bases = new Map(); let ticket = 0;
                    let wake; const ready = new Promise(resolve => {wake = resolve;});
                    worker.onerror = e => { worker.terminate(); done({error: e.message}); };
                    worker.onmessage = event => {
                        if (event.data === 'openwebide-editor-ready:2') {wake(); return;}
                        const reply = JSON.parse(event.data);
                        waiting.get(reply.ticket)?.(reply); waiting.delete(reply.ticket);
                    };
                    async function request(document, language, source) {
                        await ready;
                        return new Promise(resolve => {
                            const id = ++ticket;
                            waiting.set(id, reply => {
                                if (reply.analysis) bases.set(document, reply.ticket);
                                else bases.delete(document);
                                resolve(reply);
                            });
                            worker.postMessage(JSON.stringify({version:2,ticket:id,document,language,source,tab_width:4,base_ticket:bases.get(document)}));
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
                        const lexical = [];
                        for (const [language, text] of Object.entries({
                            Json: '{\r\n "name": "文😀", "value": 42\r\n}',
                            Toml: '[section]\r\nname = "文😀"\r\n',
                            Yaml: 'section:\r\n  name: 文😀\r\n',
                            Sql: '/* first\r\nstill comment */\r\nSELECT \'文😀\';',
                            Markdown: '# Header\r\nText 文😀\r\n'
                        })) {
                            const reply = await request(language, language, text);
                            const revised = text.replace('文😀', '😀 changed');
                            const update = await request(language, language, revised);
                            lexical.push({language, status: reply.status, structure: reply.analysis?.structure,
                                paint: !!reply.analysis?.highlights, sourceMatches: reply.analysis?.source === text,
                                updateStatus: update.status, updatePaint: !!update.analysis?.highlights,
                                updateSourceMatches: update.analysis?.source === revised,
                                reusedRows: update.analysis?.highlights?.filter(row => !Array.isArray(row) && Number.isInteger(row.reuse)).length});
                        }
                        const heavy = 'fn call() {\n if true { println!("文😀"); }\n}\n'.repeat(1000);
                        let uiEvent = false;
                        setTimeout(() => {uiEvent = true;}, 0);
                        const prepared = await request('large', 'Rust', heavy);
                        const oversized = await request('large', 'Rust', 'x'.repeat(2*1024*1024+1));
                        worker.terminate();
                        done({first, next, providers, lexical, heavyStatus: prepared.status,
                            heavySource: prepared.analysis?.source === heavy, uiEvent,
                            oversizedStatus: oversized.status, oversizedAnalysis: oversized.analysis});
                    })().catch(error => {worker.terminate(); done({error:String(error)});});
                ''', 'args': []})
                assert 'error' not in result, result
                assert result['first']['status'] == {'Ready': {'incremental': False}}, result['first']['status']
                assert result['next']['status'] == {'Ready': {'incremental': True}}, result['next']['status']
                assert result['first']['analysis']['source'] != result['next']['analysis']['source']
                assert result['first']['analysis']['folds']
                for provider in result['providers']:
                    assert 'Ready' in provider['status'] and provider['structure'] and provider['paint'] and provider['sourceMatches'], provider
                for lexical in result['lexical']:
                    assert 'Ready' in lexical['status'] and lexical['structure'] is None and lexical['paint'] and lexical['sourceMatches'], lexical
                    assert 'Ready' in lexical['updateStatus'] and lexical['updatePaint'] and lexical['updateSourceMatches'] and lexical['reusedRows'] > 0, lexical
                assert 'Ready' in result['heavyStatus'] and result['heavySource'] and result['uiEvent'], result
                assert result['oversizedStatus'] == 'TooLarge' and result['oversizedAnalysis'] is None, result
                print(json.dumps({'providers': len(result['providers'])+1, 'lexical_languages': len(result['lexical']), 'incremental': True,
                                  'ui_event_during_worker': result['uiEvent'], 'oversize_fallback': True}))
            finally:
                browser.stop()
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


if __name__ == '__main__':
    check()
