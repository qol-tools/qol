import importlib.util
import io
import json
import re
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
ROOT = SCRIPTS.parents[1]
sys.path.insert(0, str(SCRIPTS))
_SPEC = importlib.util.spec_from_file_location("main_gate", SCRIPTS / "main_gate.py")
gate = importlib.util.module_from_spec(_SPEC)
sys.modules["main_gate"] = gate
_SPEC.loader.exec_module(gate)

LISTED = gate.patterns(gate.PATHS_FILE.read_text(encoding="utf-8"))
WORKFLOW = (ROOT / ".github/workflows/main-gate.yml").read_text(encoding="utf-8")
REPO = "o/r"
HEAD = "a" * 40
OLD = "b" * 40
GROUP = "c" * 40
AHEAD = "d" * 40
BEHIND = "e" * 40
QUEUE_BRANCH = f"gh-readonly-queue/main/pr-7-{'f' * 40}"


def approval(sha: str = HEAD, login: str = "KMRH47", body: str | None = None, **changes) -> dict:
    honest = {
        "body": body if body is not None else f"/gate-approve {sha}",
        "url": "https://github.com/o/r/pull/7#issuecomment-1",
        "author": {"login": login},
        "lastEditedAt": None,
        "editor": None,
        "includesCreatedEdit": False,
    }
    return {**honest, **changes}


def pull(number: int = 7, head: str = HEAD, files: int | None = None, state: str = "open", base: str = REPO) -> dict:
    return {"number": number, "state": state, "head": {"sha": head}, "base": {"repo": {"full_name": base}}, "changed_files": files}


def entries(*paths: str) -> list[dict]:
    return [{"filename": path} for path in paths]


class FakeApi:
    repo = REPO

    def __init__(self, pulls=(), files=None, comments=None, commit_pulls=None, branch_pulls=None, compare=None, queue=None, blobs=None):
        self.pulls = {item["number"]: item for item in pulls}
        self.files = files or {}
        for number, listed in self.files.items():
            if number in self.pulls and self.pulls[number]["changed_files"] is None:
                self.pulls[number]["changed_files"] = len(listed)
        self.comment_lists = comments or {}
        self.commit_pulls = commit_pulls if commit_pulls is not None else [{"number": number} for number in self.pulls]
        self.branch_pulls = branch_pulls or []
        self.compared = compare or []
        self.queue = queue or []
        self.blob_map = blobs or {}
        self.calls = []
        self.posted = []

    def pull(self, number):
        if number not in self.pulls:
            raise RuntimeError(f"HTTP 404 pull {number}")
        return self.pulls[number]

    def pulls_of_commit(self, sha):
        return self.commit_pulls

    def pulls_of_branch(self, owner, branch):
        self.calls.append(("branch", owner, branch))
        return self.branch_pulls

    def pull_files(self, number):
        return entries(*self.files.get(number, []))

    def comments(self, number):
        return self.comment_lists.get(number, [])

    def compare_files(self, base, head):
        self.calls.append(("compare", base, head))
        return entries(*self.compared)

    def queue_entries(self):
        self.calls.append(("queue",))
        return self.queue

    def blob(self, sha, path):
        return self.blob_map.get((sha, path), f"{path}@{sha}")

    def post(self, verdict, url):
        self.posted.append((verdict, url))


def pull_env(**changes) -> dict:
    return {"GITHUB_EVENT_NAME": "workflow_run", "RUN_EVENT": "pull_request", "HEAD_SHA": HEAD, "HEAD_OWNER": "o", "HEAD_BRANCH": "topic", **changes}


def queue_env(**changes) -> dict:
    return {"GITHUB_EVENT_NAME": "workflow_run", "RUN_EVENT": "merge_group", "HEAD_SHA": GROUP, "HEAD_BRANCH": QUEUE_BRANCH, **changes}


def comment_env(number: int = 7) -> dict:
    return {"GITHUB_EVENT_NAME": "issue_comment", "PR_NUMBER": str(number)}


