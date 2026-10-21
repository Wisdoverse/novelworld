#!/usr/bin/env python3
"""Fixed-cohort observer checks; --native adds one disposable PostgreSQL fixture."""

from contextlib import redirect_stderr, redirect_stdout
import copy
from datetime import datetime, timedelta, timezone
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch
import uuid

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "infra/ops/import_readiness.py"
SPEC = importlib.util.spec_from_file_location("import_readiness", SOURCE)
observer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(observer)
PG_IMAGE = "pgvector/pgvector:pg18@sha256:2ba9ca5f2e7daa0f0e7723cba1ee9167bab54efd3640516a44ac1a928dd67e7a"
NATIVE = "--native" in sys.argv
if NATIVE:
    sys.argv.remove("--native")
ORIGIN = datetime(2026, 1, 1, tzinfo=timezone.utc)
IDS = [str(uuid.UUID(int=n)) for n in (1, 2, 3, 4)]


class ObserverTest(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.reg_path = Path(self.directory.name) / "registration.json"
        self.evidence_path = Path(self.directory.name) / "evidence.json"
        self.reg = {
            "version": 1, "source_sha": "a" * 40, "profile": "single-node-v1",
            "database": {"name": "novelworld", "system_identifier": "123456789"},
            "cutoff_seconds": 120,
            "jobs": [{"novel_id": job_id, "created_at": ORIGIN.isoformat()} for job_id in IDS[:2]],
        }
        self.write_registration()

    def write_registration(self):
        self.reg_path.write_text(json.dumps(self.reg))
        return observer.registration(self.reg_path)

    def sample(self, seconds=10, ready=()):
        return {
            "database": self.reg["database"], "read_only": "on",
            "observed_at": (ORIGIN + timedelta(seconds=seconds)).isoformat(),
            "rows": [{"novel_id": job["novel_id"], "created_at": job["created_at"],
                      "job_status": "completed" if index in ready else "pending",
                      "novel_status": "ready" if index in ready else "pending"}
                     for index, job in enumerate(self.reg["jobs"])],
        }

    def observe(self, sample):
        with patch.object(observer, "collect", return_value=sample):
            return observer.observe(self.reg_path, self.evidence_path, "fixture", "postgres")

    def cli(self, sample=None, failure=None):
        stdout, stderr = io.StringIO(), io.StringIO()
        with patch.object(observer, "collect", return_value=sample, side_effect=failure), \
                redirect_stdout(stdout), redirect_stderr(stderr):
            status = observer.main(["--registration", str(self.reg_path), "--evidence",
                                    str(self.evidence_path), "--container", "fixture",
                                    "--db-user", "postgres"])
        return status, stdout.getvalue(), stderr.getvalue()

    def test_registration_rejects_invalid_bounds_and_identity(self):
        for key, invalid in (("version", True), ("source_sha", "main"), ("profile", "public"),
                             ("cutoff_seconds", 0), ("cutoff_seconds", True),
                             ("cutoff_seconds", 1.5), ("cutoff_seconds", observer.MAX_CUTOFF + 1),
                             ("jobs", []), ("jobs", [{}] * (observer.MAX_JOBS + 1))):
            with self.subTest(key=key, invalid=invalid):
                value = copy.deepcopy(self.reg)
                value[key] = invalid
                self.reg_path.write_text(json.dumps(value))
                with self.assertRaises(observer.Failure):
                    observer.registration(self.reg_path)
        for db_name in ("host=secret.example password=SECRET", "postgresql://secret", "-option"):
            value = copy.deepcopy(self.reg)
            value["database"]["name"] = db_name
            self.reg_path.write_text(json.dumps(value))
            with self.assertRaises(observer.Failure):
                observer.registration(self.reg_path)

    def test_registration_rejects_duplicate_ids_keys_and_malformed_clocks(self):
        self.reg["jobs"].append(self.reg["jobs"][0])
        with self.assertRaisesRegex(observer.Failure, "duplicate_job"):
            self.write_registration()
        self.reg["jobs"].pop()
        for invalid in ("not-a-uuid", "0' OR true--"):
            self.reg["jobs"][0]["novel_id"] = invalid
            with self.assertRaises(observer.Failure):
                self.write_registration()
        self.reg["jobs"][0]["novel_id"] = IDS[0]
        for invalid in ("2026-01-01T00:00:00", "2026-02-30T00:00:00Z", "infinity",
                        "2026-01-01T00:00:00+99:00", "2026-01-01T00:00:00+00:99",
                        "9999-12-31T23:59:59Z"):
            self.reg["jobs"][0]["created_at"] = invalid
            with self.assertRaises(observer.Failure):
                self.write_registration()
        self.reg_path.write_text('{"version":1,"version":1}')
        with self.assertRaisesRegex(observer.Failure, "duplicate_json_key"):
            observer.registration(self.reg_path)

    def test_initial_enrollment_requires_all_jobs_and_novels_before_cutoff(self):
        for field in ("job_status", "novel_status"):
            sample = self.sample()
            sample["rows"][0][field] = None
            if field == "job_status":
                sample["rows"][0]["created_at"] = None
            with self.assertRaisesRegex(observer.Failure, "initial_enrollment_incomplete"):
                self.observe(sample)
            self.assertFalse(self.evidence_path.exists())
        for seconds in (120, 121):
            with self.assertRaisesRegex(observer.Failure, "initial_enrollment_late"):
                self.observe(self.sample(seconds, ready=(0, 1)))
            self.assertFalse(self.evidence_path.exists())

    def test_good_is_retained_through_retry_regression_deletion_and_duplicate_samples(self):
        self.assertEqual(self.observe(self.sample(10, ready=(0,)))["good"], 1)
        for sample in (self.sample(10, ready=(0,)), self.sample(20), self.sample(30), self.sample(121)):
            sample["rows"][0].update(created_at=None, job_status=None, novel_status=None)
            result = self.observe(sample)
            self.assertEqual((result["denominator"], result["good"], result["bad"]), (2, 1, 0))
            self.assertEqual((result["observed_jobs"], result["missing_jobs"]), (1, 1))
            self.assertEqual(result["readiness_latency_upper_bound_seconds"],
                             {"count": 1, "min": 10.0, "max": 10.0})
        self.assertEqual((result["open"], result["unknown"]), (0, 1))

    def test_missing_and_failed_rows_stay_open_then_unknown(self):
        self.observe(self.sample())
        for seconds in (119, 120, 121):
            sample = self.sample(seconds)
            sample["rows"][0].update(created_at=None, job_status=None, novel_status=None)
            sample["rows"][1].update(job_status="failed", novel_status="error")
            result = self.observe(sample)
            self.assertEqual(result["denominator"], 2)
            self.assertEqual(result["bad"], 0)
            self.assertEqual(result["open"], 2 if seconds < 120 else 0)
            self.assertEqual(result["unknown"], 0 if seconds < 120 else 2)

    def test_both_statuses_and_on_time_clock_are_required_for_good(self):
        self.observe(self.sample())
        sample = self.sample(119, ready=(0, 1))
        sample["rows"][0]["novel_status"] = "parsing"
        sample["rows"][1]["job_status"] = "in_progress"
        self.assertEqual(self.observe(sample)["good"], 0)
        self.assertEqual(self.observe(self.sample(120, ready=(0,)))["good"], 1)
        result = self.observe(self.sample(121, ready=(0, 1)))
        self.assertEqual((result["good"], result["unknown"]), (1, 1))

    def test_identity_snapshot_and_clock_errors_preserve_bytes(self):
        self.observe(self.sample(10, ready=(0,)))
        before = self.evidence_path.read_bytes()
        cases = []
        sample = self.sample(20)
        sample["database"] = {**sample["database"], "system_identifier": "987654321"}
        cases.append(sample)
        sample = self.sample(20)
        sample["rows"][0]["created_at"] = (ORIGIN + timedelta(seconds=1)).isoformat()
        cases.append(sample)
        sample = self.sample(20)
        sample["rows"].pop()
        cases.append(sample)
        sample = self.sample(20)
        sample["rows"][1] = sample["rows"][0]
        cases.append(sample)
        sample = self.sample(20)
        sample["read_only"] = "off"
        cases.extend((sample, self.sample(9)))
        for sample in cases:
            with self.assertRaises(observer.Failure):
                self.observe(sample)
            self.assertEqual(self.evidence_path.read_bytes(), before)

    def test_changed_registration_and_corrupt_evidence_refuse_collection(self):
        self.observe(self.sample())
        saved = self.evidence_path.read_bytes()
        self.reg["source_sha"] = "b" * 40
        self.write_registration()
        with patch.object(observer, "collect") as collect:
            with self.assertRaisesRegex(observer.Failure, "registration_changed"):
                observer.observe(self.reg_path, self.evidence_path, "fixture", "postgres")
            collect.assert_not_called()
        self.assertEqual(self.evidence_path.read_bytes(), saved)
        self.reg["source_sha"] = "a" * 40
        self.write_registration()
        corrupt = json.loads(saved)
        corrupt["jobs"][IDS[0]] = (ORIGIN + timedelta(seconds=121)).isoformat()
        for raw in (b"null", b"{", b"{}", json.dumps(corrupt).encode()):
            self.evidence_path.write_bytes(raw)
            with patch.object(observer, "collect") as collect:
                with self.assertRaises(observer.Failure):
                    observer.observe(self.reg_path, self.evidence_path, "fixture", "postgres")
                collect.assert_not_called()
            self.assertEqual(self.evidence_path.read_bytes(), raw)

    def test_failure_and_success_cli_are_private(self):
        status, stdout, stderr = self.cli(self.sample(10, ready=(0,)))
        self.assertEqual((status, stderr), (0, ""))
        self.assertEqual(set(json.loads(stdout)), {
            "denominator", "good", "bad", "unknown", "open", "observed_jobs", "missing_jobs",
            "readiness_latency_upper_bound_seconds"})
        for private in (*IDS, str(self.directory.name), self.reg["source_sha"], "novelworld"):
            self.assertNotIn(private, stdout)
        before = self.evidence_path.read_bytes()
        status, stdout, stderr = self.cli(failure=observer.Failure("collection_failed"))
        self.assertEqual((status, stdout, json.loads(stderr)), (1, "", {"error": "collection_failed"}))
        self.assertEqual(self.evidence_path.read_bytes(), before)

    def test_collection_error_suppresses_raw_output_and_has_bounded_no_retry(self):
        failed = subprocess.CompletedProcess([], 1, "PRIVATE-CONTENT", "password=SECRET job=" + IDS[0])
        with patch.object(observer.subprocess, "run", return_value=failed) as run:
            with self.assertRaisesRegex(observer.Failure, "^collection_failed$"):
                observer.collect(self.reg, "fixture", "postgres")
            self.assertEqual(run.call_count, 1)
            self.assertEqual(run.call_args.kwargs["timeout"], 15)
        for failure in (subprocess.TimeoutExpired("SECRET", 15), OSError("SECRET")):
            with patch.object(observer.subprocess, "run", side_effect=failure):
                with self.assertRaisesRegex(observer.Failure, "^collection_failed$"):
                    observer.collect(self.reg, "fixture", "postgres")
        for container, user in (("-option", "postgres"), ("fixture", "password=SECRET")):
            with patch.object(observer.subprocess, "run") as run:
                with self.assertRaises(observer.Failure):
                    observer.collect(self.reg, container, user)
                run.assert_not_called()

    def test_private_permissions_symlink_and_atomic_failure(self):
        self.observe(self.sample())
        for path in (self.evidence_path, Path(str(self.evidence_path) + ".lock")):
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
        before = self.evidence_path.read_bytes()
        with patch.object(observer.os, "replace", side_effect=OSError("SECRET")):
            status, stdout, stderr = self.cli(self.sample(20, ready=(0,)))
        self.assertEqual((status, stdout, json.loads(stderr)), (1, "", {"error": "observation_failed"}))
        self.assertEqual(self.evidence_path.read_bytes(), before)
        self.assertEqual(list(Path(self.directory.name).glob(".import-readiness-*")), [])
        self.evidence_path.chmod(0o644)
        with self.assertRaisesRegex(observer.Failure, "unsafe_evidence_permissions"):
            self.observe(self.sample(20))
        self.evidence_path.chmod(0o600)
        alias = Path(self.directory.name) / "alias.json"
        alias.symlink_to(self.evidence_path)
        with self.assertRaises(OSError):
            observer.observe(self.reg_path, alias, "fixture", "postgres")
        self.assertEqual(self.evidence_path.read_bytes(), before)

    def test_stable_lock_rejects_separate_process_through_capture_and_replace(self):
        self.observe(self.sample())
        before = self.evidence_path.read_bytes()

        def competing_process(*args):
            result = subprocess.run([sys.executable, str(SOURCE), "--registration", str(self.reg_path),
                                     "--evidence", str(self.evidence_path), "--container", "unused",
                                     "--db-user", "postgres"], capture_output=True, text=True, timeout=5)
            self.assertEqual((result.returncode, result.stdout, json.loads(result.stderr)),
                             (1, "", {"error": "observation_in_progress"}))
            self.assertEqual(self.evidence_path.read_bytes(), before)
            return self.sample(20, ready=(0,))

        original_save = observer.save

        def save(path, value):
            competing_process()
            original_save(path, value)

        with patch.object(observer, "collect", side_effect=competing_process), \
                patch.object(observer, "save", side_effect=save):
            result = observer.observe(self.reg_path, self.evidence_path, "fixture", "postgres")
        self.assertEqual(result["good"], 1)


@unittest.skipUnless(NATIVE, "use --native for the disposable PostgreSQL fixture")
class NativePostgresTest(unittest.TestCase):
    @classmethod
    def command(cls, command, sql=None):
        result = subprocess.run(command, input=sql, capture_output=True, text=True, timeout=60)
        if result.returncode:
            raise RuntimeError("disposable PostgreSQL fixture command failed")
        return result.stdout.strip()

    @classmethod
    def sql(cls, sql):
        return cls.command(["docker", "exec", "-i", cls.container, "psql", "-X", "-qAt",
                            "-v", "ON_ERROR_STOP=1", "-U", "postgres", "-d", "readiness"], sql)

    @classmethod
    def setUpClass(cls):
        cls.container = "nw-import-readiness-" + uuid.uuid4().hex
        cls.addClassCleanup(lambda: cls.command(["docker", "rm", "-f", "-v", cls.container]))
        cls.command(["docker", "run", "-d", "--name", cls.container, "--network", "none",
                     "--tmpfs", "/var/lib/postgresql:rw", "-e", "POSTGRES_HOST_AUTH_METHOD=trust",
                     "-e", "POSTGRES_DB=readiness", PG_IMAGE])
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            try:
                cls.sql("SELECT 1")
                break
            except RuntimeError:
                time.sleep(0.2)
        else:
            raise RuntimeError("disposable PostgreSQL readiness timed out")
        cls.sql((ROOT / "infra/postgres/init.sql").read_text())

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.reg_path = Path(self.directory.name) / "registration.json"
        self.evidence_path = Path(self.directory.name) / "evidence.json"
        self.ids = [str(uuid.uuid4()) for _ in range(4)]
        for job_id, novel, stage, job, attempt, lease, failure in zip(
                self.ids, ("pending", "parsing", "error", "ready"),
                ("source", "chapters", "enriched", "completed"),
                ("pending", "in_progress", "failed", "completed"), (0, 1, 1, 1),
                ("NULL", "now() + interval '1 minute'", "NULL", "NULL"),
                ("NULL", "NULL", "'fixture_failure'", "NULL")):
            self.sql(f"""INSERT INTO novels(id,user_id,title,status)
                VALUES ('{job_id}','{uuid.uuid4()}','PRIVATE-SYNTHETIC-TITLE','{novel}');
                INSERT INTO novel_import_jobs(novel_id,stage,status,attempt,lease_expires_at,failure_code)
                VALUES ('{job_id}','{stage}','{job}',{attempt},{lease},{failure});""")
        ids = ",".join("'" + job_id + "'::uuid" for job_id in self.ids)
        self.reg = json.loads(self.sql(f"""SELECT json_build_object(
            'version',1,'source_sha',repeat('a',40),'profile','single-node-v1','cutoff_seconds',120,
            'database',json_build_object('name',current_database(),'system_identifier',
                (SELECT system_identifier::text FROM pg_control_system())),
            'jobs',(SELECT json_agg(json_build_object('novel_id',novel_id,'created_at',created_at))
                FROM novel_import_jobs WHERE novel_id = ANY(ARRAY[{ids}])));"""))
        self.reg_path.write_text(json.dumps(self.reg))

    def observe(self):
        return observer.observe(self.reg_path, self.evidence_path, self.container, "postgres")

    def test_actual_schema_clock_statuses_retry_regression_and_missing_rows(self):
        before = observer.timestamp(json.loads(self.sql("SELECT to_json(clock_timestamp())")))
        result = self.observe()
        after = observer.timestamp(json.loads(self.sql("SELECT to_json(clock_timestamp())")))
        evidence = json.loads(self.evidence_path.read_text())
        self.assertLessEqual(before, observer.timestamp(evidence["last_observed_at"]))
        self.assertLessEqual(observer.timestamp(evidence["last_observed_at"]), after)
        self.assertEqual((result["denominator"], result["good"], result["open"]), (4, 1, 3))
        self.sql(f"""UPDATE novel_import_jobs SET status='in_progress',stage='chapters',
            attempt=attempt+1,lease_expires_at=now()+interval '1 minute',failure_code=NULL
            WHERE novel_id IN ('{self.ids[2]}','{self.ids[3]}');
            UPDATE novels SET status='parsing' WHERE id IN ('{self.ids[2]}','{self.ids[3]}');""")
        result = self.observe()
        self.assertEqual((result["denominator"], result["good"]), (4, 1))
        self.sql(f"DELETE FROM novels WHERE id IN ('{self.ids[0]}','{self.ids[3]}')")
        result = self.observe()
        self.assertEqual((result["denominator"], result["good"], result["missing_jobs"]), (4, 1, 2))
        self.assertEqual(json.loads(self.evidence_path.read_text())["jobs"][self.ids[3]],
                         evidence["jobs"][self.ids[3]])

    def test_native_read_only_transaction_rejects_mutation_and_preserves_evidence(self):
        self.observe()
        before = self.evidence_path.read_bytes()
        query = observer.query(self.reg)
        injected = query.replace("SELECT json_build_object(",
                                 "DELETE FROM public.novel_import_jobs;\nSELECT json_build_object(", 1)
        with patch.object(observer, "query", return_value=injected):
            with self.assertRaisesRegex(observer.Failure, "collection_failed"):
                self.observe()
        self.assertEqual(self.evidence_path.read_bytes(), before)
        self.assertEqual(self.sql(f"SELECT count(*) FROM novel_import_jobs WHERE novel_id='{self.ids[0]}'"), "1")

    def test_native_initial_missing_late_wrong_database_and_origin(self):
        self.sql(f"DELETE FROM novel_import_jobs WHERE novel_id='{self.ids[0]}'")
        with self.assertRaisesRegex(observer.Failure, "initial_enrollment_incomplete"):
            self.observe()
        self.assertFalse(self.evidence_path.exists())
        self.reg["jobs"] = [job for job in self.reg["jobs"] if job["novel_id"] != self.ids[0]]
        self.reg["database"]["system_identifier"] = "123"
        self.reg_path.write_text(json.dumps(self.reg))
        with self.assertRaisesRegex(observer.Failure, "database_identity_mismatch"):
            self.observe()
        self.reg["database"]["system_identifier"] = self.sql("SELECT system_identifier FROM pg_control_system()")
        self.reg_path.write_text(json.dumps(self.reg))
        self.sql(f"UPDATE novel_import_jobs SET created_at=now()-interval '10 minutes' WHERE novel_id='{self.ids[1]}'")
        with self.assertRaisesRegex(observer.Failure, "job_origin_mismatch"):
            self.observe()
        for job in self.reg["jobs"]:
            if job["novel_id"] == self.ids[1]:
                job["created_at"] = json.loads(self.sql(
                    f"SELECT to_json(created_at) FROM novel_import_jobs WHERE novel_id='{self.ids[1]}'"))
        self.reg_path.write_text(json.dumps(self.reg))
        with self.assertRaisesRegex(observer.Failure, "initial_enrollment_late"):
            self.observe()
        self.assertFalse(self.evidence_path.exists())


if __name__ == "__main__":
    unittest.main()
