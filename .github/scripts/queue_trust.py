#!/usr/bin/env python3
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

import affected_crates
from queue_verdict import gh_json, queued_pull

SCRIPTS = Path(__file__).resolve().parent
ARTIFACT = "tested-base"
RECORD = "tested-base.json"
GATE_JOB = "merge gate"


class Untrusted(Exception):
    pass


def affected_plan(base: str, head: str) -> dict:
    with tempfile.TemporaryDirectory() as scratch:
        path = Path(scratch) / "plan.json"
        env = {key: value for key, value in os.environ.items() if key != "GITHUB_OUTPUT"}
        env.update(BASE_SHA=base, HEAD_SHA=head, QOL_AFFECTED_OUTPUT=str(path))
        result = subprocess.run(
            [sys.executable, str(SCRIPTS / "affected_crates.py")],
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )
        sys.stderr.write(result.stderr)
        if result.returncode != 0:
            raise RuntimeError(f"affected_crates.py exited {result.returncode}")
        return json.loads(path.read_text())


def planned_crates(plan: dict) -> set[str] | None:
    if plan.get("full") is not False or not isinstance(plan.get("crates"), str):
        return None
    return set(plan["crates"].split())


def pull_head(repo: str, number: int) -> str:
    return gh_json([f"repos/{repo}/pulls/{number}"])["head"]["sha"]


def passed_pull_run(repo: str, head: str) -> dict | None:
    runs = gh_json(
        [
            "--method",
            "GET",
            f"repos/{repo}/actions/workflows/ci.yml/runs",
            "-f",
            f"head_sha={head}",
            "-f",
            "event=pull_request",
            "-f",
            "status=success",
            "-f",
            "per_page=20",
        ]
    )["workflow_runs"]
    for run in sorted(runs, key=lambda run: run["id"], reverse=True):
        jobs = gh_json([f"repos/{repo}/actions/runs/{run['id']}/jobs?per_page=100"])["jobs"]
        if any(job.get("name") == GATE_JOB and job.get("conclusion") == "success" for job in jobs):
            return run
    return None


def tested_record(repo: str, run_id: int) -> dict | None:
    with tempfile.TemporaryDirectory() as scratch:
        result = subprocess.run(
            ["gh", "run", "download", str(run_id), "--repo", repo, "--name", ARTIFACT, "--dir", scratch],
            capture_output=True,
            text=True,
            check=False,
        )
        path = Path(scratch) / RECORD
        if result.returncode != 0 or not path.is_file():
            return None
        return json.loads(path.read_text())


def fallback(reason: str, base: str, head: str) -> dict:
    print(f"[trust] running every affected job: {reason}", file=sys.stderr)
    return {**affected_plan(base, head), "trusted": False}


def trusted_plan(repo: str, head_ref: str, queue_base: str) -> dict:
    number = queued_pull(head_ref)
    if number is None:
        raise Untrusted(f"{head_ref} names no pull request")
    head = pull_head(repo, number)
    run = passed_pull_run(repo, head)
    if run is None:
        raise Untrusted(f"no pull request run passed the merge gate for {head}")
    record = tested_record(repo, run["id"])
    if not record or record.get("head") != head or not isinstance(record.get("base"), str):
        raise Untrusted(f"pull request run {run['id']} recorded no tested base for {head}")
    tested = planned_crates(record.get("plan") or {})
    if tested is None:
        raise Untrusted(f"pull request run {run['id']} planned the full workspace")
    landed = planned_crates(affected_plan(record["base"], queue_base))
    if landed is None:
        raise Untrusted(f"a change that landed since {record['base']} reaches the full workspace")
    pkgs = affected_crates.workspace_graph()
    if not pkgs:
        raise Untrusted("cargo metadata unavailable")
    overlap = (
        affected_crates.dependents_closure(tested, pkgs)
        & affected_crates.dependents_closure(landed, pkgs)
        & set(pkgs)
    )
    print(
        f"[trust] pull request run {run['id']} tested {sorted(tested)} on {record['base']}; "
        f"landed since reaches {sorted(landed)}; overlap {sorted(overlap)}",
        file=sys.stderr,
    )
    return {**affected_crates.affected_outputs(overlap, pkgs), "trusted": not overlap}


def decide(repo: str, head_ref: str, queue_base: str, queue_head: str) -> dict:
    try:
        return trusted_plan(repo, head_ref, queue_base)
    except Untrusted as reason:
        return fallback(str(reason), queue_base, queue_head)
    except (RuntimeError, KeyError, TypeError, ValueError, AttributeError) as error:
        return fallback(f"the pull request run could not be read: {error}", queue_base, queue_head)


def record(head: str, base: str, plan_path: str, out_path: str) -> None:
    out = Path(out_path)
    out.parent.mkdir(parents=True, exist_ok=True)
    plan = json.loads(Path(plan_path).read_text())
    out.write_text(json.dumps({"head": head, "base": base, "plan": plan}, indent=2, sort_keys=True) + "\n")


def main() -> int:
    if sys.argv[1:2] == ["record"]:
        if len(sys.argv) != 6:
            print(f"usage: {sys.argv[0]} record <head> <base> <plan.json> <out.json>", file=sys.stderr)
            return 2
        record(*sys.argv[2:6])
        return 0
    affected_crates.emit(
        decide(
            os.environ["GITHUB_REPOSITORY"],
            os.environ["HEAD_REF"],
            os.environ["BASE_SHA"],
            os.environ["HEAD_SHA"],
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
