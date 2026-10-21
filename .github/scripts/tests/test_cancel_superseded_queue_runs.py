import importlib.util
import sys
import unittest
from pathlib import Path
from unittest import mock

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
_SPEC = importlib.util.spec_from_file_location(
    "cancel_superseded_queue_runs", SCRIPTS / "cancel_superseded_queue_runs.py"
)
csq = importlib.util.module_from_spec(_SPEC)
sys.modules["cancel_superseded_queue_runs"] = csq
_SPEC.loader.exec_module(csq)

OLD_BASE = "7e60d77947c7a54d10306a0db3655f13743ce1c0"
NEW_BASE = "96cf827b69354624ddbc39075ba8221091dac7f8"


def run(run_id: int, branch: str, status: str = "in_progress") -> dict:
    return {"id": run_id, "head_branch": branch, "status": status}


class QueuePrTests(unittest.TestCase):
    def test_reads_the_pull_request_from_a_queue_ref_only(self):
        cases = [
            (f"refs/heads/gh-readonly-queue/main/pr-185-{OLD_BASE}", "185"),
            (f"gh-readonly-queue/main/pr-7-{NEW_BASE}", "7"),
            ("refs/heads/main", None),
            ("refs/heads/pr-185-feature", None),
            (f"gh-readonly-queue/main/pr-18-{OLD_BASE}-extra", None),
        ]
        for ref, expected in cases:
            with self.subTest(ref=ref):
                self.assertEqual(csq.queue_pr(ref), expected)


class SupersededTests(unittest.TestCase):
    def test_only_older_unfinished_runs_of_the_same_pull_request(self):
        runs = [
            run(10, f"gh-readonly-queue/main/pr-185-{OLD_BASE}"),
            run(11, f"gh-readonly-queue/main/pr-185-{OLD_BASE}", "queued"),
            run(12, f"gh-readonly-queue/main/pr-185-{OLD_BASE}", "completed"),
            run(13, f"gh-readonly-queue/main/pr-1850-{OLD_BASE}"),
            run(14, f"gh-readonly-queue/main/pr-186-{OLD_BASE}"),
            run(20, f"gh-readonly-queue/main/pr-185-{NEW_BASE}"),
            run(21, f"gh-readonly-queue/main/pr-185-{NEW_BASE}"),
        ]
        self.assertEqual(csq.superseded(runs, "185", 20), [10, 11])


class CancelTests(unittest.TestCase):
    def test_posts_the_cancel_and_raises_when_it_is_refused(self):
        ok = mock.Mock(returncode=0, stdout="", stderr="")
        with mock.patch.object(csq.subprocess, "run", return_value=ok) as run_gh:
            csq.cancel("o/r", 10)
        self.assertEqual(run_gh.call_args.args[0], ["gh", "api", "--method", "POST", "repos/o/r/actions/runs/10/cancel"])
        refused = mock.Mock(returncode=1, stdout="", stderr="HTTP 403")
        with mock.patch.object(csq.subprocess, "run", return_value=refused), self.assertRaisesRegex(RuntimeError, "HTTP 403"):
            csq.cancel("o/r", 10)


class CancelSupersededTests(unittest.TestCase):
    def test_cancels_each_superseded_run(self):
        runs = {"workflow_runs": [run(10, f"gh-readonly-queue/main/pr-185-{OLD_BASE}")]}
        with (
            mock.patch.object(csq, "gh_json", return_value=runs) as gh_json,
            mock.patch.object(csq, "cancel") as cancel,
        ):
            message = csq.cancel_superseded("o/r", "5", f"refs/heads/gh-readonly-queue/main/pr-185-{NEW_BASE}", 20)
        self.assertIn("workflows/5/runs?event=merge_group", gh_json.call_args.args[0][0])
        self.assertEqual([call.args for call in cancel.call_args_list], [("o/r", 10)])
        self.assertEqual(message, "cancelled 1 superseded run(s) for #185: [10]")

    def test_a_failed_cancel_still_cancels_the_rest(self):
        runs = {
            "workflow_runs": [
                run(10, f"gh-readonly-queue/main/pr-185-{OLD_BASE}"),
                run(11, f"gh-readonly-queue/main/pr-185-{OLD_BASE}"),
            ]
        }

        def cancel(repo, run_id):
            if run_id == 10:
                raise RuntimeError("run 10 already finished")

        with (
            mock.patch.object(csq, "gh_json", return_value=runs),
            mock.patch.object(csq, "cancel", side_effect=cancel) as cancelled,
        ):
            message = csq.cancel_superseded("o/r", "5", f"gh-readonly-queue/main/pr-185-{NEW_BASE}", 20)
        self.assertEqual([call.args[1] for call in cancelled.call_args_list], [10, 11])
        self.assertEqual(message, "cancelled 1 superseded run(s) for #185: [11]; could not cancel [10]")

    def test_a_non_queue_ref_queries_nothing(self):
        with mock.patch.object(csq, "gh_json") as gh_json:
            self.assertEqual(csq.cancel_superseded("o/r", "5", "refs/heads/main", 20), "refs/heads/main is not a merge queue ref")
        gh_json.assert_not_called()


class MainTests(unittest.TestCase):
    def test_a_refused_cancel_only_warns(self):
        env = {"GITHUB_REPOSITORY": "o/r", "GITHUB_RUN_ID": "20", "HEAD_REF": f"gh-readonly-queue/main/pr-185-{NEW_BASE}"}
        with (
            mock.patch.dict(csq.os.environ, env),
            mock.patch.object(csq, "gh_json", side_effect=RuntimeError("HTTP 403")),
            mock.patch("sys.stderr") as stderr,
        ):
            self.assertEqual(csq.main(), 0)
        self.assertIn("HTTP 403", "".join(call.args[0] for call in stderr.write.call_args_list))


if __name__ == "__main__":
    unittest.main()
