#!/usr/bin/env python3
"""Invoke the exact reviewed shared publisher; preparation remains separate."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import sys
import tarfile
import tempfile

REPOSITORY = "corbet-foss/ccht"
LIMIT = 128 * 1024 * 1024


def require(condition, message):
    if not condition:
        raise ValueError(message)


def main():
    revision = os.environ.get("CCID_REVISION", "")
    require(re.fullmatch(r"[0-9a-f]{40}", revision), "A reviewed shared publisher revision is required")
    archive_path = Path(os.environ["CI_TOOL_ARCHIVE"])
    require(archive_path.is_file() and not archive_path.is_symlink() and archive_path.stat().st_size <= LIMIT,
            "Expected a bounded regular shared source archive")
    archive_hash = hashlib.sha256(archive_path.read_bytes()).hexdigest()
    require(archive_hash == os.environ.get("CI_TOOL_SHA256"), "Shared publisher source checksum mismatch")
    with tarfile.open(archive_path, "r:*") as archive:
        require(archive.pax_headers.get("comment", "").strip() == revision,
                "Shared publisher archive revision mismatch")
        entries = [item for item in archive.getmembers() if item.name == "adapters/registry_publish.py"]
        require(len(entries) == 1 and entries[0].isfile() and entries[0].size <= 1024 * 1024,
                "Shared publisher resource is missing or ambiguous")
        module_bytes = archive.extractfile(entries[0]).read()
    if not os.environ.get("RELEASE_JOURNAL_ROOT") and os.environ.get("CARGO_TARGET_DIR"):
        target = Path(os.environ["CARGO_TARGET_DIR"])
        require(target.is_absolute(), "The persistent Cargo target must be absolute")
        os.environ["RELEASE_JOURNAL_ROOT"] = str(target / "publication")
    print(json.dumps({"publisher_revision": revision, "publisher_source_sha256": archive_hash}), file=sys.stderr)
    with tempfile.TemporaryDirectory(prefix="release-publisher-", dir=os.environ.get("TMPDIR")) as temporary:
        path = Path(temporary) / "registry_publish.py"
        path.write_bytes(module_bytes)
        spec = importlib.util.spec_from_file_location("ccid_registry_publish", path)
        module = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = module
        spec.loader.exec_module(module)
        arguments = sys.argv[1:] or [os.environ.get("RELEASE_OPERATION", "status")]
        return module.main(arguments, repository=REPOSITORY)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, OSError, KeyError, tarfile.TarError) as error:
        # Do not include request URLs, command arguments or authentication values.
        print(str(error) if isinstance(error, ValueError) else "Publisher setup failed (" + type(error).__name__ + ")", file=sys.stderr)
        sys.exit(1)
