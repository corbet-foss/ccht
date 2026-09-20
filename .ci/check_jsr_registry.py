"""Exercise the immutable JSR version through an independent Deno consumer."""

import json
import os
from pathlib import Path
import subprocess
import tempfile


root = Path.cwd()
origin = json.loads((root / ".ci/jsr-input.json").read_text())
name, version = origin["package"], origin["version"]
specifier = f"jsr:{name}@{version}"
wasm_url = f"https://jsr.io/{name}/{version}/wasm/ccht_bg.wasm"
consumer = (root / ".ci/web-consumer.mjs").read_text()
consumer = consumer.replace("import { readFile } from 'node:fs/promises';\n", "")
consumer = consumer.replace("from '@corbet-foss/ccht'", "from " + json.dumps(specifier))
old_load = "const wasm = await readFile(new URL(import.meta.resolve('@corbet-foss/ccht/ccht_bg.wasm')));"
assert consumer.count(old_load) == 1
consumer = consumer.replace(old_load, "const wasm = new URL(" + json.dumps(wasm_url) + ");")
with tempfile.TemporaryDirectory(prefix="ccht-jsr-registry-", dir=os.environ.get("TMPDIR")) as directory:
    work = Path(directory)
    (work / "check.mjs").write_text(consumer)
    # A fresh package cache proves that the actual JSR imports and versioned
    # Wasm are available; only JSR network access is granted to the consumer.
    environment = dict(os.environ, DENO_DIR=str(work / "deno-cache"))
    subprocess.run(["deno", "--version"], check=True, timeout=30)
    # Verify our exact just-published version, whose complete file inventory was
    # already checked by the publisher, before Deno's default age window expires.
    subprocess.run(["deno", "run", "--no-config", "--minimum-dependency-age=0",
                    "--allow-import=jsr.io", "--allow-net=jsr.io",
                    "check.mjs"], cwd=work, env=environment, check=True, timeout=180)
print(json.dumps({"registry": "jsr", "package": name, "version": version,
                  "consumer": "passed", "wasm_url": wasm_url}))
