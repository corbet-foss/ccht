"""Build and consume the Python binding and its complete LGPL source kit on CI."""

import argparse
import base64
import csv
import email.parser
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
import zipfile

from artifact_root import output_directory


def run(*args, **kwargs):
    print("+ " + " ".join(map(str, args)), flush=True)
    return subprocess.run(list(map(str, args)), check=True, **kwargs)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def preserve(path, data):
    try:
        with path.open("xb") as stream:
            stream.write(data)
    except FileExistsError:
        if path.read_bytes() != data:
            raise ValueError(f"Existing artifact differs: {path.name}") from None


def repair_nix_rpath(wheel, scratch):
    """Remove build-host search paths, preserving maturin's audited ABI tag."""
    files, tags, extension = verify_wheel(wheel)
    if not extension.endswith(".so"):
        return None
    scratch.mkdir()
    library = scratch / "inspect.so"
    library.write_bytes(files[extension])
    rpath = subprocess.check_output(["patchelf", "--print-rpath", str(library)], text=True).strip()
    if not rpath:
        return None
    assert all(part.startswith("/nix/store/") for part in rpath.split(":")), rpath
    # Use the standard wheel tool to regenerate RECORD after this one ELF edit.
    wheel_tool = ["uv", "run", "--no-project", "--no-python-downloads", "--python", shutil.which("python3"),
                  "--with", "wheel==0.48.0", "--with", "packaging==26.3", "python", "-m", "wheel"]
    unpacked = scratch / "unpacked"
    run(*wheel_tool, "unpack", "--dest", unpacked, wheel)
    directory = next(unpacked.iterdir())
    run("patchelf", "--remove-rpath", directory / extension)
    repaired = scratch / "repaired"
    repaired.mkdir()
    run(*wheel_tool, "pack", "--dest-dir", repaired, directory)
    replacement = repaired / wheel.name
    changed, new_tags, new_extension = verify_wheel(replacement)
    record = next(name for name in files if name.endswith(".dist-info/RECORD"))
    assert set(files) == set(changed) and tags == new_tags and extension == new_extension
    assert all(files[name] == changed[name] for name in files if name not in {extension, record})
    repair = {"kind": "remove Nix build-host RPATH", "original_sha256": digest(files[extension]),
              "repaired_sha256": digest(changed[extension]), "wheel_tool": "0.48.0", "packaging": "26.3"}
    shutil.copy2(replacement, wheel)
    return repair


def verify_wheel(path):
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        assert len(names) == len(set(names))
        files = {name: archive.read(name) for name in names if not name.endswith("/")}
    records = [name for name in files if name.endswith(".dist-info/RECORD")]
    assert len(records) == 1
    rows = list(csv.reader(io.StringIO(files[records[0]].decode())))
    assert {row[0] for row in rows} == set(files)
    for name, checksum, size in rows:
        if name == records[0]:
            assert checksum == size == ""
        else:
            encoded = base64.urlsafe_b64encode(hashlib.sha256(files[name]).digest()).rstrip(b"=").decode()
            assert checksum == "sha256=" + encoded and int(size) == len(files[name])
    wheel_name = records[0].removesuffix("RECORD") + "WHEEL"
    wheel = email.parser.Parser().parsestr(files[wheel_name].decode())
    assert wheel["Root-Is-Purelib"] == "false"
    extensions = [name for name in files if name.startswith("ccht/_native.") and name.endswith((".so", ".pyd"))]
    assert len(extensions) == 1
    return files, wheel.get_all("Tag"), extensions[0]


parser = argparse.ArgumentParser()
parser.add_argument("--resolve-only", action="store_true")
args = parser.parse_args()
root = Path.cwd()
source = root / "py"
os.environ["MATURIN_NO_INSTALL_RUST"] = "1"
project = tomllib.loads((source / "pyproject.toml").read_text())["project"]
version = project["version"]
assert project["name"] == "ccht" and project["license"] == "LGPL-3.0-only WITH LGPL-3.0-linking-exception"
output = output_directory("python")
if args.resolve_only:
    run("cargo", "generate-lockfile", "--manifest-path", source / "Cargo.toml")
    lock = (source / "Cargo.lock").read_bytes()
    preserve(output / "Cargo.lock", lock)
    print(json.dumps({"lock_sha256": digest(lock), "output": str(output)}))
    raise SystemExit(0)

