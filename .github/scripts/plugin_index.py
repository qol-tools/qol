#!/usr/bin/env python3
"""Build the plugin index from the plugin registry.

Every plugin release pushes one OCI artifact tagged `<plugin-id>-vX.Y.Z`: the
released plugin.toml as its config blob, the plugin directory as a tree layer,
and one layer per release asset. The index lists the newest versions of each
plugin with every digest the tray needs, so the tray never lists tags or reads
manifests. It is rebuilt from the registry alone, so concurrent releases cannot
drop each other's entries.

The index is append-only: given the deployed index, the build fails when a
version it lists now points at a different manifest, so a tag pushed over in the
registry is never signed.

A tag counts as absent when the registry answers "not found" or "denied":
ghcr.io denies reads of a package that does not exist yet, and a caller that
can push can also read, so "denied" never hides a tag a push would overwrite.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import time
import tomllib
from pathlib import Path
from typing import Callable

from prune_releases import parse_tag, plan_prune

SCHEMA = 1
CONFIG_MEDIA_TYPE = "application/vnd.qol.plugin.manifest.v1+toml"
TREE_MEDIA_TYPE = "application/vnd.qol.plugin.tree.v1.tar+gzip"
TITLE_ANNOTATION = "org.opencontainers.image.title"
DIR_ANNOTATION = "dev.qol.plugin.dir"
ABSENT_MARKERS = (": not found", "denied: requested access to the resource is denied")
DIGEST_PATTERN = re.compile(r"sha256:[0-9a-f]{64}")


class Registry:
    def __init__(self, reference: str, plain_http: bool) -> None:
        self.reference = reference
        self.plain_http = plain_http

    def oras(self, args: list[str], cwd: Path | None = None, text: bool = True):
        flags = ["--plain-http"] if self.plain_http else []
        result = subprocess.run(
            ["oras", *args, *flags], check=True, capture_output=True, text=text, cwd=cwd
        )
        return result.stdout

    def blob(self, digest: str) -> bytes:
        return self.oras(
            ["blob", "fetch", "--output", "-", f"{self.reference}@{digest}"], text=False
        )

    def tags(self) -> list[str]:
        return self.oras(["repo", "tags", self.reference]).split()

    def published_manifest(self, tag: str) -> dict | None:
        try:
            return json.loads(self.oras(["manifest", "fetch", f"{self.reference}:{tag}"]))
        except subprocess.CalledProcessError as error:
            if any(marker in (error.stderr or "") for marker in ABSENT_MARKERS):
                return None
            raise

    def version(self, tag: str) -> dict:
        descriptor = json.loads(
            self.oras(["manifest", "fetch", "--descriptor", f"{self.reference}:{tag}"])
        )
        digest = descriptor["digest"]
        manifest = json.loads(self.oras(["manifest", "fetch", f"{self.reference}@{digest}"]))
        config = manifest["config"]
        if config["mediaType"] != CONFIG_MEDIA_TYPE:
            raise ValueError(f"{tag}: config media type is {config['mediaType']!r}")
        plugin_toml = self.oras(
            ["blob", "fetch", "--output", "-", f"{self.reference}@{config['digest']}"]
        )
        return version_entry(digest, manifest, plugin_toml)

    def location(self) -> dict:
        host, _, repository = self.reference.partition("/")
        scheme = "http" if self.plain_http else "https"
        return {"url": f"{scheme}://{host}", "repository": repository}


def layer_blob(manifest_digest: str, layer: dict) -> dict:
    digest, size = layer.get("digest"), layer.get("size")
    if not isinstance(digest, str) or not DIGEST_PATTERN.fullmatch(digest):
        raise ValueError(f"{manifest_digest}: invalid layer digest {digest!r}")
    if type(size) is not int or size < 0:
        raise ValueError(f"{manifest_digest}: invalid layer size {size!r}")
    return {"digest": digest, "size": size}


def version_entry(manifest_digest: str, manifest: dict, plugin_toml: str) -> dict:
    trees = []
    assets = {}
    for layer in manifest["layers"]:
        blob = layer_blob(manifest_digest, layer)
        if layer["mediaType"] == TREE_MEDIA_TYPE:
            trees.append(blob)
            continue
        title = layer.get("annotations", {}).get(TITLE_ANNOTATION)
        if not title or title in assets:
            raise ValueError(f"{manifest_digest}: missing or repeated asset title {title!r}")
        assets[title] = blob
    if len(trees) != 1:
        raise ValueError(f"{manifest_digest}: expected one tree layer, found {len(trees)}")
    plugin_dir = manifest.get("annotations", {}).get(DIR_ANNOTATION, "")
    if not plugin_dir or "/" in plugin_dir or plugin_dir.startswith("."):
        raise ValueError(f"{manifest_digest}: invalid {DIR_ANNOTATION} {plugin_dir!r}")
    return {
        "manifest_digest": manifest_digest,
        "dir": plugin_dir,
        "plugin_toml": plugin_toml,
        "tree": trees[0],
        "assets": assets,
    }


def require_matching_manifest(tag: str, plugin_id: str, version: str, plugin_toml: str) -> None:
    plugin = tomllib.loads(plugin_toml).get("plugin", {})
    if plugin.get("id") != plugin_id or plugin.get("version") != version:
        raise ValueError(
            f"{tag}: plugin.toml declares {plugin.get('id')} {plugin.get('version')}"
        )


def build_index(
    tags: list[str],
    fetch_version: Callable[[str], dict],
    location: dict,
    keep: int,
    serial: int,
) -> dict:
    released = [tag for tag in tags if parse_tag(tag) is not None]
    retained = sorted(set(released) - set(plan_prune(released, keep)))
    plugins: dict[str, dict] = {}
    newest: dict[str, tuple[tuple[int, int, int], str]] = {}
    for tag in retained:
        plugin_id, number = parse_tag(tag)
        version = tag.removeprefix(f"{plugin_id}-v")
        entry = fetch_version(tag)
        require_matching_manifest(tag, plugin_id, version, entry["plugin_toml"])
        plugins.setdefault(plugin_id, {"versions": {}})["versions"][version] = entry
        newest[plugin_id] = max(newest.get(plugin_id, (number, version)), (number, version))
    for plugin_id, (_, version) in newest.items():
        plugins[plugin_id]["latest"] = version
    return {"schema": SCHEMA, "serial": serial, "registry": location, "plugins": plugins}


def listed_versions(index: dict) -> dict[tuple[str, str], str]:
    return {
        (plugin_id, version): entry["manifest_digest"]
        for plugin_id, plugin in index.get("plugins", {}).items()
        for version, entry in plugin["versions"].items()
    }


def require_append_only(previous: dict, index: dict) -> None:
    before = listed_versions(previous)
    changed = [
        f"{plugin_id} {version}"
        for (plugin_id, version), digest in sorted(listed_versions(index).items())
        if before.get((plugin_id, version), digest) != digest
    ]
    if changed:
        raise ValueError(f"published versions changed in the registry: {', '.join(changed)}")


def index_changes(previous: dict, index: dict) -> list[str]:
    before = set(listed_versions(previous))
    after = set(listed_versions(index))
    added = [f"+ {plugin_id} {version}" for plugin_id, version in sorted(after - before)]
    removed = [f"- {plugin_id} {version}" for plugin_id, version in sorted(before - after)]
    return added + removed


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registry", required=True, help="for example ghcr.io/qol-tools/plugins")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--keep", type=int, default=3)
    parser.add_argument("--plain-http", action="store_true")
    parser.add_argument(
        "--previous", type=Path, help="the deployed index, already verified against its signature"
    )
    args = parser.parse_args()
    if args.keep < 1:
        print("refusing to run with --keep < 1", file=sys.stderr)
        return 1

    registry = Registry(args.registry, args.plain_http)
    try:
        previous = json.loads(args.previous.read_text()) if args.previous else {}
        serial = max(int(time.time()), previous.get("serial", 0) + 1)
        index = build_index(
            registry.tags(), registry.version, registry.location(), args.keep, serial
        )
        require_append_only(previous, index)
    except (subprocess.CalledProcessError, KeyError, ValueError, tomllib.TOMLDecodeError) as error:
        stderr = getattr(error, "stderr", "") or ""
        print(f"::error::{error} {stderr}".strip(), file=sys.stderr)
        return 1
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(index, sort_keys=True, separators=(",", ":")))
    for plugin_id, plugin in sorted(index["plugins"].items()):
        print(f"{plugin_id} {plugin['latest']} ({len(plugin['versions'])} versions)")
    for change in index_changes(previous, index):
        print(change)
    return 0


if __name__ == "__main__":
    sys.exit(main())
