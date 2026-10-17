import importlib.util
import sys
import unittest
from pathlib import Path
from unittest import mock

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
_SPEC = importlib.util.spec_from_file_location("wait_for_review", SCRIPTS / "wait_for_review.py")
wfr = importlib.util.module_from_spec(_SPEC)
sys.modules["wait_for_review"] = wfr
_SPEC.loader.exec_module(wfr)

SHA = "b28d8825b" + "0" * 31


def run(run_id: int, status: str = "in_progress", conclusion: str | None = None) -> dict:
    return {"id": run_id, "status": status, "conclusion": conclusion, "html_url": f"https://x/runs/{run_id}"}


class WaitTests(unittest.TestCase):
    def wait(self, polls: list[list[dict]]):
        responses = iter([{"workflow_runs": runs} for runs in polls])

        def gh_json(args):
            self.assertIn("workflows/claude-review.yml/runs", args[0])
            self.assertIn(f"head_sha={SHA}", args[0])
            return next(responses)

        sleep = mock.Mock()
        with mock.patch.object(wfr, "gh_json", gh_json):
            review = wfr.wait("o/r", SHA, sleep)
        return review, sleep.call_count

    def test_waits_for_the_review_to_appear_and_finish(self):
        review, sleeps = self.wait([[], [run(5)], [run(5, "completed", "success")]])
        self.assertEqual(review["id"], 5)
        self.assertEqual(sleeps, 2)

    def test_the_latest_run_of_the_commit_decides(self):
        review, sleeps = self.wait(
            [[run(4, "completed", "failure"), run(6)], [run(4, "completed", "failure"), run(6, "completed", "success")]]
        )
        self.assertEqual(review["id"], 6)
        self.assertEqual(sleeps, 1)


class MainTests(unittest.TestCase):
    def main(self, conclusion: str) -> tuple[int, str]:
        env = {"GITHUB_REPOSITORY": "o/r", "HEAD_SHA": SHA}
        with (
            mock.patch.dict(wfr.os.environ, env),
            mock.patch.object(wfr, "wait", return_value=run(7, "completed", conclusion)),
            mock.patch("sys.stdout") as stdout,
        ):
            code = wfr.main()
        return code, "".join(call.args[0] for call in stdout.write.call_args_list)

    def test_only_a_passed_review_lets_the_run_continue(self):
        for conclusion, expected in [("success", 0), ("failure", 1), ("cancelled", 1), ("timed_out", 1)]:
            with self.subTest(conclusion=conclusion):
                code, output = self.main(conclusion)
                self.assertEqual(code, expected)
                self.assertIn("https://x/runs/7", output)


if __name__ == "__main__":
    unittest.main()
