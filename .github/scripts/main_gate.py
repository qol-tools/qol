#!/usr/bin/env python3

import json
import os
import re
import sys
import time
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path

from live_test import paths_of, pattern_regex, patterns
from queue_verdict import gh_json

CHECK_NAME = "main gate"
OWNER = "KMRH47"
PATHS_FILE = Path(__file__).resolve().parents[1] / "main-gate-paths"
LIST_NAME = "`.github/main-gate-paths`"
SHA = re.compile(r"[0-9a-f]{40}")
CONTROL = re.compile(r"[\x00-\x1f\x7f]")
APPROVAL = re.compile(r"/gate-approve ([0-9a-f]{40})")
QUEUE_BRANCH = re.compile(r"gh-readonly-queue/main/pr-(\d+)-[0-9a-f]{40}")
PULL_FILES_CAP = 3000
COMPARE_FILES_CAP = 300
QUEUE_CAP = 100
LISTED_FILES = 50
MAX_ATTEMPTS = 3
RETRY_SECONDS = 5
SUCCESS = "success"
FAILURE = "failure"

COMMENTS_QUERY = """
query($owner: String!, $name: String!, $number: Int!, $cursor: String) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      comments(first: 100, after: $cursor) {
        pageInfo { hasNextPage endCursor }
        nodes { body url author { login } lastEditedAt editor { login } includesCreatedEdit }
      }
    }
  }
}
"""

QUEUE_QUERY = f"""
query($owner: String!, $name: String!) {{
  repository(owner: $owner, name: $name) {{
    mergeQueue(branch: "main") {{
      entries(first: {QUEUE_CAP}) {{
        pageInfo {{ hasNextPage }}
        nodes {{
          position
          headCommit {{ oid }}
          pullRequest {{ number headRefOid changedFiles }}
        }}
      }}
    }}
  }}
}}
"""

OBJECT_QUERY = """
query($owner: String!, $name: String!, $expression: String!) {
  repository(owner: $owner, name: $name) {
    object(expression: $expression) { oid }
  }
}
"""


class Unverifiable(Exception):
    pass


FAILURES = (Unverifiable, RuntimeError, KeyError, TypeError, ValueError, json.JSONDecodeError)


@dataclass(frozen=True)
class Verdict:
    sha: str
    conclusion: str
    title: str
    summary: str


@dataclass
class Review:
    number: int
    head: str
    files: list[str]
    approval: str | None = None
    found: int = 0

    @property
    def passes(self) -> bool:
        return not self.files or self.approval is not None


