#!/usr/bin/env python3
"""Check the release adapter contract without builds, network or secrets."""
import ast
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import textwrap
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def require(condition, message):
    if not condition:
        raise SystemExit(message)


def check_shell_blocks(source, path):
    lines = source.splitlines()
    for index, line in enumerate(lines):
        match = re.fullmatch(r"( +)(?:run: \||- \|)", line)
        if not match:
            continue
        body = []
        for candidate in lines[index + 1:]:
            if candidate.strip() and len(candidate) - len(candidate.lstrip()) <= len(match[1]):
                break
            body.append(candidate)
        script = textwrap.dedent("\n".join(body)) + "\n"
        subprocess.run(["bash", "-n"], input=script, text=True, check=True)
        for embedded in re.findall(r"python3 - <<'PYTHON'\n(.*?)^PYTHON$", script, re.M | re.S):
            ast.parse(embedded, filename=path + ":python-heredoc")


def check_crow_secrets(source, package):
    sections = re.split(r"^  - name: ", source, flags=re.M)
    require("from_secret:" not in sections[0], "Shared release configuration must have no credentials")
    steps = dict(section.split("\n", 1) for section in sections[1:])
    expected = {"validate-release-selection", "inspect-prepared-packages", "release-status",
                "publish-cargo", "publish-npm", "publish-jsr", "publish-pypi"}
    require(len(sections) == len(expected) + 1 and set(steps) == expected, "Unexpected release credential boundaries")
    require("when:" not in steps["validate-release-selection"], "Release input validation must always run")
    for name, block in steps.items():
        secrets = re.findall(r"from_secret:\s*([A-Za-z0-9_]+)", block)
        expected_secrets = []
        if name == "release-status" or name.startswith("publish-"):
            expected_secrets.append(package + "_release_github_token")
        if name.startswith("publish-"):
            channel = name.removeprefix("publish-")
            expected_secrets.append(package + "_release_" + channel + "_token")
            require('RELEASE_OPERATION == "publish"' in block and 'RELEASE_CHANNELS == "' + channel + '"' in block,
                    "Publication credentials require compile-time operation/channel guards")
        elif name in {"inspect-prepared-packages", "release-status"}:
            operation = "inspect" if name == "inspect-prepared-packages" else "status"
            require('RELEASE_OPERATION == "' + operation + '"' in block, "Inspection/status operation guard is missing")
        require(sorted(secrets) == sorted(expected_secrets), "Step has unexpected or missing release credentials: " + name)
        if name != "validate-release-selection":
            require("when:" in block and "evaluate:" in block and "validate-release-selection" in block,
                    "Release executor must be compile-time guarded and depend on input validation")
            require(re.search(r"[&*]publisher-environment\b", block) and re.search(r"[&*]publisher-commands\b", block),
                    "Release executors must reuse the same verified publisher identity and commands")


def workflow_jobs(source):
    """Split a workflow's top-level jobs into {name: block} without a YAML parser."""
    body = source.split("\njobs:\n", 1)[1]
    parts = re.split(r"^  ([A-Za-z0-9_-]+):\n", body, flags=re.M)
    return dict(zip(parts[1::2], parts[2::2]))


