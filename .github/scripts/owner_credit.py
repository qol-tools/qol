#!/usr/bin/env python3
"""Refuse a commit that GitHub credits to the agent account and not to the owner.

The agent account opens the pull requests, so the squash commit the merge queue
lands is its commit. The owner is credited only through the Co-authored-by line
GitHub adds for each author of the pull request's commits. A pull request run
predicts the credit from those authors; a merge queue run reads the accounts
GitHub credits on the commit that lands.
"""

import os
import sys
import time

from queue_verdict import gh_json

OWNER = "KMRH47"
AGENT = "kmrh47-agents"
ATTEMPTS = 3
RETRY_SECONDS = 5

PULL_QUERY = """
query($owner: String!, $name: String!, $number: Int!, $after: String) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      author { login }
      commits(first: 100, after: $after) {
        pageInfo { hasNextPage endCursor }
        nodes { commit { author { user { login } } } }
      }
    }
  }
}
"""

COMMIT_QUERY = """
query($owner: String!, $name: String!, $oid: GitObjectID!) {
  repository(owner: $owner, name: $name) {
    object(oid: $oid) {
      ... on Commit { authors(first: 100) { nodes { user { login } } } }
    }
  }
}
"""


def logins(nodes: list[dict | None]) -> list[str]:
    found = []
    for node in nodes:
        login = ((node or {}).get("user") or {}).get("login")
        if login and login not in found:
            found.append(login)
    return found


def owner_missing(credited: list[str]) -> bool:
    names = {login.lower() for login in credited}
    return AGENT.lower() in names and OWNER.lower() not in names


def pull_credit(pages: list[dict]) -> list[str]:
    nodes = []
    for page in pages:
        nodes.append({"user": page.get("author")})
        nodes.extend(node["commit"]["author"] for node in page["commits"]["nodes"])
    return logins(nodes)


def commit_credit(commit: dict | None) -> list[str]:
    return logins(((commit or {}).get("authors") or {}).get("nodes") or [])


def verdict(credited: list[str], landing: bool) -> tuple[bool, str]:
    shown = ", ".join(credited) or "no GitHub account"
    if not owner_missing(credited):
        return True, f"credited: {shown}"
    if landing:
        lead = f"this commit lands credited to {shown}, without {OWNER}"
    else:
        lead = f"this pull request would land credited to {shown}, without {OWNER}"
    fix = (
        f"Push a commit whose author email belongs to the {OWNER} account "
        "(`git commit --allow-empty` is enough), so the squash commit names "
        f"{OWNER} as co-author."
    )
    return False, f"{lead}. {fix}"


def graphql(query: str, fields: dict[str, str | int | None]) -> dict:
    args = ["graphql", "-f", f"query={query}"]
    for key, value in fields.items():
        if value is None:
            continue
        args += ["-F" if isinstance(value, int) else "-f", f"{key}={value}"]
    failure = None
    for attempt in range(ATTEMPTS):
        if attempt:
            time.sleep(RETRY_SECONDS)
        try:
            return gh_json(args)["data"]["repository"]
        except (RuntimeError, KeyError, TypeError) as error:
            failure = error
    raise RuntimeError(f"GitHub did not return the credit: {failure}")


def pull_pages(owner: str, name: str, number: int) -> list[dict]:
    pages = []
    after = None
    while True:
        pull = graphql(PULL_QUERY, {"owner": owner, "name": name, "number": number, "after": after})["pullRequest"]
        pages.append(pull)
        info = pull["commits"]["pageInfo"]
        if not info["hasNextPage"]:
            return pages
        after = info["endCursor"]


def main() -> int:
    owner, name = os.environ["REPO"].split("/", 1)
    sha = os.environ.get("HEAD_SHA", "")
    if sha:
        commit = graphql(COMMIT_QUERY, {"owner": owner, "name": name, "oid": sha})["object"]
        if not commit:
            print(f"::error::GitHub has no commit {sha} to read the credit from")
            return 1
        credited = commit_credit(commit)
    else:
        credited = pull_credit(pull_pages(owner, name, int(os.environ["PR_NUMBER"])))
    passed, text = verdict(credited, bool(sha))
    print(text if passed else f"::error::{text}")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
