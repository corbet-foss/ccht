"""Prepare JSR from the immutable npm Wasm release, with a lossless source kit."""

import gzip
import hashlib
import io
import json
import lzma
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request


def digest(data):
    return hashlib.sha256(data).hexdigest()


def run(*args, **kwargs):
    subprocess.run(args, check=True, timeout=180, **kwargs)


def files_from_tar(data):
    files = {}
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        for entry in archive:
            path = PurePosixPath(entry.name)
            if path.is_absolute() or ".." in path.parts:
                raise ValueError("Unsafe archive path")
            if entry.isdir():
                continue
            if not entry.isfile() or str(path) in files:
                raise ValueError("Archive must contain unique regular files")
            files[str(path)] = archive.extractfile(entry).read()
    return files


root = Path.cwd()
inputs = json.loads((root / ".ci/jsr-input.json").read_text())
if not shutil.which("deno"):
    raise RuntimeError("JSR checks require an already installed Deno")
run("deno", "--version")
with urllib.request.urlopen(inputs["archive_url"], timeout=60) as response:
    original_archive = response.read(25_000_001)
assert digest(original_archive) == inputs["archive_sha256"]
original = files_from_tar(original_archive)
assert all(name.startswith("package/") for name in original)
original = {name.removeprefix("package/"): data for name, data in original.items()}
manifest = json.loads(original["package.json"])
assert manifest["name"] == inputs["package"]
assert manifest["version"] == inputs["version"]
assert manifest["gitHead"] == inputs["source_commit"]
assert manifest["license"] == "LGPL-3.0-only WITH LGPL-3.0-linking-exception"
for name in (
    "index.js",
    "index.d.ts",
    "src/auth.ts",
    "src/dock.ts",
    "src/components/AccountConnection.svelte",
    "src/components/ChatDock.svelte",
    "src/components/Dock.svelte",
    "src/components/StepConfig.svelte",
):
    assert original[name] == (root / "web" / name).read_bytes(), name
assert original["README.md"] == (root / "web" / "README.md").read_bytes()

published = dict(original)
published["jsr.json"] = (root / "web/jsr.json").read_bytes()
config = json.loads(published["jsr.json"])
assert config["name"] == manifest["name"] and config["version"] == manifest["version"]
assert config["exports"]["."] == "./index.js"
assert config["exports"]["./auth"] == "./src/auth.ts"
assert config["exports"]["./dock"] == "./src/dock.ts"
assert (
    config["exports"]["./components/AccountConnection.svelte"]
    == "./src/components/AccountConnection.svelte"
)
assert config["exports"]["./components/Dock.svelte"] == "./src/components/Dock.svelte"
assert (
    config["exports"]["./components/ChatDock.svelte"]
    == "./src/components/ChatDock.svelte"
)
assert (
    config["exports"]["./components/StepConfig.svelte"]
    == "./src/components/StepConfig.svelte"
)
assert "src/**" in config["publish"]["include"]
# JSR limits the sum of package files to 20 MB. XZ preserves the original tar
# byte for byte while reducing the corresponding source kit below this limit.
vendor_tar = gzip.decompress(published.pop("source/dependencies.tar.gz"))
vendor_xz = lzma.compress(vendor_tar, preset=6)
assert lzma.decompress(vendor_xz) == vendor_tar
published["source/dependencies.tar.xz"] = vendor_xz
# The top-level README stays as the web package README; only the
# corresponding source kit carries vendor rebuild instructions.
for name in ("source/README.md",):
    text = published[name].decode()
    assert "dependencies.tar.gz" in text and "tar -xzf dependencies.tar.gz" in text
    published[name] = text.replace("dependencies.tar.gz", "dependencies.tar.xz").replace(
        "tar -xzf dependencies.tar.xz", "tar -xJf dependencies.tar.xz"
    ).encode()
    for substitution in json.loads((root / ".ci/jsr-readme-replacements.json").read_text()):
        before, after = substitution["original"].encode(), substitution["replacement"].encode()
        assert published[name].count(before) == 1
        published[name] = published[name].replace(before, after)
published["README.md"] = (root / ".ci/jsr-readme.md").read_bytes() + b"\n" + published["README.md"]
assert sum(map(len, published.values())) < 20_000_000
assert max(map(len, published.values())) < 20_000_000
folded = [name.casefold() for name in published]
assert len(folded) == len(set(folded)), "JSR paths must not differ only in casing"

