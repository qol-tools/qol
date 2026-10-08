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


def note(message: str, path: str = ".github", line: int | None = None, level: str = "failure") -> dict:
    return {"message": message, "path": path, "start_line": line, "annotation_level": level}


class ErrorsTests(unittest.TestCase):
    def test_keeps_failures_with_their_place_and_drops_exit_codes(self):
        lines = qr.errors(
            [
                note("This edit makes a test race\n a short clock", "libs/a.rs", 12),
                note("plan failed"),
                note("Process completed with exit code 1."),
                note("deprecated", level="warning"),
            ]
        )
        self.assertEqual(lines, ["- `libs/a.rs:12` This edit makes a test race a short clock", "- plan failed"])


class CommentTests(unittest.TestCase):
    def test_is_a_caution_alert_naming_each_failed_job_and_its_errors(self):
        body = qr.comment([(job("Plan affected crates", "failure"), ["- boom"]), (job("gate", "failure"), [])], RUN_URL)
        self.assertEqual(
            body,
            "> [!CAUTION]\n"
            "> **Merge queue failed** · [Plan affected crates](https://job/Plan affected crates), [gate](https://job/gate)\n"
            "> Queueing this head again fails the same way unless the failure was flaky. Push a fix first.\n\n"
            "#### Plan affected crates\n\n- boom\n\n"
            f"<sub>merge queue · [queue run]({RUN_URL})</sub>",
        )

    def test_caps_the_errors_per_job(self):
        body = qr.comment([(job("lint", "failure"), [f"- e{i}" for i in range(12)])], RUN_URL)
        self.assertIn("- e9\n- 2 more in [the log](https://job/lint)", body)
        self.assertNotIn("- e10", body)


class ReportTests(unittest.TestCase):
    def test_comments_and_fails_the_pull_request_head(self):
        calls = []

        def gh(args):
            calls.append(args)
            if args[0].endswith("/jobs?per_page=100"):
                return {"jobs": [dict(job("ubuntu", "failure"), id=7)]}
            if args[0].endswith("/annotations"):
                return [note("boom", "a.rs", 3)]
            if args[0] == "repos/o/r/pulls/40":
                return {"head": {"sha": SHA}}
            return {}

        with mock.patch.object(qr, "gh_json", gh):
            result = qr.report("o/r", f"refs/heads/gh-readonly-queue/main/pr-40-{SHA}", "9", RUN_URL)
        self.assertEqual(result, f"commented on #40 and failed {SHA[:7]}")
        self.assertEqual(calls[1], ["repos/o/r/check-runs/7/annotations"])
        self.assertEqual(calls[2][:3], ["--method", "POST", "repos/o/r/issues/40/comments"])
        self.assertIn("`a.rs:3` boom", calls[2][4])
        self.assertEqual(
            calls[4],
            [
                "--method", "POST", f"repos/o/r/statuses/{SHA}",
                "-f", "state=failure", "-f", "context=merge queue",
                "-f", "description=Sent back: ubuntu", "-f", f"target_url={RUN_URL}",
            ],
        )

    def test_other_branches_post_nothing(self):
        with mock.patch.object(qr, "gh_json", side_effect=AssertionError("no api call")):
            self.assertEqual(qr.report("o/r", "refs/heads/main", "9", RUN_URL), "no pull request in refs/heads/main")


if __name__ == "__main__":
    unittest.main()
