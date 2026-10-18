#!/usr/bin/env python3
"""Run one release build for each merge queue batch.

Every queue entry lints and tests its own queue commit, which holds main, the
entries ahead of it and its own change. Only the tail, the last entry GitHub is
building, runs the release build, planned against main so it compiles every
change in the batch. When that build and the tail's own jobs pass, the tail
marks each entry its commit holds as built and passes the merge gate of each
one whose own jobs passed. An entry whose jobs finish later passes its own gate
when it finds the mark, so no entry waits on a runner.

A failed batch release build comes back as one pull request holding the whole
batch, and the originals close. When the tail leaves without a build, the
reconciler, which runs after every queue run, reruns the new tail's batch role
so it builds. Each action first checks that its commit is still a live queue
entry, so a run GitHub rebuilt acts on nothing.
"""

import json
import os
import re
import sys
import time

from queue_report import comment, errors, failed_jobs
from queue_verdict import gh_json

BATCH_JOB = "batch role"
RELEASE_JOB_PREFIX = "release build ("
GATE = "merge gate"
CHECKED = "queue entry checks"
BUILT = "batch release build"
BUILDING = {"AWAITING_CHECKS", "MERGEABLE", "LOCKED"}
BATCH_BRANCH = "queue-batch/"
MAX_ATTEMPTS = 3
CONVENTIONAL = re.compile(r"^([a-z]+)(?:\([^)]*\))?(!)?:")
QUERY = """
query($owner: String!, $name: String!) {
  repository(owner: $owner, name: $name) {
    mergeQueue(branch: "main") {
      entries(first: 50) {
        nodes {
          position
          state
          headCommit { oid }
          baseCommit { oid }
          pullRequest { number title headRefName }
        }
      }
    }
  }
}
"""


def queue_entries(data: dict) -> list[dict]:
    queue = data["data"]["repository"]["mergeQueue"]
    nodes = queue["entries"]["nodes"] if queue else []
    return sorted(
        (
            {
                "position": node["position"],
                "state": node["state"],
                "oid": (node.get("headCommit") or {}).get("oid"),
                "base": (node.get("baseCommit") or {}).get("oid"),
                "number": node["pullRequest"]["number"],
                "title": node["pullRequest"]["title"],
                "branch": node["pullRequest"]["headRefName"],
            }
            for node in nodes
        ),
        key=lambda entry: entry["position"],
    )


def own_entry(entries: list[dict], sha: str) -> dict | None:
    return next((entry for entry in entries if entry["oid"] == sha), None)


def role(entries: list[dict], sha: str) -> str:
    own = own_entry(entries, sha)
    if own is None:
        return "gone"
    behind = any(entry["position"] > own["position"] and entry["state"] in BUILDING for entry in entries)
    return "ahead" if behind else "tail"


def batch(entries: list[dict], own: dict) -> list[dict]:
    """The entries own's commit holds, front to back, following each queue commit's parent."""
    by_oid = {entry["oid"]: entry for entry in entries if entry["oid"]}
    held = [own]
    while held[-1]["base"] in by_oid and len(held) <= len(entries):
        held.append(by_oid[held[-1]["base"]])
    return held[::-1]


def numbers(entries: list[dict]) -> str:
    refs = [f"#{entry['number']}" for entry in entries]
    return refs[0] if len(refs) == 1 else f"{', '.join(refs[:-1])} and {refs[-1]}"


def combined_title(entries: list[dict]) -> str:
    """Versioning releases on feat, fix and perf, so the title keeps the highest type."""
    parsed = [CONVENTIONAL.match(entry["title"]) for entry in entries]
    types = [match.group(1) for match in parsed if match]
    kind = next((name for name in ("feat", "fix", "perf") if name in types), types[0] if types else "chore")
    breaking = "!" if any(match and match.group(2) for match in parsed) else ""
    return f"{kind}{breaking}: land {numbers(entries)} together"


def combined_body(entries: list[dict], failure: str) -> str:
    originals = "\n".join(f"* {entry['title']} (#{entry['number']})" for entry in entries)
    return "\n\n".join(
        [
            "The batch release build failed, so these pull requests come back as this one and the "
            "originals are closed. Push the fix here: a pull request GitHub Actions opens starts no "
            "checks until its first push.",
            originals,
            failure,
        ]
    )


def api(args: list[str]) -> dict:
    for attempt in range(MAX_ATTEMPTS):
        try:
            return gh_json(args)
        except RuntimeError:
            if attempt == MAX_ATTEMPTS - 1:
                raise
            time.sleep(5)
    raise AssertionError("unreachable")


def live_entries(repo: str) -> list[dict]:
    owner, name = repo.split("/")
    return queue_entries(api(["graphql", "-f", f"query={QUERY}", "-f", f"owner={owner}", "-f", f"name={name}"]))


