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
            ("plugin-removeapp", ubuntu, False),
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
                        "full": False,
                        "ubuntu_build": "",
                        "ubuntu_skip": True,
                        "ubuntu_test": "",
                        "macos_build": "",
                        "windows_process": True,
                        "windows_dev_build": True,
                        "windows_qol": False,
                    }
                )

            self.assertEqual(
                json.loads(output.read_text()),
                {
                    "full": False,
                    "macos_build": "",
                    "ubuntu_build": "",
                    "ubuntu_skip": True,
                    "ubuntu_test": "",
                    "windows_process": True,
                    "windows_dev_build": True,
                    "windows_qol": False,
                },
            )
            self.assertEqual(
                github_output.read_text(),
                "full=false\nubuntu_build=\nubuntu_skip=true\nubuntu_test=\n"
                "macos_build=\n"
                "windows_process=true\nwindows_dev_build=true\n"
                "windows_qol=false\n",
            )

    def test_terminal_plans_set_windows_targets(self):
        cases = [(ac.full_workspace, True), (ac.skip_all, False)]
        for planner, expected in cases:
            with self.subTest(planner=planner.__name__):
                with patch.object(ac, "emit") as emit:
                    planner("test")

                    self.assertIs(emit.call_args.args[0]["full"], expected)
                    self.assertIs(emit.call_args.args[0]["ubuntu_doctest"], expected)
                    self.assertIs(emit.call_args.args[0]["macos_doctest"], expected)
                    self.assertIs(
                        emit.call_args.args[0]["windows_process"], expected
                    )
                    self.assertIs(
                        emit.call_args.args[0]["windows_dev_build"], expected
                    )
                    self.assertIs(
                        emit.call_args.args[0]["windows_qol"], expected
                    )
                    self.assertIs(
                        emit.call_args.args[0]["ubuntu_skip"], not expected
                    )
                    build_args = emit.call_args.args[0]["ubuntu_build"]
                    self.assertEqual(bool(build_args), expected)
                    self.assertIs(
                        emit.call_args.args[0]["macos_skip"], not expected
                    )

    @patch.object(ac, "full_workspace")
    @patch.object(ac, "changed_files")
    def test_global_change_uses_full_workspace(self, changed_files, full_workspace):
        for path in [".github/workflows/ci.yml", ".gitattributes", ".gitmodules"]:
            with self.subTest(path=path):
                changed_files.return_value = [path]
                full_workspace.reset_mock()
                with patch.dict(
                    os.environ, {"BASE_SHA": "base", "HEAD_SHA": "head"}
                ):
                    ac.main()

                full_workspace.assert_called_once_with(f"global file changed: {path}")

    @patch.object(ac, "run")
    def test_workspace_metadata_is_locked(self, run):
        run.return_value.returncode = 1

        self.assertIsNone(ac.workspace_graph())

        self.assertEqual(
            run.call_args.args[0],
            ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
        )

    @patch.object(ac, "emit")
    @patch.object(ac, "workspace_graph")
    @patch.object(ac, "changed_files")
    def test_windows_targets_track_affected_packages(self, changed_files, graph, emit):
        graph.return_value = {
            "foundation": {"dir": "libs/foundation", "deps": set(), "doctest": True},
            "qol-process": {"dir": "libs/qol-process", "deps": {"foundation"}, "doctest": True},
            "qol-dev-build": {
                "dir": "libs/qol-dev-build",
                "deps": {"qol-process"},
                "doctest": True,
            },
            "qol": {"dir": "tools/qol-cli", "deps": {"qol-dev-build"}, "doctest": False},
            "unrelated": {"dir": "libs/unrelated", "deps": set(), "doctest": True},
        }
        cases = [
            ("libs/qol-process/src/lib.rs", True, True, True),
            ("libs/foundation/src/lib.rs", True, True, True),
            ("libs/qol-dev-build/src/lib.rs", False, True, True),
            ("tools/qol-cli/src/main.rs", False, False, True),
            ("libs/unrelated/src/lib.rs", False, False, False),
        ]
        with patch.dict(os.environ, {"BASE_SHA": "base", "HEAD_SHA": "head"}):
            for path, process_expected, dev_build_expected, qol_expected in cases:
                with self.subTest(path=path):
                    changed_files.return_value = [path]
                    emit.reset_mock()

                    ac.main()

                    self.assertIs(
                        emit.call_args.args[0]["windows_process"],
                        process_expected,
                    )
                    self.assertIs(
                        emit.call_args.args[0]["windows_dev_build"],
                        dev_build_expected,
                    )
                    self.assertIs(
                        emit.call_args.args[0]["windows_qol"], qol_expected
                    )
                    self.assertIs(
                        emit.call_args.args[0]["ubuntu_doctest"],
                        path != "tools/qol-cli/src/main.rs",
                    )

    def test_documentation_targets_follow_cargo_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers=["binary", "library", "disabled"]\nresolver="2"\n'
            )
            for name in ["binary", "library", "disabled"]:
                package = root / name
                (package / "src").mkdir(parents=True)
                manifest = f'[package]\nname="{name}"\nversion="0.1.0"\nedition="2021"\n'
                if name == "disabled":
                    manifest += '[lib]\ndoctest=false\n'
                (package / "Cargo.toml").write_text(manifest)
                source = "main.rs" if name == "binary" else "lib.rs"
                (package / "src" / source).write_text("fn main() {}" if name == "binary" else "")
            subprocess.run(
                ["cargo", "generate-lockfile", "--offline"], cwd=root, check=True,
                capture_output=True,
            )
            with patch.object(ac, "run", side_effect=lambda argv: subprocess.run(
                argv, cwd=root, capture_output=True, text=True,
            )):
                graph = ac.workspace_graph()
            self.assertEqual(
                {name: package["doctest"] for name, package in graph.items()},
                {"binary": False, "library": True, "disabled": False},
            )


if __name__ == "__main__":
    unittest.main()