def queue_entry(number: int, position: int, oid: str, head: str, files: int) -> dict:
    return {"number": number, "position": position, "oid": oid, "head": head, "changed_files": files}


class PathTests(unittest.TestCase):
    def test_the_repository_list_protects_what_decides_the_required_checks(self):
        guarded = [
            ".github/workflows/ci.yml",
            ".github/workflows/claude-review.yml",
            ".github/scripts/review_gate.py",
            ".github/actions/rust-setup/action.yml",
            ".github/main-gate-paths",
            ".githooks/commit-msg",
            ".githooks/commit-msg-lib.sh",
            ".githooks/single-source-guard.mjs",
            "verify/repository-inventory.mjs",
            "verify/repository-inventory.test.mjs",
            ".qac/judges.json",
            ".cargo/config.toml",
            ".config/nextest.toml",
            ".config/hakari.toml",
            "rust-toolchain.toml",
            "rustfmt.toml",
            "clippy.toml",
            ".claude/settings.json",
            ".mcp.json",
            "CLAUDE.md",
            "plugins/shot/CLAUDE.md",
            "apps/tray/CLAUDE.local.md",
        ]
        free = [
            "Cargo.toml",
            "Cargo.lock",
            "deny.toml",
            "README.md",
            "plugins/shot/src/lib.rs",
            "apps/tray/build.rs",
            "libs/conventions/src/lib.rs",
            "verify/sessions-relay-verify.sh",
            "docs/github/ci.md",
            "plugins/shot/CLAUDE.md.bak",
        ]
        self.assertEqual(gate.protected([*guarded, *free], LISTED), guarded)

    def test_a_rename_counts_its_old_path(self):
        api = FakeApi([pull()])
        api.files[7] = []
        api.pull_files = lambda number: [{"filename": "docs/ci.yml", "previous_filename": ".github/workflows/ci.yml"}]
        api.pulls[7]["changed_files"] = 1
        found = gate.review(api, 7, HEAD, 1, LISTED)
        self.assertEqual(found.files, [".github/workflows/ci.yml"])

    def test_a_path_listed_twice_is_named_once(self):
        self.assertEqual(gate.protected([".github/a.yml", ".github/a.yml"], LISTED), [".github/a.yml"])


class ApprovalTests(unittest.TestCase):
    def test_only_the_owners_unedited_line_naming_the_head_approves(self):
        self.assertTrue(gate.is_approval(approval(), HEAD))
        self.assertTrue(gate.is_approval(approval(body=f"  /gate-approve {HEAD}\n"), HEAD))
        for label, comment in [
            ("another commit", approval(OLD)),
            ("another login", approval(login="someone")),
            ("a bot", approval(login="github-actions")),
            ("no author", approval(author=None)),
            ("edited", approval(lastEditedAt="2026-11-16T10:00:00Z")),
            ("edited by someone", approval(editor={"login": "someone"})),
            ("created with an edit", approval(includesCreatedEdit=True)),
            ("edit fields missing", {key: value for key, value in approval().items() if key != "lastEditedAt"}),
            ("editor field missing", {key: value for key, value in approval().items() if key != "editor"}),
            ("words after the commit", approval(body=f"/gate-approve {HEAD} please")),
            ("a line below other words", approval(body=f"ok\n/gate-approve {HEAD}")),
            ("a short commit", approval(body=f"/gate-approve {HEAD[:7]}")),
            ("an upper case commit", approval(body=f"/gate-approve {HEAD.upper()}")),
            ("another command", approval(body=f"/review-override {HEAD}")),
            ("no body", approval(body="")),
        ]:
            with self.subTest(label):
                self.assertFalse(gate.is_approval(comment, HEAD))

    def test_one_valid_approval_among_refused_ones_is_found(self):
        found = gate.approval_of([approval(OLD), approval(lastEditedAt="x"), approval(url="https://c/9")], HEAD)
        self.assertEqual(found, "https://c/9")


