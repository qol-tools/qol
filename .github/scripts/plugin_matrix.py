"""Resolve a plugin crate to its build package, binary name, and platform matrix.

Reads the crate's plugin.toml ([plugin].platforms, [dependencies].binaries) and
Cargo.toml ([package].name), then emits GITHUB_OUTPUT lines consumed by the
release workflow: package, bin_name, and a fromJSON-ready matrix.

Usage: plugin_matrix.py <crate_dir> <plugin_id>
"""

import json
import sys
import tomllib
from pathlib import Path

RUNNERS = {
    "linux": [("ubuntu-latest", "x86_64-unknown-linux-gnu", "linux", "x86_64", "")],
    "macos": [
        ("macos-latest", "aarch64-apple-darwin", "macos", "aarch64", ""),
        ("macos-latest", "x86_64-apple-darwin", "macos", "x86_64", ""),
    ],
    "windows": [("windows-latest", "x86_64-pc-windows-msvc", "windows", "x86_64", ".exe")],
}


def main() -> int:
    crate_dir = Path(sys.argv[1])
    plugin_id = sys.argv[2]

    plugin = tomllib.loads((crate_dir / "plugin.toml").read_text())
    cargo = tomllib.loads((crate_dir / "Cargo.toml").read_text())

    package = cargo["package"]["name"]
    binaries = plugin.get("dependencies", {}).get("binaries", [])
    bin_name = binaries[0]["name"] if binaries else package
    asset_pattern = (binaries[0].get("pattern") if binaries else None) or f"{plugin_id}-{{os}}-{{arch}}"
    platforms = plugin.get("plugin", {}).get("platforms", ["linux"])

    include = [
        {
            "os": runner,
            "target": target,
            "ext": ext,
            "artifact_name": asset_pattern.replace("{os}", os_token).replace("{arch}", arch_token),
        }
        for platform in platforms
        for runner, target, os_token, arch_token, ext in RUNNERS.get(platform, [])
    ]
    if not include:
        print(f"::error::no buildable platforms for {plugin_id}", file=sys.stderr)
        return 1

    lines = [
        f"package={package}",
        f"bin_name={bin_name}",
        "matrix=" + json.dumps({"include": include}, separators=(",", ":")),
    ]
    print("\n".join(lines))
    return 0


if __name__ == "__main__":
    sys.exit(main())
