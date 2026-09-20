"""Build and verify the published Wasm package using the official Rust generator."""

import hashlib
import gzip
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import tomllib


def run(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def metadata(*args):
    return json.loads(subprocess.check_output([
        "cargo", "metadata", "--locked", "--format-version", "1", *args
    ]))


root = Path.cwd()
meta = metadata("--no-default-features", "--features", "web", "--filter-platform", "wasm32-unknown-unknown")
package = next(p for p in meta["packages"] if Path(p["manifest_path"]).parent == root)
target = Path(meta["target_directory"])
# Nix's rustc may ship its linker outside PATH. Use this compiler's own
# installed linker; never download or install a tool during a package check.
linker_key = "CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER"
if linker_key not in os.environ and not shutil.which("lld"):
    sysroot = Path(subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip())
    host = next(line.split(": ", 1)[1] for line in subprocess.check_output(["rustc", "-vV"], text=True).splitlines() if line.startswith("host: "))
    linker = sysroot / "lib/rustlib" / host / "bin" / ("rust-lld.exe" if os.name == "nt" else "rust-lld")
    if not linker.is_file():
        llvm = next(line.split(": ", 1)[1] for line in subprocess.check_output(["rustc", "-vV"], text=True).splitlines() if line.startswith("LLVM version: "))
        # Nix's source-built compiler keeps LLD in a separate store output.
        # Probe only already installed linkers matching this compiler's LLVM.
        for candidate in sorted(Path("/nix/store").glob(f"*-lld-{llvm}/bin/lld")):
            try:
                probe = subprocess.run([str(candidate), "-flavor", "wasm", "--version"], capture_output=True, text=True)
                if probe.returncode == 0:
                    linker = candidate
                    print(f"Using installed Wasm linker: {linker} ({probe.stdout.strip()})")
                    break
            except OSError:
                continue
        else:
            raise ValueError("Wasm linker unavailable; configure an already installed linker explicitly")
    os.environ[linker_key] = str(linker)
build = subprocess.run(["cargo", "build", "--locked", "--release", "--no-default-features", "--features", "web", "--target", "wasm32-unknown-unknown", "--message-format=json"], stdout=subprocess.PIPE, text=True)
if build.returncode:
    print(build.stdout)
    build.check_returncode()
built_packages = {entry["package_id"] for line in build.stdout.splitlines()
                  if (entry := json.loads(line)).get("reason") == "compiler-artifact"}

output = Path(os.environ["CARGO_HOME"]) / "ccid-artifacts" / os.environ["CI_COMMIT_SHA"] / "web"
output.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory(prefix="ccht-web-", dir=os.environ.get("TMPDIR")) as directory:
    stage = Path(directory) / "package"
    shutil.copytree(root / "web", stage, ignore=shutil.ignore_patterns("wasm", "source", "node_modules", "*.tgz"))
    run("cargo", "run", "--locked", "--release", "-p", "ccht-wasm-bundle", "--", str(target / "wasm32-unknown-unknown/release/ccht.wasm"), str(stage / "wasm"))
    # Keep the web README as the npm README; the repo-root README travels as
    # source/README.md below for rebuild documentation.
    for name in ("LICENSE", "LICENSE.md", "THIRD-PARTY.md"):
        shutil.copy2(root / name, stage / name)
    assert (stage / "README.md").read_bytes() == (root / "web" / "README.md").read_bytes()
    manifest_path = stage / "package.json"
    staged_manifest = json.loads(manifest_path.read_text())
    staged_manifest["gitHead"] = os.environ["CI_COMMIT_SHA"]
    manifest_path.write_text(json.dumps(staged_manifest, indent=2) + "\n")
    (stage / "LICENSES").mkdir(exist_ok=True)
    for name in ("LGPL-3.0-only.txt", "LGPL-3.0-only WITH LGPL-3.0-linking-exception.txt", "LGPL-3.0-linking-exception.txt", "GPL-3.0-only.txt"):
        shutil.copy2(root / "LICENSES" / name, stage / "LICENSES" / name)
    # Exact corresponding Rust source and generator inputs accompany the binary.
    source = stage / "source"
    source.mkdir()
    for name in ("Cargo.toml", "Cargo.lock", "README.md", "LICENSE", "LICENSE.md", "THIRD-PARTY.md", "CHANGELOG.md"):
        shutil.copy2(root / name, source / name)
    for name in ("src", "examples", ".ci/wasm-bundle", ".ci/upstream-notices"):
        shutil.copytree(root / name, source / name)
    shutil.copytree(stage / "LICENSES", source / "LICENSES")
    # Preserve the full locked source closure. A tar member avoids npm's
    # per-file exclusions changing Cargo's checksum-protected vendor contents.
    vendor = Path(directory) / "vendor"
    vendor_config = subprocess.check_output(["cargo", "vendor", "--locked", "--offline", "--versioned-dirs", str(vendor)], text=True)
    cargo_config = source / ".cargo/config.toml"
    cargo_config.parent.mkdir()
    cargo_config.write_text(vendor_config.replace(str(vendor), "vendor"))
    vendor_sources = sorted(path for path in vendor.iterdir() if path.is_dir())
    def archive_metadata(entry):
        entry.uid = entry.gid = entry.mtime = 0
        entry.uname = entry.gname = ""
        return entry
    with (source / "dependencies.tar.gz").open("wb") as file:
        with gzip.GzipFile(fileobj=file, mode="wb", mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w") as bundle:
                bundle.add(vendor, arcname="vendor", filter=archive_metadata)
    for location in vendor_sources:
        dependency = tomllib.loads((location / "Cargo.toml").read_text())["package"]
        destination = stage / "LICENSES/dependencies" / f"{dependency['name']}-{dependency['version']}"
        notices = [p for p in location.iterdir() if p.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "COPYRIGHT", "NOTICE", "UNLICENSE", "AUTHORS"))]
        if not notices:
            # Some upstream crate archives omit their own root notice. Reviewed,
            # version-specific originals accompany our build tooling instead.
            fallback = root / ".ci/upstream-notices" / f"{dependency['name']}-{dependency['version']}"
            notices = list(fallback.iterdir()) if fallback.is_dir() else []
        if not notices:
            raise ValueError(f"Missing redistribution notices for {dependency['name']}")
        destination.mkdir(parents=True)
        for notice in notices:
            if notice.is_dir():
                shutil.copytree(notice, destination / notice.name)
            else:
                shutil.copy2(notice, destination / notice.name)
    pack_output = json.loads(subprocess.check_output(["npm", "pack", "--json", "--ignore-scripts", "--pack-destination", str(output)], cwd=stage))
    # npm v11 packs to an object keyed by package name; older npm emitted an array.
    if isinstance(pack_output, list):
        packed = pack_output[0]
    else:
        staged_name = json.loads((stage / "package.json").read_text())["name"]
        packed = pack_output[staged_name]
    archive = output / packed["filename"]
    consumer = Path(directory) / "consumer"
    module = consumer / "node_modules/@corbet-labs/ccht"
    module.mkdir(parents=True)
    with tarfile.open(archive, "r:gz") as bundle:
        for entry in bundle.getmembers():
            if not entry.name.startswith("package/"):
                raise ValueError("Unexpected npm archive prefix")
            entry.name = entry.name.removeprefix("package/")
            if entry.name:
                bundle.extract(entry, module, filter="data")
    manifest = json.loads((module / "package.json").read_text())
    assert manifest["name"] == "@corbet-labs/ccht"
    assert manifest["license"] == "LGPL-3.0-only WITH LGPL-3.0-linking-exception"
    assert manifest["version"] == package["version"]
    for name in ("LICENSE", "LICENSES/LGPL-3.0-only.txt", "LICENSES/LGPL-3.0-only WITH LGPL-3.0-linking-exception.txt", "LICENSES/LGPL-3.0-linking-exception.txt", "LICENSES/GPL-3.0-only.txt"):
        assert (module / name).read_bytes() == (root / name).read_bytes()
    assert (module / "source/src/conversation.rs").read_bytes() == (root / "src/conversation.rs").read_bytes()
    assert (module / "source/.ci/wasm-bundle/src/main.rs").is_file()
    shutil.copy2(root / ".ci/web-consumer.mjs", consumer / "check.mjs")
    run("node", "check.mjs", cwd=consumer)
    wasm_sha256 = hashlib.sha256((module / "wasm/ccht_bg.wasm").read_bytes()).hexdigest()
    # Prove the published kit can replace the shipped module with no registry
    # cache or network. Also rebuild the official binding tool from that kit.
    with tarfile.open(module / "source/dependencies.tar.gz", "r:gz") as bundle:
        bundle.extractall(module / "source", filter="data")
    clean_target = Path(directory) / "rebuild-target"
    clean_env = dict(os.environ, CARGO_HOME=str(Path(directory) / "empty-cargo-home"), CARGO_TARGET_DIR=str(clean_target))
    run("cargo", "build", "--locked", "--offline", "--release", "--no-default-features", "--features", "web", "--target", "wasm32-unknown-unknown", cwd=module / "source", env=clean_env)
    run("cargo", "run", "--locked", "--offline", "--release", "-p", "ccht-wasm-bundle", "--", str(clean_target / "wasm32-unknown-unknown/release/ccht.wasm"), str(module / "wasm"), cwd=module / "source", env=clean_env)
    run("node", "check.mjs", cwd=consumer)
    report = {
        "source_commit": os.environ["CI_COMMIT_SHA"],
        "source_lock_sha256": hashlib.sha256((root / "Cargo.lock").read_bytes()).hexdigest(),
        "package": manifest["name"], "version": manifest["version"],
        "license": manifest["license"], "archive": archive.name,
        "archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
        "wasm_sha256": wasm_sha256,
        "rebuilt_wasm_sha256": hashlib.sha256((module / "wasm/ccht_bg.wasm").read_bytes()).hexdigest(),
        "offline_source_rebuild": "passed",
        "vendored_packages": len(vendor_sources),
        "dependency_notices": len(vendor_sources), "compiled_packages": len(built_packages) - 1, "independent_consumer": "passed",
    }
    (output / "receipt.json").write_text(json.dumps(report, indent=2) + "\n")
    release_receipt = {"schema": 1, "package": package["name"], "version": package["version"],
                       "commit": os.environ["CI_COMMIT_SHA"], "source_sha256": os.environ["SOURCE_SHA256"],
                       "check": "js-package", "tool_revision": os.environ["CCID_REVISION"],
                       "artifacts": {archive.name: report["archive_sha256"]},
                       "offline_source_rebuild": "passed", "independent_consumer": "passed"}
    (output / "js-package.json").write_text(json.dumps(release_receipt, indent=2) + "\n")
    (output / "SOURCE_COMMIT").write_text(os.environ["CI_COMMIT_SHA"] + "\n")
    (output / "SHA256SUMS").write_text(report["archive_sha256"] + "  " + archive.name + "\n")
    print(json.dumps(report))
