import contextlib
import importlib.util
import io
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
_SPEC = importlib.util.spec_from_file_location("release_notes", SCRIPTS / "release_notes.py")
rn = importlib.util.module_from_spec(_SPEC)
sys.modules["release_notes"] = rn
_SPEC.loader.exec_module(rn)

PR = " by @KMRH47 in https://github.com/qol-tools/qol/pull/1"
CHANGELOG = "**Full Changelog**: https://github.com/qol-tools/qol/compare/a...b"


def notes(*titles: str) -> str:
    entries = "\n".join(f"* {title}{PR}" for title in titles)
    return f"## What's Changed\n{entries}\n\n\n{CHANGELOG}"


class IsUserFacing(unittest.TestCase):
    def test_types_and_scopes(self):
        cases = [
            ("feat(settings): hide settings", True),
            ("fix(os-themes): crisp cursor", True),
            ("perf: faster start", True),
            ("refactor!: drop the old api", True),
            ("ci: start tests after review", False),
            ("fix(ci): test only in the merge queue", False),
            ("feat(CI): new gate", False),
            ("refactor(tray): split module", False),
            ("docs: link tiles", False),
            ("chore(plugins): bump plugin versions", False),
            ("test(cli): cover parser", False),
            ('Revert "fix(os-themes): crisp cursor"', True),
            ('Revert "fix(ci): test only in the merge queue"', False),
            ('Revert "docs: link tiles"', False),
            ("Update README", False),
        ]
        for title, expected in cases:
            self.assertEqual(rn.is_user_facing(title), expected, title)


class FilterNotes(unittest.TestCase):
    def test_keeps_only_user_facing_entries(self):
        body = notes("ci: gate", "feat(settings): hide", "fix(ci): queue")

        self.assertEqual(rn.filter_notes(body), notes("feat(settings): hide"))

    def test_all_internal_entries_leave_a_note(self):
        body = notes("ci: gate", "refactor: split")

        self.assertEqual(
            rn.filter_notes(body),
            f"## What's Changed\n{rn.EMPTY_NOTE}\n\n\n{CHANGELOG}",
        )

    def test_new_contributors_section_is_untouched(self):
        contributors = "## New Contributors\n* @someone made their first contribution in https://x/pull/2"
        body = f"{notes('feat: a', 'ci: b')}\n\n{contributors}"

        self.assertEqual(rn.filter_notes(body), f"{notes('feat: a')}\n\n{contributors}")

    def test_body_without_changes_section_is_unchanged(self):
        self.assertEqual(rn.filter_notes(CHANGELOG), CHANGELOG)

    def test_unit_pulls_keep_only_the_pull_requests_released_for_the_unit(self):
        body = (
            "## What's Changed\n"
            "* fix(tray): a by @KMRH47 in https://github.com/qol-tools/qol/pull/250\n"
            "* fix(explorer): b by @KMRH47 in https://github.com/qol-tools/qol/pull/251\n"
            "* feat(gpui): c by @KMRH47 in https://github.com/qol-tools/qol/commit/abc\n"
        )

        self.assertEqual(
            rn.filter_notes(body, {"250"}),
            "## What's Changed\n"
            "* fix(tray): a by @KMRH47 in https://github.com/qol-tools/qol/pull/250\n"
            "* feat(gpui): c by @KMRH47 in https://github.com/qol-tools/qol/commit/abc\n",
        )

    def test_no_pull_released_for_the_unit_leaves_a_note(self):
        body = notes("fix(explorer): b")

        self.assertEqual(
            rn.filter_notes(body, set()),
            f"## What's Changed\n{rn.EMPTY_NOTE}\n\n\n{CHANGELOG}",
        )


class PullNumbers(unittest.TestCase):
    def test_reads_the_pull_request_from_squash_subjects(self):
        commits = [
            rn.pv.Commit(sha="a", subject="fix(explorer): drag the window (#251)", body=""),
            rn.pv.Commit(sha="b", subject="fix(tray): direct push", body=""),
            rn.pv.Commit(sha="c", subject="fix(tray): see #12 for context", body=""),
        ]

        self.assertEqual(rn.pull_numbers(commits), {"251"})


