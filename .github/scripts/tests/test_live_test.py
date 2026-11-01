import importlib.util
import sys
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
_SPEC = importlib.util.spec_from_file_location("live_test", SCRIPTS / "live_test.py")
live = importlib.util.module_from_spec(_SPEC)
sys.modules["live_test"] = live
_SPEC.loader.exec_module(live)


class PatternTests(unittest.TestCase):
    def test_globs_match_whole_paths_by_segment(self):
        cases = [
            ("plugins/*/src/ui/**", "plugins/shot/src/ui/overlay.rs", True),
            ("plugins/*/src/ui/**", "plugins/shot/src/ui/a/b.rs", True),
            ("plugins/*/src/ui/**", "plugins/shot/src/uix/a.rs", False),
            ("plugins/*/src/ui/**", "plugins/a/b/src/ui/c.rs", False),
            ("libs/gpui/**", "libs/gpui/Cargo.toml", True),
            ("libs/gpui/**", "libs/gpuix/Cargo.toml", False),
            ("**/view.rs", "view.rs", True),
            ("**/view.rs", "a/b/view.rs", True),
            ("a/?.rs", "a/b.rs", True),
            ("a/?.rs", "a/bc.rs", False),
            ("a.rs", "a.rs.bak", False),
            ("a.rs", "xa.rs", False),
        ]
        for pattern, path, expected in cases:
            with self.subTest(pattern=pattern, path=path):
                self.assertEqual(bool(live.pattern_regex(pattern).match(path)), expected)

    def test_the_repository_list_flags_ui_and_leaves_logic_alone(self):
        listed = live.patterns(live.PATHS_FILE.read_text(encoding="utf-8"))
        files = [
            "plugins/cli-sessions/src/ui/render.rs",
            "libs/sessions-engine/src/switchboard/rows.rs",
            "apps/tray/src/settings_surface/mod.rs",
            "plugins/cli-sessions/src/attention/mod.rs",
            "libs/sessions-engine/src/switchboard/resume.rs",
            ".github/workflows/ci.yml",
            "README.md",
        ]
        self.assertEqual(
            live.needing(files, listed),
            [
                "plugins/cli-sessions/src/ui/render.rs",
                "libs/sessions-engine/src/switchboard/rows.rs",
                "apps/tray/src/settings_surface/mod.rs",
            ],
        )

    def test_a_rename_counts_both_its_old_and_new_path(self):
        entries = [
            {"filename": "libs/sessions-engine/src/rows.rs", "previous_filename": "libs/gpui/src/rows.rs"},
            {"filename": "README.md"},
        ]
        paths = live.paths_of(entries)
        self.assertEqual(paths, ["libs/sessions-engine/src/rows.rs", "libs/gpui/src/rows.rs", "README.md"])
        self.assertEqual(live.needing(paths, ["libs/gpui/**"]), ["libs/gpui/src/rows.rs"])

    def test_blank_lines_are_skipped(self):
        self.assertEqual(live.patterns("a/**\n\n  b.rs  \n"), ["a/**", "b.rs"])


class SummaryTests(unittest.TestCase):
    def test_the_title_says_whether_a_live_test_is_needed(self):
        self.assertEqual(live.summary([])[0], live.NOT_NEEDED_TITLE)
        title, text = live.summary(["a.rs"])
        self.assertEqual(title, live.NEEDED_TITLE)
        self.assertIn("- `a.rs`", text)
        self.assertIn(f"`{live.BUILD_ARTIFACT}`", text)
        self.assertNotIn(live.BUILD_ARTIFACT, live.summary([])[1])

    def test_a_long_list_is_cut(self):
        found = [f"f{index}.rs" for index in range(live.LISTED_FILES + 3)]
        text = live.summary(found)[1]
        self.assertIn("- and 3 more", text)
        self.assertNotIn(f"f{live.LISTED_FILES}.rs", text)


if __name__ == "__main__":
    unittest.main()
