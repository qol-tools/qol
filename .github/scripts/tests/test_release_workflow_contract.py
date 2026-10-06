import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
CARGO_GATE = re.compile(r"\bcargo (?:build|test|clippy|check|nextest run)\b")


def named_step(document, name, indent):
    start = document.index(f"{indent}- name: {name}\n")
    end = document.find(f"\n{indent}- ", start + 1)
    return document[start:end] if end != -1 else document[start:]


class ReleaseWorkflowContractTests(unittest.TestCase):
    def test_setup_reports_whether_the_build_cache_will_be_saved(self):
        action = (ROOT / ".github/actions/rust-setup/action.yml").read_text()
        cache = named_step(action, "Restore or save build cache", "    ")
        self.assertIn("id: build-cache", cache)
        self.assertIn("uses: Swatinem/rust-cache@", cache)
        self.assertIn("save-if: ${{ github.ref == 'refs/heads/main' }}", cache)
        outputs = action.split("outputs:\n", 1)[1].split("runs:\n", 1)[0]
        self.assertIn("build-cache-hit:", outputs)
        self.assertIn("value: ${{ steps.build-cache.outputs.cache-hit }}", outputs)

    def test_main_run_that_saves_the_build_cache_also_checks_the_release_profile(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        check = workflow.split("  check:\n", 1)[1].split("  process-windows:\n", 1)[0]
        self.assertIn("id: rust_setup", check)
        step = named_step(check, "Release profile check", "      ")
        for contract in [
            "matrix.skip != 'true'",
            "github.event_name == 'pull_request' ||",
            "github.ref == 'refs/heads/main'",
            "steps.rust_setup.outputs.build-cache-hit != 'true'",
            "cargo check --release --locked $BUILD_ARGS",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, step)
        build = named_step(check, "Release profile build", "      ")
        self.assertIn("github.event_name != 'pull_request'", build)
        self.assertIn("cargo build --release --locked $BUILD_ARGS", build)

    def test_build_jobs_never_hold_a_cache_deletion_token(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        check = workflow.split("  check:\n", 1)[1].split("  process-windows:\n", 1)[0]
        self.assertNotIn("actions: write", check)
        self.assertNotIn("github.token", check)
        self.assertNotIn("cache_prune.py", check)

    def test_queue_entry_that_reuses_a_verdict_builds_under_its_own_name(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        check = workflow.split("  check:\n", 1)[1].split("  process-windows:\n", 1)[0]
        self.assertIn("if: ${{ needs.plan.outputs.reused != 'true' }}", check)
        build = workflow.split("  release-build:\n", 1)[1].split("  gate:\n", 1)[0]
        for contract in [
            "name: release build (${{ matrix.os }})",
            "github.event_name == 'merge_group' && needs.plan.outputs.reused == 'true'",
            "RUSTFLAGS: -D warnings",
            "cache-key: ci-${{ matrix.os }}",
            "cargo build --release --locked $BUILD_ARGS",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, build)
        self.assertNotIn("actions: write", build)
        self.assertNotIn("github.token", build)

    def test_one_gate_job_carries_the_merge_verdict(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        gate = workflow.split("  gate:\n", 1)[1].split("  queue-report:\n", 1)[0]
        for contract in [
            "name: merge gate",
            "needs: [plan, check, release-build, process-windows]",
            "if: ${{ always() && github.event_name != 'push' }}",
            '[ "$PLAN" = success ] || exit 1',
            '*" failure "*|*" cancelled "*) exit 1 ;;',
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, gate)
        report = workflow.split("  queue-report:\n", 1)[1]
        self.assertIn("needs: [plan, check, release-build, process-windows]", report)

    def test_no_workflow_wraps_the_compiler(self):
        sources = [ROOT / ".github/actions/rust-setup/action.yml"]
        sources.extend((ROOT / ".github/workflows").glob("*.yml"))
        for path in sources:
            with self.subTest(path=path.name):
                self.assertNotIn("RUSTC_WRAPPER", path.read_text())

    def test_versioning_waits_for_main_ci(self):
        workflow = (ROOT / ".github/workflows/plugin-version.yml").read_text()

        self.assertIn("workflow_run:", workflow)
        self.assertIn("conclusion == 'success'", workflow)
        self.assertIn(
            "SOURCE_CI_RUN_ID: ${{ github.event.workflow_run.id }}", workflow
        )
        self.assertIn('args+=(--run-id "${SOURCE_CI_RUN_ID}")', workflow)
        self.assertNotIn("push:\n    branches: [main]", workflow)

    def test_candidates_are_attested_before_tags_are_created(self):
        workflow = (ROOT / ".github/workflows/plugin-version.yml").read_text()

        attest = workflow.index("release_candidate.py attest")
        tag = workflow.index('git tag "${tag}"')
        self.assertLess(attest, tag)

    def test_candidate_release_set_remains_atomic(self):
        workflow = (ROOT / ".github/workflows/plugin-version.yml").read_text()

        required = [
            "needs.plugin_candidate.result == 'success'",
            "needs.qol_tray_linux_candidate.result == 'success'",
            "needs.qol_tray_macos_candidate.result == 'success'",
            'git push --atomic origin "${new_tags[@]}"',
        ]
        for contract in required:
            self.assertIn(contract, workflow)

    def test_shared_crate_releases_wait_for_binary_probes(self):
        workflow = (ROOT / ".github/workflows/plugin-version.yml").read_text()
        release = (ROOT / ".github/workflows/release.yml").read_text()

        self.assertIn("needs: [probe_plan, probe]", workflow)
        self.assertIn("needs.probe.result == 'skipped'", workflow)
        self.assertLess(workflow.index("release_probe.py"), workflow.index("--probe-reports"))
        for contract in [
            "cache-key: plugin-release-${{ matrix.target }}",
            "RUSTFLAGS: -D warnings ${{ matrix.os == 'ubuntu-latest' && '-C link-arg=-fuse-ld=lld' || '' }}",
            "--kind plugin",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, release)
                self.assertIn(contract, workflow)

    def test_release_workflows_verify_exact_candidate(self):
        for name in ["release.yml", "qol-tray-release.yml"]:
            with self.subTest(workflow=name):
                workflow = (ROOT / ".github/workflows" / name).read_text()
                verify = workflow.index("release_candidate.py verify")
                build = workflow.index("release_candidate.py build")
                self.assertLess(verify, build)

    def test_ci_runs_release_profile_builds(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()

        self.assertIn("merge_group:", workflow)
        self.assertIn("cargo check --release --locked $BUILD_ARGS", workflow)
        self.assertIn("cargo build --release --locked $BUILD_ARGS", workflow)
        self.assertIn("github.event_name != 'pull_request'", workflow)
        self.assertIn("RUSTFLAGS: -D warnings", workflow)
        self.assertNotIn("debug-assertions", workflow)
        self.assertIn("timeout-minutes:", workflow)

    def test_ci_matches_the_release_pipeline_lockfile_strictness(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()

        lenient = [
            line.strip()
            for line in workflow.splitlines()
            if CARGO_GATE.search(line)
            and not line.strip().startswith("#")
            and "--locked" not in line
        ]
        self.assertEqual(
            lenient,
            [],
            "every CI cargo gate must pass --locked so a stale Cargo.lock cannot "
            "reach the --locked release pipeline",
        )

    def test_ci_asserts_lockfile_freshness_with_a_full_resolve(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()

        self.assertIn("cargo metadata --locked --format-version 1", workflow)
        self.assertNotIn(
            "cargo metadata --locked --no-deps",
            workflow,
            "--no-deps skips resolution and exits 0 against a stale lockfile",
        )

    def test_nextest_preserves_affected_scope_and_documentation_on_each_platform(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        check_job = workflow.split("  check:\n", 1)[1].split("  process-windows:\n", 1)[0]
        test_steps = re.findall(r"      - name: Test\n(.*?)(?=\n      - name:|\Z)", check_job, re.S)
        self.assertEqual(len(test_steps), 2)
        self.assertRegex(check_job, r"tool: cargo-nextest@\d+\.\d+\.\d+")
        for step in test_steps:
            with self.subTest(step=step.splitlines()[0]):
                self.assertIn("TEST_ARGS: ${{ matrix.test_args }}", step)
                self.assertIn("DOCTEST: ${{ matrix.doctest }}", step)
                self.assertIn('if [ "$DOCTEST" = "true" ]; then', step)
                self.assertIn(
                    "cargo nextest run --locked $TEST_ARGS --no-fail-fast --retries 0 --no-tests=pass --ignore-default-filter",
                    step,
                )
                self.assertIn("cargo test --locked $TEST_ARGS --doc", step)
                self.assertLess(step.index("cargo nextest run"), step.index("cargo test"))

    def test_windows_dev_build_tests_follow_the_affected_plan(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()

        for contract in [
            "windows_dev_build: ${{ steps.affected.outputs.windows_dev_build }}",
            "fromJSON(needs.plan.outputs.windows_dev_build || 'false')",
            "if: ${{ fromJSON(needs.plan.outputs.windows_dev_build) }}",
            "run: cargo test --locked -p qol-dev-build --all-targets",
        ]:
            self.assertIn(contract, workflow)

    def test_plugins_publish_no_github_release(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()

        self.assertNotIn("gh release", workflow)
        self.assertNotIn("contents: write", workflow)

    def test_plugin_registry_push_runs_after_the_build(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        registry = workflow.split("  registry:\n", 1)[1].split("  index:\n", 1)[0]
        self.assertIn("needs: [setup, build]", registry)
        self.assertIn("packages: write", registry)
        self.assertIn("ref: ${{ env.RELEASE_REF }}", registry)
        push = named_step(registry, "Push registry artifact", "      ")
        for contract in [
            "set -euo pipefail",
            '--registry "ghcr.io/${GITHUB_REPOSITORY_OWNER}/plugins"',
            '--tag "${RELEASE_TAG}" --files release_files',
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, push)

    def test_registry_tool_compiles_in_a_step_that_holds_no_token(self):
        jobs = {
            "release.yml": ("  registry:\n", "  index:\n"),
            "plugin-index.yml": ("  build:\n", "  sign:\n"),
            "plugin-registry-backfill.yml": ("  backfill:\n", "  index:\n"),
        }
        for name, (begin, finish) in jobs.items():
            with self.subTest(workflow=name):
                workflow = (ROOT / ".github/workflows" / name).read_text()
                job = workflow.split(begin, 1)[1].split(finish, 1)[0]
                self.assertIn("persist-credentials: false", job)
                self.assertIn("cache-key: plugin-registry", job)
                self.assertIn('system-dependencies: "false"', job)
                self.assertNotIn("actions: write", job)
                build = named_step(job, "Build registry tool", "      ")
                self.assertIn("cargo build --locked -p qol-plugin-registry", build)
                self.assertNotIn("github.token", build)
                self.assertNotIn("secrets.", build)
                self.assertEqual(job.count("cargo "), 1)

    def test_plugin_release_refreshes_the_signed_index(self):
        release = (ROOT / ".github/workflows/release.yml").read_text()
        dispatch = release.split("  index:\n", 1)[1]
        self.assertIn("needs: registry", dispatch)
        self.assertIn("actions: write", dispatch)
        self.assertIn("gh workflow run plugin-index.yml --ref main", dispatch)

        workflow = (ROOT / ".github/workflows/plugin-index.yml").read_text()
        build = workflow.split("  build:\n", 1)[1].split("  sign:\n", 1)[0]
        sign = workflow.split("  sign:\n", 1)[1].split("  deploy:\n", 1)[0]
        deploy = workflow.split("  deploy:\n", 1)[1]
        self.assertIn("cancel-in-progress: false", workflow)
        self.assertIn("if: github.ref == 'refs/heads/main'", build)
        self.assertNotIn("environment:", build)
        self.assertNotIn("secrets.", build)
        self.assertIn("needs: build", sign)
        self.assertIn("environment: plugin-index", sign)
        self.assertIn("secrets.PLUGIN_INDEX_MINISIGN_KEY", sign)
        self.assertIn("-x site/plugins/index.json.minisig", sign)
        for job in (build, sign):
            self.assertNotIn("pages: write", job)
        self.assertIn("needs: sign", deploy)
        self.assertIn("pages: write", deploy)
        self.assertNotIn("secrets.", deploy)

    def test_plugin_index_signs_in_a_job_that_compiles_nothing(self):
        workflow = (ROOT / ".github/workflows/plugin-index.yml").read_text()
        build = workflow.split("  build:\n", 1)[1].split("  sign:\n", 1)[0]
        sign = workflow.split("  sign:\n", 1)[1].split("  deploy:\n", 1)[0]
        public_key = re.search(r"^  PUBLIC_KEY: (\S+)$", workflow, re.MULTILINE).group(1)
        self.assertTrue((ROOT / public_key).is_file(), public_key)
        self.assertNotIn("apt-get", workflow)

        index = named_step(build, "Build index", "      ")
        for contract in [
            "set -euo pipefail",
            '--public-key "${PUBLIC_KEY}"',
            '--previous-url "${INDEX_URL}"',
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, index)

        for compiles in ["cargo", "rust-setup", "github.token"]:
            with self.subTest(compiles=compiles):
                self.assertNotIn(compiles, sign)
        self.assertIn(str(Path(public_key).parent), sign)
        install = named_step(sign, "Install minisign", "      ")
        self.assertIn("sha256sum --check --strict", install)
        signing = named_step(sign, "Sign index", "      ")
        self.assertIn("trap 'rm -f \"${key}\"' EXIT", signing)
        self.assertIn("< /dev/null", signing)
        verify = named_step(sign, "Verify with the key the tray ships", "      ")
        self.assertIn('minisign -V -p "${PUBLIC_KEY}" -m site/plugins/index.json', verify)

        secret_steps = [step for step in workflow.split("\n      - ") if "secrets." in step]
        self.assertEqual(len(secret_steps), 1)
        self.assertIn("name: Sign index", secret_steps[0])
        order = [
            "- name: Install minisign",
            "actions/download-artifact",
            "- name: Sign index",
            "- name: Verify with the key the tray ships",
            "actions/upload-pages-artifact",
        ]
        positions = [sign.index(marker) for marker in order]
        self.assertEqual(positions, sorted(positions))

    def test_registry_backfill_pushes_only_from_main(self):
        workflow = (ROOT / ".github/workflows/plugin-registry-backfill.yml").read_text()
        backfill = workflow.split("  backfill:\n", 1)[1].split("  index:\n", 1)[0]
        dispatch = workflow.split("  index:\n", 1)[1]
        self.assertIn("if: github.ref == 'refs/heads/main'", backfill)
        self.assertIn("packages: write", backfill)
        self.assertIn("fetch-depth: 0", backfill)
        self.assertIn("needs: backfill", dispatch)
        self.assertIn("actions: write", dispatch)
        self.assertNotIn("cargo", dispatch)

    def test_tray_publish_claims_latest(self):
        workflow = (ROOT / ".github/workflows/qol-tray-release.yml").read_text()

        self.assertIn(
            'gh release edit "${RELEASE_TAG}" --draft=false --latest=true',
            workflow,
        )
        self.assertNotIn("--latest=false", workflow)

    def test_release_creation_steps_carry_an_explicit_latest_policy(self):
        for workflow_path in sorted((ROOT / ".github/workflows").glob("*.yml")):
            with self.subTest(workflow=workflow_path.name):
                workflow = workflow_path.read_text()
                lines = workflow.splitlines()
                for index, line in enumerate(lines):
                    if "gh release create" in line:
                        self.assertIn(
                            "--latest",
                            line,
                            f"release creation must carry an explicit latest flag: "
                            f"{workflow_path.name}:{index + 1}",
                        )
                    if "action-gh-release" in line:
                        block = lines[index + 1 : index + 12]
                        self.assertTrue(
                            any("make_latest:" in b for b in block),
                            f"action-gh-release step must declare make_latest: "
                            f"{workflow_path.name}:{index + 1}",
                        )


if __name__ == "__main__":
    unittest.main()
