#!/usr/bin/env python3
"""Static server plus evaluation bridge for a built frontend bundle.

Serves the SPA output directory on the Tauri dev URL port and relays
`POST /__dbunk/eval` bodies to the page, which long-polls `/__dbunk/next` and
posts results to `/__dbunk/result`. Dev-only walkthrough tooling.
Usage: serve.py <dist-dir> [port]
"""
import http.server
import json
import queue
import sys
import threading
import uuid

ROOT = sys.argv[1]
PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 3000
commands = queue.Queue()
results = {}
lock = threading.Condition()
hello = None


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=ROOT, **kwargs)

    def log_message(self, *args):
        pass

    def _json(self, payload, status=200):
        body = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header('content-type', 'application/json')
        self.send_header('content-length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _body(self):
        return self.rfile.read(int(self.headers.get('content-length', 0)))

    def do_GET(self):
        global hello
        if self.path == '/__dbunk/status':
            return self._json({'hello': hello, 'pending': commands.qsize()})
        if self.path.startswith('/__dbunk/next'):
            try:
                command = commands.get(timeout=25)
            except queue.Empty:
                return self._json(None, 204)
            return self._json(command)
        return super().do_GET()

    def do_POST(self):
        global hello
        if self.path == '/__dbunk/hello':
            hello = json.loads(self._body())
            return self._json({'ok': True})
        if self.path == '/__dbunk/result':
            data = json.loads(self._body())
            with lock:
                results[data['id']] = data
                lock.notify_all()
            return self._json({'ok': True})
        if self.path == '/__dbunk/eval':
            expr = self._body().decode()
            timeout = float(self.headers.get('x-timeout-ms', 60000)) / 1000
            job_id = uuid.uuid4().hex
            commands.put({'id': job_id, 'expr': expr})
            with lock:
                ok = lock.wait_for(lambda: job_id in results, timeout=timeout)
                data = results.pop(job_id, None) if ok else None
            return self._json(data or {'id': job_id, 'ok': False, 'error': f'timeout after {timeout}s'})
        self.send_error(404)


class Server(http.server.ThreadingHTTPServer):
    daemon_threads = True


if __name__ == '__main__':
    print(f'serving {ROOT} on http://localhost:{PORT}', flush=True)
    Server(('127.0.0.1', PORT), Handler).serve_forever()
