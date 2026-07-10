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
            with patch.dict(
                os.environ, {"QOL_AFFECTED_OUTPUT": str(output)}, clear=True
            ):
                ac.emit({"ubuntu_skip": "true", "ubuntu_test": ""})

            self.assertEqual(
                json.loads(output.read_text()),
                {"ubuntu_skip": "true", "ubuntu_test": ""},
            )


if __name__ == "__main__":
    unittest.main()
