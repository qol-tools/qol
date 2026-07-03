#!/usr/bin/env python3
"""Prune old component releases and tags, keeping the newest N per component.

Tags follow `<component>-v<major>.<minor>.<patch>`. The newest tags per
component survive so plugin_version.py keeps its base version and collision
set; older ones lose their GitHub release and git tag. Tags that do not match
the pattern are never touched.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from collections import defaultdict

TAG_PATTERN = re.compile(r"^(?P<component>.+)-v(?P<version>\d+\.\d+\.\d+)$")


def parse_tag(tag: str) -> tuple[str, tuple[int, int, int]] | None:
    match = TAG_PATTERN.match(tag)
    if match is None:
        return None
    version = tuple(int(part) for part in match.group("version").split("."))
    return match.group("component"), version


def plan_prune(tags: list[str], keep: int) -> list[str]:
    grouped: dict[str, list[tuple[tuple[int, int, int], str]]] = defaultdict(list)
    for tag in tags:
        parsed = parse_tag(tag)
        if parsed is None:
            continue
        component, version = parsed
        grouped[component].append((version, tag))
    doomed = []
    for versions in grouped.values():
        versions.sort(reverse=True)
        doomed.extend(tag for _, tag in versions[keep:])
    return sorted(doomed)


def gh_api(args: list[str]) -> str:
    result = subprocess.run(
        ["gh", "api", *args], check=True, capture_output=True, text=True
    )
    return result.stdout


def list_remote_tags() -> list[str]:
    output = gh_api(
        ["--paginate", "repos/{owner}/{repo}/git/matching-refs/tags", "--jq", ".[].ref"]
    )
    return [line.removeprefix("refs/tags/") for line in output.splitlines() if line]


def release_id_for_tag(tag: str) -> int | None:
    try:
        output = gh_api([f"repos/{{owner}}/{{repo}}/releases/tags/{tag}"])
    except subprocess.CalledProcessError:
        return None
    return json.loads(output)["id"]


def delete_tag(tag: str) -> None:
    release_id = release_id_for_tag(tag)
    if release_id is not None:
        gh_api(["-X", "DELETE", f"repos/{{owner}}/{{repo}}/releases/{release_id}"])
    gh_api(["-X", "DELETE", f"repos/{{owner}}/{{repo}}/git/refs/tags/{tag}"])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--keep", type=int, default=3)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    if args.keep < 1:
        print("refusing to run with --keep < 1", file=sys.stderr)
        return 1

    tags = list_remote_tags()
    doomed = plan_prune(tags, args.keep)
    kept = len(tags) - len(doomed)
    print(f"{len(tags)} tags total, keeping {kept}, pruning {len(doomed)}")
    for tag in doomed:
        if args.dry_run:
            print(f"would delete {tag}")
            continue
        delete_tag(tag)
        print(f"deleted {tag}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
