#!/usr/bin/env python3
"""Verify trusted Chrome clicks and insertion in syntax-painted editor drafts.

Uses disposable accounts/state. Local recovery does not grant a folder handle.
This checks browser-engine mouse events, not physical touch handles or PWA input.
"""
import argparse
import base64
import json
from pathlib import Path
import runpy
import secrets
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
SUPPORT = runpy.run_path(str(ROOT / 'tools/check-editor-input.py'))
Browser, Runtime, wait, cdp = (SUPPORT[key] for key in ['Browser', 'Runtime', 'wait', 'cdp'])


FIXTURES = {
    'rs': 'fn main() {\r\n    let message = "文😀";\r\n    println!("hello");\r\n}\r\n',
    'cs': 'class Main {\r\n    string message = "文😀";\r\n    void Run() { }\r\n}\r\n',
    'json': '{\r\n    "message": "文😀",\r\n    "enabled": true\r\n}\r\n',
}


def check(mode, repeat, language):
    row = FIXTURES[language]
    name = 'pointer.' + language
    body = row.splitlines()[1]
    end_column = len(body.encode('utf-16-le')) // 2
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
            (Path(folder) / name).write_bytes(source.encode())
            encoded = base64.b64encode(source.encode()).decode()
            endpoint = f"/api/projects/{project['id']}/editor-recovery"
            runtime.request('PUT', endpoint, {'revision': 0, 'state': {'format': 1,
                'root': {'mode': mode, 'path': path if mode == 'remote' else None}, 'selected': name,
                'files': [{'path': name, 'document': {'text': encoded, 'saved': encoded,
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
            wait('editable source-owned syntax paint', lambda: browser.script("""
                const input=document.querySelector('textarea[data-editor-path]');
                const paint=document.querySelector('.editor-highlight-content');
                return input && !input.readOnly && paint &&
                    !document.querySelector('.editor-recovery[aria-busy="true"]') &&
                    input.dataset.editorScope===paint.dataset.editorScope &&
                    !!document.querySelector('.highlight-ready .tok-string');
            """))
            # Font loading and recovery can move rows after their first paint.
            # Measure the interactive layout, rather than a transient startup frame.
            browser.call('POST', '/execute/async', {'args': [], 'script': """
                const done=arguments[0];
                document.fonts.ready.then(()=>requestAnimationFrame(()=>requestAnimationFrame(()=>done(true))));
            """})
            for line in ([2] if repeat == 1 else [2, 2002]):
                if line > 2:
                    browser.script("const s=document.querySelector('.editor-scroll-surface');s.scrollTop=2000*parseFloat(getComputedStyle(document.querySelector('textarea[data-editor-path]')).lineHeight);")
                wait('painted target row', lambda: browser.script(f"return !!document.querySelector('.editor-source-line[data-line=\"{line}\"]');"))
                for column, beyond, gap, drag in [
                        (8, False, 0, True), (16, False, 0, True),
                        (end_column, True, None, True),
                        (end_column, True, 4, False), (end_column, True, 40, False)]:
                    browser.script("document.querySelector('textarea[data-editor-path]').blur();")
                    x = 'rect.left+0.25'
                    if beyond:
                        x = 'input.getBoundingClientRect().right-20' if gap is None else f'rect.left+{gap}'
                    point = browser.script(f"""
                        const input=document.querySelector('textarea[data-editor-path]');
                        const row=document.querySelector('.editor-source-line[data-line="{line}"]');
                        const walker=document.createTreeWalker(row,NodeFilter.SHOW_TEXT);
                        let node,at={column}; while((node=walker.nextNode())) {{if(at<=node.length) break;at-=node.length;}}
                        const range=document.createRange();range.setStart(node,at);range.collapse(true);
                        const rect=range.getBoundingClientRect();
                        window.pointerEvents=[];
                        for(const type of ['mousedown','mouseup','click']) input.addEventListener(type,e=>{{
                            const event={{type,trusted:e.isTrusted}};pointerEvents.push(event);
                            queueMicrotask(()=>{{event.prevented=e.defaultPrevented;event.bounds=input.getBoundingClientRect().toJSON();event.row=row.getBoundingClientRect().toJSON();}});
                        }},{{once:true}});
                        return {{x:Math.min(input.getBoundingClientRect().right-2,{x}),y:rect.top+rect.height/2,bound:input.dataset.editorNativeBound==='true',bounds:input.getBoundingClientRect().toJSON()}};
                    """)
                    for kind in ['mouseMoved', 'mousePressed', 'mouseReleased']:
                        args = {'type': kind, 'x': point['x'], 'y': point['y']}
                        if kind != 'mouseMoved': args.update(button='left', clickCount=1)
                        cdp(browser, 'Input.dispatchMouseEvent', args)
                        if kind == 'mousePressed' and drag:
                            time.sleep(0.12)
                            cdp(browser, 'Input.dispatchMouseEvent', {'type': 'mouseMoved',
                                'x': point['x'], 'y': point['y'] + 0.25, 'buttons': 1})
                    start = len(''.join(source.splitlines(keepends=True)[:line-1]).encode())
                    expected = start + (len(body.encode()) if beyond else column)
                    def selection_matches():
                        saved = runtime.request('GET', endpoint)['state']['files'][0]['document']
                        return saved['selections'][0] == {'anchor': expected, 'head': expected}
                    try:
                        wait('trusted highlighted click position', selection_matches, seconds=5)
                    except AssertionError:
                        saved = runtime.request('GET', endpoint)['state']['files'][0]['document']
                        raise AssertionError({'mode': mode, 'repeat': repeat, 'language': language, 'line': line, 'column': column,
                            'gap': gap, 'drag': drag, 'expected': expected, 'point': point, 'events': browser.script('return pointerEvents;'), 'selection': saved['selections'], 'native': browser.script(f"const i=document.querySelector('textarea[data-editor-path]');const p=document.querySelector('.editor-highlight-content');const row=p.querySelector('.editor-source-line[data-line=\"{line}\"]');return {{start:i.selectionStart,end:i.selectionEnd,bound:i.dataset.editorNativeBound,readonly:i.readOnly,scope:i.dataset.editorScope,paint_scope:p.dataset.editorScope,scroll_top:i.scrollTop,scroll_left:i.scrollLeft,fonts:document.fonts.status,bounds:i.getBoundingClientRect().toJSON(),row:row?.getBoundingClientRect().toJSON()}};")}) from None
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
            print(json.dumps({'mode': mode, 'repeat': repeat, 'language': language, 'trusted_clicks': True}))
        finally:
            if browser: browser.stop()
            runtime.stop()


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--language', choices=list(FIXTURES), nargs='+', default=list(FIXTURES))
    args = parser.parse_args()
    for language in args.language:
        for mode in ['local', 'remote']:
            for repeat in [1, 1000]:
                check(mode, repeat, language)
