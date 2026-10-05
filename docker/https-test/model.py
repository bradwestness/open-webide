"""Deterministic streaming provider for the HTTPS transport smoke test."""
import json
import threading

release = threading.Event()
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

class Model(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        data = json.dumps({"data": [{"id": "proxy-test"}]}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        self.rfile.read(int(self.headers.get("Content-Length", 0)))
        if self.path == "/release":
            release.set()
            self.send_response(200)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        release.clear()
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Connection", "close")
        self.end_headers()
        for content in ["HTTPS ", "stream works"]:
            self.wfile.write(("data: " + json.dumps({"choices": [{"index": 0, "delta": {"content": content}, "finish_reason": None}]}) + "\n\n").encode())
            self.wfile.flush()
            if content == "HTTPS " and not release.wait(timeout=15):
                raise RuntimeError("First SSE delta was not delivered before the release deadline")
        self.wfile.write(b'data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}\n\ndata: [DONE]\n\n')
        self.wfile.flush()

ThreadingHTTPServer(("127.0.0.1", 5005), Model).serve_forever()
