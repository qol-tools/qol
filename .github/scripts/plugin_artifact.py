#!/usr/bin/env python3
"""Push plugin releases to the plugin registry as OCI artifacts.

One artifact per release tag `<plugin-id>-vX.Y.Z`: plugin.toml at the tagged
revision is the config blob, the plugin directory at that revision is one tree
layer, and every release asset is one layer named after its file. Artifacts are
never overwritten.

`push` publishes the release being built. `backfill` publishes releases that
predate the registry from their GitHub release assets, skipping tags the
registry already holds.
"""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

from plugin_index import (
    CONFIG_MEDIA_TYPE,
    DIR_ANNOTATION,
    TREE_MEDIA_TYPE,
    Registry,
)
from prune_releases import parse_tag

ARTIFACT_TYPE = "application/vnd.qol.plugin.v1"
ASSET_MEDIA_TYPE = "application/octet-stream"
SOURCE_ANNOTATION = "org.opencontainers.image.source"


def run(args: list[str]) -> str:
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout


def declared_id(revision: str, plugin_dir: str) -> str | None:
    try:
        text = run(["git", "show", f"{revision}:plugins/{plugin_dir}/plugin.toml"])
    except subprocess.CalledProcessError:
        return None
    return tomllib.loads(text).get("plugin", {}).get("id")


def plugin_dir_at(revision: str, plugin_id: str) -> str:
    names = run(["git", "ls-tree", "--name-only", "-d", f"{revision}:plugins"]).split()
    matches = [name for name in names if declared_id(revision, name) == plugin_id]
    if len(matches) != 1:
        raise ValueError(f"{revision}: {len(matches)} plugin directories declare {plugin_id!r}")
    return matches[0]


def push_args(
    reference: str, plugin_dir: str, source: str, config: Path, tree: str, assets: list[str]
) -> list[str]:
    layers = [f"{tree}:{TREE_MEDIA_TYPE}"]
    layers += [f"{name}:{ASSET_MEDIA_TYPE}" for name in sorted(assets)]
    return [
        "push",
        reference,
        "--artifact-type",
        ARTIFACT_TYPE,
        "--annotation",
        f"{SOURCE_ANNOTATION}={source}",
        "--annotation",
        f"{DIR_ANNOTATION}={plugin_dir}",
        "--config",
        f"{config}:{CONFIG_MEDIA_TYPE}",
        *layers,
    ]


def publish(registry: Registry, tag: str, revision: str, files: Path, source: str) -> None:
    parsed = parse_tag(tag)
    if parsed is None:
        raise ValueError(f"tag {tag!r} must match <plugin-id>-vX.Y.Z")
    plugin_id = parsed[0]
    plugin_dir = plugin_dir_at(revision, plugin_id)
    assets = sorted(path.name for path in files.iterdir() if path.is_file())
    tree = f"{plugin_id}-tree.tar.gz"
    if tree in assets:
        raise ValueError(f"release asset {tree} collides with the tree layer")
    with tempfile.TemporaryDirectory() as scratch:
        layers = Path(scratch) / "layers"
        layers.mkdir()
        for name in assets:
            shutil.copy2(files / name, layers / name)
        archive = ["git", "archive", "--format=tar.gz", f"--output={layers / tree}"]
        run([*archive, f"{revision}:plugins/{plugin_dir}"])
        config = Path(scratch) / "plugin.toml"
        config.write_text(run(["git", "show", f"{revision}:plugins/{plugin_dir}/plugin.toml"]))
        reference = f"{registry.reference}:{tag}"
        registry.oras(push_args(reference, plugin_dir, source, config, tree, assets), cwd=layers)


def push(args: argparse.Namespace) -> int:
    registry = Registry(args.registry, args.plain_http)
    if registry.has(args.tag):
        print(f"{args.registry}:{args.tag} already exists; artifacts are never overwritten")
        return 1
    publish(registry, args.tag, args.revision, args.files, args.source)
    print(f"pushed {args.registry}:{args.tag}")
    return 0


def current_plugin_ids(root: Path) -> set[str]:
    ids = set()
    for manifest in root.glob("plugins/*/plugin.toml"):
        plugin_id = tomllib.loads(manifest.read_text()).get("plugin", {}).get("id")
        if plugin_id:
            ids.add(plugin_id)
    return ids


def backfill_tags(release_tags: list[str], plugin_ids: set[str]) -> list[str]:
    selected = []
    for tag in release_tags:
        parsed = parse_tag(tag)
        if parsed is not None and parsed[0] in plugin_ids:
            selected.append(tag)
    return sorted(selected)


def backfill(args: argparse.Namespace) -> int:
    registry = Registry(args.registry, args.plain_http)
    releases = json.loads(run(["gh", "release", "list", "--limit", "1000", "--json", "tagName"]))
    tags = backfill_tags([release["tagName"] for release in releases], current_plugin_ids(Path(".")))
    for tag in tags:
        if registry.has(tag):
            print(f"skipped {tag}: already in the registry")
            continue
        with tempfile.TemporaryDirectory() as downloads:
            run(["gh", "release", "download", tag, "--dir", downloads])
            publish(registry, tag, tag, Path(downloads), args.source)
        print(f"pushed {tag}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registry", required=True, help="for example ghcr.io/qol-tools/plugins")
    parser.add_argument("--source", required=True, help="repository URL for the source annotation")
    parser.add_argument("--plain-http", action="store_true")
    commands = parser.add_subparsers(dest="command", required=True)
    push_parser = commands.add_parser("push")
    push_parser.add_argument("--tag", required=True)
    push_parser.add_argument("--revision", default="HEAD")
    push_parser.add_argument("--files", required=True, type=Path)
    commands.add_parser("backfill")
    args = parser.parse_args()
    try:
        return push(args) if args.command == "push" else backfill(args)
    except (subprocess.CalledProcessError, ValueError) as error:
        stderr = getattr(error, "stderr", "") or ""
        print(f"::error::{error} {stderr}".strip(), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
