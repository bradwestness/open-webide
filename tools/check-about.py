#!/usr/bin/env python3
"""Verify the built About page's layout and actual offline PWA navigation."""
import argparse
import base64
import functools
import http.server
import os
from pathlib import Path
import runpy
import tempfile
import threading

ROOT = Path(__file__).resolve().parent.parent
Browser = runpy.run_path(str(ROOT / 'tools/check-editor-recovery.py'))['Browser']


def check(screenshots=None):
    assert os.environ.get('CHROMEDRIVER'), 'Set CHROMEDRIVER to a compatible driver'
    dist = ROOT / 'frontend/dist'
    assert (dist / 'about.html').is_file(), 'Build the frontend first'
    assert '"/about.html"' in (dist / 'service-worker.js').read_text()
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(dist))
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix='openwebide-about-browser-') as directory:
            browser = Browser(directory)
            try:
                url = f'http://127.0.0.1:{server.server_port}'
                browser.call('POST', '/url', {'url': url + '/about.html'})
                initial = browser.script("return document.querySelector('.about-build').textContent;")
                for width, theme in [(1280, 'dark'), (390, 'light')]:
                    browser.call('POST', '/window/rect', {'width': width, 'height': 900})
                    browser.call('POST', '/goog/cdp/execute', {'cmd': 'Emulation.setDeviceMetricsOverride', 'params': {'width': width, 'height': 900, 'deviceScaleFactor': 1, 'mobile': width < 600}})
                    browser.script(f"document.documentElement.dataset.theme = '{theme}'; document.querySelector('.about-package').open = true;")
                    layout = browser.script("""
                        return {width: innerWidth, scroll: document.documentElement.scrollWidth,
                            height: document.documentElement.scrollHeight, viewport: innerHeight,
                            font: document.body.textContent.includes('SIL OPEN FONT LICENSE'),
                            lucide: document.body.textContent.includes('Lucide icons')};
                    """)
                    assert layout['width'] == width, layout
                    assert layout['scroll'] <= layout['width'], layout
                    assert layout['height'] > layout['viewport'], 'Notices must remain scrollable'
                    assert layout['font'] and layout['lucide']
                    if screenshots:
                        screenshots.mkdir(parents=True, exist_ok=True)
                        (screenshots / f'about-{width}-{theme}.png').write_bytes(base64.b64decode(browser.call('GET', '/screenshot')))
                result = browser.call('POST', '/execute/async', {'script': """
                    const done = arguments[0];
                    navigator.serviceWorker.register('/service-worker.js')
                        .then(() => navigator.serviceWorker.ready)
                        .then(() => {
                            if (navigator.serviceWorker.controller) done(true);
                            else navigator.serviceWorker.addEventListener('controllerchange', () => done(true), {once:true});
                        }).catch(error => done({error: String(error)}));
                """, 'args': []})
                assert result is True, result
                # Stop the real origin: a page-scoped CDP offline flag does not
                # consistently stop fetches made by the service worker itself.
                server.shutdown()
                server.server_close()
                thread.join()
                browser.call('POST', '/url', {'url': url + '/'})
                assert browser.script("return !!document.querySelector('a[href=\"/about.html\"]');")
                browser.call('POST', '/url', {'url': url + '/about.html'})
                assert browser.script("return document.querySelector('.about-build').textContent;") == initial
                assert browser.script("return document.body.textContent.includes('SIL OPEN FONT LICENSE');")
                print('PASS: built About notices, desktop/phone themes, scroll/overflow and real offline PWA navigation')
            finally:
                browser.stop()
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--screenshots', type=Path)
    check(parser.parse_args().screenshots)
