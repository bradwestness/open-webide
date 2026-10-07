#!/usr/bin/env python3
"""Measure the built Rust/WASM editor using disposable Spin/SQLite and Chrome.

Set CHROMEDRIVER (and optionally CHROME) after spin build. Results are JSON lines;
each case has a fresh account, runtime and browser process tree. Local cases use
recovery without an OS directory handle, so these are editor/view measurements,
not local filesystem permission checks. Summed RSS includes shared pages more
than once; Linux PSS, when readable, apportions those pages across processes.
"""
import argparse
import base64
import json
import os
from pathlib import Path
import platform
import runpy
import secrets
import subprocess
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parent.parent
SUPPORT = runpy.run_path(str(ROOT / "tools/check-editor-recovery.py"))
Browser, Runtime = SUPPORT["Browser"], SUPPORT["Runtime"]


def host_constraints():
    """Record Linux resource limits so container samples keep their context."""
    constraints = {"cpuCount": os.cpu_count()}
    for field, name in [("cgroupMemoryMax", "memory.max"), ("cgroupCpuMax", "cpu.max")]:
        try:
            constraints[field] = Path("/sys/fs/cgroup", name).read_text().strip()
        except OSError:
            constraints[field] = None
    return constraints


def process_memory(driver):
    records = {}
    for line in subprocess.check_output(["ps", "-axo", "pid=,ppid=,rss="], text=True).splitlines():
        pid, parent, rss = map(int, line.split())
        records[pid] = (parent, rss)
    owned = {driver}
    while True:
        children = {pid for pid, (parent, _) in records.items() if parent in owned}
        if children <= owned:
            break
        owned |= children
    owned.discard(driver)
    pss = []
    for pid in owned:
        try:
            fields = Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines()
            pss.append(int(next(line for line in fields if line.startswith("Pss:")).split()[1]))
        except (OSError, StopIteration):
            pass
    return {"chrome_processes": len(owned),
            "chrome_summed_rss_kib": sum(records[pid][1] for pid in owned),
            "chrome_pss_kib": sum(pss) if len(pss) == len(owned) and owned else None}