@dataclass
class GitHub:
    repo: str
    call: Callable[[list[str]], dict | list] = gh_json
    sleep: Callable[[float], None] = time.sleep
    blobs: dict[tuple[str, str], str | None] = field(default_factory=dict)

    def get(self, args: list[str]) -> dict | list:
        for attempt in range(MAX_ATTEMPTS):
            try:
                return self.call(args)
            except RuntimeError:
                if attempt == MAX_ATTEMPTS - 1:
                    raise
                self.sleep(RETRY_SECONDS)
        raise AssertionError("unreachable")

    def graphql(self, query: str, *fields: str) -> dict:
        owner, name = self.repo.split("/")
        data = self.get(["graphql", "-f", f"query={query}", "-f", f"owner={owner}", "-f", f"name={name}", *fields])
        if data.get("errors"):
            raise RuntimeError(json.dumps(data["errors"]))
        return data["data"]["repository"]

    def pull(self, number: int) -> dict:
        return self.get([f"repos/{self.repo}/pulls/{number}"])

    def pulls_of_commit(self, sha: str) -> list[dict]:
        return self.get([f"repos/{self.repo}/commits/{sha}/pulls?per_page=100"])

    def pulls_of_branch(self, owner: str, branch: str) -> list[dict]:
        return self.get(
            ["--method", "GET", f"repos/{self.repo}/pulls", "-f", "state=open", "-f", f"head={owner}:{branch}", "-f", "per_page=100"]
        )

    def pull_files(self, number: int) -> list[dict]:
        pages = self.get(["--paginate", "--slurp", f"repos/{self.repo}/pulls/{number}/files?per_page=100"])
        return [entry for page in pages for entry in page]

    def compare_files(self, base: str, head: str) -> list[dict]:
        return self.get([f"repos/{self.repo}/compare/{base}...{head}"])["files"]

    def comments(self, number: int) -> list[dict]:
        found, cursor = [], None
        while True:
            fields = ["-F", f"number={number}", *(["-f", f"cursor={cursor}"] if cursor else [])]
            pull = self.graphql(COMMENTS_QUERY, *fields)["pullRequest"]
            if pull is None:
                raise Unverifiable(f"#{number} is not a pull request of {self.repo}")
            page = pull["comments"]
            found.extend(page["nodes"])
            if not page["pageInfo"]["hasNextPage"]:
                return found
            cursor = page["pageInfo"]["endCursor"]

    def queue_entries(self) -> list[dict]:
        queue = self.graphql(QUEUE_QUERY)["mergeQueue"]
        if queue is None:
            raise Unverifiable("main has no merge queue")
        if queue["entries"]["pageInfo"]["hasNextPage"]:
            raise Unverifiable(f"the merge queue holds more than {QUEUE_CAP} entries")
        return [
            {
                "position": node["position"],
                "oid": (node.get("headCommit") or {}).get("oid"),
                "number": node["pullRequest"]["number"],
                "head": node["pullRequest"]["headRefOid"],
                "changed_files": node["pullRequest"]["changedFiles"],
            }
            for node in queue["entries"]["nodes"]
        ]

    def blob(self, sha: str, path: str) -> str | None:
        key = (sha, path)
        if key not in self.blobs:
            found = self.graphql(OBJECT_QUERY, "-f", f"expression={sha}:{path}")["object"]
            self.blobs[key] = found["oid"] if found else None
        return self.blobs[key]

    def post(self, verdict: Verdict, url: str) -> None:
        self.get(
            [
                "--method",
                "POST",
                f"repos/{self.repo}/check-runs",
                "-f",
                f"name={CHECK_NAME}",
                "-f",
                f"head_sha={verdict.sha}",
                "-f",
                "status=completed",
                "-f",
                f"conclusion={verdict.conclusion}",
                "-f",
                f"details_url={url}",
                "-f",
                f"output[title]={verdict.title}",
                "-f",
                f"output[summary]={verdict.summary}",
            ]
        )


def protected(paths: list[str], listed: list[str]) -> list[str]:
    unreadable = [path for path in paths if CONTROL.search(path)]
    if unreadable:
        raise Unverifiable(f"{len(unreadable)} changed paths hold control characters, such as {unreadable[0]!r}")
    compiled = [pattern_regex(pattern) for pattern in listed]
    return list(dict.fromkeys(path for path in paths if any(regex.match(path) for regex in compiled)))


def is_approval(comment: dict, head: str) -> bool:
    if (comment.get("author") or {}).get("login") != OWNER:
        return False
    if comment.get("lastEditedAt", "missing") is not None or comment.get("editor", "missing") is not None:
        return False
    if comment.get("includesCreatedEdit", True):
        return False
    named = APPROVAL.fullmatch((comment.get("body") or "").strip())
    return bool(named) and named.group(1) == head


def approval_of(comments: list[dict], head: str) -> str | None:
    return next((comment.get("url") or "an unedited comment" for comment in comments if is_approval(comment, head)), None)


def review(api: GitHub, number: int, head: str, expected: int | None, listed: list[str]) -> Review:
    if not SHA.fullmatch(head or ""):
        raise Unverifiable(f"#{number} has no head commit to judge")
    entries = api.pull_files(number)
    if not isinstance(expected, int) or len(entries) < expected or len(entries) >= PULL_FILES_CAP:
        raise Unverifiable(f"#{number} changes {expected} files and the API listed {len(entries)}, so the gate cannot see every changed file")
    files = protected(paths_of(entries), listed)
    approval = approval_of(api.comments(number), head) if files else None
    return Review(number, head, files, approval, len(entries))


