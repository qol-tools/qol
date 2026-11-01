#!/usr/bin/env python3
"""Plan the build a person downloads to live test a pull request.

Only the files on `.github/live-test-paths` count: a change that needs no
person is proved by the checks and gets no build. Their crates and every crate
that depends on them decide what builds: each Linux plugin among them by its
release tag, and the tray when it is among them. Release tags carry the
versions in the tree, so release_candidate.py builds them exactly as a release
would.
"""
import argparse
import json
import os
import sys
import tomllib
from pathlib import Path

import affected_crates
import live_test

ROOT = Path(__file__).resolve().parents[2]
HOST_PACKAGE = "qol-tray"
HOST_ID = "qol-tray"
LINUX = "linux"


def plugin_units(root: Path) -> dict[str, dict]:
    units = {}
    for manifest in sorted(root.glob("plugins/*/plugin.toml")):
        plugin = tomllib.loads(manifest.read_text(encoding="utf-8"))
        cargo = tomllib.loads((manifest.parent / "Cargo.toml").read_text(encoding="utf-8"))
        meta = plugin["plugin"]
        units[cargo["package"]["name"]] = {
            "id": meta["id"],
            "version": cargo["package"]["version"],
            "linux": LINUX in meta.get("platforms", [LINUX]),
        }
    return units


def host_version(root: Path) -> str:
    cargo = tomllib.loads((root / "apps/tray/Cargo.toml").read_text(encoding="utf-8"))
    return cargo["package"]["version"]


def plan(files: list[str], listed: list[str], pkgs: dict, units: dict, tray_version: str) -> dict:
    found = live_test.needing(files, listed)
    seeds = {owner for owner in (affected_crates.owning_package(path, pkgs) for path in found) if owner}
    affected = affected_crates.dependents_closure(seeds, pkgs) if seeds else set()
    plugins = sorted(
        f"{units[name]['id']}-v{units[name]['version']}"
        for name in affected
        if name in units and units[name]["linux"]
    )
    tray = f"{HOST_ID}-v{tray_version}" if HOST_PACKAGE in affected else ""
    return {"found": found, "plugins": plugins, "tray": tray}


def emit(path: str | None, result: dict) -> None:
    if not path:
        return
    with open(path, "a", encoding="utf-8") as handle:
        handle.write(f"plugin_tags={' '.join(result['plugins'])}\n")
        handle.write(f"tray_tag={result['tray']}\n")
        handle.write(f"build={'true' if result['plugins'] or result['tray'] else 'false'}\n")


def manifest(result: dict, pr: str, sha: str, run_id: str) -> dict:
    return {
        "schema": 1,
        "pull_request": int(pr),
        "head_sha": sha,
        "run_id": int(run_id),
        "target": "x86_64-unknown-linux-gnu",
        "plugins": result["plugins"],
        "tray": result["tray"],
    }


def plan_command(args: argparse.Namespace) -> int:
    files = live_test.changed_files(os.environ["REPO"], os.environ["PR_NUMBER"])
    listed = live_test.patterns(live_test.PATHS_FILE.read_text(encoding="utf-8"))
    pkgs = affected_crates.workspace_graph()
    if not pkgs:
        print("::error::cargo metadata is unavailable, so the build cannot be planned", file=sys.stderr)
        return 1
    result = plan(files, listed, pkgs, plugin_units(ROOT), host_version(ROOT))
    emit(args.github_output, result)
    if args.manifest:
        out = Path(args.manifest)
        out.parent.mkdir(parents=True, exist_ok=True)
        body = manifest(result, os.environ["PR_NUMBER"], os.environ["HEAD_SHA"], os.environ["GITHUB_RUN_ID"])
        out.write_text(json.dumps(body, indent=2) + "\n", encoding="utf-8")
    print(f"plugins: {' '.join(result['plugins']) or 'none'}; tray: {result['tray'] or 'no'}")
    return 0


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)
    planned = commands.add_parser("plan")
    planned.add_argument("--github-output", default=os.environ.get("GITHUB_OUTPUT"))
    planned.add_argument("--manifest")
    planned.set_defaults(handler=plan_command)
    return root


if __name__ == "__main__":
    arguments = parser().parse_args()
    sys.exit(arguments.handler(arguments))
