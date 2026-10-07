#!/usr/bin/env python3
"""Verify trusted Chrome clicks and insertion in syntax-painted editor drafts.

Uses disposable accounts/state. Local recovery does not grant a folder handle.
This checks browser-engine mouse events, not physical touch handles or PWA input.
"""
import base64
import json
from pathlib import Path
import runpy
import secrets
import tempfile

ROOT = Path(__file__).resolve().parent.parent
SUPPORT = runpy.run_path(str(ROOT / 'tools/check-editor-input.py'))
Browser, Runtime, wait, cdp = (SUPPORT[key] for key in ['Browser', 'Runtime', 'wait', 'cdp'])


def check(mode, repeat):
    row = 'fn main() {\r\n    let message = "文😀";\r\n    println!("hello");\r\n}\r\n'
    source = row * repeat
    with tempfile.TemporaryDirectory(prefix='openwebide-pointer-') as state, \
            tempfile.TemporaryDirectory(prefix='editor-pointer-probe-', dir=ROOT) as folder:
        runtime, browser = Runtime(state, frontend=True), None
        try:
            runtime.start()
            runtime.request('POST', '/api/auth/register', {
                'username': 'pointer-' + secrets.token_hex(8), 'password': secrets.token_urlsafe(32),
            }, expected=201)
            path = Path(folder).relative_to(ROOT.parent.parent).as_posix() if mode == 'remote' else 'pointer-folder'
            project = runtime.request('POST', '/api/projects', {'name': 'Pointer verification', 'mode': mode, 'path': path}, expected=201)
            (Path(folder) / 'pointer.rs').write_bytes(source.encode())
            encoded = base64.b64encode(source.encode()).decode()
            endpoint = f"/api/projects/{project['id']}/editor-recovery"
            runtime.request('PUT', endpoint, {'revision': 0, 'state': {'format': 1,
                'root': {'mode': mode, 'path': path if mode == 'remote' else None}, 'selected': 'pointer.rs',
                'files': [{'path': 'pointer.rs', 'document': {'text': encoded, 'saved': encoded,
                    'selections': [{'anchor': 0, 'head': 0}], 'collapsed': []},
                    'scroll': {'top': 0, 'left': 0}, 'read_only': False}]}})
            for key, value in {'open_tabs': json.dumps([project['id']]), 'active_project': str(project['id']),
                    'bridge_url': 'http://127.0.0.1:1', 'editor_preferences': json.dumps({'word_wrap': False})}.items():
                runtime.request('PUT', '/api/settings', {'key': key, 'value': value})
            browser = Browser(state)
            browser.call('POST', '/url', {'url': runtime.url + '/offline.html'})
            for cookie in runtime.cookies:
                browser.call('POST', '/cookie', {'cookie': {'name': cookie.name, 'value': cookie.value,
                    'domain': '127.0.0.1', 'path': cookie.path, 'httpOnly': True}})
            browser.call('POST', '/url', {'url': runtime.url + '/'})
            wait('syntax paint', lambda: browser.script("return !!document.querySelector('.highlight-ready .tok-keyword');"))
            for line in ([2] if repeat == 1 else [2, 2002]):
                if line > 2:
                    browser.script("const s=document.querySelector('.editor-scroll-surface');s.scrollTop=2000*parseFloat(getComputedStyle(document.querySelector('textarea[data-editor-path]')).lineHeight);")
                wait('painted target row', lambda: browser.script(f"return !!document.querySelector('.editor-source-line[data-line=\"{line}\"]');"))
                for column, beyond in [(8, False), (16, False), (24, True)]:
                    browser.script("document.querySelector('textarea[data-editor-path]').blur();")
                    point = browser.script(f"""
                        const input=document.querySelector('textarea[data-editor-path]');
                        const row=document.querySelector('.editor-source-line[data-line="{line}"]');
                        const walker=document.createTreeWalker(row,NodeFilter.SHOW_TEXT);
                        let node,at={column}; while((node=walker.nextNode())) {{if(at<=node.length) break;at-=node.length;}}
                        const range=document.createRange();range.setStart(node,at);range.collapse(true);
                        const rect=range.getBoundingClientRect();
                        window.pointerEvents=[];
                        for(const type of ['mousedown','mouseup','click']) input.addEventListener(type,e=>pointerEvents.push({{type,trusted:e.isTrusted}}),{{once:true}});
                        return {{x:Math.min(input.getBoundingClientRect().right-2,rect.left+{30 if beyond else 0.25}),y:rect.top+rect.height/2,bound:input.dataset.editorNativeBound==='true',bounds:input.getBoundingClientRect().toJSON()}};
                    """)
                    for kind in ['mouseMoved', 'mousePressed', 'mouseReleased']:
                        args = {'type': kind, 'x': point['x'], 'y': point['y']}
                        if kind != 'mouseMoved': args.update(button='left', clickCount=1)
                        cdp(browser, 'Input.dispatchMouseEvent', args)
                    start = len(''.join(source.splitlines(keepends=True)[:line-1]).encode())
                    expected = start + (28 if beyond else column)
                    def selection_matches():
                        saved = runtime.request('GET', endpoint)['state']['files'][0]['document']
                        return saved['selections'][0] == {'anchor': expected, 'head': expected}
                    try:
                        wait('trusted highlighted click position', selection_matches, seconds=5)
                    except AssertionError:
                        saved = runtime.request('GET', endpoint)['state']['files'][0]['document']
                        raise AssertionError({'mode': mode, 'repeat': repeat, 'line': line, 'column': column,
                            'expected': expected, 'point': point, 'events': browser.script('return pointerEvents;'), 'selection': saved['selections'], 'native': browser.script("const i=document.querySelector('textarea[data-editor-path]');return {start:i.selectionStart,end:i.selectionEnd,bound:i.dataset.editorNativeBound};")}) from None
                    events = browser.script('return pointerEvents;')
                    assert len(events) == 3 and all(event['trusted'] for event in events), events
                    # Prove the next actual insertion uses that source position,
                    # rather than only checking a painted/native caret marker.
                    cdp(browser, 'Input.insertText', {'text': 'X'})
                    wanted = source.encode()[:expected] + b'X' + source.encode()[expected:]
                    wait('insertion at clicked source position', lambda:
                        base64.b64decode(runtime.request('GET', endpoint)['state']['files'][0]['document']['text']) == wanted)
                    SUPPORT['history_key'](browser)
                    wait('undo preserves unrelated source', lambda:
                        runtime.request('GET', endpoint)['state']['files'][0]['document']['text'] == encoded)
            print(json.dumps({'mode': mode, 'repeat': repeat, 'trusted_clicks': True}))
        finally:
            if browser: browser.stop()
            runtime.stop()


if __name__ == '__main__':
    for mode in ['local', 'remote']:
        for repeat in [1, 1000]:
            check(mode, repeat)
