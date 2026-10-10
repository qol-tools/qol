import importlib.util
import sys
import unittest
from pathlib import Path
from unittest import mock

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
_SPEC = importlib.util.spec_from_file_location("stop_on_failure", SCRIPTS / "stop_on_failure.py")
sof = importlib.util.module_from_spec(_SPEC)
sys.modules["stop_on_failure"] = sof
_SPEC.loader.exec_module(sof)

SHA = "b28d8825b" + "0" * 31


def job(name: str, status: str = "in_progress", conclusion: str | None = None) -> dict:
    return {"name": name, "status": status, "conclusion": conclusion}


class FirstFailureTests(unittest.TestCase):
    def test_finds_a_failed_or_timed_out_job_but_not_the_gate(self):
        cases = [
            ([job("ubuntu", "completed", "success"), job("macos")], None),
            ([job("ubuntu", "completed", "failure"), job("macos")], "ubuntu"),
            ([job("windows", "completed", "timed_out")], "windows"),
            ([job("plan", "completed", "skipped"), job("macos", "completed", "cancelled")], None),
            ([job(sof.GATE, "completed", "failure")], None),
        ]
        for jobs, expected in cases:
            with self.subTest(jobs=jobs):
                failed = sof.first_failure(jobs)
                self.assertEqual(failed and failed["name"], expected)


class GateFinishedTests(unittest.TestCase):
    def test_only_a_completed_gate_ends_the_watch(self):
        self.assertFalse(sof.gate_finished([job("plan", "completed", "success")]))
        self.assertFalse(sof.gate_finished([job(sof.GATE, "queued")]))
        self.assertTrue(sof.gate_finished([job(sof.GATE, "completed", "success")]))


class WatchTests(unittest.TestCase):
    def watch(self, polls: list[list[dict]], reviews: list[dict]):
        responses = iter([{"jobs": jobs} for jobs in polls])

        def gh_json(args):
            if "/workflows/" in args[0]:
                self.assertIn(f"head_sha={SHA}", args[0])
                return {"workflow_runs": reviews}
            self.assertIn("runs/9/attempts/2/jobs", args[0])
            return next(responses)

        sleep = mock.Mock()
        with mock.patch.object(sof, "gh_json", gh_json), mock.patch.object(sof, "cancel") as cancel:
            message = sof.watch("o/r", "9", "2", SHA, sleep)
        return message, [call.args[1] for call in cancel.call_args_list], sleep.call_count

    def test_first_failure_cancels_the_open_review_then_this_run(self):
        message, cancelled, sleeps = self.watch(
            [[job("plan", "completed", "success"), job("ubuntu")], [job("ubuntu", "completed", "failure")]],
            [{"id": 5, "status": "in_progress"}, {"id": 4, "status": "completed"}],
        )
        self.assertEqual(cancelled, [5, "9"])
        self.assertEqual(sleeps, 1)
        self.assertEqual(message, "ubuntu failed: cancelled this run and 1 review run(s)")

    def test_a_green_run_ends_at_the_gate_without_cancelling(self):
        message, cancelled, _ = self.watch(
            [[job("ubuntu", "completed", "success"), job(sof.GATE, "completed", "success")]], []
        )
        self.assertEqual(cancelled, [])
        self.assertEqual(message, "every job finished")


class MainTests(unittest.TestCase):
    def test_a_refused_cancel_only_warns(self):
        env = {"GITHUB_REPOSITORY": "o/r", "GITHUB_RUN_ID": "9", "GITHUB_RUN_ATTEMPT": "1", "HEAD_SHA": SHA}
        with (
            mock.patch.dict(sof.os.environ, env),
            mock.patch.object(sof, "watch", side_effect=RuntimeError("HTTP 403")),
            mock.patch("sys.stderr") as stderr,
        ):
            self.assertEqual(sof.main(), 0)
        self.assertIn("HTTP 403", "".join(call.args[0] for call in stderr.write.call_args_list))


if __name__ == "__main__":
    unittest.main()