class PullRunTests(unittest.TestCase):
    def judge(self, api, **env):
        return gate.judge(pull_env(**env), api, LISTED)

    def test_a_pull_request_that_changes_no_protected_path_passes(self):
        verdict = self.judge(FakeApi([pull()], files={7: ["plugins/shot/src/lib.rs", "README.md"]}))
        self.assertEqual((verdict.sha, verdict.conclusion, verdict.title), (HEAD, "success", "No protected path changed"))
        self.assertIn("None of the 2 changed files", verdict.summary)

    def test_a_protected_change_without_approval_is_refused_with_the_line_to_post(self):
        verdict = self.judge(FakeApi([pull()], files={7: [".github/workflows/ci.yml", "README.md"]}))
        self.assertEqual(verdict.conclusion, "failure")
        self.assertIn("- `.github/workflows/ci.yml`", verdict.summary)
        self.assertNotIn("README.md", verdict.summary)
        self.assertIn(f"    /gate-approve {HEAD}", verdict.summary)

    def test_the_owners_approval_of_the_head_passes(self):
        api = FakeApi([pull()], files={7: [".githooks/commit-msg"]}, comments={7: [approval(url="https://c/1")]})
        verdict = self.judge(api)
        self.assertEqual((verdict.conclusion, verdict.title), ("success", "The owner approved the protected paths"))
        self.assertIn("https://c/1", verdict.summary)

    def test_an_approval_is_bound_to_the_commit_it_names(self):
        api = FakeApi([pull()], files={7: [".githooks/commit-msg"]}, comments={7: [approval(OLD)]})
        self.assertEqual(self.judge(api).conclusion, "failure")

    def test_an_edited_approval_or_one_by_another_login_is_refused(self):
        for comment in [approval(lastEditedAt="2026-11-16T10:00:00Z", editor={"login": "writer"}), approval(login="writer")]:
            with self.subTest(comment=comment):
                api = FakeApi([pull()], files={7: [".github/workflows/claude-review.yml"]}, comments={7: [comment]})
                self.assertEqual(self.judge(api).conclusion, "failure")

    def test_a_fork_pull_request_found_by_its_branch_follows_the_same_rule(self):
        api = FakeApi([pull()], files={7: [".github/workflows/ci.yml"]}, commit_pulls=[], branch_pulls=[{"number": 7}])
        verdict = self.judge(api, HEAD_OWNER="forker", HEAD_BRANCH="patch-1")
        self.assertIn(("branch", "forker", "patch-1"), api.calls)
        self.assertEqual(verdict.conclusion, "failure")
        self.assertIn(f"/gate-approve {HEAD}", verdict.summary)

    def test_a_commit_lookup_the_api_refuses_falls_back_to_the_branch(self):
        api = FakeApi([pull()], files={7: ["README.md"]}, branch_pulls=[{"number": 7}])

        def refused(sha):
            raise RuntimeError("HTTP 422 No commit found")

        api.pulls_of_commit = refused
        self.assertEqual(self.judge(api, HEAD_OWNER="forker").conclusion, "success")

    def test_every_open_pull_request_with_the_head_must_pass(self):
        api = FakeApi(
            [pull(7), pull(8)],
            files={7: ["README.md"], 8: [".cargo/config.toml"]},
        )
        verdict = self.judge(api)
        self.assertEqual(verdict.conclusion, "failure")
        self.assertIn("#8 changes", verdict.summary)

    def test_a_commit_without_an_open_pull_request_of_this_repository_fails_closed(self):
        for label, api in [
            ("no pull request", FakeApi([], commit_pulls=[])),
            ("closed", FakeApi([pull(state="closed")], files={7: ["README.md"]})),
            ("another head", FakeApi([pull(head=OLD)], files={7: ["README.md"]})),
            ("another repository", FakeApi([pull(base="x/y")], files={7: ["README.md"]})),
        ]:
            with self.subTest(label):
                verdict = self.judge(api)
                self.assertEqual((verdict.sha, verdict.conclusion), (HEAD, "failure"))
                self.assertEqual(verdict.title, "The gate could not verify this commit")

    def test_a_file_list_the_api_cut_short_fails_closed(self):
        for label, api in [
            ("fewer than counted", FakeApi([pull(files=5)], files={7: ["README.md"]})),
            ("at the cap", FakeApi([pull()], files={7: [f"docs/{n}.md" for n in range(gate.PULL_FILES_CAP)]})),
            ("no count", FakeApi([pull()], files={})),
        ]:
            with self.subTest(label):
                if label == "no count":
                    api.pulls[7]["changed_files"] = None
                verdict = self.judge(api)
                self.assertEqual(verdict.conclusion, "failure")
                self.assertIn("cannot see every changed file", verdict.summary)

    def test_an_api_failure_fails_closed_on_the_head(self):
        api = FakeApi([pull()], files={7: [".github/x"]})

        def broken(number):
            raise RuntimeError("HTTP 502")

        api.comments = broken
        verdict = self.judge(api)
        self.assertEqual((verdict.sha, verdict.conclusion), (HEAD, "failure"))
        self.assertIn("HTTP 502", verdict.summary)

    def test_a_run_without_a_commit_to_judge_raises(self):
        for sha in ["", "main", HEAD[:39]]:
            with self.subTest(sha=sha), self.assertRaises(gate.Unverifiable):
                self.judge(FakeApi([pull()]), HEAD_SHA=sha)

    def test_a_push_run_is_not_judged(self):
        self.assertIsNone(self.judge(FakeApi(), RUN_EVENT="push"))


