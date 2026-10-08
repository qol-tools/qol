#!/usr/bin/env python3
"""Tell a pull request why the merge queue sent it back.

GitHub only records "removed from the merge queue" on the pull request timeline,
so a failed queue run posts a caution comment naming each failed job with its
errors, and marks the pull request head with a failed "merge queue" status that
stays until a new commit is pushed.
"""

import json
import os
import re
import sys

from queue_verdict import gh_json, queued_pull

STATUS_CONTEXT = "merge queue"
EXIT_CODE = re.compile(r"^Process completed with exit code \d+\.$")
MAX_ERRORS = 10


def failed_jobs(jobs: list[dict]) -> list[dict]:
    return [job for job in jobs if job.get("conclusion") in ("failure", "timed_out")]


def errors(annotations: list[dict]) -> list[str]:
    lines = []
    for note in annotations:
        message = " ".join(str(note.get("message", "")).split())
        if note.get("annotation_level") != "failure" or not message or EXIT_CODE.match(message):
            continue
        where = note.get("path") or ""
        if where and where != ".github":
            line = note.get("start_line")
            message = f"`{where}{f':{line}' if line else ''}` {message}"
        lines.append(f"- {message}")
    return lines


def comment(failed: list[tuple[dict, list[str]]], run_url: str) -> str:
    names = ", ".join(f"[{job['name']}]({job['html_url']})" for job, _ in failed) or "no job reported"
    parts = [
        "\n".join(
            [
                "> [!CAUTION]",
                f"> **Merge queue failed** · {names}",
                "> Queueing this head again fails the same way unless the failure was flaky. Push a fix first.",
            ]
        )
    ]
    for job, lines in failed:
        if lines:
            shown = lines[:MAX_ERRORS]
            if len(lines) > len(shown):
                shown.append(f"- {len(lines) - len(shown)} more in [the log]({job['html_url']})")
            parts.append(f"#### {job['name']}\n\n" + "\n".join(shown))
    parts.append(f"<sub>merge queue · [queue run]({run_url})</sub>")
    return "\n\n".join(parts)


def report(repo: str, head_ref: str, run_id: str, run_url: str) -> str:
    number = queued_pull(head_ref)
    if number is None:
        return f"no pull request in {head_ref}"
    jobs = failed_jobs(gh_json([f"repos/{repo}/actions/runs/{run_id}/jobs?per_page=100"])["jobs"])
    failed = [(job, errors(gh_json([f"repos/{repo}/check-runs/{job['id']}/annotations"]))) for job in jobs]
    head = gh_json([f"repos/{repo}/pulls/{number}"])["head"]["sha"]
    description = f"Sent back: {', '.join(job['name'] for job in jobs) or 'failed'}"[:140]
    gh_json(
        [
            "--method",
            "POST",
            f"repos/{repo}/statuses/{head}",
            "-f",
            "state=failure",
            "-f",
            f"context={STATUS_CONTEXT}",
            "-f",
            f"description={description}",
            "-f",
            f"target_url={run_url}",
        ]
    )
    gh_json(["--method", "POST", f"repos/{repo}/issues/{number}/comments", "-f", f"body={comment(failed, run_url)}"])
    return f"failed {head[:7]} and commented on #{number}"


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
