import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path

import plugin_version

GLIBC_BASELINE = (2, 39, 0)
NAMED_ABIS = {"GLIBC_ABI_DT_RELR": (2, 36, 0)}


def version_string(version: tuple[int, int, int]) -> str:
    return ".".join(map(str, version if version[2] else version[:2]))


def glibc_requirement(version_info: str) -> tuple[int, int, int]:
    required = (0, 0, 0)
    for name in re.findall(r"\bName: (GLIBC_\S+)", version_info):
        numeric = re.fullmatch(r"GLIBC_(\d+)\.(\d+)(?:\.(\d+))?", name)
        if numeric:
            version = tuple(int(part or 0) for part in numeric.groups())
        elif name in NAMED_ABIS:
            version = NAMED_ABIS[name]
        else:
            raise ValueError(f"unsupported GLIBC requirement: {name}")
        required = max(required, version)
    return required


def verify_binary(path: Path) -> dict:
    with path.open("rb") as binary:
        if binary.read(4) != b"\x7fELF":
            raise ValueError(f"{path}: expected an ELF release binary")
        binary.seek(0)
        digest = hashlib.file_digest(binary, "sha256").hexdigest()
    result = subprocess.run(
        ["readelf", "--version-info", "--wide", str(path)],
        capture_output=True,
        text=True,
        check=True,
        env={**os.environ, "LC_ALL": "C"},
    )
    required = glibc_requirement(result.stdout)
    if required > GLIBC_BASELINE:
        raise ValueError(f"{path}: requires GLIBC_{version_string(required)}; release baseline is GLIBC_{version_string(GLIBC_BASELINE)}")
    return {"path": str(path), "sha256": digest, "glibc_required": required}


def release_binaries(root: Path, package: str, target: str | None) -> list[Path]:
    manifest = plugin_version.load_packages(root)[package].manifest
    cargo = tomllib.loads(manifest.read_text())
    if package == "qol-tray":
        assets = cargo["package"]["metadata"]["deb"]["assets"]
        binaries = [root / source for source, _, mode in assets if source.startswith("target/") and int(mode, 8) & 0o111]
        if not binaries:
            raise ValueError("tray Debian package has no executable build artifacts")
        return binaries
    plugin = tomllib.loads((manifest.parent / "plugin.toml").read_text())
    binaries = plugin.get("dependencies", {}).get("binaries", [])
    name = binaries[0]["name"] if binaries else package
    if not target or not target.endswith("-linux-gnu"):
        raise ValueError("Linux plugin compatibility requires a Linux GNU target")
    return [root / "target" / target / "release" / name]


def main() -> int:
    parser = argparse.ArgumentParser(description="Reject Linux release binaries exceeding the supported GLIBC baseline.")
    parser.add_argument("--root", type=Path, default=Path("."))
    parser.add_argument("--package", required=True)
    parser.add_argument("--target")
    args = parser.parse_args()
    try:
        evidence = [verify_binary(binary) for binary in release_binaries(args.root.resolve(), args.package, args.target)]
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        print(f"::error::Linux release compatibility failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps({"status": "pass", "glibc_baseline": GLIBC_BASELINE, "binaries": evidence}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
