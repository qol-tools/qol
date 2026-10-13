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
        ubuntu, macos = ac.platform_sets()
        cases = [
            ("qol-keyremap", ubuntu, True),
            ("qol-removeapp", ubuntu, False),
            ("qol-os-themes", ubuntu, False),
            ("qol-os-themes", macos, True),
            ("qol-keyremap", macos, False),
            ("qol-alt-tab", ubuntu, False),
            ("qol-alt-tab", macos, False),
        ]
        for package, excluded, expected in cases:
            self.assertEqual(
                package in excluded, expected, f"{package} in {sorted(excluded)}"
            )

    def test_declared_platforms_decide_each_platform_set(self):
        cases = [
            ("qol-all", '["linux", "macos", "windows"]', (False, False)),
            ("qol-unix", '["linux", "macos"]', (False, False)),
            ("qol-mac-win", '["macos", "windows"]', (True, False)),
            ("qol-linux-win", '["linux", "windows"]', (False, True)),
            ("qol-undeclared", None, (False, True)),
        ]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for package, platforms, _ in cases:
                plugin = root / "plugins" / package
                plugin.mkdir(parents=True)
                declared = f"platforms = {platforms}\n" if platforms else ""
                (plugin / "plugin.toml").write_text(f"[plugin]\n{declared}")
                (plugin / "Cargo.toml").write_text(f'[package]\nname = "{package}"\n')
            with patch.object(ac, "REPO_ROOT", root):
                ubuntu, macos = ac.platform_sets()
        for package, _, (no_ubuntu, no_macos) in cases:
            self.assertEqual(
                (package in ubuntu, package in macos),
                (no_ubuntu, no_macos),
                package,
            )

    def test_windows_excludes_plugins_that_do_not_list_windows(self):
        cases = [
            ("qol-all", '["linux", "macos", "windows"]', False),
            ("qol-unix", '["linux", "macos"]', True),
            ("qol-mac-win", '["macos", "windows"]', False),
            ("qol-undeclared", None, True),
        ]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for package, platforms, _ in cases:
                plugin = root / "plugins" / package
                plugin.mkdir(parents=True)
                declared = f"platforms = {platforms}\n" if platforms else ""
                (plugin / "plugin.toml").write_text(f"[plugin]\n{declared}")
                (plugin / "Cargo.toml").write_text(f'[package]\nname = "{package}"\n')
            with patch.object(ac, "REPO_ROOT", root):
                excluded = ac.windows_excludes()
        for package, _, expected in cases:
            self.assertEqual(package in excluded, expected, package)

    def test_exclude_flags_sorted_and_spaced(self):
        self.assertEqual(ac.exclude_flags(set()), "")
        self.assertEqual(
            ac.exclude_flags({"b", "a"}), " --exclude a --exclude b"
        )


