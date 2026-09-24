"""Record the exact Cargo dependency licenses for review on CI."""

import json
import os
import subprocess

from artifact_root import output_directory


metadata = json.loads(
    subprocess.check_output(["cargo", "metadata", "--locked", "--format-version", "1"])
)
packages = sorted(
    (
        {
            "name": package["name"],
            "version": package["version"],
            "source": package["source"],
            "license": package["license"],
            "license_file": package["license_file"],
        }
        for package in metadata["packages"]
    ),
    key=lambda package: (package["name"], package["version"]),
)
report = {
    "source_commit": os.environ["CI_COMMIT_SHA"],
    "packages": packages,
    "copyleft_expressions_for_review": [
        package for package in packages if "GPL" in (package["license"] or "")
    ],
    "missing_license_metadata": [
        package
        for package in packages
        if not package["license"] and not package["license_file"]
    ],
}
output = output_directory() / "license-inventory.json"
output.write_text(json.dumps(report, indent=2) + "\n")
print(f"License inventory: {output} ({len(packages)} packages)")
print(json.dumps({key: value for key, value in report.items() if key != "packages"}))

if report["missing_license_metadata"]:
    raise ValueError("Dependency license metadata needs review")
