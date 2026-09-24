"""Verify the crate archive and consume its public API from an independent crate."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import shutil
import tarfile
import tempfile
import tomllib

from artifact_root import output_directory


metadata = json.loads(
    subprocess.check_output(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"]
    )
)
package = next(
    package
    for package in metadata["packages"]
    if Path(package["manifest_path"]).parent == Path.cwd()
)
if package["license"] != "LGPL-3.0-only WITH LGPL-3.0-linking-exception":
    raise ValueError("The current crate must declare LGPL-3.0-only WITH LGPL-3.0-linking-exception")
required_notices = {
    name: Path(name).read_bytes()
    for name in (
        "LICENSE",
        "LICENSE.md",
        "LICENSES/LGPL-3.0-only.txt",
        "LICENSES/LGPL-3.0-linking-exception.txt",
        "LICENSES/GPL-3.0-only.txt",
    )
}
subprocess.run(["cargo", "package", "--locked"], check=True)
archive = (
    Path(metadata["target_directory"])
    / "package"
    / f"{package['name']}-{package['version']}.crate"
)
with tempfile.TemporaryDirectory(prefix="ccht-package-", dir=os.environ.get("TMPDIR")) as directory:
    root = Path(directory)
    with tarfile.open(archive, "r:gz") as packed:
        packed.extractall(root, filter="data")
    source = root / f"{package['name']}-{package['version']}"
    packaged_metadata = tomllib.loads((source / "Cargo.toml").read_text())["package"]
    if packaged_metadata["license"] != "LGPL-3.0-only WITH LGPL-3.0-linking-exception":
        raise ValueError("Packaged crate license differs from the current grant")
    for name, expected in required_notices.items():
        if (source / name).read_bytes() != expected:
            raise ValueError(f"Packaged license notice differs: {name}")
    packaged_licenses = sorted(
        path.name for path in (source / "LICENSES").iterdir() if path.is_file()
    )
    if packaged_licenses != [
        "GPL-3.0-only.txt",
        "LGPL-3.0-linking-exception.txt",
        "LGPL-3.0-only.txt",
    ]:
        raise ValueError("Packaged license inventory differs from the current grant")
    consumer = root / "consumer"
    (consumer / "src").mkdir(parents=True)
    (consumer / "Cargo.toml").write_text(
        '[package]\nname = "ccht-package-consumer"\nversion = "0.0.0"\nedition = "2024"\n'
        '[dependencies]\nlibrary_under_test = { package = '
        + json.dumps(package["name"])
        + ', path = '
        + json.dumps(str(source))
        + ' }\n'
    )
    (consumer / "src" / "lib.rs").write_text('''#[test]
fn consume_packaged_api() {
    use library_under_test::{Conversation, Event, WireEvent, acp};
    let mut conversation = Conversation::new("independent");
    conversation.apply(WireEvent::new("independent", "turn", 1,
        Event::Completed { stop_reason: acp::StopReason::EndTurn })).unwrap();
    assert_eq!(conversation.turns()[0].status, "completed");
    let _command = library_under_test::native::AgentCommand::new("installed-agent");
}
''')
    subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=consumer, check=True)
    subprocess.run(["cargo", "test", "--locked", "--offline"], cwd=consumer, check=True)
report = {
    "source_commit": os.environ["CI_COMMIT_SHA"],
    "package": package["name"],
    "version": package["version"],
    "license": package["license"],
    "archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
    "source_lock_sha256": hashlib.sha256(Path("Cargo.lock").read_bytes()).hexdigest(),
    "notices": {
        name: hashlib.sha256(contents).hexdigest()
        for name, contents in required_notices.items()
    },
}
output = output_directory()
shutil.copy2(archive, output / archive.name)
(output / "package-license.json").write_text(json.dumps(report, indent=2) + "\n")
release_receipt = {"schema": 1, "package": package["name"], "version": package["version"],
                   "commit": os.environ["CI_COMMIT_SHA"], "source_sha256": os.environ["SOURCE_SHA256"],
                   "check": "rust-package", "tool_revision": os.environ.get("CCID_REVISION"),
                   "artifacts": {archive.name: report["archive_sha256"]}, "independent_consumer": "passed"}
(output / "rust-package.json").write_text(json.dumps(release_receipt, indent=2) + "\n")
(output / "SOURCE_COMMIT").write_text(os.environ["CI_COMMIT_SHA"] + "\n")
(output / "SHA256SUMS").write_text(report["archive_sha256"] + "  " + archive.name + "\n")
print(json.dumps(report))
print(f"Verified packaged crate and independent consumer: {archive}")
