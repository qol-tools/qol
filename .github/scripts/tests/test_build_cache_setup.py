import hashlib
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
import os, subprocess, sys
from pathlib import Path

args = sys.argv[1:]
if args[0] == "tee":
    Path(args[1]).write_text(sys.stdin.read())
    sys.exit(0)
if args[0] == "cp":
    sys.exit(subprocess.run(args).returncode)
archives = Path(os.environ["APT_ARCHIVES"])
current = os.environ["APT_CURRENT"].split()
verb = "download" if "--download-only" in args else next(
    arg for arg in args if arg in ("update", "install", "autoclean")
)
mirror = Path(os.environ["APT_MIRRORS"]).read_text().strip()
present = ",".join(sorted(deb.name for deb in archives.glob("*.deb"))) or "-"
calls = Path(os.environ["APT_CALLS"])
nth = sum(line.split()[0] == verb for line in calls.read_text().splitlines()) + 1
with calls.open("a") as log:
    log.write(f"{verb} {args[1]} {mirror} {present}\\n")
if f"{verb}:{nth}" in os.environ["APT_FAIL"].split():
    sys.exit(124)
if verb == "download":
    for name in current:
        (archives / name).touch()
if verb == "autoclean":
    for deb in archives.glob("*.deb"):
        if deb.name not in current:
            deb.unlink()