def source_for(case):
    row = "let value = call(\"文😀 café\"); // words for wrapping\r\n"
    if case == "small":
        return row * 1_000
    if case == "medium":
        return row * (2 * 1024 * 1024 // len(row.encode()))
    if case == "byte-limit":
        row = "let value = call(\"文😀 café\"); // " + "words " * 64 + "\r\n"
        return row * (8 * 1024 * 1024 // len(row.encode()))
    if case == "line-limit":
        return "x\n" * 99_999
    if case == "long-line":
        return "文😀 words " * (1024 * 1024 // len("文😀 words ".encode()))
    raise ValueError(case)


class MemorySamples:
    """Observe owned Chrome processes; never collect unrelated process arguments."""

    def __init__(self, driver):
        self.driver = driver
        self.initial = process_memory(driver)
        self.peak_rss = self.initial["chrome_summed_rss_kib"]
        self.peak_pss = self.initial["chrome_pss_kib"]
        self.count = 1
        self.error = None
        self.stopped = threading.Event()
        self.thread = threading.Thread(target=self.sample, daemon=True)
        self.thread.start()

    def sample(self):
        while not self.stopped.wait(.2):
            try:
                value = process_memory(self.driver)
                self.peak_rss = max(self.peak_rss, value["chrome_summed_rss_kib"])
                if value["chrome_pss_kib"] is not None:
                    self.peak_pss = max(self.peak_pss or 0, value["chrome_pss_kib"])
                self.count += 1
            except (OSError, ValueError) as error:
                self.error = type(error).__name__
                return

    def finish(self):
        self.stopped.set()
        self.thread.join()
        assert self.error is None, f"Process memory sampling failed: {self.error}"
        final = process_memory(self.driver)
        self.peak_rss = max(self.peak_rss, final["chrome_summed_rss_kib"])
        if final["chrome_pss_kib"] is not None:
            self.peak_pss = max(self.peak_pss or 0, final["chrome_pss_kib"])
        self.count += 1
        return {"beforeLoadMemory": self.initial, "peakChromeSummedRssKiB": self.peak_rss,
                "peakChromePssKiB": self.peak_pss, "memorySamples": self.count,
                "memorySampleIntervalMs": 200}


READY = """
    const input = document.querySelector('textarea[data-editor-path]');
    const paint = document.querySelector('.editor-highlight:not(.editor-caret-measure) .editor-highlight-content');
    const extent = input?.parentElement.querySelector('.editor-scroll-extent');
    const bound = input?.dataset.editorNativeBound === 'true' &&
        input.dataset.editorNativeGeneration && extent?.dataset.editorScope === input.dataset.editorScope;
    return input && paint && (input.value.length === arguments[0] || bound) &&
        paint.dataset.editorScope === input.dataset.editorScope &&
        (getComputedStyle(input).whiteSpace !== 'pre-wrap' || Number(paint.dataset.documentHeight) > 0) &&
        input.parentElement.classList.contains('highlight-ready');
"""


def measure(case, mode, wrapped, trace=False, repetition=1):
    source = source_for(case)
    native = source.replace("\r\n", "\n")
    # JS lengths are UTF-16, not Python's Unicode scalar count.
    native_length = len(native.encode("utf-16-le")) // 2
    with tempfile.TemporaryDirectory(prefix="openwebide-view-") as state, \
            tempfile.TemporaryDirectory(prefix="editor-view-probe-", dir=ROOT) as folder:
        runtime = Runtime(state, frontend=True)
        browser = None
        samples = None
        phase = "runtime"
        try:
            runtime.start()
            runtime.request("POST", "/api/auth/register", {
                "username": "measure-" + secrets.token_hex(8), "password": secrets.token_urlsafe(32),
            }, expected=201)
            path = Path(folder).relative_to(ROOT.parent.parent).as_posix() if mode == "remote" else "measure-folder"
            project = runtime.request("POST", "/api/projects", {
                "name": "View measurement", "mode": mode, "path": path,
            }, expected=201)
            (Path(folder) / "measure.rs").write_bytes(source.encode())
            encoded = base64.b64encode(source.encode()).decode()
            recovery = {"format": 1, "root": {"mode": mode, "path": path if mode == "remote" else None},
                        "selected": "measure.rs", "files": [{"path": "measure.rs",
                        "document": {"text": encoded, "saved": encoded,
                                     "selections": [{"anchor": 0, "head": 0}], "collapsed": []},
                        "scroll": {"top": 0, "left": 0}, "read_only": False}]}
            runtime.request("PUT", f"/api/projects/{project['id']}/editor-recovery",
                            {"revision": 0, "state": recovery})
            for key, value in {
                "open_tabs": json.dumps([project["id"]]), "active_project": str(project["id"]),
                "bridge_url": "http://127.0.0.1:1",
                "editor_preferences": json.dumps({"indentation": {"style": "Spaces", "width": 4, "tab_width": 4},
                                                   "word_wrap": wrapped, "show_whitespace": False}),
            }.items():
                runtime.request("PUT", "/api/settings", {"key": key, "value": value})
            browser = Browser(state)
            browser.call("POST", "/url", {"url": runtime.url + "/offline.html"})
            for cookie in runtime.cookies:
                browser.call("POST", "/cookie", {"cookie": {"name": cookie.name, "value": cookie.value,
                             "domain": "127.0.0.1", "path": cookie.path, "httpOnly": True}})
            samples = MemorySamples(browser.process.pid)
            browser.call("POST", "/goog/cdp/execute", {"cmd": "Page.addScriptToEvaluateOnNewDocument", "params": {
                "source": """
                    window.editorViewScroll = input => {
                        const scroll = input.parentElement?.querySelector('.editor-scroll-surface');
                        return scroll && getComputedStyle(scroll).position === 'absolute' ? scroll : input;
                    };
                    window.editorViewMeasurement = {maxFrameMs: 0, inputAt: 0,
                        longTasks: 0, longTaskMs: 0, maxLongTaskMs: 0, phase: "cold", probes: [], batches: [], workers: [], fonts: [], traceTruncated: false};
                    if (__TRACE__) {
                        const batches = new WeakMap();
                        window.__openwebideEditorProbeTiming = (paint, phase, elapsedMs, units) => {
                            let record = batches.get(paint);
                            if (phase === "render") {
                                record = null;
                                if (editorViewMeasurement.batches.length < 256) {
                                    record = {at: performance.now(), phase: editorViewMeasurement.phase,
                                        fontFaces: Array.from(document.fonts).slice(0, 16)
                                            .map(face => ({family: face.family, status: face.status})),
                                        scope: Object.fromEntries(Array.from(paint.closest(".editor-row-measure").attributes)
                                            .filter(attribute => attribute.name.startsWith("data-measure-"))
                                            .map(attribute => [attribute.name, attribute.value]))};
                                    editorViewMeasurement.batches.push(record);
                                } else editorViewMeasurement.traceTruncated = true;
                                batches.set(paint, record);
                            }
                            if (record) record[phase] = {elapsedMs, units};
                        };
                        const rows = new WeakMap();
                        function recordFor(element) {
                            const row = element?.closest(".editor-row-measure .editor-source-line");
                            let record = row && rows.get(row);
                            if (row && !record && editorViewMeasurement.probes.length < 256) {
                                record = {at: performance.now(), phase: editorViewMeasurement.phase,
                                    cold: !!row.closest(".editor-height-measure"),
                                    sliced: row.hasAttribute("data-source-start"),
                                    sourceNative: row.textContent.length,
                                    probeStyle: row.closest(".editor-row-measure").getAttribute("style"),
                                    rowStyle: row.getAttribute("style"),
                                    scope: Object.fromEntries(Array.from(row.closest(".editor-row-measure").attributes)
                                        .filter(attribute => attribute.name.startsWith("data-measure-"))
                                        .map(attribute => [attribute.name, attribute.value])),
                                    calls: 0, layoutMs: 0,
                                    boundsCalls: 0, boundsMs: 0};
                                rows.set(row, record); editorViewMeasurement.probes.push(record);
                            }
                            if (row && !record) editorViewMeasurement.traceTruncated = true;
                            return record;
                        }
                        const original = Range.prototype.getClientRects;
                        Range.prototype.getClientRects = function(...args) {
                            const node = this.startContainer;
                            const record = recordFor(node.nodeType === 1 ? node : node.parentElement);
                            const started = performance.now();
                            try { return original.apply(this, args); }
                            finally { if (record) { record.calls++; record.layoutMs += performance.now()-started; } }
                        };
                        const originalBounds = Element.prototype.getBoundingClientRect;
                        Element.prototype.getBoundingClientRect = function(...args) {
                            const record = recordFor(this);
                            const started = performance.now();
                            try { return originalBounds.apply(this, args); }
                            finally { if (record) { record.boundsCalls++; record.boundsMs += performance.now()-started; } }
                        };
                        function recordEvent(events, kind) {
                            if (events.length >= 256) {
                                editorViewMeasurement.traceTruncated = true;
                                return null;
                            }
                            const record = {at: performance.now(), phase: editorViewMeasurement.phase, kind};
                            events.push(record);
                            return record;
                        }
                        for (const kind of ["loading", "loadingdone", "loadingerror"])
                            document.fonts.addEventListener(kind, event => {
                                const record = recordEvent(editorViewMeasurement.fonts, kind);
                                if (record) record.faces = Array.from(event.fontfaces || []).slice(0, 16)
                                    .map(face => ({family: face.family, status: face.status}));
                            });
                        const OriginalWorker = Worker;
                        Worker = class extends OriginalWorker {
                            constructor(...args) {
                                super(...args);
                                this.addEventListener("message", () => recordEvent(editorViewMeasurement.workers, "reply"));
                            }
                            postMessage(...args) {
                                recordEvent(editorViewMeasurement.workers, "request");
                                return super.postMessage(...args);
                            }
                        };
                    }
                    new PerformanceObserver(list => {
                        for (const entry of list.getEntries()) {
                            editorViewMeasurement.longTasks++;
                            editorViewMeasurement.longTaskMs += entry.duration;
                            editorViewMeasurement.maxLongTaskMs = Math.max(editorViewMeasurement.maxLongTaskMs, entry.duration);
                        }
                    }).observe({type: 'longtask', buffered: true});
                    let last;
                    function tick(at) {
                        if (last !== undefined) editorViewMeasurement.maxFrameMs = Math.max(editorViewMeasurement.maxFrameMs, at-last);
                        last = at; requestAnimationFrame(tick);
                    }
                    requestAnimationFrame(tick);
                    document.addEventListener('beforeinput', event => {
                        if (event.target.matches('textarea[data-editor-path]')) editorViewMeasurement.inputAt = performance.now();
                    }, true);
                """.replace("__TRACE__", json.dumps(trace))}})
            started = time.monotonic()
            phase = "cold paint"
            browser.call("POST", "/url", {"url": runtime.url + "/"})
            deadline = time.monotonic() + 60
            while not browser.call("POST", "/execute/sync", {"script": READY, "args": [native_length]}):
                assert time.monotonic() < deadline, "Editor view readiness timed out"
                time.sleep(0.05)
            load_ms = (time.monotonic() - started) * 1000
            snapshot = browser.call("POST", "/execute/async", {"script": r"""
                const done = arguments[0];
                (async () => {
                    const link = [...document.querySelectorAll('link[rel=modulepreload]')]
                        .find(link => /\/openwebide-frontend-[^/]+\.js$/.test(link.href));
                    const module = await import(link.href); const wasm = await module.default();
                    window.editorViewWasmMemory = wasm.memory;
                    const input = document.querySelector('textarea[data-editor-path]');
                    const scroll = editorViewScroll(input);
                    done({nativeInputLength: input.value.length, nativeBound: input.dataset.editorNativeBound === 'true', appModule: new URL(link.href).pathname, wasmCommittedBytes: wasm.memory.buffer.byteLength,
                        domNodes: document.getElementsByTagName('*').length,
                        paintRows: document.querySelectorAll('.editor-source-line').length,
                        scrollHeight: scroll.scrollHeight, scrollWidth: scroll.scrollWidth, clientWidth: scroll.clientWidth,
                        maxFrameMs: editorViewMeasurement.maxFrameMs,
                        wrapped: getComputedStyle(input).whiteSpace === 'pre-wrap'});
                })().catch(error => done({error: String(error)}));
            """, "args": []})
            assert "error" not in snapshot, snapshot
            assert snapshot["wrapped"] == wrapped, "Editor preference was not applied"
            memory = process_memory(browser.process.pid)
            phase = "scroll paint"
            scroll = browser.call("POST", "/execute/async", {"script": """
                const done = arguments[arguments.length - 1], input = document.querySelector('textarea[data-editor-path]');
                const singleRow = arguments[0];
                const scroll = editorViewScroll(input);
                const horizontal = singleRow && getComputedStyle(input).whiteSpace !== 'pre-wrap';
                editorViewMeasurement.phase = "scroll";
                const started = performance.now(); const old = document.querySelector('.editor-source-line')?.dataset.line;
                if (horizontal) scroll.scrollLeft = (scroll.scrollWidth - scroll.clientWidth) * .7;
                else scroll.scrollTop = scroll.scrollHeight * .7;
                scroll.dispatchEvent(new Event('scroll'));
                const deadline = started + 10000;
                function check() {
                    const row = document.querySelector('.editor-source-line');
                    const first = row?.dataset.line;
                    const paint = document.querySelector('.editor-highlight-content');
                    if (performance.now() > deadline) return done({error: 'Scroll paint timed out'});
                    const offset = Number(horizontal ? row?.dataset.paintLeft : row?.dataset.paintTop);
                    const target = horizontal ? scroll.scrollLeft : scroll.scrollTop;
                    const margin = horizontal ? scroll.clientWidth * 2 + 30 : scroll.clientHeight + 8 * parseFloat(getComputedStyle(input).lineHeight) + 32;
                    const moved = singleRow ? Number.isFinite(offset) && offset <= target + 30 && target - offset <= margin : first !== old;
                    if (moved && paint?.dataset.editorScope === input.dataset.editorScope && input.parentElement.classList.contains('highlight-ready'))
                        return done({scrollToPaintMs: performance.now()-started, scrollAxis: horizontal ? 'horizontal' : 'vertical', scrollOffset: target});
                    requestAnimationFrame(check);
                }
                requestAnimationFrame(check);
            """, "args": ["\n" not in native]})
            assert "error" not in scroll, scroll
            browser.script("const input=document.querySelector('textarea[data-editor-path]'); input.focus(); input.setSelectionRange(0,0);")
            phase = "input paint"
            browser.script("editorViewMeasurement.phase = 'input';")
            browser.call("POST", "/goog/cdp/execute", {"cmd": "Input.insertText", "params": {"text": "z"}})
            edited = browser.call("POST", "/execute/async", {"script": """
                const done=arguments[0], deadline=performance.now()+10000;
                function check() {
                    const input=document.querySelector('textarea[data-editor-path]');
                    const paint=document.querySelector('.editor-highlight-content');
                    if (performance.now()>deadline) return done({error:'Input paint timed out'});
                    if (input.value.startsWith('z') && paint.dataset.editorScope===input.dataset.editorScope &&
                        (getComputedStyle(input).whiteSpace !== 'pre-wrap' || Number(paint.dataset.documentHeight)>0) &&
                        input.parentElement.classList.contains('highlight-ready'))
                        return done({inputToPaintMs:performance.now()-editorViewMeasurement.inputAt});
                    requestAnimationFrame(check);
                }
                requestAnimationFrame(check);
            """, "args": []})
            assert "error" not in edited, edited
            browser.script("editorViewMeasurement.phase = 'deferred';")
            # Include deferred syntax preparation and queued recovery saves.
            browser.call("POST", "/execute/async", {"script": "setTimeout(arguments[0], 1200);", "args": []})
            sampled = samples.finish()
            samples = None
            tasks = browser.script("return {...window.editorViewMeasurement, wasmBytes: window.editorViewWasmMemory.buffer.byteLength};")
            return {"case": case, "mode": mode, "wrap": wrapped, "repetition": repetition, "sourceBytes": len(source.encode()),
                    "loadToPaintMs": load_ms, **snapshot, **scroll, **edited, **memory,
                    "afterInputMemory": process_memory(browser.process.pid), **sampled,
                    "afterInputWasmCommittedBytes": tasks["wasmBytes"],
                    "longTasks": tasks["longTasks"], "longTaskMs": tasks["longTaskMs"],
                    "maxLongTaskMs": tasks["maxLongTaskMs"], "maxFrameIncludingInputMs": tasks["maxFrameMs"],
                    **({"layoutProbes": tasks["probes"], "preparationBatches": tasks["batches"], "workerEvents": tasks["workers"], "fontEvents": tasks["fonts"], "traceTruncated": tasks["traceTruncated"]} if trace else {})}
        except (AssertionError, TimeoutError, RuntimeError) as error:
            observed = samples.finish() if samples else {}
            samples = None
            print(json.dumps({"case": case, "mode": mode, "wrap": wrapped, "repetition": repetition, "sourceBytes": len(source.encode()),
                              "failedPhase": phase, "failure": type(error).__name__, **observed}), flush=True)
            raise
        finally:
            try:
                if samples:
                    samples.finish()
            finally:
                try:
                    if browser:
                        browser.stop()
                finally:
                    runtime.stop()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", nargs="+", choices=["small", "medium", "byte-limit", "line-limit", "long-line"], default=["small"])
    parser.add_argument("--modes", nargs="+", choices=["local", "remote"], default=["local", "remote"])
    parser.add_argument("--wrap", action="store_true")
    parser.add_argument("--trace", action="store_true", help="Record bounded probe/worker diagnostics; timings include instrumentation overhead")
    parser.add_argument("--repeat", type=int, default=1, help="Fresh browser/runtime runs for each case and mode")
    parser.add_argument("--require-pss", action="store_true", help="Fail if apportioned Chrome process memory cannot be measured")
    args = parser.parse_args()
    if args.repeat < 1:
        parser.error("--repeat must be positive")
    assert os.environ.get("CHROMEDRIVER"), "Set CHROMEDRIVER to a compatible driver"
    print(json.dumps({"host": platform.platform(), "measurement": "production editor; process-tree memory",
                      **host_constraints(),
                      "repetitions": args.repeat,
                      "measurementImage": os.environ.get("EDITOR_VIEW_IMAGE"),
                      "inputTimingStart": "beforeinput",
                      "checkoutHead": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                      "browser": subprocess.check_output([os.environ["CHROME"], "--version"], text=True).strip() if os.environ.get("CHROME") else "WebDriver default"}), flush=True)
    for case in args.cases:
        for mode in args.modes:
            for repetition in range(1, args.repeat + 1):
                result = measure(case, mode, args.wrap, args.trace, repetition)
                print(json.dumps(result), flush=True)
                if args.require_pss:
                    assert result["peakChromePssKiB"] is not None, "Chrome peak PSS was unavailable"
                    assert result["afterInputMemory"]["chrome_pss_kib"] is not None, "Chrome final PSS was unavailable"
