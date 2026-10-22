#!/usr/bin/env python3
"""Delete remote branches whose exact tip merged into the default branch.

A branch goes only when a pull request from this repository merged it into the
default branch from the very commit the branch still points at, that merge
commit is in the default branch's history, and no open pull request uses the
branch as its head or base. Every delete is a push with a lease on that commit,
so a branch someone pushed to after the plan survives.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from typing import Callable

PULL_REQUEST_FIELDS = "number,state,headRefName,headRefOid,baseRefName,isCrossRepository,mergeCommit"


def plan_prune(
    heads: dict[str, str],
    pulls: list[dict],
    default_branch: str,
    in_default: Callable[[str], bool],
) -> list[tuple[str, str, int]]:
    open_refs = {
        ref
        for pull in pulls
        if pull["state"] == "OPEN"
        for ref in (pull["headRefName"], pull["baseRefName"])
    }
    doomed = []
    for branch, sha in sorted(heads.items()):
        if branch == default_branch or branch in open_refs:
            continue
        for pull in pulls:
            merge = (pull.get("mergeCommit") or {}).get("oid")
            if (
                pull["state"] == "MERGED"
                and not pull["isCrossRepository"]
                and pull["headRefName"] == branch
                and pull["headRefOid"] == sha
                and pull["baseRefName"] == default_branch
                and merge
                and in_default(merge)
            ):
                doomed.append((branch, sha, pull["number"]))
                break
    return doomed


def run(args: list[str]) -> str:
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout


def remote_heads() -> dict[str, str]:
    heads = {}
    for line in run(["git", "ls-remote", "--heads", "origin"]).splitlines():
        sha, ref = line.split("\t", 1)
        heads[ref.removeprefix("refs/heads/")] = sha
    return heads


def pull_requests() -> list[dict]:
    return json.loads(
        run(["gh", "pr", "list", "--state", "all", "--limit", "100000", "--json", PULL_REQUEST_FIELDS])
    )


def ancestor_check(default_branch: str) -> Callable[[str], bool]:
    def in_default(sha: str) -> bool:
        result = subprocess.run(
            ["git", "merge-base", "--is-ancestor", sha, f"origin/{default_branch}"],
            capture_output=True,
        )
        return result.returncode == 0

    return in_default


def delete(branch: str, sha: str) -> bool:
    result = subprocess.run(
        ["git", "push", f"--force-with-lease=refs/heads/{branch}:{sha}", "origin", f":refs/heads/{branch}"],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        print(f"kept {branch}: {result.stderr.strip()}", file=sys.stderr)
    return result.returncode == 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--default-branch", default="main")
    parser.add_argument("--dry-run", action="store_true")
    options = parser.parse_args()

    run(["git", "fetch", "--quiet", "origin", options.default_branch])
    doomed = plan_prune(
        remote_heads(), pull_requests(), options.default_branch, ancestor_check(options.default_branch)
    )
    deleted = 0
    for branch, sha, number in doomed:
        if options.dry_run:
            print(f"would delete {branch} at {sha[:9]} (merged by #{number})")
        elif delete(branch, sha):
            deleted += 1
            print(f"deleted {branch} at {sha[:9]} (merged by #{number})")
    print(f"{len(doomed)} merged branches, {deleted} deleted")
    return 0 if options.dry_run or deleted == len(doomed) else 1


if __name__ == "__main__":
    sys.exit(main())
