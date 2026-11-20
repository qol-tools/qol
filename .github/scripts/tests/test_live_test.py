import importlib.util
import re
import subprocess
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
            "plugins/lights/qol-config.toml",
            "plugins/lights/qol-runtime.toml",
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
                "plugins/lights/qol-config.toml",
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


REPO = SCRIPTS.parents[1]
PAGE_CONTENT = {"Page", "Group", "Row", "Notice", "Control", "Tone"}
PAGES_USE = re.compile(r"\buse\s+qol_config::pages::(\{[^}]*\}|\w+)")
PAGES_PATH = re.compile(r"\bpages::(?:" + "|".join(sorted(PAGE_CONTENT)) + r")\b")
UI_FOLDER = re.compile(r"(?:^|/)src/(?:.+/)?ui/")


def builds_pages(text: str) -> bool:
    if "qol_config::pages" not in text:
        return False
    for found in PAGES_USE.finditer(text):
        if PAGE_CONTENT & set(re.findall(r"\w+", found.group(1))):
            return True
    return bool(PAGES_PATH.search(text))


def draws(path: str, text: str) -> bool:
    if path.startswith("vendor/"):
        return False
    return bool(UI_FOLDER.search(path)) or (path.endswith(".rs") and builds_pages(text))


class UiCoverageTests(unittest.TestCase):
    def test_page_content_counts_and_the_press_plumbing_does_not(self):
        cases = [
            ("use qol_config::pages::{Go, Group, Page};", True),
            ("use qol_config::pages::{\n    Go,\n    Row,\n};", True),
            ("use qol_config::pages::Notice;", True),
            ("use qol_config::pages;\nfn f() { pages::Row::new(\"a\", \"b\"); }", True),
            ("let x: qol_config::pages::Group = todo!();", True),
            ("use qol_config::pages::{Go, Press, PressReply, Stack};", False),
            ("let pages: qol_config::pages::Pages = todo!();", False),
            ("use crate::pages::Row;", False),
        ]
        for text, expected in cases:
            with self.subTest(text=text):
                self.assertEqual(builds_pages(text), expected)

    def test_a_file_draws_when_it_sits_under_src_ui_or_builds_pages(self):
        self.assertTrue(draws("plugins/a/src/ui/view.rs", ""))
        self.assertTrue(draws("apps/tray/src/app/ui/strip.rs", ""))
        self.assertTrue(draws("plugins/a/src/pages/home.rs", "use qol_config::pages::Row;"))
        self.assertFalse(draws("plugins/a/src/uix/view.rs", ""))
        self.assertFalse(draws("plugins/a/README.md", "use qol_config::pages::Row;"))
        self.assertFalse(draws("vendor/gpui/src/ui/a.rs", ""))

    def test_every_file_that_draws_is_on_the_live_test_list(self):
        listed = live.patterns(live.PATHS_FILE.read_text(encoding="utf-8"))
        tracked = subprocess.run(
            ["git", "ls-files", "-z"], cwd=REPO, capture_output=True, check=True, text=True
        ).stdout.split("\0")
        drawing = []
        for path in filter(None, tracked):
            file = REPO / path
            if not file.is_file():
                continue
            text = file.read_text(encoding="utf-8", errors="replace") if path.endswith(".rs") else ""
            if draws(path, text):
                drawing.append(path)
        self.assertTrue(drawing)
        missing = sorted(set(drawing) - set(live.needing(drawing, listed)))
        self.assertEqual(
            missing,
            [],
            "these files draw or build what the tray shows, so add a glob for them to .github/live-test-paths",
        )


class SummaryTests(unittest.TestCase):
    def test_the_title_says_whether_a_live_test_is_needed(self):
        self.assertEqual(live.summary([])[0], live.NOT_NEEDED_TITLE)
        title, text = live.summary(["a.rs"])
        self.assertEqual(title, live.NEEDED_TITLE)
        self.assertIn("- `a.rs`", text)
        for artifact in live.BUILD_ARTIFACTS.values():
            self.assertIn(f"`{artifact}`", text)
            self.assertNotIn(artifact, live.summary([])[1])

    def test_a_long_list_is_cut(self):
        found = [f"f{index}.rs" for index in range(live.LISTED_FILES + 3)]
        text = live.summary(found)[1]
        self.assertIn("- and 3 more", text)
        self.assertNotIn(f"f{live.LISTED_FILES}.rs", text)


if __name__ == "__main__":
    unittest.main()
