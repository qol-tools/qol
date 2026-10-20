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
    ENV = {"GITHUB_REPOSITORY": "o/r", "PR_NUMBER": "7", "HEAD_SHA": SHA, "EVENT": "pull_request", "BOT": "false"}

    def verdict(self, review: str, entries: list[dict] | None = None, **env) -> tuple[int, str, str]:
        fetch = mock.Mock(return_value=entries or [])
        with tempfile.NamedTemporaryFile("r", suffix=".md") as summary, mock.patch.object(gate, "comments", fetch):
            out = io.StringIO()
            with redirect_stdout(out):
                code = gate.verdict({**self.ENV, "VERDICT": review, **env}, summary.name)
            return code, out.getvalue(), summary.read()

    def test_a_pass_verdict_a_queue_run_and_dependabot_pass_without_reading_comments(self):
        for review, env in [("pass", {}), ("", {"EVENT": "merge_group"}), ("", {"BOT": "true"})]:
            with self.subTest(review=review, env=env), mock.patch.object(gate, "comments") as fetch:
                with redirect_stdout(io.StringIO()):
                    self.assertEqual(gate.verdict({**self.ENV, "VERDICT": review, **env}, None), 0)
                fetch.assert_not_called()

    def test_a_blocking_verdict_without_an_override_fails(self):
        for review in ["conditional", "block", ""]:
            with self.subTest(review=review):
                code, out, summary = self.verdict(review, [comment(gate.record(OTHER, "KMRH47"))])
                self.assertEqual(code, 1)
                self.assertIn(f"the review verdict is {review or 'missing'}", out)
                self.assertEqual(summary, "")

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
    def __init__(self, head: str = SHA, runs: list[dict] | None = None, jobs: dict | None = None, review: list[dict] | None = None):
        self.head, self.runs, self.jobs, self.review = head, runs or [], jobs or {}, review or []
        self.calls: list[list[str]] = []
        self.statuses: dict[int, list[str]] = {}

    def gh_json(self, args: list[str]) -> dict:
        route = args[0]
        if route == "repos/o/r/pulls/7":
            return {"head": {"sha": self.head}}
        if route.startswith("repos/o/r/actions/runs?"):
            self.assertIn(f"head_sha={SHA}", route)
            return {"workflow_runs": self.runs}
        if route.endswith("/jobs?per_page=100"):
            return {"jobs": self.jobs[int(route.split("/")[-2])]}
        run_id = int(route.split("/")[-1])
        return {"status": self.statuses[run_id].pop(0)}

    def gh(self, args: list[str]) -> str:
        self.calls.append(args)
        return "{}"

    def assertIn(self, needle, haystack):
        assert needle in haystack, (needle, haystack)


class RerunWaitsTests(unittest.TestCase):
    def rerun(self, api: FakeApi) -> list[str]:
        with mock.patch.object(gate, "gh_json", api.gh_json), mock.patch.object(gate, "gh", api.gh):
            with redirect_stdout(io.StringIO()):
                return gate.rerun_waits("o/r", "7", SHA)

    def test_only_the_latest_run_of_each_workflow_that_stopped_on_the_review_reruns(self):
        wait_failed = [{"name": "Wait for the review", "conclusion": "failure"}, {"name": "merge gate", "conclusion": "failure"}]
        api = FakeApi(
            runs=[
                run(10, workflow=1),
                run(11, workflow=1),
                run(20, workflow=2),
                run(30, workflow=3),
                run(40, "in_progress", None, workflow=4),
                run(50, conclusion="success", workflow=5),
            ],
            jobs={
                11: wait_failed,
                20: wait_failed,
                30: [{"name": "Wait for the review", "conclusion": "success"}, {"name": "lint + test", "conclusion": "failure"}],
            },
        )
        self.assertEqual(self.rerun(api), ["https://x/runs/11", "https://x/runs/20"])
        self.assertEqual(
            api.calls,
            [
                ["--method", "POST", "repos/o/r/actions/runs/11/rerun-failed-jobs"],
                ["--method", "POST", "repos/o/r/actions/runs/20/rerun-failed-jobs"],
            ],
        )

    def test_a_pull_request_that_moved_on_keeps_its_checks(self):
        api = FakeApi(head=OTHER, runs=[run(11)], jobs={11: [{"name": "Wait for the review", "conclusion": "failure"}]})
        self.assertEqual(self.rerun(api), [])
        self.assertEqual(api.calls, [])


