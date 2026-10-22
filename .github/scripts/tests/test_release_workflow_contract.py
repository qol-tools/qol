import os
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / ".github/scripts"))

import release_candidate as rc
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
        self.assertIn("save-if: ${{ github.ref == 'refs/heads/main' && inputs.save-cache == 'true' && inputs.lookup-only != 'true' }}", cache)
        outputs = action.split("outputs:\n", 1)[1].split("runs:\n", 1)[0]
        self.assertIn("build-cache-hit:", outputs)
        self.assertIn("value: ${{ steps.build-cache.outputs.cache-hit }}", outputs)

    def test_only_pull_requests_check_the_release_profile(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        check = workflow.split("  check:\n", 1)[1].split("  process-windows:\n", 1)[0]
        step = named_step(check, "Release profile check", "      ")
        for contract in [
            "if: matrix.skip != 'true' && github.event_name == 'pull_request'",
            "cargo check --release --locked $BUILD_ARGS",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, step)
        self.assertNotIn("cargo build --release", check)

    def test_checks_restore_the_cache_without_competing_with_the_warmer(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        check = workflow.split("  check:\n", 1)[1].split("  process-windows:\n", 1)[0]
        setup = check.split("      - uses: ./.github/actions/rust-setup\n", 1)[1].split("\n      - ", 1)[0]
        self.assertIn("cache-key: ci-${{ matrix.os }}", setup)
        self.assertIn('save-cache: "false"', setup)

    def test_build_jobs_never_hold_a_cache_deletion_token(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        check = workflow.split("  check:\n", 1)[1].split("  process-windows:\n", 1)[0]
        self.assertNotIn("actions: write", check)
        self.assertNotIn("github.token", check)
        self.assertNotIn("cache_prune.py", check)

    def test_release_build_runs_beside_lint_and_test(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        check = workflow.split("  check:\n", 1)[1].split("  process-windows:\n", 1)[0]
        self.assertIn(
            "if: ${{ !cancelled() && needs.plan.result == 'success' && needs.plan.outputs.reused != 'true' && needs.plan.outputs.trusted != 'true' }}",
            check,
        )
        build = workflow.split("  release-build:\n", 1)[1].split("  registry-tool:\n", 1)[0]
        for contract in [
            "name: release build (${{ matrix.os }})",
            "needs: [plan, batch]",
            "${{ !cancelled() && needs.plan.result == 'success' && needs.batch.result == 'success' && needs.batch.outputs.role == 'tail' &&\n",
            "!(needs.plan.outputs.trusted == 'true' && needs.batch.outputs.ubuntu_skip == 'true' &&\n",
            "needs.batch.outputs.macos_skip == 'true' && needs.batch.outputs.windows_skip == 'true') }}",
            "RUSTFLAGS: -D warnings",
            "cache-key: ci-${{ matrix.os }}",
            "cargo build --release --locked $BUILD_ARGS",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, build)
        self.assertNotIn("actions: write", build)
        self.assertNotIn("github.token", build)

    def test_main_warms_the_build_cache_outside_the_tests_workflow(self):
        warm = (ROOT / ".github/workflows/build-cache.yml").read_text()
        tests = (ROOT / ".github/workflows/ci.yml").read_text()
        rustflags = (
            "RUSTFLAGS: -D warnings ${{ matrix.os == 'ubuntu-latest' && "
            "'-C link-arg=-fuse-ld=lld' || matrix.os == 'windows-latest' && "
            "'-C linker=rust-lld -C target-feature=+crt-static' || '' }}"
        )
        # The check job, its release check step and the release build share
        # the cache key the warm job saves, so each restates its RUSTFLAGS.
        self.assertEqual(tests.count(rustflags), 3)
        for contract in [
            "branches: [main]",
            "cancel-in-progress: false",
            "persist-credentials: false",
            "cache-key: ci-${{ matrix.os }}",
            rustflags,
            "steps.rust_setup.outputs.build-cache-hit != 'true'",
            "cargo clippy --locked --keep-going $CLIPPY_ARGS -- -D warnings",
            "cargo check --release --locked $BUILD_ARGS",
            "cargo build --release --locked $BUILD_ARGS",
            "cargo nextest run --locked $TEST_ARGS --no-run",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, warm)
        self.assertNotIn("actions: write", warm)
        self.assertNotIn("github.token", warm)
        versioning = (ROOT / ".github/workflows/plugin-version.yml").read_text()
        self.assertIn("workflows: [tests]", versioning)
        self.assertNotIn("linker=rust-lld", versioning)

    def test_only_the_queue_tail_plans_a_release_build_against_main(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        batch = workflow.split("  batch:\n", 1)[1].split("  release-build:\n", 1)[0]
        self.assertIn("    needs: plan\n", batch)
        for contract in [
            "name: batch role",
            "if: github.event_name == 'merge_group'",
            "fetch-depth: 0",
            "run: python3 .github/scripts/queue_batch.py role",
            "if: steps.role.outputs.role == 'tail'",
            "TRUSTED: ${{ needs.plan.outputs.trusted }}",
            "AHEAD_SHA: ${{ github.event.merge_group.base_sha }}",
            'head="$GITHUB_SHA"',
            'if [[ "$TRUSTED" == true ]]; then\n            head="$AHEAD_SHA"\n          fi',
            'BASE_SHA="$(git merge-base origin/main HEAD)" HEAD_SHA="$head" python3 .github/scripts/affected_crates.py',
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, batch)
        for os_name in ["ubuntu", "macos", "windows"]:
            for output in ["build", "skip"]:
                with self.subTest(os=os_name, output=output):
                    self.assertIn(f"{os_name}_{output}: ${{{{ steps.affected.outputs.{os_name}_{output} }}}}", batch)
        self.assertNotIn("rust-setup", batch)
        self.assertNotIn("write", batch)

    def test_a_failed_batch_release_build_combines_the_batch(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        failure = workflow.split("  batch-failure:\n", 1)[1].split("\n\n  #", 1)[0]
        for contract in [
            "needs: [batch, release-build]",
            "github.event_name == 'merge_group' && needs.batch.outputs.role == 'tail' && needs.release-build.result == 'failure'",
            "contents: write",
            "pull-requests: write",
            "combined: ${{ steps.combine.outputs.combined }}",
            "run: python3 .github/scripts/queue_batch.py combine",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, failure)
        self.assertNotIn("cargo", failure)

    def test_the_queue_reconciler_runs_from_main_after_every_queue_run(self):
        workflow = (ROOT / ".github/workflows/queue-batch.yml").read_text()
        for contract in [
            "workflow_run:\n    workflows: [tests]\n    types: [completed]",
            "permissions:\n  contents: read",
            "group: ${{ github.event.workflow_run.event == 'merge_group' && 'queue-batch' || "
            "format('queue-batch-{0}', github.run_id) }}\n  cancel-in-progress: false",
            "if: github.event.workflow_run.event == 'merge_group'",
            "actions: write",
            "checks: write",
            "run: python3 .github/scripts/queue_batch.py reconcile",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, workflow)
        self.assertNotIn("ref:", workflow)
        self.assertNotIn("cargo", workflow)

    def test_the_queue_eject_sweep_runs_from_main_after_every_queue_run(self):
        workflow = (ROOT / ".github/workflows/queue-eject.yml").read_text()
        for contract in [
            "workflow_run:\n    workflows: [tests]\n    types: [completed]",
            "permissions:\n  contents: read",
            "group: ${{ github.event.workflow_run.event == 'merge_group' && 'queue-eject' || "
            "format('queue-eject-{0}', github.run_id) }}\n  cancel-in-progress: false",
            "if: github.event.workflow_run.event == 'merge_group'",
            "contents: write",
            "pull-requests: write",
            "run: python3 .github/scripts/queue_eject.py",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, workflow)
        self.assertNotIn("ref:", workflow)
        report = (ROOT / ".github/workflows/ci.yml").read_text().split("  queue-report:\n", 1)[1]
        self.assertIn("contents: write", report.split("    steps:", 1)[0])

    def test_one_gate_job_carries_the_merge_verdict(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        gate = workflow.split("  gate:\n", 1)[1].split("  queue-report:\n", 1)[0]
        for contract in [
            "name: ${{ github.event_name == 'merge_group' && 'queue gate' || 'merge gate' }}",
            "needs: [plan, check, batch, release-build, process-windows, registry-tool, batch-failure, queue-report]",
            "checks: write",
            "VERDICT: ${{ steps.verdict.outcome }}",
            "run: python3 .github/scripts/queue_batch.py gate",
            "if: ${{ always() && github.event_name != 'push' }}",
            '[ "$PLAN" = success ] || exit 1',
            'TRUSTED: ${{ needs.plan.outputs.trusted }}',
            '[ "$ROLE" != tail ] || [ "$BUILD" = success ] || { [ "$TRUSTED" = true ] && [ "$BUILD" = skipped ]; } || exit 1',
            '[ "$REUSED" = true ] || [ "$TRUSTED" = true ] || [ "$CHECK" = success ] || exit 1',
            '*" failure "*|*" cancelled "*) exit 1 ;;',
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, gate)
        report = workflow.split("  queue-report:\n", 1)[1]
        self.assertIn("needs: [plan, check, batch, release-build, process-windows, registry-tool, batch-failure]", report)
        self.assertIn("contains(needs.*.result, 'failure') && needs.batch-failure.outputs.combined == ''", report)

    def test_repository_lint_runs_in_the_plan_before_any_runner_compiles(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        self.assertNotIn("repository-lint", workflow)
        plan = workflow.split("  plan:\n", 1)[1].split("  check:\n", 1)[0]
        self.assertNotIn("needs:", plan.split("    steps:", 1)[0])
        self.assertIn("fetch-depth: 0", plan)
        lint = named_step(plan, "qac lint", "      ")
        for contract in [
            "if: github.event_name == 'pull_request' || github.event_name == 'merge_group'",
            "BASE_SHA: ${{ github.event.merge_group.base_sha }}",
            'base="${BASE_SHA:-$(git rev-parse HEAD^1)}"',
            'git worktree add --quiet --detach "$RUNNER_TEMP/qol-monorepo" HEAD',
            'lint "--cwd=$RUNNER_TEMP/qol-monorepo" "--base=$base"',
            "process.exit(r.findings.length ? 1 : 0);",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, lint)
        self.assertNotIn("cargo", plan)
        for job in ["check", "process-windows", "registry-tool"]:
            with self.subTest(job=job):
                dependencies = workflow.split(f"  {job}:\n", 1)[1].split("    runs-on:", 1)[0]
                self.assertIn("needs: plan", dependencies)
                self.assertIn("!cancelled() && needs.plan.result == 'success' && ", dependencies)
        build = workflow.split("  release-build:\n", 1)[1].split("    env:", 1)[0]
        self.assertIn("needs: [plan, batch]", build)
        self.assertIn("!cancelled() && needs.plan.result == 'success' && ", build)

    def test_merge_gate_requires_a_successful_plan(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        gate = workflow.split("  gate:\n", 1)[1].split("  supersede:\n", 1)[0]
        step = named_step(gate, "Every job passed or was skipped", "      ")
        script = textwrap.dedent(step.split("        run: |\n", 1)[1])
        for event in ["pull_request", "merge_group"]:
            for plan in ["success", "failure", "cancelled", "skipped"]:
                with self.subTest(event=event, plan=plan):
                    result = subprocess.run(
                        ["bash", "-e", "-c", script],
                        capture_output=True,
                        text=True,
                        env={
                            **os.environ,
                            "GITHUB_EVENT_NAME": event,
                            "PLAN": plan,
                            "REUSED": "false",
                            "CHECK": "success",
                            "BATCH": "success",
                            "ROLE": "tail",
                            "BUILD": "success",
                            "RESULTS": f"{plan} success success success skipped skipped",
                        },
                    )
                    self.assertEqual(result.returncode == 0, plan == "success", result.stdout + result.stderr)

    def test_queue_gate_needs_a_release_build_only_from_the_tail(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        gate = workflow.split("  gate:\n", 1)[1].split("  supersede:\n", 1)[0]
        step = named_step(gate, "Every job passed or was skipped", "      ")
        script = textwrap.dedent(step.split("        run: |\n", 1)[1].split("\n\n", 1)[0])
        cases = [
            ("success", "tail", "success", True),
            ("success", "tail", "failure", False),
            ("success", "tail", "skipped", False),
            ("success", "ahead", "skipped", True),
            ("success", "gone", "skipped", True),
            ("failure", "", "skipped", False),
            ("skipped", "", "skipped", False),
        ]
        for batch, role, build, passes in cases:
            with self.subTest(batch=batch, role=role, build=build):
                result = subprocess.run(
                    ["bash", "-e", "-c", script],
                    capture_output=True,
                    text=True,
                    env={
                        **os.environ,
                        "GITHUB_EVENT_NAME": "merge_group",
                        "PLAN": "success",
                        "REUSED": "false",
                        "TRUSTED": "false",
                        "CHECK": "success",
                        "BATCH": batch,
                        "ROLE": role,
                        "BUILD": build,
                        "RESULTS": f"success success {batch} {build} skipped success skipped skipped",
                    },
                )
                self.assertEqual(result.returncode == 0, passes, result.stdout + result.stderr)

    def test_queue_gate_treats_a_trusted_plan_like_a_reused_verdict(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        gate = workflow.split("  gate:\n", 1)[1].split("  supersede:\n", 1)[0]
        step = named_step(gate, "Every job passed or was skipped", "      ")
        script = textwrap.dedent(step.split("        run: |\n", 1)[1].split("\n\n", 1)[0])
        cases = [
            ("true", "skipped", "tail", "skipped", True),
            ("true", "skipped", "tail", "success", True),
            ("true", "skipped", "tail", "failure", False),
            ("true", "skipped", "ahead", "skipped", True),
            ("false", "skipped", "tail", "success", False),
            ("false", "success", "tail", "skipped", False),
            ("", "skipped", "ahead", "skipped", False),
        ]
        for trusted, check, role, build, passes in cases:
            with self.subTest(trusted=trusted, check=check, role=role, build=build):
                result = subprocess.run(
                    ["bash", "-e", "-c", script],
                    capture_output=True,
                    text=True,
                    env={
                        **os.environ,
                        "GITHUB_EVENT_NAME": "merge_group",
                        "PLAN": "success",
                        "REUSED": "false",
                        "TRUSTED": trusted,
                        "CHECK": check,
                        "BATCH": "success",
                        "ROLE": role,
                        "BUILD": build,
                        "RESULTS": f"success {check} success {build} skipped skipped skipped skipped",
                    },
                )
                self.assertEqual(result.returncode == 0, passes, result.stdout + result.stderr)

    def test_pull_request_runs_record_what_they_tested(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        plan = workflow.split("  plan:\n", 1)[1].split("  check:\n", 1)[0]
        affected = named_step(plan, "Compute affected crates", "      ")
        for contract in [
            "if: github.event_name != 'merge_group'",
            "QOL_AFFECTED_OUTPUT: ${{ runner.temp }}/affected.json",
            "run: python3 .github/scripts/affected_crates.py",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, affected)
        record = named_step(plan, "Record what this run tests", "      ")
        for contract in [
            "if: github.event_name == 'pull_request'",
            "PR_HEAD_SHA: ${{ github.event.pull_request.head.sha }}",
            'python3 .github/scripts/queue_trust.py record "$PR_HEAD_SHA" "$(git rev-parse HEAD^1)"',
            '"$RUNNER_TEMP/affected.json" "$RUNNER_TEMP/tested-base/tested-base.json"',
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, record)
        upload = named_step(plan, "Upload the tested base", "      ")
        for contract in [
            "if: github.event_name == 'pull_request'",
            "uses: actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7.0.2",
            "name: tested-base",
            "path: ${{ runner.temp }}/tested-base/tested-base.json",
            "retention-days: 7",
            "overwrite: true",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, upload)
        self.assertEqual(plan.count("queue_trust.py"), 2)
        self.assertIn("actions: read", plan.split("    steps:", 1)[0])

    def test_the_queue_plans_from_the_pull_request_run_it_trusts(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        plan = workflow.split("  plan:\n", 1)[1].split("  check:\n", 1)[0]
        outputs = plan.split("    outputs:\n", 1)[1].split("    steps:\n", 1)[0]
        self.assertIn("trusted: ${{ steps.trust.outputs.trusted }}", outputs)
        for system in ["ubuntu", "macos", "windows"]:
            for key in ["clippy", "build", "test", "doctest", "skip"]:
                name = f"{system}_{key}"
                with self.subTest(output=name):
                    self.assertIn(
                        f"{name}: ${{{{ steps.trust.outputs.{name} || steps.affected.outputs.{name} }}}}",
                        outputs,
                    )
        trust = named_step(plan, "Trust the pull request run", "      ")
        for contract in [
            "id: trust",
            "if: github.event_name == 'merge_group'",
            "GH_TOKEN: ${{ github.token }}",
            "HEAD_REF: ${{ github.event.merge_group.head_ref }}",
            "BASE_SHA: ${{ github.event.merge_group.base_sha }}",
            "HEAD_SHA: ${{ github.sha }}",
            "run: python3 .github/scripts/queue_trust.py\n",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, trust)
        covered = named_step(plan, "Pull request results cover this entry", "      ")
        self.assertIn("if: steps.trust.outputs.trusted == 'true'", covered)
        for job in ["check", "process-windows", "registry-tool"]:
            with self.subTest(job=job):
                condition = workflow.split(f"  {job}:\n", 1)[1].split("    runs-on:", 1)[0]
                self.assertIn("needs.plan.outputs.reused != 'true' && needs.plan.outputs.trusted != 'true'", condition)

    def test_pull_request_runs_test_what_the_queue_trusts(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        check = workflow.split("  check:\n", 1)[1].split("  process-windows:\n", 1)[0]
        test_steps = re.findall(r"      - name: Test\n(.*?)(?=\n      - name:|\Z)", check, re.S)
        self.assertEqual(len(test_steps), 2)
        for step in [*test_steps, named_step(check, "Install cargo-nextest", "      ")]:
            with self.subTest(step=step.splitlines()[0]):
                self.assertNotIn("pull_request", step)
        self.assertIn("if: runner.os == 'Linux' && matrix.skip != 'true'\n", check)
        self.assertIn("if: runner.os != 'Linux' && matrix.skip != 'true'\n", check)

    def test_no_run_waits_for_the_review_or_watches_for_failures(self):
        scripts = {path.name for path in (ROOT / ".github/scripts").glob("*.py")}
        for name in ["wait_for_review.py", "stop_on_failure.py"]:
            with self.subTest(script=name):
                self.assertNotIn(name, scripts)
        for name, first in [("ci.yml", "  plan:\n"), ("codeql.yml", "  analyze:\n"), ("claude-review.yml", "  review:\n")]:
            with self.subTest(workflow=name):
                workflow = (ROOT / ".github/workflows" / name).read_text()
                for loose in ["Wait for the review", "wait_for_review", "stop_on_failure", "rerun-waits", "\n  follow:\n"]:
                    self.assertNotIn(loose, workflow)
                self.assertNotIn("needs:", workflow.split(first, 1)[1].split("    runs-on:", 1)[0])
        codeql = (ROOT / ".github/workflows/codeql.yml").read_text()
        self.assertIn("group: codeql-${{ github.event_name }}-${{ github.ref }}", codeql)
        self.assertIn("cancel-in-progress: true", codeql)

    def test_a_queue_run_stops_its_matrix_at_the_first_failure_and_still_reports_it(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        check = workflow.split("  check:\n", 1)[1].split("  process-windows:\n", 1)[0]
        self.assertIn("fail-fast: ${{ github.event_name == 'merge_group' }}", check)
        build = workflow.split("  release-build:\n", 1)[1].split("  registry-tool:\n", 1)[0]
        self.assertIn("fail-fast: true", build)
        self.assertNotIn("actions: write", workflow.split("  gate:\n", 1)[1].split("  supersede:\n", 1)[0])
        for job in ["gate", "queue-report", "batch-failure"]:
            with self.subTest(job=job):
                condition = workflow.split(f"  {job}:\n", 1)[1].split("    runs-on:", 1)[0]
                self.assertIn("if: ${{ always() && ", condition)
        self.assertIn("contains(needs.*.result, 'failure')", workflow.split("  queue-report:\n", 1)[1])

    def test_only_a_queue_run_cancels_superseded_runs_and_compiles_nothing(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        supersede = workflow.split("  supersede:\n", 1)[1].split("\n\n  #", 1)[0]
        self.assertIn("if: github.event_name == 'merge_group'", supersede)
        self.assertIn("run: python3 .github/scripts/cancel_superseded_queue_runs.py", supersede)
        self.assertNotIn("rust-setup", supersede)
        self.assertNotIn("cargo", supersede)

    def test_only_a_pass_verdict_or_an_owner_override_satisfies_the_review_check(self):
        workflow = (ROOT / ".github/workflows/claude-review.yml").read_text()
        self.assertIn("  merge_group:\n", workflow.split("jobs:\n", 1)[0])
        verdict = workflow.split("  verdict:\n", 1)[1]
        for contract in [
            "name: claude review",
            "needs: [review, publish]",
            "if: ${{ always() && (github.event_name == 'pull_request' || github.event_name == 'merge_group') }}",
            "run: python3 .github/scripts/review_gate.py verdict",
            "VERDICT: ${{ needs.review.outputs.verdict }}",
            "github.event.pull_request.user.login == 'dependabot[bot]'",
            "github.event.pull_request.head.repo.full_name == github.repository",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, verdict)
        self.assertIn("verdict: ${{ steps.verdict.outputs.verdict }}", workflow)
        superseded = named_step(verdict, "End a run a newer review replaced", "      ")
        self.assertIn("if: github.event_name == 'pull_request' && needs.review.result == 'cancelled'", superseded)
        self.assertIn("run: python3 .github/scripts/review_gate.py superseded", superseded)
        publish = workflow.split("  publish:\n", 1)[1].split("    runs-on:", 1)[0]
        self.assertIn("needs.review.result != 'cancelled'", publish)

    def test_only_the_owner_overrides_the_review_and_only_for_the_commit_named(self):
        workflow = (ROOT / ".github/workflows/review-override.yml").read_text()
        self.assertIn("  issue_comment:\n    types: [created]\n", workflow.split("jobs:\n", 1)[0])
        override = workflow.split("jobs:\n", 1)[1]
        for contract in [
            "github.event.issue.pull_request &&",
            "startsWith(github.event.comment.body, '/review-override ')",
            "github.event.comment.user.login == 'KMRH47'",
            "github.triggering_actor == 'KMRH47'",
            "COMMENT: ${{ github.event.comment.body }}",
            "run: python3 .github/scripts/review_gate.py override",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, override)
        for loose in ["head.sha", "ref:", "name: claude review", "cargo"]:
            with self.subTest(loose=loose):
                self.assertNotIn(loose, override)

    def test_a_pull_request_retargeted_to_main_is_reviewed_again_before_it_arms(self):
        workflow = (ROOT / ".github/workflows/auto-merge-ready.yml").read_text()
        arm = workflow.split("  arm:\n", 1)[1]
        self.assertIn("(github.event.action != 'edited' || github.event.changes.base != null)", arm)
        self.assertIn("    permissions:\n      checks: write\n", arm)
        again = named_step(arm, "Review again against the new base", "      ")
        self.assertIn("if: github.event.action == 'edited'", again)
        self.assertLess(arm.index("- name: Review again against the new base"), arm.index("- name: Enable auto-merge"))
        script = textwrap.dedent(again.split("        run: |\n", 1)[1])
        url = "https://github.com/qol-tools/qol/pull/9"
        placeholder = "workflow api repos/qol-tools/qol/check-runs -f name=claude review -f head_sha=abc123 -f status=in_progress"
        for token, calls in [
            ("pat", [placeholder, f"pat pr close {url}", f"pat pr reopen {url}"]),
            ("", [placeholder]),
        ]:
            with self.subTest(token=token), tempfile.TemporaryDirectory() as temp:
                log = Path(temp) / "gh.log"
                gh = Path(temp) / "gh"
                gh.write_text(f'#!/usr/bin/env bash\necho "$GH_TOKEN $*" >> "{log}"\n')
                gh.chmod(0o755)
                result = subprocess.run(
                    ["bash", "-e", "-c", script],
                    capture_output=True,
                    text=True,
                    env={
                        **os.environ,
                        "PATH": f"{temp}:{os.environ['PATH']}",
                        "GH_TOKEN": "workflow",
                        "REOPEN_TOKEN": token,
                        "REPO": "qol-tools/qol",
                        "HEAD_SHA": "abc123",
                        "BASE": "main",
                        "PR_URL": url,
                    },
                )
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                logged = log.read_text().splitlines()
                self.assertEqual(len(logged), len(calls), logged)
                for line, call in zip(logged, calls):
                    self.assertTrue(line.startswith(call), line)

    def test_the_review_reads_the_live_conversation(self):
        workflow = (ROOT / ".github/workflows/claude-review.yml").read_text()
        conversation = named_step(workflow, "Read the conversation", "      ")
        self.assertIn('node "$NODE" conversation --repo "$GITHUB_REPOSITORY" --pr "$PR_NUMBER" --owner KMRH47 --out "$OUT"', conversation)
        review = named_step(workflow, "Review", "      ")
        self.assertIn('--conversation "$OUT/conversation.md"', review)
        self.assertNotIn("github.event.pull_request.body", workflow)

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

    def test_only_green_main_pushes_share_the_versioning_queue(self):
        workflow = (ROOT / ".github/workflows/plugin-version.yml").read_text()
        trigger = workflow.split("  workflow_run:\n", 1)[1].split("permissions:", 1)[0]
        self.assertIn("branches: [main]", trigger)
        group = workflow.split("concurrency:\n", 1)[1].split("jobs:\n", 1)[0]
        prepare = workflow.split("  prepare:\n", 1)[1].split("    runs-on:", 1)[0]
        condition = prepare.split("${{", 1)[1].split("}}", 1)[0].strip()
        self.assertIn(" ".join(condition.split()), " ".join(group.split()))
        self.assertIn("&& 'plugin-version-main'", group)
        self.assertIn("|| format('plugin-version-idle-{0}', github.run_id)", group)
        self.assertIn("cancel-in-progress: false", group)

    def test_versioning_releases_mains_head_when_a_stale_run_cancelled_its_run(self):
        workflow = (ROOT / ".github/workflows/plugin-version.yml").read_text()
        prepare = workflow.split("  prepare:\n", 1)[1].split("\n  plugin_candidate:", 1)[0]

        self.assertIn("      actions: read\n", prepare.split("    steps:", 1)[0])
        self.assertLess(
            prepare.index("release_candidate.py newest-source"),
            prepare.index("release_candidate.py verify-source-ci"),
        )
        verify = named_step(prepare, "Verify successful source CI", "      ")
        self.assertIn("SOURCE_SHA: ${{ steps.source.outputs.sha }}", verify)
        self.assertIn("SOURCE_CI_RUN_ID: ${{ steps.source.outputs.run_id }}", verify)
        for name in ["Verify successful source CI", "Compute release versions"]:
            with self.subTest(step=name):
                step = named_step(prepare, name, "      ")
                self.assertIn("if: steps.source.outputs.superseded != 'true'", step)
        candidate = named_step(prepare, "Commit version candidate", "      ")
        self.assertIn("SOURCE_SHA: ${{ steps.source.outputs.sha }}", candidate)

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
            "needs.qol_tray_windows_candidate.result == 'success'",
            'git push --atomic origin "${new_tags[@]}"',
        ]
        for contract in required:
            self.assertIn(contract, workflow)
        attest = named_step(workflow, "Attest exact release candidate", "      ")
        self.assertIn('--plan "${RUNNER_TEMP}/candidate-plan/release-candidate-report.json"', attest)
        self.assertIn('--build-reports "${RUNNER_TEMP}/candidate-builds"', attest)
        reports = named_step(workflow, "Download candidate build reports", "      ")
        self.assertIn("pattern: candidate-*", reports)

    def test_releases_publish_the_attested_candidate_files(self):
        workflow = (ROOT / ".github/workflows/plugin-version.yml").read_text()
        self.assertNotIn("probe", workflow)
        self.assertFalse((ROOT / ".github/scripts/release_probe.py").exists())
        for platform in ["linux", "macos", "windows"]:
            with self.subTest(platform=platform):
                job = workflow.split(f"  qol_tray_{platform}_candidate:\n", 1)[1].split("\n  qol_tray_", 1)[0].split("\n  tag_and_dispatch:", 1)[0]
                upload = named_step(job, "Upload release files", "      ")
                self.assertIn(f"name: dist-${{{{ needs.prepare.outputs.qol_tray_tag }}}}-{platform}", upload)
                self.assertIn("retention-days: 90", upload)
        for name, needs in [("release.yml", "setup"), ("qol-tray-release.yml", "gate")]:
            with self.subTest(workflow=name):
                release = (ROOT / ".github/workflows" / name).read_text()
                self.assertNotIn("release_candidate.py build", release)
                self.assertNotIn("cache-key: plugin-release-", release)
                self.assertNotIn("cache-key: qol-tray-", release)
                download = named_step(release, "Download candidate files", "      ")
                self.assertIn(f"run-id: ${{{{ needs.{needs}.outputs.candidate_run }}}}", download)
                self.assertIn("github-token: ${{ github.token }}", download)
                self.assertIn("pattern: dist-${{ env.RELEASE_TAG }}-*", download)
                self.assertIn("candidate_run: ${{ steps.verify.outputs.run_id }}", release)
                self.assertIn("actions: read", release)
                self.assertNotIn("actions: write", release.split("  index:\n", 1)[0])

    def test_versioning_builds_every_plugin_of_a_target_in_one_job(self):
        workflow = (ROOT / ".github/workflows/plugin-version.yml").read_text()
        plugin = workflow.split("  plugin_candidate:\n", 1)[1].split("  qol_tray_linux_candidate:\n", 1)[0]
        self.assertIn("name: Candidate plugins (${{ matrix.target }})", plugin)
        self.assertIn("matrix: ${{ fromJSON(needs.prepare.outputs.plugin_matrix) }}", plugin)
        build = named_step(plugin, "Build exact plugin candidates", "      ")
        for contract in [
            "RELEASE_TAGS: ${{ needs.prepare.outputs.tags }}",
            "release_candidate.py build-plugins",
            '--target "${TARGET}"',
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, build)
        self.assertNotIn("matrix.package", plugin)
        self.assertIn("id: candidates", build)
        self.assertNotIn("upload-next-dist", plugin)
        plugins = len(list(ROOT.glob("plugins/*/plugin.toml")))
        self.assertGreaterEqual(
            rc.UPLOAD_SLOTS,
            plugins,
            "raise UPLOAD_SLOTS and add upload slots to plugin_candidate: one target can build every plugin",
        )
        self.assertEqual(plugin.count("      - name: Upload release files "), rc.UPLOAD_SLOTS)
        for slot in range(rc.UPLOAD_SLOTS):
            with self.subTest(slot=slot):
                upload = named_step(plugin, f"Upload release files {slot}", "      ")
                for contract in [
                    f"if: steps.candidates.outputs.dist_name_{slot} != ''",
                    "uses: actions/upload-artifact@",
                    f"name: ${{{{ steps.candidates.outputs.dist_name_{slot} }}}}",
                    f"path: ${{{{ steps.candidates.outputs.dist_path_{slot} }}}}",
                    "retention-days: 90",
                    "if-no-files-found: error",
                ]:
                    self.assertIn(contract, upload)
                self.assertLess(
                    plugin.index(f"- name: Upload release files {slot}\n"),
                    plugin.index("- name: Require every plugin candidate"),
                )
        require = named_step(plugin, "Require every plugin candidate", "      ")
        self.assertIn("if: steps.candidates.outputs.failed != ''", require)
        self.assertIn("FAILED: ${{ steps.candidates.outputs.failed }}", require)
        self.assertIn("exit 1", require)
        reports = named_step(plugin, "Upload build reports", "      ")
        self.assertIn("if: always()", reports)
        self.assertIn("name: candidate-plugins-${{ matrix.target }}", reports)
        self.assertIn("path: ${{ runner.temp }}/release-build-reports", reports)

    def test_release_workflows_verify_exact_candidate(self):
        for name in ["release.yml", "qol-tray-release.yml"]:
            with self.subTest(workflow=name):
                workflow = (ROOT / ".github/workflows" / name).read_text()
                verify = workflow.index("release_candidate.py verify")
                download = workflow.index("- name: Download candidate files")
                self.assertLess(verify, download)

    def test_ci_runs_release_profile_builds(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()

        self.assertIn("merge_group:", workflow)
        self.assertIn("cargo check --release --locked $BUILD_ARGS", workflow)
        self.assertIn("cargo build --release --locked $BUILD_ARGS", workflow)
        self.assertIn("if: github.event_name == 'merge_group'", workflow)
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

    def test_every_workflow_runs_cargo_against_the_lockfile(self):
        command = re.compile(
            r"^\s*(?:run: )?cargo (?:build|check|clippy|metadata|nextest|run|test|tree)\b"
        )
        for path in sorted((ROOT / ".github/workflows").glob("*.yml")):
            with self.subTest(workflow=path.name):
                lenient = [
                    line.strip()
                    for line in path.read_text().splitlines()
                    if command.search(line) and "--locked" not in line
                ]
                self.assertEqual(lenient, [])

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
        install = named_step(check_job, "Install cargo-nextest", "      ")
        self.assertIn("        if: matrix.skip != 'true'\n", install)
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
            "windows_dev_build: ${{ steps.trust.outputs.windows_dev_build || steps.affected.outputs.windows_dev_build }}",
            "fromJSON(needs.plan.outputs.windows_dev_build || 'false')",
            "if: ${{ fromJSON(needs.plan.outputs.windows_dev_build) }}",
            "run: cargo test --locked -p qol-dev-build --all-targets",
        ]:
            self.assertIn(contract, workflow)

    def test_windows_lints_and_tests_the_affected_plan_like_the_other_runners(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        check = workflow.split("  check:\n", 1)[1].split("  process-windows:\n", 1)[0]
        for key in ["clippy", "build", "test", "doctest", "skip"]:
            with self.subTest(key=key):
                self.assertIn(
                    f"windows_{key}: ${{{{ steps.trust.outputs.windows_{key} || steps.affected.outputs.windows_{key} }}}}",
                    workflow,
                )
        for contract in [
            "          - os: windows-latest\n",
            "clippy_args: ${{ needs.plan.outputs.windows_clippy }}",
            "build_args: ${{ needs.plan.outputs.windows_build }}",
            "test_args: ${{ needs.plan.outputs.windows_test }}",
            "doctest: ${{ needs.plan.outputs.windows_doctest }}",
            "skip: ${{ needs.plan.outputs.windows_skip }}",
            "    defaults:\n      run:\n        shell: bash\n",
            "if: runner.os != 'Linux' && matrix.skip != 'true'\n",
            "cargo clippy --locked --keep-going $CLIPPY_ARGS -- -D warnings",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, check)

    def test_main_warms_the_release_build_cache_on_every_os(self):
        warm = (ROOT / ".github/workflows/build-cache.yml").read_text()
        self.assertNotIn("release_build", warm)
        for contract in [
            "          - os: windows-latest\n            plan: windows\n",
            "          - os: ubuntu-latest\n            plan: ubuntu\n",
            "          - os: macos-latest\n            plan: macos\n",
            "    defaults:\n      run:\n        shell: bash\n",
            "          cargo build --release --locked $BUILD_ARGS\n",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, warm)

    def test_tray_release_ships_the_windows_executable_and_installer_bundle(self):
        versioning = (ROOT / ".github/workflows/plugin-version.yml").read_text()
        workflow = (ROOT / ".github/workflows/qol-tray-release.yml").read_text()
        job = versioning.split("  qol_tray_windows_candidate:\n", 1)[1].split("  tag_and_dispatch:\n", 1)[0]
        for contract in [
            "runs-on: windows-latest",
            "RUSTFLAGS: -D warnings -C link-arg=/Brepro -C target-feature=+crt-static",
            "cache-key: qol-tray-windows",
            "--kind qol-tray-windows",
            'cp "${BUILT}/qol-tray.exe" "dist/${BUNDLE}.exe"',
            'cp "${BUILT}/qol-tray.exe" "${BUILT}/qol-tray-install.exe" "${BUNDLE}/"',
            '7z a -tzip "dist/${BUNDLE}.zip" "${BUNDLE}"',
            "BUNDLE=qol-tray-windows-x86_64",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, job)
        release = workflow.split("  release:\n", 1)[1]
        self.assertIn("needs: gate", release)
        self.assertIn("for platform in linux macos windows; do", named_step(release, "Require every platform", "      "))
        updater = (ROOT / "apps/tray/src/updates/platform/windows.rs").read_text()
        self.assertIn('format!("qol-tray-windows-{}.exe", std::env::consts::ARCH)', updater)

    def test_merge_queue_builds_windows_release_binaries(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        sandbox = workflow.split("  process-windows:\n", 1)[1].split("  release-build:\n", 1)[0]
        build = workflow.split("  release-build:\n", 1)[1].split("  registry-tool:\n", 1)[0]

        self.assertNotIn("windows_apps", workflow)
        self.assertNotIn("cargo build", sandbox)
        self.assertNotIn("upload-artifact", sandbox)
        self.assertIn(
            "RUSTFLAGS: -D warnings -C linker=rust-lld -C target-feature=+crt-static", sandbox
        )
        for contract in [
            "          - os: windows-latest\n"
            "            build_args: ${{ needs.batch.outputs.windows_build }}\n"
            "            skip: ${{ needs.batch.outputs.windows_skip }}\n",
            "timeout-minutes: ${{ matrix.os == 'windows-latest' && 75 || 45 }}",
            "    defaults:\n      run:\n        shell: bash\n",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, build)

    def test_windows_builds_link_the_c_runtime_statically(self):
        for name in [
            "ci.yml",
            "build-cache.yml",
            "release.yml",
            "plugin-version.yml",
            "qol-tray-release.yml",
        ]:
            workflow = (ROOT / ".github/workflows" / name).read_text()
            for line in workflow.splitlines():
                if "RUSTFLAGS:" in line and ("windows" in line or "/Brepro" in line):
                    with self.subTest(workflow=name, line=line.strip()):
                        self.assertIn("-C target-feature=+crt-static", line)
                        self.assertIn("-D warnings", line)

    def test_windows_release_jobs_get_a_longer_timeout(self):
        conditional = "timeout-minutes: ${{ matrix.os == 'windows-latest' && 180 || 120 }}"
        cases = [
            ("plugin-version.yml", "  plugin_candidate:\n", "  qol_tray_linux_candidate:\n", conditional),
            ("plugin-version.yml", "  qol_tray_windows_candidate:\n", "    steps:\n", "timeout-minutes: 75"),
        ]
        for name, start, end, timeout in cases:
            with self.subTest(workflow=name, job=start.strip()):
                workflow = (ROOT / ".github/workflows" / name).read_text()
                job = workflow.split(start, 1)[1].split(end, 1)[0]
                self.assertIn(timeout, job)

    def test_tray_builds_reach_the_index_only_after_the_release_is_tagged(self):
        versioning = (ROOT / ".github/workflows/plugin-version.yml").read_text()
        publish = (ROOT / ".github/workflows/qol-tray-host-publish.yml").read_text()
        self.assertLess(versioning.index("  tag_and_dispatch:\n"), versioning.index("  qol_tray_linux_publish:\n"))
        for platform in ["linux", "macos", "windows"]:
            with self.subTest(platform=platform):
                job = versioning.split(f"  qol_tray_{platform}_publish:\n", 1)[1].split("\n  qol_tray_", 1)[0]
                needs = job.split("    needs:\n", 1)[1].split("    if:", 1)[0]
                self.assertEqual(needs.split(), ["-", "prepare", "-", "tag_and_dispatch"])
                self.assertIn("needs.tag_and_dispatch.result == 'success'", job)
                self.assertIn("!cancelled() &&", job)
                self.assertIn("uses: ./.github/workflows/qol-tray-host-publish.yml", job)
                self.assertIn(f"os: {platform}", job)
        attest = publish.split("  attest:\n", 1)[1].split("\n  registry:\n", 1)[0]
        self.assertIn("actions/attest-build-provenance@", attest)
        self.assertNotIn("cargo", attest)
        registry = publish.split("  registry:\n", 1)[1].split("\n  index:\n", 1)[0]
        self.assertIn("needs: attest", registry)
        self.assertIn("persist-credentials: false", registry)
        self.assertNotIn("id-token", registry)
        build = named_step(registry, "Build registry tool", "      ")
        self.assertNotIn("token", build)
        index = publish.split("  index:\n", 1)[1]
        self.assertIn("needs: registry", index)
        self.assertIn("gh workflow run plugin-index.yml --ref main", index)

    def test_tray_release_builds_drop_checkout_credentials(self):
        workflow = (ROOT / ".github/workflows/plugin-version.yml").read_text()
        for job, end in [
            ("  qol_tray_linux_candidate:\n", "  qol_tray_macos_candidate:\n"),
            ("  qol_tray_macos_candidate:\n", "  qol_tray_windows_candidate:\n"),
            ("  qol_tray_windows_candidate:\n", "  tag_and_dispatch:\n"),
        ]:
            with self.subTest(job=job.strip()):
                body = workflow.split(job, 1)[1].split(end, 1)[0]
                self.assertIn("persist-credentials: false", body)

    def test_plugin_release_builds_run_bash_on_windows(self):
        cases = [
            (".github/workflows/plugin-version.yml", "  plugin_candidate:\n", "  qol_tray_linux_candidate:\n"),
            (".github/workflows/plugin-version.yml", "  qol_tray_windows_candidate:\n", "  tag_and_dispatch:\n"),
        ]
        for path, start, end in cases:
            with self.subTest(path=path, job=start.strip()):
                workflow = (ROOT / path).read_text()
                job = workflow.split(start, 1)[1].split(end, 1)[0]
                self.assertIn("    defaults:\n      run:\n        shell: bash\n", job)

    def test_plugins_publish_no_github_release(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()

        self.assertNotIn("gh release", workflow)
        self.assertNotIn("contents: write", workflow)

    def test_plugin_registry_push_waits_for_every_target_file(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        registry = workflow.split("  registry:\n", 1)[1].split("  index:\n", 1)[0]
        self.assertIn("needs: setup", registry)
        self.assertLess(
            registry.index("release_candidate.py require-release-files"),
            registry.index("- name: Push registry artifact"),
        )
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
            "ci.yml": ("  registry-tool:\n", "  gate:\n"),
        }
        for name, (begin, finish) in jobs.items():
            with self.subTest(workflow=name):
                workflow = (ROOT / ".github/workflows" / name).read_text()
                job = workflow.split(begin, 1)[1].split(finish, 1)[0]
                self.assertIn("persist-credentials: false", job)
                self.assertIn("cache-key: plugin-registry", job)
                self.assertIn('system-dependencies: "false"', job)
                self.assertIn("RUSTFLAGS: -D warnings -C link-arg=-fuse-ld=lld", job)
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
        self.assertIn("if: needs.build.outputs.changed == 'true'", sign)
        self.assertIn("name: plugin-index\n      deployment: false", sign)
        compare = named_step(build, "Compare with the deployed index", "      ")
        self.assertIn("jq -S 'del(.serial)' index.json", compare)
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
        bundle = named_step(sign, "Bundle the signature with the index", "      ")
        self.assertIn("> site/plugins/index.signed.json", bundle)
        self.assertIn("jq -j .index site/plugins/index.signed.json | cmp - site/plugins/index.json", bundle)
        self.assertIn(
            "jq -j .signature site/plugins/index.signed.json | cmp - site/plugins/index.json.minisig",
            bundle,
        )
        self.assertRegex(build, r"INDEX_URL: [^\n]+/plugins/index\.signed\.json\n")

        secret_steps = [step for step in workflow.split("\n      - ") if "secrets." in step]
        self.assertEqual(len(secret_steps), 1)
        self.assertIn("name: Sign index", secret_steps[0])
        order = [
            "- name: Install minisign",
            "actions/download-artifact",
            "- name: Sign index",
            "- name: Verify with the key the tray ships",
            "- name: Bundle the signature with the index",
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

    def test_tray_release_notes_list_only_the_changes_released_for_the_tray(self):
        workflow = (ROOT / ".github/workflows/qol-tray-release.yml").read_text()
        release = workflow.split("  release:\n", 1)[1]
        source = release.split("      - uses: actions/checkout@", 1)[1].split("\n      - ", 1)[0]
        self.assertIn("path: source", source)
        self.assertIn("fetch-depth: 0", source)
        notes = named_step(release, "Write user-facing release notes", "      ")
        unguarded, guarded = notes.split('if [[ -n "${PREVIOUS_TAG}" ]]; then\n', 1)
        guarded = guarded.split("\n          fi\n", 1)[0]
        for contract, text in [
            ("filter=(--root source)", unguarded),
            ('filter+=(--unit qol-tray --range "${PREVIOUS_TAG}..${RELEASE_TAG}")', guarded),
            ('python3 source/.github/scripts/release_notes.py "${filter[@]}"', notes),
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, text)

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
