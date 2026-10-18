#!/usr/bin/env python3
"""Cancel the older merge queue runs this run replaces.

When main moves under the queue, GitHub rebuilds each waiting group on a new
`gh-readonly-queue/main/pr-<n>-<base>` ref but leaves the run on the old ref
going, though its result can no longer land. This cancels every unfinished run
of this workflow for the same pull request that started before this one, which
frees its runners for the run that counts.
"""

import json
import os
import re
import sys

from queue_verdict import gh_json
from stop_on_failure import cancel

QUEUE_REF = re.compile(r"^(?:refs/heads/)?gh-readonly-queue/[^/]+/pr-(\d+)-[0-9a-f]+$")


def queue_pr(ref: str) -> str | None:
    match = QUEUE_REF.match(ref)
    return match and match.group(1)


def superseded(runs: list[dict], pr: str, run_id: int) -> list[int]:
    return [
        run["id"]
        for run in runs
        if run["id"] < run_id and run.get("status") != "completed" and queue_pr(run.get("head_branch", "")) == pr
    ]


def cancel_superseded(repo: str, workflow_id: str, ref: str, run_id: int) -> str:
    pr = queue_pr(ref)
    if pr is None:
        return f"{ref} is not a merge queue ref"
    runs = gh_json([f"repos/{repo}/actions/workflows/{workflow_id}/runs?event=merge_group&per_page=50"])[
        "workflow_runs"
    ]
    cancelled, failed = [], []
    for old in superseded(runs, pr, run_id):
        try:
            cancel(repo, old)
            cancelled.append(old)
        except RuntimeError:
            failed.append(old)
    message = f"cancelled {len(cancelled)} superseded run(s) for #{pr}: {cancelled}"
    return f"{message}; could not cancel {failed}" if failed else message


def main() -> int:
    try:
        repo = os.environ["GITHUB_REPOSITORY"]
        run_id = int(os.environ["GITHUB_RUN_ID"])
        workflow_id = gh_json([f"repos/{repo}/actions/runs/{run_id}"])["workflow_id"]
        print(cancel_superseded(repo, workflow_id, os.environ["HEAD_REF"], run_id))
    except (RuntimeError, KeyError, ValueError, json.JSONDecodeError) as error:
        print(f"::warning::left superseded queue runs running: {error}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