def live_entry(repo: str, sha: str) -> dict | None:
    return own_entry(live_entries(repo), sha)


def merged(repo: str, sha: str) -> bool:
    return api([f"repos/{repo}/compare/main...{sha}"])["status"] in ("behind", "identical")


def latest_check(repo: str, sha: str, name: str) -> str | None:
    runs = api(
        [
            "--method",
            "GET",
            f"repos/{repo}/commits/{sha}/check-runs",
            "-f",
            f"check_name={name}",
            "-f",
            "filter=latest",
        ]
    )["check_runs"]
    return runs[0].get("conclusion") if runs else None


def passed(repo: str, sha: str, name: str) -> bool:
    return latest_check(repo, sha, name) == "success"


def post_check(repo: str, sha: str, name: str, conclusion: str, summary: str, url: str) -> None:
    api(
        [
            "--method",
            "POST",
            f"repos/{repo}/check-runs",
            "-f",
            f"name={name}",
            "-f",
            f"head_sha={sha}",
            "-f",
            "status=completed",
            "-f",
            f"conclusion={conclusion}",
            "-f",
            f"details_url={url}",
            "-f",
            f"output[title]={name}",
            "-f",
            f"output[summary]={summary}",
        ]
    )


def decide(repo: str, sha: str, sleep=time.sleep) -> str:
    """A just-enqueued entry can lag the queue listing, so a missing entry is looked up again."""
    for attempt in range(MAX_ATTEMPTS):
        entries = live_entries(repo)
        if own_entry(entries, sha) or attempt == MAX_ATTEMPTS - 1:
            return role(entries, sha)
        sleep(10)
    raise AssertionError("unreachable")


def gate(repo: str, sha: str, own_passed: bool, own_role: str, run_url: str) -> str:
    entries = live_entries(repo)
    own = own_entry(entries, sha)
    if own is None:
        if merged(repo, sha):
            return "merged: nothing left to gate"
        return "stale: GitHub rebuilt or removed this entry, so this run gates nothing"
    if not own_passed:
        post_check(repo, sha, GATE, "failure", "A job of this queue entry failed.", run_url)
        return f"failed the merge gate of #{own['number']}"
    post_check(repo, sha, CHECKED, "success", "Every job of this queue entry passed.", run_url)
    if own_role == "tail":
        held = batch(entries, own)
        summary = f"The batch release build of {numbers(held)} passed."
        for entry in reversed(held):
            post_check(repo, entry["oid"], BUILT, "success", summary, run_url)
        gated = [entry for entry in reversed(held) if entry is own or passed(repo, entry["oid"], CHECKED)]
        for entry in gated:
            post_check(repo, entry["oid"], GATE, "success", summary, run_url)
        return f"built {numbers(held)} and passed the merge gate of {numbers(gated[::-1])}"
    if passed(repo, sha, BUILT):
        post_check(repo, sha, GATE, "success", "A batch release build behind this entry covered it.", run_url)
        return f"#{own['number']} was already built by the batch: passed its merge gate"
    return f"#{own['number']} waits for the batch release build of an entry behind it"


def create_branch(repo: str, branch: str, sha: str) -> None:
    try:
        api(["--method", "POST", f"repos/{repo}/git/refs", "-f", f"ref=refs/heads/{branch}", "-f", f"sha={sha}"])
    except RuntimeError:
        if api([f"repos/{repo}/git/ref/heads/{branch}"])["object"]["sha"] != sha:
            raise


def open_pull(repo: str, branch: str, title: str, body: str) -> int:
    owner = repo.split("/")[0]
    existing = api(["--method", "GET", f"repos/{repo}/pulls", "-f", f"head={owner}:{branch}", "-f", "state=open"])
    if existing:
        return existing[0]["number"]
    created = api(
        [
            "--method",
            "POST",
            f"repos/{repo}/pulls",
            "-f",
            f"title={title}",
            "-f",
            f"head={branch}",
            "-f",
            "base=main",
            "-f",
            f"body={body}",
        ]
    )
    return created["number"]


def failure_report(repo: str, run_id: str, run_url: str) -> str:
    jobs = failed_jobs(api([f"repos/{repo}/actions/runs/{run_id}/jobs?per_page=100"])["jobs"])
    failed = [(job, errors(api([f"repos/{repo}/check-runs/{job['id']}/annotations"]))) for job in jobs]
    return comment(failed, run_url)


