import io
import json
import urllib.error
from unittest.mock import patch

import pytest

from export import create_uploaded_model


def test_import_failure_keeps_exact_request_and_reports_server_body(tmp_path):
    body = {"model": "mem-extractor:test", "files": {"model.gguf": "sha256:abc"}, "stream": False,
            "template": "frozen template", "system": "frozen system", "parameters": {"temperature": 0}}
    recovery = tmp_path / "ollama-create-recovery.json"
    error = urllib.error.HTTPError("http://localhost/api/create", 500, "Internal Server Error", {},
                                   io.BytesIO(b'{"error":"no space left on device"}'))
    with patch("urllib.request.urlopen", side_effect=error):
        with pytest.raises(RuntimeError, match="no space left on device") as failure:
            create_uploaded_model("http://localhost", body, recovery)
    assert json.loads(recovery.read_text()) == body
    assert "sha256:abc" in str(failure.value)
    assert "do not remerge or retrain" in str(failure.value)
    assert str(recovery) in str(failure.value)
