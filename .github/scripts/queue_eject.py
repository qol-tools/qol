#!/usr/bin/env python3

import json
import os
import sys

import queue_batch as qb

DEQUEUE = "mutation($id: ID!) { dequeuePullRequest(input: {id: $id}) { clientMutationId } }"


def verdict(repo: str, oid: str) -> str:
    if qb.latest_check(repo, oid, qb.GATE) == "failure":
        return "failed"
    if qb.passed(repo, oid, qb.CHECKED):
        return "passed"
    return "pending"


def judge(repo: str, entries: list[dict], sha: str) -> tuple[str, dict | None]:
    by_oid = {entry["oid"]: entry for entry in entries if entry["oid"]}
    own = by_oid.get(sha)
    if own is None:
        return "gone", None
    ahead = by_oid.get(own["base"])
    if ahead is None:
        return "own", None
    state = verdict(repo, ahead["oid"])
    return {"passed": "own", "failed": "inherited"}.get(state, "pending"), ahead


def dequeue(repo: str, number: int) -> str:
    node = qb.api([f"repos/{repo}/pulls/{number}"])["node_id"]
    qb.api(["graphql", "-f", f"query={DEQUEUE}", "-f", f"id={node}"])
    return f"dequeued #{number}: its own change failed, so the entries behind it rebuild now"


def sweep(repo: str) -> str:
    entries = qb.live_entries(repo)
    for entry in entries:
        if not entry["oid"] or verdict(repo, entry["oid"]) != "failed":
            continue
        cause, ahead = judge(repo, entries, entry["oid"])
        if cause == "own":
            return dequeue(repo, entry["number"])
        why = "failed first" if cause == "inherited" else "is still running"
        return f"#{entry['number']} failed, but #{ahead['number']} ahead of it {why}: left queued"
    return "no failed entry"


def main() -> int:
    try:
        print(sweep(os.environ["GITHUB_REPOSITORY"]))
    except (RuntimeError, KeyError, json.JSONDecodeError) as error:
        print(f"::warning::no merge queue sweep: {error}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