def git(root: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(root), *args], capture_output=True, text=True, check=True
    ).stdout.strip()


class UnitPulls(unittest.TestCase):
    def test_lists_the_pull_requests_whose_commits_count_toward_the_unit(self):
        crate = '[package]\nname = "{}"\nversion = "0.1.0"\n\n[dependencies]\nshared = {{ path = "../../libs/shared" }}\n'
        files = {
            "Cargo.toml": '[workspace]\nresolver = "2"\nmembers = ["apps/*", "libs/*", "plugins/*"]\n',
            "libs/shared/Cargo.toml": '[package]\nname = "shared"\nversion = "0.1.0"\n',
            "apps/tray/Cargo.toml": crate.format("qol-tray"),
            "plugins/alpha/Cargo.toml": crate.format("qol-alpha"),
            "plugins/alpha/plugin.toml": (
                '[plugin]\nid = "qol-alpha"\nname = "X"\ndescription = "x"\n'
                'version = "0.1.0"\nplatforms = ["linux"]\n'
            ),
        }
        changes = [
            ("feat: baseline", "README.md"),
            ("fix(alpha): shared (#11)", "libs/shared/lib.rs"),
            ("fix(tray): tray (#12)", "apps/tray/main.rs"),
            ("fix: shared (#13)", "libs/shared/lib.rs"),
            ("fix(tray): notes (#14)", "docs/notes.md"),
        ]
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            git(root, "init", "--quiet")
            git(root, "config", "user.name", "Fixture")
            git(root, "config", "user.email", "fixture@example.invalid")
            for path, text in files.items():
                (root / path).parent.mkdir(parents=True, exist_ok=True)
                (root / path).write_text(text)
            for subject, path in changes:
                (root / path).parent.mkdir(parents=True, exist_ok=True)
                (root / path).write_text(subject)
                git(root, "add", ".")
                git(root, "commit", "--quiet", "-m", subject)
            base = git(root, "rev-list", "--max-parents=0", "HEAD")

            self.assertEqual(rn.unit_pulls(root, "qol-tray", f"{base}..HEAD"), {"12", "13"})


class Main(unittest.TestCase):
    BODY = notes("feat(tray): a", "ci: b")

    def run_main(self, *argv: str) -> tuple[str, str]:
        stdout, stderr = io.StringIO(), io.StringIO()
        with mock.patch.object(sys, "argv", ["release_notes.py", *argv]), mock.patch.object(
            sys, "stdin", io.StringIO(self.BODY)
        ), contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            self.assertEqual(rn.main(), 0)
        return stdout.getvalue(), stderr.getvalue()

    def test_without_a_unit_every_user_facing_pull_request_stays(self):
        self.assertEqual(self.run_main(), (rn.filter_notes(self.BODY), ""))

    def test_a_unit_keeps_only_its_pull_requests(self):
        with mock.patch.object(rn, "unit_pulls", return_value=set()) as pulls:
            output = self.run_main("--root", "/x", "--unit", "qol-tray", "--range", "a..b")

        pulls.assert_called_once_with(Path("/x"), "qol-tray", "a..b")
        self.assertEqual(output, (rn.filter_notes(self.BODY, set()), ""))

    def test_a_unit_without_a_range_is_refused(self):
        with self.assertRaises(SystemExit) as refused:
            self.run_main("--unit", "qol-tray")

        self.assertEqual(refused.exception.code, 2)

    def test_a_planning_failure_warns_and_keeps_every_pull_request(self):
        with mock.patch.object(rn, "unit_pulls", side_effect=RuntimeError("git failed")):
            stdout, stderr = self.run_main("--unit", "qol-tray", "--range", "a..b")

        self.assertEqual(stdout, rn.filter_notes(self.BODY))
        self.assertIn("::warning::", stderr)
        self.assertIn("git failed", stderr)


if __name__ == "__main__":
    unittest.main()
