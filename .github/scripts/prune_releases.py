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
import time
from collections import defaultdict
from urllib.parse import quote

MUTATION_PAUSE_SECONDS = 0.5
RETRY_PAUSE_SECONDS = 30
RETRIES = 3

TAG_PATTERN = re.compile(r"^(?P<component>.+)-v(?P<version>\d+\.\d+\.\d+)$")
RULE_VIOLATION_MARKER = "Repository rule violations found"


class RuleRefusedError(RuntimeError):
    pass


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


GONE_MARKERS = ("HTTP 404", "Reference does not exist")


def gh_api_mutation(args: list[str]) -> None:
    for attempt in range(1, RETRIES + 1):
        try:
            gh_api(args)
            return
        except subprocess.CalledProcessError as error:
            if any(marker in (error.stderr or "") for marker in GONE_MARKERS):
                return
            if attempt == RETRIES:
                print(error.stderr, file=sys.stderr)
                raise
            time.sleep(RETRY_PAUSE_SECONDS * attempt)


def list_remote_tags() -> list[str]:
    output = gh_api(
        ["--paginate", "repos/{owner}/{repo}/git/matching-refs/tags", "--jq", ".[].ref"]
    )
    return [line.removeprefix("refs/tags/") for line in output.splitlines() if line]


def release_id_for_tag(tag: str) -> int | None:
    for attempt in range(1, RETRIES + 1):
        try:
            output = gh_api([f"repos/{{owner}}/{{repo}}/releases/tags/{quote(tag, safe='')}"])
            return json.loads(output)["id"]
        except subprocess.CalledProcessError as error:
            if any(marker in (error.stderr or "") for marker in GONE_MARKERS):
                return None
            if attempt == RETRIES:
                raise
            time.sleep(RETRY_PAUSE_SECONDS * attempt)


def delete_tag(tag: str) -> None:
    release_id = release_id_for_tag(tag)
    if release_id is not None:
        gh_api_mutation(
            ["-X", "DELETE", f"repos/{{owner}}/{{repo}}/releases/{release_id}"]
        )
    for attempt in range(1, RETRIES + 1):
        try:
            gh_api(
                ["-X", "DELETE", f"repos/{{owner}}/{{repo}}/git/refs/tags/{quote(tag, safe='')}"]
            )
            break
        except subprocess.CalledProcessError as error:
            stderr = error.stderr or ""
            if any(marker in stderr for marker in GONE_MARKERS):
                break
            if "Cannot delete this tag" in stderr:
                release_id = release_id_for_tag(tag)
                if release_id is None and RULE_VIOLATION_MARKER in stderr:
                    raise RuleRefusedError(
                        f"tag deletion refused by a repository rule: {stderr.strip()}"
                    )
                if release_id is not None:
                    gh_api_mutation(
                        [
                            "-X",
                            "DELETE",
                            f"repos/{{owner}}/{{repo}}/releases/{release_id}",
                        ]
                    )
            time.sleep(RETRY_PAUSE_SECONDS * attempt)
            if attempt == RETRIES:
                raise
    time.sleep(MUTATION_PAUSE_SECONDS)


def current_latest_tag() -> str | None:
    try:
        return gh_api(["repos/{owner}/{repo}/releases/latest", "--jq", ".tag_name"]).strip() or None
    except subprocess.CalledProcessError:
        return None


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
    latest_tag = current_latest_tag()
    if latest_tag in doomed:
        print(f"keeping {latest_tag}: it holds the Latest badge")
        doomed.remove(latest_tag)
    failed = False
    failures = []
    rule_refused = []
    for tag in doomed:
        if args.dry_run:
            print(f"would delete {tag}")
            continue
        try:
            delete_tag(tag)
        except RuleRefusedError as error:
            rule_refused.append(tag)
            print(f"unprunable {tag}: {error}", file=sys.stderr)
            continue
        except Exception as error:
            failed = True
            failures.append((tag, error))
            print(f"failed to delete {tag}: {error}", file=sys.stderr)
            continue
        print(f"deleted {tag}")
    if rule_refused:
        names = ", ".join(rule_refused)
        print(f"kept {len(rule_refused)} rule-refused tag(s): {names}")
    if failed:
        names = ", ".join(tag for tag, _ in failures)
        print(f"failed to prune {len(failures)} tag(s): {names}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