class LocalPlannerContract(unittest.TestCase):
    @patch.object(ac, "run")
    def test_worktree_diff_includes_untracked_files(self, run):
        run.side_effect = [
            subprocess.CompletedProcess([], 0, "tools/cli/src/main.rs\n", ""),
            subprocess.CompletedProcess([], 0, "new-file.txt\n", ""),
        ]

        self.assertEqual(
            ac.changed_files("base", ac.WORKTREE_HEAD),
            ["new-file.txt", "tools/cli/src/main.rs"],
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
                },
            )
            self.assertEqual(
                github_output.read_text(),
                "full=false\nubuntu_build=\nubuntu_skip=true\nubuntu_test=\n"
                "macos_build=\n"
                "windows_process=true\nwindows_dev_build=true\n",
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
                        emit.call_args.args[0]["windows_doctest"], expected
                    )
                    self.assertIs(
                        emit.call_args.args[0]["windows_skip"], not expected
                    )
                    for key in ["windows_clippy", "windows_build", "windows_test"]:
                        self.assertEqual(
                            bool(emit.call_args.args[0][key]), expected, key
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
        for path in [
            ".github/workflows/ci.yml",
            ".github/actions/rust-setup/action.yml",
            ".gitattributes",
            ".gitmodules",
            ".config/hakari.toml",
            "vendor/gpui/src/window.rs",
            "vendor/ravif/src/lib.rs",
        ]:
            with self.subTest(path=path):
                changed_files.return_value = [path]
                full_workspace.reset_mock()
                with patch.dict(
                    os.environ, {"BASE_SHA": "base", "HEAD_SHA": "head"}
                ):
                    ac.main()

                full_workspace.assert_called_once_with(f"global file changed: {path}")

    @patch.object(ac, "emit")
    @patch.object(ac, "lock_changed_packages")
    @patch.object(ac, "workspace_graph")
    @patch.object(ac, "changed_files")
    def test_member_only_lock_change_seeds_those_members(
        self, changed_files, graph, lock_changed_packages, emit
    ):
        changed_files.return_value = ["Cargo.lock"]
        graph.return_value = {
            "qol-alt-tab": {"dir": "plugins/alt-tab", "deps": set(), "doctest": True},
            "unrelated": {"dir": "libs/unrelated", "deps": set(), "doctest": True},
        }
        lock_changed_packages.return_value = {"qol-alt-tab"}
        with patch.dict(os.environ, {"BASE_SHA": "base", "HEAD_SHA": "head"}):
            ac.main()

        self.assertIn("-p qol-alt-tab", emit.call_args.args[0]["ubuntu_build"])
        self.assertNotIn("unrelated", emit.call_args.args[0]["ubuntu_build"])
        self.assertIs(emit.call_args.args[0]["full"], False)

    @patch.object(ac, "full_workspace")
    @patch.object(ac, "lock_changed_packages")
    @patch.object(ac, "workspace_graph")
    @patch.object(ac, "changed_files")
    def test_third_party_lock_change_uses_full_workspace(
        self, changed_files, graph, lock_changed_packages, full_workspace
    ):
        changed_files.return_value = ["Cargo.lock"]
        graph.return_value = {
            "qol-alt-tab": {"dir": "plugins/alt-tab", "deps": set(), "doctest": True},
            "unrelated": {"dir": "libs/unrelated", "deps": set(), "doctest": True},
        }
        lock_changed_packages.return_value = {"serde"}
        with patch.dict(os.environ, {"BASE_SHA": "base", "HEAD_SHA": "head"}):
            ac.main()

        full_workspace.assert_called_once()
        self.assertIn("serde", full_workspace.call_args.args[0])

    @patch.object(ac, "full_workspace")
    @patch.object(ac, "lock_changed_packages")
    @patch.object(ac, "workspace_graph")
    @patch.object(ac, "changed_files")
    def test_unreadable_lock_uses_full_workspace(
        self, changed_files, graph, lock_changed_packages, full_workspace
    ):
        changed_files.return_value = ["Cargo.lock"]
        graph.return_value = {
            "qol-alt-tab": {"dir": "plugins/alt-tab", "deps": set(), "doctest": True},
        }
        lock_changed_packages.return_value = None
        with patch.dict(os.environ, {"BASE_SHA": "base", "HEAD_SHA": "head"}):
            ac.main()

        full_workspace.assert_called_once_with("Cargo.lock diff unreadable")

    @patch.object(ac, "run")
    def test_lock_change_set_is_keyed_by_name_and_version(self, run):
        before = (
            '[[package]]\nname = "qol-alt-tab"\nversion = "0.1.0"\n'
            'dependencies = ["qol-runtime"]\n\n'
            '[[package]]\nname = "serde"\nversion = "1.0.0"\n\n'
            '[[package]]\nname = "unrelated"\nversion = "0.2.0"\n'
        )
        after = (
            '[[package]]\nname = "qol-alt-tab"\nversion = "0.1.0"\n'
            'dependencies = ["qol-runtime", "tracing"]\n\n'
            '[[package]]\nname = "serde"\nversion = "1.0.0"\n\n'
            '[[package]]\nname = "unrelated"\nversion = "0.2.0"\n'
        )
        run.side_effect = [
            subprocess.CompletedProcess([], 0, before, ""),
            subprocess.CompletedProcess([], 0, after, ""),
        ]

        self.assertEqual(ac.lock_changed_packages("base", "head"), {"qol-alt-tab"})

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
            "qol-process": {"dir": "libs/process", "deps": {"foundation"}, "doctest": True},
            "qol-dev-build": {
                "dir": "libs/dev-build",
                "deps": {"qol-process"},
                "doctest": True,
            },
            "qol": {"dir": "tools/cli", "deps": {"qol-dev-build"}, "doctest": False},
            "qol-launcher": {
                "dir": "plugins/launcher",
                "deps": {"qol-process"},
                "doctest": False,
            },
            "unrelated": {"dir": "libs/unrelated", "deps": set(), "doctest": True},
        }
        cases = [
            ("libs/process/src/lib.rs", True, True, "qol-dev-build"),
            ("libs/foundation/src/lib.rs", True, True, "foundation"),
            ("libs/dev-build/src/lib.rs", False, True, "qol-dev-build"),
            ("tools/cli/src/main.rs", False, False, "qol"),
            ("plugins/launcher/src/lib.rs", False, False, "qol-launcher"),
            ("libs/unrelated/src/lib.rs", False, False, "unrelated"),
        ]
        with patch.dict(os.environ, {"BASE_SHA": "base", "HEAD_SHA": "head"}):
            for path, process_expected, dev_build_expected, checked in cases:
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
                    self.assertIn(
                        f"-p {checked}", emit.call_args.args[0]["windows_clippy"]
                    )
                    self.assertIn("--all-targets", emit.call_args.args[0]["windows_clippy"])
                    self.assertIn(f"-p {checked}", emit.call_args.args[0]["windows_test"])
                    self.assertIs(emit.call_args.args[0]["windows_skip"], False)
                    self.assertIs(
                        emit.call_args.args[0]["ubuntu_doctest"],
                        path not in ("tools/cli/src/main.rs", "plugins/launcher/src/lib.rs"),
                    )

    @patch.object(ac, "emit")
    @patch.object(ac, "workspace_graph")
    @patch.object(ac, "changed_files")
    def test_windows_plan_drops_plugins_that_do_not_list_windows(
        self, changed_files, graph, emit
    ):
        graph.return_value = {
            "shared": {"dir": "libs/shared", "deps": set(), "doctest": True},
            "qol-everywhere": {"dir": "plugins/everywhere", "deps": {"shared"}, "doctest": False},
            "qol-unix-only": {"dir": "plugins/unix-only", "deps": {"shared"}, "doctest": False},
        }
        changed_files.return_value = ["libs/shared/src/lib.rs"]
        cases = [
            ("windows_clippy", "-p qol-everywhere", True),
            ("windows_clippy", "-p qol-unix-only", False),
            ("windows_test", "-p shared", True),
            ("windows_test", "-p qol-unix-only", False),
            ("ubuntu_test", "-p qol-unix-only", True),
        ]
        with patch.object(ac, "WINDOWS_EXCLUDE", {"qol-unix-only"}), patch.object(
            ac, "UBUNTU_EXCLUDE", set()
        ), patch.dict(os.environ, {"BASE_SHA": "base", "HEAD_SHA": "head"}):
            ac.main()
        for key, flag, expected in cases:
            with self.subTest(key=key, flag=flag):
                self.assertEqual(flag in emit.call_args.args[0][key], expected)

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
