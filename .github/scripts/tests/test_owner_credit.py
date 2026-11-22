import contextlib
import importlib.util
import io
import os
import sys
import unittest
from pathlib import Path
from unittest import mock

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
_SPEC = importlib.util.spec_from_file_location("owner_credit", SCRIPTS / "owner_credit.py")
credit = importlib.util.module_from_spec(_SPEC)
sys.modules["owner_credit"] = credit
_SPEC.loader.exec_module(credit)

WORKFLOW = SCRIPTS.parent / "workflows" / "ci.yml"
SHA = "a" * 40


def user(login):
    return {"user": {"login": login} if login else None}


def page(author, commit_authors, cursor=None):
    return {
        "author": {"login": author} if author else None,
        "commits": {
            "pageInfo": {"hasNextPage": cursor is not None, "endCursor": cursor},
            "nodes": [{"commit": {"author": user(login)}} for login in commit_authors],
        },
    }


def pull_answer(*pages):
    return [{"data": {"repository": {"pullRequest": item}}} for item in pages]


def commit_answer(authors):
    commit = None if authors is None else {"authors": {"nodes": [user(login) for login in authors]}}
    return [{"data": {"repository": {"object": commit}}}]


class RuleTests(unittest.TestCase):
    def test_the_owner_is_missing_only_beside_the_agent(self):
        cases = [
            ("agent alone", ["kmrh47-agents"], True),
            ("agent with a bot", ["dependabot[bot]", "kmrh47-agents"], True),
            ("agent and owner", ["kmrh47-agents", "KMRH47"], False),
            ("owner first", ["KMRH47", "kmrh47-agents"], False),
            ("owner alone", ["KMRH47"], False),
            ("bot alone", ["github-actions[bot]"], False),
            ("outside contributor", ["someone"], False),
            ("nobody resolved", [], False),
            ("logins compare without case", ["KMRH47-Agents", "kmrh47"], False),
            ("agent in another case", ["KMRH47-AGENTS"], True),
            ("a longer login is not the owner", ["kmrh47-agents", "KMRH47x"], True),
            ("a longer login is not the agent", ["kmrh47-agents-2"], False),
        ]
        for name, credited, expected in cases:
            with self.subTest(name):
                self.assertEqual(credit.owner_missing(credited), expected)

    def test_logins_skip_unresolved_authors_and_repeats(self):
        nodes = [user("kmrh47-agents"), user(None), None, {}, user("KMRH47"), user("kmrh47-agents")]
        self.assertEqual(credit.logins(nodes), ["kmrh47-agents", "KMRH47"])

    def test_a_pull_request_counts_its_author_and_every_commit_author(self):
        cases = [
            ("agent pull request, owner commits", [page("kmrh47-agents", ["KMRH47", "KMRH47"])], ["kmrh47-agents", "KMRH47"]),
            ("agent pull request, agent commits", [page("kmrh47-agents", ["kmrh47-agents"])], ["kmrh47-agents"]),
            ("bot pull request the agent pushed to", [page("dependabot", ["dependabot[bot]", "kmrh47-agents"])], ["dependabot", "dependabot[bot]", "kmrh47-agents"]),
            ("owner commit on a later page", [page("kmrh47-agents", ["kmrh47-agents"], "c1"), page("kmrh47-agents", ["KMRH47"])], ["kmrh47-agents", "KMRH47"]),
            ("deleted author account", [page(None, [None])], []),
        ]
        for name, pages, expected in cases:
            with self.subTest(name):
                self.assertEqual(credit.pull_credit(pages), expected)

    def test_a_commit_counts_every_account_github_credits(self):
        cases = [
            ("author and co-author", {"authors": {"nodes": [user("kmrh47-agents"), user("KMRH47")]}}, ["kmrh47-agents", "KMRH47"]),
            ("co-author email GitHub cannot place", {"authors": {"nodes": [user("kmrh47-agents"), user(None)]}}, ["kmrh47-agents"]),
            ("not a commit", {}, []),
            ("missing", None, []),
        ]
        for name, commit, expected in cases:
            with self.subTest(name):
                self.assertEqual(credit.commit_credit(commit), expected)

    def test_the_verdict_names_who_is_credited_and_the_fix(self):
        passed, text = credit.verdict(["kmrh47-agents", "KMRH47"], landing=True)
        self.assertTrue(passed)
        self.assertEqual(text, "credited: kmrh47-agents, KMRH47")
        passed, text = credit.verdict([], landing=False)
        self.assertTrue(passed)
        self.assertEqual(text, "credited: no GitHub account")
        for landing, lead in [(True, "this commit lands credited to"), (False, "this pull request would land credited to")]:
            with self.subTest(landing=landing):
                passed, text = credit.verdict(["kmrh47-agents"], landing=landing)
                self.assertFalse(passed)
                self.assertTrue(text.startswith(f"{lead} kmrh47-agents, without KMRH47. "))
                self.assertIn("git commit --allow-empty", text)


