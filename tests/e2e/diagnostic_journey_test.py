"""Offline registration/accounting checks; never opens provider or Docker I/O."""
import copy
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest import mock
import uuid


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("diagnostic_journey", Path(__file__).with_name("diagnostic_journey.py"))
CONTROL = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CONTROL)
RUNNER_SPEC = importlib.util.spec_from_file_location("live_deepseek_journey", Path(__file__).with_name("live_deepseek_journey.py"))
RUNNER = importlib.util.module_from_spec(RUNNER_SPEC)
RUNNER_SPEC.loader.exec_module(RUNNER)


class DiagnosticJourneyTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="nwq-registration-")
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.output = self.directory / "output"
        self.output.mkdir(mode=0o700)
        self.profile = json.loads((ROOT / CONTROL.PROFILE_PATH).read_bytes())
        self.base = self.directory / "base.env"
        self.candidate = self.directory / "candidate.env"
        self.base.write_bytes(b"test-base")
        self.candidate.write_bytes(b"test-candidate")
        self.registration_file = self.directory / "registration.json"
        self.now = datetime(2030, 1, 1, tzinfo=timezone.utc)
        budget_id = str(uuid.uuid4())
        self.value = {
            "schema": CONTROL.REGISTRATION_SCHEMA, "budget_id": budget_id,
            "hypothesis": "Synthetic offline registration validation",
            "candidate_git_sha": "a" * 40,
            "base_manifest_sha256": CONTROL.digest(self.base.read_bytes()),
            "candidate_manifest_sha256": CONTROL.digest(self.candidate.read_bytes()),
            "base_application_image_ids": {key: "sha256:" + "a" * 64 for key in CONTROL.APP_KEYS},
            "candidate_application_image_ids": {key: "sha256:" + "b" * 64 for key in CONTROL.APP_KEYS},
            "profile_sha256": CONTROL.digest((ROOT / CONTROL.PROFILE_PATH).read_bytes()),
            "product_fixture_sha256": CONTROL.digest((ROOT / "tests/e2e/fixtures/h4-journey-v1.json").read_bytes()),
            "prompt_schema_identities": {"canon_prompt": "test-version"},
            "limits": {"profile": CONTROL.PROFILE, "max_attempts": 5,
                       "max_tokens": 20000000, "max_cost_micro_cny": 35000000,
                       "expires_at": "2030-01-01T01:00:00Z"},
            "output_dir": str(self.output),
            "ledger_path": str(self.directory / (budget_id + ".jsonl")),
        }

    def load(self, value=None, approved=None):
        value = self.value if value is None else value
        encoded = CONTROL.canonical(value)
        self.registration_file.write_bytes(encoded)
        self.registration_file.chmod(0o600)
        return CONTROL.load_registration(
            self.registration_file, approved or CONTROL.digest(encoded), root=ROOT,
            git_sha="a" * 40, output=self.output, base_manifest=self.base,
            candidate_manifest=self.candidate,
            prompt_schema_identities={"canon_prompt": "test-version"}, now=self.now,
        )

    def test_registration_preserves_exact_environment_and_immutable_value(self):
        registration = self.load()
        environment = registration.environment()
        self.assertEqual(environment["LLM_DIAGNOSTIC_BUDGET_ID"], self.value["budget_id"])
        self.assertEqual(json.loads(environment["LLM_DIAGNOSTIC_BUDGET_LIMITS"]), self.value["limits"])
        modified = registration.value
        modified["limits"]["max_attempts"] = 100
        self.assertEqual(registration.value, self.value)
        self.assertNotIn("api_key", CONTROL.canonical(registration.value).decode())

    def test_registration_rejects_changed_reviewed_identity(self):
        approved = CONTROL.digest(CONTROL.canonical(self.value))
        for key, replacement in (
            ("candidate_git_sha", "b" * 40), ("budget_id", str(uuid.uuid4())),
            ("profile_sha256", "0" * 64), ("base_manifest_sha256", "0" * 64),
            ("output_dir", str(self.directory)),
        ):
            with self.subTest(key=key):
                value = copy.deepcopy(self.value)
                value[key] = replacement
                with self.assertRaises(CONTROL.DiagnosticFailure):
                    self.load(value, approved)

    def test_registration_rejects_invalid_caps_and_expiry_even_with_matching_hash(self):
        for key, replacement in (
            ("max_attempts", True), ("max_attempts", -1), ("max_attempts", 2001),
            ("max_tokens", 20000001), ("max_cost_micro_cny", 35000001),
            ("expires_at", "2030-01-01T00:00:00Z"),
            ("expires_at", "2030-01-01T04:00:01Z"),
            ("expires_at", "2030-01-01T01:00:00+00:00"),
            ("expires_at", "2030-02-30T01:00:00Z"),
        ):
            with self.subTest(key=key, replacement=replacement):
                value = copy.deepcopy(self.value)
                value["limits"][key] = replacement
                with self.assertRaises(CONTROL.DiagnosticFailure):
                    self.load(value)

    def test_duplicates_nonfinite_unknowns_private_permissions_and_symlinks_fail(self):
        for raw in ('{"a":1,"a":2}', '{"a":NaN}', '{"a":Infinity}'):
            with self.assertRaises(CONTROL.DiagnosticFailure):
                CONTROL.strict_json(raw)
        value = {**self.value, "unexpected": 1}
        with self.assertRaises(CONTROL.DiagnosticFailure):
            self.load(value)
        self.output.chmod(0o755)
        with self.assertRaises(CONTROL.DiagnosticFailure):
            self.load()
        self.output.chmod(0o700)
        linked = self.directory / "linked-output"
        linked.symlink_to(self.output, target_is_directory=True)
        with self.assertRaises(CONTROL.DiagnosticFailure):
            CONTROL.private_path(linked, ROOT, directory=True)

    def test_single_start_race_and_terminal_reuse(self):
        registration = self.load()

        def start(_):
            ledger = CONTROL.DiagnosticLedger(registration)
            try:
                ledger.start()
                return ledger
            except CONTROL.DiagnosticFailure:
                return None

        with ThreadPoolExecutor(max_workers=4) as workers:
            winners = [ledger for ledger in workers.map(start, range(4)) if ledger]
        self.assertEqual(len(winners), 1)
        winners[0].finish(False, ["synthetic_failure"])
        path = Path(self.value["ledger_path"])
        records = [json.loads(line) for line in path.read_text().splitlines()]
        self.assertEqual([record["status"] for record in records], ["Started", "Failed"])
        self.assertEqual(records[0]["registration"], self.value)
        self.assertEqual(path.stat().st_mode & 0o777, 0o600)
        with self.assertRaises(CONTROL.DiagnosticFailure):
            CONTROL.DiagnosticLedger(registration).start()
        with self.assertRaises(CONTROL.DiagnosticFailure):
            self.load()

    def test_abandoned_and_failed_fsync_starts_remain_non_resumable(self):
        registration = self.load()
        ledger = CONTROL.DiagnosticLedger(registration)
        with mock.patch.object(CONTROL.os, "fsync", side_effect=OSError("synthetic")):
            with self.assertRaises(OSError):
                ledger.start()
        self.assertIsNone(ledger.descriptor)
        with self.assertRaises(CONTROL.DiagnosticFailure):
            CONTROL.DiagnosticLedger(registration).start()
        self.assertTrue(Path(self.value["ledger_path"]).exists())

    def snapshot(self, settled=False):
        registration = self.load()
        row = {"budget_id": self.value["budget_id"], "attempt_id": str(uuid.uuid4()),
               "ordinal": 1, "operation": "setup_connection", "output_limit": 8,
               "reservation_tokens": 1048584, "reservation_cost_micro_cny": 3145800,
               "settled": settled, "settlement_model": CONTROL.MODEL if settled else None,
               "input_tokens": 10 if settled else None, "output_tokens": 2 if settled else None,
               "cached_input_tokens": None}
        budget = {**registration.binding, **{
            key: item for key, item in self.value["limits"].items() if key != "profile"},
            "charged_attempts": 1, "charged_tokens": 12 if settled else 1048584,
            "charged_cost_micro_cny": 48 if settled else 3145800, "sealed": False}
        return registration, {"budget": budget, "receipts": [row]}

    def test_reconcile_metrics_bounds_receipts_against_generation_counters(self):
        registration, snapshot = self.snapshot(settled=True)
        snapshot["budget"]["sealed"] = True

        def metrics(rows):
            totals = {"attempts": len(rows), "tokens.input": 0, "tokens.output": 0,
                      "tokens.cached_input": 0, "billable_tokens.cached_input": 0,
                      "billable_tokens.uncached_input": 0, "billable_tokens.output": 0}
            for row in rows:
                cached = row["cached_input_tokens"] or 0
                totals["tokens.input"] += row["input_tokens"]
                totals["tokens.output"] += row["output_tokens"]
                totals["tokens.cached_input"] += cached
                totals["billable_tokens.cached_input"] += cached
                totals["billable_tokens.uncached_input"] += row["input_tokens"] - cached
                totals["billable_tokens.output"] += row["output_tokens"]
            return {"counter_totals": [
                {"operation": "setup_connection", "provider_model": "deepseek/" + CONTROL.MODEL,
                 "counter": "attempts.requests" if counter == "attempts" else counter,
                 "value": value}
                for counter, value in totals.items()
            ]}

        CONTROL.reconcile_metrics(registration, snapshot, metrics(snapshot["receipts"]))

        second = copy.deepcopy(snapshot["receipts"][0])
        second.update(attempt_id=str(uuid.uuid4()), ordinal=2)
        snapshot["receipts"].append(second)
        snapshot["budget"].update(charged_attempts=2, charged_tokens=24, charged_cost_micro_cny=96)
        first_metrics = metrics([snapshot["receipts"][0]])["counter_totals"]
        second_metrics = metrics([second])["counter_totals"]
        CONTROL.reconcile_metrics(
            registration, snapshot, {"counter_totals": [
                {**left, "value": left["value"] + right["value"]}
                for left, right in zip(first_metrics, second_metrics)
            ]}
        )

        for summary in (
            {"counter_totals": [item for item in metrics(snapshot["receipts"])["counter_totals"]
                                 if item["counter"] != "attempts.requests"]},
            {"counter_totals": [*metrics(snapshot["receipts"])["counter_totals"], {
                "operation": "setup_connection", "provider_model": "deepseek/" + CONTROL.MODEL,
                "counter": "attempts.requests", "value": 3}]},
        ):
            with self.assertRaises(CONTROL.DiagnosticFailure):
                CONTROL.reconcile_metrics(registration, snapshot, summary)

        changed = copy.deepcopy(snapshot)
        changed["receipts"][0]["output_tokens"] += 1
        changed["budget"]["charged_tokens"] += 1
        changed["budget"]["charged_cost_micro_cny"] += 9
        with self.assertRaises(CONTROL.DiagnosticFailure) as mismatch:
            CONTROL.reconcile_metrics(registration, changed, metrics(snapshot["receipts"]))
        self.assertEqual(mismatch.exception.code, "diagnostic_metrics_receipt_mismatch")

        changed = copy.deepcopy(snapshot)
        changed["receipts"][0]["cached_input_tokens"] = 2
        with self.assertRaises(CONTROL.DiagnosticFailure):
            CONTROL.reconcile_metrics(registration, changed, metrics(snapshot["receipts"]))
        CONTROL.reconcile_metrics(registration, changed, metrics(changed["receipts"]))

        # Final PG may contain a late settlement absent from the last scrape.
        with self.assertRaises(CONTROL.DiagnosticFailure):
            CONTROL.reconcile_metrics(registration, snapshot, metrics([second]))
        _, unresolved = self.snapshot()
        unresolved["budget"]["sealed"] = True
        with self.assertRaises(CONTROL.DiagnosticFailure) as pending:
            CONTROL.reconcile_metrics(registration, unresolved, metrics([second]))
        self.assertEqual(pending.exception.code, "diagnostic_metrics_receipts_unresolved")

        changed = copy.deepcopy(snapshot)
        changed["receipts"][0]["operation"] = "unknown_operation"
        with self.assertRaises(CONTROL.DiagnosticFailure):
            CONTROL.reconcile_metrics(registration, changed, metrics(snapshot["receipts"]))

        empty = copy.deepcopy(snapshot)
        empty["receipts"] = []
        empty["budget"].update(charged_attempts=0, charged_tokens=0, charged_cost_micro_cny=0)
        CONTROL.reconcile_metrics(registration, empty, {"counter_totals": []})
        with self.assertRaises(CONTROL.DiagnosticFailure):
            CONTROL.reconcile_metrics(registration, empty, {
                "counter_totals": [{
                    "operation": "setup_connection", "provider_model": "deepseek/" + CONTROL.MODEL,
                    "counter": "attempts.requests", "value": 1,
                }]
            })

    def test_exact_charges_and_one_time_late_settlement(self):
        registration, before = self.snapshot()
        before["budget"]["sealed"] = True
        unresolved = CONTROL.reconcile_snapshot(registration, before)
        self.assertEqual(unresolved["unresolved_attempts"], 1)
        after = copy.deepcopy(before)
        after["receipts"][0].update(settled=True, settlement_model=CONTROL.MODEL,
                                      input_tokens=10, output_tokens=2, cached_input_tokens=3)
        after["budget"].update(charged_tokens=12, charged_cost_micro_cny=48, sealed=True)
        settled = CONTROL.reconcile_snapshot(registration, after, before)
        self.assertEqual(settled["charged"], {"attempts": 1, "tokens": 12, "cost_micro_cny": 48})
        self.assertEqual(settled["unresolved_attempts"], 0)
        self.assertEqual(CONTROL.reconcile_snapshot(registration, after, after), settled)
        with self.assertRaises(CONTROL.DiagnosticFailure):
            CONTROL.reconcile_snapshot(registration, before, after)

    def test_sealed_budget_cannot_gain_a_new_reservation(self):
        registration, after = self.snapshot()
        after["budget"]["sealed"] = True
        before = copy.deepcopy(after)
        before["receipts"] = []
        before["budget"].update(charged_attempts=0, charged_tokens=0, charged_cost_micro_cny=0)
        with self.assertRaises(CONTROL.DiagnosticFailure) as rejected:
            CONTROL.reconcile_snapshot(registration, after, before)
        self.assertEqual(rejected.exception.code, "diagnostic_reservation_after_seal")

    def test_receipt_corruption_reset_refill_and_identity_drift_fail(self):
        registration, before = self.snapshot(settled=True)
        mutations = [
            lambda s: s["receipts"].clear(),
            lambda s: s["receipts"].append(copy.deepcopy(s["receipts"][0])),
            lambda s: s["receipts"][0].update(ordinal=2),
            lambda s: s["receipts"][0].update(input_tokens=True),
            lambda s: s["receipts"][0].update(settlement_model="deepseek-v4-flash"),
            lambda s: s["receipts"][0].update(cached_input_tokens=11),
            lambda s: s["receipts"][0].update(reservation_tokens=8),
            lambda s: s["receipts"][0].update(output_limit=9),
            lambda s: s["budget"].update(charged_tokens=0),
            lambda s: s["budget"].update(max_attempts=100),
            lambda s: s["budget"].update(expires_at="2030-01-01T02:00:00Z"),
        ]
        for mutate in mutations:
            with self.subTest(mutation=mutate):
                after = copy.deepcopy(before)
                mutate(after)
                with self.assertRaises(CONTROL.DiagnosticFailure):
                    CONTROL.reconcile_snapshot(registration, after, before)

    def test_bounded_command_keeps_input_out_of_argv_and_bounds_output_and_time(self):
        command = [sys.executable, "-c", "import sys; sys.stdout.buffer.write(sys.stdin.buffer.read())"]
        self.assertEqual(CONTROL.bounded_command(command, stdin=b"test-only", maximum=10), b"test-only")
        with self.assertRaises(CONTROL.DiagnosticFailure) as oversized:
            CONTROL.bounded_command(command, stdin=b"test-only", maximum=3)
        self.assertEqual(oversized.exception.code, "diagnostic_command_output_oversized")
        started = time.monotonic()
        with self.assertRaises(CONTROL.DiagnosticFailure) as timeout:
            CONTROL.bounded_command([sys.executable, "-c", "import time; time.sleep(30)"], timeout=0.3)
        self.assertEqual(timeout.exception.code, "diagnostic_command_timeout")
        self.assertLess(time.monotonic() - started, 2)

    def test_snapshot_command_has_fixed_scope_and_parameterized_read_only_sql(self):
        command, sql = CONTROL.snapshot_command("nwq-abcdef1234", self.value["budget_id"])
        self.assertIn("budget_id=" + self.value["budget_id"], command)
        self.assertNotIn(self.value["budget_id"].encode(), sql)
        self.assertIn(b"REPEATABLE READ READ ONLY", sql)
        self.assertIn(b"lock_timeout = '2s'", sql)
        self.assertIn(b"statement_timeout = '5s'", sql)
        for prefix, identifier in (("novel", self.value["budget_id"]), ("nwq-abcdef1234", "';DROP TABLE users;")):
            with self.assertRaises(CONTROL.DiagnosticFailure):
                CONTROL.snapshot_command(prefix, identifier)

    def journey(self):
        for path, marker in ((self.base, "b"), (self.candidate, "a")):
            manifest = {"RELEASE_VERSION": "test", "RELEASE_GIT_SHA": marker * 40,
                        **{key: "registry.invalid/service@sha256:" + marker * 64
                           for key in RUNNER.RELEASE_IMAGE_KEYS}}
            path.write_text("".join(f"{key}={item}\n" for key, item in manifest.items()))
        self.value["base_manifest_sha256"] = CONTROL.digest(self.base.read_bytes())
        self.value["candidate_manifest_sha256"] = CONTROL.digest(self.candidate.read_bytes())
        registration = self.load()
        config = self.directory / "config.json"
        config.write_text(json.dumps({"provider": "deepseek", "model": CONTROL.MODEL,
                                      "thinking_enabled": False, "api_key": "test-only",
                                      "api_url": "https://api.deepseek.com"}))
        config.chmod(0o600)
        return RUNNER.Journey(ROOT, config, self.output, "a" * 40, self.base,
                              self.candidate, None, None, "bash", "Diagnostic",
                              diagnostic_registration=registration)

    def test_generated_environment_uses_registration_and_strips_ambient_overrides(self):
        journey = self.journey()

        def fake_run(command, **_):
            if command[:2] == ["git", "clone"]:
                Path(command[-1]).mkdir()
            elif command[:2] != ["git", "checkout"]:
                raise AssertionError("unexpected subprocess")
            return ""

        with mock.patch.object(RUNNER, "run", side_effect=fake_run), mock.patch.dict(
            RUNNER.os.environ, {"LLM_DIAGNOSTIC_BUDGET_ID": "untrusted", "LLM_API_KEY": "test-only"}
        ):
            journey.prepare_runtime()
        self.addCleanup(journey.runtime_temp.cleanup)
        environment = dict(line.split("=", 1) for line in (journey.runtime_root / ".env").read_text().splitlines() if line)
        self.assertEqual(environment["LLM_DIAGNOSTIC_BUDGET_ID"], self.value["budget_id"])
        self.assertEqual(json.loads(environment["LLM_DIAGNOSTIC_BUDGET_LIMITS"]), self.value["limits"])
        self.assertEqual(environment["LLM_API_KEY"], "")
        self.assertNotIn("LLM_DIAGNOSTIC_BUDGET_ID", journey.compose_env)
        self.assertNotIn("LLM_API_KEY", journey.compose_env)
        self.assertEqual(journey.expected_model, CONTROL.MODEL)
        self.assertFalse(journey.config["thinking_enabled"])
        self.assertEqual(journey.public_report()["report_kind"], "h4-vision-diagnostic-v1")

    def test_vision_public_report_has_bounded_budget_and_no_private_evidence(self):
        journey = self.journey()
        registration, snapshot = self.snapshot(settled=True)
        journey.diagnostic_last_snapshot = snapshot
        journey.internal_service_token = "synthetic-internal-service-token"
        public = journey.public_report()

        self.assertEqual(public["schema_version"], 3)
        self.assertEqual(public["report_kind"], "h4-vision-diagnostic-v1")
        self.assertEqual(public["provider"]["configured_model"], CONTROL.MODEL)
        self.assertFalse(public["thinking_enabled"])
        budget = public["diagnostic_budget"]
        self.assertEqual(
            set(budget),
            {"limits", "charged", "settled_attempts", "unresolved_attempts", "sealed"},
        )
        self.assertEqual(set(budget["limits"]), {"attempts", "tokens", "micro_cny"})
        self.assertEqual(set(budget["charged"]), {"attempts", "tokens", "micro_cny"})
        self.assertEqual(budget["settled_attempts"], 1)
        self.assertEqual(budget["unresolved_attempts"], 0)
        self.assertTrue(budget["sealed"] is False)
        encoded = json.dumps(public, sort_keys=True)
        for private in (
            registration.value["budget_id"],
            journey.internal_service_token,
            snapshot["receipts"][0]["attempt_id"],
        ):
            self.assertNotIn(private, encoded)

    def test_legacy_public_report_shape_remains_unchanged(self):
        self.journey()
        config = self.directory / "legacy-config.json"
        config.write_text(json.dumps({
            "provider": "deepseek",
            "model": RUNNER.EXPECTED_MODEL,
            "thinking_enabled": True,
            "api_key": "test-only",
            "api_url": "https://api.deepseek.com",
        }))
        config.chmod(0o600)
        journey = RUNNER.Journey(
            ROOT, config, self.output, "a" * 40, self.base, self.candidate,
            None, None, "bash", "Diagnostic",
        )
        public = journey.public_report()
        self.assertEqual(public["schema_version"], 2)
        self.assertEqual(public["report_kind"], "h4-journey-qualification-v1")
        self.assertEqual(public["provider"]["configured_model"], RUNNER.EXPECTED_MODEL)
        self.assertNotIn("diagnostic_profile", public)
        self.assertNotIn("thinking_enabled", public)
        self.assertNotIn("diagnostic_budget", public)

    def test_owner_seal_is_once_bounded_and_token_only_in_stdin(self):
        journey = self.journey()
        journey.internal_service_token = "synthetic-internal-control-value"
        response = {"binding": journey.diagnostic_registration.binding, "sealed": True}
        with mock.patch.object(RUNNER.diagnostic, "bounded_command", return_value=CONTROL.canonical(response)) as invoke:
            self.assertEqual(journey.diagnostic_owner_control(seal=True), response)
            command = invoke.call_args.args[0]
            self.assertNotIn(journey.internal_service_token, " ".join(command))
            self.assertIn(journey.internal_service_token.encode(), invoke.call_args.kwargs["stdin"])
            self.assertEqual(invoke.call_args.kwargs["timeout"], 5)
            self.assertEqual(invoke.call_args.kwargs["maximum"], 4096)
            with self.assertRaises(RUNNER.QualificationFailure):
                journey.diagnostic_owner_control(seal=True)
            self.assertEqual(invoke.call_count, 1)

    def terminal_runner(self, journey, *, execute=None, terminal=None, cleanup=None):
        events = []

        def execute_default():
            rows = Path(self.value["ledger_path"]).read_text().splitlines()
            self.assertEqual(json.loads(rows[0])["status"], "Started")
            self.assertEqual(len(rows), 1)
            events.append("execute")
            journey.report["outcome"] = "completed"

        def terminal_default():
            events.append("terminal")
            journey.diagnostic_evidence_durable = True

        def cleanup_default():
            events.append("cleanup")
            journey.report["environment"].update(
                isolated_cleanup_completed=True, existing_user_stack_unchanged=True,
            )

        stack = self.enterContext(RUNNER.contextlib.ExitStack())
        for name, function in (("execute", execute or execute_default),
                               ("diagnostic_terminal", terminal or terminal_default),
                               ("cleanup", cleanup or cleanup_default)):
            stack.enter_context(mock.patch.object(journey, name, side_effect=function))
        return events

    def test_terminal_started_precedes_execution_and_passed_follows_durable_reports(self):
        journey = self.journey()
        events = self.terminal_runner(journey)
        original = journey.diagnostic_ledger.finish

        def finish(passed, codes):
            self.assertTrue(passed)
            public = json.loads((self.output / "journey-report.json").read_bytes())
            private = json.loads((self.output / "journey-private.json").read_bytes())
            self.assertEqual(public["outcome"], "completed")
            self.assertEqual(private["runner_report"]["outcome"], "completed")
            events.append("finish")
            original(passed, codes)

        with mock.patch.object(journey.diagnostic_ledger, "finish", side_effect=finish):
            self.assertEqual(RUNNER.run_diagnostic(journey), 0)
        self.assertEqual(events, ["execute", "terminal", "cleanup", "finish"])
        rows = [json.loads(line) for line in Path(self.value["ledger_path"]).read_text().splitlines()]
        self.assertEqual([row["status"] for row in rows], ["Started", "Passed"])

    def test_terminal_repeated_int_term_runs_cleanup_once_and_preserves_original_failure(self):
        journey = self.journey()
        previous = {number: RUNNER.signal.getsignal(number)
                    for number in (RUNNER.signal.SIGINT, RUNNER.signal.SIGTERM)}
        events = []

        def execute():
            journey.diagnostic_failure("original_product_failure")
            RUNNER.signal.getsignal(RUNNER.signal.SIGINT)(RUNNER.signal.SIGINT, None)
            self.fail("cancel must stop product requests")

        def terminal():
            events.append("terminal")
            for number in (RUNNER.signal.SIGINT, RUNNER.signal.SIGTERM):
                RUNNER.signal.getsignal(number)(number, None)

        self.terminal_runner(journey, execute=execute, terminal=terminal)
        self.assertEqual(RUNNER.run_diagnostic(journey), 1)
        self.assertEqual(events, ["terminal"])
        journey.cleanup.assert_called_once()
        self.assertEqual(journey.report["failure"]["code"], "original_product_failure")
        self.assertEqual(journey.diagnostic_failures.count("diagnostic_cancelled"), 1)
        for number, handler in previous.items():
            self.assertEqual(RUNNER.signal.getsignal(number), handler)
        rows = [json.loads(line) for line in Path(self.value["ledger_path"]).read_text().splitlines()]
        self.assertEqual([row["status"] for row in rows], ["Started", "Failed"])

    def test_terminal_evidence_exception_revokes_volume_deletion_before_cleanup(self):
        journey = self.journey()

        def terminal():
            journey.diagnostic_evidence_durable = True
            raise OSError("synthetic-private-detail")

        def cleanup():
            self.assertFalse(journey.diagnostic_evidence_durable)

        self.terminal_runner(journey, terminal=terminal, cleanup=cleanup)
        self.assertEqual(RUNNER.run_diagnostic(journey), 1)
        self.assertNotIn("synthetic-private-detail", (self.output / "journey-report.json").read_text())
        self.assertIn("diagnostic_terminal_unproven", journey.diagnostic_failures)

    def test_final_report_failure_does_not_pass_or_repeat_cleanup(self):
        journey = self.journey()
        self.terminal_runner(journey)
        with mock.patch.object(journey, "write_report", side_effect=OSError("private-path")):
            self.assertEqual(RUNNER.run_diagnostic(journey), 1)
        journey.cleanup.assert_called_once()
        rows = [json.loads(line) for line in Path(self.value["ledger_path"]).read_text().splitlines()]
        self.assertEqual([row["status"] for row in rows], ["Started", "Failed"])
        self.assertEqual(rows[-1]["failure_codes"], ["diagnostic_report_write_failed"])
        self.assertTrue((self.output / "terminal-failure.json").is_file())

    def test_terminal_ledger_failure_never_retries_or_returns_success(self):
        journey = self.journey()
        self.terminal_runner(journey)
        with mock.patch.object(journey.diagnostic_ledger, "finish", side_effect=OSError("fsync")) as finish:
            self.assertEqual(RUNNER.run_diagnostic(journey), 1)
        finish.assert_called_once()
        self.assertIsNone(journey.diagnostic_ledger.descriptor)
        marker = json.loads((self.output / "terminal-failure.json").read_bytes())
        self.assertEqual(marker["outcome"], "failed")
        self.assertIn("diagnostic_terminal_write_failed", marker["failure_codes"])
        public = json.loads((self.output / "journey-report.json").read_bytes())
        private = json.loads((self.output / "journey-private.json").read_bytes())
        self.assertEqual(public["outcome"], "failed")
        self.assertEqual(private["runner_report"]["outcome"], "failed")

    def test_existing_started_refusal_does_not_execute_cleanup_or_write_reports(self):
        journey = self.journey()
        ledger = Path(self.value["ledger_path"])
        ledger.write_bytes(b"partial-started")
        with mock.patch.object(journey, "execute") as execute, mock.patch.object(
            journey, "cleanup"
        ) as cleanup, mock.patch.object(journey, "write_report") as report:
            self.assertEqual(RUNNER.run_diagnostic(journey), 1)
        execute.assert_not_called()
        cleanup.assert_not_called()
        report.assert_not_called()
        self.assertEqual(ledger.read_bytes(), b"partial-started")

    def test_terminal_exact_payers_and_unresolved_drain_bound(self):
        for scenario in ("complete", "missing", "unresolved", "recreated", "image-drift", "metrics-drift"):
            with self.subTest(scenario=scenario):
                journey = self.journey()
                journey.cleanup_required = True
                registration, snapshot = self.snapshot(settled=True)
                snapshot["receipts"] = []
                snapshot["budget"].update(sealed=True, charged_attempts=0, charged_tokens=0,
                                          charged_cost_micro_cny=0)
                journey.diagnostic_registration = registration
                journey.diagnostic_last_snapshot = snapshot

                def observability(*_):
                    journey.report["llm_metrics"] = {"counter_totals": [] if scenario != "metrics-drift" else [{
                        "operation": "setup_connection", "provider_model": "deepseek/" + CONTROL.MODEL,
                        "counter": "attempts.success", "value": 1,
                    }]}
                ids = {service: f"{index:064x}" for index, service in enumerate(RUNNER.SERVICE_PORTS, 1)}
                journey.private_report["release_images"] = {"base": {
                    service.upper().replace("-", "_") + "_IMAGE": {
                        "container_id": identifier, "image_id": "sha256:" + "a" * 64,
                        "repository_digest": "registry.invalid/service@sha256:" + "b" * 64,
                    }
                    for service, identifier in ids.items()
                }}
                if scenario in ("recreated", "image-drift"):
                    ids["agent-service"] = "d" * 64

                def bounded(command, **_):
                    if command[1:4] == ["ps", "--all", "--no-trunc"]:
                        return "".join(f"{identifier} {journey.prefix}-{service}\n"
                                       for service, identifier in ids.items()
                                       if scenario != "missing" or service != "agent-service").encode()
                    if command[1] == "inspect":
                        return CONTROL.canonical([{
                            "Id": command[-1], "State": {"Running": False},
                            "Image": "sha256:" + ("e" if scenario == "image-drift" else "a") * 64,
                            "Config": {"Image": "registry.invalid/service@sha256:" + "b" * 64},
                        }])
                    return b""

                aggregate = {"unresolved_attempts": int(scenario == "unresolved"), "sealed": True}
                # Jump over the 300-second drain without real sleeping. Later
                # reads remain inside the independent 30-second stop deadline.
                tick = iter([0, 301, 302, 303])
                with mock.patch.object(journey, "diagnostic_owner_control"), mock.patch.object(
                    journey, "diagnostic_checkpoint", return_value=aggregate
                ), mock.patch.object(journey, "finalize_observability", side_effect=observability), mock.patch.object(
                    RUNNER.diagnostic, "bounded_command", side_effect=bounded
                ), mock.patch.object(RUNNER, "write_private"), mock.patch.object(
                    RUNNER.diagnostic, "sync_directory"
                ), mock.patch.object(RUNNER.time, "monotonic", side_effect=lambda: next(tick, 304)):
                    journey.diagnostic_terminal()
                self.assertEqual(journey.diagnostic_evidence_durable, scenario in ("complete", "recreated"))
                if scenario == "missing":
                    self.assertIn("diagnostic_payer_inventory_unproven", journey.diagnostic_failures)
                if scenario == "unresolved":
                    self.assertIn("diagnostic_unresolved_receipts", journey.diagnostic_failures)
                if scenario == "image-drift":
                    self.assertIn("diagnostic_payer_stop_unproven", journey.diagnostic_failures)
                if scenario == "metrics-drift":
                    self.assertIn("diagnostic_metrics_receipt_mismatch", journey.diagnostic_failures)

    def test_source_identity_reads_actual_committed_prompts_and_schema(self):
        sha = RUNNER.git(ROOT, "rev-parse", "HEAD")
        identities = CONTROL.source_identities(ROOT, sha, sha)
        self.assertEqual(identities["base"], identities["candidate"])
        self.assertEqual(identities["candidate"]["prompt_versions"], {
            "canon": RUNNER.EXPECTED_CANON_PROMPT, "branch": RUNNER.EXPECTED_BRANCH_PROMPT,
            "world": RUNNER.EXPECTED_WORLD_PROMPT,
        })
        self.assertIn("infra/postgres", identities["base"]["source_trees"])

    def test_artifact_preflight_rejects_metadata_only_changes_and_wrong_identity(self):
        journey = self.journey()
        registration = journey.diagnostic_registration
        mode = "valid"

        def command(argv, **_):
            if argv[0] == "git":
                if "diff" in argv:
                    return b"" if mode == "tooling-only" else b"services/agent-service/src/main.rs\n"
                return b""
            self.assertEqual(argv[:3], ["docker", "image", "inspect"])
            reference = argv[-1]
            label = "base" if reference.endswith("b" * 64) else "candidate"
            image_id = next(iter(registration.value[label + "_application_image_ids"].values()))
            layers = ["sha256:" + ("c" if label == "base" or mode == "metadata-only" else "d") * 64]
            return CONTROL.canonical([{
                "Id": "sha256:" + "0" * 64 if mode == "wrong-id" else image_id,
                "RepoDigests": [reference], "RootFS": {"Layers": layers},
            }])

        with mock.patch.object(CONTROL, "bounded_command", side_effect=command):
            CONTROL.verify_artifacts(registration, ROOT, journey.base_manifest, journey.candidate_manifest)
            for mode in ("metadata-only", "wrong-id", "tooling-only"):
                with self.subTest(mode=mode), self.assertRaises(CONTROL.DiagnosticFailure):
                    CONTROL.verify_artifacts(registration, ROOT, journey.base_manifest, journey.candidate_manifest)

    def test_diagnostic_cli_refuses_mixed_modes_before_config_or_execution(self):
        for arguments in (
            ["--diagnostic-registration", "/private/registration.json"],
            ["--diagnostic-registration-sha256", "a" * 64],
            ["--evidence-class", "Qualification"],
            ["--slice", "legacy-character"],
            ["--ledger", "/private/qualification-ledger.json"],
        ):
            if len(arguments) == 2 and arguments[0] not in (
                "--diagnostic-registration", "--diagnostic-registration-sha256"
            ):
                arguments += ["--diagnostic-registration", "/private/registration.json",
                              "--diagnostic-registration-sha256", "a" * 64]
            with mock.patch.object(sys, "argv", ["runner", *arguments]), mock.patch.object(
                RUNNER, "load_config"
            ) as config, self.assertRaises(RUNNER.QualificationFailure):
                RUNNER.main()
                config.assert_not_called()

    def test_diagnostic_cli_validates_registration_source_and_artifacts_before_start(self):
        journey = self.journey()
        registration = journey.diagnostic_registration
        candidate_values = dict(journey.candidate_manifest)
        for key in RUNNER.INFRASTRUCTURE_IMAGE_KEYS:
            candidate_values[key] = journey.base_manifest[key]
        self.candidate.write_text(
            "".join(f"{key}={value}\n" for key, value in candidate_values.items())
        )
        sha = "a" * 40
        arguments = [
            "runner", "--config", str(self.directory / "config.json"),
            "--output-dir", str(self.output), "--git-sha", sha,
            "--base-manifest", str(self.base), "--candidate-manifest", str(self.candidate),
            "--diagnostic-registration", str(self.registration_file),
            "--diagnostic-registration-sha256", registration.sha256,
        ]

        def fake_git(_root, *parts):
            if parts[:2] == ("rev-parse", "HEAD"):
                return sha
            if parts[:2] == ("status", "--porcelain=v1"):
                return ""
            return ""

        for failure, loader, source, artifacts in (
            ("registration", mock.DEFAULT,
             mock.DEFAULT, mock.DEFAULT),
            ("source", registration,
             RUNNER.diagnostic.DiagnosticFailure("synthetic_source_failure"), mock.DEFAULT),
            ("artifacts", registration, {},
             RUNNER.diagnostic.DiagnosticFailure("synthetic_artifact_failure")),
        ):
            with self.subTest(failure=failure), mock.patch.object(sys, "argv", arguments), \
                 mock.patch.object(RUNNER, "git", side_effect=fake_git), \
                 mock.patch.object(RUNNER, "load_config") as config, \
                 mock.patch.object(RUNNER, "Journey") as journey_type, \
                 mock.patch.object(RUNNER.diagnostic, "load_registration") as load_registration, \
                 mock.patch.object(RUNNER.diagnostic, "source_identities") as identities, \
                 mock.patch.object(RUNNER.diagnostic, "verify_artifacts") as verify:
                if failure == "registration":
                    load_registration.side_effect = RUNNER.diagnostic.DiagnosticFailure(
                        "synthetic_registration_failure"
                    )
                    identities.return_value = {}
                elif failure == "source":
                    load_registration.return_value = loader
                    identities.side_effect = source
                else:
                    load_registration.return_value = loader
                    identities.return_value = source
                    verify.side_effect = artifacts
                with self.assertRaises(RUNNER.diagnostic.DiagnosticFailure):
                    RUNNER.main()
                config.assert_not_called()
                journey_type.assert_not_called()

    def test_diagnostic_cli_valid_path_reaches_run_diagnostic_after_artifact_checks(self):
        journey = self.journey()
        registration = journey.diagnostic_registration
        candidate_values = dict(journey.candidate_manifest)
        for key in RUNNER.INFRASTRUCTURE_IMAGE_KEYS:
            candidate_values[key] = journey.base_manifest[key]
        self.candidate.write_text(
            "".join(f"{key}={value}\n" for key, value in candidate_values.items())
        )
        sha = "a" * 40
        arguments = [
            "runner", "--config", str(self.directory / "config.json"),
            "--output-dir", str(self.output), "--git-sha", sha,
            "--base-manifest", str(self.base), "--candidate-manifest", str(self.candidate),
            "--diagnostic-registration", str(self.registration_file),
            "--diagnostic-registration-sha256", registration.sha256,
        ]

        def fake_git(_root, *parts):
            if parts[:2] == ("rev-parse", "HEAD"):
                return sha
            if parts[:2] == ("status", "--porcelain=v1"):
                return ""
            return ""

        fake_journey = mock.Mock(diagnostic_ledger=object())
        with mock.patch.object(sys, "argv", arguments), \
             mock.patch.object(RUNNER, "git", side_effect=fake_git), \
             mock.patch.object(RUNNER.diagnostic, "source_identities", return_value={}), \
             mock.patch.object(RUNNER.diagnostic, "load_registration", return_value=registration), \
             mock.patch.object(RUNNER.diagnostic, "verify_artifacts"), \
             mock.patch.object(RUNNER, "Journey", return_value=fake_journey) as journey_type, \
             mock.patch.object(RUNNER, "run_diagnostic", return_value=0) as run_diagnostic:
            self.assertEqual(RUNNER.main(), 0)
        journey_type.assert_called_once()
        run_diagnostic.assert_called_once_with(fake_journey)

    def test_artifact_verification_rejects_non_ancestor_and_missing_repo_digest(self):
        journey = self.journey()
        registration = journey.diagnostic_registration
        base, candidate = journey.base_manifest, journey.candidate_manifest
        with mock.patch.object(
            CONTROL, "bounded_command",
            side_effect=CONTROL.DiagnosticFailure("synthetic_non_ancestor"),
        ) as command:
            with self.assertRaises(CONTROL.DiagnosticFailure):
                CONTROL.verify_artifacts(registration, ROOT, base, candidate)
            self.assertEqual(command.call_args.args[0][:3], ["git", "-C", str(ROOT)])

        def no_digest(argv, **_kwargs):
            if argv[0] == "git":
                return b"changed-source\n"
            return CONTROL.canonical([{
                "Id": registration.value["base_application_image_ids"]["GATEWAY_IMAGE"],
                "RepoDigests": [], "RootFS": {"Layers": ["sha256:" + "c" * 64]},
            }])

        with mock.patch.object(CONTROL, "bounded_command", side_effect=no_digest):
            with self.assertRaises(CONTROL.DiagnosticFailure) as rejected:
                CONTROL.verify_artifacts(registration, ROOT, base, candidate)
        self.assertEqual(rejected.exception.code, "diagnostic_artifact_identity_mismatch")

    def test_capability_probe_failure_in_any_version_or_payer_blocks_adoption(self):
        for failed_index in range(8):
            with self.subTest(failed_index=failed_index):
                journey = self.journey()
                journey.diagnostic_ledger.descriptor = object()
                probes = mock.Mock()
                calls = 0

                def probe(*_args):
                    nonlocal calls
                    calls += 1
                    if calls - 1 == failed_index:
                        raise RUNNER.diagnostic.DiagnosticFailure("synthetic_probe_failure")

                probes.side_effect = probe
                spec = mock.Mock()
                adapter = mock.Mock(probe=probes)
                with mock.patch.object(RUNNER, "docker_inventory_snapshot", return_value={
                    "containers": {}, "volumes": {}, "networks": {}
                }), mock.patch.object(
                    RUNNER, "run", side_effect=lambda command, **_: (
                        "fixture|linux|amd64" if command[:2] == ["docker", "version"]
                        else "fixture-compose" if command[:4] == ["docker", "compose", "version", "--short"]
                        else ""
                    )
                ), mock.patch.object(
                    journey, "validate_release_inputs"
                ), mock.patch.object(
                    journey, "release"
                ) as release, mock.patch.object(
                    RUNNER.importlib.util, "spec_from_file_location", return_value=spec
                ), mock.patch.object(
                    RUNNER.importlib.util, "module_from_spec", return_value=adapter
                ):
                    spec.loader.exec_module = mock.Mock()
                    with self.assertRaises(RUNNER.QualificationFailure) as rejected:
                        journey.preflight()
                self.assertEqual(rejected.exception.code, "diagnostic_artifact_capability_unproven")
                self.assertEqual(probes.call_count, failed_index + 1)
                release.assert_not_called()

    def test_control_seal_rejects_integer_boolean_and_still_cannot_retry(self):
        journey = self.journey()
        journey.internal_service_token = "synthetic-internal-control-value"
        response = {"binding": journey.diagnostic_registration.binding, "sealed": 1}
        with mock.patch.object(RUNNER.diagnostic, "bounded_command", return_value=CONTROL.canonical(response)) as invoke:
            for _ in range(2):
                with self.assertRaises(RUNNER.QualificationFailure):
                    journey.diagnostic_owner_control(seal=True)
            self.assertEqual(invoke.call_count, 1)

    def test_cleanup_without_required_ownership_never_invokes_docker(self):
        journey = self.journey()
        journey.inventory_captured = True
        journey.cleanup_required = False
        with mock.patch.object(RUNNER, "docker_inventory_snapshot") as inventory, mock.patch.object(
            RUNNER.diagnostic, "bounded_command"
        ) as command:
            journey.diagnostic_cleanup()
        inventory.assert_not_called()
        command.assert_not_called()

    def test_cleanup_without_durable_evidence_preserves_volume_but_removes_owned_runtime(self):
        journey = self.journey()
        journey.inventory_captured = True
        journey.cleanup_required = True
        journey.diagnostic_evidence_durable = False
        project = journey.project
        container_id = "a" * 64
        before = {
            "containers": {
                project + "-agent-service": {
                    "id": container_id,
                    "labels": {"com.docker.compose.project": project},
                }
            },
            "volumes": {
                project + "-postgres": {
                    "labels": {"com.docker.compose.project": project}
                }
            },
            "networks": {
                project + "-default": {
                    "id": "b" * 64,
                    "labels": {"com.docker.compose.project": project},
                }
            },
        }
        after = {"containers": {}, "volumes": before["volumes"], "networks": {}}
        with mock.patch.object(
            RUNNER, "docker_inventory_snapshot", side_effect=[before, after]
        ), mock.patch.object(
            RUNNER.diagnostic, "bounded_command", return_value=b""
        ) as command:
            journey.diagnostic_cleanup()
        commands = [call.args[0] for call in command.call_args_list]
        self.assertIn(["docker", "rm", "--force", container_id], commands)
        self.assertIn(["docker", "network", "rm", "b" * 64], commands)
        self.assertNotIn(["docker", "volume", "rm", project + "-postgres"], commands)
        self.assertEqual(journey.report["environment"]["isolated_cleanup_completed"], False)
        self.assertIn("diagnostic_cleanup_residue", journey.diagnostic_failures)

    def test_terminal_seal_failure_stops_payers_without_drain_or_retry(self):
        journey = self.journey()
        journey.inventory_captured = True
        journey.cleanup_required = True
        journey.active_release_process = None
        ps_output = f"{'c' * 64} {journey.project}-user-service\n".encode()

        def bounded(command, **_kwargs):
            if command[1:4] == ["ps", "--all", "--no-trunc"]:
                return ps_output
            return b""

        aggregate = {"charged": {"attempts": 0, "tokens": 0, "cost_micro_cny": 0},
                     "unresolved_attempts": 0, "sealed": True}
        with mock.patch.object(
            journey, "diagnostic_owner_control",
            side_effect=RUNNER.QualificationFailure("synthetic_seal_failure"),
        ) as seal, mock.patch.object(
            journey, "diagnostic_checkpoint", return_value=aggregate
        ), mock.patch.object(
            RUNNER.diagnostic, "bounded_command", side_effect=bounded
        ) as command, mock.patch.object(RUNNER.time, "sleep") as sleep:
            journey.diagnostic_terminal()
            journey.diagnostic_terminal()
        seal.assert_called_once_with(seal=True)
        sleep.assert_not_called()
        commands = [call.args[0] for call in command.call_args_list]
        self.assertIn(["docker", "stop", "--time", "0", "c" * 64], commands)
        self.assertTrue(journey.diagnostic_evidence_durable is False)


if __name__ == "__main__":
    unittest.main()
