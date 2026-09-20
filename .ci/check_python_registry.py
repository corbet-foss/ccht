"""Consume the released Python wheel from PyPI after verified publication."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib


def run(*args, **kwargs):
    print("+ " + " ".join(map(str, args)), flush=True)
    return subprocess.run(list(map(str, args)), check=True, **kwargs)


root = Path.cwd()
project = tomllib.loads((root / "py/pyproject.toml").read_text())["project"]
assert project["name"] == "ccht"
version = project["version"]
env = dict(os.environ, UV_PYTHON_DOWNLOADS="never")
for key in ("UV_INDEX", "UV_EXTRA_INDEX_URL", "UV_OFFLINE", "PIP_EXTRA_INDEX_URL"):
    env.pop(key, None)
with tempfile.TemporaryDirectory(prefix="ccht-pypi-consumer-", dir=os.environ.get("TMPDIR")) as directory:
    scratch = Path(directory)
    venv = scratch / "venv"
    run("uv", "--no-config", "venv", "--no-python-downloads", "--python", shutil.which("python3"), venv, env=env)
    python = venv / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
    run("uv", "--no-config", "--no-cache", "pip", "install", "--python", python,
        "--no-deps", "--only-binary", ":all:", "--index-url", "https://pypi.org/simple",
        f"ccht=={version}", env=env)
    script = scratch / "consumer.py"
    shutil.copy2(root / ".ci/python-consumer.py", script)
    run(python, script, cwd=scratch, env=env)
    actual = subprocess.check_output([str(python), "-c", "from importlib.metadata import version; print(version('ccht'))"], env=env, text=True).strip()
    assert actual == version
    print(json.dumps({"registry": "https://pypi.org/simple", "package": "ccht", "version": actual,
                      "installed_wheel_consumer": "passed", "source_commit": os.environ["CI_COMMIT_SHA"]}))
