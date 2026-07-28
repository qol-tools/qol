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


def plugin_crate_dir(root: Path, plugin_id: str) -> Path:
    matches = []
    for manifest in sorted(root.glob("plugins/*/plugin.toml")):
        plugin = tomllib.loads(manifest.read_text())
        if plugin.get("plugin", {}).get("id") == plugin_id:
            matches.append(manifest.parent)
    if not matches:
        raise ValueError(f"no plugin.toml declares id {plugin_id!r}")
    if len(matches) != 1:
        raise ValueError(f"multiple plugin.toml files declare id {plugin_id!r}")
    return matches[0]


def release_outputs(root: Path, plugin_id: str, expected_version: str) -> dict[str, str]:
    crate_dir = plugin_crate_dir(root, plugin_id)

    plugin = tomllib.loads((crate_dir / "plugin.toml").read_text())
    cargo = tomllib.loads((crate_dir / "Cargo.toml").read_text())

    package = cargo["package"]["name"]
    cargo_version = cargo["package"]["version"]
    plugin_version = plugin["plugin"]["version"]
    if cargo_version != expected_version or plugin_version != expected_version:
        raise ValueError(
            f"tag expects version {expected_version}, got cargo={cargo_version} plugin={plugin_version}"
        )
    binaries = plugin.get("dependencies", {}).get("binaries", [])
    bin_name = binaries[0]["name"] if binaries else package
    asset_pattern = (binaries[0].get("pattern") if binaries else None) or (
        f"{plugin_id}-{{os}}-{{arch}}"
    )
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
        raise ValueError(f"no buildable platforms for {plugin_id}")

    return {
        "crate_dir": crate_dir.as_posix(),
        "package": package,
        "bin_name": bin_name,
        "matrix": json.dumps({"include": include}, separators=(",", ":")),
    }


def main() -> int:
    if len(sys.argv) != 4:
        print("usage: plugin_matrix.py <repo-root> <plugin-id> <version>", file=sys.stderr)
        return 2
    try:
        outputs = release_outputs(Path(sys.argv[1]), sys.argv[2], sys.argv[3])
    except (OSError, KeyError, tomllib.TOMLDecodeError, ValueError) as error:
        print(f"::error::{error}", file=sys.stderr)
        return 1
    print("\n".join(f"{key}={value}" for key, value in outputs.items()))
    return 0


if __name__ == "__main__":
    sys.exit(main())
