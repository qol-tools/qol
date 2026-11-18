import itertools
import os
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]

RECORDING_SUDO = """
import os, sys

args = sys.argv[1:]
if args[0] == "tee":
    with open(args[1], "w") as mirrors:
        mirrors.write(sys.stdin.read())
    sys.exit(0)
verb = "download" if "--download-only" in args else next(arg for arg in args if arg in ("update", "install"))
with open(os.environ["APT_MIRRORS"]) as mirrors:
    mirror = mirrors.read().strip()
with open(os.environ["APT_CALLS"]) as calls:
    nth = sum(line.split()[0] == verb for line in calls) + 1
with open(os.environ["APT_CALLS"], "a") as calls:
    calls.write(f"{verb} {args[1]} {mirror}\\n")
sys.exit(124 if f"{verb}:{nth}" in os.environ["APT_FAIL"].split() else 0)
"""


def field(block, name):
    return re.search(rf"^\s*{re.escape(name)}: (.+)$", block, re.M)[1]


def condition(expression, values):
    expression = expression.removeprefix("${{ ").removesuffix(" }}")
    for key, value in values.items():
        expression = expression.replace(key, repr(value))
    return eval(expression.replace("&&", "and").replace("||", "or"), {"__builtins__": {}})


class BuildCacheSetupTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.action = (ROOT / ".github/actions/rust-setup/action.yml").read_text()
        cls.steps = cls.action.split("runs:\n", 1)[1].split("\n    - ")[1:]
        cls.cache = next(step for step in cls.steps if "id: build-cache" in step)
        cls.warm = (ROOT / ".github/workflows/build-cache.yml").read_text()
        cls.warm_steps = cls.warm.split("    steps:\n", 1)[1].split("\n      - ")[1:]

    def test_probing_and_disabling_saves_are_opt_in(self):
        inputs = self.action.split("inputs:\n", 1)[1].split("outputs:\n", 1)[0]
        for name, default in [("lookup-only", '"false"'), ("save-cache", '"true"')]:
            with self.subTest(input=name):
                block = re.split(r"\n  (?=\S)", inputs.split(f"  {name}:\n", 1)[1], 1)[0]
                self.assertEqual(field(block, "default"), default)
        self.assertEqual(field(self.cache, "lookup-only"), "${{ inputs.lookup-only }}")

    def test_only_main_builds_with_save_permission_can_write(self):
        expression = field(self.cache, "save-if")
        for ref, lookup, save in itertools.product(
            ["refs/heads/main", "refs/pull/1/merge", "refs/heads/gh-readonly-queue/main/pr-1"],
            ["true", "false"],
            ["true", "false"],
        ):
            with self.subTest(ref=ref, lookup=lookup, save=save):
                actual = condition(expression, {
                    "github.ref": ref,
                    "inputs.lookup-only": lookup,
                    "inputs.save-cache": save,
                })
                self.assertEqual(actual, ref == "refs/heads/main" and lookup == "false" and save == "true")

    def test_probes_never_clean_disk_or_install_system_dependencies(self):
        room = next(step for step in self.steps if step.startswith("name: Make room"))
        dependencies = next(step for step in self.steps if step.startswith("name: Install Linux"))
        for runner, lookup, install in itertools.product(
            ["Linux", "macOS", "Windows"], ["true", "false"], ["true", "false"]
        ):
            values = {
                "runner.os": runner,
                "inputs.lookup-only": lookup,
                "inputs.system-dependencies": install,
            }
            with self.subTest(**values):
                self.assertEqual(condition(field(room, "if"), values), runner == "Linux" and lookup == "false")
                self.assertEqual(condition(field(dependencies, "if"), values), runner == "Linux" and lookup == "false" and install == "true")

    def test_probe_and_restore_share_the_exact_setup_and_namespace(self):
        setup = [step for step in self.warm_steps if "uses: ./.github/actions/rust-setup\n" in step]
        self.assertEqual(len(setup), 2)
        probe, restore = setup
        self.assertEqual(field(probe, "id"), "rust_setup")
        self.assertEqual(field(probe, "lookup-only"), '"true"')
        self.assertNotIn("lookup-only:", restore)
        for name in ["cache-key", "components"]:
            self.assertEqual(field(probe, name), field(restore, name))
        self.assertEqual(field(probe, "cache-key"), "ci-${{ matrix.os }}")

    def test_python_setup_leaves_the_compile_environment_alone(self):
        python = next(step for step in self.warm_steps if "actions/setup-python@" in step)
        self.assertEqual(field(python, "update-environment"), "false")
        self.assertEqual(field(python, "id"), "python")
        workspace = next(step for step in self.warm_steps if step.startswith("name: Workspace arguments"))
        self.assertIn("${{ steps.python.outputs.python-path }}", field(workspace, "run"))

    def test_only_exact_hits_skip_restore_and_every_warming_step(self):
        probe_index = next(i for i, step in enumerate(self.warm_steps) if "id: rust_setup\n" in step)
        following = self.warm_steps[probe_index + 1:]
        self.assertGreaterEqual(len(following), 5)
        for step, hit in itertools.product(following, ["true", "false", ""]):
            with self.subTest(step=step.splitlines()[0], hit=hit):
                self.assertEqual(
                    condition(field(step, "if"), {"steps.rust_setup.outputs.build-cache-hit": hit}),
                    hit != "true",
                )

    def test_a_failed_package_update_moves_to_a_mirror_outside_ubuntu_com(self):
        install = next(step for step in self.steps if step.startswith("name: Install Linux system dependencies"))
        update = install.split("for attempt in 1 2 3; do\n", 2)[1]
        lines = [line.strip() for line in update.split("done", 1)[0].splitlines() if line.strip()]
        self.assertEqual(
            lines,
            ["apt_get 120 update && break", 'if [ "$attempt" = 3 ]; then exit 1; fi', "use_outside_mirror"],
        )
        switch = install.split("use_outside_mirror() {\n", 1)[1].split("\n        }\n", 1)[0]
        self.assertIn("local list=/etc/apt/apt-mirrors.txt", switch)
        self.assertIn('if [ -f "$list" ]; then', switch)
        mirror = re.search(r'echo "(https://[^"]+)" \| sudo tee "\$list"', switch)[1]
        self.assertEqual(mirror, "https://mirrors.edge.kernel.org/ubuntu/")
        self.assertNotIn("ubuntu.com", mirror)
        self.assertNotIn("trusted=yes", install)
        self.assertNotIn("allow-unauthenticated", install)
        self.assertNotIn("AllowUnauthenticated", install)

    def test_a_slow_package_download_moves_to_the_outside_mirror(self):
        install = next(step for step in self.steps if step.startswith("name: Install Linux system dependencies"))
        script = textwrap.dedent(install.split("      run: |\n", 1)[1])
        self.assertIn("local list=/etc/apt/apt-mirrors.txt", script)
        runner, outside = "http://runner.invalid/ubuntu/", "https://mirrors.edge.kernel.org/ubuntu/"
        healthy = [("update", "120", runner), ("download", "60", runner), ("install", "300", runner)]
        moved = [("update", "120", outside), ("download", "300", outside)]
        for failing, code, calls in [
            ([], 0, healthy),
            (["download:1"], 0, healthy[:2] + moved + [("install", "300", outside)]),
            (["download:1", "update:2"], 0, healthy[:2] + moved[:1] + moved + [("install", "300", outside)]),
            (["download:1", "download:2"], 0, healthy[:2] + moved + moved + [("install", "300", outside)]),
            (["download:1", "download:2", "download:3", "download:4"], 1, healthy[:2] + moved * 3),
            (["update:1"], 0, [healthy[0]] + [(verb, bound, outside) for verb, bound, _ in healthy]),
            (["install:1"], 0, healthy + healthy[2:]),
        ]:
            with self.subTest(failing=failing), tempfile.TemporaryDirectory() as folder:
                mirrors, log = Path(folder, "apt-mirrors.txt"), Path(folder, "calls")
                mirrors.write_text(f"{runner}\n")
                log.write_text("")
                sudo = Path(folder, "sudo")
                sudo.write_text(f"#!{sys.executable}\n{RECORDING_SUDO}")
                sudo.chmod(0o755)
                result = subprocess.run(
                    ["bash", "-e", "-c", script.replace("/etc/apt/apt-mirrors.txt", str(mirrors))],
                    capture_output=True,
                    text=True,
                    env={
                        **os.environ,
                        "PATH": f"{folder}{os.pathsep}{os.environ['PATH']}",
                        "APT_MIRRORS": str(mirrors),
                        "APT_CALLS": str(log),
                        "APT_FAIL": " ".join(failing),
                    },
                )
                self.assertEqual(result.returncode, code, result.stdout + result.stderr)
                self.assertEqual([tuple(line.split()) for line in log.read_text().splitlines()], calls)


if __name__ == "__main__":
    unittest.main()
