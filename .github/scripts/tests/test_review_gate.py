import importlib.util
import io
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
_SPEC = importlib.util.spec_from_file_location("review_gate", SCRIPTS / "review_gate.py")
gate = importlib.util.module_from_spec(_SPEC)
sys.modules["review_gate"] = gate
_SPEC.loader.exec_module(gate)

SHA = "1f8c4b72353946d1433cb338fd46c3e9c09a0432"
OTHER = "0719d4cc4" + "0" * 31
BOT = {"login": "github-actions[bot]"}


def comment(body: str, user: dict = BOT, url: str = "https://github.com/o/r/pull/7#issuecomment-1") -> dict:
    return {"user": user, "body": body, "html_url": url}


def run(run_id: int, status: str = "completed", conclusion: str | None = "failure", workflow: int = 1) -> dict:
    return {"id": run_id, "workflow_id": workflow, "status": status, "conclusion": conclusion, "html_url": f"https://x/runs/{run_id}"}


class OverrideRecordTests(unittest.TestCase):
    def test_only_the_bots_record_for_this_head_counts(self):
        entries = [
            comment(gate.record(OTHER, "KMRH47")),
            comment(gate.record(SHA, "KMRH47"), user={"login": "KMRH47"}),
            comment(f"<!-- qol-code-review -->\n\n{gate.record(SHA, 'KMRH47')}"),
            comment("ordinary comment"),
        ]
        self.assertIsNone(gate.override_for(entries, SHA))
        found = gate.override_for([*entries, comment(gate.record(SHA, "KMRH47"), url="https://c/9")], SHA)
        self.assertEqual(found, {"actor": "KMRH47", "url": "https://c/9"})

    def test_the_record_names_the_owner_and_the_commit(self):
        body = gate.record(SHA, "KMRH47")
        self.assertTrue(body.startswith(f"<!-- qol-review-override {SHA} KMRH47 -->\n"))
        self.assertIn("@KMRH47 passed the review of `1f8c4b7`", body)


class VerdictTests(unittest.TestCase):
    ENV = {"GITHUB_REPOSITORY": "o/r", "PR_NUMBER": "7", "HEAD_SHA": SHA, "EVENT": "pull_request", "BOT": "false", "BASE": "main"}

    def verdict(self, review: str, entries: list[dict] | None = None, live_base: str = "main", **env) -> tuple[int, str, str]:
        fetch = mock.Mock(return_value=entries or [])
        pull = mock.Mock(return_value={"base": {"ref": live_base}})
        with (
            tempfile.NamedTemporaryFile("r", suffix=".md") as summary,
            mock.patch.object(gate, "comments", fetch),
            mock.patch.object(gate, "gh_json", pull),
        ):
            out = io.StringIO()
            with redirect_stdout(out):
                code = gate.verdict({**self.ENV, "VERDICT": review, **env}, summary.name)
            pull.assert_called_once_with(["repos/o/r/pulls/7"])
            return code, out.getvalue(), summary.read()

    def test_a_pass_verdict_a_queue_run_and_dependabot_pass_without_reading_comments(self):
        for review, env, reads_base in [("pass", {}, True), ("", {"EVENT": "merge_group"}, False), ("", {"BOT": "true"}, False)]:
            with (
                self.subTest(review=review, env=env),
                mock.patch.object(gate, "comments") as fetch,
                mock.patch.object(gate, "gh_json", return_value={"base": {"ref": "main"}}) as pull,
            ):
                with redirect_stdout(io.StringIO()):
                    self.assertEqual(gate.verdict({**self.ENV, "VERDICT": review, **env}, None), 0)
                fetch.assert_not_called()
                self.assertEqual(pull.call_count, int(reads_base))

    def test_a_review_of_a_base_the_pull_request_left_fails_even_with_an_override(self):
        for review in ["pass", "conditional"]:
            with self.subTest(review=review):
                code, out, summary = self.verdict(review, [comment(gate.record(SHA, "KMRH47"))], BASE="fix/stacked-base")
                self.assertEqual(code, 1)
                self.assertIn(
                    "::error::1f8c4b7 was reviewed against fix/stacked-base, and the base is now main. "
                    "Close and reopen the pull request, or push a commit, to review it against main.",
                    out,
                )
                self.assertEqual(summary, "")

    def test_a_blocking_verdict_without_an_override_fails(self):
        for review in ["conditional", "block", ""]:
            with self.subTest(review=review):
                code, out, summary = self.verdict(review, [comment(gate.record(OTHER, "KMRH47"))])
                self.assertEqual(code, 1)
                self.assertIn(f"the review verdict is {review or 'missing'}", out)
                self.assertEqual(summary, "")

    def test_quota_passes_only_after_the_skipped_review_was_published(self):
        code, out, summary = self.verdict("quota", PUBLISH_RESULT="success")
        self.assertEqual(code, 0)
        self.assertIn("this commit was not reviewed", out)
        self.assertIn("Review skipped", summary)
        self.assertEqual(self.verdict("quota", PUBLISH_RESULT="failure")[0], 1)

    def test_an_owner_override_of_this_head_passes_and_is_recorded_in_the_check(self):
        code, out, summary = self.verdict("conditional", [comment(gate.record(SHA, "KMRH47"), url="https://c/3")])
        self.assertEqual(code, 0)
        note = "@KMRH47 overrode the conditional review verdict of 1f8c4b7: https://c/3"
        self.assertIn(f"::notice title=Review override::{note}", out)
        self.assertEqual(summary, f"### Review override\n\n{note}\n")


