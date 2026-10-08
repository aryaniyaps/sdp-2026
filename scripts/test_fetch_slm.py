"""scripts/fetch-slm.sh against a fake Ollama: what it uploads, what it creates, when it does nothing."""

import hashlib
import json
import os
import subprocess
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts" / "fetch-slm.sh"


class Fake(BaseHTTPRequestHandler):
    calls: list = []
    installed = False
    weights = b""
    checksum = ""

    def log_message(self, *args):
        pass

    def _send(self, status, body=b"{}"):
        self.send_response(status)
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        self.calls.append(("GET", self.path, b""))
        if self.path == "/release/SHA256SUMS":
            return self._send(200, f"{self.checksum}  mem-extractor-q8_0.gguf\n".encode())
        if self.path == "/release/mem-extractor-q8_0.gguf":
            return self._send(200, self.weights)
        self._send(200, b'{"version":"0.13.5"}')

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("content-length", 0)))
        self.calls.append(("POST", self.path, body if self.path != "/api/blobs/" else b""))
        if self.path == "/api/show":
            return self._send(200 if type(self).installed else 404)
        if self.path.startswith("/api/blobs/"):
            return self._send(201)
        if self.path == "/api/create":
            type(self).installed = True
            return self._send(200, b'{"status":"success"}')
        self._send(404)


class FetchSlm(unittest.TestCase):
    def setUp(self):
        Fake.calls, Fake.installed = [], False
        self.server = HTTPServer(("127.0.0.1", 0), Fake)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.server.server_port}"
        self.tmp = tempfile.TemporaryDirectory()
        self.weights = Path(self.tmp.name) / "m.gguf"
        self.weights.write_bytes(b"GGUF" + os.urandom(2048))
        Fake.weights = self.weights.read_bytes()
        Fake.checksum = hashlib.sha256(Fake.weights).hexdigest()

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.tmp.cleanup()

    def run_script(self, **env):
        return subprocess.run([str(SCRIPT)], capture_output=True, text=True, cwd=ROOT,
                              env={**os.environ, "OLLAMA_URL": self.url, "SLM_FILE": str(self.weights), **env})

    def test_uploads_the_blob_and_creates_the_model_from_the_shared_spec(self):
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        digest = hashlib.sha256(self.weights.read_bytes()).hexdigest()
        paths = [(m, p) for m, p, _ in Fake.calls]
        self.assertIn(("POST", f"/api/blobs/sha256:{digest}"), paths)
        create = json.loads(next(b for m, p, b in Fake.calls if p == "/api/create"))
        spec = json.loads((ROOT / "slm-distill" / "ollama-model.json").read_text())
        self.assertEqual(create["model"], "mem-extractor")
        self.assertEqual(create["files"], {"model.gguf": f"sha256:{digest}"})
        for key in ("template", "system", "parameters"):
            self.assertEqual(create[key], spec[key])
        self.assertNotIn("<think>", create["template"])
        self.assertLess(paths.index(("POST", f"/api/blobs/sha256:{digest}")), paths.index(("POST", "/api/create")))

    def test_does_nothing_when_the_model_is_already_installed(self):
        Fake.installed = True
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("already installed", result.stdout)
        self.assertFalse([c for c in Fake.calls if c[1] == "/api/create" or c[1].startswith("/api/blobs/")])

    def test_force_creates_again(self):
        Fake.installed = True
        self.assertEqual(self.run_script(FORCE="1").returncode, 0)
        self.assertTrue([c for c in Fake.calls if c[1] == "/api/create"])

    def test_a_missing_server_is_a_clear_error(self):
        result = self.run_script(OLLAMA_URL="http://127.0.0.1:1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("No Ollama server answers", result.stderr)

    def download(self):
        return self.run_script(SLM_FILE="", SLM_CACHE=self.tmp.name,
                               SLM_BASE_URL=self.url + "/release")

    def test_download_verifies_release_checksum_before_upload(self):
        result = self.download()
        self.assertEqual(result.returncode, 0, result.stderr)
        cached = Path(self.tmp.name) / "mem-extractor-q8_0.gguf"
        self.assertEqual(cached.read_bytes(), Fake.weights)
        self.assertTrue(any(p.startswith("/api/blobs/") for _, p, _ in Fake.calls))

    def test_checksum_mismatch_does_not_upload_or_replace_cache(self):
        cached = Path(self.tmp.name) / "mem-extractor-q8_0.gguf"
        cached.write_bytes(b"old incomplete download")
        Fake.checksum = "0" * 64
        result = self.download()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checksum mismatch", result.stderr)
        self.assertEqual(cached.read_bytes(), b"old incomplete download")
        self.assertFalse(any(p.startswith("/api/blobs/") for _, p, _ in Fake.calls))

    def test_verified_cached_file_is_not_downloaded_again(self):
        cached = Path(self.tmp.name) / "mem-extractor-q8_0.gguf"
        cached.write_bytes(Fake.weights)
        result = self.download()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any(p.endswith(".gguf") for _, p, _ in Fake.calls))

    def test_malformed_checksum_stops_before_weights_download(self):
        Fake.checksum = "not-a-sha256"
        result = self.download()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(any(p.endswith(".gguf") or p.startswith("/api/blobs/") for _, p, _ in Fake.calls))


if __name__ == "__main__":
    unittest.main()
