#!/usr/bin/env python3
"""Plan the build a person downloads to live test a pull request.

Only the files on `.github/live-test-paths` decide whether a person must test:
a change that needs no person is proved by the checks and gets no build. Once
one does, every changed crate counts, so the build carries the whole pull
request: those crates and every crate that depends on them decide what builds
for each platform: every plugin among
them that runs on that platform by its release tag, and the tray when it is
among them. Release tags carry the
versions in the tree, so release_candidate.py builds them exactly as a release
would. A vendored crate the root manifest patches in belongs to no workspace
crate, so its change counts for every crate that depends on it by name.
"""
import argparse
import json
import os
import sys
import tomllib
from pathlib import Path

import affected_crates
import live_test
import plugin_matrix

ROOT = Path(__file__).resolve().parents[2]
HOST_PACKAGE = "qol-tray"
HOST_ID = "qol-tray"
LINUX = "linux"
PLATFORMS = {
    "linux": {"target": "x86_64-unknown-linux-gnu", "name": "Linux"},
    "macos": {"target": "aarch64-apple-darwin", "name": "macOS"},
}
DEPENDENCY_TABLES = ("dependencies", "dev-dependencies", "build-dependencies")


def patched_paths(root: Path) -> dict[str, str]:
    cargo = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    patches = {}
    for table in cargo.get("patch", {}).values():
        for name, spec in table.items():
            if isinstance(spec, dict) and "path" in spec:
                patches[spec["path"].rstrip("/")] = name
    return patches


def declared_dependencies(manifest: dict) -> set[str]:
    names = set()
    for table in DEPENDENCY_TABLES:
        names.update(manifest.get(table, {}))
    for target in manifest.get("target", {}).values():
        for table in DEPENDENCY_TABLES:
            names.update(target.get(table, {}))
    return names


def dependents_by_name(root: Path, pkgs: dict) -> dict[str, set[str]]:
    users: dict[str, set[str]] = {}
    for name, info in pkgs.items():
        manifest = root / info["dir"] / "Cargo.toml" if info["dir"] else root / "Cargo.toml"
        if not manifest.is_file():
            continue
        for dependency in declared_dependencies(tomllib.loads(manifest.read_text(encoding="utf-8"))):
            users.setdefault(dependency, set()).add(name)
    return users


def seeds_for(found: list[str], pkgs: dict, patches: dict[str, str], users: dict[str, set[str]]) -> tuple[set[str], list[str]]:
    seeds, orphans = set(), []
    for path in found:
        owner = affected_crates.owning_package(path, pkgs)
        if owner:
            seeds.add(owner)
            continue
        patched = [name for prefix, name in patches.items() if path.startswith(prefix + "/")]
        if patched:
            for name in patched:
                seeds.update(users.get(name, set()))
            continue
        orphans.append(path)
    return seeds, orphans


def plugin_units(root: Path) -> dict[str, dict]:
    units = {}
    for manifest in sorted(root.glob("plugins/*/plugin.toml")):
        plugin = tomllib.loads(manifest.read_text(encoding="utf-8"))
        cargo = tomllib.loads((manifest.parent / "Cargo.toml").read_text(encoding="utf-8"))
        meta = plugin["plugin"]
        units[cargo["package"]["name"]] = {
            "id": meta["id"],
            "version": cargo["package"]["version"],
            "platforms": meta.get("platforms", [LINUX]),
        }
    return units


def host_version(root: Path) -> str:
    cargo = tomllib.loads((root / "apps/tray/Cargo.toml").read_text(encoding="utf-8"))
    return cargo["package"]["version"]


def plan(
    files: list[str],
    listed: list[str],
    pkgs: dict,
    units: dict,
    tray_version: str,
    patches: dict[str, str] | None = None,
    users: dict[str, set[str]] | None = None,
    platform: str = LINUX,
) -> dict:
    found = live_test.needing(files, listed)
    seeds, orphans = seeds_for(found, pkgs, patches or {}, users or {})
    if found:
        seeds |= seeds_for(files, pkgs, patches or {}, users or {})[0]
    affected = affected_crates.dependents_closure(seeds, pkgs) if seeds else set()
    runs = sorted(name for name in affected if name in units and platform in units[name]["platforms"])
    plugins = [f"{units[name]['id']}-v{units[name]['version']}" for name in runs]
    tray = f"{HOST_ID}-v{tray_version}" if HOST_PACKAGE in affected else ""
    return {
        "found": found,
        "orphans": orphans,
        "plugins": sorted(plugins),
        "units": [units[name] for name in runs],
        "tray": tray,
    }