def combine(repo: str, sha: str, run_id: str, run_url: str) -> int | None:
    entries = live_entries(repo)
    own = own_entry(entries, sha)
    if own is None:
        print("stale: GitHub rebuilt or removed this entry, so this run combines nothing")
        return None
    held = batch(entries, own)
    if len(held) < 2:
        print(f"#{own['number']} was alone: it goes back on its own")
        return None
    if own["branch"].startswith(BATCH_BRANCH):
        print(f"#{own['number']} already combines a batch: it goes back on its own")
        return None
    branch = f"{BATCH_BRANCH}{sha[:7]}"
    create_branch(repo, branch, sha)
    number = open_pull(repo, branch, combined_title(held), combined_body(held, failure_report(repo, run_id, run_url)))
    note = "\n".join(
        [
            "> [!NOTE]",
            f"> **Batched into #{number}** · the batch release build of {numbers(held)} failed.",
            f"> Push fixes to #{number}; this pull request stays closed.",
        ]
    )
    for entry in reversed(held):
        api(["--method", "POST", f"repos/{repo}/issues/{entry['number']}/comments", "-f", f"body={note}"])
        api(["--method", "PATCH", f"repos/{repo}/pulls/{entry['number']}", "-f", "state=closed"])
    print(f"combined {numbers(held)} into #{number}")
    return number


def queue_run(repo: str, sha: str) -> dict | None:
    runs = api(
        [
            "--method",
            "GET",
            f"repos/{repo}/actions/workflows/ci.yml/runs",
            "-f",
            f"head_sha={sha}",
            "-f",
            "event=merge_group",
        ]
    )["workflow_runs"]
    return max(runs, key=lambda run: run["id"]) if runs else None


def reconcile(repo: str, run_url: str) -> str:
    building = [entry for entry in live_entries(repo) if entry["state"] in BUILDING and entry["oid"]]
    notes = []
    for entry in building:
        oid = entry["oid"]
        if passed(repo, oid, BUILT) and passed(repo, oid, CHECKED) and not passed(repo, oid, GATE):
            post_check(repo, oid, GATE, "success", "A batch release build behind this entry covered it.", run_url)
            notes.append(f"passed the merge gate of #{entry['number']}")
    tails = [entry for entry in building if latest_check(repo, entry["oid"], GATE) != "failure"]
    if not tails:
        return "; ".join(notes + ["no entry is building"])
    tail = tails[-1]
    notes.append(f"#{tail['number']} {retail(repo, tail)}")
    return "; ".join(notes)


def retail(repo: str, tail: dict) -> str:
    if passed(repo, tail["oid"], BUILT):
        return "is built"
    run = queue_run(repo, tail["oid"])
    if run is None or run.get("status") != "completed":
        return "is still running"
    if run.get("run_attempt", 1) >= MAX_ATTEMPTS:
        return f"ran {run['run_attempt']} times: left to the check timeout"
    jobs = api([f"repos/{repo}/actions/runs/{run['id']}/jobs?per_page=100"])["jobs"]
    if any(job["name"].startswith(RELEASE_JOB_PREFIX) and job.get("conclusion") not in (None, "skipped") for job in jobs):
        return "already ran its release build"
    if not passed(repo, tail["oid"], CHECKED):
        return "did not pass its own jobs"
    role_job = next((job for job in jobs if job["name"] == BATCH_JOB), None)
    if role_job is None:
        return "has no batch role job"
    api(["--method", "POST", f"repos/{repo}/actions/jobs/{role_job['id']}/rerun"])
    return "is the tail now: reran its batch role to build the batch"


def write_output(name: str, value: str) -> None:
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as handle:
        handle.write(f"{name}={value}\n")


def main() -> int:
    command = sys.argv[1] if len(sys.argv) > 1 else ""
    repo = os.environ["GITHUB_REPOSITORY"]
    sha = os.environ.get("GITHUB_SHA", "")
    run_id = os.environ["GITHUB_RUN_ID"]
    run_url = f"{os.environ['GITHUB_SERVER_URL']}/{repo}/actions/runs/{run_id}"
    try:
        if command == "role":
            decided = decide(repo, sha)
        elif command == "gate":
            print(gate(repo, sha, os.environ["VERDICT"] == "success", os.environ.get("ROLE", ""), run_url))
            return 0
        elif command == "combine":
            number = combine(repo, sha, run_id, run_url)
            write_output("combined", str(number or ""))
            return 0
        elif command == "reconcile":
            print(reconcile(repo, run_url))
            return 0
        else:
            print(f"usage: {sys.argv[0]} role|gate|combine|reconcile", file=sys.stderr)
            return 2
    except (RuntimeError, KeyError, ValueError, json.JSONDecodeError) as error:
        if command != "role":
            print(f"::error::merge queue batch {command} failed: {error}", file=sys.stderr)
            return 1
        print(f"::warning::building the release here, the queue could not be read: {error}", file=sys.stderr)
        decided = "tail"
    write_output("role", decided)
    print(f"role={decided}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