output = Path(os.environ["CARGO_HOME"]) / "ccid-artifacts" / os.environ["CI_COMMIT_SHA"] / "jsr"
output.mkdir(parents=True, exist_ok=True)
origin_archive = output / f"corbet-foss-ccht-{inputs['version']}.tgz"
origin_archive.write_bytes(original_archive)
with tempfile.TemporaryDirectory(prefix="ccht-jsr-", dir=os.environ.get("TMPDIR")) as directory:
    stage = Path(directory) / "package"
    stage.mkdir()
    for name, data in published.items():
        destination = stage / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(data)
    dry_run = subprocess.run(["deno", "publish", "--dry-run", "--allow-dirty"], cwd=stage,
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=180)
    print(dry_run.stdout, end="")
    dry_run.check_returncode()
    # Deno excludes hidden source directories unless explicitly included. Check
    # its actual upload inventory so the offline source kit cannot disappear.
    listed = set(re.findall(re.escape(stage.as_uri() + "/") + r"([^\s]+) \(", dry_run.stdout))
    assert listed == set(published), {"missing": sorted(set(published) - listed),
                                     "unexpected": sorted(listed - set(published))}
    # The same assertions exercise the npm and JSR packages. Resolve the JSR
    # entrypoint through a consumer import map rather than node_modules.
    consumer = Path(directory) / "consumer"
    consumer.mkdir()
    (consumer / "deno.json").write_text(json.dumps({"imports": {
        "@corbet-foss/ccht": "../package/index.js",
        "@corbet-foss/ccht/ccht_bg.wasm": "../package/wasm/ccht_bg.wasm",
    }}))
    shutil.copy2(root / ".ci/web-consumer.mjs", consumer / "check.mjs")
    run("deno", "run", "--allow-read", "check.mjs", cwd=consumer)
    (consumer / "types.ts").write_text("""import { createConversation, type WireEvent, type ConversationSnapshot } from '@corbet-foss/ccht';
const event: WireEvent = { version: 1, conversation_id: 'types', request_id: 'one', sequence: 1,
  event: { kind: 'completed', stop_reason: 'end_turn' } };
const wasm = new URL(import.meta.resolve('@corbet-foss/ccht/ccht_bg.wasm'));
const conversation = await createConversation('types', { wasm });
const snapshot: ConversationSnapshot = conversation.applyEvent(event);
const text: string = snapshot.turns[0].text;
// @ts-expect-error: sequence must remain a number in the published declarations.
const invalid: WireEvent = { ...event, sequence: 'one' };
if (snapshot.conversation_id !== 'types' || snapshot.turns[0].status !== 'completed' || text !== '') {
  throw new Error('Wasm URL loading did not produce the expected conversation');
}
conversation.free();
console.log('Deno Wasm URL loading passed.');
""")
    run("deno", "check", "types.ts", cwd=consumer)
    run("deno", "run", "--allow-read", "types.ts", cwd=consumer)
    archive = output / f"corbet-foss-ccht-{inputs['version']}-jsr.tar.gz"
    with archive.open("wb") as stream:
        with gzip.GzipFile(fileobj=stream, mode="wb", mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as bundle:
                for name, data in sorted(published.items()):
                    info = tarfile.TarInfo(name)
                    info.size, info.mode, info.mtime = len(data), 0o644, 0
                    bundle.addfile(info, io.BytesIO(data))
    assert archive.stat().st_size < 20_000_000
    assert files_from_tar(archive.read_bytes()) == published

transformations = {
    "source/dependencies.tar.xz": {
        "kind": "lossless gzip-to-xz recompression",
        "original_path": "source/dependencies.tar.gz",
        "original_sha256": digest(original["source/dependencies.tar.gz"]),
        "published_sha256": digest(vendor_xz),
        "uncompressed_sha256": digest(vendor_tar),
    },
}
for name in ("README.md", "source/README.md"):
    transformations[name] = {
        "kind": "JSR usage instructions" if name == "README.md" else "source archive extraction and Wasm initialization instructions",
        "original_sha256": digest(original[name]), "published_sha256": digest(published[name]),
    }
receipt = {
    "schema": 1, "package": "ccht", "version": inputs["version"],
    "commit": os.environ["CI_COMMIT_SHA"], "source_sha256": os.environ["SOURCE_SHA256"],
    "check": "jsr-package", "layout": "web-wasm-v1", "tool_revision": os.environ["CCID_REVISION"],
    "artifacts": {archive.name: digest(archive.read_bytes()), origin_archive.name: inputs["archive_sha256"]},
    "original_npm": inputs, "original_npm_artifact": origin_archive.name,
    "published_files": {name: digest(data) for name, data in published.items()},
    "source_transformations": transformations,
    "validation": {"deno_publish_dry_run": "passed", "independent_deno_consumer": "passed",
                   "typescript_consumer": "passed", "wasm_url_loading": "passed",
                   "vendor_tar_bytes_preserved": "passed", "deno_file_inventory": "passed"},
    "unpacked_size": sum(map(len, published.values())), "archive_size": archive.stat().st_size,
    "vendor_tar_size": len(vendor_tar),
    "deno": subprocess.check_output(["deno", "--version"], text=True).strip(),
}
(output / "jsr-package.json").write_text(json.dumps(receipt, indent=2) + "\n")
(output / "SOURCE_COMMIT").write_text(os.environ["CI_COMMIT_SHA"] + "\n")
(output / "SHA256SUMS").write_text("".join(value + "  " + name + "\n" for name, value in receipt["artifacts"].items()))
print(json.dumps({key: value for key, value in receipt.items() if key != "published_files"}))