class CommentRunTests(unittest.TestCase):
    def test_an_approval_comment_judges_the_pull_requests_current_head(self):
        api = FakeApi([pull()], files={7: [".github/workflows/ci.yml"]}, comments={7: [approval()]})
        verdict = gate.judge(comment_env(), api, LISTED)
        self.assertEqual((verdict.sha, verdict.conclusion), (HEAD, "success"))

    def test_an_approval_of_an_older_commit_leaves_the_new_head_refused(self):
        api = FakeApi([pull()], files={7: [".github/workflows/ci.yml"]}, comments={7: [approval(OLD)]})
        verdict = gate.judge(comment_env(), api, LISTED)
        self.assertEqual((verdict.sha, verdict.conclusion), (HEAD, "failure"))

    def test_a_closed_pull_request_gets_no_verdict(self):
        self.assertIsNone(gate.judge(comment_env(), FakeApi([pull(state="closed")]), LISTED))


class QueueRunTests(unittest.TestCase):
    def judge(self, api, **env):
        return gate.judge(queue_env(**env), api, LISTED)

    def test_a_group_that_changes_no_protected_path_passes_without_reading_the_queue(self):
        api = FakeApi(compare=["plugins/shot/src/lib.rs"])
        verdict = self.judge(api)
        self.assertEqual((verdict.sha, verdict.conclusion), (GROUP, "success"))
        self.assertEqual(api.calls, [("compare", "main", GROUP)])

    def test_an_approved_entry_whose_queued_content_is_the_approved_head_passes(self):
        path = ".github/workflows/ci.yml"
        api = FakeApi(
            compare=[path, "README.md"],
            queue=[queue_entry(7, 1, GROUP, HEAD, 2)],
            files={7: [path, "README.md"]},
            comments={7: [approval()]},
            blobs={(HEAD, path): "blob-1", (GROUP, path): "blob-1"},
        )
        self.assertEqual(self.judge(api).conclusion, "success")

    def test_an_unapproved_protected_entry_ahead_refuses_the_group(self):
        api = FakeApi(
            compare=[".github/workflows/ci.yml", "README.md"],
            queue=[queue_entry(6, 1, AHEAD, OLD, 1), queue_entry(7, 2, GROUP, HEAD, 1)],
            files={6: [".github/workflows/ci.yml"], 7: ["README.md"]},
        )
        verdict = self.judge(api)
        self.assertEqual(verdict.conclusion, "failure")
        self.assertIn("#6 changes", verdict.summary)
        self.assertIn(f"/gate-approve {OLD}", verdict.summary)
        self.assertNotIn("#7 changes", verdict.summary)

    def test_an_entry_behind_this_one_is_not_judged_here(self):
        path = ".githooks/commit-msg"
        api = FakeApi(
            compare=[path],
            queue=[queue_entry(7, 1, GROUP, HEAD, 1), queue_entry(9, 2, BEHIND, OLD, 1)],
            files={7: [path], 9: [".github/workflows/ci.yml"]},
            comments={7: [approval()]},
            blobs={(HEAD, path): "same", (GROUP, path): "same"},
        )
        self.assertEqual(self.judge(api).conclusion, "success")

    def test_queued_content_that_differs_from_the_approved_head_is_refused(self):
        path = ".github/workflows/ci.yml"
        api = FakeApi(
            compare=[path],
            queue=[queue_entry(7, 1, GROUP, HEAD, 1)],
            files={7: [path]},
            comments={7: [approval()]},
            blobs={(HEAD, path): "approved", (GROUP, path): "queued"},
        )
        verdict = self.judge(api)
        self.assertEqual(verdict.conclusion, "failure")
        self.assertIn("match no approved head commit", verdict.summary)
        self.assertIn(f"- `{path}`", verdict.summary)

    def test_a_protected_change_no_queued_pull_request_claims_is_refused(self):
        api = FakeApi(
            compare=[".cargo/config.toml"],
            queue=[queue_entry(7, 1, GROUP, HEAD, 1)],
            files={7: ["README.md"]},
        )
        verdict = self.judge(api)
        self.assertEqual(verdict.conclusion, "failure")
        self.assertIn("- `.cargo/config.toml`", verdict.summary)

    def test_a_deleted_protected_file_binds_to_an_approved_head_that_deletes_it(self):
        path = ".config/nextest.toml"
        api = FakeApi(
            compare=[path],
            queue=[queue_entry(7, 1, GROUP, HEAD, 1)],
            files={7: [path]},
            comments={7: [approval()]},
            blobs={(HEAD, path): None, (GROUP, path): None},
        )
        self.assertEqual(self.judge(api).conclusion, "success")

    def test_an_entry_that_cannot_be_found_fails_closed(self):
        protected = [".github/workflows/ci.yml"]
        for label, api, env in [
            ("not in the queue", FakeApi(compare=protected, queue=[queue_entry(7, 1, AHEAD, HEAD, 1)]), {}),
            ("another pull request", FakeApi(compare=protected, queue=[queue_entry(8, 1, GROUP, HEAD, 1)]), {}),
            ("not a queue branch", FakeApi(compare=protected), {"HEAD_BRANCH": "topic"}),
            ("another base branch", FakeApi(compare=protected), {"HEAD_BRANCH": QUEUE_BRANCH.replace("/main/", "/dev/")}),
        ]:
            with self.subTest(label):
                verdict = self.judge(api, **env)
                self.assertEqual((verdict.sha, verdict.conclusion), (GROUP, "failure"))
                self.assertEqual(verdict.title, "The gate could not verify this commit")

    def test_a_group_larger_than_the_compare_api_lists_fails_closed(self):
        api = FakeApi(compare=[f"docs/{n}.md" for n in range(gate.COMPARE_FILES_CAP)])
        verdict = self.judge(api)
        self.assertEqual(verdict.conclusion, "failure")
        self.assertIn("more than the compare API lists", verdict.summary)