"""

RUNNER_MIRROR = "http://runner.invalid/ubuntu/"
OUTSIDE_MIRROR = "https://mirrors.edge.kernel.org/ubuntu/"
CURRENT_PACKAGES = ["libgtk-3-dev_3.24.41-4ubuntu1.3_amd64.deb", "pkg-config_1.8.1-2build1_amd64.deb"]
REPLACED_PACKAGES = ["libgtk-3-dev_3.24.41-4ubuntu1.2_amd64.deb", "pkg-config_1.8.1-2build1_amd64.deb"]


def saved_key(names):
    listing = "".join(f"{name}\n" for name in sorted(names))
    return f"apt-archives-ubuntu24-{hashlib.sha256(listing.encode()).hexdigest()[:8]}"


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

    def test_probes_never_clean_disk_and_only_a_main_probe_reaches_the_package_step(self):
        room = next(step for step in self.steps if step.startswith("name: Make room"))
        dependencies = next(step for step in self.steps if step.startswith("name: Install Linux"))
        main = "refs/heads/main"
        for runner, lookup, install, ref in itertools.product(
            ["Linux", "macOS", "Windows"],
            ["true", "false"],
            ["true", "false"],
            [main, "refs/pull/1/merge", "refs/heads/gh-readonly-queue/main/pr-1"],
        ):
            values = {
                "runner.os": runner,
                "inputs.lookup-only": lookup,
                "inputs.system-dependencies": install,
                "github.ref": ref,
            }
            with self.subTest(**values):
                self.assertEqual(condition(field(room, "if"), values), runner == "Linux" and lookup == "false")
                self.assertEqual(
                    condition(field(dependencies, "if"), values),
                    runner == "Linux" and install == "true" and (lookup == "false" or ref == main),
                )

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
            ["apt_get 120 update && break", 'if [ "$attempt" = 3 ]; then give_up; fi', "use_outside_mirror"],
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

    def install_packages(self, failing=(), restored=(), restored_key="", ref="refs/pull/1/merge", lookup_only="false"):
        install = next(step for step in self.steps if step.startswith("name: Install Linux system dependencies"))
        script = textwrap.dedent(install.split("      run: |\n", 1)[1])
        with tempfile.TemporaryDirectory() as folder:
            mirrors, archives, log = Path(folder, "apt-mirrors.txt"), Path(folder, "archives"), Path(folder, "calls")
            saved, output = Path(folder, "temp", "apt-archives"), Path(folder, "output")
            for path, stand_in in [("/etc/apt/apt-mirrors.txt", mirrors), ("/var/cache/apt/archives", archives)]:
                self.assertIn(path, script)
                script = script.replace(path, str(stand_in))
            archives.mkdir()
            saved.mkdir(parents=True)
            for name in restored:
                saved.joinpath(name).touch()
            mirrors.write_text(f"{RUNNER_MIRROR}\n")
            log.write_text("")
            output.write_text("")
            sudo = Path(folder, "sudo")
            sudo.write_text(f"#!{sys.executable}\n{RECORDING_SUDO}")
            sudo.chmod(0o755)
            result = subprocess.run(
                ["bash", "-e", "-c", script],
                capture_output=True,
                text=True,
                env={
                    **os.environ,
                    "PATH": f"{folder}{os.pathsep}{os.environ['PATH']}",
                    "RUNNER_TEMP": str(saved.parent),
                    "GITHUB_OUTPUT": str(output),
                    "GITHUB_REF": ref,
                    "KEY_PREFIX": "apt-archives-ubuntu24-",
                    "RESTORED_KEY": restored_key,
                    "LOOKUP_ONLY": lookup_only,
                    "APT_MIRRORS": str(mirrors),
                    "APT_ARCHIVES": str(archives),
                    "APT_CALLS": str(log),
                    "APT_CURRENT": " ".join(CURRENT_PACKAGES),
                    "APT_FAIL": " ".join(failing),
                },
            )
            return {
                "code": result.returncode,
                "calls": [tuple(line.split()) for line in log.read_text().splitlines()],
                "output": output.read_text(),
                "saved": sorted(deb.name for deb in saved.glob("*.deb")),
                "log": result.stdout + result.stderr,
            }

    def test_a_slow_package_download_moves_to_the_outside_mirror(self):
        runner, outside = RUNNER_MIRROR, OUTSIDE_MIRROR
        start = [("update", "120", runner), ("download", "60", runner)]
        healthy = start + [("autoclean", "60", runner), ("install", "300", runner)]
        finish = [("autoclean", "60", outside), ("install", "300", outside)]
        retry = [("update", "120", outside), ("download", "300", outside)]
        for failing, code, calls in [
            ([], 0, healthy),
            (["download:1"], 0, start + retry[1:] + finish),
            (["download:1", "download:2"], 0, start + retry[1:] + retry + finish),
            (["download:1", "download:2", "update:2"], 0, start + retry[1:] + retry + finish),
            (["download:1", "download:2", "download:3", "download:4"], 1, start + retry[1:] + retry * 2),
            (["update:1"], 0, start[:1] + [(verb, bound, outside) for verb, bound, _ in healthy]),
            (["install:1"], 0, healthy + healthy[3:]),
            (["autoclean:1"], 0, healthy),
        ]:
            with self.subTest(failing=failing):
                result = self.install_packages(failing)
                self.assertEqual(result["code"], code, result["log"])
                self.assertEqual([call[:3] for call in result["calls"]], calls)

    def test_package_files_come_from_the_saved_set_and_only_a_changed_set_is_saved(self):
        current, replaced = saved_key(CURRENT_PACKAGES), saved_key(REPLACED_PACKAGES)
        self.assertRegex(current, r"-[0-9a-f]{8}$")
        main, pull_request, queue = "refs/heads/main", "refs/pull/1/merge", "refs/heads/gh-readonly-queue/main/pr-1"
        for restored, restored_key, ref, saves in [
            ([], "", pull_request, True),
            ([], "", queue, True),
            ([], "", main, True),
            (CURRENT_PACKAGES, current, main, False),
            (CURRENT_PACKAGES, current, pull_request, False),
            (REPLACED_PACKAGES, replaced, main, True),
            (REPLACED_PACKAGES, replaced, pull_request, False),
            (REPLACED_PACKAGES, replaced, queue, False),
        ]:
            with self.subTest(restored_key=restored_key, ref=ref):
                result = self.install_packages(restored=restored, restored_key=restored_key, ref=ref)
                self.assertEqual(result["code"], 0, result["log"])
                download = next(call for call in result["calls"] if call[0] == "download")
                self.assertEqual(download[3], ",".join(sorted(restored)) or "-")
                self.assertEqual(result["saved"], sorted(CURRENT_PACKAGES))
                self.assertEqual(result["output"], f"save-key={current}\n" if saves else "")

    def test_a_lookup_on_main_saves_a_changed_set_and_installs_nothing(self):
        current, replaced = saved_key(CURRENT_PACKAGES), saved_key(REPLACED_PACKAGES)
        runner, outside = RUNNER_MIRROR, OUTSIDE_MIRROR
        fetched = [("update", "120", runner), ("download", "60", runner), ("autoclean", "60", runner)]
        no_update = [("update", "120", runner), ("update", "120", outside), ("update", "120", outside)]
        no_download = (
            fetched[:2]
            + [("download", "300", outside)]
            + [("update", "120", outside), ("download", "300", outside)] * 2
        )
        slow = ["download:1", "download:2", "download:3", "download:4"]
        for restored, restored_key, failing, calls, saves in [
            ([], "", [], fetched, True),
            (CURRENT_PACKAGES, current, [], fetched, False),
            (REPLACED_PACKAGES, replaced, [], fetched, True),
            (REPLACED_PACKAGES, replaced, ["update:1", "update:2", "update:3"], no_update, False),
            (REPLACED_PACKAGES, replaced, slow, no_download, False),
            ([], "", slow, no_download, False),
        ]:
            with self.subTest(restored_key=restored_key, failing=failing):
                result = self.install_packages(
                    failing, restored, restored_key, ref="refs/heads/main", lookup_only="true"
                )
                self.assertEqual(result["code"], 0, result["log"])
                self.assertEqual([call[:3] for call in result["calls"]], calls)
                self.assertEqual(result["output"], f"save-key={current}\n" if saves else "")
        install = next(step for step in self.steps if step.startswith("name: Install Linux system dependencies"))
        self.assertEqual(field(install, "LOOKUP_ONLY"), "${{ inputs.lookup-only }}")

    def test_the_saved_package_set_is_restored_newest_first_and_saved_under_the_key_of_its_files(self):
        restore = next(step for step in self.steps if "id: saved-packages" in step)
        install = next(step for step in self.steps if step.startswith("name: Install Linux system dependencies"))
        save = next(step for step in self.steps if step.startswith("name: Save downloaded Linux system packages"))
        self.assertEqual(field(restore, "if"), field(install, "if"))
        prefix = "${{ steps.package-sets.outputs.prefix }}"
        self.assertEqual(field(restore, "key"), f"{prefix}newest")
        self.assertEqual(field(restore, "restore-keys"), prefix)
        self.assertEqual(field(install, "KEY_PREFIX"), prefix)
        self.assertEqual(field(install, "RESTORED_KEY"), "${{ steps.saved-packages.outputs.cache-matched-key }}")
        naming = next(step for step in self.steps if "id: package-sets" in step)
        self.assertEqual(field(naming, "if"), field(install, "if"))
        self.assertLess(self.steps.index(naming), self.steps.index(restore))
        for step in [naming, restore, install, save]:
            self.assertNotIn("env.ImageOS", step)
        with tempfile.TemporaryDirectory() as folder:
            output = Path(folder, "output")
            subprocess.run(
                ["bash", "-e", "-c", field(naming, "run")],
                check=True,
                env={**os.environ, "ImageOS": "ubuntu24", "GITHUB_OUTPUT": str(output)},
            )
            self.assertEqual(output.read_text(), "prefix=apt-archives-ubuntu24-\n")
        self.assertEqual(field(save, "if"), "steps.packages.outputs.save-key != ''")
        self.assertEqual(field(save, "key"), "${{ steps.packages.outputs.save-key }}")
        self.assertEqual(field(install, "id"), "packages")
        self.assertEqual(field(restore, "path"), "${{ runner.temp }}/apt-archives")
        self.assertEqual(field(save, "path"), field(restore, "path"))
        self.assertIn('saved="$RUNNER_TEMP/apt-archives"', install)
        self.assertLess(self.steps.index(restore), self.steps.index(install))
        self.assertLess(self.steps.index(install), self.steps.index(save))
        for step, action in [(restore, "restore"), (save, "save")]:
            self.assertRegex(field(step, "uses"), rf"^actions/cache/{action}@[0-9a-f]{{40}} # v\d+\.\d+\.\d+$")


if __name__ == "__main__":
    unittest.main()
