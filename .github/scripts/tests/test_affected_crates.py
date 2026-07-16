import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

_SPEC = importlib.util.spec_from_file_location(
    "affected_crates", Path(__file__).resolve().parents[1] / "affected_crates.py"
)
ac = importlib.util.module_from_spec(_SPEC)
sys.modules["affected_crates"] = ac
_SPEC.loader.exec_module(ac)


class PlatformExcludeDerivation(unittest.TestCase):
    def test_excludes_derived_from_plugin_platforms(self):
        ubuntu, macos = ac.platform_excludes()
        cases = [
            ("keyremap", ubuntu, True),
            ("plugin-removeapp", ubuntu, True),
            ("plugin-os-themes", ubuntu, False),
            ("plugin-os-themes", macos, True),
            ("keyremap", macos, False),
            ("alt-tab", ubuntu, False),
            ("alt-tab", macos, False),
        ]
        for package, excluded, expected in cases:
            self.assertEqual(
                package in excluded, expected, f"{package} in {sorted(excluded)}"
            )

    def test_exclude_flags_sorted_and_spaced(self):
        self.assertEqual(ac.exclude_flags(set()), "")
        self.assertEqual(
            ac.exclude_flags({"b", "a"}), " --exclude a --exclude b"
        )


class LocalPlannerContract(unittest.TestCase):
    @patch.object(ac, "run")
    def test_worktree_diff_includes_untracked_files(self, run):
        run.side_effect = [
            subprocess.CompletedProcess([], 0, "tools/qol-cli/src/main.rs\n", ""),
            subprocess.CompletedProcess([], 0, "new-file.txt\n", ""),
        ]

        self.assertEqual(
            ac.changed_files("base", ac.WORKTREE_HEAD),
            ["new-file.txt", "tools/qol-cli/src/main.rs"],
        )
        self.assertEqual(
            run.call_args_list[0].args[0],
            ["git", "diff", "--name-only", "base"],
        )

    def test_emit_writes_structured_local_output(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "affected.json"
            github_output = Path(directory) / "github-output"
            with patch.dict(
                os.environ,
                {
                    "GITHUB_OUTPUT": str(github_output),
                    "QOL_AFFECTED_OUTPUT": str(output),
                },
                clear=True,
            ):
                ac.emit(
                    {
                        "ubuntu_skip": "true",
                        "ubuntu_test": "",
                        "windows_process": True,
                        "windows_qol": False,
                    }
                )

            self.assertEqual(
                json.loads(output.read_text()),
                {
                    "ubuntu_skip": "true",
                    "ubuntu_test": "",
                    "windows_process": True,
                    "windows_qol": False,
                },
            )
            self.assertEqual(
                github_output.read_text(),
                "ubuntu_skip=true\nubuntu_test=\n"
                "windows_process=true\nwindows_qol=false\n",
            )

    def test_terminal_plans_set_windows_targets(self):
        cases = [(ac.full_workspace, True), (ac.skip_all, False)]
        for planner, expected in cases:
            with self.subTest(planner=planner.__name__):
                with patch.object(ac, "emit") as emit:
                    planner("test")

                    self.assertIs(
                        emit.call_args.args[0]["windows_process"], expected
                    )
                    self.assertIs(
                        emit.call_args.args[0]["windows_qol"], expected
                    )

    @patch.object(ac, "full_workspace")
    @patch.object(ac, "changed_files")
    def test_global_change_uses_full_workspace(self, changed_files, full_workspace):
        changed_files.return_value = [".github/workflows/ci.yml"]
        with patch.dict(os.environ, {"BASE_SHA": "base", "HEAD_SHA": "head"}):
            ac.main()

        full_workspace.assert_called_once_with(
            "global file changed: .github/workflows/ci.yml"
        )

    @patch.object(ac, "emit")
    @patch.object(ac, "workspace_graph")
    @patch.object(ac, "changed_files")
    def test_windows_targets_track_affected_packages(self, changed_files, graph, emit):
        graph.return_value = {
            "foundation": {"dir": "libs/foundation", "deps": set()},
            "qol-process": {"dir": "libs/qol-process", "deps": {"foundation"}},
            "qol": {"dir": "tools/qol-cli", "deps": {"qol-process"}},
            "unrelated": {"dir": "libs/unrelated", "deps": set()},
        }
        cases = [
            ("libs/qol-process/src/lib.rs", True, True),
            ("libs/foundation/src/lib.rs", True, True),
            ("tools/qol-cli/src/main.rs", False, True),
            ("libs/unrelated/src/lib.rs", False, False),
        ]
        with patch.dict(os.environ, {"BASE_SHA": "base", "HEAD_SHA": "head"}):
            for path, process_expected, qol_expected in cases:
                with self.subTest(path=path):
                    changed_files.return_value = [path]
                    emit.reset_mock()

                    ac.main()

                    self.assertIs(
                        emit.call_args.args[0]["windows_process"],
                        process_expected,
                    )
                    self.assertIs(
                        emit.call_args.args[0]["windows_qol"], qol_expected
                    )


if __name__ == "__main__":
    unittest.main()