run("cargo", "fmt", "--manifest-path", source / "Cargo.toml", "--check")
run("cargo", "clippy", "--locked", "--manifest-path", source / "Cargo.toml", "--all-targets", "--", "-D", "warnings")
run("cargo", "test", "--locked", "--manifest-path", source / "Cargo.toml")
run("cargo", "fetch", "--locked", "--manifest-path", source / "Cargo.toml")
with tempfile.TemporaryDirectory(prefix="ccht-python-", dir=os.environ.get("TMPDIR")) as directory:
    scratch = Path(directory)
    stage = scratch / "package"
    shutil.copytree(source, stage, ignore=shutil.ignore_patterns("target", "dist", "__pycache__"))
    source_map = {str(path.relative_to(stage)): "py/" + str(path.relative_to(stage))
                  for path in stage.rglob("*") if path.is_file()}
    shutil.copy2(root / "THIRD-PARTY.md", stage / "THIRD-PARTY.md")
    source_map["THIRD-PARTY.md"] = "THIRD-PARTY.md"
    licenses = stage / "LICENSES"
    licenses.mkdir(exist_ok=True)
    for license_path in sorted((root / "LICENSES").iterdir()):
        assert license_path.is_file()
        name = license_path.name
        shutil.copy2(license_path, licenses / name)
        source_map["LICENSES/" + name] = "LICENSES/" + name
    vendor = stage / "vendor"
    config = subprocess.check_output(["cargo", "vendor", "--locked", "--offline", "--versioned-dirs", str(vendor)], cwd=stage, text=True)
    (stage / ".cargo").mkdir()
    (stage / ".cargo/config.toml").write_text(config.replace(str(vendor), "vendor"))
    for dependency in sorted(vendor.iterdir()):
        metadata = tomllib.loads((dependency / "Cargo.toml").read_text())["package"]
        assert metadata.get("license"), f"Missing license metadata: {dependency.name}"
        notices = [path for path in dependency.iterdir() if path.name.upper().startswith(
            ("LICENSE", "LICENCE", "COPYING", "COPYRIGHT", "NOTICE", "UNLICENSE", "AUTHORS"))]
        if not notices:
            fallback = root / ".ci/upstream-notices" / f"{metadata['name']}-{metadata['version']}"
            notices = list(fallback.iterdir()) if fallback.is_dir() else []
        assert notices, f"Missing redistribution notices: {dependency.name}"
        destination = licenses / "dependencies" / dependency.name
        destination.mkdir(parents=True)
        for notice in notices:
            if notice.is_dir():
                shutil.copytree(notice, destination / notice.name)
            else:
                shutil.copy2(notice, destination / notice.name)
    # Build source first, then include those exact bytes alongside the extension.
    dist = scratch / "dist"
    run("uv", "build", "--sdist", "--out-dir", dist, stage)
    sdists = list(dist.glob("*.tar.gz"))
    assert len(sdists) == 1
    sdist = sdists[0]
    source_delivery = stage / "ccht/source"
    source_delivery.mkdir()
    shutil.copy2(sdist, source_delivery / sdist.name)
    # Maturin's PEP 517 backend otherwise overrides its config with
    # --compatibility off. Request the audited PyPI tag explicitly.
    host = next(line.split(": ", 1)[1] for line in subprocess.check_output(["rustc", "-vV"], text=True).splitlines()
                if line.startswith("host: "))
    compatibility = "manylinux_2_34" if host == "x86_64-unknown-linux-gnu" else "pypi"
    wheel_args = ["--config-setting", f"build-args=--target {host} --compatibility {compatibility}"]
    run("uv", "build", "--wheel", *wheel_args, "--out-dir", dist, stage)
    wheels = list(dist.glob("*.whl"))
    assert len(wheels) == 1
    wheel = wheels[0]
    repair = repair_nix_rpath(wheel, scratch / "wheel-repair")
    wheel_files, tags, extension = verify_wheel(wheel)
    assert wheel_files["ccht/source/" + sdist.name] == sdist.read_bytes()
    with tarfile.open(sdist) as archive:
        members = [item for item in archive.getmembers() if item.isfile()]
        assert len(members) == len({item.name for item in members})
        prefix = f"ccht-{version}/"
        assert all(item.name.startswith(prefix) for item in members)
        sdist_files = {item.name.removeprefix(prefix): archive.extractfile(item).read() for item in members}
    for name, original in source_map.items():
        assert sdist_files[name] == (root / original).read_bytes(), name
    assert any(name.startswith(f"vendor/ccht-{version}/src/") for name in sdist_files)
    # The installed wheel and rebuilt source are consumed outside this checkout.
    consumer_script = scratch / "consumer.py"
    shutil.copy2(root / ".ci/python-consumer.py", consumer_script)
    venv = scratch / "consumer"
    run("uv", "venv", "--no-python-downloads", "--python", shutil.which("python3"), venv)
    python = venv / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
    run("uv", "pip", "install", "--no-deps", "--offline", "--python", python, wheel)
    run(python, consumer_script, cwd=scratch)
    extension_path = scratch / Path(extension).name
    extension_path.write_bytes(wheel_files[extension])
    if os.name == "posix" and shutil.which("patchelf"):
        rpath = subprocess.check_output(["patchelf", "--print-rpath", str(extension_path)], text=True).strip()
        assert "/nix/store/" not in rpath, f"Non-portable runtime search path: {rpath}"
        needed = subprocess.check_output(["patchelf", "--print-needed", str(extension_path)], text=True).splitlines()
        assert not any("/" in entry for entry in needed), needed
        assert set(needed) <= {"libc.so.6", "libm.so.6", "libgcc_s.so.1", "libpthread.so.0", "libdl.so.2", "librt.so.1", "ld-linux-x86-64.so.2"}, needed
        assert not any(".libs/" in name for name in wheel_files), "Unexpected bundled shared library needs a license/source review"
    else:
        rpath, needed = None, None
    rebuild = scratch / "rebuild"
    rebuild.mkdir()
    with tarfile.open(sdist) as archive:
        archive.extractall(rebuild, filter="data")
    clean_env = dict(os.environ, CARGO_HOME=str(scratch / "empty-cargo-home"),
                     CARGO_TARGET_DIR=str(scratch / "rebuild-target"), CARGO_NET_OFFLINE="true")
    rebuilt_dist = scratch / "rebuilt-dist"
    run("uv", "build", "--offline", "--wheel", *wheel_args, "--out-dir", rebuilt_dist, rebuild / prefix, env=clean_env)
    rebuilt = list(rebuilt_dist.glob("*.whl"))
    assert len(rebuilt) == 1
    repair_nix_rpath(rebuilt[0], scratch / "rebuilt-wheel-repair")
    run("uv", "pip", "install", "--no-deps", "--offline", "--reinstall", "--python", python, rebuilt[0])
    run(python, consumer_script, cwd=scratch)
    artifacts = {path.name: digest(path.read_bytes()) for path in (wheel, sdist)}
    receipt = {
        "schema": 1, "package": "ccht", "version": version, "check": "python-package",
        "commit": os.environ["CI_COMMIT_SHA"], "source_sha256": os.environ.get("SOURCE_SHA256"),
        "tool_revision": os.environ.get("CCID_REVISION"), "artifacts": artifacts,
        "layout": "maturin-v1",
        "native_wheel": {"filename": wheel.name, "tags": tags, "extension": extension,
                         "extension_sha256": digest(wheel_files[extension]), "rpath": rpath, "needed": needed},
        "wheel_repair": repair,
        "wheel_files": {name: digest(data) for name, data in wheel_files.items()},
        "sdist_files": {name: digest(data) for name, data in sdist_files.items()},
        "sdist_source_files": source_map,
        "validation": {"installed_consumer": True, "offline_source_rebuild": True,
                       "rebuilt_consumer": True, "cargo_lock_sha256": digest((source / "Cargo.lock").read_bytes())},
    }
    for path in (wheel, sdist):
        preserve(output / path.name, path.read_bytes())
    preserve(output / "python-package.json", (json.dumps(receipt, indent=2, sort_keys=True) + "\n").encode())
    preserve(output / "SOURCE_COMMIT", (os.environ["CI_COMMIT_SHA"] + "\n").encode())
    preserve(output / "SHA256SUMS", "".join(f"{value}  {name}\n" for name, value in sorted(artifacts.items())).encode())
    print(json.dumps({"output": str(output), "artifacts": artifacts, "native_wheel": receipt["native_wheel"]}))
