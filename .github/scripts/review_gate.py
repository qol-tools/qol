#!/usr/bin/env python3

import json
import os
import re
import subprocess
import sys
import time
from collections.abc import Callable

from queue_verdict import gh_json
from stop_on_failure import cancel
from wait_for_review import review_runs

REVIEW_BOT = "github-actions[bot]"
VERDICT_JOB = "claude review"
WAIT_JOB = "Wait for the review"
COMMAND = re.compile(r"/review-override ([0-9a-f]{7,40})")
RECORD = re.compile(r"<!-- qol-review-override ([0-9a-f]{40}) (\S+) -->")
CANCEL_GRACE_SECONDS = 300
RESTART_POLLS = 12
POLL_SECONDS = 5


def gh(args: list[str]) -> str:
    result = subprocess.run(["gh", "api", *args], capture_output=True, text=True, check=False)
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or result.stdout.strip() or f"gh api {args[-1]} failed")
    return result.stdout


def comments(repo: str, pr: str) -> list[dict]:
    pages = json.loads(gh(["--paginate", "--slurp", f"repos/{repo}/issues/{pr}/comments"]))
    return [comment for page in pages for comment in page]


def override_for(entries: list[dict], head_sha: str) -> dict | None:
    for comment in reversed(entries):
        match = RECORD.match(comment.get("body") or "")
        if comment.get("user", {}).get("login") == REVIEW_BOT and match and match.group(1) == head_sha:
            return {"actor": match.group(2), "url": comment.get("html_url", "")}
    return None


def record(head_sha: str, actor: str) -> str:
    return "\n".join(
        [
            f"<!-- qol-review-override {head_sha} {actor} -->",
            "",
            "> [!NOTE]",
            f"> **Review override** · @{actor} passed the review of `{head_sha[:7]}`.",
            "> The claude review check passes for this commit only; a new push is reviewed again.",
            "",
        ]
    )


def verdict(env: dict, summary: str | None) -> int:
    event, review = env.get("EVENT", ""), env.get("VERDICT", "")
    print(f"event: {event}, bot: {env.get('BOT', '')}, verdict: {review or 'none'}")
    if event == "merge_group" or env.get("BOT") == "true" or review == "pass":
        return 0
    override = override_for(comments(env["GITHUB_REPOSITORY"], env["PR_NUMBER"]), env["HEAD_SHA"])
    if not override:
        print(f"::error::the review verdict is {review or 'missing'}; only a pass verdict or an owner override may queue")
        return 1
    note = f"@{override['actor']} overrode the {review or 'missing'} review verdict of {env['HEAD_SHA'][:7]}: {override['url']}"
    print(f"::notice title=Review override::{note}")
    if summary:
        with open(summary, "a", encoding="utf-8") as handle:
            handle.write(f"### Review override\n\n{note}\n")
    return 0


def end_if_superseded(repo: str, head_sha: str, run_id: str, sleep: Callable[[float], None] = time.sleep) -> int:
    newer = [run for run in review_runs(repo, head_sha) if run["id"] > int(run_id)]
    if not newer:
        print("no newer review run of this commit, so the verdict decides")
        return 0
    print(f"review run {newer[0]['html_url']} replaced this one, so this run ends cancelled")
    cancel(repo, run_id)
    sleep(CANCEL_GRACE_SECONDS)
    print("::error::this run was not cancelled")
    return 1


def rerun_waits(repo: str, pr: str, head_sha: str) -> list[str]:
    if gh_json([f"repos/{repo}/pulls/{pr}"])["head"]["sha"] != head_sha:
        print(f"the pull request moved past {head_sha[:7]}, so its checks stay as they are")
        return []
    latest: dict[int, dict] = {}
    for run in gh_json([f"repos/{repo}/actions/runs?head_sha={head_sha}&event=pull_request&per_page=100"])["workflow_runs"]:
        if run["id"] > latest.get(run["workflow_id"], {"id": 0})["id"]:
            latest[run["workflow_id"]] = run
    rerun = []
    for run in latest.values():
        if run.get("status") != "completed" or run.get("conclusion") == "success":
            continue
        jobs = gh_json([f"repos/{repo}/actions/runs/{run['id']}/jobs?per_page=100"])["jobs"]
        if any(job["name"] == WAIT_JOB and job.get("conclusion") == "failure" for job in jobs):
            gh(["--method", "POST", f"repos/{repo}/actions/runs/{run['id']}/rerun-failed-jobs"])
            rerun.append(run["html_url"])
    return rerun


