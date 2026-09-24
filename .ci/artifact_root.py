"""Locate the directory that receives one check's exported packages and receipts."""

import os
from pathlib import Path
import re


def output_directory(*parts):
    """Return `<ARTIFACT_ROOT>/ccht/<commit>/<parts>`.

    Hosted selected checks and the release workflow supply an absolute
    `ARTIFACT_ROOT`. Without it, Crow's persistent Cargo home keeps the
    previous `ccid-artifacts/<commit>` location.
    """
    commit = os.environ.get("CI_COMMIT_SHA", "")
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("Package checks require the exact CI_COMMIT_SHA")
    base = os.environ.get("ARTIFACT_ROOT")
    if base:
        if not Path(base).is_absolute():
            raise ValueError("ARTIFACT_ROOT must be an absolute path")
        directory = Path(base) / "ccht" / commit
    else:
        directory = Path(os.environ["CARGO_HOME"]) / "ccid-artifacts" / commit
    directory = directory.joinpath(*parts)
    directory.mkdir(parents=True, exist_ok=True)
    return directory