def resolve_pulls(api: GitHub, sha: str, owner: str, branch: str) -> list[dict]:
    try:
        candidates = list(api.pulls_of_commit(sha))
    except RuntimeError:
        candidates = []
    if owner and branch:
        candidates.extend(api.pulls_of_branch(owner, branch))
    pulls = []
    for number in sorted({candidate["number"] for candidate in candidates}):
        pull = api.pull(number)
        if pull.get("state") == "open" and pull["head"]["sha"] == sha and pull["base"]["repo"]["full_name"] == api.repo:
            pulls.append(pull)
    return pulls


def bullets(paths: list[str]) -> list[str]:
    shown = [f"- `{path}`" for path in paths[:LISTED_FILES]]
    if len(paths) > LISTED_FILES:
        shown.append(f"- and {len(paths) - LISTED_FILES} more")
    return shown


def asking(item: Review) -> list[str]:
    return [
        f"#{item.number} changes these files on {LIST_NAME}, which decide the review or the gate itself:",
        "",
        *bullets(item.files),
        "",
        f"The owner approves its head commit by posting this line, alone, as a new comment on #{item.number}:",
        "",
        f"    /gate-approve {item.head}",
        "",
    ]


def granted(item: Review) -> list[str]:
    return [f"#{item.number} changes {len(item.files)} protected files; @{OWNER} approved `{item.head[:7]}`: {item.approval}", ""]


def closing() -> list[str]:
    return ["Only an unedited comment by the owner counts, and a new push needs a new approval."]


def pull_verdict(sha: str, reviews: list[Review]) -> Verdict:
    if all(not item.files for item in reviews):
        counted = sum(item.found for item in reviews)
        return Verdict(sha, SUCCESS, "No protected path changed", f"None of the {counted} changed files is on {LIST_NAME}.")
    if all(item.passes for item in reviews):
        lines = [line for item in reviews if item.files for line in granted(item)]
        return Verdict(sha, SUCCESS, "The owner approved the protected paths", "\n".join(lines).strip())
    lines = [line for item in reviews for line in (granted(item) if item.passes and item.files else asking(item) if item.files else [])]
    return Verdict(sha, FAILURE, "Protected paths need the owner's approval", "\n".join([*lines, *closing()]))


def queue_verdict(api: GitHub, sha: str, branch: str, listed: list[str]) -> Verdict:
    named = QUEUE_BRANCH.fullmatch(branch or "")
    if not named:
        raise Unverifiable(f"`{branch}` is not a merge queue branch of main")
    files = api.compare_files("main", sha)
    if len(files) >= COMPARE_FILES_CAP:
        raise Unverifiable(f"the merge group changes {COMPARE_FILES_CAP} or more files, more than the compare API lists")
    changed = protected(paths_of(files), listed)
    if not changed:
        return Verdict(sha, SUCCESS, "No protected path changed", f"None of the {len(files)} files this merge group changes is on {LIST_NAME}.")
    entries = api.queue_entries()
    own = next((entry for entry in entries if entry["oid"] == sha and entry["number"] == int(named.group(1))), None)
    if own is None:
        raise Unverifiable(f"no merge queue entry of #{named.group(1)} has the commit `{sha[:7]}`")
    group = sorted((entry for entry in entries if entry["position"] <= own["position"]), key=lambda entry: entry["position"])
    reviews = [review(api, entry["number"], entry["head"], entry["changed_files"], listed) for entry in group]
    unbound = [path for path in changed if not bound(api, sha, path, reviews)]
    if all(item.passes for item in reviews) and not unbound:
        lines = [line for item in reviews if item.files for line in granted(item)]
        return Verdict(sha, SUCCESS, "The owner approved the protected paths", "\n".join(lines).strip())
    lines = [line for item in reviews if not item.passes for line in asking(item)]
    if unbound:
        lines.extend(
            [
                "These protected files in the merge group match no approved head commit that changes them:",
                "",
                *bullets(unbound),
                "",
                "Update the branch that changes them, approve its new head, and queue it again.",
                "",
            ]
        )
    return Verdict(sha, FAILURE, "Protected paths need the owner's approval", "\n".join([*lines, *closing()]))