def rerun_verdict(repo: str, review: dict, sleep: Callable[[float], None]) -> None:
    jobs = gh_json([f"repos/{repo}/actions/runs/{review['id']}/jobs?per_page=100"])["jobs"]
    job = next((job for job in jobs if job["name"] == VERDICT_JOB), None)
    if job is None:
        raise RuntimeError(f"no {VERDICT_JOB} job in {review['html_url']}")
    gh(["--method", "POST", f"repos/{repo}/actions/jobs/{job['id']}/rerun"])
    print(f"reran the verdict: {review['html_url']}")
    for _ in range(RESTART_POLLS):
        if gh_json([f"repos/{repo}/actions/runs/{review['id']}"]).get("status") != "completed":
            return
        sleep(POLL_SECONDS)


def refuse(repo: str, pr: str, reason: str) -> int:
    gh(["--method", "POST", f"repos/{repo}/issues/{pr}/comments", "-f", f"body=> [!WARNING]\n> **Review override refused** · {reason}\n"])
    print(f"::error::{reason}")
    return 1


def apply_override(repo: str, pr: str, body: str, actor: str, sleep: Callable[[float], None] = time.sleep) -> int:
    named = COMMAND.fullmatch((body.strip().splitlines() or [""])[0].strip())
    if not named:
        return refuse(repo, pr, "name the head commit the override is for: `/review-override <sha>`.")
    head_sha = gh_json([f"repos/{repo}/pulls/{pr}"])["head"]["sha"]
    if not head_sha.startswith(named.group(1)):
        return refuse(repo, pr, f"`{named.group(1)}` is not the head commit `{head_sha[:7]}`, so its review was not overridden.")
    review = max(review_runs(repo, head_sha), key=lambda run: run["id"], default=None)
    if review is None or review.get("status") != "completed":
        return refuse(repo, pr, f"the review of `{head_sha[:7]}` has not finished, so there is no verdict to override yet.")
    if review.get("conclusion") == "success":
        print(f"the review of {head_sha[:7]} already passed: {review['html_url']}")
        return 0
    if not override_for(comments(repo, pr), head_sha):
        gh(["--method", "POST", f"repos/{repo}/issues/{pr}/comments", "-f", f"body={record(head_sha, actor)}"])
    print(f"@{actor} overrode the review of {head_sha[:7]}: {review['html_url']}")
    rerun_verdict(repo, review, sleep)
    for url in rerun_waits(repo, pr, head_sha):
        print(f"reran {url}")
    return 0


def main(argv: list[str]) -> int:
    env = os.environ
    command = argv[1] if len(argv) > 1 else ""
    try:
        if command == "verdict":
            return verdict(env, env.get("GITHUB_STEP_SUMMARY"))
        if command == "superseded":
            return end_if_superseded(env["GITHUB_REPOSITORY"], env["HEAD_SHA"], env["GITHUB_RUN_ID"])
        if command == "override":
            return apply_override(env["GITHUB_REPOSITORY"], env["PR_NUMBER"], env["COMMENT"], env["ACTOR"])
        if command == "rerun-waits":
            for url in rerun_waits(env["GITHUB_REPOSITORY"], env["PR_NUMBER"], env["HEAD_SHA"]):
                print(f"reran {url}")
            return 0
    except (RuntimeError, KeyError, json.JSONDecodeError) as error:
        if command == "rerun-waits":
            print(f"::warning::the waiting checks were not rerun: {error}")
            return 0
        print(f"::error::{error}")
        return 1
    print("usage: review_gate.py verdict|superseded|override|rerun-waits", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