class GitHubTests(unittest.TestCase):
    def api(self, *answers):
        calls = []
        queue = list(answers)

        def call(args):
            calls.append(args)
            answer = queue.pop(0)
            if isinstance(answer, Exception):
                raise answer
            return answer

        return gate.GitHub(REPO, call=call, sleep=lambda seconds: None), calls

    def test_a_failed_call_is_retried_and_then_gives_up(self):
        api, calls = self.api(RuntimeError("502"), {"number": 7})
        self.assertEqual(api.pull(7), {"number": 7})
        self.assertEqual(len(calls), 2)
        api, calls = self.api(*[RuntimeError("502")] * gate.MAX_ATTEMPTS)
        with self.assertRaises(RuntimeError):
            api.pull(7)
        self.assertEqual(len(calls), gate.MAX_ATTEMPTS)

    def test_comments_are_read_page_by_page_with_their_edit_fields(self):
        first = {"data": {"repository": {"pullRequest": {"comments": {"pageInfo": {"hasNextPage": True, "endCursor": "c1"}, "nodes": [approval(OLD)]}}}}}
        second = {"data": {"repository": {"pullRequest": {"comments": {"pageInfo": {"hasNextPage": False, "endCursor": "c2"}, "nodes": [approval()]}}}}}
        api, calls = self.api(first, second)
        self.assertEqual(api.comments(7), [approval(OLD), approval()])
        self.assertIn("-F", calls[0])
        self.assertIn("number=7", calls[0])
        self.assertNotIn("cursor=c1", calls[0])
        self.assertIn("cursor=c1", calls[1])
        for field in ["lastEditedAt", "editor { login }", "includesCreatedEdit", "author { login }"]:
            self.assertIn(field, gate.COMMENTS_QUERY)

    def test_graphql_errors_and_a_missing_pull_request_raise(self):
        api, _ = self.api({"errors": [{"message": "bad"}]})
        with self.assertRaises(RuntimeError):
            api.comments(7)
        api, _ = self.api({"data": {"repository": {"pullRequest": None}}})
        with self.assertRaises(gate.Unverifiable):
            api.comments(7)

    def test_queue_entries_map_the_group_commit_to_its_pull_request(self):
        node = {"position": 2, "headCommit": {"oid": GROUP}, "pullRequest": {"number": 7, "headRefOid": HEAD, "changedFiles": 3}}
        api, _ = self.api({"data": {"repository": {"mergeQueue": {"entries": {"pageInfo": {"hasNextPage": False}, "nodes": [node]}}}}})
        self.assertEqual(api.queue_entries(), [queue_entry(7, 2, GROUP, HEAD, 3)])
        for field in ["position", "headCommit { oid }", "number headRefOid changedFiles"]:
            self.assertIn(field, gate.QUEUE_QUERY)

    def test_a_queue_the_api_cannot_list_whole_fails_closed(self):
        for queue in [None, {"entries": {"pageInfo": {"hasNextPage": True}, "nodes": []}}]:
            with self.subTest(queue=queue):
                api, _ = self.api({"data": {"repository": {"mergeQueue": queue}}})
                with self.assertRaises(gate.Unverifiable):
                    api.queue_entries()

    def test_a_missing_file_has_no_blob_and_each_blob_is_read_once(self):
        api, calls = self.api({"data": {"repository": {"object": None}}}, {"data": {"repository": {"object": {"oid": "f00"}}}})
        self.assertIsNone(api.blob(HEAD, "a/b"))
        self.assertIsNone(api.blob(HEAD, "a/b"))
        self.assertEqual(api.blob(GROUP, "a/b"), "f00")
        self.assertEqual(len(calls), 2)
        self.assertIn(f"expression={HEAD}:a/b", calls[0])

    def test_the_check_is_posted_completed_on_the_verdicts_commit(self):
        api, calls = self.api({})
        api.post(gate.Verdict(HEAD, "failure", "t", "s"), "https://run")
        for field in ["name=main gate", f"head_sha={HEAD}", "status=completed", "conclusion=failure", "details_url=https://run", "output[title]=t", "output[summary]=s"]:
            self.assertIn(field, calls[0])
        self.assertIn(f"repos/{REPO}/check-runs", calls[0])

    def test_the_branch_lookup_passes_the_branch_as_a_field(self):
        api, calls = self.api([])
        api.pulls_of_branch("forker", "x;$(id)")
        self.assertIn("head=forker:x;$(id)", calls[0])
        self.assertIn(f"repos/{REPO}/pulls", calls[0])