class SupersededTests(unittest.TestCase):
    def end(self, runs: list[dict]) -> tuple[int, mock.Mock, mock.Mock]:
        cancel, sleep = mock.Mock(), mock.Mock()
        with mock.patch.object(gate, "review_runs", return_value=runs), mock.patch.object(gate, "cancel", cancel):
            with redirect_stdout(io.StringIO()):
                code = gate.end_if_superseded("o/r", SHA, "100", sleep)
        return code, cancel, sleep

    def test_a_run_a_newer_review_replaced_cancels_itself(self):
        code, cancel, sleep = self.end([run(100, "in_progress", None), run(101, "in_progress", None)])
        cancel.assert_called_once_with("o/r", "100")
        sleep.assert_called_once_with(gate.CANCEL_GRACE_SECONDS)
        self.assertEqual(code, 1)

    def test_the_newest_run_leaves_the_decision_to_the_verdict(self):
        code, cancel, _ = self.end([run(99), run(100, "in_progress", None)])
        cancel.assert_not_called()
        self.assertEqual(code, 0)


class FakeApi:
    def __init__(self, head: str = SHA, jobs: dict | None = None, review: list[dict] | None = None):
        self.head, self.jobs, self.review = head, jobs or {}, review or []
        self.calls: list[list[str]] = []
        self.statuses: dict[int, list[str]] = {}

    def gh_json(self, args: list[str]) -> dict:
        route = args[0]
        if route == "repos/o/r/pulls/7":
            return {"head": {"sha": self.head}}
        if route.endswith("/jobs?per_page=100"):
            return {"jobs": self.jobs[int(route.split("/")[-2])]}
        run_id = int(route.split("/")[-1])
        return {"status": self.statuses[run_id].pop(0)}

    def gh(self, args: list[str]) -> str:
        self.calls.append(args)
        return "{}"


class ReviewRunsTests(unittest.TestCase):
    def test_lists_the_pull_request_review_runs_of_the_commit(self):
        with mock.patch.object(gate, "gh_json", return_value={"workflow_runs": [run(6)]}) as gh_json:
            self.assertEqual(gate.review_runs("o/r", SHA), [run(6)])
        route = gh_json.call_args.args[0][0]
        self.assertTrue(route.startswith("repos/o/r/actions/workflows/claude-review.yml/runs?"))
        self.assertIn(f"head_sha={SHA}", route)
        self.assertIn("event=pull_request", route)