class MainTests(unittest.TestCase):
    def run_main(self, env, answers):
        calls = []

        def fake(args):
            calls.append(args)
            answer = answers.pop(0)
            if isinstance(answer, Exception):
                raise answer
            return answer

        out = io.StringIO()
        with (
            mock.patch.dict(os.environ, {"REPO": "qol-tools/qol", **env}, clear=True),
            mock.patch.object(credit, "gh_json", fake),
            mock.patch.object(credit.time, "sleep") as slept,
            contextlib.redirect_stdout(out),
        ):
            try:
                code = credit.main()
            except RuntimeError as error:
                code = error
        return code, out.getvalue(), calls, slept.call_count

    def test_a_pull_request_run_judges_the_pull_request(self):
        cases = [
            ("owner commits", pull_answer(page("kmrh47-agents", ["KMRH47"])), 0, "credited: kmrh47-agents, KMRH47\n"),
            ("agent commits only", pull_answer(page("kmrh47-agents", ["kmrh47-agents"])), 1, "::error::this pull request would land credited to kmrh47-agents, without KMRH47."),
            ("owner pull request", pull_answer(page("KMRH47", ["KMRH47"])), 0, "credited: KMRH47\n"),
            ("outside pull request", pull_answer(page("someone", ["someone"])), 0, "credited: someone\n"),
            ("owner on page two", pull_answer(page("kmrh47-agents", ["kmrh47-agents"], "c1"), page("kmrh47-agents", ["KMRH47"])), 0, "credited: kmrh47-agents, KMRH47\n"),
        ]
        for name, answers, expected, text in cases:
            with self.subTest(name):
                pages = len(answers)
                code, out, calls, _ = self.run_main({"PR_NUMBER": "612", "HEAD_SHA": ""}, answers)
                self.assertEqual(code, expected)
                self.assertTrue(out.startswith(text), out)
                self.assertEqual(len(calls), pages)
                self.assertIn("number=612", calls[0])
                self.assertIn("owner=qol-tools", calls[0])
                self.assertIn("name=qol", calls[0])
                self.assertFalse(any(arg.startswith("after=") for arg in calls[0]))
                if pages > 1:
                    self.assertIn("after=c1", calls[1])

    def test_a_queue_run_judges_the_commit_that_lands(self):
        cases = [
            ("squash with the owner", commit_answer(["kmrh47-agents", "KMRH47"]), 0, "credited: kmrh47-agents, KMRH47\n"),
            ("squash without the owner", commit_answer(["dependabot[bot]", "kmrh47-agents"]), 1, "::error::this commit lands credited to dependabot[bot], kmrh47-agents, without KMRH47."),
            ("bot commit", commit_answer(["dependabot[bot]"]), 0, "credited: dependabot[bot]\n"),
            ("commit GitHub does not have", commit_answer(None), 1, f"::error::GitHub has no commit {SHA}"),
        ]
        for name, answers, expected, text in cases:
            with self.subTest(name):
                code, out, calls, _ = self.run_main({"PR_NUMBER": "", "HEAD_SHA": SHA}, answers)
                self.assertEqual(code, expected)
                self.assertTrue(out.startswith(text), out)
                self.assertEqual(len(calls), 1)
                self.assertIn(f"oid={SHA}", calls[0])

    def test_a_failed_answer_is_asked_again_then_fails_the_step(self):
        good = commit_answer(["kmrh47-agents", "KMRH47"])[0]
        code, out, calls, slept = self.run_main(
            {"HEAD_SHA": SHA}, [RuntimeError("HTTP 502"), {"data": None}, good]
        )
        self.assertEqual((code, len(calls), slept), (0, 3, 2))
        self.assertEqual(out, "credited: kmrh47-agents, KMRH47\n")
        code, out, calls, slept = self.run_main({"HEAD_SHA": SHA}, [RuntimeError("HTTP 502")] * 3)
        self.assertIsInstance(code, RuntimeError)
        self.assertIn("HTTP 502", str(code))
        self.assertEqual((len(calls), slept, out), (3, 2, ""))


class WorkflowTests(unittest.TestCase):
    def step(self):
        text = WORKFLOW.read_text(encoding="utf-8")
        start = text.index("      - name: Squash commit credits the owner\n")
        end = text.index("\n      - name: ", start + 1)
        return text, text[start:end]

    def test_the_plan_runs_the_check_for_pull_requests_and_the_queue(self):
        text, step = self.step()
        self.assertIn("if: github.event_name == 'pull_request' || github.event_name == 'merge_group'\n", step)
        self.assertIn("run: python3 .github/scripts/owner_credit.py\n", step)
        self.assertIn("GH_TOKEN: ${{ github.token }}\n", step)
        self.assertIn("REPO: ${{ github.repository }}\n", step)
        self.assertIn("PR_NUMBER: ${{ github.event.pull_request.number }}\n", step)
        self.assertIn("HEAD_SHA: ${{ github.event_name == 'merge_group' && github.sha || '' }}\n", step)
        self.assertNotIn("continue-on-error", step)
        plan = text.index("\n  plan:\n")
        self.assertLess(plan, text.index(step))
        self.assertLess(text.index(step), text.index("\n  check:\n"))


if __name__ == "__main__":
    unittest.main()