class CommandTests(unittest.TestCase):
    def test_evaluate_writes_the_verdict_and_post_sends_it(self):
        with tempfile.TemporaryDirectory() as temp:
            verdict_file, summary = Path(temp) / "v.json", Path(temp) / "summary.md"
            env = {**pull_env(), "VERDICT_FILE": str(verdict_file), "GITHUB_STEP_SUMMARY": str(summary)}
            api = FakeApi([pull()], files={7: [".github/workflows/ci.yml"]})
            with redirect_stdout(io.StringIO()):
                self.assertEqual(gate.evaluate(env, api, LISTED), 0)
            self.assertEqual(json.loads(verdict_file.read_text())["conclusion"], "failure")
            self.assertIn(f"/gate-approve {HEAD}", summary.read_text())
            with redirect_stdout(io.StringIO()):
                self.assertEqual(gate.post({"VERDICT_FILE": str(verdict_file), "RUN_URL": "https://run"}, api), 0)
            self.assertEqual(api.posted[0][0].sha, HEAD)
            self.assertEqual(api.posted[0][1], "https://run")

    def test_posting_the_same_verdict_twice_sends_the_same_check(self):
        with tempfile.TemporaryDirectory() as temp:
            verdict_file = Path(temp) / "v.json"
            verdict_file.write_text(json.dumps(gate.Verdict(HEAD, "success", "t", "s").__dict__))
            api = FakeApi()
            with redirect_stdout(io.StringIO()):
                gate.post({"VERDICT_FILE": str(verdict_file)}, api)
                gate.post({"VERDICT_FILE": str(verdict_file)}, api)
            self.assertEqual(api.posted[0], api.posted[1])

    def test_nothing_is_posted_without_a_verdict(self):
        with tempfile.TemporaryDirectory() as temp:
            api = FakeApi()
            with redirect_stdout(io.StringIO()):
                self.assertEqual(gate.post({"VERDICT_FILE": str(Path(temp) / "none.json")}, api), 0)
                Path(temp, "bad.json").write_text(json.dumps({"sha": "main", "conclusion": "success", "title": "t", "summary": "s"}))
                self.assertEqual(gate.post({"VERDICT_FILE": str(Path(temp) / "bad.json")}, api), 1)
            self.assertEqual(api.posted, [])

    def test_a_comment_run_that_cannot_read_the_pull_request_fails_the_job(self):
        with tempfile.TemporaryDirectory() as temp:
            env = {**comment_env(9), "VERDICT_FILE": str(Path(temp) / "v.json")}
            out = io.StringIO()
            with redirect_stdout(out):
                self.assertEqual(gate.evaluate(env, FakeApi(), LISTED), 1)
            self.assertIn("::error::", out.getvalue())
            self.assertFalse(Path(env["VERDICT_FILE"]).exists())