class ApplyOverrideTests(unittest.TestCase):
    NAMED = f"/review-override {SHA[:7]}"

    def apply(self, api: FakeApi, entries: list[dict], body: str = NAMED) -> tuple[int, mock.Mock, str]:
        sleep = mock.Mock()
        with (
            mock.patch.object(gate, "gh_json", api.gh_json),
            mock.patch.object(gate, "gh", api.gh),
            mock.patch.object(gate, "comments", return_value=entries),
            mock.patch.object(gate, "review_runs", return_value=api.review),
        ):
            out = io.StringIO()
            with redirect_stdout(out):
                code = gate.apply_override("o/r", "7", body, "KMRH47", sleep)
        return code, sleep, out.getvalue()

    def refusal(self, reason: str) -> list[str]:
        return ["--method", "POST", "repos/o/r/issues/7/comments", "-f", f"body=> [!WARNING]\n> **Review override refused** · {reason}\n"]

    def test_the_override_records_once_and_reruns_the_failed_verdict(self):
        api = FakeApi(
            review=[run(5, conclusion="cancelled"), run(6)],
            jobs={6: [{"id": 61, "name": "Review with qol-code-review"}, {"id": 62, "name": "claude review"}]},
        )
        api.statuses[6] = ["completed", "queued"]
        code, sleep, _ = self.apply(api, [], f"{self.NAMED}\nRound 130 picked the squeezed rows.")
        self.assertEqual(code, 0)
        self.assertEqual(
            api.calls,
            [
                ["--method", "POST", "repos/o/r/issues/7/comments", "-f", f"body={gate.record(SHA, 'KMRH47')}"],
                ["--method", "POST", "repos/o/r/actions/jobs/62/rerun"],
            ],
        )
        self.assertEqual(sleep.call_count, 1)

    def test_a_second_override_does_not_record_twice(self):
        api = FakeApi(review=[run(6)], jobs={6: [{"id": 62, "name": "claude review"}]})
        api.statuses[6] = ["queued"]
        code, _, _ = self.apply(api, [comment(gate.record(SHA, "KMRH47"))], f"/review-override {SHA}")
        self.assertEqual(code, 0)
        self.assertEqual(api.calls, [["--method", "POST", "repos/o/r/actions/jobs/62/rerun"]])

    def test_the_override_names_the_head_commit_or_is_refused(self):
        for body, reason in [
            (f"/review-override {OTHER[:7]}", f"`{OTHER[:7]}` is not the head commit `1f8c4b7`, so its review was not overridden."),
            ("/review-override 1f8c", "name the head commit the override is for: `/review-override <sha>`."),
            (f"/review-override {SHA[:7]} please", "name the head commit the override is for: `/review-override <sha>`."),
        ]:
            with self.subTest(body=body):
                api = FakeApi(review=[run(6)])
                code, _, out = self.apply(api, [], body)
                self.assertEqual(code, 1)
                self.assertEqual(api.calls, [self.refusal(reason)])
                self.assertIn(f"::error::{reason}", out)

    def test_only_a_finished_review_can_be_overridden(self):
        for review in [[], [run(6, "in_progress", None)]]:
            with self.subTest(review=review):
                api = FakeApi(review=review)
                code, _, _ = self.apply(api, [])
                self.assertEqual(code, 1)
                self.assertEqual(api.calls, [self.refusal("the review of `1f8c4b7` has not finished, so there is no verdict to override yet.")])

    def test_a_passed_review_needs_no_override(self):
        api = FakeApi(review=[run(6, conclusion="success")])
        code, _, out = self.apply(api, [])
        self.assertEqual(code, 0)
        self.assertIn("the review of 1f8c4b7 already passed", out)
        self.assertEqual(api.calls, [])


class MainTests(unittest.TestCase):
    def test_an_unknown_command_is_a_usage_error(self):
        with mock.patch("sys.stderr", new_callable=io.StringIO) as stderr:
            self.assertEqual(gate.main(["review_gate.py", "rerun-waits"]), 2)
        self.assertIn("usage: review_gate.py verdict|superseded|override", stderr.getvalue())

    def test_a_failed_verdict_lookup_fails_closed(self):
        env = {"GITHUB_REPOSITORY": "o/r", "PR_NUMBER": "7", "HEAD_SHA": SHA, "EVENT": "pull_request", "VERDICT": "conditional", "BASE": "main"}
        for lookup, pull, fetch in [
            ("base", mock.Mock(side_effect=RuntimeError("HTTP 502")), mock.Mock()),
            ("comments", mock.Mock(return_value={"base": {"ref": "main"}}), mock.Mock(side_effect=RuntimeError("HTTP 502"))),
        ]:
            with (
                self.subTest(lookup=lookup),
                mock.patch.dict(gate.os.environ, env),
                mock.patch.object(gate, "gh_json", pull),
                mock.patch.object(gate, "comments", fetch),
            ):
                out = io.StringIO()
                with redirect_stdout(out):
                    self.assertEqual(gate.main(["review_gate.py", "verdict"]), 1)
                self.assertIn("::error::HTTP 502", out.getvalue())


if __name__ == "__main__":
    unittest.main()
