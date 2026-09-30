import importlib.util
import sys
import unittest
from pathlib import Path
from unittest import mock

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
_SPEC = importlib.util.spec_from_file_location("queue_report", SCRIPTS / "queue_report.py")
qr = importlib.util.module_from_spec(_SPEC)
sys.modules["queue_report"] = qr
_SPEC.loader.exec_module(qr)

SHA = "b28d8825b" + "0" * 31
RUN_URL = "https://github.com/o/r/actions/runs/9"


def job(name: str, conclusion: str | None) -> dict:
    return {"name": name, "conclusion": conclusion, "html_url": f"https://job/{name}"}


class FailedJobsTests(unittest.TestCase):
    def test_keeps_failed_and_timed_out_jobs_only(self):
        jobs = [
            job("ubuntu", "failure"),
            job("macos", "success"),
            job("windows", "skipped"),
            job("slow", "timed_out"),
            job("report", None),
        ]
        self.assertEqual([j["name"] for j in qr.failed_jobs(jobs)], ["ubuntu", "slow"])


class CommentTests(unittest.TestCase):
    def test_links_the_run_and_each_failed_job(self):
        body = qr.comment([job("lint + test (ubuntu-latest)", "failure")], RUN_URL)
        self.assertEqual(
            body,
            f"The merge queue sent this pull request back: [queue run]({RUN_URL}) failed.\n"
            "- [lint + test (ubuntu-latest)](https://job/lint + test (ubuntu-latest))",
        )


class ReportTests(unittest.TestCase):
    def test_comments_on_the_queued_pull_request(self):
        calls = []

        def gh(args):
            calls.append(args)
            return {"jobs": [job("ubuntu", "failure")]} if len(calls) == 1 else {}

        with mock.patch.object(qr, "gh_json", gh):
            result = qr.report("o/r", f"refs/heads/gh-readonly-queue/main/pr-40-{SHA}", "9", RUN_URL)
        self.assertEqual(result, "commented on #40")
        self.assertEqual(calls[1][:3], ["--method", "POST", "repos/o/r/issues/40/comments"])
        self.assertIn("https://job/ubuntu", calls[1][4])

    def test_other_branches_post_nothing(self):
        with mock.patch.object(qr, "gh_json", side_effect=AssertionError("no api call")):
            self.assertEqual(qr.report("o/r", "refs/heads/main", "9", RUN_URL), "no pull request in refs/heads/main")


if __name__ == "__main__":
    unittest.main()