def verdict(result: dict, platform: str = LINUX) -> tuple[int, str]:
    if result["orphans"]:
        listed = ", ".join(result["orphans"])
        return 1, f"::error::no workspace crate builds {listed}, so the live test has nothing to download"
    if result["plugins"] or result["tray"]:
        return 0, f"plugins: {' '.join(result['plugins']) or 'none'}; tray: {result['tray'] or 'no'}"
    name = PLATFORMS[platform]["name"]
    return 0, f"::warning::nothing these changes reach runs on {name}, so no {name} build is uploaded; test it on its own platform"


def emit(path: str | None, result: dict) -> None:
    if not path:
        return
    with open(path, "a", encoding="utf-8") as handle:
        handle.write(f"plugin_tags={' '.join(result['plugins'])}\n")
        handle.write(f"tray_tag={result['tray']}\n")
        handle.write(f"build={'true' if result['plugins'] or result['tray'] else 'false'}\n")


def installed_files(root: Path, unit: dict, platform: str = LINUX) -> dict:
    target = PLATFORMS[platform]["target"]
    outputs = plugin_matrix.release_outputs(root, unit["id"], unit["version"])
    entry = next(row for row in json.loads(outputs["matrix"])["include"] if row["target"] == target)
    return {
        "id": unit["id"],
        "version": unit["version"],
        "tag": f"{unit['id']}-v{unit['version']}",
        "file": f"{entry['artifact_name']}{entry['ext']}",
        "bin_name": outputs["bin_name"],
    }


def manifest(result: dict, pr: str, sha: str, run_id: str, files: list[dict], platform: str = LINUX) -> dict:
    return {
        "schema": 1,
        "pull_request": int(pr),
        "head_sha": sha,
        "run_id": int(run_id),
        "target": PLATFORMS[platform]["target"],
        "plugins": files,
        "tray": result["tray"],
    }


def plan_command(args: argparse.Namespace) -> int:
    files = live_test.changed_files(os.environ["REPO"], os.environ["PR_NUMBER"])
    listed = live_test.patterns(live_test.PATHS_FILE.read_text(encoding="utf-8"))
    pkgs = affected_crates.workspace_graph()
    if not pkgs:
        print("::error::cargo metadata is unavailable, so the build cannot be planned", file=sys.stderr)
        return 1
    result = plan(
        files,
        listed,
        pkgs,
        plugin_units(ROOT),
        host_version(ROOT),
        patched_paths(ROOT),
        dependents_by_name(ROOT, pkgs),
        args.platform,
    )
    code, message = verdict(result, args.platform)
    print(message)
    if code:
        return code
    emit(args.github_output, result)
    if args.manifest and (result["plugins"] or result["tray"]):
        out = Path(args.manifest)
        out.parent.mkdir(parents=True, exist_ok=True)
        entries = [installed_files(ROOT, unit, args.platform) for unit in result["units"]]
        body = manifest(
            result, os.environ["PR_NUMBER"], os.environ["HEAD_SHA"], os.environ["GITHUB_RUN_ID"], entries, args.platform
        )
        out.write_text(json.dumps(body, indent=2) + "\n", encoding="utf-8")
    return 0


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)
    planned = commands.add_parser("plan")
    planned.add_argument("--github-output", default=os.environ.get("GITHUB_OUTPUT"))
    planned.add_argument("--manifest")
    planned.add_argument("--platform", choices=sorted(PLATFORMS), default=LINUX)
    planned.set_defaults(handler=plan_command)
    return root


if __name__ == "__main__":
    arguments = parser().parse_args()
    sys.exit(arguments.handler(arguments))