class ApplyOverrideTests(unittest.TestCase):
    def apply(self, api: FakeApi, entries: list[dict]) -> tuple[int, mock.Mock, str]:
        sleep = mock.Mock()
        with (
            mock.patch.object(gate, "gh_json", api.gh_json),
            mock.patch.object(gate, "gh", api.gh),
            mock.patch.object(gate, "comments", return_value=entries),
            mock.patch.object(gate, "review_runs", return_value=api.review),
        ):
            out = io.StringIO()
            with redirect_stdout(out):
                code = gate.apply_override("o/r", "7", SHA, "KMRH47", sleep)
        return code, sleep, out.getvalue()

    def test_the_override_removes_the_label_records_once_and_reruns_the_failed_verdict_and_waits(self):
        api = FakeApi(
            review=[run(5, conclusion="cancelled"), run(6)],
            runs=[run(6), run(7, workflow=2)],
            jobs={6: [{"id": 61, "name": "Review with qol-code-review"}, {"id": 62, "name": "claude review"}], 7: [{"name": "Wait for the review", "conclusion": "failure"}]},
        )
        api.statuses[6] = ["completed", "queued"]
        code, sleep, _ = self.apply(api, [])
        self.assertEqual(code, 0)
        self.assertEqual(api.calls[0], ["--method", "DELETE", "repos/o/r/issues/7/labels/review-override"])
        self.assertEqual(api.calls[1], ["--method", "POST", "repos/o/r/issues/7/comments", "-f", f"body={gate.record(SHA, 'KMRH47')}"])
        self.assertEqual(api.calls[2], ["--method", "POST", "repos/o/r/actions/jobs/62/rerun"])
        self.assertEqual(api.calls[3], ["--method", "POST", "repos/o/r/actions/runs/7/rerun-failed-jobs"])
        self.assertEqual(sleep.call_count, 1)

    def test_a_second_label_does_not_record_twice(self):
        api = FakeApi(review=[run(6)], jobs={6: [{"id": 62, "name": "claude review"}]})
        api.statuses[6] = ["queued"]
        code, _, _ = self.apply(api, [comment(gate.record(SHA, "KMRH47"))])
        self.assertEqual(code, 0)
        self.assertEqual(
            api.calls,
            [["--method", "DELETE", "repos/o/r/issues/7/labels/review-override"], ["--method", "POST", "repos/o/r/actions/jobs/62/rerun"]],
        )

    def test_only_a_finished_review_of_the_labelled_commit_can_be_overridden(self):
        for review in [[], [run(6, "in_progress", None)]]:
            with self.subTest(review=review):
                api = FakeApi(review=review)
                code, _, out = self.apply(api, [])
                self.assertEqual(code, 1)
                self.assertIn("the review of 1f8c4b7 has not finished, so there is no verdict to override", out)
                self.assertEqual(api.calls, [["--method", "DELETE", "repos/o/r/issues/7/labels/review-override"]])

    def test_a_passed_review_needs_no_override(self):
        api = FakeApi(review=[run(6, conclusion="success")])
        code, _, out = self.apply(api, [])
        self.assertEqual(code, 0)
        self.assertIn("the review of 1f8c4b7 already passed", out)
        self.assertEqual(api.calls, [["--method", "DELETE", "repos/o/r/issues/7/labels/review-override"]])


class MainTests(unittest.TestCase):
    def test_a_failed_rerun_of_the_waits_only_warns(self):
        env = {"GITHUB_REPOSITORY": "o/r", "PR_NUMBER": "7", "HEAD_SHA": SHA}
        with mock.patch.dict(gate.os.environ, env), mock.patch.object(gate, "rerun_waits", side_effect=RuntimeError("HTTP 502")):
            out = io.StringIO()
            with redirect_stdout(out):
                self.assertEqual(gate.main(["review_gate.py", "rerun-waits"]), 0)
        self.assertIn("::warning::the waiting checks were not rerun: HTTP 502", out.getvalue())

    def test_a_failed_verdict_lookup_fails_closed(self):
        env = {"GITHUB_REPOSITORY": "o/r", "PR_NUMBER": "7", "HEAD_SHA": SHA, "EVENT": "pull_request", "VERDICT": "conditional"}
        with mock.patch.dict(gate.os.environ, env), mock.patch.object(gate, "comments", side_effect=RuntimeError("HTTP 502")):
            out = io.StringIO()
            with redirect_stdout(out):
                self.assertEqual(gate.main(["review_gate.py", "verdict"]), 1)
        self.assertIn("::error::HTTP 502", out.getvalue())


if __name__ == "__main__":
    unittest.main()