def run_bodies(document: str) -> list[str]:
    lines, bodies = document.splitlines(), []
    for index, line in enumerate(lines):
        stripped = line.lstrip()
        if not stripped.startswith("run:"):
            continue
        indent, body = len(line) - len(stripped), [stripped[4:]]
        for following in lines[index + 1 :]:
            if following.strip() and len(following) - len(following.lstrip()) <= indent:
                break
            body.append(following)
        bodies.append("\n".join(body))
    return bodies


def blocks(document: str, key: str) -> list[str]:
    return re.findall(rf"^( *){key}:\n((?:\1  .*\n)+)", document, re.MULTILINE)


class WorkflowContractTests(unittest.TestCase):
    def test_only_runs_from_mains_definition_start_the_gate(self):
        triggers = WORKFLOW.split("\npermissions:", 1)[0].split("\non:\n", 1)[1]
        self.assertIn("  workflow_run:\n    workflows: [tests]\n    types: [requested]\n", triggers)
        self.assertIn("  issue_comment:\n    types: [created]\n", triggers)
        self.assertEqual(re.findall(r"^  ([a-z_]+):", triggers, re.MULTILINE), ["workflow_run", "issue_comment"])
        self.assertIn("name: main gate\n", WORKFLOW)

    def test_the_job_checks_out_only_mains_scripts(self):
        checkouts = WORKFLOW.split("uses: actions/checkout@")[1:]
        self.assertEqual(len(checkouts), 1)
        checkout = checkouts[0].split("\n\n", 1)[0]
        for contract in ["ref: ${{ github.sha }}", "sparse-checkout: .github/scripts", "persist-credentials: false"]:
            self.assertIn(contract, checkout)
        for loose in ["head_sha", "head_branch", "head_ref", "refs/pull", "pull_request", ".head."]:
            self.assertNotIn(loose, checkout)
        self.assertEqual(gate.PATHS_FILE.parent, ROOT / ".github")

    def test_no_run_line_interpolates_an_expression(self):
        bodies = run_bodies(WORKFLOW)
        self.assertEqual(len(bodies), 3)
        for body in bodies:
            with self.subTest(body=body):
                self.assertNotIn("${{", body)

    def test_the_workflow_token_only_reads_and_only_the_app_writes_the_check(self):
        permissions = blocks(WORKFLOW, "permissions")
        self.assertEqual(len(permissions), 2)
        for _, block in permissions:
            values = [line.split(":", 1)[1].strip() for line in block.splitlines()]
            self.assertEqual(set(values), {"read"})
        self.assertNotIn("write-all", WORKFLOW)
        self.assertEqual(re.findall(r"permission-[a-z-]+: \w+", WORKFLOW), ["permission-checks: write"])

    def test_the_app_token_comes_from_the_main_only_environment_and_skips_until_set(self):
        self.assertIn("    environment:\n      name: main-gate\n      deployment: false\n", WORKFLOW)
        mint = WORKFLOW.split("- name: Mint the main gate app token\n", 1)[1].split("\n\n", 1)[0]
        for contract in [
            "if: vars.MAIN_GATE_APP_CLIENT_ID != ''",
            "uses: actions/create-github-app-token@64ab729a6294000f5aa4a4bc0b73d1f32bc45510",
            "client-id: ${{ vars.MAIN_GATE_APP_CLIENT_ID }}",
            "private-key: ${{ secrets.MAIN_GATE_APP_PRIVATE_KEY }}",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, mint)
        posting = WORKFLOW.split("- name: Post the main gate check\n", 1)[1].split("\n\n", 1)[0]
        self.assertIn("if: steps.app.outputs.token != ''", posting)
        self.assertIn("GH_TOKEN: ${{ steps.app.outputs.token }}", posting)
        judging = WORKFLOW.split("- name: Judge the commit with main's path list\n", 1)[1].split("\n\n", 1)[0]
        self.assertIn("GH_TOKEN: ${{ github.token }}", judging)
        self.assertNotIn("steps.app", judging)
        self.assertIsNone(re.search(r"^ *checks: write", WORKFLOW, re.MULTILINE))

    def test_only_the_owners_approval_comment_starts_a_comment_run(self):
        condition = WORKFLOW.split("    if: >-\n", 1)[1].split("    runs-on:", 1)[0]
        for contract in [
            "github.event.issue.pull_request",
            "startsWith(github.event.comment.body, '/gate-approve ')",
            "github.event.comment.user.login == 'KMRH47'",
            "github.event.workflow_run.event == 'pull_request'",
            "github.event.workflow_run.event == 'merge_group'",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, condition)
        self.assertEqual(gate.OWNER, "KMRH47")


if __name__ == "__main__":
    unittest.main()
