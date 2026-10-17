import importlib.util
import sys
import unittest
from pathlib import Path

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


if __name__ == "__main__":
    unittest.main()