def bound(api: GitHub, sha: str, path: str, reviews: list[Review]) -> bool:
    return any(item.approval and path in item.files and api.blob(item.head, path) == api.blob(sha, path) for item in reviews)


def refused(sha: str, reason: str) -> Verdict:
    return Verdict(sha, FAILURE, "The gate could not verify this commit", f"{reason}.\n\nThe gate refuses what it cannot verify. Rerun the main gate run once the cause is gone.")


def guarded(sha: str, decide: Callable[[], Verdict]) -> Verdict:
    if not SHA.fullmatch(sha or ""):
        raise Unverifiable(f"`{sha}` is not a commit to judge")
    try:
        return decide()
    except FAILURES as error:
        return refused(sha, str(error).rstrip("."))


def judge(env: dict, api: GitHub, listed: list[str]) -> Verdict | None:
    if env.get("GITHUB_EVENT_NAME") == "issue_comment":
        pull = api.pull(int(env["PR_NUMBER"]))
        if pull.get("state") != "open":
            return None
        sha = pull["head"]["sha"]
        return guarded(sha, lambda: pull_verdict(sha, [review(api, pull["number"], sha, pull.get("changed_files"), listed)]))
    run_event, sha = env.get("RUN_EVENT"), env.get("HEAD_SHA", "")
    if run_event == "pull_request":

        def decide() -> Verdict:
            pulls = resolve_pulls(api, sha, env.get("HEAD_OWNER", ""), env.get("HEAD_BRANCH", ""))
            if not pulls:
                raise Unverifiable(f"no open pull request of {api.repo} has the head commit `{sha[:7]}`")
            return pull_verdict(sha, [review(api, pull["number"], sha, pull.get("changed_files"), listed) for pull in pulls])

        return guarded(sha, decide)
    if run_event == "merge_group":
        return guarded(sha, lambda: queue_verdict(api, sha, env.get("HEAD_BRANCH", ""), listed))
    return None


def report(verdict: Verdict, summary: str | None) -> None:
    print(f"{CHECK_NAME} for {verdict.sha}: {verdict.conclusion}, {verdict.title}")
    print(verdict.summary)
    if summary:
        with open(summary, "a", encoding="utf-8") as handle:
            handle.write(f"### {CHECK_NAME}: {verdict.conclusion}\n\n**{verdict.title}** for `{verdict.sha}`\n\n{verdict.summary}\n")


def evaluate(env: dict, api: GitHub, listed: list[str] | None = None) -> int:
    listed = listed if listed is not None else patterns(PATHS_FILE.read_text(encoding="utf-8"))
    try:
        verdict = judge(env, api, listed)
    except FAILURES as error:
        print(f"::error::{error}")
        return 1
    if verdict is None:
        print("nothing for the main gate to judge")
        return 0
    report(verdict, env.get("GITHUB_STEP_SUMMARY"))
    Path(env["VERDICT_FILE"]).write_text(json.dumps(verdict.__dict__), encoding="utf-8")
    return 0


def post(env: dict, api: GitHub) -> int:
    path = Path(env["VERDICT_FILE"])
    if not path.exists():
        print("no verdict to post")
        return 0
    verdict = Verdict(**json.loads(path.read_text(encoding="utf-8")))
    if not SHA.fullmatch(verdict.sha) or verdict.conclusion not in (SUCCESS, FAILURE):
        print(f"::error::the verdict file holds no verdict: {verdict}")
        return 1
    try:
        api.post(verdict, env.get("RUN_URL", ""))
    except FAILURES as error:
        print(f"::error::{error}")
        return 1
    print(f"posted {CHECK_NAME} {verdict.conclusion} on {verdict.sha}")
    return 0


def main(argv: list[str]) -> int:
    env = os.environ
    command = argv[1] if len(argv) > 1 else ""
    if command == "evaluate":
        return evaluate(env, GitHub(env["GITHUB_REPOSITORY"]))
    if command == "post":
        return post(env, GitHub(env["GITHUB_REPOSITORY"]))
    print("usage: main_gate.py evaluate|post", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
