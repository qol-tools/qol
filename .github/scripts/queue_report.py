#!/usr/bin/env python3
"""Tell a pull request why the merge queue sent it back.

GitHub only records "removed from the merge queue" on the pull request timeline,
so a failed queue run posts a comment naming each failed job with its log link.
"""

import json
import os
import sys

from queue_verdict import gh_json, queued_pull


def failed_jobs(jobs: list[dict]) -> list[dict]:
    return [job for job in jobs if job.get("conclusion") in ("failure", "timed_out")]


def comment(failed: list[dict], run_url: str) -> str:
    lines = [f"The merge queue sent this pull request back: [queue run]({run_url}) failed."]
    lines += [f"- [{job['name']}]({job['html_url']})" for job in failed]
    return "\n".join(lines)


def report(repo: str, head_ref: str, run_id: str, run_url: str) -> str:
    number = queued_pull(head_ref)
    if number is None:
        return f"no pull request in {head_ref}"
    jobs = gh_json([f"repos/{repo}/actions/runs/{run_id}/jobs?per_page=100"])["jobs"]
    body = comment(failed_jobs(jobs), run_url)
    gh_json(["--method", "POST", f"repos/{repo}/issues/{number}/comments", "-f", f"body={body}"])
    return f"commented on #{number}"


def main() -> int:
    repo = os.environ["GITHUB_REPOSITORY"]
    run_id = os.environ["GITHUB_RUN_ID"]
    run_url = f"{os.environ['GITHUB_SERVER_URL']}/{repo}/actions/runs/{run_id}"
    try:
        print(report(repo, os.environ["HEAD_REF"], run_id, run_url))
    except (RuntimeError, KeyError, json.JSONDecodeError) as error:
        print(f"::warning::no merge queue comment: {error}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