def main():
    paths = [".ci/publish.py", ".ci/release-check.py", ".ci/ccid.toml", ".crow/release.yaml",
             ".github/workflows/release.yml", ".github/workflows/ci.yml"]
    files = {path: (ROOT / path).read_text() for path in paths}
    package = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["name"]
    repository = "corbet-foss/" + package
    adapter = ast.parse(files[".ci/publish.py"], filename=".ci/publish.py")
    ast.parse(files[".ci/release-check.py"], filename=".ci/release-check.py")
    assignments = {node.targets[0].id: ast.literal_eval(node.value) for node in adapter.body
                   if isinstance(node, ast.Assign) and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name)
                   and node.targets[0].id == "REPOSITORY"}
    require(assignments.get("REPOSITORY") == repository, "Publication adapter repository mismatch")
    require('"adapters/registry_publish.py"' in files[".ci/publish.py"], "Shared verified publisher resource is required")
    config = tomllib.loads(files[".ci/ccid.toml"])
    for selector, command in (("release", ".ci/publish.py"), ("release-config", ".ci/release-check.py")):
        require(config["checks"][selector] == {"kind": "commands", "commands": [["python3", command]]},
                "Unexpected manual publication selector command")
    crow, hosted = files[".crow/release.yaml"], files[".github/workflows/release.yml"]
    pins = [re.findall(r"^\s+CCID_REVISION:\s*['\"]?([0-9a-f]{40})['\"]?\s*$", value, re.M) for value in (crow, hosted)]
    require(len(pins[0]) == 1 and pins[0] == pins[1], "Release routes must pin the same reviewed publisher revision")
    require(hosted.count("repository: corbet-libs/ccid\n") == hosted.count("ref: ${{ env.CCID_REVISION }}\n") >= 1
            and "ref: " not in hosted.replace("ref: ${{ env.CCID_REVISION }}", ""),
            "Hosted checkout differs from the pinned publisher")
    require("event: manual" in crow and "branch: main" in crow and "--check release" in crow,
            "Crow publication must remain a manual main-branch release command")
    # One pushed vX.Y.Z tag is the release trigger; dispatch re-runs or rehearses.
    require(re.search(r"^on:\n  push:\n    tags: \['v\*\.\*\.\*'\]\n  workflow_dispatch:\n", hosted, re.M)
            and "branches:" not in hosted and "pull_request" not in hosted
            and "github.event.repository.private == false" in hosted, "Hosted release requires public tagged source")
    require("secrets." not in hosted and all(name not in hosted for name in ("NPM_TOKEN", "JSR_TOKEN", "PYPI_TOKEN")),
            "Long-lived registry secrets belong only to the Crow publication step")
    jobs = workflow_jobs(hosted)
    require(set(jobs) == {"prepare", "bundle", "publish"}, "Unexpected hosted release jobs")
    require("uses: ./.github/workflows/ci.yml\n" in jobs["prepare"] and not re.search(r":\s*write\b", jobs["prepare"])
            and "id-token" not in jobs["bundle"] and "contents: write" in jobs["bundle"],
            "Preparation and bundling must run without registry credentials")
    require("id-token: write" in jobs["publish"] and hosted.count("id-token") == 1,
            "Only the publication job may request OIDC tokens")
    # ccht releases one version in two phases: the tag bundles Cargo and npm; the
    # completion bundles JSR and/or PyPI, whose inputs need those uploads first.
    require("'release-config,jsr-package,python-package'" in jobs["prepare"]
            and "'release-config,python-package'" in jobs["prepare"]
            and "'release-config,rust,core,licenses,rust-package,js-package'" in jobs["prepare"]
            and "split(',')) if complete else ('cargo', 'npm')" in jobs["bundle"]
            and "<= {'jsr', 'pypi'}" in jobs["bundle"],
            "Unexpected release phase selection")
    publish = jobs["publish"]
    require("RELEASE_CARGO_AUTH: trusted" in publish and "steps.authentication.outputs.token" in publish
            and "RELEASE_JSR_AUTH: trusted" in publish, "Hosted publication must use the Cargo and JSR OIDC routes")
    # npm and PyPI stay operator uploads until their trusted publisher rules exist.
    require(all(name not in hosted for name in ("RELEASE_NPM_AUTH", "RELEASE_PYPI_AUTH")),
            "npm and PyPI have no trusted publisher rule yet")
    require(publish.count("python3 .ci/publish.py publish") == 2
            and all("RELEASE_CHANNELS: " + channel + "\n" in publish for channel in ("cargo", "jsr")),
            "Each hosted upload must select exactly one OIDC registry")
    require("from_secret: " + package + "_release_pypi_token" in crow, "Crow PyPI route is missing")
    check_crow_secrets(crow, package)
    require("rustup" not in crow and "cargo build" not in crow and "cargo test" not in crow,
            "Credentialed Crow publication must not build products")
    checks = files[".github/workflows/ci.yml"]
    require("'release'" in checks and "release-config) python3 .ci/release-check.py" in checks,
            "Hosted selected checks must route release configuration separately from publication")
    require("workflow_call:" in checks and "secrets" not in checks and not re.search(r":\s*write\b", checks),
            "Selected checks must stay a credential-free reusable preparation workflow")
    for path in (".crow/release.yaml", ".github/workflows/release.yml", ".github/workflows/ci.yml"):
        source = files[path]
        # Only release.yml's tag trigger may start work automatically.
        automatic = re.findall(r"^ +(?:push|pull_request\w*|schedule):", source, re.M)
        require(automatic == (["  push:"] if path == ".github/workflows/release.yml" else []),
                "Only a pushed release tag may start an adapter automatically")
        for action in re.findall(r"^\s+uses:\s*(\S+)", source, re.M):
            require(action == "./.github/workflows/ci.yml" or re.fullmatch(r"[^@\s]+@[0-9a-f]{40}", action),
                    "Hosted actions require exact revisions")
        check_shell_blocks(source, path)
    actionlint = shutil.which("actionlint")
    if not actionlint:
        candidates = sorted(Path("/nix/store").glob("*-actionlint-*/bin/actionlint"))
        actionlint = str(candidates[-1]) if candidates else None
    require(actionlint is not None, "Actionlint is required; use the provisioned Crow tool or hosted setup")
    version = subprocess.check_output([actionlint, "-version"], text=True).strip()
    subprocess.run([actionlint, ".github/workflows/ci.yml", ".github/workflows/release.yml"], cwd=ROOT, check=True)
    print(json.dumps({"repository": repository, "publisher_revision": pins[0][0],
                      "checks": ["python syntax", "shell syntax", "release contract"],
                      "actionlint": {"path": actionlint, "version": version, "status": "passed"},
                      "files": {path: hashlib.sha256(value.encode()).hexdigest() for path, value in files.items()}}, sort_keys=True))


if __name__ == "__main__":
    main()
