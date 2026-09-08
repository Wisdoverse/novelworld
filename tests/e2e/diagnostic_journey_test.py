"""Offline checks; optional NW_H4_TEST_POSTGRES enables only a temporary-table PG probe."""
import copy
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import importlib.util
import json
import os
from pathlib import Path
import subprocess
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
BUDGET_SPEC = importlib.util.spec_from_file_location("journey_probe_budget", ROOT / "infra/docker/diagnostic_budget.py")
BUDGET = importlib.util.module_from_spec(BUDGET_SPEC)
BUDGET_SPEC.loader.exec_module(BUDGET)


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

    def test_v2_final_world_view_capture_is_exact_private_and_bounded(self):
        view = {"session": {"turn_number": 12}, "world_state": {"state": {"threads": {"x": {"origin": "player"}}}}}
        def make_journey(output, prospective_summary):
            journey = object.__new__(RUNNER.Journey)
            journey.prospective_summary = prospective_summary
            journey.output = output
            journey.private_report = {}
            return journey

        legacy = make_journey(self.directory / "legacy", False)
        legacy.output.mkdir(mode=0o700)
        with mock.patch.object(RUNNER.diagnostic, "canonical") as canonical:
            legacy.capture_final_world_view(view)
        canonical.assert_not_called()
        self.assertEqual(list(legacy.output.iterdir()), [])
        self.assertEqual(legacy.private_report, {})

        journey = make_journey(self.output, True)
        payload = CONTROL.canonical(view) + b"\n"
        journey.capture_final_world_view(view)
        captured = journey.output / "final-world-view.json"
        self.assertEqual(captured.read_bytes(), payload)
        self.assertEqual(captured.stat().st_mode & 0o777, 0o600)
        self.assertEqual(journey.private_report["final_world_view"], {"sha256": CONTROL.digest(payload), "byte_count": len(payload)})
        with self.assertRaises(FileExistsError):
            journey.capture_final_world_view(view)
        self.assertEqual(captured.read_bytes(), payload)
        self.assertEqual(journey.private_report["final_world_view"]["byte_count"], len(payload))

        exact = self.directory / "exact"
        exact.mkdir(mode=0o700)
        exact_journey = make_journey(exact, True)
        with mock.patch.object(RUNNER.diagnostic, "canonical", return_value=b"x" * (4 * 1024 * 1024 - 1)):
            exact_journey.capture_final_world_view(view)
        self.assertEqual((exact / "final-world-view.json").stat().st_size, 4 * 1024 * 1024)
        self.assertEqual(exact_journey.private_report["final_world_view"]["byte_count"], 4 * 1024 * 1024)

        oversized = self.directory / "oversized"
        oversized.mkdir(mode=0o700)
        oversized_journey = make_journey(oversized, True)
        with mock.patch.object(RUNNER.diagnostic, "canonical", return_value=b"x" * (4 * 1024 * 1024)):
            with self.assertRaises(RUNNER.QualificationFailure) as rejected:
                oversized_journey.capture_final_world_view(view)
        self.assertEqual(rejected.exception.args[0], "final_world_view_oversized")
        self.assertEqual(list(oversized.iterdir()), [])
        self.assertEqual(oversized_journey.private_report, {})

        failed = self.directory / "writer-failure"
        failed.mkdir(mode=0o700)
        failed_journey = make_journey(failed, True)
        with mock.patch.object(RUNNER, "write_private", side_effect=OSError("synthetic")) as writer:
            with self.assertRaises(OSError):
                failed_journey.capture_final_world_view(view)
        writer.assert_called_once()
        self.assertEqual(failed_journey.private_report, {})

    def test_diagnostic_response_log_retention_uses_existing_collection_path(self):
        def make_journey(name, registered=True):
            output = self.directory / name
            output.mkdir(mode=0o700)
            journey = object.__new__(RUNNER.Journey)
            journey.prefix = "synthetic"
            journey.output = output
            journey.diagnostic_registration = object() if registered else None
            journey.expected_model = "deepseek-v4-flash"
            journey.response_model_observations = []
            journey.response_model_log_offsets = {}
            journey.private_report = {}
            return journey

        journey = make_journey("response-logs")
        output = journey.output
        calls = []
        observed = json.dumps({"fields": {
            "message": "LLM response model observed", "provider": "deepseek",
            "configured_model": "deepseek-v4-flash", "response_model": "deepseek-v4-flash",
            "operation": "chat", "mode": "stream"
        }})

        def evidence(command):
            calls.append(command)
            return "a" * 64 if command[1] == "inspect" else observed

        journey.evidence_command = evidence
        journey.collect_response_models("base", services=("user-service",))
        artifact = output / "diagnostic-log-base-user-service.log"
        payload = observed.encode()
        self.assertEqual(artifact.read_bytes(), payload)
        self.assertEqual(artifact.stat().st_mode & 0o777, 0o600)
        self.assertEqual(journey.private_report["response_model_logs"][0], {
            "phase": "base", "service": "user-service", "container_id": "a" * 64,
            "artifact": artifact.name, "byte_count": len(payload),
            "sha256": CONTROL.digest(payload),
            "format": "utf8_decoded_stripped_stdout",
        })
        self.assertEqual(len(calls), 2)
        self.assertEqual(journey.response_model_observations[0]["response_model"], "deepseek-v4-flash")

        with mock.patch.object(RUNNER, "unseen_response_models",
                               side_effect=RUNNER.QualificationFailure("synthetic_parser_failure")):
            with self.assertRaises(RUNNER.QualificationFailure):
                journey.collect_response_models("candidate-agent-before-restart", services=("user-service",))
        parser_artifact = output / "diagnostic-log-candidate-agent-before-restart-user-service.log"
        self.assertTrue(parser_artifact.is_file())
        self.assertEqual(len(journey.private_report["response_model_logs"]), 2)

        journey.collect_response_models("candidate-final", services=("user-service",))
        self.assertTrue((output / "diagnostic-log-candidate-final-user-service.log").is_file())
        with self.assertRaises(RUNNER.QualificationFailure):
            journey.collect_response_models("base", services=("user-service",))
        self.assertEqual(artifact.read_bytes(), payload)

        ordinary = make_journey("ordinary", registered=False)
        ordinary.evidence_command = evidence
        ordinary.collect_response_models("base", services=("novel-service",))
        self.assertEqual(list(ordinary.output.iterdir()), [])
        self.assertNotIn("response_model_logs", ordinary.private_report)

        oversized = make_journey("oversized-logs")
        oversized.evidence_command = lambda command: "a" * 64 if command[1] == "inspect" else "x" * (4 * 1024 * 1024 + 1)
        with self.assertRaises(RUNNER.QualificationFailure):
            oversized.collect_response_models("base", services=("user-service",))
        self.assertEqual(list(oversized.output.iterdir()), [])
        self.assertNotIn("response_model_logs", oversized.private_report)

        failed = make_journey("writer-failure")
        failed.evidence_command = evidence
        with mock.patch.object(RUNNER, "write_private", side_effect=OSError("private")) as writer:
            with self.assertRaises(RUNNER.QualificationFailure):
                failed.collect_response_models("base", services=("user-service",))
        writer.assert_called_once()
        self.assertEqual(failed.private_report["response_model_collection_errors"], {
            "base": ["user-service:response_model_log_capture_failed"]
        })

        read_failed = make_journey("read-failure")
        def failing_evidence(command):
            if command[1] == "inspect":
                return "a" * 64
            raise RUNNER.QualificationFailure("docker_logs_failed")
        read_failed.evidence_command = failing_evidence
        with self.assertRaises(RUNNER.QualificationFailure):
            read_failed.collect_response_models("base", services=("user-service",))
        self.assertEqual(list(read_failed.output.iterdir()), [])

        fsync_failed = make_journey("fsync-failure")
        fsync_failed.evidence_command = evidence
        with mock.patch.object(RUNNER.os, "fsync", side_effect=OSError("sync")):
            with self.assertRaises(RUNNER.QualificationFailure):
                fsync_failed.collect_response_models("base", services=("user-service",))
        self.assertTrue((fsync_failed.output / "diagnostic-log-base-user-service.log").exists())
        self.assertNotIn("response_model_logs", fsync_failed.private_report)


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
        for scenario in ("complete", "missing", "unresolved", "recreated", "image-drift", "metrics-drift",
                         "parser-error", "interrupt", "response-log-error"):
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
                    if scenario in ("parser-error", "interrupt"):
                        raise ValueError("synthetic invalid metrics") if scenario == "parser-error" else KeyboardInterrupt()
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
                if scenario == "response-log-error":
                    journey.private_report["response_model_collection_errors"] = {
                        "base": ["user-service:response_model_log_capture_failed"]
                    }

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
                ) as commands, mock.patch.object(RUNNER, "write_private") as evidence, mock.patch.object(
                    RUNNER.diagnostic, "sync_directory"
                ), mock.patch.object(RUNNER.time, "monotonic", side_effect=lambda: next(tick, 304)):
                    journey.diagnostic_terminal()
                self.assertEqual(journey.diagnostic_evidence_durable, scenario in ("complete", "recreated"))
                if scenario == "response-log-error":
                    self.assertFalse(journey.diagnostic_evidence_durable)
                    self.assertTrue(journey.private_report["diagnostic_payers_stopped"])
                    self.assertTrue(any(call.args[0][1] == "stop" for call in commands.call_args_list))
                if scenario == "missing":
                    self.assertIn("diagnostic_payer_inventory_unproven", journey.diagnostic_failures)
                if scenario == "unresolved":
                    self.assertIn("diagnostic_unresolved_receipts", journey.diagnostic_failures)
                if scenario == "image-drift":
                    self.assertIn("diagnostic_payer_stop_unproven", journey.diagnostic_failures)
                if scenario == "metrics-drift":
                    self.assertIn("diagnostic_metrics_receipt_mismatch", journey.diagnostic_failures)
                if scenario in ("parser-error", "interrupt"):
                    self.assertIn("diagnostic_terminal_evidence_failed", journey.diagnostic_failures)
                    self.assertTrue(journey.private_report["diagnostic_payers_stopped"])
                    self.assertTrue(any(call.args[0][1] == "stop" for call in commands.call_args_list))
                    self.assertTrue(any(call.args[0].name == "pre-cleanup-private.json"
                                        for call in evidence.call_args_list))

    def test_terminal_cancels_release_process_before_seal_and_stops_payers_on_failure(self):
        journey = self.journey()
        journey.cleanup_required = True
        process = mock.Mock(pid=4242)
        events = []

        def wait(**kwargs):
            events.append(("release_wait", kwargs["timeout"]))
            raise subprocess.TimeoutExpired(["release"], kwargs["timeout"])

        process.wait.side_effect = wait
        journey.active_release_process = process
        registration = self.load()
        journey.diagnostic_registration = registration
        journey.diagnostic_last_snapshot = {
            "budget": {**registration.binding, **{
                key: item for key, item in self.value["limits"].items() if key != "profile"},
                "charged_attempts": 0, "charged_tokens": 0,
                "charged_cost_micro_cny": 0, "sealed": True},
            "receipts": [],
        }
        journey.report["llm_metrics"] = {"counter_totals": []}
        journey.private_report["release_images"] = {"base": {
            service.upper().replace("-", "_") + "_IMAGE": {
                "container_id": f"{index:064x}", "image_id": "sha256:" + "a" * 64,
                "repository_digest": "registry.invalid/service@sha256:" + "b" * 64,
            }
            for index, service in enumerate(RUNNER.SERVICE_PORTS, 1)
        }}
        ids = {service: f"{index:064x}" for index, service in enumerate(RUNNER.SERVICE_PORTS, 1)}

        def owner_control(**kwargs):
            if kwargs.get("seal"):
                events.append("seal")

        def bounded(command, **_kwargs):
            if command[1:4] == ["ps", "--all", "--no-trunc"]:
                events.append("payer_inventory")
                return "".join(f"{identifier} {journey.prefix}-{service}\n"
                               for service, identifier in ids.items()).encode()
            if command[1] == "stop":
                events.append("payer_stop")
                return b""
            if command[1:3] == ["ps", "--format"]:
                return b""
            if command[1] == "inspect":
                identifier = command[-1]
                return CONTROL.canonical([{
                    "Id": identifier, "State": {"Running": False},
                    "Image": "sha256:" + "a" * 64,
                    "Config": {"Image": "registry.invalid/service@sha256:" + "b" * 64},
                }])
            return b""

        def observability(*_args):
            journey.report["llm_metrics"] = {"counter_totals": []}

        with mock.patch.object(RUNNER.os, "killpg", side_effect=lambda pid, sig: events.append(("killpg", pid, sig))), \
             mock.patch.object(journey, "diagnostic_owner_control", side_effect=owner_control), \
             mock.patch.object(journey, "diagnostic_checkpoint", return_value={"unresolved_attempts": 0, "sealed": True}), \
             mock.patch.object(journey, "finalize_observability", side_effect=observability), \
             mock.patch.object(RUNNER.diagnostic, "bounded_command", side_effect=bounded):
            journey.diagnostic_terminal()

        self.assertEqual(events[:4], [("killpg", 4242, RUNNER.signal.SIGKILL),
                                      ("release_wait", 10), "seal", "payer_inventory"])
        self.assertIn("payer_stop", events)
        self.assertFalse(journey.diagnostic_evidence_durable)
        self.assertIn("diagnostic_release_stop_unproven", journey.diagnostic_failures)
        self.assertTrue(journey.private_report["diagnostic_payers_stopped"])
        self.assertEqual(journey.diagnostic_failures, ["diagnostic_release_stop_unproven"])
        self.assertTrue((self.output / "pre-cleanup-private.json").is_file())

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
                    return b"docs/ROADMAP.md\0" if mode == "tooling-only" else b"services/agent-service/src/main.rs\0"
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

    def test_build_input_mapping_and_real_git_path_boundaries(self):
        rust = CONTROL.APP_KEYS - {"FRONTEND_IMAGE"}
        for path, expected in (
            ("Cargo.lock", rust), ("Cargo.toml", rust),
            ("infra/docker/Dockerfile.rust-service", rust), (".dockerignore", rust),
            ("crates/llm-client/src/lib.rs", rust),
            ("services/agent-service/Cargo.toml", {"AGENT_SERVICE_IMAGE"}),
            ("services/novel-service/build.rs", {"NOVEL_SERVICE_IMAGE"}),
            ("gateway/src/main.rs", {"GATEWAY_IMAGE"}),
            (CONTROL.PROFILE_PATH.as_posix(), rust - {"GATEWAY_IMAGE"}),
            *(("frontend/" + name, {"FRONTEND_IMAGE"}) for name in (
                "package.json", "pnpm-lock.yaml", "pnpm-workspace.yaml", "Dockerfile",
                ".dockerignore", "vite.config.ts", "postcss.config.js", "tsconfig.json",
                "tsconfig.node.json", "index.html", "nginx-spa.conf", "public/icon.svg",
            )),
            ("docs/ROADMAP.md", set()), ("tests/e2e/diagnostic_journey.py", set()),
            ("tools/h1-eval/policy-v2.json", set()),
            ("crates/llm-client/README.md", set()),
            ("crates/llm-client/examples/diagnostic_budget_driver.rs", set()),
            ("frontend/eslint.config.js", set()),
        ):
            with self.subTest(path=path):
                self.assertEqual(CONTROL.affected_application_images([path]), expected)

        repo = self.directory / "git-history"
        repo.mkdir()

        def git(*args):
            return CONTROL.bounded_command(["git", "-C", str(repo), *args])

        git("init", "-q")
        git("config", "user.name", "Offline Test")
        git("config", "user.email", "test@example.invalid")
        (repo / "frontend").mkdir()
        package = repo / "frontend/package.json"
        package.write_text("{}\n")
        git("add", ".")
        git("-c", "commit.gpgsign=false", "commit", "-qm", "base")
        package.write_text('{"private":true}\n')
        (repo / "docs").mkdir()
        (repo / "docs/newline\nfrontend.txt").write_text("not an application input\n")

        def changed_images():
            paths = git("diff", "--no-renames", "--name-only", "-z", "HEAD", "--")
            return CONTROL.affected_application_images([
                CONTROL.os.fsdecode(path) for path in paths.split(b"\0") if path
            ])

        git("add", ".")
        self.assertEqual(changed_images(), {"FRONTEND_IMAGE"})
        git("-c", "commit.gpgsign=false", "commit", "-qm", "dependency input")
        package.rename(repo / "docs/moved-package.json")
        git("add", "-A")
        self.assertEqual(changed_images(), {"FRONTEND_IMAGE"})
        git("-c", "commit.gpgsign=false", "commit", "-qm", "remove application input")
        (repo / "docs/newline\nfrontend.txt").write_text("docs only\n")
        self.assertEqual(changed_images(), set())

    def test_build_input_requires_corresponding_image_content_change(self):
        journey = self.journey()
        registration = journey.diagnostic_registration
        changed_image = "FRONTEND_IMAGE"
        for manifest in (journey.base_manifest, journey.candidate_manifest):
            for key in CONTROL.APP_KEYS:
                manifest[key] = manifest[key].replace("@sha256:", "/" + key.lower() + "@sha256:")

        def command(argv, **_):
            if argv[0] == "git":
                return b"frontend/pnpm-lock.yaml\0" if "diff" in argv else b""
            reference = argv[-1]
            label = "base" if reference.endswith("b" * 64) else "candidate"
            manifest = journey.base_manifest if label == "base" else journey.candidate_manifest
            key = next(key for key in CONTROL.APP_KEYS if manifest[key] == reference)
            changed = label == "candidate" and key == changed_image
            return CONTROL.canonical([{
                "Id": registration.value[label + "_application_image_ids"][key],
                "RepoDigests": [reference],
                "RootFS": {"Layers": ["sha256:" + ("d" if changed else "c") * 64]},
            }])

        with mock.patch.object(CONTROL, "bounded_command", side_effect=command):
            CONTROL.verify_artifacts(registration, ROOT, journey.base_manifest, journey.candidate_manifest)
            for changed_image in ("GATEWAY_IMAGE", None):
                with self.subTest(changed_image=changed_image), self.assertRaises(CONTROL.DiagnosticFailure):
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
                return b"services/agent-service/src/main.rs\0" if "diff" in argv else b""
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
                        # Real helper validation/context/serializer, with only Docker I/O mocked.
                        with mock.patch.object(BUDGET, "bounded_output", side_effect=[
                            b"b" * 64, b"arbitrary-private-child-text", b""]), \
                             mock.patch.object(BUDGET.subprocess, "run") as cleanup:
                            cleanup.return_value.returncode = 0
                            BUDGET.probe("registry/service@sha256:" + "a" * 64,
                                         journey.project, {})

                probes.side_effect = probe
                spec = mock.Mock()
                adapter = mock.Mock(probe=probes, probe_evidence=BUDGET.probe_evidence)
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
                evidence = journey.private_report["diagnostic_probe_evidence"][0]
                self.assertEqual(evidence["primary"]["reason"], "invalid_capability")
                self.assertEqual(evidence["container_id"], "b" * 64)
                self.assertTrue(evidence["name"].startswith(journey.project + "-budget-probe-"))
                private = json.dumps(evidence, allow_nan=False)
                public = json.dumps(journey.public_report(), allow_nan=False)
                self.assertNotIn("arbitrary-private-child-text", private)
                for sensitive in ("diagnostic_probe_evidence", evidence["name"],
                                  evidence["container_id"], "arbitrary-private-child-text"):
                    self.assertNotIn(sensitive, public)

    def test_batched_inventory_runner_preserves_projection_and_rejects_bad_batches(self):
        project = "nwq-abcdef1234"
        container_names = [f"{project}-service-{index:02d}" for index in range(33)]
        volume_names = [f"{project}-volume-{index}" for index in range(2)]
        network_names = ["--network", "network with space", "网络"]
        calls = []

        def runner(command):
            calls.append(command)
            if command[:4] == ["docker", "ps", "-a", "--format"]:
                return "\n".join(reversed(container_names))
            if command[:4] == ["docker", "volume", "ls", "--format"]:
                return "\n".join(reversed(volume_names))
            if command[:4] == ["docker", "network", "ls", "--format"]:
                return "\n".join(reversed(network_names))
            kind = command[1]
            names = command[command.index("--") + 1:]
            values = []
            for name in names:
                if kind == "container":
                    labels = {"com.docker.compose.project": project} if name.endswith("00") else {}
                    values.append({
                        "Id": "a" * 64, "Name": "/" + name, "Image": "sha256:" + "b" * 64,
                        "State": {"Status": "running", "StartedAt": "now"}, "RestartCount": 2,
                        "Config": {"Image": "service@sha256:" + "c" * 64, "Labels": labels},
                        "HostConfig": {"RestartPolicy": {"Name": "unless-stopped"}},
                        "NetworkSettings": {"Networks": {"z-net": {}, "a-net": {}}},
                    })
                elif kind == "volume":
                    values.append({"Name": name, "Driver": "local", "Labels": {
                        "com.docker.compose.project": project}, "Options": {}, "Scope": "local"})
                else:
                    values.append({"Name": name, "Id": "d" * 64, "Driver": "bridge",
                                   "Scope": "local", "Internal": True, "Attachable": False,
                                   "Ingress": False, "IPAM": {},
                                   "Labels": {"com.docker.compose.project": project},
                                   "Containers": {"e" * 64: {"Name": "/owner", "EndpointID": "f" * 64}}})
            return "\n".join(json.dumps(value) for value in values)

        snapshot = RUNNER.docker_inventory_snapshot(runner=runner)
        self.assertEqual(list(snapshot["containers"]), sorted(container_names))
        self.assertEqual(snapshot["containers"][container_names[0]]["networks"], ["a-net", "z-net"])
        self.assertEqual(snapshot["containers"][container_names[0]]["labels"], {
            "com.docker.compose.project": project
        })
        self.assertEqual(snapshot["networks"][network_names[0]]["containers"], {
            "e" * 64: {"name": "/owner", "endpoint_id": "f" * 64}
        })
        self.assertEqual(
            RUNNER.attempt_resources(snapshot, project, project),
            sorted(
                ["containers:" + name for name in container_names]
                + ["networks:" + name for name in network_names]
                + ["volumes:" + name for name in volume_names]
            ),
        )
        inspect_commands = [command for command in calls if "inspect" in command]
        self.assertEqual(len(inspect_commands), 4)
        self.assertTrue(all("Config.Env" not in command for command in inspect_commands))
        self.assertTrue(all("--format" in command for command in inspect_commands))
        self.assertTrue(all(command[command.index("--")] == "--" for command in inspect_commands))
        self.assertEqual(len([command for command in inspect_commands if command[1] == "container"]), 2)

        def invalid_runner(command):
            if command[:4] == ["docker", "ps", "-a", "--format"]:
                return "\n".join(container_names)
            if command[1] == "container":
                names = command[command.index("--") + 1:]
                return json.dumps({"Name": "/wrong"}) + "\n" * len(names)
            return ""

        with self.assertRaises(RUNNER.QualificationFailure) as incomplete:
            RUNNER.docker_inventory_snapshot(runner=invalid_runner)
        self.assertIn(incomplete.exception.code, {"docker_inventory_incomplete", "docker_inspect_invalid"})

        def reordered_runner(command):
            if command[:4] == ["docker", "ps", "-a", "--format"]:
                return "\n".join(container_names)
            if command[1] == "container":
                names = command[command.index("--") + 1:]
                return "\n".join(json.dumps({"Name": "/" + name}) for name in reversed(names))
            return ""

        with self.assertRaises(RUNNER.QualificationFailure) as reordered:
            RUNNER.docker_inventory_snapshot(runner=reordered_runner)
        self.assertEqual(reordered.exception.code, "docker_inventory_incomplete")

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
        foreign_id = "c" * 64
        foreign_name = project + "-foreign"
        before = {
            "containers": {
                project + "-agent-service": {
                    "id": container_id,
                    "labels": {"com.docker.compose.project": project},
                },
                foreign_name: {
                    "id": foreign_id,
                    "labels": {"com.docker.compose.project": "other-project"},
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
        after = {"containers": {foreign_name: before["containers"][foreign_name]},
                 "volumes": before["volumes"], "networks": {}}
        with mock.patch.object(
            RUNNER, "docker_inventory_snapshot", side_effect=[before, after]
        ), mock.patch.object(
            RUNNER.diagnostic, "bounded_command", return_value=b""
        ) as command:
            journey.diagnostic_cleanup()
        commands = [call.args[0] for call in command.call_args_list]
        self.assertIn(["docker", "rm", "--force", "--volumes", container_id], commands)
        self.assertNotIn(["docker", "rm", "--force", "--volumes", foreign_id], commands)
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

    def test_v2_registered_subnet_shape_and_presecret_refusal(self):
        value = copy.deepcopy(self.value)
        with self.assertRaises(CONTROL.DiagnosticFailure):
            self.load({**value, "network_subnet": None})  # V1 exact shape.
        value.update(schema=CONTROL.REGISTRATION_SCHEMA_V2, network_subnet=None,
                     product_fixture_sha256=CONTROL.digest((ROOT / CONTROL.product_fixture(
                         CONTROL.REGISTRATION_SCHEMA_V2)).read_bytes()))
        self.load(value)
        for selected in (True, "", "10.2.3.1/28", " 10.2.3.0/28", "10.2.3.0/24",
                         "8.8.8.0/28", "127.0.0.0/28", "2001:db8::/28", "10.02.3.0/28"):
            with self.subTest(subnet=selected), self.assertRaises(CONTROL.DiagnosticFailure):
                self.load({**value, "network_subnet": selected})
        for mutation in ({key: val for key, val in value.items() if key != "network_subnet"},
                         {**value, "overlay_path": "/arbitrary"}):
            with self.assertRaises(CONTROL.DiagnosticFailure): self.load(mutation)
        approved = CONTROL.digest(CONTROL.canonical(value))
        value["network_subnet"] = "10.2.3.0/28"
        with self.assertRaises(CONTROL.DiagnosticFailure): self.load(value, approved)
        registration = self.load(value)
        with mock.patch.object(RUNNER, "load_release_manifest", return_value={}), \
                mock.patch.object(RUNNER, "load_config") as config, \
                mock.patch.object(RUNNER.diagnostic.network, "preflight",
                                  side_effect=CONTROL.network.NetworkFailure("overlap")), \
                mock.patch.object(CONTROL.DiagnosticLedger, "start") as started:
            with self.assertRaises(CONTROL.network.NetworkFailure):
                RUNNER.Journey(ROOT, self.directory / "config", self.output, "a" * 40,
                               self.base, self.candidate, None, None, "bash", "Diagnostic", "core",
                               diagnostic_registration=registration)
            config.assert_not_called()
            started.assert_not_called()
        self.assertNotIn("RELEASE_QUALIFICATION_SUBNET", RUNNER.qualification_environment(
            ROOT, {"RELEASE_QUALIFICATION_SUBNET": "10.9.9.0/28"}))

    def network_fixture(self):
        project, selected = "nwq-abcdef1234", "10.2.3.0/28"
        item = {"Id": "a" * 64, "Name": project + "_novel-net", "Driver": "bridge", "Scope": "local",
                "Internal": False, "EnableIPv6": False, "Options": {},
                "Labels": {"com.docker.compose.project": project, "com.docker.compose.network": "novel-net"},
                "IPAM": {"Config": [{"Subnet": selected, "Gateway": "10.2.3.1"}]}}
        return project, selected, item

    def test_network_topology_all_tables_and_local_engine_fail_closed(self):
        network = CONTROL.network
        _, selected, item = self.network_fixture()
        network.topology(selected, [], [{"dst": "default", "table": "main"}])
        for route in ({"dst": "10.2.3.1", "table": "local"},
                      {"dst": "10.2.0.0/16", "table": 100}, {"dst": "bad"}, {}):
            with self.assertRaises(network.NetworkFailure): network.topology(selected, [], [route])
        with self.assertRaises(network.NetworkFailure): network.topology(selected, [item], [])
        for malformed in ({}, {"IPAM": {}}, {"IPAM": {"Config": [{}]}}):
            with self.assertRaises(network.NetworkFailure): network.topology(selected, [malformed], [])
        engine = {"os": "linux", "name": os.uname().nodename, "kernel": os.uname().release,
                  "distribution": "Linux", "security": []}
        outputs = [b'[{"Name":"default","Endpoints":{"docker":{"Host":"unix:///var/run/docker.sock"}}}]',
                   json.dumps(engine).encode(), b"", b'[{"dst":"default"}]']
        with mock.patch.dict(os.environ, {}, clear=True), \
                mock.patch.object(network, "read_command", side_effect=outputs) as command:
            with self.assertRaises(network.NetworkFailure): network.inventory()  # no local bridge proof
            self.assertFalse(any(call.args[0][:3] == ["docker", "network", "inspect"]
                                 for call in command.call_args_list))
            self.assertEqual(command.call_args.args[0], ["ip", "-j", "-4", "route", "show", "table", "all"])
        for host in ("ssh://elsewhere", "tcp://127.0.0.1:2375"):
            with mock.patch.dict(os.environ, {"DOCKER_HOST": host}, clear=True), \
                    mock.patch.object(network, "read_command", return_value=outputs[0]):
                with self.assertRaises(network.NetworkFailure): network.inventory()
        for bad in (b"{}", b"[", b'[{"Endpoints":{}}]', b'[{"x":1,"x":2}]'):
            with mock.patch.dict(os.environ, {}, clear=True), \
                    mock.patch.object(network, "read_command", return_value=bad):
                with self.assertRaises(network.NetworkFailure): network.inventory()
        bridge = {"Id": "c" * 64, "Name": "bridge", "Driver": "bridge", "Scope": "local",
                  "Options": {"com.docker.network.bridge.name": "docker0"},
                  "IPAM": {"Config": [{"Subnet": "172.17.0.0/16", "Gateway": "172.17.0.1"}]}}
        route = {"dev": "docker0", "dst": "172.17.0.0/16", "prefsrc": "172.17.0.1"}
        valid = [outputs[0], outputs[1], b"c" * 64, json.dumps([bridge]).encode(), json.dumps([route]).encode()]
        with mock.patch.dict(os.environ, {}, clear=True), mock.patch.object(network, "read_command", side_effect=valid):
            self.assertEqual(network.inventory(), ([bridge], [route]))
        for changed in ({**engine, "name": "other-host"}, {**engine, "kernel": "other-kernel"},
                        {**engine, "distribution": "Docker Desktop"}, {**engine, "security": ["name=rootless"]}):
            with mock.patch.dict(os.environ, {}, clear=True), \
                    mock.patch.object(network, "read_command", side_effect=[outputs[0], json.dumps(changed).encode()]):
                with self.assertRaises(network.NetworkFailure): network.inventory()
        for changed in ({**route, "dev": "other"}, {**route, "prefsrc": "172.17.0.2"},
                        {**route, "dst": "172.18.0.0/16"}):
            with mock.patch.dict(os.environ, {}, clear=True), \
                    mock.patch.object(network, "read_command", side_effect=valid[:-1] + [json.dumps([changed]).encode()]):
                with self.assertRaises(network.NetworkFailure): network.inventory()
        with mock.patch.object(network.subprocess, "run", side_effect=subprocess.TimeoutExpired("docker", 5)):
            with self.assertRaises(network.NetworkFailure): network.inventory()

    def test_network_overlay_and_one_creation_identity_across_generations(self):
        network = CONTROL.network
        project, selected, item = self.network_fixture()
        state = self.directory / "network-state"
        state.mkdir(mode=0o700)
        path = Path(network.guard("overlay", selected, state, project, ROOT))
        self.assertEqual(path.read_bytes(), network.overlay_bytes(selected))
        self.assertEqual(path.stat().st_mode & 0o777, 0o600)
        with mock.patch.object(network, "inventory", return_value=([], [])):
            network.guard("check", selected, state, project, ROOT)
            network.guard("before", selected, state, project, ROOT)
            with self.assertRaises(network.NetworkFailure): network.guard("before", selected, state, project, ROOT)
        own_routes = [{"dst": selected, "dev": "br-" + item["Id"][:12]},
                      {"dst": "10.2.3.1", "table": "local", "dev": "br-" + item["Id"][:12]}]
        with mock.patch.object(network, "inventory", return_value=([item], own_routes)):
            network.guard("after", selected, state, project, ROOT)
            for action in ("check", "before", "after", "before", "after"):
                network.guard(action, selected, state, project, ROOT)
        for mutation in ([], [{**item, "Id": "b" * 64}], [{**item, "Labels": {}}],
                         [{**item, "IPAM": {"Config": [{"Subnet": "10.2.4.0/28"}]}}]):
            with mock.patch.object(network, "inventory", return_value=(mutation, [])):
                with self.assertRaises(network.NetworkFailure): network.guard("before", selected, state, project, ROOT)
        with mock.patch.object(network, "inventory", return_value=([item], own_routes + [{"dst": selected, "dev": "other"}])):
            with self.assertRaises(network.NetworkFailure): network.guard("check", selected, state, project, ROOT)
        path.write_bytes(b"networks: changed")
        with self.assertRaises(network.NetworkFailure): network.guard("overlay", selected, state, project, ROOT)
        path.unlink()
        path.symlink_to(self.base)
        with self.assertRaises(network.NetworkFailure): network.guard("overlay", selected, state, project, ROOT)
        fresh = self.directory / "network-fresh"
        fresh.mkdir(mode=0o700)
        with mock.patch.object(network, "inventory", return_value=([item], own_routes)):
            with self.assertRaises(network.NetworkFailure): network.guard("check", selected, fresh, project, ROOT)

    def test_journey_fixed_network_overlay_and_null_original_compose_argv(self):
        journey = self.journey()
        journey.runtime_root = self.directory / "repo"
        journey.release_state = self.directory / "network-state"
        journey.release_state.mkdir(mode=0o700)
        journey.release_tool = self.directory / "release.sh"
        journey.compose_env = {"RELEASE_QUALIFICATION_SUBNET": ""}
        for selected in (None, "10.2.3.0/28"):
            journey.network_subnet = selected
            with mock.patch.object(RUNNER, "run", return_value="") as run, \
                    mock.patch.object(RUNNER.diagnostic.network, "inventory", return_value=([], [])):
                journey.compose("config", "--quiet")
            argv = run.call_args.args[0]
            files = [argv[index + 1] for index, value in enumerate(argv) if value == "-f"]
            self.assertEqual(files, [str(journey.runtime_root / "docker-compose.yml")] +
                             ([str(journey.release_state / "qualification-network.yml")] if selected else []))

    def test_release_network_guard_propagates_inside_conditional_callers(self):
        source = (ROOT / "infra/docker/release.sh").read_text()
        function = source[source.index("compose() (\n"):source.index("\nrequire_empty_qualification_project()")]
        bin_dir = self.directory / "bin"
        bin_dir.mkdir()
        docker = bin_dir / "docker"
        docker.write_text('#!/bin/sh\nprintf "mutation\\n" >> "$TRACE"\n')
        docker.chmod(0o700)
        for failure in ("overlay", "before", "after"):
            trace = self.directory / (failure + ".trace")
            script = """set -euo pipefail
cache_mode=postgres; cache_redis_password=; cache_redis_url=memory://
diagnostic_enabled=false; qualification_scope=true; qualification_subnet=10.2.3.0/28
container_prefix=nwq-abcdef1234; http_bind=127.0.0.1; http_port=18080
repo_root=/synthetic; secrets_file=/synthetic/env; active_manifest=/synthetic/manifest
compose_project_args=(); compose_deadline_args=(); compose_profile_args=(); network_overlay_args=()
die() { return 1; }
network_guard() {
  if [[ "$1" == "$FAILURE" ]]; then return 23; fi
  if [[ "$1" == overlay ]]; then printf '%s\n' /private/qualification-network.yml; fi
}
""" + function + "\nif ! compose run --rm --no-deps user-service; then exit 0; else exit 9; fi\n"
            result = subprocess.run(["bash", "-c", script], capture_output=True, timeout=5,
                env={**os.environ, "PATH": str(bin_dir) + os.pathsep + os.environ["PATH"],
                     "TRACE": str(trace), "FAILURE": failure})
            self.assertEqual(result.returncode, 0, failure)
            self.assertEqual(trace.exists(), failure == "after")
            if trace.exists(): self.assertEqual(trace.read_text().splitlines(), ["mutation"])

    def test_v2_fixture_selection_and_frozen_v1_bytes(self):
        self.assertEqual(CONTROL.digest((ROOT / "tests/e2e/fixtures/h4-journey-v1.json").read_bytes()),
                         "e01d35e1bdad197876aefce1ae32f43dc93be185f9987c247ef2525c8ed0f9a9")
        self.assertEqual(CONTROL.digest((ROOT / CONTROL.PROFILE_PATH).read_bytes()),
                         "ce1b6a7ceade2a425abacb3c791fa410fbc67d9f5b011b6ff47ce602d018e5ef")
        value = copy.deepcopy(self.value)
        value["schema"] = CONTROL.REGISTRATION_SCHEMA_V2
        value["network_subnet"] = None
        path = ROOT / CONTROL.product_fixture(value["schema"])
        value["product_fixture_sha256"] = CONTROL.digest(path.read_bytes())
        registration = self.load(value)
        fixture = RUNNER.load_product_input(path, prospective=True)
        old = RUNNER.load_product_input(ROOT / CONTROL.product_fixture(CONTROL.REGISTRATION_SCHEMA))
        self.assertEqual(len(fixture["post_adoption_chats"]), 5)
        for key in old.keys() - {"manifest_version", "case_id"}:
            self.assertEqual(fixture[key], old[key])
        self.assertEqual(registration.binding["profile_sha256"], self.value["profile_sha256"])
        for schema, digest in [("vision-journey-registration-v3", value["product_fixture_sha256"]),
                               (CONTROL.REGISTRATION_SCHEMA_V2, self.value["product_fixture_sha256"])]:
            invalid = {**value, "schema": schema, "product_fixture_sha256": digest}
            with self.assertRaises(CONTROL.DiagnosticFailure):
                self.load(invalid)

    def test_v2_malformed_fixture_and_slice_reject_before_config(self):
        value = {**self.value, "schema": CONTROL.REGISTRATION_SCHEMA_V2}
        registration = CONTROL.Registration(CONTROL.canonical(value), self.profile)
        fixture = json.loads((ROOT / CONTROL.product_fixture(value["schema"])).read_bytes())
        path = ROOT / CONTROL.product_fixture(value["schema"])
        original_read = Path.read_text
        for mutation in ("missing", "sixth", "version", "case", "v1"):
            invalid = copy.deepcopy(fixture)
            if mutation == "missing": invalid.pop("post_adoption_chats")
            elif mutation == "sixth": invalid["post_adoption_chats"].append("extra")
            elif mutation == "version": invalid["manifest_version"] = "h4-product-input-v1"
            elif mutation == "case": invalid["case_id"] = "zh-self-world"
            else: invalid = json.loads((ROOT / "tests/e2e/fixtures/h4-journey-v1.json").read_bytes())
            def read(selected, *args, **kwargs):
                return json.dumps(invalid) if selected == path else original_read(selected, *args, **kwargs)
            with self.subTest(mutation=mutation), mock.patch.object(Path, "read_text", read), \
                    mock.patch.object(RUNNER, "load_release_manifest", return_value={}), \
                    mock.patch.object(RUNNER, "load_config") as config, \
                    mock.patch.object(RUNNER, "request_bytes") as provider:
                with self.assertRaises(RUNNER.QualificationFailure):
                    RUNNER.Journey(ROOT, self.directory / "config", self.output, "a" * 40,
                                   self.base, self.candidate, None, None, "bash", "Diagnostic",
                                   diagnostic_registration=registration)
                config.assert_not_called()
                provider.assert_not_called()
        for evidence, slice_name, cohort in [("Qualification", "core", None),
                                             ("Diagnostic", "legacy-character", None),
                                             ("Diagnostic", "core", self.directory / "cohort")]:
            with mock.patch.object(RUNNER, "load_release_manifest", return_value={}), \
                    mock.patch.object(RUNNER, "load_cohort_manifest", return_value={}), \
                    mock.patch.object(RUNNER, "load_config") as config:
                with self.assertRaises(RUNNER.QualificationFailure):
                    RUNNER.Journey(ROOT, self.directory / "config", self.output, "a" * 40,
                                   self.base, self.candidate, cohort, self.directory / "ledger" if cohort else None,
                                   "bash", evidence, slice_name, diagnostic_registration=registration)
                config.assert_not_called()

    def summary_fixture(self):
        # These are projected snapshot DTOs, not raw chat_messages table rows.
        scope = {key: str(uuid.uuid4()) for key in ("user_id", "novel_id", "character_id")}
        legacy, candidate = ([str(uuid.uuid4()) for _ in range(count)] for count in (7, 11))
        turns, messages = [], []
        memory_id = str(uuid.uuid4())
        for index, turn_id in enumerate(legacy + candidate):
            sequence = index - 6 if index >= 7 else None
            row = {**scope, **CONTROL.SUMMARY_DEFAULTS, "id": turn_id, "status": "completed",
                   "reader_identity_type": "self", "reader_character_id": None,
                   "reader_identity": "reader", "chapter_context": 4,
                   "persona_source_chapter_high_water": 4, "summary_sequence": sequence}
            if sequence == 10:
                row.update(summary_state="saved", summary_memory_id=memory_id, summary_claim_attempt=2)
            turns.append(row)
            for role in ("user", "character"):
                messages.append({**scope, "id": str(uuid.uuid4()), "turn_id": turn_id, "role": role,
                                 "content": f"source {index} {role}", "reader_identity": "reader",
                                 "chapter_context": 4, "persona_source_chapter_high_water": 4})
        memory = {**scope, "id": memory_id, "layer": "mid", "importance": 6, "embedding": None,
                  "content": "bounded summary", "chapter_number": 4, "persona_source_chapter_high_water": 4}
        def snapshot(count=10, state="saved"):
            ids = set(legacy + candidate[:count])
            result = {"turns": [copy.deepcopy(row) for row in turns if row["id"] in ids],
                      "messages": [copy.deepcopy(row) for row in messages if row["turn_id"] in ids],
                      "mid": [copy.deepcopy(memory)] if count >= 10 and state == "saved" else []}
            if count >= 10:
                anchor = result["turns"][16]
                anchor["summary_state"] = state
                if state == "pending": anchor["summary_next_attempt_at"] = "2030-01-01T00:00:00Z"
                if state in ("failed", "unknown"): anchor["summary_failure_code"] = "dispatch_unknown"
            return result
        return scope, legacy, candidate, snapshot

    def test_v2_cross_schema_only_normalization_and_exact_window_negatives(self):
        scope, legacy, candidate, snapshot = self.summary_fixture()
        before = {"chat_turns": [{"id": item, "status": "completed"} for item in legacy]}
        after = copy.deepcopy(before)
        for row in after["chat_turns"]: row.update(CONTROL.SUMMARY_DEFAULTS)
        self.assertEqual(CONTROL.upgrade_authority(json.dumps(before)),
                         CONTROL.upgrade_authority(json.dumps(after)))
        self.assertNotEqual(CONTROL.canonical(before), CONTROL.canonical(after))
        after["chat_turns"][0]["status"] = "failed"
        self.assertNotEqual(CONTROL.upgrade_authority(json.dumps(before)),
                            CONTROL.upgrade_authority(json.dumps(after)))
        self.assertIsNone(CONTROL.summary_window(snapshot(0), legacy, [], scope))
        saved = CONTROL.summary_window(snapshot(), legacy, candidate[:10], scope)
        self.assertEqual(saved["source_turn_ids"], candidate[:10])
        self.assertEqual(len(saved["source_messages"]), 20)
        mutations = [lambda data: data["turns"][0].update(summary_sequence=1),
                     lambda data: data["turns"][0].pop("summary_state"),
                     lambda data: data["turns"][7].update(summary_sequence=2),
                     lambda data: data["turns"][7].update(reader_identity_type="character"),
                     lambda data: data["messages"][14].update(turn_id=candidate[1]),
                     lambda data: data["messages"][14].update(role="character"),
                     lambda data: data["messages"][14].update(persona_source_chapter_high_water=None),
                     lambda data: data["mid"][0].update(id=str(uuid.uuid4())),
                     lambda data: data["mid"][0].update(chapter_number=3),
                     lambda data: data["mid"][0].update(persona_source_chapter_high_water=3),
                     lambda data: data["mid"].append(copy.deepcopy(data["mid"][0])),
                     lambda data: data["turns"][15].update(summary_state="saved"),
                     lambda data: data["turns"][16].update(summary_claim_attempt=0)]
        for mutate in mutations:
            data = snapshot(); mutate(data)
            with self.assertRaises(CONTROL.DiagnosticFailure):
                CONTROL.summary_window(data, legacy, candidate[:10], scope)
        wrong = candidate[:10].copy(); wrong[0] = str(uuid.uuid4())
        with self.assertRaises(CONTROL.DiagnosticFailure):
            CONTROL.summary_window(snapshot(), legacy, wrong, scope)

    def test_v2_actual_extra_chat_branch_and_restart_use_one_window(self):
        scope, legacy, candidate, snapshot = self.summary_fixture()
        journey = object.__new__(RUNNER.Journey)
        journey.root, journey.expected_model = ROOT, CONTROL.MODEL
        journey.summary_legacy_ids, journey.summary_candidate_ids = legacy, candidate[:5].copy()
        journey.private_report, journey.report = {}, {"journey": {}}
        journey.product_input = RUNNER.load_product_input(
            ROOT / CONTROL.product_fixture(CONTROL.REGISTRATION_SCHEMA_V2), prospective=True)
        reads, dispatched, resumed = [], [], [False]
        def observe(*args, **kwargs):
            reads.append(len(journey.summary_candidate_ids))
            return snapshot(len(journey.summary_candidate_ids))
        def metrics(*args):
            logical = int(len(journey.summary_candidate_ids) >= 10 and not resumed[0])
            dispatched.append(logical)
            return (f'novelworld_llm_requests_started_total{{service="agent-service",contract="llm-observability-v1",provider="deepseek",'
                    f'model="{CONTROL.MODEL}",operation="memory_summary",mode="sync"}} {logical}\n'
                    f'novelworld_llm_attempts_total{{service="agent-service",contract="llm-observability-v1",provider="deepseek",'
                    f'model="{CONTROL.MODEL}",operation="memory_summary",mode="sync",status="success"}} {logical * 2}\n').encode()
        with mock.patch.object(journey, "authority_snapshot", return_value=json.dumps(
                    {"chat_turns": [], "chat_messages": [], "world_state": {"turn": 11}, "journal": list(range(11))})), \
                mock.patch.object(journey, "summary_snapshot", side_effect=observe), \
                mock.patch.object(journey, "service_metrics", side_effect=metrics), \
                mock.patch.object(journey, "internal_character_context", return_value={"world_revision": [0] * 32}), \
                mock.patch.object(journey, "assert_chat_revision") as revision, \
                mock.patch.object(journey, "chat", side_effect=lambda *args: {"turn_id": candidate[len(journey.summary_candidate_ids)]}) as chat:
            journey.complete_prospective_summary("synthetic-token", **scope)
            self.assertEqual(chat.call_count, 5)
            self.assertEqual(revision.call_count, 5)
            self.assertEqual(reads, [5, 6, 7, 8, 9, 10])
            self.assertEqual(len(snapshot(10)["messages"]), 34)
            self.assertEqual(dispatched, [0, 0, 0, 0, 0, 1])
            resumed[0] = True
            result = journey.chat("synthetic-token", scope["novel_id"], scope["character_id"], "resume")
            journey.verify_summary_restart(**scope, resumed_turn_id=result["turn_id"], selected=1)
            self.assertEqual(len(snapshot(11)["messages"]), 36)
            self.assertEqual(len(journey.summary_legacy_ids + journey.summary_candidate_ids), 18)
            self.assertEqual(len(journey.product_input["world_actions"]), 12)
            self.assertEqual(dispatched[-1], 0)
            self.assertEqual(journey.report["journey"]["summary_logical_calls"], 1)

    def test_v2_terminal_and_timeout_do_not_make_up_chats(self):
        for state in ("failed", "unknown", "pending"):
            scope, legacy, candidate, snapshot = self.summary_fixture()
            journey = object.__new__(RUNNER.Journey)
            journey.summary_legacy_ids, journey.summary_candidate_ids = legacy, candidate[:5].copy()
            journey.product_input = {"post_adoption_chats": ["fixed"] * 5}
            clock = [0.0]
            with mock.patch.object(journey, "authority_snapshot", return_value=json.dumps(
                        {"chat_turns": [], "chat_messages": [], "world_state": {"turn": 11}})), \
                    mock.patch.object(journey, "summary_snapshot", side_effect=lambda *a, **k: snapshot(len(journey.summary_candidate_ids), state)), \
                    mock.patch.object(journey, "require_summary_calls"), \
                    mock.patch.object(journey, "internal_character_context", return_value={"world_revision": [0] * 32}), \
                    mock.patch.object(journey, "assert_chat_revision"), \
                    mock.patch.object(journey, "chat", side_effect=lambda *a: {"turn_id": candidate[len(journey.summary_candidate_ids)]}) as chat, \
                    mock.patch.object(RUNNER.time, "monotonic", side_effect=lambda: clock[0]), \
                    mock.patch.object(RUNNER.time, "sleep", side_effect=lambda _: clock.__setitem__(0, 400.0)):
                with self.assertRaisesRegex((RUNNER.QualificationFailure, RUNNER.diagnostic.DiagnosticFailure),
                                            "summary_window_(terminal|timeout)"):
                    journey.complete_prospective_summary("synthetic-token", **scope)
                self.assertEqual(chat.call_count, 5)
                self.assertEqual(len(journey.summary_candidate_ids), 10)

    def test_v2_restart_rejects_fence_source_or_selection_drift(self):
        scope, legacy, candidate, snapshot = self.summary_fixture()
        for mutation in ("fence", "source", "selection", "extra_dispatch"):
            journey = object.__new__(RUNNER.Journey)
            journey.summary_legacy_ids, journey.summary_candidate_ids = legacy, candidate[:10].copy()
            journey.summary_saved = CONTROL.summary_window(snapshot(), legacy, candidate[:10], scope)
            journey.private_report, journey.report = {}, {"journey": {}}
            value = snapshot(11)
            if mutation == "fence": value["turns"][16]["summary_claim_attempt"] += 1
            if mutation == "source": value["messages"][14]["content"] = "changed source"
            with mock.patch.object(journey, "summary_snapshot", return_value=value), \
                    mock.patch.object(journey, "require_summary_calls",
                                      side_effect=RUNNER.QualificationFailure("summary_logical_dispatch_mismatch")
                                      if mutation == "extra_dispatch" else None):
                with self.assertRaises(RUNNER.QualificationFailure):
                    journey.verify_summary_restart(**scope, resumed_turn_id=candidate[10],
                                                    selected=2 if mutation == "selection" else 1)

    def test_v2_summary_snapshot_is_bounded_and_scope_checked(self):
        scope, _, _, snapshot = self.summary_fixture()
        journey = object.__new__(RUNNER.Journey)
        journey.prefix = "nwq-abcdef1234"
        with mock.patch.object(RUNNER.diagnostic, "bounded_command",
                               return_value=CONTROL.canonical(snapshot())) as command:
            journey.summary_snapshot(**scope, timeout=1.5)
            self.assertEqual(command.call_args.kwargs["timeout"], 1.5)
            self.assertIn("PGOPTIONS=-c statement_timeout=3000", command.call_args.args[0])
            self.assertIn(b":'user_id'::uuid", command.call_args.kwargs["stdin"])
            self.assertNotIn(scope["user_id"].encode(), command.call_args.kwargs["stdin"])
            command.reset_mock()
            with self.assertRaises(RUNNER.QualificationFailure):
                journey.summary_snapshot(**{**scope, "user_id": "untrusted'"})
            command.assert_not_called()

    @unittest.skipUnless(os.environ.get("NW_H4_TEST_POSTGRES"), "isolated PostgreSQL opt-in required")
    def test_v2_summary_snapshot_real_postgres_schema(self):
        """Use actual public column types, session-local tables and rollback only.

        NW_H4_TEST_POSTGRES names an already-owned migrated PostgreSQL container
        with the journey's novel/novel_world defaults (override via
        NW_H4_TEST_PGUSER/NW_H4_TEST_PGDATABASE). No service is started.
        """
        scope, legacy, candidate, snapshot = self.summary_fixture()
        journey = object.__new__(RUNNER.Journey)
        journey.prefix = "nwq-schema-probe"
        bounded = RUNNER.diagnostic.bounded_command
        raw = None

        def execute(command, *, stdin, timeout):
            # The production method supplies the SQL and scope bindings verbatim.
            command = list(command)
            command[command.index("psql") - 1] = os.environ["NW_H4_TEST_POSTGRES"]
            command[command.index("-U") + 1] = os.environ.get("NW_H4_TEST_PGUSER", "novel")
            command[command.index("-d") + 1] = os.environ.get("NW_H4_TEST_PGDATABASE", "novel_world")
            command += ["-q", "-v", "probe_fixture=" + json.dumps(raw)]
            setup = b"""BEGIN;
SET LOCAL standard_conforming_strings = on;
DO $$ BEGIN
  IF EXISTS (SELECT 1 FROM pg_attribute
             WHERE attrelid = 'public.chat_messages'::regclass
               AND attname = 'persona_source_chapter_high_water' AND NOT attisdropped) THEN
    RAISE EXCEPTION 'message provenance must come from the turn';
  END IF;
END $$;
CREATE TEMP TABLE chat_turns AS SELECT * FROM public.chat_turns WITH NO DATA;
CREATE TEMP TABLE chat_messages AS SELECT * FROM public.chat_messages WITH NO DATA;
CREATE TEMP TABLE character_memories AS SELECT * FROM public.character_memories WITH NO DATA;
INSERT INTO chat_turns SELECT * FROM jsonb_populate_recordset(NULL::chat_turns, :'probe_fixture'::jsonb->'turns');
INSERT INTO chat_messages SELECT * FROM jsonb_populate_recordset(NULL::chat_messages, :'probe_fixture'::jsonb->'messages');
INSERT INTO character_memories SELECT * FROM jsonb_populate_recordset(NULL::character_memories, :'probe_fixture'::jsonb->'mid');
"""
            return bounded(command, stdin=setup + stdin + b"ROLLBACK;\n", timeout=timeout)

        with mock.patch.object(RUNNER.diagnostic, "bounded_command", side_effect=execute):
            for count in (0, 10, 11):
                raw = snapshot(count)
                for row in raw["messages"]:
                    row.pop("persona_source_chapter_high_water")
                actual = journey.summary_snapshot(**scope)
                saved = CONTROL.summary_window(actual, legacy, candidate[:count], scope)
                self.assertEqual(saved is not None, count >= 10)
                projected = {row["id"]: row for row in actual["messages"]}
                for row in raw["messages"]:
                    self.assertTrue(all(projected[row["id"]][key] == value for key, value in row.items()))
                    self.assertEqual(projected[row["id"]]["persona_source_chapter_high_water"], 4)
            for field, wrong in (("user_id", str(uuid.uuid4())), ("novel_id", str(uuid.uuid4())),
                                 ("character_id", str(uuid.uuid4())), ("turn_id", str(uuid.uuid4())),
                                 ("reader_identity", "different reader"), ("chapter_context", 3)):
                raw = snapshot()
                for row in raw["messages"]:
                    row.pop("persona_source_chapter_high_water")
                raw["messages"][14][field] = wrong
                actual = journey.summary_snapshot(**scope)
                with self.assertRaises(CONTROL.DiagnosticFailure, msg=field):
                    CONTROL.summary_window(actual, legacy, candidate[:10], scope)
                if field in ("reader_identity", "chapter_context", "turn_id"):
                    row = next(row for row in actual["messages"] if row["id"] == raw["messages"][14]["id"])
                    self.assertEqual(row[field], wrong)
                    self.assertIsNone(row["persona_source_chapter_high_water"])

    def test_v2_report_identity_and_numeric_only_summary_evidence(self):
        self.value["schema"] = CONTROL.REGISTRATION_SCHEMA_V2
        self.value["network_subnet"] = None
        self.value["product_fixture_sha256"] = CONTROL.digest(
            (ROOT / CONTROL.product_fixture(CONTROL.REGISTRATION_SCHEMA_V2)).read_bytes())
        journey = self.journey()
        private_id = str(uuid.uuid4())
        journey.report["journey"].update(legacy_chat_turns=7, prospective_chat_turns=11,
                                        summary_logical_calls=1, summary_memory_id=private_id)
        journey.private_report["prospective_summary_before_restart"] = {"content": "private-summary"}
        public = journey.public_report()
        self.assertEqual(public["report_kind"], "h4-vision-diagnostic-v2")
        self.assertEqual(public["schema_version"], 3)
        self.assertEqual(public["aggregate"]["counts"]["summary_logical_calls"], 1)
        self.assertNotIn(private_id, json.dumps(public))
        self.assertNotIn("private-summary", json.dumps(public))
        self.assertIsNone(journey.report["policy_identity"]["qualification"])
        self.assertIsNone(journey.report["policy_identity"]["extraction"])

    def test_v2_final_observation_rejects_extra_logical_summary_generation(self):
        journey = self.journey()
        journey.prospective_summary, journey.summary_saved = True, {"saved": True}
        summary = {"counter_totals": [{"service": "agent-service", "operation": "memory_summary",
                                     "counter": "requests_started.total", "value": 2}]}
        parser = mock.Mock()
        parser.verify_many.return_value = {"passed": True}
        with mock.patch.object(journey, "collect_metrics"), \
                mock.patch.object(journey, "collect_response_models"), \
                mock.patch.object(RUNNER, "verify_response_models", return_value={}), \
                mock.patch.object(RUNNER, "summarize_metrics", return_value=summary), \
                mock.patch.object(RUNNER, "load_metric_parser", return_value=parser):
            with self.assertRaisesRegex(RUNNER.QualificationFailure, "summary_logical_dispatch_mismatch"):
                journey.finalize_observability("diagnostic-terminal")


if __name__ == "__main__":
    unittest.main()
