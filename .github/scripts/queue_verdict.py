#!/usr/bin/env python3
"""Decide whether a merge queue entry can reuse its pull request's lint and test verdict.

A queue entry whose tree is byte-identical to a pull request head that already
passed this workflow has nothing new to lint or test. Any other tree (main moved,
or the entry is batched with other pull requests) runs the full workflow.
"""

import json
import os
import re
import subprocess
import sys

QUEUE_BRANCH = re.compile(r"/pr-(\d+)-[0-9a-f]{40}$")


def queued_pull(head_ref: str) -> int | None:
    match = QUEUE_BRANCH.search(head_ref)
    return int(match.group(1)) if match else None


def reusable(queue_tree: str, pull_tree: str, runs: list[dict], workflow: str) -> bool:
    return queue_tree == pull_tree and any(
        run.get("name") == workflow and run.get("conclusion") == "success"
        for run in runs
    )


def gh_json(args: list[str]) -> dict:
    result = subprocess.run(
        ["gh", "api", *args],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or "gh api failed"
        raise RuntimeError(detail)
    return json.loads(result.stdout)


def tree_of(repo: str, sha: str) -> str:
    return gh_json([f"repos/{repo}/git/commits/{sha}"])["tree"]["sha"]


def verdict(repo: str, head_ref: str, queue_sha: str, workflow: str) -> bool:
    number = queued_pull(head_ref)
    if number is None:
        return False
    pull_sha = gh_json([f"repos/{repo}/pulls/{number}"])["head"]["sha"]
    runs = gh_json(
        [
            "--method",
            "GET",
            f"repos/{repo}/actions/runs",
            "-f",
            f"head_sha={pull_sha}",
            "-f",
            "event=pull_request",
            "-f",
            "status=success",
        ]
    )["workflow_runs"]
    return reusable(tree_of(repo, queue_sha), tree_of(repo, pull_sha), runs, workflow)


def main() -> int:
    try:
        reused = verdict(
            os.environ["GITHUB_REPOSITORY"],
            os.environ["HEAD_REF"],
            os.environ["GITHUB_SHA"],
            os.environ["GITHUB_WORKFLOW"],
        )
    except (RuntimeError, KeyError, json.JSONDecodeError) as error:
        print(f"::warning::running the full workflow: {error}", file=sys.stderr)
        reused = False
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as handle:
        handle.write(f"reused={json.dumps(reused)}\n")
    print(f"reused={json.dumps(reused)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
