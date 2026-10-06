#!/usr/bin/env python3
"""Push plugin releases to the plugin registry as OCI artifacts.

One artifact per release tag `<plugin-id>-vX.Y.Z`: plugin.toml at the tagged
revision is the config blob, the plugin directory at that revision is one tree
layer, and every release asset is one layer named after its file. Artifacts are
never overwritten: pushing a tag the registry already holds succeeds only when
it holds the same revision, plugin.toml, assets and plugin tree, so a failed
release job can be rerun. The tree is compared uncompressed because gzip output
may differ between git versions.

`push` publishes the release being built. `backfill` publishes the latest
release of each plugin from its GitHub release assets, skipping a tag the
registry already holds.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib
from pathlib import Path

from plugin_index import (
    CONFIG_MEDIA_TYPE,
    DIR_ANNOTATION,
    TITLE_ANNOTATION,
    TREE_MEDIA_TYPE,
    Registry,
)
from prune_releases import parse_tag

ARTIFACT_TYPE = "application/vnd.qol.plugin.v1"
ASSET_MEDIA_TYPE = "application/octet-stream"
SOURCE_ANNOTATION = "org.opencontainers.image.source"
REVISION_ANNOTATION = "org.opencontainers.image.revision"
PUSH_ATTEMPTS = 3


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
    reference: str, release: dict, source: str, config: Path, tree: str
) -> list[str]:
    layers = [f"{tree}:{TREE_MEDIA_TYPE}"]
    layers += [f"{name}:{ASSET_MEDIA_TYPE}" for name in sorted(release["assets"])]
    return [
        "push",
        reference,
        "--artifact-type",
        ARTIFACT_TYPE,
        "--annotation",
        f"{SOURCE_ANNOTATION}={source}",
        "--annotation",
        f"{REVISION_ANNOTATION}={release['revision']}",
        "--annotation",
        f"{DIR_ANNOTATION}={release['plugin_dir']}",
        "--config",
        f"{config}:{CONFIG_MEDIA_TYPE}",
        *layers,
    ]


def sha256_digest(data: bytes) -> str:
    return f"sha256:{hashlib.sha256(data).hexdigest()}"


def release_content(tag: str, revision: str, files: Path) -> dict:
    parsed = parse_tag(tag)
    if parsed is None:
        raise ValueError(f"tag {tag!r} must match <plugin-id>-vX.Y.Z")
    plugin_id = parsed[0]
    plugin_dir = plugin_dir_at(revision, plugin_id)
    assets = {
        path.name: sha256_digest(path.read_bytes()) for path in files.iterdir() if path.is_file()
    }
    if f"{plugin_id}-tree.tar.gz" in assets:
        raise ValueError(f"release asset {plugin_id}-tree.tar.gz collides with the tree layer")
    return {
        "plugin_id": plugin_id,
        "plugin_dir": plugin_dir,
        "revision": run(["git", "rev-parse", f"{revision}^{{commit}}"]).strip(),
        "config": run(["git", "show", f"{revision}:plugins/{plugin_dir}/plugin.toml"]),
        "assets": assets,
    }


def tree_tar(release: dict) -> bytes:
    archive = ["git", "archive", "--format=tar"]
    source = f"{release['revision']}:plugins/{release['plugin_dir']}"
    return subprocess.run([*archive, source], check=True, capture_output=True).stdout


def holds_release(registry: Registry, manifest: dict, release: dict) -> bool:
    layers = manifest["layers"]
    trees = [layer for layer in layers if layer["mediaType"] == TREE_MEDIA_TYPE]
    assets = {
        layer.get("annotations", {}).get(TITLE_ANNOTATION): layer["digest"]
        for layer in layers
        if layer["mediaType"] != TREE_MEDIA_TYPE
    }
    if (
        len(trees) != 1
        or len(layers) != 1 + len(assets)
        or manifest.get("annotations", {}).get(REVISION_ANNOTATION) != release["revision"]
        or manifest["config"]["digest"] != sha256_digest(release["config"].encode())
        or assets != release["assets"]
    ):
        return False
    return gzip.decompress(registry.blob(trees[0]["digest"])) == tree_tar(release)


def publish(registry: Registry, tag: str, release: dict, files: Path, source: str) -> None:
    tree = f"{release['plugin_id']}-tree.tar.gz"
    with tempfile.TemporaryDirectory() as scratch:
        layers = Path(scratch) / "layers"
        layers.mkdir()
        for name in release["assets"]:
            shutil.copy2(files / name, layers / name)
        archive = ["git", "archive", "--format=tar.gz", f"--output={layers / tree}"]
        run([*archive, f"{release['revision']}:plugins/{release['plugin_dir']}"])
        config = Path(scratch) / "plugin.toml"
        config.write_bytes(release["config"].encode())
        reference = f"{registry.reference}:{tag}"
        for attempt in range(1, PUSH_ATTEMPTS + 1):
            try:
                registry.oras(push_args(reference, release, source, config, tree), cwd=layers)
                return
            except subprocess.CalledProcessError:
                if attempt == PUSH_ATTEMPTS:
                    raise
                time.sleep(10 * attempt)


def push(args: argparse.Namespace) -> int:
    registry = Registry(args.registry, args.plain_http)
    release = release_content(args.tag, args.revision, args.files)
    published = registry.published_manifest(args.tag)
    if published is None:
        publish(registry, args.tag, release, args.files, args.source)
        print(f"pushed {args.registry}:{args.tag}")
        return 0
    if holds_release(registry, published, release):
        print(f"{args.registry}:{args.tag} already holds this release")
        return 0
    print(
        f"::error::{args.registry}:{args.tag} holds different content; "
        "artifacts are never overwritten",
        file=sys.stderr,
    )
    return 1


def current_plugin_ids(root: Path) -> set[str]:
    ids = set()
    for manifest in root.glob("plugins/*/plugin.toml"):
        plugin_id = tomllib.loads(manifest.read_text()).get("plugin", {}).get("id")
        if plugin_id:
            ids.add(plugin_id)
    return ids


def backfill_tags(release_tags: list[str], plugin_ids: set[str]) -> list[str]:
    latest: dict[str, tuple[tuple[int, int, int], str]] = {}
    for tag in release_tags:
        parsed = parse_tag(tag)
        if parsed is not None and parsed[0] in plugin_ids:
            plugin_id, number = parsed
            latest[plugin_id] = max(latest.get(plugin_id, (number, tag)), (number, tag))
    return sorted(tag for _, tag in latest.values())


def backfill(args: argparse.Namespace) -> int:
    registry = Registry(args.registry, args.plain_http)
    releases = json.loads(run(["gh", "release", "list", "--limit", "1000", "--json", "tagName"]))
    tags = backfill_tags([release["tagName"] for release in releases], current_plugin_ids(Path(".")))
    for tag in tags:
        if registry.published_manifest(tag) is not None:
            print(f"skipped {tag}: already in the registry")
            continue
        with tempfile.TemporaryDirectory() as downloads:
            run(["gh", "release", "download", tag, "--dir", downloads])
            release = release_content(tag, tag, Path(downloads))
            publish(registry, tag, release, Path(downloads), args.source)
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
